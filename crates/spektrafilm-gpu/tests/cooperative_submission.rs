//! Cooperative GPU progress must preserve host preemption without imposing
//! intermediate submissions on ordinary or cached rendering.
#![cfg(feature = "wgpu-backend")]

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use spektrafilm_gpu::{
    DirCouplersGpuParams, FilmChainParams, FilmRenderOptions, FilmSubmissionPolicy, FrontPass,
    HalationGpuParams, OutputError, SourceIdentity, wgpu_backend::WgpuBackend,
};
use spektrafilm_math::{image::ImageBuf, precision::from_f32};

fn backend() -> Option<WgpuBackend> {
    let backend = WgpuBackend::new();
    if backend.is_none() {
        assert!(std::env::var_os("SPEKTRAFILM_REQUIRE_GPU").is_none());
        eprintln!("SKIP: no Vulkan adapter for cooperative submission test");
    }
    backend
}

fn image(width: u32, height: u32) -> ImageBuf {
    ImageBuf::from_data(width, height, vec![from_f32(0.18); width as usize * height as usize * 3])
}

fn params(image: &ImageBuf) -> FilmChainParams<'_> {
    FilmChainParams {
        image,
        front: FrontPass::Mallett2019 {
            matrix: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        },
        film_log_exposure: &[-3., 1.],
        film_density_curves_normalized: &[[0.; 3], [3.; 3]],
        film_gamma: 1.,
        film_channel_density: &[[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        film_base_density: &[0.; 3],
        print_illuminant: &[1.; 3],
        print_sensitivity: &[[1.; 3]; 3],
        print_normalization_factor: 1.,
        print_log_exposure: &[-3., 1.],
        print_density_curves: &[[0.; 3], [3.; 3]],
        print_gamma: 1.,
        print_channel_density: &[[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        print_base_density: &[0.; 3],
        preflash: [0.; 3],
        viewing_illuminant: &[1.; 3],
        scan_normalization: 1.,
        scan_xyz_to_rgb: &[[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        bw_xyz_remap: None,
        scan_film: false,
        halation: None,
        dir_couplers: None,
        grain: None,
        glare: None,
        gamut: None,
        unsharp: None,
        camera_lens_blur_px: None,
        scanner_lens_blur_px: None,
        highlight_boost: None,
        diffusion: None,
        enlarger_diffusion: None,
        print_exposure_scale: 1.,
        output_cctf_encoding: false,
    }
}

#[test]
fn detailed_halation_and_dir_checkpoints_preserve_complete_output_bits() {
    let Some(backend) = backend() else { return };
    // Just over the retained >2048² threshold. Small, nonzero blur radii keep
    // software Vulkan practical while exercising both scatter blurs, two
    // halation bounces and both DIR diffusion blurs across actual submissions.
    let mut input = image(2049, 2048);
    for (index, pixel) in input.data.chunks_exact_mut(3).enumerate() {
        let x = index % input.width as usize;
        let y = index / input.width as usize;
        pixel[0] = from_f32(0.02 + ((x * 17 + y * 13) % 251) as f32 / 300.0);
        pixel[1] = from_f32(0.03 + ((x * 7 + y * 19) % 241) as f32 / 310.0);
        pixel[2] = from_f32(0.01 + ((x * 23 + y * 11) % 239) as f32 / 290.0);
    }
    let mut p = params(&input);
    p.halation = Some(HalationGpuParams {
        scatter_amount: 0.15,
        scatter_core_px: 0.45,
        scatter_tail_px: 0.9,
        scatter_tail_weight: [0.2, 0.3, 0.4],
        halation_amount: 0.1,
        halation_strength_avg: 0.2,
        halation_a_tot: [0.01, 0.02, 0.03],
        halation_first_sigma_px: 0.5,
        halation_n_bounces: 2,
        halation_bounce_decay: 0.5,
        halation_renormalize: true,
    });
    p.dir_couplers = Some(DirCouplersGpuParams {
        couplers_matrix_scaled: [[0.03, 0.01, 0.02], [0.02, 0.04, 0.01], [0.01, 0.02, 0.05]],
        density_max: [3.; 3],
        is_positive: false,
        diffusion_size_px: 0.6,
        diffusion_tail_px: 1.,
        diffusion_tail_weight: 0.3,
        density_curves_0: p.film_density_curves_normalized,
        log_exposure: p.film_log_exposure,
        gamma_factor: p.film_gamma,
    });
    let stop = AtomicBool::new(false);
    let options = FilmRenderOptions {
        cancellation: Some(&stop),
        ..Default::default()
    };
    let expected = backend.run_film_chain_with_options(&p, options).unwrap();
    assert!(expected.data.iter().all(|value| value.is_finite()));
    assert!(expected.data.chunks_exact(3).any(|pixel| {
        pixel[0].to_bits() != expected.data[0].to_bits()
    }), "fixture must retain spatial detail rather than saturating to one value");

    // No source identity and no grain: after the detailed DIR checkpoint there
    // is neither a snapshot copy nor new negative work, so the otherwise empty
    // after-negative submission must be skipped before printing continues.
    let actual = backend.run_film_chain_with_options(&p, FilmRenderOptions {
        submission_policy: FilmSubmissionPolicy::CooperativeStages,
        ..options
    }).unwrap();
    assert_eq!((actual.width, actual.height), (expected.width, expected.height));
    assert_eq!(actual.data.len(), expected.data.len());
    let first_difference = actual.data.iter().zip(&expected.data)
        .position(|(actual, expected)| actual.to_bits() != expected.to_bits());
    assert_eq!(first_difference, None, "internal stage submissions changed an output channel");
    assert_eq!(backend.negative_cache_stats().resident_bytes, 0);
}

#[test]
fn cancellation_preserves_only_completed_negative_and_skips_consumer_then_recovers() {
    let Some(backend) = backend() else { return };
    backend.configure_work_images(true);
    backend.configure_negative_cache(true);
    let small = image(17, 11);
    let expected = backend.run_film_chain(&params(&small));
    let (_, queue) = backend.compute_device();
    // The second geometry crosses the existing >2048² detailed boundary.
    // It is cancelled after the first optical segment, before density/grain.
    for (width, height) in [(17, 11), (2049, 2048)] {
        backend.clear_negative_cache();
        let input = image(width, height);
        let stop = Arc::new(AtomicBool::new(false));
        let delivered = Arc::new(AtomicBool::new(false));
        let callback_stop = stop.clone();
        let callback_delivered = delivered.clone();
        // A pending host completion callback is delivered at the next native
        // submit/poll. No timing assumption or racing cancellation thread is
        // needed: cooperative rendering must observe it before its consumer.
        queue.on_submitted_work_done(move || {
            callback_delivered.store(true, Ordering::Release);
            callback_stop.store(true, Ordering::Release);
        });
        assert!(!delivered.load(Ordering::Acquire));
        let options = FilmRenderOptions {
            source: Some(SourceIdentity(71)),
            cancellation: Some(stop.as_ref()),
            submission_policy: FilmSubmissionPolicy::CooperativeStages,
            ..Default::default()
        };
        let before = backend.negative_cache_stats();
        let called = std::cell::Cell::new(false);
        let result = backend.with_film_output(&params(&input), options, None, |_, _, _| {
            called.set(true);
            Ok(())
        });
        assert_eq!(result, Err(OutputError::Cancelled));
        assert!(delivered.load(Ordering::Acquire));
        assert!(!called.get(), "cancellation reached final output encoding");
        let completed_negative = width == small.width && height == small.height;
        let after_cancel = backend.negative_cache_stats();
        assert_eq!(after_cancel.stores, before.stores + u64::from(completed_negative));
        assert_eq!(
            after_cancel.resident_bytes,
            if completed_negative { width as u64 * height as u64 * 12 } else { 0 },
        );
        assert_eq!(backend.work_image_bytes(), 0);

        stop.store(false, Ordering::Release);
        let recovered = backend.run_film_chain_with_options(&params(&small), options).unwrap();
        assert_eq!(recovered.data, expected.data);
        let before_hit = backend.negative_cache_stats();
        assert_eq!(before_hit.stores, before.stores + 1);
        assert_eq!(before_hit.hits, after_cancel.hits + u64::from(completed_negative));
        assert_eq!(before_hit.misses, after_cancel.misses + u64::from(!completed_negative));
        let repeated = backend.run_film_chain_with_options(&params(&small), options).unwrap();
        assert_eq!(repeated.data, expected.data);
        assert_eq!(backend.negative_cache_stats().hits, before_hit.hits + 1);
    }
}

#[test]
fn consumer_failure_keeps_completed_negative_without_submitting_output() {
    let Some(backend) = backend() else { return };
    backend.configure_work_images(true);
    backend.configure_negative_cache(true);
    let input = image(17, 11);
    let expected = backend.run_film_chain(&params(&input));
    let (device, _) = backend.compute_device();
    let marker = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cooperative-consumer-abort-marker"),
        size: 16,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: true,
    });
    marker.slice(..).get_mapped_range_mut().unwrap().copy_from_slice(&[0x55; 16]);
    marker.unmap();
    let read_marker = || {
        let slice = marker.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        }).unwrap();
        rx.recv().unwrap().unwrap();
        let mapped = slice.get_mapped_range().unwrap();
        let data = mapped.to_vec();
        drop(mapped);
        marker.unmap();
        data
    };
    for cancel_in_consumer in [false, true] {
        backend.clear_negative_cache();
        let stop = AtomicBool::new(false);
        let options = FilmRenderOptions {
            source: Some(SourceIdentity(91)),
            cancellation: Some(&stop),
            submission_policy: FilmSubmissionPolicy::CooperativeStages,
            ..Default::default()
        };
        let before = backend.negative_cache_stats();
        let result = backend.with_film_output(&params(&input), options, None, |_, encoder, _| {
            assert_eq!(backend.negative_cache_stats().stores, before.stores + 1);
            encoder.clear_buffer(&marker, 0, None);
            if cancel_in_consumer {
                stop.store(true, Ordering::Release);
                Ok(())
            } else {
                Err(OutputError::GpuFailure)
            }
        });
        assert_eq!(result, Err(if cancel_in_consumer {
            OutputError::Cancelled
        } else {
            OutputError::GpuFailure
        }));
        // Only the negative stage was submitted. A failed consumer cannot
        // mutate the host's output destination or publish a completed frame.
        assert_eq!(read_marker(), [0x55; 16]);
        let retained = backend.negative_cache_stats();
        assert_eq!(retained.stores, before.stores + 1);
        assert_eq!(retained.resident_bytes, input.width as u64 * input.height as u64 * 12);
        assert_eq!(backend.work_image_bytes(), 0);

        stop.store(false, Ordering::Release);
        let recovered = backend.run_film_chain_with_options(&params(&input), options).unwrap();
        assert_eq!(recovered.data, expected.data);
        let after = backend.negative_cache_stats();
        assert_eq!(after.hits, retained.hits + 1);
        assert_eq!(after.stores, retained.stores);
    }
}

#[test]
fn cleared_or_disabled_stage_snapshot_cannot_reappear_after_final_output() {
    let Some(backend) = backend() else { return };
    let backend = Arc::new(backend);
    backend.configure_work_images(true);
    let input = image(17, 11);
    let expected = backend.run_film_chain(&params(&input));
    let (_, queue) = backend.compute_device();
    for disable in [false, true] {
        for invalidate_at_completion in [false, true] {
            backend.configure_negative_cache(true);
            backend.clear_negative_cache();
            let stop = AtomicBool::new(false);
            let options = FilmRenderOptions {
                source: Some(SourceIdentity(101)),
                cancellation: Some(&stop),
                submission_policy: FilmSubmissionPolicy::CooperativeStages,
                ..Default::default()
            };
            let before = backend.negative_cache_stats();
            let invalidated = Arc::new(AtomicBool::new(false));
            if invalidate_at_completion {
                let callback_backend = backend.clone();
                let callback_invalidated = invalidated.clone();
                // Invalidating during the checkpoint's submit/poll must make
                // its captured publication ticket ineligible before publish.
                queue.on_submitted_work_done(move || {
                    if disable {
                        callback_backend.configure_negative_cache(false);
                    } else {
                        callback_backend.clear_negative_cache();
                    }
                    callback_invalidated.store(true, Ordering::Release);
                });
                assert!(!invalidated.load(Ordering::Acquire));
            }
            backend.with_film_output(&params(&input), options, None, |_, _, _| {
                if invalidate_at_completion {
                    assert!(invalidated.load(Ordering::Acquire));
                    assert_eq!(backend.negative_cache_stats().stores, before.stores);
                } else {
                    // Also invalidate a snapshot that was already published
                    // at the negative boundary, before final output succeeds.
                    assert_eq!(backend.negative_cache_stats().stores, before.stores + 1);
                    if disable {
                        backend.configure_negative_cache(false);
                    } else {
                        backend.clear_negative_cache();
                    }
                }
                Ok(())
            }).unwrap();
            let after = backend.negative_cache_stats();
            assert_eq!(after.stores, before.stores + u64::from(!invalidate_at_completion));
            assert_eq!(after.resident_bytes, 0);

            // Re-enabling cannot restore the discarded snapshot. A fresh
            // request has to miss and produce a new, numerically equal one.
            backend.configure_negative_cache(true);
            assert_eq!(backend.negative_cache_stats().resident_bytes, 0);
            let recovered = backend.run_film_chain_with_options(&params(&input), options).unwrap();
            assert_eq!(recovered.data, expected.data);
            let recovered_stats = backend.negative_cache_stats();
            assert_eq!(recovered_stats.misses, after.misses + 1);
            assert_eq!(recovered_stats.hits, after.hits);
            assert_eq!(recovered_stats.stores, after.stores + 1);
        }
    }
}

#[test]
fn whole_chain_no_signal_and_cached_cooperative_encode_before_any_submission() {
    let Some(backend) = backend() else { return };
    backend.configure_work_images(true);
    backend.configure_negative_cache(true);
    let input = image(17, 11);
    let stop = AtomicBool::new(false);
    let source = Some(SourceIdentity(81));
    let cached = FilmRenderOptions { source, ..Default::default() };
    backend.run_film_chain_with_options(&params(&input), cached).unwrap();
    let (_, queue) = backend.compute_device();
    let cases = [
        // Cancellation alone must not opt an existing caller into more waits.
        FilmRenderOptions { cancellation: Some(&stop), ..Default::default() },
        FilmRenderOptions {
            submission_policy: FilmSubmissionPolicy::CooperativeStages,
            ..Default::default()
        },
        FilmRenderOptions {
            source,
            cancellation: Some(&stop),
            submission_policy: FilmSubmissionPolicy::CooperativeStages,
            ..Default::default()
        },
    ];
    for (index, options) in cases.into_iter().enumerate() {
        let submitted = Arc::new(AtomicBool::new(false));
        let callback = submitted.clone();
        queue.on_submitted_work_done(move || callback.store(true, Ordering::Release));
        backend.with_film_output(&params(&input), options, None, |_, _, _| {
            assert!(!submitted.load(Ordering::Acquire), "case {index} submitted before output encoding");
            Ok(())
        }).unwrap();
        assert!(submitted.load(Ordering::Acquire), "render never completed its submission");
    }
    assert_eq!(backend.negative_cache_stats().hits, 1);
}
