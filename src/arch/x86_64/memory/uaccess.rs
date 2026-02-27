use core::{future::Future, pin::Pin, task::{Context, Poll}};
use libkernel::{error::{Result, KernelError}, memory::address::UA};
use super::PAGE_OFFSET;

unsafe impl Send for X86CopyFromUser {}
unsafe impl Send for X86CopyToUser {}
unsafe impl Send for X86CopyStrnFromUser {}

/// Walk the active page table to check whether a user virtual address is
/// mapped and accessible.  Returns `true` if the page is present (and, when
/// `write` is set, writable).  Does **not** read or write any user memory.
///
/// The linear map (physical 0..4 GiB at `PAGE_OFFSET`) is used to traverse
/// the page-table structures safely from kernel mode.
unsafe fn is_user_page_mapped(vaddr: usize, write: bool) -> bool {
    let cr3: usize;
    unsafe { core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack, nomem)) };
    // Mask off PCID bits (bits 0-11) to get the physical base of PML4.
    let pml4_pa = cr3 & !0xFFF;

    let pml4_idx = (vaddr >> 39) & 0x1FF;
    let pdpt_idx = (vaddr >> 30) & 0x1FF;
    let pd_idx   = (vaddr >> 21) & 0x1FF;
    let pt_idx   = (vaddr >> 12) & 0x1FF;

    macro_rules! read_entry {
        ($pa:expr, $idx:expr) => {{
            let va = PAGE_OFFSET + $pa + $idx * 8;
            unsafe { (va as *const u64).read_volatile() }
        }};
    }

    let pml4e = read_entry!(pml4_pa, pml4_idx);
    if pml4e & 1 == 0 { return false; }
    let pdpt_pa = (pml4e & 0x000F_FFFF_FFFF_F000) as usize;

    let pdpte = read_entry!(pdpt_pa, pdpt_idx);
    if pdpte & 1 == 0 { return false; }
    if pdpte & (1 << 7) != 0 {
        // 1 GiB page: present; check writable if needed.
        return !write || (pdpte & (1 << 1) != 0);
    }
    let pd_pa = (pdpte & 0x000F_FFFF_FFFF_F000) as usize;

    let pde = read_entry!(pd_pa, pd_idx);
    if pde & 1 == 0 { return false; }
    if pde & (1 << 7) != 0 {
        // 2 MiB page: present; check writable if needed.
        return !write || (pde & (1 << 1) != 0);
    }
    let pt_pa = (pde & 0x000F_FFFF_FFFF_F000) as usize;

    let pte = read_entry!(pt_pa, pt_idx);
    if pte & 1 == 0 { return false; }
    !write || (pte & (1 << 1) != 0)
}

/// Verify that the entire user address range `[addr, addr+len)` is mapped
/// (and writable when `write` is set).  Returns `Err(Fault)` if any page in
/// the range is absent or lacks the required permissions.
unsafe fn check_user_range(addr: usize, len: usize, write: bool) -> Result<()> {
    if addr.checked_add(len).map_or(true, |end| end > 0x0000_8000_0000_0000) {
        return Err(KernelError::Fault);
    }
    if len == 0 {
        return Ok(());
    }
    // Walk every page that overlaps [addr, addr+len).
    // Because the range-check above ensures addr+len <= 0x0000_8000_0000_0000,
    // last_page is at most 0x0000_7FFF_FFFF_F000 and page+0x1000 cannot wrap.
    let first_page = addr & !0xFFF;
    let last_page  = (addr + len - 1) & !0xFFF;
    let mut page = first_page;
    while page <= last_page {
        if !unsafe { is_user_page_mapped(page, write) } {
            return Err(KernelError::Fault);
        }
        // Overflow is impossible here: last_page <= 0x0000_7FFF_FFFF_F000.
        page = page.wrapping_add(0x1000);
    }
    Ok(())
}

pub unsafe fn try_copy_from_user(src: UA, dst: *mut (), len: usize) -> Result<()> {
    unsafe { check_user_range(src.value(), len, false) }?;
    unsafe {
        core::ptr::copy_nonoverlapping(src.value() as *const u8, dst as *mut u8, len);
    }
    Ok(())
}

pub unsafe fn try_copy_to_user(src: *const (), dst: UA, len: usize) -> Result<()> {
    unsafe { check_user_range(dst.value(), len, true) }?;
    unsafe {
        core::ptr::copy_nonoverlapping(src as *const u8, dst.value() as *mut u8, len);
    }
    Ok(())
}

pub struct X86CopyFromUser {
    src: UA,
    dst: *mut (),
    len: usize,
}

impl X86CopyFromUser {
    pub fn new(src: UA, dst: *mut (), len: usize) -> Self {
        Self { src, dst, len }
    }
}

impl Future for X86CopyFromUser {
    type Output = Result<()>;
    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(unsafe { try_copy_from_user(self.src, self.dst, self.len) })
    }
}

pub struct X86CopyToUser {
    src: *const (),
    dst: UA,
    len: usize,
}

impl X86CopyToUser {
    pub fn new(src: *const (), dst: UA, len: usize) -> Self {
        Self { src, dst, len }
    }
}

impl Future for X86CopyToUser {
    type Output = Result<()>;
    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(unsafe { try_copy_to_user(self.src, self.dst, self.len) })
    }
}

pub struct X86CopyStrnFromUser {
    src: UA,
    dst: *mut u8,
    len: usize,
}

impl X86CopyStrnFromUser {
    pub fn new(src: UA, dst: *mut u8, len: usize) -> Self {
        Self { src, dst, len }
    }
}

impl Future for X86CopyStrnFromUser {
    type Output = Result<usize>;
    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Walk one page at a time so we return Fault rather than causing a
        // kernel page fault if an unmapped page is encountered mid-string.
        let src_base = self.src.value();
        if src_base.checked_add(self.len).map_or(true, |end| end > 0x0000_8000_0000_0000) {
            return Poll::Ready(Err(KernelError::Fault));
        }
        let src_ptr = src_base as *const u8;
        let mut i = 0;
        while i < self.len {
            // Check the current page before accessing it.
            let page = (src_base + i) & !0xFFF;
            if !unsafe { is_user_page_mapped(page, false) } {
                return Poll::Ready(Err(KernelError::Fault));
            }
            // Read to the end of this page (or self.len, whichever comes first).
            let page_end = (page + 0x1000).min(src_base + self.len);
            while src_base + i < page_end {
                let c = unsafe { *src_ptr.add(i) };
                unsafe { *self.dst.add(i) = c };
                if c == 0 {
                    return Poll::Ready(Ok(i));
                }
                i += 1;
            }
        }
        Poll::Ready(Ok(i))
    }
}
