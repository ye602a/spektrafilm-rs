# Shared-device submission gate

A host sharing its wgpu device between film processing and display can pass
one mutex for short submission and surface-reconfiguration sections. The host
does not need to hold that mutex around a complete engine render or GPU wait.

## Constructor and ownership

```rust,ignore
let backend = WgpuBackend::from_device_with_submission_lock(
    device.clone(),
    queue.clone(),
    Some(surface_submission_lock.clone()),
);
```

The added constructor takes:

```rust,ignore
pub fn from_device_with_submission_lock(
    device: wgpu::Device,
    queue: wgpu::Queue,
    submission_lock: Option<std::sync::Arc<std::sync::Mutex<()>>>,
) -> WgpuBackend;
```

It reuses the supplied device and matching queue, and retains a cloneable handle
to the host's mutex. The existing two-argument `from_device(device, queue)` calls
this constructor with `None`; standalone initialization and callers without a
shared surface keep their existing behavior.

The host uses the same mutex for its own coordinated submissions and surface
changes. It must not hold the mutex while calling an engine render method. The
engine acquires it immediately before its actual `queue.submit` call and drops
the guard when the submission helper returns. Parameter preparation, shader
encoding, the scoped film-output consumer, explicit `device.poll` waits,
readback, and completed-cache publication all run outside this section.

All production submission paths use the helper: generic spectral dispatch,
single Gaussian blur, batched Gaussian blur, and the shared resident film/output
chain. Tests that operate directly on borrowed raw queue handles still own their
own synchronization, just as host code does.

## Cancellation and callback boundary

The resident chain rechecks `FilmRenderOptions::cancellation` **after acquiring
the mutex and before submitting its command buffer**. If cancellation arrived
while the host held the gate, the call returns `OutputError::Cancelled` without
executing the encoded commands or publishing a density snapshot. Existing
infallible legacy trait methods retain their prior behavior and do not gain a
cancellation parameter.

Cancellation is cooperative: waiting for `std::sync::Mutex` is not interrupted.
The cancelled call returns once the host releases the mutex and the engine can
observe the flag. Commands already submitted still follow the ordinary
completion and final-publication rules.

The engine's `with_film_output` consumer is called before gate acquisition. That
guarantee does not extend to every callback in the raw wgpu API: wgpu 30 may run
previously registered `Queue::on_submitted_work_done` callbacks from inside
`Queue::submit` itself. Such native completion callbacks should signal or enqueue
work rather than synchronously acquire this same non-reentrant mutex. The
engine does not add a second callback scheduler or broaden the locked section.

## Compatibility

The optional gate preserves the submission boundary of the reference engine at
`ae3428acd98e39c48784a79dc8f3369ab1236cb1`. The separately named constructor
retains the existing two-argument API and standalone behavior.

## Regression

`crates/spektrafilm-core/tests/submission_gate.rs` uses actual GPU command
submission on one device and its injected handles. It verifies:

1. Both constructor signatures remain available, injected handles compare equal
   to the host's handles, and gated full-chain output equals the ungated output.
2. While the host holds the gate, the film consumer can encode, a GPU marker
   remains unchanged, and the render cannot complete. After release it executes.
3. The film consumer can acquire the gate. A completion callback attached to the
   current command encoder observes it released during the following poll; the
   host can then hold it while that render finishes.
4. Cancellation during the blocked submission leaves the marker unchanged and
   publishes no density snapshot. A subsequent render succeeds with exact pixels.
5. The other three production submission paths also block on the gate, resume,
   and preserve their output values.

```sh
SPEKTRAFILM_REQUIRE_GPU=1 cargo test --locked \
  -p spektrafilm-core -p spektrafilm-gpu --features openblas-src/system \
  --test submission_gate -- --nocapture
```

The local Mesa Vulkan run exercises actual command execution and the cancellation
boundary. Window resize/reconfiguration remains a host integration check; this
headless regression does not simulate a window-system surface.
