//! VM-exit counters, benchmark protocol state, and trace summary line.

use std::time::Instant;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub io_exits: u64,
    pub hlt_exits: u64,
    pub mmio_exits: u64,

    /// Set by the serial state machine when the start marker completes.
    /// (`Stats: PartialEq` is derived and DOES compare these via `Instant`'s
    /// own `PartialEq`. That's harmless in practice — only the existing
    /// `counts_and_formats` test compares Stats values, and it never touches
    /// the bench fields.)
    pub bench_start: Option<Instant>,
    pub bench_end: Option<Instant>,
    /// Iteration count carried in the start marker's payload (LE u64).
    /// **Set by the user program, not the host** — eliminates cross-language
    /// constant drift.
    pub bench_iters: u64,
}

impl Stats {
    /// Benchmark-aware constructor; preferred over `Default::default()`.
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_io(&mut self) {
        self.io_exits += 1;
    }

    pub fn record_hlt(&mut self) {
        self.hlt_exits += 1;
    }

    pub fn record_mmio(&mut self) {
        self.mmio_exits += 1;
    }

    /// Returns per-syscall latency in nanoseconds, or `None` if either marker
    /// is missing or the iteration count is zero.
    pub fn avg_syscall_ns(&self) -> Option<u128> {
        let start = self.bench_start?;
        let end = self.bench_end?;
        if self.bench_iters == 0 {
            return None;
        }
        Some(end.duration_since(start).as_nanos() / self.bench_iters as u128)
    }

    /// One-line summary matching the project's demo format.
    pub fn summary(&self) -> String {
        format!(
            "[host] VM exits: io={}, hlt={}, mmio={}",
            self.io_exits, self.hlt_exits, self.mmio_exits
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_and_formats() {
        let mut s = Stats::new();
        s.record_io();
        s.record_io();
        s.record_hlt();
        assert_eq!(s.io_exits, 2);
        assert_eq!(s.hlt_exits, 1);
        assert_eq!(s.summary(), "[host] VM exits: io=2, hlt=1, mmio=0");
    }

    #[test]
    fn avg_syscall_ns_returns_none_when_incomplete() {
        let s = Stats::new();
        assert_eq!(s.avg_syscall_ns(), None);
    }

    #[test]
    fn avg_syscall_ns_returns_none_when_iters_zero() {
        let mut s = Stats::new();
        s.bench_start = Some(Instant::now());
        s.bench_end = Some(Instant::now());
        // bench_iters stays 0
        assert_eq!(s.avg_syscall_ns(), None);
    }
}
