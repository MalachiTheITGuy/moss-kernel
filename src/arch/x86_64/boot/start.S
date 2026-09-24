/*
 * x86_64 boot assembly for the moss kernel.
 *
 * Entered by a Multiboot2-compatible bootloader (GRUB) in 32-bit
 * protected mode.  This code:
 *
 *   1. Validates the Multiboot2 magic.
 *   2. Zeros the BSS.
 *   3. Builds identity-map + higher-half 4-level page tables.
 *   4. Enables PAE, long mode, and paging.
 *   5. Far-jumps into 64-bit long mode.
 *   6. Reloads segment registers and sets up the boot stack.
 *   7. Calls arch_init_stage1(mboot_info_ptr) in Rust.
 *   8. Switches to the stack returned by stage 1 and calls
 *      arch_init_stage2().
 *
 * Linked at physical addresses (PHYS_BASE = 0x100000) via linker.ld.
 * x86_64 position-independent code means RIP-relative addressing
 * works identically at both physical and virtual addresses.
 */

/* ---- Multiboot2 constants ---- */
#define MB2_MAGIC    0xe85250d6
#define MB2_REG_MAGIC 0x36d76289
#define MB2_ARCH_i386 0

/* ---- Page-table entry flags ---- */
#define PTE_P         (1 << 0)   /* Present              */
#define PTE_RW        (1 << 1)   /* Read / Write         */
#define PTE_PS_L2     (1 << 7)   /* Page Size in PDE (L2) */

#define PTE_RW_P      (PTE_RW | PTE_P)

/* ---- GDT selectors ---- */
#define SEL_NULL      0x00
#define SEL_CODE64    0x08
#define SEL_DATA64    0x10

/* =================================================================
 * .text.boot — placed first in the ELF so GRUB finds the Multiboot2
 *              header within the first 32 KiB.
 * ================================================================= */
.section .text.boot._start, "ax", @progbits
.code32

/* ---------- PVH ELF note for QEMU x86_64 boot (XEN_ELFNOTE_PV_INFO) ----------
 *
 * QEMU's x86_64 -kernel flag does not support Multiboot2 directly.
 * It requires a PVH (Paravirtualized Hardware Virtualization) ELF
 * note of type 0x12 with name "Xen" so it knows the entry point.
 * The _start address must match the vaddr field below.
 * ------------------------------------------------------------------ */
.section .note.Xen, "", @note
.align 4
    .long   4                                   /* namesz: "Xen\0"             */
    .long   24                                  /* descsz: pv_info struct      */
    .long   0x12                                /* type: XEN_ELFNOTE_PV_INFO  */
    .asciz  "Xen"                               /* name                        */
    .align 4
    /* Xen PVH info struct (version 2) */
    .long   2                                   /* version = 2                 */
    .long   0                                   /* flags = 0                   */
    .quad   _start                              /* vaddr: entry point          */
    .quad   0                                   /* paddr: 0 (unused)           */
    .quad   0                                   /* raddr: 0 (unused)           */
    .long   0                                   /* mem_size: 0 (let QEMU -m)   */

/* ---------- Multiboot2 header (must be 8-byte aligned) ---------- */
.section .text.boot._start, "ax", @progbits
.align 8
mb2_header:
    .long   MB2_MAGIC                          /* magic                      */
    .long   MB2_ARCH_i386                      /* architecture: i386         */
    .long   mb2_header_end - mb2_header        /* header length              */
    .long   -(MB2_MAGIC + MB2_ARCH_i386 + \
              (mb2_header_end - mb2_header))   /* checksum                   */
    /* End tag (type=0, flags=0, size=8) */
    .word   0                                  /* type: end                  */
    .word   0                                  /* flags                      */
    .long   8                                  /* size of this tag           */
mb2_header_end:

/* =================================================================
 * _start — 32-bit protected-mode entry point
 * ================================================================= */
.globl _start
.type _start, @function
_start:
    /* ---- 0. Early diagnostic: write 'S' to QEMU debug port (0xe9) ---- */
    pushl   %eax                /* save Multiboot2 register magic        */
    movb    $0x53, %al          /* 'S' = start */
    movw    $0xe9, %dx
    outb    %al, %dx

    /* Also write to COM1 (0x3F8) for serial output */
    movw    $0x3F8+5, %dx       /* LSR */
.Lwait_uart1:
    inb     %dx, %al
    testb   $0x20, %al          /* THR empty? */
    jz      .Lwait_uart1
    movb    $0x53, %al          /* 'S' = start */
    movw    $0x3F8, %dx
    outb    %al, %dx
    popl    %eax                /* restore Multiboot2 register magic */

    /* ---- 1. Validate Multiboot2 magic ---- */
    cmpl    $MB2_REG_MAGIC, %eax
    jne     .Lhang

    /* EBX = Multiboot2 info pointer (physical address).
     * Keep it in EBX across the BSS zero (EBX is callee-saved). */

    /* ---- 2. Zero BSS ---- */
    movl    $__bss_start, %edi
    movl    $__bss_end, %ecx
    subl    %edi, %ecx
    shrl    $2, %ecx                     /* bytes → dwords              */
    xorl    %eax, %eax
    cld
    rep stosl

    /* ---- Save multiboot info pointer (now that BSS is clean) ---- */
    movl    %ebx, (mboot_info_ptr)

    /* ---- 3. Build 4-level page tables ----
     *
     * Layout at __init_pages_start:
     *   +0x0000  PML4       (4 KiB)
     *   +0x1000  PDPT_id    (4 KiB) — identity-map PDPT
     *   +0x2000  PDPT_kh    (4 KiB) — higher-half PDPT (entry 510)
     *   +0x3000  PD_id      (4 KiB) — identity-map PD (2 MiB large pages)
     *   +0x4000  PD_kh      (4 KiB) — higher-half PD (2 MiB large pages)
     *
     * Uses 2 MiB large pages (PSE) so no separate page tables are needed.
     * Each PD entry with PS bit set maps a 2 MiB aligned physical range.
     *
     * PML4[0]   → PDPT_id → PD_id  (identity: phys 0x000000–0x1FFFFFF)
     * PML4[511] → PDPT_kh → PD_kh  (higher-half: phys 0x000000–0x1FFFFFF)
     *
     * Higher-half virtual 0xFFFFFFFF80000000 = PML4[511], PDPT[510], PD[0]
     * We map 16 PD entries × 2 MiB = 32 MiB, covering the entire kernel
     * image, init pages, and boot stack.
     */

    movl    $__init_pages_start, %esi   /* ESI = PML4 base             */

    /* -- PML4[0] → PDPT_id -- */
    lea     0x1000(%esi), %eax            /* PDPT_id phys addr           */
    orl     $PTE_RW_P, %eax
    movl    %eax, 0x000(%esi)            /* PML4 entry 0                */

    /* -- PML4[511] → PDPT_kh -- */
    lea     0x2000(%esi), %eax            /* PDPT_kh phys addr           */
    orl     $PTE_RW_P, %eax
    movl    %eax, 0xFF8(%esi)            /* PML4 entry 511 (= 511*8)   */

    /* -- PDPT_id[0] → PD_id -- */
    lea     0x3000(%esi), %eax
    orl     $PTE_RW_P, %eax
    movl    %eax, 0x1000(%esi)           /* PDPT_id entry 0             */

    /* -- PDPT_kh[510] → PD_kh --
     *
     * Virtual address 0xFFFFFFFF80000000 decodes to:
     *   PML4[511], PDPT[510], PD[0], PT[0]
     *
     * So the PDPT entry index must be 510 (byte offset 510*8 = 0xFF0
     * within the PDPT page at offset 0x2000).
     */
    lea     0x4000(%esi), %eax
    orl     $PTE_RW_P, %eax
    movl    %eax, 0x2FF0(%esi)           /* PDPT_kh entry 510 (0x2000+0xFF0) */

    /*
     * Fill PD_id and PD_kh with 2 MiB large-page (PSE) entries.
     * Each entry maps a 2 MiB physical range directly — no page tables.
     * The PS bit (1 << 7) marks a large page.  We map 16 entries = 32 MiB,
     * which covers the kernel image, init pages, and boot stack.
     *
     * PTE_RW_P = 0x03 (Present + Writable).  OR in 0x80 for PS.
     */
    movl    $(0x03 | 0x80), %ecx        /* ECX = PSE_LARGE_PTE          */

    /* -- PD_id: identity map phys 0x000000–0x1FFFFFF -- */
    movl    $(__init_pages_start + 0x3000), %edi
    movl    $0x00000000, %eax            /* first 2 MiB base             */
    movl    $16, %edx                    /* 16 entries = 32 MiB          */
.Lfill_pd_id:
    movl    %eax, %ebx
    orl     %ecx, %ebx                  /* PSE_LARGE_PTE                */
    movl    %ebx, (%edi)
    addl    $0x200000, %eax             /* next 2 MiB base              */
    addl    $8, %edi                     /* next PDE slot                */
    decl    %edx
    jnz     .Lfill_pd_id

    /* -- PD_kh: higher-half map phys 0x000000–0x1FFFFFF -- */
    movl    $(__init_pages_start + 0x4000), %edi
    movl    $0x00000000, %eax
    movl    $16, %edx
.Lfill_pd_kh:
    movl    %eax, %ebx
    orl     %ecx, %ebx
    movl    %ebx, (%edi)
    addl    $0x200000, %eax
    addl    $8, %edi
    decl    %edx
    jnz     .Lfill_pd_kh

    /* ---- 4. Enable PAE + Long Mode + Paging ---- */

    /* CR3 = PML4 physical address */
    movl    $__init_pages_start, %eax
    movl    %eax, %cr3

    /* CR4.PAE (bit 5) = 1 */
    movl    %cr4, %eax
    orl     $(1 << 5), %eax
    movl    %eax, %cr4

    /* EFER.LME (bit 8) = 1 — enables long mode */
    movl    $0xC0000080, %ecx
    rdmsr
    orl     $(1 << 8), %eax
    wrmsr

    /* CR0.PG (bit 31) = 1 — enable paging.
     * CPU transitions to compatibility mode (32-bit sub-mode of long
     * mode) because EFER.LME is set. */
    movl    %cr0, %eax
    orl     $(1 << 31), %eax
    movl    %eax, %cr0

    /* ---- 5. Load GDT (identity-mapped via .text.boot) ----
     * The GDT lives in .text.boot at physical address 0x100000+N,
     * which is identity-mapped.  We must load it BEFORE the far
     * jump because SEL_CODE64 (0x08) must resolve through a valid
     * GDT. */
    lgdt    boot_gdt_desc

    /* ---- 6. Far jump to 64-bit code segment ----
     * Loads CS with SEL_CODE64 (a 64-bit code descriptor) and
     * transfers to long_mode_start.  The CPU enters full 64-bit
     * mode. */
    ljmp    $SEL_CODE64, $long_mode_start

/* =================================================================
 * 64-bit long mode
 * ================================================================= */
.align 4
.code64
long_mode_start:
    /* ---- 6. Reload segment registers ---- */
    movw    $SEL_DATA64, %ax
    movw    %ax, %ds
    movw    %ax, %es
    movw    %ax, %fs
    movw    %ax, %gs
    movw    %ax, %ss

    /* ---- 6b. Early diagnostic: write 'L' to debug port ---- */
    movb    $0x4C, %al          /* 'L' = long mode */
    movw    $0xe9, %dx
    outb    %al, %dx
    /* Also write to COM1 */
    movw    $0x3F8+5, %dx
.Lwait_uart_long:
    inb     %dx, %al
    testb   $0x20, %al
    jz      .Lwait_uart_long
    movb    $0x4C, %al          /* 'L' = long mode */
    movw    $0x3F8, %dx
    outb    %al, %dx

    /* ---- 7. Set up boot stack ----
     * __boot_stack is at a physical address (linked at PHYS_BASE).
     * Identity mapping is active so the physical address is valid. */
    movq    $__boot_stack, %rsp

    /* Clear frame pointer for clean backtraces */
    xorq    %rbp, %rbp

    /* ---- 8. Call arch_init_stage1(mboot_info_ptr) ----
     * System V AMD64 ABI: first argument in RDI.
     * Returns: new stack pointer in RAX. */

    /* Early diagnostic: write 'C' to debug port (before calling Rust) */
    movb    $0x43, %al          /* 'C' = calling stage1 */
    movw    $0xe9, %dx
    outb    %al, %dx
    /* Also write to COM1 */
    movw    $0x3F8+5, %dx
.Lwait_uart_call:
    inb     %dx, %al
    testb   $0x20, %al
    jz      .Lwait_uart_call
    movb    $0x43, %al          /* 'C' = calling stage1 */
    movw    $0x3F8, %dx
    outb    %al, %dx

    movq    (mboot_info_ptr), %rdi
    call    arch_init_stage1

    /* RAX = new stack pointer from stage 1 (higher-half stack) */
    movq    %rax, %rsp

    /* ---- 9. Call arch_init_stage2() ---- */
    call    arch_init_stage2

    /* ---- GDT (in .text.boot so it is at a physical address) ---- */
    .align 4
boot_gdt:
    .quad   0x0000000000000000       /* 0x00: null descriptor          */
    .quad   0x00AF9A000000FFFF       /* 0x08: 64-bit code (L=1,D=0)   */
    .quad   0x00CF92000000FFFF       /* 0x10: 64-bit data              */
boot_gdt_end:

    .align 4
boot_gdt_desc:
    .word   boot_gdt_end - boot_gdt - 1   /* limit (size − 1)         */
    .long   boot_gdt                       /* base address             */

    /* Should never return.  Halt if it does. */
.Lhang:
    cli
    hlt
    jmp     .Lhang

/* =================================================================
 * .bss — uninitialized storage
 * ================================================================= */
.section .bss, "aw", @nobits
.align 4
mboot_info_ptr:
    .space 4

.end
