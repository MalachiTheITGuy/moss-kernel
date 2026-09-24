//! x86_64 Virtual Dynamic Shared Object (VDSO) support.
//!
//! Provides kernel-side VDSO initialisation so that user-space can
//! call into the kernel's sigreturn trampoline without a full syscall
//! transition.  The layout mirrors the arm64 implementation.

use libkernel::error::Result;
use libkernel::memory::address::VA;
use libkernel::memory::paging::permissions::PtePermissions;
use libkernel::memory::proc_vm::address_space::{KernAddressSpace, VirtualMemory};
use libkernel::memory::region::{PhysMemoryRegion, VirtMemoryRegion};

use log::info;

use crate::arch::ArchImpl;
use crate::ksym_pa;

/// Virtual base address of the VDSO page mapped into user space.
///
/// Must agree with the address used by the linker script and the
/// trampoline in `vdso.s`.
pub const VDSO_BASE: VA = VA::from_value(0xffff_8100_0000_0000);

unsafe extern "C" {
    static __vdso_start: u8;
    static __vdso_end: u8;
}

/// Initialise the VDSO by mapping the compiled trampoline page into
/// the kernel address space at `VDSO_BASE`.
///
/// This must be called **after** `setup_kern_addr_space()` during
/// early boot so that the kernel page tables are available.
pub fn vdso_init() -> Result<()> {
    let start = ksym_pa!(__vdso_start);
    let end = ksym_pa!(__vdso_end);
    let region = PhysMemoryRegion::from_start_end_address(start, end);

    let mappable_region = region.to_mappable_region();

    let mut kspc = ArchImpl::kern_address_space().lock_save_irq();

    let vregion = VirtMemoryRegion::new(VDSO_BASE, mappable_region.region().size());

    kspc.map_normal(mappable_region.region(), vregion, PtePermissions::rx(true))?;

    info!(
        "vdso: mapped at 0x{:x} (0x{:x} bytes)",
        vregion.start_address().value(),
        vregion.size()
    );

    Ok(())
}
