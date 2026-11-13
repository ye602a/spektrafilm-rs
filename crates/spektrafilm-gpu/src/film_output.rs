//! Consumers of the completed physical-chain image. The physical passes live
//! in one implementation; these methods only choose the final transfer route.
use super::*;
use wgpu::util::DeviceExt;

/// A mapping registered on the same submission as the output writes. The
/// backend's existing submission wait drives its completion; no second queue
/// wait is needed when the CPU result is collected.
struct OutputReadback {
    buffer: wgpu::Buffer,
    mapping: std::sync::mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
}

impl WgpuBackend {
    pub fn run_film_chain_with_options(
        &self,
        p: &crate::FilmChainParams<'_>,
        options: FilmRenderOptions<'_>,
    ) -> Result<ImageBuf, OutputError> {
        self.run_film_chain_region(p, options, None)
    }

    /// Full physical development followed by an optional display crop/reduction
    /// and f32 readback. A display crop never changes upstream grain geometry.
    pub fn run_film_chain_region(
        &self,
        p: &crate::FilmChainParams<'_>,
        options: FilmRenderOptions<'_>,
        region: Option<DisplayRegion>,
    ) -> Result<ImageBuf, OutputError> {
        self.with_film_output_impl(
            p,
            options,
            region,
            |_, encoder, output| {
                Ok((
                    self.stage_output_readback(encoder, output.buffer),
                    output.width,
                    output.height,
                ))
            },
            |(readback, width, height)| {
                let samples: Vec<f32> = self.read_completed_output(&readback, |bytes| {
                    bytemuck::cast_slice(bytes).to_vec()
                })?;
                Ok(ImageBuf::from_data(width, height, f32_to_scalars(samples)))
            },
        )
    }

    /// Exact FPW-compatible f32 -> RGB16 quantization on the GPU. Only packed
    /// 16-bit samples cross the readback boundary, including after a region.
    pub fn run_film_chain_rgb16(
        &self,
        p: &crate::FilmChainParams<'_>,
        options: FilmRenderOptions<'_>,
        region: Option<DisplayRegion>,
    ) -> Result<Rgb16Image, OutputError> {
        self.with_film_output_impl(
            p,
            options,
            region,
            |_, encoder, output| {
                let channels = output_channel_count(output.width, output.height)?;
                let packed = self.encode_rgb16(encoder, output.buffer, channels)?;
                Ok((
                    self.stage_output_readback(encoder, &packed),
                    output.width,
                    output.height,
                    channels,
                ))
            },
            |(readback, width, height, channels)| {
                let data = self.read_completed_output(&readback, |bytes| {
                    let mut data = Vec::with_capacity(channels as usize);
                    for word in bytes.chunks_exact(4) {
                        let packed = u32::from_le_bytes(word.try_into().unwrap());
                        data.push(packed as u16);
                        if data.len() < channels as usize {
                            data.push((packed >> 16) as u16);
                        }
                    }
                    data
                })?;
                Ok(Rgb16Image {
                    width,
                    height,
                    data,
                })
            },
        )
    }

    /// Record a same-device consumer of the final interleaved RGB f32 buffer.
    /// The callback runs while commands are being encoded, after the physical
    /// and optional display passes. The engine submits and waits for all of
    /// them before returning the callback value or recycling its work buffers.
    ///
    /// Copy into host-owned buffers/textures inside this callback. Retaining the
    /// engine's buffer or submitting its consumers later violates this lease.
    /// No full-image CPU readback is allocated for this route.
    pub fn with_film_output<R>(
        &self,
        p: &crate::FilmChainParams<'_>,
        options: FilmRenderOptions<'_>,
        region: Option<DisplayRegion>,
        consume: impl FnOnce(
            &wgpu::Device,
            &mut wgpu::CommandEncoder,
            FilmOutput<'_>,
        ) -> Result<R, OutputError>,
    ) -> Result<R, OutputError> {
        self.with_film_output_impl(p, options, region, consume, Ok)
    }

    fn stage_output_readback(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::Buffer,
    ) -> OutputReadback {
        // Keep the existing integrated-GPU direct-map route. Those work planes
        // are never pooled and stay owned through map completion and unmap.
        let buffer = if source.usage().contains(wgpu::BufferUsages::MAP_READ) {
            source.clone()
        } else {
            let target = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("film_output_readback"),
                size: source.size(),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_buffer_to_buffer(source, 0, &target, 0, source.size());
            target
        };
        let (tx, mapping) = std::sync::mpsc::channel();
        // Starting the map while encoding would make the buffer unavailable
        // to this command. wgpu defers it until immediately after submission,
        // so the one existing wait completes both GPU writes and mapping.
        encoder.map_buffer_on_submit(&buffer, wgpu::MapMode::Read, .., move |result| {
            let _ = tx.send(result);
        });
        OutputReadback { buffer, mapping }
    }

    fn read_completed_output<R>(
        &self,
        readback: &OutputReadback,
        read: impl FnOnce(&[u8]) -> R,
    ) -> Result<R, OutputError> {
        // Native poll completes this submission's mapping callbacks before
        // returning. A concurrent polling thread may be delivering the tiny
        // callback; receive its result without waiting for unrelated GPU work.
        readback.mapping.recv()
            .map_err(|_| OutputError::GpuFailure)?
            .map_err(|_| OutputError::GpuFailure)?;
        let buffer = &readback.buffer;
        let slice = buffer.slice(..);
        let mapped = slice
            .get_mapped_range()
            .map_err(|_| OutputError::GpuFailure)?;
        let result = read(&mapped);
        drop(mapped);
        buffer.unmap();
        Ok(result)
    }

    pub(super) fn encode_display_region(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::Buffer,
        width: u32,
        height: u32,
        region: DisplayRegion,
    ) -> Result<wgpu::Buffer, OutputError> {
        let bytes = output_channel_count(region.width, region.height)? as u64 * 4;
        let groups = [region.width.div_ceil(16), region.height.div_ceil(16)];
        self.check_output_limits(bytes, groups)?;
        let target = self.output_storage("film_display_region", bytes);
        let uniform = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("film_display_region_params"),
                contents: bytemuck::cast_slice(&region.uniform(width, height)),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        self.encode_output_kernel(
            encoder,
            include_str!("../../spektrafilm-shaders/wgsl/film_display.wgsl"),
            source,
            &target,
            &uniform,
            groups,
        );
        Ok(target)
    }

    fn encode_rgb16(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::Buffer,
        channels: u32,
    ) -> Result<wgpu::Buffer, OutputError> {
        let pairs = channels.div_ceil(2);
        let groups = pairs.div_ceil(256);
        let x = groups.min(self.device.limits().max_compute_workgroups_per_dimension);
        let y = groups.div_ceil(x);
        let bytes = pairs as u64 * 4;
        self.check_output_limits(bytes, [x, y])?;
        // The original kernel computes first = pair * 2 using u32.
        if x as u64 * y as u64 * 256 * 2 > u32::MAX as u64 {
            return Err(OutputError::ChannelCountOverflow);
        }
        let target = self.output_storage("film_packed_rgb16", bytes);
        let uniform = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("film_rgb16_params"),
                contents: bytemuck::cast_slice(&[channels, x * 256, 0, 0]),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        self.encode_output_kernel(
            encoder,
            include_str!("../../spektrafilm-shaders/wgsl/film_quantize.wgsl"),
            source,
            &target,
            &uniform,
            [x, y],
        );
        Ok(target)
    }

    fn check_output_limits(&self, bytes: u64, groups: [u32; 2]) -> Result<(), OutputError> {
        let limits = self.device.limits();
        if bytes > limits.max_buffer_size
            || bytes > limits.max_storage_buffer_binding_size as u64
            || groups
                .iter()
                .any(|&g| g == 0 || g > limits.max_compute_workgroups_per_dimension)
        {
            return Err(OutputError::DeviceLimitExceeded);
        }
        Ok(())
    }

    fn output_storage(&self, label: &str, bytes: u64) -> wgpu::Buffer {
        let mut usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC;
        // Match the backend's existing primary-buffer mapping policy. Devices
        // without this optional feature retain their ordinary staging path.
        if self.device.features().contains(wgpu::Features::MAPPABLE_PRIMARY_BUFFERS) {
            usage |= wgpu::BufferUsages::MAP_READ;
        }
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: bytes,
            usage,
            mapped_at_creation: false,
        })
    }

    fn encode_output_kernel(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        shader: &'static str,
        source: &wgpu::Buffer,
        target: &wgpu::Buffer,
        uniform: &wgpu::Buffer,
        groups: [u32; 2],
    ) {
        let pipeline = self.cached_pipeline(
            shader,
            &[
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
                wgpu::BufferBindingType::Uniform,
            ],
        );
        let bindings = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("film_output_bindings"),
            layout: &pipeline.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: source.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: target.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("film_output_consumer"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline.pipeline);
        pass.set_bind_group(0, &bindings, &[]);
        pass.dispatch_workgroups(groups[0], groups[1], 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_mapping_completes_with_its_submission_and_abandoned_encoding_recovers() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = match pollster::block_on(
            instance.request_adapter(&wgpu::RequestAdapterOptions::default()),
        ) {
            Ok(adapter) => adapter,
            Err(error) => {
                assert!(std::env::var_os("SPEKTRAFILM_REQUIRE_GPU").is_none(), "{error}");
                eprintln!("SKIP: no Vulkan adapter for output mapping test: {error}");
                return;
            }
        };
        for direct in [false, true] {
            let mapping_feature = wgpu::Features::MAPPABLE_PRIMARY_BUFFERS;
            if direct && !adapter.features().contains(mapping_feature) {
                eprintln!("SKIP: optional primary-buffer mapping is unavailable");
                continue;
            }
            let mut descriptor = WgpuBackend::device_descriptor(&adapter);
            descriptor.required_features.set(mapping_feature, direct);
            let (device, queue) = pollster::block_on(adapter.request_device(&descriptor))
                .expect("output mapping device");
            let backend = WgpuBackend::from_device(device, queue);
            let source = backend.output_storage("output mapping fixture", 20);
            assert_eq!(source.usage().contains(wgpu::BufferUsages::MAP_READ), direct);

            // Abandoning an encoded output must not map the buffer early and
            // prevent a later GPU request from using it.
            {
                let mut encoder = backend.device.create_command_encoder(&Default::default());
                let abandoned = backend.stage_output_readback(&mut encoder, &source);
                assert!(matches!(abandoned.mapping.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty)));
                drop(encoder);
                drop(abandoned);
            }

            let shader = backend.device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("output mapping fixture"),
                source: wgpu::ShaderSource::Wgsl(
                    "@group(0) @binding(0) var<storage, read_write> result: array<u32>;\n\
                     @compute @workgroup_size(1) fn main(@builtin(global_invocation_id) p: vec3<u32>) {\n\
                     result[p.x] = 1048577u + p.x; }".into(),
                ),
            });
            let pipeline = backend.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("output mapping fixture"),
                layout: None,
                module: &shader,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });
            let bindings = backend.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("output mapping fixture"),
                layout: &pipeline.get_bind_group_layout(0),
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: source.as_entire_binding(),
                }],
            });
            let mut encoder = backend.device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &bindings, &[]);
                pass.dispatch_workgroups(5, 1, 1);
            }
            let readback = backend.stage_output_readback(&mut encoder, &source);
            assert_eq!(readback.buffer == source, direct, "direct mapping must not allocate a staging copy");
            let submitted = backend.submit([encoder.finish()]);
            backend.device.poll(wgpu::PollType::Wait {
                submission_index: Some(submitted),
                timeout: Some(std::time::Duration::from_secs(30)),
            }).expect("one output submission wait");
            readback.mapping.try_recv()
                .expect("this submission's mapping callback must already have run")
                .expect("output mapping");
            let mapped = readback.buffer.slice(..).get_mapped_range().unwrap();
            assert_eq!(bytemuck::cast_slice::<u8, u32>(&mapped), &[1048577, 1048578, 1048579, 1048580, 1048581]);
            drop(mapped);
            readback.buffer.unmap();
        }
    }
}
