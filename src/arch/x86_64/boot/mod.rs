//! x86_64 boot sequence.
//!
//! Multiboot2-compliant early boot entry and initialization.
//!
//! TODO(#14): Implement Multiboot2 header, start.S entry, GDT, paging setup.

use alloc::string::String;

/// Early boot initialization for x86_64.
///
//! # Safety
//!
//! This function is called very early in the boot process, before the
//! heap is initialized. Only stack-local operations are permitted.
pub unsafe fn early_init() {
    // TODO(#14): Multiboot2 header and entry point.
    // TODO(#14): GDT setup (kernel CS, kernel SS, user CS, user SS).
    // TODO(#14): IDT setup.
    // TODO(#14): Initial 4-level page table identity + higher-half mapping.
    // TODO(#14): Transition to long mode and call into Rust.
    todo!("x86_64 early_init: boot sequence not yet implemented")
}

/// Parse the kernel command line from Multiboot2 information.
// TODO(#14): Implement Multiboot2 tag parsing.
pub fn parse_cmdline(_mb_info: usize) -> Option<String> {
    None
}
