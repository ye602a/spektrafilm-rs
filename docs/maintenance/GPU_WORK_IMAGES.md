# Host-neutral GPU work image reuse

2026-11-13. RS-owned engine follow-up to merged multilayer grain and Vulkan-IIR migration. PR #7 is under validation; its merge and final CI result must be recorded when complete.

- API: `WgpuBackend::configure_work_images(bool)`, `configure_work_image_budget_bytes(u64)`, `work_image_bytes()`. Disabled by default to preserve legacy allocations and render results.
- A single idle pair of storage/COPY buffers may be retained only after the final GPU submission, readback, and unmap. Concurrent renders check the pair out exclusively; another call allocates its own buffers rather than accessing an in-flight image. A generation counter refuses late returns from a disabled/reconfigured cache.
- All image pixels are uploaded again on each render. Cached buffers represent **allocations**, never cached film density, RGB pixel contents or a host's photo identity.
- Integrated-GPU MAP_WRITE/MAP_READ remains on the pre-existing dedicated path. On discrete GPUs the existing `Queue::write_buffer` path is unchanged in this patch; StagingBelt pooling may be considered later only with explicit submission/cancel ownership tests.
- Default idle budget is 1.5 GiB (up to roughly 60MP RGB f32 two-plane frames); host can reduce it, or disable residency entirely. Reported bytes are the *idle pair only*, not total GPU memory.
- Full-chain regression target: Portra 400 -> Portra Endura on actual Vulkan, comparing byte-identical output with reuse disabled across same image / different image / geometry resize / budget shrink. Both legacy and opt-in multilayer routes additionally exercise concurrent full-chain renders on one backend. The resumed local Mesa Vulkan run passed both end-to-end tests and all four ownership tests; Windows and hosted Linux validation is required against the final PR commit.
- GPU allocation ownership tests hold overlapping checkouts and compare actual buffer handles. They require exclusive ownership, rejection of late returns after disable/re-enable or any budget change, eviction on a size mismatch or insufficient budget, and bypass of the dedicated direct-mapped path. `SPEKTRAFILM_REQUIRE_GPU=1` makes an unavailable adapter fail instead of silently skipping these tests.
- Final six-photo FPW visual parity, full IIR hardware qualification and controlled performance measurements remain required before FPW pin cutover. These allocation tests establish no speedup claim.
- FPW main, output RGB16, negative density cache and wide-gamut research are outside this patch.
