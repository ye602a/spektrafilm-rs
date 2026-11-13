//! Full physical-chain output and same-device ownership regressions.
//! Software Vulkan is mandatory when SPEKTRAFILM_REQUIRE_GPU=1.
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use spektrafilm_core::{params::RuntimeParams, pipeline::Pipeline, profile};
use spektrafilm_gpu::{
    output::{DisplayRegion, OutputError},
    wgpu,
    wgpu_backend::WgpuBackend,
    FilmRenderOptions, SourceIdentity,
};
use spektrafilm_math::{
    image::ImageBuf,
    precision::{from_f32, to_f32},
};

fn backend() -> Option<WgpuBackend> {
    let gpu = WgpuBackend::new();
    if gpu.is_none() {
        assert!(
            std::env::var_os("SPEKTRAFILM_REQUIRE_GPU").is_none(),
            "required full-chain GPU output regression cannot be skipped"
        );
        eprintln!("SKIP: no Vulkan device for full-chain output regression");
    }
    gpu
}

fn pipeline(encoded: bool) -> Pipeline {
    let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let film = profile::load_profile_by_name(&data, "kodak_portra_400").unwrap();
    let paper = profile::load_profile_by_name(&data, "kodak_portra_endura").unwrap();
    let mut params = RuntimeParams::default();
    params.io.input_color_space = "sRGB".into();
    params.io.output_cctf_encoding = encoded;
    params.camera.auto_exposure = false;
    params.film_render.grain.gpu_multilayer = true;
    Pipeline::new_with_spectral(film, paper, params, &data).unwrap()
}

fn fixture(width: u32, height: u32, other: bool) -> ImageBuf {
    let mut image = ImageBuf::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let value = 0.09 + ((x * 19 + y * 7) % 100) as f32 / 140.0;
            let value = if other { 0.95 - value } else { value };
            image.set(
                x,
                y,
                [
                    from_f32(value),
                    from_f32(value * 0.85),
                    from_f32(value * 0.65),
                ],
            );
        }
    }
    image
}

fn options(source: u128) -> FilmRenderOptions<'static> {
    FilmRenderOptions {
        source: Some(SourceIdentity(source)),
        cancellation: None,
        ..Default::default()
    }
}

fn f32_values(image: &ImageBuf) -> Vec<f32> {
    image.data.iter().copied().map(to_f32).collect()
}

fn codes(image: &ImageBuf) -> Vec<u16> {
    f32_values(image)
        .into_iter()
        .map(|value| ((value as f64).clamp(0.0, 1.0) * 65535.0).round() as u16)
        .collect()
}

fn assert_pixels_equal(left: &ImageBuf, right: &ImageBuf) {
    assert_eq!((left.width, left.height), (right.width, right.height));
    assert_eq!(
        f32_values(left)
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>(),
        f32_values(right)
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>()
    );
}

fn crop_reference(full: &ImageBuf, view: DisplayRegion) -> ImageBuf {
    assert_eq!((view.width, view.height), (view.crop[2], view.crop[3]));
    let mut out = ImageBuf::new(view.width, view.height);
    for y in 0..view.height {
        for x in 0..view.width {
            out.set(x, y, full.get(x + view.crop[0], y + view.crop[1]));
        }
    }
    out
}

fn reduction_reference(full: &ImageBuf, view: DisplayRegion) -> ImageBuf {
    let mut out = ImageBuf::new(view.width, view.height);
    for oy in 0..view.height {
        for ox in 0..view.width {
            let rx = view.crop[2] as f32 / view.width as f32;
            let ry = view.crop[3] as f32 / view.height as f32;
            let x0 = ((ox as f32 * rx).ceil() as u32).min(view.crop[2] - 1);
            let y0 = ((oy as f32 * ry).ceil() as u32).min(view.crop[3] - 1);
            let x1 = ((ox as f32 * rx + rx).ceil() as u32).min(view.crop[2]);
            let y1 = ((oy as f32 * ry + ry).ceil() as u32).min(view.crop[3]);
            let mut pixel = [from_f32(0.); 3];
            for c in 0..3 {
                let mut total = 0.0_f32;
                for y in y0..y1 {
                    let mut row = 0_u32;
                    for x in x0..x1 {
                        let value = to_f32(full.get(x + view.crop[0], y + view.crop[1])[c]);
                        row += (value.clamp(0., 1.) * 65535. + 0.5).floor() as u32;
                    }
                    total += row as f32;
                }
                pixel[c] =
                    from_f32((total / ((x1 - x0) * (y1 - y0)) as f32 + 0.5).floor() / 65535.);
            }
            out.set(ox, oy, pixel);
        }
    }
    out
}

#[test]
fn full_float_rgb16_and_regions_preserve_multilayer_pixels_with_and_without_caches() {
    let Some(backend) = backend() else { return };
    let input = fixture(17, 11, false); // 561 RGB samples: packed output has an odd tail.
    let crop = DisplayRegion {
        crop: [3, 2, 9, 7],
        width: 9,
        height: 7,
    };
    let reduced = DisplayRegion {
        width: 5,
        height: 3,
        ..crop
    };
    for encoded in [false, true] {
        let pipeline = pipeline(encoded);
        backend.configure_work_images(false);
        backend.configure_negative_cache(false);
        let ordinary = pipeline.process(input.clone(), &backend);
        assert!(f32_values(&ordinary).iter().all(|v| v.is_finite()));
        assert!(
            ordinary.data.windows(2).any(|p| p[0] != p[1]),
            "physical fixture must not be uniform"
        );

        for reuse in [false, true] {
            backend.configure_work_images(reuse);
            backend.configure_negative_cache(reuse);
            let before = backend.negative_cache_stats();
            pipeline
                .with_resident_params(&input, |params| {
                    assert!(
                        params.grain.as_ref().unwrap().layers.is_some(),
                        "this regression must exercise the migrated multilayer model"
                    );
                    let full = backend
                        .run_film_chain_region(params, options(1), None)
                        .unwrap();
                    assert_pixels_equal(&ordinary, &full);
                    let rgb16 = backend
                        .run_film_chain_rgb16(params, options(1), None)
                        .unwrap();
                    assert_eq!((rgb16.width, rgb16.height), (17, 11));
                    assert_eq!(
                        rgb16.data.len(),
                        17 * 11 * 3,
                        "packing padding must not escape"
                    );
                    assert_eq!(rgb16.data, codes(&full));

                    let cropped = backend
                        .run_film_chain_region(params, options(1), Some(crop))
                        .unwrap();
                    assert_pixels_equal(&cropped, &crop_reference(&full, crop));
                    let crop_codes = backend
                        .run_film_chain_rgb16(params, options(1), Some(crop))
                        .unwrap();
                    assert_eq!(crop_codes.data, codes(&cropped));
                    assert_eq!((crop_codes.width, crop_codes.height), (9, 7));

                    let small = backend
                        .run_film_chain_region(params, options(1), Some(reduced))
                        .unwrap();
                    assert_pixels_equal(&small, &reduction_reference(&full, reduced));
                    let small_codes = backend
                        .run_film_chain_rgb16(params, options(1), Some(reduced))
                        .unwrap();
                    assert_eq!((small_codes.width, small_codes.height), (5, 3));
                    assert_eq!(small_codes.data, codes(&small));

                    let again = backend
                        .run_film_chain_region(params, options(1), None)
                        .unwrap();
                    assert_pixels_equal(&full, &again);
                })
                .expect("physical resident route required");
            let after = backend.negative_cache_stats();
            if reuse {
                assert!(
                    after.hits >= before.hits + 6,
                    "changing only output encoding/storage/region must reuse developed density"
                );
                assert_eq!(after.resident_bytes, input.pixel_count() as u64 * 12);
                assert_eq!(backend.work_image_bytes(), input.pixel_count() as u64 * 24);
            } else {
                assert_eq!(after.resident_bytes, 0);
                assert_eq!(backend.work_image_bytes(), 0);
            }
        }
    }
}

#[test]
fn tiny_rgb16_and_invalid_regions_have_explicit_results_and_cancelled_callbacks_do_not_run() {
    let Some(backend) = backend() else { return };
    let pipeline = pipeline(true);
    let input = fixture(1, 1, false);
    pipeline
        .with_resident_params(&input, |params| {
            let full = backend
                .run_film_chain_region(params, options(2), None)
                .unwrap();
            let rgb16 = backend
                .run_film_chain_rgb16(params, options(2), None)
                .unwrap();
            assert_eq!((rgb16.width, rgb16.height, rgb16.data.len()), (1, 1, 3));
            assert_eq!(rgb16.data, codes(&full));
            let identity = DisplayRegion {
                crop: [0, 0, 1, 1],
                width: 1,
                height: 1,
            };
            let cropped = backend
                .run_film_chain_region(params, options(2), Some(identity))
                .unwrap();
            assert_pixels_equal(&full, &cropped);
            for (view, expected) in [
                (
                    DisplayRegion {
                        width: 0,
                        ..identity
                    },
                    OutputError::EmptyDimensions,
                ),
                (
                    DisplayRegion {
                        crop: [1, 0, 1, 1],
                        ..identity
                    },
                    OutputError::CropOutOfBounds,
                ),
                (
                    DisplayRegion {
                        crop: [u32::MAX, 0, 1, 1],
                        ..identity
                    },
                    OutputError::CropOutOfBounds,
                ),
                (
                    DisplayRegion {
                        width: 2,
                        ..identity
                    },
                    OutputError::EnlargementUnsupported,
                ),
            ] {
                assert_eq!(
                    backend
                        .run_film_chain_rgb16(params, options(2), Some(view))
                        .unwrap_err(),
                    expected
                );
                let called = std::cell::Cell::new(false);
                let result = backend.with_film_output(params, options(2), Some(view), |_, _, _| {
                    called.set(true);
                    Ok(())
                });
                assert_eq!(result, Err(expected));
                assert!(
                    !called.get(),
                    "invalid request must not expose an output view"
                );
            }
            let stop = AtomicBool::new(true);
            let called = std::cell::Cell::new(false);
            let cancelled = FilmRenderOptions {
                source: Some(SourceIdentity(2)),
                cancellation: Some(&stop),
                ..Default::default()
            };
            assert_eq!(
                backend.with_film_output(params, cancelled, None, |_, _, _| {
                    called.set(true);
                    Ok(())
                }),
                Err(OutputError::Cancelled)
            );
            assert!(!called.get());
            stop.store(false, Ordering::Release);
            let recovered = backend
                .run_film_chain_region(params, cancelled, None)
                .unwrap();
            assert_pixels_equal(&full, &recovered);
        })
        .unwrap();
}

const RGB_TO_HOST_TEXTURE: &str = r#"
@group(0) @binding(0) var<storage, read> source: array<f32>;
@group(0) @binding(1) var output: texture_storage_2d<rgba32float, write>;
@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let dimensions = textureDimensions(output);
    if any(id.xy >= dimensions) { return; }
    let first = (id.y * dimensions.x + id.x) * 3u;
    textureStore(output, vec2<i32>(id.xy), vec4<f32>(source[first], source[first+1u], source[first+2u], 1.0));
}
"#;

fn read_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) -> Vec<[f32; 4]> {
    let width = texture.width();
    let height = texture.height();
    let bytes_per_row = (width * 16).div_ceil(256) * 256;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("host-owned-texture-readback"),
        size: bytes_per_row as u64 * height as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(height),
            },
        },
        texture.size(),
    );
    let submission = queue.submit([encoder.finish()]);
    let slice = readback.slice(..);
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
    let mut output = Vec::new();
    for row in mapped.chunks_exact(bytes_per_row as usize) {
        for rgba in row[..width as usize * 16].chunks_exact(16) {
            let mut pixel = [0.; 4];
            for c in 0..4 {
                pixel[c] = f32::from_le_bytes(rgba[c * 4..c * 4 + 4].try_into().unwrap());
            }
            output.push(pixel);
        }
    }
    drop(mapped);
    readback.unmap();
    output
}

#[test]
fn scoped_same_device_texture_survives_the_next_render_and_work_buffer_reuse() {
    let Some(backend) = backend() else { return };
    let pipeline = pipeline(true);
    let input = fixture(17, 11, false);
    let other = fixture(17, 11, true);
    let view = DisplayRegion {
        crop: [2, 1, 9, 7],
        width: 9,
        height: 7,
    };
    backend.configure_work_images(true);
    backend.configure_negative_cache(true);
    let expected = pipeline
        .with_resident_params(&input, |params| {
            backend
                .run_film_chain_region(params, options(3), Some(view))
                .unwrap()
        })
        .unwrap();
    let texture = pipeline
        .with_resident_params(&input, |params| {
            backend
                .with_film_output(params, options(3), Some(view), |device, encoder, output| {
                    assert_eq!((output.width, output.height), (view.width, view.height));
                    assert!(output.cctf_encoding);
                    assert_eq!(
                        backend.work_image_bytes(),
                        0,
                        "consumer must hold an exclusive work-plane lease"
                    );
                    let texture = device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("host-owned-film-texture"),
                        size: wgpu::Extent3d {
                            width: output.width,
                            height: output.height,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba32Float,
                        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
                        view_formats: &[],
                    });
                    let texture_view = texture.create_view(&Default::default());
                    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                        label: Some("host-film-consumer"),
                        source: wgpu::ShaderSource::Wgsl(RGB_TO_HOST_TEXTURE.into()),
                    });
                    let consumer =
                        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                            label: Some("host-film-consumer"),
                            layout: None,
                            module: &module,
                            entry_point: Some("main"),
                            compilation_options: Default::default(),
                            cache: None,
                        });
                    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("host-film-consumer"),
                        layout: &consumer.get_bind_group_layout(0),
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: output.buffer.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(&texture_view),
                            },
                        ],
                    });
                    {
                        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                            label: Some("host-film-to-texture"),
                            timestamp_writes: None,
                        });
                        pass.set_pipeline(&consumer);
                        pass.set_bind_group(0, &group, &[]);
                        pass.dispatch_workgroups(
                            output.width.div_ceil(8),
                            output.height.div_ceil(8),
                            1,
                        );
                    }
                    Ok(texture)
                })
                .unwrap()
        })
        .unwrap();
    let (device, queue) = backend.compute_device();
    let captured = read_texture(device, queue, &texture);
    let expected_rgba: Vec<[f32; 4]> = f32_values(&expected)
        .chunks_exact(3)
        .map(|rgb| [rgb[0], rgb[1], rgb[2], 1.])
        .collect();
    assert_eq!(captured, expected_rgba);
    assert_eq!(backend.work_image_bytes(), input.pixel_count() as u64 * 24);
    let later = pipeline
        .with_resident_params(&other, |params| {
            backend
                .run_film_chain_region(params, options(4), Some(view))
                .unwrap()
        })
        .unwrap();
    assert_ne!(
        f32_values(&later),
        f32_values(&expected),
        "second source must have different output"
    );
    assert_eq!(
        read_texture(device, queue, &texture),
        captured,
        "host-owned texture must not alias work planes reused by the next render"
    );
}

#[test]
fn consumer_error_or_pre_submission_cancellation_does_not_publish_density_and_recovers() {
    let Some(backend) = backend() else { return };
    let pipeline = pipeline(true);
    let input = fixture(17, 11, false);
    let oracle = pipeline.process(input.clone(), &backend);
    backend.configure_work_images(true);
    backend.configure_negative_cache(true);
    let (device, _) = backend.compute_device();
    // A host destination initialized before encoding. Discarded consumer
    // commands must not clear it after callback error or cancellation.
    let marker = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("consumer-abort-marker"),
        size: 16,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: true,
    });
    marker
        .slice(..)
        .get_mapped_range_mut()
        .unwrap()
        .copy_from_slice(&[0x55; 16]);
    marker.unmap();
    let read_marker = || {
        let slice = marker.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .unwrap();
        rx.recv().unwrap().unwrap();
        let mapped = slice.get_mapped_range().unwrap();
        let data = mapped.to_vec();
        drop(mapped);
        marker.unmap();
        data
    };
    pipeline
        .with_resident_params(&input, |params| {
            let before = backend.negative_cache_stats();
            let rejected = backend.with_film_output(params, options(5), None, |_, encoder, _| {
                encoder.clear_buffer(&marker, 0, None);
                Err::<(), _>(OutputError::GpuFailure)
            });
            assert_eq!(rejected, Err(OutputError::GpuFailure));
            assert_eq!(read_marker(), [0x55; 16]);
            assert_eq!(backend.negative_cache_stats().stores, before.stores);
            assert_eq!(backend.negative_cache_stats().resident_bytes, 0);
            assert_eq!(backend.work_image_bytes(), 0);

            let stop = AtomicBool::new(false);
            let invocation = FilmRenderOptions {
                source: Some(SourceIdentity(5)),
                cancellation: Some(&stop),
                ..Default::default()
            };
            let cancelled = backend.with_film_output(params, invocation, None, |_, encoder, _| {
                encoder.clear_buffer(&marker, 0, None);
                stop.store(true, Ordering::Release);
                Ok(())
            });
            assert_eq!(cancelled, Err(OutputError::Cancelled));
            assert_eq!(read_marker(), [0x55; 16]);
            assert_eq!(backend.negative_cache_stats().stores, before.stores);
            assert_eq!(backend.negative_cache_stats().resident_bytes, 0);
            assert_eq!(backend.work_image_bytes(), 0);

            // This proves the pre-submission abort boundary only. The separate
            // completion-callback regression below covers submitted work.
            stop.store(false, Ordering::Release);
            let recovered = backend
                .run_film_chain_region(params, invocation, None)
                .unwrap();
            assert_pixels_equal(&oracle, &recovered);
            assert_eq!(backend.negative_cache_stats().stores, before.stores + 1);
            let hits = backend.negative_cache_stats().hits;
            let repeated = backend
                .run_film_chain_region(params, invocation, None)
                .unwrap();
            assert_pixels_equal(&recovered, &repeated);
            assert_eq!(backend.negative_cache_stats().hits, hits + 1);
            assert_eq!(backend.work_image_bytes(), input.pixel_count() as u64 * 24);
        })
        .unwrap();
}

#[test]
fn cancellation_after_gpu_completion_discards_publication_and_recovers() {
    let Some(backend) = backend() else { return };
    let pipeline = pipeline(true);
    let input = fixture(17, 11, false);
    let oracle = pipeline.process(input.clone(), &backend);
    backend.configure_work_images(true);
    backend.configure_negative_cache(true);
    let before = backend.negative_cache_stats();
    let (device, _) = backend.compute_device();
    let marker = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("submitted-cancellation-marker"),
        size: 16,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: true,
    });
    marker
        .slice(..)
        .get_mapped_range_mut()
        .unwrap()
        .copy_from_slice(&[0x55; 16]);
    marker.unmap();

    let stop = std::sync::Arc::new(AtomicBool::new(false));
    let invocation = FilmRenderOptions {
        source: Some(SourceIdentity(6)),
        cancellation: Some(stop.as_ref()),
        ..Default::default()
    };
    pipeline
        .with_resident_params(&input, |params| {
            let completed_stop = stop.clone();
            let cancelled = backend.with_film_output(params, invocation, None, |_, encoder, _| {
                encoder.clear_buffer(&marker, 0, None);
                // This callback is attached to this command buffer. It cannot
                // request cancellation until the submitted GPU writes finish;
                // it does not depend on sleeps or a racing host thread.
                encoder.on_submitted_work_done(move || {
                    completed_stop.store(true, Ordering::Release);
                });
                assert!(!stop.load(Ordering::Acquire));
                Ok(())
            });
            assert_eq!(cancelled, Err(OutputError::Cancelled));
            assert!(stop.load(Ordering::Acquire), "completion callback must run");

            let slice = marker.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
            let timeout = std::time::Duration::from_secs(30);
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: Some(timeout),
                })
                .expect("submitted cancellation marker completion");
            rx.recv_timeout(timeout)
                .expect("submitted cancellation marker map callback")
                .expect("submitted cancellation marker mapping");
            let mapped = slice.get_mapped_range().unwrap();
            assert_eq!(
                &*mapped, &[0; 16],
                "submitted writes must have executed; cancellation cannot undo them"
            );
            drop(mapped);
            marker.unmap();

            let cancelled_stats = backend.negative_cache_stats();
            assert_eq!(cancelled_stats.misses, before.misses + 1);
            assert_eq!(cancelled_stats.stores, before.stores);
            assert_eq!(cancelled_stats.resident_bytes, 0);
            assert_eq!(backend.work_image_bytes(), 0);

            stop.store(false, Ordering::Release);
            let recovered = backend
                .run_film_chain_with_options(params, invocation)
                .unwrap();
            assert_pixels_equal(&oracle, &recovered);
            assert_eq!(backend.negative_cache_stats().stores, before.stores + 1);
            let hits = backend.negative_cache_stats().hits;
            let repeated = backend
                .run_film_chain_with_options(params, invocation)
                .unwrap();
            assert_pixels_equal(&recovered, &repeated);
            assert_eq!(backend.negative_cache_stats().hits, hits + 1);
            assert_eq!(backend.work_image_bytes(), input.pixel_count() as u64 * 24);
        })
        .unwrap();
}
