//! Same-device latitude probes must not consume, replace, or restore stale
//! photograph caches. Real film output, cache counters, and controlled overlap
//! are checked together; an implementation that disables all caching fails.

use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

use spektrafilm_core::{params::RuntimeParams, pipeline::Pipeline, profile};
use spektrafilm_gpu::{
    FilmRenderOptions, NegativeCacheStats, SourceIdentity, output::OutputError, wgpu,
    wgpu_backend::WgpuBackend,
};
use spektrafilm_math::{image::ImageBuf, precision::from_f32};

const PHOTO: SourceIdentity = SourceIdentity(0x1001);
const OTHER: SourceIdentity = SourceIdentity(0x1002);
const WAIT: Duration = Duration::from_secs(30);

fn options(source: SourceIdentity) -> FilmRenderOptions<'static> {
    FilmRenderOptions {
        source: Some(source),
        ..Default::default()
    }
}

fn probe_options() -> FilmRenderOptions<'static> {
    FilmRenderOptions {
        // Deliberately supply a photo identity: isolation must override it,
        // rather than relying on a host to remember to clear this field.
        isolate_photo_caches: true,
        ..options(PHOTO)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PhotoCaches {
    density: NegativeCacheStats,
    work_bytes: u64,
}

fn caches(backend: &WgpuBackend) -> PhotoCaches {
    PhotoCaches {
        density: backend.negative_cache_stats(),
        work_bytes: backend.work_image_bytes(),
    }
}

fn render(
    pipeline: &Pipeline,
    backend: &WgpuBackend,
    input: &ImageBuf,
    options: FilmRenderOptions<'_>,
) -> Result<ImageBuf, OutputError> {
    pipeline
        .process_resident_with_options(input, backend, options)
        .map(|result| result.expect("real resident film pipeline required"))
}

/// Own the output copy before the engine releases its private work buffers.
/// The hook runs at the consumer boundary, enabling deterministic nested calls
/// and overlap instead of relying on sleeps or image size to trigger a race.
fn render_at_consumer<T>(
    pipeline: &Pipeline,
    backend: &WgpuBackend,
    input: &ImageBuf,
    options: FilmRenderOptions<'_>,
    hook: impl FnOnce() -> Result<T, OutputError>,
) -> Result<(ImageBuf, T), OutputError> {
    let (readback, width, height, value) = pipeline
        .with_resident_params(input, |params| {
            backend.with_film_output(params, options, None, |device, encoder, output| {
                let value = hook()?;
                let bytes = output.width as u64 * output.height as u64 * 3 * 4;
                let readback = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("probe-isolation-owned-output"),
                    size: bytes,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                encoder.copy_buffer_to_buffer(output.buffer, 0, &readback, 0, bytes);
                Ok((readback, output.width, output.height, value))
            })
        })
        .expect("real resident film parameters required")?;
    let slice = readback.slice(..);
    let (tx, rx) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    backend
        .compute_device()
        .0
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .map_err(|_| OutputError::GpuFailure)?;
    rx.recv_timeout(WAIT)
        .map_err(|_| OutputError::GpuFailure)?
        .map_err(|_| OutputError::GpuFailure)?;
    let mapped = slice
        .get_mapped_range()
        .map_err(|_| OutputError::GpuFailure)?;
    let pixels = mapped
        .chunks_exact(4)
        .map(|bytes| from_f32(f32::from_le_bytes(bytes.try_into().unwrap())))
        .collect();
    drop(mapped);
    readback.unmap();
    Ok((ImageBuf::from_data(width, height, pixels), value))
}

fn assert_pixels(actual: &ImageBuf, expected: &ImageBuf) {
    assert_eq!(
        (actual.width, actual.height),
        (expected.width, expected.height)
    );
    assert_eq!(
        actual.data, expected.data,
        "probe isolation changed film pixels"
    );
}

fn assert_photo_hit(
    pipeline: &Pipeline,
    backend: &WgpuBackend,
    input: &ImageBuf,
    source: SourceIdentity,
    expected: &ImageBuf,
) {
    let before = caches(backend);
    assert_pixels(
        &render(pipeline, backend, input, options(source)).unwrap(),
        expected,
    );
    let after = caches(backend);
    assert_eq!(after.density.hits, before.density.hits + 1);
    assert_eq!(after.density.misses, before.density.misses);
    assert_eq!(after.density.stores, before.density.stores);
    assert_eq!(
        after.density.resident_bytes,
        input.pixel_count() as u64 * 12
    );
    let direct_map = backend
        .compute_device()
        .0
        .features()
        .contains(wgpu::Features::MAPPABLE_PRIMARY_BUFFERS);
    assert_eq!(
        after.work_bytes,
        if direct_map {
            0
        } else {
            input.pixel_count() as u64 * 24
        }
    );
}

fn fixture(width: u32, height: u32) -> ImageBuf {
    let mut input = ImageBuf::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let value = 0.08 + ((x * 19 + y * 7) % 100) as f32 / 140.;
            input.set(
                x,
                y,
                [
                    from_f32(value),
                    from_f32(value * 0.83),
                    from_f32(value * 0.61),
                ],
            );
        }
    }
    input
}

fn latitude_ramp() -> ImageBuf {
    // The FPW latitude probe's dimensions and -12..12 EV neutral input.
    let row: Vec<_> = (0..1024)
        .flat_map(|i| [from_f32(0.184 * 2_f32.powf(-12. + 24. * i as f32 / 1023.)); 3])
        .collect();
    ImageBuf::from_data(1024, 8, row.repeat(8))
}

#[test]
fn same_device_probes_preserve_photo_caches_across_success_abort_and_overlap() {
    let Some(backend) = WgpuBackend::new() else {
        assert!(
            std::env::var_os("SPEKTRAFILM_REQUIRE_GPU").is_none(),
            "required GPU probe-isolation regression cannot be skipped"
        );
        eprintln!("SKIP: no GPU for probe-isolation regression");
        return;
    };
    let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let film = profile::load_profile_by_name(&data, "kodak_portra_400").unwrap();
    let paper = profile::load_profile_by_name(&data, "kodak_portra_endura").unwrap();
    let mut params = RuntimeParams::default();
    params.camera.auto_exposure = false;
    params.film_render.grain.gpu_multilayer = true;
    let pipeline = Pipeline::new_with_spectral(film, paper, params, &data).unwrap();
    let mut point = pipeline.clone();
    point.params.film_render.grain.active = false;
    point.params.film_render.halation.active = false;
    point.params.film_render.glare.active = false;
    point.params.film_render.dir_couplers.diffusion_size_um = 0.;
    point.params.film_render.dir_couplers.diffusion_tail_um = 0.;
    point.params.scanner.unsharp_mask = [0., 0.];
    point.params.io.output_cctf_encoding = false;
    let photo = fixture(32, 16);
    let other = fixture(17, 11);
    let ramp = latitude_ramp();

    // Establish independent cache-off expectations before populating caches.
    // Every later call uses this exact device and backend, including probes.
    let expected_photo = render(&pipeline, &backend, &photo, Default::default()).unwrap();
    let expected_other = render(&pipeline, &backend, &other, Default::default()).unwrap();
    let expected_ramp = render(&point, &backend, &ramp, Default::default()).unwrap();
    let expected_same_size = render(&point, &backend, &photo, Default::default()).unwrap();
    backend.configure_work_images(true);
    backend.configure_negative_cache(true);
    assert_pixels(
        &render(&pipeline, &backend, &photo, options(PHOTO)).unwrap(),
        &expected_photo,
    );
    assert_photo_hit(&pipeline, &backend, &photo, PHOTO, &expected_photo);

    let before = caches(&backend);
    // Both equal-size and resized probes leave the photo pair available even
    // during their own consumer. A checkout-then-restore implementation fails.
    for (input, expected) in [(&photo, &expected_same_size), (&ramp, &expected_ramp)] {
        let (actual, ()) = render_at_consumer(&point, &backend, input, probe_options(), || {
            assert_eq!(caches(&backend), before);
            Ok(())
        })
        .unwrap();
        assert_pixels(&actual, expected);
        assert_eq!(caches(&backend), before);
    }

    let rejected = render_at_consumer(&point, &backend, &ramp, probe_options(), || {
        assert_eq!(caches(&backend), before);
        Err::<(), _>(OutputError::GpuFailure)
    });
    assert!(matches!(rejected, Err(OutputError::GpuFailure)));
    assert_eq!(caches(&backend), before);
    let stop = AtomicBool::new(true);
    let cancellable = FilmRenderOptions {
        cancellation: Some(&stop),
        ..probe_options()
    };
    assert!(matches!(
        render(&point, &backend, &ramp, cancellable),
        Err(OutputError::Cancelled)
    ));
    assert_eq!(caches(&backend), before);
    stop.store(false, Ordering::Release);
    let cancelled = render_at_consumer(&point, &backend, &ramp, cancellable, || {
        stop.store(true, Ordering::Release);
        Ok(())
    });
    assert!(matches!(cancelled, Err(OutputError::Cancelled)));
    assert_eq!(caches(&backend), before);
    stop.store(false, Ordering::Release);
    assert_pixels(
        &render(&point, &backend, &ramp, cancellable).unwrap(),
        &expected_ramp,
    );
    assert_eq!(caches(&backend), before);
    assert_photo_hit(&pipeline, &backend, &photo, PHOTO, &expected_photo);

    // A photo consumer may synchronously run a probe while holding its own
    // exclusive work pair. The nested probe neither returns nor steals it.
    let (actual, ()) = render_at_consumer(&pipeline, &backend, &photo, options(PHOTO), || {
        let in_flight = caches(&backend);
        assert_eq!(in_flight.work_bytes, 0);
        assert_pixels(
            &render(&point, &backend, &ramp, probe_options())?,
            &expected_ramp,
        );
        assert_eq!(caches(&backend), in_flight);
        Ok(())
    })
    .unwrap();
    assert_pixels(&actual, &expected_photo);
    assert_photo_hit(&pipeline, &backend, &photo, PHOTO, &expected_photo);

    // The inverse nesting proves isolation is local to the probe invocation:
    // a normal photo inside it still updates caches, and is not later undone.
    let (actual, updated) = render_at_consumer(&point, &backend, &ramp, probe_options(), || {
        assert_pixels(
            &render(&pipeline, &backend, &other, options(OTHER))?,
            &expected_other,
        );
        Ok(caches(&backend))
    })
    .unwrap();
    assert_pixels(&actual, &expected_ramp);
    assert_eq!(caches(&backend), updated);
    assert_photo_hit(&pipeline, &backend, &other, OTHER, &expected_other);

    // Suspend a photo miss before submission. A concurrent probe must not
    // advance/invalidate its pending density ticket or return a synthetic pair.
    let (entered_tx, entered_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    std::thread::scope(|scope| {
        let backend = &backend;
        let pipeline = &pipeline;
        let photo = &photo;
        let worker = scope.spawn(move || {
            render_at_consumer(pipeline, backend, photo, options(PHOTO), || {
                entered_tx.send(()).unwrap();
                resume_rx.recv_timeout(WAIT).expect("photo must resume");
                Ok(())
            })
        });
        entered_rx
            .recv_timeout(WAIT)
            .expect("photo consumer must start");
        let pending = caches(backend);
        let probe = render(&point, backend, &ramp, probe_options());
        let after_probe = caches(backend);
        resume_tx.send(()).unwrap();
        let photo_result = worker.join().unwrap().unwrap().0;
        assert_pixels(&probe.unwrap(), &expected_ramp);
        assert_eq!(after_probe, pending);
        assert_eq!(
            pending.density.resident_bytes, 0,
            "photo miss must be pending"
        );
        assert_eq!(pending.work_bytes, 0);
        assert_eq!(
            backend.negative_cache_stats().stores,
            pending.density.stores + 1
        );
        assert_pixels(&photo_result, &expected_photo);
    });
    assert_photo_hit(&pipeline, &backend, &photo, PHOTO, &expected_photo);

    // Suspend the probe instead. A concurrently completed photo becomes the
    // current entry and survives probe success, rejection, and cancellation.
    // Error cleanup must not restore a stale entry either.
    for outcome in 0..3 {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        let stop = AtomicBool::new(false);
        let source = SourceIdentity(0x2000 + outcome);
        std::thread::scope(|scope| {
            let backend = &backend;
            let point = &point;
            let ramp = &ramp;
            let stop = &stop;
            let invocation = FilmRenderOptions {
                cancellation: Some(stop),
                ..probe_options()
            };
            let worker = scope.spawn(move || {
                render_at_consumer(point, backend, ramp, invocation, || {
                    entered_tx.send(()).unwrap();
                    resume_rx.recv_timeout(WAIT).expect("probe must resume");
                    if outcome == 1 {
                        return Err(OutputError::GpuFailure);
                    }
                    if outcome == 2 {
                        stop.store(true, Ordering::Release);
                    }
                    Ok(())
                })
            });
            entered_rx
                .recv_timeout(WAIT)
                .expect("probe consumer must start");
            let photo_result = render(&pipeline, backend, &other, options(source));
            let updated = caches(backend);
            resume_tx.send(()).unwrap();
            let probe_result = worker.join().unwrap();
            assert_pixels(&photo_result.unwrap(), &expected_other);
            match outcome {
                0 => assert_pixels(&probe_result.unwrap().0, &expected_ramp),
                1 => assert!(matches!(probe_result, Err(OutputError::GpuFailure))),
                2 => assert!(matches!(probe_result, Err(OutputError::Cancelled))),
                _ => unreachable!(),
            }
            assert_eq!(caches(backend), updated);
        });
        assert_photo_hit(&pipeline, &backend, &other, source, &expected_other);
    }
}
