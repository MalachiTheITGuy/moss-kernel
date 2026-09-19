//! x86_64 memory management module.
//!
//! Provides virtual memory management, page tables, heap, and
//! user-memory access for the x86_64 architecture.

pub mod address_space;
pub mod heap;
pub mod mmu;
pub mod uaccess;

pub use mmu::Mmu;
