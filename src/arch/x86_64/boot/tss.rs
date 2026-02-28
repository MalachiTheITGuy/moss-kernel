//! TSS (Task State Segment) and GDT initialisation for x86_64.
//!
//! Sets up a static TSS with:
//!   - RSP0: kernel interrupt stack (used when any ring-3 exception fires)
//!   - IST[0]  (hardware IST1): dedicated stack for the double-fault handler
//!
//! Then replaces the bootstrap assembly GDT (which only had code/data segments)
//! with an equivalent Rust-managed GDT that also includes the TSS descriptor.

use x86_64::VirtAddr;
use x86_64::structures::tss::TaskStateSegment;

// ── per-CPU kernel stacks ────────────────────────────────────────────────────

const KERN_IST_SIZE: usize = 64 * 1024; // 64 KB: ring-0 interrupt stack
const DF_IST_SIZE: usize = 16 * 1024; // 16 KB: double-fault IST

#[repr(align(16))]
struct Stack<const N: usize>([u8; N]);

static KERN_IST: Stack<KERN_IST_SIZE> = Stack([0; KERN_IST_SIZE]);
static DF_IST: Stack<DF_IST_SIZE> = Stack([0; DF_IST_SIZE]);

// ── TSS ─────────────────────────────────────────────────────────────────────

static mut KERN_TSS: TaskStateSegment = TaskStateSegment::new();

// ── GDT ─────────────────────────────────────────────────────────────────────
//
// Entries mirror the assembly GDT in start.s, extended with a 16-byte TSS
// descriptor that occupies two adjacent slots (entries 6 and 7).
//
// Selector map:
//   0x00 – null
//   0x08 – kernel code  (DPL=0, 64-bit)
//   0x10 – kernel data  (DPL=0, writable)
//   0x18 – placeholder  (null)
//   0x23 – user data    (DPL=3, writable)  [index 4, RPL=3]
//   0x2b – user code    (DPL=3, 64-bit)    [index 5, RPL=3]
//   0x30 – TSS low      (filled at runtime, DPL=0)
//   0x38 – TSS high     (filled at runtime)

const GDT_KERNEL_CODE: u64 = (1 << 43) | (1 << 44) | (1 << 47) | (1 << 53);
const GDT_KERNEL_DATA: u64 = (3 << 40) | (1 << 44) | (1 << 47);
const GDT_USER_DATA: u64 = (3 << 40) | (1 << 44) | (1 << 47) | (3 << 45);
const GDT_USER_CODE: u64 = (1 << 43) | (1 << 44) | (1 << 47) | (1 << 53) | (3 << 45);

#[repr(C, align(8))]
struct RawGdt([u64; 8]);

static mut GDT: RawGdt = RawGdt([
    0,              // 0: null
    GDT_KERNEL_CODE, // 1: kernel code   → 0x08
    GDT_KERNEL_DATA, // 2: kernel data   → 0x10
    0,              // 3: placeholder   → 0x18
    GDT_USER_DATA,  // 4: user data     → 0x23
    GDT_USER_CODE,  // 5: user code     → 0x2b
    0,              // 6: TSS low       → 0x30 (set at runtime)
    0,              // 7: TSS high      → 0x38 (set at runtime)
]);

/// LGDT operand: 10-byte memory descriptor for the `lgdt` instruction.
#[repr(C, packed)]
struct GdtPtr {
    limit: u16,
    base: u64,
}

// ── public init function ─────────────────────────────────────────────────────

/// Initialise the TSS and reload the GDT.
///
/// Must be called once, on the boot CPU, before enabling interrupts for
/// user-space.  After this call:
///   - The TSS is valid with RSP0 and IST[0] set.
///   - The GDT contains the TSS descriptor.
///   - The TSS is loaded into the TR register via `ltr`.
///
/// # Safety
/// Accesses mutable statics; must be called exactly once before SMP init.
pub unsafe fn tss_init() {
    // ── Populate the TSS ─────────────────────────────────────────────────────

    let tss = unsafe { &mut *core::ptr::addr_of_mut!(KERN_TSS) };

    // RSP0: kernel stack used when a ring-3 exception/interrupt fires.
    let kern_top = (KERN_IST.0.as_ptr() as u64) + KERN_IST_SIZE as u64;
    tss.privilege_stack_table[0] = VirtAddr::new(kern_top & !0xF);

    // IST[0] (hardware IST1): used by the double-fault handler
    // (exceptions_init calls `.set_stack_index(0)` which maps to IST1).
    let df_top = (DF_IST.0.as_ptr() as u64) + DF_IST_SIZE as u64;
    tss.interrupt_stack_table[0] = VirtAddr::new(df_top & !0xF);

    // ── Build the TSS segment descriptor ────────────────────────────────────

    let tss_base = tss as *const _ as u64;
    let tss_limit = (core::mem::size_of::<TaskStateSegment>() - 1) as u64;

    // Low quad: [limit15:0][base23:0][type=0x9][S=0][DPL=0][P=1][limit19:16][G=0][base31:24]
    let tss_low = (tss_limit & 0xFFFF)
        | ((tss_base & 0x00FF_FFFF) << 16)
        | (0x9u64 << 40)       // Type 9 = 64-bit TSS Available
        | (1u64 << 47)         // Present
        | ((tss_limit >> 16 & 0xF) << 48)
        | ((tss_base >> 24 & 0xFF) << 56);

    // High quad: [base63:32] in bits 31:0; rest reserved/zero.
    let tss_high = tss_base >> 32;

    // ── Write into GDT ───────────────────────────────────────────────────────

    let gdt = unsafe { &mut *core::ptr::addr_of_mut!(GDT) };
    gdt.0[6] = tss_low;
    gdt.0[7] = tss_high;

    // ── Load GDT ─────────────────────────────────────────────────────────────

    let gdt_ptr = GdtPtr {
        limit: (core::mem::size_of::<RawGdt>() - 1) as u16,
        base: gdt as *const _ as u64,
    };

    unsafe {
        core::arch::asm!(
            "lgdt [{ptr}]",
            ptr = in(reg) &gdt_ptr,
            options(nostack, readonly),
        );
    }

    // The kernel code/data segment descriptors carry the same values as the
    // assembly GDT that was loaded in start.s, so no segment-register reload
    // is required.  We only need to load the TSS selector into TR.

    // ── Load TR ──────────────────────────────────────────────────────────────
    // TSS selector = index 6, TI=0, RPL=0 → 0x30.

    unsafe {
        core::arch::asm!(
            "ltr {sel:x}",
            sel = in(reg) 0x30u16,
            options(nostack),
        );
    }
}

/// Return the address of the kernel interrupt stack top (RSP0).
///
/// The context-switch code must call this to update TSS.RSP0 when switching
/// to a new task so the correct kernel stack is used on re-entry.
pub fn kern_ist_top() -> u64 {
    (KERN_IST.0.as_ptr() as u64) + KERN_IST_SIZE as u64 & !0xF
}

/// Update TSS.RSP0 to the given kernel stack top.
///
/// Should be called on every context switch so that exceptions from the
/// newly-scheduled task land on the right kernel stack.
pub unsafe fn set_rsp0(rsp0: u64) {
    unsafe {
        (*core::ptr::addr_of_mut!(KERN_TSS)).privilege_stack_table[0] = VirtAddr::new(rsp0);
    }
}

/// Configure the x86-64 SYSCALL / SYSRET mechanism.
///
/// Must be called **after** `tss_init()` so that the kernel interrupt stack
/// top is already known.
///
/// Sets:
///   - EFER.SCE  (bit 0): enables the `syscall` instruction
///   - STAR      (0xC0000081): kernel CS / SYSRET CS selectors
///   - LSTAR     (0xC0000082): 64-bit SYSCALL entry point (`lstar_entry`)
///   - SFMASK    (0xC0000084): RFLAGS bits to clear on syscall (IF, TF, DF)
///
/// Also primes the assembly-side scratch variables used by `lstar_entry`.
///
/// # Safety
/// Writes MSRs; must be called once before userspace is entered.
pub unsafe fn syscall_init() {
    let stack_top = kern_ist_top();

    // ── Prime the assembly scratch variables ─────────────────────────────────

    // SAFETY: These symbols are defined in trap.s and only touched here and
    // in the lstar_entry assembly stub (with interrupts off).
    unsafe {
        syscall_scratch_rsp = stack_top;
        tss_kern_rsp_top = stack_top;
    }

    // ── Set EFER.SCE ─────────────────────────────────────────────────────────

    unsafe {
        let efer_lo: u32;
        let efer_hi: u32;
        core::arch::asm!(
            "rdmsr",
            in("ecx") 0xC0000080u32,
            out("eax") efer_lo,
            out("edx") efer_hi,
        );
        let efer = (efer_hi as u64) << 32 | efer_lo as u64;
        let new_efer = efer | 1u64; // set SCE (bit 0)
        core::arch::asm!(
            "wrmsr",
            in("ecx") 0xC0000080u32,
            in("eax") (new_efer & 0xFFFF_FFFF) as u32,
            in("edx") (new_efer >> 32) as u32,
        );
    }

    // ── STAR: kernel CS and SYSRET CS base ───────────────────────────────────
    //
    // SYSCALL sets:  CS = STAR[47:32] = 0x08 (kernel code)
    //                SS = STAR[47:32] + 8 = 0x10 (kernel data)
    // SYSRET  sets:  CS = STAR[63:48] + 16 = 0x18 + 16 = 0x28 | 3 = 0x2b (user code)
    //                SS = STAR[63:48] +  8 = 0x18 +  8 = 0x20 | 3 = 0x23 (user data/stack)

    let star: u64 = (0x08u64 << 32) | (0x18u64 << 48);
    unsafe {
        core::arch::asm!(
            "wrmsr",
            in("ecx") 0xC0000081u32,
            in("eax") (star & 0xFFFF_FFFF) as u32,
            in("edx") (star >> 32) as u32,
        );
    }

    // ── LSTAR: 64-bit syscall entry point ────────────────────────────────────

    let lstar = lstar_entry as *const () as u64;
    unsafe {
        core::arch::asm!(
            "wrmsr",
            in("ecx") 0xC0000082u32,
            in("eax") (lstar & 0xFFFF_FFFF) as u32,
            in("edx") (lstar >> 32) as u32,
        );
    }

    // ── SFMASK: RFLAGS bits to clear on SYSCALL ───────────────────────────────
    //
    // Clear IF (9) to prevent interrupts on user stack, TF (8) and DF (10).

    let sfmask: u64 = (1 << 9) | (1 << 8) | (1 << 10);
    unsafe {
        core::arch::asm!(
            "wrmsr",
            in("ecx") 0xC0000084u32,
            in("eax") (sfmask & 0xFFFF_FFFF) as u32,
            in("edx") (sfmask >> 32) as u32,
        );
    }
}

unsafe extern "C" {
    fn lstar_entry();
    static mut syscall_scratch_rsp: u64;
    static mut tss_kern_rsp_top: u64;
}
