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



    // 4. Identity-map LAPIC MMIO (physical 0xFEE00000) so interrupts::init() can access it
    map_range(
        pml4_pa,
        MapAttributes {
            phys: PhysMemoryRegion::new(PA::from_value(0xFEE00000), 4096),
            virt: VirtMemoryRegion::new(VA::from_value(0xFEE00000), 4096),
            mem_type: MemoryType::Normal,
            perms: PtePermissions::rw(false),
        },
        &mut ctx,
    )
    .unwrap();





    // Load new CR3
    unsafe {
        core::arch::asm!("mov cr3, rax", in("rax") pml4_pa.value());
    }



    setup_kern_addr_space(pml4_pa).unwrap();


}

// Storage for boot cmdline across stages
static BOOT_CMDLINE: OnceLock<String> = OnceLock::new();

// Physical region of the initrd (first multiboot2 module), set in arch_init_stage1
pub static INITRD_REGION: OnceLock<PhysMemoryRegion> = OnceLock::new();

#[unsafe(no_mangle)]
pub extern "C" fn arch_init_stage1(
    mb_info_ptr: usize,
    _image_start: usize,
    _image_end: usize,
) -> ! {
    // Guard clause: check for null/invalid multiboot info pointer
    // This happens when using QEMU's -kernel option instead of multiboot
    let mb_info_valid = mb_info_ptr != 0;

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
    let boot_info = if mb_info_valid {
        unsafe { Multiboot::from_ptr(mb_info_ptr as PAddr, &mut mem) }
    } else {
        None
    };

    // Extract initrd and cmdline only if multiboot info is valid
    if let Some(ref info) = boot_info {
        // Remember the first module (initrd)
        if let Some(mut modules) = info.modules() {
            if let Some(module) = modules.next() {
                let start = PA::from_value(module.start as usize);
                let size = (module.end - module.start) as usize;
                INITRD_REGION.set(PhysMemoryRegion::new(start, size)).ok();
            }
        }

        // Extract cmdline
        let cmdline = info
            .command_line()
            .map(|s| alloc::string::String::from(s))
            .unwrap_or_default();
        BOOT_CMDLINE.set(cmdline).ok();
    }

    // Set up page tables (also maps LAPIC MMIO)
    bootstrap_memory(_image_start, _image_end);

    // Set GS base to valid per-CPU storage BEFORE any heap use
    setup_gs_base();

    // Populate INITAL_ALLOCATOR from the multiboot memory map or use default values
    {
        let mut alloc = INITAL_ALLOCATOR.lock_save_irq();
        let alloc = alloc.as_mut().unwrap();

        if let Some(ref info) = boot_info {
            // Try to get memory regions from multiboot info
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

    // Build cmdline from multiboot command line
    let cmdline_str = BOOT_CMDLINE.get().cloned().unwrap_or_default();

    crate::kmain(cmdline_str, core::ptr::null_mut());

    #[allow(clippy::never_loop)]
    loop {
        X86_64::halt();
    }
}

pub fn get_cmdline() -> Option<String> {
    BOOT_CMDLINE.get().cloned()
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

    crate::console::set_active_console(
        uart.clone(),
        libkernel::driver::CharDevDescriptor {
            major: crate::drivers::ReservedMajors::Uart as _,
            minor: 0,
        },
    )
    .expect("Failed to set active console");
}
