
use crate::arch::x86_64::memory::mmu::{
    page_mapper::PageOffsetPgTableMapper, setup_kern_addr_space,
};
use alloc::vec::Vec;
use crate::console::setup_console_logger;
use crate::memory::{PageOffsetTranslator, INITAL_ALLOCATOR, PAGE_ALLOC};
use crate::sync::OnceLock;
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
use multiboot2::BootInformation;
use crate::arch::x86_64::X86_64;
use crate::arch::x86_64::memory::heap::SLAB_ALLOC;

const STATIC_PAGE_COUNT: usize = 64;
static mut BOOT_PAGES: [[u8; 4096]; STATIC_PAGE_COUNT] = [[0; 4096]; STATIC_PAGE_COUNT];
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
            let ptr = BOOT_PAGES[BOOT_PAGES_ALLOCATED].as_mut_ptr();
            BOOT_PAGES_ALLOCATED += 1;
            ptr::write_bytes(ptr, 0, 4096);

            let va = ptr as usize;
            let pa = va - crate::arch::x86_64::memory::IMAGE_BASE.value();
            Ok(TPA::from_value(pa))
        }
    }
}

pub fn bootstrap_memory(image_start: usize, image_end: usize) {
    // Use the BootPageAllocator for bootstrap memory management
    // This was set up in paging_bootstrap.rs
    
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
    let kernel_start_pa = 0x100000;
    let kernel_size = 2 * 1024 * 1024;

    map_range(
        pml4_pa,
        MapAttributes {
            phys: PhysMemoryRegion::new(PA::from_value(kernel_start_pa), kernel_size),
            virt: VirtMemoryRegion::new(VA::from_value(0xffffffff80100000), kernel_size),
            mem_type: MemoryType::Normal,
            // TODO: W^X - split into separate rx (.text) and rw (.data/.bss) mappings
            // using linker script symbols once section layout is finalized.
            perms: PtePermissions::rwx(false),
        },
        &mut ctx,
    ).unwrap();

    // 2. Setup Linear Map (first 1GiB)
    map_range(
        pml4_pa,
        MapAttributes {
            phys: PhysMemoryRegion::new(PA::from_value(0), 1024 * 1024 * 1024),
            virt: VirtMemoryRegion::new(VA::from_value(X86_64::PAGE_OFFSET), 1024 * 1024 * 1024),
            mem_type: MemoryType::Normal,
            perms: PtePermissions::rw(false),
        },
        &mut ctx,
    ).unwrap();

    // 3. Identity map first 2MiB (to keep running during boot transition)
    map_range(
        pml4_pa,
        MapAttributes {
            phys: PhysMemoryRegion::new(PA::from_value(0), 2 * 1024 * 1024),
            virt: VirtMemoryRegion::new(VA::from_value(0), 2 * 1024 * 1024),
            mem_type: MemoryType::Normal,
            perms: PtePermissions::rx(false),
        },
        &mut ctx,
    ).unwrap();

    // Load new CR3
    unsafe {
        core::arch::asm!("mov cr3, {}", in(reg) pml4_pa.value());
    }

    log::info!("New page tables loaded!");
    setup_kern_addr_space(pml4_pa).unwrap();
}

// Storage for boot info across stages
static BOOT_INFO: OnceLock<BootInformation> = OnceLock::new();

#[unsafe(no_mangle)]
pub extern "C" fn arch_init_stage1(mb_info_ptr: usize, _image_start: usize, _image_end: usize) -> ! {
    // Stage 1 must initialize its own minimal logging if needed,
    // but here we assume it was already set up or we'll set it up soon.
    
    let boot_info = unsafe { multiboot2::BootInformation::load(mb_info_ptr as *const _).expect("Failed to load Multiboot2 info") };
    
    // Store boot info for stage2
    BOOT_INFO.set(boot_info).ok();

    bootstrap_memory(_image_start, _image_end);

    log::info!("Setting up exceptions");
    crate::arch::x86_64::exceptions::exceptions_init().expect("Failed to init exceptions");

    log::info!("Setting up interrupts");
    crate::arch::x86_64::interrupts::init();

    log::info!("Enabling interrupts");
    X86_64::enable_interrupts();

    setup_gs_base();
    arch_init_stage2();

    loop {
        X86_64::halt();
    }
}

pub fn setup_gs_base() {
    unsafe {
        core::arch::asm!("wrmsr", in("ecx") 0xC0000101u32, in("eax") 0u32, in("edx") 0u32);
    }
}

pub fn arch_init_stage2() {
    // Early debug: write to serial port
    unsafe {
        // Write 'S' to serial port 0x3F8 (COM1)
        let _ = core::ptr::write_volatile(0x3F8 as *mut u8, b'S');
    }
    
    // Get boot info
    let boot_info = BOOT_INFO.get().expect("Boot info not set");
    
    // Early debug: write '1'
    unsafe { let _ = core::ptr::write_volatile(0x3F8 as *mut u8, b'1'); }

    setup_serial();
    
    // Initialize INITAL_ALLOCATOR from multiboot2 memory map
    log::info!("Initializing memory allocator from Multiboot2");
    {
        let mut alloc = INITAL_ALLOCATOR.lock_save_irq();
        let alloc = alloc.as_mut().unwrap();
        
        // Early debug: write '2'
        unsafe { let _ = core::ptr::write_volatile(0x3F8 as *mut u8, b'2'); }
        
        // Add memory regions from Multiboot2 memory map
        if let Some(memory_map_tag) = boot_info.memory_map_tag() {
            for entry in memory_map_tag.memory_areas() {
                if entry.typ() == multiboot2::MemoryAreaType::Available {
                    let start = PA::from_value(entry.start_address() as usize);
                    let size = entry.size() as usize;
                    log::info!("Adding memory region: {:x} - {:x}", entry.start_address(), entry.start_address() + entry.size());
                    let _ = alloc.add_memory(PhysMemoryRegion::new(start, size));
                }
            }
        }
        
        // Reserve kernel image
        let kernel_start = PA::from_value(0x100000);
        let kernel_size = 0x200000;
        let _ = alloc.add_reservation(PhysMemoryRegion::new(kernel_start, kernel_size));
        
        // Reserve modules (initrd) if present
        for module in boot_info.module_tags() {
            let start = PA::from_value(module.start_address() as usize);
            let size = (module.end_address() - module.start_address()) as usize;
            log::info!("Reserving module: {:x} - {:x}", module.start_address(), module.end_address());
            let _ = alloc.add_reservation(PhysMemoryRegion::new(start, size));
        }
        
        // Allow reallocations now
        unsafe { alloc.permit_region_list_reallocs(); }
    }
    
    // Early debug: write '3'
    unsafe { let _ = core::ptr::write_volatile(0x3F8 as *mut u8, b'3'); }
    
    // Set up console logger
    log::info!("Setting up console logger");
    unsafe { setup_console_logger(); }
    
    // Early debug: write '4'
    unsafe { let _ = core::ptr::write_volatile(0x3F8 as *mut u8, b'4'); }
    
    log::info!("Stage 2 init complete - calling kmain");
    
    // Get cmdline
    let cmdline_str = boot_info.command_line_tag()
        .and_then(|tag| tag.cmdline().ok())
        .map(|cstr| alloc::string::String::from_utf8_lossy(cstr.as_bytes()).into_owned())
        .unwrap_or_default();
    
    // Early debug: write '5'
    unsafe { let _ = core::ptr::write_volatile(0x3F8 as *mut u8, b'5'); }
    
    // Call kmain
    crate::kmain(cmdline_str, core::ptr::null_mut());
    
    // Should not return
    // Early debug: write 'X' if we return
    unsafe { let _ = core::ptr::write_volatile(0x3F8 as *mut u8, b'X'); }
    loop {
        X86_64::halt();
    }
}

pub fn get_cmdline() -> Option<String> {
    BOOT_INFO.get()
        .and_then(|info| info.command_line_tag())
        .map(|tag| {
            let s = tag.cmdline().unwrap_or_default();
            String::from(s)
        })
}

pub fn setup_serial() {
    use crate::drivers::uart::ns16550::Ns16550;
    use crate::drivers::uart::Uart;
    use crate::interrupts::{get_interrupt_root, InterruptConfig, InterruptDescriptor, TriggerMode};

    let mut uart_hw = Ns16550::new(0x3F8);
    uart_hw.init();

    let root = get_interrupt_root().expect("Interrupt root not initialized");
    let uart = root.claim_interrupt(
        InterruptConfig {
            descriptor: InterruptDescriptor::Spi(4), // COM1 is usually IRQ 4
            trigger: TriggerMode::EdgeRising,
        },
        |handle| Uart::new(uart_hw, handle, "com1"),
    ).expect("Failed to claim UART interrupt");

    crate::console::set_active_console(uart.clone(), libkernel::driver::CharDevDescriptor {
        major: crate::drivers::ReservedMajors::Uart as _,
        minor: 0,
    }).expect("Failed to set active console");
}
