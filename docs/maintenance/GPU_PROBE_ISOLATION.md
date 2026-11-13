# Same-device probes without disturbing photo caches

Synthetic measurement probes can share the rendering device without evicting
a photograph's negative-density snapshot or idle work planes. Isolation changes
resource ownership, while the caller retains control of measurement parameters
and scheduling.

## Per-call contract

Pass `isolate_photo_caches: true` for the probe invocation:

```rust,ignore
let output = point.process_resident_with_options(
    &neutral_ramp,
    &backend,
    spektrafilm_gpu::FilmRenderOptions {
        isolate_photo_caches: true,
        cancellation: Some(stop),
        ..Default::default()
    },
)?;
```

`point` is the caller's already prepared point-response pipeline. The option
changes resource ownership only; it does not disable grain, blur, or other
physical effects on the caller's behalf. The same option works with the f32,
RGB16, display-region, and scoped same-device consumer entry points because they
all use the same film-chain implementation.

For an isolated invocation:

- The device, queue, and compiled compute-pipeline cache are shared with normal
  photo renders. No adapter or device is created.
- Both full-image work planes belong to the invocation. It neither checks out
  nor returns the photo cache's idle pair, including when dimensions match.
- It does not look up or publish a density snapshot. An optional `source` is
  ignored, and photo cache counters, publication tickets, and generations are
  untouched.
- Success, validation error, consumer error, and cooperative cancellation all
  leave photo cache ownership unchanged. Normal buffer lifetime and submission
  completion rules still apply to the probe's private resources.
- No shared enable flag is toggled, and there is no save/restore operation. A
  concurrently or reentrantly completed photograph remains the current cache
  entry after the probe finishes. Explicit cache reconfiguration by another
  caller is also never undone by a probe.

The default is `false`, preserving ordinary photo rendering and retention.
`source: None` alone is different: it bypasses negative-density reuse while
ordinary work-plane reuse still applies. Use the isolation option for latitude
ramps so their different geometry cannot evict a photograph's work pair.

The existing work and density budgets bound retained photo resources. Private
probe allocations are transient, just like other in-flight allocations, and
remain subject to the normal image validation and device limits. Cached shader
pipelines may be compiled by probes and are deliberately shared.

## Compatibility

The request-local option preserves the cache-isolation behavior of the reference
engine at `ae3428acd98e39c48784a79dc8f3369ab1236cb1`. Unlike temporarily removing
and restoring global caches, it also supports overlapping and nested calls.

## Executable regression

`crates/spektrafilm-core/tests/probe_isolation.rs` uses the real spectral
Portra 400 / Portra Endura pipeline and one backend/device. Expectations are
computed with retention disabled before both photo caches are populated. The
test then checks exact channel equality and real cache hit/store/byte counters:

1. Equal-size probes and a 1024-by-8 neutral ramp preserve photo caches, including
   while the probe consumer is running.
2. Consumer rejection, a pre-cancelled request, and cancellation in the consumer
   preserve the populated caches; a subsequent successful probe and photo work.
3. A photo consumer can run a nested probe without losing its exclusive planes.
   A probe consumer can also run a normal photo, whose cache update survives.
4. Channels fix the overlap order for a pending photo miss and a concurrent
   probe. The photo's pending density publication still succeeds.
5. A probe held at its consumer boundary can overlap a completed new photo; the
   probe must not restore the preceding photo's cache on success, consumer
   rejection, or cancellation afterward.

Run on a Vulkan-capable host with system OpenBLAS using:

```sh
SPEKTRAFILM_REQUIRE_GPU=1 cargo test --locked \
  -p spektrafilm-core -p spektrafilm-gpu --features openblas-src/system \
  --test probe_isolation -- --nocapture
```

`SPEKTRAFILM_REQUIRE_GPU=1` makes lack of a GPU a failure instead of a skipped
success. Software Vulkan validates the real command submission, buffer lifetime,
and pixel behavior here. Real surface interaction and host image acceptance
remain separate integration checks; see [VALIDATION.md](VALIDATION.md).
