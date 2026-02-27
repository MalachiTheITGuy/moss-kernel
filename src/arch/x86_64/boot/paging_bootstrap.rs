use crate::arch::x86_64::memory::mmu::{
    page_mapper::PageOffsetPgTableMapper, setup_kern_addr_space,
};
use crate::memory::PageOffsetTranslator;
use crate::INITAL_ALLOCATOR;
use core::ptr;
use libkernel::arch::x86_64::memory::pg_descriptors::MemoryType;
use libkernel::arch::x86_64::memory::pg_tables::{
    map_range, MapAttributes, MappingContext, PML4Table, PageAllocator, PageTableMapper, PgTable,
    PgTableArray,
};
use libkernel::memory::address::{IdentityTranslator, PA, TPA, TVA, VA};
use libkernel::memory::allocators::smalloc::Smalloc;
use libkernel::memory::permissions::PtePermissions;
use libkernel::memory::region::{PhysMemoryRegion, VirtMemoryRegion};
use multiboot2::BootInformation;
use multiboot2::MemoryMapTag;

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

            // We need the physical address of this static buffer.
            // Since it's in the kernel image, PA = VA - KERNEL_BASE.
            let va = ptr as usize;
            let pa = va - 0xffffffff80000000; // FIXME: Use a proper constant or calculation
            Ok(TPA::from_value(pa))
        }
    }
}

struct SmallocPageAlloc {
    smalloc: &'static mut Smalloc<PageOffsetTranslator>,
}

impl PageAllocator for SmallocPageAlloc {
    fn allocate_page_table<T: PgTable>(
        &mut self,
    ) -> libkernel::error::Result<TPA<PgTableArray<T>>> {
        let pa = self.smalloc.alloc(4096, 4096)?;
        Ok(TPA::from_value(pa.value()))
    }
}

pub fn setup_allocator(boot_info: &BootInformation) {
    let mut smalloc = INITAL_ALLOCATOR.lock();
    let smalloc = smalloc.as_mut().unwrap();

    // Add memory regions from Multiboot2 memory map
    let memory_map_tag = boot_info.memory_map_tag().unwrap();
    for entry in memory_map_tag.memory_areas() {
        if entry.typ == multiboot2::MemoryAreaType::Available {
            smalloc.add_memory_region(PhysMemoryRegion::new(
                PA::from_value(entry.start_address as usize),
                entry.size as usize,
            ));
        }
    }

    // Reserve kernel image
    let kernel_start = 0x100000;
    let kernel_size = 2 * 1024 * 1024; // FIXME: get real size
    smalloc.add_reservation(PhysMemoryRegion::new(
        PA::from_value(kernel_start),
        kernel_size,
    ));

    // Reserve modules (initrd)
    if let Some(modules_tag) = boot_info.module_tags().next() {
        let start = modules_tag.start_address();
        let end = modules_tag.end_address();
        smalloc.add_reservation(PhysMemoryRegion::new(
            PA::from_value(start as usize),
            (end - start) as usize,
        ));
    }

    // Reserve the local APIC MMIO region so the allocator never hands out
    // pages that overlap the hardware registers.  Without this reservation
    // we have seen the heap return 0xFEE0_0000 as an allocation, which
    // subsequently causes a page fault when the kernel writes to it.
    // The APIC occupies at least one page; reserve 0x1000 bytes to be safe.
    const LOCAL_APIC_PHYS: usize = 0xFEE0_0000;
    const LOCAL_APIC_SIZE: usize = 0x1000;
    let _ = smalloc.add_reservation(PhysMemoryRegion::new(
        PA::from_value(LOCAL_APIC_PHYS),
        LOCAL_APIC_SIZE,
    ));

    // Permit reallocs
    smalloc.permit_reallocs();
}

pub fn bootstrap_memory(boot_info: &BootInformation, _image_start: usize, _image_end: usize) {
    setup_allocator(boot_info);
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

    let smalloc = INITAL_ALLOCATOR.lock().as_mut().unwrap();
    let mut allocator = SmallocPageAlloc { smalloc };

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
            // TODO: W^X - split into separate rx (.text) and rw (.data/.bss) mappings
            // using linker script symbols once section layout is finalized.
            perms: PtePermissions::rwx(false),
        },
        &mut ctx,
    )
    .unwrap();

    // 2. Setup Linear Map (first 1GiB for now)
    map_range(
        pml4_pa,
        MapAttributes {
            phys: PhysMemoryRegion::new(PA::from_value(0), 4 * 1024 * 1024 * 1024),
            virt: VirtMemoryRegion::new(
                VA::from_value(0xffff_8000_0000_0000),
                4 * 1024 * 1024 * 1024,
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

    // Load new CR3
    unsafe {
        core::arch::asm!("mov cr3, {}", in(reg) pml4_pa.value());
    }

    log::info!("New page tables loaded!");

    setup_kern_addr_space(pml4_pa).unwrap();
}
