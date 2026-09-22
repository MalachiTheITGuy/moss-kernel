//! x86_64 Port I/O primitives.
//!
//! Provides safe(ish) wrappers around the `in` and `out` x86 instructions
//! for communicating with hardware devices on the I/O port bus.
//!
//! All functions are thin wrappers around inline assembly and do not
//! perform any validation beyond what the hardware enforces.

/// Read a byte from the given I/O port.
#[inline]
pub unsafe fn inb(port: u16) -> u8 {
    let val: u8;
    unsafe {
        core::arch::asm!(
            "in al, dx",
            out("al") val,
            in("dx") port,
            options(nomem, nostack),
        );
    }
    val
}

/// Write a byte to the given I/O port.
#[inline]
pub unsafe fn outb(port: u16, val: u8) {
    unsafe {
        core::arch::asm!(
            "out dx, al",
            in("dx") port,
            in("al") val,
            options(nomem, nostack),
        );
    }
}

/// Read a 16-bit word from the given I/O port.
#[inline]
pub unsafe fn inw(port: u16) -> u16 {
    let val: u16;
    unsafe {
        core::arch::asm!(
            "in ax, dx",
            out("ax") val,
            in("dx") port,
            options(nomem, nostack),
        );
    }
    val
}

/// Write a 16-bit word to the given I/O port.
#[inline]
pub unsafe fn outw(port: u16, val: u16) {
    unsafe {
        core::arch::asm!(
            "out dx, ax",
            in("dx") port,
            in("ax") val,
            options(nomem, nostack),
        );
    }
}

/// Read a 32-bit dword from the given I/O port.
#[inline]
pub unsafe fn inl(port: u16) -> u32 {
    let val: u32;
    unsafe {
        core::arch::asm!(
            "in eax, dx",
            out("eax") val,
            in("dx") port,
            options(nomem, nostack),
        );
    }
    val
}

/// Write a 32-bit dword to the given I/O port.
#[inline]
pub unsafe fn outl(port: u16, val: u32) {
    unsafe {
        core::arch::asm!(
            "out dx, eax",
            in("dx") port,
            in("eax") val,
            options(nomem, nostack),
        );
    }
}

/// Delay by doing a dummy I/O read from port 0x84.
///
/// This is a common technique on x86 to enforce a small delay after
/// port I/O writes to slow devices.  The read itself has no meaningful
/// result.
#[inline]
pub fn io_delay() {
    unsafe {
        inb(0x84);
    }
}
