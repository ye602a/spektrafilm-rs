//! Geometry and transfer contract for post-scan GPU output.
//!
//! Display extraction happens after the complete physical film chain. It never
//! changes development, optical effects, grain coordinates or density caching.

/// A display-only crop and optional reduction of the completed full image.
///
/// `crop` is `[x, y, width, height]` in full-resolution output pixels. Output
/// dimensions must be positive and at most the crop dimensions: this interface
/// preserves the established FPW area-reduction rule and does not interpolate
/// an enlargement. At 1:1 the original f32 samples are copied exactly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayRegion {
    pub crop: [u32; 4],
    pub width: u32,
    pub height: u32,
}

/// An invalid, cancelled or unsuccessful output request. Geometry failures are
/// rejected before GPU execution; a completion failure or late cancellation can
/// happen after submitted work has run, but never returns a successful output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum OutputError {
    #[error("render cancelled; no output or cache entry was published")]
    Cancelled,
    #[error("image dimensions do not match the interleaved RGB input data")]
    InvalidImageData,
    #[error("image or output exceeds this device's buffer or dispatch limits")]
    DeviceLimitExceeded,
    #[error("GPU submission, completion, or buffer mapping failed")]
    GpuFailure,
    #[error("wide multilayer grain requires a device with the validated Vulkan IIR shader capability")]
    UnsupportedGrainIir,
    #[error("this GPU backend does not support the selected multilayer grain model; use the Vulkan backend")]
    UnsupportedMultilayerGrain,
    #[error("source, crop and output dimensions must all be positive")]
    EmptyDimensions,
    #[error("the display crop lies outside the full-resolution output")]
    CropOutOfBounds,
    #[error("display extraction supports 1:1 pixels or reduction, not enlargement")]
    EnlargementUnsupported,
    #[error("RGB channel count exceeds the shader's u32 addressing limit")]
    ChannelCountOverflow,
    #[error("a reduction block is too wide for the retained RGB16 row accumulator")]
    ReductionAccumulatorOverflow,
}

/// Interleaved R, G, B 16-bit codes, with exactly width * height * 3 samples.
/// The internal packed GPU padding word is never exposed as an image channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rgb16Image {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u16>,
}

/// Scoped view of the final RGB f32 buffer in the caller's wgpu device.
///
/// Record consumers into the encoder provided to the callback. The engine
/// submits and waits for those commands before recycling its work buffers.
/// Do not retain/clone this buffer for later submissions; copy into host-owned
/// storage or a texture inside the callback instead.
#[cfg(feature = "wgpu-backend")]
pub struct FilmOutput<'a> {
    pub buffer: &'a wgpu::Buffer,
    pub width: u32,
    pub height: u32,
    /// Mirrors the original chain's final optional sRGB transfer encoding.
    pub cctf_encoding: bool,
}

impl DisplayRegion {
    /// Validate the region against the completed full-resolution image.
    ///
    /// Checked crop arithmetic prevents wrapping a far-out origin back into
    /// the image. Both source and output channel indices must fit in u32.
    pub fn validate(self, source_width: u32, source_height: u32) -> Result<(), OutputError> {
        output_channel_count(source_width, source_height)?;
        output_channel_count(self.width, self.height)?;
        let [x, y, width, height] = self.crop;
        if width == 0 || height == 0 {
            return Err(OutputError::EmptyDimensions);
        }
        if x.checked_add(width)
            .is_none_or(|right| right > source_width)
            || y.checked_add(height)
                .is_none_or(|bottom| bottom > source_height)
        {
            return Err(OutputError::CropOutOfBounds);
        }
        if self.width > width || self.height > height {
            return Err(OutputError::EnlargementUnsupported);
        }
        if (self.width != width || self.height != height)
            && reduction_row_can_overflow(width, self.width)
        {
            return Err(OutputError::ReductionAccumulatorOverflow);
        }
        Ok(())
    }

    /// Uniform layout consumed by the byte-preserved display shader.
    pub(crate) fn uniform(self, source_width: u32, source_height: u32) -> [u32; 12] {
        [
            source_width,
            source_height,
            0,
            0,
            self.crop[0],
            self.crop[1],
            self.crop[2],
            self.crop[3],
            self.width,
            self.height,
            0,
            0,
        ]
    }
}

// The source shader accumulates each row as u32 RGB16 codes. At most 65,537
// full-white samples fit (65,537 * 65,535 == u32::MAX). Check the shader's actual
// f32 block boundaries, rather than refusing a wide panorama that is reduced
// into many narrow blocks. The fast upper bound makes the exact loop relevant
// only for very wide blocks, where output width is at most about 22,000.
fn reduction_row_can_overflow(crop_width: u32, output_width: u32) -> bool {
    const MAX_SAMPLES: u32 = u32::MAX / u16::MAX as u32;
    if crop_width <= MAX_SAMPLES {
        return false;
    }
    let ratio = crop_width as f32 / output_width as f32;
    let largest_sum = crop_width as f32 * 2.0;
    let ulp = largest_sum.next_up() - largest_sum;
    if ratio + 2.0 * ulp + 2.0 <= MAX_SAMPLES as f32 {
        return false;
    }
    (0..output_width).any(|x| {
        let start = x as f32 * ratio;
        let first = (start.ceil() as u32).min(crop_width - 1);
        let last = ((start + ratio).ceil() as u32).min(crop_width);
        last.saturating_sub(first) > MAX_SAMPLES
    })
}

/// Check the nonempty interleaved RGB geometry used by output shaders.
/// Device-specific storage and dispatch limits must additionally be checked by
/// the backend; a valid channel count alone is not an allocation guarantee.
pub fn output_channel_count(width: u32, height: u32) -> Result<u32, OutputError> {
    if width == 0 || height == 0 {
        return Err(OutputError::EmptyDimensions);
    }
    width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(3))
        .ok_or(OutputError::ChannelCountOverflow)
}

/// Validate an input before CPU parameter preparation or GPU encoding.
/// `ImageBuf` exposes its fields, so callers may supply inconsistent dimensions
/// even when they do not use its checked constructor.
pub fn validate_rgb_image(image: &spektrafilm_math::image::ImageBuf) -> Result<u32, OutputError> {
    let channels = output_channel_count(image.width, image.height)?;
    if image.data.len() != channels as usize {
        return Err(OutputError::InvalidImageData);
    }
    Ok(channels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_rejects_empty_outside_overflow_and_enlargement() {
        let valid = DisplayRegion {
            crop: [1, 2, 3, 4],
            width: 3,
            height: 4,
        };
        assert_eq!(valid.validate(4, 6), Ok(()));
        assert_eq!(
            DisplayRegion {
                width: 2,
                height: 1,
                ..valid
            }
            .validate(4, 6),
            Ok(())
        );
        assert_eq!(
            DisplayRegion { width: 0, ..valid }.validate(4, 6),
            Err(OutputError::EmptyDimensions)
        );
        assert_eq!(
            DisplayRegion {
                crop: [0, 0, 0, 1],
                ..valid
            }
            .validate(4, 6),
            Err(OutputError::EmptyDimensions)
        );
        assert_eq!(
            DisplayRegion {
                crop: [u32::MAX, 2, 3, 4],
                ..valid
            }
            .validate(4, 6),
            Err(OutputError::CropOutOfBounds)
        );
        assert_eq!(
            DisplayRegion {
                crop: [2, 2, 3, 4],
                ..valid
            }
            .validate(4, 6),
            Err(OutputError::CropOutOfBounds)
        );
        assert_eq!(
            DisplayRegion { width: 4, ..valid }.validate(4, 6),
            Err(OutputError::EnlargementUnsupported)
        );
        assert_eq!(
            valid.validate(u32::MAX, 2),
            Err(OutputError::ChannelCountOverflow)
        );
    }

    #[test]
    fn region_rejects_only_overflowing_horizontal_reduction_blocks() {
        let view = DisplayRegion {
            crop: [0, 0, 65_538, 1],
            width: 1,
            height: 1,
        };
        assert_eq!(
            view.validate(65_538, 1),
            Err(OutputError::ReductionAccumulatorOverflow)
        );
        // The same very wide image remains valid when each block is small.
        assert_eq!(
            DisplayRegion { width: 2, ..view }.validate(65_538, 1),
            Ok(())
        );
        // The exact full-white accumulator boundary is representable.
        let exact = DisplayRegion {
            crop: [0, 0, 131_074, 1],
            width: 2,
            height: 1,
        };
        assert_eq!(exact.validate(131_074, 1), Ok(()));
        assert_eq!(
            DisplayRegion {
                crop: [0, 0, 131_076, 1],
                ..exact
            }
            .validate(131_076, 1),
            Err(OutputError::ReductionAccumulatorOverflow)
        );
        // A 1:1 copy never invokes the row accumulator.
        assert_eq!(
            DisplayRegion {
                width: 65_538,
                ..view
            }
            .validate(65_538, 1),
            Ok(())
        );
    }
}
