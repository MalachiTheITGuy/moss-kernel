/*
 * x86_64 Global Descriptor Table definitions.
 *
 * The boot assembly (start.S) sets up a minimal 3-entry GDT for the
 * transition to long mode.  This module provides the Rust-side types
 * needed to reload a full GDT (with TSS) once the kernel is running.
 *
 * Follows the same structural conventions as the arm64 equivalent:
 * bare-metal types only, no heap allocation, no unsafe outside
 * inline-asm helpers.
 */

use core::arch::asm;

/// x86_64 GDT limit (bytes − 1).
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct GdtDescriptor {
    /// Size of the GDT in bytes minus one.
    pub limit: u16,
    /// Linear base address of the GDT.
    pub base: u64,
}

/// An 8-byte GDT / LDT segment descriptor.
///
/// Layout matches the Intel SDM Vol. 3A §3.4.5.
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct SegmentDescriptor {
    /// Limit bits 0–15.
    pub limit_lo: u16,
    /// Base bits 0–15.
    pub base_lo: u16,
    /// Base bits 16–23.
    pub base_mid: u8,
    /// Access byte.
    pub access: u8,
    /// Flags (4 bits) + Limit bits 16–19 (4 bits).
    pub flags_limit_hi: u8,
    /// Base bits 24–31.
    pub base_hi: u8,
}

impl SegmentDescriptor {
    /// Create a null descriptor.
    pub const fn zero() -> Self {
        Self {
            limit_lo: 0,
            base_lo: 0,
            base_mid: 0,
            access: 0,
            flags_limit_hi: 0,
            base_hi: 0,
        }
    }

    /// Create a 64-bit code segment descriptor.
    ///
    /// * Base = 0, Limit = 0xFFFFF (ignored in long mode)
    /// * Present, DPL=0, S=1, Type=0xA (execute/read)
    /// * G=1, D=0 (mandatory for long mode), L=1 (long mode)
    pub const fn code64() -> Self {
        Self {
            limit_lo: 0xFFFF,
            base_lo: 0,
            base_mid: 0,
            // P=1 | DPL=00 | S=1 | Type=1010 → 0x9A
            access: 0x9A,
            // G=1 | D=0 | L=1 | AVL=0 | Limit[19:16]=1111 → 0xAF
            flags_limit_hi: 0xAF,
            base_hi: 0,
        }
    }

    /// Create a 64-bit data segment descriptor.
    ///
    /// * Base = 0, Limit = 0xFFFFF
    /// * Present, DPL=0, S=1, Type=0x2 (read/write)
    /// * G=1, D=0, L=0
    pub const fn data64() -> Self {
        Self {
            limit_lo: 0xFFFF,
            base_lo: 0,
            base_mid: 0,
            // P=1 | DPL=00 | S=1 | Type=0010 → 0x92
            access: 0x92,
            // G=1 | D=0 | L=0 | AVL=0 | Limit[19:16]=1111 → 0xCF
            flags_limit_hi: 0xCF,
            base_hi: 0,
        }
    }
}

/// System-visible portion of the Task State Segment (TSS).
///
/// Only the fields required for ring-0 stack switching (RSP0–RSP2,
/// IST1–IST7) are modelled here; the full TSS is 104 bytes on
/// x86_64 and lives in a `#[repr(C, packed)]` struct below.
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct Tss {
    /// Reserved, must be zero.
    pub reserved: u32,
    /// RSP for ring 0.
    pub rsp0: u64,
    /// RSP for ring 1.
    pub rsp1: u64,
    /// RSP for ring 2.
    pub rsp2: u64,
    /// Reserved, must be zero.
    pub reserved2: u32,
    /// Interrupt Stack Table entries.
    pub ist1: u64,
    pub ist2: u64,
    pub ist3: u64,
    pub ist4: u64,
    pub ist5: u64,
    pub ist6: u64,
    pub ist7: u64,
    /// Reserved, must be zero.
    pub reserved3: u64,
    /// I/O port bitmap base (offset from TSS base).
    pub iopb_offset: u16,
}

impl Tss {
    /// Create a zeroed TSS.
    pub const fn new() -> Self {
        Self {
            reserved: 0,
            rsp0: 0,
            rsp1: 0,
            rsp2: 0,
            reserved2: 0,
            ist1: 0,
            ist2: 0,
            ist3: 0,
            ist4: 0,
            ist5: 0,
            ist6: 0,
            ist7: 0,
            reserved3: 0,
            iopb_offset: core::mem::size_of::<Self>() as u16,
        }
    }
}

/// 16-byte TSS descriptor for the GDT (2 consecutive GDT entries).
///
/// x86_64 TSS descriptors are 16 bytes and use the "available 64-bit
/// TSS" type (Type=0x9).
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct TssDescriptor {
    /// System descriptor — base/limit for the TSS in memory.
    pub limit_lo: u16,
    pub base_lo: u16,
    pub base_mid: u8,
    /// Access byte: P=1 | DPL=001 | 0 | Type=1001 → 0x89
    pub access: u8,
    /// Granularity: G=0 | 0 | 0 | 0 | Limit[19:16]=0000 → 0x00
    pub granularity: u8,
    pub base_hi: u8,
    /// Base bits 32–63.
    pub base_upper: u32,
    /// Reserved, must be zero.
    pub reserved: u32,
}

impl TssDescriptor {
    /// Build a TSS descriptor from a TSS reference.
    ///
    /// # Safety
    /// Caller must ensure `tss` points to a valid, live TSS and is
    /// properly aligned.
    pub fn from_tss(tss: &Tss) -> Self {
        let base = tss as *const Tss as u64;
        let limit = core::mem::size_of::<Tss>() as u32 - 1;

        Self {
            limit_lo: limit as u16,
            base_lo: (base & 0xFFFF) as u16,
            base_mid: ((base >> 16) & 0xFF) as u8,
            // P=1 | DPL=001 | 0 | Type=1001 (available 64-bit TSS)
            access: 0x89,
            // G=0 | 0 | 0 | AVL=0 | Limit[19:16]=0000
            granularity: 0x00,
            base_hi: ((base >> 24) & 0xFF) as u8,
            base_upper: (base >> 32) as u32,
            reserved: 0,
        }
    }
}

/// Combined layout of a 16-byte TSS descriptor (two consecutive GDT
/// entries, as required by the architecture).
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct TssDescriptorPair {
    pub desc: TssDescriptor,
}

impl TssDescriptorPair {
    /// Create a TSS descriptor pair from a TSS reference.
    pub fn from_tss(tss: &Tss) -> Self {
        Self {
            desc: TssDescriptor::from_tss(tss),
        }
    }
}

// ──────────────────────────────────────────────
//  Boot GDT + TSS — loaded by exceptions_init()
// ──────────────────────────────────────────────

/// GDT selector for kernel code segment (offset 0x08).
pub const KERNEL_CODE_SEL: u16 = 0x08;
/// GDT selector for kernel data segment (offset 0x10).
pub const KERNEL_DATA_SEL: u16 = 0x10;
/// GDT selector for the TSS (offset 0x18).
pub const TSS_SEL: u16 = 0x18;

/// The boot TSS — RSP0 points to the top of the kernel boot stack.
///
/// IST entries are filled by `super::idt::setup_ist()`.
#[unsafe(no_mangle)]
#[unsafe(link_section = ".bss")]
pub static mut BOOT_TSS: Tss = Tss::new();

/// Descriptor for `BOOT_GDT` used by `lgdt`.
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
struct GdtPtr {
    limit: u16,
    base: u64,
}

/// Set up the kernel GDT (with TSS) and load it.
///
/// This replaces the minimal 3-entry boot GDT from `start.S` with a
/// full GDT containing:
///   0x00  Null descriptor
///   0x08  64-bit kernel code
///   0x10  64-bit kernel data
///   0x18  TSS descriptor (16 bytes = 2 GDT entries)
///
/// After loading the GDT, CS is reloaded via a far return, DS/ES/SS
/// are reset to the kernel data selector, and the TSS is loaded via
/// `ltr`.
///
/// # Safety
///
/// Must be called exactly once during early boot, with interrupts
/// disabled, before `setup_idt()` and `enable_interrupts()`.
pub unsafe fn setup_boot_gdt_tss() {
    unsafe extern "C" {
        static __boot_stack: u64;
    }

    unsafe {
        let tss = core::ptr::addr_of_mut!(BOOT_TSS);

        // RSP0 = top of the boot kernel stack.
        (*tss).rsp0 = core::ptr::addr_of!(__boot_stack) as u64;

        // IOPB offset = sizeof(TSS) → no I/O permission bitmap.
        (*tss).iopb_offset = core::mem::size_of::<Tss>() as u16;

        // IST entries are filled by setup_ist() in idt.rs, called
        // right after this function.

        // Build the TSS descriptor (16 bytes, occupies GDT entries 3–4).
        let tss_base = tss as u64;
        let tss_limit =
            (core::mem::size_of::<Tss>() as u32).wrapping_sub(1);

        // Low 8 bytes of TSS descriptor:
        //   limit_lo(16) | base_lo(16) | base_mid(8) | access(8)
        //   | flags_limit_hi(8) | base_hi(8)
        //   access = 0x89 (P=1, DPL=00, Type=0x9 = available 64-bit TSS)
        let tss_lo: u64 = (tss_limit as u64 & 0xFFFF)
            | ((tss_base & 0xFFFF) << 16)
            | (((tss_base >> 16) & 0xFF) << 32)
            | (0x89 << 40)
            | (((tss_base >> 24) & 0xFF) << 56);

        // High 8 bytes of TSS descriptor:
        //   base_upper(32) | reserved(32)
        let tss_hi: u64 = (tss_base >> 32) & 0xFFFF_FFFF;

        // Populate a static 5-entry GDT in .bss:
        //   [0] Null
        //   [1] 64-bit kernel code  (matches start.S encoding)
        //   [2] 64-bit kernel data  (matches start.S encoding)
        //   [3] TSS descriptor low
        //   [4] TSS descriptor high
        #[unsafe(link_section = ".bss")]
        static mut GDT: [u64; 5] = [0u64; 5];

        let gdt = core::ptr::addr_of_mut!(GDT);
        (*gdt)[0] = 0x0000_0000_0000_0000;
        (*gdt)[1] = 0x00AF_9A00_0000_FFFF; // code64
        (*gdt)[2] = 0x00CF_9200_0000_FFFF; // data64
        (*gdt)[3] = tss_lo;
        (*gdt)[4] = tss_hi;

        // Load the new GDT.
        let gdtr = GdtPtr {
            limit: (5 * 8 - 1) as u16, // 5 entries × 8 bytes − 1
            base: gdt as u64,
        };
        asm!("lgdt [{gdtr}]", gdtr = in(reg) &gdtr);

        // Reload data segment selectors to the new GDT's data entry.
        // CS (selector 0x08) has the same code descriptor in both the
        // old boot GDT and the new GDT, so the CPU keeps the cached
        // descriptor — no CS reload needed.
        asm!(
            "mov ds, {ds}",
            "mov es, {ds}",
            "mov ss, {ds}",
            "xor ax, ax",
            "mov fs, ax",
            "mov gs, ax",
            ds = in(reg) KERNEL_DATA_SEL,
        );

        // Load TSS — ltr requires a 16-bit register or memory operand.
        // Pin the input to ax to ensure 16-bit register width.
        asm!(
            "ltr ax",
            in("ax") TSS_SEL,
        );
    }
}
