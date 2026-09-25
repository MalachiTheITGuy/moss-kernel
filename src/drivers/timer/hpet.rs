//! x86_64 HPET (High Precision Event Timer) driver.
//!
//! Provides the free-running counter for `Instant::now()` and one-shot
//! timer interrupts via Timer 0 for `schedule_interrupt()`.  Mirrors
//! the structure and patterns of the ARM generic timer in `armv8_arch.rs`.

use alloc::sync::Arc;
use libkernel::{
    error::{KernelError, Result},
    memory::{address::PA, region::PhysMemoryRegion},
};
use log::info;
use tock_registers::{
    interfaces::{Readable, Writeable},
    register_structs,
    registers::{ReadOnly, ReadWrite},
};

use libkernel::memory::proc_vm::address_space::{KernAddressSpace, VirtualMemory};

use crate::{
    arch::ArchImpl,
    drivers::{
        Driver, DriverManager,
        init::PlatformBus,
        timer::{HwTimer, Instant, SYS_TIMER, SysTimer},
    },
    interrupts::{
        ClaimedInterrupt, InterruptConfig, InterruptDescriptor, InterruptManager, TriggerMode,
        get_interrupt_root,
    },
    kernel_driver,
    sync::SpinLock,
};

// ── HPET constants ────────────────────────────────────────────────

/// Legacy-mapped MMIO base address of the HPET.
const HPET_BASE_PHYS: usize = 0xFED0_0000;

/// Size of the HPET MMIO register block.
const HPET_MMIO_SIZE: usize = 0x1000;

/// General configuration register bits.
const HPET_CFG_ENABLE: u64 = 1 << 0;
const HPET_CFG_LEG_RT: u64 = 1 << 1;

/// Timer 0 configuration register bits.
const HPET_T0_CFG_ENABLE: u64 = 1 << 2;
const HPET_T0_CFG_PERIODIC: u64 = 1 << 3;
const HPET_T0_CFG_FSB: u64 = 1 << 14;

/// HPET Timer 0 is wired to I/O APIC SPI 2 (LAPIC vector 34).
const HPET_TIMER0_IRQ: usize = 2;

// ── Register layout ───────────────────────────────────────────────

register_structs! {
    HpetRegs {
        /// 0x000: Capabilities and ID (Read-Only).
        (0x000 => capabilities: ReadOnly<u64>),
        (0x008 => _reserved_0),
        /// 0x010: General Configuration (Read-Write).
        (0x010 => config: ReadWrite<u64>),
        (0x018 => _reserved_1),
        /// 0x020: General Interrupt Status (Read-Write).
        (0x020 => interrupt_status: ReadWrite<u64>),
        (0x028 => _reserved_2),
        /// 0x0F0: Main Counter Value (Read-Write).
        (0x0F0 => main_counter: ReadWrite<u64>),
        (0x0F8 => _reserved_3),
        /// 0x100: Timer 0 Configuration and Capabilities (Read-Write).
        (0x100 => timer0_config: ReadWrite<u64>),
        (0x108 => _reserved_4),
        /// 0x110: Timer 0 Comparator Value (Read-Write).
        (0x110 => timer0_comparator: ReadWrite<u64>),
        (0x118 => @END),
    }
}

impl HpetRegs {
    /// Counter clock period in femtoseconds (bits 32–63 of Capabilities).
    fn counter_period_fs(&self) -> u32 {
        ((self.capabilities.get() >> 32) & 0xFFFF_FFFF) as u32
    }

    /// Revision ID (bits 0–7 of Capabilities).
    fn revision_id(&self) -> u8 {
        (self.capabilities.get() & 0xFF) as u8
    }

    /// Number of timers minus one (bits 8–12 of Capabilities).
    fn num_timers(&self) -> u8 {
        ((self.capabilities.get() >> 8) & 0x1F) as u8
    }
}

// ── Driver + HwTimer implementation ──────────────────────────────

/// x86_64 HPET driver.
///
/// Maps the legacy MMIO region at `0xFED0_0000` and exposes the
/// free-running main counter for `now()` as well as Timer 0
/// one-shot interrupts for `schedule_interrupt()`.
pub struct HpetTimer {
    /// Reference to the memory-mapped HPET register block.
    regs: &'static mut HpetRegs,
    /// Counter frequency in Hz, derived from the capabilities register.
    frequency: u64,
    /// Holds the claimed IRQ so it is released when the driver is dropped.
    claimed: SpinLock<Option<ClaimedInterrupt>>,
}

// SAFETY: HPET registers are in a dedicated MMIO region accessed
// exclusively through this driver.  All register accesses are
// atomic 64-bit reads/writes.
unsafe impl Send for HpetTimer {}
unsafe impl Sync for HpetTimer {}

impl HpetTimer {
    /// Perform initial hardware configuration.
    fn init(&mut self) {
        // Enable the global HPET counter.
        self.regs.config.set(HPET_CFG_ENABLE);

        // Clear legacy-replacement mode (we route through I/O APIC, not PIC).
        let cfg = self.regs.config.get();
        self.regs.config.set(cfg & !HPET_CFG_LEG_RT);

        // Ensure Timer 0 is in one-shot mode (clear periodic + FSB bits).
        let t0cfg = self.regs.timer0_config.get();
        self.regs
            .timer0_config
            .set(t0cfg & !(HPET_T0_CFG_PERIODIC | HPET_T0_CFG_FSB));

        // Acknowledge any pending Timer 0 interrupt.
        self.regs.interrupt_status.set(1 << 0);

        info!(
            "x86_64 HPET: rev={}, timers={}, freq={}",
            self.regs.revision_id(),
            self.regs.num_timers() + 1,
            self.frequency,
        );
    }

    /// Read the raw main counter value.
    fn counter(&self) -> u64 {
        self.regs.main_counter.get()
    }
}

impl Driver for HpetTimer {
    fn name(&self) -> &'static str {
        "x86_64-hpet"
    }
}

impl HwTimer for HpetTimer {
    /// Returns the current time derived from the HPET free-running counter.
    fn now(&self) -> Instant {
        let ticks = self.counter();
        Instant {
            ticks,
            freq: self.frequency,
        }
    }

    /// Schedules or cancels the next timer interrupt.
    ///
    /// - `Some(instant)`: Arms Timer 0 to fire when the main counter
    ///   reaches `instant.ticks`.
    /// - `None`: Disables the Timer 0 interrupt.
    fn schedule_interrupt(&self, when: Option<Instant>) {
        match when {
            Some(instant) => {
                // Set the comparator value for the next deadline.
                self.regs.timer0_comparator.set(instant.ticks);
                // Enable the Timer 0 interrupt.
                let cfg = self.regs.timer0_config.get();
                self.regs.timer0_config.set(cfg | HPET_T0_CFG_ENABLE);
            }
            None => {
                // Disable the Timer 0 interrupt.
                let cfg = self.regs.timer0_config.get();
                self.regs.timer0_config.set(cfg & !HPET_T0_CFG_ENABLE);
            }
        }
    }
}

// ── Init function ─────────────────────────────────────────────────

/// Initialise the x86_64 HPET, wire Timer 0 interrupt via the I/O APIC,
/// and install it as the system-wide [`SysTimer`].
///
/// Follows the same structural pattern as [`x86_64_lapic_init`] and
/// [`x86_64_ioapic_init`]: map MMIO → `Arc::new_cyclic` → claim
/// interrupt → register with [`DriverManager`].
///
/// # Panics
///
/// - If the root interrupt controller has not been set up yet (by
///   `x86_64_lapic_init`).
/// - If [`SYS_TIMER`] has already been set.
pub fn x86_64_hpet_init(_bus: &mut PlatformBus, _dm: &mut DriverManager) -> Result<()> {
    // 1. Map the HPET MMIO region.
    let hpet_va = {
        let addr_spc = <ArchImpl as VirtualMemory>::kern_address_space();
        let mut kern_addr_spc = addr_spc.lock_save_irq();
        kern_addr_spc.map_mmio(PhysMemoryRegion::new(
            PA::from_value(HPET_BASE_PHYS),
            HPET_MMIO_SIZE,
        ))?
    };

    info!("x86_64 HPET: MMIO mapped at VA {:#x}", hpet_va.value());

    // 2. Read the counter period from capabilities to compute frequency.
    let frequency = {
        // SAFETY: `hpet_va` points to a valid MMIO region backed by the
        // HPET hardware.  We use a shared reference for the read-only
        // capabilities register.
        let regs = unsafe { &*(hpet_va.value() as *const HpetRegs) };
        let period_fs = regs.counter_period_fs();
        if period_fs > 0 {
            // freq (Hz) = 10^15 / period (fs)
            1_000_000_000_000_000u64 / period_fs as u64
        } else {
            // Fallback: assume 10 MHz if the period field is zero.
            10_000_000
        }
    };

    // 3. Obtain the root interrupt controller (set by LAPIC init).
    let interrupt_manager =
        get_interrupt_root().ok_or(KernelError::Other("HPET: no interrupt root"))?;

    // 4. Claim the HPET Timer 0 interrupt and wire up the SysTimer.
    //
    //    This mirrors the ARM timer probe pattern in `armv8_arch.rs`:
    //    the HwTimer driver is created inside the `claim_interrupt`
    //    callback so that the `ClaimedInterrupt` token is held by the
    //    driver itself.
    let timer_config = InterruptConfig {
        descriptor: InterruptDescriptor::Spi(HPET_TIMER0_IRQ),
        trigger: TriggerMode::LevelHigh,
    };

    let sys_timer = interrupt_manager.claim_interrupt(timer_config, |claimed| {
        // SAFETY: `hpet_va` was mapped above and is valid for
        // the lifetime of this driver.
        let regs: &mut HpetRegs = unsafe { &mut *(hpet_va.value() as *mut HpetRegs) };

        // Configure the hardware before wrapping in Arc.
        // Enable the global HPET counter.
        regs.config.set(HPET_CFG_ENABLE);

        // Clear legacy-replacement mode (we route through I/O APIC, not PIC).
        let cfg = regs.config.get();
        regs.config.set(cfg & !HPET_CFG_LEG_RT);

        // Ensure Timer 0 is in one-shot mode (clear periodic + FSB bits).
        let t0cfg = regs.timer0_config.get();
        regs.timer0_config
            .set(t0cfg & !(HPET_T0_CFG_PERIODIC | HPET_T0_CFG_FSB));

        // Acknowledge any pending Timer 0 interrupt.
        regs.interrupt_status.set(1 << 0);

        info!(
            "x86_64 HPET: rev={}, timers={}, freq={} Hz",
            regs.capabilities.get() & 0xFF,
            ((regs.capabilities.get() >> 8) & 0x1F) + 1,
            frequency,
        );

        let timer = Arc::new(HpetTimer {
            regs,
            frequency,
            claimed: SpinLock::new(Some(claimed)),
        });

        // Schedule the first timer interrupt 5 ms from now.
        let now_ticks = timer.counter();
        let deadline_ticks = now_ticks + frequency / 200; // 5 ms
        timer.schedule_interrupt(Some(Instant {
            ticks: deadline_ticks,
            freq: frequency,
        }));

        SysTimer::from_driver(timer)
    })?;

    // 5. Install as the system-wide timer.
    SYS_TIMER
        .set(sys_timer)
        .map_err(|_| KernelError::Other("HPET: SYS_TIMER already set"))?;

    info!("x86_64 HPET: system timer installed ({} Hz)", frequency);
    Ok(())
}

// Register the HPET init function in the `.driver_inits` section so
// that `run_initcalls` picks it up during boot.
kernel_driver!(x86_64_hpet_init);
