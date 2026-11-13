# FPW-first engine coupling: implementation and acceptance record

Date: 2026-11-13
RS base: `7994b672593c2b8eab697b3d51f22b797dd1cc0d` (merged PR #8)
FPW reference: `ae3428acd98e39c48784a79dc8f3369ab1236cb1`, read-only

## Goal and current state

FPW remains the primary product and defines the accepted physical-film output.
RS is already a public, independently buildable engine repository. Engine math,
versioned stock/calibration behavior and GPU execution are maintained here;
FPW retains RAW preparation, recipes, masks, scheduling, UI, undo, output files
and publication of completed frames. Independent RS releases do not silently
upgrade FPW projects. A later adoption records an exact engine revision and
verified resources together with the saved rendering route.

The previous GPU migration was useful but left real FPW behaviors in its adapter.
This change closes those engine API gaps without changing FPW main, the existing
standalone defaults or the working-space research decision. It does not claim
that FPW has already switched to this revision.

## Implemented contracts

| Contract | Main files | Behavior preserved |
| --- | --- | --- |
| Accepted stock setup | `core/src/stock_setup.rs`, `core/src/pipeline.rs` | FPW halation constants before calibration, accepted slide DIR afterward; explicit `fpw-accepted-stock-v1`, defaults unchanged |
| Frozen calibrated output | `core/src/calibrated_output_v1.rs`, `shaders/wgsl/physical_output_v1.wgsl` | Pro400H/X-Tra calibration; inversion → RGB16 → legacy finish → region reduction; same encoder, owned output textures |
| Probe isolation | `gpu/src/render.rs`, `gpu/src/wgpu_backend.rs` | Same device and compiled pipelines, private probe work buffers, no photo cache/ticket/counter mutation |
| Host submission gate | `gpu/src/wgpu_backend.rs` | Optional existing `Arc<Mutex<()>>`, only around queue submission; cancellation rechecked after acquiring gate |
| Resource identity | `tools/validation/check_fpw_resource_contract.py`, resource manifest | 55 shared engine files verified against FPW pins and Git blobs; installed-directory mode available |
| Public integration consumer | `core/examples/fpw_host_contract.rs` | Five accepted stock/route combinations using public APIs, shared host handles and full/finished region output |

The `core`, `gpu` and `shaders` paths above are under `crates/spektrafilm-*`.
Detailed contracts are linked from [HOST_GPU_INTEGRATION_CONTRACT.md](HOST_GPU_INTEGRATION_CONTRACT.md).

## Verification actually completed locally

The local GPU is Mesa software Vulkan (`llvmpipe`, LLVM 19.1.7), using the same
wgpu 30.0.1 build as the migration. This is execution of Vulkan compute commands,
not dedicated NVIDIA/AMD hardware qualification.

- **Stock setup:** four module tests and four integration tests passed. Coverage
  includes every metadata branch, all 22 filming profiles, seven accepted
  constructor combinations, resolved runtime parameters and calibration data,
  unsupported slide rejection, ordinary defaults and `with_params` preservation.
- **Calibrated output:** five tests passed. The original FPW CPU source and
  original WGSL are compiled independently as frozen references. CPU inversion
  matches the old scalar output; independent Python heldout error stays within
  `2e-6`; the original and new GPU output codes match for all tested stock,
  finish and region combinations. The existing CPU/GPU negative-output bound
  of three RGB16 codes is retained. Full-chain failures do not publish density;
  retry/store/hit and output ownership are checked.
- **Probe isolation:** one full regression covers the 1024×8 latitude-shaped
  probe, both directions of nested calls, overlapping photo/probe requests and
  success/error/cancellation. Retained photo resources and later pixels survive.
- **Submission gate:** one full regression covers all four production submit
  entry points, held-gate ordering, film-consumer access, explicit wait outside
  the gate, blocked cancellation, no marker execution/cache publication and retry.
  Three existing host-device tests also passed.
- Existing full-chain work-buffer, density and scoped-output regressions passed
  after the probe change (two, one and four tests respectively).
- The public host example ran paper, Provia, Velvia, Pro400H negative and X-Tra
  negative. Each produced full 17×11 and finished 7×5 region textures on the
  injected device; the second paper request reused the density snapshot.
- All 55 engine resource checks, three calibrated artifacts and seven frozen
  source/reference hashes passed. The checker also rejects content changes and
  missing files, with a strict text-only CRLF rule; binary LUT bytes are exact.
- f64 core/GPU and CPU-only GPU library builds passed. No dependency versions or
  Cargo.lock entries changed. Existing Rust 1.99 workspace minimum is now
  inherited by the six package manifests instead of remaining unused metadata.

The exact submitted implementation must also pass the Windows/Linux workflow.
The PR records that run and its tested commit; an earlier PR's CI is not evidence
for this change. Linux CI executes the new contracts and public example in
addition to the complete 1,215-case IIR and earlier GPU regressions.

## Reproduction

On Windows with the checked-in lockfile:

```powershell
cargo test --locked -p spektrafilm-core --test stock_setup
cargo test --locked -p spektrafilm-core --test calibrated_output_v1 cpu_
cargo check --locked -p spektrafilm-core --example fpw_host_contract
python tools/validation/check_fpw_resource_contract.py
python scripts/verify_calibrated_output_sources.py
```

On the qualified Linux environment with system OpenBLAS and Vulkan available:

```sh
export SPEKTRAFILM_REQUIRE_GPU=1
cargo test --locked -p spektrafilm-core -p spektrafilm-gpu \
  --features openblas-src/system --lib stock_setup
cargo test --locked -p spektrafilm-core -p spektrafilm-gpu \
  --features openblas-src/system --test stock_setup --test calibrated_output_v1 \
  --test probe_isolation --test submission_gate
cargo build --locked -p spektrafilm-core -p spektrafilm-gpu \
  --features openblas-src/system --example fpw_host_contract
./target/debug/examples/fpw_host_contract
```

The local runs used the same commands with `--offline` where appropriate, and
the explicitly configured Mesa/OpenBLAS runtime from the prior validation.

## Open-source delivery

The README now identifies FPW as the primary acceptance target, credits the
original Rust port and Python/SpektraLab sources, describes current Vulkan/Rust
requirements, and labels old upstream performance/parity numbers as historical.
It no longer promises current 60 fps or Metal support that this configuration
does not establish. The complete upstream CC BY-SA 4.0 data license is preserved
byte-for-byte; the calibrated data has attribution, provenance and a changelog.
`.gitattributes` preserves the frozen source bytes on Windows as well as Linux.
No FPW UI, private photo or external digital-LUT payload is bundled.

## What is not yet complete

1. The production FPW thin adapter and dependency pin have not switched. The
   [migration plan](FPW_PARITY_MIGRATION_PLAN.md) maps its existing calls to the
   new APIs and keeps recipe/undo/application state in FPW.
2. The actual installed `engine_resources` directory is unavailable here. Run
   the resource checker with `--data-dir` against that directory. FPW's old
   manifest-identity check alone does not verify its actual files. ICC delivery
   and external digital LUTs remain host responsibilities.
3. Frozen six-photo, dedicated RTX/AMD, real surface/zoom/rotation/undo/export,
   submitted cancellation and device-loss recovery acceptance are still needed.
4. The original FPW reduced CPU negative fallback can resize before nonlinear
   inversion, while its GPU route inverts first. This implementation preserves
   the accepted GPU order and full-size CPU inversion. Panorama/fallback adoption
   needs explicit comparison; no silently equivalent route is claimed.
5. This does not pool all auxiliary GPU resources or establish a measured
   performance gain. Wider-gamut/ACES/OpenDRT decisions remain separate research.

No database, schema, resource-number or existing profile changes were made.
