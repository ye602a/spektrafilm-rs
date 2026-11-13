# Host-neutral GPU work image reuse

This cache reuses one pair of full-frame work-plane allocations. Auxiliary
resources and staging retain their own lifetimes.

- API: `WgpuBackend::configure_work_images(bool)`, `configure_work_image_budget_bytes(u64)`, `work_image_bytes()`. Disabled by default to preserve legacy allocations and render results.
- A single idle pair of storage/COPY buffers may be retained only after the final GPU submission and all required consumption/readback/unmap. Concurrent renders check the pair out exclusively; another call allocates its own buffers rather than accessing an in-flight image. A generation counter refuses late returns from a disabled/reconfigured cache.
- Cached work buffers represent **allocations**, never valid film density, RGB pixels or a host's photo identity. Allocation reuse never trusts previous pixels. A negative-cache miss uploads all prepared input pixels; an independently validated negative hit instead completely overwrites the private work plane from its immutable snapshot. Allocation reuse itself never authorizes that shortcut.
- Integrated-GPU MAP_WRITE/MAP_READ remains on the pre-existing dedicated path. On discrete GPUs the existing `Queue::write_buffer` path is unchanged in this patch; StagingBelt pooling may be considered later only with explicit submission/cancel ownership tests.
- Default idle budget is 1.5 GiB (up to roughly 60MP RGB f32 two-plane frames); host can reduce it, or disable residency entirely. Reported bytes are the *idle pair only*, not total GPU memory.
- Auxiliary blur/LUT/parameter buffers and upload/readback staging are not pooled. The 1.5 GiB default does not bound those allocations, active requests or the separate density snapshot.
- Full-chain regression uses Portra 400 -> Portra Endura on actual Vulkan, comparing byte-identical output with reuse disabled across same image / different image / geometry resize / budget shrink. Both legacy and opt-in multilayer routes exercise concurrent full-chain renders on one backend. Local Mesa validation and the linked Linux CI passed both end-to-end tests and all four ownership tests; the Windows build/check job also passed.
- GPU allocation ownership tests hold overlapping checkouts and compare actual buffer handles. They require exclusive ownership, rejection of late returns after disable/re-enable or any budget change, eviction on a size mismatch or insufficient budget, and bypass of the dedicated direct-mapped path. `SPEKTRAFILM_REQUIRE_GPU=1` makes an unavailable adapter fail instead of silently skipping these tests.
- These allocation tests establish correctness, not a measured speedup. Target-device, host-image and performance qualification remain separate.

Negative-density reuse and scoped output have separate contracts in
[GPU_NEGATIVE_CACHE.md](GPU_NEGATIVE_CACHE.md) and
[GPU_OUTPUT_CONTRACT.md](GPU_OUTPUT_CONTRACT.md). Completed checks and their
scope are recorded in [VALIDATION.md](VALIDATION.md).
