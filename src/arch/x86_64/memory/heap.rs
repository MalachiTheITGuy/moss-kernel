//! x86_64 kernel heap — minimal stub for Phase 0.
//!
//! The heap is initialized by the boot code after the physical frame
//! allocator is operational.

/// Initialize the kernel heap.
///
//! # Safety
//!
//! Must be called exactly once, after the physical frame allocator
//! and the initial page tables are set up.
pub unsafe fn init() {
    // TODO(#15): Initialize the kernel heap using a physical frame
    // allocator backed by the Multiboot2 memory map.
    todo!("x86_64 heap init")
}
