//! x86_64 Memory Management Unit (MMU) operations.
//!
//! Handles 4-level page table manipulation, TLB management,
//! and memory-mapped I/O region setup.

use libkernel::memory::{
    address::{PA, VA},
    paging::permissions::PtePermissions,
    proc_vm::address_space::KernAddressSpace,
    region::{PhysMemoryRegion, VirtMemoryRegion},
};

/// The x86_64 MMU.
pub struct Mmu;

impl Mmu {
    /// Map a physical address to a virtual address with the given flags.
    pub unsafe fn map_mmio(
        &self,
        phys: PA,
        virt: VA,
        size: usize,
    ) {
        // TODO(#4, #15):
        // 1. Walk/create 4-level page tables for the given virtual range.
        // 2. Set PRESENT | WRITABLE | NO_CACHE | NO_EXECUTE bits.
        // 3. Flush TLB for the affected pages.
        todo!("map_mmio: not yet implemented")
    }

    /// Map a normal (cacheable) page.
    pub unsafe fn map_normal(
        &self,
        phys: PA,
        virt: VA,
        size: usize,
    ) {
        // TODO(#4, #15):
        // 1. Walk/create 4-level page tables for the given virtual range.
        // 2. Set PRESENT | WRITABLE bits (no NO_CACHE, no NO_EXECUTE).
        // 3. Flush TLB for the affected pages.
        todo!("map_normal: not yet implemented")
    }

    /// Flush the TLB entry for a given virtual address.
    pub fn flush_tlb_entry(&self, virt: VA) {
        // SAFETY: INVLPG is a privileged instruction that invalidates
        // a single TLB entry. Safe to call with any valid virtual address.
        unsafe {
            core::arch::asm!(
                "invlpg [{virt}]",
                virt = in(reg) virt.value() as u64,
                options(nostack),
            );
        }
    }

    /// Flush the entire TLB by reloading CR3.
    pub fn flush_tlb_all(&self) {
        // SAFETY: Reading CR3 and writing it back invalidates all
        // non-global TLB entries.
        unsafe {
            let cr3: u64;
            core::arch::asm!("mov {cr3}, cr3", cr3 = out(reg) cr3);
            core::arch::asm!("mov cr3, {cr3}", cr3 = in(reg) cr3);
        }
    }

    /// Read the current CR3 value (PML4 physical base).
    pub fn read_cr3() -> PA {
        let cr3: u64;
        // SAFETY: Reading CR3 is always safe.
        unsafe {
            core::arch::asm!("mov {cr3}, cr3", cr3 = out(reg) cr3);
        }
        PA::from_value(cr3 as usize)
    }
}

/// x86_64 kernel address space wrapping an `X86_64AddressSpace`.
pub struct X86_64KernelAddressSpace {
    inner: super::address_space::X86_64AddressSpace,
}

impl X86_64KernelAddressSpace {
    /// Create a new kernel address space.
    pub fn new() -> Self {
        Self {
            inner: super::address_space::X86_64AddressSpace::new_kernel(),
        }
    }
}

impl KernAddressSpace for X86_64KernelAddressSpace {
    fn map_mmio(
        &mut self,
        _region: PhysMemoryRegion,
    ) -> libkernel::error::Result<VA> {
        // TODO(#15): Map an MMIO region using 4-level page tables.
        todo!("KernAddressSpace::map_mmio")
    }

    fn map_normal(
        &mut self,
        _phys_range: PhysMemoryRegion,
        _virt_range: VirtMemoryRegion,
        _perms: PtePermissions,
    ) -> libkernel::error::Result<()> {
        // TODO(#15): Map normal memory using 4-level page tables.
        todo!("KernAddressSpace::map_normal")
    }
}

/// Global kernel address space, protected by a spinlock.
static KERN_ADDR_SPACE: crate::sync::SpinLock<X86_64KernelAddressSpace> =
    crate::sync::SpinLock::new(X86_64KernelAddressSpace {
        inner: super::address_space::X86_64AddressSpace {
            pml4: PA::from_value(0),
        },
    });

/// Obtain a reference to the global kernel address space lock.
pub fn kern_address_space() -> &'static crate::sync::SpinLock<X86_64KernelAddressSpace> {
    &KERN_ADDR_SPACE
}
