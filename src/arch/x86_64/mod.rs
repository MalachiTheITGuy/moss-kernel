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
use memory::{
    PAGE_OFFSET,
    mmu::KERN_ADDR_SPC,
    uaccess::{X86_64CopyFromUser, X86_64CopyStrnFromUser, X86_64CopyToUser, try_copy_from_user},
};
use ptrace::X86_64PtraceGPRegs;

use crate::{
    process::{
        Task,
        owned::OwnedTask,
        thread_group::signal::{SigId, ksigaction::UserspaceSigAction},
    },
    sched::syscall_ctx::ProcessCtx,
    sync::SpinLock,
};

use super::Arch;

mod boot;
mod cpu_ops;
mod exceptions;
mod memory;
mod proc;
pub mod portio;
pub mod ptrace;

pub struct X86_64 {}

impl VirtualMemory for X86_64 {
    type PageTableRoot = libkernel::memory::paging::PgTableArray<
        libkernel::arch::x86_64::memory::pg_tables::PML4Table,
    >;
    type ProcessAddressSpace = memory::address_space::X86_64ProcessAddressSpace;
    type KernelAddressSpace = memory::mmu::X86_64KernelAddressSpace;

    fn kern_address_space() -> &'static SpinLock<Self::KernelAddressSpace> {
        KERN_ADDR_SPC.get().unwrap()
    }
}

impl Arch for X86_64 {
    type UserContext = ptrace::X86_64PtraceGPRegs;
    type PTraceGpRegs = X86_64PtraceGPRegs;

    const PAGE_OFFSET: usize = PAGE_OFFSET;

    fn new_user_context(entry_point: VA, stack_top: VA) -> Self::UserContext {
        X86_64PtraceGPRegs::new(entry_point.value() as u64, stack_top.value() as u64)
    }

    fn name() -> &'static str {
        "x86_64"
    }

    fn cpu_count() -> usize {
        // x86_64: single CPU for now (SMP support in Phase 4).
        1
    }

    fn do_signal(
        ctx: ProcessCtx,
        sig: SigId,
        action: UserspaceSigAction,
    ) -> impl Future<Output = Result<<Self as Arch>::UserContext>> {
        proc::signal::do_signal(ctx, sig, action)
    }

    fn do_signal_return(
        ctx: ProcessCtx,
    ) -> impl Future<Output = Result<<Self as Arch>::UserContext>> {
        proc::signal::do_signal_return(ctx)
    }

    fn context_switch(new: Arc<Task>) {
        proc::context_switch(new);
    }

    fn create_idle_task() -> OwnedTask {
        proc::idle::create_idle_task()
    }

    fn power_off() -> ! {
        // x86_64: triple fault to power off, or use ACPI.
        // For now, halt indefinitely.
        loop {
            unsafe {
                core::arch::asm!("hlt");
            }
        }
    }

    fn restart() -> ! {
        // x86_64: trigger a reset via the 8042 keyboard controller
        // or ACPI reset register.
        loop {
            unsafe {
                core::arch::asm!("hlt");
            }
        }
    }

    fn get_cmdline() -> Option<String> {
        let cmd = boot::get_cmdline();
        if cmd.is_empty() {
            None
        } else {
            Some(String::from(cmd))
        }
    }

    unsafe fn copy_from_user(
        src: UA,
        dst: *mut (),
        len: usize,
    ) -> impl Future<Output = Result<()>> {
        X86_64CopyFromUser::new(src, dst, len)
    }

    unsafe fn try_copy_from_user(src: UA, dst: *mut (), len: usize) -> Result<()> {
        try_copy_from_user(src, dst, len)
    }

    unsafe fn copy_to_user(
        src: *const (),
        dst: UA,
        len: usize,
    ) -> impl Future<Output = Result<()>> {
        X86_64CopyToUser::new(src, dst, len)
    }

    unsafe fn copy_strn_from_user(
        src: UA,
        dst: *mut u8,
        len: usize,
    ) -> impl Future<Output = Result<usize>> {
        X86_64CopyStrnFromUser::new(src, dst, len)
    }
}
