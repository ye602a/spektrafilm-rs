# RS → FPW engine parity gate and remaining migration plan

Date: 2026-11-13
Maintained in: `ye602a/spektrafilm-rs`
FPW reference (read-only): `ye602a/fursuit-photo-workflow` main at `ae3428acd98e39c48784a79dc8f3369ab1236cb1`.

## Governing rule

**RS is the only maintained film simulation / GPU compute engine.** FPW owns RAW preparation, source identity, recipes/UI, editing undo, scheduling, and file exports. Transfer algorithms and GPU resources once to RS; FPW should later update its dependency pin and use stable host-neutral APIs. Do not turn FPW into a second maintained copy of shader or film algorithms. Preserve the existing linear-sRGB working contract while wide-gamut research remains separate.

Merged on RS main:

- Host device/queue injection and wgpu 30.0.1 unification.
- Source-coordinate anchored physical grain.
- Opt-in GPU multilayer emulsion, dye clouds and microstructure with original Vulkan precise large-radius IIR. The migrated grain implementation/shader blobs listed in [GRAIN_MULTILAYER_MIGRATION.md](GRAIN_MULTILAYER_MIGRATION.md) match FPW; original `grain_iir.spv` blob `e23544d20d7f402a70b7005fb7531721793870e3` was copied byte-identically.
- RS grain migration `gpu_multilayer = false` by default preserves standalone legacy output. It is not a statement that the FPW existing multilayer route is optional when eventually updating FPW's engine pin.
- One-pair GPU work-plane reuse, merged by [PR #7](https://github.com/ye602a/spektrafilm-rs/pull/7) as `fdef88aa95a852a4c82ba58150ed23ae2bfe7436`. Its [Windows/Linux CI 43424836925](https://github.com/ye602a/spektrafilm-rs/actions/runs/43424836925) also passed the complete 1,215-case IIR precision suite on software Vulkan.

The follow-up implementation in `codex/rs-density-output-20261113` adds negative-density caching and final GPU/RGB16/region output. Local independent tests have passed; Windows/Linux CI for the exact change remains a merge gate. The sections below describe implemented RS behavior and the remaining FPW acceptance work, rather than a request to re-port completed algorithms.

## 1. Reuse intermediate GPU allocations (allocation-only)

Implemented in [GPU_WORK_IMAGES.md](GPU_WORK_IMAGES.md): at most one idle pair of full-frame work planes per backend/device, exact buffer-size matching, and exclusive checkout. Allocation reuse never trusts previous pixels. A negative-cache miss uploads the complete prepared input; only an exact negative-cache hit replaces that upload by completely overwriting the private work plane from its immutable density snapshot. Integrated-GPU direct MAP_WRITE/MAP_READ keeps its dedicated path.

The merged tests cover repeat first photo / change photo / return to first / resize / disable / budget shrink, simultaneous complete renders, distinct active buffer identities, and stale-generation rejection. Idle bytes are reported separately from in-flight/total GPU memory. Auxiliary blur/LUT/parameter buffers and upload/readback staging have **not** been pooled. Broader pooling and performance claims need their own measurements and submission/cancellation lifecycle qualification.

## 2. Cache the developed negative **density**, not the output

FPW reference: vendored `wgpu_backend.rs` has a one-plane `negative` cache (max 384 MiB) and `configure_negative_cache(enabled,input_unchanged)`. It stores density **after DIR and grain but before print/spectral paper processing**. FPW `film.rs` owns typed source identity and its existing `negative_key` includes image format/geometry.

Implemented in [GPU_NEGATIVE_CACHE.md](GPU_NEGATIVE_CACHE.md):

- RS owns the pre-print stage boundary, one immutable density snapshot, budget/eviction and the exact prepared stage dependency key. Default retention is **384 MiB for one snapshot**. The host passes per-call `SourceIdentity` for immutable prepared input pixels, without duplicating film-parameter invalidation logic.
- The key binds source, geometry/layout, complete front-pass LUT/matrix data, developed-density curves/gamma, camera optics/diffusion, halation, DIR, and every grain/seed/sampling/source-transform dependency consumed before the boundary. Exhaustive field matching requires new engine parameters to be classified explicitly. A source token alone cannot authorize a hit.
- Only settings whose prepared upstream inputs remain unchanged may reuse the negative. Paper or scan-route changes can still miss when preparation changes a front-pass exposure correction. Film dye spectra are downstream when their first consumer is printing/direct scanning; the implementation follows actual stage consumers.
- Generation and request-sequence publication rules prevent cancelled, failed or stale completions from becoming cache entries. Hits hold immutable handles; they never reuse a writable work plane as the cached snapshot. A newly created backend/device starts empty.

Local CPU and software-Vulkan tests passed the dependency invalidation matrix, downstream-edit hits, source/geometry/grain misses, counters, concurrent requests, pre-cancellation and recovery, with exact cache-on/off outputs. The real FPW photograph, submitted-cancellation and device-recovery gates remain separate. Residency telemetry excludes in-flight clones, work buffers and CPU key storage; no total-VRAM or performance claim follows from the 384 MiB retention cap.

## 3. Final GPU-owned output + RGB16 + display-region interface

FPW reference: `DisplayRegion {crop:[u32;4],width,height}`, `with_film_output`, `try_run_film_chain_rgb16`, and `film_display.wgsl` / `film_quantize.wgsl` in vendored GPU crate.

Implemented in [GPU_OUTPUT_CONTRACT.md](GPU_OUTPUT_CONTRACT.md):

- Full-resolution development, grain, DIR, paper, scanner and final post-processing run **before** region extraction. Display crops cannot alter the upstream physical calculation or negative-cache identity.
- `with_film_output` exposes a scoped same-device GPU view and encoder. The consumer records work into its own destination, with no full-image f32 CPU readback. RS submits and waits before a successful result can be published and the source plane reused; retaining or later submitting a cloned source handle violates that scope.
- Packed RGB16 preserves the original FPW quantization shader, clamp/rounding rule and odd-channel padding. `DisplayRegion` preserves the original 1:1 and reduction semantics and rejects unsupported geometry explicitly. Regular full-f32 readback remains available.
- Per-call cooperative cancellation returns no successful output or density publication when observed before the final commit boundary. Already-submitted GPU writes cannot be retracted; the caller publishes its destination only after `Ok`. Input/working/output color space remains unchanged.

Local kernel and complete-chain tests passed RGB16 rounding boundaries, odd/tiny images, 1:1/reduced regions, explicit invalid-request errors, and a host-owned GPU texture whose pixels survive subsequent work-plane reuse. Consumer errors and callback-triggered pre-submission cancellation recover without publishing density. These tests do not qualify interruption during already-submitted work or application device recovery.

## 4. Independent RS regression baseline

Eight deterministic synthetic Portra 400 → Portra Endura cases were generated in a separate worktree pinned to `85eb835a0116b8ef77beb4952ae80272a21b0b8a`. They cover legacy/multilayer grain, print/direct scan, wide IIR, dye clouds/microstructure, optics/DIR, paper/scanner controls, odd and 1×17 geometry, and source-coordinate scale/amount. A second baseline process reproduced every byte.

The follow-up implementation matched all **5,952 f32 values / 23,808 bytes** exactly, with maximum absolute error zero. `crates/spektrafilm-core/examples/migration_probe.rs` and `tools/validation/compare_migration_probe.py` preserve this workflow. The comparison first checks original pixels, resolved parameters, geometry, source reference and film/paper configuration; a change in those inputs invalidates the comparison. This checks a real old revision against the new code, independently of cache-on/off tests in a single implementation. The inputs are synthetic small images, not the six frozen FPW photographs.

See [GPU_MIGRATION_VALIDATION_20261113.md](GPU_MIGRATION_VALIDATION_20261113.md) and its linked JSON for the complete local evidence, hashes and reproduction commands.

## 5. No-change FPW cutover gate

RS Actions smoke is necessary but not sufficient for replacing FPW's vendor pin. On the **same driver/device**, replay the original 2026-10-21 frozen six source RAW-derived linear work images at their per-image fixed EV:
`DSC_5930`, `DSC_0003`, `DSC_3870`, `DSC_0449-3`, `DSC_0581`, `DSC_6230`, each 4032×6048. Compare the FPW accepted vendor route vs RS explicit multilayer route, using matching parameters, paper, DIR, light scatter, glare, grain coordinate transform, output sampling and scan route.

For legacy unchanged branches, prefer bit-exact pixel/hash equality. For stochastic/physical grain, additionally compare the actual seed-repeatability, per-channel brightness/density distribution, six-luminance-band statistics, local spatial correlations, and magnified crop look. **Do not treat a close global mean as sufficient visual parity.**

Dedicated RTX Vulkan-IIR qualification still requires all 1,215 reference cases (nine sizes, sigma 3/12/130, disabled/single/mixed channels, constant/change/impulse), `abs(GPU - reference) <= 4*f32::EPSILON*max(1,abs(reference))`. RS local software Vulkan and CI [43424836925](https://github.com/ye602a/spektrafilm-rs/actions/runs/43424836925) passed the suite with maximum absolute error `1.1920928955078125e-7`. Previous FPW hardware results had the same maximum; neither that historical result nor the new software result proves a new RS RTX/AMD run.

Check FPW real editing interactions: repeated render, output crop, source-anchored grain, zoom/rotation, cancellation, undo/redo, initial scene open, preview to export, device recovery, GPU resource sharing, memory residency and output files. No regressions permitted.

**FPW main must remain untouched during migration/verification.** Only after the full independent RS path and parity checks pass should one scoped FPW dependency-pin/integration change be considered. No broad application refactor.

## 6. Explicitly out of scope

Wide-gamut/ACES/OpenDRT choice, new film looks, algorithm replacement, GPU shader approximation to bypass Vulkan NoContraction, FPW UI component work, or a second maintained FPW shader fork. Each requires independent research, acceptance and/or versioned color contract.
