//! Frozen AP0 negative inversion. Calibration is stock and response specific.
use anyhow::{Result, ensure};
use serde::Deserialize;
#[derive(Deserialize)]
pub(crate) struct Curve {
    pub(crate) absorption: Vec<f64>,
    pub(crate) asinh_exposure: Vec<f64>,
}
#[derive(Deserialize)]
pub(crate) struct Calibration {
    pub(crate) base: [f64; 3],
    pub(crate) curves: Vec<Curve>,
}
pub(crate) fn calibration(stock:&str)->Result<&'static Calibration> {
    use std::sync::OnceLock;
    static PRO:OnceLock<Calibration>=OnceLock::new();
    static XTRA:OnceLock<Calibration>=OnceLock::new();
    let (slot,text)=match stock {
        "fujifilm_pro_400h"=>(&PRO,include_str!("../../../../data/negative_scan_ap0_v1/pro400h.json")),
        "fujifilm_xtra_400"=>(&XTRA,include_str!("../../../../data/negative_scan_ap0_v1/xtra.json")),
        _=>anyhow::bail!("该胶片没有匹配的无纸正像标定"),
    };
    Ok(slot.get_or_init(||serde_json::from_str(text).expect("bundled negative calibration")))
}
pub fn positive(scan:&[f32],stock:&str)->Result<Vec<f32>> {positive_with_cancel(scan,stock,None)}
pub(crate) fn positive_with_cancel(scan:&[f32],stock:&str,stop:Option<&std::sync::atomic::AtomicBool>)->Result<Vec<f32>> {
    let cal=calibration(stock)?;
    ensure!(scan.len()%3==0,"negative scan must contain RGB pixels");
    let check=||->Result<()> {ensure!(!stop.is_some_and(|s|s.load(std::sync::atomic::Ordering::Relaxed)),"Film cancelled");Ok(())};
    check()?;
    let mut output=vec![0.;scan.len()];
    let workers=if scan.len()>=3*512*512 {std::thread::available_parallelism().map_or(1,|n|n.get().min(4))}else{1};
    let chunk=(scan.len()/3).div_ceil(workers).max(1)*3;
    std::thread::scope(|scope|->Result<()> {
        let jobs:Vec<_>=scan.chunks(chunk).zip(output.chunks_mut(chunk)).map(|(src,dst)|scope.spawn(move||->Result<()> {
            for (src,dst) in src.chunks(4096*3).zip(dst.chunks_mut(4096*3)) {check()?;convert(src,dst,cal)?;}Ok(())
        })).collect();
        for job in jobs {job.join().expect("negative inversion worker panicked")?;}Ok(())
    })?;
    check()?;Ok(output)
}
fn convert(scan:&[f32],output:&mut [f32],cal:&Calibration)->Result<()> {
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
    for (p,out_pixel) in scan.chunks_exact(3).zip(output.chunks_exact_mut(3)) {
        ensure!(p.iter().all(|v| v.is_finite()), "nonfinite scan");
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
        // Frozen display shoulder followed by symmetric gamut mapping.
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
        for (dest,c) in out_pixel.iter_mut().zip(rgb) {
            *dest=crate::images::encode_srgb((out + f * (c - out)) as f32).clamp(0., 1.);
        }
    }
    Ok(())
}
