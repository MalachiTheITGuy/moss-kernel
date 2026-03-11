/// Minimal I/O APIC driver for x86_64.
///
/// Programs the I/O APIC redirection table to route ISA IRQs to specific CPU
/// interrupt vectors so the Local APIC can deliver them to the BSP.
///
/// Physical base: 0xFEC0_0000 (QEMU Q35 default).
/// Virtual base: PAGE_OFFSET + 0xFEC0_0000 (accessed via kernel linear map).

use crate::arch::x86_64::memory::PAGE_OFFSET;

/// Physical base address of the I/O APIC MMIO registers.
const IOAPIC_PHYS_BASE: u64 = 0xFEC0_0000;

/// Offset of IOREGSEL (index register) within the MMIO block.
const IOREGSEL_OFFSET: usize = 0x00;
/// Offset of IOWIN (data register) within the MMIO block.
const IOWIN_OFFSET: usize = 0x10;

/// Index of the first redirection-table register (low half of entry 0).
const IOREDTBL_BASE: u32 = 0x10;

pub struct IoApic {
    base: usize,
}

impl IoApic {
    /// Create a new `IoApic` accessor using the kernel linear map.
    pub fn new() -> Self {
        Self {
            base: PAGE_OFFSET + IOAPIC_PHYS_BASE as usize,
        }
    }

    /// Write `val` to the I/O APIC indexed register `reg`.
    unsafe fn write(&self, reg: u32, val: u32) {
        let sel = (self.base + IOREGSEL_OFFSET) as *mut u32;
        let win = (self.base + IOWIN_OFFSET) as *mut u32;
        sel.write_volatile(reg);
        win.write_volatile(val);
    }

    /// Program redirection entry `gsi` to deliver `vector` to APIC ID
    /// `dest_id` as edge-triggered, active-high, fixed-delivery, unmasked.
    pub fn route_irq(&self, gsi: u8, vector: u8, dest_id: u8) {
        let lo_reg = IOREDTBL_BASE + 2 * gsi as u32;
        let hi_reg = lo_reg + 1;

        // High DWORD: destination APIC ID in bits [63:56] (relative to hi dword).
        let hi_val: u32 = (dest_id as u32) << 24;
        // Low DWORD: vector | delivery=Fixed(000) | dest=Physical(0) |
        //            polarity=ActiveHigh(0) | trigger=Edge(0) | mask=0.
        let lo_val: u32 = vector as u32;

        unsafe {
            // Write high half first so the entry isn't half-valid.
            self.write(hi_reg, hi_val);
            self.write(lo_reg, lo_val);
        }
    }

    /// Mask (disable) redirection entry `gsi`.
    pub fn mask_irq(&self, gsi: u8) {
        let lo_reg = IOREDTBL_BASE + 2 * gsi as u32;
        unsafe {
            self.write(lo_reg, 1 << 16); // mask bit
        }
    }
}
