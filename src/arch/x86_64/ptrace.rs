use crate::memory::uaccess::UserCopyable;
use super::exceptions::ExceptionState;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct X86_64PtraceGPRegs {
    pub r15: u64, pub r14: u64, pub r13: u64, pub r12: u64,
    pub rbp: u64, pub rbx: u64,
    pub r11: u64, pub r10: u64, pub r9: u64,  pub r8: u64,
    pub rax: u64, pub rcx: u64, pub rdx: u64, pub rsi: u64, pub rdi: u64,
    pub orig_rax: u64,
    pub rip: u64, pub cs: u64, pub eflags: u64,
    pub rsp: u64, pub ss: u64,
    pub fs_base: u64, pub gs_base: u64,
    pub ds: u64, pub es: u64, pub fs: u64, pub gs: u64,
}

unsafe impl UserCopyable for X86_64PtraceGPRegs {}

impl From<&ExceptionState> for X86_64PtraceGPRegs {
    fn from(value: &ExceptionState) -> Self {
        X86_64PtraceGPRegs {
            r15: value.r15,
            r14: value.r14,
            r13: value.r13,
            r12: value.r12,
            rbp: value.rbp,
            rbx: value.rbx,
            r11: value.r11,
            r10: value.r10,
            r9: value.r9,
            r8: value.r8,
            rax: value.rax,
            rcx: value.rcx,
            rdx: value.rdx,
            rsi: value.rsi,
            rdi: value.rdi,
            orig_rax: 0,
            rip: value.rip,
            cs: value.cs,
            eflags: value.rflags,
            rsp: value.rsp,
            ss: value.ss,
            fs_base: 0,
            gs_base: 0,
            ds: 0,
            es: 0,
            fs: 0,
            gs: 0,
        }
    }
}
