//! x86_64 ptrace register layout.
//!
//! Defines the general-purpose register layout exposed to userspace via
//! `PTRACE_GETREGSET` / `PTRACE_SETREGSET`, matching the Linux
//! `user_regs_struct` for x86_64.

use super::exceptions::ExceptionState;
use crate::memory::uaccess::UserCopyable;

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
    /// Create a new register set with the given instruction and stack pointers.
    pub fn new(rip: u64, rsp: u64) -> Self {
        Self {
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
            rip,
            cs: 0,
            rflags: 0,
            rsp,
            ss: 0,
            fs_base: 0,
            gs_base: 0,
            ds: 0,
            es: 0,
            fs: 0,
            gs: 0,
        }
    }

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

impl From<[u64; 27]> for X86_64PtraceGPRegs {
    fn from(regs: [u64; 27]) -> Self {
        Self {
            r15: regs[0],
            r14: regs[1],
            r13: regs[2],
            r12: regs[3],
            rbp: regs[4],
            rbx: regs[5],
            r11: regs[6],
            r10: regs[7],
            r9: regs[8],
            r8: regs[9],
            rax: regs[10],
            rcx: regs[11],
            rdx: regs[12],
            rsi: regs[13],
            rdi: regs[14],
            orig_rax: regs[15],
            rip: regs[16],
            cs: regs[17],
            rflags: regs[18],
            rsp: regs[19],
            ss: regs[20],
            fs_base: regs[21],
            gs_base: regs[22],
            ds: regs[23],
            es: regs[24],
            fs: regs[25],
            gs: regs[26],
        }
    }
}

impl From<&X86_64PtraceGPRegs> for [u64; 27] {
    fn from(regs: &X86_64PtraceGPRegs) -> Self {
        [
            regs.r15,
            regs.r14,
            regs.r13,
            regs.r12,
            regs.rbp,
            regs.rbx,
            regs.r11,
            regs.r10,
            regs.r9,
            regs.r8,
            regs.rax,
            regs.rcx,
            regs.rdx,
            regs.rsi,
            regs.rdi,
            regs.orig_rax,
            regs.rip,
            regs.cs,
            regs.rflags,
            regs.rsp,
            regs.ss,
            regs.fs_base,
            regs.gs_base,
            regs.ds,
            regs.es,
            regs.fs,
            regs.gs,
        ]
    }
}

// ──────────────────────────────────────────────
//  ExceptionState ↔ GPRegs conversions
// ──────────────────────────────────────────────

/// Convert a CPU exception frame into the userspace-visible register set.
///
/// Copies the 20 common GP, IP, and segment fields directly.
/// `vector_num` and `error_code` from the exception frame are dropped.
/// The remaining fields (`orig_rax`, `fs_base`, `gs_base`, `ds`, `es`,
/// `fs`, `gs`) are zeroed; they are not saved by the assembly entry
/// stubs and will be populated on a per-use basis (e.g. `orig_rax` is
/// set by the syscall entry path).
impl From<&ExceptionState> for X86_64PtraceGPRegs {
    fn from(state: &ExceptionState) -> Self {
        Self {
            // Common GP registers
            r15: state.r15,
            r14: state.r14,
            r13: state.r13,
            r12: state.r12,
            rbp: state.rbp,
            rbx: state.rbx,
            r11: state.r11,
            r10: state.r10,
            r9: state.r9,
            r8: state.r8,
            rax: state.rax,
            rcx: state.rcx,
            rdx: state.rdx,
            rsi: state.rsi,
            rdi: state.rdi,
            // Control registers
            rip: state.rip,
            cs: state.cs,
            rflags: state.rflags,
            rsp: state.rsp,
            ss: state.ss,
            // Fields not present in ExceptionState — zeroed.
            orig_rax: 0,
            fs_base: 0,
            gs_base: 0,
            ds: 0,
            es: 0,
            fs: 0,
            gs: 0,
        }
    }
}

/// Convert a userspace register set back into a CPU exception frame.
///
/// Restores the 20 common fields from the GPRegs.  `vector_num` and
/// `error_code` are set to zero; the caller must fix them up before
/// returning to userspace (or they are irrelevant for `sysretq`).
/// The extra GPRegs-only fields (`orig_rax`, `fs_base`, `gs_base`,
/// `ds`, `es`, `fs`, `gs`) are silently dropped.
impl From<&X86_64PtraceGPRegs> for ExceptionState {
    fn from(regs: &X86_64PtraceGPRegs) -> Self {
        Self {
            r15: regs.r15,
            r14: regs.r14,
            r13: regs.r13,
            r12: regs.r12,
            rbp: regs.rbp,
            rbx: regs.rbx,
            r11: regs.r11,
            r10: regs.r10,
            r9: regs.r9,
            r8: regs.r8,
            rax: regs.rax,
            rcx: regs.rcx,
            rdx: regs.rdx,
            rsi: regs.rsi,
            rdi: regs.rdi,
            vector_num: 0,
            error_code: 0,
            rip: regs.rip,
            cs: regs.cs,
            rflags: regs.rflags,
            rsp: regs.rsp,
            ss: regs.ss,
        }
    }
}

// Identity From impl required by the Arch trait bound:
// `type PTraceGpRegs: for<'a> From<&'a Self::UserContext>;`
impl From<&X86_64PtraceGPRegs> for X86_64PtraceGPRegs {
    fn from(regs: &X86_64PtraceGPRegs) -> Self {
        *regs
    }
}
