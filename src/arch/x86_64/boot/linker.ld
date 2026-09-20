/*
 * x86_64 kernel linker script for moss kernel.
 *
 * The kernel is loaded at PHYS_BASE (1 MiB) by the Multiboot2
 * bootloader (GRUB). Before the MMU is enabled the CPU executes at
 * physical addresses; after the MMU is enabled the kernel transitions
 * to the higher-half virtual address KERNEL_BASE (0xFFFFFFFF80000000).
 *
 * The boot assembly (.text.boot) is the very first section so GRUB
 * can locate the Multiboot2 header within the first 32 KiB of the
 * ELF image.
 *
 * Memory layout:
 *   Physical: 0x00100000 – end
 *   Virtual (higher-half): 0xFFFFFFFF80100000 – end
 */

OUTPUT_FORMAT("elf64-x86-64")
ENTRY(_start)

PAGE_SIZE = 4K;
PAGE_MASK = PAGE_SIZE - 1;

PHYS_BASE    = 0x100000;
KERNEL_BASE  = 0xFFFFFFFF80000000;

SECTIONS
{
    /*
     * Discard unwanted ELF sections.
     */
    /DISCARD/ : {
        *(.interp .dynamic)
        *(.dynsym .dynstr .hash .gnu.hash)
        *(.eh_frame)
        *(.comment)
        *(.note*)
    }

    /*
     * Place everything at physical addresses starting at PHYS_BASE.
     *
     * After the MMU is enabled with identity + higher-half mappings the
     * code runs at KERNEL_BASE + offset.  Because x86_64 code is
     * position-independent by default (RIP-relative addressing), the
     * same binary works at both physical and virtual addresses.
     */
    . = PHYS_BASE;

    ASSERT((. & PAGE_MASK) == 0, "Kernel load address not page aligned")

    __image_start = .;

    /*
     * .text.boot must come first — GRUB scans this region for the
     * Multiboot2 magic number.
     */
    __text_boot_start = .;
    .text.boot : {
        KEEP(*(.text.boot._start))
        KEEP(*(.text.boot))
    }
    __text_boot_end = .;

    __text_start = .;
    .text : ALIGN(4096) { *(.text .text.*) }
    __text_end = .;

    __rodata_start = .;
    .rodata : ALIGN(4096) {
        *(.rodata .rodata.*)
        . = ALIGN(8);
        __driver_inits_start = .;
        *(.driver_inits*)
        __driver_inits_end = .;
        __fixups_start = .;
        *(.exception_fixups*)
        __fixups_end = .;
    }
    __rodata_end = .;

    .exception_fixups : ALIGN(8) {
    }

    __data_start = .;
    .data : ALIGN(4096) { *(.data .data.*) }
    __data_end = .;

    .bss : ALIGN(4096) {
        . = ALIGN(16);
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        . = ALIGN(16);
        __bss_end = .;
    }

    /*
     * Scratch pages used by the boot assembly to build the initial
     * 4-level page tables.  64 pages = 256 KiB is more than enough
     * for identity + higher-half mappings of the first 4 MiB.
     */
    . = ALIGN(PAGE_SIZE);
    __init_pages_start = .;
    . += PAGE_SIZE * 64;
    __init_pages_end = .;

    /*
     * Early boot stack, placed just before __image_end so the stack
     * grows downward into free space.
     */
    . = ALIGN(16);
    __boot_stack_top = .;
    . += 32768;                 /* 32 KiB */
    __boot_stack = .;

    __image_end = .;
}
