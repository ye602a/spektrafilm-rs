# Standalone engine host-GPU contract (phase 1)

Date: 2026-11-13  
Status: **host handle API implemented; external build and real GPU verification pending**  
Target: independently maintained `ye602a/spektrafilm-rs`  
Source: `turbasvin/spektrafilm-rs` (GPL-3.0)

## Intended ownership

The Spektrafilm-rs repository is the single source of maintenance for film simulation, film/print/scanner physics, GPU execution, grain, and future algorithm/performance changes. FPW remains a downstream photo workflow; it owns RAW, user recipes, masks, catalog state, application scheduling, undo and final photo delivery. Only small, host-neutral integration interfaces belong in RS.

This PR intentionally introduces **only a wgpu device/queue injection boundary**. It does not import the FPW vendored implementation, alter film physics, copy private resources, change GPU algorithms, or change the FPW application.

## Phase 1 API

`WgpuBackend::from_device(device, queue)` wraps caller-provided, matching wgpu handles. It does **not** create another adapter or device. `WgpuBackend::compute_device()` borrows the handles for host-side integration. `WgpuBackend::new()` retains its prior behavior and now uses the same constructor internally.

- Each backend owns its own pipeline cache; wgpu handles are reference-counted clones.
- Callers are responsible for passing a queue associated with the device.
- Callers remain responsible for coordinating GPU queue submissions with surface reconfiguration and any host-specific locks.
- The backend does not own host frames, cache keys, application-level memory budgets, or the host's cancellation/scheduling policies.
- No promise of zero-copy interop is made until the full rendering interface and output-buffer ownership contract are added.

The crate currently uses **wgpu 24**. FPW has moved to **wgpu 30.0.1**; **this API alone does not enable FPW to reuse its wgpu-30 device**. A later, separately validated dependency/API migration will unify the versions. Do not add a second RS codebase for FPW.

## Tests and acceptance

- `cargo check --locked -p spektrafilm-gpu --all-targets`
- `cargo test --locked -p spektrafilm-gpu --test host_device`
- The handle API compiles without a GPU; the optional runtime test checks equal handles and a real queue submission if an adapter is available.
- On a verified GPU, test the host's concurrent render/surface-configuration boundaries before relying on shared submission behavior.
- Before FPW cuts over: align wgpu revisions, preserve the accepted film output pixel baseline, confirm cancellation/recovery and physical grain behavior.

No local Rust toolchain or GitHub Actions result was available to this review session; the test source was added, but these commands have **not** yet been reported as passing.

## Color-space hold

FPW's wider-gamut processing space remains **research only**. Do not implement a change to the linear-sRGB film input/transfer conventions, model resources or output color contract as part of the GPU decoupling. Any eventual color-space change must be a separate, measured, versioned decision.

## Subsequent independent RS work

1. Validate and migrate wgpu 24 → 30.0.1 with the existing standalone CLI/GUI/CPU reference preserved.
2. Add host-neutral result/texture ownership, cancellation, regional output and packed RGB16 paths.
3. Port separately attributed multilayer grain, source-anchored coordinates and optional Vulkan IIR behind explicit quality gates.
4. Port bounded stage-specific GPU density cache, with dependency invalidation owned by the RS engine.
5. Revisit stock-specific processing defaults and scan inversion currently residing in FPW. Migrate physics to RS only after preserving the accepted reference images and auditing resources.
6. Update FPW's pinned dependency only after RS runs independently and the FPW integration tests pass.

The original import SHA in FPW's provenance is not retrievable through GitHub; see the separately committed audit branch `audit/fpw-engine-extraction-20261113`. Do not present that SHA as an independently verified Git ancestor.
