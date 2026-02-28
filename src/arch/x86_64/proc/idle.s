.att_syntax prefix
.global __idle_start
.global __idle_end

__idle_start:
    # Tight pause loop — ring-3 friendly, no syscall, no stack.
    # The CPU will be preempted by the timer interrupt and rescheduled
    # to a real task when one becomes runnable.
1:  .byte 0xf3, 0x90   # PAUSE (F3 90)
    jmp 1b

__idle_end:
