use libkernel::memory::address::{PA, VA};
use crate::console::setup_console_logger;

pub mod paging_bootstrap;

#[unsafe(no_mangle)]
pub extern "C" fn arch_init_stage1(
    mb_info_ptr: usize,
    _image_start: usize,
    _image_end: usize,
) -> ! {
    let mut serial_port = unsafe { uart_16550::SerialPort::new(0x3F8) };
    serial_port.init();

    struct EarlyConsole;
    impl crate::console::Console for EarlyConsole {
        fn write_char(&self, c: char) {
            unsafe { uart_16550::SerialPort::new(0x3F8).send(c as u8) };
        }
        fn write_fmt(&self, args: core::fmt::Arguments) -> core::fmt::Result {
            use core::fmt::Write;
            unsafe { uart_16550::SerialPort::new(0x3F8).write_fmt(args) }
        }
        fn write_buf(&self, buf: &[u8]) {
            let mut s = unsafe { uart_16550::SerialPort::new(0x3F8) };
            for &b in buf {
                s.send(b);
            }
        }
        fn register_input_handler(&self, _: alloc::sync::Weak<dyn crate::console::tty::TtyInputHandler>) {}
    }

    let console = alloc::sync::Arc::new(EarlyConsole);
    let _ = crate::console::set_active_console(console, libkernel::driver::CharDevDescriptor { major: 0, minor: 0 });
    setup_console_logger();

    log::info!("Moss x86_64 booting...");

    let boot_info = unsafe { multiboot2::BootInformation::load(mb_info_ptr as *const _).expect("Failed to load Multiboot2 info") };

    if let Some(mmap) = boot_info.memory_map_tag() {
        for region in mmap.memory_areas() {
            log::info!("Memory region: {:#x} - {:#x} ({:?})", 
                region.start_address(), region.end_address(), region.typ());
        }
    }

    log::info!("Image (VA): {:#x} - {:#x}", _image_start, _image_end);

    paging_bootstrap::bootstrap_memory(&boot_info, _image_start, _image_end);

    loop {
        unsafe { core::arch::asm!("hlt") };
    }
}
pub fn arch_init_stage2(_frame: *mut super::exceptions::ExceptionState) -> *mut super::exceptions::ExceptionState {
    unimplemented!()
}
