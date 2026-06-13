//! A tiny emulated memory-mapped I/O device — the MMIO sibling of `serial.rs`.
//!
//! The device lives at the first guest-physical address past RAM
//! (`MMIO_BASE == vm::MEM_SIZE`), which KVM does not back with a memslot, so
//! any guest access traps as `KVM_EXIT_MMIO` and lands in the run loop, which
//! routes it here. Two registers:
//!   - 0x00 (read):  host nanoseconds since the VMM started (u64 LE)
//!   - 0x08 (write): write POWEROFF_MAGIC → ask the VMM to stop the VM
//!
//! See docs/superpowers/specs/2026-06-11-minikvm-mmio-device-design.md.

use std::time::Instant;

/// First GPA past the 64 MiB RAM region — unbacked, so accesses trap as MMIO.
/// Tied to `vm::MEM_SIZE` so the two can never drift.
pub const MMIO_BASE: u64 = crate::vm::MEM_SIZE as u64;
/// Device window size (one page is ample for two registers).
pub const MMIO_LEN: u64 = 0x1000;

const REG_UPTIME: u64 = 0x00;
const REG_POWEROFF: u64 = 0x08;
const POWEROFF_MAGIC: u64 = 0x600D_F00D;

/// What a write asks the run loop to do.
pub enum MmioAction {
    None,
    PowerOff,
}

pub struct MmioDevice {
    start: Instant,
    powered_off: bool,
}

impl MmioDevice {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
            powered_off: false,
        }
    }

    /// True iff `addr` falls in this device's MMIO window. Associated (not a
    /// method) so the run loop can range-check before borrowing the device.
    pub fn claims(addr: u64) -> bool {
        (MMIO_BASE..MMIO_BASE + MMIO_LEN).contains(&addr)
    }

    /// True once the guest has written the poweroff register (for the host
    /// summary line in `main`).
    pub fn powered_off(&self) -> bool {
        self.powered_off
    }

    /// Service a guest read: fill `data` (1..=8 bytes) with the register's
    /// little-endian value. Unknown offsets read as 0. Clamps to 8 bytes so a
    /// wider access cannot panic.
    pub fn read(&self, addr: u64, data: &mut [u8]) {
        let val: u64 = match addr - MMIO_BASE {
            REG_UPTIME => self.start.elapsed().as_nanos() as u64,
            _ => 0,
        };
        let bytes = val.to_le_bytes();
        let n = data.len().min(bytes.len());
        data[..n].copy_from_slice(&bytes[..n]);
    }

    /// Service a guest write: returns the action the run loop must take.
    /// Clamps to 8 bytes defensively.
    pub fn write(&mut self, addr: u64, data: &[u8]) -> MmioAction {
        let mut buf = [0u8; 8];
        let n = data.len().min(buf.len());
        buf[..n].copy_from_slice(&data[..n]);
        let val = u64::from_le_bytes(buf);
        match addr - MMIO_BASE {
            REG_POWEROFF if val == POWEROFF_MAGIC => {
                self.powered_off = true;
                MmioAction::PowerOff
            }
            _ => MmioAction::None,
        }
    }
}

impl Default for MmioDevice {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claims_window_boundaries() {
        assert!(MmioDevice::claims(MMIO_BASE));
        assert!(MmioDevice::claims(MMIO_BASE + MMIO_LEN - 1));
        assert!(!MmioDevice::claims(MMIO_BASE - 1));
        assert!(!MmioDevice::claims(MMIO_BASE + MMIO_LEN));
    }

    #[test]
    fn uptime_read_fills_eight_bytes_and_is_monotonic() {
        let dev = MmioDevice::new();
        let mut a = [0u8; 8];
        dev.read(MMIO_BASE + REG_UPTIME, &mut a);
        std::thread::sleep(std::time::Duration::from_millis(2));
        let mut b = [0u8; 8];
        dev.read(MMIO_BASE + REG_UPTIME, &mut b);
        let (va, vb) = (u64::from_le_bytes(a), u64::from_le_bytes(b));
        assert!(vb >= va, "uptime must be monotonic: {vb} >= {va}");
        assert!(vb > 0, "uptime must advance");
    }

    #[test]
    fn uptime_read_honors_narrow_buffer() {
        let dev = MmioDevice::new();
        let mut buf = [0xAAu8; 4];
        dev.read(MMIO_BASE + REG_UPTIME, &mut buf);
        // Exactly 4 bytes written, no panic. (We can't assert the value, only
        // that it didn't over-read / panic.)
        assert_eq!(buf.len(), 4);
    }

    #[test]
    fn unknown_offset_reads_zero() {
        let dev = MmioDevice::new();
        let mut buf = [0xFFu8; 8];
        dev.read(MMIO_BASE + 0x100, &mut buf);
        assert_eq!(u64::from_le_bytes(buf), 0);
    }

    #[test]
    fn poweroff_magic_triggers_action_and_flag() {
        let mut dev = MmioDevice::new();
        assert!(!dev.powered_off());
        let action = dev.write(MMIO_BASE + REG_POWEROFF, &POWEROFF_MAGIC.to_le_bytes());
        assert!(matches!(action, MmioAction::PowerOff));
        assert!(dev.powered_off());
    }

    #[test]
    fn poweroff_wrong_value_is_noop() {
        let mut dev = MmioDevice::new();
        let action = dev.write(MMIO_BASE + REG_POWEROFF, &0u64.to_le_bytes());
        assert!(matches!(action, MmioAction::None));
        assert!(!dev.powered_off());
    }

    #[test]
    fn poweroff_magic_at_wrong_offset_is_noop() {
        let mut dev = MmioDevice::new();
        // Magic written to the uptime register, not the poweroff register.
        let action = dev.write(MMIO_BASE + REG_UPTIME, &POWEROFF_MAGIC.to_le_bytes());
        assert!(matches!(action, MmioAction::None));
        assert!(!dev.powered_off());
    }
}
