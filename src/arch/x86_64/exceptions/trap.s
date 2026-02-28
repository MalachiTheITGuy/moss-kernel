.section .text
.code64

# Use AT&T syntax explicitly
.att_syntax prefix

.macro PUSH_REGS
    pushq %r15
    pushq %r14
    pushq %r13
    pushq %r12
    pushq %r11
    pushq %r10
    pushq %r9
    pushq %r8
    pushq %rdi
    pushq %rsi
    pushq %rbp
    pushq %rbx
    pushq %rdx
    pushq %rcx
    pushq %rax
    pushq $0    # fs_base slot (populated by Rust handler from IA32_FS_BASE MSR)
.endm

.macro POP_REGS
    addq $8, %rsp   # skip fs_base (Rust handler writes it to IA32_FS_BASE MSR)
    popq %rax
    popq %rcx
    popq %rdx
    popq %rbx
    popq %rbp
    popq %rsi
    popq %rdi
    popq %r8
    popq %r9
    popq %r10
    popq %r11
    popq %r12
    popq %r13
    popq %r14
    popq %r15
.endm

# Common exception handler
exception_common:
    PUSH_REGS
    
    # Arg 1: *mut ExceptionState (RSP)
    movq %rsp, %rdi
    callq x86_64_exception_handler
    
    # Return value is the new stack pointer
    movq %rax, %rsp
    
    POP_REGS
    addq $16, %rsp # Skip vector and error code
    iretq

# Exception entry points
.macro EXCEPTION_ERR name, num
.global \name
\name:
    # CPU already pushed error code
    pushq $\num # Push vector
    jmp exception_common
.endm

.macro EXCEPTION_NOERR name, num
.global \name
\name:
    pushq $0 # Dummy error code
    pushq $\num # Push vector
    jmp exception_common
.endm

# Define common exceptions
EXCEPTION_NOERR exc_divide_by_zero, 0
EXCEPTION_NOERR exc_debug, 1
EXCEPTION_NOERR exc_nmi, 2
EXCEPTION_NOERR exc_breakpoint, 3
EXCEPTION_NOERR exc_overflow, 4
EXCEPTION_NOERR exc_bound_range_exceeded, 5
EXCEPTION_NOERR exc_invalid_opcode, 6
EXCEPTION_NOERR exc_device_not_available, 7
EXCEPTION_ERR   exc_double_fault, 8
EXCEPTION_ERR   exc_invalid_tss, 10
EXCEPTION_ERR   exc_segment_not_present, 11
EXCEPTION_ERR   exc_stack_segment_fault, 12
EXCEPTION_ERR   exc_general_protection_fault, 13
EXCEPTION_ERR   exc_page_fault, 14
EXCEPTION_NOERR exc_x87_floating_point, 16
EXCEPTION_ERR   exc_alignment_check, 17
EXCEPTION_NOERR exc_machine_check, 18
EXCEPTION_NOERR exc_simd_floating_point, 19
EXCEPTION_NOERR exc_virtualization, 20
EXCEPTION_ERR   exc_control_protection, 21
EXCEPTION_NOERR exc_hypervisor_injection, 28
EXCEPTION_ERR   exc_vmm_communication, 29
EXCEPTION_ERR   exc_security, 30

.global exc_syscall
exc_syscall:
    pushq $0 # Dummy error code
    pushq $0x80 # Use 0x80 as vector for syscall (standard)
    PUSH_REGS
    
    movq %rsp, %rdi
    callq x86_64_syscall_handler
    
    movq %rax, %rsp
    
    POP_REGS
    addq $16, %rsp
    iretq

# IRQ entry points
.macro IRQ_ENTRY name, num
.global \name
\name:
    pushq $0 # Dummy error code
    pushq $\num
    jmp exception_common
.endm

IRQ_ENTRY exc_timer, 0x20
IRQ_ENTRY exc_com1, 0x24

# ── SYSCALL fast-path entry ──────────────────────────────────────────────────
#
# On entry from userspace `syscall` instruction:
#   RCX = saved user RIP (return address)
#   R11 = saved user RFLAGS
#   RSP = still the user RSP (SYSCALL does NOT switch stacks)
#   CS = kernel CS (set by STAR[47:32])
#   SS = kernel SS (set by STAR[47:32] + 8)
#   IF = 0  (cleared by SFMASK)
#
# We need to build the same ExceptionState frame that exception_common uses,
# then call x86_64_syscall_handler, and return via iretq.
#
# The trick for saving user RSP without clobbering any register before PUSH_REGS:
#   - We swap RSP with a static "kernel RSP top" pointer using `xchg`.
#     After the xchg: RSP = kernel stack top, static = user RSP.
#   - We then push the static value (user RSP) onto the kernel stack as part
#     of the fake exception frame.
#
# .bss scratch variables (see below):
#   syscall_scratch_rsp: holds the kernel stack top between calls;
#                        temporarily holds user RSP during the prologue.
.global lstar_entry
lstar_entry:
    # Atomically swap RSP (user) with the kernel stack top pointer.
    # After: RSP = kernel stack top, syscall_scratch_rsp = user RSP.
    xchgq %rsp, syscall_scratch_rsp(%rip)

    # Build the 5-word iretq frame + error_code + vector on the kernel stack.
    pushq $0x23                             # SS  = USER_SS
    pushq syscall_scratch_rsp(%rip)         # RSP = user RSP (pushed from scratch)
    pushq %r11                              # RFLAGS (saved by SYSCALL in R11)
    pushq $0x2b                             # CS  = USER_CS
    pushq %rcx                              # RIP (saved by SYSCALL in RCX)
    pushq $0                                # error_code (none for syscall)
    pushq $0x80                             # vector 0x80 = syscall

    PUSH_REGS                               # push r15..rax + fs_base slot

    movq %rsp, %rdi
    callq x86_64_syscall_handler            # returns new ExceptionState* in RAX

    movq %rax, %rsp
    POP_REGS
    addq $16, %rsp                          # skip vector and error_code

    # Restore kernel stack top pointer for the next syscall.
    # The kernel RSP at the entry of the NEXT syscall must be the same top.
    # We hijacked syscall_scratch_rsp; restore it from the tss_kern_rsp_top var.
    movq tss_kern_rsp_top(%rip), %r11       # clobber R11 (will be set by iretq RFLAGS)
    movq %r11, syscall_scratch_rsp(%rip)

    iretq

# ── Boot-time userspace entry trampoline ────────────────────────────────────
# Given a pointer to a fully-populated ExceptionState (same struct layout used
# by exception_common), restore all registers and execute iretq to jump to
# userspace for the very first time.
#
# Calling convention: rdi = *const ExceptionState
# Never returns.
.global boot_jump_to_userspace
boot_jump_to_userspace:
    # Write IA32_FS_BASE MSR (0xC0000100) from ctx->fs_base (offset 0).
    movl  (%rdi), %eax
    movl  4(%rdi), %edx
    movl  $0xC0000100, %ecx
    wrmsr

    # Point RSP at the start of the ExceptionState (the fs_base field).
    # From here we replicate what exception_common does after the handler call.
    movq  %rdi, %rsp

    # POP_REGS: skip fs_base, then pop all GPRs (rax..r15).
    addq  $8, %rsp      # skip fs_base
    popq  %rax
    popq  %rcx
    popq  %rdx
    popq  %rbx
    popq  %rbp
    popq  %rsi
    popq  %rdi
    popq  %r8
    popq  %r9
    popq  %r10
    popq  %r11
    popq  %r12
    popq  %r13
    popq  %r14
    popq  %r15

    # Skip the vector and error_code fields.
    addq  $16, %rsp

    # iretq: CPU pops rip, cs, rflags, rsp, ss → jumps to userspace.
    iretq

# ── SYSCALL scratch data ─────────────────────────────────────────────────────
#
# syscall_scratch_rsp: initially set to the kernel interrupt stack top by
#   the boot code; temporarily holds the user RSP during syscall entry.
#
# tss_kern_rsp_top: permanent copy of the kernel stack top, read-only after
#   boot, used to re-prime syscall_scratch_rsp after each syscall.
.section .data
.align 8
.global syscall_scratch_rsp
syscall_scratch_rsp:
    .quad 0

.global tss_kern_rsp_top
tss_kern_rsp_top:
    .quad 0
