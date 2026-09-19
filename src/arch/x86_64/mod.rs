//! x86_64 architecture implementation.
//!
//! This module provides the x86_64 implementation of the kernel's
//! architecture abstraction layer.

mod boot;
pub mod cpu_ops;
pub mod exceptions;
pub mod memory;
pub mod proc;
pub mod ptrace;

use crate::arch::Arch;
use crate::process::{Task, owned::OwnedTask};
use crate::sched::syscall_ctx::ProcessCtx;
use crate::process::thread_group::signal::{SigId, ksigaction::UserspaceSigAction};
use alloc::string::String;
use alloc::sync::Arc;
use libkernel::{
    CpuOps,
    error::Result,
    memory::{
        address::{UA, VA},
        proc_vm::address_space::VirtualMemory,
    },
};

/// x86_64 kernel virtual base address.
pub const KERNEL_BASE: usize = 0xFFFF_FFFF_8000_0000;

/// x86_64 direct-mapped physical memory offset.
pub const PAGE_OFFSET: usize = 0xFFFF_8880_0000_0000;

/// x86_64 implementation of the `Arch` trait.
pub struct X86_64;

impl CpuOps for X86_64 {
    fn enable_interrupts() {
        unsafe { core::arch::asm!("sti", options(nomem, nostack)) };
    }

    fn disable_interrupts() {
        unsafe { core::arch::asm!("cli", options(nomem, nostack)) };
    }

    fn halt() {
        unsafe { core::arch::asm!("hlt", options(nomem, nostack)) };
    }

    fn pause() {
        unsafe { core::arch::asm!("pause", options(nomem, nostack)) };
    }

    fn sfence() {
        unsafe { core::arch::asm!("sfence", options(nomem, nostack)) };
    }
}

impl VirtualMemory for X86_64 {
    // Page table operations will be implemented in Phase 2 (Issue #15).
}

impl Arch for X86_64 {
    type UserContext = crate::arch::x86_64::ptrace::X86_64PtraceGPRegs;
    type PTraceGpRegs = crate::arch::x86_64::ptrace::X86_64PtraceGPRegs;

    const PAGE_OFFSET: usize = PAGE_OFFSET;

    fn name() -> &'static str {
        "x86_64"
    }

    fn cpu_count() -> usize {
        // TODO(#14): Query from ACPI/MADT.
        1
    }

    fn new_user_context(entry_point: VA, stack_top: VA) -> Self::UserContext {
        // TODO(#17): Set up x86_64 user-mode context.
        // RFLAGS.IF=1, CS=0x23 (user CS), SS=0x2B (user SS),
        // RIP=entry_point, RSP=stack_top.
        todo!("new_user_context: x86_64 user-mode context setup")
    }

    fn context_switch(new: Arc<Task>) {
        // TODO(#17): x86_64 context switch via CR3 + FXSAVE/FXRSTOR + stack swap.
        todo!("context_switch: x86_64 context switch")
    }

    fn create_idle_task() -> OwnedTask {
        crate::arch::x86_64::proc::idle::create_idle_task()
    }

    fn power_off() -> ! {
        use acpi::fadt::PowerManagementProfile;
        // TODO(#18): Attempt ACPI power-off, then fall back to APM.
        crate::arch::CpuOps::disable_interrupts();
        loop {
            crate::arch::CpuOps::halt();
        }
    }

    fn restart() -> ! {
        // TODO(#18): Triple-fault or ACPI reset.
        crate::arch::CpuOps::disable_interrupts();
        loop {
            crate::arch::CpuOps::halt();
        }
    }

    fn get_cmdline() -> Option<String> {
        // TODO(#14): Parse kernel command line from Multiboot2 info.
        None
    }

    async fn do_signal(
        ctx: ProcessCtx,
        sig: SigId,
        action: crate::process::thread_group::signal::ksigaction::UserspaceSigAction,
    ) -> Result<Self::UserContext> {
        crate::arch::x86_64::proc::signal::do_signal(ctx, sig, action).await
    }

    async fn do_signal_return(ctx: ProcessCtx) -> Result<Self::UserContext> {
        crate::arch::x86_64::proc::signal::do_signal_return(ctx).await
    }

    unsafe fn copy_from_user(
        src: UA,
        dst: *mut (),
        len: usize,
    ) -> Result<()> {
        crate::arch::x86_64::memory::uaccess::copy_from_user(src, dst, len).await
    }

    unsafe fn try_copy_from_user(
        src: UA,
        dst: *mut (),
        len: usize,
    ) -> Result<()> {
        crate::arch::x86_64::memory::uaccess::try_copy_from_user(src, dst, len)
    }

    unsafe fn copy_to_user(
        src: *const (),
        dst: UA,
        len: usize,
    ) -> Result<()> {
        crate::arch::x86_64::memory::uaccess::copy_to_user(src, dst, len).await
    }

    unsafe fn copy_strn_from_user(
        src: UA,
        dst: *mut u8,
        len: usize,
    ) -> Result<usize> {
        crate::arch::x86_64::memory::uaccess::copy_strn_from_user(src, dst, len).await
    }
}