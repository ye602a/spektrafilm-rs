//! Real-film-chain work-buffer residency regression. Runs on an actual Vulkan
//! adapter in Linux CI; never substitutes CPU rendering for the oracle.
use std::path::PathBuf;
use spektrafilm_core::{params::RuntimeParams, pipeline::Pipeline, profile};
use spektrafilm_gpu::wgpu_backend::WgpuBackend;
use spektrafilm_math::{image::ImageBuf, precision::from_f32};

fn fixture(width: u32, height: u32, invert: bool) -> ImageBuf {
    let mut img = ImageBuf::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let v = (x as f32 * 0.037 + y as f32 * 0.021).min(0.78);
            let p = if invert { 0.82 - v } else { 0.12 + v };
            img.set(x, y, [from_f32(p), from_f32((p * 0.8).min(1.)), from_f32(p * 0.65)]);
        }
    }
    img
}

fn require_backend() -> Option<WgpuBackend> {
    let gpu = WgpuBackend::new();
    if gpu.is_none() && std::env::var_os("SPEKTRAFILM_REQUIRE_GPU").is_some() {
        panic!("required GPU backend unavailable: refusing skipped success");
    }
    gpu
}

#[test]
fn gpu_work_planes_do_not_change_film_pixels_or_leak_between_photos() {
    let Some(backend) = require_backend() else { return };
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop();
    path.pop();
    path.push("data");
    let film = profile::load_profile_by_name(&path, "kodak_portra_400").unwrap();
    let paper = profile::load_profile_by_name(&path, "kodak_portra_endura").unwrap();
    let mut params = RuntimeParams::default();
    params.film_render.grain.gpu_multilayer = false;
    let pipeline = Pipeline::new_with_spectral(film, paper, params, &path)
        .expect("real film pipeline fixture required");

    let first = fixture(16, 12, false);
    let other = fixture(16, 12, true);
    let resized = fixture(17, 11, true);

    // Frozen route without retained intermediate allocations.
    backend.configure_work_images(false);
    let oracle_first = pipeline.process(first.clone(), &backend);
    let oracle_other = pipeline.process(other.clone(), &backend);
    let oracle_resized = pipeline.process(resized.clone(), &backend);
    assert_eq!(backend.work_image_bytes(), 0, "default cache must stay off");

    backend.configure_work_images(true);
    let a = pipeline.process(first.clone(), &backend);
    assert_eq!(oracle_first.data, a.data);
    let pair_bytes = first.pixel_count() as u64 * 3 * 4 * 2;
    assert_eq!(backend.work_image_bytes(), pair_bytes);

    let b = pipeline.process(other, &backend);
    assert_eq!(oracle_other.data, b.data, "second image must not inherit old pixels");
    assert_eq!(backend.work_image_bytes(), pair_bytes);

    let again = pipeline.process(first, &backend);
    assert_eq!(oracle_first.data, again.data, "returning to first image must reproduce");
    assert_eq!(backend.work_image_bytes(), pair_bytes);

    let next = pipeline.process(resized.clone(), &backend);
    assert_eq!(oracle_resized.data, next.data, "resize must evict old pair");
    let resized_bytes = resized.pixel_count() as u64 * 3 * 4 * 2;
    assert_eq!(backend.work_image_bytes(), resized_bytes);

    backend.configure_work_image_budget_bytes(1);
    assert_eq!(backend.work_image_bytes(), 0, "shrinking budget must release old pair");
    let no_cache = pipeline.process(resized, &backend);
    assert_eq!(oracle_resized.data, no_cache.data);
    assert_eq!(backend.work_image_bytes(), 0, "over-budget render must not retain memory");

    backend.configure_work_image_budget_bytes(1536 * 1024 * 1024);
    backend.configure_work_images(false);
    assert_eq!(backend.work_image_bytes(), 0, "disable must release residency");
}
