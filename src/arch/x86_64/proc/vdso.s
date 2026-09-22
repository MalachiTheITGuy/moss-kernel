.section .vdso, "ax"
.global __vdso_sig_return
.code64

__vdso_sig_return:
    movq $0x8b, %rax
    syscall
