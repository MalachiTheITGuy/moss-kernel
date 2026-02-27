use crate::drivers::Driver;
use crate::drivers::timer::{HwTimer, Instant};
use x86_64::VirtAddr;
use core::ptr::{read_volatile, write_volatile};

pub struct LocalApic {
    base: VirtAddr,
}

impl LocalApic {
    pub const fn new(base: VirtAddr) -> Self {
        Self { base }
    }

    unsafe fn read(&self, reg: u32) -> u32 {
        unsafe { read_volatile((self.base.as_u64() + reg as u64) as *const u32) }
    }

    unsafe fn write(&self, reg: u32, val: u32) {
        unsafe { write_volatile((self.base.as_u64() + reg as u64) as *mut u32, val); }
    }

    pub fn init(&self) {
        unsafe {
            // Enable APIC by setting bit 8 of Spurious Interrupt Vector Register
            let svr = self.read(0xF0);
            self.write(0xF0, svr | 0x100 | 0xFF); // Vector 0xFF for spurious
        }
    }

    pub fn setup_timer(&self, vector: u8) {
        unsafe {
            // Divide configuration register: divide by 16
            self.write(0x3E0, 0x03);
            // LVT Timer register: one-shot mode (unset bits 17-18)
            self.write(0x320, vector as u32);
        }
    }

    pub fn set_timer_count(&self, count: u32) {
        unsafe {
            self.write(0x380, count);
        }
    }

    pub fn get_timer_current(&self) -> u32 {
        unsafe { self.read(0x390) }
    }

    pub fn eoi(&self) {
        unsafe { self.write(0x0B0, 0); }
    }
}

pub struct ApicTimer {
    apic: LocalApic,
    freq: u64, // Ticks per second
}

impl ApicTimer {
    pub fn new(apic: LocalApic) -> Self {
        // TODO: Calibrate frequency. For now, assume 10MHz (typical for many emulators if divider is large)
        // Actually, let's use a conservative 100MHz.
        Self { apic, freq: 100_000_000 }
    }
}

impl Driver for ApicTimer {
    fn name(&self) -> &'static str {
        "apic-timer"
    }
}

impl HwTimer for ApicTimer {
    fn now(&self) -> Instant {
        let tsc: u64;
        unsafe {
            core::arch::asm!("rdtsc", "shl rdx, 32", "or rax, rdx", out("rax") tsc, out("rdx") _);
        }
        Instant::new(tsc, 2_000_000_000)
    }

    fn schedule_interrupt(&self, when: Option<Instant>) {
        if let Some(target) = when {
            let now = self.now();
            if target <= now {
                self.apic.set_timer_count(1);
            } else {
                let diff = target.ticks().saturating_sub(now.ticks());
                // TSC freq (2GHz) -> APIC freq (100MHz)
                let apic_ticks = (diff as u128 * self.freq as u128 / 2_000_000_000) as u32;
                self.apic.set_timer_count(apic_ticks.max(1));
            }
        } else {
            self.apic.set_timer_count(0);
        }
    }
}
