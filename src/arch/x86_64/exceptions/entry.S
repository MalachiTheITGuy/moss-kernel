/*
 * x86_64 interrupt / exception entry stubs.
 *
 * Each of the 256 IDT vectors has a tiny assembly stub that:
 *
 *   1. Pushes a dummy error code (0) for exceptions that don't
 *      architecturally push one, so the stack frame is uniform.
 *   2. Pushes the vector number.
 *   3. Jumps to `interrupt_common` which saves every GP register
 *      and calls the Rust handler.
 *
 * The stack layout on entry to interrupt_common is:
 *
 *   [SS]          ← pushed by CPU on privilege-level change
 *   [RSP]         ← pushed by CPU on privilege-level change
 *   [RFLAGS]      ← pushed by CPU
 *   [CS]          ← pushed by CPU on privilege-level change
 *   [RIP]         ← pushed by CPU
 *   [error_code]  ← pushed by CPU or by stub (dummy 0)
 *   [vector_num]  ← pushed by stub
 *
 * After common handler saves GP registers, RSP points to the
 * ExceptionState struct which is passed to the Rust handler.
 *
 * Naming convention: `isr_stub_N` where N is the vector number.
 * These symbols are referenced from Rust via `extern` declarations.
 *
 * Linked into .text via the .text.isr section so the IDT entries
 * can reference them as 64-bit addresses.
 */

.section .text, "ax", @progbits

/* =================================================================
 * Macros for entry stubs
 * ================================================================= */

/*
 * ISR_NOERR — for exceptions that do NOT push an error code.
 * Pushes a dummy 0 then the vector number, then jumps to common.
 */
.macro ISR_NOERR num
.global isr_stub_\num
isr_stub_\num:
    pushq   $0              /* dummy error code */
    pushq   $\num           /* vector number    */
    jmp     interrupt_common
.endm

/*
 * ISR_ERR — for exceptions that DO push an error code.
 * The CPU has already placed the error code on the stack.
 * We just push the vector number.
 */
.macro ISR_ERR num
.global isr_stub_\num
isr_stub_\num:
    pushq   $\num           /* vector number    */
    jmp     interrupt_common
.endm

/* =================================================================
 * Vectors 0–31: CPU exceptions
 * ================================================================= */

ISR_NOERR  0   /* #DE  Divide-by-zero Error            */
ISR_NOERR  1   /* #DB  Debug / Breakpoint              */
ISR_NOERR  2   /*      Non-Maskable Interrupt (NMI)    */
ISR_NOERR  3   /* #BP  Breakpoint (INT3)               */
ISR_NOERR  4   /* #OF  Overflow (INTO)                 */
ISR_NOERR  5   /* #BR  Bound Range Exceeded            */
ISR_NOERR  6   /* #UD  Invalid Opcode                  */
ISR_NOERR  7   /* #NM  Device Not Available (FPU)      */
ISR_ERR    8   /* #DF  Double Fault                    */
ISR_NOERR  9   /*      Coprocessor Segment (reserved)  */
ISR_ERR   10   /* #TS  Invalid TSS                     */
ISR_ERR   11   /* #NP  Segment Not Present              */
ISR_ERR   12   /* #SS  Stack-Segment Fault              */
ISR_ERR   13   /* #GP  General Protection Fault         */
ISR_ERR   14   /* #PF  Page Fault                      */
ISR_NOERR 15   /*      Reserved                        */
ISR_NOERR 16   /* #MF  x87 FPU Floating-Point Error    */
ISR_ERR   17   /* #AC  Alignment Check                 */
ISR_NOERR 18   /* #MC  Machine Check                   */
ISR_NOERR 19   /* #XM  SIMD Floating-Point Exception   */
ISR_NOERR 20   /* #VE  Virtualisation Exception        */
ISR_ERR   21   /* #CP  Control Protection Exception    */
ISR_NOERR 22
ISR_NOERR 23
ISR_NOERR 24
ISR_NOERR 25
ISR_NOERR 26
ISR_NOERR 27
ISR_NOERR 28
ISR_NOERR 29   /* #SR  Security Exception              */
ISR_ERR   30   /* #SX  Shadow Stack Fault              */
ISR_NOERR 31

/* =================================================================
 * Vectors 32–255: hardware IRQs and software interrupts
 * No error codes — all use dummy 0.
 * ================================================================= */

.set vec, 32
.rept 224
    ISR_NOERR vec
    .set vec, vec + 1
.endr

/* =================================================================
 * Global symbol table — generated from the .rept above.
 * We export the base address so Rust can look up individual stubs
 * as `isr_base + vector * size` (all stubs are identical in size
 * for NOERR; ERR stubs are 1 byte shorter, so we export each
 * explicitly).
 * ================================================================= */

/*
 * interrupt_common — save registers, call Rust handler, restore.
 *
 * On entry the stack is:
 *
 *   [ss]        (optional — ring-3 only)
 *   [rsp]       (optional — ring-3 only)
 *   [rflags]
 *   [cs]        (optional — ring-3 only)
 *   [rip]
 *   [err_code]
 *   [vector]
 *
 * We build the ExceptionState by pushing all GP registers so that
 * the Rust handler receives a pointer to a complete register frame.
 *
 * ExceptionState layout (matches Rust #[repr(C)] struct):
 *   [0x00] r15       ← RSP on entry to Rust handler
 *   [0x08] r14
 *   [0x10] r13
 *   [0x18] r12
 *   [0x20] rbp
 *   [0x28] rbx
 *   [0x30] r11
 *   [0x38] r10
 *   [0x40] r9
 *   [0x48] r8
 *   [0x50] rax
 *   [0x58] rcx
 *   [0x60] rdx
 *   [0x68] rsi
 *   [0x70] rdi
 *   [0x78] vector    ← the stub's pushq $\num lands here
 *   [0x80] err_code
 *   [0x88] rip
 *   [0x90] cs
 *   [0x98] rflags
 *   [0xA0] rsp
 *   [0xA8] ss
 */
.global interrupt_common
.type interrupt_common, @function
interrupt_common:
    /* ---- Save general-purpose registers ---- */
    pushq   %r15
    pushq   %r14
    pushq   %r13
    pushq   %r12
    pushq   %rbp
    pushq   %rbx
    pushq   %r11
    pushq   %r10
    pushq   %r9
    pushq   %r8
    pushq   %rax
    pushq   %rcx
    pushq   %rdx
    pushq   %rsi
    pushq   %rdi

    /* ---- First argument to Rust handler = &ExceptionState ---- */
    movq    %rsp, %rdi

    /* ---- Call the Rust exception / IRQ dispatcher ---- */
    call    x86_64_interrupt_handler

    /* ---- Restore general-purpose registers ---- */
    popq    %rdi
    popq    %rsi
    popq    %rdx
    popq    %rcx
    popq    %rax
    popq    %r8
    popq    %r9
    popq    %r10
    popq    %r11
    popq    %rbx
    popq    %rbp
    popq    %r12
    popq    %r13
    popq    %r14
    popq    %r15

    /* ---- Drop vector number and error code ---- */
    addq    $16, %rsp

    /* ---- Return from interrupt ---- */
    iretq
