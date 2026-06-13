//! Guest-side accessors for the emulated host MMIO device (slice 8).
//!
//! The device lives at GPA 0x0400_0000, mapped uncacheable by
//! `paging::map_mmio_page`. `read_volatile`/`write_volatile` forbid the
//! compiler from eliding/reordering/coalescing the accesses; the uncacheable
//! page handles the CPU/cache side.

const MMIO_BASE: usize = 0x0400_0000;
const REG_UPTIME: usize = 0x00;
const REG_POWEROFF: usize = 0x08;
const POWEROFF_MAGIC: u64 = 0x600D_F00D;

/// Read the host's uptime (nanoseconds since the VMM started).
pub fn read_host_uptime_ns() -> u64 {
    unsafe { core::ptr::read_volatile((MMIO_BASE + REG_UPTIME) as *const u64) }
}

/// Ask the host to power the VM off. The host stops the run loop on this
/// write, so on the host side nothing after it runs; the *guest* instruction
/// itself completes, so this returns.
pub fn request_poweroff() {
    unsafe {
        core::ptr::write_volatile((MMIO_BASE + REG_POWEROFF) as *mut u64, POWEROFF_MAGIC);
    }
}
