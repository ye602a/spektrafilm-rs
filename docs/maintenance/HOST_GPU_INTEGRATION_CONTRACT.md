# Standalone engine host-GPU contract (phase 1)

Date: 2026-11-13  
Status: **host GPU API, wgpu 30, source-coordinate grain and opt-in multilayer/Vulkan IIR migration merged; allocation reuse in PR #7 remains under validation; FPW cutover and hardware/image parity pending**

Target: independently maintained `ye602a/spektrafilm-rs`  
Source: `turbasvin/spektrafilm-rs` (GPL-3.0)

## Intended ownership

The Spektrafilm-rs repository is the single source of maintenance for film simulation, film/print/scanner physics, GPU execution, grain, and future algorithm/performance changes. FPW remains a downstream photo workflow; it owns RAW, user recipes, masks, catalog state, application scheduling, undo and final photo delivery. Only small, host-neutral integration interfaces belong in RS.

The merged standalone patches establish host-owned device/queue injection, shared adapter requirements, wgpu 30, a single-version standalone GUI, source-anchored grain, and opt-in multilayer grain with precise Vulkan IIR. PR #6 merged as `c487cb3a724ae7f6c05d8eac45c86d3394859eda`. They do not import the complete FPW vendored engine, copy private resources, or change FPW application code.

## Phase 1 API

`WgpuBackend::from_device(device, queue)` wraps caller-provided, matching wgpu handles. It does **not** create another adapter or device. `WgpuBackend::compute_device()` borrows the handles for host-side integration. `WgpuBackend::new()` retains its prior behavior and now uses the same constructor internally.

- Each backend owns its own pipeline cache; wgpu handles are reference-counted clones.
- Callers are responsible for passing a queue associated with the device.
- Callers remain responsible for coordinating GPU queue submissions with surface reconfiguration and any host-specific locks.
- The backend does not own host frames, cache keys, application-level memory budgets, or the host's cancellation/scheduling policies.
- No promise of zero-copy interop is made until the full rendering interface and output-buffer ownership contract are added.

The independent RS `Cargo.lock` now resolves **wgpu 30.0.1 only**, with **eframe/egui 0.37.0**. The FPW native application separately locks wgpu 30.0.1. Version alignment does **not** yet mean that RS standalone GUI and film compute share a single GPU device: they currently use separate device instances. Full FPW shared-device integration remains gated on the remaining vendor-feature migration and image parity. Do not add a second RS codebase for FPW.

## Tests and acceptance

- `cargo check --locked -p spektrafilm-gpu --all-targets`
- `cargo test --locked -p spektrafilm-gpu --test host_device`
- The handle API compiles without a GPU; the optional runtime test checks equal handles and a real queue submission if an adapter is available.
- On a verified GPU, test the host's concurrent render/surface-configuration boundaries before relying on shared submission behavior.
- Before FPW cuts over: align wgpu revisions, preserve the accepted film output pixel baseline, confirm cancellation/recovery and physical grain behavior.

GitHub Actions verified independent Windows CLI/GUI compilation, **72 CPU math/model/core tests**, a single wgpu-30 lockfile, and actual Mesa llvmpipe Vulkan submission. PR #6 validation [43397365918](https://github.com/ye602a/spektrafilm-rs/actions/runs/43397365918) and its merged-main run [43398567694](https://github.com/ye602a/spektrafilm-rs/actions/runs/43398567694) passed Windows and Linux checks. Linux exercised source-coordinate crop/rotation parity, multilayer statistical moments, seed/sampling controls, dye clouds, microstructure, and limited wide-IIR execution.

The complete 1,215-case IIR precision test is present but explicitly ignored by default; these CI runs did **not** execute it. They also did **not** compare the six frozen FPW full-resolution images or exercise NVIDIA/AMD hardware. The resumed validation additionally passed all 1,215 IIR reference cases on local Mesa llvmpipe with maximum absolute error `1.1920928955078125e-7`; the Linux workflow now explicitly requires that suite. Algorithm migration and software-Vulkan smoke are complete for PR #6; FPW image/hardware qualification is still pending. See [GRAIN_MULTILAYER_MIGRATION.md](GRAIN_MULTILAYER_MIGRATION.md) for exact scope and [FPW_PARITY_MIGRATION_PLAN.md](FPW_PARITY_MIGRATION_PLAN.md) for the cutover gates.

## Color-space hold

FPW's wider-gamut processing space remains **research only**. Do not implement a change to the linear-sRGB film input/transfer conventions, model resources or output color contract as part of the GPU decoupling. Any eventual color-space change must be a separate, measured, versioned decision.

## Subsequent independent RS work

1. Finish PR #7 allocation-only work-buffer reuse and its independent same-device output/ownership checks. Reusing allocations must not imply reusing image content. See [GPU_WORK_IMAGES.md](GPU_WORK_IMAGES.md).
2. Qualify the already-merged multilayer grain and precise Vulkan IIR against the frozen FPW pixels and dedicated 1,215-case hardware suite. Keep `gpu_multilayer = false` as the standalone compatibility default; the future FPW integration must explicitly enable its existing multilayer route.
3. Port bounded stage-specific GPU negative-density caching, with exact film-stage invalidation in RS and opaque source identity in FPW. Then port host-neutral output/texture ownership, cancellation, regional output and packed RGB16 paths; test full image and callback lifetimes.
4. Revisit stock-specific processing defaults, scan inversion and current film-processing code still inside FPW; audit resources and freeze existing output for each route.
5. Update FPW's pinned dependency only after the independent RS engine implements the existing production route and FPW pixel, cancellation, resource and UI regression tests pass.
6. Evaluate wider-gamut input/working/output spaces only in a separate research and rendering-contract migration, not as part of these portability patches.

The original import SHA in FPW's provenance is not retrievable through GitHub; see the separately committed audit branch `audit/fpw-engine-extraction-20261113`. Do not present that SHA as an independently verified Git ancestor.
