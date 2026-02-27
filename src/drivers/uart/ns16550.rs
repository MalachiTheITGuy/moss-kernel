#[cfg(target_arch = "x86_64")]
use super::UartDriver;
#[cfg(target_arch = "x86_64")]
use core::fmt;
#[cfg(target_arch = "x86_64")]
use uart_16550::SerialPort;

/// Offset of the Line Status Register from the UART base port.
#[cfg(target_arch = "x86_64")]
const LSR_OFFSET: u16 = 5;

#[cfg(target_arch = "x86_64")]
pub struct Ns16550 {
    inner: SerialPort,
    port: u16,
}

#[cfg(target_arch = "x86_64")]
impl Ns16550 {
    pub const fn new(port: u16) -> Self {
        Self {
            inner: unsafe { SerialPort::new(port) },
            port,
        }
    }

    pub fn init(&mut self) {
        self.inner.init();
    }
}

#[cfg(target_arch = "x86_64")]
impl fmt::Write for Ns16550 {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.inner.write_str(s)
    }
}

#[cfg(target_arch = "x86_64")]
impl UartDriver for Ns16550 {
    fn write_buf(&mut self, buf: &[u8]) {
        for &b in buf {
            self.inner.send(b);
        }
    }

    fn drain_uart_rx(&mut self, buf: &mut [u8]) -> usize {
        let mut read = 0;
        while read < buf.len() {
            let lsr: u8;
            unsafe {
                core::arch::asm!("in al, dx", out("al") lsr, in("dx") self.port + LSR_OFFSET);
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
