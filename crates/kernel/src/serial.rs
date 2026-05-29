//! Polling 16550 serial driver. Transmit waits for the LSR THR-empty bit, which is
//! exactly why the host models the LSR (see vmm/src/serial.rs).

use core::fmt::{self, Write};

use crate::io::{inb, outb};

const COM1: u16 = 0x3f8;
const LSR_THR_EMPTY: u8 = 0x20;

pub struct Serial;

impl Serial {
    /// Classic 16550 init (the emulated UART ignores the divisor, but the sequence is real).
    pub fn init() {
        unsafe {
            outb(COM1 + 1, 0x00); // disable UART interrupts
            outb(COM1 + 3, 0x80); // DLAB on
            outb(COM1 + 0, 0x01); // divisor low (115200 baud, ignored by emulator)
            outb(COM1 + 1, 0x00); // divisor high
            outb(COM1 + 3, 0x03); // 8N1, DLAB off
            outb(COM1 + 2, 0xC7); // enable + clear FIFO, 14-byte threshold
            outb(COM1 + 4, 0x0B); // DTR | RTS | OUT2
        }
    }

    fn write_byte(b: u8) {
        unsafe {
            while inb(COM1 + 5) & LSR_THR_EMPTY == 0 {}
            outb(COM1, b);
        }
    }
}

impl Write for Serial {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for b in s.bytes() {
            Serial::write_byte(b);
        }
        Ok(())
    }
}

/// Write raw bytes to COM1. Used by `sys_write` to forward arbitrary user
/// buffers (which may not be valid UTF-8, so `write_str` won't do).
pub fn write_bytes(bytes: &[u8]) {
    for &b in bytes {
        Serial::write_byte(b);
    }
}
