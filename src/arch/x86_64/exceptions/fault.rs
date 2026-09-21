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
fn format_pf_error_code(ec: u64) -> alloc::string::String {
    use core::fmt::Write;

    let mut buf = alloc::string::String::new();

    if ec & pfec::NOT_PRESENT != 0 {
        let _ = write!(&mut buf, "PRESENT ");
    }
    if ec & pfec::WRITE != 0 {
        let _ = write!(&mut buf, "WRITE ");
    }
    if ec & pfec::USER != 0 {
        let _ = write!(&mut buf, "USER ");
    }
    if ec & pfec::RESERVED != 0 {
        let _ = write!(&mut buf, "RESERVED ");
    }
    if ec & pfec::INSTRUCTION_FETCH != 0 {
        let _ = write!(&mut buf, "EXEC ");
    }
    if ec & pfec::PROTECTION_KEY != 0 {
        let _ = write!(&mut buf, "PK ");
    }
    if ec & pfec::SHADOW_STACK != 0 {
        let _ = write!(&mut buf, "SHADOW_STACK ");
    }
    if buf.is_empty() {
        let _ = write!(&mut buf, "UNKNOWN");
    }
    buf
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
    let access = format_pf_error_code(ec);

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
