//! x86_64 kernel heap allocator.
//!
//! Provides a stub global allocator for compilation. A real implementation
//! (slab allocator backed by the physical page allocator) will arrive in
//! Phase 2 (Issue #15).

use core::alloc::{GlobalAlloc, Layout};

/// Minimal stub heap that panics on every allocation.
///
/// This exists solely to satisfy the `#[global_allocator]` requirement
/// during Phase 1.  It will be replaced by a proper slab-based allocator
/// once the physical memory manager is in place (Issue #15).
struct X86_64Heap;

// SAFETY: The stub allocator either panics or delegates to nothing,
// so it cannot violate any memory-safety invariants.
unsafe impl GlobalAlloc for X86_64Heap {
    unsafe fn alloc(&self, _layout: Layout) -> *mut u8 {
        panic!("x86_64 heap not yet initialised (Issue #15)")
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        panic!("x86_64 heap not yet initialised (Issue #15)")
    }
}

#[global_allocator]
static ALLOCATOR: X86_64Heap = X86_64Heap;

/// Initialise the x86_64 heap allocator.
///
/// # Safety
///
/// Must be called exactly once, after the physical page allocator is ready.
pub unsafe fn init() {
    // TODO(#15): Set up slab allocator backed by the physical page allocator.
}
