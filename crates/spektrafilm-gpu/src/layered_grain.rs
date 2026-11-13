//! Resident multilayer grain. Optional dye clouds and microstructure stay on
//! GPU. Only small curves/parameters cross the CPU boundary, never image data.
use super::grain_iir::{self, GrainJob};
use super::*;
use wgpu::util::DeviceExt;

/// The effective blur radii used both for capability checks and dispatch.
/// Sampling scales radii only after the original cloud/micro activation gates.
struct GrainBlurPlan {
    dye: Option<[[f32; 3]; 3]>,
    micro: Option<[f32; 3]>,
    grain: [f32; 3],
}

impl GrainBlurPlan {
    fn new(gp: &crate::GrainGpuParams) -> Option<Self> {
        let g = gp.layers.as_ref()?;
        let scale = g.output_sampling.map_or(1.0, |v| v[0]);
        Some(Self {
            dye: g.dye_sigma.iter().flatten().any(|s| *s > 0.4).then(|| {
                g.dye_sigma.map(|row| row.map(|s| if s > 0.4 { s * scale } else { 0.0 }))
            }),
            micro: (g.micro_sigma > 0.05 && g.micro_blur > 0.4)
                .then_some([g.micro_blur * scale; 3]),
            grain: [gp.grain_blur * scale; 3],
        })
    }

    fn requires_iir(&self) -> bool {
        self.dye.as_ref().is_some_and(|rows| rows.iter().any(uses_iir))
            || self.micro.as_ref().is_some_and(uses_iir)
            || uses_iir(&self.grain)
    }
}

fn uses_iir(sigmas: &[f32; 3]) -> bool {
    sigmas.iter().any(|s| *s >= 3.0)
}

pub(super) fn requires_iir(gp: &crate::GrainGpuParams) -> bool {
    GrainBlurPlan::new(gp).is_some_and(|plan| plan.requires_iir())
}

pub(super) struct LayeredGrainState {
    jobs: Vec<GrainJob>,
    accumulator: Option<wgpu::Buffer>,
    _baseline: Option<wgpu::Buffer>,
}

impl LayeredGrainState {
    pub(super) fn new(
        device: &wgpu::Device,
        gp: &crate::GrainGpuParams,
        width: u32,
        height: u32,
        a: &wgpu::Buffer,
        b: &wgpu::Buffer,
        backend: &WgpuBackend,
    ) -> Self {
        let g = gp.layers.as_ref().expect("layered grain parameters");
        let blur_plan = GrainBlurPlan::new(gp).expect("layered grain parameters");
        let shader = backend.cached_pipeline(
            concat!(include_str!("../../spektrafilm-shaders/wgsl/grain_reference.wgsl"), include_str!("../../spektrafilm-shaders/wgsl/grain_layers.wgsl")),
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: false },
                wgpu::BufferBindingType::Storage { read_only: false },
                wgpu::BufferBindingType::Storage { read_only: true },
            ],
        );
        let n = width * height;
        let total = n.div_ceil(256);
        let x = total.min(65535);
        let groups = (x, total.div_ceil(x));
        let mut jobs = Vec::new();
        let mut accumulator = None;
        let add = |jobs: &mut Vec<GrainJob>,
                   op: u32,
                   layer: u32,
                   values: [f32; 4],
                   data: &[f32],
                   src: &wgpu::Buffer,
                   dst: &wgpu::Buffer| {
            #[repr(C)]
            #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
            struct P {
                shape: [u32; 4],
                control: [u32; 4],
                values: [f32; 4],
                sampling: [f32; 4],
                reference: GrainReference,
            }
            let params = P {
                reference: backend.grain_reference.unwrap_or_else(bytemuck::Zeroable::zeroed),
                shape: [n, width, height, gp.base_seed],
                control: [g.curve_len, g.positive as u32, op, layer],
                values,
                sampling: g.output_sampling.map_or([0.;4], |v| [v[0],v[1],1.,0.]),
            };
            let pb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("layered_grain_params"),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let db = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("layered_grain_constants"),
                contents: bytemuck::cast_slice(data),
                usage: wgpu::BufferUsages::STORAGE,
            });
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("layered_grain"),
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
                        resource: db.as_entire_binding(),
                    },
                ],
            });
            jobs.push((
                DispatchJob {
                    _params_buf: pb,
                    pipeline: shader.clone(),
                    bg,
                },
                groups,
            ));
        };
        let alloc = || {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("layered_grain_scratch"),
                size: n as u64 * 12,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let blur =
            |jobs: &mut Vec<GrainJob>, sigmas: [f32; 3], src: &wgpu::Buffer, mid: &wgpu::Buffer| {
                if sigmas.iter().all(|s| *s <= 0.0) {
                    return;
                }
                // Lab runs FIR only for narrow channels, then IIR only for wide
                // channels. Inactive channels pass through both separable stages.
                if sigmas.iter().any(|s| *s > 0.0 && *s < 3.0) {
                    let mut data = vec![0.0; 3];
                    for c in 0..3 {
                        // Lab uses scipy-compatible round(3*sigma) and reflect edges.
                        let s = if sigmas[c] < 3.0 {
                            sigmas[c] as f64
                        } else {
                            0.0
                        };
                        let radius = if s > 0.0 {
                            (3.0 * s + 0.5).floor() as i32
                        } else {
                            0
                        };
                        data[c] = radius as f32;
                        let kernel: Vec<f64> = (-radius..=radius)
                            .map(|x| {
                                if radius == 0 {
                                    1.0
                                } else {
                                    (-(x * x) as f64 / (2.0 * s * s)).exp()
                                }
                            })
                            .collect();
                        let sum: f64 = kernel.iter().sum();
                        data.extend(kernel.iter().map(|x| (x / sum) as f32));
                    }
                    add(jobs, 3, 0, [0.0; 4], &data, src, mid);
                    add(jobs, 2, 0, [0.0; 4], &data, mid, src);
                }
                if uses_iir(&sigmas) {
                    grain_iir::append_pass(jobs, backend, width, height, sigmas, 1, src, mid);
                    grain_iir::append_pass(jobs, backend, width, height, sigmas, 0, mid, src);
                }
            };
        // Copy exact post-DIR density before generating one grain realization.
        let baseline = g.output_sampling.map(|_| {
            let buffer = alloc();
            add(&mut jobs, 7, 0, [0.;4], &[0.], a, &buffer);
            buffer
        });
        let cloud = blur_plan.dye.is_some();
        let micro = g.micro_sigma > 0.05;
        let tmp = if cloud || blur_plan.micro.is_some() {
            Some(alloc())
        } else {
            None
        };
        let values = [0.0, 0.0, 0.0, gp.monochrome as u32 as f32];
        if let Some(dye_sigmas) = blur_plan.dye {
            let acc = alloc();
            for sl in 0..3 {
                add(&mut jobs, 1, sl as u32, values, &g.packed, a, b);
                blur(
                    &mut jobs,
                    dye_sigmas[sl],
                    b,
                    tmp.as_ref().unwrap(),
                );
                add(&mut jobs, 5, 0, [0.0; 4], &[0.0], &acc, b);
            }
            add(&mut jobs, 7, 0, [0.0; 4], &[0.0], &acc, a);
            accumulator = Some(acc);
        } else {
            add(&mut jobs, 0, 0, values, &g.packed, a, b);
        }
        if micro {
            let sigma = ((1.0 + (g.micro_sigma as f64).powi(2)).ln()).sqrt();
            add(
                &mut jobs,
                4,
                0,
                [-0.5 * (sigma * sigma) as f32, sigma as f32, 0.0, 0.0],
                &[0.0],
                a,
                b,
            );
            if let Some(sigmas) = blur_plan.micro {
                blur(&mut jobs, sigmas, b, tmp.as_ref().unwrap());
            }
        }
        add(
            &mut jobs,
            6,
            0,
            [
                gp.density_min[0],
                gp.density_min[1],
                gp.density_min[2],
                micro as u32 as f32,
            ],
            &[0.0],
            a,
            b,
        );
        blur(&mut jobs, blur_plan.grain, a, b);
        if let Some(original) = &baseline { add(&mut jobs, 8, 0, [0.;4], &[0.], original, a); }
        Self { jobs, accumulator, _baseline: baseline }
    }
    pub(super) fn encode(&self, encoder: &mut wgpu::CommandEncoder) {
        // Cached state is reused across photos; creation-time zeroing alone
        // would retain the previous image's dye-cloud sum on the next render.
        if let Some(accumulator) = &self.accumulator {
            encoder.clear_buffer(accumulator, 0, None);
        }
        for (job, groups) in &self.jobs {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("multilayer_grain"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&job.pipeline.pipeline);
            pass.set_bind_group(0, &job.bg, &[]);
            pass.dispatch_workgroups(groups.0, groups.1, 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn render(backend: &WgpuBackend, gp: &crate::GrainGpuParams) -> Vec<f32> { render_size(backend,gp,[256,256]) }
    fn render_size(backend: &WgpuBackend, gp: &crate::GrainGpuParams, [width,height]:[u32;2]) -> Vec<f32> {
        let n = (width * height) as usize;
        let input = vec![1.5f32; n * 3];
        let bytes = (n * 12) as u64;
        let a = backend
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("grain_test_input"),
                contents: bytemuck::cast_slice(&input),
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
            });
        let b = backend.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let output = backend.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let state = super::super::build_grain_state(&backend.device, gp, width, height, &a, &b, backend);
        let mut first: Option<Vec<f32>> = None;
        for _ in 0..2 {
            backend
                .queue
                .write_buffer(&a, 0, bytemuck::cast_slice(&input));
            let mut encoder = backend.device.create_command_encoder(&Default::default());
            state.encode_passes(&mut encoder,n as u32,width,(width.div_ceil(16),height.div_ceil(16)));
            encoder.copy_buffer_to_buffer(&a, 0, &output, 0, bytes);
            backend.queue.submit(Some(encoder.finish()));
            let slice = output.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
            backend.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).expect("GPU work completion");
            rx.recv().unwrap().unwrap();
            let mapped = slice.get_mapped_range().expect("completed buffer mapping");
            let result: Vec<f32> = bytemuck::cast_slice(&mapped).to_vec();
            drop(mapped);
            output.unmap();
            if let Some(previous) = &first {
                assert!(
                    previous == &result,
                    "cached GPU grain state must not accumulate previous image data"
                );
            } else {
                first = Some(result);
            }
        }
        first.unwrap()
    }
    fn stats(v: &[f32]) -> (f64, f64) {
        let vals: Vec<f64> = v.iter().step_by(3).map(|x| *x as f64).collect();
        assert!(vals.iter().all(|x| x.is_finite()));
        let mean = vals.iter().sum::<f64>() / vals.len() as f64;
        (
            mean,
            vals.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / vals.len() as f64,
        )
    }
    fn parameters() -> crate::GrainGpuParams {
        let mut packed = vec![
            0.,
            0.,
            0.,
            3.,
            3.,
            3.,
            1. / 3.,
            1. / 3.,
            1. / 3.,
            0.,
            0.,
            0.,
        ];
        packed.extend([0.; 9]);
        for _ in 0..3 {
            packed.extend([0.5, 1., 1.5]);
        }
        for _ in 0..3 {
            for sl in 0..3 {
                packed.extend([0., 0.5 * (sl + 1) as f32, 30. + 10. * sl as f32, 0.98]);
            }
        }
        crate::GrainGpuParams {
            layers: Some(crate::LayeredGrainGpuParams {
                output_sampling: None,
                curve_len: 2,
                positive: false,
                packed,
                dye_sigma: [[0.; 3]; 3],
                micro_sigma: 0.,
                micro_blur: 0.,
            }),
            density_min: [0.; 3],
            density_max: [3.; 3],
            n_particles_per_pixel: [1.; 3],
            grain_uniformity: [0.98; 3],
            n_sub_layers: 1,
            base_seed: 0,
            grain_blur: 0.,
            monochrome: false,
        }
    }

    #[test]
    fn actual_gpu_multilayer_requires_iir_capability_before_cache_or_consumer() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = match pollster::block_on(instance.request_adapter(
            &wgpu::RequestAdapterOptions::default(),
        )) {
            Ok(adapter) => adapter,
            Err(error) => {
                assert!(std::env::var_os("SPEKTRAFILM_REQUIRE_GPU").is_none(), "{error}");
                eprintln!("SKIP: no Vulkan adapter for grain capability test: {error}");
                return;
            }
        };
        // A narrow-grain device deliberately has no passthrough shader feature.
        // The same valid device must stay usable after a wide-grain rejection.
        let mut descriptor = WgpuBackend::device_descriptor(&adapter);
        descriptor.required_features = wgpu::Features::empty();
        let (device, queue) = pollster::block_on(adapter.request_device(&descriptor))
            .expect("narrow-grain test device");
        let backend = WgpuBackend::from_device(device, queue);
        backend.configure_work_images(true);
        backend.configure_negative_cache(true);
        let input = ImageBuf::from_data(2, 2, f32_to_scalars(vec![0.18; 12]));
        let options = crate::FilmRenderOptions {
            source: Some(crate::SourceIdentity(11)),
            ..Default::default()
        };
        // Small, valid spectral inputs keep this a public full-chain check,
        // without relying on a particular film profile or malformed dummy data.
        let chain = |grain| crate::FilmChainParams {
            image: &input,
            front: crate::FrontPass::Mallett2019 {
                matrix: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            },
            film_log_exposure: &[-3., 1.],
            film_density_curves_normalized: &[[0.; 3], [3.; 3]],
            film_gamma: 1.,
            film_channel_density: &[[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            film_base_density: &[0.; 3],
            print_illuminant: &[1.; 3],
            print_sensitivity: &[[1.; 3]; 3],
            print_normalization_factor: 1.,
            print_log_exposure: &[-3., 1.],
            print_density_curves: &[[0.; 3], [3.; 3]],
            print_gamma: 1.,
            print_channel_density: &[[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            print_base_density: &[0.; 3],
            preflash: [0.; 3],
            viewing_illuminant: &[1.; 3],
            scan_normalization: 1.,
            scan_xyz_to_rgb: &[[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            bw_xyz_remap: None,
            scan_film: true,
            halation: None,
            dir_couplers: None,
            grain: Some(grain),
            glare: None,
            gamut: None,
            unsharp: None,
            camera_lens_blur_px: None,
            scanner_lens_blur_px: None,
            highlight_boost: None,
            diffusion: None,
            enlarger_diffusion: None,
            print_exposure_scale: 1.,
            output_cctf_encoding: false,
        };
        let baseline = backend.run_film_chain_with_options(&chain(parameters()), options)
            .expect("unblurred multilayer grain needs no IIR capability");
        assert!(baseline.data.iter().all(|v| v.is_finite()));
        let before_cache = backend.negative_cache_stats();
        let before_work = backend.work_image_bytes();
        assert!(before_cache.resident_bytes > 0 && before_work > 0);

        // Each possible source of an IIR pass, including output-space scaling,
        // must reject before taking a retained pair or replacing a density key.
        for case in 0..5 {
            let mut gp = parameters();
            match case {
                0 => gp.grain_blur = 3.,
                1 => {
                    gp.grain_blur = 1.5;
                    gp.layers.as_mut().unwrap().output_sampling = Some([2., 1.]);
                }
                2 => gp.layers.as_mut().unwrap().dye_sigma[1][2] = 3.,
                3 => {
                    let g = gp.layers.as_mut().unwrap();
                    g.dye_sigma[0][0] = 1.5;
                    g.output_sampling = Some([2., 1.]);
                }
                4 => {
                    let g = gp.layers.as_mut().unwrap();
                    g.micro_sigma = 0.3;
                    g.micro_blur = 1.5;
                    g.output_sampling = Some([2., 1.]);
                }
                _ => unreachable!(),
            }
            let called = std::cell::Cell::new(false);
            let result = backend.with_film_output(&chain(gp), options, None, |_, _, _| {
                called.set(true);
                Ok(())
            });
            assert_eq!(result, Err(OutputError::UnsupportedGrainIir), "case {case}");
            assert!(!called.get(), "unsupported grain must not invoke the consumer");
            assert_eq!(backend.negative_cache_stats(), before_cache);
            assert_eq!(backend.work_image_bytes(), before_work);
        }

        // Original activation thresholds and scaled narrow radii still work.
        // The legacy sampler uses its own FIR implementation even for sigma 3+.
        for case in 0..4 {
            let mut gp = parameters();
            match case {
                0 => gp.grain_blur = 2.999,
                1 => {
                    let g = gp.layers.as_mut().unwrap();
                    g.dye_sigma = [[0.4; 3]; 3];
                    g.micro_sigma = 0.05;
                    g.micro_blur = 100.;
                    g.output_sampling = Some([20., 1.]);
                }
                2 => {
                    gp.grain_blur = 4.;
                    let g = gp.layers.as_mut().unwrap();
                    g.dye_sigma[0][1] = 4.;
                    g.micro_sigma = 0.3;
                    g.micro_blur = 4.;
                    g.output_sampling = Some([0.5, 1.]);
                }
                3 => {
                    gp.layers = None;
                    gp.grain_blur = 12.;
                }
                _ => unreachable!(),
            }
            let out = backend.run_film_chain_with_options(&chain(gp), options)
                .expect("narrow and legacy grain remain available");
            assert_eq!((out.width, out.height), (2, 2));
            assert!(out.data.iter().all(|v| v.is_finite()), "case {case}");
        }
    }

    #[test]
    fn actual_gpu_grain_follows_source_coordinates() {
        let mut backend=WgpuBackend::new().expect("real GPU required");
        for mode in 0..4 {
            let mut gp=parameters();
            if mode==0 {gp.layers=None;}
            if mode==2 {gp.layers.as_mut().unwrap().output_sampling=Some([2.,1.]);}
            if mode==3 {gp.layers.as_mut().unwrap().micro_sigma=0.3;}
            backend.configure_grain_reference(None);
            let full=render(&backend,&gp);
            let original=full.clone();
            // Integer translations, 90-degree rotation and reduced input all
            // select independently known original pixel coordinates.
            for (matrix,size,kind) in [([1.,0.,37.,0.,1.,23.],[64,48],0),
                ([0.,1.,37.,-1.,0.,87.],[64,48],1),
                ([2.,0.,0.,0.,2.,0.],[128,128],2),
                ([0.6,0.8,40.,-0.8,0.6,120.],[64,48],3)] {
                backend.configure_grain_reference(Some(GrainReference::new(matrix,[256,256],size[0])));
                let actual=render_size(&backend,&gp,size);
                for y in 0..size[1] {for x in 0..size[0] {
                    let (sx,sy)=match kind {0=>(x+37,y+23),1=>(y+37,86-x),2=>(2*x+1,2*y+1),_=>
                        ((0.6*(x as f32+0.5)+0.8*(y as f32+0.5)+40.).floor() as u32,
                         (-0.8*(x as f32+0.5)+0.6*(y as f32+0.5)+120.).floor() as u32)};
                    let i=((y*size[0]+x)*3) as usize;let j=((sy*256+sx)*3) as usize;
                    // Continuous enlarged-grain sampling uses subpixel positions,
                    // so the reduced case is not a nearest-pixel reference.
                    if !(mode==2 && kind>=2) {assert_eq!(&actual[i..i+3],&full[j..j+3],"mode {mode}, kind {kind}, {x},{y}");}
                }}
            }
            backend.configure_grain_reference(None);
            assert_eq!(render(&backend,&gp),original,"later legacy dispatch must reset reference coordinates");
        }
    }
    #[test]
    fn actual_gpu_multilayer_moments_and_controls() {
        let backend = WgpuBackend::new().expect("real GPU required; no skipped success");
        let mut gp=parameters();
        let original = render(&backend, &gp);
        let (m, v) = stats(&original);
        // Independent analytic mean and variance of a sum of thinned Poisson
        // layers. Six standard errors, plus 5% variance tolerance at N=65536.
        let expected_var: f64 = (0..3)
            .map(|sl| {
                let max = 0.5 * (sl + 1) as f64;
                let density = 0.5 * max;
                density * max * (1. - 0.5 * 0.98 * (1. - 1e-6)) / (30. + 10. * sl as f64)
            })
            .sum();
        assert!(
            (m - 1.5).abs() < 6. * (expected_var / 65536.).sqrt(),
            "mean {m}"
        );
        assert!(
            (v / expected_var - 1.).abs() < 0.05,
            "variance {v}, analytic {expected_var}"
        );
        assert_eq!(original, render(&backend, &gp), "same seed must reproduce");
        // Independent density-residual oracle, not a second noise layer.
        gp.layers.as_mut().unwrap().output_sampling=Some([2.,0.]);
        assert!(render(&backend,&gp).iter().all(|v|*v==1.5));
        gp.layers.as_mut().unwrap().output_sampling=Some([1.,0.5]);
        let half=render(&backend,&gp);
        for (a,b) in half.iter().zip(&original) {assert!((*a-(1.5+0.5*(*b-1.5))).abs()<1e-6);}
        gp.layers.as_mut().unwrap().output_sampling=Some([2.,1.]);
        let scaled=render(&backend,&gp);assert_ne!(scaled,original);
        let diff=|a:&[f32]|a.chunks_exact(3).take(65535).zip(a.chunks_exact(3).skip(1)).map(|(a,b)|(a[0]-b[0]).powi(2) as f64).sum::<f64>();
        assert!(diff(&scaled)<diff(&original)*0.75,"scale changes spatial correlation");
        gp.layers.as_mut().unwrap().output_sampling=None;
        assert_eq!(original,render(&backend,&gp),"off restores pinned route");
        gp.base_seed = 1;
        assert_ne!(
            original,
            render(&backend, &gp),
            "seed must change the realisation"
        );
        gp.base_seed = 0;
        gp.layers.as_mut().unwrap().dye_sigma = [[0.8; 3]; 3];
        let (_, cloud) = stats(&render(&backend, &gp));
        assert!(cloud < v * 0.5);
        gp.layers.as_mut().unwrap().dye_sigma = [[0.; 3]; 3];
        gp.layers.as_mut().unwrap().micro_sigma = 0.3;
        let (_, micro) = stats(&render(&backend, &gp));
        assert!(micro > v * 2.);
        gp.grain_blur = 3.0;
        let (_, wide) = stats(&render(&backend, &gp));
        assert!(wide < micro, "wide grain blur must run and reduce variance");
        gp.grain_blur = 0.0;
        gp.layers.as_mut().unwrap().micro_sigma = 0.0;
        gp.layers.as_mut().unwrap().dye_sigma = [[3.0, 0.8, 0.0]; 3];
        let (_, mixed_cloud) = stats(&render(&backend, &gp));
        assert!(
            mixed_cloud < cloud,
            "mixed IIR/FIR/inactive dye channels must run"
        );
        gp.layers.as_mut().unwrap().dye_sigma = [[0.0; 3]; 3];
        gp.layers.as_mut().unwrap().micro_sigma = 0.3;
        gp.layers.as_mut().unwrap().micro_blur = 12.0;
        let (_, wide_micro) = stats(&render(&backend, &gp));
        assert!(
            wide_micro < micro,
            "microstructure must use the wide GPU blur too"
        );
        println!(
            "actual GPU moments: mean={m}, variance={v}, analytic={expected_var}, dye_cloud_variance={cloud}, micro_variance={micro}"
        );
    }
}
