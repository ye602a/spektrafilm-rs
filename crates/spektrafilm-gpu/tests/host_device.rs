//! Smoke tests for embedding the standalone film backend in a host application.
//!
//! No FPW types or application code are involved.

#![cfg(feature = "wgpu-backend")]

use spektrafilm_gpu::wgpu_backend::WgpuBackend;

#[test]
fn host_device_constructor_and_handle_accessor_are_public() {
    // This compile-time check works even when the machine has no GPU.
    let _constructor: fn(wgpu::Device, wgpu::Queue) -> WgpuBackend =
        WgpuBackend::from_device;
    let _accessor: fn(&WgpuBackend) -> (&wgpu::Device, &wgpu::Queue) =
        WgpuBackend::compute_device;
}

#[test]
fn host_can_request_the_engines_device_requirements() {
    let _descriptor: fn(&wgpu::Adapter) -> wgpu::DeviceDescriptor<'static> =
        WgpuBackend::device_descriptor;

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let Some(adapter) = pollster::block_on(instance.request_adapter(
        &wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
            ..Default::default()
        },
    )).ok() else {
        eprintln!("SKIP: no compatible adapter for descriptor test");
        return;
    };
    let descriptor = WgpuBackend::device_descriptor(&adapter);
    let adapter_limits = adapter.limits();
    assert_eq!(
        descriptor.required_limits.max_storage_buffer_binding_size,
        adapter_limits.max_storage_buffer_binding_size
    );
    assert_eq!(
        descriptor.required_limits.max_buffer_size,
        adapter_limits.max_buffer_size
    );
    assert_eq!(
        descriptor.required_limits.max_compute_invocations_per_workgroup,
        adapter_limits.max_compute_invocations_per_workgroup
    );
    assert!(adapter.features().contains(descriptor.required_features));
}

#[test]
fn host_handles_are_reused_without_allocating_a_second_device() {
    // Headless CI runners may not expose a compute adapter. In that case
    // the compile-time API test above still checks the interface.
    let Some(host) = WgpuBackend::new() else {
        eprintln!("SKIP: no compatible GPU adapter was available");
        return;
    };
    let (host_device, host_queue) = host.compute_device();
    let external = WgpuBackend::from_device(host_device.clone(), host_queue.clone());
    let (device, queue) = external.compute_device();

    assert_eq!(device, host_device);
    assert_eq!(queue, host_queue);

    // Submitting through the wrapped handle exercises both the borrowed device
    // and its matching queue, rather than only comparing handle identities.
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("standalone-host-device-test"),
        size: 256,
        usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("standalone-host-device-test"),
    });
    encoder.clear_buffer(&buffer, 0, None);
    queue.submit([encoder.finish()]);
    device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).expect("GPU work completion");
}
