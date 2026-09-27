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
