use crate::arch::x86_64::memory::heap::SLAB_ALLOC;
use crate::arch::x86_64::memory::mmu::{
    page_mapper::PageOffsetPgTableMapper, setup_kern_addr_space,
};
use crate::memory::{PageOffsetTranslator, INITAL_ALLOCATOR, PAGE_ALLOC};
use crate::sync::OnceLock;
use alloc::string::String;
use core::arch::asm;
use core::ptr;
use libkernel::arch::x86_64::memory::pg_descriptors::MemoryType;
use libkernel::arch::x86_64::memory::pg_tables::{
    map_range, MapAttributes, MappingContext, PML4Table, PageAllocator, PageTableMapper, PgTable,
    PgTableArray,
};
use libkernel::memory::address::{PA, TPA, TVA, VA};
use libkernel::memory::allocators::phys::FrameAllocator;
use libkernel::memory::allocators::slab::allocator::SlabAllocator;
use libkernel::memory::permissions::PtePermissions;
use libkernel::memory::region::{PhysMemoryRegion, VirtMemoryRegion};
use libkernel::{CpuOps, VirtualMemory};
use multiboot::information::{
    MemoryManagement, MemoryType as MultibootMemoryType, Multiboot, PAddr,
};

use crate::arch::x86_64::X86_64;

pub mod tss;
use crate::console::setup_console_logger;

/// Write a nibble (4 bits) as a hex character to the debug serial port
#[inline(always)]
fn debug_serial_puthex_nibble(nibble: u8) {
    let c = if nibble < 10 {
        b'0' + nibble
    } else {
        b'A' + nibble - 10
    };
    unsafe {
        core::arch::asm!("outb %al, %dx", in("al") c, in("dx") 0x3F8u16, options(att_syntax));
    }
}

/// Write a byte as two hex characters to the debug serial port
#[inline(always)]
fn debug_serial_puthex_byte(byte: u8) {
    debug_serial_puthex_nibble((byte >> 4) & 0xF);
    debug_serial_puthex_nibble(byte & 0xF);
}

/// Write a full address (usize) as hex to the debug serial port
#[inline(always)]
fn debug_serial_puthex_addr(addr: usize) {
    let bytes = addr.to_ne_bytes();
    for byte in bytes.iter().rev() {
        debug_serial_puthex_nibble((byte >> 4) & 0xF);
        debug_serial_puthex_nibble(byte & 0xF);
    }
}

/// Write a byte to the debug serial port
#[inline(always)]
fn debug_serial_putchar(c: u8) {
    unsafe {
        core::arch::asm!("outb %al, %dx", in("al") c, in("dx") 0x3F8u16, options(att_syntax));
    }
}

#[repr(C, align(4096))]
#[derive(Copy, Clone)]
struct BootPage([u8; 4096]);

const STATIC_PAGE_COUNT: usize = 128;
static mut BOOT_PAGES: [BootPage; STATIC_PAGE_COUNT] = [BootPage([0u8; 4096]); STATIC_PAGE_COUNT];
static mut BOOT_PAGES_ALLOCATED: usize = 0;



struct BootPageAllocator;

impl PageAllocator for BootPageAllocator {
    fn allocate_page_table<T: PgTable>(
        &mut self,
    ) -> libkernel::error::Result<TPA<PgTableArray<T>>> {
        unsafe {
            if BOOT_PAGES_ALLOCATED >= STATIC_PAGE_COUNT {
                panic!("Out of boot pages!");
            }
            let ptr = BOOT_PAGES[BOOT_PAGES_ALLOCATED].0.as_mut_ptr();
            BOOT_PAGES_ALLOCATED += 1;
            ptr::write_bytes(ptr, 0, 4096);

            let va = ptr as usize;
            // Static pages are in the kernel image. The kernel is linked at
            // 0xffffffff80100000 and loaded at 0x100000 physical.
            // So PA = VA - 0xffffffff80000000.
            let pa = va - 0xffffffff80000000;

            Ok(TPA::from_value(pa))
        }
    }
}

pub fn bootstrap_memory(image_start: usize, image_end: usize) {
    let mut allocator = BootPageAllocator;



    struct IdentityPgTableMapper;
    impl PageTableMapper for IdentityPgTableMapper {
        unsafe fn with_page_table<T: PgTable, R>(
            &mut self,
            pa: TPA<PgTableArray<T>>,
            f: impl FnOnce(TVA<PgTableArray<T>>) -> R,
        ) -> libkernel::error::Result<R> {
            Ok(f(TVA::from_value(pa.value())))
        }
    }

    let pml4_pa = allocator.allocate_page_table::<PML4Table>().unwrap();
    let mut ctx = MappingContext {
        allocator: &mut allocator,
        mapper: &mut IdentityPgTableMapper,
    };



    // 1. Map Kernel Image
    let kernel_phys_base = image_start - 0xffffffff80000000;
    let kernel_size = (image_end - image_start + 0x1fffff) & !0x1fffff; // Align to 2MiB boundary
    


    map_range(
        pml4_pa,
        MapAttributes {
            phys: PhysMemoryRegion::new(PA::from_value(kernel_phys_base), kernel_size),
            virt: VirtMemoryRegion::new(VA::from_value(image_start), kernel_size),
            mem_type: MemoryType::Normal,
            perms: PtePermissions::rwx(false),
        },
        &mut ctx,
    )
    .unwrap();



    // 2. Linear map of first 4GiB (covers RAM and MMIO including LAPIC at 0xFEE00000)


    map_range(
        pml4_pa,
        MapAttributes {
            phys: PhysMemoryRegion::new(PA::from_value(0), 4 * 1024 * 1024 * 1024usize),
            virt: VirtMemoryRegion::new(
                VA::from_value(X86_64::PAGE_OFFSET),
                4 * 1024 * 1024 * 1024usize,
            ),
            mem_type: MemoryType::Normal,
            perms: PtePermissions::rw(false),
        },
        &mut ctx,
    )
    .unwrap();



    // 3. Identity map early memory including the whole kernel image
    // (to keep code, stack and BOOT_PAGES running during boot transition)
    let identity_size = (kernel_phys_base + kernel_size + 0x1fffff) & !0x1fffff;


    map_range(
        pml4_pa,
        MapAttributes {
            phys: PhysMemoryRegion::new(PA::from_value(0), identity_size),
            virt: VirtMemoryRegion::new(VA::from_value(0), identity_size),
            mem_type: MemoryType::Normal,
            perms: PtePermissions::rwx(false),
        },
        &mut ctx,
    )
    .unwrap();



    // Note: The LAPIC at physical 0xFEE0_0000 is accessible via the linear map at
    // 0xffff_8000_fee0_0000 (PAGE_OFFSET + phys). No separate identity map is needed;
    // using upper-half addresses ensures the mapping survives CR3 switches to
    // process page tables (which only copy upper-half PML4 entries 256–511).

    // Load new CR3
    unsafe {
        core::arch::asm!("mov cr3, rax", in("rax") pml4_pa.value());
    }



    setup_kern_addr_space(pml4_pa).unwrap();


}

// Storage for boot cmdline across stages.
// We cannot use a heap-allocated String here because arch_init_stage1 runs
// before the allocator is initialized.  Instead we copy the raw bytes into a
// fixed-size static buffer and build the String later in arch_init_stage2.
const CMDLINE_BUF_LEN: usize = 4096;
static mut BOOT_CMDLINE_BUF: [u8; CMDLINE_BUF_LEN] = [0u8; CMDLINE_BUF_LEN];
static BOOT_CMDLINE_LEN: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);

fn store_cmdline_raw(s: &str) {
    let bytes = s.as_bytes();
    let len = bytes.len().min(CMDLINE_BUF_LEN);
    unsafe {
        core::ptr::copy_nonoverlapping(
            bytes.as_ptr(),
            core::ptr::addr_of_mut!(BOOT_CMDLINE_BUF) as *mut u8,
            len,
        );
    }
    BOOT_CMDLINE_LEN.store(len, core::sync::atomic::Ordering::Relaxed);
}

fn load_cmdline_string() -> String {
    let len = BOOT_CMDLINE_LEN.load(core::sync::atomic::Ordering::Relaxed);
    let bytes = unsafe { core::slice::from_raw_parts(core::ptr::addr_of!(BOOT_CMDLINE_BUF) as *const u8, len) };
    String::from(core::str::from_utf8(bytes).unwrap_or(""))
}

// Physical region of the initrd (first multiboot2 module), set in arch_init_stage1
pub static INITRD_REGION: OnceLock<PhysMemoryRegion> = OnceLock::new();

/// Magic value placed in EAX by QEMU/Xen when booting via the PVH entry point.
const XEN_HVM_START_MAGIC: u32 = 0x336e_c578;

/// Xen/PVH hvm_start_info v0 (QEMU only ever uses v0).
#[repr(C)]
struct HvmStartInfo {
    magic: u32,
    version: u32,
    flags: u32,
    nr_modules: u32,
    modlist_paddr: u64,
    cmdline_paddr: u64,
    rsdp_paddr: u64,
}

/// Entry in the hvm_modlist (one per initrd module).
#[repr(C)]
struct HvmModlistEntry {
    paddr: u64,
    size: u64,
    cmdline_paddr: u64,
    _reserved: u64,
}

#[unsafe(no_mangle)]
pub extern "C" fn arch_init_stage1(
    mb_info_ptr: usize,
    _image_start: usize,
    _image_end: usize,
    boot_magic: u32,
) -> ! {
    // Early serial: print boot magic and info ptr before any heap use.
    // Format: "BM:" <8-hex-digits> " BI:" <16-hex-digits> "\n"
    debug_serial_putchar(b'B');
    debug_serial_putchar(b'M');
    debug_serial_putchar(b':');
    debug_serial_puthex_byte((boot_magic >> 24) as u8);
    debug_serial_puthex_byte((boot_magic >> 16) as u8);
    debug_serial_puthex_byte((boot_magic >> 8) as u8);
    debug_serial_puthex_byte(boot_magic as u8);
    debug_serial_putchar(b' ');
    debug_serial_putchar(b'B');
    debug_serial_putchar(b'I');
    debug_serial_putchar(b':');
    debug_serial_puthex_addr(mb_info_ptr);
    debug_serial_putchar(b'\r');
    debug_serial_putchar(b'\n');

    // Detect PVH by reading hvm_start_info.magic (first field at mb_info_ptr).
    // QEMU does NOT reliably set EAX to the PVH magic; the magic lives inside
    // the struct itself.  Multiboot2 is identified by EAX == 0x36d76289.
    let is_pvh = mb_info_ptr != 0
        && unsafe { (mb_info_ptr as *const u32).read() } == XEN_HVM_START_MAGIC;

    debug_serial_putchar(b' ');
    debug_serial_putchar(if is_pvh { b'P' } else { b'M' });
    debug_serial_putchar(b'\r');
    debug_serial_putchar(b'\n');

    if is_pvh {
        // ---- PVH boot path -----------------------------------------------
        // EBX → hvm_start_info; extract cmdline and first initrd module.
        let info = unsafe { &*(mb_info_ptr as *const HvmStartInfo) };

        debug_serial_putchar(b'C');
        debug_serial_putchar(b'P');
        debug_serial_putchar(b':');
        debug_serial_puthex_addr(info.cmdline_paddr as usize);
        debug_serial_putchar(b'\r');
        debug_serial_putchar(b'\n');

        if info.cmdline_paddr != 0 {
            let cmdline = unsafe {
                let ptr = info.cmdline_paddr as *const u8;
                let mut len = 0usize;
                while *ptr.add(len) != 0 {
                    len += 1;
                }
                core::str::from_utf8_unchecked(core::slice::from_raw_parts(ptr, len))
            };
            store_cmdline_raw(cmdline);
        }

        if info.nr_modules > 0 && info.modlist_paddr != 0 {
            let entry = unsafe { &*(info.modlist_paddr as *const HvmModlistEntry) };
            debug_serial_putchar(b'I');
            debug_serial_putchar(b'R');
            debug_serial_putchar(b':');
            debug_serial_puthex_addr(entry.paddr as usize);
            debug_serial_putchar(b'+');
            debug_serial_puthex_addr(entry.size as usize);
            debug_serial_putchar(b'\r');
            debug_serial_putchar(b'\n');
            let start = PA::from_value(entry.paddr as usize);
            let size = entry.size as usize;
            INITRD_REGION.set(PhysMemoryRegion::new(start, size)).ok();
        } else {
            debug_serial_putchar(b'I');
            debug_serial_putchar(b'R');
            debug_serial_putchar(b':');
            debug_serial_putchar(b'N');
            debug_serial_putchar(b'O');
            debug_serial_putchar(b'N');
            debug_serial_putchar(b'E');
            debug_serial_putchar(b'\r');
            debug_serial_putchar(b'\n');
        }
    } else {
        // ---- Multiboot2 boot path ----------------------------------------
        // boot_magic == 0x36d76289 if QEMU passed multiboot2 magic; fall
        // through here for any non-PVH case.
        struct IdentityMem;

        impl MemoryManagement for IdentityMem {
            unsafe fn paddr_to_slice(&self, addr: PAddr, size: usize) -> Option<&'static [u8]> {
                unsafe { Some(core::slice::from_raw_parts(addr as *const u8, size)) }
            }
            unsafe fn allocate(&mut self, _size: usize) -> Option<(PAddr, &mut [u8])> {
                None
            }
            unsafe fn deallocate(&mut self, _addr: PAddr) {}
        }

        let mut mem = IdentityMem;
        let boot_info = if mb_info_ptr != 0 {
            unsafe { Multiboot::from_ptr(mb_info_ptr as PAddr, &mut mem) }
        } else {
            None
        };

        if let Some(ref info) = boot_info {
            if let Some(mut modules) = info.modules() {
                if let Some(module) = modules.next() {
                    let start = PA::from_value(module.start as usize);
                    let size = (module.end - module.start) as usize;
                    INITRD_REGION.set(PhysMemoryRegion::new(start, size)).ok();
                }
            }

            let cmdline = info
                .command_line()
                .unwrap_or("");
            store_cmdline_raw(cmdline);
        }
    }

    // Snapshot the boot_info for the Multiboot2 memory-map walk below.
    // Re-derive it from mb_info_ptr so both branches can share the code.
    struct IdentityMem2;
    impl MemoryManagement for IdentityMem2 {
        unsafe fn paddr_to_slice(&self, addr: PAddr, size: usize) -> Option<&'static [u8]> {
            unsafe { Some(core::slice::from_raw_parts(addr as *const u8, size)) }
        }
        unsafe fn allocate(&mut self, _size: usize) -> Option<(PAddr, &mut [u8])> { None }
        unsafe fn deallocate(&mut self, _addr: PAddr) {}
    }
    let mut mem2 = IdentityMem2;
    let boot_info = if !is_pvh && mb_info_ptr != 0 {
        unsafe { Multiboot::from_ptr(mb_info_ptr as PAddr, &mut mem2) }
    } else {
        None
    };

    // Set up page tables (also maps LAPIC MMIO)
    bootstrap_memory(_image_start, _image_end);

    // Set GS base to valid per-CPU storage BEFORE any heap use
    setup_gs_base();

    // Populate INITAL_ALLOCATOR from the multiboot memory map or use default values
    {
        let mut alloc = INITAL_ALLOCATOR.lock_save_irq();
        let alloc = alloc.as_mut().unwrap();

        if let Some(ref info) = boot_info {
            // Try to get memory regions from multiboot2 info
            if let Some(regions) = info.memory_regions() {
                for entry in regions {
                    if entry.memory_type() == MultibootMemoryType::Available {
                        let start = PA::from_value(entry.base_address() as usize);
                        let size = entry.length() as usize;
                        let _ = alloc.add_memory(PhysMemoryRegion::new(start, size));
                    }
                }
            }
        }
        // PVH boot does not provide a Multiboot2 memory map; the fallback
        // 256 MB region below covers the QEMU default.

        // If no memory was added (or multiboot info invalid), use default/builtin memory map
        // This provides memory from 1MB to 256MB for QEMU -kernel mode
        if alloc.base_ram_base_address().is_none() {
            let default_start = PA::from_value(0x100000); // 1MB
            let default_size = 256 * 1024 * 1024; // 256MB
            let _ = alloc.add_memory(PhysMemoryRegion::new(default_start, default_size));
        }

        // Reserve kernel image
        let kernel_image_phys_start = PA::from_value(_image_start - 0xffffffff80000000);
        let kernel_image_size = _image_end - _image_start;
        let _ = alloc.add_reservation(PhysMemoryRegion::new(kernel_image_phys_start, kernel_image_size));

        // Also reserve the local APIC region here to be doubly sure.  The
        // initial allocator may already have reserved it above, but once we
        // transition to the full page allocator we still want to make sure
        // the APIC space isn't used as normal RAM.
        const LOCAL_APIC_PHYS: usize = 0xFEE0_0000;
        const LOCAL_APIC_SIZE: usize = 0x1000;
        let _ = alloc.add_reservation(PhysMemoryRegion::new(
            PA::from_value(LOCAL_APIC_PHYS),
            LOCAL_APIC_SIZE,
        ));

        // Reserve all multiboot modules (initrd) if available
        if let Some(ref info) = boot_info {
            if let Some(modules) = info.modules() {
                for module in modules {
                    let start = PA::from_value(module.start as usize);
                    let size = (module.end - module.start) as usize;
                    let _ = alloc.add_reservation(PhysMemoryRegion::new(start, size));
                }
            }
        }

        // For PVH boot, reserve the initrd region we extracted earlier.
        if let Some(region) = INITRD_REGION.get().copied() {
            let _ = alloc.add_reservation(region);
        }

        unsafe {
            alloc.permit_region_list_reallocs();
        }
    }

    // Promote to full page/slab allocators and init the per-CPU heap.
    // Must happen before any Box/Arc/Vec allocation (e.g. in interrupts::init).
    {
        use crate::arch::x86_64::memory::heap::KernelHeap;

        let smalloc = INITAL_ALLOCATOR
            .lock_save_irq()
            .take()
            .expect("INITAL_ALLOCATOR already taken");

        let (page_alloc, frame_list) = unsafe { FrameAllocator::init(smalloc) };

        if PAGE_ALLOC.set(page_alloc).is_err() {
            panic!("Cannot setup physical memory allocator");
        }
        if SLAB_ALLOC.set(SlabAllocator::new(frame_list)).is_err() {
            panic!("Cannot setup slab allocator");
        }

        KernelHeap::init_for_this_cpu();
    }

    // Initialize per-CPU variables (requires heap)
    unsafe {
        libkernel::sync::per_cpu::setup_percpu(1);
    }

    // Run kernel driver initcalls (registers null/zero/console/uart chardevs, etc.)
    unsafe {
        crate::drivers::init::run_initcalls();
    }

    // Install the IDT
    crate::arch::x86_64::exceptions::exceptions_init().expect("Failed to init exceptions");

    // Initialize interrupt controller and timer (requires Arc allocation → needs heap)
    crate::arch::x86_64::interrupts::init();

    X86_64::enable_interrupts();

    arch_init_stage2();

    #[allow(clippy::never_loop)]
    loop {
        X86_64::halt();
    }
}

// Per-CPU storage: first qword holds the SlabCache pointer written by KernelHeap::init_for_this_cpu()
static mut PER_CPU_STORAGE: [u8; 8] = [0u8; 8];

pub fn setup_gs_base() {
    let addr = unsafe { core::ptr::addr_of_mut!(PER_CPU_STORAGE) as u64 };
    unsafe {
        core::arch::asm!(
            "wrmsr",
            in("ecx") 0xC0000101u32,  // IA32_GS_BASE
            in("eax") (addr & 0xFFFFFFFF) as u32,
            in("edx") (addr >> 32) as u32,
        );
    }
}

pub fn arch_init_stage2() {
    // Set up serial UART (requires allocator + interrupt controller)
    setup_serial();

    // Set up console logger
    unsafe {
        setup_console_logger();
    }

    log::info!("x86_64: boot stage 2 complete");

    // Set up the TSS and reload the GDT so that ring-3 exceptions have a
    // valid kernel stack (RSP0) to switch to.
    unsafe { tss::tss_init() };

    // Configure the SYSCALL instruction (STAR, LSTAR, SFMASK, EFER.SCE)
    // so that musl's native syscall ABI works.
    unsafe { tss::syscall_init() };

    // Build cmdline string now that the heap is initialized.
    let cmdline_str = load_cmdline_string();

    // The early boot path has no saved userspace context, but
    // `kmain`/`dispatch_userspace_task` expect a valid pointer.  Allocate a
    // temporary `UserCtx` on the stack and pass its address so that the
    // scheduler can write a context into it if necessary.  The value is not
    // otherwise used by any code on the boot path.
    let mut initial_ctx: crate::process::ctx::UserCtx = unsafe { core::mem::zeroed() };
    crate::kmain(cmdline_str, &mut initial_ctx as *mut _);

    // `dispatch_userspace_task` has written the init process's register state
    // into `initial_ctx`.  Use the boot-time iretq trampoline to jump to
    // userspace.  This never returns.
    log::info!("boot: jumping to userspace RIP=0x{:x} RSP=0x{:x} CS=0x{:x} SS=0x{:x}",
        initial_ctx.rip, initial_ctx.rsp, initial_ctx.cs, initial_ctx.ss);
    crate::arch::x86_64::exceptions::boot_jump_to_userspace_wrapper(&initial_ctx);
}

pub fn get_cmdline() -> Option<String> {
    let len = BOOT_CMDLINE_LEN.load(core::sync::atomic::Ordering::Relaxed);
    if len == 0 {
        None
    } else {
        Some(load_cmdline_string())
    }
}

pub fn setup_serial() {
    use crate::drivers::uart::ns16550::Ns16550;
    use crate::drivers::uart::Uart;
    use crate::interrupts::{
        get_interrupt_root, InterruptConfig, InterruptDescriptor, TriggerMode,
    };

    let mut uart_hw = Ns16550::new(0x3F8);
    uart_hw.init();

    let root = get_interrupt_root().expect("Interrupt root not initialized");
    let uart = root
        .claim_interrupt(
            InterruptConfig {
                descriptor: InterruptDescriptor::Spi(4), // COM1 is IRQ 4
                trigger: TriggerMode::EdgeRising,
            },
            |handle| Uart::new(uart_hw, handle, "com1"),
        )
        .expect("Failed to claim UART interrupt");

    // Register this UART instance with the generic UART char driver so
    // that userspace can open it via /dev/ttyS* (and console).  The
    // register_console helper allocates a minor number and creates the
    // corresponding devfs node.
    // Use the public helper which handles the global static internally.
    let _desc = crate::drivers::uart::register_uart_console(uart.clone(), true)
        .expect("Failed to register UART as char device");
}
