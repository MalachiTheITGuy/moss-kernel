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

/* ---------- Multiboot2 header (must be 8-byte aligned) ---------- */
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
    /* ---- 1. Validate Multiboot2 magic ---- */
    cmpl    $MB2_MAGIC, %eax
    jne     .Lhang

    /* EBX = Multiboot2 info pointer (physical address).
     * Keep it in EBX across the BSS zero (EBX is callee-saved). */

    /* ---- 2. Zero BSS ---- */
    movl    $___bss_start, %edi
    movl    $___bss_end, %ecx
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
     *   +0x2000  PDPT_kh    (4 KiB) — higher-half PDPT
     *   +0x3000  PD_id      (4 KiB) — identity-map PD
     *   +0x4000  PD_kh      (4 KiB) — higher-half PD
     *   +0x5000  PT_lo      (4 KiB) — maps phys 0x00000–0x1FFFFF
     *   +0x6000  PT_hi      (4 KiB) — maps phys 0x200000–0x3FFFFF
     *
     * PML4[0]   → PDPT_id → PD_id → PT_lo, PT_hi  (identity)
     * PML4[511] → PDPT_kh → PD_kh → PT_lo, PT_hi  (higher-half)
     *
     * PT_lo / PT_hi are shared between both mappings.
     * PML4 covers: 0xFFFFFFFF80000000 at index 511, bits 47:39.
     * PDPT within PML4[511] covers: index 0 → 0xFFFFFFFF80000000.
     * PD within PDPT[0] covers:     index 0 → first 2 MiB of mapping.
     */

    movl    $___init_pages_start, %esi   /* ESI = PML4 base             */

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

    /* -- PDPT_kh[0] → PD_kh -- */
    lea     0x4000(%esi), %eax
    orl     $PTE_RW_P, %eax
    movl    %eax, 0x2000(%esi)           /* PDPT_kh entry 0             */

    /* -- PD_id[0] → PT_lo,  PD_id[1] → PT_hi -- */
    lea     0x5000(%esi), %eax            /* PT_lo                       */
    orl     $PTE_RW_P, %eax
    movl    %eax, 0x3000(%esi)           /* PD_id entry 0               */

    lea     0x6000(%esi), %eax            /* PT_hi                       */
    orl     $PTE_RW_P, %eax
    movl    %eax, 0x3008(%esi)           /* PD_id entry 1               */

    /* -- PD_kh[0] → PT_lo,  PD_kh[1] → PT_hi -- */
    lea     0x5000(%esi), %eax
    orl     $PTE_RW_P, %eax
    movl    %eax, 0x4000(%esi)           /* PD_kh entry 0               */

    lea     0x6000(%esi), %eax
    orl     $PTE_RW_P, %eax
    movl    %eax, 0x4008(%esi)           /* PD_kh entry 1               */

    /* -- Fill PT_lo: map phys 0x000000–0x1FFFFF (512 × 4 KiB) -- */
    movl    $(___init_pages_start + 0x5000), %edi
    movl    $PTE_RW_P, %eax              /* first PTE: phys 0x0000 | f */
    movl    $512, %ecx
.Lfill_pt_lo:
    movl    %eax, (%edi)
    addl    $0x1000, %eax                /* next physical page           */
    addl    $8, %edi                     /* next PTE slot                */
    decl    %ecx
    jnz     .Lfill_pt_lo

    /* -- Fill PT_hi: map phys 0x200000–0x3FFFFF (512 × 4 KiB) -- */
    movl    $(___init_pages_start + 0x6000), %edi
    movl    $(0x200000 + PTE_RW_P), %eax /* first PTE: phys 0x200000   */
    movl    $512, %ecx
.Lfill_pt_hi:
    movl    %eax, (%edi)
    addl    $0x1000, %eax
    addl    $8, %edi
    decl    %ecx
    jnz     .Lfill_pt_hi

    /* ---- 4. Enable PAE + Long Mode + Paging ---- */

    /* CR3 = PML4 physical address */
    movl    $___init_pages_start, %eax
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

    /* ---- 7. Set up boot stack ----
     * __boot_stack is at a physical address (linked at PHYS_BASE).
     * Identity mapping is active so the physical address is valid. */
    movq    $___boot_stack, %rsp

    /* Clear frame pointer for clean backtraces */
    xorq    %rbp, %rbp

    /* ---- 8. Call arch_init_stage1(mboot_info_ptr) ----
     * System V AMD64 ABI: first argument in RDI.
     * Returns: new stack pointer in RAX. */
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
