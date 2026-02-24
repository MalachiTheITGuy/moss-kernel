use alloc::vec::Vec;
use libkernel::{
    PageInfo, UserAddressSpace,
    arch::x86_64::memory::{
        pg_descriptors::MemoryType,
        pg_tables::{PML4Table, MapAttributes, MappingContext, PageAllocator, PgTableArray, map_range},
    },
    error::Result,
    memory::{
        PAGE_SIZE,
        address::{TPA, VA},
        page::PageFrame,
        permissions::PtePermissions,
        region::{PhysMemoryRegion, VirtMemoryRegion},
    },
};
use super::mmu::{page_allocator::PageTableAllocator, page_mapper::PageOffsetPgTableMapper};

pub struct X86_64ProcessAddressSpace {
    pml4_table: TPA<PgTableArray<PML4Table>>,
}

unsafe impl Send for X86_64ProcessAddressSpace {}
unsafe impl Sync for X86_64ProcessAddressSpace {}

impl UserAddressSpace for X86_64ProcessAddressSpace {
    fn new() -> Result<Self>
    where
        Self: Sized,
    {
        let pml4_table = PageTableAllocator::new().allocate_page_table()?;
        Ok(Self { pml4_table })
    }

    fn activate(&self) {
        // TODO: mov cr3, self.pml4_table
    }

    fn deactivate(&self) {
    }

    fn map_page(&mut self, page: PageFrame, va: VA, perms: PtePermissions) -> Result<()> {
        let mut ctx = MappingContext {
            allocator: &mut PageTableAllocator::new(),
            mapper: &mut PageOffsetPgTableMapper {},
        };

        map_range(
            self.pml4_table,
            MapAttributes {
                phys: page.as_phys_range(),
                virt: VirtMemoryRegion::new(va, PAGE_SIZE),
                mem_type: MemoryType::Normal,
                perms,
            },
            &mut ctx,
        )
    }

    fn unmap(&mut self, _va: VA) -> Result<PageFrame> {
        unimplemented!()
    }

    fn protect_range(&mut self, _va_range: VirtMemoryRegion, _perms: PtePermissions) -> Result<()> {
        unimplemented!()
    }

    fn unmap_range(&mut self, _va_range: VirtMemoryRegion) -> Result<Vec<PageFrame>> {
        unimplemented!()
    }

    fn remap(&mut self, _va: VA, _new_page: PageFrame, _perms: PtePermissions) -> Result<PageFrame> {
        unimplemented!()
    }

    fn translate(&self, _va: VA) -> Option<PageInfo> {
        unimplemented!()
    }

    fn protect_and_clone_region(
        &mut self,
        _region: VirtMemoryRegion,
        _other: &mut Self,
        _new_perms: PtePermissions,
    ) -> Result<()>
    where
        Self: Sized,
    {
        unimplemented!()
    }
}
