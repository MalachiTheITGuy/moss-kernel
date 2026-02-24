pub mod address_space;
pub mod heap;
pub mod mmu;
pub mod uaccess;

pub const PAGE_OFFSET: usize = 0xffff_8000_0000_0000;
pub const IMAGE_BASE: libkernel::memory::address::VA = libkernel::memory::address::VA::from_value(0xffffffff80000000);
