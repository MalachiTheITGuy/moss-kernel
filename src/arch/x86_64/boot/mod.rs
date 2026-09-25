//! x86_64 boot module.
//!
//! Contains the Multiboot2 boot flow that transitions from the assembly
//! entry point (`start.S`) into the common kernel entry point (`kmain`).
//!
//! The flow is:
//! 1. `start.S` — 32-bit protected mode → page tables → long mode → stack
//! 2. `arch_init_stage1` — logger, console, parse Multiboot2 info
//! 3. `arch_init_stage2` — IDT, interrupt setup → `kmain()`

pub mod gdt;
pub mod idt;

use alloc::string::String;

extern crate alloc;

use core::arch::{asm, global_asm};
use core::ptr::{addr_of, addr_of_mut};
use core::slice;

use super::memory::heap::{KernelHeap, SLAB_ALLOC};
use super::memory::mmu::setup_kern_addr_space;
use super::proc::vdso::vdso_init;
use crate::{
    arch::{ArchImpl, CpuOps},
    drivers::init::run_initcalls,
    memory::{FrameAllocator, INITAL_ALLOCATOR, PAGE_ALLOC},
};
use libkernel::error::Result;
use libkernel::memory::address::{PA, TPA};
use libkernel::memory::allocators::slab::allocator::SlabAllocator;
use libkernel::memory::region::PhysMemoryRegion;

/// Kernel base address in the higher-half virtual address space.
/// Physical address 0x0 maps to this virtual address via the identity
/// and higher-half page tables built in `start.S`.
const KERNEL_BASE: u64 = 0xFFFF_FFFF_8000_0000;

/// Number of bytes needed for the kernel boot stack.
const BOOT_STACK_SIZE: usize = 8 * 1024;

unsafe extern "C" {
    static __init_pages_start: u8;
    static __image_start: u8;
    static __image_end: u8;
}

/// Kernel boot stack.
#[unsafe(no_mangle)]
#[unsafe(link_section = ".bss")]
static mut BOOT_STACK: [u8; BOOT_STACK_SIZE] = [0; BOOT_STACK_SIZE];

// ──────────────────────────────────────────────
//  Multiboot2 info structure (tag-based format)
// ──────────────────────────────────────────────

/// Multiboot2 tag header — every tag starts with this.
#[repr(C)]
struct MbootTag {
    ty: u32,
    size: u32,
}

/// Known Multiboot2 tag type: command line string.
const MBOOT_TAG_CMDLINE: u32 = 1;

/// Known Multiboot2 tag type: boot loader name.
const MBOOT_TAG_BOOT_LOADER: u32 = 2;

/// Known Multiboot2 tag type: module (initrd).
const MBOOT_TAG_MODULE: u32 = 3;

/// Known Multiboot2 tag type: memory map.
const MBOOT_TAG_MMAP: u32 = 6;

/// End-of-tags sentinel.
const MBOOT_TAG_END: u32 = 0;

// ──────────────────────────────────────────────
//  BSS statics for stage1 → stage2 handoff
// ──────────────────────────────────────────────

/// Maximum length of the kernel command line stored by stage1.
const BOOT_CMDLINE_MAX: usize = 512;

/// Command line bytes copied from Multiboot2 info during stage1.
#[unsafe(link_section = ".bss")]
static mut BOOT_CMDLINE: [u8; BOOT_CMDLINE_MAX] = [0; BOOT_CMDLINE_MAX];

/// Length of the valid data in `BOOT_CMDLINE`.
#[unsafe(link_section = ".bss")]
static mut BOOT_CMDLINE_LEN: usize = 0;

/// Physical address of the initrd module start (0 if none).
#[unsafe(link_section = ".bss")]
static mut INITRD_START: u64 = 0;

/// Physical address of the initrd module end (0 if none).
#[unsafe(link_section = ".bss")]
static mut INITRD_END: u64 = 0;

/// Multiboot2 memory map entry — one per contiguous physical region.
///
/// Each entry is 24 bytes: `base_addr` (8), `length` (8), `type` (4),
/// `_reserved` (4) per the Multiboot2 specification, version 0.6.96.
#[repr(C)]
#[derive(Copy, Clone)]
struct MbootMmapEntry {
    base_addr: u64,
    length: u64,
    typ: u32,
    _reserved: u32,
}

/// Multiboot2 memory type: usable RAM.
const MBOOT_MMAP_TYPE_AVAIL: u32 = 1;

/// Parse the Multiboot2 memory map and register all usable RAM regions
/// with the physical frame allocator.
///
/// The `info_phys` parameter is the physical address of the Multiboot2
/// information structure passed by GRUB in EBX.
///
/// # Safety
///
/// `info_phys` must point to a valid Multiboot2 info structure mapped
/// by the initial page tables (identity or higher-half).
unsafe fn parse_memory_map(info_phys: u64) -> Result<()> {
    // Convert physical address to higher-half virtual address.
    let info_virt = info_phys.wrapping_add(KERNEL_BASE);

    // Read total size of the info structure (first u32).
    let total_size = unsafe { core::ptr::read(info_virt as *const u32) as usize };

    // Tags start at offset 8 (total_size:u32 + reserved:u32).
    let tags_base = info_virt.wrapping_add(8);
    let tags_end = info_virt.wrapping_add(total_size as u64);

    let mut offset: u64 = 0;

    loop {
        let tag_addr = tags_base.wrapping_add(offset);

        // Bounds check — prevent reading past the structure.
        if tag_addr.wrapping_add(8) > tags_end {
            break;
        }

        let tag = unsafe { &*(tag_addr as *const MbootTag) };

        match tag.ty {
            MBOOT_TAG_END => break,
            MBOOT_TAG_MMAP => {
                // The memory map tag layout:
                //   [u32 ty][u32 size][u32 entry_size][u32 entry_version]
                // followed by a sequence of MbootMmapEntry records.
                if tag.size < 16 {
                    break;
                }

                let header = tag_addr as *const u32;
                let entry_size = unsafe { core::ptr::read(header.add(2)) } as u64;
                let entries_base = tag_addr.wrapping_add(16);
                let entries_end = tag_addr.wrapping_add(tag.size as u64);

                // Guard against zero entry_size.
                if entry_size == 0 {
                    break;
                }

                let mut entry_off: u64 = 0;
                loop {
                    let entry_addr = entries_base.wrapping_add(entry_off);

                    if entry_addr.wrapping_add(core::mem::size_of::<MbootMmapEntry>() as u64)
                        > entries_end
                    {
                        break;
                    }

                    let entry = unsafe { &*(entry_addr as *const MbootMmapEntry) };

                    if entry.typ == MBOOT_MMAP_TYPE_AVAIL && entry.length > 0 {
                        let mut alloc = INITAL_ALLOCATOR.lock_save_irq();
                        if let Some(ref mut a) = *alloc {
                            let region = PhysMemoryRegion::from_start_end_address(
                                PA::from_value(entry.base_addr as usize),
                                PA::from_value((entry.base_addr + entry.length) as usize),
                            );
                            if let Err(e) = a.add_memory(region) {
                                log::warn!(
                                    "mboot: failed to add RAM region at 0x{:x} ({} KiB): {}",
                                    entry.base_addr,
                                    entry.length / 1024,
                                    e,
                                );
                            }
                        }
                    }

                    // Advance by the reported entry_size (allows forward compat).
                    entry_off = entry_off.wrapping_add(entry_size);
                    if entry_off + core::mem::size_of::<MbootMmapEntry>() as u64 > tag.size as u64 {
                        break;
                    }
                }
            }
            _ => {}
        }

        // Advance to the next tag, aligned to 8 bytes.
        let next = (offset + tag.size as u64 + 7) & !7u64;
        if next == offset {
            // Tag size was zero — avoid infinite loop.
            break;
        }
        offset = next;
    }

    Ok(())
}

/// Copy the command line from a Multiboot2 info structure into the BSS
/// static `BOOT_CMDLINE` so it survives the stage1 → stage2 transition.
///
/// # Safety
///
/// `info_phys` must point to a valid Multiboot2 info structure mapped
/// by the initial page tables.  Must be called before the boot stack
/// is reclaimed (i.e. during stage1).
///
/// # Returns
///
/// `true` if a non-empty command line was stored.
unsafe fn store_cmdline(info_phys: u64) -> bool {
    let info_virt = info_phys.wrapping_add(KERNEL_BASE);
    let total_size = unsafe { core::ptr::read(info_virt as *const u32) as usize };
    let tags_base = info_virt.wrapping_add(8);
    let tags_end = info_virt.wrapping_add(total_size as u64);

    let mut offset: u64 = 0;

    loop {
        let tag_addr = tags_base.wrapping_add(offset);
        if tag_addr.wrapping_add(8) > tags_end {
            break;
        }

        let tag = unsafe { &*(tag_addr as *const MbootTag) };

        match tag.ty {
            MBOOT_TAG_END => break,
            MBOOT_TAG_CMDLINE => {
                let str_ptr = tag_addr.wrapping_add(8) as *const u8;
                let max_len = (tag.size as usize).saturating_sub(9);

                if max_len == 0 {
                    return false;
                }

                let bytes = unsafe { slice::from_raw_parts(str_ptr, max_len) };
                let len = bytes.iter().position(|&b| b == 0).unwrap_or(max_len);

                if len == 0 || len >= BOOT_CMDLINE_MAX {
                    return false;
                }

                // SAFETY: BOOT_CMDLINE is a BSS static with exclusive access
                // during stage1 (single-threaded boot path).
                unsafe {
                    let dst = addr_of_mut!(BOOT_CMDLINE) as *mut u8;
                    core::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, len);
                    // SAFETY: `dst` is a valid u8 pointer into the 512-byte BOOT_CMDLINE
                    // buffer; `len` is bounds-checked above (< BOOT_CMDLINE_MAX).
                    *dst.add(len) = 0;
                    *addr_of_mut!(BOOT_CMDLINE_LEN) = len;
                }
                return true;
            }
            _ => {}
        }

        let next = (offset + tag.size as u64 + 7) & !7u64;
        if next == offset {
            break;
        }
        offset = next;
    }

    false
}

/// Parse Multiboot2 MODULE tags to find the initrd (first module entry).
///
/// The Multiboot2 module tag has layout:
///   [u32 ty][u32 size][u64 mod_start][u64 mod_end][string name...]
///
/// `mod_start` and `mod_end` are physical addresses of the module in memory.
///
/// # Safety
///
/// `info_phys` must point to a valid Multiboot2 info structure mapped
/// by the initial page tables.
unsafe fn parse_modules(info_phys: u64) {
    let info_virt = info_phys.wrapping_add(KERNEL_BASE);
    let total_size = unsafe { core::ptr::read(info_virt as *const u32) as usize };
    let tags_base = info_virt.wrapping_add(8);
    let tags_end = info_virt.wrapping_add(total_size as u64);

    let mut offset: u64 = 0;

    loop {
        let tag_addr = tags_base.wrapping_add(offset);
        if tag_addr.wrapping_add(8) > tags_end {
            break;
        }

        let tag = unsafe { &*(tag_addr as *const MbootTag) };

        match tag.ty {
            MBOOT_TAG_END => break,
            MBOOT_TAG_MODULE => {
                // Module tag: [u32 ty][u32 size][u64 mod_start][u64 mod_end][u8 name...]
                if tag.size < 24 {
                    break;
                }

                let header = tag_addr as *const u32;
                let mod_start = unsafe { core::ptr::read(header.add(2) as *const u64) };
                let mod_end = unsafe { core::ptr::read(header.add(4) as *const u64) };

                if mod_start > 0 && mod_end > mod_start {
                    // Store the first module as the initrd.
                    unsafe {
                        INITRD_START = mod_start;
                        INITRD_END = mod_end;
                    }

                    // Log the module name (null-terminated string at offset 24).
                    if tag.size > 24 {
                        let name_ptr = tag_addr.wrapping_add(24) as *const u8;
                        let name_len = (tag.size as usize) - 25; // exclude type+size+null
                        if name_len > 0 {
                            let name_bytes = unsafe { slice::from_raw_parts(name_ptr, name_len) };
                            let name_len =
                                name_bytes.iter().position(|&b| b == 0).unwrap_or(name_len);
                            if let Ok(name) = core::str::from_utf8(&name_bytes[..name_len]) {
                                log::info!(
                                    "mboot: module '{}' at 0x{:x}–0x{:x} ({} bytes)",
                                    name,
                                    mod_start,
                                    mod_end,
                                    mod_end - mod_start,
                                );
                            }
                        }
                    }
                }
            }
            _ => {}
        }

        let next = (offset + tag.size as u64 + 7) & !7u64;
        if next == offset {
            break;
        }
        offset = next;
    }
}

// ──────────────────────────────────────────────
//  Arch init entry points (called from start.S)
// ──────────────────────────────────────────────

/// First-stage architecture initialisation.
///
/// Called from `start.S` after transitioning to 64-bit long mode
/// and setting up the initial stack. Runs on the boot stack.
///
/// # Arguments
///
/// * `mboot_info_ptr` — physical address of the Multiboot2 info
///   structure passed by GRUB in EBX.
///
/// # Returns
///
/// A virtual address of a new kernel stack to be used by
/// `arch_init_stage2`.
///
/// # Safety
///
/// Must only be called once from the assembly entry point.
#[unsafe(no_mangle)]
unsafe extern "C" fn arch_init_stage1(mboot_info_ptr: u64) -> u64 {
    // Set up the early console logger so log::info! and friends work.
    crate::console::setup_console_logger();

    log::info!("moss: x86_64 boot (Multiboot2)");

    // Store the command line into BSS for stage2 access.
    if unsafe { store_cmdline(mboot_info_ptr) } {
        // SAFETY: BOOT_CMDLINE was populated by store_cmdline above.
        let len = unsafe { BOOT_CMDLINE_LEN };
        let bytes = unsafe { &BOOT_CMDLINE[..len] };
        if let Ok(cmd) = core::str::from_utf8(bytes) {
            log::info!("moss: boot cmdline: {}", cmd);
        }
    }

    // Parse Multiboot2 modules — extract initrd physical address range.
    unsafe { parse_modules(mboot_info_ptr) };

    // Set up physical frame allocator from Multiboot2 memory map (tag type 6).
    // SAFETY: Called during single-threaded stage1 boot; mboot_info_ptr is valid
    // and points to the Multiboot2 information structure provided by the bootloader.
    unsafe {
        parse_memory_map(mboot_info_ptr)
            .expect("parse_memory_map: failed to initialise frame allocator");
    }

    // Set up the kernel address space using the page tables built in start.S.
    // SAFETY: __init_pages_start is an extern static defined in start.S, valid
    // during stage1 after page tables are constructed.
    let init_pages_pa = unsafe { TPA::from_value(__init_pages_start as usize) };
    setup_kern_addr_space(init_pages_pa).expect("setup_kern_addr_space failed");

    log::info!("moss: stage1 complete, switching stack");

    // Return the address of the kernel boot stack. The assembly
    // code will switch RSP to this address before calling stage2.
    //
    // Use the static BOOT_STACK. Since the higher-half mapping is
    // active, its address is already valid as a virtual address.
    #[allow(static_mut_refs)]
    unsafe {
        BOOT_STACK.as_ptr().add(BOOT_STACK_SIZE) as u64
    }
}

/// Second-stage architecture initialisation.
///
/// Called from `start.S` after switching to the kernel stack
/// returned by `arch_init_stage1`. Runs on the proper kernel stack.
///
/// # Safety
///
/// Must only be called once, after `arch_init_stage1` has completed.
#[unsafe(no_mangle)]
unsafe extern "C" fn arch_init_stage2() {
    log::info!("moss: stage2 — x86_64 early init");

    // Initialize the frame allocator from the early bootstrap allocator
    // (mirrors ARM64 arch_init_stage2 flow).
    let smalloc = crate::memory::INITAL_ALLOCATOR
        .lock_save_irq()
        .take()
        .expect("INITAL_ALLOCATOR already consumed");
    // SAFETY: FrameAllocator::init is safe to call here — we are in single-threaded
    // stage-2 init with exclusive ownership of `smalloc` and no other CPU is active.
    let (page_alloc, frame_list) = unsafe { crate::memory::FrameAllocator::init(smalloc) };
    crate::memory::PAGE_ALLOC
        .set(page_alloc)
        .unwrap_or_else(|_| panic!("PAGE_ALLOC already set"));
    super::memory::heap::SLAB_ALLOC
        .set(libkernel::memory::allocators::slab::allocator::SlabAllocator::new(frame_list))
        .unwrap_or_else(|_| panic!("SLAB_ALLOC already set"));
    super::memory::heap::KernelHeap::init_for_this_cpu();

    // Initialize exceptions: IDT + syscall entry (Issue #16).
    crate::arch::x86_64::exceptions::exceptions_init().expect("exceptions init failed");

    // Enable hardware interrupts so that driver IRQ handlers can fire.
    // Must happen before run_initcalls() so that interrupt-driven drivers
    // can claim their IRQs.
    ArchImpl::enable_interrupts();

    // Run all kernel_driver! init functions (LAPIC, I/O APIC, UART, HPET,
    // LAPIC timer, etc.).  The LAPIC init must link first so that the
    // interrupt root is established before drivers that call
    // `get_interrupt_root()` (e.g. the UART).
    //
    // This mirrors the ARM64 arch_init_stage2() flow:
    //   exceptions_init → enable_interrupts → run_initcalls → kmain
    unsafe { run_initcalls() };

    // Reconstruct the command line from BSS statics populated by stage1.
    let args = unsafe {
        let len = BOOT_CMDLINE_LEN;
        if len > 0 {
            let bytes = &BOOT_CMDLINE[..len];
            match core::str::from_utf8(bytes) {
                Ok(s) => String::from(s),
                Err(_) => String::new(),
            }
        } else {
            String::new()
        }
    };

    // Log the initrd range if one was found.
    //
    // SAFETY: Stage 1 is single-threaded; INITRD_START/INITRD_END are written
    // by the Multiboot2 parser before kmain and read-only thereafter.
    let initrd_start = unsafe { *addr_of!(INITRD_START) };
    let initrd_end = unsafe { *addr_of!(INITRD_END) };
    if initrd_start != 0 {
        log::info!(
            "moss: initrd 0x{:x}–0x{:x} ({} bytes)",
            initrd_start,
            initrd_end,
            initrd_end - initrd_start,
        );
    }

    log::info!("moss: entering kmain (arch: x86_64)");

    if let Err(e) = vdso_init() {
        log::error!("vdso: {}", e);
    }

    // Allocate a zeroed initial userspace context on the stack.
    // kmain → dispatch_userspace_task will write the real context
    // (from exec / signal delivery) into this frame via
    // copy_nonoverlapping before returning.
    let mut initial_ctx: crate::process::ctx::UserCtx = unsafe { core::mem::zeroed() };
    crate::kmain(args, &mut initial_ctx as *mut _);
}

/// Park a CPU core in a halt loop.
///
/// Called when a CPU has nothing to do — enters a tight `HLT`
/// loop that waits for the next interrupt.
pub extern "C" fn park_cpu() -> ! {
    loop {
        unsafe { asm!("hlt") };
    }
}

/// Read the current value of the RFLAGS register.
///
/// # Safety
///
/// No safety concerns — this is a pure read.
#[inline]
pub fn read_rflags() -> u64 {
    let val: u64;
    unsafe { asm!("pushfq; pop {}", out(reg) val) };
    val
}

/// Return the kernel command line string.
///
/// Stored in BSS by `store_cmdline()` during stage1 from the Multiboot2
/// info structure. Returns an empty string if no command line was provided.
pub fn get_cmdline() -> &'static str {
    // SAFETY: BOOT_CMDLINE is written once during stage1 and read from
    // stage2 onward — single writer, then single reader.
    unsafe {
        let len = BOOT_CMDLINE_LEN;
        if len == 0 {
            ""
        } else {
            core::str::from_utf8_unchecked(&BOOT_CMDLINE[..len])
        }
    }
}

/// Return the initrd physical address range, if one was provided by
/// the bootloader via a Multiboot2 MODULE tag.
///
/// Returns `(start_phys, end_phys)` or `None` if no module was found.
pub fn get_initrd() -> Option<(u64, u64)> {
    // SAFETY: INITRD_START/END are written once during stage1 and read
    // from stage2 onward.
    unsafe {
        if INITRD_START != 0 && INITRD_END > INITRD_START {
            Some((INITRD_START, INITRD_END))
        } else {
            None
        }
    }
}
