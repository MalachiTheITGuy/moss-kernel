//! x86_64 kernel address space management.
//!
//! Manages 4-level page tables (PML4 → PDPT → PD → PT) for the
//! x86_64 architecture.

use crate::memory::proc_vm::address_space::{self, AddressSpace};
use alloc::sync::Arc;
use libkernel::{
    error::Result,
    memory::address::{PA, VA},
};

/// x86_64 virtual address space.
pub struct X86_64AddressSpace {
    // TODO(#15): Page table root (PML4 physical address).
    pml4: PA,
}

impl X86_64AddressSpace {
    /// Create a new kernel address space.
    pub fn new_kernel() -> Self {
        // TODO(#15): Allocate and initialize a PML4 page table.
        todo!("X86_64AddressSpace::new_kernel")
    }

    /// Create a new user address space by copying from kernel space.
    pub fn new_user() -> Self {
        // TODO(#15): Fork PML4, copying kernel-space entries.
        todo!("X86_64AddressSpace::new_user")
    }

    /// Map a virtual page to a physical page with given flags.
    pub fn map(
        &mut self,
        va: VA,
        pa: PA,
        flags: MapFlags,
    ) -> Result<()> {
        // TODO(#15): Walk/create page table entries.
        todo!("X86_64AddressSpace::map")
    }

    /// Unmap a virtual page.
    pub fn unmap(&mut self, va: VA) -> Result<()> {
        // TODO(#15): Clear page table entry, flush TLB.
        todo!("X86_64AddressSpace::unmap")
    }

    /// Translate a virtual address to a physical address.
    pub fn translate(&self, va: VA) -> Option<PA> {
        // TODO(#15): Walk page tables.
        todo!("X86_64AddressSpace::translate")
    }

    /// Switch to this address space by loading PML4 into CR3.
    pub fn activate(&self) {
        // SAFETY: Loading CR3 with a valid PML4 address switches the
        // active address space. The PML4 is kept alive by Arc.
        unsafe {
            core::arch::asm!(
                "mov cr3, {pml4}",
                pml4 = in(reg) self.pml4.value() as u64,
                options(nostack),
            );
        }
    }
}

/// Page table entry flags for x86_64.
bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct MapFlags: u64 {
        const PRESENT       = 1 << 0;
        const WRITABLE      = 1 << 1;
        const USER_ACCESS   = 1 << 2;
        const WRITE_THROUGH = 1 << 3;
        const NO_CACHE      = 1 << 4;
        const ACCESSED      = 1 << 5;
        const DIRTY         = 1 << 6;
        const HUGE_PAGE     = 1 << 7;
        const GLOBAL        = 1 << 8;
        const NO_EXECUTE    = 1 << 63;
    }
}

impl From<MapFlags> for u64 {
    fn from(flags: MapFlags) -> Self {
        flags.bits()
    }
}
