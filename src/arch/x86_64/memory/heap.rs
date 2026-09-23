//! x86_64 kernel heap allocator.
//!
//! Provides a slab-backed global allocator mirroring the ARM64 implementation.
//! Per-CPU slab cache storage uses the GS segment base register (IA32_GS_BASE)
//! instead of ARM64's TPIDR_EL1.
use super::super::cpu_ops::X86_64InterruptFlags;
use crate::{
    arch::ArchImpl,
    memory::{page::PgAllocGetter, PageOffsetTranslator},
    sync::OnceLock,
};
use core::{
    arch::asm,
    ops::{Deref, DerefMut},
};
use libkernel::{
    memory::allocators::slab::{
        allocator::SlabAllocator,
        cache::SlabCache,
        heap::{KHeap, SlabCacheStorage, SlabGetter},
    },
    CpuOps,
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
    flags: X86_64InterruptFlags,
}

/// IA32_GS_BASE MSR — used for kernel per-CPU storage on x86_64.
const IA32_GS_BASE: u32 = 0xC000_0101;

impl PerCpuCache {
    fn get_ptr() -> *mut SlabCache {
        let mut low: u32;
        let mut high: u32;

        // SAFETY: Reading IA32_GS_BASE via RDMSR is safe; the MSR was set
        // during early boot and only changes during context switch.
        unsafe {
            asm!(
                "rdmsr",
                in("ecx") IA32_GS_BASE,
                out("eax") low,
                out("edx") high,
                options(nostack),
            );
        }

        let cache = ((high as u64) << 32) | (low as u64);
        let cache = cache as *mut SlabCache;

        if cache.is_null() {
            panic!("Attempted to use alloc/free before CPU initialisation!");
        }

        cache
    }

    fn store_ptr(ptr: *mut SlabCache) {
        let val = ptr as u64;
        let low = val as u32;
        let high = (val >> 32) as u32;

        // SAFETY: Writing IA32_GS_BASE via WRMSR is safe during early init
        // when we are setting up per-CPU storage for the first time.
        #[allow(clippy::pointers_in_nomem_asm_block)]
        unsafe {
            asm!(
                "wrmsr",
                in("ecx") IA32_GS_BASE,
                in("eax") low,
                in("edx") high,
                options(nostack),
            );
        }
    }
}

impl SlabCacheStorage for PerCpuCache {
    fn store(ptr: *mut SlabCache) {
        Self::store_ptr(ptr);
    }

    fn get() -> impl DerefMut<Target = SlabCache> {
        let flags = ArchImpl::disable_interrupts();

        Self { flags }
    }
}

impl Deref for PerCpuCache {
    type Target = SlabCache;

    fn deref(&self) -> &Self::Target {
        // SAFETY: The pointer uses the GS segment register for per-CPU
        // storage. We've disabled interrupts so we cannot be preempted,
        // therefore mutable access to the cache is safe.
        unsafe { &(*Self::get_ptr()) }
    }
}

impl DerefMut for PerCpuCache {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: The pointer uses the GS segment register for per-CPU
        // storage. We've disabled interrupts so we cannot be preempted,
        // therefore mutable access to the cache is safe.
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
