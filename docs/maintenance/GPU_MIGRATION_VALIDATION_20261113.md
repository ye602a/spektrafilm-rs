# Resumed standalone GPU migration: implementation and validation

Date: 2026-11-13. Repository: `ye602a/spektrafilm-rs` only.

## Recovered progress

The interrupted session had already merged multilayer physical grain and precise
Vulkan IIR in PR #6 (`c487cb3`). Its next work item was draft PR #7, whose original
Windows/Linux run had succeeded. No evidence in the repository showed a continuing
build failure. The resumed work reviewed that draft, added overlapping-render and
cache-generation tests, and explicitly ran the previously ignored IIR precision
suite. PR #7 merged as `fdef88aa95a852a4c82ba58150ed23ae2bfe7436` after
[Windows and Linux validation](https://github.com/ye602a/spektrafilm-rs/actions/runs/43424836925).

## Subsequent implementation

The developed-negative cache and output consumers extend that verified work:

- One optional immutable density snapshot after DIR/grain and before paper/scan,
  with a 384 MiB default residency budget. RS owns the complete stage key; hosts
  pass only a per-call immutable source identity. Sequence tickets and generations
  prevent stale concurrent publication.
- Full f32 readback, packed RGB16 readback, display-region extraction and a scoped
  same-device consumer all share one physical-chain implementation. The original
  FPW output shaders are copied byte-identically, including exact RGB16 rounding.
- A consumer records copies or computation into the engine's encoder. Submission,
  completion, readback/unmap and the final cancellation decision precede density
  publication and work-plane recycling. Host-owned textures can outlive a render.
- `Pipeline::with_resident_params` prepares the existing physical parameters for
  these output consumers. FPW does not need to reimplement film-stage preparation.

Default rendering and both optional caches retain their previous default behavior:
multilayer and caching are opt-in. No film profiles, spectral curves, accepted
physical shaders, color spaces or FPW application files were changed.

## Measured evidence

| Check | Observed result |
| --- | --- |
| CPU math/model/core | 72 tests passed |
| GPU crate unit tests | 15 passed; the long IIR test was run explicitly |
| Precise IIR | 1,215 cases passed; max absolute error `1.1920928955078125e-7` |
| Work-plane output parity | 2 full-chain tests, legacy and multilayer, including concurrent renders |
| Negative-cache invalidation | 1 full-chain matrix test; actual hits/misses and exact cache-off pixel equality |
| Output kernels | 2 tests; 196,617 RGB16 inputs plus exact crop/reduction comparisons |
| Complete output consumers | 4 tests; cache on/off, linear/sRGB transfer, RGB16, regions, host texture ownership, errors/cancellation/recovery |
| Previous RS commit vs new implementation | 8 cases, 5,952 f32 channels, all bytes identical; maximum difference 0 |
| API compatibility | CPU-only GPU-crate build and f64 core/GPU library builds passed |

Local execution used Rust 1.100.0, wgpu 30.0.1 and Mesa 25.0.7 llvmpipe Vulkan
(LLVM 19) on Debian 13, with `SPEKTRAFILM_REQUIRE_GPU=1`. Missing GPU support could
not silently skip the required runtime tests. The hosted PR #7 Vulkan run used
LLVM 20.1.2 and independently reproduced the complete IIR result. These are
software-Vulkan results, not a new NVIDIA/AMD hardware qualification.

The eight cross-version cases cover legacy/multilayer, printing/direct scan,
dye clouds, microstructure, wide IIR, optical effects and DIR, paper/scanner edits,
odd/thin geometry, and source-coordinate scale/amount controls. The baseline was
compiled separately from `85eb835a0116b8ef77beb4952ae80272a21b0b8a`. Inputs, resolved
parameters, dimensions and source-coordinate transforms were checked before
comparing complete output bytes. The machine-readable report, hashes and exact
scope are in [validation/20261113_gpu_migration.json](validation/20261113_gpu_migration.json).

## Reproduction

The normal standalone workflow covers Windows builds and Linux Vulkan execution.
For a Linux environment with system OpenBLAS, the local feature selection is:

```sh
SPEKTRAFILM_REQUIRE_GPU=1 cargo test --locked \
  -p spektrafilm-core -p spektrafilm-gpu --features openblas-src/system \
  --test work_images --test negative_cache --test gpu_output --test output_kernels

SPEKTRAFILM_REQUIRE_GPU=1 cargo test --locked \
  -p spektrafilm-gpu --features openblas-src/system --lib \
  actual_gpu_iir_matches_f64_reference -- --ignored --nocapture
```

For cross-version pixel comparison, copy the identical
`crates/spektrafilm-core/examples/migration_probe.rs` into a separate worktree at
the pinned reference revision, build and run it there, then run it on the candidate:

```sh
cargo build --locked -p spektrafilm-core -p spektrafilm-gpu \
  --features openblas-src/system --example migration_probe
SPEKTRAFILM_REQUIRE_GPU=1 target/debug/examples/migration_probe /tmp/rs-candidate-pixels
python tools/validation/compare_migration_probe.py /tmp/rs-baseline-pixels /tmp/rs-candidate-pixels
```

The comparator rejects changed input pixels, geometry, source transforms or
parameter configurations before making any pixel-parity claim.

## Gates that remain separate

The fixtures above are deliberately small synthetic images. The original frozen
six-photo FPW inputs, dedicated RTX/AMD validation, production editing and device
recovery checks remain outstanding. Cooperative cancellation discards a failed
result and prevents new cache publication; it cannot undo GPU writes already
submitted to a host-owned destination. The tested callback cancellation boundary
is before submission, and hosts publish a destination only after overall success.

Allocation reuse currently retains one pair of full-frame work planes. It is not
pooling of every optical/grain scratch allocation or the upload staging system.
Cache statistics describe retained objects, not total VRAM. Performance gains
require controlled measurements of real workloads, including exact-key creation.

FPW's dependency pin remains unchanged. Complete the gates in
[FPW_PARITY_MIGRATION_PLAN.md](FPW_PARITY_MIGRATION_PLAN.md) before considering a
separate host integration change. Wide-gamut research remains independent.
