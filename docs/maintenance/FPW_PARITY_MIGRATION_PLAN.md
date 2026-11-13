# FPW-first engine parity and independent development plan

Date: 2026-11-13
Maintained in: `ye602a/spektrafilm-rs`
FPW reference (read-only): `ye602a/fursuit-photo-workflow` main at `ae3428acd98e39c48784a79dc8f3369ab1236cb1`.

## Governing rule

**FPW is the primary product and acceptance target; RS is its independently
maintained, public film engine.** Product priorities and accepted looks come
from FPW. Film algorithms, versioned stock/calibration behavior and engine GPU
resources evolve once in RS; RAW preparation, recipe persistence, masks, UI,
undo, application scheduling and file delivery remain in FPW. Standalone RS
builds stay usable without FPW. Independent development does not mean automatic
adoption of RS main by existing FPW projects, nor a second maintained shader
fork. Preserve the accepted linear-sRGB input contract while wide-gamut research
remains separate.

Merged on RS main:

- Host device/queue injection and wgpu 30.0.1 unification.
- Source-coordinate anchored physical grain.
- Opt-in GPU multilayer emulsion, dye clouds and microstructure with original Vulkan precise large-radius IIR. The migrated grain implementation/shader blobs listed in [GRAIN_MULTILAYER_MIGRATION.md](GRAIN_MULTILAYER_MIGRATION.md) match FPW; original `grain_iir.spv` blob `e23544d20d7f402a70b7005fb7531721793870e3` was copied byte-identically.
- RS grain migration `gpu_multilayer = false` by default preserves standalone legacy output. It is not a statement that the FPW existing multilayer route is optional when eventually updating FPW's engine pin.
- One-pair GPU work-plane reuse, merged by [PR #7](https://github.com/ye602a/spektrafilm-rs/pull/7) as `fdef88aa95a852a4c82ba58150ed23ae2bfe7436`. Its [Windows/Linux CI 43424836925](https://github.com/ye602a/spektrafilm-rs/actions/runs/43424836925) also passed the complete 1,215-case IIR precision suite on software Vulkan.

- Negative-density caching and final GPU/RGB16/region output, merged by
  [PR #8](https://github.com/ye602a/spektrafilm-rs/pull/8) as `7994b672593c2b8eab697b3d51f22b797dd1cc0d`.
  Its exact implementation tree passed [Windows/Linux CI 43427632637](https://github.com/ye602a/spektrafilm-rs/actions/runs/43427632637).

The current FPW-first integration work adds the four missing host contracts
below. It preserves existing standalone defaults and does not change the FPW
production dependency. Every code change still requires its own exact-commit
Windows/Linux CI before merging.

| FPW behavior | RS implementation | Adoption boundary |
| --- | --- | --- |
| Stock halation and accepted slide DIR | [Versioned opt-in stock setup](FPW_ACCEPTED_STOCK_SETUP_V1.md) | Preserve pre/post-calibration order; normal constructors keep their defaults |
| Calibrated negative inversion and legacy finish | [Frozen output v1](CALIBRATED_OUTPUT_V1.md) | Match stock and linear AP0 transport; invert/quantize/finish before display reduction |
| Same-device latitude/colour probes | [Request-local cache isolation](GPU_PROBE_ISOLATION.md) | Use `isolate_photo_caches`; no global save/restore of photo cache state |
| Shared display/compute surface coordination | [Optional host submission gate](GPU_SUBMISSION_GATE.md) | Same mutex for host surface changes and engine submissions; no lock across explicit wait |

The [public consumer example](../../crates/spektrafilm-core/examples/fpw_host_contract.rs)
exercises the paper route, both accepted slides and both calibrated negatives
using the real RS GPU chain and public APIs. It is a reproducible integration
smoke, not a build or window test of the FPW application.

## Remaining work in adoption order

1. **Bind the thin FPW adapter to an accepted RS revision.** Replace the old
   constructor, global cache flags, stock constants, local negative-output code
   and probe wrapper with these APIs; retain FPW recipes, scheduling and undo.
   Keep the initial paper-only density-cache policy. Prepared-pixel identities
   cover RAW/pre-development/masks/geometry/feedback; RS adds film dependencies.
2. **Verify the real installed resources.** The [resource audit](FPW_RESOURCE_COMPATIBILITY.md)
   proves all 55 shared repository resources match FPW after strictly defined
   text line-ending normalization; binary LUTs match byte-for-byte. The actual
   `engine_resources` directory must pass the provided checker. A matching
   RESOURCE_PIN declaration alone does not prove the files are present/correct.
3. **Resolve the preserved CPU-fallback distinction explicitly.** FPW's old
   reduced CPU negative path can resize before nonlinear inversion, unlike its
   accepted GPU path. The new output v1 preserves GPU order and full-size CPU
   conversion. Check panorama/CPU-export fallbacks before switching the host;
   do not silently change an old saved route or label the two orders identical.
4. **Run the actual photograph and host gates below**, including shared-device
   surface resize, submitted cancellation, stale-frame publication and device
   recovery. Synthetic/probe success is not application acceptance.
5. **Adopt once and remove migrated duplicates.** Record the RS revision,
   verified resource identity, `fpw-accepted-stock-v1`, existing output route and
   accepted pixel evidence together. Replace the old vendor pin in one scoped
   FPW integration; retain a rollback revision. A new look requires a new route
   identity and old-recipe replay, rather than a floating dependency update.

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

## 5. FPW production cutover gate

RS Actions smoke is necessary but not sufficient for replacing FPW's vendor pin. On the **same driver/device**, replay the original 2026-10-21 frozen six source RAW-derived linear work images at their per-image fixed EV:
`DSC_5930`, `DSC_0003`, `DSC_3870`, `DSC_0449-3`, `DSC_0581`, `DSC_6230`, each 4032×6048. Compare the FPW accepted vendor route vs RS explicit multilayer route, using matching parameters, paper, DIR, light scatter, glare, grain coordinate transform, output sampling and scan route.

For legacy unchanged branches, prefer bit-exact pixel/hash equality. For stochastic/physical grain, additionally compare the actual seed-repeatability, per-channel brightness/density distribution, six-luminance-band statistics, local spatial correlations, and magnified crop look. **Do not treat a close global mean as sufficient visual parity.**

Dedicated RTX Vulkan-IIR qualification still requires all 1,215 reference cases (nine sizes, sigma 3/12/130, disabled/single/mixed channels, constant/change/impulse), `abs(GPU - reference) <= 4*f32::EPSILON*max(1,abs(reference))`. RS local software Vulkan and CI [43424836925](https://github.com/ye602a/spektrafilm-rs/actions/runs/43424836925) passed the suite with maximum absolute error `1.1920928955078125e-7`. Previous FPW hardware results had the same maximum; neither that historical result nor the new software result proves a new RS RTX/AMD run.

Check FPW real editing interactions: repeated render, output crop, source-anchored grain, zoom/rotation, cancellation, undo/redo, initial scene open, preview to export, device recovery, GPU resource sharing, memory residency and output files. No regressions permitted.

**Keep the accepted FPW main stable while qualifying this engine migration.**
After the independent path, real resources and parity/interaction checks pass,
make one scoped FPW dependency and thin-adapter integration. Do not turn this
into a broad application refactor or require RS development to stop while FPW
continues using its previous accepted revision.

## 6. Explicitly out of scope

Wide-gamut/ACES/OpenDRT choice, new film looks, algorithm replacement, GPU shader approximation to bypass Vulkan NoContraction, FPW UI component work, or a second maintained FPW shader fork. Each requires independent research, acceptance and/or versioned color contract.
