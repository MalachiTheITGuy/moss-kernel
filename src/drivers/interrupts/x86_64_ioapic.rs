//! x86_64 I/O APIC driver.
//!
//! The I/O APIC (I/O Advanced Programmable Interrupt Controller) routes
//! external hardware interrupts (keyboard, timer, PCI, etc.) from IRQ pins
//! to the Local APIC.
//!
//! The I/O APIC is memory-mapped at `0xFEC0_0000` (standard). Register
//! access is **indirect** — a register index is written to IOREGSEL, then
//! the value is read or written through IOWIN.
//!
//! Each redirection table entry maps one IRQ line to an LAPIC vector with
//! configurable delivery mode, polarity, and mask state.

use alloc::{
    boxed::Box,
    sync::{Arc, Weak},
};
use libkernel::{
    error::{KernelError, Result},
    memory::{address::PA, region::PhysMemoryRegion},
};
use log::info;
use tock_registers::registers::{ReadOnly, ReadWrite};
use tock_registers::{
    interfaces::{Readable, Writeable},
    register_structs,
};

use crate::{
    arch::ArchImpl,
    drivers::{Driver, DriverManager},
    interrupts::{
        InterruptConfig, InterruptContext, InterruptController, InterruptDescriptor,
        InterruptManager, TriggerMode,
    },
    sync::SpinLock,
};
use libkernel::memory::proc_vm::address_space::{KernAddressSpace, VirtualMemory};

use super::x86_64_lapic;

// ──────────────────────────────────────────────
//  I/O APIC MMIO register layout
// ──────────────────────────────────────────────

register_structs! {
    /// I/O APIC MMIO register set.
    ///
    /// Only two registers are directly addressable; all other I/O APIC
    /// registers are accessed indirectly via the index/data pair.
    #[allow(non_snake_case)]
    IoApicRegs {
        /// I/O APIC Register Select (read/write).
        /// Writing a register index here selects which register is
        /// accessible through IOWIN.
        (0x0000 => IOREGSEL: ReadWrite<u32>),
        /// Reserved: 0x004–0x00F
        (0x0004 => _reserved_0),
        /// I/O APIC Register Data Window (read/write).
        /// Read/write the register selected by IOREGSEL.
        (0x0010 => IOWIN: ReadWrite<u32>),
        (0x0014 => @END),
    }
}

// ──────────────────────────────────────────────
//  I/O APIC constants
// ──────────────────────────────────────────────

/// Standard I/O APIC physical base address.
const IOAPIC_BASE_PHYS: usize = 0xFEC0_0000;

/// Size of the I/O APIC MMIO region (4 KiB).
const IOAPIC_MMIO_SIZE: usize = 0x1000;

/// I/O APIC indirect register indices.
const IOAPIC_ID: u32 = 0x00;
const IOAPIC_VER: u32 = 0x01;
const IOAPIC_REDIR_BASE: u32 = 0x10;

/// Maximum redirection table entries (most I/O APICs have 24).
const MAX_REDIRECT_ENTRIES: usize = 24;

/// Redirection table entry flags.
const REDTBL_DELIVERY_FIXED: u32 = 0;
const REDTBL_DELIVERY_LOW_PRI: u32 = 1 << 8;
const REDTBL_STATUS_IDLE: u32 = 0;
const REDTBL_TRIGGER_EDGE: u32 = 1 << 13;
const REDTBL_TRIGGER_LEVEL: u32 = 0;
const REDTBL_POLARITY_HIGH: u32 = 0;
const REDTBL_MASK_DISABLED: u32 = 1 << 16;
const REDTBL_MASK_ENABLED: u32 = 0;

// ──────────────────────────────────────────────
//  IoApicInterruptContext
// ──────────────────────────────────────────────

/// Tracks an active I/O APIC interrupt. Dropping this sends EOI to the
/// LAPIC (since the I/O APIC routes through the LAPIC).
struct IoApicInterruptContext {
    desc: InterruptDescriptor,
    lapic: Arc<SpinLock<x86_64_lapic::X86_64Lapic>>,
}

impl InterruptContext for IoApicInterruptContext {
    fn descriptor(&self) -> InterruptDescriptor {
        self.desc
    }
}

impl Drop for IoApicInterruptContext {
    fn drop(&mut self) {
        let lapic = self.lapic.lock_save_irq();
        lapic.eoi();
    }
}

// ──────────────────────────────────────────────
//  X86_64IoApic — driver core
// ──────────────────────────────────────────────

/// x86_64 I/O APIC driver.
///
/// Provides indirect register access to the I/O APIC and manages the
/// redirection table for external IRQ routing.
pub(crate) struct X86_64IoApic {
    regs: &'static mut IoApicRegs,
    this: Weak<SpinLock<Self>>,
    lapic: Option<Arc<SpinLock<x86_64_lapic::X86_64Lapic>>>,
}

unsafe impl Sync for X86_64IoApic {}
unsafe impl Send for X86_64IoApic {}

impl X86_64IoApic {
    /// Construct a new I/O APIC driver.
    fn new(
        regs: &'static mut IoApicRegs,
        this: Weak<SpinLock<Self>>,
        lapic: Option<Arc<SpinLock<x86_64_lapic::X86_64Lapic>>>,
    ) -> Self {
        let mut ioapic = Self { regs, this, lapic };
        ioapic.init();
        ioapic
    }

    /// One-time I/O APIC initialisation.
    ///
    /// 1. Read the ID and version to confirm the hardware is present.
    /// 2. Mask all redirection table entries.
    fn init(&mut self) {
        let id = self.read_register(IOAPIC_ID);
        let ver = self.read_register(IOAPIC_VER);

        let max_redir = ((ver >> 16) & 0xFF) as usize; // bits [23:16]
        let version = (ver & 0xFF) as u8;

        info!(
            "x86_64 I/O APIC: ID={:#x}, version={:#x}, max redirect entries={}",
            (id >> 24) & 0xF,
            version,
            max_redir + 1
        );

        // Mask all redirection entries to prevent spurious interrupts.
        let entries = MAX_REDIRECT_ENTRIES.min(max_redir + 1);
        for i in 0..entries {
            let low_idx = IOAPIC_REDIR_BASE + (i as u32) * 2;
            let high_idx = low_idx + 1;

            let low = self.read_register(low_idx);
            self.write_register(low_idx, low | REDTBL_MASK_DISABLED);

            // Clear the high register (destination APIC ID = 0).
            self.write_register(high_idx, 0);
        }
    }

    /// Read an I/O APIC register via the indirect access mechanism.
    fn read_register(&self, index: u32) -> u32 {
        self.regs.IOREGSEL.set(index);
        self.regs.IOWIN.get()
    }

    /// Write an I/O APIC register via the indirect access mechanism.
    fn write_register(&mut self, index: u32, value: u32) {
        self.regs.IOREGSEL.set(index);
        self.regs.IOWIN.set(value);
    }

    /// Configure a redirection table entry.
    ///
    /// * `irq` — IRQ line (0-based index into the redirection table).
    /// * `vector` — LAPIC vector to deliver on this IRQ.
    /// * `trigger` — Edge or level triggered.
    /// * `mask` — `true` to keep the entry masked.
    /// * `dest_apic_id` — Target LAPIC APIC ID (for physical destination).
    fn configure_redirection(
        &mut self,
        irq: usize,
        vector: u32,
        trigger: TriggerMode,
        mask: bool,
        dest_apic_id: u32,
    ) {
        let low_idx = IOAPIC_REDIR_BASE + (irq as u32) * 2;
        let high_idx = low_idx + 1;

        // Low 32 bits: vector + delivery mode + trigger mode + polarity
        // + mask.
        let mut low = vector & 0xFF;
        low |= REDTBL_DELIVERY_FIXED;
        low |= REDTBL_STATUS_IDLE;
        low |= REDTBL_POLARITY_HIGH;

        match trigger {
            TriggerMode::EdgeRising | TriggerMode::EdgeFalling => {
                low |= REDTBL_TRIGGER_EDGE;
            }
            TriggerMode::LevelHigh | TriggerMode::LevelLow => {
                low |= REDTBL_TRIGGER_LEVEL;
            }
        }

        if mask {
            low |= REDTBL_MASK_DISABLED;
        } else {
            low |= REDTBL_MASK_ENABLED;
        }

        self.write_register(low_idx, low);

        // High 32 bits: destination APIC ID in bits [31:24].
        self.write_register(high_idx, (dest_apic_id & 0xF) << 24);
    }

    /// Mask (disable) a redirection table entry.
    fn mask_irq(&mut self, irq: usize) {
        let low_idx = IOAPIC_REDIR_BASE + (irq as u32) * 2;
        let low = self.read_register(low_idx);
        self.write_register(low_idx, low | REDTBL_MASK_DISABLED);
    }

    /// Unmask (enable) a redirection table entry.
    fn unmask_irq(&mut self, irq: usize) {
        let low_idx = IOAPIC_REDIR_BASE + (irq as u32) * 2;
        let low = self.read_register(low_idx);
        self.write_register(low_idx, low & !REDTBL_MASK_DISABLED);
    }
}

// ──────────────────────────────────────────────
//  InterruptController trait
// ──────────────────────────────────────────────

impl InterruptController for X86_64IoApic {
    fn enable_interrupt(&mut self, config: InterruptConfig) {
        match config.descriptor {
            InterruptDescriptor::Spi(spi) => {
                // SPI N on the I/O APIC maps to IRQ line N and LAPIC
                // vector N + 32 (same offset as the IDT vector base).
                let irq = spi;
                let vector = (spi + 32) as u32;

                if irq < MAX_REDIRECT_ENTRIES {
                    // Determine the destination LAPIC. If the LAPIC is
                    // available, use its APIC ID; otherwise default to 0.
                    let dest_id = self
                        .lapic
                        .as_ref()
                        .map(|l| {
                            let lapic = l.lock_save_irq();
                            lapic.apic_id()
                        })
                        .unwrap_or(0);

                    self.configure_redirection(
                        irq,
                        vector,
                        config.trigger,
                        false, // unmasked = enabled
                        dest_id,
                    );
                }
            }
            InterruptDescriptor::Ppi(_) | InterruptDescriptor::Ipi(_) => {
                // PPI and IPI are not routed through the I/O APIC.
            }
        }
    }

    fn disable_interrupt(&mut self, descriptor: InterruptDescriptor) {
        if let InterruptDescriptor::Spi(spi) = descriptor {
            if spi < MAX_REDIRECT_ENTRIES {
                self.mask_irq(spi);
            }
        }
    }

    fn read_active_interrupt(&mut self) -> Option<Box<dyn InterruptContext>> {
        // The I/O APIC does not have an IAR like the GIC. Active
        // interrupt state is managed by the LAPIC. This method is a no-op
        // for the I/O APIC — the LAPIC's `read_active_interrupt` is what
        // actually services IRQs.
        None
    }

    fn raise_ipi(&mut self, _target_cpu_id: usize) {
        // IPIs are sent through the LAPIC, not the I/O APIC.
    }

    fn enable_core(&mut self, _cpu_id: usize) {
        // No per-core state in the I/O APIC.
    }

    fn parse_fdt_interrupt_regs(
        &self,
        _iter: &mut dyn Iterator<Item = u32>,
    ) -> Result<InterruptConfig> {
        // x86_64 does not use FDT.
        Err(KernelError::NotSupported)
    }
}

// ──────────────────────────────────────────────
//  I/O APIC probe / init
// ──────────────────────────────────────────────

/// Probe and initialise the I/O APIC.
///
/// This maps the I/O APIC MMIO region, creates the driver, and registers
/// it with the [`DriverManager`]. The I/O APIC is a **secondary** interrupt
/// controller — it is not set as the root (the LAPIC is the root).
pub fn x86_64_ioapic_init(
    lapic: Option<Arc<SpinLock<x86_64_lapic::X86_64Lapic>>>,
    dm: &mut DriverManager,
) -> Result<()> {
    // Map the I/O APIC MMIO region.
    let ioapic_va = {
        let addr_spc = <ArchImpl as VirtualMemory>::kern_address_space();
        let mut kern_addr_spc = addr_spc.lock_save_irq();

        kern_addr_spc.map_mmio(PhysMemoryRegion::new(
            PA::from_value(IOAPIC_BASE_PHYS),
            IOAPIC_MMIO_SIZE,
        ))?
    };

    info!(
        "x86_64 I/O APIC: MMIO mapped at VA {:#x}",
        ioapic_va.value()
    );

    let dev: Arc<SpinLock<X86_64IoApic>> = Arc::new_cyclic(|this| {
        SpinLock::new(X86_64IoApic::new(
            // SAFETY: `ioapic_va` points to a valid MMIO mapping of the
            // I/O APIC hardware register space.
            unsafe { &mut *(ioapic_va.value() as *mut IoApicRegs) },
            this.clone(),
            lapic,
        ))
    });

    // Register with the driver manager.
    let manager = InterruptManager::new("x86_64-ioapic", dev);
    dm.insert_driver(manager);

    info!("x86_64 I/O APIC: driver registered");

    Ok(())
}
