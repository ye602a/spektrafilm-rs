# RS physical multilayer GPU grain migration

Date: 2026-11-13
Status: ported to independent RS candidate branch, awaiting standalone CI and frozen FPW image parity before host cutover.

## Source and scope

The original FPW-vendored engine (FPW main ae3428acd98e39c48784a79dc8f3369ab1236cb1) is the reference for this migration, including GPL-3.0-attributed SpektraLab algorithm 99c2d5c9883be485085895157279cc7c4eaa8db6. The staged port includes GPU Philox/Poisson thinning, layer density curves, dye clouds, microstructure, source-coordinate sampling and the verified original NoContraction Vulkan SPIR-V recursive IIR. No FPW application, private photo asset or upstream data/model resource is moved.

## Opt-in compatibility

`GrainParams.gpu_multilayer` defaults to `false`, independent of the existing `sublayers_active` field. Existing RS standalone GPU output uses the original sampler until a client explicitly enables the new route. On enabled color profiles only, the core prepares `LayeredGrainGpuParams`; `GrainGpuParams.layers = None` always selects the original simple path. Optional `output_sampling` defaults to `None`.

With wide-sigma grain (>=3 pixels), the original precise Vulkan SPIR-V path must be used. `WgpuBackend::device_descriptor` requests `PASSTHROUGH_SHADERS` on capable Vulkan adapters. Host-injected devices must request the same feature when using wide IIR; unsupported Metal/DX12 devices must not silently switch to a visually different approximation. Narrow FIR and the pre-existing simple sampler continue unchanged.

## Regression requirements

```sh
cargo check --locked -p spektrafilm-core -p spektrafilm-gpu --all-targets
cargo test --locked -p spektrafilm-gpu --test grain_source -- --nocapture
cargo test --locked -p spektrafilm-gpu --lib actual_gpu_grain_follows_source_coordinates -- --nocapture
cargo test --locked -p spektrafilm-gpu --lib actual_gpu_multilayer_moments_and_controls -- --nocapture
cargo test --locked -p spektrafilm-gpu --lib actual_gpu_iir_matches_f64_reference -- --ignored --nocapture
```

FPW's prior RTX 3060 Ti reference tests (2026-10-21) found multilayer mean 1.500647 (expected 1.5), variance 0.020059 (analytic 0.019975), and 1215 IIR reference cases at maximum absolute error 1.1920928955078125e-7. These are previous FPW measurements, **not independent RS results**.

Before any FPW pin change: independently run frozen six 4032x6048 RAW-based linear-input/equal-EV images (including DSC_5930), compare actual pixels to the FPW accepted baseline, verify deterministic source-crop/rotation behavior, dye-cloud scratch clearing across images, seed controls, IIR precision, cancellation and resource ownership. FPW main and wide-gamut research remain untouched by this migration.
