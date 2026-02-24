# Multiboot2 header
.section .text.boot
.code32
.align 8
multiboot_header:
    .long 0xe85250d6                # magic number (multiboot 2)
    .long 0                         # architecture 0 (protected mode i386)
    .long multiboot_header_end - multiboot_header # header length
    # checksum
    .long 0x100000000 - (0xe85250d6 + 0 + (multiboot_header_end - multiboot_header))

    # tags here if needed

    .short 0    # type
    .short 0    # flags
    .long 8     # size
multiboot_header_end:

.global _start
_start:
    cli
    cld

    # Update stack pointer (physical)
    mov $(__boot_stack - 0xffffffff80000000), %esp

    # Check if we were booted by Multiboot2
    cmp $0x36d76289, %eax
    jne .error

    # Bootstrapping page tables
    # PML4[0] -> boot_pdpt (Identity)
    # PML4[511] -> boot_pdpt (Higher half 0xffffffff80000000)
    
    mov $(boot_pml4 - 0xffffffff80000000), %edi
    mov $(boot_pdpt - 0xffffffff80000000), %eax
    or $0x3, %eax # Present + Writable
    mov %eax, (%edi)
    mov %eax, 4088(%edi) # 511 * 8

    # PDPT[510] -> boot_pd (0xffffffff80000000)
    # PDPT[0] -> boot_pd (Identity)
    mov $(boot_pdpt - 0xffffffff80000000), %edi
    mov $(boot_pd - 0xffffffff80000000), %eax
    or $0x3, %eax
    mov %eax, (%edi)
    mov %eax, 4080(%edi) # 510 * 8

    # PD[0] -> 0 (2MiB huge page)
    mov $(boot_pd - 0xffffffff80000000), %edi
    mov $0x00000083, %eax # Present + Writable + Huge
    mov %eax, (%edi)

    # Load CR3
    mov $(boot_pml4 - 0xffffffff80000000), %eax
    mov %eax, %cr3

    # Enable PAE
    mov %cr4, %eax
    or $(1 << 5), %eax
    mov %eax, %cr4

    # Enable Long Mode in EFER MSR
    mov $0xc0000080, %ecx
    rdmsr
    or $(1 << 8), %eax
    wrmsr

    # Enable Paging
    mov %cr0, %eax
    or $(1 << 31), %eax
    mov %eax, %cr0

    # Load 64-bit GDT
    lgdt (gdt64_ptr - 0xffffffff80000000)

    # Far jump to 64-bit code
    ljmp $0x8, $(.long_mode - 0xffffffff80000000)

.error:
    # Just hang
    hlt
    jmp .error

.code64
.long_mode:
    # Clear segment registers
    mov $0, %ax
    mov %ax, %ds
    mov %ax, %es
    mov %ax, %fs
    mov %ax, %gs
    mov %ax, %ss

    # Load higher half stack
    mov $__boot_stack, %rsp

    # Jump to Rust arch_init_stage1
    # Arguments: rdi = mb_info_ptr, rsi = image_start, rdx = image_end
    mov %ebx, %edi
    mov $__image_start, %rsi
    mov $__image_end, %rdx
    call arch_init_stage1

    # Should not return
    hlt

.section .rodata
.align 8
gdt64:
    .quad 0 # Null
    .quad (1 << 43) | (1 << 44) | (1 << 47) | (1 << 53) # Code (exec/read, user=0, present, 64-bit)
gdt64_ptr:
    .short . - gdt64 - 1
    .quad gdt64 - 0xffffffff80000000

.section .bss
.align 4096
boot_pml4:
    .fill 4096, 1, 0
boot_pdpt:
    .fill 4096, 1, 0
boot_pd:
    .fill 4096, 1, 0
