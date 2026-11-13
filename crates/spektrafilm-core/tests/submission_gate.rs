//! FPW's shared-device surface gate covers submission only. Real GPU markers
//! distinguish a cancelled, unsubmitted command from work that ran and was
//! merely reported as cancelled afterward.

use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

use spektrafilm_core::{params::RuntimeParams, pipeline::Pipeline, profile};
use spektrafilm_gpu::{
    ComputeBackend, FilmRenderOptions, SourceIdentity, output::OutputError, wgpu,
    wgpu_backend::WgpuBackend,
};
use spektrafilm_math::{image::ImageBuf, precision::from_f32};

const WAIT: Duration = Duration::from_secs(30);
const BLOCKED: Duration = Duration::from_millis(100);

fn options(source: u128) -> FilmRenderOptions<'static> {
    FilmRenderOptions {
        source: Some(SourceIdentity(source)),
        ..Default::default()
    }
}

fn consume(
    pipeline: &Pipeline,
    backend: &WgpuBackend,
    input: &ImageBuf,
    options: FilmRenderOptions<'_>,
    hook: impl FnOnce(&mut wgpu::CommandEncoder) -> Result<(), OutputError>,
) -> Result<(), OutputError> {
    pipeline
        .with_resident_params(input, |params| {
            backend.with_film_output(params, options, None, |_, encoder, _| hook(encoder))
        })
        .expect("resident film parameters required")
}

fn marker(backend: &WgpuBackend) -> wgpu::Buffer {
    let buffer = backend
        .compute_device()
        .0
        .create_buffer(&wgpu::BufferDescriptor {
            label: Some("host-submission-gate-marker"),
            size: 16,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: true,
        });
    buffer
        .slice(..)
        .get_mapped_range_mut()
        .unwrap()
        .copy_from_slice(&[0x55; 16]);
    buffer.unmap();
    buffer
}

fn marker_bytes(backend: &WgpuBackend, marker: &wgpu::Buffer) -> Vec<u8> {
    let (tx, rx) = mpsc::channel();
    marker
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
    backend
        .compute_device()
        .0
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
    rx.recv_timeout(WAIT).unwrap().unwrap();
    let mapped = marker.slice(..).get_mapped_range().unwrap();
    let bytes = mapped.to_vec();
    drop(mapped);
    marker.unmap();
    bytes
}

fn fixture() -> ImageBuf {
    let data = (0..17 * 11 * 3)
        .map(|i| from_f32(0.05 + (i * 29 % 170) as f32 / 200.))
        .collect();
    ImageBuf::from_data(17, 11, data)
}

#[test]
fn host_submission_gate_blocks_only_submit_and_rechecks_waiting_cancellation() {
    // Preserve the existing public constructor while adding the optional gate.
    let _: fn(wgpu::Device, wgpu::Queue) -> WgpuBackend = WgpuBackend::from_device;
    let _: fn(wgpu::Device, wgpu::Queue, Option<Arc<Mutex<()>>>) -> WgpuBackend =
        WgpuBackend::from_device_with_submission_lock;

    let Some(host) = WgpuBackend::new() else {
        assert!(
            std::env::var_os("SPEKTRAFILM_REQUIRE_GPU").is_none(),
            "required GPU submission-gate regression cannot be skipped"
        );
        eprintln!("SKIP: no GPU for submission-gate regression");
        return;
    };
    let gate = Arc::new(Mutex::new(()));
    let (device, queue) = host.compute_device();
    let backend = WgpuBackend::from_device_with_submission_lock(
        device.clone(),
        queue.clone(),
        Some(gate.clone()),
    );
    assert_eq!(backend.compute_device().0, device);
    assert_eq!(backend.compute_device().1, queue);
    let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let film = profile::load_profile_by_name(&data, "kodak_portra_400").unwrap();
    let paper = profile::load_profile_by_name(&data, "kodak_portra_endura").unwrap();
    let mut params = RuntimeParams::default();
    params.camera.auto_exposure = false;
    params.film_render.grain.gpu_multilayer = true;
    let pipeline = Pipeline::new_with_spectral(film, paper, params, &data).unwrap();
    let input = fixture();
    let expected = pipeline.process_resident_borrowed(&input, &host).unwrap();
    backend.configure_negative_cache(true);
    backend.configure_work_images(true);
    let actual = pipeline
        .process_resident_with_options(&input, &backend, options(1))
        .unwrap()
        .unwrap();
    assert_eq!(actual.data, expected.data);

    // While the host holds the gate, encoding and the consumer still run. The
    // GPU marker and completion remain blocked until the host releases it.
    let first_marker = marker(&backend);
    let guard = gate.lock().unwrap();
    let (encoded_tx, encoded_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let blocked = std::thread::scope(|scope| {
        scope.spawn(|| {
            let result = consume(&pipeline, &backend, &input, options(2), |encoder| {
                encoder.clear_buffer(&first_marker, 0, None);
                encoded_tx.send(()).unwrap();
                Ok(())
            });
            done_tx.send(result).unwrap();
        });
        encoded_rx
            .recv_timeout(WAIT)
            .expect("consumer must run outside the held gate");
        let blocked = matches!(
            done_rx.recv_timeout(BLOCKED),
            Err(mpsc::RecvTimeoutError::Timeout)
        );
        let unchanged = marker_bytes(&backend, &first_marker);
        drop(guard);
        let completion = done_rx.recv_timeout(WAIT);
        assert!(blocked, "engine submitted while the host gate was held");
        assert_eq!(
            unchanged, [0x55; 16],
            "blocked command unexpectedly executed"
        );
        completion.unwrap().unwrap();
        blocked
    });
    assert!(blocked);
    assert_eq!(marker_bytes(&backend, &first_marker), [0; 16]);

    // An output consumer can briefly acquire the host gate itself. A completion
    // callback attached to THIS encoder runs during the engine's following
    // poll, after submit has returned; it also observes the gate unlocked.
    let (poll_tx, poll_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    std::thread::scope(|scope| {
        let callback_gate = gate.clone();
        scope.spawn(|| {
            let result = consume(&pipeline, &backend, &input, options(2), |encoder| {
                assert!(
                    gate.try_lock().is_ok(),
                    "consumer must run before gate acquisition"
                );
                encoder.on_submitted_work_done(move || {
                    let unlocked = callback_gate.try_lock().is_ok();
                    poll_tx.send(unlocked).unwrap();
                    resume_rx
                        .recv_timeout(WAIT)
                        .expect("completion callback must resume");
                });
                Ok(())
            });
            done_tx.send(result).unwrap();
        });
        let unlocked = poll_rx
            .recv_timeout(WAIT)
            .expect("engine must poll the submitted work");
        let host_guard = gate.try_lock();
        let held_by_host = host_guard.is_ok();
        resume_tx.send(()).unwrap();
        // Keep the host gate until the render has returned, proving there is no
        // reacquisition around GPU wait, completion, or cache publication.
        let completed = done_rx.recv_timeout(WAIT);
        drop(host_guard);
        assert!(unlocked, "engine held the gate during its completion wait");
        assert!(
            held_by_host,
            "host must be able to acquire the gate during poll"
        );
        completed.unwrap().unwrap();
    });

    // A cancellation arriving while submission is blocked must discard the
    // encoded clear, not execute it and only then return Cancelled.
    let cancelled_marker = marker(&backend);
    let stop = AtomicBool::new(false);
    let guard = gate.lock().unwrap();
    let before = backend.negative_cache_stats();
    let (encoded_tx, encoded_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let invocation = FilmRenderOptions {
        cancellation: Some(&stop),
        ..options(3)
    };
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let result = consume(&pipeline, &backend, &input, invocation, |encoder| {
                encoder.clear_buffer(&cancelled_marker, 0, None);
                encoded_tx.send(()).unwrap();
                Ok(())
            });
            done_tx.send(result).unwrap();
        });
        encoded_rx
            .recv_timeout(WAIT)
            .expect("cancellable render must finish encoding");
        let waiting = matches!(
            done_rx.recv_timeout(BLOCKED),
            Err(mpsc::RecvTimeoutError::Timeout)
        );
        stop.store(true, Ordering::Release);
        drop(guard);
        let result = done_rx.recv_timeout(WAIT);
        assert!(
            waiting,
            "request must be waiting at the host gate before cancellation"
        );
        assert_eq!(result.unwrap(), Err(OutputError::Cancelled));
    });
    assert_eq!(marker_bytes(&backend, &cancelled_marker), [0x55; 16]);
    let after = backend.negative_cache_stats();
    assert_eq!(after.misses, before.misses + 1);
    assert_eq!(
        after.stores, before.stores,
        "cancelled submission cannot publish density"
    );
    assert_eq!(after.resident_bytes, 0);
    stop.store(false, Ordering::Release);
    let recovered = pipeline
        .process_resident_with_options(&input, &backend, options(3))
        .unwrap()
        .unwrap();
    assert_eq!(recovered.data, expected.data);
    assert_eq!(backend.negative_cache_stats().stores, before.stores + 1);

    // Cover the other three production submission routes, including legacy
    // infallible methods. Each is warmed first, then blocked on the same gate.
    let identity = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
    let legacy = |route| match route {
        0 => vec![backend.scan_spectral(
            &input,
            &[[0.2, 0.3, 0.4]; 81],
            &[0.1; 81],
            &[1.; 81],
            1.,
            &identity,
            &identity,
        )],
        1 => vec![backend.gaussian_blur(&input, 1.2)],
        2 => backend.gaussian_blur_multi(&input, &[0.7, 1.3]),
        _ => unreachable!(),
    };
    for route in 0..3 {
        let expected = legacy(route);
        let guard = gate.lock().unwrap();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                entered_tx.send(()).unwrap();
                done_tx.send(legacy(route)).unwrap();
            });
            entered_rx.recv_timeout(WAIT).unwrap();
            let waiting = matches!(
                done_rx.recv_timeout(BLOCKED),
                Err(mpsc::RecvTimeoutError::Timeout)
            );
            drop(guard);
            let result = done_rx.recv_timeout(WAIT);
            assert!(waiting, "legacy route {route} bypassed the host gate");
            let actual = result.unwrap();
            assert_eq!(actual.len(), expected.len());
            for (actual, expected) in actual.iter().zip(expected) {
                assert_eq!(actual.data, expected.data);
            }
        });
    }
}
