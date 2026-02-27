pub mod syscall;

use core::fmt::Display;
use core::arch::global_asm;
use x86_64::structures::idt::InterruptDescriptorTable;
use x86_64::{PrivilegeLevel, VirtAddr};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExceptionState {
    pub fs_base: u64,                                                              // FS_BASE MSR (saved by Rust handler, slot pushed by PUSH_REGS)
    pub rax: u64, pub rcx: u64, pub rdx: u64, pub rbx: u64, pub rbp: u64,
    pub rsi: u64, pub rdi: u64, pub r8: u64, pub r9: u64, pub r10: u64,
    pub r11: u64, pub r12: u64, pub r13: u64, pub r14: u64, pub r15: u64,
    pub vector: u64,
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
            "RIP: 0x{:016x} RSP: 0x{:016x} RFLAGS: 0x{:016x} VEC: {}\n",
            self.rip, self.rsp, self.rflags, self.vector
        )?;
        write!(
            f,
            "ERR: 0x{:016x}\n",
            self.error_code
        )
    }
}

global_asm!(include_str!("trap.s"));

/// Read the IA32_FS_BASE MSR (user-space FS segment base).
#[inline]
fn read_fs_base() -> u64 {
    let lo: u32;
    let hi: u32;
    unsafe {
        core::arch::asm!(
            "rdmsr",
            in("ecx") 0xC0000100u32,
            out("eax") lo,
            out("edx") hi,
            options(nostack, nomem),
        );
    }
    (hi as u64) << 32 | lo as u64
}

/// Write the IA32_FS_BASE MSR (user-space FS segment base).
#[inline]
pub fn write_fs_base(val: u64) {
    unsafe {
        core::arch::asm!(
            "wrmsr",
            in("ecx") 0xC0000100u32,
            in("eax") (val & 0xFFFF_FFFF) as u32,
            in("edx") (val >> 32) as u32,
            options(nostack, nomem),
        );
    }
}

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
    fn exc_timer();
    fn exc_com1();
}

#[unsafe(no_mangle)]
extern "C" fn x86_64_exception_handler(state: *mut ExceptionState) -> *mut ExceptionState {
    let state_ref = unsafe { state.as_mut().unwrap() };

    // Save the user FS_BASE from the MSR when coming from user space.
    // The assembly stub pushed a placeholder $0; fill it with the real value.
    let from_user = state_ref.cs & 0x3 != 0;
    if from_user {
        state_ref.fs_base = read_fs_base();
    }
    
    let vector = state_ref.vector;
    
    if vector >= 32 && vector <= 255 {
        if let Some(root) = crate::interrupts::get_interrupt_root() {
            crate::arch::x86_64::interrupts::set_pending_vector(vector as u8);
            root.handle_interrupt();
        }
        
        // Signal EOI if it's an APIC interrupt
        // For now, we'll just assume it's handled.
    } else {
        // Fetch the faulting address if this is a page fault so we have
        // something concrete to debug with.  The `read()` helper returns a
        // `Result` because some crate configurations make `VirtAddr` validation
        // fallible; ignore that possibility here since there is nothing we can
        // do about it during a panic.
        let cr2 = x86_64::registers::control::Cr2::read()
            .unwrap_or(x86_64::VirtAddr::new(0));

        // Log extra registers to help diagnose memcpy faults.
        let rsi = state_ref.rsi;
        let rdi = state_ref.rdi;
        log::error!("regs: RSI=0x{:016x} RDI=0x{:016x}", rsi, rdi);

        // ── Kernel upper-half PML4 propagation ──────────────────────────────
        // When a new kernel virtual mapping is installed (e.g. for a ramdisk)
        // AFTER a process address space was cloned from the kernel PML4, the
        // new PML4 entry is only present in the kernel PML4, not in the
        // process PML4.  On the first access from that process, a page fault
        // fires with a "not-present" error code.  We detect that case here and
        // copy the kernel PML4 entry into the process PML4, then return (which
        // causes the CPU to retry the faulting instruction).
        if vector == 14 {
            use crate::arch::x86_64::memory::PAGE_OFFSET;
            use crate::arch::x86_64::memory::mmu::KERN_ADDR_SPC;

            let cr2_val = cr2.as_u64() as usize;
            let pml4_idx = (cr2_val >> 39) & 0x1FF;

            // Kernel upper-half: canonical address with bit 47 set (PML4 idx
            // 256..511) and error code bit 0 clear (not-present fault).
            if pml4_idx >= 256 && (state_ref.error_code & 1) == 0 {
                if let Some(kern_spc) = KERN_ADDR_SPC.get() {
                    let kern_pml4_pa = kern_spc.lock_save_irq().table_pa().value();

                    // Read the kernel's PML4 entry for this index through the
                    // PAGE_OFFSET linear map (covers all physical memory).
                    let kern_entry: u64 = unsafe {
                        let ptr = (PAGE_OFFSET + kern_pml4_pa + pml4_idx * 8) as *const u64;
                        ptr.read_volatile()
                    };

                    if kern_entry & 1 != 0 {
                        // Kernel PML4 entry is present.  Get the active CR3
                        // (process PML4 PA) and check it differs from the
                        // kernel PML4 (i.e. we are in a process page-table
                        // context, not already the kernel's).
                        let current_cr3: usize;
                        unsafe { core::arch::asm!("mov {}, cr3", out(reg) current_cr3) };

                        if current_cr3 != kern_pml4_pa {
                            // Propagate the entry and retry.
                            unsafe {
                                let ptr = (PAGE_OFFSET + current_cr3 + pml4_idx * 8) as *mut u64;
                                ptr.write_volatile(kern_entry);
                                // Flush the TLB for the faulting address so
                                // the CPU picks up the new mapping immediately.
                                core::arch::asm!("invlpg [{addr}]", addr = in(reg) cr2_val);
                            }
                            return state;
                        }
                    }
                }
            }
        }

        log::error!(
            "x86_64 exception occurred:\n{}CR2: 0x{:016x}\n",
            state_ref,
            cr2.as_u64()
        );

        // Try to dump a few words from the faulting stack pointer.  In the
        // memcpy crash we're debugging the destination pointer ended up null,
        // so the return address stored at `rsp` should point at the caller of
        // memcpy.  Print several entries in case the frame pointer or call
        // thunk pushed additional data.
        let rsp_val = state_ref.rsp as *const usize;
        unsafe {
            log::error!("stack trace words from faulting rsp = {:#x}", rsp_val as usize);
            for i in 0..6 {
                let w = rsp_val.add(i).read();
                log::error!("  [rsp + {}] = {:#018x}", i * 8, w);
            }
        }

        if state_ref.cs & 0x3 == 0 {
            panic!("Kernel exception");
        }
    }

    // Restore the user FS_BASE MSR when returning to user space.
    if from_user {
        write_fs_base(state_ref.fs_base);
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

        idt[0x20]
            .set_handler_addr(VirtAddr::new(exc_timer as *const () as u64))
            .set_privilege_level(PrivilegeLevel::Ring0);
        
        idt[0x24]
            .set_handler_addr(VirtAddr::new(exc_com1 as *const () as u64))
            .set_privilege_level(PrivilegeLevel::Ring0);

        // Install int 0x80 as the syscall entry point (Ring3 callable)
        idt[0x80]
            .set_handler_addr(VirtAddr::new(exc_syscall as *const () as u64))
            .set_privilege_level(PrivilegeLevel::Ring3);

        idt.load();
    }

    Ok(())
}
