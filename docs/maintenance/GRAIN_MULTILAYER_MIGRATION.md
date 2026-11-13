# RS physical multilayer GPU grain migration

Date: 2026-11-13
Status: merged into independent RS main by [PR #6](https://github.com/ye602a/spektrafilm-rs/pull/6), commit `c487cb3a724ae7f6c05d8eac45c86d3394859eda`. Windows compile/CPU, Linux grain regressions and all 1,215 IIR precision cases on software Vulkan have passed; dedicated hardware qualification and frozen FPW image parity remain open.

## Source and scope

The original FPW-vendored engine (FPW main ae3428acd98e39c48784a79dc8f3369ab1236cb1) is the reference for this migration, including GPL-3.0-attributed SpektraLab algorithm 99c2d5c9883be485085895157279cc7c4eaa8db6. The merged port includes GPU Philox/Poisson thinning, layer density curves, dye clouds, microstructure, source-coordinate sampling and the original NoContraction Vulkan SPIR-V recursive IIR. No FPW application, private photo asset or upstream data/model resource is moved.

Source comparison at RS `e57d0d1a33dadb2fd647201f49e497c14c1d7a9e` confirmed identical Git blobs for `grain_layers.rs`, `layered_grain.rs`, `grain_layers.wgsl`, `grain_iir.comp` and `grain_iir.spv`. In particular, the copied SPIR-V blob is `e23544d20d7f402a70b7005fb7531721793870e3`. The IIR Rust wrapper differs only by adding the explicit ignore annotation and hardware-qualification comment to its long precision test; the source-coordinate helper differs only in its introductory comment.

## Opt-in compatibility

`GrainParams.gpu_multilayer` defaults to `false`, independent of the existing `sublayers_active` field. Existing RS standalone GPU output uses the original sampler until a client explicitly enables the new route. On enabled color profiles only, the core prepares `LayeredGrainGpuParams`; `GrainGpuParams.layers = None` always selects the original simple path. Optional `output_sampling` defaults to `None`.

With wide-sigma grain (>=3 pixels), the original precise Vulkan SPIR-V path must be used. `WgpuBackend::device_descriptor` requests `PASSTHROUGH_SHADERS` on capable Vulkan adapters. Host-injected devices must request the same feature when using wide IIR; unsupported Metal/DX12 devices must not silently switch to a visually different approximation. Narrow FIR and the pre-existing simple sampler continue unchanged.

## Verified standalone results and remaining gates

[PR validation 43397365918](https://github.com/ye602a/spektrafilm-rs/actions/runs/43397365918) and [merged-main validation 43398567694](https://github.com/ye602a/spektrafilm-rs/actions/runs/43398567694) passed. Windows checked the independent crates, CLI and GUI, compiled GPU tests, and ran 72 CPU math/model/core tests. Linux used `llvmpipe (LLVM 20.1.2, 256 bits)` for actual Vulkan submission and ran source-coordinate and multilayer tests without skipping the required GPU path.

The RS Linux run measured multilayer mean `1.5006471944243458`, variance `0.020058962077850902`, and analytic variance `0.01997501919166667`. It also tested repeated state use/accumulator clearing, changed seeds, sampling amount/scale, dye clouds, microstructure, and limited mixed FIR/IIR channel execution. These are new RS software-Vulkan results, not an NVIDIA/AMD hardware qualification.

The original PR #6 CI did not execute the complete IIR test, which is marked `#[ignore]` for ordinary runs. The resumed validation on 2026-11-13 explicitly ran all **1,215** cases locally on Mesa llvmpipe (Debian Mesa 25.0.7, LLVM 19), and [Windows/Linux CI 43424836925](https://github.com/ye602a/spektrafilm-rs/actions/runs/43424836925) subsequently passed with the full suite explicitly invoked through `--ignored`. The CI IIR log records maximum absolute error `1.1920928955078125e-7`. This establishes independent RS software-Vulkan numerical evidence, beyond the earlier limited wide-blur execution and variance checks.

[PR #7](https://github.com/ye602a/spektrafilm-rs/pull/7) merged that qualification and work-plane reuse as `fdef88aa95a852a4c82ba58150ed23ae2bfe7436`. The later density/output implementation additionally passed local synthetic old-revision/new-revision full-chain comparisons, including multilayer, wide IIR, clouds/microstructure and source-coordinate sampling, with all pixels byte-identical. See [FPW_PARITY_MIGRATION_PLAN.md](FPW_PARITY_MIGRATION_PLAN.md), [GPU_NEGATIVE_CACHE.md](GPU_NEGATIVE_CACHE.md) and [GPU_OUTPUT_CONTRACT.md](GPU_OUTPUT_CONTRACT.md). Final-commit CI is a gate for each follow-up change. The dedicated RTX/AMD, six-photo, real interaction/cancellation and device-recovery checks remain separate; software Vulkan does not complete them.

The full resumed validation record and machine-readable results are in [GPU_MIGRATION_VALIDATION_20261113.md](GPU_MIGRATION_VALIDATION_20261113.md).

## Reproduction and qualification commands

```sh
cargo check --locked -p spektrafilm-core -p spektrafilm-gpu --all-targets
cargo test --locked -p spektrafilm-gpu --test grain_source -- --nocapture
cargo test --locked -p spektrafilm-gpu --lib actual_gpu_grain_follows_source_coordinates -- --nocapture
cargo test --locked -p spektrafilm-gpu --lib actual_gpu_multilayer_moments_and_controls -- --nocapture
cargo test --locked -p spektrafilm-gpu --lib actual_gpu_iir_matches_f64_reference -- --ignored --nocapture
```

FPW's prior RTX 3060 Ti reference tests (2026-10-21) found multilayer mean 1.500647 (expected 1.5), variance 0.020059 (analytic 0.019975), and 1215 IIR reference cases at maximum absolute error 1.1920928955078125e-7. These are previous FPW measurements, **not independent RS results**.

Before any FPW pin change: independently run frozen six 4032x6048 RAW-based linear-input/equal-EV images (including DSC_5930), compare actual pixels to the FPW accepted baseline, verify deterministic source-crop/rotation behavior, dye-cloud scratch clearing across images, seed controls, IIR precision, cancellation and resource ownership. FPW main and wide-gamut research remain untouched by this migration.
