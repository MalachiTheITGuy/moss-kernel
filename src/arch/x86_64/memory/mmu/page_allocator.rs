//! x86_64 physical page allocator.
//!
//! Manages allocation of 4 KiB physical frames for page tables,
//! kernel data, and user mappings.

use alloc::vec::Vec;
use libkernel::memory::address::PA;
use crate::sync::SpinLock;

/// A simple free-list physical page allocator for x86_64.
pub struct PageAllocator {
    free_frames: Vec<PA>,
}

impl PageAllocator {
    /// Create an empty allocator.
    pub const fn new() -> Self {
        Self {
            free_frames: Vec::new(),
        }
    }

    /// Register a range of physical memory as available.
    ///
    //! # Safety
    //!
    //! The caller must ensure that the range is not already in use
    //! and that no two registrations overlap.
    pub unsafe fn add_region(&mut self, start: PA, end: PA) {
        // TODO(#15): Populate free_frames with 4 KiB-aligned frames
        // from the given physical range.
        todo!("PageAllocator::add_region")
    }

    /// Allocate a single 4 KiB physical frame.
    pub fn alloc_frame(&mut self) -> Option<PA> {
        // TODO(#15): Pop a frame from the free list.
        todo!("PageAllocator::alloc_frame")
    }

    /// Return a physical frame to the free list.
    pub fn free_frame(&mut self, frame: PA) {
        // TODO(#15): Push frame onto the free list.
        todo!("PageAllocator::free_frame")
    }
}
