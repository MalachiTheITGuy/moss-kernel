//! x86_64 process management.

use crate::process::Task;
use alloc::sync::Arc;

pub mod idle;
pub mod signal;
pub mod vdso;

/// Switch to the address space of `new` by loading its PML4 into CR3.
///
/// On x86_64, register save/restore happens at the exception/interrupt
/// boundary (entry paths and iretq), so the context switch itself only
/// needs to swap the top-level page table — matching the ARM64 pattern
/// where `context_switch` calls `new.vm.activate()` to switch TTBR0/TTBR1.
pub fn context_switch(new: Arc<Task>) {
    new.vm.activate();
}
