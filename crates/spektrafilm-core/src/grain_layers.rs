//! Small host-side curve preparation for the GPU multilayer model.
//! Image-sized sampling, dye clouds, clumping and blur remain on the GPU.
use crate::{params::GrainParams, profile::Profile};

pub(super) fn prepare(
    film: &Profile,
    g: &GrainParams,
    pixel_um: f32,
) -> spektrafilm_gpu::LayeredGrainGpuParams {
    let curves = &film.data.density_curves;
    let layers = &film.data.density_curves_layers;
    let k = curves.len();
    assert!(
        k >= 2
            && layers.len() == k
            && curves.iter().all(|r| r.len() == 3)
            && layers
                .iter()
                .all(|r| r.len() == 3 && r.iter().all(|c| c.len() == 3)),
        "GPU multilayer grain requires K x 3 curves and K x 3 x 3 layer curves"
    );
    let positive = film.is_positive();
    let mut minimum = [f64::INFINITY; 3];
    let mut maxima = [[0.0f64; 3]; 3];
    for i in 0..k {
        for c in 0..3 {
            minimum[c] = minimum[c].min(curves[i][c]);
            for sl in 0..3 {
                maxima[sl][c] = maxima[sl][c].max(layers[i][sl][c]);
            }
        }
    }
    let mut axis = vec![0.0f64; k * 3];
    let mut packed = Vec::with_capacity(k * 15 + 36);
    for i in 0..k {
        for c in 0..3 {
            axis[i * 3 + c] = (curves[i][c] - minimum[c]) * if positive { -1.0 } else { 1.0 };
        }
    }
    packed.extend(axis.iter().map(|x| *x as f32));
    for i in 0..k {
        for c in 0..3 {
            let delta = if i + 1 < k {
                axis[(i + 1) * 3 + c] - axis[i * 3 + c]
            } else {
                0.0
            };
            packed.push(if delta != 0.0 {
                (1.0 / delta) as f32
            } else {
                0.0
            });
        }
    }
    for row in layers {
        for c in 0..3 {
            for layer in row {
                packed.push(layer[c] as f32);
            }
        }
    }
    let mut dye_sigma = [[0.0f32; 3]; 3];
    for c in 0..3 {
        let total: f64 = maxima.iter().map(|r| r[c]).sum();
        assert!(total > 0.0, "empty multilayer density curves");
        for sl in 0..3 {
            let fraction = maxima[sl][c] / total;
            let dmin = fraction * g.density_min[c];
            let dmax = maxima[sl][c] + dmin;
            let area =
                g.agx_particle_area_um2 * g.agx_particle_scale[c] * g.agx_particle_scale_layers[sl];
            assert!(
                area > 0.0 && fraction > 0.0,
                "invalid multilayer particle area or layer fraction"
            );
            let n = (pixel_um as f64).powi(2) * fraction / area;
            packed.extend([dmin as f32, dmax as f32, n as f32, g.uniformity[c] as f32]);
            dye_sigma[sl][c] = (g.blur_dye_clouds_um as f64 * (dmax / n).sqrt()) as f32;
        }
    }
    spektrafilm_gpu::LayeredGrainGpuParams {
        output_sampling: g.output_sampling,
        curve_len: k as u32,
        positive,
        packed,
        dye_sigma,
        micro_sigma: g.micro_structure[1] * 0.001 / pixel_um,
        micro_blur: g.micro_structure[0] / pixel_um,
    }
}
