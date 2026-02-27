.macro PUSH_ALL
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

.macro POP_ALL
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

.extern x86_64_exception_handler

common_exception_handler:
    PUSH_ALL
    mov %rsp, %rdi
    call x86_64_exception_handler
    mov %rax, %rsp
    POP_ALL
    add $8, %rsp
    iretq

.macro ISR_NOERR name, num
.global \name
\name:
    pushq $0
    pushq $\num
    jmp common_exception_handler
.endm

.macro ISR_ERR name, num
.global \name
\name:
    pushq $\num
    jmp common_exception_handler
.endm

ISR_NOERR isr0, 0
ISR_NOERR isr1, 1
ISR_NOERR isr2, 2
ISR_NOERR isr3, 3
ISR_NOERR isr4, 4
ISR_NOERR isr5, 5
ISR_NOERR isr6, 6
ISR_NOERR isr7, 7
ISR_ERR   isr8, 8
ISR_NOERR isr9, 9
ISR_ERR   isr10, 10
ISR_ERR   isr11, 11
ISR_ERR   isr12, 12
ISR_ERR   isr13, 13
ISR_ERR   isr14, 14
ISR_NOERR isr15, 15
ISR_NOERR isr16, 16
ISR_ERR   isr17, 17
ISR_NOERR isr18, 18
ISR_NOERR isr19, 19
ISR_NOERR isr20, 20
ISR_ERR   isr21, 21
ISR_NOERR isr22, 22
ISR_NOERR isr23, 23
ISR_NOERR isr24, 24
ISR_NOERR isr25, 25
ISR_NOERR isr26, 26
ISR_NOERR isr27, 27
ISR_NOERR isr28, 28
ISR_ERR   isr29, 29
ISR_ERR   isr30, 30
ISR_NOERR isr31, 31
