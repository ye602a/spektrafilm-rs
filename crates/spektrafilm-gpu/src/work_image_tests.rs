//! GPU allocation ownership tests for the private work-image cache boundary.
//! These tests hold simultaneous checkouts without relying on thread timing.

use super::{WgpuBackend, WorkImages};

fn backend() -> Option<WgpuBackend> {
    let backend = WgpuBackend::new();
    if backend.is_none() {
        assert!(
            std::env::var_os("SPEKTRAFILM_REQUIRE_GPU").is_none(),
            "a real GPU is required for work-image ownership tests"
        );
        eprintln!("SKIP: no compatible GPU for work-image ownership tests");
    }
    backend
}

fn allocate_pair(backend: &WgpuBackend, image_bytes: u64) -> WorkImages {
    let allocate = |label| {
        backend.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: image_bytes,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    };
    WorkImages {
        a: allocate("work-image-ownership-a"),
        b: allocate("work-image-ownership-b"),
    }
}

fn acquire(backend: &WgpuBackend, image_bytes: u64) -> (WorkImages, u64) {
    let (pair, generation) = backend.checkout_work_images(image_bytes, false);
    (
        pair.unwrap_or_else(|| allocate_pair(backend, image_bytes)),
        generation,
    )
}

fn release(backend: &WgpuBackend, pair: WorkImages, generation: u64) {
    backend.release_work_images(pair.a, pair.b, generation, false);
}

#[test]
fn work_images_simultaneous_checkouts_own_distinct_buffers() {
    let Some(backend) = backend() else {
        return;
    };
    let image_bytes = 16 * 3 * 4;
    backend.configure_work_images(true);

    let (warm, generation) = acquire(&backend, image_bytes);
    let original_a = warm.a.clone();
    let original_b = warm.b.clone();
    release(&backend, warm, generation);
    assert_eq!(backend.work_image_bytes(), image_bytes * 2);

    let (first, first_generation) = backend.checkout_work_images(image_bytes, false);
    let first = first.expect("the first caller must receive the idle pair");
    assert_eq!(first.a, original_a);
    assert_eq!(first.b, original_b);
    assert_eq!(backend.work_image_bytes(), 0, "checked-out bytes are not idle");

    let (second, second_generation) = backend.checkout_work_images(image_bytes, false);
    assert!(second.is_none(), "an active pair cannot be checked out twice");
    let second = allocate_pair(&backend, image_bytes);
    assert_ne!(first.a, second.a);
    assert_ne!(first.a, second.b);
    assert_ne!(first.b, second.a);
    assert_ne!(first.b, second.b);

    // A completed second caller may become idle while the first retains its
    // own GPU allocations. The next checkout must only receive that second pair.
    let second_a = second.a.clone();
    let second_b = second.b.clone();
    release(&backend, second, second_generation);
    let (third, third_generation) = backend.checkout_work_images(image_bytes, false);
    let third = third.expect("the completed second pair is reusable");
    assert_eq!(third.a, second_a);
    assert_eq!(third.b, second_b);
    assert_ne!(third.a, first.a);
    assert_ne!(third.b, first.b);
    release(&backend, first, first_generation);
    release(&backend, third, third_generation);
    assert_eq!(backend.work_image_bytes(), image_bytes * 2);
}

#[test]
fn work_images_disable_and_reenable_rejects_late_generation() {
    let Some(backend) = backend() else {
        return;
    };
    let image_bytes = 16 * 3 * 4;
    backend.configure_work_images(true);
    let (old, old_generation) = acquire(&backend, image_bytes);

    backend.configure_work_images(false);
    assert_eq!(backend.work_image_bytes(), 0);
    backend.configure_work_images(true);
    let (current, current_generation) = acquire(&backend, image_bytes);
    assert_ne!(old_generation, current_generation);
    let current_a = current.a.clone();
    let current_b = current.b.clone();
    release(&backend, current, current_generation);

    // The late result must neither repopulate nor replace a new-generation pair.
    release(&backend, old, old_generation);
    let (retained, generation) = backend.checkout_work_images(image_bytes, false);
    let retained = retained.expect("the new-generation pair must survive");
    assert_eq!(retained.a, current_a);
    assert_eq!(retained.b, current_b);

    backend.configure_work_images(false);
    release(&backend, retained, generation);
    assert_eq!(backend.work_image_bytes(), 0, "disabled cache cannot be restored");
}

#[test]
fn work_images_budget_changes_reject_late_returns_and_evict_oversize() {
    let Some(backend) = backend() else {
        return;
    };
    let image_bytes = 16 * 3 * 4;
    backend.configure_work_images(true);
    backend.configure_work_image_budget_bytes(image_bytes * 2);

    // Both an increase and a decrease invalidate leases, even when the old
    // allocation would still fit. A future policy change cannot revive it.
    for budget in [image_bytes * 4, image_bytes * 2] {
        let (late, generation) = acquire(&backend, image_bytes);
        backend.configure_work_image_budget_bytes(budget);
        release(&backend, late, generation);
        assert_eq!(backend.work_image_bytes(), 0);
    }

    let (current, generation) = acquire(&backend, image_bytes);
    release(&backend, current, generation);
    assert_eq!(backend.work_image_bytes(), image_bytes * 2);
    backend.configure_work_image_budget_bytes(image_bytes * 2 - 1);
    assert_eq!(backend.work_image_bytes(), 0, "shrinking evicts an idle pair");

    backend.configure_work_image_budget_bytes(image_bytes * 2);
    let (late, generation) = acquire(&backend, image_bytes);
    backend.configure_work_image_budget_bytes(0);
    release(&backend, late, generation);
    assert_eq!(backend.work_image_bytes(), 0, "late oversize pair stays evicted");

    // An allocation completed under the current zero budget also cannot enter.
    let (over_budget, generation) = acquire(&backend, image_bytes);
    release(&backend, over_budget, generation);
    assert_eq!(backend.work_image_bytes(), 0);
}

#[test]
fn work_images_size_change_evicts_idle_pair_and_mappable_bypasses_reuse() {
    let Some(backend) = backend() else {
        return;
    };
    let small_bytes = 16 * 3 * 4;
    let large_bytes = 25 * 3 * 4;
    backend.configure_work_images(true);
    let (small, generation) = acquire(&backend, small_bytes);
    release(&backend, small, generation);

    let (mismatched, _) = backend.checkout_work_images(large_bytes, false);
    assert!(mismatched.is_none(), "work-plane byte size must match exactly");
    assert_eq!(backend.work_image_bytes(), 0);
    let (evicted, _) = backend.checkout_work_images(small_bytes, false);
    assert!(evicted.is_none(), "a size mismatch must discard the previous pair");

    let (large, generation) = acquire(&backend, large_bytes);
    release(&backend, large, generation);
    let (mappable, generation) = backend.checkout_work_images(large_bytes, true);
    assert!(mappable.is_none(), "direct mapping keeps its dedicated allocations");
    assert_eq!(backend.work_image_bytes(), 0);
    let direct = allocate_pair(&backend, large_bytes);
    backend.release_work_images(direct.a, direct.b, generation, true);
    assert_eq!(backend.work_image_bytes(), 0, "direct-mapped path is never retained");
}
