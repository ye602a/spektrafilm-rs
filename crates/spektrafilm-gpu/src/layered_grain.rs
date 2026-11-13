//! Resident multilayer grain. Optional dye clouds and microstructure stay on
//! GPU. Only small curves/parameters cross the CPU boundary, never image data.
use super::grain_iir::{self, GrainJob};
use super::*;
use wgpu::util::DeviceExt;

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
                if sigmas.iter().any(|s| *s >= 3.0) {
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
        let scale = g.output_sampling.map_or(1., |v| v[0]);
        let cloud = g.dye_sigma.iter().flatten().any(|s| *s > 0.4);
        let micro = g.micro_sigma > 0.05;
        let tmp = if cloud || (micro && g.micro_blur > 0.4) {
            Some(alloc())
        } else {
            None
        };
        let values = [0.0, 0.0, 0.0, gp.monochrome as u32 as f32];
        if cloud {
            let acc = alloc();
            for sl in 0..3 {
                add(&mut jobs, 1, sl as u32, values, &g.packed, a, b);
                blur(
                    &mut jobs,
                    g.dye_sigma[sl].map(|s| if s > 0.4 { s * scale } else { 0.0 }),
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
            if g.micro_blur > 0.4 {
                blur(&mut jobs, [g.micro_blur * scale; 3], b, tmp.as_ref().unwrap());
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
        blur(&mut jobs, [gp.grain_blur * scale; 3], a, b);
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
