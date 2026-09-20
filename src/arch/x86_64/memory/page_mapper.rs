use libkernel::{
    error::Result,
    memory::address::{TPA, TVA},
    memory::paging::{PageTableMapper, PgTable, PgTableArray},
};

use crate::memory::PageOffsetTranslator;

pub struct PageOffsetPgTableMapper {}

impl PageTableMapper for PageOffsetPgTableMapper {
    /// # Safety
    ///
    /// The caller must ensure `pa` points to a valid, mapped page table.
    unsafe fn with_page_table<T: PgTable, R>(
        &mut self,
        pa: TPA<PgTableArray<T>>,
        f: impl FnOnce(TVA<PgTableArray<T>>) -> R,
    ) -> Result<R> {
        Ok(f(pa.to_va::<PageOffsetTranslator>()))
    }
}
