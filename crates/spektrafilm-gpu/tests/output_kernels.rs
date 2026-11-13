//! Execute the migrated FPW output kernels on Vulkan and compare their public
//! numerical contract with independent CPU calculations. An absent GPU fails
//! when SPEKTRAFILM_REQUIRE_GPU is set by CI.
#![cfg(feature = "wgpu-backend")]

use spektrafilm_gpu::{output::DisplayRegion, wgpu_backend::WgpuBackend};
use wgpu::util::DeviceExt;

const DISPLAY: &str = include_str!("../../spektrafilm-shaders/wgsl/film_display.wgsl");
const QUANTIZE: &str = include_str!("../../spektrafilm-shaders/wgsl/film_quantize.wgsl");

fn backend() -> Option<WgpuBackend> {
    let backend = WgpuBackend::new();
    if backend.is_none() {
        assert!(
            std::env::var_os("SPEKTRAFILM_REQUIRE_GPU").is_none(),
            "required Vulkan output-kernel test cannot be skipped"
        );
        eprintln!("SKIP: no Vulkan adapter for output-kernel test");
    }
    backend
}

fn dispatch(
    backend: &WgpuBackend,
    shader: &'static str,
    input: &[f32],
    uniform: &[u32],
    output_bytes: u64,
    groups: [u32; 2],
) -> Vec<u8> {
    let (device, queue) = backend.compute_device();
    let source = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("output-kernel-source"),
        contents: bytemuck::cast_slice(input),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let parameters = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("output-kernel-parameters"),
        contents: bytemuck::cast_slice(uniform),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let result = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("output-kernel-result"),
        size: output_bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("output-kernel-readback"),
        size: output_bytes,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("migrated-output-kernel"),
        source: wgpu::ShaderSource::Wgsl(shader.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("output-kernel-pipeline"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("output-kernel-bindings"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: source.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: result.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: parameters.as_entire_binding(),
            },
        ],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("output-kernel-test"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(groups[0], groups[1], 1);
    }
    encoder.copy_buffer_to_buffer(&result, 0, &readback, 0, output_bytes);
    let submitted = queue.submit([encoder.finish()]);
    let slice = readback.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |mapped| {
        let _ = tx.send(mapped);
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submitted),
            timeout: None,
        })
        .expect("output-kernel GPU completion");
    rx.recv()
        .expect("map callback")
        .expect("output-kernel readback");
    let mapped = slice
        .get_mapped_range()
        .expect("output-kernel mapped bytes");
    let bytes = mapped.to_vec();
    drop(mapped);
    readback.unmap();
    bytes
}

fn cpu_code(value: f32) -> u16 {
    ((value as f64).clamp(0.0, 1.0) * 65535.0).round() as u16
}

#[test]
fn packed_rgb16_matches_f64_rounding_at_every_half_code_and_pads_odd_tail() {
    let Some(backend) = backend() else { return };
    let mut values = vec![
        f32::NEG_INFINITY,
        -1.0,
        -0.0,
        0.0,
        f32::from_bits(1),
        f32::MIN_POSITIVE,
        0.5,
        1.0,
        1.25,
        f32::INFINITY,
        f32::NAN,
        f32::from_bits(0xffc00001),
    ];
    // On either side of every rounding boundary, include the nearest f32 and
    // its two neighbours. A plain f32 multiplication demonstrably differs here.
    for code in 0..65535 {
        let middle = ((code as f64 + 0.5) / 65535.0) as f32;
        values.extend([
            f32::from_bits(middle.to_bits() - 1),
            middle,
            f32::from_bits(middle.to_bits() + 1),
        ]);
    }
    if values.len() % 2 == 0 {
        values.push(0.314159);
    }
    let expected: Vec<u16> = values.iter().copied().map(cpu_code).collect();
    assert!(
        values
            .iter()
            .zip(&expected)
            .any(|(&v, &code)| { (v.clamp(0.0, 1.0) * 65535.0).round() as u16 != code }),
        "fixture must expose the rejected f32-multiply approximation"
    );
    let channels = values.len() as u32;
    let pairs = channels.div_ceil(2);
    // Force a multi-row dispatch without needing an enormous test image.
    let groups_x = 7;
    let groups_y = pairs.div_ceil(256).div_ceil(groups_x);
    let bytes = dispatch(
        &backend,
        QUANTIZE,
        &values,
        &[channels, groups_x * 256, 0, 0],
        pairs as u64 * 4,
        [groups_x, groups_y],
    );
    let actual: Vec<u16> = bytes
        .chunks_exact(4)
        .flat_map(|word| {
            let word = u32::from_le_bytes(word.try_into().unwrap());
            [word as u16, (word >> 16) as u16]
        })
        .collect();
    assert_eq!(&actual[..expected.len()], &expected);
    assert_eq!(
        actual[expected.len()],
        0,
        "odd final channel must have zero high padding"
    );
}

fn display(
    backend: &WgpuBackend,
    input: &[f32],
    source: [u32; 2],
    view: DisplayRegion,
) -> Vec<f32> {
    view.validate(source[0], source[1]).unwrap();
    let uniform = [
        source[0],
        source[1],
        0,
        0,
        view.crop[0],
        view.crop[1],
        view.crop[2],
        view.crop[3],
        view.width,
        view.height,
        0,
        0,
    ];
    let bytes = dispatch(
        backend,
        DISPLAY,
        input,
        &uniform,
        view.width as u64 * view.height as u64 * 12,
        [view.width.div_ceil(16), view.height.div_ceil(16)],
    );
    bytes
        .chunks_exact(4)
        .map(|word| f32::from_le_bytes(word.try_into().unwrap()))
        .collect()
}

fn area_reference(input: &[f32], source_width: u32, view: DisplayRegion) -> Vec<f32> {
    let mut expected = Vec::new();
    for oy in 0..view.height {
        for ox in 0..view.width {
            let rx = view.crop[2] as f32 / view.width as f32;
            let ry = view.crop[3] as f32 / view.height as f32;
            let x0 = ((ox as f32 * rx).ceil() as u32).min(view.crop[2] - 1);
            let y0 = ((oy as f32 * ry).ceil() as u32).min(view.crop[3] - 1);
            let x1 = ((ox as f32 * rx + rx).ceil() as u32).min(view.crop[2]);
            let y1 = ((oy as f32 * ry + ry).ceil() as u32).min(view.crop[3]);
            for c in 0..3 {
                let mut total = 0.0_f32;
                for y in y0..y1 {
                    let mut row = 0_u32;
                    for x in x0..x1 {
                        let i = (((y + view.crop[1]) * source_width + x + view.crop[0]) * 3)
                            as usize
                            + c;
                        row += (input[i].clamp(0.0, 1.0) * 65535.0 + 0.5).floor() as u32;
                    }
                    total += row as f32;
                }
                expected.push((total / ((x1 - x0) * (y1 - y0)) as f32 + 0.5).floor() / 65535.0);
            }
        }
    }
    expected
}

#[test]
fn display_region_preserves_one_to_one_values_and_existing_area_reduction() {
    let Some(backend) = backend() else { return };
    let source = [11, 9];
    let input: Vec<f32> = (0..source[0] * source[1] * 3)
        .map(|i| (i % 101) as f32 / 80.0 - 0.1)
        .collect();
    let crop = DisplayRegion {
        crop: [2, 1, 7, 5],
        width: 7,
        height: 5,
    };
    let actual = display(&backend, &input, source, crop);
    let mut exact = Vec::new();
    for y in 1..6 {
        for x in 2..9 {
            let first = ((y * source[0] + x) * 3) as usize;
            exact.extend_from_slice(&input[first..first + 3]);
        }
    }
    assert_eq!(
        actual.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        exact.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        "1:1 extraction must preserve even unclamped original f32 samples"
    );
    for (width, height) in [(4, 3), (1, 1), (7, 2)] {
        let reduced = DisplayRegion {
            width,
            height,
            ..crop
        };
        assert_eq!(
            display(&backend, &input, source, reduced),
            area_reference(&input, source[0], reduced)
        );
    }
    let tiny = DisplayRegion {
        crop: [10, 8, 1, 1],
        width: 1,
        height: 1,
    };
    assert_eq!(
        display(&backend, &input, source, tiny),
        input[input.len() - 3..]
    );
}
