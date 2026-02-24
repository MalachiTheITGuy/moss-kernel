use crate::{
    arch::ArchImpl,
    memory::{PageOffsetTranslator, page::PgAllocGetter},
    sync::OnceLock,
};
use core::{
    arch::asm,
    ops::{Deref, DerefMut},
    ptr,
};
use libkernel::{
    CpuOps,
    memory::allocators::slab::{
        allocator::SlabAllocator,
        cache::SlabCache,
        heap::{KHeap, SlabCacheStorage, SlabGetter},
    },
};

type SlabAlloc = SlabAllocator<ArchImpl, PgAllocGetter, PageOffsetTranslator>;

pub static SLAB_ALLOC: OnceLock<SlabAlloc> = OnceLock::new();

pub struct StaticSlabGetter {}

impl SlabGetter<ArchImpl, PgAllocGetter, PageOffsetTranslator> for StaticSlabGetter {
    fn global_slab_alloc() -> &'static SlabAlloc {
        SLAB_ALLOC.get().unwrap()
    }
}

pub struct PerCpuCache {
    flags: usize,
}

impl PerCpuCache {
    fn get_ptr() -> *mut SlabCache {
        let mut cache: *mut SlabCache;
        unsafe {
            // For now, let's use the swapgs/rdgsbase logic or just assume GS base points to the pointer.
            // Actually, a simpler way for early boot with 1 CPU is a static pointer.
            // But to be generic, we use GS.
            asm!("mov {}, QWORD PTR gs:[0]", out(reg) cache, options(nostack, readonly, preserves_flags));
        }

        if cache.is_null() {
            panic!("Attempted to use alloc/free before CPU initialisation!");
        }

        cache
    }
}

impl SlabCacheStorage for PerCpuCache {
    fn store(ptr: *mut SlabCache) {
        unsafe {
            asm!("mov QWORD PTR gs:[0], {}", in(reg) ptr, options(nostack, nomem));
        }
    }

    fn get() -> impl DerefMut<Target = SlabCache> {
        let flags = ArchImpl::disable_interrupts();
        Self { flags }
    }
}

impl Deref for PerCpuCache {
    type Target = SlabCache;

    fn deref(&self) -> &Self::Target {
        unsafe { &(*Self::get_ptr()) }
    }
}

impl DerefMut for PerCpuCache {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut (*Self::get_ptr()) }
    }
}

impl Drop for PerCpuCache {
    fn drop(&mut self) {
        ArchImpl::restore_interrupt_state(self.flags);
    }
}

pub type KernelHeap =
    KHeap<ArchImpl, PerCpuCache, PgAllocGetter, PageOffsetTranslator, StaticSlabGetter>;

#[global_allocator]
static K_HEAP: KernelHeap = KernelHeap::new();
