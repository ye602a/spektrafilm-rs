//! The references are extracted from frozen FPW, independently compiled here.
//! Original Python-heldout values provide an additional independent oracle.
//! GPU references compile the original shader fixture, not the production path.

extern crate self as anyhow;

// The original scalar source only uses this narrow anyhow surface. Keep the
// mathematical source unchanged without adding a runtime dependency to RS.
pub type Result<T> = std::result::Result<T, String>;
#[macro_export]
macro_rules! bail {
    ($($arg:tt)*) => { return Err(format!($($arg)*)) };
}
#[macro_export]
macro_rules! ensure {
    ($condition:expr, $($arg:tt)*) => {
        if !$condition { $crate::bail!($($arg)*); }
    };
}

#[rustfmt::skip]
#[path = "fixtures/fpw_finish_v1.rs"]
mod images;
#[rustfmt::skip]
#[allow(unused_imports)]
#[path = "fixtures/fpw_negative_scan_v1.rs"]
mod original_negative;

use serde::Deserialize;
use spektrafilm_core::calibrated_output_v1::{
    finish_exposure_table_v1, positive_rgb16_v1, positive_scan_v1, CalibratedOutputError,
    NegativeStockV1, PhysicalOutputMappingV1, PhysicalOutputOptionsV1, PhysicalOutputTexturesV1,
    PhysicalOutputV1,
};
use spektrafilm_core::{
    params::RuntimeParams,
    pipeline::Pipeline,
    profile,
    stock_setup::{AcceptedStockSetup, StockRoute},
};
use spektrafilm_gpu::{
    output::{DisplayRegion, FilmOutput, OutputError},
    wgpu::{self, util::DeviceExt},
    wgpu_backend::WgpuBackend,
    FilmRenderOptions, SourceIdentity,
};
use spektrafilm_math::{
    image::ImageBuf,
    precision::{from_f32, to_f32},
};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

#[derive(Deserialize)]
struct Heldout {
    stock: String,
    scan: Vec<f32>,
    expected: Vec<f64>,
}

fn heldout() -> Vec<Heldout> {
    serde_json::from_str(include_str!(
        "../../../data/negative_scan_ap0_v1/heldout.json"
    ))
    .unwrap()
}

fn stock_name(stock: Option<NegativeStockV1>) -> Option<&'static str> {
    stock.map(NegativeStockV1::stock_id)
}

fn options(
    stock: Option<NegativeStockV1>,
    finish_centi_ev: i32,
    region: Option<DisplayRegion>,
) -> PhysicalOutputOptionsV1 {
    PhysicalOutputOptionsV1 {
        mapping: stock.map_or(PhysicalOutputMappingV1::EncodedSrgb, |stock| {
            PhysicalOutputMappingV1::NegativeScanAp0(stock)
        }),
        finish_centi_ev,
        region,
    }
}

// This is the original FPW physical_output.rs 257x17 acceptance fixture,
// including the exact node and adjacent representable scan samples.
fn scans(stock: Option<&str>) -> Vec<f32> {
    let mut values: Vec<f32> = (0..257 * 17 * 3)
        .map(|i| {
            let fraction = ((i * 7919) % 65536) as f64 / 65535.;
            stock.map_or(fraction as f32, |s| {
                (original_negative::calibration(s).unwrap().base[i % 3] * (1. - fraction * 1.05))
                    as f32
            })
        })
        .collect();
    if let Some(stock) = stock {
        let cal = original_negative::calibration(stock).unwrap();
        for (i, pixel) in values
            .chunks_exact_mut(3)
            .take(3 * cal.curves.iter().map(|c| c.absorption.len()).max().unwrap())
            .enumerate()
        {
            for c in 0..3 {
                let curve = &cal.curves[c];
                let index = (i / 3).min(curve.absorption.len() - 1);
                let scan = (cal.base[c] * (1. - curve.absorption[index])) as f32;
                pixel[c] = match i % 3 {
                    0 => scan.next_down(),
                    1 => scan,
                    _ => scan.next_up(),
                };
            }
        }
    }
    values
}

#[test]
fn cpu_inversion_matches_original_fpw_and_independent_python_heldout() {
    for fixture in heldout() {
        let actual = positive_scan_v1(&fixture.scan, &fixture.stock, None).unwrap();
        let original = original_negative::positive(&fixture.scan, &fixture.stock).unwrap();
        assert_eq!(actual, original, "{} scalar parity", fixture.stock);
        let max = actual
            .iter()
            .zip(&fixture.expected)
            .map(|(a, b)| (*a as f64 - b).abs())
            .fold(0_f64, f64::max);
        assert!(max <= 2e-6, "{} heldout maximum error {max}", fixture.stock);
    }
    for stock in [NegativeStockV1::Pro400H, NegativeStockV1::XTra400] {
        let scan = scans(Some(stock.stock_id()));
        let original = original_negative::positive(&scan, stock.stock_id()).unwrap();
        let actual = positive_scan_v1(&scan, stock.stock_id(), None).unwrap();
        assert_eq!(actual, original);
        for finish in [-125, 0, 125] {
            let table = images::finish_exposure_table(finish);
            let expected: Vec<u16> = original
                .iter()
                .map(|v| {
                    let code = (*v as f64 * 65535. + 0.5) as u16;
                    if finish == 0 {
                        code
                    } else {
                        table[code as usize]
                    }
                })
                .collect();
            assert_eq!(
                positive_rgb16_v1(&scan, stock.stock_id(), finish, None).unwrap(),
                expected
            );
        }
    }
    for ev in [-400, -75, 0, 125, 400] {
        assert_eq!(
            finish_exposure_table_v1(ev),
            images::finish_exposure_table(ev)
        );
    }
}

#[test]
fn cpu_inversion_rejects_unversioned_stocks_invalid_data_and_cancellation() {
    assert!(matches!(
        positive_scan_v1(&[0.1; 3], "kodak_portra_400", None),
        Err(CalibratedOutputError::UnsupportedStock(_))
    ));
    assert!(matches!(
        positive_scan_v1(&[0.1; 2], "fujifilm_pro_400h", None),
        Err(CalibratedOutputError::InvalidScanLength)
    ));
    assert!(matches!(
        positive_scan_v1(&[f32::NAN, 0.1, 0.2], "fujifilm_pro_400h", None),
        Err(CalibratedOutputError::NonFiniteScan)
    ));
    let stop = AtomicBool::new(true);
    assert!(matches!(
        positive_scan_v1(&[0.1; 3], "fujifilm_pro_400h", Some(&stop)),
        Err(CalibratedOutputError::Output(OutputError::Cancelled))
    ));
    assert!(matches!(
        positive_rgb16_v1(&[0.1; 3], "fujifilm_pro_400h", 100, Some(&stop)),
        Err(CalibratedOutputError::Output(OutputError::Cancelled))
    ));
}

fn backend() -> Option<WgpuBackend> {
    let gpu = WgpuBackend::new();
    if gpu.is_none() {
        assert!(
            std::env::var_os("SPEKTRAFILM_REQUIRE_GPU").is_none(),
            "required calibrated GPU output regression cannot be skipped"
        );
        eprintln!("SKIP: no Vulkan device for calibrated output regression");
    }
    gpu
}

fn storage(device: &wgpu::Device, bytes: &[u8]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("independent FPW fixture input"),
        contents: bytes,
        usage: wgpu::BufferUsages::STORAGE,
    })
}

fn input(device: &wgpu::Device, values: &[f32]) -> wgpu::Buffer {
    storage(
        device,
        &values
            .iter()
            .flat_map(|v| v.to_ne_bytes())
            .collect::<Vec<_>>(),
    )
}

/// Independent driver for the byte-preserved original FPW shader. Node
/// preparation and uniform order are extracted from original Output::render.
/// It never invokes the new output implementation or its calibration parser.
fn original_gpu(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    input: &wgpu::Buffer,
    width: u32,
    height: u32,
    stock: Option<&str>,
    finish: i32,
    region: Option<DisplayRegion>,
) -> PhysicalOutputTexturesV1 {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("independent original FPW output shader"),
        source: wgpu::ShaderSource::Wgsl(
            include_str!("fixtures/fpw_physical_output_v1.wgsl").into(),
        ),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("independent original FPW output"),
        layout: None,
        module: &shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let mut offsets = [0; 4];
    let mut lengths = [0; 4];
    let mut nodes: Vec<[f32; 4]> = vec![];
    if let Some(stock) = stock {
        let cal = original_negative::calibration(stock).unwrap();
        for (c, curve) in cal.curves.iter().enumerate() {
            offsets[c] = nodes.len() as u32;
            lengths[c] = curve.absorption.len() as u32;
            for (i, &a) in curve.absorption.iter().enumerate() {
                let scan = cal.base[c] * (1. - a);
                let high = scan as f32;
                let inverse = curve
                    .absorption
                    .get(i + 1)
                    .map_or(0., |b| 1. / (cal.base[c] * (b - a)));
                nodes.push([
                    high,
                    (scan - high as f64) as f32,
                    curve.asinh_exposure[i] as f32,
                    inverse as f32,
                ]);
            }
        }
    }
    if nodes.is_empty() {
        nodes.push([0.; 4]);
    }
    let curves = storage(
        device,
        &nodes
            .iter()
            .flatten()
            .flat_map(|v| v.to_ne_bytes())
            .collect::<Vec<_>>(),
    );
    let finish_words: Vec<u32> = if finish == 0 {
        vec![0]
    } else {
        images::finish_exposure_table(finish)
            .into_iter()
            .map(u32::from)
            .collect()
    };
    let finish_table = storage(
        device,
        &finish_words
            .iter()
            .flat_map(|v| v.to_ne_bytes())
            .collect::<Vec<_>>(),
    );
    let view = region.unwrap_or(DisplayRegion {
        crop: [0, 0, width, height],
        width,
        height,
    });
    let mut words = vec![width, height, stock.is_some() as u32, (finish != 0) as u32];
    words.extend(view.crop);
    words.extend([view.width, view.height, 0, 0]);
    words.extend(offsets);
    words.extend(lengths);
    let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("original FPW uniform"),
        contents: &words
            .iter()
            .flat_map(|v| v.to_ne_bytes())
            .collect::<Vec<_>>(),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let texture = |format| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("original FPW immutable output"),
            size: wgpu::Extent3d {
                width: view.width,
                height: view.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    };
    let display = texture(wgpu::TextureFormat::Rgba8Unorm);
    let precision = texture(wgpu::TextureFormat::Rgba16Uint);
    let v8 = display.create_view(&Default::default());
    let v16 = precision.create_view(&Default::default());
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: input.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: curves.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: params.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&v8),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(&v16),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: finish_table.as_entire_binding(),
            },
        ],
    });
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(view.width.div_ceil(16), view.height.div_ceil(16), 1);
    }
    PhysicalOutputTexturesV1 {
        display,
        precision,
        width: view.width,
        height: view.height,
    }
}

fn complete(device: &wgpu::Device, queue: &wgpu::Queue, encoder: wgpu::CommandEncoder) {
    let submission = queue.submit([encoder.finish()]);
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .unwrap();
}

fn read_texture(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<u8> {
    let pixel_bytes = match texture.format() {
        wgpu::TextureFormat::Rgba8Unorm => 4,
        wgpu::TextureFormat::Rgba16Uint => 8,
        other => panic!("unexpected test texture {other:?}"),
    };
    let row_bytes = texture.width() * pixel_bytes;
    let padded = row_bytes.div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("output assertion readback"),
        size: padded as u64 * texture.height() as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(texture.height()),
            },
        },
        texture.size(),
    );
    let submission = queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .unwrap();
    rx.recv().unwrap().unwrap();
    let mapped = slice.get_mapped_range().unwrap();
    let bytes = mapped
        .chunks_exact(padded as usize)
        .flat_map(|row| row[..row_bytes as usize].iter().copied())
        .collect();
    drop(mapped);
    buffer.unmap();
    bytes
}

fn rgb_codes(bytes: &[u8]) -> Vec<u16> {
    bytes
        .chunks_exact(8)
        .flat_map(|rgba| {
            assert_eq!(u16::from_le_bytes([rgba[6], rgba[7]]), 65535);
            [
                u16::from_le_bytes([rgba[0], rgba[1]]),
                u16::from_le_bytes([rgba[2], rgba[3]]),
                u16::from_le_bytes([rgba[4], rgba[5]]),
            ]
        })
        .collect()
}

#[test]
fn gpu_matches_independently_compiled_fpw_for_inversion_finish_region_and_texture_ownership() {
    let Some(backend) = backend() else { return };
    let (device, queue) = backend.compute_device();
    let output = PhysicalOutputV1::new(device);
    let views = [
        None,
        Some(DisplayRegion {
            crop: [11, 3, 73, 11],
            width: 73,
            height: 11,
        }),
        Some(DisplayRegion {
            crop: [0, 0, 257, 17],
            width: 31,
            height: 2,
        }),
    ];
    let mut retained = None;
    for stock in [
        None,
        Some(NegativeStockV1::Pro400H),
        Some(NegativeStockV1::XTra400),
    ] {
        let scan = scans(stock_name(stock));
        let buffer = input(device, &scan);
        let reference = stock_name(stock).map_or_else(
            || scan.clone(),
            |stock| original_negative::positive(&scan, stock).unwrap(),
        );
        let rgb: Vec<u16> = reference
            .iter()
            .map(|v| (*v as f64 * 65535. + 0.5) as u16)
            .collect();
        let cpu = image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::from_raw(257, 17, rgb).unwrap();
        for finish in [-125, 0, 125, 0] {
            for region in views {
                let mut encoder = device.create_command_encoder(&Default::default());
                let current = output
                    .encode(
                        device,
                        &mut encoder,
                        FilmOutput {
                            buffer: &buffer,
                            width: 257,
                            height: 17,
                            cctf_encoding: stock.is_none(),
                        },
                        options(stock, finish, region),
                    )
                    .unwrap();
                let original = original_gpu(
                    device,
                    &mut encoder,
                    &buffer,
                    257,
                    17,
                    stock_name(stock),
                    finish,
                    region,
                );
                complete(device, queue, encoder);
                let precision = read_texture(device, queue, &current.precision);
                let display = read_texture(device, queue, &current.display);
                assert_eq!(
                    precision,
                    read_texture(device, queue, &original.precision),
                    "{stock:?} finish {finish} view {region:?}"
                );
                assert_eq!(display, read_texture(device, queue, &original.display));
                let codes = rgb_codes(&precision);
                for (rgb, rgba) in codes.chunks_exact(3).zip(display.chunks_exact(4)) {
                    assert_eq!(
                        rgba,
                        &[
                            ((rgb[0] as u32 + 128) / 257) as u8,
                            ((rgb[1] as u32 + 128) / 257) as u8,
                            ((rgb[2] as u32 + 128) / 257) as u8,
                            255
                        ]
                    );
                }
                if finish == 0 {
                    let reference = region.map_or_else(
                        || cpu.clone(),
                        |v| {
                            image::imageops::thumbnail(
                                &image::imageops::crop_imm(
                                    &cpu, v.crop[0], v.crop[1], v.crop[2], v.crop[3],
                                )
                                .to_image(),
                                v.width,
                                v.height,
                            )
                        },
                    );
                    let max = codes
                        .iter()
                        .zip(reference.as_raw())
                        .map(|(a, b)| a.abs_diff(*b))
                        .max()
                        .unwrap();
                    let allowed = if stock.is_some() {
                        3
                    } else if region.is_some() {
                        1
                    } else {
                        0
                    };
                    assert!(max <= allowed, "{stock:?} {region:?} FPW CPU/GPU maximum code delta {max} exceeds {allowed}");
                }
                if retained.is_none() {
                    retained = Some((current, precision, display));
                }
            }
        }
    }
    let (first, expected16, expected8) = retained.unwrap();
    assert_eq!(read_texture(device, queue, &first.precision), expected16);
    assert_eq!(read_texture(device, queue, &first.display), expected8);
}

#[test]
fn gpu_heldout_tiny_outputs_and_invalid_source_contracts_are_explicit() {
    let Some(backend) = backend() else { return };
    let (device, queue) = backend.compute_device();
    let output = PhysicalOutputV1::new(device);
    for fixture in heldout() {
        let stock = NegativeStockV1::try_from(fixture.stock.as_str()).unwrap();
        let buffer = input(device, &fixture.scan);
        let width = fixture.scan.len() as u32 / 3;
        for region in [
            None,
            Some(DisplayRegion {
                crop: [7, 0, 1, 1],
                width: 1,
                height: 1,
            }),
        ] {
            let mut encoder = device.create_command_encoder(&Default::default());
            let frame = output
                .encode(
                    device,
                    &mut encoder,
                    FilmOutput {
                        buffer: &buffer,
                        width,
                        height: 1,
                        cctf_encoding: false,
                    },
                    options(Some(stock), 0, region),
                )
                .unwrap();
            complete(device, queue, encoder);
            let codes = rgb_codes(&read_texture(device, queue, &frame.precision));
            let expected = match region {
                None => fixture.expected.as_slice(),
                Some(_) => &fixture.expected[21..24],
            };
            let max = codes
                .iter()
                .zip(expected)
                .map(|(a, b)| a.abs_diff((b * 65535. + 0.5) as u16))
                .max()
                .unwrap();
            assert!(max <= 3, "{} heldout GPU code error {max}", fixture.stock);
        }
    }
    let buffer = input(device, &[0.1, 0.2, 0.3]);
    for (mapping, encoded) in [
        (PhysicalOutputMappingV1::EncodedSrgb, false),
        (
            PhysicalOutputMappingV1::NegativeScanAp0(NegativeStockV1::Pro400H),
            true,
        ),
    ] {
        let mut encoder = device.create_command_encoder(&Default::default());
        assert!(matches!(
            output.encode(
                device,
                &mut encoder,
                FilmOutput {
                    buffer: &buffer,
                    width: 1,
                    height: 1,
                    cctf_encoding: encoded,
                },
                PhysicalOutputOptionsV1 {
                    mapping,
                    ..Default::default()
                }
            ),
            Err(CalibratedOutputError::InvalidTransport)
        ));
    }
    for (region, error) in [
        (
            DisplayRegion {
                crop: [0, 0, 1, 1],
                width: 0,
                height: 1,
            },
            OutputError::EmptyDimensions,
        ),
        (
            DisplayRegion {
                crop: [1, 0, 1, 1],
                width: 1,
                height: 1,
            },
            OutputError::CropOutOfBounds,
        ),
        (
            DisplayRegion {
                crop: [0, 0, 1, 1],
                width: 2,
                height: 1,
            },
            OutputError::EnlargementUnsupported,
        ),
    ] {
        let mut encoder = device.create_command_encoder(&Default::default());
        assert!(matches!(output.encode(device, &mut encoder, FilmOutput {
            buffer: &buffer, width: 1, height: 1, cctf_encoding: true,
        }, options(None, 0, Some(region))), Err(CalibratedOutputError::Output(actual)) if actual == error));
    }
    let mut encoder = device.create_command_encoder(&Default::default());
    assert!(matches!(
        output.encode(
            device,
            &mut encoder,
            FilmOutput {
                buffer: &buffer,
                width: 2,
                height: 1,
                cctf_encoding: true,
            },
            options(None, 0, None)
        ),
        Err(CalibratedOutputError::Output(OutputError::InvalidImageData))
    ));
}

fn accepted_pipeline(stock: &str, route: StockRoute) -> Pipeline {
    let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
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
    params.film_render.grain.gpu_multilayer = true;
    params.film_render.dir_couplers.active = true;
    params.camera.diffusion_filter.active = false;
    if route == StockRoute::NegativeScan {
        params.io.scan_film = true;
        params.film_render.glare.active = false;
        params.io.output_color_space = "ACES2065-1".into();
        params.io.output_cctf_encoding = false;
        params.io.output_gamut_compress.algorithm = "off".into();
        params.scanner.unsharp_mask = [0.0, 0.0];
    }
    Pipeline::new_with_spectral_and_stock_setup(
        profile::load_profile_by_name(&data, stock).unwrap(),
        profile::load_profile_by_name(&data, "kodak_portra_endura").unwrap(),
        params,
        &data,
        AcceptedStockSetup::FpwV1(route),
    )
    .unwrap()
}

#[test]
fn complete_render_propagates_failures_before_cache_commit_and_matches_negative_cpu_inversion() {
    let Some(backend) = backend() else { return };
    let (device, queue) = backend.compute_device();
    let output = PhysicalOutputV1::new(device);
    backend.configure_negative_cache(true);
    backend.configure_work_images(true);
    let mut image = ImageBuf::new(17, 11);
    for y in 0..image.height {
        for x in 0..image.width {
            let value = 0.02 + ((x * 7 + y * 19) % 97) as f32 / 120.0;
            image.set(x, y, [value, value * 0.81, value * 0.63].map(from_f32));
        }
    }
    let pipeline = accepted_pipeline("kodak_portra_400", StockRoute::Paper);
    pipeline
        .with_resident_params(&image, |params| {
            let render = FilmRenderOptions {
                source: Some(SourceIdentity(0xCA11_BA7E)),
                ..Default::default()
            };
            let before = backend.negative_cache_stats();
            assert_eq!(before.resident_bytes, 0);
            // This fails inside encode, after a cold density lookup and physical
            // command recording. The wrapper must return outer Err, not Ok(Err).
            let invalid = output.render(
                &backend,
                params,
                render,
                options(
                    None,
                    0,
                    Some(DisplayRegion {
                        crop: [17, 0, 1, 1],
                        width: 1,
                        height: 1,
                    }),
                ),
            );
            assert!(matches!(
                invalid,
                Err(CalibratedOutputError::Output(OutputError::CropOutOfBounds))
            ));
            let failed = backend.negative_cache_stats();
            assert_eq!(failed.misses, before.misses + 1);
            assert_eq!(failed.stores, before.stores);
            assert_eq!(failed.resident_bytes, 0);

            assert!(matches!(
                output.render(
                    &backend,
                    params,
                    render,
                    options(Some(NegativeStockV1::Pro400H), 0, None)
                ),
                Err(CalibratedOutputError::InvalidTransport)
            ));
            assert_eq!(backend.negative_cache_stats().stores, before.stores);
            assert_eq!(backend.negative_cache_stats().resident_bytes, 0);

            let stop = AtomicBool::new(true);
            assert!(matches!(
                output.render(
                    &backend,
                    params,
                    FilmRenderOptions {
                        cancellation: Some(&stop),
                        ..render
                    },
                    options(None, 0, None)
                ),
                Err(CalibratedOutputError::Output(OutputError::Cancelled))
            ));
            assert_eq!(backend.negative_cache_stats().stores, before.stores);
            assert_eq!(backend.negative_cache_stats().resident_bytes, 0);

            let first = output
                .render(&backend, params, render, options(None, 0, None))
                .unwrap();
            let stored = backend.negative_cache_stats();
            assert_eq!(stored.stores, before.stores + 1);
            assert_eq!(stored.resident_bytes, image.pixel_count() as u64 * 12);
            let again = output
                .render(&backend, params, render, options(None, 0, None))
                .unwrap();
            assert_eq!(backend.negative_cache_stats().hits, stored.hits + 1);
            assert_eq!(
                read_texture(device, queue, &first.precision),
                read_texture(device, queue, &again.precision)
            );
        })
        .unwrap();

    backend.configure_negative_cache(false);
    let negative = accepted_pipeline("fujifilm_pro_400h", StockRoute::NegativeScan);
    negative
        .with_resident_params(&image, |params| {
            assert!(params.scan_film);
            assert!(!params.output_cctf_encoding);
            assert!(params.grain.as_ref().unwrap().layers.is_some());
            let scan = backend
                .run_film_chain_region(params, FilmRenderOptions::default(), None)
                .unwrap();
            let scan: Vec<f32> = scan.data.into_iter().map(to_f32).collect();
            let expected = original_negative::positive(&scan, "fujifilm_pro_400h").unwrap();
            let expected: Vec<u16> = expected
                .iter()
                .map(|v| (*v as f64 * 65535. + 0.5) as u16)
                .collect();
            let frame = output
                .render(
                    &backend,
                    params,
                    FilmRenderOptions::default(),
                    options(Some(NegativeStockV1::Pro400H), 0, None),
                )
                .unwrap();
            let actual = rgb_codes(&read_texture(device, queue, &frame.precision));
            assert_eq!(actual.len(), expected.len());
            let max = actual
                .iter()
                .zip(&expected)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap();
            assert!(
                max <= 3,
                "complete physical negative route exceeded original CPU/GPU 3-code envelope: {max}"
            );
            assert!(
                actual.iter().any(|v| *v != actual[0]),
                "negative physical fixture must be nonuniform"
            );
        })
        .unwrap();
}
