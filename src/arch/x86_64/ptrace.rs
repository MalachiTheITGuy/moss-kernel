//! x86_64 ptrace register layout.
//!
//! Defines the general-purpose register layout exposed to userspace via
//! `PTRACE_GETREGSET` / `PTRACE_SETREGSET`, matching the Linux
//! `user_regs_struct` for x86_64.

use crate::memory::uaccess::UserCopyable;
use core::arch::x86_64::Xmm Registers;

/// x86_64 general-purpose register set for ptrace.
///
/// Layout matches Linux `struct user_regs_struct`:
/// <https://github.com/torvalds/linux/blob/master/arch/x86/include/uapi/asm/ptrace.h>
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct X86_64PtraceGPRegs {
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

// SAFETY: The register layout is a plain #[repr(C)] struct of u64 fields.
// All fields are initialized to zero by default, which is a valid
// register state (RIP=0 will fault on first use, which is the desired
// behavior for an uninitialized context).
unsafe impl UserCopyable for X86_64PtraceGPRegs {}

impl Default for X86_64PtraceGPRegs {
    fn default() -> Self {
        // SAFETY: Zeros are a valid register state for this struct.
        // An all-zero register context will fault on first use (RIP=0),
        // which provides clear debugging information.
        unsafe { core::mem::zeroed() }
    }
}

impl X86_64PtraceGPRegs {
    /// Get the instruction pointer.
    pub fn ip(&self) -> usize {
        self.rip as usize
    }

    /// Get the stack pointer.
    pub fn sp(&self) -> usize {
        self.rsp as usize
    }

    /// Get the base pointer.
    pub fn bp(&self) -> usize {
        self.rbp as usize
    }
}
