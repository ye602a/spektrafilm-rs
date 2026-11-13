# Engine validation

## Boundary review (2026-11-13)

The follow-up review adds checks for malformed resident input, nonempty metering
grids for thin images, missing Vulkan IIR capability, unsupported CUDA multilayer
selection, cancellation raised after GPU completion, and failed Windows builds
leaving old executables available. These are input, capability and publication
boundaries; the accepted shader kernels, profiles and calibration tables are
unchanged.

Local verification used Rust 1.100.0, wgpu 30.0.1, system OpenBLAS and Mesa 25.0.7
llvmpipe Vulkan (LLVM 19.1.7), with `SPEKTRAFILM_REQUIRE_GPU=1`:

- All 121 math/model/core/GPU library and integration tests passed. The one
  ignored IIR test was run explicitly: all 1,215 cases passed with maximum
  absolute error `1.1920928955078125e-7` and the original tolerance.
- The CUDA selection check passed without initializing CUDA, and both resident
  input tests passed again with `precision-f64`.
- The public host example completed all five routes, including full and regional
  output on the injected device. The 55-resource, 3-calibration and 7-source-hash
  checks passed with unchanged content identities.
- Separately compiled pre-review main `fc0e4a985de62cae388c5ceb8069aa383de401bc`
  and the candidate produced identical inputs and resolved parameters for all
  eight migration-probe cases. All 5,952 f32 values matched byte-for-byte.

CI also runs the Windows packaging regression in
[`package_windows_failures.ps1`](../../scripts/tests/package_windows_failures.ps1).
It deliberately fails each of the four build positions across directory and AIO
packaging and checks that processing stops, old executables and the previous
package stay intact, and the embedding environment is restored. This is failure
handling coverage, not installation or launch qualification of a native package.

## Recorded baseline

[PR #9](https://github.com/ye602a/spektrafilm-rs/pull/9) merged as
`1cd33b5af4a1a8dd48644962b66ea5b7e787c313`. Its implementation tree passed
[Windows and required Linux Vulkan CI](https://github.com/ye602a/spektrafilm-rs/actions/runs/43434521405)
on 2026-11-13. The workflow is
[standalone-engine-smoke.yml](../../.github/workflows/standalone-engine-smoke.yml).

| Check | Observed result |
| --- | --- |
| Windows standalone applications/API | Builds passed; 76 core/math/model library tests passed |
| Precise Vulkan IIR | 1,215 cases passed; maximum absolute error `1.1920928955078125e-7` |
| Work-plane reuse and density cache | Exact cache-on/off output, concurrent ownership and invalidation regressions passed |
| Generic output | RGB16 threshold/padding, crop/reduction, retained texture and failure/cancellation tests passed |
| Stock setup | 4 unit and 4 integration tests passed, preserving initialization order and ordinary defaults |
| Calibrated output | 5 tests passed against independent original CPU/GPU references and heldout data |
| Probe isolation and submission gate | Both GPU integration regressions passed |
| Public host example | Paper, two slide and two calibrated-negative routes produced full and regional outputs |
| Resource/source checks | 55 shared resources, 3 calibration artifacts and 7 frozen source hashes passed |

Linux CI used Mesa llvmpipe Vulkan (LLVM 20.1.2), with
`SPEKTRAFILM_REQUIRE_GPU=1`; GPU absence was a failure. Local validation used
Rust 1.100.0, wgpu 30.0.1 and Mesa 25.0.7 llvmpipe (LLVM 19) on Debian 13.
Local f64 core/GPU and CPU-only GPU-crate checks also passed.

The calibrated-output tests compile the original scalar and GPU references
independently. Original/new GPU RGB16 and RGBA8 codes match across stock,
finish and region cases. The Python-heldout tolerance remains `2e-6`, and the
negative CPU/GPU comparison retains its original maximum of 3 RGB16 codes.
See [output validation details](CALIBRATED_OUTPUT_V1.md).

An earlier separately compiled RS revision,
`85eb835a0116b8ef77beb4952ae80272a21b0b8a`, supplied eight synthetic full-chain
baselines. All 5,952 f32 values matched byte-for-byte after checking identical
inputs, resolved parameters, geometry and source transforms. The preserved
[machine-readable record](validation/20261113_gpu_migration.json) contains the
hashes and scope; it is an engine comparison, not a full application benchmark.

A separate compiled-reference comparison also matched all **41,295 f32 channels**
across ten synthetic full-chain cases (five routes with local/source sampling),
with maximum absolute error and RMSE both zero. The
[archived qualification record](https://github.com/ye602a/spektrafilm-rs/blob/7f156e2b437a7667f82a0b19e1cde93413988122/docs/maintenance/validation/fpw_vendor_20261113/REPORT.md)
retains source identities, dependency locks and reproduction steps. This was a
fresh build of the pinned reference numerical sources on software Vulkan, not
validation of a deployed application binary.

## Reproduce

For Linux with system OpenBLAS:

```sh
python tools/validation/check_fpw_resource_contract.py
python scripts/verify_calibrated_output_sources.py

SPEKTRAFILM_REQUIRE_GPU=1 cargo test --locked \
  -p spektrafilm-math -p spektrafilm-model -p spektrafilm-core -p spektrafilm-gpu \
  --features openblas-src/system --lib --tests

SPEKTRAFILM_REQUIRE_GPU=1 cargo test --locked \
  -p spektrafilm-gpu --features openblas-src/system --lib \
  actual_gpu_iir_matches_f64_reference -- --ignored --nocapture

cargo test --locked -p spektrafilm-gpu \
  --features openblas-src/system,cuda-backend --lib \
  multilayer_grain_selection_is_rejected_without_initializing_cuda

cargo build --locked \
  -p spektrafilm-core -p spektrafilm-gpu --features openblas-src/system \
  --example fpw_host_contract
SPEKTRAFILM_REQUIRE_GPU=1 target/debug/examples/fpw_host_contract
```

Other supported platforms use their normal BLAS configuration. For cross-version
pixel checks, run the same
[migration probe](../../crates/spektrafilm-core/examples/migration_probe.rs)
in separate baseline/candidate worktrees and compare the resulting directories:

```sh
python tools/validation/compare_migration_probe.py /path/to/baseline /path/to/candidate
```

The comparator rejects changed inputs or parameters before reporting parity.

## Scope

Small synthetic images and software Vulkan establish the tested numerical and
ownership behavior. They do not establish production-image acceptance,
dedicated-hardware results, real window interaction or device recovery.
Cancellation regressions cover rejection before submission and cancellation
raised by the submitted command's completion callback. The latter confirms that
destination writes can already have executed while the request still returns an
error and publishes no density; a subsequent successful render recovers. It does
not establish interruption of GPU execution or device-loss recovery.
Destinations may be published only after the whole render succeeds. Host adoption
follows the [compatibility checks](FPW_PARITY_MIGRATION_PLAN.md).
