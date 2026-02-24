pub mod syscall;
pub use syscall::x86_64_syscall_handler;

use core::fmt::Display;
use core::arch::global_asm;
use x86_64::structures::idt::InterruptDescriptorTable;
use x86_64::{PrivilegeLevel, VirtAddr};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExceptionState {
    pub rax: u64, pub rcx: u64, pub rdx: u64, pub rbx: u64, pub rbp: u64,
    pub rsi: u64, pub rdi: u64, pub r8: u64, pub r9: u64, pub r10: u64,
    pub r11: u64, pub r12: u64, pub r13: u64, pub r14: u64, pub r15: u64,
    pub error_code: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

impl Display for ExceptionState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "RAX: 0x{:016x} RBX: 0x{:016x} RCX: 0x{:016x} RDX: 0x{:016x}\n",
            self.rax, self.rbx, self.rcx, self.rdx
        )?;
        write!(
            f,
            "RIP: 0x{:016x} RSP: 0x{:016x} RFLAGS: 0x{:016x}\n",
            self.rip, self.rsp, self.rflags
        )
    }
}

global_asm!(include_str!("trap.s"));

static mut IDT: InterruptDescriptorTable = InterruptDescriptorTable::new();

unsafe extern "C" {
    fn exc_divide_by_zero();
    fn exc_debug();
    fn exc_nmi();
    fn exc_breakpoint();
    fn exc_overflow();
    fn exc_bound_range_exceeded();
    fn exc_invalid_opcode();
    fn exc_device_not_available();
    fn exc_double_fault();
    fn exc_invalid_tss();
    fn exc_segment_not_present();
    fn exc_stack_segment_fault();
    fn exc_general_protection_fault();
    fn exc_page_fault();
    fn exc_x87_floating_point();
    fn exc_alignment_check();
    fn exc_machine_check();
    fn exc_simd_floating_point();
    fn exc_virtualization();
    fn exc_control_protection();
    fn exc_hypervisor_injection();
    fn exc_vmm_communication();
    fn exc_security();
    fn exc_syscall();
}

#[no_mangle]
extern "C" fn x86_64_exception_handler(state: *mut ExceptionState) -> *mut ExceptionState {
    let state_ref = unsafe { state.as_mut().unwrap() };
    log::error!("x86_64 exception occurred:\n{}", state_ref);

    if state_ref.cs & 0x3 == 0 {
        panic!("Kernel exception");
    }

    state
}

pub fn exceptions_init() -> libkernel::error::Result<()> {
    unsafe {
        let idt = &mut *core::ptr::addr_of_mut!(IDT);
        idt.divide_error
            .set_handler_addr(VirtAddr::new(exc_divide_by_zero as *const () as u64))
            .set_privilege_level(PrivilegeLevel::Ring0);
        idt.debug
            .set_handler_addr(VirtAddr::new(exc_debug as *const () as u64))
            .set_privilege_level(PrivilegeLevel::Ring0);
        idt.non_maskable_interrupt
            .set_handler_addr(VirtAddr::new(exc_nmi as *const () as u64))
            .set_privilege_level(PrivilegeLevel::Ring0);
        idt.breakpoint
            .set_handler_addr(VirtAddr::new(exc_breakpoint as *const () as u64))
            .set_privilege_level(PrivilegeLevel::Ring0);
        idt.overflow
            .set_handler_addr(VirtAddr::new(exc_overflow as *const () as u64))
            .set_privilege_level(PrivilegeLevel::Ring0);
        idt.bound_range_exceeded
            .set_handler_addr(VirtAddr::new(exc_bound_range_exceeded as *const () as u64))
            .set_privilege_level(PrivilegeLevel::Ring0);
        idt.invalid_opcode
            .set_handler_addr(VirtAddr::new(exc_invalid_opcode as *const () as u64))
            .set_privilege_level(PrivilegeLevel::Ring0);
        idt.device_not_available
            .set_handler_addr(VirtAddr::new(exc_device_not_available as *const () as u64))
            .set_privilege_level(PrivilegeLevel::Ring0);
        idt.double_fault
            .set_handler_addr(VirtAddr::new(exc_double_fault as *const () as u64))
            .set_privilege_level(PrivilegeLevel::Ring0)
            .set_stack_index(0);
        idt.invalid_tss
            .set_handler_addr(VirtAddr::new(exc_invalid_tss as *const () as u64))
            .set_privilege_level(PrivilegeLevel::Ring0);
        idt.segment_not_present
            .set_handler_addr(VirtAddr::new(exc_segment_not_present as *const () as u64))
            .set_privilege_level(PrivilegeLevel::Ring0);
        idt.stack_segment_fault
            .set_handler_addr(VirtAddr::new(exc_stack_segment_fault as *const () as u64))
            .set_privilege_level(PrivilegeLevel::Ring0);
        idt.general_protection_fault
            .set_handler_addr(VirtAddr::new(exc_general_protection_fault as *const () as u64))
            .set_privilege_level(PrivilegeLevel::Ring0);
        idt.page_fault
            .set_handler_addr(VirtAddr::new(exc_page_fault as *const () as u64))
            .set_privilege_level(PrivilegeLevel::Ring0);
        idt.x87_floating_point
            .set_handler_addr(VirtAddr::new(exc_x87_floating_point as *const () as u64))
            .set_privilege_level(PrivilegeLevel::Ring0);
        idt.alignment_check
            .set_handler_addr(VirtAddr::new(exc_alignment_check as *const () as u64))
            .set_privilege_level(PrivilegeLevel::Ring0);
        idt.machine_check
            .set_handler_addr(VirtAddr::new(exc_machine_check as *const () as u64))
            .set_privilege_level(PrivilegeLevel::Ring0);
        idt.simd_floating_point
            .set_handler_addr(VirtAddr::new(exc_simd_floating_point as *const () as u64))
            .set_privilege_level(PrivilegeLevel::Ring0);
        idt.virtualization
            .set_handler_addr(VirtAddr::new(exc_virtualization as *const () as u64))
            .set_privilege_level(PrivilegeLevel::Ring0);
        
        idt.security_exception
            .set_handler_addr(VirtAddr::new(exc_security as *const () as u64))
            .set_privilege_level(PrivilegeLevel::Ring0);

        // TODO: Set up syscall MSR for x86_64 using inline assembly
        // This requires proper handling of the syscall instruction
        
        idt.load();
    }

    Ok(())
}
