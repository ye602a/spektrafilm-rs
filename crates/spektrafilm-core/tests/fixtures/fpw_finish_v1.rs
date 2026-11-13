pub fn encode_srgb(value: f32) -> f32 {
    if value <= 0.0031308 {
        12.92 * value
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

pub fn decode_srgb(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

pub(crate) fn finish_exposure_table(centi_ev:i32)->Vec<u16> {
    let gain=2_f32.powf(centi_ev as f32/100.);
    (0..=u16::MAX).map(|v|
        (encode_srgb(decode_srgb(v as f32/65535.)*gain).clamp(0.,1.)*65535.).round() as u16).collect()
}
