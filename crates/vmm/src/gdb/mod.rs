//! GDB remote-serial-protocol stub for the MiniKVM VMM.
//!
//! Slice 7 — see docs/superpowers/specs/2026-06-07-minikvm-month3plus-gdb-stub-design.md.

pub mod errors;
mod breakpoints;
mod packets;
mod regs;
pub(crate) mod rsp;

pub use errors::StubError;

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;

use anyhow::Result;
use kvm_ioctls::VcpuFd;
use vm_memory::GuestMemoryMmap;

use crate::serial::Uart;
use crate::stats::Stats;

use packets::{DispatchAction, dispatch_stateless};

/// Single-session GDB stub state. Owned by `serve`.
///
/// Three generic params: `'a` for the borrowed VcpuFd/mem/uart/stats,
/// `S: Read + Write` for the gdb socket (TcpStream in production, BiCursor/etc.
/// in tests), `W: Write` for the Uart sink (StdoutLock in production).
pub struct Stub<'a, S: Read + Write, W: Write> {
    pub(crate) vcpu: &'a mut VcpuFd,
    pub(crate) mem: &'a GuestMemoryMmap,
    #[allow(dead_code)]
    pub(crate) uart: &'a mut Uart<W>,
    #[allow(dead_code)]
    pub(crate) stats: &'a mut Stats,
    pub(crate) stream: S,
    pub(crate) breakpoints: HashMap<u64, u8>,
    pub(crate) noack_mode: bool,
    pub(crate) singlestep: bool,
}

impl<'a, S: Read + Write, W: Write> Stub<'a, S, W> {
    pub fn new(
        vcpu: &'a mut VcpuFd,
        mem: &'a GuestMemoryMmap,
        uart: &'a mut Uart<W>,
        stats: &'a mut Stats,
        stream: S,
    ) -> Self {
        Self {
            vcpu,
            mem,
            uart,
            stats,
            stream,
            breakpoints: HashMap::new(),
            noack_mode: false,
            singlestep: false,
        }
    }

    /// Run the Stopped↔Running loop until D / EOF / Hlt / k.
    /// In Task 4, only the Stopped state is implemented; Running comes in Task 7.
    pub fn run_session(&mut self) -> Result<(), StubError> {
        loop {
            let pkt = match rsp::read_packet(&mut self.stream, self.noack_mode) {
                Ok(p) => p,
                // A dead socket OR a corrupt/oversized inbound packet means the
                // stream is unusable; detach cleanly rather than crash the VMM.
                Err(StubError::SocketDied) | Err(StubError::ProtocolViolation) => {
                    self.cleanup()?;
                    return Ok(());
                }
                Err(e) => return Err(e), // only Fatal (KVM ioctl) reaches here
            };

            let action = self.dispatch(&pkt)?;
            match action {
                DispatchAction::Reply(payload) => {
                    rsp::write_packet(&mut self.stream, &payload, self.noack_mode)?;
                }
                DispatchAction::AlreadyReplied => continue,
                DispatchAction::Resume { singlestep: _ } => {
                    // Task 7 fills this in. For now, treat as protocol violation
                    // to keep tests focused on Stopped-state behavior.
                    return Err(StubError::ProtocolViolation);
                }
                DispatchAction::Detach => {
                    self.cleanup()?;
                    return Ok(());
                }
                DispatchAction::Kill => {
                    self.cleanup()?;
                    std::process::exit(0);
                }
            }
        }
    }

    /// Run dispatch for one packet. Folds in the QStartNoAckMode cutover.
    fn dispatch(&mut self, pkt: &[u8]) -> Result<DispatchAction, StubError> {
        if pkt == b"QStartNoAckMode" {
            return self.handle_qstart_noack();
        }
        if pkt == b"g" {
            let regs = self.vcpu.get_regs()?;
            let sregs = self.vcpu.get_sregs()?;
            return Ok(DispatchAction::Reply(regs::encode_regs(&regs, &sregs).into()));
        }
        if pkt.starts_with(b"G") {
            let Ok(hex) = std::str::from_utf8(&pkt[1..]) else {
                return Ok(DispatchAction::Reply(std::borrow::Cow::Borrowed("E22")));
            };
            if hex.len() != regs::G_PACKET_HEX_CHARS {
                return Ok(DispatchAction::Reply(std::borrow::Cow::Borrowed("E22")));
            }
            let mut regs = self.vcpu.get_regs()?;
            let mut sregs = self.vcpu.get_sregs()?;
            if regs::decode_regs(hex, &mut regs, &mut sregs).is_err() {
                return Ok(DispatchAction::Reply(std::borrow::Cow::Borrowed("E22")));
            }
            self.vcpu.set_regs(&regs)?;
            return Ok(DispatchAction::Reply(std::borrow::Cow::Borrowed("OK")));
        }
        if pkt.starts_with(b"m") {
            // m<addr>,<len> — addr and len are hex.
            let Ok(tail) = std::str::from_utf8(&pkt[1..]) else {
                return Ok(DispatchAction::Reply(std::borrow::Cow::Borrowed("E22")));
            };
            let Some((addr_s, len_s)) = tail.split_once(',') else {
                return Ok(DispatchAction::Reply(std::borrow::Cow::Borrowed("E22")));
            };
            let (Ok(addr), Ok(len)) = (
                u64::from_str_radix(addr_s, 16),
                usize::from_str_radix(len_s, 16),
            ) else {
                return Ok(DispatchAction::Reply(std::borrow::Cow::Borrowed("E22")));
            };
            if !bounded(addr, len) {
                return Ok(DispatchAction::Reply(std::borrow::Cow::Borrowed("E14")));
            }
            let mut buf = vec![0u8; len];
            use vm_memory::{Bytes, GuestAddress};
            self.mem.read_slice(&mut buf, GuestAddress(addr))?; // Fatal-by-design if it fails
            // Capacity hint only; `len` is already bounds-checked (<= MEM_SIZE),
            // so this cannot overflow. Use saturating_mul to keep every length
            // multiply in the dispatch path overflow-safe by construction.
            let mut hex = String::with_capacity(len.saturating_mul(2));
            for b in &buf {
                hex.push_str(&format!("{:02x}", b));
            }
            return Ok(DispatchAction::Reply(hex.into()));
        }
        if pkt.starts_with(b"M") {
            // M<addr>,<len>:<hex>
            let Ok(tail) = std::str::from_utf8(&pkt[1..]) else {
                return Ok(DispatchAction::Reply(std::borrow::Cow::Borrowed("E22")));
            };
            let Some((head, payload)) = tail.split_once(':') else {
                return Ok(DispatchAction::Reply(std::borrow::Cow::Borrowed("E22")));
            };
            let Some((addr_s, len_s)) = head.split_once(',') else {
                return Ok(DispatchAction::Reply(std::borrow::Cow::Borrowed("E22")));
            };
            let (Ok(addr), Ok(len)) = (
                u64::from_str_radix(addr_s, 16),
                usize::from_str_radix(len_s, 16),
            ) else {
                return Ok(DispatchAction::Reply(std::borrow::Cow::Borrowed("E22")));
            };
            // Overflow-safe: a `len` so large that 2*len wraps cannot match any
            // real payload length, so treat it as a malformed write (E22).
            if !hex_len_matches(len, payload.len()) {
                return Ok(DispatchAction::Reply(std::borrow::Cow::Borrowed("E22")));
            }
            if !bounded(addr, len) {
                return Ok(DispatchAction::Reply(std::borrow::Cow::Borrowed("E14")));
            }
            let mut buf = Vec::with_capacity(len);
            for i in 0..len {
                let Ok(byte) = u8::from_str_radix(&payload[i * 2..i * 2 + 2], 16) else {
                    return Ok(DispatchAction::Reply(std::borrow::Cow::Borrowed("E22")));
                };
                buf.push(byte);
            }
            use vm_memory::{Bytes, GuestAddress};
            self.mem.write_slice(&buf, GuestAddress(addr))?; // Fatal-by-design if it fails
            return Ok(DispatchAction::Reply(std::borrow::Cow::Borrowed("OK")));
        }
        if pkt.starts_with(b"qXfer:features:read:target.xml:") {
            // ".:<off>,<len>" — offset and length are hex per RSP. A malformed
            // qXfer must NOT crash the stub: reply empty (gdb's "unsupported"
            // signal, the safe fallback for a read it can't fulfill).
            let tail = &pkt[b"qXfer:features:read:target.xml:".len()..];
            let Ok(s) = std::str::from_utf8(tail) else {
                return Ok(DispatchAction::Reply(std::borrow::Cow::Borrowed("")));
            };
            let Some((off_s, len_s)) = s.split_once(',') else {
                return Ok(DispatchAction::Reply(std::borrow::Cow::Borrowed("")));
            };
            let Ok(off) = usize::from_str_radix(off_s, 16) else {
                return Ok(DispatchAction::Reply(std::borrow::Cow::Borrowed("")));
            };
            let Ok(len) = usize::from_str_radix(len_s, 16) else {
                return Ok(DispatchAction::Reply(std::borrow::Cow::Borrowed("")));
            };
            return Ok(DispatchAction::Reply(regs::target_xml_chunk(off, len).into()));
        }
        // Stateful handlers (g, G, m, M, Z0, z0, c, s) come in Tasks 5/6/7.
        // For now, fall through to dispatch_stateless and reply empty for
        // anything else.
        if let Some(a) = dispatch_stateless(pkt) {
            return Ok(a);
        }
        Ok(DispatchAction::Reply(std::borrow::Cow::Borrowed("")))
    }

    fn handle_qstart_noack(&mut self) -> Result<DispatchAction, StubError> {
        // §6.6 five-step cutover. We're at step 1 (just received the packet).
        // The read_packet that delivered us emitted '+' as step 2 already.
        // Steps 3–5 live in qstart_noack_cutover so they're unit-testable
        // without a live VcpuFd: flip the flag only AFTER the cutover returns Ok.
        qstart_noack_cutover(&mut self.stream)?;
        self.noack_mode = true;
        Ok(DispatchAction::AlreadyReplied)
    }

    fn cleanup(&mut self) -> Result<(), StubError> {
        // Task 7 implements memory restoration + KVM_SET_GUEST_DEBUG clear.
        Ok(())
    }
}

/// True iff [addr .. addr+len) fits inside the guest physical address space.
/// Overflow-safe: a wrapping addr+len returns false (rejected).
pub(crate) fn bounded(addr: u64, len: usize) -> bool {
    match addr.checked_add(len as u64) {
        Some(end) => end <= crate::vm::MEM_SIZE as u64,
        None => false,
    }
}

/// True iff `payload_hex_len` is exactly twice `len` (the M-packet contract),
/// overflow-safe. A `len` so large that 2*len wraps returns false.
pub(crate) fn hex_len_matches(len: usize, payload_hex_len: usize) -> bool {
    len.checked_mul(2) == Some(payload_hex_len)
}

/// Perform the QStartNoAckMode cutover on a raw stream. Returns Ok(()) only
/// if the OK reply was written AND gdb acked it (write_packet in ack mode
/// reads the trailing '+'). The caller flips its `noack_mode` flag ONLY after
/// this returns Ok — so a write/ack failure leaves both sides in ack mode,
/// which is the spec-mandated recovery (§6.6).
pub(crate) fn qstart_noack_cutover<S: std::io::Read + std::io::Write>(
    stream: &mut S,
) -> Result<(), StubError> {
    rsp::write_packet(stream, "OK", false)
}

/// Connection handshake. Binds, accepts one client, runs the session.
pub fn serve<W: Write>(
    vcpu: &mut VcpuFd,
    mem: &GuestMemoryMmap,
    uart: &mut Uart<W>,
    stats: &mut Stats,
    port: u16,
) -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let bound = listener.local_addr()?;
    // stderr so the integration test can capture the line independently of stdout.
    eprintln!(
        "[host] gdb stub listening on 127.0.0.1:{} (waiting for client)",
        bound.port()
    );
    let (stream, _) = listener.accept()?;
    eprintln!("[host] gdb client connected");
    let _ = stream.set_read_timeout(None);

    let mut stub = Stub::new(vcpu, mem, uart, stats, stream);
    stub.run_session().map_err(|e| match e {
        StubError::Fatal(a) => a,
        other => anyhow::anyhow!("unexpected StubError leak: {other:?}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Cursor};

    /// Read+Write helper for unit tests.
    struct BiCursor {
        inbox: Cursor<Vec<u8>>,
        outbox: Vec<u8>,
    }
    impl BiCursor {
        fn new(input: &[u8]) -> Self {
            Self { inbox: Cursor::new(input.to_vec()), outbox: Vec::new() }
        }
    }
    impl Read for BiCursor {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> { self.inbox.read(buf) }
    }
    impl Write for BiCursor {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.outbox.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }

    fn process_one_packet(input: &[u8], noack_mode: bool) -> (Vec<u8>, Vec<u8>) {
        let mut b = BiCursor::new(input);
        let pkt = rsp::read_packet(&mut b, noack_mode).unwrap();
        let action = dispatch_stateless(&pkt).expect("test packets must be stateless");
        let DispatchAction::Reply(payload) = action else {
            panic!("test packets must reply");
        };
        rsp::write_packet(&mut b, &payload, noack_mode).unwrap();
        (b.outbox, pkt)
    }

    #[test]
    fn session_handles_qattached_with_correct_checksum() {
        // qAttached payload: 0x71+0x41+0x74+0x74+0x61+0x63+0x68+0x65+0x64
        //                  = 0x38F → low byte 0x8F.
        // In ack-mode we'd need to provide a '+' for the reply ack; use noack mode
        // to keep the test simple (cutover happens before this in real sessions).
        let pkt = b"$qAttached#8f";
        let (out, _) = process_one_packet(pkt, true);
        let s = std::str::from_utf8(&out).unwrap();
        // payload "1" → checksum 0x31, framed as "$1#31".
        assert_eq!(s, "$1#31");
    }

    #[test]
    fn session_handles_question_mark() {
        // '?' = 0x3F → checksum = 0x3F.
        let pkt = b"$?#3f";
        let (out, _) = process_one_packet(pkt, true);
        let s = std::str::from_utf8(&out).unwrap();
        // payload "T05thread:1;" — checksum hand-computed:
        // T05thread:1; = 0x54+0x30+0x35+0x74+0x68+0x72+0x65+0x61+0x64+0x3a+0x31+0x3b
        //              = 0x3D7 → low byte 0xD7.
        assert_eq!(s, "$T05thread:1;#d7");
    }

    #[test]
    fn qstart_noack_cutover_writes_ok_then_succeeds() {
        // write_packet in ack mode writes "$OK#9a" then reads gdb's '+' ack.
        // Feed a '+' in the inbox so the ack-read succeeds.
        let mut b = BiCursor::new(b"+");
        qstart_noack_cutover(&mut b).unwrap();
        assert_eq!(b.outbox, b"$OK#9a");
    }

    #[test]
    fn qstart_noack_cutover_propagates_write_failure() {
        // Empty inbox: write_packet in ack mode will block trying to read the
        // '+' ack, then hit EOF (BiCursor returns Ok(0)) → read_exact_retry
        // returns UnexpectedEof → StubError::SocketDied. The cutover must
        // propagate the error so the caller does NOT flip noack_mode.
        let mut b = BiCursor::new(b"");
        let result = qstart_noack_cutover(&mut b);
        assert!(result.is_err());
    }

    #[test]
    fn bounds_inside_ok() {
        assert!(bounded(0x0, 8));
        assert!(bounded(0x1000, 0x1000));
        assert!(bounded((crate::vm::MEM_SIZE - 8) as u64, 8));
    }

    #[test]
    fn bounds_past_end_rejected() {
        assert!(!bounded(crate::vm::MEM_SIZE as u64, 1));
        assert!(!bounded((crate::vm::MEM_SIZE - 4) as u64, 8));
    }

    #[test]
    fn bounds_overflow_rejected() {
        assert!(!bounded(u64::MAX - 4, 8));
        assert!(!bounded(u64::MAX, 1));
    }

    #[test]
    fn hex_len_matches_normal() {
        assert!(hex_len_matches(4, 8));       // 4 bytes = 8 hex chars
        assert!(!hex_len_matches(4, 7));      // mismatch
    }

    #[test]
    fn hex_len_matches_overflow_is_false() {
        // 0x8000000000000000 * 2 overflows usize → must be false, not a panic.
        assert!(!hex_len_matches(0x8000_0000_0000_0000, 0));
        assert!(!hex_len_matches(usize::MAX, 0));
    }

    #[test]
    fn vm_memory_roundtrip_via_read_write_slice() {
        use vm_memory::{Bytes, GuestAddress, GuestMemoryMmap};
        let mem: GuestMemoryMmap<()> = GuestMemoryMmap::from_ranges(&[
            (GuestAddress(0), crate::vm::MEM_SIZE)
        ]).unwrap();
        mem.write_slice(&[0xCC], GuestAddress(0x800000)).unwrap();
        let mut buf = [0u8; 1];
        mem.read_slice(&mut buf, GuestAddress(0x800000)).unwrap();
        assert_eq!(buf[0], 0xCC);
    }
}
