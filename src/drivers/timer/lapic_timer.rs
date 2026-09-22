//! x86_64 Local APIC Timer driver.
//!
//! The LAPIC timer provides a per-CPU periodic or one-shot interrupt
//! source using the Local APIC's built-in timer facility.  It
//! complements the HPET by offering a low-overhead, on-core interrupt
//! suitable for context-switch ticks and short-duration timeouts.
//!
//! Register layout (offsets from LAPIC base `0xFEE0_0000`):
//!
//! | Offset | Name            | Access | Description                    |
//! |--------|-----------------|--------|--------------------------------|
//! | 0x320  | LVT Timer       | RW     | Vector, mode, mask             |
//! | 0x380  | Initial Count   | RW     | Load value for the timer       |
//! | 0x390  | Current Count   | RO     | Current counter value          |
//! | 0x3E0  | Divide Config   | RW     | Clock divider ratio            |

use alloc::sync::Arc;
use core::sync::atomic::{AtomicU64, Ordering};

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
        init::PlatformBus,
        timer::{HwTimer, Instant},
        Driver, DriverManager,
    },
    interrupts::{
        get_interrupt_root, ClaimedInterrupt, InterruptConfig, InterruptDescriptor,
        InterruptHandler, TriggerMode,
    },
    kernel_driver,
};

// ──────────────────────────────────────────────
//  LAPIC MMIO register block (timer-specific)
// ──────────────────────────────────────────────

/// Physical base of the Local APIC memory-mapped region.
const LAPIC_BASE_PHYS: u64 = 0xFEE0_0000;

/// Size of the LAPIC MMIO page (4 KiB).
const LAPIC_MMIO_SIZE: usize = 0x1000;

register_structs! {
    /// LAPIC timer register block.
    ///
    /// Only the four timer-relevant offsets are defined here; the
    /// remainder of the LAPIC page is owned by `x86_64_lapic`.
    pub LapicTimerRegs {
        /// Reserved: 0x000–0x31F (owned by x86_64_lapic)
        (0x000 => _reserved_0),
        (0x320 => lvt_timer: ReadWrite<u32>),
        /// Reserved: 0x324–0x37F
        (0x324 => _reserved_1),
        (0x380 => initial_count: ReadWrite<u32>),
        /// Reserved: 0x384–0x38F
        (0x384 => _reserved_2),
        (0x390 => current_count: ReadOnly<u32>),
        /// Reserved: 0x394–0x3DF
        (0x394 => _reserved_3),
        (0x3E0 => divide_config: ReadWrite<u32>),
        (0x3E4 => @END),
    }
}

// ──────────────────────────────────────────────
//  LVT Timer constants
// ──────────────────────────────────────────────

/// Vector delivered when the LAPIC timer fires (matches
/// `x86_64_lapic::LAPIC_TIMER_VECTOR`).
const LAPIC_TIMER_VECTOR: u32 = 0x40;

/// LVT Timer mode field (bits [12:8]).
const LVT_MODE_ONE_SHOT: u32 = 0;
const LVT_MODE_PERIODIC: u32 = 1;

/// LVT Timer mask bit (bit 13).
const LVT_MASK: u32 = 1 << 13;

/// Divide configuration: divide by 16.
const TMRDIV_DIV16: u32 = 0b011;

/// Default timer frequency in Hz (1 MHz).  This is a conservative
/// default suitable for most virtualised environments; real hardware
/// should calibrate against the HPET or TSC at boot.
const DEFAULT_FREQUENCY: u64 = 1_000_000;

// ──────────────────────────────────────────────
//  Driver, HwTimer, and InterruptHandler
// ──────────────────────────────────────────────

/// x86_64 LAPIC Timer driver.
///
/// Maps the LAPIC MMIO region and configures the LVT timer for
/// periodic interrupts.  Implements [`HwTimer`] so the timer can
/// serve as a system or per-CPU time source, and [`InterruptHandler`]
/// so it receives its own timer IRQs via the interrupt manager's
/// dispatch path.
pub struct LapicTimer {
    /// Reference to the memory-mapped LAPIC timer register block.
    regs: &'static mut LapicTimerRegs,
    /// Timer frequency in Hz (derived from divide config and bus freq).
    frequency: u64,
    /// Software tick counter, incremented on each timer interrupt.
    ticks: AtomicU64,
    /// Holds the claimed IRQ so it is released on drop.
    _interrupt: ClaimedInterrupt,
}

// SAFETY: LAPIC registers reside in a dedicated MMIO region accessed
// exclusively through this driver.  All register accesses are
// naturally-aligned 32-bit loads/stores.  The software tick counter
// uses atomics.
unsafe impl Send for LapicTimer {}
unsafe impl Sync for LapicTimer {}

impl Driver for LapicTimer {
    fn name(&self) -> &'static str {
        "x86_64-lapic-timer"
    }
}

impl HwTimer for LapicTimer {
    /// Returns the current time derived from the software tick counter.
    ///
    /// Each timer interrupt increments the counter by one, so
    /// `ticks / frequency` equals elapsed seconds.
    fn now(&self) -> Instant {
        let ticks = self.ticks.load(Ordering::Relaxed);
        Instant {
            ticks,
            freq: self.frequency,
        }
    }

    /// Schedules or cancels the next timer interrupt.
    ///
    /// - `Some(instant)`: Arms the LVT timer in one-shot mode to
    ///   fire after the appropriate number of ticks.
    /// - `None`: Masks the LVT timer interrupt.
    fn schedule_interrupt(&self, when: Option<Instant>) {
        match when {
            Some(instant) => {
                let current = self.ticks.load(Ordering::Relaxed);
                let ticks_until = instant.ticks.saturating_sub(current);
                if ticks_until == 0 {
                    return;
                }
                // One-shot mode, unmasked, with the computed count.
                let lvt_entry = LAPIC_TIMER_VECTOR | (LVT_MODE_ONE_SHOT << 8);
                self.regs.lvt_timer.set(lvt_entry);
                self.regs.initial_count.set(ticks_until as u32);
            }
            None => {
                // Mask the LVT timer interrupt.
                let lvt_entry = self.regs.lvt_timer.get() | LVT_MASK;
                self.regs.lvt_timer.set(lvt_entry);
            }
        }
    }
}

impl InterruptHandler for LapicTimer {
    /// Called by the interrupt dispatch path when vector 0x40 fires.
    ///
    /// Simply bumps the software tick counter.  The actual EOI is
    /// handled by the `LapicInterruptContext` created in
    /// `read_active_interrupt`.
    fn handle_irq(&self, _desc: InterruptDescriptor) {
        self.ticks.fetch_add(1, Ordering::Relaxed);
    }
}

// ──────────────────────────────────────────────
//  Init
// ──────────────────────────────────────────────

/// Initialises the LAPIC timer driver.
///
/// This function:
///
/// 1. Maps the LAPIC MMIO page into the kernel address space.
/// 2. Configures the divide-by-16 prescaler.
/// 3. Claims interrupt vector `0x40` from the interrupt root
///    (the LAPIC's own [`InterruptManager`]).
/// 4. Arms the LVT timer in periodic mode for the default tick.
fn x86_64_lapic_timer_init(_bus: &mut PlatformBus, _dm: &mut DriverManager) -> Result<()> {
    // 1. Map the LAPIC MMIO page.
    let addr_spc = <ArchImpl as VirtualMemory>::kern_address_space();
    let mut kern_addr_spc = addr_spc.lock_save_irq();
    let virt_base = kern_addr_spc
        .map_mmio(PhysMemoryRegion::new(
            PA::from_value(LAPIC_BASE_PHYS as usize),
            LAPIC_MMIO_SIZE,
        ))
        .map_err(|_| KernelError::Other("Failed to map LAPIC timer MMIO"))?;

    // SAFETY: `virt_base` was returned by `map_mmio` for a region
    // of size `LAPIC_MMIO_SIZE` which is >= `size_of::<LapicTimerRegs>()`.
    // The pointer is properly aligned and the region is UC-mapped.
    let regs: &'static mut LapicTimerRegs =
        unsafe { &mut *(virt_base.value() as *mut LapicTimerRegs) };

    let frequency = DEFAULT_FREQUENCY;

    // 2. Get the interrupt root (the LAPIC's InterruptManager).
    let interrupt_root =
        get_interrupt_root().ok_or_else(|| KernelError::Other("LAPIC timer: no interrupt root"))?;

    // 3. Claim vector 0x40 via the interrupt manager.
    let _timer = interrupt_root.claim_interrupt(
        InterruptConfig {
            descriptor: InterruptDescriptor::Spi(LAPIC_TIMER_VECTOR as usize),
            trigger: TriggerMode::EdgeRising,
        },
        |claimed| {
            // Configure the divide-by-16 prescaler.
            regs.divide_config.set(TMRDIV_DIV16);

            // Arm the LVT timer in periodic mode.
            let lvt_entry = LAPIC_TIMER_VECTOR | (LVT_MODE_PERIODIC << 8);
            regs.lvt_timer.set(lvt_entry);

            // Load the initial count for one tick period.
            regs.initial_count.set(frequency as u32);

            LapicTimer {
                regs,
                frequency,
                ticks: AtomicU64::new(0),
                _interrupt: claimed,
            }
        },
    )?;

    info!(
        "x86_64 LAPIC Timer initialized: vector 0x{:x}, freq {} Hz",
        LAPIC_TIMER_VECTOR, frequency,
    );

    Ok(())
}

kernel_driver!(x86_64_lapic_timer_init);
