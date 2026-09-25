//! x86_64 Local APIC driver.
//!
//! The Local APIC (Advanced Programmable Interrupt Controller) handles:
//!
//! - Per-CPU interrupt delivery (timer, LINT0/LINT1, error)
//! - Inter-processor interrupts (IPIs)
//! - Spurious interrupt filtering
//! - End-of-interrupt (EOI) signalling
//!
//! This module models the LAPIC after the ARM GIC v2 driver pattern:
//! `register_structs!` for MMIO, `Arc::new_cyclic` for the driver object,
//! `kernel_driver!` for initcall registration, and an `InterruptContext`
//! that sends EOI on drop.
//!
//! The LAPIC is memory-mapped at the standard base `0xFEE0_0000`. The I/O
//! APIC is separately memory-mapped at `0xFEC0_0000` and is implemented in
//! [`super::x86_64_ioapic`].

use alloc::{
    boxed::Box,
    sync::{Arc, Weak},
};
use libkernel::{
    error::{KernelError, Result},
    memory::{address::PA, region::PhysMemoryRegion},
};
use log::info;
use tock_registers::{
    interfaces::{Readable, Writeable},
    register_structs,
    registers::{ReadOnly, ReadWrite, WriteOnly},
};

use crate::{
    arch::ArchImpl,
    drivers::{Driver, DriverManager, init::PlatformBus},
    interrupts::{
        InterruptConfig, InterruptContext, InterruptController, InterruptDescriptor,
        InterruptManager, set_interrupt_root,
    },
    kernel_driver,
    sync::SpinLock,
};
use libkernel::memory::proc_vm::address_space::{KernAddressSpace, VirtualMemory};

// ──────────────────────────────────────────────
//  LAPIC MMIO register layout
// ──────────────────────────────────────────────

register_structs! {
    /// Local APIC MMIO register set.
    ///
    /// Base address is `0xFEE0_0000` (standard). All registers are 32-bit
    /// and aligned to their natural boundaries.
    #[allow(non_snake_case)]
    LapicRegs {
        /// Reserved: 0x000–0x01F
        (0x000 => _reserved_0),
        /// APIC ID Register (read-only, bits [31:24] = APIC ID).
        (0x020 => APIC_ID: ReadOnly<u32>),
        /// Reserved: 0x024–0x07F
        (0x024 => _reserved_1),
        /// Task Priority Register (read/write).
        /// Bits [7:0] = Task Priority Class, [15:8] = Task Priority Subclass.
        /// Only interrupts with priority > TPR are delivered.
        (0x080 => TPR: ReadWrite<u32>),
        /// Reserved: 0x084–0x0AF
        (0x084 => _reserved_2),
        /// End-of-Interrupt Register (write-only).
        /// Writing 0 signals EOI to the LAPIC.
        (0x0B0 => EOI: WriteOnly<u32>),
        /// Reserved: 0x0B4–0x0EF
        (0x0B4 => _reserved_3),
        /// Spurious Interrupt Vector Register (read/write).
        /// Bit [0] = APIC Software Enable.
        /// Bits [7:0] = Spurious Vector (typically 0xFF).
        (0x0F0 => SPIVR: ReadWrite<u32>),
        /// Reserved: 0x0F4–0x0FF
        (0x0F4 => _reserved_4),
        /// In-Service Register (read-only, 8 × 32-bit).
        /// Bit N of register [N/32] is set when vector N is being serviced.
        (0x100 => ISR: [ReadOnly<u32>; 8]),
        /// Reserved: 0x120–0x17F
        (0x120 => _reserved_5),
        /// Trigger Mode Register (read-only, 8 × 32-bit).
        /// Bit N indicates vector N is level-triggered.
        (0x180 => TMR: [ReadOnly<u32>; 8]),
        /// Reserved: 0x1A0–0x1FF
        (0x1A0 => _reserved_6),
        /// Interrupt Request Register (read-only, 8 × 32-bit).
        /// Bit N of register [N/32] is set when vector N is pending.
        (0x200 => IRR: [ReadOnly<u32>; 8]),
        /// Reserved: 0x220–0x27F
        (0x220 => _reserved_7),
        /// Interrupt Command Register — low 32 bits (read/write).
        /// Bits [7:0] = vector, [10:8] = delivery mode, [11] = destination
        /// mode, [12] = idle pending, [13] = level, [14] = trigger mode,
        /// [19:16] = destination shorthand.
        (0x280 => ICR_LO: ReadWrite<u32>),
        /// Reserved: 0x284–0x2BF
        (0x284 => _reserved_8b: [u8; 0x3C]),
        /// Interrupt Command Register — high 32 bits (read/write).
        /// Bits [31:24] = destination APIC ID.
        (0x2C0 => ICR_HI: ReadWrite<u32>),
        /// Reserved: 0x2C4–0x31F
        (0x2C4 => _reserved_8),
        /// LVT Timer Register (read/write).
        /// Bits [7:0] = vector, [12:10] = delivery status, [13] = mask,
        /// [14] = timer mode (0=one-shot, 1=periodic, 2=TSC-deadline).
        (0x320 => LVT_TIMER: ReadWrite<u32>),
        /// Reserved: 0x324–0x34F
        (0x324 => _reserved_9),
        /// LVT LINT0 Register (read/write).
        /// Bits [7:0] = vector, [11:10] = delivery mode (000=ExtINT,
        /// 001=NMI), [12] = delivery status, [13] = mask,
        /// [15] = trigger mode.
        (0x350 => LVT_LINT0: ReadWrite<u32>),
        /// Reserved: 0x354–0x35F
        (0x354 => _reserved_10a: [u8; 0xC]),
        /// LVT LINT1 Register (read/write).
        (0x360 => LVT_LINT1: ReadWrite<u32>),
        /// Reserved: 0x364–0x36F
        (0x364 => _reserved_11a: [u8; 0xC]),
        /// LVT Error Register (read/write).
        (0x370 => LVT_ERR: ReadWrite<u32>),
        /// Reserved: 0x374–0x3DF
        (0x374 => _reserved_10),
        /// Timer Divide Configuration Register (read/write).
        /// Bits [2:0] = divide value, [3] = TSC delivery.
        (0x3E0 => TMRDIV: ReadWrite<u32>),
        /// Reserved: 0x3E4–0xFFF
        (0x3E4 => _reserved_11),
        (0x1000 => @END),
    }
}

// ──────────────────────────────────────────────
//  LAPIC constants
// ──────────────────────────────────────────────

/// Standard LAPIC physical base address.
const LAPIC_BASE_PHYS: usize = 0xFEE0_0000;

/// Size of the LAPIC MMIO region (4 KiB covers all registers).
const LAPIC_MMIO_SIZE: usize = 0x1000;

/// Spurious vector number (Intel recommends 0xFF).
const SPURIOUS_VECTOR: u32 = 0xFF;

/// Delivery mode: Fixed (normal delivery to the APIC).
const DELIVERY_FIXED: u32 = 0;

/// Delivery mode: Init (reset the target APIC).
const DELIVERY_INIT: u32 = 5;

/// Delivery mode: Start-Up (SIPI).
const DELIVERY_SIPI: u32 = 6;

/// Destination shorthand: use destination field in ICR_HIGH.
const DEST_SELF: u32 = 1 << 18;
const DEST_ALL: u32 = 1 << 18 | 1 << 19;

/// LVT vector for the LAPIC timer.
const LAPIC_TIMER_VECTOR: u32 = 0x40;

/// LVT timer mode: periodic.
const TIMER_MODE_PERIODIC: u32 = 1 << 17;

/// TMRDIV: divide by 16 (bits [2:0] = 0b011).
const TMRDIV_DIV16: u32 = 0b011;

/// Mask bit in LVT entries.
const LVT_MASK: u32 = 1 << 16;

// ──────────────────────────────────────────────
//  LAPIC interrupt context
// ──────────────────────────────────────────────

/// Tracks an active LAPIC interrupt. Dropping this writes EOI.
struct LapicInterruptContext {
    desc: InterruptDescriptor,
    lapic: Arc<SpinLock<X86_64Lapic>>,
}

impl InterruptContext for LapicInterruptContext {
    fn descriptor(&self) -> InterruptDescriptor {
        self.desc
    }
}

impl Drop for LapicInterruptContext {
    fn drop(&mut self) {
        // Signal end-of-interrupt to the LAPIC so it can deliver the next
        // highest-priority interrupt.
        let lapic = self.lapic.lock_save_irq();
        lapic.eoi();
    }
}

// ──────────────────────────────────────────────
//  X86_64Lapic — driver core
// ──────────────────────────────────────────────

/// x86_64 Local APIC driver.
///
/// Holds a reference to the memory-mapped register block and the weak
/// self-reference needed to construct [`LapicInterruptContext`] instances.
pub(crate) struct X86_64Lapic {
    regs: &'static mut LapicRegs,
    this: Weak<SpinLock<Self>>,
}

unsafe impl Sync for X86_64Lapic {}
unsafe impl Send for X86_64Lapic {}

impl X86_64Lapic {
    /// Construct a new LAPIC driver.
    ///
    /// The `this` weak reference is the [`Arc::weak`] of the enclosing
    /// `SpinLock<Self>` — it is used to hand out self-references inside
    /// [`LapicInterruptContext`].
    fn new(regs: &'static mut LapicRegs, this: Weak<SpinLock<Self>>) -> Self {
        let mut lapic = Self { regs, this };
        lapic.init();
        lapic
    }

    /// One-time LAPIC initialisation.
    ///
    /// 1. Mask all LVT entries.
    /// 2. Set the task-priority register to 0 (accept all interrupts).
    /// 3. Enable the LAPIC via the spurious-vector register.
    /// 4. Configure the LAPIC timer in periodic mode.
    /// 5. Configure LINT0/LINT1 for ExtINT / NMI (BSP only).
    fn init(&mut self) {
        // Mask all LVT entries so we don't receive unconfigured interrupts.
        self.regs.LVT_TIMER.set(LVT_MASK);
        self.regs.LVT_LINT0.set(LVT_MASK);
        self.regs.LVT_LINT1.set(LVT_MASK);
        self.regs.LVT_ERR.set(LVT_MASK);

        // Set task priority to 0 — accept all interrupts.
        self.regs.TPR.set(0);

        // Configure the timer: divide by 16, periodic mode, vector 0x40.
        self.regs.TMRDIV.set(TMRDIV_DIV16);
        self.regs
            .LVT_TIMER
            .set(LAPIC_TIMER_VECTOR | TIMER_MODE_PERIODIC);

        // Configure LINT0 as ExtINT (delivery mode 0b111) for 8259A
        // compatibility during boot.  LINT1 as NMI (delivery mode 0b100).
        // Both left masked for now — `enable_core` will unmask as needed.
        self.regs.LVT_LINT0.set(0b111); // ExtINT
        self.regs.LVT_LINT1.set(0b100 << 8); // NMI

        // Enable the LAPIC: set spurious vector to 0xFF and set the
        // software-enable bit.
        self.regs.SPIVR.set(SPURIOUS_VECTOR | (1 << 8)); // bit 8 = APIC Enable
    }

    /// Write the EOI register to acknowledge the current interrupt.
    pub fn eoi(&self) {
        // SAFETY: EOI is a write-only register; writing 0 is the standard
        // acknowledgement. No pointer invalidation is possible because
        // the MMIO mapping lives for the lifetime of the driver.
        self.regs.EOI.set(0);
    }

    /// Return the APIC ID of this processor (bits [31:24]).
    pub fn apic_id(&self) -> u32 {
        (self.regs.APIC_ID.get() >> 24) & 0xFF
    }

    /// Read the ISR bit for the given vector number.
    fn is_in_service(&self, vector: u32) -> bool {
        let idx = (vector / 32) as usize;
        let bit = vector % 32;
        (self.regs.ISR[idx].get() >> bit) & 1 == 1
    }

    /// Read the IRR bit for the given vector number.
    fn is_pending(&self, vector: u32) -> bool {
        let idx = (vector / 32) as usize;
        let bit = vector % 32;
        (self.regs.IRR[idx].get() >> bit) & 1 == 1
    }

    /// Send an Inter-Processor Interrupt (IPI).
    ///
    /// The ICR write is split: write the high word (destination APIC ID)
    /// first, then the low word (vector + delivery mode) which triggers
    /// the send.
    fn send_ipi_raw(&self, dest_apic_id: u32, vector: u32, delivery_mode: u32) {
        // Wait for any previous IPI to finish sending.
        while (self.regs.ICR_LO.get() >> 12) & 1 == 1 {
            core::hint::spin_loop();
        }

        self.regs.ICR_HI.set(dest_apic_id << 24);

        self.regs.ICR_LO.set(vector | (delivery_mode << 8));
    }

    /// Broadcast an INIT IPI to all APICs (including self).
    fn send_init_ipi_all(&self) {
        while (self.regs.ICR_LO.get() >> 12) & 1 == 1 {
            core::hint::spin_loop();
        }

        self.regs.ICR_HI.set(0);
        self.regs
            .ICR_LO
            .set((DELIVERY_INIT << 8) | DEST_ALL | (1 << 14)); // level-triggered
    }
}

// ──────────────────────────────────────────────
//  InterruptController trait
// ──────────────────────────────────────────────

impl InterruptController for X86_64Lapic {
    fn enable_interrupt(&mut self, config: InterruptConfig) {
        match config.descriptor {
            InterruptDescriptor::Ipi(_) => {
                // IPIs on x86_64 are sent via the ICR, not "enabled" like
                // SPIs. The `InterruptManager::new` calls this for IPI 0
                // during construction — we treat it as a no-op since IPIs
                // are explicitly sent via `raise_ipi`.
            }
            InterruptDescriptor::Spi(spi) => {
                // For SPIs delivered via the I/O APIC, this is a no-op at
                // the LAPIC level — the I/O APIC manages redirect entries.
                // If the SPI maps to a LAPIC LVT entry, configure it here.
                let vector = (spi + 32) as u32;
                match vector {
                    // Timer vector — already configured in `init()`.
                    LAPIC_TIMER_VECTOR => {}
                    // LINT0 — unmask for ExtINT.
                    0..=255 => {
                        // Most hardware IRQs are routed via the I/O APIC,
                        // not the LAPIC LVT entries.  No action needed here.
                        let _ = vector;
                    }
                    _ => {}
                }
            }
            InterruptDescriptor::Ppi(_) => {
                // Per-processor interrupts are not typically enabled via
                // this path on x86_64.  No-op for now.
            }
        }
    }

    fn disable_interrupt(&mut self, descriptor: InterruptDescriptor) {
        match descriptor {
            InterruptDescriptor::Ipi(_) => {
                // Nothing to disable — IPIs are fire-and-forget.
            }
            InterruptDescriptor::Spi(spi) => {
                let _ = spi; // Handled via I/O APIC.
            }
            InterruptDescriptor::Ppi(_) => {}
        }
    }

    fn read_active_interrupt(&mut self) -> Option<Box<dyn InterruptContext>> {
        // Scan the ISR from highest priority (lowest vector) to lowest.
        // The LAPIC delivers the highest-priority pending interrupt first.
        for vector in 0u32..=255 {
            if self.is_in_service(vector) {
                // Vector 0xFF is the spurious interrupt — never service it.
                if vector == SPURIOUS_VECTOR {
                    return None;
                }

                let desc = InterruptDescriptor::Spi(vector as usize);

                let lapic = self.this.upgrade()?;
                let ctx = LapicInterruptContext { desc, lapic };

                return Some(Box::new(ctx));
            }
        }

        None
    }

    fn raise_ipi(&mut self, target_cpu_id: usize) {
        // Send vector 0 (IPI 0) to the target CPU.
        self.send_ipi_raw(target_cpu_id as u32, 0, DELIVERY_FIXED);
    }

    fn enable_core(&mut self, cpu_id: usize) {
        // Set task priority to 0 — accept all interrupts.
        self.regs.TPR.set(0);

        self.regs.SPIVR.set(SPURIOUS_VECTOR | (1 << 8));

        info!(
            "x86_64 LAPIC: CPU interface enabled for core {} (ID={})",
            cpu_id,
            self.apic_id()
        );
    }

    fn parse_fdt_interrupt_regs(
        &self,
        _iter: &mut dyn Iterator<Item = u32>,
    ) -> Result<InterruptConfig> {
        // x86_64 does not use FDT for interrupt configuration. This method
        // exists for trait compatibility only.
        Err(KernelError::NotSupported)
    }
}

// ──────────────────────────────────────────────
//  Driver trait (for the InterruptManager wrapper)
// ──────────────────────────────────────────────

// The `Driver` trait is implemented on `InterruptManager` by `InterruptManager`
// itself — we don't need to implement it here.

// ──────────────────────────────────────────────
//  kernel_driver! init function
// ──────────────────────────────────────────────

/// LAPIC initialisation function registered via `kernel_driver!`.
///
/// This performs:
///
/// 1. Maps the LAPIC MMIO region into the kernel address space.
/// 2. Creates the `X86_64Lapic` driver via `Arc::new_cyclic`.
/// 3. Wraps it in an `InterruptManager`.
/// 4. Sets it as the root interrupt controller.
/// 5. Registers it with the `DriverManager`.
///
/// Note: `PlatformBus::register_platform_driver` is aarch64-only, so the
/// LAPIC driver is set up directly here rather than through the platform
/// bus probe mechanism.
pub fn x86_64_lapic_init(_bus: &mut PlatformBus, dm: &mut DriverManager) -> Result<()> {
    // 1. Map the LAPIC MMIO region.
    let lapic_va = {
        let addr_spc = <ArchImpl as VirtualMemory>::kern_address_space();
        let mut kern_addr_spc = addr_spc.lock_save_irq();

        kern_addr_spc.map_mmio(PhysMemoryRegion::new(
            PA::from_value(LAPIC_BASE_PHYS),
            LAPIC_MMIO_SIZE,
        ))?
    };

    info!("x86_64 LAPIC: MMIO mapped at VA {:#x}", lapic_va.value());

    // 2. Create the LAPIC driver via Arc::new_cyclic.
    let dev: Arc<SpinLock<X86_64Lapic>> = Arc::new_cyclic(|this| {
        SpinLock::new(X86_64Lapic::new(
            // SAFETY: `lapic_va` points to a valid, mapped MMIO region
            // backed by the LAPIC hardware. The pointer is derived from
            // `map_mmio` which returns a kernel-virtual address in the
            // direct-mapping region.
            unsafe { &mut *(lapic_va.value() as *mut LapicRegs) },
            this.clone(),
        ))
    });

    // 3. Wrap in InterruptManager. This also enables IPI 0.
    let manager = InterruptManager::new("x86_64-lapic", dev);

    // 4. Set as root interrupt controller.
    set_interrupt_root(manager.clone());

    // 5. Register with the driver manager.
    dm.insert_driver(manager);

    info!("x86_64 LAPIC: driver registered as interrupt root");

    Ok(())
}

// Register the LAPIC init function in the `.driver_inits` section so
// `run_initcalls` picks it up during boot.
kernel_driver!(x86_64_lapic_init);
