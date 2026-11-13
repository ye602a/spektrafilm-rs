//! Exact pre-print dependencies and publication rules for the developed negative.
//!
//! The host supplies only the identity of the immutable input pixels. The engine
//! classifies its own parameters at the density-after-DIR-and-grain boundary.
//! No GPU work is submitted here; callers publish a snapshot only after their
//! complete render has succeeded, and never mutate a published snapshot.

use crate::wgpu_backend::GrainReference;
use crate::{
    DiffusionGpuPlan, DirCouplersGpuParams, FilmChainParams, FrontPass, GrainGpuParams,
    HalationGpuParams, HighlightBoostGpuParams, LayeredGrainGpuParams, SourceIdentity,
};

/// Cumulative counters for this backend's negative-density cache.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NegativeCacheStats {
    /// Eligible requests that used an already completed negative snapshot.
    pub hits: u64,
    /// Eligible requests that had to develop a new negative.
    pub misses: u64,
    /// Completed snapshots successfully published into the cache.
    pub stores: u64,
    /// Requests without an identity, with caching disabled, or over budget.
    pub bypasses: u64,
    /// Retained snapshot bytes only; excludes all in-flight clones/allocations.
    pub resident_bytes: u64,
}

/// Exact typed bit-pattern sequence, not a lossy hash or a pointer identity.
///
/// Conservative misses for equivalent floats (for example +0 and -0) are fine.
/// This deliberately keeps the prepared LUT/curve values themselves so a hash
/// collision can never reuse another physical negative.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NegativeKey(Vec<u64>);

impl NegativeKey {
    pub(crate) fn new(
        p: &FilmChainParams<'_>,
        source: SourceIdentity,
        reference: Option<&GrainReference>,
    ) -> Self {
        // Do not use `..`: adding a chain parameter must force an explicit
        // decision here about which side of the cache boundary consumes it.
        let FilmChainParams {
            image,
            front,
            film_log_exposure,
            film_density_curves_normalized,
            film_gamma,
            halation,
            dir_couplers,
            grain,
            camera_lens_blur_px,
            highlight_boost,
            diffusion,
            // These are consumed after the negative-density boundary. Even the
            // film dye spectra are first consumed by printing/direct scanning.
            film_channel_density: _,
            film_base_density: _,
            print_illuminant: _,
            print_sensitivity: _,
            print_normalization_factor: _,
            print_log_exposure: _,
            print_density_curves: _,
            print_gamma: _,
            print_channel_density: _,
            print_base_density: _,
            preflash: _,
            viewing_illuminant: _,
            scan_normalization: _,
            scan_xyz_to_rgb: _,
            bw_xyz_remap: _,
            scan_film: _,
            glare: _,
            gamut: _,
            unsharp: _,
            scanner_lens_blur_px: _,
            enlarger_diffusion: _,
            print_exposure_scale: _,
            output_cctf_encoding: _,
        } = p;

        let mut k = KeyWriter(Vec::new());
        k.u64(1); // Internal schema; entries never survive a backend/process.
        k.u64(source.0 as u64);
        k.u64((source.0 >> 64) as u64);
        k.u32(image.width);
        k.u32(image.height);
        k.usize(image.data.len());
        k.usize(std::mem::size_of::<spektrafilm_math::precision::Scalar>());
        match front {
            FrontPass::Hanatos2025 {
                tc_lut,
                rgb_to_adapted_xyz,
            } => {
                let spektrafilm_math::spectral::TcLut {
                    size,
                    channels,
                    data,
                } = *tc_lut;
                k.u32(0);
                k.usize(*size);
                k.usize(*channels);
                k.f64s(data);
                k.f64_rows(rgb_to_adapted_xyz);
            }
            FrontPass::Mallett2019 { matrix } => {
                k.u32(1);
                k.f64_rows(matrix);
            }
        }
        k.f64s(film_log_exposure);
        k.f64_rows(film_density_curves_normalized);
        k.f64(*film_gamma);
        k.optional(halation.as_ref(), KeyWriter::halation);
        k.optional(dir_couplers.as_ref(), KeyWriter::dir);
        k.optional(grain.as_ref(), KeyWriter::grain);
        k.optional(camera_lens_blur_px.as_ref(), |k, &v| k.f32(v));
        k.optional(highlight_boost.as_ref(), KeyWriter::highlight);
        k.optional(diffusion.as_ref(), KeyWriter::diffusion);
        k.optional(reference, |k, r| {
            // GrainReference is a padding-free Pod. Include all coordinate,
            // source-size and enabled fields without duplicating its layout.
            let bytes = bytemuck::bytes_of(r);
            k.usize(bytes.len());
            for word in bytes.chunks_exact(4) {
                k.u32(u32::from_ne_bytes(word.try_into().unwrap()));
            }
        });
        Self(k.0)
    }
}

struct KeyWriter(Vec<u64>);

impl KeyWriter {
    fn u64(&mut self, v: u64) {
        self.0.push(v);
    }
    fn u32(&mut self, v: u32) {
        self.u64(v as u64);
    }
    fn usize(&mut self, v: usize) {
        self.u64(v as u64);
    }
    fn boolean(&mut self, v: bool) {
        self.u64(u64::from(v));
    }
    fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }
    fn f64(&mut self, v: f64) {
        self.u64(v.to_bits());
    }
    fn f32s(&mut self, values: &[f32]) {
        self.usize(values.len());
        for &v in values {
            self.f32(v);
        }
    }
    fn f64s(&mut self, values: &[f64]) {
        self.usize(values.len());
        for &v in values {
            self.f64(v);
        }
    }
    fn f32_rows(&mut self, values: &[[f32; 3]]) {
        self.usize(values.len());
        for row in values {
            self.f32s(row);
        }
    }
    fn f64_rows(&mut self, values: &[[f64; 3]]) {
        self.usize(values.len());
        for row in values {
            self.f64s(row);
        }
    }
    fn optional<T>(&mut self, value: Option<&T>, write: impl FnOnce(&mut Self, &T)) {
        self.boolean(value.is_some());
        if let Some(value) = value {
            write(self, value);
        }
    }
    fn highlight(&mut self, p: &HighlightBoostGpuParams) {
        let HighlightBoostGpuParams {
            boost_ev,
            boost_range,
            protect_ev,
        } = p;
        self.f32(*boost_ev);
        self.f32(*boost_range);
        self.f32(*protect_ev);
    }
    fn diffusion(&mut self, p: &DiffusionGpuPlan) {
        let DiffusionGpuPlan {
            d,
            small_w,
            small_h,
            p_s,
            sigmas,
            coeffs,
        } = p;
        self.u32(*d);
        self.u32(*small_w);
        self.u32(*small_h);
        self.f32(*p_s);
        self.f32s(sigmas);
        self.f32_rows(coeffs);
    }
    fn halation(&mut self, p: &HalationGpuParams) {
        let HalationGpuParams {
            scatter_amount,
            scatter_core_px,
            scatter_tail_px,
            scatter_tail_weight,
            halation_amount,
            halation_strength_avg,
            halation_a_tot,
            halation_first_sigma_px,
            halation_n_bounces,
            halation_bounce_decay,
            halation_renormalize,
        } = p;
        self.f32(*scatter_amount);
        self.f32(*scatter_core_px);
        self.f32(*scatter_tail_px);
        self.f32s(scatter_tail_weight);
        self.f32(*halation_amount);
        self.f32(*halation_strength_avg);
        self.f32s(halation_a_tot);
        self.f32(*halation_first_sigma_px);
        self.u32(*halation_n_bounces);
        self.f32(*halation_bounce_decay);
        self.boolean(*halation_renormalize);
    }
    fn dir(&mut self, p: &DirCouplersGpuParams<'_>) {
        let DirCouplersGpuParams {
            couplers_matrix_scaled,
            density_max,
            is_positive,
            diffusion_size_px,
            diffusion_tail_px,
            diffusion_tail_weight,
            density_curves_0,
            log_exposure,
            gamma_factor,
        } = p;
        self.f32_rows(couplers_matrix_scaled);
        self.f32s(density_max);
        self.boolean(*is_positive);
        self.f32(*diffusion_size_px);
        self.f32(*diffusion_tail_px);
        self.f32(*diffusion_tail_weight);
        self.f64_rows(density_curves_0);
        self.f64s(log_exposure);
        self.f64(*gamma_factor);
    }
    fn grain(&mut self, p: &GrainGpuParams) {
        let GrainGpuParams {
            layers,
            density_min,
            density_max,
            n_particles_per_pixel,
            grain_uniformity,
            n_sub_layers,
            base_seed,
            grain_blur,
            monochrome,
        } = p;
        self.optional(layers.as_ref(), KeyWriter::layers);
        self.f32s(density_min);
        self.f32s(density_max);
        self.f32s(n_particles_per_pixel);
        self.f32s(grain_uniformity);
        self.u32(*n_sub_layers);
        self.u32(*base_seed);
        self.f32(*grain_blur);
        self.boolean(*monochrome);
    }
    fn layers(&mut self, p: &LayeredGrainGpuParams) {
        let LayeredGrainGpuParams {
            output_sampling,
            curve_len,
            positive,
            packed,
            dye_sigma,
            micro_sigma,
            micro_blur,
        } = p;
        self.optional(output_sampling.as_ref(), |k, values| k.f32s(values));
        self.u32(*curve_len);
        self.boolean(*positive);
        self.f32s(packed);
        self.f32_rows(dye_sigma);
        self.f32(*micro_sigma);
        self.f32(*micro_blur);
    }
}

struct Entry<T> {
    key: NegativeKey,
    value: T,
    bytes: u64,
}

/// Permission to publish one completed miss. Dropping it publishes nothing.
pub(crate) struct NegativePublication {
    key: NegativeKey,
    bytes: u64,
    generation: u64,
    ticket: u64,
}

pub(crate) enum NegativeLookup<T> {
    Bypass,
    /// Immutable snapshot. Copy into the render's own work plane before use.
    Hit(T),
    /// Create a private snapshot after grain, then publish only after success.
    Miss(NegativePublication),
}

/// One retained immutable snapshot, with no assumption of a serialized host.
/// Protected by the owning backend's mutex. GPU waits must happen outside it.
pub(crate) struct NegativeCache<T> {
    enabled: bool,
    max_bytes: u64,
    generation: u64,
    latest_ticket: u64,
    entry: Option<Entry<T>>,
    stats: NegativeCacheStats,
}

impl<T> Default for NegativeCache<T> {
    fn default() -> Self {
        Self {
            enabled: false,
            max_bytes: 384 * 1024 * 1024,
            generation: 0,
            latest_ticket: 0,
            entry: None,
            stats: NegativeCacheStats::default(),
        }
    }
}

impl<T: Clone> NegativeCache<T> {
    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn configure(&mut self, enabled: bool) {
        if self.enabled != enabled {
            self.invalidate_pending();
        }
        self.enabled = enabled;
        if !enabled {
            self.entry = None;
        }
    }

    pub(crate) fn set_budget(&mut self, max_bytes: u64) {
        if self.max_bytes != max_bytes {
            self.invalidate_pending();
        }
        self.max_bytes = max_bytes;
        if self
            .entry
            .as_ref()
            .is_some_and(|entry| entry.bytes > max_bytes)
        {
            self.entry = None;
        }
    }

    pub(crate) fn clear(&mut self) {
        self.invalidate_pending();
        self.entry = None;
    }

    fn invalidate_pending(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.latest_ticket = 0;
    }

    pub(crate) fn stats(&self) -> NegativeCacheStats {
        NegativeCacheStats {
            resident_bytes: self.entry.as_ref().map_or(0, |entry| entry.bytes),
            ..self.stats
        }
    }

    pub(crate) fn begin(
        &mut self,
        key: Option<NegativeKey>,
        required_bytes: u64,
    ) -> NegativeLookup<T> {
        if required_bytes == 0 || required_bytes > self.max_bytes {
            // A geometry/budget bypass must also prevent an older in-flight
            // request from silently repopulating the previous residency.
            self.clear();
            self.stats.bypasses = self.stats.bypasses.saturating_add(1);
            return NegativeLookup::Bypass;
        }
        let Some(key) = key.filter(|_| self.enabled) else {
            self.stats.bypasses = self.stats.bypasses.saturating_add(1);
            return NegativeLookup::Bypass;
        };
        match self.latest_ticket.checked_add(1) {
            Some(ticket) => self.latest_ticket = ticket,
            None => {
                self.invalidate_pending();
                self.latest_ticket = 1;
            }
        }
        if let Some(entry) = self.entry.as_ref() {
            if entry.key == key && entry.bytes == required_bytes {
                self.stats.hits = self.stats.hits.saturating_add(1);
                return NegativeLookup::Hit(entry.value.clone());
            }
        }
        self.entry = None;
        self.stats.misses = self.stats.misses.saturating_add(1);
        NegativeLookup::Miss(NegativePublication {
            key,
            bytes: required_bytes,
            generation: self.generation,
            ticket: self.latest_ticket,
        })
    }

    /// Call only after successful completion of the render's GPU work/output.
    /// `value` must be an immutable density snapshot, not a recyclable work plane.
    pub(crate) fn publish(
        &mut self,
        publication: NegativePublication,
        value: T,
        actual_bytes: u64,
    ) -> bool {
        if !self.enabled
            || publication.generation != self.generation
            || publication.ticket != self.latest_ticket
            || publication.bytes != actual_bytes
            || actual_bytes == 0
            || actual_bytes > self.max_bytes
        {
            return false;
        }
        self.entry = Some(Entry {
            key: publication.key,
            value,
            bytes: actual_bytes,
        });
        self.stats.stores = self.stats.stores.saturating_add(1);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GamutGpuParams, GlareGpuParams, UnsharpGpuParams};
    use spektrafilm_math::{image::ImageBuf, spectral::TcLut};

    const CURVE: [[f64; 3]; 2] = [[0.1, 0.2, 0.3], [1.1, 1.2, 1.3]];
    const X: [f64; 2] = [-1.0, 1.0];
    const MATRIX: [[f64; 3]; 3] = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
    const SOURCE: SourceIdentity = SourceIdentity(21);

    fn fixture<'a>(image: &'a ImageBuf, lut: &'a TcLut) -> FilmChainParams<'a> {
        FilmChainParams {
            image,
            front: FrontPass::Hanatos2025 {
                tc_lut: lut,
                rgb_to_adapted_xyz: MATRIX,
            },
            film_log_exposure: &X,
            film_density_curves_normalized: &CURVE,
            film_gamma: 1.,
            film_channel_density: &CURVE,
            film_base_density: &X,
            print_illuminant: &X,
            print_sensitivity: &CURVE,
            print_normalization_factor: 1.,
            print_log_exposure: &X,
            print_density_curves: &CURVE,
            print_gamma: 1.,
            print_channel_density: &CURVE,
            print_base_density: &X,
            preflash: [0.; 3],
            viewing_illuminant: &X,
            scan_normalization: 1.,
            scan_xyz_to_rgb: &MATRIX,
            bw_xyz_remap: None,
            scan_film: false,
            halation: Some(HalationGpuParams {
                scatter_amount: 0.1,
                scatter_core_px: 0.5,
                scatter_tail_px: 1.,
                scatter_tail_weight: [0.1; 3],
                halation_amount: 0.1,
                halation_strength_avg: 0.2,
                halation_a_tot: [0.1; 3],
                halation_first_sigma_px: 1.,
                halation_n_bounces: 2,
                halation_bounce_decay: 0.5,
                halation_renormalize: false,
            }),
            dir_couplers: Some(DirCouplersGpuParams {
                couplers_matrix_scaled: [[0.1; 3]; 3],
                density_max: [1.; 3],
                is_positive: false,
                diffusion_size_px: 1.,
                diffusion_tail_px: 2.,
                diffusion_tail_weight: 0.1,
                density_curves_0: &CURVE,
                log_exposure: &X,
                gamma_factor: 1.,
            }),
            grain: Some(GrainGpuParams {
                layers: Some(LayeredGrainGpuParams {
                    output_sampling: Some([1., 1.]),
                    curve_len: 2,
                    positive: false,
                    packed: vec![0.1, 0.2],
                    dye_sigma: [[0.1; 3]; 3],
                    micro_sigma: 0.1,
                    micro_blur: 0.2,
                }),
                density_min: [0.1; 3],
                density_max: [1.; 3],
                n_particles_per_pixel: [50.; 3],
                grain_uniformity: [0.5; 3],
                n_sub_layers: 1,
                base_seed: 42,
                grain_blur: 0.6,
                monochrome: false,
            }),
            glare: None,
            gamut: None,
            unsharp: None,
            camera_lens_blur_px: Some(0.5),
            scanner_lens_blur_px: None,
            highlight_boost: Some(HighlightBoostGpuParams {
                boost_ev: 1.,
                boost_range: 2.,
                protect_ev: 0.1,
            }),
            diffusion: Some(DiffusionGpuPlan {
                d: 2,
                small_w: 1,
                small_h: 1,
                p_s: 0.1,
                sigmas: vec![1.],
                coeffs: vec![[0.1; 3]],
            }),
            enlarger_diffusion: None,
            print_exposure_scale: 1.,
            output_cctf_encoding: true,
        }
    }

    #[test]
    fn negative_key_tracks_each_preprint_dependency_family() {
        let image = ImageBuf::new(2, 2);
        let lut = TcLut {
            size: 1,
            channels: 3,
            data: vec![0.1, 0.2, 0.3],
        };
        let baseline = NegativeKey::new(&fixture(&image, &lut), SOURCE, None);
        macro_rules! changed {
            ($p:ident, $change:expr) => {{
                let mut $p = fixture(&image, &lut);
                $change;
                assert_ne!(
                    baseline,
                    NegativeKey::new(&$p, SOURCE, None),
                    "{}",
                    stringify!($change)
                );
            }};
        }
        changed!(p, p.front = FrontPass::Mallett2019 { matrix: MATRIX });
        changed!(
            p,
            p.front = FrontPass::Hanatos2025 {
                tc_lut: &lut,
                rgb_to_adapted_xyz: [[2.; 3]; 3]
            }
        );
        changed!(p, p.film_log_exposure = &[0., 1.]);
        changed!(p, p.film_density_curves_normalized = &[[0.; 3], [2.; 3]]);
        changed!(p, p.film_gamma = 1.1);
        changed!(p, p.halation = None);
        changed!(p, p.halation.as_mut().unwrap().scatter_tail_weight[1] = 0.2);
        changed!(p, p.halation.as_mut().unwrap().halation_n_bounces = 3);
        changed!(p, p.halation.as_mut().unwrap().halation_renormalize = true);
        changed!(p, p.dir_couplers = None);
        changed!(
            p,
            p.dir_couplers.as_mut().unwrap().couplers_matrix_scaled[0][1] = 0.2
        );
        changed!(
            p,
            p.dir_couplers.as_mut().unwrap().density_curves_0 = &[[0.; 3], [2.; 3]]
        );
        changed!(p, p.dir_couplers.as_mut().unwrap().gamma_factor = 1.1);
        changed!(p, p.grain = None);
        changed!(p, p.grain.as_mut().unwrap().layers = None);
        changed!(p, p.grain.as_mut().unwrap().base_seed = 43);
        changed!(p, p.grain.as_mut().unwrap().monochrome = true);
        changed!(p, p.grain.as_mut().unwrap().n_particles_per_pixel[2] = 51.);
        changed!(p, p.grain.as_mut().unwrap().grain_uniformity[0] = 0.6);
        changed!(p, p.grain.as_mut().unwrap().n_sub_layers = 3);
        changed!(
            p,
            p.grain.as_mut().unwrap().layers.as_mut().unwrap().packed[1] = 0.3
        );
        changed!(
            p,
            p.grain.as_mut().unwrap().layers.as_mut().unwrap().dye_sigma[2][1] = 0.3
        );
        changed!(
            p,
            p.grain
                .as_mut()
                .unwrap()
                .layers
                .as_mut()
                .unwrap()
                .micro_blur = 0.3
        );
        changed!(
            p,
            p.grain
                .as_mut()
                .unwrap()
                .layers
                .as_mut()
                .unwrap()
                .output_sampling = Some([2., 1.])
        );
        changed!(p, p.camera_lens_blur_px = Some(0.6));
        changed!(p, p.highlight_boost.as_mut().unwrap().protect_ev = 0.2);
        changed!(p, p.diffusion.as_mut().unwrap().coeffs[0][2] = 0.2);
        changed!(p, p.diffusion.as_mut().unwrap().sigmas[0] = 2.);
        changed!(p, p.diffusion.as_mut().unwrap().d = 3);
        changed!(p, p.diffusion.as_mut().unwrap().small_w = 2);
        assert_ne!(
            baseline,
            NegativeKey::new(&fixture(&image, &lut), SourceIdentity(22), None)
        );
        let resized = ImageBuf::new(1, 4); // Same byte count, different geometry.
        assert_ne!(
            baseline,
            NegativeKey::new(&fixture(&resized, &lut), SOURCE, None)
        );
        let mut other_lut = lut.clone();
        other_lut.data[1] += 0.1;
        assert_ne!(
            baseline,
            NegativeKey::new(&fixture(&image, &other_lut), SOURCE, None)
        );
        let reference = GrainReference::new([1., 0., 1., 0., 1., 0.], [4, 4], 2);
        let referenced = NegativeKey::new(&fixture(&image, &lut), SOURCE, Some(&reference));
        assert_ne!(baseline, referenced);
        let moved = GrainReference::new([1., 0., 2., 0., 1., 0.], [4, 4], 2);
        assert_ne!(
            referenced,
            NegativeKey::new(&fixture(&image, &lut), SOURCE, Some(&moved))
        );
    }

    #[test]
    fn negative_key_excludes_every_post_boundary_parameter() {
        let image = ImageBuf::new(2, 2);
        let lut = TcLut {
            size: 1,
            channels: 3,
            data: vec![0.1, 0.2, 0.3],
        };
        let baseline = NegativeKey::new(&fixture(&image, &lut), SOURCE, None);
        macro_rules! unchanged {
            ($p:ident, $change:expr) => {{
                let mut $p = fixture(&image, &lut);
                $change;
                assert_eq!(
                    baseline,
                    NegativeKey::new(&$p, SOURCE, None),
                    "{}",
                    stringify!($change)
                );
            }};
        }
        unchanged!(p, p.film_channel_density = &[[2.; 3]]);
        unchanged!(p, p.film_base_density = &[2.]);
        unchanged!(p, p.print_illuminant = &[2.]);
        unchanged!(p, p.print_sensitivity = &[[2.; 3]]);
        unchanged!(p, p.print_normalization_factor = 2.);
        unchanged!(p, p.print_log_exposure = &[2.]);
        unchanged!(p, p.print_density_curves = &[[2.; 3]]);
        unchanged!(p, p.print_gamma = 2.);
        unchanged!(p, p.print_channel_density = &[[2.; 3]]);
        unchanged!(p, p.print_base_density = &[2.]);
        unchanged!(p, p.preflash = [0.1; 3]);
        unchanged!(p, p.viewing_illuminant = &[2.]);
        unchanged!(p, p.scan_normalization = 2.);
        unchanged!(p, p.scan_xyz_to_rgb = &[[2.; 3]; 3]);
        unchanged!(p, p.bw_xyz_remap = Some((0.5, 0.1)));
        unchanged!(p, p.scan_film = true);
        unchanged!(
            p,
            p.glare = Some(GlareGpuParams {
                mu: 0.,
                sigma: 1.,
                blur_px: 1.,
                base_seed: 7,
                rgb_offset: [0.1; 3]
            })
        );
        unchanged!(
            p,
            p.gamut = Some(GamutGpuParams {
                mode: 0,
                knee: [0.1; 3],
                lightness: None,
                l_white: 1.,
                l_min: 0.,
                l_max: 1.,
                cmax: &[],
                n_l: 0,
                n_h: 0
            })
        );
        unchanged!(
            p,
            p.unsharp = Some(UnsharpGpuParams {
                sigma_px: 1.,
                amount: 0.2
            })
        );
        unchanged!(p, p.scanner_lens_blur_px = Some(1.));
        unchanged!(p, p.enlarger_diffusion = p.diffusion.clone());
        unchanged!(p, p.print_exposure_scale = 2.);
        unchanged!(p, p.output_cctf_encoding = false);
    }

    fn key(n: u64) -> NegativeKey {
        NegativeKey(vec![n])
    }
    fn miss(cache: &mut NegativeCache<u64>, n: u64, bytes: u64) -> NegativePublication {
        match cache.begin(Some(key(n)), bytes) {
            NegativeLookup::Miss(publication) => publication,
            _ => panic!("expected a miss"),
        }
    }

    #[test]
    fn negative_cache_is_opt_in_and_dropped_requests_never_publish() {
        let mut cache = NegativeCache::<u64>::default();
        assert!(!cache.enabled());
        assert!(matches!(
            cache.begin(Some(key(1)), 24),
            NegativeLookup::Bypass
        ));
        cache.configure(true);
        assert!(matches!(cache.begin(None, 24), NegativeLookup::Bypass));
        drop(miss(&mut cache, 1, 24)); // Failed/cancelled render.
        assert_eq!(cache.stats().resident_bytes, 0);
        let publication = miss(&mut cache, 1, 24);
        assert!(cache.publish(publication, 99, 24));
        assert!(matches!(
            cache.begin(Some(key(1)), 24),
            NegativeLookup::Hit(99)
        ));
        assert_eq!(
            cache.stats(),
            NegativeCacheStats {
                hits: 1,
                misses: 2,
                stores: 1,
                bypasses: 2,
                resident_bytes: 24,
            }
        );
    }

    #[test]
    fn negative_cache_rejects_late_or_reconfigured_publication() {
        let mut cache = NegativeCache::<u64>::default();
        cache.configure(true);
        let older = miss(&mut cache, 1, 24);
        let newer = miss(&mut cache, 2, 24);
        assert!(cache.publish(newer, 2, 24));
        assert!(!cache.publish(older, 1, 24));
        assert!(matches!(
            cache.begin(Some(key(2)), 24),
            NegativeLookup::Hit(2)
        ));
        let pending = miss(&mut cache, 3, 24);
        cache.configure(false);
        cache.configure(true);
        assert!(!cache.publish(pending, 3, 24));
        assert_eq!(cache.stats().resident_bytes, 0);
        let pending = miss(&mut cache, 4, 24);
        cache.set_budget(48);
        assert!(!cache.publish(pending, 4, 24));
        let pending = miss(&mut cache, 5, 24);
        cache.clear();
        assert!(!cache.publish(pending, 5, 24));
    }

    #[test]
    fn negative_cache_enforces_geometry_bytes_and_residency_budget() {
        let mut cache = NegativeCache::<u64>::default();
        cache.configure(true);
        cache.set_budget(48);
        let first = miss(&mut cache, 1, 24);
        assert!(
            !cache.publish(first, 1, 48),
            "wrong buffer size must not publish"
        );
        let first = miss(&mut cache, 1, 24);
        assert!(cache.publish(first, 1, 24));
        let resized = miss(&mut cache, 1, 48);
        assert_eq!(cache.stats().resident_bytes, 0);
        assert!(cache.publish(resized, 2, 48));
        cache.set_budget(24);
        assert_eq!(cache.stats().resident_bytes, 0);
        let pending = miss(&mut cache, 2, 24);
        assert!(matches!(
            cache.begin(Some(key(3)), 72),
            NegativeLookup::Bypass
        ));
        assert!(
            !cache.publish(pending, 2, 24),
            "over-budget geometry invalidates pending"
        );
        cache.set_budget(0);
        assert!(matches!(
            cache.begin(Some(key(4)), 24),
            NegativeLookup::Bypass
        ));
        assert_eq!(cache.stats().resident_bytes, 0);
    }
}
