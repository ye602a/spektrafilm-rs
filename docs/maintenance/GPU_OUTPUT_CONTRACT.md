# GPU output, RGB16 and display-region contract

Date: 2026-11-13. Scope: the independently maintained RS engine only.

## Provenance and unchanged physical rendering

The output kernels come from the accepted FPW vendor route at
[`ae3428acd98e39c48784a79dc8f3369ab1236cb1`](https://github.com/ye602a/fursuit-photo-workflow/tree/ae3428acd98e39c48784a79dc8f3369ab1236cb1/native/vendor/spektrafilm).
The migrated files retain their exact Git blob identities:

| RS file | FPW Git blob |
| --- | --- |
| `crates/spektrafilm-shaders/wgsl/film_display.wgsl` | `99c58ba9e2e8514537395c805cbc1628883c6e4e` |
| `crates/spektrafilm-shaders/wgsl/film_quantize.wgsl` | `81903151d3a160002e6d187081063458f087eb74` |

The full-resolution film, DIR, grain, paper, scan, glare, sharpening and final
clamp/transfer passes execute before either output kernel. A display crop does
not change film geometry, grain source coordinates, pre-print density identity,
or the upstream optical calculation. The output contract retains the current
linear-sRGB input convention. The existing `output_cctf_encoding` option chooses
linear or sRGB-encoded final values; RGB16 describes storage precision and does
not select another color space or transfer function.

## DisplayRegion

`DisplayRegion { crop: [x, y, width, height], width, height }` describes a crop in
completed full-resolution output pixels and its requested output dimensions.
`validate(self, source_width, source_height) -> Result<(), OutputError>` rejects
empty dimensions, out-of-bounds/overflowing crops, enlargement, or RGB channel
counts that exceed the shaders' u32 indexing. The backend must additionally
respect the actual device's storage-buffer and dispatch limits. Invalid
requests return an error before GPU execution; they are not silently clamped.

Reduction also rejects any output block containing more than 65,537 samples
across one row, because the preserved shader accumulates that row's RGB16 codes
in u32. It evaluates the existing f32 block boundaries, so wide panoramas with
smaller blocks and 1:1 extraction remain valid. The caller can choose another
output size or a separate CPU path after this explicit error. RS does not
silently replace the accepted shader or change its arithmetic. Bounding the
full source RGB channel count also bounds the shader's integer row-major index
and sample-count products; its crop scaling itself uses f32 arithmetic.

At 1:1, the shader copies the original f32 samples exactly. Reduction preserves
FPW's existing discrete area-block rule: each output pixel covers integer
source samples between the ceiling of its scaled start and end coordinates.
The shader first converts those samples to RGB16 code values, averages them,
rounds the average and expresses it as f32 divided by 65535. This is neither a
bilinear resize nor a new high-quality resampling algorithm. Changing it would
be a separate output-parity decision. Enlargements are rejected because the
retained reduction kernel does not define them.

## RGB16

The final CPU result is row-major, channel-interleaved `[R, G, B, R, G, B, ...]`.
GPU storage packs two u16 channels per u32, low half first. An odd channel count
sets the unused high half of the last word to zero; that padding is not a fourth
channel and is removed from the returned pixel vector.

For every f32 input, the quantizer reproduces clamped
`round(f64(value) * 65535)`: negative values and NaNs map to zero, values at or
above one (including positive infinity) map to 65535. It uses integer operations
on the f32 significand. Replacing it with an f32 multiply can move samples
across half-code boundaries and change accepted export bytes. The shader's
two-dimensional dispatch and `row_stride` support outputs larger than one
dispatch row, subject to the device limits checked by RS.

## Scoped same-device consumption

A GPU consumer records work after the complete interleaved RGB f32 chain in
the same command encoder. RS owns the plane and its work-buffer lease. The
callback receives a scoped output view and an RS-owned command encoder; it
only records copies or computation into its own destination on the same
device. Pixels are not yet ready for CPU access or publication inside the
callback. RS submits and waits for that work before returning the callback
value and allowing the plane to return to the reusable work-buffer cache.

The caller publishes or displays its own destination texture/buffer only after
the whole method returns `Ok`. It retains that destination, not RS's source plane.
Saving a cloned source handle, recording a later submission that reads it, or
publishing an in-flight source plane violates the lease contract. Consumer
recording does not require CPU readback of the full f32 image. Cancellation or
an unsuccessful render returns an error and does not publish a density-cache
snapshot. A cancellation detected after submission cannot retract GPU writes
that already reached a host-owned destination; the caller must therefore wait
for `Ok` before publishing it. The encoding callback itself is not a publication
notification. `run_film_chain_region`, `run_film_chain_rgb16`, and
`with_film_output` share this implementation; the legacy full-f32 method stays
available. `Pipeline::with_resident_params` supplies the same prepared physical
parameters without duplicating pipeline construction in the host.

## Independent checks and remaining FPW acceptance

`cargo test --locked -p spektrafilm-gpu --test output_kernels` executes both
production shaders and checks:

- RGB16 values immediately surrounding every half-code rounding boundary,
  finite extrema, infinities/NaNs, odd-channel padding and two-row-or-more
  dispatch against the f64 CPU rule.
- Exact 1:1 crop values, odd crop dimensions, noninteger reduction ratios,
  one-pixel output and an edge pixel against the established area-block rule.

CI sets `SPEKTRAFILM_REQUIRE_GPU=1`, so missing Vulkan is a failure instead of a
skipped success. Geometry rejection is covered by unit checks in `output.rs`.
On 2026-11-13 both kernel tests passed on the local Mesa Vulkan adapter with
`SPEKTRAFILM_REQUIRE_GPU=1` (no skipped or ignored tests); both geometry tests
also passed. Linux linked the installed OpenBLAS through the existing
`openblas-src/system` feature; no Cargo dependency or lockfile change was needed.

`crates/spektrafilm-core/tests/gpu_output.rs` additionally passed four full-chain
tests on the local Mesa Vulkan adapter on the same date. The Portra 400 to
Portra Endura fixture explicitly enables the migrated multilayer model and
checks both linear and sRGB-encoded output with allocation/density caches off
and on. It compares full f32, RGB16, 1:1 crop and reduced pixels, one-pixel and
odd-channel outputs, invalid requests, and a same-device consumer that writes
an independently owned RGBA32F texture. That texture retains its pixels after
the next source image reuses the engine's work buffers.

The consumer-error and callback-triggered cancellation test verifies the
**pre-submission** boundary: its recorded write never reaches the host marker
buffer, no density is published, and a later successful render recovers and
caches normally. It is not evidence of interruption during an in-flight GPU
submission. Submitted completion/cancellation needs its own backend test.

These small deterministic full-chain cases do not replace the frozen
six-photo FPW parity qualification on the same hardware and driver. Do not
switch FPW's engine pin based solely on software Vulkan success.
