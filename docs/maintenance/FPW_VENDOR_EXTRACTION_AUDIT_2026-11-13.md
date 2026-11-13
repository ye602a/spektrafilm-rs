# FPW vendored Spektrafilm-rs: extraction audit

Status: **read-only source audit complete; no engine code ported; build and image-parity gates pending**  
Date: 2026-11-13  
Public fork: [ye602a/spektrafilm-rs](https://github.com/ye602a/spektrafilm-rs)  
Upstream: [turbasvin/spektrafilm-rs](https://github.com/turbasvin/spektrafilm-rs)  
Comparison base: `9dd59b0380194b93686aaa230a8bb9680aa270a4` (upstream / fork `main`)  
FPW source snapshot: `ae3428acd98e39c48784a79dc8f3369ab1236cb1` (`main`). The later `codex/settings-resources` snapshot at `41123ea81d982283990019a22f0dcb55d9b00a33` contains the same vendored Spektrafilm blobs.

## Purpose and boundary

The standalone Spektrafilm-rs fork is an independent physical-film research engine. FPW is a separate photo-processing application and should eventually consume a pinned, tested RS revision. This document audits the **engine** delta only; it does not import FPW application code, private photographs, private configurations or binary test outputs.

The original vendoring provenance file records `27ca80e3edab27f99abcc8c2860f5cc3617775f3`, but GitHub currently returns **No commit found** for that SHA. Do not rewrite it as a verified ancestor. Treat the publicly reachable upstream `main` SHA above as a **comparison base**, not proof of the original import ancestry. Recover the import history independently before representing the patch stack as exact cherry-picks.

## Method and inventory

Compared GitHub repository trees, and for **81 overlapping code/configuration/README paths**, fetched both UTF-8 versions and compared after CRLF→LF normalization and removal of trailing EOF whitespace. This distinguishes meaningful changes from Windows line-ending changes. Also compared all **31 film/paper JSON profiles** and **15 optical-filter/auxiliary data text files** with the same normalization; every one matched. Binary resources were not independently re-hashed in this audit; preserve them byte-for-byte and verify retained resource manifests during extraction.

The vendored source contains **159 Git blobs**, versus **157** in the upstream tree:
- 7 matching Git blob SHA values; 139 differently hashed common blobs (many are only CRLF/LF differences).
- 13 files exist only in the vendored directory.
- 11 upstream files are absent from the vendored directory (mostly packaging, assets, CI and parity scripts).
- After actual text normalization, only **12 of the 81 audited overlapping code/config/README paths differ materially**.
- `Cargo.lock` differs due to dependency updates and needs a full lockfile/dependency graph review; it is **not included** in the 81-file logical-code tally.

### Twelve materially modified existing paths

| File | Main difference | Recommended disposition |
| --- | --- | --- |
| `Cargo.toml` | Pinned dependency versions; wgpu 24 → 30.0.1, Vulkan-only selection, rand and related updates | Keep as an isolated dependency/API migration; test portability |
| `crates/spektrafilm-core/src/params.rs` | Optional grain `output_sampling` parameter | Keep experimental and optional; avoid hard-wiring FPW policy |
| `crates/spektrafilm-core/src/pipeline.rs` | Resident pipeline outputs/cancellation, multilayer grain preparation and host-consumer interfaces | Split into smaller engine patches |
| `crates/spektrafilm-gpu/Cargo.toml` | Dependency pinning, wgpu/BLAS/CUDA adapter updates | Follow dependency migration |
| `crates/spektrafilm-gpu/src/lib.rs` | New layered-grain structures and packed-RGB16 backend hook | Candidate general-purpose API |
| `crates/spektrafilm-gpu/src/wgpu_backend.rs` | Shared Device/Queue, source coordinates, bounded negative cache, reusable staging/work images, resident output hooks, RGB16 path, cancellation | Split by ownership and test each |
| `crates/spektrafilm-math/Cargo.toml` | Dependency pin (half) | Dependency-only |
| `crates/spektrafilm-math/src/numpy_rng.rs` | Removed obsolete rand trait import | Dependency-only |
| `crates/spektrafilm-math/src/stats.rs` | rand `RngExt` import | Dependency-only |
| `crates/spektrafilm-model/Cargo.toml` | rand_mt dependency update | Dependency-only |
| `crates/spektrafilm-shaders/wgsl/grain.wgsl` | Introduces source-anchored grain indexing | General algorithm, requires geometry parity |
| `crates/spektrafilm-shaders/wgsl/scan_spectral.wgsl` | Removes per-wavelength NaN guard in favor of expected host-side sanitization | **Do not merge without a non-finite-input regression** |

The existing spectral reconstruction, profile interpretation, film-development, printing, scanning, CPU backend and CUDA backend implementation files were text-identical to the upstream comparison base. This is **not** evidence that full image outputs are identical: added GPU functionality can intentionally change the output.

### Thirteen vendored-only paths

**Engine implementation and shaders (9):**
- `crates/spektrafilm-core/src/grain_layers.rs`
- `crates/spektrafilm-gpu/src/film_display.wgsl`
- `crates/spektrafilm-gpu/src/film_quantize.wgsl`
- `crates/spektrafilm-gpu/src/grain_iir.rs`
- `crates/spektrafilm-gpu/src/layered_grain.rs`
- `crates/spektrafilm-shaders/vulkan/grain_iir.comp`
- `crates/spektrafilm-shaders/vulkan/grain_iir.spv`
- `crates/spektrafilm-shaders/wgsl/grain_layers.wgsl`
- `crates/spektrafilm-shaders/wgsl/grain_reference.wgsl`

**Provenance / resources (4):**
- `FPW_SOURCE.json`
- `SPEKTRAFILM_DATA_LICENSE.txt`
- `data/RESOURCE_PIN.json`
- `data/sRGB.icc`

The new GPU multilayer grain explicitly attributes its reference to **SpektraLab**; retain exact license/commit attribution and audit third-party distribution terms before publishing the code.

### Eleven upstream-only paths missing from FPW's vendor copy

`.github/workflows/release.yml`, `.gitignore`, `assets/spektrafilm-icon.jpg`, `docs/card.jpg`, `scripts/package_macos_app.sh`, `scripts/package_windows_app.ps1`, and `scripts/parity/{README.md,py_bisect.py,py_stages.py,rs_bisect.rs,spektra_compare.py}`.

**Do not delete these upstream files in the independent fork.** A standalone project benefits from retaining CLI, GUI, release workflows and parity tooling even when FPW did not vendor them.

## Extraction plan (staged; not applied)

1. **Upstream provenance/lockfile gate** — preserve the upstream `main` as baseline; document the unreachable import SHA; review the Cargo.lock change, Rust/wgpu compatibility and binary-resource hashes.
2. **Minimal general-purpose GPU API** — external `Device`/`Queue` injection and ownership, host-neutral output consumer, optional regional/RGB16 readback and cancellation, without FPW-specific device locks or recipe types.
3. **Source-anchored grain** — add source geometry with explicit defaults that preserve original pixel output when disabled; test crop, rotation, tiling, scale and signed mirror/reflection boundaries.
4. **Multilayer physical grain** — isolated feature and shader modules, with SpektraLab credit and CPU/reference quality gates. Avoid presenting visual plausibility as physical measurement.
5. **Performance features** — bounded post-grain/pre-print density cache, pooled work buffers / staging and optional Vulkan wide-grain IIR; use valid parameter dependencies and hard memory limits.
6. **Experimental output-grain sampling** — explicit opt-in, versioned, output-size-aware, with no changes to the default physical route.
7. **Standalone verification** — keep RS CLI/GUI independently buildable; CPU f64 parity, GPU deterministic tests, static assets, Linux/macOS/Windows portability as available, high-resolution and cancellation tests; only then mark an RS tag suitable for FPW.
8. **FPW pin** — downstream FPW consumes a tested RS Git revision; do not migrate the FPW vendor directory until independent RS builds and regression images are validated.

## Current blockers and non-goals

- **No builds, GPU benchmarks or image-parity tests were run in this audit.** This is source inspection, not production qualification.
- The upstream source SHA recorded by FPW is unavailable via the public GitHub API; it must not be silently replaced in historical provenance.
- The Vulkan-only SPIR-V passthrough path may limit GPU portability; avoid breaking the CPU reference or default portable backends.
- The removal of spectral NaN guards must be tested for incomplete/non-finite upstream data, not assumed equivalent.
- Per-photo cache keys, FPW output recipes, masks, RAW pipelines and scheduling remain FPW responsibilities.
- The standalone fork's future scientific plan may evaluate OFX and other engines, but does not adopt their numerical results or code without independent tests and appropriate licensing.

**Scope of this branch:** documentation-only. The independently maintained engine code remains identical to upstream on this branch.
