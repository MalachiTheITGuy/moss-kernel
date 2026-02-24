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
.endm

.macro POP_REGS
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
    addq $8, %rsp # Skip error code
    iretq

# Exception entry points
.macro EXCEPTION_ERR name, num
.global \name
\name:
    # CPU already pushed error code
    jmp exception_common
.endm

.macro EXCEPTION_NOERR name, num
.global \name
\name:
    pushq $0 # Dummy error code
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
    # x86_64 syscall handler
    # Arguments: RAX = syscall nr, RDI, RSI, RDX, R10, R8, R9
    # Return: RAX
    # CPU has already pushed: RIP, CS, RFLAGS, RSP, SS
    
    # Save general purpose registers
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
    # RAX is already on stack from pushq %rax below
    
    # Save RFLAGS using pushfq (push flags)
    pushfq
    popq %r11
    
    # Save original syscall number
    movq %rax, %r15
    
    # Call the Rust syscall handler
    # Arguments: rdi = syscall nr, rsi = arg1, rdx = arg2, r10 = arg3, r8 = arg4, r9 = arg5
    movq %r15, %rdi  # syscall number
    movq %rsi, %rsi  # arg1
    movq %rdx, %rdx  # arg2
    movq %r10, %r10  # arg3 (note: r10 not rcx!)
    movq %r8, %r8    # arg4
    movq %r9, %r9    # arg5
    
    callq x86_64_syscall_handler
    
    # Return value is in RAX
    
    # Restore registers in reverse order
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
    
    # Restore RFLAGS from saved value
    pushq %r11
    popfq
    
    # Skip the saved RIP and CS that CPU pushed
    addq $16, %rsp
    
    # Return to userspace - CPU will pop RSP, SS, RFLAGS, RIP, CS
    iretq
