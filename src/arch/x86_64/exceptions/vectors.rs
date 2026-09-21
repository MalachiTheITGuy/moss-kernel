//! x86_64 exception and interrupt vector numbers.
//!
//! These correspond to the ISR entries populated in the IDT and the
//! stubs defined in `entry.S`.

/// Double Fault (#DF) — always uses IST1.
pub const DOUBLE_FAULT: usize = 8;

/// Invalid TSS (#TS).
pub const INVALID_TSS: usize = 10;

/// Segment Not Present (#NP).
pub const SEGMENT_NOT_PRESENT: usize = 11;

/// Stack-Segment Fault (#SS).
pub const STACK_SEGMENT_FAULT: usize = 12;

/// General Protection Fault (#GP).
pub const GENERAL_PROTECTION_FAULT: usize = 13;

/// Page Fault (#PF).
pub const PAGE_FAULT: usize = 14;

/// x87 FPU Error (#MF).
pub const X87_FPU_ERROR: usize = 16;

/// Alignment Check (#AC).
pub const ALIGNMENT_CHECK: usize = 17;

/// Machine Check (#MC) — always uses IST3.
pub const MACHINE_CHECK: usize = 18;

/// SIMD Floating-Point Exception (#XM).
pub const SIMD_FLOAT_EXCEPTION: usize = 19;

/// Virtualization Exception (#VE).
pub const VIRTUALIZATION_EXCEPTION: usize = 20;

/// First user-defined (hardware IRQ) vector.
///
/// All vectors >= [`IRQ_BASE`] are routed through the root interrupt
/// controller (`InterruptManager`).
pub const IRQ_BASE: usize = 32;

/// Number of CPU exception vectors (0–31).
pub const NUM_EXCEPTION_VECTORS: usize = 32;

/// Software interrupt vector used for SYSCALL entry.
///
/// When user space executes `syscall`, the entry trampoline pushes
/// this vector number so the dispatcher can route it to the syscall
/// handler.  This is not a hardware-defined vector — it is chosen
/// by the kernel to avoid collisions with CPU exceptions (0–31) and
/// hardware IRQs (32–255).  We use 0x80 to mirror the Linux
/// convention for the legacy `int 0x80` path.
pub const SYSCALL: usize = 0x80;

/// Total number of IDT entries.
pub const NUM_IDT_ENTRIES: usize = 256;
