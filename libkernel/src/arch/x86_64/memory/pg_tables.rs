use core::marker::PhantomData;

/// Write a nibble (4 bits) as a hex character to the debug serial port
#[inline(always)]
fn debug_puthex_nibble(_nibble: u8) {
    // Debug output disabled - inline asm not configurable in current setup
}

/// Write a full address (usize) as hex to the debug serial port
#[inline(always)]
fn debug_puthex_addr(_addr: usize) {
    // Debug output disabled - inline asm not configurable in current setup
}

/// Write a byte to the debug serial port
#[inline(always)]
fn debug_putchar(_c: u8) {
    // Debug output disabled - inline asm not configurable in current setup
}

use super::pg_descriptors::{
    MemoryType, PDDescriptor, PDPTDescriptor, PML4Descriptor, PTDescriptor, PaMapper,
    PageTableEntry, TableMapper,
};
use crate::error::{MapError, Result};
use crate::memory::{
    address::{TPA, TVA, VA},
    permissions::PtePermissions,
    region::{PhysMemoryRegion, VirtMemoryRegion},
    PAGE_SIZE,
};

pub const DESCRIPTORS_PER_PAGE: usize = 512;
pub const LEVEL_MASK: usize = 511;

pub trait PgTable: Clone + Copy {
    const SHIFT: usize;
    type Descriptor: PageTableEntry;
    fn from_ptr(ptr: TVA<PgTableArray<Self>>) -> Self;
    fn to_raw_ptr(self) -> *mut u64;
    fn pg_index(va: VA) -> usize {
        (va.value() >> Self::SHIFT) & LEVEL_MASK
    }
    fn get_desc(self, va: VA) -> Self::Descriptor;
    fn get_idx(self, idx: usize) -> Self::Descriptor;
    fn set_desc(self, va: VA, desc: Self::Descriptor);
}

pub(super) trait TableMapperTable: PgTable<Descriptor: TableMapper> + Clone + Copy {
    type NextLevel: PgTable;
}

#[derive(Clone)]
#[repr(C, align(4096))]
pub struct PgTableArray<K: PgTable> {
    pages: [u64; DESCRIPTORS_PER_PAGE],
    _phantom: PhantomData<K>,
}

impl<K: PgTable> PgTableArray<K> {
    pub const fn new() -> Self {
        Self {
            pages: [0; DESCRIPTORS_PER_PAGE],
            _phantom: PhantomData,
        }
    }
}

macro_rules! impl_pgtable {
    ($table:ident, $shift:expr, $desc_type:ident) => {
        #[derive(Clone, Copy)]
        pub struct $table {
            base: *mut u64,
        }

        impl PgTable for $table {
            const SHIFT: usize = $shift;
            type Descriptor = $desc_type;

            fn from_ptr(ptr: TVA<PgTableArray<Self>>) -> Self {
                Self {
                    base: ptr.as_ptr_mut().cast(),
                }
            }

            fn to_raw_ptr(self) -> *mut u64 {
                self.base
            }

            fn get_idx(self, idx: usize) -> Self::Descriptor {
                let raw = unsafe { self.base.add(idx).read_volatile() };
                Self::Descriptor::from_raw(raw)
            }

            fn get_desc(self, va: VA) -> Self::Descriptor {
                self.get_idx(Self::pg_index(va))
            }

            fn set_desc(self, va: VA, desc: Self::Descriptor) {
                unsafe {
                    self.base
                        .add(Self::pg_index(va))
                        .write_volatile(PageTableEntry::as_raw(desc))
                };
            }
        }
    };
}

impl_pgtable!(PML4Table, 39, PML4Descriptor);
impl TableMapperTable for PML4Table {
    type NextLevel = PDPTTable;
}

impl_pgtable!(PDPTTable, 30, PDPTDescriptor);
impl TableMapperTable for PDPTTable {
    type NextLevel = PDTable;
}

impl_pgtable!(PDTable, 21, PDDescriptor);
impl TableMapperTable for PDTable {
    type NextLevel = PTTable;
}

impl_pgtable!(PTTable, 12, PTDescriptor);

pub trait PageTableMapper {
    unsafe fn with_page_table<T: PgTable, R>(
        &mut self,
        pa: TPA<PgTableArray<T>>,
        f: impl FnOnce(TVA<PgTableArray<T>>) -> R,
    ) -> Result<R>;
}

pub trait PageAllocator {
    fn allocate_page_table<T: PgTable>(&mut self) -> Result<TPA<PgTableArray<T>>>;
}

pub struct MapAttributes {
    pub phys: PhysMemoryRegion,
    pub virt: VirtMemoryRegion,
    pub mem_type: MemoryType,
    pub perms: PtePermissions,
}

pub struct MappingContext<'a, PA, PM>
where
    PA: PageAllocator + 'a,
    PM: PageTableMapper + 'a,
{
    pub allocator: &'a mut PA,
    pub mapper: &'a mut PM,
}

pub fn map_range<PA, PM>(
    pml4_table: TPA<PgTableArray<PML4Table>>,
    mut attrs: MapAttributes,
    ctx: &mut MappingContext<PA, PM>,
) -> Result<()>
where
    PA: PageAllocator,
    PM: PageTableMapper,
{
    if attrs.phys.size() != attrs.virt.size() {
        return Err(MapError::SizeMismatch.into());
    }

    while attrs.virt.size() > 0 {
        let va = attrs.virt.start_address();

        let pdpt = map_at_level(pml4_table, va, ctx)?;

        if let Some(pgs_mapped) = try_map_pa(pdpt, va, attrs.phys, &attrs, ctx)? {
            attrs.virt = attrs.virt.add_pages(pgs_mapped);
            attrs.phys = attrs.phys.add_pages(pgs_mapped);
            continue;
        }

        let pd = map_at_level(pdpt, va, ctx)?;

        if let Some(pgs_mapped) = try_map_pa(pd, va, attrs.phys, &attrs, ctx)? {
            attrs.virt = attrs.virt.add_pages(pgs_mapped);
            attrs.phys = attrs.phys.add_pages(pgs_mapped);
            continue;
        }

        let pt = map_at_level(pd, va, ctx)?;

        try_map_pa(pt, va, attrs.phys, &attrs, ctx)?;

        attrs.virt = attrs.virt.add_pages(1);
        attrs.phys = attrs.phys.add_pages(1);
    }

    Ok(())
}

fn try_map_pa<L, PA, PM>(
    table: TPA<PgTableArray<L>>,
    va: VA,
    phys_region: PhysMemoryRegion,
    attrs: &MapAttributes,
    ctx: &mut MappingContext<PA, PM>,
) -> Result<Option<usize>>
where
    L: PgTable<Descriptor: PaMapper>,
    PA: PageAllocator,
    PM: PageTableMapper,
{
    if L::Descriptor::could_map(phys_region, va) {
        unsafe {
            if ctx
                .mapper
                .with_page_table(table, |tbl| L::from_ptr(tbl).get_desc(va))?
                .is_valid()
            {
                return Err(MapError::AlreadyMapped.into());
            }

            ctx.mapper.with_page_table(table, |tbl| {
                L::from_ptr(tbl).set_desc(
                    va,
                    L::Descriptor::new_map_pa(
                        phys_region.start_address(),
                        attrs.mem_type,
                        attrs.perms,
                    ),
                );
            })?;
        }

        Ok(Some(1 << (L::Descriptor::map_shift() - 12)))
    } else {
        Ok(None)
    }
}

pub(super) fn map_at_level<L, PA, PM>(
    table: TPA<PgTableArray<L>>,
    va: VA,
    ctx: &mut MappingContext<PA, PM>,
) -> Result<TPA<PgTableArray<L::NextLevel>>>
where
    L: TableMapperTable,
    PA: PageAllocator,
    PM: PageTableMapper,
{
    unsafe {
        let desc = ctx
            .mapper
            .with_page_table(table, |pgtable| L::from_ptr(pgtable).get_desc(va))?;

        if let Some(pa) = desc.next_table_address() {
            return Ok(TPA::from_value(pa.value()));
        }

        if desc.is_valid() {
            return Err(MapError::AlreadyMapped.into());
        }

        let new_pa = ctx.allocator.allocate_page_table::<L::NextLevel>()?;

        ctx.mapper.with_page_table(new_pa, |new_pgtable| {
            core::ptr::write_bytes(new_pgtable.as_ptr_mut() as *mut _ as *mut u8, 0, PAGE_SIZE);
        })?;

        ctx.mapper.with_page_table(table, |pgtable| {
            L::from_ptr(pgtable).set_desc(va, L::Descriptor::new_next_table(new_pa.to_untyped()));
        })?;

        Ok(new_pa)
    }
}
