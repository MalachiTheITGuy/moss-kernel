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
use crate::memory::PageOffsetTranslator;

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

        // Copy the kernel's upper-half PML4 entries (entries 256–511) into the new
        // process PML4 so that kernel code remains reachable after CR3 switches.
        {
            use super::mmu::KERN_ADDR_SPC;
            if let Some(kern) = KERN_ADDR_SPC.get() {
                let kern_pml4_pa = kern.lock_save_irq().table_pa();

                // 512 u64 entries; upper half starts at index 256
                let kern_entries = unsafe {
                    core::slice::from_raw_parts(
                        kern_pml4_pa.cast::<u64>()
                            .to_va::<PageOffsetTranslator>()
                            .value() as *const u64,
                        512,
                    )
                };
                let proc_entries = unsafe {
                    core::slice::from_raw_parts_mut(
                        pml4_table.to_untyped().cast::<u64>()
                            .to_va::<PageOffsetTranslator>()
                            .value() as *mut u64,
                        512,
                    )
                };
                proc_entries[256..512].copy_from_slice(&kern_entries[256..512]);
            }
        }

        Ok(Self { pml4_table })
    }

    fn activate(&self) {
        unsafe {
            core::arch::asm!("mov cr3, {}", in(reg) self.pml4_table.value());
        }
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

    fn protect_range(&mut self, va_range: VirtMemoryRegion, perms: PtePermissions) -> Result<()> {
        use libkernel::arch::x86_64::memory::pg_tables::{PgTable, PDPTTable, PDTable, PTTable};
        use libkernel::arch::x86_64::memory::pg_descriptors::{TableMapper, PaMapper, PTDescriptor, PageTableEntry};
        use libkernel::memory::PAGE_SIZE;
        use crate::memory::PageOffsetTranslator;
        use libkernel::memory::address::{TPA, TVA};
        use libkernel::arch::x86_64::memory::pg_tables::PgTableArray;

        let mut va = va_range.start_address();
        let end = va_range.end_address();

        while va < end {
            // Walk PML4 → PDPT
            let pdpt_pa: TPA<PgTableArray<PDPTTable>> = unsafe {
                let pml4 = PML4Table::from_ptr(self.pml4_table.to_va::<PageOffsetTranslator>());
                let desc = pml4.get_desc(va);
                match desc.next_table_address() {
                    Some(pa) => TPA::from_value(pa.value()),
                    None => { va = va.add_bytes(PAGE_SIZE); continue; }
                }
            };
            // Walk PDPT → PD
            let pd_pa: TPA<PgTableArray<PDTable>> = unsafe {
                let pdpt = PDPTTable::from_ptr(pdpt_pa.to_va::<PageOffsetTranslator>());
                let desc = pdpt.get_desc(va);
                match desc.next_table_address() {
                    Some(pa) => TPA::from_value(pa.value()),
                    None => { va = va.add_bytes(PAGE_SIZE); continue; }
                }
            };
            // Walk PD → PT
            let pt_pa: TPA<PgTableArray<PTTable>> = unsafe {
                let pd = PDTable::from_ptr(pd_pa.to_va::<PageOffsetTranslator>());
                let desc = pd.get_desc(va);
                match desc.next_table_address() {
                    Some(pa) => TPA::from_value(pa.value()),
                    None => { va = va.add_bytes(PAGE_SIZE); continue; }
                }
            };
            // Update the leaf PTE
            unsafe {
                let pt = PTTable::from_ptr(pt_pa.to_va::<PageOffsetTranslator>());
                let old = pt.get_desc(va);
                if old.is_valid() {
                    pt.set_desc(va, old.set_permissions(perms));
                }
            }

            va = va.add_bytes(PAGE_SIZE);
        }

        // Flush the TLB for this address space (full flush for simplicity).
        unsafe {
            core::arch::asm!("mov %cr3, %rax; mov %rax, %cr3", options(att_syntax, nostack));
        }

        Ok(())
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
