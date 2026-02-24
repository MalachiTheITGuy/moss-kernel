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
    sync::SpinLock,
};

pub mod boot;
pub mod cpu_ops;
pub mod exceptions;
pub mod interrupts;
pub mod memory;
pub mod proc;
pub mod ptrace;

use self::exceptions::ExceptionState;
use self::memory::mmu::{X86_64KernelAddressSpace, KERN_ADDR_SPC};
use self::memory::address_space::X86_64ProcessAddressSpace;
use self::memory::uaccess::{X86CopyFromUser, X86CopyToUser, X86CopyStrnFromUser, try_copy_from_user};
use self::ptrace::X86_64PtraceGPRegs;
use libkernel::arch::x86_64::memory::pg_tables::{PML4Table, PgTableArray};

pub struct X86_64 {}

/// User code segment selector: GDT entry 5, RPL=3 (0x2b)
const USER_CS: u64 = (5 << 3) | 3;
/// User data/stack segment selector: GDT entry 4, RPL=3 (0x23)
const USER_SS: u64 = (4 << 3) | 3;

impl crate::arch::Arch for X86_64 {
    type UserContext = ExceptionState;
    type PTraceGpRegs = X86_64PtraceGPRegs;

    fn name() -> &'static str {
        "x86_64"
    }

    fn cpu_count() -> usize {
        1 // TODO: Support SMP
    }

    fn new_user_context(entry_point: VA, stack_top: VA) -> Self::UserContext {
        ExceptionState {
            rax: 0, rcx: 0, rdx: 0, rbx: 0, rbp: 0, rsi: 0, rdi: 0,
            r8: 0, r9: 0, r10: 0, r11: 0, r12: 0, r13: 0, r14: 0, r15: 0,
            vector: 0,
            error_code: 0,
            rip: entry_point.value() as _,
            cs: USER_CS,
            rflags: 0x202, // IF
            rsp: stack_top.value() as _,
            ss: USER_SS,
        }
    }

    fn set_user_return_value(context: &mut Self::UserContext, val: usize) {
        context.rax = val as u64;
    }

    fn set_user_stack(context: &mut Self::UserContext, sp: VA) {
        context.rsp = sp.value() as u64;
    }

    fn set_user_thread_area(_context: &mut Self::UserContext, _area: VA) {
        // TODO: Handle FS_BASE/GS_BASE in Task
    }

    fn context_switch(new: Arc<Task>) {
        proc::context_switch(new);
    }

    fn create_idle_task() -> OwnedTask {
        proc::idle::create_idle_task()
    }

    fn power_off() -> ! {
        loop {
            unsafe { core::arch::asm!("hlt") };
        }
    }

    fn restart() -> ! {
        Self::power_off()
    }

    fn get_cmdline() -> Option<String> {
        self::boot::get_cmdline()
    }

    fn do_signal(
        _sig: SigId,
        _action: UserspaceSigAction,
    ) -> impl Future<Output = Result<Self::UserContext>> {
        async { Err(libkernel::error::KernelError::NotSupported) }
    }

    fn do_signal_return() -> impl Future<Output = Result<Self::UserContext>> {
        async { Err(libkernel::error::KernelError::NotSupported) }
    }

    unsafe fn copy_from_user(
        src: UA,
        dst: *mut (),
        len: usize,
    ) -> impl Future<Output = Result<()>> {
        X86CopyFromUser::new(src, dst, len)
    }

    unsafe fn try_copy_from_user(src: UA, dst: *mut (), len: usize) -> Result<()> {
        unsafe { try_copy_from_user(src, dst, len) }
    }

    unsafe fn copy_to_user(
        src: *const (),
        dst: UA,
        len: usize,
    ) -> impl Future<Output = Result<()>> {
        X86CopyToUser::new(src, dst, len)
    }

    unsafe fn copy_strn_from_user(
        src: UA,
        dst: *mut u8,
        len: usize,
    ) -> impl Future<Output = Result<usize>> {
        X86CopyStrnFromUser::new(src, dst, len)
    }
}

impl VirtualMemory for X86_64 {
    type PageTableRoot = PgTableArray<PML4Table>;
    type ProcessAddressSpace = X86_64ProcessAddressSpace;
    type KernelAddressSpace = X86_64KernelAddressSpace;

    const PAGE_OFFSET: usize = self::memory::PAGE_OFFSET;

    fn kern_address_space() -> &'static SpinLock<Self::KernelAddressSpace> {
        KERN_ADDR_SPC.get().expect("Kernel address space not initialized")
    }
}
