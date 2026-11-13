//! Invalid host input must fail before metering or invoking a GPU consumer.

use std::cell::Cell;
use std::path::Path;
use std::sync::OnceLock;

use spektrafilm_core::{params::RuntimeParams, pipeline::Pipeline, profile};
use spektrafilm_gpu::{FilmRenderOptions, OutputError, cpu_backend::CpuBackend};
use spektrafilm_math::{image::ImageBuf, precision::from_f32};

fn pipeline() -> &'static Pipeline {
    static PIPELINE: OnceLock<Pipeline> = OnceLock::new();
    PIPELINE.get_or_init(|| {
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let mut params = RuntimeParams::default();
        params.settings.rgb_to_raw_method = "mallett2019".into();
        params.io.input_color_space = "sRGB".into();
        params.camera.auto_exposure = true;
        // An empty luminance grid used to reach the median index calculation
        // before the backend had a chance to report the invalid input.
        params.camera.auto_exposure_method = "median".into();
        Pipeline::new_with_spectral(
            profile::load_profile_by_name(&data, "kodak_portra_400").unwrap(),
            profile::load_profile_by_name(&data, "kodak_portra_endura").unwrap(),
            params,
            &data,
        )
        .unwrap()
    })
}

#[test]
fn invalid_input_is_rejected_before_preparation_and_consumer() {
    let cases = [
        (ImageBuf::new(0, 1), OutputError::EmptyDimensions),
        (ImageBuf::new(1, 0), OutputError::EmptyDimensions),
        (
            ImageBuf {
                width: 1,
                height: 1,
                data: vec![from_f32(0.2); 2],
            },
            OutputError::InvalidImageData,
        ),
        (
            ImageBuf {
                width: u32::MAX,
                height: 2,
                data: Vec::new(),
            },
            OutputError::ChannelCountOverflow,
        ),
    ];
    for (image, expected) in cases {
        let called = Cell::new(false);
        assert_eq!(
            pipeline().try_with_resident_params(&image, |_| called.set(true)),
            Err(expected)
        );
        assert!(!called.get(), "invalid input reached the output consumer");
        assert_eq!(
            pipeline()
                .process_resident_with_options(&image, &CpuBackend, FilmRenderOptions::default())
                .unwrap_err(),
            expected
        );
        assert!(
            pipeline()
                .with_resident_params(&image, |_| called.set(true))
                .is_none()
        );
        assert!(!called.get());
        assert!(pipeline().process_resident_borrowed(&image, &CpuBackend).is_none());
    }
}

#[test]
fn valid_input_reaches_consumer_and_preserves_no_backend_result() {
    let image = ImageBuf::from_data(1, 1, vec![from_f32(0.18); 3]);
    assert_eq!(
        pipeline()
            .try_with_resident_params(&image, |params| {
                assert!(std::ptr::eq(params.image, &image));
                (params.image.width, params.image.height)
            })
            .unwrap(),
        Some((1, 1))
    );
    assert!(
        pipeline()
            .process_resident_with_options(&image, &CpuBackend, FilmRenderOptions::default())
            .unwrap()
            .is_none(),
        "a CPU backend must still report that no resident route exists"
    );
}
