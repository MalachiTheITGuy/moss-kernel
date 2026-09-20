//! x86_64 CPU operation primitives.
//!
//! Low-level CPU instructions for interrupt control, memory barriers,
//! and halt operations.

use libkernel::CpuOps;

/// x86_64 interrupt flags type.
#[derive(Clone, Copy)]
pub struct X86_64InterruptFlags(u64);

impl CpuOps for super::X86_64 {
    type InterruptFlags = X86_64InterruptFlags;

    fn id() -> usize {
        0
    }

    fn enable_interrupts() {
        // SAFETY: STI is a user-privileged instruction but only called
        // from kernel context when we explicitly want interrupts enabled.
        unsafe { core::arch::asm!("sti", options(nomem, nostack)) };
    }

    fn disable_interrupts() -> Self::InterruptFlags {
        // SAFETY: CLI is a user-privileged instruction but only called
        // from kernel context when we explicitly want interrupts disabled.
        let flags: u64;
        unsafe {
            core::arch::asm!("pushfq; pop {f}", f = out(reg) flags);
            core::arch::asm!("cli", options(nomem, nostack));
        }
        X86_64InterruptFlags(flags)
    }

    fn restore_interrupt_state(flags: Self::InterruptFlags) {
        // SAFETY: Restoring RFLAGS from a saved state captured by
        // disable_interrupts. The IF bit in RFLAGS controls interrupts.
        unsafe {
            core::arch::asm!("push {f}; popfq", f = in(reg) flags.0, options(nomem, nostack));
        }
    }

    fn halt() -> ! {
        // SAFETY: HLT halts the CPU until the next interrupt. Safe to
        // call from kernel context.
        loop {
            unsafe { core::arch::asm!("hlt", options(nomem, nostack)) };
        }
    }
}
