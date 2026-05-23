//! Month-2 16550 UART model (transmit-only, polling-friendly).
//! Models just enough that a guest polling driver works:
//!   - reading LSR always reports "ready to transmit"
//!   - writes to the data register go to the sink, but ONLY when DLAB is clear
//!     (when DLAB is set, a write to the data port is the baud-divisor low byte, not data)
//!   - all other register writes are accepted and ignored
//! This is still NOT a full UART (no receive, no interrupts).

use std::io::Write;

pub const COM1_BASE: u16 = 0x3f8;
pub const COM1_LAST: u16 = COM1_BASE + 7;

const REG_DATA: u16 = 0; // THR (or divisor-low when DLAB=1)
const REG_LCR: u16 = 3; // line control; bit 7 = DLAB
const REG_LSR: u16 = 5; // line status (read-only)

const LSR_READY: u8 = 0x60; // THR-empty (0x20) | transmitter-empty (0x40)
const LCR_DLAB: u8 = 0x80;

pub struct Uart<W: Write> {
    out: W,
    dlab: bool,
}

impl<W: Write> Uart<W> {
    pub fn new(out: W) -> Self {
        Self { out, dlab: false }
    }

    /// Handle a guest `out` of one byte to a UART port.
    pub fn write_reg(&mut self, port: u16, value: u8) {
        match port - COM1_BASE {
            REG_DATA if !self.dlab => {
                let _ = self.out.write_all(&[value]);
                let _ = self.out.flush();
            }
            REG_LCR => self.dlab = value & LCR_DLAB != 0,
            _ => {} // divisor latch, IER, FCR, MCR, ... — accept and ignore
        }
    }

    /// Handle a guest `in` from a UART port; returns the byte to give the guest.
    pub fn read_reg(&self, port: u16) -> u8 {
        match port - COM1_BASE {
            REG_LSR => LSR_READY,
            _ => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_write_forwards_when_dlab_clear() {
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut u = Uart::new(&mut buf);
            u.write_reg(COM1_BASE, b'A');
        }
        assert_eq!(buf, b"A");
    }

    #[test]
    fn divisor_write_suppressed_when_dlab_set() {
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut u = Uart::new(&mut buf);
            u.write_reg(COM1_BASE + 3, 0x80); // LCR: set DLAB
            u.write_reg(COM1_BASE, 0x0C); // divisor low — must NOT print
            u.write_reg(COM1_BASE + 3, 0x03); // LCR: clear DLAB, 8N1
            u.write_reg(COM1_BASE, b'B'); // real data
        }
        assert_eq!(buf, b"B");
    }

    #[test]
    fn lsr_reports_ready() {
        let u = Uart::new(Vec::new());
        assert_eq!(u.read_reg(COM1_BASE + 5), 0x60);
    }

    #[test]
    fn other_reads_are_zero() {
        let u = Uart::new(Vec::new());
        assert_eq!(u.read_reg(COM1_BASE + 1), 0);
    }
}
