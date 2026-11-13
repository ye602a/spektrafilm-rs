# RS → FPW engine parity gate and remaining migration plan

Date: 2026-11-13
Maintained in: `ye602a/spektrafilm-rs`
FPW reference (read-only): `ye602a/fursuit-photo-workflow` main at `ae3428acd98e39c48784a79dc8f3369ab1236cb1`.

## Governing rule

**RS is the only maintained film simulation / GPU compute engine.** FPW owns RAW preparation, source identity, recipes/UI, editing undo, scheduling, and file exports. Transfer algorithms and GPU resources once to RS; FPW should later update its dependency pin and use stable host-neutral APIs. Do not turn FPW into a second maintained copy of shader or film algorithms. Preserve the existing linear-sRGB working contract while wide-gamut research remains separate.

Previously completed on RS main:
- Host device/queue injection and wgpu 30.0.1 unification.
- Source-coordinate anchored physical grain.
- Opt-in GPU multilayer emulsion, dye clouds and microstructure with original Vulkan precise large-radius IIR. Source implementations in FPW and RS were verified to have the same Git blob SHA; original `grain_iir.spv` blob `e23544d20d7f402a70b7005fb7531721793870e3` was copied byte-identically.
- RS grain migration `gpu_multilayer = false` by default preserves standalone legacy output. It is not a statement that the FPW existing multilayer route is optional when eventually updating FPW's engine pin.

GPU work-buffer reuse is developed separately from both negative density caching and final RGB16 output; independently verify each change before accepting it.

## 1. Reuse intermediate GPU allocations (allocation-only)

Retain at most one idle pair of full-frame work planes per backend/device. Reacquire only matching buffer sizes; never share an active pair across concurrent renders. Re-upload *all* source pixels on every render; do not equate reusing an allocation with reusing image content. Integrated-GPU direct MAP_WRITE/MAP_READ must not regress. Treat staging-belt reuse as a distinct change requiring proof of cancellation, encoder/queue ordering, and lifecycle; do not copy FPW's assumption of one serialized render worker into independent RS.

Acceptance: repeat first photo / change photo / return to first / change dimensions / disable cache / reduce budget; outputs must match reuse-off on the same device. Record idle allocation bytes separately from in-flight/entire GPU occupancy. No speed claim until fair repeated timings.

## 2. Cache the developed negative **density**, not the output

FPW reference: vendored `wgpu_backend.rs` has a one-plane `negative` cache (max 384 MiB) and `configure_negative_cache(enabled,input_unchanged)`. It stores density **after DIR and grain but before print/spectral paper processing**. FPW `film.rs` owns typed source identity and its existing `negative_key` includes image format/geometry.

Design target:
- RS owns the pre-print stage boundary, GPU density plane, budget/eviction, and internal algorithm/stage parameter dependency fingerprint. FPW owns only the opaque source/frame identity and signals source changes; an added RS film parameter must **not** require a new FPW-specific invalidation rule.
- Negative-cache identity must bind source token, image dimensions, film/profile/development/exposure/format, camera diffusion/optics, halation, DIR, grain model/seed/sampling/coordinate transform, and all pre-print spectral inputs. A source token alone is **not sufficient** if these changed.
- Only downstream print/scan/enlarger/post-processing settings may reuse the cached density once upstream dependencies are unchanged. `scan_film` mode and different paper routes require stage-by-stage checks but must not silently change the physical negative.
- Cancelled/failed/in-flight results are never published as cache hits; device change, resize, invalid key, disabled cache, and budget overrun invalidate cleanly. Explicitly define an immutable token and generation or lease mechanism for concurrent callers.

Acceptance: one-setting-at-a-time invalidation matrix; first render + repeated paper edits + film/development/grain/source/geometry edits; cache hit/miss counters, exact image equality against cache off, no GPU buffer use-after-recycle, cancellation and recovery. Verify actual FPW baseline before a host pin change.

## 3. Final GPU-owned output + RGB16 + display-region interface

FPW reference: `DisplayRegion {crop:[u32;4],width,height}`, `with_film_output`, `try_run_film_chain_rgb16`, and `film_display.wgsl` / `film_quantize.wgsl` in vendored GPU crate.

Design target:
- Full-resolution physical development, grain, DIR, paper and scanner run **before** region extraction. A display/preview crop must not alter grain geometry or cache dependencies upstream.
- Provide a scoped same-device GPU image callback for a caller that needs to copy into its own long-lived texture. Explicitly document buffer ownership, output dimensions, channel order and transfer function. Do not hand out a borrowed buffer that remains valid after RS begins another film render.
- Provide packed interleaved RGB16 with identical rounding/clamping and odd-channel padding to FPW's accepted path, and a display/region sampling API matching pixel-center/crop semantics. Keep regular f32 full readback available.
- Input/working/output color-space change is **not** part of this interface migration. Cancellation must not publish partial output.

Acceptance: full vs region equality, odd dimensions and bounds, tiny image, same-device texture consumption, GPU/CPU export comparison, and byte-identical RGB16 pixel references where possible. Keep large-panorama fallback behavior explicit.

## 4. No-change FPW cutover gate

RS Actions smoke is necessary but not sufficient for replacing FPW's vendor pin. On the **same driver/device**, replay the original 2026-10-21 frozen six source RAW-derived linear work images at their per-image fixed EV:
`DSC_5930`, `DSC_0003`, `DSC_3870`, `DSC_0449-3`, `DSC_0581`, `DSC_6230`, each 4032×6048. Compare the FPW accepted vendor route vs RS explicit multilayer route, using matching parameters, paper, DIR, light scatter, glare, grain coordinate transform, output sampling and scan route.

For legacy unchanged branches, prefer bit-exact pixel/hash equality. For stochastic/physical grain, additionally compare the actual seed-repeatability, per-channel brightness/density distribution, six-luminance-band statistics, local spatial correlations, and magnified crop look. **Do not treat a close global mean as sufficient visual parity.**

Dedicated RTX Vulkan-IIR qualification: all 1,215 reference cases (nine sizes, sigma 3/12/130, disabled/single/mixed channels, constant/change/impulse), `abs(GPU - reference) <= 4*f32::EPSILON*max(1,abs(reference))`. Previous FPW hardware result had max error `1.1920928955078125e-7`; that result is **not** proof of a new RS hardware run.

Check FPW real editing interactions: repeated render, output crop, source-anchored grain, zoom/rotation, cancellation, undo/redo, initial scene open, preview to export, device recovery, GPU resource sharing, memory residency and output files. No regressions permitted.

**FPW main must remain untouched during migration/verification.** Only after the full independent RS path and parity checks pass should one scoped FPW dependency-pin/integration change be considered. No broad application refactor.

## 5. Explicitly out of scope

Wide-gamut/ACES/OpenDRT choice, new film looks, algorithm replacement, GPU shader approximation to bypass Vulkan NoContraction, FPW UI component work, or a second maintained FPW shader fork. Each requires independent research, acceptance and/or versioned color contract.
