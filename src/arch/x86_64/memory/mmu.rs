use crate::sync::{OnceLock, SpinLock};
use libkernel::{
    KernAddressSpace,
    arch::x86_64::memory::pg_tables::{PML4Table, MapAttributes, MappingContext, PgTableArray, map_range},
    error::Result,
    memory::{
        address::{PA, TPA, VA},
        permissions::PtePermissions,
        region::{PhysMemoryRegion, VirtMemoryRegion},
    },
};
use libkernel::arch::x86_64::memory::pg_descriptors::MemoryType;

pub mod page_allocator;
pub mod page_mapper;

pub static KERN_ADDR_SPC: OnceLock<SpinLock<X86_64KernelAddressSpace>> = OnceLock::new();

pub struct X86_64KernelAddressSpace {
    kernel_pml4: TPA<PgTableArray<PML4Table>>,
}

impl X86_64KernelAddressSpace {
    fn do_map(&self, _map_attrs: MapAttributes) -> Result<()> {
        unimplemented!()
    }

    pub fn table_pa(&self) -> PA {
        self.kernel_pml4.to_untyped()
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
        let mut ctx = MappingContext {
            allocator: &mut page_allocator::PageTableAllocator::new(),
            mapper: &mut page_mapper::PageOffsetPgTableMapper {},
        };

        map_range(
            self.kernel_pml4,
            MapAttributes {
                phys: phys_range,
                virt: virt_range,
                mem_type: MemoryType::Normal,
                perms,
            },
            &mut ctx,
        )
    }

    fn map_mmio(&mut self, _phys_range: PhysMemoryRegion) -> Result<VA> {
        unimplemented!()
    }
}

pub fn setup_kern_addr_space(pa: TPA<PgTableArray<PML4Table>>) -> Result<()> {
    let addr_space = SpinLock::new(X86_64KernelAddressSpace {
        kernel_pml4: pa,
    });

    KERN_ADDR_SPC
        .set(addr_space)
        .map_err(|_| libkernel::error::KernelError::InUse)
}
