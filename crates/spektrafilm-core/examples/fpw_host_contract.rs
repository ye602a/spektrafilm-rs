//! Runnable, public-API-only consumer for FPW's accepted physical routes.
//!
//! This is a synthetic integration smoke, not the FPW application or its photo
//! acceptance suite. A real host passes its existing device/queue and maps its
//! persisted recipe into RuntimeParams. No FPW checkout or private assets are
//! required. Run with `cargo run -p spektrafilm-core --example fpw_host_contract`.

use std::{
    error::Error,
    path::Path,
    sync::{Arc, Mutex, atomic::AtomicBool},
};

use spektrafilm_core::{
    calibrated_output_v1::{
        NegativeStockV1, PhysicalOutputMappingV1, PhysicalOutputOptionsV1,
        PhysicalOutputTexturesV1, PhysicalOutputV1,
    },
    params::RuntimeParams,
    pipeline::Pipeline,
    profile,
    stock_setup::{AcceptedStockSetup, StockRoute},
};
use spektrafilm_gpu::{
    DisplayRegion, FilmRenderOptions, FilmSubmissionPolicy, SourceIdentity, wgpu,
    wgpu_backend::WgpuBackend,
};
use spektrafilm_math::{image::ImageBuf, precision::from_f32};

/// Example recipe mapping, kept separate from the engine's versioned stock
/// constants. These are the existing FPW transport/control choices; AP0 is
/// limited to the frozen negative inversion route.
fn recipe_params(route: StockRoute) -> RuntimeParams {
    let mut params = RuntimeParams::default();
    params.camera.auto_exposure = false;
    params.camera.exposure_compensation_ev = 0.0;
    params.camera.film_format_mm = 36.0;
    params.enlarger.print_exposure_compensation = false;
    params.enlarger.normalize_print_exposure = true;
    params.enlarger.print_exposure = 1.0;
    params.settings.neutral_print_filters_from_database = true;
    params.io.input_color_space = "sRGB".into();
    params.io.input_cctf_decoding = false;
    params.io.output_color_space = "sRGB".into();
    params.io.output_cctf_encoding = true;
    params.film_render.grain.active = true;
    params.film_render.grain.sublayers_active = true;
    // FPW uses this existing physical route; standalone RS defaults stay intact.
    params.film_render.grain.gpu_multilayer = true;
    params.film_render.dir_couplers.active = true;
    params.camera.diffusion_filter.active = false;
    match route {
        StockRoute::Paper => {}
        StockRoute::Slide => {
            params.io.scan_film = true;
            params.film_render.glare.active = false;
        }
        StockRoute::NegativeScan => {
            params.io.scan_film = true;
            params.film_render.glare.active = false;
            params.io.output_color_space = "ACES2065-1".into();
            params.io.output_cctf_encoding = false;
            params.io.output_gamut_compress.algorithm = "off".into();
            params.scanner.unsharp_mask = [0.0, 0.0];
        }
    }
    params
}

fn render_frame(
    pipeline: &Pipeline,
    image: &ImageBuf,
    backend: &WgpuBackend,
    output: &PhysicalOutputV1,
    render: FilmRenderOptions<'_>,
    finish: PhysicalOutputOptionsV1,
) -> Result<PhysicalOutputTexturesV1, Box<dyn Error>> {
    render.check_cancelled()?;
    let result = pipeline
        .try_with_resident_params(image, |params| {
            output.render(backend, params, render, finish)
        })?
        .ok_or("accepted physical route unavailable; do not silently select a different look")?;
    // Success here is the publication boundary. The caller can now retain the
    // textures or hand them to its display/post-processing code on this device.
    Ok(result?)
}

fn run_on_host_device(
    device: wgpu::Device,
    queue: wgpu::Queue,
    submission_gate: Arc<Mutex<()>>,
    data: &Path,
) -> Result<(), Box<dyn Error>> {
    let backend = WgpuBackend::from_device_with_submission_lock(
        device.clone(),
        queue.clone(),
        Some(submission_gate.clone()),
    );
    assert_eq!(backend.compute_device(), (&device, &queue));
    backend.configure_work_images(true);
    let output = PhysicalOutputV1::new(&device);
    let mut input = ImageBuf::new(17, 11);
    for y in 0..input.height {
        for x in 0..input.width {
            let value = 0.02 + ((x * 7 + y * 19) % 97) as f32 / 120.0;
            input.set(x, y, [value, value * 0.81, value * 0.63].map(from_f32));
        }
    }
    let cases = [
        (
            StockRoute::Paper,
            "kodak_portra_400",
            PhysicalOutputMappingV1::EncodedSrgb,
        ),
        (
            StockRoute::Slide,
            "fujifilm_provia_100f",
            PhysicalOutputMappingV1::EncodedSrgb,
        ),
        (
            StockRoute::Slide,
            "fujifilm_velvia_100",
            PhysicalOutputMappingV1::EncodedSrgb,
        ),
        (
            StockRoute::NegativeScan,
            "fujifilm_pro_400h",
            PhysicalOutputMappingV1::NegativeScanAp0(NegativeStockV1::Pro400H),
        ),
        (
            StockRoute::NegativeScan,
            "fujifilm_xtra_400",
            PhysicalOutputMappingV1::NegativeScanAp0(NegativeStockV1::XTra400),
        ),
    ];
    let mut published = Vec::new();
    for (index, (route, stock, mapping)) in cases.into_iter().enumerate() {
        let pipeline = Pipeline::new_with_spectral_and_stock_setup(
            profile::load_profile_by_name(data, stock)?,
            profile::load_profile_by_name(data, "kodak_portra_endura")?,
            recipe_params(route),
            data,
            AcceptedStockSetup::FpwV1(route),
        )?;
        // Match FPW's initial paper-only density reuse policy. The source token
        // binds prepared pixels only; RS supplies the film-stage dependencies.
        backend.configure_negative_cache(route == StockRoute::Paper);
        // The host owns this request signal. Cooperative stages retain useful
        // cancellation points without reading intermediate pixels back to CPU.
        let cancellation = AtomicBool::new(false);
        let render = FilmRenderOptions {
            source: (route == StockRoute::Paper).then_some(SourceIdentity(index as u128 + 1)),
            cancellation: Some(&cancellation),
            submission_policy: FilmSubmissionPolicy::CooperativeStages,
            ..Default::default()
        };
        let full = render_frame(
            &pipeline,
            &input,
            &backend,
            &output,
            render,
            PhysicalOutputOptionsV1 {
                mapping,
                ..Default::default()
            },
        )?;
        assert_eq!(full.precision.format(), wgpu::TextureFormat::Rgba16Uint);
        assert_eq!(full.display.format(), wgpu::TextureFormat::Rgba8Unorm);
        assert_eq!((full.width, full.height), (17, 11));
        let before = backend.negative_cache_stats();
        let view = render_frame(
            &pipeline,
            &input,
            &backend,
            &output,
            render,
            PhysicalOutputOptionsV1 {
                mapping,
                finish_centi_ev: 25,
                region: Some(DisplayRegion {
                    crop: [2, 1, 13, 9],
                    width: 7,
                    height: 5,
                }),
            },
        )?;
        assert_eq!((view.width, view.height), (7, 5));
        if route == StockRoute::Paper {
            assert_eq!(backend.negative_cache_stats().hits, before.hits + 1);
        }
        assert!(
            submission_gate.try_lock().is_ok(),
            "gate outlived queue submission"
        );
        // Neither full-size nor display texture borrows the engine work plane.
        published.push(full);
        published.push(view);
        println!(
            "{route:?}: {stock}, full 17x11 and finished region 7x5 submitted on the host device"
        );
    }
    assert_eq!(published.len(), 10);
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    // Headless bootstrap for this standalone example only. FPW supplies its
    // already-created display device using WgpuBackend::device_descriptor.
    // Rewrapping the cloned handles below creates no second GPU device.
    let bootstrap = WgpuBackend::new().ok_or("a Vulkan device is required for this example")?;
    let (device, queue) = bootstrap.compute_device();
    let handles = (device.clone(), queue.clone());
    drop(bootstrap);
    run_on_host_device(
        handles.0,
        handles.1,
        Arc::new(Mutex::new(())),
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data"),
    )
}
