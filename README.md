![spektrafilm-rs](docs/card.jpg)

# spektrafilm-rs

**English** | [简体中文](README.zh-CN.md)

A Rust engine for physically based film, print and scan simulation, with spectral
processing and GPU acceleration. Based on the Rust port by
[turbasvin](https://github.com/turbasvin/spektrafilm-rs) of
[Andrea Volpato's spektrafilm](https://github.com/andreavolpato/spektrafilm).

Development is driven by **FPW**, a photography application that is **not yet
public**. The film engine is maintained here and can be built and used independently.

## Development goals

- Improve the physical film pipeline while preserving established rendering results.
- Keep film algorithms and GPU resource management in one engine, with documented host interfaces.
- Verify numerical and performance changes with reproducible comparisons; version changes that affect the look.

## Capabilities

- Spectral film exposure, development, printing and scanning, with bundled film and paper profiles.
- Halation, DIR couplers, physical grain, glare and optical effects.
- f32 GPU processing and an f64 CPU reference path.
- Host-provided GPU devices, source-coordinate grain, bounded caches, and GPU / RGB16 / region output.
- Standalone CLI and preview GUI.

## Get started

Requires Rust **1.99+** and the platform's native build tools. Vulkan is the
validated physical-engine GPU backend. Windows builds and Linux software-Vulkan
compute are covered by CI; other platform/backend combinations need separate
qualification. See [build and usage](docs/USAGE.md) for native dependencies,
GUI/export setup and the experimental CUDA option.

```sh
git clone https://github.com/ye602a/spektrafilm-rs.git
cd spektrafilm-rs
cargo build --locked --release -p spektrafilm-cli --bin spektrafilm
./target/release/spektrafilm list-profiles --data-dir data
./target/release/spektrafilm process input.tif -o output.png \
    --film kodak_portra_400 --paper kodak_portra_endura --data-dir data
```

On Windows, use `target\release\spektrafilm.exe` and run the process command on one line.

## Documentation

- [Build and usage](docs/USAGE.md)
- [Host GPU integration](docs/maintenance/HOST_GPU_INTEGRATION_CONTRACT.md)
- [Public integration example](crates/spektrafilm-core/examples/fpw_host_contract.rs)
- [Compatibility and development plan](docs/maintenance/FPW_PARITY_MIGRATION_PLAN.md)
- [Validation scope and evidence](docs/maintenance/VALIDATION.md)

## Credits and licenses

The original Rust implementation is by [turbasvin](https://github.com/turbasvin/spektrafilm-rs).
The film model and spectral/profile data originate from
[Andrea Volpato's spektrafilm](https://github.com/andreavolpato/spektrafilm).
Spectral upsampling builds on work by [Johannes Hanatos](https://github.com/hanatos).
Numerical routines draw on NumPy, SciPy, scikit-image and colour-science;
[multilayer grain and precise IIR provenance](docs/maintenance/GRAIN_MULTILAYER_MIGRATION.md)
includes the SpektraLab contribution.

Code: [GPL-3.0](LICENSE). Spectral data, profiles and their derivatives:
[upstream attribution and CC BY-SA 4.0 terms](SPEKTRAFILM_DATA_LICENSE.txt).
[Versioned calibration provenance](data/negative_scan_ap0_v1/PROVENANCE.md)
is retained alongside the data.
