.global __idle_start
.global __idle_end

__idle_start:
1:  .byte 0xf3, 0x90   # pause (ring-3 friendly spin hint)
    movl $24, %eax     # sys_sched_yield (x86_64 nr=24)
    syscall
    jmp 1b

__idle_end:
