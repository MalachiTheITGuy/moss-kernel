use core::fmt::Display;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExceptionState {
    pub r15: u64, pub r14: u64, pub r13: u64, pub r12: u64,
    pub r11: u64, pub r10: u64, pub r9: u64,  pub r8: u64,
    pub rdi: u64, pub rsi: u64, pub rbp: u64, pub rbx: u64,
    pub rdx: u64, pub rcx: u64, pub rax: u64,
    pub error_code: u64,
    pub rip: u64, pub cs: u64, pub rflags: u64,
    pub rsp: u64, pub ss: u64,
    pub fs_base: u64, pub gs_base: u64,
}

impl Display for ExceptionState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "RAX: 0x{:016x} RBX: 0x{:016x} RCX: 0x{:016x} RDX: 0x{:016x}\n", self.rax, self.rbx, self.rcx, self.rdx)?;
        write!(f, "RIP: 0x{:016x} RSP: 0x{:016x} RFLAGS: 0x{:016x}\n", self.rip, self.rsp, self.rflags)
    }
}

core::arch::global_asm!(include_str!("trap.s"));

#[unsafe(no_mangle)]
extern "C" fn x86_64_exception_handler(state: *mut ExceptionState) -> *mut ExceptionState {
    let state_ref = unsafe { state.as_mut().unwrap() };
    log::error!("x86_64 exception occurred:\n{}", state_ref);
    
    // For now, just hang if it was a kernel exception
    if state_ref.cs & 0x3 == 0 {
        panic!("Kernel exception");
    }
    
    state
}

pub fn exceptions_init() -> libkernel::error::Result<()> {
    // TODO: Phase 3: Setup IDT
    Ok(())
}
