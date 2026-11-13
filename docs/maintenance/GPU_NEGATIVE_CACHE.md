# Developed-negative density cache

The optional cache reuses developed film density while RS owns the stage
boundary, exact dependencies and publication rules.

## Stored stage and unchanged rendering

The retained image is full-resolution interleaved CMY density **after film
development, DIR and grain, immediately before spectral printing or direct film
scanning**. On a miss the existing physical passes execute in their existing
order. The backend copies the resulting density into a separate snapshot. On a
hit it copies that immutable snapshot into the current render's private work
plane, then executes printing/scanning and all later effects normally.

The snapshot is never a recyclable work buffer. No output RGB pixels are cached.
Cache use does not change the input, working, or output color-space contracts.

## Input identity and exact engine dependencies

The host passes `FilmRenderOptions.source: Option<SourceIdentity>` **for each
call**. `SourceIdentity` describes the immutable prepared input pixels; it must
change for a new photograph, RAW development change, upstream edit, or any other
change in those pixels. An image path, pointer, or mutable "current photo" value
is insufficient. An absent identity bypasses reuse for that call. It does not
invent an identity from a buffer address or a sampled image hash.

RS appends its own exact prepared dependencies. The key stores typed bit patterns
and sequence lengths, including complete LUT/curve contents. It does not use a
lossy digest whose collision could select another negative. Equal-value objects
at different addresses compare equal. Equivalent but differently represented
floating-point values can conservatively miss.

| Dependency | Cached negative invalidated? | Reason |
| --- | --- | --- |
| Source identity, width, height, scalar layout | Yes | Defines the uploaded prepared image |
| Front-pass kind, complete Hanatos LUT, adapted RGB matrix | Yes | Controls spectral exposure; matrix already contains exposure corrections |
| Film exposure grid, normalized density curves, film gamma | Yes | Defines developed density |
| Highlight reconstruction, camera diffusion, camera lens blur | Yes | Changes exposure before development |
| All halation/scatter and DIR settings and curves | Yes | Changes exposure or developed density |
| Grain model, seed, particle parameters, multilayer curves, dye/microstructure, output sampling | Yes | Changes the retained physical grain field |
| Grain source transform, source size and coordinate enable state | Yes | Changes deterministic source sampling |
| Film dye spectra and base absorption | No, when all preceding prepared inputs are unchanged | Their first consumers are spectral printing or direct scanning after the boundary |
| Paper, print illumination/exposure/preflash/gamma and enlarger diffusion | No, when all preceding prepared inputs are unchanged | Recomputed after the negative |
| Direct film scan mode, scanner settings, glare, gamut handling, sharpening and output transfer | No, when all preceding prepared inputs are unchanged | Recomputed after the negative |

This classification operates on `FilmChainParams`, after core preparation.
For example, changing a paper or scan route can still miss if preparation also
changes a front-pass exposure correction. The engine detects that changed matrix
automatically. Hosts do not maintain their own duplicate list of film parameters.

`NegativeKey::new` exhaustively destructures `FilmChainParams` and each pre-print
parameter structure without `..`. Adding a field requires an explicit engine-side
classification during compilation. New stages also need an invalidation and
image-parity regression; the compile check does not replace that review.

## Concurrency, residency and publication

The default is disabled. The default GPU residency limit is **384 MiB for one
snapshot**. The host can configure the limit, clear the entry, or disable reuse.
`negative_cache_stats()` reports cumulative hits, misses, successful stores,
bypasses, and the currently retained snapshot bytes. Retained bytes exclude
in-flight snapshots/clones, ordinary work buffers, LUT key storage, and other GPU
allocations; this is not a total VRAM budget.

Each eligible request obtains a sequence ticket. A cache miss carries a private
publication token. A snapshot can be published only after successful completion
of the render, with its expected exact byte size, while its ticket is still the
latest eligible request and the cache generation is unchanged. Thus an older
concurrent render cannot replace a newer request's result. Reconfiguration,
explicit clearing, budget changes, or a request exceeding the retention budget
invalidate pending publication tokens. Invalid image geometry is rejected before
cache lookup. Dropping a token after a failure or cancellation publishes nothing.

The backend makes the final cancellation decision immediately before committing
successful output and cache publication, after GPU consumers and any readback
have completed. Cancellation arriving after that decision does not retroactively
change a completed request into a cancelled one; the core must not perform a
second cancellation decision after a successful backend return.

Hits clone an immutable buffer handle under the short cache mutex. GPU commands,
waits and output consumption happen outside that mutex. Clearing or replacing the
entry does not invalidate handles already held by in-flight readers. No request
writes to the cached buffer, including after another request changes the source.

The cache is scoped to one `WgpuBackend` and its device. A new backend/device
starts empty; entries cannot cross devices. Overlapping calls do not require
a serialized host render worker.

## Validation and remaining gates

Local validation on 2026-11-13 passed **5 CPU unit tests** and **1 complete film-chain
regression on Mesa software Vulkan** with `SPEKTRAFILM_REQUIRE_GPU=1` (GPU absence
cannot produce a skipped success). The independent CPU unit suite covers one-setting-at-a-time upstream invalidation,
all downstream dependency families, different geometry with equal byte count,
source and coordinate changes, exact LUT-content identity, disabled/unidentified
calls, discarded publication tokens, reordered completion, disable/re-enable,
budget changes, explicit clearing, and wrong-sized snapshots.

The integrated GPU regression compares cache-on results with cache-off results
on the same device, including repeated downstream edits, upstream exposure/gamma/
grain edits, new source pixels/tokens, equal-byte-count geometry changes, grain
source transforms, multilayer and legacy grain, concurrency, pre-cancellation and
recovery. Its hit/miss/store/bypass counters demonstrate that the intended routes
actually execute. All output channels compare exactly. The test uses 16×12 and
24×8 synthetic inputs with real Portra 400 and Portra Endura profiles. It validates
cache behavior; it is not a full-resolution photograph or performance result.

The local Linux commands used the available system OpenBLAS library:

```sh
cargo test --locked -p spektrafilm-gpu --features openblas-src/system --lib negative_cache
SPEKTRAFILM_REQUIRE_GPU=1 cargo test --locked -p spektrafilm-core -p spektrafilm-gpu --features openblas-src/system --test negative_cache
```

The shared output regression also raises cancellation from the submitted command's
completion callback. Its destination write has already executed, but the request
returns an error, publishes no density, and can be retried successfully. This does
not establish interruption of GPU execution or recovery from device loss.

Software Vulkan regression does not replace target-device and host-image
acceptance. Performance measurements must include key construction, GPU copies,
memory occupancy and actual print/scan edit workloads. See the
[compatibility plan](FPW_PARITY_MIGRATION_PLAN.md).

## Provenance

The stage boundary and one-plane retention follow FPW's accepted vendored route,
pinned to `ae3428acd98e39c48784a79dc8f3369ab1236cb1`,
`wgpu_backend.rs`
(blob `934e80eb47aa5bf58fa80ee2bc9c4037fe14d459`). The independent RS key and
publication controller replace that host's `input_unchanged` boolean with
engine-owned exact dependencies and per-request concurrency rules.
