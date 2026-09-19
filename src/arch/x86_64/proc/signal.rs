//! x86_64 signal frame and context.
//!
//! Defines the rt_sigframe layout for x86_64 Linux ABI signal delivery.
//! The layout must match the kernel's signal frame so that sigreturn
//! can restore the interrupted context.

use crate::process::task::{Task, task_struct};
use libkernel::error::Result;

/// x86_64 general-purpose register set saved in the signal frame.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct X86_64GPRegs {
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
    pub orig_rax: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
    pub fs_base: u64,
    pub gs_base: u64,
    pub ds: u64,
    pub es: u64,
    pub fs: u64,
    pub gs: u64,
}

/// x86_64 signal frame.
///
//! The layout must match the x86_64 rt_sigframe ABI:
//! - rt_sigframe.header (siginfo_t + ucontext pointer)
//! - rt_sigframe.uc (ucontext with saved registers)
#[repr(C)]
#[derive(Debug, Clone)]
pub struct RtSigFrame {
    // TODO(#17): Populate with actual rt_sigframe layout.
    // pub header: SigInfoHeader,
    // pub uc: UContext,
    pub _reserved: [u8; 512],
}

/// Initialize the signal frame for signal delivery.
pub fn setup_sig_frame(
    task: &mut Task<task_struct>,
    signo: usize,
    handler: usize,
    _oldcontext: usize,
    _new_sp: usize,
) -> Result<()> {
    // TODO(#17): Build the rt_sigframe on the task's kernel stack
    // so that sigreturn restores the interrupted context.
    todo!("x86_64 setup_sig_frame")
}
