//! x86_64 exception and interrupt handling.
//!
//! IDT setup and exception vector dispatch.
//!
//! TODO(#16): Implement full IDT, ISR dispatch, and APIC integration.

/// Set up the Interrupt Descriptor Table (IDT).
///
//! # Safety
//!
//! This function modifies the CPU's IDTR register. It must only be
//! called once during boot, with interrupts disabled.
pub unsafe fn setup_idt() {
    // TODO(#16): Populate all 256 IDT entries.
    // TODO(#16): Set up exception vectors (#DE, #DB, #NMI, #BP, #OF, #BR,
    //            #UD, #NM, #DF, #TS, #NP, #SS, #GP, #PF, #MF, #AC, #MC,
    //            #XM, #VE).
    // TODO(#16): Set up APIC IRQ vectors (32-255).
    // TODO(#16): Load IDTR with lidt.
    todo!("setup_idt: IDT not yet implemented")
}

/// Exception vector numbers for x86_64.
pub mod vectors {
    pub const DIVIDE_BY_ZERO: usize = 0;
    pub const DEBUG: usize = 1;
    pub const NON_MASKABLE_INTERRUPT: usize = 2;
    pub const BREAKPOINT: usize = 3;
    pub const OVERFLOW: usize = 4;
    pub const BOUND_RANGE_EXCEEDED: usize = 5;
    pub const INVALID_OPCODE: usize = 6;
    pub const DEVICE_NOT_AVAILABLE: usize = 7;
    pub const DOUBLE_FAULT: usize = 8;
    pub const INVALID_TSS: usize = 10;
    pub const SEGMENT_NOT_PRESENT: usize = 11;
    pub const STACK_SEGMENT_FAULT: usize = 12;
    pub const GENERAL_PROTECTION_FAULT: usize = 13;
    pub const PAGE_FAULT: usize = 14;
    pub const x87_FLOATING_POINT_EXCEPTION: usize = 16;
    pub const ALIGNMENT_CHECK: usize = 17;
    pub const MACHINE_CHECK: usize = 18;
    pub const SIMD_FLOATING_POINT_EXCEPTION: usize = 19;
    pub const VIRTUALIZATION_EXCEPTION: usize = 20;
    pub const SECURITY_EXCEPTION: usize = 30;

    /// First usable vector for hardware IRQs (after CPU exceptions).
    pub const IRQ_BASE: usize = 32;
}
