# Frozen calibrated negative output v1

`spektrafilm_core::calibrated_output_v1` owns FPW's accepted calibrated negative
inversion, finish exposure, quantization and display sampling. The host supplies
its existing device and recipe choices, then retains engine-produced textures.
It does not need another film inversion or final output shader.

This is the existing `negative_scan_ap0_v1` route, frozen from FPW
[`ae3428acd98e39c48784a79dc8f3369ab1236cb1`](https://github.com/ye602a/fursuit-photo-workflow/tree/ae3428acd98e39c48784a79dc8f3369ab1236cb1/native/crates/fpw-runtime/src).
AP0 is the established transmission transport for these two calibrations. It is
not a new engine working-space selection or an implementation of the separate
wide-gamut research. An algorithm/table change requires a new route version.

## Source and output contract

| Mapping | Required complete physical-chain source | Result |
| --- | --- | --- |
| `PhysicalOutputMappingV1::EncodedSrgb` | Full-resolution encoded sRGB, with `FilmOutput.cctf_encoding == true` | Positive sRGB codes; no additional inversion or transfer encoding |
| `PhysicalOutputMappingV1::NegativeScanAp0(NegativeStockV1::Pro400H)` | The accepted Pro400H linear AP0 transmission scan, `scan_film == true`, `output_cctf_encoding == false` | Frozen Pro400H calibrated positive sRGB |
| `PhysicalOutputMappingV1::NegativeScanAp0(NegativeStockV1::XTra400)` | The accepted X-Tra 400 linear AP0 transmission scan, with the same route requirements | Frozen X-Tra calibrated positive sRGB |

Stock IDs are exactly `fujifilm_pro_400h` and `fujifilm_xtra_400`. Converting
another stock string into `NegativeStockV1` returns `UnsupportedStock`; there is
no nearest-stock fallback. The calibration must match the physical film profile.
The scoped buffer contains no stock or color-space metadata, so callers remain
responsible for that match. The safe complete-render wrapper additionally checks
the scan and transfer flags; the encoding method checks the supplied transfer
flag and device identity.

The accepted negative route retains its fixed spectral/response settings and
original DIR, disables camera diffusion, scan glare, transport gamut mapping and
transport sharpening, and applies the v1 inversion/toe/shoulder/gamut mapping
once. `AcceptedStockSetup::FpwV1` supplies the accepted stock constants; the
host's persisted recipe supplies the route/transport controls. See the
[public host example](../../crates/spektrafilm-core/examples/fpw_host_contract.rs)
and [accepted stock contract](FPW_ACCEPTED_STOCK_SETUP_V1.md).

Both output textures have the requested positive dimensions:

| Field | Format | Meaning |
| --- | --- | --- |
| `PhysicalOutputTexturesV1::precision` | `Rgba16Uint` | Final RGB16 integer codes, alpha 65535 |
| `PhysicalOutputTexturesV1::display` | `Rgba8Unorm` | Encoded sRGB display codes derived as `(rgb16 + 128) / 257`, alpha 255 |

The display texture's channels already contain encoded sRGB. A host must not
apply a second sRGB transfer to these values. The precision texture uses an
integer format so the accepted RGB16 codes survive exactly. Texture usage is
`STORAGE_BINDING | TEXTURE_BINDING | COPY_SRC`. Host display/export consumers
can retain or read these textures after successful completion.

## Exact order and regional semantics

The full physical film chain completes before this consumer runs. The retained
FPW shader then performs these operations, in order:

1. Read a source pixel at its full-resolution output coordinates.
2. For a calibrated negative, invert transmission, apply the frozen AP0-to-sRGB
   matrix, toe neutralization, shoulder, symmetric gamut mapping and sRGB transfer.
3. Quantize the resulting positive RGB to RGB16 with the original integer
   significand algorithm, equivalent to f64 multiplication and rounding of the
   f32 input. This avoids an extra f32 multiply changing threshold decisions.
4. Apply the original 65,536-entry finish table when `finish_centi_ev != 0`.
   The table decodes each positive code, multiplies linear exposure by
   `2^(centi_ev/100)`, re-encodes, clamps and rounds.
5. Apply the display crop and optional area reduction to the finished RGB16
   source pixels, with the original f32 ceil bounds, row integer sums and
   final rounding. A 1:1 crop copies the selected codes directly.
6. Store RGB16 and derive the RGBA8 display result.

`PhysicalOutputV1::render` calls `with_film_output` with **no engine region**.
It passes `PhysicalOutputOptionsV1::region` only to the calibrated consumer.
An advanced host using `encode` must follow the same rule. Reducing negative
transmission before inversion, or reducing positive pixels before finish,
changes this nonlinear order. Calling the generic `run_film_chain_rgb16` first
also quantizes the wrong intermediate for a calibrated negative.

`DisplayRegion` uses full-resolution `[x, y, width, height]` coordinates and
accepts only 1:1 output or reduction. Invalid or out-of-bounds rectangles,
enlargement, zero dimensions, shader addressing overflow and unsupported row
accumulation sizes return explicit errors. The retained u32 row accumulator
supports at most 65,537 maximum-valued samples per reduction row; an actual wider
block is rejected. Source/output buffer and texture/dispatch device limits are
also checked. The shader itself is unchanged.

### Observed original CPU fallback discrepancy

The pinned FPW `film.rs` reference/oversize fallback obtains a resident display
plane before its CPU calibrated inversion. With a reduced region, that route can
reduce negative transmission before inversion. Its later CPU finish stage can
also occur after display reduction. This ordering differs from the accepted GPU
`physical_output.wgsl` route described above.

This migration preserves the accepted GPU order and records that discrepancy.
It does not claim that the original reduced CPU fallback is identical, silently
rewrite the original behavior, or qualify every FPW export/oversize fallback.
`positive_scan_v1` and `positive_rgb16_v1` are deliberately full-size CPU APIs.
A CPU consumer that needs GPU-compatible regional semantics must invert the
full plane, quantize/finish, then crop/reduce the resulting codes. The new tests
use that order for their independent CPU comparison.

## Ownership and the publication boundary

Create one `PhysicalOutputV1::new(&host_device)` for a device and reuse it. It
retains one compiled compute pipeline, two immutable calibration buffers and at
most one current finish table. A warm render with the same finish does not
rebuild or upload its 65,536 entries. A finish change replaces one cached buffer;
commands already recorded retain their own resource handles. Calibration data
is parsed once per stock. This is a small per-output-object cache, not another
engine work-plane pool.

`encode(&self, device, encoder, FilmOutput, options)` records commands into the
encoder supplied by the scoped film callback. It does not submit, wait or read
the physical float plane back to the CPU. Each result owns fresh display and
precision textures; it never retains a recycled engine work buffer. The same
object can encode subsequent frames without overwriting published textures.

An `encode` result means destinations and commands were created. **The host may
publish/display a frame only after the whole `PhysicalOutputV1::render` or
enclosing `with_film_output` call returns `Ok`.** The backend submits and waits
for the entire encoder, observes late cancellation, then commits density-cache
publication and returns the host-owned result.

The `render` wrapper propagates an encoding failure through the callback as a
backend error while preserving the original `CalibratedOutputError` for the
caller. This prevents an `Ok(Err(error))` callback from being mistaken for a
successful density-cache commit. Advanced callback callers must likewise return
an outer `Err` for encoding failure. After submission, cancellation cannot
undo GPU writes that already executed; it still prevents a successful returned
frame and density publication. The host must not publish an early captured
texture merely because the callback ran.

The existing backend submission gate serializes queue ownership. Keep using the
host's shared gate when injecting device/queue handles; this consumer creates
neither another device nor another queue and performs no hidden submission.

## CPU API

`positive_scan_v1(scan, stock_id, stop)` returns encoded positive sRGB f32 using
the original f64 CPU calibration math. It rejects incomplete RGB input,
nonfinite scan values, unsupported stocks and cancellation. Larger images use
the original bounded worker/chunk scheduling with cancellation checks.

`positive_rgb16_v1(scan, stock_id, finish_centi_ev, stop)` performs full-size CPU
inversion, original RGB16 rounding and optional finish. It returns interleaved
RGB codes without padding. `finish_exposure_table_v1(centi_ev)` exposes the
original table for consumers that already have positive encoded RGB16. Avoid
applying it again to GPU results, which already include the requested finish.

## Provenance and validation

FPW runtime source is GPL-3.0-or-later. The profile-derived calibration data
retains Andrea Volpato's CC BY-SA 4.0 attribution and the full preserved
[`SPEKTRAFILM_DATA_LICENSE.txt`](../../SPEKTRAFILM_DATA_LICENSE.txt).
The source shaders, extracted JSONs and independent original shader fixture are
byte-identical to the pinned FPW commit, including absence of a final newline.
Data hashes and original research-calibration identities are recorded in
[`data/negative_scan_ap0_v1/PROVENANCE.md`](../../data/negative_scan_ap0_v1/PROVENANCE.md).

| Frozen source | Original Git blob | Use in RS |
| --- | --- | --- |
| `negative_scan.rs` | `24e92fcc1ef7b3999d53a7eb2033c725acf0ab54` | Original scalar math in production; separately compiled scalar test reference |
| `physical_output.rs` | `144b7e54321df7a79243ec43ea4219d9736e8071` | Original node preparation, bindings, finish and texture semantics |
| `physical_output.wgsl` | `10f716f83285227875af6a726e1e89c1176b7c60` | Exact production shader and independent original-shader test fixture |
| `images.rs` | `1764d528f4141a1edbbbbcbf595dd5d4cda43683` | Original transfer/finish functions; separately compiled test reference |

The independent scalar fixture is the original `negative_scan.rs` prefix before
its `#[cfg(test)]` module, with only its two `include_str!` paths redirected to
the preserved JSONs. The original timing-dependent self-tests are not imported.
The test crate supplies the small original anyhow macro surface without changing
the math. The independent transfer fixture consists of the exact original
`encode_srgb`, `decode_srgb` and `finish_exposure_table` definitions. Formatting
is skipped for both frozen fixtures, and their hashes are checked.

`tests/calibrated_output_v1.rs` runs the following qualifications:

- CPU f32 outputs match the independently compiled original FPW CPU implementation
  exactly for both stock calibrations, including calibration knots and adjacent
  representable f32 values.
- The original Python-heldout fixture remains within the original `2e-6`
  encoded-channel tolerance. It is an independent oracle, not newly generated
  expected output from the implementation under test.
- Independently compiled original and production GPU consumers produce identical
  RGB16 and RGBA8 codes for encoded sRGB, both calibrated negatives, finish
  `-125/0/+125` centi-EV and full/cropped/reduced outputs.
- The original no-finish CPU/GPU envelope remains at most 3 RGB16 codes for the
  calibrated negative fixture, at most 1 for ordinary regional output and zero
  for ordinary full output. Finish-enabled outputs are compared exactly to the
  original GPU consumer; the CPU envelope is not silently widened.
- Heldout GPU output, tiny 1x1 output, invalid transport/geometry/input buffers
  and CPU stock/data/cancellation errors are exercised.
- Retained host textures remain unchanged after later renders, stock changes and
  finish-cache replacements on the same output object.
- A full paper-chain wrapper regression reaches a cold density miss, rejects
  malformed output geometry and publishes no density. Wrong transport and
  pre-cancellation also return errors without stores. A valid retry stores the
  density and a following request hits it, with identical output codes.
- A complete Pro400H physical render with multilayer grain supplies its linear
  AP0 plane to the independently compiled original CPU inversion. The new
  complete-render GPU result stays within the original 3-code RGB16 envelope.

Run the offline source gate and required GPU test with:

```sh
python scripts/verify_calibrated_output_sources.py
SPEKTRAFILM_REQUIRE_GPU=1 cargo test --locked \
  -p spektrafilm-core -p spektrafilm-gpu --features openblas-src/system \
  --test calibrated_output_v1
```

The explicit system-OpenBLAS feature is for the Linux validation environment;
other supported environments use their normal BLAS configuration. These checks
qualify the extracted output, heldout data and the focused complete-render
wrapper behavior. The public host example additionally exercises the accepted
paper, two slide and two calibrated negative routes. Real-photo acceptance
remains a distinct gate; synthetic fixtures do not certify all hardware backends
or every host fallback route.
