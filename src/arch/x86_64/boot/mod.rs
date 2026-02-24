
use crate::arch::x86_64::memory::mmu::{
    page_mapper::PageOffsetPgTableMapper, setup_kern_addr_space,
};
use crate::memory::PageOffsetTranslator;
use core::ptr;
use libkernel::arch::x86_64::memory::pg_descriptors::MemoryType;
use libkernel::arch::x86_64::memory::pg_tables::{
    map_range, MapAttributes, MappingContext, PML4Table, PageAllocator, PageTableMapper, PgTable,
    PgTableArray,
};
use libkernel::memory::address::{PA, TPA, TVA, VA};
use libkernel::memory::permissions::PtePermissions;
use libkernel::memory::region::{PhysMemoryRegion, VirtMemoryRegion};
use libkernel::{CpuOps, VirtualMemory};
use multiboot2::BootInformation;
use crate::arch::x86_64::X86_64;

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

pub fn bootstrap_memory(boot_info: &BootInformation, _image_start: usize, _image_end: usize) {
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

    // 3. Identity map first 2MiB
    map_range(
        pml4_pa,
        MapAttributes {
            phys: PhysMemoryRegion::new(PA::from_value(0), 2 * 1024 * 1024),
            virt: VirtMemoryRegion::new(VA::from_value(0), 2 * 1024 * 1024),
            mem_type: MemoryType::Normal,
            perms: PtePermissions::rwx(false),
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

#[unsafe(no_mangle)]
pub extern "C" fn arch_init_stage1(mb_info_ptr: usize, _image_start: usize, _image_end: usize) -> ! {
    // Stage 1 must initialize its own minimal logging if needed,
    // but here we assume it was already set up or we'll set it up soon.
    
    let boot_info = unsafe { multiboot2::BootInformation::load(mb_info_ptr as *const _).expect("Failed to load Multiboot2 info") };

    bootstrap_memory(&boot_info, _image_start, _image_end);

    log::info!("Setting up exceptions");
    crate::arch::x86_64::exceptions::exceptions_init().expect("Failed to init exceptions");

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
    log::info!("Stage 2 init complete");
    // TODO: Call kmain
}
