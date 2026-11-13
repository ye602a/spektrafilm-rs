//! Consumers of the completed physical-chain image. The physical passes live
//! in one implementation; these methods only choose the final transfer route.
use super::*;
use wgpu::util::DeviceExt;

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
    ) -> wgpu::Buffer {
        // Keep the existing integrated-GPU direct-map route. Those work planes
        // are never pooled and stay owned through map completion and unmap.
        if source.usage().contains(wgpu::BufferUsages::MAP_READ) {
            return source.clone();
        }
        let target = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("film_output_readback"),
            size: source.size(),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(source, 0, &target, 0, source.size());
        target
    }

    fn read_completed_output<R>(
        &self,
        buffer: &wgpu::Buffer,
        read: impl FnOnce(&[u8]) -> R,
    ) -> Result<R, OutputError> {
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .map_err(|_| OutputError::GpuFailure)?;
        rx.recv()
            .map_err(|_| OutputError::GpuFailure)?
            .map_err(|_| OutputError::GpuFailure)?;
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
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
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
