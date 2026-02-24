use super::UartDriver;
use core::fmt;
use uart_16550::SerialPort;

pub struct Ns16550 {
    inner: SerialPort,
}

impl Ns16550 {
    pub const fn new(port: u16) -> Self {
        Self {
            inner: unsafe { SerialPort::new(port) },
        }
    }

    pub fn init(&mut self) {
        self.inner.init();
    }
}

impl fmt::Write for Ns16550 {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.inner.write_str(s)
    }
}

impl UartDriver for Ns16550 {
    fn write_buf(&mut self, buf: &[u8]) {
        for &b in buf {
            self.inner.send(b);
        }
    }

    fn drain_uart_rx(&mut self, buf: &mut [u8]) -> usize {
        let mut read = 0;
        // In uart_16550 0.3, it doesn't seem to expose the LSR.
        // We'll use a raw port read.
        while read < buf.len() {
            let lsr: u8;
            unsafe {
                core::arch::asm!("in al, dx", out("al") lsr, in("dx") 0x3F8 + 5);
            }
            if (lsr & 1) != 0 {
                buf[read] = self.inner.receive();
                read += 1;
            } else {
                break;
            }
        }
        read
    }
}
