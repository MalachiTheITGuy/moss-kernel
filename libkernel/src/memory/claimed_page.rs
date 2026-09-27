//! RAII wrapper for a page frame claimed from the physical page allocator.

use super::{
    PAGE_SIZE,
    address::AddressTranslator,
    allocators::phys::{PageAllocGetter, PageAllocation},
    page::PageFrame,
};
use crate::{
    CpuOps,
    error::Result,
    memory::address::{PA, VA},
};
use alloc::slice;
use core::fmt::Display;
use core::marker::PhantomData;

/// A convenience wrapper for dealing with single-page allocations.
pub struct ClaimedPage<A: CpuOps, G: PageAllocGetter<A>, T: AddressTranslator<()>>(
    PageAllocation<'static, A>,
    PhantomData<G>,
    PhantomData<T>,
);

impl<A: CpuOps, G: PageAllocGetter<A>, T: AddressTranslator<()>> Display for ClaimedPage<A, G, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0.region().start_address().to_pfn())
    }
}

impl<A: CpuOps, G: PageAllocGetter<A>, T: AddressTranslator<()>> ClaimedPage<A, G, T> {
    /// Allocates a single physical page. The contents of the page are
    /// undefined.
    fn alloc() -> Result<Self> {
        // Diagnostic: about to call alloc_frames (marker 'x')
        unsafe { core::arch::asm!("out dx, al", in("dx") 0xE9u16, in("al") b'x', options(nostack)); }
        let frame = G::global_page_alloc().alloc_frames(0)?;
        // Diagnostic: alloc_frames returned (marker 'y')
        unsafe { core::arch::asm!("out dx, al", in("dx") 0xE9u16, in("al") b'y', options(nostack)); }
        Ok(Self(frame, PhantomData, PhantomData))
    }

    /// Allocates a single physical page and zeroes its contents.
    pub fn alloc_zeroed() -> Result<Self> {
        // Diagnostic: about to call alloc (marker 'u')
        unsafe { core::arch::asm!("out dx, al", in("dx") 0xE9u16, in("al") b'u', options(nostack)); }
        let mut page = Self::alloc()?;
        // Diagnostic: alloc returned, about to fill (marker 'v')
        unsafe { core::arch::asm!("out dx, al", in("dx") 0xE9u16, in("al") b'v', options(nostack)); }
        page.as_slice_mut().fill(0);
        // Diagnostic: fill done (marker 'w')
        unsafe { core::arch::asm!("out dx, al", in("dx") 0xE9u16, in("al") b'w', options(nostack)); }
        Ok(page)
    }

    /// Takes ownership of the page at pfn.
    ///
    /// # Safety
    ///
    /// Ensure that the calling context does indeed own this page. Otherwise,
    /// the page may be freed when it's owned by another context.
    pub unsafe fn from_pfn(pfn: PageFrame) -> Self {
        Self(
            unsafe { G::global_page_alloc().alloc_from_region(pfn.as_phys_range()) },
            PhantomData,
            PhantomData,
        )
    }

    /// Returns the physical address of this claimed page.
    #[inline(always)]
    pub fn pa(&self) -> PA {
        self.0.region().start_address()
    }

    /// Returns the kernel virtual address where this page is mapped.
    #[inline(always)]
    pub fn va(&self) -> VA {
        self.pa().to_va::<T>()
    }

    /// Returns a raw pointer to the page's content.
    #[inline(always)]
    pub fn as_ptr(&self) -> *const u8 {
        self.va().as_ptr() as *const _
    }

    /// Returns a mutable raw pointer to the page's content.
    #[inline(always)]
    pub fn as_ptr_mut(&self) -> *mut u8 {
        self.va().as_ptr_mut() as *mut _
    }

    /// Returns a slice representing the page's content.
    #[inline(always)]
    pub fn as_slice(&self) -> &[u8] {
        // This is safe because:
        // 1. We have a reference `&self`, guaranteeing safe access.
        // 2. The pointer is valid and aligned.
        // 3. The lifetime of the slice is tied to `&self` by the compiler.
        unsafe { slice::from_raw_parts(self.as_ptr(), PAGE_SIZE) }
    }

    /// Returns a mutable slice representing the page's content.
    #[inline(always)]
    pub fn as_slice_mut(&mut self) -> &mut [u8] {
        // This is safe because:
        // 1. We have a mutable reference `&mut self`, guaranteeing exclusive access.
        // 2. The pointer is valid and aligned.
        // 3. The lifetime of the slice is tied to `&mut self` by the compiler.
        unsafe { slice::from_raw_parts_mut(self.as_ptr_mut(), PAGE_SIZE) }
    }

    /// Consumes the claimed page, returning the underlying page frame without freeing it.
    pub fn leak(self) -> PageFrame {
        self.0.leak().start_address().to_pfn()
    }
}
