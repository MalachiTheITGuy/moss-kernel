use crate::memory::PAGE_ALLOC;

use super::{
    mmu::page_allocator::PageTableAllocator, page_mapper::PageOffsetPgTableMapper,
    tlb::AllTlbInvalidator,
};
use alloc::vec::Vec;
use libkernel::{
    arch::x86_64::memory::{
        pg_descriptors::{MemoryType, PTE},
        pg_tables::{MapAttributes, MappingContext, PML4Table, map_range},
        pg_tear_down::tear_down_address_space,
        pg_walk::{get_pte, walk_and_modify_region},
    },
    error::{KernelError, MapError, Result},
    memory::{
        PAGE_SIZE,
        address::{TPA, VA},
        page::PageFrame,
        paging::{
            PaMapper, PageAllocator, PageTableEntry, PgTableArray, permissions::PtePermissions,
            tear_down::TeardownAction, walk::WalkContext,
        },
        proc_vm::address_space::{PageInfo, UserAddressSpace},
        region::{PhysMemoryRegion, VirtMemoryRegion},
    },
};
use log::warn;

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
        let _invalidator = AllTlbInvalidator::new();

        // SAFETY: Loading CR3 with a valid PML4 physical address switches
        // the active address space. The PML4 is kept alive by the struct.
        unsafe {
            core::arch::asm!(
                "mov cr3, {pml4}",
                pml4 = in(reg) self.pml4_table.value() as u64,
                options(nostack),
            );
        }
    }

    fn deactivate(&self) {
        let _invalidator = AllTlbInvalidator::new();
    }

    fn map_page(&mut self, page: PageFrame, va: VA, perms: PtePermissions) -> Result<()> {
        let mut ctx = MappingContext {
            allocator: &mut PageTableAllocator::new(),
            mapper: &mut PageOffsetPgTableMapper {},
            invalidator: &AllTlbInvalidator::new(),
        };

        map_range(
            self.pml4_table,
            MapAttributes {
                phys: page.as_phys_range(),
                virt: VirtMemoryRegion::new(va, PAGE_SIZE),
                mem_type: MemoryType::WB,
                perms,
            },
            &mut ctx,
        )
    }

    fn unmap(&mut self, _va: VA) -> Result<PageFrame> {
        todo!()
    }

    fn protect_range(&mut self, va_range: VirtMemoryRegion, perms: PtePermissions) -> Result<()> {
        let mut walk_ctx = WalkContext {
            mapper: &mut PageOffsetPgTableMapper {},
            invalidator: &AllTlbInvalidator::new(),
        };

        walk_and_modify_region(self.pml4_table, va_range, &mut walk_ctx, |_, desc| {
            match (perms.is_execute(), perms.is_read(), perms.is_write()) {
                (false, false, false) => PTE::invalid(),
                _ => desc.set_permissions(perms),
            }
        })
    }

    fn unmap_range(&mut self, va_range: VirtMemoryRegion) -> Result<Vec<PageFrame>> {
        let mut walk_ctx = WalkContext {
            mapper: &mut PageOffsetPgTableMapper {},
            invalidator: &AllTlbInvalidator::new(),
        };
        let mut claimed_pages = Vec::new();

        walk_and_modify_region(self.pml4_table, va_range, &mut walk_ctx, |_, desc| {
            if let Some(addr) = desc.mapped_address() {
                claimed_pages.push(addr.to_pfn());
            }

            PTE::invalid()
        })?;

        Ok(claimed_pages)
    }

    fn remap(&mut self, va: VA, new_page: PageFrame, perms: PtePermissions) -> Result<PageFrame> {
        let mut walk_ctx = WalkContext {
            mapper: &mut PageOffsetPgTableMapper {},
            invalidator: &AllTlbInvalidator::new(),
        };

        let mut old_pte = None;

        walk_and_modify_region(
            self.pml4_table,
            va.page_region(),
            &mut walk_ctx,
            |_, pte| {
                old_pte = Some(pte);
                PTE::new_map_pa(new_page.pa(), MemoryType::WB, perms)
            },
        )?;

        old_pte
            .and_then(|pte| pte.mapped_address())
            .map(|a| a.to_pfn())
            .ok_or(KernelError::MappingError(MapError::NotL3Mapped))
    }

    fn translate(&self, va: VA) -> Option<PageInfo> {
        let pte = get_pte(
            self.pml4_table,
            va.page_aligned(),
            &mut PageOffsetPgTableMapper {},
        )
        .unwrap()?;

        Some(PageInfo {
            pfn: pte.mapped_address()?.to_pfn(),
            perms: pte.permissions(),
        })
    }

    fn protect_and_clone_region(
        &mut self,
        region: VirtMemoryRegion,
        other: &mut Self,
        new_perms: PtePermissions,
    ) -> Result<()>
    where
        Self: Sized,
    {
        let mut walk_ctx = WalkContext {
            mapper: &mut PageOffsetPgTableMapper {},
            invalidator: &AllTlbInvalidator::new(),
        };

        walk_and_modify_region(self.pml4_table, region, &mut walk_ctx, |va, pgd| {
            if let Some(addr) = pgd.mapped_address() {
                let page_region = PhysMemoryRegion::new(addr, PAGE_SIZE);

                // SAFETY: This is safe since the page will have allocated when
                // handling faults.
                let alloc1 = unsafe { PAGE_ALLOC.get().unwrap().alloc_from_region(page_region) };

                // Increase ref count.
                alloc1.clone().leak();
                alloc1.leak();

                let mut ctx = MappingContext {
                    allocator: &mut PageTableAllocator::new(),
                    mapper: &mut PageOffsetPgTableMapper {},
                    invalidator: &AllTlbInvalidator::new(),
                };

                map_range(
                    other.pml4_table,
                    MapAttributes {
                        phys: PhysMemoryRegion::new(addr, PAGE_SIZE),
                        virt: VirtMemoryRegion::new(va, PAGE_SIZE),
                        mem_type: MemoryType::WB,
                        perms: new_perms,
                    },
                    &mut ctx,
                )
                .unwrap();

                pgd.set_permissions(new_perms)
            } else {
                pgd
            }
        })
    }
}

impl Drop for X86_64ProcessAddressSpace {
    fn drop(&mut self) {
        let mut walk_ctx = WalkContext {
            mapper: &mut PageOffsetPgTableMapper {},
            invalidator: &AllTlbInvalidator::new(),
        };

        if tear_down_address_space(
            self.pml4_table,
            &mut walk_ctx,
            |_| TeardownAction::Free,
            |region| unsafe {
                PAGE_ALLOC.get().unwrap().alloc_from_region(region);
            },
        )
        .is_err()
        {
            warn!("Address space tear down failed.  Probable memory leakage!");
        }
    }
}
