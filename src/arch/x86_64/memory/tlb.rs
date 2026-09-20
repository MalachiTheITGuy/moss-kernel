use core::arch::asm;

use libkernel::memory::paging::TLBInvalidator;

/// TLB invalidator for all address spaces (kernel + user).
/// Equivalent to AArch64's `AllEl1TlbInvalidator`.
pub struct AllTlbInvalidator;

impl AllTlbInvalidator {
    pub fn new() -> Self {
        Self
    }
}

impl Drop for AllTlbInvalidator {
    fn drop(&mut self) {
        unsafe {
            // Reload CR3 to flush all TLB entries.
            // This is the x86_64 equivalent of AArch64's dsb/tlbi/dsb/isb sequence.
            let cr3: u64;
            asm!("mov {cr3}, cr3", cr3 = out(reg) cr3);
            asm!("mov cr3, {cr3}", cr3 = in(reg) cr3);
        }
    }
}

impl TLBInvalidator for AllTlbInvalidator {}

/// TLB invalidator for a single virtual address.
/// Uses INVLPG instruction to invalidate a single TLB entry.
pub struct SingleTlbInvalidator {
    addr: u64,
}

impl SingleTlbInvalidator {
    pub fn new(addr: u64) -> Self {
        Self { addr }
    }
}

impl Drop for SingleTlbInvalidator {
    fn drop(&mut self) {
        unsafe {
            asm!(
                "invlpg [{addr}]",
                addr = in(reg) self.addr,
                options(nostack, preserves_flags)
            );
        }
    }
}

impl TLBInvalidator for SingleTlbInvalidator {}
