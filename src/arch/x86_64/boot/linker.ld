ENTRY(_start)

SECTIONS
{
    /* Load the kernel at 1MB physical address */
    . = 0xffffffff80100000;
    __image_start = .;

    .text : AT(0x100000)
    {
        __text_start = .;
        *(.text.boot)
        *(.text*)
        __text_end = .;
    }

    .rodata : AT(ADDR(.rodata) - 0xffffffff80000000)
    {
        __rodata_start = .;
        *(.rodata*)
        __driver_inits_start = .;
        *(.driver_inits*)
        __driver_inits_end = .;
        __fixups_start = .;
        *(.exception_fixups*)
        __fixups_end = .;
        __rodata_end = .;
    }

    .data : AT(ADDR(.data) - 0xffffffff80000000)
    {
        __data_start = .;
        *(.data*)
        __data_end = .;
    }

    .percpu : AT(ADDR(.percpu) - 0xffffffff80000000)
    {
        __percpu_start = .;
        *(.percpu)
        __percpu_end = .;
    }

    . = ALIGN(4K);
    .vectors : AT(ADDR(.vectors) - 0xffffffff80000000)
    {
        __vectors_start = .;
        *(.vectors)
        __vectors_end = .;
    }

    .vdso : AT(ADDR(.vdso) - 0xffffffff80000000)
    {
        __vdso_start = .;
        *(.vdso)
        __vdso_end = .;
    }

    .bss : AT(ADDR(.bss) - 0xffffffff80000000)
    {
        __bss_start = .;
        *(.bss*)
        __bss_end = .;
    }

    . = ALIGN(4K);
    __init_pages_start = .;
    . += 4K * 64;
    __init_pages_end = .;

    . = ALIGN(16);
    . += 16K;
    __boot_stack = .;

    __image_end = .;
}
