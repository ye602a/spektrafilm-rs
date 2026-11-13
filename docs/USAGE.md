# Build and usage

Run commands from the repository root. Examples use a POSIX shell; on Windows,
use `.exe` binaries, PowerShell environment syntax, and one-line commands.

## Requirements

- Rust 1.99 or later, with the platform's C/C++ build tools.
- A compatible GPU/driver for the chosen backend. Vulkan is the engine path
  exercised by the required Linux compute tests.
- Linux uses OpenBLAS and needs a native build toolchain, including a Fortran
  compiler when building OpenBLAS from source. The GUI also needs the usual
  X11/Wayland development libraries.
- Windows uses the CPU fallback implementation without a system OpenBLAS install.
  macOS CPU builds use Accelerate.

The lockfile pins wgpu 30.0.1 and eframe/egui 0.37.0. The engine dependency enables
Vulkan explicitly; building the GUI can enable additional wgpu backends through
Cargo feature unification. Those backends are not qualified by the Vulkan suite.

## CLI

```sh
cargo build --locked --release -p spektrafilm-cli --bin spektrafilm
./target/release/spektrafilm --help
./target/release/spektrafilm list-profiles --data-dir data
./target/release/spektrafilm process input.tif -o output.png \
    --film kodak_portra_400 --paper kodak_portra_endura --data-dir data
```

`--params settings.json` accepts runtime parameter overrides. `--scan-film`
selects direct film scanning instead of printing; use it only with the intended
profile and output contract. The generic CLI does not automatically select the
versioned host stock/calibrated-output setup.

The CLI accepts supported image and camera RAW formats through its existing
decoders. Camera/model coverage depends on those decoders. Run `process --help`
for available options.

### f64 CPU binary

Build separately to keep the default f32 applications intact:

```sh
cargo build --locked --release -p spektrafilm-cli \
    --features precision-f64 --bin spektrafilm-f64 --target-dir target/f64
SPEKTRAFILM_BACKEND=cpu ./target/f64/release/spektrafilm-f64 \
    process input.tif -o output.png --film kodak_portra_400 \
    --paper kodak_portra_endura --data-dir data
```

In PowerShell, set `$env:SPEKTRAFILM_BACKEND = "cpu"` before invoking
`target\f64\release\spektrafilm-f64.exe`.

## Preview GUI

```sh
cargo build --locked --release -p spektrafilm-gui
cargo build --locked --release -p spektrafilm-cli --bin decode_raw_gui
./target/release/spektrafilm-gui
```

The GUI provides image loading, profile selection, effect controls, zoom/pan and
preview saving. **Save** writes the preview; **Export** reruns processing through
the f64 CPU helper. Build the f64 binary above and point `SPEKTRAFILM_F64_CLI` to
its path before launching the GUI. The helper can also be found next to the GUI
or on `PATH`.

For a portable package, see [the Windows script](../scripts/package_windows_app.ps1)
and [the macOS script](../scripts/package_macos_app.sh). Platform packaging is
separate from numerical and interactive-GUI qualification.

## Backends

`SPEKTRAFILM_BACKEND` selects `wgpu`, `cpu`, or the optional `cuda` backend.
CUDA is experimental and is not covered by the current Vulkan compatibility
results. Building it does not establish parity with the versioned host routes.

```sh
cargo build --locked --release -p spektrafilm-cli --bin spektrafilm \
    --features spektrafilm-gpu/cuda-backend
```

CUDA requires the NVIDIA driver and CUDA 12 NVRTC runtime libraries. Select a
zero-based device with `SPEKTRAFILM_CUDA_DEVICE` when needed.

## Embed the engine

Start with the [host integration contract](maintenance/HOST_GPU_INTEGRATION_CONTRACT.md)
and [public example](../crates/spektrafilm-core/examples/fpw_host_contract.rs).
The example uses bundled profiles and the engine's public APIs:

```sh
cargo run --locked -p spektrafilm-core --example fpw_host_contract
```

Keep the engine revision, resource identity and output route together when
integrating it into an application. See [validation](maintenance/VALIDATION.md)
for what has been tested and what requires application-specific acceptance.
