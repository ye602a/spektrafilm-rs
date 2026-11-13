# Python reference helper

`spektra_compare.py` generates a shared input TIFF and a Python reference render
for a manual comparison with Rust. It requires a separate checkout of the
[upstream Python implementation](https://github.com/andreavolpato/spektrafilm).
It is not part of this repository's automated regression suite.

## Setup

Use the upstream project's supported Python environment and dependencies. Pin
the upstream revision used for a comparison. For a sibling checkout named
`spektrafilm`, install it in an isolated environment from this repository root:

```bash
python -m venv .venv
# Activate .venv using the command for your shell, then:
python -m pip install -e ../spektrafilm
```

## Generate reference files

```bash
python scripts/parity/spektra_compare.py path/to/input.dng target/parity/reference \
    kodak_gold_200 fujifilm_crystal_archive_typeii 1500
```

The arguments are the RAW input, output directory, film, paper and maximum image
dimension. The final three are optional. The helper writes a linear ACES2065-1
32-bit float TIFF and an 8-bit Python output PNG.

Render that exact TIFF in Rust with the same film and paper, explicitly selecting
`io.input_color_space = "ACES2065-1"` and `io.input_cctf_decoding = false`.
Match exposure, effects, calibration, precision and output settings before
interpreting a difference. Record both revisions, dependency versions and
parameters with the result; this helper does not enforce their equivalence.

Current engine regressions live under `crates/*/tests/` and run through
[the standalone smoke workflow](../../.github/workflows/standalone-engine-smoke.yml).
