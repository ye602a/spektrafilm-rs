use std::{fs, io::Write, path::PathBuf, time::Instant};
use spektrafilm_core::{params::RuntimeParams, pipeline::Pipeline, profile};
use spektrafilm_gpu::wgpu_backend::{GrainReference, WgpuBackend};
use spektrafilm_math::{image::ImageBuf, precision::{from_f32, to_f32}};

#[cfg(feature = "candidate")]
use spektrafilm_core::stock_setup::{AcceptedStockSetup, StockRoute};

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let data = PathBuf::from(&args[1]);
    let output = PathBuf::from(&args[2]);
    fs::create_dir_all(&output).unwrap();
    let mut backend = WgpuBackend::new().expect("real Vulkan device required");
    assert!(backend.compute_device().0.features().contains(wgpu::Features::PASSTHROUGH_SHADERS));
    let cases = [
        ("paper_pro400h", "fujifilm_pro_400h", 0),
        ("slide_provia", "fujifilm_provia_100f", 1),
        ("slide_velvia", "fujifilm_velvia_100", 1),
        ("negative_pro400h_ap0", "fujifilm_pro_400h", 2),
        ("negative_xtra_ap0", "fujifilm_xtra_400", 2),
    ];
    let mut records = Vec::new();
    for (case, stock, route) in cases {
        for source_mode in [false, true] {
            let name = format!("{case}_{}", if source_mode { "source" } else { "local" });
            let (width, height) = if source_mode { (65, 33) } else { (32, 19) };
            let mut input = ImageBuf::new(width, height);
            for y in 0..height {
                for x in 0..width {
                    let r = 0.015 + x as f32 / (width - 1) as f32 * 1.3;
                    let g = 0.025 + y as f32 / (height - 1) as f32 * 0.83;
                    let b = 0.02 + ((x * 17 + y * 11) % 67) as f32 / 66.0 * 0.94;
                    let flash = if (x + 3 * y) % 31 == 0 { 2.0 } else { 1.0 };
                    input.set(x, y, [r * flash, g * flash, b * flash].map(from_f32));
                }
            }
            let mut p = RuntimeParams::default();
            p.camera.auto_exposure = false;
            p.camera.exposure_compensation_ev = 0.25;
            p.camera.film_format_mm = 36.0;
            p.enlarger.print_exposure_compensation = false;
            p.enlarger.normalize_print_exposure = true;
            p.enlarger.print_exposure = 1.15;
            p.settings.neutral_print_filters_from_database = true;
            p.io.input_color_space = "sRGB".into();
            p.io.input_cctf_decoding = false;
            p.io.output_color_space = "sRGB".into();
            p.io.output_cctf_encoding = true;
            p.film_render.grain.active = true;
            p.film_render.grain.sublayers_active = true;
            p.film_render.grain.blur = 0.65;
            p.film_render.grain.blur_dye_clouds_um = 1.0;
            p.film_render.grain.output_sampling = source_mode.then_some([2.0, 0.75]);
            #[cfg(feature = "candidate")]
            { p.film_render.grain.gpu_multilayer = true; }
            p.film_render.halation.active = true;
            p.film_render.halation.halation_amount = 1.0;
            p.film_render.dir_couplers.active = true;
            p.camera.diffusion_filter.active = false;
            if route != 0 {
                p.io.scan_film = true;
                p.film_render.glare.active = false;
            }
            if route == 2 {
                p.io.output_color_space = "ACES2065-1".into();
                p.io.output_cctf_encoding = false;
                p.io.output_gamut_compress.algorithm = "off".into();
                p.scanner.unsharp_mask = [0., 0.];
            }
            let film = profile::load_profile_by_name(&data, stock).unwrap();
            let paper = profile::load_profile_by_name(&data, "kodak_portra_endura").unwrap();
            #[cfg(not(feature = "candidate"))]
            let pipeline = {
                // Exact host stock statements from pinned FPW film.rs 108–122.
                let sigma = if film.info.usage == "cine" { 50.0 } else { 65.0 };
                p.film_render.halation.halation_first_sigma_um = [sigma; 3];
                p.film_render.halation.halation_strength = match film.info.antihalation.as_str() {
                    "strong" => [0.015, 0.005, 0.0],
                    "weak" => [0.08, 0.02, 0.0],
                    "no" => [0.30, 0.10, 0.015],
                    _ => p.film_render.halation.halation_strength,
                };
                let mut pipeline = Pipeline::new_with_spectral(film, paper, p, &data).unwrap();
                if route == 1 {
                    let scale = match stock {
                        "fujifilm_provia_100f" => 1.3,
                        "fujifilm_velvia_100" => 0.9,
                        _ => unreachable!(),
                    };
                    let d = &mut pipeline.params.film_render.dir_couplers;
                    d.gamma_samelayer_rgb = [0.12 * scale, 0.08 * scale, 0.06 * scale];
                    d.gamma_interlayer_r_to_gb = [0.12 * scale, 0.06 * scale];
                    d.gamma_interlayer_g_to_rb = [0.08 * scale, 0.06 * scale];
                    d.gamma_interlayer_b_to_rg = [0.06 * scale, 0.06 * scale];
                }
                pipeline
            };
            #[cfg(feature = "candidate")]
            let pipeline = Pipeline::new_with_spectral_and_stock_setup(
                film, paper, p, &data,
                AcceptedStockSetup::FpwV1(match route {
                    0 => StockRoute::Paper,
                    1 => StockRoute::Slide,
                    2 => StockRoute::NegativeScan,
                    _ => unreachable!(),
                }),
            ).unwrap();
            backend.configure_grain_reference(source_mode.then(||
                GrainReference::new([0., 1., 2., -1., 0., 96.], [128, 128], width)));
            let start = Instant::now();
            let image = pipeline.process_resident_borrowed(&input, &backend)
                .expect("no CPU fallback: resident full physical output required");
            let elapsed = start.elapsed().as_secs_f64();
            let values: Vec<f32> = image.data.into_iter().map(to_f32).collect();
            assert!(values.iter().all(|v| v.is_finite()), "nonfinite output in {name}");
            let mut file = fs::File::create(output.join(format!("{name}.f32"))).unwrap();
            for v in &values { file.write_all(&v.to_le_bytes()).unwrap(); }
            let mut input_file = fs::File::create(output.join(format!("{name}.input.f32"))).unwrap();
            for &v in &input.data { input_file.write_all(&to_f32(v).to_le_bytes()).unwrap(); }
            fs::write(output.join(format!("{name}.params.json")), serde_json::to_vec_pretty(&pipeline.params).unwrap()).unwrap();
            let record = serde_json::json!({"name":name,"stock":stock,"route":route,"width":width,"height":height,
                "source_coordinates":source_mode,"output_sampling":pipeline.params.film_render.grain.output_sampling,
                "channels":values.len(),"seconds":elapsed,"output_color_space":pipeline.params.io.output_color_space,
                "encoded":pipeline.params.io.output_cctf_encoding});
            eprintln!("{}", record);
            records.push(record);
        }
    }
    fs::write(output.join("cases.json"), serde_json::to_vec_pretty(&records).unwrap()).unwrap();
}
