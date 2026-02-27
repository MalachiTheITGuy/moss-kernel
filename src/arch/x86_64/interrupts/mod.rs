use crate::interrupts::{InterruptController, InterruptDescriptor, InterruptConfig, InterruptContext};
use libkernel::error::Result;
use crate::sync::SpinLock;
use alloc::{sync::Arc, boxed::Box};
use crate::interrupts::InterruptManager;
use x86_64::VirtAddr;
use crate::per_cpu_private;

/// Virtual address of the Local APIC MMIO registers.
/// Mapped via the kernel's linear map: PAGE_OFFSET (0xffff_8000_0000_0000) + physical 0xFEE0_0000.
/// This is in the upper half and is therefore shared across all process page tables.
const LAPIC_VIRT_BASE: u64 = 0xffff_8000_fee0_0000;

pub mod apic;
use self::apic::{LocalApic, ApicTimer};

per_cpu_private! {
    static PENDING_VECTOR: Option<u8> = || None;
}

pub struct X86InterruptController {
    lapic: LocalApic,
}

impl X86InterruptController {
    pub fn new() -> Self {
        // Disable legacy PIC
        unsafe {
            core::arch::asm!(
                "outb %al, %dx", 
                in("dx") 0x21u16, in("al") 0xFFu8,
                options(att_syntax, nomem, nostack)
            );
            core::arch::asm!(
                "outb %al, %dx", 
                in("dx") 0xA1u16, in("al") 0xFFu8,
                options(att_syntax, nomem, nostack)
            );
        }

        let lapic = LocalApic::new(VirtAddr::new(LAPIC_VIRT_BASE));
        lapic.init();
        Self { lapic }
    }
}

pub struct X86InterruptContext {
    vector: u8,
    lapic: LocalApic,
}

impl InterruptContext for X86InterruptContext {
    fn descriptor(&self) -> InterruptDescriptor {
        InterruptDescriptor::Spi(self.vector as usize)
    }
}

impl Drop for X86InterruptContext {
    fn drop(&mut self) {
        self.lapic.eoi();
    }
}

impl InterruptController for X86InterruptController {
    fn enable_interrupt(&mut self, _i: InterruptConfig) {
        // TODO: Handle I/O APIC
    }

    fn disable_interrupt(&mut self, _i: InterruptDescriptor) {
        // TODO
    }

    fn read_active_interrupt(&mut self) -> Option<Box<dyn InterruptContext>> {
        let vector = PENDING_VECTOR.borrow_mut().take()?;
        Some(Box::new(X86InterruptContext {
            vector,
            lapic: LocalApic::new(VirtAddr::new(LAPIC_VIRT_BASE)),
        }))
    }

    fn raise_ipi(&mut self, _target_cpu_id: usize) {
        // TODO
    }

    fn enable_core(&mut self, _cpu_id: usize) {
        self.lapic.init();
    }

    fn parse_fdt_interrupt_regs(
        &self,
        _iter: &mut dyn Iterator<Item = u32>,
    ) -> Result<InterruptConfig> {
        Err(libkernel::error::KernelError::NotSupported)
    }
}

pub fn init() {
    let controller = Arc::new(SpinLock::new(X86InterruptController::new()));
    let manager = InterruptManager::new("x86-intc", controller);
    crate::interrupts::set_interrupt_root(manager);
    
    // Initialize timer
    let lapic = LocalApic::new(VirtAddr::new(LAPIC_VIRT_BASE));
    lapic.setup_timer(0x20);
    crate::drivers::timer::set_sys_timer(Arc::new(ApicTimer::new(lapic)));
}

pub fn set_pending_vector(vector: u8) {
    *PENDING_VECTOR.borrow_mut() = Some(vector);
}
