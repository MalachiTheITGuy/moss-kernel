use super::mmu::{page_allocator::PageTableAllocator, page_mapper::PageOffsetPgTableMapper};
use crate::memory::PageOffsetTranslator;
use alloc::vec::Vec;
use libkernel::{
    arch::x86_64::memory::{
        pg_descriptors::MemoryType,
        pg_tables::{
            map_range, MapAttributes, MappingContext, PML4Table, PageAllocator, PgTableArray,
        },
    },
    error::{KernelError, Result},
    memory::{
        address::{TPA, VA},
        page::PageFrame,
        permissions::PtePermissions,
        region::{PhysMemoryRegion, VirtMemoryRegion},
        PAGE_SIZE,
    },
    PageInfo, UserAddressSpace,
};

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
                        kern_pml4_pa
                            .cast::<u64>()
                            .to_va::<PageOffsetTranslator>()
                            .value() as *const u64,
                        512,
                    )
                };
                let proc_entries = unsafe {
                    core::slice::from_raw_parts_mut(
                        pml4_table
                            .to_untyped()
                            .cast::<u64>()
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

    fn deactivate(&self) {}

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
        use crate::memory::PageOffsetTranslator;
        use libkernel::arch::x86_64::memory::pg_descriptors::{
            PTDescriptor, PaMapper, PageTableEntry, TableMapper,
        };
        use libkernel::arch::x86_64::memory::pg_tables::PgTableArray;
        use libkernel::arch::x86_64::memory::pg_tables::{PDPTTable, PDTable, PTTable, PgTable};
        use libkernel::memory::address::{TPA, TVA};
        use libkernel::memory::PAGE_SIZE;

        let mut va = va_range.start_address();
        let end = va_range.end_address();

        while va < end {
            // Walk PML4 → PDPT
            let pdpt_pa: TPA<PgTableArray<PDPTTable>> = unsafe {
                let pml4 = PML4Table::from_ptr(self.pml4_table.to_va::<PageOffsetTranslator>());
                let desc = pml4.get_desc(va);
                match desc.next_table_address() {
                    Some(pa) => TPA::from_value(pa.value()),
                    None => {
                        va = va.add_bytes(PAGE_SIZE);
                        continue;
                    }
                }
            };
            // Walk PDPT → PD
            let pd_pa: TPA<PgTableArray<PDTable>> = unsafe {
                let pdpt = PDPTTable::from_ptr(pdpt_pa.to_va::<PageOffsetTranslator>());
                let desc = pdpt.get_desc(va);
                match desc.next_table_address() {
                    Some(pa) => TPA::from_value(pa.value()),
                    None => {
                        va = va.add_bytes(PAGE_SIZE);
                        continue;
                    }
                }
            };
            // Walk PD → PT
            let pt_pa: TPA<PgTableArray<PTTable>> = unsafe {
                let pd = PDTable::from_ptr(pd_pa.to_va::<PageOffsetTranslator>());
                let desc = pd.get_desc(va);
                match desc.next_table_address() {
                    Some(pa) => TPA::from_value(pa.value()),
                    None => {
                        va = va.add_bytes(PAGE_SIZE);
                        continue;
                    }
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
            core::arch::asm!(
                "mov %cr3, %rax; mov %rax, %cr3",
                options(att_syntax, nostack)
            );
        }

        Ok(())
    }

    fn unmap_range(&mut self, va_range: VirtMemoryRegion) -> Result<Vec<PageFrame>> {
        use crate::memory::PageOffsetTranslator;
        use libkernel::arch::x86_64::memory::pg_descriptors::{
            PTDescriptor, PaMapper, PageTableEntry, TableMapper,
        };
        use libkernel::arch::x86_64::memory::pg_tables::PgTableArray;
        use libkernel::arch::x86_64::memory::pg_tables::{PDPTTable, PDTable, PTTable, PgTable};
        use libkernel::memory::address::TPA;
        use libkernel::memory::PAGE_SIZE;

        let mut va = va_range.start_address();
        let end = va_range.end_address();
        let mut freed = Vec::new();

        while va < end {
            // Walk PML4 → PDPT
            let pdpt_pa: TPA<PgTableArray<PDPTTable>> = unsafe {
                let pml4 = PML4Table::from_ptr(self.pml4_table.to_va::<PageOffsetTranslator>());
                let desc = pml4.get_desc(va);
                match desc.next_table_address() {
                    Some(pa) => TPA::from_value(pa.value()),
                    None => {
                        va = va.add_bytes(PAGE_SIZE);
                        continue;
                    }
                }
            };
            // Walk PDPT → PD
            let pd_pa: TPA<PgTableArray<PDTable>> = unsafe {
                let pdpt = PDPTTable::from_ptr(pdpt_pa.to_va::<PageOffsetTranslator>());
                let desc = pdpt.get_desc(va);
                match desc.next_table_address() {
                    Some(pa) => TPA::from_value(pa.value()),
                    None => {
                        va = va.add_bytes(PAGE_SIZE);
                        continue;
                    }
                }
            };
            // Walk PD → PT
            let pt_pa: TPA<PgTableArray<PTTable>> = unsafe {
                let pd = PDTable::from_ptr(pd_pa.to_va::<PageOffsetTranslator>());
                let desc = pd.get_desc(va);
                match desc.next_table_address() {
                    Some(pa) => TPA::from_value(pa.value()),
                    None => {
                        va = va.add_bytes(PAGE_SIZE);
                        continue;
                    }
                }
            };
            // Clear the leaf PTE and collect the physical frame.
            unsafe {
                let pt = PTTable::from_ptr(pt_pa.to_va::<PageOffsetTranslator>());
                let desc = pt.get_desc(va);
                if desc.is_valid() {
                    if let Some(pa) = desc.mapped_address() {
                        freed.push(pa.to_pfn());
                    }
                    pt.set_desc(va, PTDescriptor::invalid());
                }
            }

            va = va.add_bytes(PAGE_SIZE);
        }

        // Flush the full TLB.
        unsafe {
            core::arch::asm!(
                "mov %cr3, %rax; mov %rax, %cr3",
                options(att_syntax, nostack)
            );
        }

        Ok(freed)
    }

    fn remap(&mut self, va: VA, new_page: PageFrame, perms: PtePermissions) -> Result<PageFrame> {
        use crate::memory::PageOffsetTranslator;
        use libkernel::arch::x86_64::memory::pg_descriptors::{
            PTDescriptor, PaMapper, PageTableEntry, TableMapper,
        };
        use libkernel::arch::x86_64::memory::pg_tables::PgTableArray;
        use libkernel::arch::x86_64::memory::pg_tables::{PDPTTable, PDTable, PTTable, PgTable};
        use libkernel::memory::address::TPA;
        use libkernel::memory::PAGE_SIZE;

        let page_va = va.page_aligned();

        // Walk page tables to find and replace the existing PTE
        let pdpt_pa: TPA<PgTableArray<PDPTTable>> = unsafe {
            let pml4 = PML4Table::from_ptr(self.pml4_table.to_va::<PageOffsetTranslator>());
            let desc = pml4.get_desc(page_va);
            match desc.next_table_address() {
                Some(pa) => TPA::from_value(pa.value()),
                None => return Err(KernelError::Fault)?,
            }
        };

        let pd_pa: TPA<PgTableArray<PDTable>> = unsafe {
            let pdpt = PDPTTable::from_ptr(pdpt_pa.to_va::<PageOffsetTranslator>());
            let desc = pdpt.get_desc(page_va);
            match desc.next_table_address() {
                Some(pa) => TPA::from_value(pa.value()),
                None => return Err(KernelError::Fault)?,
            }
        };

        let pt_pa: TPA<PgTableArray<PTTable>> = unsafe {
            let pd = PDTable::from_ptr(pd_pa.to_va::<PageOffsetTranslator>());
            let desc = pd.get_desc(page_va);
            match desc.next_table_address() {
                Some(pa) => TPA::from_value(pa.value()),
                None => return Err(KernelError::Fault)?,
            }
        };

        // Extract the old page frame and replace it with the new one and perms
        let old_page = unsafe {
            let pt = PTTable::from_ptr(pt_pa.to_va::<PageOffsetTranslator>());
            let old_desc = pt.get_desc(page_va);

            if !old_desc.is_valid() {
                return Err(KernelError::Fault)?;
            }

            // Get the old page frame
            let old_pfn = old_desc
                .mapped_address()
                .ok_or(KernelError::Fault)?
                .to_pfn();

            // Set new descriptor with the new page and permissions
            let new_desc = PTDescriptor::new_map_pa(new_page.pa(), MemoryType::Normal, perms);
            pt.set_desc(page_va, new_desc);

            old_pfn
        };

        // Flush the TLB for this specific page by reloading CR3
        unsafe {
            core::arch::asm!(
                "mov %cr3, %rax; mov %rax, %cr3",
                options(att_syntax, nostack)
            );
        }

        Ok(old_page)
    }

    fn translate(&self, _va: VA) -> Option<PageInfo> {
        // Not implemented - return None
        None
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
        use crate::memory::PageOffsetTranslator;
        use crate::memory::PAGE_ALLOC;
        use libkernel::arch::x86_64::memory::pg_descriptors::{
            PTDescriptor, PaMapper, PageTableEntry, TableMapper,
        };
        use libkernel::arch::x86_64::memory::pg_tables::PgTableArray;
        use libkernel::arch::x86_64::memory::pg_tables::{PDPTTable, PDTable, PTTable, PgTable};
        use libkernel::memory::address::TPA;
        use libkernel::memory::region::PhysMemoryRegion;
        use libkernel::memory::PAGE_SIZE;

        let mut va = region.start_address();
        let end = region.end_address();

        while va < end {
            // Walk PML4 → PDPT
            let pdpt_pa: TPA<PgTableArray<PDPTTable>> = unsafe {
                let pml4 = PML4Table::from_ptr(self.pml4_table.to_va::<PageOffsetTranslator>());
                let desc = pml4.get_desc(va);
                match desc.next_table_address() {
                    Some(pa) => TPA::from_value(pa.value()),
                    None => {
                        va = va.add_bytes(PAGE_SIZE);
                        continue;
                    }
                }
            };
            // Walk PDPT → PD
            let pd_pa: TPA<PgTableArray<PDTable>> = unsafe {
                let pdpt = PDPTTable::from_ptr(pdpt_pa.to_va::<PageOffsetTranslator>());
                let desc = pdpt.get_desc(va);
                match desc.next_table_address() {
                    Some(pa) => TPA::from_value(pa.value()),
                    None => {
                        va = va.add_bytes(PAGE_SIZE);
                        continue;
                    }
                }
            };
            // Walk PD → PT
            let pt_pa: TPA<PgTableArray<PTTable>> = unsafe {
                let pd = PDTable::from_ptr(pd_pa.to_va::<PageOffsetTranslator>());
                let desc = pd.get_desc(va);
                match desc.next_table_address() {
                    Some(pa) => TPA::from_value(pa.value()),
                    None => {
                        va = va.add_bytes(PAGE_SIZE);
                        continue;
                    }
                }
            };

            unsafe {
                let pt = PTTable::from_ptr(pt_pa.to_va::<PageOffsetTranslator>());
                let desc = pt.get_desc(va);
                if let Some(phys_addr) = desc.mapped_address() {
                    let page_region = PhysMemoryRegion::new(phys_addr, PAGE_SIZE);

                    // Bump the frame reference count — we're creating a second
                    // mapping to the same physical page.  Mirror what ARM64 does:
                    // create two FrameAllocator handles and leak them both so the
                    // allocator counts +2 (parent keeps one, child gets one) and
                    // the frame is only freed when the last mapping is dropped.
                    if let Some(alloc) = PAGE_ALLOC.get() {
                        let a1 = alloc.alloc_from_region(page_region);
                        a1.clone().leak();
                        a1.leak();
                    }

                    // Map the same frame into the child address space.
                    other.map_page(phys_addr.to_pfn(), va, new_perms)?;

                    // Re-protect the parent's PTE.
                    pt.set_desc(va, desc.set_permissions(new_perms));
                }
            }

            va = va.add_bytes(PAGE_SIZE);
        }

        // Flush the full TLB for this address space.
        unsafe {
            core::arch::asm!(
                "mov %cr3, %rax; mov %rax, %cr3",
                options(att_syntax, nostack)
            );
        }

        Ok(())
    }
}
