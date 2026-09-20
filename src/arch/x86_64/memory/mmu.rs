//! x86_64 Memory Management Unit (MMU) operations.
//!
//! Handles 4-level page table manipulation, TLB management,
//! and memory-mapped I/O region setup for the x86_64 architecture.
//!
//! Mirrors the AArch64 `mmu.rs` implementation, using x86_64 page
//! table types (`PML4Table`) instead of `L0Table`.

use super::{MMIO_BASE, page_mapper::PageOffsetPgTableMapper, tlb::AllTlbInvalidator};
use crate::sync::{OnceLock, SpinLock};
use libkernel::{
    arch::x86_64::memory::{
        pg_descriptors::MemoryType,
        pg_tables::{MapAttributes, MappingContext, PML4Table, map_range},
        pg_walk::get_pte,
    },
    error::Result,
    memory::{
        address::{PA, TPA, VA},
        paging::{PaMapper, PgTableArray, permissions::PtePermissions},
        proc_vm::address_space::KernAddressSpace,
        region::{PhysMemoryRegion, VirtMemoryRegion},
    },
};
use page_allocator::PageTableAllocator;

pub mod page_allocator;

pub static KERN_ADDR_SPC: OnceLock<SpinLock<X86_64KernelAddressSpace>> = OnceLock::new();

pub struct X86_64KernelAddressSpace {
    pml4_table: TPA<PgTableArray<PML4Table>>,
    mmio_ptr: VA,
}

impl X86_64KernelAddressSpace {
    fn do_map(&self, map_attrs: MapAttributes) -> Result<()> {
        let mut ctx = MappingContext {
            allocator: &mut PageTableAllocator::new(),
            mapper: &mut PageOffsetPgTableMapper {},
            invalidator: &AllTlbInvalidator::new(),
        };

        map_range(self.pml4_table, map_attrs, &mut ctx)
    }

    pub fn translate(&self, va: VA) -> Option<PA> {
        let pg_offset = va.page_offset();

        let pte = get_pte(self.pml4_table, va, &mut PageOffsetPgTableMapper {})
            .ok()
            .flatten()?;

        let pa = pte.mapped_address()?;

        Some(pa.add_bytes(pg_offset))
    }

    pub fn table_pa(&self) -> PA {
        self.pml4_table.to_untyped()
    }
}

unsafe impl Send for X86_64KernelAddressSpace {}

impl KernAddressSpace for X86_64KernelAddressSpace {
    fn map_normal(
        &mut self,
        phys_range: PhysMemoryRegion,
        virt_range: VirtMemoryRegion,
        perms: PtePermissions,
    ) -> Result<()> {
        self.do_map(MapAttributes {
            phys: phys_range,
            virt: virt_range,
            mem_type: MemoryType::WB,
            perms,
        })
    }

    fn map_mmio(&mut self, phys_range: PhysMemoryRegion) -> Result<VA> {
        let phys_mappable_region = phys_range.to_mappable_region();
        let base_va = self.mmio_ptr;

        let virt_range = VirtMemoryRegion::new(base_va, phys_mappable_region.region().size());

        self.do_map(MapAttributes {
            phys: phys_mappable_region.region(),
            virt: virt_range,
            mem_type: MemoryType::UC,
            perms: PtePermissions::rw(false),
        })?;

        self.mmio_ptr =
            VA::from_value(self.mmio_ptr.value() + phys_mappable_region.region().size());

        Ok(VA::from_value(
            base_va.value() + phys_mappable_region.offset(),
        ))
    }
}

/// The x86_64 MMU.
pub struct Mmu;

impl Mmu {
    /// Flush the TLB entry for a given virtual address.
    pub fn flush_tlb_entry(&self, virt: VA) {
        // SAFETY: INVLPG is a privileged instruction that invalidates
        // a single TLB entry. Safe to call with any valid virtual address.
        unsafe {
            core::arch::asm!(
                "invlpg [{virt}]",
                virt = in(reg) virt.value() as u64,
                options(nostack),
            );
        }
    }

    /// Flush the entire TLB by reloading CR3.
    pub fn flush_tlb_all(&self) {
        // SAFETY: Reading CR3 and writing it back invalidates all
        // non-global TLB entries.
        unsafe {
            let cr3: u64;
            core::arch::asm!("mov {cr3}, cr3", cr3 = out(reg) cr3);
            core::arch::asm!("mov cr3, {cr3}", cr3 = in(reg) cr3);
        }
    }

    /// Read the current CR3 value (PML4 physical base).
    pub fn read_cr3() -> PA {
        let cr3: u64;
        // SAFETY: Reading CR3 is always safe.
        unsafe {
            core::arch::asm!("mov {cr3}, cr3", cr3 = out(reg) cr3);
        }
        PA::from_value(cr3 as usize)
    }
}

pub fn setup_kern_addr_space(pa: TPA<PgTableArray<PML4Table>>) -> Result<()> {
    let addr_space = SpinLock::new(X86_64KernelAddressSpace {
        pml4_table: pa,
        mmio_ptr: MMIO_BASE,
    });

    KERN_ADDR_SPC
        .set(addr_space)
        .map_err(|_| libkernel::error::KernelError::InUse)
}
