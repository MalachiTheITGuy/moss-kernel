use alloc::string::String;
use alloc::sync::Arc;
use core::future::Future;
use libkernel::{
    CpuOps, VirtualMemory,
    error::Result,
    memory::address::{UA, VA},
};

use crate::{
    process::{
        Task,
        owned::OwnedTask,
        thread_group::signal::{SigId, ksigaction::UserspaceSigAction},
    },
};

use super::Arch;

pub mod boot;
mod cpu_ops;
mod exceptions;
mod memory;
mod proc;
pub mod ptrace;

pub struct X86_64 {}

impl CpuOps for X86_64 {
    fn id() -> usize {
        0 // TODO: CPUID
    }

    fn halt() -> ! {
        loop {
            unsafe { core::arch::asm!("hlt") };
        }
    }

    fn disable_interrupts() -> usize {
        let rflags: usize;
        unsafe {
            core::arch::asm!(
                "pushfq",
                "pop {}",
                "cli",
                out(reg) rflags,
            );
        }
        rflags
    }

    fn restore_interrupt_state(state: usize) {
        unsafe {
            core::arch::asm!(
                "push {}",
                "popfq",
                in(reg) state,
            );
        }
    }

    fn enable_interrupts() {
        unsafe { core::arch::asm!("sti") };
    }
}

impl VirtualMemory for X86_64 {
    type PageTableRoot = libkernel::arch::x86_64::memory::pg_tables::PgTableArray<libkernel::arch::x86_64::memory::pg_tables::PML4Table>;
    type ProcessAddressSpace = memory::address_space::X86_64ProcessAddressSpace;
    type KernelAddressSpace = memory::mmu::X86_64KernelAddressSpace;

    const PAGE_OFFSET: usize = 0xffff_8000_0000_0000;

    fn kern_address_space() -> &'static crate::sync::SpinLock<Self::KernelAddressSpace> {
        memory::mmu::KERN_ADDR_SPC.get().unwrap()
    }
}

impl Arch for X86_64 {
    type UserContext = exceptions::ExceptionState;
    type PTraceGpRegs = ptrace::X86_64PtraceGPRegs;

    fn name() -> &'static str {
        "x86_64"
    }

    fn cpu_count() -> usize {
        unimplemented!()
    }

    fn new_user_context(entry_point: VA, stack_top: VA) -> Self::UserContext {
        exceptions::ExceptionState {
            rax: 0, rbx: 0, rcx: 0, rdx: 0,
            rdi: 0, rsi: 0, rbp: 0,
            r8: 0,  r9: 0,  r10: 0, r11: 0,
            r12: 0, r13: 0, r14: 0, r15: 0,
            error_code: 0,
            rip: entry_point.value() as _,
            cs: 0x2b, // User CS (32 | 3)
            rflags: 0x202, // IF | bit 1
            rsp: stack_top.value() as _,
            ss: 0x23, // User SS (24 | 3)
            fs_base: 0,
            gs_base: 0,
        }
    }

    fn set_user_return_value(context: &mut Self::UserContext, val: usize) {
        context.rax = val as _;
    }

    fn set_user_stack(context: &mut Self::UserContext, sp: VA) {
        context.rsp = sp.value() as _;
    }

    fn set_user_thread_area(context: &mut Self::UserContext, area: VA) {
        context.fs_base = area.value() as _;
    }

    fn context_switch(_new: Arc<Task>) {
        unimplemented!()
    }

    fn create_idle_task() -> OwnedTask {
        unimplemented!()
    }

    fn power_off() -> ! {
        unimplemented!()
    }

    fn restart() -> ! {
        unimplemented!()
    }

    fn get_cmdline() -> Option<String> {
        unimplemented!()
    }

    fn do_signal(
        _sig: SigId,
        _action: UserspaceSigAction,
    ) -> impl Future<Output = Result<Self::UserContext>> {
        async { unimplemented!() }
    }

    fn do_signal_return() -> impl Future<Output = Result<Self::UserContext>> {
        async { unimplemented!() }
    }

    unsafe fn copy_from_user(_src: UA, _dst: *mut (), _len: usize) -> impl Future<Output = Result<()>> {
        async { unimplemented!() }
    }

    unsafe fn try_copy_from_user(_src: UA, _dst: *mut (), _len: usize) -> Result<()> {
        unimplemented!()
    }

    unsafe fn copy_to_user(_src: *const (), _dst: UA, _len: usize) -> impl Future<Output = Result<()>> {
        async { unimplemented!() }
    }

    unsafe fn copy_strn_from_user(
        _src: UA,
        _dst: *mut u8,
        _len: usize,
    ) -> impl Future<Output = Result<usize>> {
        async { unimplemented!() }
    }
}
