//! Wide grain blur only. All image/state arithmetic stays on the GPU;
//! host f64 is used solely to prepare 24 small coefficient words, as in Lab.
use super::*;
use wgpu::util::DeviceExt;

pub(super) type GrainJob = (DispatchJob, (u32, u32));

fn pipeline(backend: &WgpuBackend) -> CachedPipelineRef {
    const SPIRV: &[u8] = include_bytes!("../../spektrafilm-shaders/vulkan/grain_iir.spv");
    assert!(backend.device.features().contains(wgpu::Features::PASSTHROUGH_SHADERS),
        "wide grain IIR requires validated Vulkan SPIR-V; narrow FIR remains available on other backends");
    let mut cache = backend.pipeline_cache.lock().unwrap();
    let key = SPIRV.as_ptr() as usize;
    let item = cache.entry(key).or_insert_with(|| {
        let types = [
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
            wgpu::BufferBindingType::Storage { read_only: true },
        ];
        let entries: Vec<_> = types
            .iter()
            .enumerate()
            .map(|(i, ty)| wgpu::BindGroupLayoutEntry {
                binding: i as u32,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: *ty,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            })
            .collect();
        let layout = backend
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("grain_iir_layout"),
                entries: &entries,
            });
        // SAFETY: checked-in shader is compiled/validated with pinned glslang;
        // four bindings match the layout, complete line ownership prevents
        // races, and every invocation checks its line bounds before indexing.
        let module = unsafe {
            backend
                .device
                .create_shader_module_passthrough(wgpu::ShaderModuleDescriptorPassthrough {
                    label: Some("grain_iir_precise"),
                    // wgpu 30 validates passthrough entry points from this metadata.
                    entry_points: std::borrow::Cow::Borrowed(&[wgpu::PassthroughShaderEntryPoint {
                        name: std::borrow::Cow::Borrowed("main"), workgroup_size: (256,1,1),
                    }]),
                    spirv: Some(wgpu::util::make_spirv_raw(SPIRV)),
                    ..Default::default()
                })
        };
        let pl = backend
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
        let pipeline = backend
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("grain_iir_precise"),
                layout: Some(&pl),
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });
        CachedPipeline {
            bind_group_layout: layout,
            pipeline,
        }
    });
    CachedPipelineRef {
        layout: item.bind_group_layout.clone(),
        pipeline: item.pipeline.clone(),
    }
}

fn coefficients(sigma: f64) -> [f64; 4] {
    let q = if sigma >= 2.5 {
        0.98711 * sigma - 0.96330
    } else {
        3.97156 - 4.14554 * (1.0 - 0.26891 * sigma).sqrt()
    };
    let q2 = q * q;
    let q3 = q2 * q;
    let b0 = 1.57825 + 2.44413 * q + 1.4281 * q2 + 0.422205 * q3;
    let b1 = 2.44413 * q + 2.85619 * q2 + 1.26661 * q3;
    let b2 = -(1.4281 * q2 + 1.26661 * q3);
    let b3 = 0.422205 * q3;
    [1.0 - (b1 + b2 + b3) / b0, b1 / b0, b2 / b0, b3 / b0]
}

pub(super) fn append_pass(
    jobs: &mut Vec<GrainJob>,
    backend: &WgpuBackend,
    width: u32,
    height: u32,
    sigmas: [f32; 3],
    axis: u32,
    src: &wgpu::Buffer,
    dst: &wgpu::Buffer,
) {
    let device = &backend.device;
    let shader = pipeline(backend);
    let meta = [
        width,
        height,
        axis,
        0,
        (sigmas[0] >= 3.0) as u32,
        (sigmas[1] >= 3.0) as u32,
        (sigmas[2] >= 3.0) as u32,
        0,
    ];
    let mut pairs = Vec::with_capacity(24);
    for sigma in sigmas {
        for value in if sigma >= 3.0 {
            coefficients(sigma as f64)
        } else {
            [1.0, 0.0, 0.0, 0.0]
        } {
            let hi = value as f32;
            pairs.extend([hi, (value - hi as f64) as f32]);
        }
    }
    let pb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("grain_iir_params"),
        contents: bytemuck::cast_slice(&meta),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let cb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("grain_iir_coefficients"),
        contents: bytemuck::cast_slice(&pairs),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("grain_iir"),
        layout: &shader.layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: pb.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: src.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: dst.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: cb.as_entire_binding(),
            },
        ],
    });
    let total = ((if axis == 0 { width } else { height }) * 3).div_ceil(256);
    let x = total.min(65535);
    jobs.push((
        DispatchJob {
            _params_buf: pb,
            pipeline: shader,
            bg,
        },
        (x, total.div_ceil(x)),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    // Independent f64 recurrence using the *rounded* f32 forward plane,
    // following pinned Lab's gpu_vulkan_smoke.cpp numerical contract.
    fn reference(input: &[f32], w: usize, h: usize, sigma: f64, mask: u32, axis: u32) -> Vec<f32> {
        let q = 0.98711 * sigma - 0.96330;
        let b0 = 1.57825 + 2.44413 * q + 1.4281 * q * q + 0.422205 * q * q * q;
        let b = [
            2.44413 * q + 2.85619 * q * q + 1.26661 * q * q * q,
            -(1.4281 * q * q + 1.26661 * q * q * q),
            0.422205 * q * q * q,
        ];
        let c = [
            1.0 - (b[0] + b[1] + b[2]) / b0,
            b[0] / b0,
            b[1] / b0,
            b[2] / b0,
        ];
        let mut out = input.to_vec();
        for line in 0..if axis == 0 { w } else { h } {
            for channel in 0..3 {
                if mask & (1 << channel) == 0 {
                    continue;
                }
                let base = if axis == 0 {
                    line * 3 + channel
                } else {
                    line * w * 3 + channel
                };
                let stride = if axis == 0 { w * 3 } else { 3 };
                let length = if axis == 0 { h } else { w };
                let mut history = [input[base] as f64; 3];
                for i in 0..length {
                    let v = c[0] * input[base + i * stride] as f64
                        + c[1] * history[0]
                        + c[2] * history[1]
                        + c[3] * history[2];
                    out[base + i * stride] = v as f32;
                    history = [v, history[0], history[1]];
                }
                history = [out[base + (length - 1) * stride] as f64; 3];
                for i in (0..length).rev() {
                    let v = c[0] * out[base + i * stride] as f64
                        + c[1] * history[0]
                        + c[2] * history[1]
                        + c[3] * history[2];
                    out[base + i * stride] = v as f32;
                    history = [v, history[0], history[1]];
                }
            }
        }
        out
    }

    fn render(
        backend: &WgpuBackend,
        input: &[f32],
        w: u32,
        h: u32,
        sigmas: [f32; 3],
        axes: &[u32],
    ) -> Vec<f32> {
        let size = (input.len() * 4) as u64;
        let a = backend
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(input),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            });
        let b = backend.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let output = backend.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut jobs = Vec::new();
        for (i, axis) in axes.iter().enumerate() {
            let (src, dst) = if i % 2 == 0 { (&a, &b) } else { (&b, &a) };
            append_pass(&mut jobs, backend, w, h, sigmas, *axis, src, dst);
        }
        let mut encoder = backend.device.create_command_encoder(&Default::default());
        for (job, groups) in jobs {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&job.pipeline.pipeline);
            pass.set_bind_group(0, &job.bg, &[]);
            pass.dispatch_workgroups(groups.0, groups.1, 1);
        }
        encoder.copy_buffer_to_buffer(
            if axes.len() % 2 == 0 { &a } else { &b },
            0,
            &output,
            0,
            size,
        );
        backend.queue.submit(Some(encoder.finish()));
        let slice = output.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
        backend.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).expect("GPU work completion");
        rx.recv().unwrap().unwrap();
        bytemuck::cast_slice(&slice.get_mapped_range().expect("completed buffer mapping")).to_vec()
    }

    /// Full 1,215-case precision check should run explicitly on Vulkan hardware.
    #[test]
    #[ignore = "explicit Vulkan hardware qualification: 1215 precision cases"]
    fn actual_gpu_iir_matches_f64_reference() {
        let backend = WgpuBackend::new().expect("real GPU required; never skip");
        let mut count = 0;
        let mut max_error = 0.0f64;
        for (w, h) in [
            (1, 1),
            (1, 2),
            (2, 1),
            (2, 3),
            (5, 7),
            (257, 3),
            (3, 257),
            (6048, 3),
            (3, 6048),
        ] {
            for sigma in [3.0, 12.0, 130.0] {
                for mask in [0u32, 1, 2, 4, 7] {
                    for pattern in 0..3 {
                        let input: Vec<f32> = (0..w * h * 3)
                            .map(|i| {
                                let c = i % 3;
                                if pattern == 0 {
                                    [-0.25, 0.5, 1.5][c]
                                } else if pattern == 1 {
                                    ((i as f64 * 0.17).sin() * 0.2 + 0.4 + c as f64 * 0.3) as f32
                                } else {
                                    if i / 3 == w * h / 2 {
                                        1.0 + c as f32
                                    } else {
                                        0.0
                                    }
                                }
                            })
                            .collect();
                        for axes in [&[0u32][..], &[1u32][..], &[1u32, 0][..]] {
                            let sigmas = std::array::from_fn(|c| {
                                if mask & (1 << c) != 0 {
                                    sigma as f32
                                } else {
                                    0.0
                                }
                            });
                            let actual = render(&backend, &input, w as u32, h as u32, sigmas, axes);
                            let mut expected = input.clone();
                            for axis in axes {
                                expected = reference(&expected, w, h, sigma, mask, *axis);
                            }
                            for (i, (a, e)) in actual.iter().zip(&expected).enumerate() {
                                let error = (*a as f64 - *e as f64).abs();
                                max_error = max_error.max(error);
                                let tolerance =
                                    4.0 * f32::EPSILON as f64 * (*e as f64).abs().max(1.0);
                                assert!(a.is_finite() && error<=tolerance,
                                    "{w}x{h}, sigma={sigma}, mask={mask}, pattern={pattern}, axes={axes:?}, i={i}: GPU={a}, f64={e}, error={error}, tolerance={tolerance}");
                                if mask & (1 << (i % 3)) == 0 {
                                    assert_eq!(*a, input[i], "inactive channel must be exact");
                                }
                            }
                            count += 1;
                        }
                    }
                }
            }
        }
        println!("real GPU IIR: {count} cases passed, max absolute error {max_error}; Lab tolerance 4*f32::EPSILON*max(1,abs(reference))");
    }
}
