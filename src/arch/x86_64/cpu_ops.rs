use super::X86_64;
use libkernel::CpuOps;

impl CpuOps for X86_64 {
    fn id() -> usize {
        // For now, return 0. In a real implementation with APIC, we'd read the APIC ID.
        0
    }

    fn halt() -> ! {
        loop {
            unsafe {
                core::arch::asm!("hlt");
            }
        }
    }

    fn disable_interrupts() -> usize {
        let rflags: usize;
        unsafe {
            core::arch::asm!(
                "pushfq",
                "pop {}",
                "cli",
                out(reg) rflags,
            );
        }
        rflags
    }

    fn restore_interrupt_state(state: usize) {
        unsafe {
            core::arch::asm!(
                "push {}",
                "popfq",
                in(reg) state,
            );
        }
    }

    fn enable_interrupts() {
        unsafe {
            core::arch::asm!("sti");
        }
    }
}
