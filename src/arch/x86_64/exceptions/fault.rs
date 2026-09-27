//! x86_64 fault handlers.
//!
//! Handles the most common CPU exceptions:
//!
//! - **Page Fault (#PF, vector 14)** — translates error code bits, logs
//!   diagnostics, and delegates to the memory-management fault path.
//! - **General Protection Fault (#GP, vector 13)** — logs the fault and
//!   panics with a register dump.
//! - **Double Fault (#DF, vector 8)** — always fatal; panics immediately.

use super::ExceptionState;

// ──────────────────────────────────────────────
//  Page Fault
// ──────────────────────────────────────────────

/// Page Fault error-code bits (from the Intel SDM).
mod pfec {
    /// The fault was caused by a non-present page.
    pub const NOT_PRESENT: u64 = 1 << 0;
    /// The fault was caused by a write access.
    pub const WRITE: u64 = 1 << 1;
    /// The fault was caused by a user-mode access (CPL=3).
    pub const USER: u64 = 1 << 2;
    /// The fault was caused by a reserved-bit violation.
    pub const RESERVED: u64 = 1 << 3;
    /// The fault was caused by an instruction fetch.
    pub const INSTRUCTION_FETCH: u64 = 1 << 4;
    /// The fault was caused by a protection-key violation.
    pub const PROTECTION_KEY: u64 = 1 << 5;
    /// The fault was caused by a shadow-stack access.
    pub const SHADOW_STACK: u64 = 1 << 6;
    /// Hardware set this bit when the fault was suppressed by a
    /// shadow-stack access-control check.
    pub const SGX: u64 = 1 << 7;
}

/// Decode a page-fault error code into human-readable flags.
///
/// Uses a fixed-size stack buffer to avoid heap allocation — exception
/// handlers must never allocate because the heap may not be initialized
/// yet when a fault fires during early boot.
fn format_pf_error_code(ec: u64, buf: &mut [u8; 64]) -> &str {
    use core::fmt::Write;

    // Create a wrapper that writes into the fixed buffer
    struct BufWriter<'a> {
        buf: &'a mut [u8],
        pos: usize,
    }

    impl<'a> Write for BufWriter<'a> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let bytes = s.as_bytes();
            let remaining = self.buf.len() - self.pos;
            let n = core::cmp::min(bytes.len(), remaining);
            self.buf[self.pos..self.pos + n].copy_from_slice(&bytes[..n]);
            self.pos += n;
            Ok(())
        }
    }

    let mut w = BufWriter { buf, pos: 0 };

    if ec & pfec::NOT_PRESENT != 0 {
        let _ = write!(&mut w, "NOT_PRESENT ");
    }
    if ec & pfec::WRITE != 0 {
        let _ = write!(&mut w, "WRITE ");
    }
    if ec & pfec::USER != 0 {
        let _ = write!(&mut w, "USER ");
    }
    if ec & pfec::RESERVED != 0 {
        let _ = write!(&mut w, "RESERVED ");
    }
    if ec & pfec::INSTRUCTION_FETCH != 0 {
        let _ = write!(&mut w, "EXEC ");
    }
    if ec & pfec::PROTECTION_KEY != 0 {
        let _ = write!(&mut w, "PK ");
    }
    if ec & pfec::SHADOW_STACK != 0 {
        let _ = write!(&mut w, "SHADOW_STACK ");
    }
    if w.pos == 0 {
        let _ = write!(&mut w, "UNKNOWN");
    }
    let len = w.pos;
    core::mem::drop(w);
    // Convert to &str — safe because we only wrote valid UTF-8 (ASCII)
    core::str::from_utf8(&buf[..len]).unwrap_or("UNKNOWN")
}

/// Read the faulting linear address from CR2.
///
/// # Safety
///
/// Must only be called from the page-fault handler context with
/// interrupts disabled.
unsafe fn read_cr2() -> u64 {
    let cr2: u64;
    unsafe {
        core::arch::asm!("mov {}, cr2", out(reg) cr2);
    }
    cr2
}

/// Write a u64 value as hex to the debugcon port (0xE9).
unsafe fn debugcon_write_u64(val: u64) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    // Output "0x" prefix
    unsafe {
        core::arch::asm!("out 0xe9, al", in("al") b'0', options(nostack));
        core::arch::asm!("out 0xe9, al", in("al") b'x', options(nostack));
    }
    // Output each hex digit from MSB to LSB
    let mut started = false;
    for i in (0..16).rev() {
        let nibble = ((val >> (i * 4)) & 0xF) as usize;
        if nibble != 0 || started || i == 0 {
            started = true;
            unsafe {
                core::arch::asm!("out 0xe9, al", in("al") HEX[nibble], options(nostack));
            }
        }
    }
}

/// Write a raw string to the debugcon port (0xE9) without newline.
unsafe fn debugcon_write_str(s: &str) {
    for &b in s.as_bytes() {
        unsafe {
            core::arch::asm!("out 0xe9, al", in("al") b, options(nostack));
        }
    }
}

/// Write a single byte to the debugcon port (0xE9).
unsafe fn debugcon_write_byte(b: u8) {
    unsafe {
        core::arch::asm!("out 0xe9, al", in("al") b, options(nostack));
    }
}

/// Walk the page table hierarchy for a given virtual address and dump
/// each entry value via the debugcon port (0xE9).
///
/// This is a diagnostic tool for debugging #PF — it manually walks the
/// 4-level page table using CR3 and the direct-map alias to read
/// paging structure entries, printing each raw value.
///
/// # Safety
///
/// Must only be called from the page-fault handler context with
/// interrupts disabled.  The faulting address is read from CR2.
unsafe fn dump_page_table_walk(fault_va: u64) {
    unsafe {
        // Read CR3 (PML4 base physical address)
        let cr3: u64;
        core::arch::asm!("mov {r}, cr3", r = out(reg) cr3);
        let pml4_pa = cr3 & 0x000FFFFFFFFFF000; // Mask off flags

        debugcon_write_str("\n  [PTWALK] fault_va=");
        debugcon_write_u64(fault_va);
        debugcon_write_str(" pml4_pa=");
        debugcon_write_u64(pml4_pa);

        // PAGE_OFFSET = 0xffff_8880_0000_0000
        // PA-to-VA: va = pa + PAGE_OFFSET
        let page_offset: u64 = 0xffff_8880_0000_0000;

        // Extract indices from the faulting VA
        let pml4_idx = ((fault_va >> 39) & 0x1FF) as usize;
        let pdpt_idx = ((fault_va >> 30) & 0x1FF) as usize;
        let pd_idx = ((fault_va >> 21) & 0x1FF) as usize;
        let pt_idx = ((fault_va >> 12) & 0x1FF) as usize;

        // Level 0: PML4E
        let pml4_va = pml4_pa + page_offset;
        let pml4e = core::ptr::read_volatile((pml4_va as *const u64).add(pml4_idx));
        debugcon_write_str("\n  [PTWALK] PML4[");
        debugcon_write_u64(pml4_idx as u64);
        debugcon_write_str("] = ");
        debugcon_write_u64(pml4e);

        if pml4e & 1 == 0 {
            debugcon_write_str(" (NOT PRESENT)\n");
            return;
        }

        // Level 1: PDPT
        let pdpt_pa = pml4e & 0x000FFFFFFFFFF000;
        let pdpt_va = pdpt_pa + page_offset;
        let pdpte = core::ptr::read_volatile((pdpt_va as *const u64).add(pdpt_idx));
        debugcon_write_str("\n  [PTWALK] PDPT[");
        debugcon_write_u64(pdpt_idx as u64);
        debugcon_write_str("] = ");
        debugcon_write_u64(pdpte);

        if pdpte & 1 == 0 {
            debugcon_write_str(" (NOT PRESENT)\n");
            return;
        }
        // Check if 1GB page (PS=1 at PDPT level)
        if pdpte & (1 << 7) != 0 {
            debugcon_write_str(" (1GB PAGE)\n");
            return;
        }

        // Level 2: PD
        let pd_pa = pdpte & 0x000FFFFFFFFFF000;
        let pd_va = pd_pa + page_offset;
        let pde = core::ptr::read_volatile((pd_va as *const u64).add(pd_idx));
        debugcon_write_str("\n  [PTWALK] PD[");
        debugcon_write_u64(pd_idx as u64);
        debugcon_write_str("] = ");
        debugcon_write_u64(pde);

        if pde & 1 == 0 {
            debugcon_write_str(" (NOT PRESENT)\n");
            return;
        }
        // Check if 2MB page (PS=1 at PD level)
        if pde & (1 << 7) != 0 {
            debugcon_write_str(" (2MB PAGE)\n");
            return;
        }

        // Level 3: PT
        let pt_pa = pde & 0x000FFFFFFFFFF000;
        let pt_va = pt_pa + page_offset;
        let pte = core::ptr::read_volatile((pt_va as *const u64).add(pt_idx));
        debugcon_write_str("\n  [PTWALK] PT[");
        debugcon_write_u64(pt_idx as u64);
        debugcon_write_str("] = ");
        debugcon_write_u64(pte);

        if pte & 1 == 0 {
            debugcon_write_str(" (NOT PRESENT)");
            // Check reserved bits 62:52 for non-present entries
            let reserved = pte & 0x000FFFFFFFFFF000; // bits [51:12]
            if reserved != 0 {
                debugcon_write_str(" [RESERVED BITS IN ADDRESS FIELD!]");
            }
        }
        debugcon_write_byte(b'\n');

        // Also dump PT[0] (lapic_timer's mapping) for comparison
        let pte0 = core::ptr::read_volatile((pt_va as *const u64).add(0));
        debugcon_write_str("\n  [PTWALK] PT[0x0] = ");
        debugcon_write_u64(pte0);

        // Dump CR0 to check WP bit
        let cr0: u64;
        core::arch::asm!("mov {r}, cr0", r = out(reg) cr0);
        debugcon_write_str("\n  [PTWALK] CR0=");
        debugcon_write_u64(cr0);
        debugcon_write_str(" WP=");
        debugcon_write_u64((cr0 >> 16) & 1);

        // Dump EFER to check NXE bit
        let efer_lo: u32;
        let efer_hi: u32;
        core::arch::asm!(
            "rdmsr",
            in("ecx") 0xC000_0080u32,
            out("eax") efer_lo,
            out("edx") efer_hi,
        );
        let efer: u64 = ((efer_hi as u64) << 32) | (efer_lo as u64);
        debugcon_write_str("\n  [PTWALK] EFER=");
        debugcon_write_u64(efer);
        debugcon_write_str(" NXE=");
        debugcon_write_u64((efer >> 11) & 1);

        // Dump raw 32 bytes at the PT page to verify both PTEs
        debugcon_write_str("\n  [PTWALK] PT raw[0..4]:");
        for i in 0..4 {
            let val = core::ptr::read_volatile((pt_va as *const u64).add(i));
            debugcon_write_str(" ");
            debugcon_write_u64(val);
        }

        // Now try invlpg on the faulting address and re-read the PTE
        core::arch::asm!("invlpg [{}]", in(reg) fault_va, options(nostack));
        // Re-read PTE after invlpg
        let pte_after = core::ptr::read_volatile((pt_va as *const u64).add(pt_idx));
        debugcon_write_str("\n  [PTWALK] PT[after_invlpg] = ");
        debugcon_write_u64(pte_after);
        debugcon_write_byte(b'\n');
    }
}

/// Handle a Page Fault (#PF, vector 14).
///
/// This is the first-stage handler.  It reads the faulting address from
/// CR2, formats the error code, logs diagnostics, and panics for now.
/// A full implementation will delegate to the memory-management
/// subsystem to resolve demand faults, copy-on-write, etc.
pub fn handle_page_fault(state: &ExceptionState) {
    let cr2 = unsafe { read_cr2() };
    let ec = state.error_code;
    let mut pf_buf = [0u8; 64];
    let access = format_pf_error_code(ec, &mut pf_buf);

    // Diagnostic: walk the page table and dump each entry via debugcon
    unsafe {
        dump_page_table_walk(cr2);
    }

    let user = (ec & pfec::USER) != 0;
    let level = if user { "user" } else { "kernel" };

    panic!(
        "#PF ({level}) at {cr2:#018x}  error_code={ec:#06x} [{access}]\n\
         \trax={:#018x}  rbx={:#018x}  rcx={:#018x}  rdx={:#018x}\n\
         \trsi={:#018x}  rdi={:#018x}  rbp={:#018x}  rsp={:#018x}\n\
         \tr8 ={:#018x}  r9 ={:#018x}  r10={:#018x}  r11={:#018x}\n\
         \tr12={:#018x}  r13={:#018x}  r14={:#018x}  r15={:#018x}\n\
         \trip={:#018x}  cs={:#018x}  rflags={:#018x}",
        state.rax,
        state.rbx,
        state.rcx,
        state.rdx,
        state.rsi,
        state.rdi,
        state.rbp,
        state.rsp,
        state.r8,
        state.r9,
        state.r10,
        state.r11,
        state.r12,
        state.r13,
        state.r14,
        state.r15,
        state.rip,
        state.cs,
        state.rflags,
    );
}

// ──────────────────────────────────────────────
//  General Protection Fault
// ──────────────────────────────────────────────

/// Handle a General Protection Fault (#GP, vector 13).
///
/// Always panics — the kernel cannot recover from a GP fault.
pub fn handle_gp_fault(state: &ExceptionState) {
    panic!("#GP error_code={:#06x}\n{state}", state.error_code);
}

// ──────────────────────────────────────────────
//  Double Fault
// ──────────────────────────────────────────────

/// Handle a Double Fault (#DF, vector 8).
///
/// Always fatal — the processor has failed to invoke another exception
/// handler.  We panic immediately with a register dump.
pub fn handle_double_fault(state: &ExceptionState) {
    panic!(
        "#DF (DOUBLE FAULT) error_code={:#06x}\n{state}",
        state.error_code
    );
}

// ──────────────────────────────────────────────
//  Invalid Opcode (#UD)
// ──────────────────────────────────────────────

/// Handle an Invalid Opcode fault (#UD, vector 6).
///
/// Panics with the faulting instruction pointer so the developer can
/// disassemble at the exact location.
pub fn handle_invalid_opcode(state: &ExceptionState) {
    panic!("#UD (Invalid Opcode) at RIP={:#018x}\n{state}", state.rip);
}

// ──────────────────────────────────────────────
//  Device Not Available (#NM)
// ──────────────────────────────────────────────

/// Handle a Device Not Available fault (#NM, vector 7).
///
/// This typically means `FNINIT`/`FPU` was used before the FPU was
/// initialised.
pub fn handle_device_not_available(state: &ExceptionState) {
    panic!(
        "#NM (Device Not Available) at RIP={:#018x}\n{state}",
        state.rip
    );
}

// ──────────────────────────────────────────────
//  Machine Check (#MC)
// ──────────────────────────────────────────────

/// Handle a Machine Check (#MC, vector 18).
///
/// Always fatal — hardware-detected uncorrectable error.
pub fn handle_machine_check(state: &ExceptionState) {
    panic!("#MC (Machine Check) — hardware error\n{state}");
}
