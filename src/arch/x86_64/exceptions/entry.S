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

.altmacro
.section .text, "ax", @progbits

/* =================================================================
 * Macros for entry stubs — uses \num text substitution for macro
 * arguments, and %expr arithmetic in .irp/.rept contexts.
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
    ISR_NOERR %vec
    .set vec, vec + 1
.endr

/* =================================================================
 * ISR stub address table — referenced by Rust code (idt.rs).
 * Each entry is the 64-bit address of the corresponding isr_stub_N
 * symbol.  The table lives in .rodata.isr_stubs so that the linker
 * can place it near the stubs themselves.
 * ================================================================= */
.macro isr_table_entry n
    .quad isr_stub_\n
.endm

.section .rodata.isr_stubs, "a", @progbits
.global isr_stub_table
.balign 8
isr_stub_table:
.set vec, 0
.rept 256
    isr_table_entry %vec
    .set vec, vec + 1
.endr

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

/* =================================================================
 * syscall_entry — SYSCALL instruction handler (LSTAR target)
 *
 * The SYSCALL instruction (AMD64 Syscall extension) transfers
 * control from ring 3 to ring 0 with these side-effects:
 *
 *   RCX  ← user RIP        (return address)
 *   R11  ← user RFLAGS     (saved flags)
 *   CS   ← kernel CS       (from STAR MSR bits 47:32)
 *   SS   ← kernel SS       (from STAR MSR bits 47:32 + 8)
 *   RIP  ← LSTAR           (this function)
 *   RFLAGS.IF ← 0          (masked by FMASK)
 *   RSP  ← unchanged       (still user RSP!)
 *
 * RSP is NOT changed, so we must switch stacks manually.
 * We build an ExceptionState frame identical to the one built
 * by interrupt_common so the Rust dispatcher handles syscalls
 * uniformly alongside hardware interrupts.
 *
 * SYSRETQ returns to ring 3 using:
 *   RIP ← RCX   (from ExceptionState.rcx, which holds user RIP)
 *   RFLAGS ← R11 (from ExceptionState.r11, which holds user RFLAGS)
 *   CS, SS ← STAR MSR (user segments)
 *   RSP is NOT restored by SYSRETQ — we must set it explicitly.
 *
 * Per-CPU kernel stacks are not yet available (TODO(#17)).
 * A static 8 KiB stack in .bss is used as a temporary measure.
 * ================================================================= */

/* ---- Static kernel stack for syscall entry ---- */
.section .bss, "aw", @nobits
.align 16
syscall_kernel_stack:
    .skip 8192  /* 8 KiB — sufficient for syscall handling */
syscall_kernel_stack_top:

/* ---- Save slot for user RSP (handler may modify it for signals) ---- */
.section .data, "aw", @progbits
.align 8
syscall_user_rsp_save:
    .quad 0

/* ---- Standard x86_64 user-mode segment selectors ----
 * These match the Linux ABI GDT layout:
 *   0x33 = user CS (entry 6, DPL=3: index 6 << 3 | RPL 3)
 *   0x2B = user SS (entry 5, DPL=3: index 5 << 3 | RPL 3) */
.set USER_CS,  0x33
.set USER_SS,  0x2B

/* ---- Syscall vector number (must match vectors::SYSCALL) ---- */
.set SYSCALL_VECTOR, 0x80

.section .text, "ax", @progbits
.global syscall_entry
.type syscall_entry, @function
syscall_entry:
    /* ---- 1. Switch to kernel GS base ---- */
    swapgs

    /* ---- 2. Save user RSP to static slot ----
     * We cannot use per-CPU data yet, so a static variable in .data
     * serves as the save location.  On exit the handler may have
     * updated ExceptionState.rsp (e.g. for signal delivery), so we
     * re-read it from the frame rather than using this saved copy. */
    movq    %rsp, (syscall_user_rsp_save)

    /* ---- 3. Load kernel stack ---- */
    movq    $syscall_kernel_stack_top, %rsp

    /* ---- 4. Build ExceptionState frame ----
     *
     * The frame must match the layout expected by the Rust handler
     * (see ExceptionState #[repr(C)] in exceptions/mod.rs).
     * We push in bottom-up order: first push = highest address.
     *
     * Frame layout (high → low addresses):
     *   [0xA8] ss
     *   [0xA0] rsp       ← user RSP
     *   [0x98] rflags    ← user RFLAGS (from R11)
     *   [0x90] cs
     *   [0x88] rip       ← user RIP (from RCX)
     *   [0x80] error_code (0 — no error code for SYSCALL)
     *   [0x78] vector_num (0x80)
     *   [0x70] rdi       ← GP register save area begins
     *   ...
     *   [0x00] r15
     */

    /* ---- CPU-equivalent fields (normally pushed by CPU) ---- */
    pushq   $USER_SS           /* [0xA8] ss  = user data segment       */
    pushq   %rsp               /* placeholder — overwritten below      */
    /* Above push decrements RSP by 8 before storing, so the value
     * on the stack is the pre-push RSP.  We overwrite it with the
     * real user RSP which we saved before switching stacks.        */
    movq    (syscall_user_rsp_save), %rax
    movq    %rax, 8(%rsp)      /* [0xA0] rsp = user stack pointer       */
    pushq   %r11               /* [0x98] rflags = user flags (SYSCALL)  */
    pushq   $USER_CS           /* [0x90] cs  = user code segment        */
    pushq   %rcx               /* [0x88] rip = user return addr (SYSCALL)*/
    pushq   $0                 /* [0x80] error_code = 0                  */
    pushq   $SYSCALL_VECTOR    /* [0x78] vector_num = 0x80              */

    /* ---- General-purpose registers (same order as interrupt_common) */
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

    /* ---- 5. Call the Rust exception dispatcher ----
     * First argument (RDI) = pointer to ExceptionState on stack. */
    movq    %rsp, %rdi
    call    x86_64_interrupt_handler

    /* ---- 6. Exit path — restore user state ----
     *
     * After the handler returns, the ExceptionState on the kernel
     * stack may have been modified (e.g. signal delivery changes
     * rip, rflags, and rsp).  We must use the potentially-modified
     * values.
     *
     * SYSRETQ requires:
     *   RCX = user RIP   (from ExceptionState.rcx at frame offset 0x10)
     *   R11 = user RFLAGS (from ExceptionState.r11 at frame offset 0x20)
     *   RSP = user RSP   (from ExceptionState.rsp  at frame offset 0x28)
     *
     * We cannot change RSP while reading from the kernel stack, so
     * we use a scratch slot in .bss to hold the user RSP across the
     * GP register restore.  RAX is temporarily clobbered but will be
     * restored from the ExceptionState before we return.             */

    /* Save user RSP (possibly modified by handler) to scratch slot */
    movq    0x28(%rsp), %rax   /* RAX = ExceptionState.rsp              */
    movq    %rax, (syscall_user_rsp_save)

    /* ---- 7. Restore general-purpose registers ---- */
    popq    %rdi
    popq    %rsi
    popq    %rdx
    popq    %rcx               /* RCX = user RIP (for SYSRETQ)          */
    popq    %rax               /* RAX = user RAX / syscall return value */
    popq    %r8
    popq    %r9
    popq    %r10
    popq    %r11               /* R11 = user RFLAGS (for SYSRETQ)       */
    popq    %rbx
    popq    %rbp
    popq    %r12
    popq    %r13
    popq    %r14
    popq    %r15

    /* ---- 8. Drop frame (7 qwords = 56 bytes) ---- */
    addq    $56, %rsp

    /* ---- 9. Restore user RSP ---- */
    movq    (syscall_user_rsp_save), %rsp

    /* ---- 10. Switch back to user GS base ---- */
    swapgs

    /* ---- 11. Return to ring 3 ----
     * SYSRETQ loads:
     *   CS  ← STAR[31:16] | 3   (user code segment)
     *   SS  ← STAR[31:16] + 8 | 3 (user data segment)
     *   RIP ← RCX               (user return address)
     *   RFLAGS ← R11            (user flags)
     *   CPL  ← 3                (ring 3)
     * RSP is left unchanged — it already holds the user value. */
    sysretq
