//! Month-1 COM1 handler: bytes the guest `out`s to port 0x3f8 are written to a sink.
//! This is intentionally NOT a 16550 UART — no status/IER registers. The guest blob
//! writes raw data bytes only. Full UART emulation arrives in Month 2.

use std::io::Write;

pub const COM1_PORT: u16 = 0x3f8;

pub struct Serial<W: Write> {
    out: W,
}

impl<W: Write> Serial<W> {
    pub fn new(out: W) -> Self {
        Self { out }
    }

    /// Write the bytes from a single KVM IO-out exit to the sink.
    /// Errors are intentionally swallowed: a console write failure must not abort the VM.
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        let _ = self.out.write_all(bytes);
        let _ = self.out.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwards_bytes_to_sink() {
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut serial = Serial::new(&mut buf);
            serial.write_bytes(b"hi");
            serial.write_bytes(&[b'!']);
        }
        assert_eq!(buf, b"hi!");
    }
}
