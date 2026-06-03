//! 16550 UART model (transmit-only, polling-friendly) + benchmark-protocol
//! state machine.
//!
//! UART model:
//!   - reading LSR always reports "ready to transmit"
//!   - writes to the data register go to the sink, but ONLY when DLAB is clear
//!     (when DLAB is set, a write to the data port is the baud-divisor low byte)
//!   - all other register writes are accepted and ignored
//!
//! Benchmark protocol (slice 6; design doc:
//! docs/superpowers/specs/2026-06-03-minikvm-month3-polish-design.md §5):
//!   start marker: 0x1B 0x42 0x30 + 8 LE bytes of u64 iteration count
//!   end marker:   0x1B 0x42 0x31
//! Any byte sequence that starts to match but bails out mid-marker is
//! forwarded to the sink intact (fail-safe: collisions degrade to "extra
//! bytes on the wire", never to silent data loss).

use std::io::Write;

use crate::stats::Stats;
use std::time::Instant;

pub const COM1_BASE: u16 = 0x3f8;
pub const COM1_LAST: u16 = COM1_BASE + 7;

const REG_DATA: u16 = 0; // THR (or divisor-low when DLAB=1)
const REG_LCR: u16 = 3; // line control; bit 7 = DLAB
const REG_LSR: u16 = 5; // line status (read-only)

const LSR_READY: u8 = 0x60; // THR-empty (0x20) | transmitter-empty (0x40)
const LCR_DLAB: u8 = 0x80;

// Benchmark protocol bytes.
const ESC: u8 = 0x1B;
const BENCH_LEAD: u8 = b'B'; // 0x42
const BENCH_START: u8 = b'0'; // 0x30
const BENCH_END: u8 = b'1'; // 0x31

/// Benchmark-protocol decoder state.
#[derive(Default)]
enum BenchState {
    #[default]
    Idle,
    SawEsc,
    SawEscB,
    ReadingIters {
        buf: [u8; 8],
        nread: usize,
    },
}

pub struct Uart<W: Write> {
    out: W,
    dlab: bool,
    bench: BenchState,
}

impl<W: Write> Uart<W> {
    pub fn new(out: W) -> Self {
        Self {
            out,
            dlab: false,
            bench: BenchState::Idle,
        }
    }

    /// Handle a guest `out` of one byte to a UART port.
    ///
    /// `stats` is threaded through so the data-port path can drive the
    /// benchmark state machine.
    pub fn write_reg(&mut self, port: u16, value: u8, stats: &mut Stats) {
        match port - COM1_BASE {
            REG_DATA if !self.dlab => {
                self.handle_data_byte(value, stats);
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

    /// Drive the benchmark state machine on a single data-port byte.
    /// Forwards bytes that are not part of the protocol — including bytes
    /// that initially matched but then bailed out.
    fn handle_data_byte(&mut self, value: u8, stats: &mut Stats) {
        match self.bench {
            BenchState::Idle => {
                if value == ESC {
                    self.bench = BenchState::SawEsc;
                } else {
                    self.forward(&[value]);
                }
            }
            BenchState::SawEsc => {
                if value == BENCH_LEAD {
                    self.bench = BenchState::SawEscB;
                } else {
                    // Mismatch: flush stashed ESC + this byte; back to Idle.
                    self.forward(&[ESC, value]);
                    self.bench = BenchState::Idle;
                }
            }
            BenchState::SawEscB => {
                match value {
                    BENCH_START => {
                        self.bench = BenchState::ReadingIters {
                            buf: [0; 8],
                            nread: 0,
                        };
                    }
                    BENCH_END => {
                        stats.bench_end = Some(Instant::now());
                        self.bench = BenchState::Idle;
                    }
                    _ => {
                        // Unknown phase byte: flush stashed ESC, 'B', this byte.
                        self.forward(&[ESC, BENCH_LEAD, value]);
                        self.bench = BenchState::Idle;
                    }
                }
            }
            BenchState::ReadingIters { ref mut buf, ref mut nread } => {
                buf[*nread] = value;
                *nread += 1;
                if *nread == 8 {
                    let iters = u64::from_le_bytes(*buf);
                    stats.bench_iters = iters;
                    stats.bench_start = Some(Instant::now());
                    self.bench = BenchState::Idle;
                }
            }
        }
    }

    fn forward(&mut self, bytes: &[u8]) {
        let _ = self.out.write_all(bytes);
        let _ = self.out.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::Stats;

    #[test]
    fn data_write_forwards_when_dlab_clear() {
        let mut buf: Vec<u8> = Vec::new();
        let mut s = Stats::new();
        {
            let mut u = Uart::new(&mut buf);
            u.write_reg(COM1_BASE, b'A', &mut s);
        }
        assert_eq!(buf, b"A");
    }

    #[test]
    fn divisor_write_suppressed_when_dlab_set() {
        let mut buf: Vec<u8> = Vec::new();
        let mut s = Stats::new();
        {
            let mut u = Uart::new(&mut buf);
            u.write_reg(COM1_BASE + 3, 0x80, &mut s); // LCR: set DLAB
            u.write_reg(COM1_BASE, 0x0C, &mut s); // divisor low — must NOT print
            u.write_reg(COM1_BASE + 3, 0x03, &mut s); // LCR: clear DLAB, 8N1
            u.write_reg(COM1_BASE, b'B', &mut s); // real data
        }
        assert_eq!(buf, b"B");
    }

    #[test]
    fn lsr_reports_ready() {
        let u = Uart::new(Vec::<u8>::new());
        assert_eq!(u.read_reg(COM1_BASE + 5), 0x60);
    }

    #[test]
    fn other_reads_are_zero() {
        let u = Uart::new(Vec::<u8>::new());
        assert_eq!(u.read_reg(COM1_BASE + 1), 0);
    }

    // --- benchmark state machine ---

    #[test]
    fn esc_alone_then_mismatch_forwards_both() {
        let mut buf: Vec<u8> = Vec::new();
        let mut s = Stats::new();
        {
            let mut u = Uart::new(&mut buf);
            u.write_reg(COM1_BASE, 0x1B, &mut s); // ESC: suppress for now
            u.write_reg(COM1_BASE, b'X', &mut s); // not 'B': forward both
        }
        assert_eq!(buf, &[0x1B, b'X']);
        assert!(s.bench_start.is_none());
    }

    #[test]
    fn esc_b_then_mismatch_forwards_three_bytes() {
        let mut buf: Vec<u8> = Vec::new();
        let mut s = Stats::new();
        {
            let mut u = Uart::new(&mut buf);
            for b in [0x1B, b'B', b'Z'] {
                u.write_reg(COM1_BASE, b, &mut s);
            }
        }
        assert_eq!(buf, &[0x1B, b'B', b'Z']);
        assert!(s.bench_start.is_none());
        assert!(s.bench_end.is_none());
    }

    #[test]
    fn full_start_marker_records_start_and_iters() {
        let mut buf: Vec<u8> = Vec::new();
        let mut s = Stats::new();
        {
            let mut u = Uart::new(&mut buf);
            let n: u64 = 10000;
            let mut payload = vec![0x1B, b'B', b'0'];
            payload.extend_from_slice(&n.to_le_bytes());
            for b in payload {
                u.write_reg(COM1_BASE, b, &mut s);
            }
        }
        assert!(buf.is_empty()); // every byte suppressed
        assert!(s.bench_start.is_some());
        assert_eq!(s.bench_iters, 10000);
    }

    #[test]
    fn full_end_marker_records_end() {
        let mut buf: Vec<u8> = Vec::new();
        let mut s = Stats::new();
        {
            let mut u = Uart::new(&mut buf);
            for b in [0x1B, b'B', b'1'] {
                u.write_reg(COM1_BASE, b, &mut s);
            }
        }
        assert!(buf.is_empty());
        assert!(s.bench_end.is_some());
    }

    #[test]
    fn end_marker_then_regular_byte_passes_through() {
        let mut buf: Vec<u8> = Vec::new();
        let mut s = Stats::new();
        {
            let mut u = Uart::new(&mut buf);
            for b in [0x1B, b'B', b'1'] {
                u.write_reg(COM1_BASE, b, &mut s); // end marker
            }
            u.write_reg(COM1_BASE, b'H', &mut s); // regular byte after
        }
        assert_eq!(buf, &[b'H']);
        assert!(s.bench_end.is_some());
    }

}
