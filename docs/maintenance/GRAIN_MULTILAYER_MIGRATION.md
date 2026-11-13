# Physical multilayer GPU grain

The opt-in model includes Philox/Poisson thinning, layer density curves, dye
clouds, microstructure, source-coordinate sampling and precise recursive IIR.

## Source attribution

The migration uses the FPW-vendored engine snapshot
`ae3428acd98e39c48784a79dc8f3369ab1236cb1`, including the GPL-3.0-attributed
SpektraLab algorithm at `99c2d5c9883be485085895157279cc7c4eaa8db6`.
[PR #6](https://github.com/ye602a/spektrafilm-rs/pull/6) introduced it as
`c487cb3a724ae7f6c05d8eac45c86d3394859eda`.

Source comparison at RS `e57d0d1a33dadb2fd647201f49e497c14c1d7a9e` confirmed
identical Git blobs for `grain_layers.rs`, `layered_grain.rs`,
`grain_layers.wgsl`, `grain_iir.comp` and `grain_iir.spv`. The original SPIR-V
blob is `e23544d20d7f402a70b7005fb7531721793870e3`. The Rust IIR wrapper only
added an ignore annotation and qualification comment to its long test; the
source-coordinate helper only changed its introductory comment.

## Compatibility and device requirements

`GrainParams.gpu_multilayer` defaults to `false`, independently of
`sublayers_active`. Existing standalone GPU output keeps its simple sampler
until a caller enables the multilayer route. Core prepares
`LayeredGrainGpuParams` for enabled color profiles;
`GrainGpuParams.layers = None` selects the original simple path.
`output_sampling` defaults to `None`.

Wide-sigma grain (at least 3 pixels) uses the original NoContraction Vulkan
SPIR-V path. `WgpuBackend::device_descriptor` requests `PASSTHROUGH_SHADERS`
on capable Vulkan adapters. An injected device must request the same feature
when using wide IIR. Unsupported backends must not silently substitute a
visually different approximation. Narrow FIR and the simple sampler retain
their existing behavior.

## Verification

Software Vulkan tests exercise repeated state use, accumulator clearing, changed
seeds, source transforms, sampling controls, dye clouds, microstructure and mixed
FIR/IIR channels. The multilayer regression measured mean `1.5006471944243458`
and variance `0.020058962077850902` against analytic variance
`0.01997501919166667`.

The long IIR test is ignored during ordinary test runs and must be invoked
explicitly. All 1,215 cases passed in
[CI](https://github.com/ye602a/spektrafilm-rs/actions/runs/43434521405), with
maximum absolute error `1.1920928955078125e-7`. The tolerance is
`4*f32::EPSILON*max(1,abs(reference))` for each sample.

```sh
SPEKTRAFILM_REQUIRE_GPU=1 cargo test --locked -p spektrafilm-gpu --test grain_source
SPEKTRAFILM_REQUIRE_GPU=1 cargo test --locked -p spektrafilm-gpu --lib actual_gpu_grain_follows_source_coordinates
SPEKTRAFILM_REQUIRE_GPU=1 cargo test --locked -p spektrafilm-gpu --lib actual_gpu_multilayer_moments_and_controls
SPEKTRAFILM_REQUIRE_GPU=1 cargo test --locked -p spektrafilm-gpu --lib actual_gpu_iir_matches_f64_reference -- --ignored --nocapture
```

These are software-Vulkan numerical results. Target-device qualification and
host image acceptance remain separate; see [VALIDATION.md](VALIDATION.md) and
the [compatibility plan](FPW_PARITY_MIGRATION_PLAN.md).
