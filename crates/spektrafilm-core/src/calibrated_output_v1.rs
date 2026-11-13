//! Frozen `negative_scan_ap0_v1` inversion and FPW's accepted final output order.
//!
//! Derived from GPL-3.0-or-later FPW at ae3428acd98e39c48784a79dc8f3369ab1236cb1,
//! `negative_scan.rs`, `physical_output.rs`, and the transfer/finish functions in
//! `images.rs`. Calibration data derives from Andrea Volpato's spektrafilm
//! profiles under CC BY-SA 4.0; see data/negative_scan_ap0_v1/PROVENANCE.md.
//! AP0 is the existing v1 negative transmission transport, not a new engine
//! working-space choice. Do not alter these tables or math under this version.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex, OnceLock,
};

use serde::Deserialize;
use spektrafilm_gpu::{
    output::{output_channel_count, DisplayRegion, FilmOutput, OutputError},
    wgpu::{self, util::DeviceExt},
    wgpu_backend::WgpuBackend,
    FilmChainParams, FilmRenderOptions,
};

pub const NEGATIVE_SCAN_AP0_V1: &str = "negative_scan_ap0_v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NegativeStockV1 {
    Pro400H,
    XTra400,
}

impl NegativeStockV1 {
    pub const fn stock_id(self) -> &'static str {
        match self {
            Self::Pro400H => "fujifilm_pro_400h",
            Self::XTra400 => "fujifilm_xtra_400",
        }
    }

    const fn index(self) -> usize {
        match self {
            Self::Pro400H => 0,
            Self::XTra400 => 1,
        }
    }
}

impl TryFrom<&str> for NegativeStockV1 {
    type Error = CalibratedOutputError;
    fn try_from(stock: &str) -> Result<Self, Self::Error> {
        match stock {
            "fujifilm_pro_400h" => Ok(Self::Pro400H),
            "fujifilm_xtra_400" => Ok(Self::XTra400),
            _ => Err(CalibratedOutputError::UnsupportedStock(stock.to_owned())),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhysicalOutputMappingV1 {
    /// The physical chain already supplied encoded sRGB (paper/slide routes).
    EncodedSrgb,
    /// The calibrated stock's linear AP0 transmission scan, with no prior
    /// positive inversion, shoulder, gamut mapping or transfer encoding.
    NegativeScanAp0(NegativeStockV1),
}

#[derive(Clone, Copy, Debug)]
pub struct PhysicalOutputOptionsV1 {
    pub mapping: PhysicalOutputMappingV1,
    /// Existing finish exposure in hundredths of an EV, applied to quantized
    /// positive RGB16 codes before display area reduction.
    pub finish_centi_ev: i32,
    pub region: Option<DisplayRegion>,
}

impl Default for PhysicalOutputOptionsV1 {
    fn default() -> Self {
        Self {
            mapping: PhysicalOutputMappingV1::EncodedSrgb,
            finish_centi_ev: 0,
            region: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CalibratedOutputError {
    #[error("stock has no frozen negative_scan_ap0_v1 calibration: {0}")]
    UnsupportedStock(String),
    #[error("negative scan must contain complete RGB pixels")]
    InvalidScanLength,
    #[error("negative scan contains a nonfinite sample")]
    NonFiniteScan,
    #[error("the source transfer/scan route does not match the selected output mapping")]
    InvalidTransport,
    #[error("the output encoder was created on a different wgpu device")]
    WrongDevice,
    #[error(transparent)]
    Output(#[from] OutputError),
}

#[derive(Deserialize)]
struct Curve {
    absorption: Vec<f64>,
    asinh_exposure: Vec<f64>,
}
#[derive(Deserialize)]
struct Calibration {
    base: [f64; 3],
    curves: Vec<Curve>,
}

fn calibration(stock: NegativeStockV1) -> &'static Calibration {
    static PRO: OnceLock<Calibration> = OnceLock::new();
    static XTRA: OnceLock<Calibration> = OnceLock::new();
    let (slot, text) = match stock {
        NegativeStockV1::Pro400H => (
            &PRO,
            include_str!("../../../data/negative_scan_ap0_v1/pro400h.json"),
        ),
        NegativeStockV1::XTra400 => (
            &XTRA,
            include_str!("../../../data/negative_scan_ap0_v1/xtra.json"),
        ),
    };
    slot.get_or_init(|| serde_json::from_str(text).expect("frozen negative v1 calibration"))
}

fn check_cancelled(stop: Option<&AtomicBool>) -> Result<(), CalibratedOutputError> {
    if stop.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
        return Err(OutputError::Cancelled.into());
    }
    Ok(())
}

/// Original CPU f64 inversion reference, returning encoded positive sRGB f32.
/// This is also the bounded-cancellation fallback for CPU export consumers.
/// A display reduction must follow inversion, quantization and finish; passing
/// an already reduced negative scan would change the nonlinear output order.
pub fn positive_scan_v1(
    scan: &[f32],
    stock: &str,
    stop: Option<&AtomicBool>,
) -> Result<Vec<f32>, CalibratedOutputError> {
    let cal = calibration(NegativeStockV1::try_from(stock)?);
    if scan.len() % 3 != 0 {
        return Err(CalibratedOutputError::InvalidScanLength);
    }
    check_cancelled(stop)?;
    let mut output = vec![0.; scan.len()];
    let workers = if scan.len() >= 3 * 512 * 512 {
        std::thread::available_parallelism().map_or(1, |n| n.get().min(4))
    } else {
        1
    };
    let chunk = (scan.len() / 3).div_ceil(workers).max(1) * 3;
    std::thread::scope(|scope| -> Result<(), CalibratedOutputError> {
        let jobs: Vec<_> = scan
            .chunks(chunk)
            .zip(output.chunks_mut(chunk))
            .map(|(src, dst)| {
                scope.spawn(move || -> Result<(), CalibratedOutputError> {
                    for (src, dst) in src.chunks(4096 * 3).zip(dst.chunks_mut(4096 * 3)) {
                        check_cancelled(stop)?;
                        convert_positive(src, dst, cal)?;
                    }
                    Ok(())
                })
            })
            .collect();
        for job in jobs {
            job.join().expect("negative inversion worker panicked")?;
        }
        Ok(())
    })?;
    check_cancelled(stop)?;
    Ok(output)
}

// Keep the conversion's scalar order aligned with the independent frozen
// reference. No resampling or fitting of calibration nodes is performed.
fn convert_positive(
    scan: &[f32],
    output: &mut [f32],
    cal: &Calibration,
) -> Result<(), CalibratedOutputError> {
    let matrix = [
        [2.5216494298433036, -1.13688855422226, -0.3849175931944449],
        [
            -0.27521355124402597,
            1.3697051510263254,
            -0.09439245077651998,
        ],
        [
            -0.015925010090464285,
            -0.14780636811079967,
            1.1638058159424312,
        ],
    ];
    let toe = 0.18 / 256.;
    for (p, out_pixel) in scan.chunks_exact(3).zip(output.chunks_exact_mut(3)) {
        if !p.iter().all(|v| v.is_finite()) {
            return Err(CalibratedOutputError::NonFiniteScan);
        }
        let mut ap = [0.; 3];
        for c in 0..3 {
            let x = 1. - p[c] as f64 / cal.base[c];
            let q = &cal.curves[c];
            let i = q.absorption.partition_point(|v| *v <= x);
            let v = if i == 0 {
                0.
            } else if i == q.absorption.len() {
                *q.asinh_exposure.last().unwrap()
            } else {
                let t = (x - q.absorption[i - 1]) / (q.absorption[i] - q.absorption[i - 1]);
                q.asinh_exposure[i - 1] * (1. - t) + q.asinh_exposure[i] * t
            };
            ap[c] = toe * v.sinh();
        }
        let mut rgb = matrix.map(|m| m[0] * ap[0] + m[1] * ap[1] + m[2] * ap[2]);
        let y = (rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722).max(0.);
        let t = (y / toe).clamp(0., 1.);
        let confidence = t * t * (3. - 2. * t);
        for c in &mut rgb {
            *c = y + (*c - y) * confidence;
        }
        let out = if y <= 0.75 {
            y
        } else {
            0.75 + 0.25 * (1. - (-(y - 0.75) / 0.25).exp())
        };
        for c in &mut rgb {
            *c *= out / y.max(1e-12);
        }
        let low = rgb.iter().copied().fold(f64::INFINITY, f64::min);
        let high = rgb.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let f = 1_f64
            .min(out / (out - low).max(1e-12))
            .min((1. - out) / (high - out).max(1e-12))
            .max(0.);
        for (dest, c) in out_pixel.iter_mut().zip(rgb) {
            *dest = encode_srgb((out + f * (c - out)) as f32).clamp(0., 1.);
        }
    }
    Ok(())
}

fn encode_srgb(value: f32) -> f32 {
    if value <= 0.0031308 {
        12.92 * value
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}
fn decode_srgb(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

/// Exact original per-code finish mapping. It operates on encoded positive
/// RGB16 after inversion/quantization and before display reduction.
pub fn finish_exposure_table_v1(centi_ev: i32) -> Vec<u16> {
    let gain = 2_f32.powf(centi_ev as f32 / 100.);
    (0..=u16::MAX)
        .map(|v| {
            (encode_srgb(decode_srgb(v as f32 / 65535.) * gain).clamp(0., 1.) * 65535.).round()
                as u16
        })
        .collect()
}

/// Full-size CPU export fallback. No display crop/reduction happens here.
pub fn positive_rgb16_v1(
    scan: &[f32],
    stock: &str,
    finish_centi_ev: i32,
    stop: Option<&AtomicBool>,
) -> Result<Vec<u16>, CalibratedOutputError> {
    let positive = positive_scan_v1(scan, stock, stop)?;
    let finish = (finish_centi_ev != 0).then(|| finish_exposure_table_v1(finish_centi_ev));
    let mut codes = Vec::with_capacity(positive.len());
    for chunk in positive.chunks(65536) {
        check_cancelled(stop)?;
        codes.extend(chunk.iter().map(|value| {
            let code = (*value as f64 * 65535. + 0.5) as u16;
            finish.as_ref().map_or(code, |table| table[code as usize])
        }));
    }
    check_cancelled(stop)?;
    Ok(codes)
}

struct GpuCalibration {
    buffer: wgpu::Buffer,
    offsets: [u32; 4],
    lengths: [u32; 4],
}
struct FinishCache {
    centi_ev: i32,
    buffer: wgpu::Buffer,
}

/// Owns the fixed output pipeline, two immutable calibration buffers and at
/// most one cached finish table. It never creates another device or queue.
/// Each encoded result owns fresh output textures, separate from work planes.
pub struct PhysicalOutputV1 {
    device: wgpu::Device,
    pipeline: wgpu::ComputePipeline,
    calibrations: [GpuCalibration; 2],
    empty_curve: wgpu::Buffer,
    finish: Mutex<FinishCache>,
}

/// The textures belong to the consumer. `encode` returns encoded destinations;
/// they become publishable only after the enclosing render returns `Ok`.
pub struct PhysicalOutputTexturesV1 {
    pub display: wgpu::Texture,
    pub precision: wgpu::Texture,
    pub width: u32,
    pub height: u32,
}

impl PhysicalOutputV1 {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("physical output v1"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../../spektrafilm-shaders/wgsl/physical_output_v1.wgsl").into(),
            ),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("physical output v1"),
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let calibrations = [NegativeStockV1::Pro400H, NegativeStockV1::XTra400].map(|stock| {
            let cal = calibration(stock);
            let mut offsets = [0; 4];
            let mut lengths = [0; 4];
            let mut nodes = Vec::<[f32; 4]>::new();
            for (c, curve) in cal.curves.iter().enumerate() {
                offsets[c] = nodes.len() as u32;
                lengths[c] = curve.absorption.len() as u32;
                for (i, &a) in curve.absorption.iter().enumerate() {
                    // Original residual-preserving scan-space preparation.
                    let scan = cal.base[c] * (1. - a);
                    let high = scan as f32;
                    let inverse = curve
                        .absorption
                        .get(i + 1)
                        .map_or(0., |b| 1. / (cal.base[c] * (b - a)));
                    nodes.push([
                        high,
                        (scan - high as f64) as f32,
                        curve.asinh_exposure[i] as f32,
                        inverse as f32,
                    ]);
                }
            }
            let bytes: Vec<u8> = nodes
                .iter()
                .flatten()
                .flat_map(|v| v.to_ne_bytes())
                .collect();
            GpuCalibration {
                buffer: storage(device, "negative v1 calibration", &bytes),
                offsets,
                lengths,
            }
        });
        Self {
            device: device.clone(),
            pipeline,
            calibrations,
            empty_curve: storage(device, "empty calibration", &[0; 16]),
            finish: Mutex::new(FinishCache {
                centi_ev: 0,
                buffer: storage(device, "identity finish", &[0; 4]),
            }),
        }
    }

    /// Complete a full physical render and its calibrated output on the same
    /// encoder. This wrapper preserves detailed encoding errors while ensuring
    /// they abort the backend's output/density publication commit boundary.
    pub fn render(
        &self,
        backend: &WgpuBackend,
        params: &FilmChainParams<'_>,
        render_options: FilmRenderOptions<'_>,
        output_options: PhysicalOutputOptionsV1,
    ) -> Result<PhysicalOutputTexturesV1, CalibratedOutputError> {
        if matches!(
            output_options.mapping,
            PhysicalOutputMappingV1::NegativeScanAp0(_)
        ) && (!params.scan_film || params.output_cctf_encoding)
        {
            return Err(CalibratedOutputError::InvalidTransport);
        }
        let mut encoding_error = None;
        let output =
            backend.with_film_output(params, render_options, None, |device, encoder, output| {
                self.encode(device, encoder, output, output_options)
                    .map_err(|error| {
                        encoding_error = Some(error);
                        OutputError::GpuFailure
                    })
            });
        if let Some(error) = encoding_error {
            return Err(error);
        }
        output.map_err(CalibratedOutputError::from)
    }

    /// Record inversion -> RGB16 quantization -> finish -> crop/reduction ->
    /// display conversion, preserving the original GPU shader byte-for-byte.
    /// Do not pass a pre-cropped engine output: options.region is post-inversion.
    /// This method records commands only; use `render` for completion/publication.
    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        output: FilmOutput<'_>,
        options: PhysicalOutputOptionsV1,
    ) -> Result<PhysicalOutputTexturesV1, CalibratedOutputError> {
        if device != &self.device {
            return Err(CalibratedOutputError::WrongDevice);
        }
        let negative = matches!(options.mapping, PhysicalOutputMappingV1::NegativeScanAp0(_));
        if output.cctf_encoding == negative {
            return Err(CalibratedOutputError::InvalidTransport);
        }
        let channels = output_channel_count(output.width, output.height)?;
        if output.buffer.size() < channels as u64 * 4
            || !output.buffer.usage().contains(wgpu::BufferUsages::STORAGE)
        {
            return Err(OutputError::InvalidImageData.into());
        }
        let view = options.region.unwrap_or(DisplayRegion {
            crop: [0, 0, output.width, output.height],
            width: output.width,
            height: output.height,
        });
        view.validate(output.width, output.height)?;
        let limits = device.limits();
        if view.width.max(view.height) > limits.max_texture_dimension_2d
            || view.width.div_ceil(16) > limits.max_compute_workgroups_per_dimension
            || view.height.div_ceil(16) > limits.max_compute_workgroups_per_dimension
            || output.buffer.size() > limits.max_storage_buffer_binding_size as u64
        {
            return Err(OutputError::DeviceLimitExceeded.into());
        }
        let (curves, offsets, lengths) = match options.mapping {
            PhysicalOutputMappingV1::EncodedSrgb => (&self.empty_curve, [0; 4], [0; 4]),
            PhysicalOutputMappingV1::NegativeScanAp0(stock) => {
                let cal = &self.calibrations[stock.index()];
                (&cal.buffer, cal.offsets, cal.lengths)
            }
        };
        let finish = {
            let mut cache = self.finish.lock().unwrap();
            if cache.centi_ev != options.finish_centi_ev {
                let words: Vec<u32> = if options.finish_centi_ev == 0 {
                    vec![0]
                } else {
                    finish_exposure_table_v1(options.finish_centi_ev)
                        .into_iter()
                        .map(u32::from)
                        .collect()
                };
                let bytes: Vec<u8> = words.iter().flat_map(|v| v.to_ne_bytes()).collect();
                *cache = FinishCache {
                    centi_ev: options.finish_centi_ev,
                    buffer: storage(device, "legacy finish codes", &bytes),
                };
            }
            cache.buffer.clone()
        };
        let mut words = vec![
            output.width,
            output.height,
            negative as u32,
            (options.finish_centi_ev != 0) as u32,
        ];
        words.extend(view.crop);
        words.extend([view.width, view.height, 0, 0]);
        words.extend(offsets);
        words.extend(lengths);
        let bytes: Vec<u8> = words.iter().flat_map(|v| v.to_ne_bytes()).collect();
        let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("physical output v1 view"),
            contents: &bytes,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let texture = |format| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("physical output v1"),
                size: wgpu::Extent3d {
                    width: view.width,
                    height: view.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let display = texture(wgpu::TextureFormat::Rgba8Unorm);
        let precision = texture(wgpu::TextureFormat::Rgba16Uint);
        let v8 = display.create_view(&Default::default());
        let v16 = precision.create_view(&Default::default());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("physical output v1"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: output.buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: curves.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&v8),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&v16),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: finish.as_entire_binding(),
                },
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("physical output v1"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(view.width.div_ceil(16), view.height.div_ceil(16), 1);
        }
        Ok(PhysicalOutputTexturesV1 {
            display,
            precision,
            width: view.width,
            height: view.height,
        })
    }
}

fn storage(device: &wgpu::Device, label: &str, contents: &[u8]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents,
        usage: wgpu::BufferUsages::STORAGE,
    })
}
