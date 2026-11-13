# FPW-first independent engine integration contract

Date: 2026-11-13  
Status: **GPU migration through PR #8 merged and Windows/Linux validated; accepted stock setup, calibrated output, probe isolation and submission-gate compatibility implemented and locally validated; FPW dependency-pin cutover remains gated**

Target: independently maintained `ye602a/spektrafilm-rs`  
Source: `turbasvin/spektrafilm-rs` (GPL-3.0)

## Intended ownership

**FPW is the primary product and acceptance target.** Independent RS development
exists to let the same film engine evolve in one repository while FPW continues
to develop its application. The public library, CLI and GUI must remain buildable
without FPW, but speculative portability or SDK design is not a reason to alter
accepted FPW pixels, delay a concrete integration need, or maintain duplicate
film implementations.

Spektrafilm-rs owns film/print/scanner physics, GPU execution, physical grain,
versioned calibrated output, exact engine invalidation and bounded engine caches.
FPW owns RAW preparation, recipe persistence, masks, catalog state, application
scheduling, undo, final file delivery and frame publication. FPW maps its recipe
and source identity into the engine API; it does not copy the engine's film math
or cache-dependency list. The RS repository is already public under its code and
data licenses; open source does not imply a completed FPW dependency cutover.

The merged standalone patches establish host-owned device/queue injection, shared adapter requirements, wgpu 30, a single-version standalone GUI, source-anchored grain, and opt-in multilayer grain with precise Vulkan IIR. PR #6 merged as `c487cb3a724ae7f6c05d8eac45c86d3394859eda`; [PR #7](https://github.com/ye602a/spektrafilm-rs/pull/7) merged work-plane reuse as `fdef88aa95a852a4c82ba58150ed23ae2bfe7436`.

[PR #8](https://github.com/ye602a/spektrafilm-rs/pull/8) merged as
`7994b672593c2b8eab697b3d51f22b797dd1cc0d`, adding engine-owned negative-density
caching, per-call cancellation and scoped GPU/RGB16/region output. Its exact
implementation tree passed [Windows/Linux CI 43427632637](https://github.com/ye602a/spektrafilm-rs/actions/runs/43427632637).
No FPW application code, private photographs or working-space change was included.

## Device and ownership contract

`WgpuBackend::from_device(device, queue)` wraps caller-provided, matching wgpu
handles without another adapter or device. The additional
`from_device_with_submission_lock(device, queue, Option<Arc<Mutex<()>>>)`
accepts FPW's existing surface-submission gate. `compute_device()` borrows the
handles; `new()` remains the standalone convenience constructor.

- Each backend owns its own pipeline cache; wgpu handles are reference-counted clones.
- Callers are responsible for passing a queue associated with the device.
- All engine queue submissions honor the optional shared gate, held only around
  submission and released before explicit polling/readback. The resident route
  rechecks cancellation after acquiring it. The host uses the same gate for
  surface reconfiguration and its own submissions. Film-output consumers run
  outside the gate; native wgpu completion callbacks can execute inline during
  submit and must not recursively acquire that same mutex. See [GPU_SUBMISSION_GATE.md](GPU_SUBMISSION_GATE.md).
- RS owns the exact pre-print stage dependency key, density snapshot, publication rules, and bounded engine caches. The host supplies only immutable prepared-input identity and its scheduling/cancellation signals; it must not maintain a second list of film-parameter invalidation rules.
- Work-plane retention holds at most one idle pair. The separate negative cache holds at most one immutable density snapshot, with a default 384 MiB limit. These are component residency limits, not an application-wide VRAM budget. Auxiliary blur/LUT/parameter buffers and upload/readback staging are not pooled by this migration.
- Allocation reuse never trusts previous pixels. A negative-cache miss uploads the complete prepared input; an independently validated negative hit completely overwrites the private work plane from its immutable density snapshot.
- A scoped same-device GPU callback records consumption after the complete physical chain and does not require full-image f32 CPU readback. RS submits and waits before the callback result can be published or its work plane reused. The host retains its own destination, not RS's borrowed source plane. See [GPU_OUTPUT_CONTRACT.md](GPU_OUTPUT_CONTRACT.md).
- The host owns source frames, application-wide memory policy, scheduling, and when to request cancellation. RS checks that request at defined preparation/submission/completion boundaries; already-submitted GPU commands finish before resources are released.
- `FilmRenderOptions::isolate_photo_caches` gives a probe private work buffers and
  bypasses the photo density cache, including counters and publication tickets.
  It shares the device and compiled pipelines without saving/restoring global
  cache state. Concurrent photo updates therefore survive successful, failed or
  cancelled probes. See [GPU_PROBE_ISOLATION.md](GPU_PROBE_ISOLATION.md).

The independent RS `Cargo.lock` now resolves **wgpu 30.0.1 only**, with **eframe/egui 0.37.0**. The FPW native application separately locks wgpu 30.0.1. Version alignment does **not** mean that RS standalone GUI and film compute share a single GPU device: they currently use separate device instances. Full FPW shared-device integration remains gated on photograph/hardware/interaction parity. Do not add a second RS codebase for FPW.

## Tests and acceptance

- `cargo check --locked -p spektrafilm-gpu --all-targets`
- `cargo test --locked -p spektrafilm-gpu --test host_device`
- The handle API compiles without a GPU; the optional runtime test checks equal handles and a real queue submission if an adapter is available.
- On a verified GPU, test the host's concurrent render/surface-configuration boundaries before relying on shared submission behavior.
- Both projects already use wgpu 30.0.1. Maintain that alignment; before a pin
  change preserve accepted pixels and qualify host cancellation/recovery,
  surface coordination and physical grain on the actual FPW device.

GitHub Actions [43424836925](https://github.com/ye602a/spektrafilm-rs/actions/runs/43424836925) passed Windows and Linux validation for the work-plane/IIR qualification change: independent CLI/GUI compilation, CPU tests, one wgpu version, actual Vulkan execution, full-chain reuse/ownership checks, and all **1,215** IIR reference cases. The IIR log reports maximum absolute error `1.1920928955078125e-7`. The test remains ignored in an unfiltered default run; Linux CI explicitly invokes it with `--ignored`.

The density/output follow-up passed local and hosted software-Vulkan cache
invalidation, output-kernel, scoped-consumer and full-chain regressions. An
independent worktree pinned to `85eb835a0116b8ef77beb4952ae80272a21b0b8a` supplied
eight deterministic synthetic Portra/Endura baselines. All 5,952 f32 values
(23,808 bytes) matched byte-for-byte with identical inputs and parameters.
These checks do not replace the six frozen FPW photos, a new RTX/AMD run,
real editing interactions, cancellation during submitted work or device recovery.

The earlier migration environment and hashes are recorded in
[GPU_MIGRATION_VALIDATION_20261113.md](GPU_MIGRATION_VALIDATION_20261113.md).
The current FPW-first changes, actual commands and remaining adoption work are
recorded in [FPW_ENGINE_COUPLING_20261113.md](FPW_ENGINE_COUPLING_20261113.md).

## Color-space hold

FPW's wider-gamut processing space remains **research only**. Do not implement a change to the linear-sRGB film input/transfer conventions, model resources or output color contract as part of the GPU decoupling. Any eventual color-space change must be a separate, measured, versioned decision.

## Implemented interfaces and remaining integration work

1. [GPU_WORK_IMAGES.md](GPU_WORK_IMAGES.md): one-pair allocation reuse is merged. Pooling auxiliary resources or staging is separate work requiring measured benefit and lifecycle checks.
2. [GPU_NEGATIVE_CACHE.md](GPU_NEGATIVE_CACHE.md): one-snapshot density caching, exact RS stage keys, host source identity, generation/ticket publication and bounded residency are implemented.
3. [GPU_OUTPUT_CONTRACT.md](GPU_OUTPUT_CONTRACT.md): scoped GPU output, RGB16,
   display-region extraction and cooperative cancellation are merged and tested.
   The [frozen FPW final-output consumer](CALIBRATED_OUTPUT_V1.md) adds the existing negative inversion and
   finish sequence before display reduction; it does not redefine the generic
   output APIs or the engine working space.
4. Replay the frozen FPW pixels and dedicated hardware IIR suite; validate real source/zoom/rotation/undo/export interactions, submitted cancellation and device recovery. Keep `gpu_multilayer = false` as the standalone compatibility default; future FPW integration must explicitly enable its existing multilayer route.
5. Use the opt-in [accepted stock setup](FPW_ACCEPTED_STOCK_SETUP_V1.md), frozen
   calibrated output, isolated probes and shared submission gate when preparing
   the FPW adapter. The [public consumer example](../../crates/spektrafilm-core/examples/fpw_host_contract.rs)
   compiles/runs paper, the two accepted slides and both calibrated negatives
   using only RS APIs and bundled data. It is not the FPW binary or UI test.
6. Check [resource compatibility](FPW_RESOURCE_COMPATIBILITY.md), including the
   actual installed physical-resource directory. Preserve the recipe's route,
   engine revision and resource identity together. Update FPW's dependency only
   after the image, resource and host interaction gates pass.
7. Keep wider-gamut input/working/output choices in separate research and any
   later versioned rendering-contract change.

## Independent development and adoption

- Develop film algorithms and performance changes in RS. Add a regression for
  the affected FPW contract and keep the standalone Windows/Linux builds working.
- FPW follows an exact accepted revision and matching resources, not floating
  `main`. New RS work can land without automatically changing existing projects.
- A change to calibrated tables, accepted defaults, output order or working
  space needs a new rendering-contract identity and old-recipe replay. An API
  addition that preserves pixels does not justify silently changing that look.
- After local FPW acceptance, one scoped host integration can replace the vendor
  fork and remove moved algorithm copies. Keep a rollback pin until acceptance
  evidence is recorded. Do not keep two actively maintained film engines.

The original import SHA in FPW's provenance is not retrievable through GitHub; see the separately committed audit branch `audit/fpw-engine-extraction-20261113`. Do not present that SHA as an independently verified Git ancestor.
