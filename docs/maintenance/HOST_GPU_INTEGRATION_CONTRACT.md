# Standalone engine host-GPU integration contract

Date: 2026-11-13  
Status: **host GPU, grain/IIR and work-plane reuse merged; negative-density caching and scoped output/RGB16/region interfaces implemented and locally validated; FPW hardware, photograph and interaction cutover gates remain open**

Target: independently maintained `ye602a/spektrafilm-rs`  
Source: `turbasvin/spektrafilm-rs` (GPL-3.0)

## Intended ownership

The Spektrafilm-rs repository is the single source of maintenance for film simulation, film/print/scanner physics, GPU execution, grain, and future algorithm/performance changes. FPW remains a downstream photo workflow; it owns RAW, user recipes, masks, catalog state, application scheduling, undo and final photo delivery. Only small, host-neutral integration interfaces belong in RS.

The merged standalone patches establish host-owned device/queue injection, shared adapter requirements, wgpu 30, a single-version standalone GUI, source-anchored grain, and opt-in multilayer grain with precise Vulkan IIR. PR #6 merged as `c487cb3a724ae7f6c05d8eac45c86d3394859eda`; [PR #7](https://github.com/ye602a/spektrafilm-rs/pull/7) merged work-plane reuse as `fdef88aa95a852a4c82ba58150ed23ae2bfe7436`.

The follow-up implementation in `codex/rs-density-output-20261113` adds an engine-owned negative-density cache, cooperative per-call cancellation, scoped GPU consumption, packed RGB16 and display-region output. Local CPU and software-Vulkan regressions have passed. Windows/Linux CI for the exact follow-up commit is a merge gate; the earlier PR #7 result must not be presented as its CI result. No FPW application code, private photo asset or working-color-space migration is included.

## Device and ownership contract

`WgpuBackend::from_device(device, queue)` wraps caller-provided, matching wgpu handles. It does **not** create another adapter or device. `WgpuBackend::compute_device()` borrows the handles for host-side integration. `WgpuBackend::new()` retains its prior behavior and now uses the same constructor internally.

- Each backend owns its own pipeline cache; wgpu handles are reference-counted clones.
- Callers are responsible for passing a queue associated with the device.
- Callers remain responsible for coordinating GPU queue submissions with surface reconfiguration and any host-specific locks.
- RS owns the exact pre-print stage dependency key, density snapshot, publication rules, and bounded engine caches. The host supplies only immutable prepared-input identity and its scheduling/cancellation signals; it must not maintain a second list of film-parameter invalidation rules.
- Work-plane retention holds at most one idle pair. The separate negative cache holds at most one immutable density snapshot, with a default 384 MiB limit. These are component residency limits, not an application-wide VRAM budget. Auxiliary blur/LUT/parameter buffers and upload/readback staging are not pooled by this migration.
- Allocation reuse never trusts previous pixels. A negative-cache miss uploads the complete prepared input; an independently validated negative hit completely overwrites the private work plane from its immutable density snapshot.
- A scoped same-device GPU callback records consumption after the complete physical chain and does not require full-image f32 CPU readback. RS submits and waits before the callback result can be published or its work plane reused. The host retains its own destination, not RS's borrowed source plane. See [GPU_OUTPUT_CONTRACT.md](GPU_OUTPUT_CONTRACT.md).
- The host owns source frames, application-wide memory policy, scheduling, and when to request cancellation. RS checks that request at defined preparation/submission/completion boundaries; already-submitted GPU commands finish before resources are released.

The independent RS `Cargo.lock` now resolves **wgpu 30.0.1 only**, with **eframe/egui 0.37.0**. The FPW native application separately locks wgpu 30.0.1. Version alignment does **not** mean that RS standalone GUI and film compute share a single GPU device: they currently use separate device instances. Full FPW shared-device integration remains gated on photograph/hardware/interaction parity. Do not add a second RS codebase for FPW.

## Tests and acceptance

- `cargo check --locked -p spektrafilm-gpu --all-targets`
- `cargo test --locked -p spektrafilm-gpu --test host_device`
- The handle API compiles without a GPU; the optional runtime test checks equal handles and a real queue submission if an adapter is available.
- On a verified GPU, test the host's concurrent render/surface-configuration boundaries before relying on shared submission behavior.
- Before FPW cuts over: align wgpu revisions, preserve the accepted film output pixel baseline, confirm cancellation/recovery and physical grain behavior.

GitHub Actions [43424836925](https://github.com/ye602a/spektrafilm-rs/actions/runs/43424836925) passed Windows and Linux validation for the work-plane/IIR qualification change: independent CLI/GUI compilation, CPU tests, one wgpu version, actual Vulkan execution, full-chain reuse/ownership checks, and all **1,215** IIR reference cases. The IIR log reports maximum absolute error `1.1920928955078125e-7`. The test remains ignored in an unfiltered default run; Linux CI explicitly invokes it with `--ignored`.

The density/output follow-up passed local software-Vulkan cache invalidation, output-kernel, scoped-consumer and full-chain regressions. An independent worktree pinned to `85eb835a0116b8ef77beb4952ae80272a21b0b8a` supplied eight deterministic synthetic Portra/Endura baselines. All 5,952 output f32 values (23,808 bytes) matched the follow-up implementation byte-for-byte, with identical inputs, geometry, source coordinates and parameters. These checks cover software-Vulkan behavior; they do **not** replace the six frozen FPW photos, a new RTX/AMD run, real editing interactions, cancellation during submitted work, or device recovery. See [FPW_PARITY_MIGRATION_PLAN.md](FPW_PARITY_MIGRATION_PLAN.md) for those gates.

The exact local environment, commands, test scope and machine-readable hashes are recorded in [GPU_MIGRATION_VALIDATION_20261113.md](GPU_MIGRATION_VALIDATION_20261113.md).

## Color-space hold

FPW's wider-gamut processing space remains **research only**. Do not implement a change to the linear-sRGB film input/transfer conventions, model resources or output color contract as part of the GPU decoupling. Any eventual color-space change must be a separate, measured, versioned decision.

## Implemented interfaces and remaining integration work

1. [GPU_WORK_IMAGES.md](GPU_WORK_IMAGES.md): one-pair allocation reuse is merged. Pooling auxiliary resources or staging is separate work requiring measured benefit and lifecycle checks.
2. [GPU_NEGATIVE_CACHE.md](GPU_NEGATIVE_CACHE.md): one-snapshot density caching, exact RS stage keys, host source identity, generation/ticket publication and bounded residency are implemented.
3. [GPU_OUTPUT_CONTRACT.md](GPU_OUTPUT_CONTRACT.md): scoped GPU output, RGB16, display-region extraction and cooperative cancellation are implemented. Final-commit Windows/Linux CI remains an acceptance requirement for their changes.
4. Replay the frozen FPW pixels and dedicated hardware IIR suite; validate real source/zoom/rotation/undo/export interactions, submitted cancellation and device recovery. Keep `gpu_multilayer = false` as the standalone compatibility default; future FPW integration must explicitly enable its existing multilayer route.
5. Audit remaining stock-specific defaults, scan inversion, resources and host adaptation against existing FPW behavior. Update FPW's pin only after these image, resource and interaction gates pass, with one scoped integration change.
6. Keep wider-gamut input/working/output choices in separate research and any later versioned rendering-contract change.

The original import SHA in FPW's provenance is not retrievable through GitHub; see the separately committed audit branch `audit/fpw-engine-extraction-20261113`. Do not present that SHA as an independently verified Git ancestor.
