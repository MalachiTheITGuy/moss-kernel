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
    map_range(
        pml4_pa,
        MapAttributes {
            phys: PhysMemoryRegion::new(PA::from_value(0x100000), 2 * 1024 * 1024),
            virt: VirtMemoryRegion::new(VA::from_value(0xffffffff80000000), 2 * 1024 * 1024),
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
    // unsafe {
    //     asm!("outb %al, %dx", in(reg_byte) 40u8);
    // }
    unsafe {
        core::arch::asm!("outb %al, %dx", in("al") b'A', in("dx") 0x3F8u16, options(att_syntax));
    }
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
    let boot_info = unsafe {
        Multiboot::from_ptr(mb_info_ptr as PAddr, &mut mem).expect("Failed to load Multiboot info")
    };

    unsafe {
        core::arch::asm!("outb %al, %dx", in("al") b'B', in("dx") 0x3F8u16, options(att_syntax));
    }
    // Remember the first module (initrd) before we store boot_info
    if let Some(mut modules) = boot_info.modules() {
        if let Some(module) = modules.next() {
            let start = PA::from_value(module.start as usize);
            let size = (module.end - module.start) as usize;
            INITRD_REGION.set(PhysMemoryRegion::new(start, size)).ok();
        }
    }

    // Extract cmdline
    let cmdline = boot_info
        .command_line()
        .map(|s| alloc::string::String::from(s))
        .unwrap_or_default();
    BOOT_CMDLINE.set(cmdline).ok();

    unsafe {
        core::arch::asm!("outb %al, %dx", in("al") b'C', in("dx") 0x3F8u16, options(att_syntax));
    }
    // Set up page tables (also maps LAPIC MMIO)
    bootstrap_memory(_image_start, _image_end);

    unsafe {
        core::arch::asm!("outb %al, %dx", in("al") b'D', in("dx") 0x3F8u16, options(att_syntax));
    }
    // Set GS base to valid per-CPU storage BEFORE any heap use
    setup_gs_base();

    unsafe {
        core::arch::asm!("outb %al, %dx", in("al") b'E', in("dx") 0x3F8u16, options(att_syntax));
    }
    // Populate INITAL_ALLOCATOR from the multiboot memory map
    {
        let mut alloc = INITAL_ALLOCATOR.lock_save_irq();
        let alloc = alloc.as_mut().unwrap();

        if let Some(regions) = boot_info.memory_regions() {
            for entry in regions {
                if entry.memory_type() == MultibootMemoryType::Available {
                    let start = PA::from_value(entry.base_address() as usize);
                    let size = entry.length() as usize;
                    let _ = alloc.add_memory(PhysMemoryRegion::new(start, size));
                }
            }
        }

        // Reserve kernel image
        let _ = alloc.add_reservation(PhysMemoryRegion::new(PA::from_value(0x100000), 0x200000));

        // Reserve all multiboot modules (initrd)
        if let Some(modules) = boot_info.modules() {
            for module in modules {
                let start = PA::from_value(module.start as usize);
                let size = (module.end - module.start) as usize;
                let _ = alloc.add_reservation(PhysMemoryRegion::new(start, size));
            }
        }

        unsafe {
            alloc.permit_region_list_reallocs();
        }
    }

    unsafe {
        core::arch::asm!("outb %al, %dx", in("al") b'F', in("dx") 0x3F8u16, options(att_syntax));
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

    unsafe {
        core::arch::asm!("outb %al, %dx", in("al") b'G', in("dx") 0x3F8u16, options(att_syntax));
    }
    // Initialize per-CPU variables (requires heap)
    unsafe {
        libkernel::sync::per_cpu::setup_percpu(1);
    }

    unsafe {
        core::arch::asm!("outb %al, %dx", in("al") b'H', in("dx") 0x3F8u16, options(att_syntax));
    }
    // Run kernel driver initcalls (registers null/zero/console/uart chardevs, etc.)
    unsafe {
        crate::drivers::init::run_initcalls();
    }

    unsafe {
        core::arch::asm!("outb %al, %dx", in("al") b'I', in("dx") 0x3F8u16, options(att_syntax));
    }
    // Install the IDT
    crate::arch::x86_64::exceptions::exceptions_init().expect("Failed to init exceptions");

    unsafe {
        core::arch::asm!("outb %al, %dx", in("al") b'J', in("dx") 0x3F8u16, options(att_syntax));
    }
    // Initialize interrupt controller and timer (requires Arc allocation → needs heap)
    crate::arch::x86_64::interrupts::init();

    unsafe {
        core::arch::asm!("outb %al, %dx", in("al") b'K', in("dx") 0x3F8u16, options(att_syntax));
    }
    X86_64::enable_interrupts();

    unsafe {
        core::arch::asm!("outb %al, %dx", in("al") b'L', in("dx") 0x3F8u16, options(att_syntax));
    }
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
