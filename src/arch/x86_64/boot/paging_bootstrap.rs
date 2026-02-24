use multiboot2::BootInformation;
use libkernel::memory::address::{PA, VA, TPA, TVA, IdentityTranslator};
use libkernel::arch::x86_64::memory::pg_tables::{PML4Table, PgTableArray, map_range, MapAttributes, MappingContext, PageAllocator, PageTableMapper, PgTable};
use libkernel::arch::x86_64::memory::pg_descriptors::MemoryType;
use libkernel::memory::permissions::PtePermissions;
use libkernel::memory::region::{PhysMemoryRegion, VirtMemoryRegion};
use crate::arch::x86_64::memory::mmu::{setup_kern_addr_space, page_mapper::PageOffsetPgTableMapper};
use crate::memory::PageOffsetTranslator;
use core::ptr;

const STATIC_PAGE_COUNT: usize = 64;
static mut BOOT_PAGES: [[u8; 4096]; STATIC_PAGE_COUNT] = [[0; 4096]; STATIC_PAGE_COUNT];
static mut BOOT_PAGES_ALLOCATED: usize = 0;

struct BootPageAllocator;

impl PageAllocator for BootPageAllocator {
    fn allocate_page_table<T: PgTable>(&mut self) -> libkernel::error::Result<TPA<PgTableArray<T>>> {
        unsafe {
            if BOOT_PAGES_ALLOCATED >= STATIC_PAGE_COUNT {
                panic!("Out of boot pages!");
            }
            let ptr = BOOT_PAGES[BOOT_PAGES_ALLOCATED].as_mut_ptr();
            BOOT_PAGES_ALLOCATED += 1;
            ptr::write_bytes(ptr, 0, 4096);
            
            // We need the physical address of this static buffer.
            // Since it's in the kernel image, PA = VA - KERNEL_BASE.
            let va = ptr as usize;
            let pa = va - 0xffffffff80000000; // FIXME: Use a proper constant or calculation
            Ok(TPA::from_value(pa))
        }
    }
}

pub fn bootstrap_memory(boot_info: &BootInformation, _image_start: usize, _image_end: usize) {
    let mut allocator = BootPageAllocator;
    let mut mapper = PageOffsetPgTableMapper {}; // This might not work if linear map isn't set up yet!
    
    // Wait, PageOffsetPgTableMapper uses PageOffsetTranslator which uses PAGE_OFFSET.
    // But currently we ONLY have identity mapping for 1st 2MiB and higher-half mapping for the same.
    
    // During bootstrap, we should use Identity mapping for accessing page tables.
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
    
    let mut bootstrap_mapper = IdentityPgTableMapper;

    let pml4_pa = allocator.allocate_page_table::<PML4Table>().unwrap();
    
    let mut ctx = MappingContext {
        allocator: &mut allocator,
        mapper: &mut bootstrap_mapper,
    };

    // 1. Map Kernel Image
    let kernel_start_pa = 0x100000; // 1MiB
    // We should get this from symbols but for now...
    let kernel_size = 2 * 1024 * 1024; // 2MiB
    
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

    // 2. Setup Linear Map (first 1GiB for now)
    map_range(
        pml4_pa,
        MapAttributes {
            phys: PhysMemoryRegion::new(PA::from_value(0), 1024 * 1024 * 1024),
            virt: VirtMemoryRegion::new(VA::from_value(0xffff_8000_0000_0000), 1024 * 1024 * 1024),
            mem_type: MemoryType::Normal,
            perms: PtePermissions::rw(false),
        },
        &mut ctx,
    ).unwrap();

    // 3. Identity map first 2MiB (to keep running)
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
