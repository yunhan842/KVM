//! VM-exit counters and a trace summary line.

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub io_exits: u64,
    pub hlt_exits: u64,
    pub mmio_exits: u64,
}

impl Stats {
    pub fn record_io(&mut self) {
        self.io_exits += 1;
    }

    pub fn record_hlt(&mut self) {
        self.hlt_exits += 1;
    }

    pub fn record_mmio(&mut self) {
        self.mmio_exits += 1;
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
        let mut s = Stats::default();
        s.record_io();
        s.record_io();
        s.record_hlt();
        assert_eq!(s.io_exits, 2);
        assert_eq!(s.hlt_exits, 1);
        assert_eq!(s.summary(), "[host] VM exits: io=2, hlt=1, mmio=0");
    }
}
