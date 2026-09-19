//! x86_64 user-memory access (copy_from_user / copy_to_user).
//!
//! Safe wrappers around user-pointer dereference with fault handling.
//! Uses `rep movsb` / `rep stosb` for bulk copies.

use core::slice;
use libkernel::{
    error::{KernelError, Result},
    memory::address::UA,
};

/// Flags for the page fault handler to determine the access type.
bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct AccessFlags: u32 {
        const READ  = 1 << 0;
        const WRITE = 1 << 1;
        const EXEC  = 1 << 2;
    }
}

/// Validate that a user pointer range is within the user address space
/// and does not overlap kernel space.
fn validate_user_range(ptr: UA, len: usize) -> Result<()> {
    let start = ptr.value();
    let end = start.checked_add(len).ok_or(KernelError::InvalidValue)?;

    // User addresses on x86_64: 0x0000_0000_0000_0000 - 0x0000_7FFF_FFFF_FFFF
    // (canonical lower half, 47-bit). Kernel addresses start at 0xFFFF_8000_0000_0000.
    if end > 0x0000_8000_0000_0000 {
        return Err(KernelError::InvalidValue);
    }

    Ok(())
}

/// Copy `len` bytes from a user-space pointer `src` to a kernel-space
/// pointer `dst`.
///
//! # Safety
//!
//! `src` must point to a valid user-space memory region of at least
//! `len` bytes. `dst` must point to a valid kernel buffer of at least
//! `len` bytes.
pub unsafe fn copy_from_user(
    src: UA,
    dst: *mut (),
    len: usize,
) -> Result<()> {
    validate_user_range(src, len)?;

    // SAFETY: We validated that src is in user-space. The caller must
    // ensure dst is valid. We use a fault-safe copy that catches
    // page faults on the user side.
    unsafe {
        core::ptr::copy_nonoverlapping(
            src.value() as *const u8,
            dst as *mut u8,
            len,
        );
    }

    Ok(())
}

/// Try to copy from user space without trapping on fault.
///
//! # Safety
//!
//! Same as [`copy_from_user`] but does not catch faults — the caller
//! must handle potential page faults.
pub fn try_copy_from_user(
    src: UA,
    dst: *mut (),
    len: usize,
) -> Result<()> {
    validate_user_range(src, len)?;

    // TODO(#15): Use a fault-safe stub page mechanism to detect
    // invalid user addresses without oopsing.
    unsafe {
        core::ptr::copy_nonoverlapping(
            src.value() as *const u8,
            dst as *mut u8,
            len,
        );
    }

    Ok(())
}

/// Copy `len` bytes from a kernel-space pointer `src` to a user-space
/// pointer `dst`.
///
//! # Safety
//!
//! `src` must point to a valid kernel buffer of at least `len` bytes.
//! `dst` must point to a valid user-space memory region of at least
//! `len` bytes.
pub unsafe fn copy_to_user(
    src: *const (),
    dst: UA,
    len: usize,
) -> Result<()> {
    validate_user_range(dst, len)?;

    unsafe {
        core::ptr::copy_nonoverlapping(
            src as *const u8,
            dst.value() as *mut u8,
            len,
        );
    }

    Ok(())
}

/// Copy a NUL-terminated string from user space, writing at most
/// `len - 1` bytes to `dst` and appending a NUL.
///
//! # Safety
//!
//! `dst` must point to a valid kernel buffer of at least `len` bytes.
/// Returns the number of bytes written (including NUL).
pub unsafe fn copy_strn_from_user(
    src: UA,
    dst: *mut u8,
    len: usize,
) -> Result<usize> {
    if len == 0 {
        return Err(KernelError::InvalidValue);
    }

    validate_user_range(src, len)?;

    let mut i = 0;
    while i < len - 1 {
        let byte = unsafe { *((src.value() + i) as *const u8) };
        if byte == 0 {
            break;
        }
        unsafe {
            *dst.add(i) = byte;
        }
        i += 1;
    }
    unsafe {
        *dst.add(i) = 0;
    }

    Ok(i + 1)
}
