//! x86_64 CPU operation primitives.
//!
//! Low-level CPU instructions for interrupt control, memory barriers,
//! and halt operations.

use libkernel::CpuOps;

impl CpuOps for super::X86_64 {
    fn enable_interrupts() {
        // SAFETY: STI is a user-privileged instruction but only called
        // from kernel context when we explicitly want interrupts enabled.
        unsafe { core::arch::asm!("sti", options(nomem, nostack)) };
    }

    fn disable_interrupts() {
        // SAFETY: CLI is a user-privileged instruction but only called
        // from kernel context when we explicitly want interrupts disabled.
        unsafe { core::arch::asm!("cli", options(nomem, nostack)) };
    }

    fn halt() {
        // SAFETY: HLT halts the CPU until the next interrupt. Safe to
        // call from kernel context.
        unsafe { core::arch::asm!("hlt", options(nomem, nostack)) };
    }

    fn pause() {
        // SAFETY: PAUSE hints to the processor that we are in a spin-wait
        // loop, improving performance on hyperthreaded CPUs.
        unsafe { core::arch::asm!("pause", options(nomem, nostack)) };
    }

    fn sfence() {
        // SAFETY: SFENCE serializes stores, ensuring memory ordering.
        unsafe { core::arch::asm!("sfence", options(nomem, nostack)) };
    }
}
