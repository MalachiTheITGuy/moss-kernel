//! x86_64 memory management module.
//!
//! Provides virtual memory management, page tables, heap, and
//! user-memory access for the x86_64 architecture.

use libkernel::memory::address::{PA, VA};

use self::mmu::KERN_ADDR_SPC;

/// Translate a kernel virtual address to its physical address via
/// the kernel address space page tables. Mirrors the arithmetic
/// translation on arm64 but uses the page-table walk on x86_64.
pub fn translate_kernel_va(va: VA) -> Option<PA> {
    KERN_ADDR_SPC.get()?.lock_save_irq().translate(va)
}

/// Look up the physical address of a kernel linker symbol.
///
/// Usage: `let pa = ksym_pa!(__vdso_start);`
#[macro_export]
macro_rules! ksym_pa {
    ($sym:expr) => {{
        let v = libkernel::memory::address::VA::from_value(core::ptr::addr_of!($sym) as usize);
        $crate::arch::x86_64::memory::translate_kernel_va(v).expect("ksym_pa: translate failed")
    }};
}

/// Start of the kernel direct-mapping region in the higher-half
/// virtual address space on x86_64.
pub const PAGE_OFFSET: usize = 0xffff_8880_0000_0000;

/// Start of the MMIO mapping region in the higher-half virtual address
/// space on x86_64, located immediately after the direct-mapping region.
pub const MMIO_BASE: VA = VA::from_value(0xffff_c900_0000_0000);

pub mod address_space;
pub mod heap;
pub mod mmu;
pub mod page_mapper;
pub mod tlb;
pub mod uaccess;

pub use mmu::Mmu;
