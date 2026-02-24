.section .text

.macro PUSH_REGS
    push %rax
    push %rcx
    push %rdx
    push %rbx
    push %rbp
    push %rsi
    push %rdi
    push %r8
    push %r9
    push %r10
    push %r11
    push %r12
    push %r13
    push %r14
    push %r15
.endm

.macro POP_REGS
    pop %r15
    pop %r14
    pop %r13
    pop %r12
    pop %r11
    pop %r10
    pop %r9
    pop %r8
    pop %rdi
    pop %rsi
    pop %rbp
    pop %rbx
    pop %rdx
    pop %rcx
    pop %rax
.endm

# Common exception handler
exception_common:
    PUSH_REGS
    
    # Save GS/FS base would go here if we used it in ExceptionState
    
    mov %rsp, %rdi # Arg 1: *mut ExceptionState
    call x86_64_exception_handler
    
    # x86_64_exception_handler might return a new stack pointer for context switch
    mov %rax, %rsp
    
    POP_REGS
    add $8, %rsp # Skip error code
    iretq

# Exception entry points
.macro EXCEPTION_ERR name, num
.global \name
\name:
    # Error code already pushed by CPU
    push $\num # Just to be sure we have the number? No, usually we push it if CPU doesn't.
    # Wait, if CPU pushes error code, we don't push it.
    jmp exception_common
.endm

.macro EXCEPTION_NOERR name, num
.global \name
\name:
    push $0 # Dummy error code
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
    # Syscall uses 'syscall' instruction on x86_64, which jumps to LSTAR MSR.
    # It doesn't push as much state as an interrupt.
    # But for now, let's stub it or use an interrupt for early porting if easier.
    # Linux uses 'int 0x80' for 32-bit and 'syscall' for 64-bit.
    hlt
