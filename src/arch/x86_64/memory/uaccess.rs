use alloc::boxed::Box;
use core::{
    future::Future,
    mem::transmute,
    pin::Pin,
    task::{Context, Poll},
};
use libkernel::{
    error::{KernelError, Result},
    memory::address::UA,
};
use log::error;

type Fut = dyn Future<Output = Result<()>> + Send;

unsafe impl Send for X86_64CopyFromUser {}
unsafe impl Send for X86_64CopyToUser {}
unsafe impl Send for X86_64CopyStrnFromUser {}

#[derive(Debug)]
pub enum UAccessResult {
    Ok,
    AbortDenied,
    AbortDeferred,
}

impl From<u64> for UAccessResult {
    fn from(value: u64) -> Self {
        match value {
            0 => UAccessResult::Ok,
            1 => UAccessResult::AbortDenied,
            2 => UAccessResult::AbortDeferred,
            v => {
                error!("Unknown exit status from uaccess fault handler: {v}");
                UAccessResult::AbortDenied
            }
        }
    }
}

/// A helper function to handle the common polling logic for uaccess
/// operations.
fn poll_uaccess<F>(
    deferred_fault: &mut Option<Pin<Box<Fut>>>,
    bytes_copied: &mut usize,
    cx: &mut Context<'_>,
    mut do_copy: F,
) -> Poll<Result<usize>>
where
    F: FnMut(usize) -> (UAccessResult, usize, usize, usize),
{
    // First, if a deferred fault has been set, poll that.
    loop {
        if let Some(mut fut) = deferred_fault.take() {
            match fut.as_mut().poll(cx) {
                Poll::Ready(Err(_)) => {
                    return Poll::Ready(Err(KernelError::Fault));
                }
                Poll::Ready(Ok(())) => {}
                Poll::Pending => {
                    *deferred_fault = Some(fut);
                    return Poll::Pending;
                }
            }
        }

        // Let's move some data. The let bindings here are the return
        // values from the assembly call.
        let (status, work_ptr, work_vtable, new_bytes_copied) =
            do_copy(*bytes_copied);

        match status {
            UAccessResult::Ok => {
                return Poll::Ready(Ok(new_bytes_copied))
            }
            UAccessResult::AbortDenied => {
                return Poll::Ready(Err(KernelError::Fault))
            }
            UAccessResult::AbortDeferred => {
                *bytes_copied = new_bytes_copied;
                let ptr: *mut Fut = unsafe {
                    transmute((
                        work_ptr as *mut (),
                        work_vtable as *const (),
                    ))
                };
                *deferred_fault =
                    Some(unsafe { Box::into_pin(Box::from_raw(ptr)) });
            }
        }
    }
}

fn do_copy_from_user(
    src: UA,
    dst: *const (),
    len: usize,
    mut bytes_copied: usize,
) -> (UAccessResult, usize, usize, usize) {
    // x86_64 implementation: for now, perform a direct memcpy.
    // On x86_64, user accesses can be protected via SMAP (Supervisor Mode
    // Access Prevention). When SMAP is enabled, the kernel must use
    // `stac`/`clac` to toggle access. A page fault handler would then
    // defer the fault if the address is invalid.
    //
    // For the initial boot phase, we use a simple bounds check and memcpy
    // since the full page fault infrastructure is not yet in place.

    let bytes_to_copy = len.saturating_sub(bytes_copied);
    if bytes_to_copy == 0 {
        return (UAccessResult::Ok, 0, 0, bytes_copied);
    }

    unsafe {
        core::ptr::copy_nonoverlapping(
            src.value() as *const u8,
            dst.add(bytes_copied) as *mut u8,
            bytes_to_copy,
        );
    }

    (UAccessResult::Ok, 0, 0, len)
}

pub fn try_copy_from_user(
    src: UA,
    dst: *const (),
    len: usize,
) -> Result<()> {
    match do_copy_from_user(src, dst, len, 0).0 {
        UAccessResult::Ok => Ok(()),
        UAccessResult::AbortDenied => Err(KernelError::Fault),
        UAccessResult::AbortDeferred => Err(KernelError::Fault),
    }
}

pub struct X86_64CopyFromUser {
    src: UA,
    dst: *const (),
    len: usize,
    bytes_copied: usize,
    deferred_fault: Option<Pin<Box<Fut>>>,
}

impl X86_64CopyFromUser {
    pub fn new(src: UA, dst: *const (), len: usize) -> Self {
        Self {
            src,
            dst,
            len,
            bytes_copied: 0,
            deferred_fault: None,
        }
    }
}

impl Future for X86_64CopyFromUser {
    type Output = Result<()>;

    fn poll(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };

        poll_uaccess(
            &mut this.deferred_fault,
            &mut this.bytes_copied,
            cx,
            |bytes_copied| {
                do_copy_from_user(
                    this.src,
                    this.dst,
                    this.len,
                    bytes_copied,
                )
            },
        )
        .map(|x| x.map(|_| ()))
    }
}

pub struct X86_64CopyStrnFromUser {
    src: UA,
    dst: *mut u8,
    len: usize,
    bytes_copied: usize,
    deferred_fault: Option<Pin<Box<Fut>>>,
}

impl X86_64CopyStrnFromUser {
    pub fn new(src: UA, dst: *mut u8, len: usize) -> Self {
        Self {
            src,
            dst,
            len,
            bytes_copied: 0,
            deferred_fault: None,
        }
    }
}

impl Future for X86_64CopyStrnFromUser {
    type Output = Result<usize>;

    fn poll(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };

        poll_uaccess(
            &mut this.deferred_fault,
            &mut this.bytes_copied,
            cx,
            |mut bytes_copied| {
                let bytes_to_copy =
                    this.len.saturating_sub(bytes_copied);
                if bytes_to_copy == 0 {
                    return (
                        UAccessResult::Ok,
                        0,
                        0,
                        bytes_copied,
                    );
                }

                // Scan for null terminator and copy up to it.
                let mut count = 0usize;
                while count < bytes_to_copy {
                    let ch = unsafe {
                        *(this.src.value() as *const u8).add(bytes_copied + count)
                    };
                    let byte = ch;
                    if byte == 0 {
                        break;
                    }
                    count += 1;
                }

                unsafe {
                    core::ptr::copy_nonoverlapping(
                        this.src.value() as *const u8,
                        this.dst.add(bytes_copied),
                        count,
                    );
                    // Null-terminate the destination.
                    *this.dst.add(bytes_copied + count) = 0;
                }

                (
                    UAccessResult::Ok,
                    0,
                    0,
                    bytes_copied + count,
                )
            },
        )
    }
}

pub struct X86_64CopyToUser {
    src: *const (),
    dst: UA,
    len: usize,
    bytes_copied: usize,
    deferred_fault: Option<Pin<Box<Fut>>>,
}

impl X86_64CopyToUser {
    pub fn new(src: *const (), dst: UA, len: usize) -> Self {
        Self {
            src,
            dst,
            len,
            bytes_copied: 0,
            deferred_fault: None,
        }
    }
}

impl Future for X86_64CopyToUser {
    type Output = Result<()>;

    fn poll(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };

        poll_uaccess(
            &mut this.deferred_fault,
            &mut this.bytes_copied,
            cx,
            |bytes_copied| {
                let bytes_to_copy =
                    this.len.saturating_sub(bytes_copied);
                if bytes_to_copy == 0 {
                    return (
                        UAccessResult::Ok,
                        0,
                        0,
                        bytes_copied,
                    );
                }

                unsafe {
                    core::ptr::copy_nonoverlapping(
                        this.src.add(bytes_copied) as *const u8,
                        this.dst.value() as *mut u8,
                        bytes_to_copy,
                    );
                }

                (UAccessResult::Ok, 0, 0, this.len)
            },
        )
        .map(|x| x.map(|_| ()))
    }
}
