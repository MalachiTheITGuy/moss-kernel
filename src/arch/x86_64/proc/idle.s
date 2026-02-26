.global __idle_start
.global __idle_end

__idle_start:
1:  hlt
    jmp 1b

__idle_end:
