use crate::interrupts::{InterruptController, InterruptDescriptor, InterruptConfig, InterruptContext};
use libkernel::error::Result;
use crate::sync::SpinLock;
use alloc::{sync::Arc, boxed::Box};
use crate::interrupts::InterruptManager;
use x86_64::VirtAddr;
use crate::per_cpu_private;

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
        let lapic = LocalApic::new(VirtAddr::new(0xFEE00000));
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
            lapic: LocalApic::new(VirtAddr::new(0xFEE00000)),
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
    let lapic = LocalApic::new(VirtAddr::new(0xFEE00000));
    lapic.setup_timer(0x20);
    crate::drivers::timer::set_sys_timer(Arc::new(ApicTimer::new(lapic)));
}

pub fn set_pending_vector(vector: u8) {
    *PENDING_VECTOR.borrow_mut() = Some(vector);
}
