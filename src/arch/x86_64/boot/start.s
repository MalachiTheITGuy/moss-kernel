# Multiboot2 header
.section .text.boot
.code32
.align 8
multiboot_header:
    .long 0xe85250d6                # magic number (multiboot2)
    .long 0                         # architecture (i386 protected mode)
    .long multiboot_header_end - multiboot_header # header length
    .long -(0xe85250d6 + 0 + (multiboot_header_end - multiboot_header)) # checksum
    # Information request tag
    .short 1                        # type
    .short 0                        # flags
    .long 12                        # size
    .long 1                         # memory map
    # End tag
    .short 0                        # type
    .short 0                        # flags
    .long 8                         # size
multiboot_header_end:

.section .note.PVH, "a"
.align 4
    .long 4             # Name size
    .long 4             # Desc size
    .long 18            # Type: ELF_NOTE_PVH_BOOT (18)
    .ascii "Xen\0"      # Name
    .long _start - 0xffffffff80000000 # Entry point (physical)

.section .text.boot
.code32
.global _start
_start:
    cli
    cld

    # Early serial debug: write '!' to COM1
    mov $0x3F8, %dx
    mov $0x21, %al
    out %al, %dx

    # Update stack pointer (physical)
    mov $(__boot_stack - 0xffffffff80000000), %esp

    # Debug: write '@' after setting stack
    mov $0x3F8, %dx
    mov $0x40, %al
    out %al, %dx

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

    # Map first 1GiB using 512 x 2MiB huge pages in boot_pd
    # This covers kernel image + BSS + boot stack + multiboot info
    mov $(boot_pd - 0xffffffff80000000), %edi
    mov $0x00000083, %eax  # Present + Writable + Huge (2MiB)
    mov $512, %ecx          # 512 entries = 1GiB
.fill_pd:
    mov %eax, (%edi)
    add $0x200000, %eax     # Next 2MiB physical page
    add $8, %edi            # Next PD entry
    dec %ecx
    jnz .fill_pd

    # Debug: write '&' after filling PD
    mov $0x3F8, %dx
    mov $0x26, %al
    out %al, %dx

    # Load CR3
    mov $(boot_pml4 - 0xffffffff80000000), %eax
    mov %eax, %cr3

    # Debug: write '*' to check if CR3 load reached
    mov $0x3F8, %dx
    mov $0x2A, %al
    out %al, %dx

    # Marker '#'
    mov $0x3F8, %dx
    mov $0x23, %al
    out %al, %dx

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

    # Debug: write '^' before ljmp
    mov $0x3F8, %dx
    mov $0x5E, %al
    out %al, %dx

    # Far jump to 64-bit code
    ljmp $0x8, $(.long_mode - 0xffffffff80000000)

    # Marker '$'
    mov $0x3F8, %dx
    mov $0x24, %al
    out %al, %dx

.error:
    # Just hang
    hlt
    jmp .error

.code64
.long_mode:
    # Debug: write '+' to confirm long mode reached
    mov $0x3F8, %dx
    mov $0x2B, %al
    out %al, %dx

    # Load kernel data selector into all data segment registers
    mov $0x10, %ax
    mov %ax, %ds
    mov %ax, %es
    mov %ax, %fs
    mov %ax, %gs
    mov %ax, %ss

    # Load higher half stack
    mov $__boot_stack, %rsp

    # Marker '%'
    mov $0x3F8, %dx
    mov $0x25, %al
    out %al, %dx

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
    .quad 0                                                                   # Entry 0: Null
    .quad (1 << 43) | (1 << 44) | (1 << 47) | (1 << 53)                      # Entry 1: Kernel code  (DPL=0, 64-bit) → 0x08
    .quad (3 << 40) | (1 << 44) | (1 << 47)                                               # Entry 2: Kernel data  (DPL=0)         → 0x10
    .quad 0                                                                   # Entry 3: Placeholder  (padding)        → 0x18
    .quad (1 << 44) | (1 << 47) | (3 << 45)                                  # Entry 4: User data    (DPL=3)          → 0x23 (USER_SS)
    .quad (1 << 43) | (1 << 44) | (1 << 47) | (1 << 53) | (3 << 45)          # Entry 5: User code    (DPL=3, 64-bit)  → 0x2b (USER_CS)
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
