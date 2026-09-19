//! x86_64 idle thread implementation.
//!
//! The idle thread executes the HLT instruction in a loop,
//! halting the CPU until the next interrupt arrives.

use crate::process::owned::OwnedTask;

/// Enter the idle loop on x86_64.
///
/// # Safety
///
/// Must only be called from the idle thread context. Halts the
/// CPU until the next interrupt wakes it.
pub fn idle_enter() {
    loop {
        // SAFETY: HLT is a privileged instruction that suspends the
        // CPU until the next interrupt. Safe to call from idle context.
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack));
        }
    }
}

/// Create the idle task for x86_64.
///
/// # Panics
///
/// Always panics with `todo!()` — idle task creation for x86_64
/// is not yet implemented.
pub fn create_idle_task() -> OwnedTask {
    todo!("x86_64 create_idle_task")
}
