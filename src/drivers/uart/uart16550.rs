//! x86_64 16550-compatible UART driver.
//!
//! The 16550 UART is the standard serial port on x86_64 systems and is
//! the primary console output mechanism during early boot.  QEMU exposes
//! a 16550-compatible UART at I/O port `0x3F8` (COM1).
//!
//! # Register Map (I/O ports, base = 0x3F8)
//!
//! | Offset | Name     | Description                          |
//! |--------|----------|--------------------------------------|
//! | 0x00   | THR/BRDL | Transmit Hold / Divisor Latch Low    |
//! | 0x01   | IER      | Interrupt Enable Register            |
//! | 0x02   | IIR/FCR  | Interrupt ID / FIFO Control          |
//! | 0x03   | LCR      | Line Control Register                |
//! | 0x04   | MCR      | Modem Control Register               |
//! | 0x05   | LSR      | Line Status Register                 |
//! | 0x06   | MSR      | Modem Status Register                |
//! | 0x07   | SCR      | Scratch Register                     |
//!
//! This driver implements the [`UartDriver`] trait so it can be plugged
//! into the generic [`Uart`] wrapper for console integration, interrupt
//! handling, and TTY forwarding.

use crate::{
    arch::x86_64::portio,
    drivers::{
        Driver, DriverManager,
        init::PlatformBus,
        uart::{UART_CHAR_DEV, Uart, UartDriver},
    },
    interrupts::{ClaimedInterrupt, InterruptDescriptor},
    kernel_driver,
};
use alloc::sync::Arc;
use libkernel::{
    error::{KernelError, Result},
    memory::{
        address::VA,
        proc_vm::address_space::{KernAddressSpace, VirtualMemory},
    },
};
use log::info;

// ──────────────────────────────────────────────
//  16550 register offsets
// ──────────────────────────────────────────────

/// Transmit Hold Register (write) / Receive Buffer Register (read).
const THR_RBR: u16 = 0;
/// Interrupt Enable Register.
const IER: u16 = 1;
/// Interrupt Identification Register (read) / FIFO Control Register (write).
const IIR_FCR: u16 = 2;
/// Line Control Register.
const LCR: u16 = 3;
/// Modem Control Register.
const MCR: u16 = 4;
/// Line Status Register.
const LSR: u16 = 5;

// ──────────────────────────────────────────────
//  LSR bits
// ──────────────────────────────────────────────

/// Data Ready — at least one byte in the receive buffer.
const LSR_DR: u8 = 1 << 0;
/// Transmit Hold Register Empty — ready to accept a new byte.
const LSR_THRE: u8 = 1 << 5;
/// Transmitter Empty — both THR and shift register are empty.
const LSR_TEMT: u8 = 1 << 6;

// ──────────────────────────────────────────────
//  LCR bits
// ──────────────────────────────────────────────

/// 8 data bits.
const LCR_8BIT: u8 = 0x03;
/// Enable DLAB (Divisor Latch Access Bit).
const LCR_DLAB: u8 = 1 << 7;

// ──────────────────────────────────────────────
//  IER bits
// ──────────────────────────────────────────────

/// Enable Received Data Available interrupt.
const IER_RX_DATA_AVAIL: u8 = 1 << 0;
/// Enable Transmitter Holding Register Empty interrupt.
const IER_THRE_INT: u8 = 1 << 1;

// ──────────────────────────────────────────────
//  FCR bits
// ──────────────────────────────────────────────

/// Enable FIFO.
const FCR_FIFO_EN: u8 = 1 << 0;
/// Clear receive FIFO.
const FCR_RX_CLR: u8 = 1 << 1;
/// Clear transmit FIFO.
const FCR_TX_CLR: u8 = 1 << 2;

// ──────────────────────────────────────────────
//  MCR bits
// ──────────────────────────────────────────────

/// Data Terminal Ready.
const MCR_DTR: u8 = 1 << 0;
/// Request To Send.
const MCR_RTS: u8 = 1 << 1;

// ──────────────────────────────────────────────
//  COM1 constants
// ──────────────────────────────────────────────

/// Default COM1 I/O port base.
const COM1_BASE: u16 = 0x3F8;

/// Standard COM1 IRQ.
const COM1_IRQ: usize = 4;

/// Baud rate divisor for 115200 baud (with 1.8432 MHz crystal).
const BAUD_115200_DIV: u16 = 1;

// ──────────────────────────────────────────────
//  Uart16550
// ──────────────────────────────────────────────

/// x86_64 16550 UART driver.
///
/// Each instance communicates with one serial port via I/O port
/// instructions.  The base address is configurable so that multiple
/// 16550-compatible UARTs (COM1–COM4) can be supported.
pub struct Uart16550 {
    base: u16,
    name: &'static str,
}

impl Uart16550 {
    /// Create a new 16550 UART driver for the given I/O port base.
    ///
    /// This performs one-time hardware initialisation:
    ///
    /// 1. Disable all interrupts.
    /// 2. Set baud rate divisor to 1 (115200 baud).
    /// 3. Configure 8N1 line format.
    /// 4. Enable and reset FIFOs.
    /// 5. Set DTR/RTS.
    pub fn new(base: u16) -> Self {
        let mut uart = Self {
            base,
            name: "uart16550",
        };
        uart.init();
        uart
    }

    /// One-time hardware initialisation.
    fn init(&mut self) {
        // 1. Disable all interrupts.
        unsafe {
            portio::outb(self.base + IER, 0x00);
        }

        // 2. Enable DLAB to set baud rate divisor.
        unsafe {
            portio::outb(self.base + LCR, LCR_DLAB);
        }

        // 3. Set divisor low byte (1 = 115200 baud).
        unsafe {
            portio::outb(self.base + THR_RBR, (BAUD_115200_DIV & 0xFF) as u8);
        }

        // 4. Set divisor high byte.
        unsafe {
            portio::outb(self.base + IER, ((BAUD_115200_DIV >> 8) & 0xFF) as u8);
        }

        // 5. Configure 8 data bits, no parity, 1 stop bit (8N1), clear DLAB.
        unsafe {
            portio::outb(self.base + LCR, LCR_8BIT);
        }

        // 6. Enable and reset FIFOs.
        unsafe {
            portio::outb(
                self.base + IIR_FCR,
                FCR_FIFO_EN | FCR_RX_CLR | FCR_TX_CLR,
            );
        }

        // 7. Set DTR/RTS (modem control).
        unsafe {
            portio::outb(self.base + MCR, MCR_DTR | MCR_RTS);
        }

        // 8. Enable RX interrupt (for future interrupt-driven use).
        unsafe {
            portio::outb(self.base + IER, IER_RX_DATA_AVAIL);
        }
    }

    /// Wait until the transmit holding register is empty.
    fn wait_for_tx(&self) {
        while unsafe { portio::inb(self.base + LSR) & LSR_THRE == 0 } {
            core::hint::spin_loop();
        }
    }
}

impl core::fmt::Write for Uart16550 {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.write_buf(s.as_bytes());
        Ok(())
    }
}

impl UartDriver for Uart16550 {
    fn write_buf(&mut self, buf: &[u8]) {
        for &byte in buf {
            // Wait until the transmit holding register is empty.
            self.wait_for_tx();
            unsafe {
                portio::outb(self.base + THR_RBR, byte);
            }
        }
    }

    fn drain_uart_rx(&mut self, buf: &mut [u8]) -> usize {
        let mut bytes_read = 0;

        while bytes_read < buf.len() {
            let lsr = unsafe { portio::inb(self.base + LSR) };

            if lsr & LSR_DR == 0 {
                break;
            }

            buf[bytes_read] = unsafe { portio::inb(self.base + THR_RBR) };
            bytes_read += 1;
        }

        bytes_read
    }
}

impl Driver for Uart16550 {
    fn name(&self) -> &'static str {
        self.name
    }
}

// ──────────────────────────────────────────────
//  Init function
// ──────────────────────────────────────────────

/// Initialise the 16550 UART and register it as the active console.
///
/// On x86_64, the UART is discovered at a fixed I/O port (COM1 = 0x3F8)
/// rather than via FDT/ACPI probing.  This function creates the driver,
/// wraps it in the generic [`Uart`] wrapper with an interrupt claim, and
/// registers the character device.
pub fn uart16550_init(
    _bus: &mut PlatformBus,
    dm: &mut DriverManager,
) -> Result<()> {
    let interrupt_root = crate::interrupts::get_interrupt_root()
        .ok_or(KernelError::NotSupported)?;

    let interrupt_config = crate::interrupts::InterruptConfig {
        descriptor: InterruptDescriptor::Spi(COM1_IRQ),
        trigger: crate::interrupts::TriggerMode::EdgeRising,
    };

    let uart_cdev = UART_CHAR_DEV
        .get()
        .ok_or(KernelError::NotSupported)?;

    let driver = interrupt_root.claim_interrupt(
        interrupt_config,
        |claimed_interrupt| {
            let hw = Uart16550::new(COM1_BASE);
            Uart::new(hw, claimed_interrupt, "uart16550")
        },
    )?;

    // Register as the active console (minor 0).
    uart_cdev.register_console(driver, true)?;

    info!("x86_64 UART: 16550 registered at COM1 (I/O 0x{:x})", COM1_BASE);

    Ok(())
}

kernel_driver!(uart16550_init);
