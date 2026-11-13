# Standalone engine host-GPU contract (phase 1)

Date: 2026-11-13  
Status: **host GPU API, wgpu 30 migration and source-coordinate grain tested; FPW cutover and real-hardware film parity pending**  
Target: independently maintained `ye602a/spektrafilm-rs`  
Source: `turbasvin/spektrafilm-rs` (GPL-3.0)

## Intended ownership

The Spektrafilm-rs repository is the single source of maintenance for film simulation, film/print/scanner physics, GPU execution, grain, and future algorithm/performance changes. FPW remains a downstream photo workflow; it owns RAW, user recipes, masks, catalog state, application scheduling, undo and final photo delivery. Only small, host-neutral integration interfaces belong in RS.

These initial standalone patches establish host-owned device/queue injection, shared adapter requirements, wgpu 30, a single-version standalone GUI, and optional source-anchored grain. They do not import the complete FPW vendored engine, copy private resources, or change FPW application code.

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

GitHub Actions verified independent Windows CLI/GUI compilation, **72 CPU math/model/core tests**, a single wgpu-30 lockfile, and real Mesa llvmpipe Vulkan submission. The source-coordinate grain regression also passed with full/crop/rotated deterministic samples. Latest combined validation: https://github.com/ye602a/spektrafilm-rs/actions/runs/43391616163 . This is not an NVIDIA/AMD hardware test or FPW full-resolution image-parity qualification.

## Color-space hold

FPW's wider-gamut processing space remains **research only**. Do not implement a change to the linear-sRGB film input/transfer conventions, model resources or output color contract as part of the GPU decoupling. Any eventual color-space change must be a separate, measured, versioned decision.

## Subsequent independent RS work

1. Port host-neutral output/texture ownership, cancellation, regional output and packed RGB16 paths; test full image and callback lifetimes.
2. Port the separately attributed physical multilayer grain and optional Vulkan IIR, keeping default quality and reference pixels versioned. Source-anchored coordinates have already been migrated as an opt-in API.
3. Port bounded stage-specific GPU density caching and work-buffer pooling, with exact film-stage invalidation in RS and app content identity in FPW.
4. Revisit stock-specific processing defaults, scan inversion and current film-processing code still inside FPW; audit resources and freeze existing output for each route.
5. Update FPW's pinned dependency only after the independent RS engine implements the existing production route and FPW pixel, cancellation, resource and UI regression tests pass.
6. Evaluate wider-gamut input/working/output spaces only in a separate research and rendering-contract migration, not as part of these portability patches.

The original import SHA in FPW's provenance is not retrievable through GitHub; see the separately committed audit branch `audit/fpw-engine-extraction-20261113`. Do not present that SHA as an independently verified Git ancestor.
