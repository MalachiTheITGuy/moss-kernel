# PVH ELF Note (required for QEMU -kernel to load an uncompressed ELF).
# QEMU enters here in 32-bit protected mode with:
#   EAX = 0x336ec578 (XEN_HVM_START_MAGIC_VALUE)
#   EBX = physical address of hvm_start_info
.section .note.PVH, "a"
.align 4
    .long 4             # Name size
    .long 4             # Desc size
    .long 18            # Type: XEN_ELFNOTE_PHYS32_ENTRY
    .ascii "Xen\0"      # Name
    .long _start - 0xffffffff80000000 # Entry point (physical)

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

.section .text.boot
.code32
.global _start
_start:
    cli
    cld

    # Save boot magic (EAX) and boot info pointer (EBX) before they are
    # clobbered.  Under PVH boot EAX=0x336ec578 and EBX->hvm_start_info;
    # under Multiboot2 EAX=0x36d76289 and EBX->multiboot2 info.
    # We store them in .data (not .bss) so BSS zeroing does not wipe them.
    mov %eax, (boot_magic_saved - 0xffffffff80000000)
    mov %ebx, (boot_info_ptr_saved - 0xffffffff80000000)

    # Update stack pointer (physical)
    mov $(__boot_stack - 0xffffffff80000000), %esp



    # Zero BSS
    cld
    mov $(__bss_start - 0xffffffff80000000), %edi
    mov $(__bss_end - 0xffffffff80000000), %ecx
    sub %edi, %ecx
    xor %eax, %eax
    rep stosb

    # Verify Multiboot2 magic in EAX. QEMU loads via the multiboot2 header
    # and passes 0x36d76289 in EAX with EBX pointing to the info structure.
    # cmp $0x36d76289, %eax
    # jne .error

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



    # Load CR3
    mov $(boot_pml4 - 0xffffffff80000000), %eax
    mov %eax, %cr3





    # Enable PAE + SSE/SSE2 (OSFXSR=bit9, OSXMMEXCPT=bit10)
    mov %cr4, %eax
    or $(1 << 5) | (1 << 9) | (1 << 10), %eax
    mov %eax, %cr4

    # Clear CR0.EM (bit2) and set CR0.MP (bit1) so SSE instructions don't #UD/trap
    mov %cr0, %eax
    and $(~(1 << 2)), %eax  # clear EM
    or $(1 << 1), %eax      # set MP
    mov %eax, %cr0

    # 2. Enable Long Mode and NX in EFER
    mov $0xC0000080, %ecx
    rdmsr
    or $(1 << 8) | (1 << 11), %eax # Bit 8: LME (Long Mode Enable), Bit 11: NXE (NX Enable)
    wrmsr

    # Enable Paging
    mov %cr0, %eax
    or $(1 << 31), %eax
    mov %eax, %cr0

    # Load 64-bit GDT
    lgdt (gdt64_ptr_phys - 0xffffffff80000000)



    # Far jump to 64-bit code
    ljmp $0x8, $(.long_mode - 0xffffffff80000000)

.error:
    # Just hang
    hlt
    jmp .error

.code64
.long_mode:
    # Transition to the higher half
    movabsq $.long_mode_high, %rax
    jmp *%rax

.long_mode_high:
    # Now running in higher half. Reload GDT with virtual address.
    lgdt gdt64_ptr_virt



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
    # Arguments: rdi = mb_info_ptr, rsi = image_start, rdx = image_end, rcx = boot_magic
    # Use RIP-relative loads: bare symbol refs at 0xffffffff8... can't be
    # encoded as 32-bit absolute addresses in 64-bit mode.
    movl boot_info_ptr_saved(%rip), %edi
    mov $__image_start, %rsi
    mov $__image_end, %rdx
    movl boot_magic_saved(%rip), %ecx
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
    .quad (3 << 40) | (1 << 44) | (1 << 47) | (3 << 45)                      # Entry 4: User data    (DPL=3, W=1)     → 0x23 (USER_SS)
    .quad (1 << 43) | (1 << 44) | (1 << 47) | (1 << 53) | (3 << 45)          # Entry 5: User code    (DPL=3, 64-bit)  → 0x2b (USER_CS)
gdt64_ptr_phys:
    .short . - gdt64 - 1
    .quad gdt64 - 0xffffffff80000000

gdt64_ptr_virt:
    .short . - gdt64 - 1
    .quad gdt64

# Saved across the 32-bit → 64-bit transition.  Must be in .data
# (not .bss) because BSS is zeroed after these are written.
.section .data
.align 4
boot_magic_saved:
    .long 0
boot_info_ptr_saved:
    .long 0

.section .bss
.align 4096
boot_pml4:
    .fill 4096, 1, 0
boot_pdpt:
    .fill 4096, 1, 0
boot_pd:
    .fill 4096, 1, 0
