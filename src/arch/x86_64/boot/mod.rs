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

// ── Early boot debug diagnostics ──────────────────────
// Write a byte to QEMU debug port 0xe9 AND COM1 (0x3F8).
// These are raw port I/O — safe to call before any driver init.

/// Write a single diagnostic byte to QEMU debug port (0xe9) and COM1.
#[inline]
pub(crate) fn boot_diag(byte: u8) {
    unsafe {
        // QEMU debug console (ISA debugcon)
        core::arch::asm!("out dx, al", in("dx") 0xe9_u16, in("al") byte);
        // COM1 — wait for TX ready
        let mut ready: u8;
        core::arch::asm!(
            "in al, dx",
            out("al") ready,
            in("dx") 0x3FD_u16,  // COM1 LSR
        );
        // Wait until bit 5 (THRE) is set
        while ready & 0x20 == 0 {
            core::arch::asm!(
                "in al, dx",
                out("al") ready,
                in("dx") 0x3FD_u16,
            );
        }
        core::arch::asm!("out dx, al", in("dx") 0x3F8_u16, in("al") byte);
    }
}

/// Disable the local APIC via IA32_APIC_BASE MSR (bit 11 = APIC Software Disable).
///
/// Called during early boot to prevent SeaBIOS-configured LAPIC interrupts
/// from firing before the driver sets up the interrupt root.  The LAPIC
/// driver re-enables this bit in `x86_64_lapic_init`.
#[inline]
unsafe fn disable_lapic_msr() {
    const IA32_APIC_BASE: u32 = 0x1B;
    const APIC_DISABLE_BIT: u32 = 1 << 11;
    let low: u32;
    let high: u32;
    // SAFETY: rdmsr is a ring-0 instruction; MSR 0x1B (IA32_APIC_BASE)
    // is always available on x86_64 CPUs with an LAPIC.
    unsafe {
        core::arch::asm!(
            "rdmsr",
            out("eax") low,
            out("edx") high,
            in("ecx") IA32_APIC_BASE,
        );
    }
    let val = ((high as u64) << 32) | (low as u64);
    let disabled = val & !(APIC_DISABLE_BIT as u64);
    let d_low = disabled as u32;
    let d_high = (disabled >> 32) as u32;
    // SAFETY: writes back the same MSR with bit 11 cleared.
    unsafe {
        core::arch::asm!(
            "wrmsr",
            in("ecx") IA32_APIC_BASE,
            in("eax") d_low,
            in("edx") d_high,
        );
    }
}

/// Kernel base address in the higher-half virtual address space.
/// Physical address 0x0 maps to this virtual address via the identity
/// and higher-half page tables built in `start.S`.
const KERNEL_BASE: u64 = 0xFFFF_FFFF_8000_0000;

/// Number of bytes needed for the kernel boot stack.
const BOOT_STACK_SIZE: usize = 64 * 1024;

unsafe extern "C" {
    static __init_pages_start: u8;
    static __image_start: u8;
    static __image_end: u8;
    /// Boot protocol EAX value saved by start.S (in .text, survives BSS zero).
    /// 0x36d76289 = Multiboot2, 0x2BADB009 = Multiboot1, 0 = PVH.
    static boot_eax: u32;
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

/// Multiboot2 register magic — EAX value passed by GRUB on entry.
const MB2_REG_MAGIC: u32 = 0x36d76289;

/// Multiboot1 register magic — EAX value passed by QEMU `-kernel` on entry.
const MB1_REG_MAGIC: u32 = 0x2BADB009;

// ──────────────────────────────────────────────
//  Multiboot1 info structure (flags-based format)
// ──────────────────────────────────────────────

/// Multiboot1 info header (at offset 0 of the info structure).
///
/// The `flags` field is a bitmask:
/// - bit 0: mem_lower/mem_upper valid
/// - bit 1: boot_device valid
/// - bit 2: cmdline valid
/// - bit 3: mods_count/mods_addr valid
/// - bit 6: mmap_length/mmap_addr valid
#[repr(C)]
#[derive(Copy, Clone)]
struct Mb1Info {
    flags: u32,
    mem_lower: u32,
    mem_upper: u32,
    boot_device: u32,
    cmdline: u32,
    mods_count: u32,
    mods_addr: u32,
    syms: [u32; 4],
    mmap_length: u32,
    mmap_addr: u32,
}

/// Multiboot1 info flags.
const MB1_FLAG_MEMORY: u32 = 1 << 0;
const MB1_FLAG_CMDLINE: u32 = 1 << 2;
const MB1_FLAG_MODS: u32 = 1 << 3;
const MB1_FLAG_MMAP: u32 = 1 << 6;

/// Multiboot1 memory map entry (at `mmap_addr`).
///
/// The `size` field is the "size of this structure - 4"; it does NOT include
/// the size field itself.  Advance by `size + 4` to reach the next entry.
#[repr(C)]
#[derive(Copy, Clone)]
struct Mb1MmapEntry {
    size: u32,
    base_addr: u64,
    length: u64,
    typ: u32,
}

/// Multiboot1 module entry (at `mods_addr`).
#[repr(C)]
#[derive(Copy, Clone)]
struct Mb1ModEntry {
    mod_start: u32,
    mod_end: u32,
    cmdline: u32,
    _reserved: u32,
}

/// Multiboot1 memory type: usable RAM.
const MB1_MMAP_TYPE_AVAIL: u32 = 1;

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

/// Initialise the physical frame allocator for PVH boot (no Multiboot2 tags).
///
/// When QEMU loads an ELF via `-kernel`, it uses PVH boot: EAX=0,
/// EBX=hvm_start_info.  The kernel image is placed at the ELF-specified
/// addresses.  We provide a conservative memory map covering physical RAM
/// from `__image_end` to 2 GiB (matching QEMU's default `-m 2G`).
///
/// # Safety
///
/// Must be called once during single-threaded stage1, before the frame
/// allocator is used.  The identity and higher-half page tables are active.
unsafe fn init_pvh_memory() {
    // Page-align __image_end.
    let image_end = unsafe { addr_of!(__image_end) as usize };
    let start = (image_end + 0xFFF) & !0xFFF; // round up to page boundary

    // Conservative end: 2 GiB (matches QEMU -m 2G default).
    const RAM_END: usize = 0x8000_0000;

    if start >= RAM_END {
        log::warn!(
            "mboot: PVH memory init — image_end ({:#x}) >= RAM_END, no usable RAM",
            start
        );
        return;
    }

    let region =
        PhysMemoryRegion::from_start_end_address(PA::from_value(start), PA::from_value(RAM_END));

    let mut alloc = INITAL_ALLOCATOR.lock_save_irq();
    if let Some(ref mut a) = *alloc {
        if let Err(e) = a.add_memory(region) {
            log::warn!(
                "mboot: PVH failed to add RAM region 0x{:x}–0x{:x}: {}",
                start,
                RAM_END,
                e,
            );
        } else {
            log::info!(
                "mboot: PVH added RAM 0x{:x}–0x{:x} ({} MiB)",
                start,
                RAM_END,
                (RAM_END - start) / (1024 * 1024),
            );
        }
    }
}

// ──────────────────────────────────────────────
//  PVH hvm_start_info parsing (Xen / QEMU -kernel)
// ──────────────────────────────────────────────

/// Xen PVH hvm_start_info — physical address passed in EBX on PVH entry.
///
/// Layout (48 bytes, version >= 1):
/// ```text
///  Off  Size  Field
///   0   4     version          (uint32)
///   4   4     num_modules      (uint32)
///   8   8     modlist          (uint64 — phys addr of first hvm_modlist_entry)
///  16   8     cmdline          (uint64 — phys addr of null-terminated string)
///  24   8     rsdp             (uint64 — unused)
///  32   8     memmap_paddr     (uint64 — phys addr of hvm_memmap_entry array)
///  40   4     memmap_entries   (uint32)
///  44   4     reserved         (uint32)
/// ```
#[repr(C)]
#[derive(Copy, Clone)]
struct HvmStartInfo {
    version: u32,
    _num_modules: u32,
    modlist: u64,
    cmdline: u64,
    _rsdp: u64,
    memmap_paddr: u64,
    memmap_entries: u32,
    _reserved: u32,
}

/// Xen PVH hvm_modlist_entry — describes one boot module (e.g. initrd).
///
/// Layout (24 bytes):
/// ```text
///  Off  Size  Field
///   0   8     mod_start  (uint64 — phys addr)
///   8   8     mod_end    (uint64 — phys addr)
///  16   8     cmdline    (uint64 — phys addr of string)
/// ```
#[repr(C)]
#[derive(Copy, Clone)]
struct HvmModlistEntry {
    mod_start: u64,
    mod_end: u64,
    _cmdline: u64,
}

/// Xen PVH hvm_memmap_entry — one region of the physical memory map.
///
/// Layout (24 bytes):
/// ```text
///  Off  Size  Field
///   0   8     addr      (uint64 — phys addr of region start)
///   8   8     size      (uint64 — region size in bytes)
///  16   4     type      (uint32 — 1 = RAM, other = reserved)
///  20   4     reserved  (uint32)
/// ```
#[repr(C)]
#[derive(Copy, Clone)]
struct HvmMemmapEntry {
    addr: u64,
    size: u64,
    typ: u32,
    _reserved: u32,
}

/// Memory type indicating usable RAM in hvm_memmap_entry.
const HVM_MEMMAP_TYPE_RAM: u32 = 1;

/// Parse the Xen PVH hvm_start_info structure and extract initrd, cmdline,
/// and memory map information for the kernel boot.
///
/// The `info_phys` parameter is the physical address of the hvm_start_info
/// structure passed by QEMU in EBX during PVH boot.
///
/// This function:
/// 1. Extracts initrd module address (mod_start/mod_end) into INITRD_START/END.
/// 2. Extracts kernel cmdline into BOOT_CMDLINE/BOOT_CMDLINE_LEN.
/// 3. Parses the e820-style memory map and registers RAM regions with the
///    frame allocator.
///
/// Falls back to `init_pvh_memory()` if the structure version is < 1 or
/// has no memory map entries.
///
/// # Safety
///
/// `info_phys` must be a valid physical address pointing to an hvm_start_info
/// structure. The identity and higher-half page tables are active.
unsafe fn init_pvh_boot_info(info_phys: u64) {
    // Convert physical address to higher-half virtual address for reading.
    let info_virt = info_phys.wrapping_add(KERNEL_BASE);

    // Read the hvm_start_info header (48 bytes).
    let info = unsafe { core::ptr::read(info_virt as *const HvmStartInfo) };

    log::info!(
        "moss: PVH hvm_start_info v{}, memmap_entries={}",
        info.version,
        info.memmap_entries,
    );

    // --- 1. Extract initrd from first module entry ---
    if info.modlist != 0 && info._num_modules > 0 {
        let modlist_virt = info.modlist.wrapping_add(KERNEL_BASE);
        let module = unsafe { core::ptr::read(modlist_virt as *const HvmModlistEntry) };

        if module.mod_start > 0 && module.mod_end > module.mod_start {
            unsafe {
                INITRD_START = module.mod_start;
                INITRD_END = module.mod_end;
            }
            log::info!(
                "moss: PVH initrd 0x{:x}–0x{:x} ({} bytes)",
                module.mod_start,
                module.mod_end,
                module.mod_end - module.mod_start,
            );
        }
    }

    // --- 2. Extract kernel command line ---
    if info.cmdline != 0 {
        let cmdline_virt = info.cmdline.wrapping_add(KERNEL_BASE);
        // Read up to BOOT_CMDLINE_MAX bytes, looking for null terminator.
        let src = cmdline_virt as *const u8;
        let mut len: usize = 0;
        while len < BOOT_CMDLINE_MAX {
            let byte = unsafe { core::ptr::read(src.add(len)) };
            if byte == 0 {
                break;
            }
            len += 1;
        }

        if len > 0 && len < BOOT_CMDLINE_MAX {
            unsafe {
                let dst = addr_of_mut!(BOOT_CMDLINE) as *mut u8;
                core::ptr::copy_nonoverlapping(src, dst, len);
                *dst.add(len) = 0;
                *addr_of_mut!(BOOT_CMDLINE_LEN) = len;
            }
            // Log the cmdline (up to 128 chars for diagnostics).
            let log_len = len.min(128);
            let bytes = unsafe { slice::from_raw_parts(cmdline_virt as *const u8, log_len) };
            if let Ok(cmd) = core::str::from_utf8(bytes) {
                log::info!("moss: PVH boot cmdline: {}", cmd);
            }
        }
    }

    // --- 3. Parse memory map and register RAM regions ---
    if info.version >= 1 && info.memmap_paddr != 0 && info.memmap_entries > 0 {
        let memmap_virt = info.memmap_paddr.wrapping_add(KERNEL_BASE);
        let entries = memmap_virt as *const HvmMemmapEntry;
        let mut total_ram: u64 = 0;
        let mut regions_added: u32 = 0;

        for i in 0..info.memmap_entries as usize {
            let entry = unsafe { core::ptr::read(entries.add(i)) };

            if entry.typ == HVM_MEMMAP_TYPE_RAM && entry.size > 0 {
                let start = entry.addr as usize;
                let end = (entry.addr + entry.size) as usize;

                let region = PhysMemoryRegion::from_start_end_address(
                    PA::from_value(start),
                    PA::from_value(end),
                );

                let mut alloc = INITAL_ALLOCATOR.lock_save_irq();
                if let Some(ref mut a) = *alloc {
                    if let Err(e) = a.add_memory(region) {
                        log::warn!(
                            "moss: PVH failed to add RAM 0x{:x}–0x{:x}: {}",
                            start,
                            end,
                            e,
                        );
                    } else {
                        regions_added += 1;
                        total_ram += entry.size;
                    }
                }
            }
        }

        log::info!(
            "moss: PVH memory map: {} RAM regions, {} MiB total",
            regions_added,
            total_ram / (1024 * 1024),
        );
    } else {
        // Fallback: no valid memory map — use conservative 2 GiB mapping.
        log::info!(
            "moss: PVH no memmap (v{}), using conservative 2 GiB map",
            info.version
        );
        unsafe { init_pvh_memory() };
    }
}

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

                // Clamp start to past the kernel image so the frame allocator
                // doesn't allocate metadata on top of kernel code/data.
                let image_end_page = {
                    let e = unsafe { addr_of!(__image_end) as usize };
                    (e + 0xFFF) & !0xFFF  // page-align up
                };

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
                        // Clamp base to past the kernel image
                        let base = core::cmp::max(entry.base_addr, image_end_page as u64);
                        let end = entry.base_addr + entry.length;
                        if base < end {
                            let mut alloc = INITAL_ALLOCATOR.lock_save_irq();
                            if let Some(ref mut a) = *alloc {
                                let region = PhysMemoryRegion::from_start_end_address(
                                    PA::from_value(base as usize),
                                    PA::from_value(end as usize),
                                );
                                log::warn!(
                                    "mboot: MMAP region 0x{:x}–0x{:x} ({} KiB)",
                                    base,
                                    end,
                                    (end - base) / 1024,
                                );
                                if let Err(e) = a.add_memory(region) {
                                    log::warn!(
                                        "mboot: failed to add RAM region: {}",
                                        e,
                                    );
                                }
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
//  Multiboot1 info parsing (flags-based format)
// ──────────────────────────────────────────────

/// Read the Multiboot1 info header from a physical address.
///
/// Returns `None` if the physical address is NULL or the flags field is zero
/// (which implies the structure is not a valid Multiboot1 info).
///
/// # Safety
///
/// `info_phys` must point to a valid Multiboot1 info structure mapped by the
/// initial page tables.  Must only be called during single-threaded stage1.
unsafe fn read_mb1_info(info_phys: u64) -> Option<Mb1Info> {
    if info_phys == 0 {
        return None;
    }
    let info_virt = info_phys.wrapping_add(KERNEL_BASE);
    let info = unsafe { core::ptr::read(info_virt as *const Mb1Info) };
    if info.flags == 0 {
        return None;
    }
    Some(info)
}

/// Parse the Multiboot1 memory map and populate the physical frame allocator.
///
/// The memory map is at `info.mmap_addr` (physical) and consists of
/// `Mb1MmapEntry` records.  Each record's `size` field is "size of this
/// structure minus 4"; advance by `size + 4` to reach the next entry.
///
/// # Safety
///
/// Must be called once during single-threaded stage1 with a valid Multiboot1
/// info structure.
unsafe fn parse_mb1_memory_map(info_phys: u64) -> Result<()> {
    let info_virt = info_phys.wrapping_add(KERNEL_BASE);
    let info = unsafe { core::ptr::read(info_virt as *const Mb1Info) };

    if info.flags & MB1_FLAG_MMAP == 0 {
        log::warn!("mboot1: no memory map flag set, using fallback");
        unsafe { init_pvh_memory() };
        return Ok(());
    }

    let mmap_virt = (info.mmap_addr as u64).wrapping_add(KERNEL_BASE);
    let mmap_end = mmap_virt + info.mmap_length as u64;

    // Clamp start to past the kernel image so the frame allocator
    // doesn't allocate metadata on top of kernel code/data.
    let image_end_page = {
        let e = unsafe { addr_of!(__image_end) as usize };
        (e + 0xFFF) & !0xFFF // page-align up
    };

    let mut offset: u64 = 0;
    loop {
        let entry_addr = mmap_virt + offset;
        if entry_addr + core::mem::size_of::<Mb1MmapEntry>() as u64 > mmap_end {
            break;
        }
        let entry = unsafe { &*(entry_addr as *const Mb1MmapEntry) };

        if entry.typ == MB1_MMAP_TYPE_AVAIL && entry.length > 0 {
            // Clamp base to past the kernel image.
            let base = core::cmp::max(entry.base_addr, image_end_page as u64);
            let end = entry.base_addr + entry.length;
            if base < end {
                let mut alloc = INITAL_ALLOCATOR.lock_save_irq();
                if let Some(ref mut a) = *alloc {
                    let region = PhysMemoryRegion::from_start_end_address(
                        PA::from_value(base as usize),
                        PA::from_value(end as usize),
                    );
                    log::warn!(
                        "mboot1: MMAP region 0x{:x}–0x{:x} ({} KiB)",
                        base,
                        end,
                        (end - base) / 1024,
                    );
                    if let Err(e) = a.add_memory(region) {
                        log::warn!("mboot1: failed to add RAM region: {}", e);
                    }
                }
            }
        }

        // Advance: size is "size of structure minus 4".
        offset += entry.size as u64 + 4;
    }

    Ok(())
}

/// Parse Multiboot1 modules to find the initrd.
///
/// Module entries are at `info.mods_addr` (physical), `info.mods_count` entries
/// of 16 bytes each.  The first module's physical address range is stored in
/// `INITRD_START` / `INITRD_END`.
///
/// # Safety
///
/// Must be called once during single-threaded stage1 with a valid Multiboot1
/// info structure.
unsafe fn parse_mb1_modules(info_phys: u64) {
    let info_virt = info_phys.wrapping_add(KERNEL_BASE);
    let info = unsafe { core::ptr::read(info_virt as *const Mb1Info) };

    if info.flags & MB1_FLAG_MODS == 0 || info.mods_count == 0 {
        log::warn!("mboot1: no modules found");
        return;
    }

    let mods_virt = (info.mods_addr as u64).wrapping_add(KERNEL_BASE);
    let mods_end =
        mods_virt + info.mods_count as u64 * core::mem::size_of::<Mb1ModEntry>() as u64;

    let mut offset: u64 = 0;
    while mods_virt + offset + core::mem::size_of::<Mb1ModEntry>() as u64 <= mods_end {
        let entry = unsafe { &*((mods_virt + offset) as *const Mb1ModEntry) };
        let mod_start = entry.mod_start as u64;
        let mod_end = entry.mod_end as u64;

        if mod_start > 0 && mod_end > mod_start {
            // Store the first module as the initrd.
            unsafe {
                INITRD_START = mod_start;
                INITRD_END = mod_end;
            }

            // Log module name (null-terminated string at cmdline pointer).
            if entry.cmdline != 0 {
                let name_virt = (entry.cmdline as u64).wrapping_add(KERNEL_BASE);
                let name_bytes = unsafe {
                    core::slice::from_raw_parts(name_virt as *const u8, 256)
                };
                let name_len = name_bytes.iter().position(|&b| b == 0).unwrap_or(255);
                if let Ok(name) = core::str::from_utf8(&name_bytes[..name_len]) {
                    log::info!(
                        "mboot1: module '{}' at 0x{:x}–0x{:x} ({} bytes)",
                        name,
                        mod_start,
                        mod_end,
                        mod_end - mod_start,
                    );
                }
            }
        }
        offset += core::mem::size_of::<Mb1ModEntry>() as u64;
    }
}

/// Copy the command line from a Multiboot1 info structure into the BSS
/// static `BOOT_CMDLINE` so it survives the stage1 → stage2 transition.
///
/// # Safety
///
/// `info_phys` must point to a valid Multiboot1 info structure mapped by the
/// initial page tables.  Must be called before the boot stack is reclaimed.
///
/// # Returns
///
/// `true` if a non-empty command line was stored.
unsafe fn store_mb1_cmdline(info_phys: u64) -> bool {
    let info_virt = info_phys.wrapping_add(KERNEL_BASE);
    let info = unsafe { core::ptr::read(info_virt as *const Mb1Info) };

    if info.flags & MB1_FLAG_CMDLINE == 0 || info.cmdline == 0 {
        return false;
    }

    let str_virt = (info.cmdline as u64).wrapping_add(KERNEL_BASE);
    let bytes = unsafe { core::slice::from_raw_parts(str_virt as *const u8, 256) };
    let len = bytes.iter().position(|&b| b == 0).unwrap_or(255);

    if len == 0 || len >= BOOT_CMDLINE_MAX {
        return false;
    }

    unsafe {
        let dst = addr_of_mut!(BOOT_CMDLINE) as *mut u8;
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, len);
        *dst.add(len) = 0;
        *addr_of_mut!(BOOT_CMDLINE_LEN) = len;
    }
    true
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
/// Second-stage architecture initialisation.
///
/// Called from `start.S` after switching to the kernel stack
/// returned by `arch_init_stage1`. Runs on the proper kernel stack.
///
/// # Safety
///
/// Must only be called once, after `arch_init_stage1` has completed.
#[unsafe(no_mangle)]
unsafe extern "C" fn arch_init_stage1(mboot_info_ptr: u64) -> u64 {
    boot_diag(b'1'); // '1' — stage1 entry

    // Set up the early console logger so log::info! and friends work.
    crate::console::setup_console_logger();
    boot_diag(b'A'); // 'A' — logger set up

    // Detect boot protocol: boot_eax was saved by start.S before BSS zero.
    // 0x36d76289 = Multiboot2 (GRUB), 0x2BADB009 = Multiboot1 (QEMU -kernel),
    // 0 = PVH (QEMU -kernel with ELF).
    let boot_eax_val = unsafe { boot_eax };

    if boot_eax_val == MB1_REG_MAGIC {
        // ---- Multiboot1 boot path (QEMU -kernel) ----
        log::info!("moss: x86_64 boot (Multiboot1/QEMU)");
        boot_diag(b'M'); // 'M' — Multiboot1 path

        // Store the command line into BSS for stage2 access.
        if unsafe { store_mb1_cmdline(mboot_info_ptr) } {
            let len = unsafe { BOOT_CMDLINE_LEN };
            let bytes = unsafe { &BOOT_CMDLINE[..len] };
            if let Ok(cmd) = core::str::from_utf8(bytes) {
                log::info!("moss: boot cmdline: {}", cmd);
            }
        }
        boot_diag(b'C'); // 'C' — cmdline parsed

        // Parse Multiboot1 modules — extract initrd physical address range.
        unsafe { parse_mb1_modules(mboot_info_ptr) };
        boot_diag(b'K'); // 'K' — modules parsed

        // Set up physical frame allocator from Multiboot1 memory map.
        unsafe {
            parse_mb1_memory_map(mboot_info_ptr)
                .expect("parse_mb1_memory_map: failed to initialise frame allocator");
        }
        boot_diag(b'F'); // 'F' — frame allocator done
    } else if boot_eax_val == MB2_REG_MAGIC {
        // ---- Multiboot2 boot path (GRUB) ----
        log::info!("moss: x86_64 boot (Multiboot2)");
        boot_diag(b'B'); // 'B' — Multiboot2 path

        // Store the command line into BSS for stage2 access.
        if unsafe { store_cmdline(mboot_info_ptr) } {
            // SAFETY: BOOT_CMDLINE was populated by store_cmdline above.
            let len = unsafe { BOOT_CMDLINE_LEN };
            let bytes = unsafe { &BOOT_CMDLINE[..len] };
            if let Ok(cmd) = core::str::from_utf8(bytes) {
                log::info!("moss: boot cmdline: {}", cmd);
            }
        }
        boot_diag(b'C'); // 'C' — cmdline parsed

        // Parse Multiboot2 modules — extract initrd physical address range.
        unsafe { parse_modules(mboot_info_ptr) };
        boot_diag(b'K'); // 'K' — modules parsed

        // Set up physical frame allocator from Multiboot2 memory map (tag type 6).
        // SAFETY: Called during single-threaded stage1 boot; mboot_info_ptr is valid
        // and points to the Multiboot2 information structure provided by the bootloader.
        unsafe {
            parse_memory_map(mboot_info_ptr)
                .expect("parse_memory_map: failed to initialise frame allocator");
        }
        boot_diag(b'F'); // 'F' — frame allocator done
    } else {
        // ---- PVH boot path (QEMU -kernel with ELF) ----
        log::info!("moss: x86_64 boot (PVH/QEMU)");
        boot_diag(b'P'); // 'P' — PVH path

        // Parse hvm_start_info: extract initrd, cmdline, and memory map.
        // mboot_info_ptr contains the hvm_start_info physical address from EBX.
        if mboot_info_ptr != 0 {
            unsafe { init_pvh_boot_info(mboot_info_ptr) };
        } else {
            // PVH but no start_info — shouldn't happen with modern QEMU.
            log::warn!("moss: PVH but hvm_start_info is NULL, using fallback");
            unsafe { init_pvh_memory() };
        }
        boot_diag(b'M'); // 'M' — memory init done
    }

    boot_diag(b'S'); // 'S' — setting up kern addr space

    // Set up the kernel address space using the page tables built in start.S.
    // SAFETY: __init_pages_start is an extern static defined in start.S, valid
    // during stage1 after page tables are constructed.
    let init_pages_pa = unsafe { TPA::from_value(addr_of!(__init_pages_start) as usize) };
    setup_kern_addr_space(init_pages_pa).expect("setup_kern_addr_space failed");
    boot_diag(b'A'); // 'A' — kern addr space done (was 'K' to avoid clash with modules)

    log::info!("moss: stage1 complete, switching stack");
    boot_diag(b'2'); // '2' — stage1 complete, going to stage2

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
    boot_diag(b'a'); // 'a' — stage2 entry
    boot_diag(b'A'); // 'A' — about to take INITAL_ALLOCATOR

    // Initialize the frame allocator from the early bootstrap allocator
    // (mirrors ARM64 arch_init_stage2 flow).
    let smalloc = crate::memory::INITAL_ALLOCATOR
        .lock_save_irq()
        .take()
        .expect("INITAL_ALLOCATOR already consumed");
    boot_diag(b'B'); // 'B' — smalloc taken
    // SAFETY: FrameAllocator::init is safe to call here — we are in single-threaded
    // stage-2 init with exclusive ownership of `smalloc` and no other CPU is active.
    let (page_alloc, frame_list) = unsafe { crate::memory::FrameAllocator::init(smalloc) };
    boot_diag(b'C'); // 'C' — FrameAllocator::init done
    crate::memory::PAGE_ALLOC
        .set(page_alloc)
        .unwrap_or_else(|_| panic!("PAGE_ALLOC already set"));
    super::memory::heap::SLAB_ALLOC
        .set(libkernel::memory::allocators::slab::allocator::SlabAllocator::new(frame_list))
        .unwrap_or_else(|_| panic!("SLAB_ALLOC already set"));
    boot_diag(b'b'); // 'b' — slab allocator set

    // Initialize exceptions: IDT + syscall entry (Issue #16).
    crate::arch::x86_64::exceptions::exceptions_init().expect("exceptions init failed");
    boot_diag(b'c'); // 'c' — exceptions (IDT) done

    // Mask all 8259 PIC interrupts before enabling interrupts.
    // SeaBIOS leaves the legacy 8259 PIC active with its default mapping
    // (IRQ0→vector 0x08, etc.), which overlaps CPU exception vectors.
    // The LAPIC/I/O APIC drivers (loaded in run_initcalls) will take over
    // interrupt delivery.  Masking here prevents the stale 8259 PIC from
    // firing vectors that conflict with our IDT (e.g., IRQ0→#DF with IST1).
    unsafe {
        core::arch::asm!(
            "out dx, al",  // master PIC mask (port 0x21)
            in("dx") 0x21u16,
            in("al") 0xFFu8,
        );
        core::arch::asm!(
            "out dx, al",  // slave PIC mask (port 0xA1)
            in("dx") 0xA1u16,
            in("al") 0xFFu8,
        );
    }
    boot_diag(b'P'); // 'P' — PIC masked

    // Initialize the per-CPU heap (sets IA32_GS_BASE MSR to point at the
    // per-CPU SlabCache).  This MUST happen after exceptions_init() because
    // setup_boot_gdt_tss() clears the GS segment selector (mov gs, ax=0),
    // which in some environments can interfere with the MSR.  We also need
    // this before enable_interrupts() and run_initcalls() because every
    // allocation goes through the slab cache via GS.
    super::memory::heap::KernelHeap::init_for_this_cpu();
    boot_diag(b'g'); // 'g' — per-cpu heap (GS base) set

    // Disable the local APIC before enabling interrupts.
    // SeaBIOS leaves the LAPIC in an unknown state — it may have LINT0/LINT1
    // configured to deliver interrupts, or pending vectors.  If an IRQ fires
    // before the LAPIC driver in run_initcalls() sets up the interrupt root,
    // the handler panics (no root controller) and the panic path itself
    // triple-faults.
    //
    // We clear bit 11 (APIC Software Disable) in IA32_APIC_BASE MSR (0x1B).
    // The x86_64_lapic_init driver will re-enable it later by setting this
    // bit and configuring all LVT entries.
    unsafe { disable_lapic_msr(); }
    boot_diag(b'z'); // 'z' — LAPIC disabled via MSR

    // Enable hardware interrupts so that driver IRQ handlers can fire.
    // Must happen before run_initcalls() so that interrupt-driven drivers
    // can claim their IRQs.
    ArchImpl::enable_interrupts();
    boot_diag(b'd'); // 'd' — interrupts enabled
    boot_diag(b'R'); // 'R' — about to call run_initcalls()

    // Run all kernel_driver! init functions (LAPIC, I/O APIC, UART, HPET,
    // LAPIC timer, etc.).  The LAPIC init must link first so that the
    // interrupt root is established before drivers that call
    // `get_interrupt_root()` (e.g. the UART).
    //
    // This mirrors the ARM64 arch_init_stage2() flow:
    //   exceptions_init → enable_interrupts → run_initcalls → kmain
    unsafe { run_initcalls() };
    boot_diag(b'e'); // 'e' — initcalls done

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
    boot_diag(b'f'); // 'f' — about to vdso_init

    if let Err(e) = vdso_init() {
        log::error!("vdso: {}", e);
    }
    boot_diag(b'g'); // 'g' — vdso done, about to kmain

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
