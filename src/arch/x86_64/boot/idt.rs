/*
 * x86_64 Interrupt Descriptor Table (IDT) definitions.
 *
 * Provides types and helpers for the 256-entry IDT used by the moss
 * kernel to dispatch CPU exceptions and hardware interrupts on x86_64.
 *
 * Follows the same structural conventions as the GDT module:
 * bare-metal types, no heap allocation, no unsafe outside
 * inline-asm helpers.
 */

use core::arch::asm;

/// Number of IDT entries (256 for x86_64).
const IDT_ENTRIES: usize = 256;

/// Total byte size of the IDT (256 × 16 bytes).
const IDT_SIZE: usize = IDT_ENTRIES * 16;

/// IST (Interrupt Stack Table) index for the double-fault handler.
/// The CPU switches to IST1 when entering the #DF handler, providing
/// a guaranteed valid stack even if the normal kernel stack is corrupt.
pub const IST_DF: u8 = 1;

/// IST index for the NMI handler.
pub const IST_NMI: u8 = 2;

/// IST index for the machine-check handler.
pub const IST_MC: u8 = 3;

/// Kernel code segment selector in the GDT (offset 0x08).
const KERNEL_CODE_SELECTOR: u16 = 0x08;

/// IDT gate type for an interrupt gate (clears IF on entry).
const GATE_INTERRUPT: u8 = 0xE;

/// IDT gate type for a trap gate (does not clear IF on entry).
const GATE_TRAP: u8 = 0xF;

// ──────────────────────────────────────────────
//  IDT entry (16 bytes, Intel SDM Vol. 3A §6.10)
// ──────────────────────────────────────────────

/// A single 16-byte IDT gate descriptor.
///
/// Layout matches the Intel SDM:
/// ```text
/// Bits [15:0]   Offset [15:0]
/// Bits [16:31]  Segment Selector
/// Bits [32:34]  IST index (bits 0–2)
/// Bits [35:39]  Reserved (must be zero)
/// Bits [40:44]  Type (interrupt / trap)
/// Bit  45       DPL
/// Bit  47       Present
/// Bits [48:63]  Offset [31:16]
/// Bits [64:95]  Offset [63:32]
/// Bits [96:127] Reserved (must be zero)
/// ```
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct IdtEntry {
    /// Offset bits 0–15 of the handler address.
    pub offset_lo: u16,
    /// Segment selector (always kernel code for ring-0 handlers).
    pub selector: u16,
    /// IST index (bits 0–2) and two reserved bits.
    /// The upper 3 bits are always zero.
    ist_flags: u8,
    /// Gate type (4 bits), DPL (2 bits), always-1 (1 bit),
    /// present (1 bit).
    type_attr: u8,
    /// Offset bits 16–31 of the handler address.
    pub offset_mid: u16,
    /// Offset bits 32–63 of the handler address.
    pub offset_hi: u32,
    /// Reserved — must be zero.
    reserved: u32,
}

impl IdtEntry {
    /// Create a disabled (not-present) IDT entry.
    pub const fn absent() -> Self {
        Self {
            offset_lo: 0,
            selector: 0,
            ist_flags: 0,
            type_attr: 0,
            offset_mid: 0,
            offset_hi: 0,
            reserved: 0,
        }
    }

    /// Create an IDT entry for a 64-bit interrupt gate.
    ///
    /// * `handler` — virtual address of the handler function.
    /// * `ist`     — IST index (0 = use normal RSP switch, 1–7 = IST stack).
    /// * `dpl`     — Descriptor Privilege Level (0 = kernel-only, 3 = user).
    pub fn interrupt_gate(handler: u64, ist: u8, dpl: u8) -> Self {
        assert!(ist <= 7, "IST index must be 0–7");
        assert!(dpl <= 3, "DPL must be 0–3");

        let offset_lo = (handler & 0xFFFF) as u16;
        let offset_mid = ((handler >> 16) & 0xFFFF) as u16;
        let offset_hi = ((handler >> 32) & 0xFFFF_FFFF) as u32;

        // IST occupies bits 0–2 of byte 4; byte 5 is reserved (zero).
        let ist_flags = ist & 0x07;

        // Type = 0xE (interrupt gate), P = 1, DPL = dpl.
        // Bit layout: P(1) | DPL(2) | 0 | Type(4)
        let type_attr = 0x80 | ((dpl & 0x03) << 5) | GATE_INTERRUPT;

        Self {
            offset_lo,
            selector: KERNEL_CODE_SELECTOR,
            ist_flags,
            type_attr,
            offset_mid,
            offset_hi,
            reserved: 0,
        }
    }

    /// Create an IDT entry for a 64-bit trap gate.
    ///
    /// Trap gates do not clear IF on entry (interrupts stay enabled).
    pub fn trap_gate(handler: u64, ist: u8, dpl: u8) -> Self {
        assert!(ist <= 7, "IST index must be 0–7");
        assert!(dpl <= 3, "DPL must be 0–3");

        let offset_lo = (handler & 0xFFFF) as u16;
        let offset_mid = ((handler >> 16) & 0xFFFF) as u16;
        let offset_hi = ((handler >> 32) & 0xFFFF_FFFF) as u32;

        let ist_flags = ist & 0x07;
        let type_attr = 0x80 | ((dpl & 0x03) << 5) | GATE_TRAP;

        Self {
            offset_lo,
            selector: KERNEL_CODE_SELECTOR,
            ist_flags,
            type_attr,
            offset_mid,
            offset_hi,
            reserved: 0,
        }
    }
}

// ──────────────────────────────────────────────
//  IDT register (IDTR) — 10 bytes
// ──────────────────────────────────────────────

/// IDTR value loaded by `lidt`.
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct Idtr {
    /// Size of the IDT in bytes minus one.
    pub limit: u16,
    /// Linear base address of the IDT.
    pub base: u64,
}

// ──────────────────────────────────────────────
//  Static IDT and IST stacks
// ──────────────────────────────────────────────

/// The kernel's static IDT — 256 entries, 4096 bytes total.
///
/// Placed in `.bss` (zeroed at load). Entries are populated by
/// `setup_idt()` during early boot.
#[unsafe(no_mangle)]
#[unsafe(link_section = ".bss")]
pub static mut KERNEL_IDT: [IdtEntry; IDT_ENTRIES] = [IdtEntry::absent(); IDT_ENTRIES];

/// Double-fault stack — IST1.
///
/// 4 KiB is sufficient because #DF is immediately fatal (the CPU
/// switches to this stack because the normal stack is presumably
/// corrupt).
#[unsafe(no_mangle)]
#[unsafe(link_section = ".bss")]
pub static mut IST_DF_STACK: [u8; 4096] = [0u8; 4096];

/// NMI stack — IST2.
#[unsafe(no_mangle)]
#[unsafe(link_section = ".bss")]
pub static mut IST_NMI_STACK: [u8; 4096] = [0u8; 4096];

/// Machine-check stack — IST3.
#[unsafe(no_mangle)]
#[unsafe(link_section = ".bss")]
pub static mut IST_MC_STACK: [u8; 4096] = [0u8; 4096];

/// Return the top (highest valid address) of the double-fault IST stack.
pub fn ist_df_top() -> u64 {
    unsafe { core::ptr::addr_of!(IST_DF_STACK).add(1) as u64 }
}

/// Return the top of the NMI IST stack.
pub fn ist_nmi_top() -> u64 {
    unsafe { core::ptr::addr_of!(IST_NMI_STACK).add(1) as u64 }
}

/// Return the top of the machine-check IST stack.
pub fn ist_mc_top() -> u64 {
    unsafe { core::ptr::addr_of!(IST_MC_STACK).add(1) as u64 }
}

// ──────────────────────────────────────────────
//  IDT population helpers
// ──────────────────────────────────────────────

/// Populate an IDT entry by handler address.
///
/// # Safety
///
/// `handler` must be a valid 64-bit handler address (typically obtained
/// from the linker via `extern` symbols or `as u64` casts).
pub unsafe fn set_idt_entry(vector: usize, handler: u64, ist: u8, dpl: u8) {
    assert!(vector < IDT_ENTRIES);
    // SAFETY: single-threaded boot context; `vector` bounds-checked above.
    unsafe {
        KERNEL_IDT[vector] = IdtEntry::interrupt_gate(handler, ist, dpl);
    }
}

/// Populate an IDT entry as a trap gate.
pub unsafe fn set_idt_trap(vector: usize, handler: u64, dpl: u8) {
    assert!(vector < IDT_ENTRIES);
    // SAFETY: single-threaded boot context; `vector` bounds-checked above.
    unsafe {
        KERNEL_IDT[vector] = IdtEntry::trap_gate(handler, 0, dpl);
    }
}

/// Set up IST entries in the TSS for double-fault, NMI, and
/// machine-check stacks.
///
/// # Safety
///
/// `tss` must point to the live TSS loaded in the GDT.
pub unsafe fn setup_ist(tss: *mut super::gdt::Tss) {
    // SAFETY: caller guarantees `tss` is a valid, live TSS pointer.
    unsafe {
        (*tss).ist1 = ist_df_top();
        (*tss).ist2 = ist_nmi_top();
        (*tss).ist3 = ist_mc_top();
    }
}

/// Load the IDTR register with the address and size of the kernel IDT.
///
/// # Safety
///
/// Must only be called once, with interrupts disabled. The IDT must
/// already be populated with valid entries.
pub unsafe fn load_idt() {
    let idtr = Idtr {
        limit: (IDT_SIZE - 1) as u16,
        // SAFETY: takes the physical address of the static IDT; single-threaded boot.
        base: core::ptr::addr_of!(KERNEL_IDT) as u64,
    };
    // SAFETY: loads IDTR with a valid descriptor; interrupts must be disabled.
    unsafe {
        asm!("lidt [{}]", in(reg) &idtr);
    }
}

// ──────────────────────────────────────────────
//  ISR stub address table (entry.S symbols)
// ──────────────────────────────────────────────
//
// Each of the 256 ISR stubs (`isr_stub_0` – `isr_stub_255`) is
// defined in `entry.S` and pushes a vector number (and error code
// or dummy zero) before jumping to `interrupt_common`.
//
// The lookup table (`isr_stub_table`) is generated in `entry.S`
// via the `cc` crate so that LLVM never has to process 256
// individual `.quad` directives through `global_asm!`.

/// Table of ISR stub virtual addresses, indexed by vector number.
///
/// Defined in `entry.S` in the `.rodata.isr_stubs` section.
/// Each entry is the 64-bit virtual address of `isr_stub_N`.
unsafe extern "C" {
    #[link_name = "isr_stub_table"]
    static ISR_STUB_TABLE: [u64; 256];
}

/// Return the virtual address of the ISR stub for vector `n`.
fn isr_stub_addr(n: usize) -> u64 {
    assert!(n < IDT_ENTRIES, "ISR vector out of range: {n}");
    // SAFETY: `ISR_STUB_TABLE` is defined in `entry.S` with 256
    // `.quad` entries, one per ISR stub.  The linker resolves the
    // symbol to the correct virtual address at link time.
    unsafe { ISR_STUB_TABLE[n] }
}

// ──────────────────────────────────────────────
//  setup_idt — populate all 256 entries and load
// ──────────────────────────────────────────────

/// Populate every IDT entry and load the IDTR register.
///
/// Each vector 0–255 is wired to its corresponding `isr_stub_N`
/// trampoline in `entry.S`.  Special handling:
///
/// | Vector | Exception            | Gate    | IST  | DPL |
/// |--------|----------------------|---------|------|-----|
/// |      2 | Non-Maskable Int.    | Interrupt | IST2 | 0 |
/// |      8 | Double Fault         | Interrupt | IST1 | 0 |
/// |     18 | Machine Check        | Interrupt | IST3 | 0 |
/// |  0x80  | Syscall              | Trap    | —    | 3  |
///
/// # Safety
///
/// Must be called exactly once with interrupts disabled.
pub unsafe fn setup_idt() {
    for vector in 0..IDT_ENTRIES {
        let addr = isr_stub_addr(vector);

        // Vectors with IST assignments or user-visible DPL are handled
        // explicitly; everything else is a plain kernel interrupt gate.
        // SAFETY: single-threaded boot; vector in 0..IDT_ENTRIES range.
        unsafe {
            KERNEL_IDT[vector] = match vector {
                2 => IdtEntry::interrupt_gate(addr, IST_NMI, 0), // NMI — IST2
                8 => IdtEntry::interrupt_gate(addr, IST_DF, 0),  // #DF — IST1
                18 => IdtEntry::interrupt_gate(addr, IST_MC, 0), // #MC — IST3
                0x80 => IdtEntry::trap_gate(addr, 0, 3),         // SYSCALL — DPL=3
                _ => IdtEntry::interrupt_gate(addr, 0, 0),
            };
        }
    }

    // SAFETY: IDT fully populated; interrupts disabled; called once.
    unsafe {
        load_idt();
    }
}
