//! End-to-end density-cache regression using the real film/paper pipeline.
//! Requires a Vulkan adapter in CI; equality alone is paired with cache counters
//! so accidentally bypassing all lookups cannot produce a passing cache test.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use spektrafilm_core::{params::RuntimeParams, pipeline::Pipeline, profile};
use spektrafilm_gpu::{
    FilmRenderOptions, SourceIdentity,
    wgpu_backend::{GrainReference, WgpuBackend},
};
use spektrafilm_math::{image::ImageBuf, precision::from_f32};

fn image(width: u32, height: u32, reverse: bool) -> ImageBuf {
    let mut image = ImageBuf::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let level = (x as f32 * 0.027 + y as f32 * 0.019).min(0.68);
            let v = if reverse { 0.81 - level } else { 0.11 + level };
            image.set(x, y, [from_f32(v), from_f32(v * 0.78), from_f32(v * 0.61)]);
        }
    }
    image
}

fn render(
    pipeline: &Pipeline,
    backend: &WgpuBackend,
    image: &ImageBuf,
    source: Option<SourceIdentity>,
) -> ImageBuf {
    pipeline
        .process_resident_with_options(
            image,
            backend,
            FilmRenderOptions {
                source,
                cancellation: None,
            },
        )
        .expect("valid film render")
        .expect("test requires the resident GPU route")
}

fn compare(
    pipeline: &Pipeline,
    cached: &WgpuBackend,
    oracle: &WgpuBackend,
    image: &ImageBuf,
    source: SourceIdentity,
    hit: bool,
) {
    let before = cached.negative_cache_stats();
    let actual = render(pipeline, cached, image, Some(source));
    let expected = pipeline
        .process_resident_borrowed(image, oracle)
        .expect("cache-off oracle must run the GPU film chain");
    assert_eq!(
        actual.data, expected.data,
        "cache must preserve every output channel"
    );
    let after = cached.negative_cache_stats();
    assert_eq!(
        after.hits - before.hits,
        u64::from(hit),
        "unexpected hit count"
    );
    assert_eq!(
        after.misses - before.misses,
        u64::from(!hit),
        "unexpected miss count"
    );
    assert_eq!(
        after.stores - before.stores,
        u64::from(!hit),
        "only a completed miss stores"
    );
    assert_eq!(after.resident_bytes, image.pixel_count() as u64 * 3 * 4);
}

#[test]
fn negative_cache_preserves_full_film_pixels_with_exact_stage_invalidation() {
    let Some(mut cached) = WgpuBackend::new() else {
        assert!(
            std::env::var_os("SPEKTRAFILM_REQUIRE_GPU").is_none(),
            "GPU required: refusing skipped density-cache success"
        );
        return;
    };
    // Two independent cache states on the very same device/driver/queue.
    let (device, queue) = cached.compute_device();
    let mut oracle = WgpuBackend::from_device(device.clone(), queue.clone());
    cached.configure_work_images(true);
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop();
    path.pop();
    path.push("data");
    let film = profile::load_profile_by_name(&path, "kodak_portra_400").unwrap();
    let paper = profile::load_profile_by_name(&path, "kodak_portra_endura").unwrap();
    let make = |params: RuntimeParams| {
        Pipeline::new_with_spectral(film.clone(), paper.clone(), params, &path)
            .expect("real film and spectral data required")
    };
    let mut params = RuntimeParams::default();
    params.camera.auto_exposure = false;
    params.film_render.grain.gpu_multilayer = true;
    let base = make(params.clone());
    let first = image(16, 12, false);
    let other = image(16, 12, true);
    // Equal channel bytes and token, different row geometry: size matching alone
    // cannot distinguish the optical/grain interpretation of these pixels.
    let resized = ImageBuf::from_data(24, 8, first.data.clone());
    let source = SourceIdentity(0x1001);

    let initial = cached.negative_cache_stats();
    render(&base, &cached, &first, Some(source));
    let disabled = cached.negative_cache_stats();
    assert_eq!(disabled.resident_bytes, 0, "cache defaults to disabled");
    assert_eq!(disabled.stores, initial.stores);

    cached.configure_negative_cache(true);
    compare(&base, &cached, &oracle, &first, source, false);
    compare(&base, &cached, &oracle, &first, source, true);
    let before = cached.negative_cache_stats();
    let unidentified = render(&base, &cached, &first, None);
    let expected = render(&base, &oracle, &first, None);
    assert_eq!(unidentified.data, expected.data);
    let after = cached.negative_cache_stats();
    assert_eq!(after.bypasses, before.bypasses + 1);
    assert_eq!(after.hits, before.hits);
    assert_eq!(after.stores, before.stores);

    // Each pipeline is rebuilt so private calibration/LUT values are refreshed.
    let mut changed = params.clone();
    changed.enlarger.print_exposure = 1.2;
    compare(&make(changed), &cached, &oracle, &first, source, true);
    let mut changed = params.clone();
    changed.scanner.unsharp_mask = [0.9, 0.2];
    compare(&make(changed), &cached, &oracle, &first, source, true);
    let mut changed = params.clone();
    changed.io.output_cctf_encoding = false;
    compare(&make(changed), &cached, &oracle, &first, source, true);
    let mut changed = params.clone();
    changed.io.scan_film = true;
    compare(&make(changed), &cached, &oracle, &first, source, true);

    let mut changed = params.clone();
    changed.camera.exposure_compensation_ev = 0.25;
    compare(&make(changed), &cached, &oracle, &first, source, false);
    compare(&base, &cached, &oracle, &first, source, false);
    let mut changed = params.clone();
    changed.film_render.density_curve_gamma = 1.05;
    compare(&make(changed), &cached, &oracle, &first, source, false);
    compare(&base, &cached, &oracle, &first, source, false);
    let mut changed = params.clone();
    changed.film_render.grain.blur = 0.8;
    compare(&make(changed), &cached, &oracle, &first, source, false);
    compare(&base, &cached, &oracle, &first, source, false);
    let mut changed = params.clone();
    changed.film_render.grain.gpu_multilayer = false;
    compare(&make(changed), &cached, &oracle, &first, source, false);
    compare(&base, &cached, &oracle, &first, source, false);

    // An explicit coordinate transform must invalidate even at identical sizes.
    let reference = GrainReference::new([1., 0., 2., 0., 1., 3.], [64, 64], first.width);
    cached.configure_grain_reference(Some(reference));
    oracle.configure_grain_reference(Some(reference));
    compare(&base, &cached, &oracle, &first, source, false);
    compare(&base, &cached, &oracle, &first, source, true);
    cached.configure_grain_reference(None);
    oracle.configure_grain_reference(None);
    compare(&base, &cached, &oracle, &first, source, false);
    compare(&base, &cached, &oracle, &resized, source, false);
    compare(
        &base,
        &cached,
        &oracle,
        &other,
        SourceIdentity(0x1003),
        false,
    );
    compare(&base, &cached, &oracle, &first, source, false);

    // Overlapping source identities on one shared backend cannot mix density.
    let expected_first = render(&base, &oracle, &first, None);
    let expected_other = render(&base, &oracle, &other, None);
    let (a, b) = std::thread::scope(|scope| {
        let a = scope.spawn(|| render(&base, &cached, &first, Some(source)));
        let b = scope.spawn(|| render(&base, &cached, &other, Some(SourceIdentity(0x1003))));
        (a.join().unwrap(), b.join().unwrap())
    });
    assert_eq!(a.data, expected_first.data);
    assert_eq!(b.data, expected_other.data);

    cached.configure_negative_cache(false);
    assert_eq!(cached.negative_cache_stats().resident_bytes, 0);
    cached.configure_negative_cache(true);
    compare(&base, &cached, &oracle, &first, source, false);
    cached.configure_negative_cache_budget_bytes(1);
    assert_eq!(cached.negative_cache_stats().resident_bytes, 0);
    let before = cached.negative_cache_stats();
    assert_eq!(
        render(&base, &cached, &first, Some(source)).data,
        expected_first.data
    );
    let after = cached.negative_cache_stats();
    assert_eq!(after.resident_bytes, 0);
    assert_eq!(after.bypasses, before.bypasses + 1);
    assert_eq!(after.stores, before.stores);

    cached.configure_negative_cache_budget_bytes(384 * 1024 * 1024);
    cached.clear_negative_cache();
    let cancelled = AtomicBool::new(true);
    let before = cached.negative_cache_stats();
    let result = base.process_resident_with_options(
        &first,
        &cached,
        FilmRenderOptions {
            source: Some(source),
            cancellation: Some(&cancelled),
        },
    );
    assert!(
        result.is_err(),
        "pre-cancelled render must not return partial output"
    );
    assert_eq!(cached.negative_cache_stats().stores, before.stores);
    assert_eq!(cached.negative_cache_stats().resident_bytes, 0);
    cancelled.store(false, Ordering::Relaxed);
    let recovered = base
        .process_resident_with_options(
            &first,
            &cached,
            FilmRenderOptions {
                source: Some(source),
                cancellation: Some(&cancelled),
            },
        )
        .expect("successful render after cancellation")
        .expect("resident GPU output");
    assert_eq!(recovered.data, expected_first.data);
    assert_eq!(cached.negative_cache_stats().stores, before.stores + 1);
    compare(&base, &cached, &oracle, &first, source, true);
}
