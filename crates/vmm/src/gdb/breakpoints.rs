//! Software breakpoint byte-swap helpers — slice 7.
//!
//! KVM_GUESTDBG_USE_SW_BP makes KVM vmexit on any guest int3 (KVM_EXIT_DEBUG).
//! The stub places the 0xCC byte (gdb sends Z0,addr,kind, NOT raw M+0xCC) and
//! restores the original on z0.

use vm_memory::{Bytes, GuestAddress, GuestMemoryMmap};

use crate::gdb::errors::StubError;

/// Save the byte at `addr` and write 0xCC. Returns the saved byte.
pub fn place(mem: &GuestMemoryMmap, addr: u64) -> Result<u8, StubError> {
    let mut buf = [0u8; 1];
    mem.read_slice(&mut buf, GuestAddress(addr))?;
    mem.write_slice(&[0xCC], GuestAddress(addr))?;
    Ok(buf[0])
}

/// Restore `byte` at `addr`.
pub fn restore(mem: &GuestMemoryMmap, addr: u64, byte: u8) -> Result<(), StubError> {
    mem.write_slice(&[byte], GuestAddress(addr))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use vm_memory::GuestMemoryMmap;

    fn mem() -> GuestMemoryMmap<()> {
        GuestMemoryMmap::from_ranges(&[(GuestAddress(0), crate::vm::MEM_SIZE)]).unwrap()
    }

    #[test]
    fn place_swaps_byte_for_0xcc() {
        let m = mem();
        m.write_slice(&[0xAA], GuestAddress(0x12345)).unwrap();
        let saved = place(&m, 0x12345).unwrap();
        assert_eq!(saved, 0xAA);
        let mut buf = [0u8; 1];
        m.read_slice(&mut buf, GuestAddress(0x12345)).unwrap();
        assert_eq!(buf[0], 0xCC);
    }

    #[test]
    fn restore_writes_saved_byte() {
        let m = mem();
        m.write_slice(&[0xCC], GuestAddress(0x67890)).unwrap();
        restore(&m, 0x67890, 0x11).unwrap();
        let mut buf = [0u8; 1];
        m.read_slice(&mut buf, GuestAddress(0x67890)).unwrap();
        assert_eq!(buf[0], 0x11);
    }
}
