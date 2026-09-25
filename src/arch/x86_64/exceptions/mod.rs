//! x86_64 exception and interrupt handling.
//!
//! This module provides the core exception/interrupt infrastructure for
//! x86_64:
//!
//! - [`ExceptionState`] — the register frame saved by the assembly entry
//!   stubs (`entry.S`) and passed to Rust handlers.
//! - [`exception_dispatch`] — the central Rust dispatcher that receives
//!   every exception/interrupt and routes it to the appropriate handler.
//! - [`exceptions_init`] — one-time initialisation of the IDT and
//!   interrupt controller.
//!
//! The assembly entry stubs live in `entry.S` and are linked into the
//! `.text.isr` section.  Each stub pushes a vector number (and an error
//! code or dummy zero) then jumps to `interrupt_common`, which saves all
//! GP registers and calls [`x86_64_interrupt_handler`].

use crate::{
    interrupts::{ClaimedInterrupt, get_interrupt_root},
    sched::{spawn_kernel_work, syscall_ctx::ProcessCtx, uspc_ret::dispatch_userspace_task},
};
use core::{arch::global_asm, fmt::Display};
use libkernel::{error::Result, memory::address::VA};

use super::ptrace::X86_64PtraceGPRegs;

pub mod esr;
pub mod fault;
mod syscall;
pub mod vectors;

// ──────────────────────────────────────────────
//  ExceptionState — register frame
// ──────────────────────────────────────────────

/// Saved register frame for an exception or interrupt.
///
/// The layout **must** match the stack built by `interrupt_common` in
/// `entry.S` (offsets are byte offsets from the base):
///
/// ```text
/// [0x00] r15          [0x58] rcx
/// [0x08] r14          [0x60] rdx
/// [0x10] r13          [0x68] rsi
/// [0x18] r12          [0x70] rdi
/// [0x20] rbp          [0x78] vector_num
/// [0x28] rbx          [0x80] error_code
/// [0x30] r11          [0x88] rip
/// [0x38] r10          [0x90] cs
/// [0x40] r9           [0x98] rflags
/// [0x48] r8           [0xA0] rsp
/// [0x50] rax          [0xA8] ss
/// ```
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExceptionState {
    // ---- GP registers (saved by interrupt_common in entry.S) ----
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub rbp: u64,
    pub rbx: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rax: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,

    // ---- CPU-pushed / stub-pushed fields ----
    pub vector_num: u64,
    pub error_code: u64,

    // ---- Stack frame pushed by CPU (or by stub for ring-3) ----
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

impl Display for ExceptionState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        writeln!(f, "RAX: 0x{:016x}, RBX: 0x{:016x}", self.rax, self.rbx)?;
        writeln!(f, "RCX: 0x{:016x}, RDX: 0x{:016x}", self.rcx, self.rdx)?;
        writeln!(f, "RSI: 0x{:016x}, RDI: 0x{:016x}", self.rsi, self.rdi)?;
        writeln!(f, "RBP: 0x{:016x}, RSP: 0x{:016x}", self.rbp, self.rsp)?;
        writeln!(f, "R8:  0x{:016x}, R9:  0x{:016x}", self.r8, self.r9)?;
        writeln!(f, "R10: 0x{:016x}, R11: 0x{:016x}", self.r10, self.r11)?;
        writeln!(f, "R12: 0x{:016x}, R13: 0x{:016x}", self.r12, self.r13)?;
        writeln!(f, "R14: 0x{:016x}, R15: 0x{:016x}", self.r14, self.r15)?;
        writeln!(f)?;
        writeln!(f, "RIP: 0x{:016x}, CS:  0x{:016x}", self.rip, self.cs)?;
        writeln!(f, "RFLAGS: 0x{:016x}, SS: 0x{:016x}", self.rflags, self.ss)?;
        writeln!(
            f,
            "Vector: {}, Error code: 0x{:016x}",
            self.vector_num, self.error_code
        )
    }
}

// ──────────────────────────────────────────────
//  Exception dispatch
// ──────────────────────────────────────────────

/// Central exception / interrupt dispatcher.
///
/// Called from `interrupt_common` in `entry.S` with a pointer to the
/// saved [`ExceptionState`].  Routes the event based on the vector
/// number:
///
/// - **0–31**: CPU exceptions — forwarded to `fault` handlers or the
///   default panic handler.
/// - **0x80**: System call — dispatched via the syscall handler.
/// - **32–255**: hardware IRQs — dispatched through the root interrupt
///   controller.
///
/// When the exception originates from userspace (ring 3), the full
/// user register state is saved into the current task's context before
/// handling and restored on return via [`dispatch_userspace_task`].
///
/// # Safety
///
/// Called only from assembly entry stubs; the pointer is guaranteed
/// valid for the duration of the call.
#[unsafe(no_mangle)]
unsafe extern "C" fn x86_64_interrupt_handler(state: &mut ExceptionState) {
    let vector = state.vector_num as usize;

    // Determine if we entered from userspace (ring 3) by inspecting
    // the CS segment selector's Requested Privilege Level.
    let from_user = (state.cs & 0x3) == 3;

    // If entering from userspace, snapshot the full user register state
    // into the current task's context so the scheduler can inspect or
    // modify it (e.g. for signal delivery).
    if from_user {
        // SAFETY: Since we've just entered from ring 3, there *cannot*
        // be another syscall currently running for this task, therefore
        // exclusive access to `OwnedTask` is guaranteed.
        let mut ctx = unsafe { ProcessCtx::from_current() };
        let gp_regs = X86_64PtraceGPRegs::from(&*state);
        ctx.task_mut().ctx.save_user_ctx(&gp_regs as *const _);
    }

    match vector {
        // ── CPU exceptions ──
        vectors::PAGE_FAULT => fault::handle_page_fault(state),
        vectors::GENERAL_PROTECTION_FAULT => fault::handle_gp_fault(state),
        vectors::DOUBLE_FAULT => fault::handle_double_fault(state),

        // ── System call (syscall instruction) ──
        vectors::SYSCALL => {
            // SAFETY: We only reach here when from_user is true (syscall
            // transitions ring 3→ring 0), so ProcessCtx::from_current is valid.
            let ctx = unsafe { ProcessCtx::from_current() };
            // SAFETY: The ctx clone won't be polled until
            // `dispatch_userspace_task` at which point this variable will have
            // gone out of scope.
            let mut ctx2 = unsafe { ctx.clone() };
            spawn_kernel_work(&mut ctx2, syscall::handle_syscall(ctx));
        }

        // ── Hardware IRQs ──
        vectors::IRQ_BASE..=255 => {
            // Hardware IRQ — delegate to the root interrupt controller.
            match get_interrupt_root() {
                Some(ref im) => im.handle_interrupt(),
                None => panic!(
                    "IRQ handled before root interrupt controller set.\n{}",
                    state
                ),
            }
        }

        _ => {
            panic!("Unhandled CPU exception.\n{}", state);
        }
    }

    // If we entered from userspace, restore the (possibly different)
    // task's user context and return to userspace.
    if from_user {
        // Allocate space for the restored user context on the stack.
        // The scheduler may have switched tasks inside
        // `dispatch_userspace_task`, so this will be populated with the
        // next task's register state.
        let mut gp_regs = X86_64PtraceGPRegs::new(state.rip, state.rsp);
        dispatch_userspace_task(&mut gp_regs as *mut _);

        // Convert the restored `X86_64PtraceGPRegs` back to an
        // `ExceptionState` and write it to the stack frame so that the
        // assembly return (`iretq`) picks up the correct registers.
        let restored = ExceptionState::from(&gp_regs);
        *state = restored;
    }
}

// ──────────────────────────────────────────────
//  Default unhandled-exception handler
// ──────────────────────────────────────────────

/// Panic with a register dump for an unhandled CPU exception.
pub fn default_handler(state: &ExceptionState) {
    panic!("Unhandled CPU exception.  Program state:\n{state}");
}

// ──────────────────────────────────────────────
//  exceptions_init — one-time boot setup
// ──────────────────────────────────────────────

/// Initialise the x86_64 exception and interrupt subsystem.
///
/// This is called once during `arch_init_stage2` and performs:
///
/// 1. Populates all 256 IDT entries with the assembly stubs from
///    `entry.S`.
/// 2. Sets up the IST (Interrupt Stack Table) entries in the TSS for
///    the double-fault, NMI, and machine-check handlers.
/// 3. Loads the IDT register via `lidt`.
///
/// # Safety
///
/// Must only be called once, with interrupts disabled, during early
/// boot.
pub fn exceptions_init() -> Result<()> {
    unsafe {
        crate::arch::x86_64::boot::idt::setup_idt();
    }

    unsafe {
        syscall::setup_syscall_entry();
    }

    log::info!("moss: x86_64 exceptions & syscall entry ready");

    Ok(())
}
