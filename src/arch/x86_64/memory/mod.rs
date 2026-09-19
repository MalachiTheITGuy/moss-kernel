//! x86_64 memory management module.
//!
//! Provides virtual memory management, page tables, heap, and
//! user-memory access for the x86_64 architecture.

/// Start of the kernel direct-mapping region in the higher-half
/// virtual address space on x86_64.
pub const PAGE_OFFSET: usize = 0xffff_8880_0000_0000;

pub mod address_space;
pub mod heap;
pub mod mmu;
pub mod uaccess;

pub use mmu::Mmu;
