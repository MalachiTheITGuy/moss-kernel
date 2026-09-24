//! x86_64 idle thread implementation.
//!
//! Mirrors `src/arch/arm64/proc/idle.rs`. The idle thread maps a tiny
//! userspace code page containing an HLT loop and executes there so
//! that the standard exception-return path (`iretq`) handles all
//! register restore.

use super::super::ptrace::X86_64PtraceGPRegs;
use crate::{
    memory::{page::ClaimedPage, PageOffsetTranslator},
    process::owned::OwnedTask,
};
use libkernel::memory::{
    address::VA,
    paging::permissions::PtePermissions,
    proc_vm::{
        address_space::{UserAddressSpace, VirtualMemory},
        vmarea::{VMAPermissions, VMArea, VMAreaKind},
    },
    region::VirtMemoryRegion,
};

use crate::arch::ArchImpl;

/// User-visible code segment and stack segment selectors.
const USER_CS: u64 = 0x0033;
const USER_SS: u64 = 0x002b;

/// RFLAGS with interrupts enabled (bit 9).
const RFLAGS_IF: u64 = 0x200;

/// Fixed virtual address for the idle code page.
const IDLE_CODE_VA: u64 = 0xd00d_0000;

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
/// Allocates a single page, copies the HLT-loop code image (from
/// `idle.s`) into it, maps it in a fresh user address space at
/// [`IDLE_CODE_VA`], and hands the result to
/// [`OwnedTask::create_idle_task`].
///
/// This mirrors `src/arch/arm64/proc/idle.rs` with x86_64 register
/// conventions: the initial user context sets RIP to the code page,
/// CS/SS to ring-3 selectors, and RFLAGS with interrupts enabled.
pub fn create_idle_task() -> OwnedTask {
    // 1.  Allocate a zeroed physical page for the code image.
    let code_page = ClaimedPage::alloc_zeroed().unwrap().leak();
    let code_addr = VA::from_value(IDLE_CODE_VA as usize);

    // 2.  Obtain the linker-asm code image bounds.
    unsafe extern "C" {
        static __idle_start: u8;
        static __idle_end: u8;
    }

    let idle_start_ptr = unsafe { &__idle_start } as *const u8;
    let idle_end_ptr = unsafe { &__idle_end } as *const u8;
    let code_sz = idle_end_ptr.addr() - idle_start_ptr.addr();

    // 3.  Copy the code image into the physical page (via its kernel VA).
    unsafe {
        idle_start_ptr.copy_to(
            code_page
                .pa()
                .to_va::<PageOffsetTranslator>()
                .cast::<u8>()
                .as_ptr_mut(),
            code_sz,
        );
    }

    // 4.  Create a fresh user address space and map the code page.
    let mut addr_space = <ArchImpl as VirtualMemory>::ProcessAddressSpace::new().unwrap();

    addr_space
        .map_page(code_page, code_addr, PtePermissions::rx(true))
        .unwrap();

    // 5.  Build the initial userspace register context.
    //
    // The idle loop runs in ring 3 (matching the Linux userspace
    // dispatch path).  CS and SS carry the ring-3 selectors and
    // RFLAGS has the IF bit set so that interrupts are unmasked.
    let ctx = X86_64PtraceGPRegs {
        r15: 0,
        r14: 0,
        r13: 0,
        r12: 0,
        rbp: 0,
        rbx: 0,
        r11: 0,
        r10: 0,
        r9: 0,
        r8: 0,
        rax: 0,
        rcx: 0,
        rdx: 0,
        rsi: 0,
        rdi: 0,
        orig_rax: 0,
        rip: IDLE_CODE_VA,
        cs: USER_CS,
        rflags: RFLAGS_IF,
        rsp: 0,
        ss: USER_SS,
        fs_base: 0,
        gs_base: 0,
        ds: 0,
        es: 0,
        fs: 0,
        gs: 0,
    };

    // 6.  Describe the code mapping for the process VM.
    let code_map = VMArea::new(
        VirtMemoryRegion::new(code_addr, code_sz),
        VMAreaKind::Anon,
        VMAPermissions::rx(),
    );

    OwnedTask::create_idle_task(addr_space, ctx, code_map)
}
