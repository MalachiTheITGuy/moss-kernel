//! x86_64 memory management module.
//!
//! Provides virtual memory management, page tables, heap, and
//! user-memory access for the x86_64 architecture.

use libkernel::memory::address::VA;

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
