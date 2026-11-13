//! Cross-revision qualification probe; synthetic inputs, not FPW RAW parity.
//! Build/run this identical file on the pinned baseline and candidate revision.

use std::path::{Path, PathBuf};

use spektrafilm_core::{params::RuntimeParams, pipeline::Pipeline, profile};
use spektrafilm_gpu::wgpu_backend::{GrainReference, WgpuBackend};
use spektrafilm_math::{image::ImageBuf, precision::from_f32};

fn fixture(width: u32, height: u32, variant: u32) -> ImageBuf {
    let mut image = ImageBuf::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let ramp = ((x * 13 + y * 7 + variant * 11) % 97) as f32 / 96.0;
            let tile = ((x / 3 + y / 2 + variant) % 3) as f32 * 0.04;
            let mut pixel = [
                0.01 + ramp * 0.82,
                0.02 + (1.0 - ramp) * 0.63 + tile,
                0.015 + ((x * 5 + y * 17 + variant * 3) % 61) as f32 / 60.0 * 0.72,
            ];
            if x == width / 2 && y == height / 2 {
                pixel = [1.5, 1.15, 0.85];
            } else if x == 0 && y == 0 {
                pixel = [0.0, 0.0, 0.0];
            }
            image.set(x, y, pixel.map(from_f32));
        }
    }
    image
}

fn write_f32(path: &Path, image: &ImageBuf) {
    let data: Vec<u8> = image
        .data
        .iter()
        .flat_map(|v| (*v as f32).to_le_bytes())
        .collect();
    std::fs::write(path, data).expect("write complete f32 pixels");
}

fn main() {
    assert!(
        std::env::var_os("SPEKTRAFILM_REQUIRE_GPU").is_some(),
        "qualification requires SPEKTRAFILM_REQUIRE_GPU=1"
    );
    let output = PathBuf::from(std::env::args().nth(1).expect("output directory argument"));
    std::fs::create_dir_all(output.join("inputs")).unwrap();
    std::fs::create_dir_all(output.join("params")).unwrap();
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let film = profile::load_profile_by_name(&data_dir, "kodak_portra_400").unwrap();
    let paper = profile::load_profile_by_name(&data_dir, "kodak_portra_endura").unwrap();
    let mut backend = WgpuBackend::new().expect("real GPU required; no CPU fallback accepted");
    let mut manifest = Vec::new();
    let cases = [
        ("legacy_print", 17, 11, false, false),
        ("layered_print_cloud_micro", 19, 13, true, false),
        ("legacy_direct_scan_linear", 23, 9, false, true),
        ("layered_direct_scan_wide_iir", 13, 17, true, true),
        ("legacy_print_optics_dir", 21, 15, false, false),
        ("layered_print_paper_scanner", 15, 21, true, false),
        ("legacy_skinny_no_grain", 1, 17, false, false),
        ("layered_source_scaled_amount", 25, 19, true, false),
    ];
    for (index, (name, width, height, layered, scan)) in cases.into_iter().enumerate() {
        let mut params = RuntimeParams::default();
        // Retain the current linear-sRGB host input/output conventions.
        params.io.input_color_space = "sRGB".into();
        params.io.input_cctf_decoding = false;
        params.io.output_color_space = "sRGB".into();
        params.io.scan_film = scan;
        params.camera.auto_exposure = false;
        params.film_render.grain.gpu_multilayer = layered;
        let mut source_reference = None;
        match index {
            0 => {}
            1 => {
                params.camera.film_format_mm = 0.12;
                params.film_render.grain.blur_dye_clouds_um = 8.0;
                params.film_render.grain.micro_structure = [6.0, 400.0];
            }
            2 => {
                params.camera.exposure_compensation_ev = -0.4;
                params.io.output_cctf_encoding = false;
                params.scanner.lens_blur = 0.7;
                params.scanner.unsharp_mask = [0.9, 0.55];
            }
            3 => {
                params.camera.film_format_mm = 0.12;
                params.film_render.grain.blur = 3.0;
                params.film_render.grain.blur_dye_clouds_um = 8.0;
                params.film_render.grain.micro_structure = [20.0, 400.0];
                source_reference = Some(([0.0, 1.0, 23.0, -1.0, 0.0, 50.0], [100, 80]));
            }
            4 => {
                params.camera.film_format_mm = 0.18;
                params.camera.exposure_compensation_ev = 0.4;
                params.camera.lens_blur_um = 3.0;
                params.camera.diffusion_filter.active = true;
                params.camera.diffusion_filter.strength = 0.125;
                params.film_render.halation.boost_ev = 0.2;
                params.film_render.dir_couplers.amount = 0.65;
                params.enlarger.lens_blur = 1.1;
                params.enlarger.diffusion_filter.active = true;
                params.enlarger.diffusion_filter.strength = 0.125;
                params.scanner.lens_blur = 0.7;
                params.scanner.unsharp_mask = [0.9, 0.6];
            }
            5 => {
                params.camera.film_format_mm = 0.14;
                params.enlarger.print_exposure = 1.3;
                params.enlarger.y_filter_shift = 3.0;
                params.enlarger.m_filter_shift = -2.0;
                params.enlarger.preflash_exposure = 0.01;
                params.print_render.density_curve_gamma = 1.17;
                params.print_render.glare.percent = 0.05;
                params.scanner.white_correction = true;
                params.scanner.black_correction = true;
                params.io.output_gamut_compress.algorithm = "off".into();
            }
            6 => {
                params.film_render.grain.active = false;
                params.film_render.halation.active = false;
                params.film_render.dir_couplers.active = false;
                params.print_render.glare.active = false;
                params.scanner.unsharp_mask = [0.0, 0.0];
            }
            7 => {
                params.camera.film_format_mm = 0.2;
                params.film_render.grain.output_sampling = Some([1.8, 0.65]);
                params.film_render.grain.blur_dye_clouds_um = 8.0;
                source_reference = Some(([1.0, 0.0, 7.0, 0.0, 1.0, 9.0], [100, 80]));
            }
            _ => unreachable!(),
        }
        backend.configure_grain_reference(
            source_reference.map(|(matrix, source)| GrainReference::new(matrix, source, width)),
        );
        // The cache's reported bytes also prove the resident GPU film chain
        // was used. Stage-by-stage CPU fallback cannot satisfy this assertion.
        backend.configure_work_images(false);
        backend.configure_work_images(true);
        let pipeline = Pipeline::new_with_spectral(film.clone(), paper.clone(), params, &data_dir)
            .expect("calibrated Portra 400 / Endura pipeline");
        let input = fixture(width, height, index as u32);
        write_f32(&output.join("inputs").join(format!("{name}.f32")), &input);
        std::fs::write(
            output.join("params").join(format!("{name}.json")),
            serde_json::to_vec_pretty(&pipeline.params).unwrap(),
        )
        .unwrap();
        let result = pipeline.process(input, &backend);
        assert_eq!((result.width, result.height), (width, height));
        assert_eq!(result.data.len(), width as usize * height as usize * 3);
        assert!(
            result.data.iter().all(|v| v.is_finite()),
            "{name}: nonfinite result"
        );
        assert_eq!(
            backend.work_image_bytes(),
            width as u64 * height as u64 * 24,
            "{name}: a full resident GPU render was required"
        );
        write_f32(&output.join(format!("{name}.f32")), &result);
        let min = result
            .data
            .iter()
            .map(|v| *v as f64)
            .fold(f64::INFINITY, f64::min);
        let max = result
            .data
            .iter()
            .map(|v| *v as f64)
            .fold(f64::NEG_INFINITY, f64::max);
        let mean = result.data.iter().map(|v| *v as f64).sum::<f64>() / result.data.len() as f64;
        manifest.push(serde_json::json!({
            "name": name, "width": width, "height": height,
            "gpu_multilayer": layered, "scan_film": scan,
            "source_reference": source_reference,
            "bytes": result.data.len() * 4, "min": min, "max": max, "mean": mean,
        }));
        println!(
            "{name}: {width}x{height}, {} f32 values, min={min}, max={max}, mean={mean}",
            result.data.len()
        );
    }
    std::fs::write(
        output.join("manifest.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "fixture": "deterministic synthetic linear-sRGB input; not frozen FPW RAW photos",
            "reference_revision": "85eb835a0116b8ef77beb4952ae80272a21b0b8a",
            "format": "complete interleaved RGB f32 little-endian",
            "film": "kodak_portra_400", "paper": "kodak_portra_endura", "cases": manifest
        }))
        .unwrap(),
    )
    .unwrap();
}
