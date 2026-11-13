//! Regression for FPW-derived source-anchored stochastic grain.
#![cfg(feature = "wgpu-backend")]

use spektrafilm_gpu::wgpu_backend::{GrainReference, WgpuBackend};
use wgpu::util::DeviceExt;

const GRAIN_SHADER: &str = concat!(
    include_str!("../../spektrafilm-shaders/wgsl/grain_reference.wgsl"),
    include_str!("../../spektrafilm-shaders/wgsl/grain.wgsl")
);

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GrainParams {
    n_pixels: u32,
    seed: u32,
    layers: u32,
    monochrome: u32,
    density_min: [f32; 4],
    density_max: [f32; 4],
    particles: [f32; 4],
    uniformity: [f32; 4],
    reference: GrainReference,
}

fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    width: u32,
    height: u32,
    reference: GrainReference,
) -> Vec<f32> {
    let count = width * height;
    let input = vec![0.45_f32; count as usize * 3];
    let params = GrainParams {
        n_pixels: count,
        seed: 1,
        layers: 3,
        monochrome: 0,
        density_min: [0.2, 0.2, 0.2, 0.0],
        density_max: [2.0, 2.0, 2.0, 0.0],
        particles: [100.0, 110.0, 120.0, 0.0],
        uniformity: [0.2, 0.3, 0.4, 0.0],
        reference,
    };
    let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("source-grain-params"),
        contents: bytemuck::bytes_of(&params),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let data_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("source-grain-data"),
        contents: bytemuck::cast_slice(&input),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    });
    let bytes = (input.len() * std::mem::size_of::<f32>()) as u64;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("source-grain-readback"),
        size: bytes,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("production-grain-shader"),
        source: wgpu::ShaderSource::Wgsl(GRAIN_SHADER.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("source-grain-pipeline"),
        layout: None,
        module: &shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("source-grain-bindings"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: params_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: data_buffer.as_entire_binding(),
            },
        ],
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("source-grain-encoder"),
    });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("source-grain-dispatch"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(count.div_ceil(1024), 1, 1);
    }
    encoder.copy_buffer_to_buffer(&data_buffer, 0, &readback, 0, bytes);
    queue.submit([encoder.finish()]);
    let slice = readback.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    device
        .poll(wgpu::PollType::Wait { submission_index: None, timeout: None })
        .expect("grain submission");
    rx.recv().expect("grain mapping callback").expect("grain map");
    let mapped = slice.get_mapped_range().expect("grain mapped view");
    let output: Vec<f32> = bytemuck::cast_slice(&mapped).to_vec();
    drop(mapped);
    readback.unmap();
    output
}

fn pixel(image: &[f32], width: usize, x: usize, y: usize) -> &[f32] {
    &image[3 * (y * width + x)..3 * (y * width + x + 1)]
}

#[test]
fn source_anchored_grain_matches_full_frame_after_crop_and_rotation() {
    let Some(backend) = WgpuBackend::new() else {
        if std::env::var_os("SPEKTRAFILM_REQUIRE_GPU").is_some() {
            panic!("Vulkan GPU was required for the grain source-coordinate test");
        }
        eprintln!("SKIP: no Vulkan adapter for grain source-coordinate test");
        return;
    };
    let (device, queue) = backend.compute_device();
    let full = render(device, queue, 4, 4, GrainReference::default());

    // Opting into identity source mapping must reproduce the old seed field.
    let identity = render(
        device, queue, 4, 4,
        GrainReference::new([1., 0., 0., 0., 1., 0.], [4, 4], 4),
    );
    assert_eq!(full, identity, "identity source reference must not change grain");

    // Crop [1..3, 1..3] keeps original indices rather than resetting at zero.
    let cropped = render(
        device, queue, 2, 2,
        GrainReference::new([1., 0., 1., 0., 1., 1.], [4, 4], 2),
    );
    for y in 0..2 {
        for x in 0..2 {
            assert_eq!(
                pixel(&cropped, 2, x, y),
                pixel(&full, 4, x + 1, y + 1),
                "crop should retain original pixel RNG"
            );
        }
    }

    // 90° orientation: dest (x,y) samples source (3-y,x).
    let rotated = render(
        device, queue, 4, 4,
        GrainReference::new([0., -1., 3., 1., 0., 0.], [4, 4], 4),
    );
    for y in 0..4 {
        for x in 0..4 {
            assert_eq!(
                pixel(&rotated, 4, x, y),
                pixel(&full, 4, 3 - y, x),
                "rotation should follow source sampling"
            );
        }
    }
}
