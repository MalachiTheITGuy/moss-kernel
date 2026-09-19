// moss-kernel: a microkernel OS written in Rust
// This is the main entry point for the kernel.

#![no_std]
#![no_main]

mod arch;
mod drivers;
mod kernel;
mod process;

use core::panic::PanicInfo;

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
