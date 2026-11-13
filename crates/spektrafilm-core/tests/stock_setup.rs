//! Constructor parity with the stock-setup statements extracted from FPW
//! ae3428acd98e39c48784a79dc8f3369ab1236cb1, fpw-runtime/src/film.rs.
//! This does not claim a complete FPW recipe mapping or six-photo acceptance.

use std::path::{Path, PathBuf};

use spektrafilm_core::{
    params::RuntimeParams,
    pipeline::Pipeline,
    profile::{self, Profile},
    stock_setup::{AcceptedStockSetup, FPW_STOCK_SETUP_V1_ID, StockRoute},
};

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

fn profiles(stock: &str) -> (Profile, Profile) {
    (
        profile::load_profile_by_name(&data_dir(), stock).unwrap(),
        profile::load_profile_by_name(&data_dir(), "kodak_portra_endura").unwrap(),
    )
}

fn parameters(route: StockRoute) -> RuntimeParams {
    let mut params = RuntimeParams::default();
    params.camera.auto_exposure = false;
    params.camera.exposure_compensation_ev = 0.37;
    params.enlarger.print_exposure_compensation = false;
    params.enlarger.print_exposure = 1.11;
    params.io.input_color_space = "sRGB".into();
    params.io.input_cctf_decoding = false;
    params.io.scan_film = route != StockRoute::Paper;
    params.film_render.grain.gpu_multilayer = true;
    params.film_render.grain.output_sampling = Some([1.5, 0.7]);
    params
}

// Frozen adapter statements, intentionally independent of stock_setup.rs.
// Preserve their original pre-constructor / post-constructor placement.
fn pinned_adapter(
    stock: &str,
    route: StockRoute,
    film: Profile,
    paper: Profile,
    mut params: RuntimeParams,
) -> Pipeline {
    assert_eq!(film.is_positive(), route == StockRoute::Slide);
    let sigma = if film.info.usage == "cine" {
        50.0
    } else {
        65.0
    };
    params.film_render.halation.halation_first_sigma_um = [sigma; 3];
    params.film_render.halation.halation_strength = match film.info.antihalation.as_str() {
        "strong" => [0.015, 0.005, 0.0],
        "weak" => [0.08, 0.02, 0.0],
        "no" => [0.30, 0.10, 0.015],
        _ => params.film_render.halation.halation_strength,
    };
    let mut pipeline = Pipeline::new_with_spectral(film, paper, params, &data_dir()).unwrap();
    if route == StockRoute::Slide {
        let scale = match stock {
            "fujifilm_provia_100f" => 1.3,
            "fujifilm_velvia_100" => 0.9,
            _ => panic!("unaccepted slide in frozen reference"),
        };
        let dir = &mut pipeline.params.film_render.dir_couplers;
        dir.gamma_samelayer_rgb = [0.12 * scale, 0.08 * scale, 0.06 * scale];
        dir.gamma_interlayer_r_to_gb = [0.12 * scale, 0.06 * scale];
        dir.gamma_interlayer_g_to_rb = [0.08 * scale, 0.06 * scale];
        dir.gamma_interlayer_b_to_rg = [0.06 * scale, 0.06 * scale];
    }
    pipeline
}

fn assert_calibration_equal(actual: &Pipeline, expected: &Pipeline) {
    assert_eq!(
        actual.print_exposure_factor().to_bits(),
        expected.print_exposure_factor().to_bits(),
        "the stock wrapper must preserve the original calibration ordering"
    );
    assert_eq!(
        actual.print_illuminant_slice(),
        expected.print_illuminant_slice()
    );
    assert_eq!(actual.preflash_raw(), expected.preflash_raw());
    let actual_lut = actual.tc_lut().expect("actual Hanatos calibration");
    let expected_lut = expected.tc_lut().expect("reference Hanatos calibration");
    assert_eq!(actual_lut.size, expected_lut.size);
    assert_eq!(
        actual_lut
            .data
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>(),
        expected_lut
            .data
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>()
    );
}

#[test]
fn accepted_stock_constructor_matches_the_extracted_adapter_and_calibration() {
    for (stock, route) in [
        ("kodak_portra_400", StockRoute::Paper),
        ("kodak_gold_200", StockRoute::Paper),
        ("kodak_vision3_500t", StockRoute::Paper),
        ("kodak_doublex", StockRoute::Paper),
        ("fujifilm_provia_100f", StockRoute::Slide),
        ("fujifilm_velvia_100", StockRoute::Slide),
        ("kodak_portra_400", StockRoute::NegativeScan),
    ] {
        let (film, paper) = profiles(stock);
        let params = parameters(route);
        let expected = pinned_adapter(stock, route, film.clone(), paper.clone(), params.clone());
        let selection = AcceptedStockSetup::FpwV1(route);
        let actual = Pipeline::new_with_spectral_and_stock_setup(
            film,
            paper,
            params,
            &data_dir(),
            selection,
        )
        .unwrap();
        assert_eq!(actual.accepted_stock_setup(), Some(selection));
        assert_eq!(selection.id(), FPW_STOCK_SETUP_V1_ID);
        assert_eq!(
            serde_json::to_value(&actual.params).unwrap(),
            serde_json::to_value(&expected.params).unwrap(),
            "accepted stock setup differed from pinned FPW for {stock} / {route:?}"
        );
        assert_calibration_equal(&actual, &expected);
    }
}

#[test]
fn ordinary_constructors_and_defaults_do_not_select_fpw_stock_setup() {
    let params = RuntimeParams::default();
    assert!(!params.film_render.grain.gpu_multilayer);
    let (film, paper) = profiles("kodak_vision3_500t");
    let plain = Pipeline::new(film.clone(), paper.clone(), params.clone());
    assert_eq!(plain.accepted_stock_setup(), None);
    assert_eq!(
        serde_json::to_value(&plain.params).unwrap(),
        serde_json::to_value(&params).unwrap()
    );
    let spectral = Pipeline::new_with_spectral(film, paper, params, &data_dir()).unwrap();
    assert_eq!(spectral.accepted_stock_setup(), None);
    assert_eq!(
        spectral.params.film_render.halation.halation_first_sigma_um,
        [65.0; 3]
    );
    assert_eq!(
        spectral.params.film_render.halation.halation_strength,
        [0.05, 0.015, 0.0]
    );
    let (slide, paper) = profiles("fujifilm_provia_100f");
    let legacy =
        Pipeline::new_with_spectral(slide, paper, parameters(StockRoute::Slide), &data_dir())
            .unwrap();
    assert_eq!(legacy.accepted_stock_setup(), None);
    assert_eq!(
        legacy.params.film_render.dir_couplers.gamma_samelayer_rgb,
        [0.12, 0.08, 0.06]
    );
}

#[test]
fn runtime_refresh_keeps_accepted_slide_controls_and_bw_broadcast() {
    for (stock, route) in [
        ("fujifilm_provia_100f", StockRoute::Slide),
        ("fujifilm_velvia_100", StockRoute::Slide),
        ("kodak_doublex", StockRoute::Paper),
    ] {
        let (film, paper) = profiles(stock);
        let original = parameters(route);
        let selection = AcceptedStockSetup::FpwV1(route);
        let pipeline = Pipeline::new_with_spectral_and_stock_setup(
            film.clone(),
            paper.clone(),
            original.clone(),
            &data_dir(),
            selection,
        )
        .unwrap();
        let factor = pipeline.print_exposure_factor().to_bits();
        // Retain the constructor's resolved neutral filters and all other
        // calibration inputs; only runtime-safe controls change on reuse.
        let mut changed = pipeline.params.clone();
        changed.camera.exposure_compensation_ev = -0.2;
        changed.enlarger.print_exposure = 0.8;
        changed.scanner.unsharp_mask = [0.4, 0.5];
        let refreshed = pipeline.with_params(changed.clone());
        let expected = pinned_adapter(stock, route, film, paper, changed);
        assert_eq!(refreshed.accepted_stock_setup(), Some(selection));
        assert_eq!(refreshed.print_exposure_factor().to_bits(), factor);
        assert_eq!(
            serde_json::to_value(&refreshed.params).unwrap(),
            serde_json::to_value(&expected.params).unwrap()
        );
        if stock == "kodak_doublex" {
            assert_eq!(
                refreshed.params.film_render.halation.halation_strength,
                [0.015; 3]
            );
        }
    }
}

#[test]
fn unsupported_stock_and_route_fail_before_spectral_resource_loading() {
    let missing_data = data_dir().join("not-a-resource-directory");
    for stock in ["kodak_ektachrome_100", "kodak_kodachrome_64", "kodak_trix"] {
        let (film, paper) = profiles(stock);
        let result = Pipeline::new_with_spectral_and_stock_setup(
            film,
            paper,
            parameters(StockRoute::Slide),
            &missing_data,
            AcceptedStockSetup::FpwV1(StockRoute::Slide),
        );
        assert!(result.err().unwrap().contains("FPW v1 slide controls"));
    }
    let (film, paper) = profiles("kodak_portra_400");
    let result = Pipeline::new_with_spectral_and_stock_setup(
        film,
        paper,
        parameters(StockRoute::Slide),
        &missing_data,
        AcceptedStockSetup::FpwV1(StockRoute::Slide),
    );
    assert!(result.err().unwrap().contains("does not match route"));
}
