use core::{future::Future, pin::Pin, task::{Context, Poll}};
use libkernel::{error::{Result, KernelError}, memory::address::UA};

unsafe impl Send for X86CopyFromUser {}
unsafe impl Send for X86CopyToUser {}
unsafe impl Send for X86CopyStrnFromUser {}

pub unsafe fn try_copy_from_user(src: UA, dst: *mut (), len: usize) -> Result<()> {
    if src.value().checked_add(len).map_or(true, |end| end > 0x0000800000000000) {
        return Err(KernelError::Fault);
    }
    unsafe {
        core::ptr::copy_nonoverlapping(src.value() as *const u8, dst as *mut u8, len);
    }
    Ok(())
}

pub unsafe fn try_copy_to_user(src: *const (), dst: UA, len: usize) -> Result<()> {
    if dst.value().checked_add(len).map_or(true, |end| end > 0x0000800000000000) {
        return Err(KernelError::Fault);
    }
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
        if self.src.value().checked_add(self.len).map_or(true, |end| end > 0x0000800000000000) {
            return Poll::Ready(Err(KernelError::Fault));
        }
        let src_ptr = self.src.value() as *const u8;
        let mut i = 0;
        while i < self.len {
            unsafe {
                let c = *src_ptr.add(i);
                *self.dst.add(i) = c;
                if c == 0 {
                    return Poll::Ready(Ok(i));
                }
            }
            i += 1;
        }
        Poll::Ready(Ok(i))
    }
}
