/*
 * x86_64 kernel linker script for moss kernel.
 *
 * Links the kernel at the higher-half virtual address 0xFFFFFFFF80000000
 * while loading at physical address 0x100000 (1 MiB).
 *
 * Memory layout:
 *   Physical: 0x00100000 - 0x00FFFFFF (1 MiB - 16 MiB)
 *   Virtual:  0xFFFFFFFF80100000 - 0xFFFFFFFF80FFFFFF
 */

OUTPUT_FORMAT("elf64-x86-64")
ENTRY(_start)

KERNEL_BASE = 0xFFFFFFFF80000000;
PHYS_OFFSET = 0x100000;

SECTIONS
{
    /*
     * The text segment starts at PHYS_OFFSET so that we identity-map the
     * first 2 MiB during early boot (before the full page tables are up).
     */
    . = PHYS_OFFSET;

    .text : AT(ADDR(.text) - KERNEL_BASE) {
        _stext = .;
        *(.text .text.*)
        _etext = .;
    }

    .rodata : ALIGN(4096) {
        _srodata = .;
        *(.rodata .rodata.*)
        _erodata = .;
    }

    .data : ALIGN(4096) {
        _sdata = .;
        *(.data .data.*)
        _edata = .;
    }

    .bss : ALIGN(4096) {
        _sbss = .;
        *(.bss .bss.*)
        *(COMMON)
        _ebss = .;
    }

    _kernel_end = .;

    /*
     * Discard unwanted sections.
     */
    /DISCARD/ : {
        *(.eh_frame)
        *(.comment)
        *(.note*)
    }
}
