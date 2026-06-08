//! GDB Remote Serial Protocol wire framing.
//!
//! Wire format: $<payload>#<2-hex-checksum>.
//! Acknowledgement: '+' / '-' before each packet (until QStartNoAckMode flips).
//! Escape: 0x7d <byte ^ 0x20> for $, #, *, } in payload.
//! RLE inbound: <byte>*<count_byte> = (count_byte - 29) ADDITIONAL copies.
//! Skip-noise: read_packet drops leading '+' / '-' / 0x03 before '$'.

use std::io::{self, Read, Write};

use crate::gdb::errors::StubError;

/// Maximum inbound packet payload we'll buffer before declaring the stream
/// corrupt. We advertise PacketSize=4000 (16 KiB) to gdb; allow generous
/// headroom for escaping/RLE, then reject. Prevents an unbounded-Vec OOM from
/// a peer that never sends the closing '#'.
const MAX_INBOUND_PAYLOAD: usize = 0x10000; // 64 KiB

/// EINTR-resistant blocking read into a slice.
pub fn read_exact_retry<R: Read>(r: &mut R, buf: &mut [u8]) -> io::Result<()> {
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..]) {
            Ok(0) => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "peer closed")),
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Read one framed packet from `s`. Strips '$' / '#cc' framing, validates
/// checksum, sends '+'/'-' on `s` unless `noack_mode`. Skips leading
/// '+', '-', 0x03 noise bytes before '$'.
///
/// Returns the payload bytes (post-RLE-decode, post-escape-decode).
/// On EOF (peer closed) returns `Err(StubError::SocketDied)`.
pub fn read_packet<S: Read + Write>(
    s: &mut S,
    noack_mode: bool,
) -> Result<Vec<u8>, StubError> {
    // Outer retry loop on bad checksum (in ack mode).
    loop {
        // Skip-noise: read until '$'. Drop everything else (including any
        // stray byte) — '$' is the only legal packet-start signal.
        let mut byte = [0u8; 1];
        loop {
            read_exact_retry(s, &mut byte)?;
            if byte[0] == b'$' {
                break;
            }
            // bytes consumed and discarded; in particular leading +/-/0x03
            // are gdb's prior-stop ack / Ctrl-C, never payload.
        }

        // Read payload up to '#'.
        let mut payload: Vec<u8> = Vec::with_capacity(64);
        loop {
            read_exact_retry(s, &mut byte)?;
            if byte[0] == b'#' {
                break;
            }
            payload.push(byte[0]);
            if payload.len() > MAX_INBOUND_PAYLOAD {
                return Err(StubError::ProtocolViolation);
            }
        }

        // Read 2 hex chars of checksum.
        let mut cksum_bytes = [0u8; 2];
        read_exact_retry(s, &mut cksum_bytes)?;
        let want = parse_hex_u8(&cksum_bytes).ok_or(StubError::ProtocolViolation)?;
        let got = payload.iter().fold(0u8, |a, b| a.wrapping_add(*b));

        if want != got {
            if !noack_mode {
                s.write_all(b"-")?;
                s.flush()?;
            }
            continue; // gdb will retransmit
        }

        if !noack_mode {
            s.write_all(b"+")?;
            s.flush()?;
        }

        // Decode RLE (inbound only) and escapes.
        let decoded = decode_payload(&payload)?;
        return Ok(decoded);
    }
}

/// Write `payload` as a framed packet. Returns after the wire bytes are flushed.
/// If `!noack_mode`, blocks reading one byte (the gdb '+' ack) and retransmits
/// on '-' up to MAX_RETRIES times.
pub fn write_packet<S: Read + Write>(
    s: &mut S,
    payload: &str,
    noack_mode: bool,
) -> Result<(), StubError> {
    // Max retransmission attempts (initial send + retries on '-' / stray bytes).
    const MAX_RETRIES: usize = 5;
    let framed = frame_packet(payload);

    for _ in 0..MAX_RETRIES {
        s.write_all(&framed)?;
        s.flush()?;

        if noack_mode {
            return Ok(());
        }
        let mut byte = [0u8; 1];
        read_exact_retry(s, &mut byte)?;
        match byte[0] {
            b'+' => return Ok(()),
            b'-' => continue,
            // Stray byte: tolerate by retransmitting.
            _ => continue,
        }
    }
    Err(StubError::ProtocolViolation)
}

/// Build `$<escaped-payload>#<2-hex-checksum>` over the given payload string.
/// Outbound: no RLE; escape only $, #, *, } via 0x7d <byte ^ 0x20>.
pub fn frame_packet(payload: &str) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::with_capacity(payload.len() + 5);
    let mut cksum: u8 = 0;
    out.push(b'$');
    for &b in payload.as_bytes() {
        if matches!(b, b'$' | b'#' | b'*' | b'}') {
            out.push(b'}');
            cksum = cksum.wrapping_add(b'}');
            let xored = b ^ 0x20;
            out.push(xored);
            cksum = cksum.wrapping_add(xored);
        } else {
            out.push(b);
            cksum = cksum.wrapping_add(b);
        }
    }
    out.push(b'#');
    out.extend_from_slice(format!("{:02x}", cksum).as_bytes());
    out
}

fn parse_hex_u8(s: &[u8]) -> Option<u8> {
    if s.len() != 2 {
        return None;
    }
    let hi = (s[0] as char).to_digit(16)? as u8;
    let lo = (s[1] as char).to_digit(16)? as u8;
    Some((hi << 4) | lo)
}

/// Decode RLE and escape sequences in a payload that has already had the
/// '$' .. '#cc' framing stripped. Inbound only — no outbound RLE.
pub(crate) fn decode_payload(raw: &[u8]) -> Result<Vec<u8>, StubError> {
    let mut out: Vec<u8> = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        let b = raw[i];
        if b == b'}' {
            // Escape: next byte XOR 0x20.
            i += 1;
            if i >= raw.len() {
                return Err(StubError::ProtocolViolation);
            }
            out.push(raw[i] ^ 0x20);
            i += 1;
        } else if b == b'*' {
            // RLE: count = next_byte - 29 ADDITIONAL copies of the prior byte.
            // Reject count_byte < 29 or framing collisions.
            let &prev = out.last().ok_or(StubError::ProtocolViolation)?;
            i += 1;
            if i >= raw.len() {
                return Err(StubError::ProtocolViolation);
            }
            let count_byte = raw[i];
            if count_byte < 29 || count_byte == b'#' || count_byte == b'$' {
                return Err(StubError::ProtocolViolation);
            }
            let extra = (count_byte - 29) as usize;
            for _ in 0..extra {
                out.push(prev);
            }
            i += 1;
        } else {
            out.push(b);
            i += 1;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Read+Write helper for unit tests. Reads draw from `inbox` (a Cursor),
    /// writes append to `outbox`.
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

    #[test]
    fn frame_ok() {
        assert_eq!(std::str::from_utf8(&frame_packet("OK")).unwrap(), "$OK#9a");
    }

    #[test]
    fn decode_empty_packet() {
        // "$#00" — empty payload, checksum 0.
        let mut b = BiCursor::new(b"$#00");
        let payload = read_packet(&mut b, true).unwrap();
        assert!(payload.is_empty());
    }

    #[test]
    fn escape_round_trip() {
        // Payload containing $, #, *, }
        let payload = "ab$cd#ef*gh}ij";
        let framed = frame_packet(payload);
        // Strip framing and decode.
        // $...#cc — strip leading '$' and trailing '#xx'.
        let s = &framed[1..framed.len() - 3];
        let decoded = decode_payload(s).unwrap();
        assert_eq!(std::str::from_utf8(&decoded).unwrap(), payload);
    }

    #[test]
    fn checksum_over_escaped_bytes() {
        // Hand-computed: payload "a$" frames as "$a}\x04#cc".
        // Bytes inside $..#: 'a' = 0x61, '}' = 0x7d, '\x04' = 0x04 (= $ ^ 0x20).
        // Sum = 0x61 + 0x7d + 0x04 = 0xE2.
        let framed = frame_packet("a$");
        let s = std::str::from_utf8(&framed).unwrap();
        assert_eq!(s, "$a}\x04#e2");
    }

    #[test]
    fn rle_decode_basic() {
        // Payload "0*\x1f": '0' then '*' then 0x1f=31 → (31-29)=2 additional copies.
        // Total: "000".
        let raw = b"0*\x1f";
        let decoded = decode_payload(raw).unwrap();
        assert_eq!(decoded, b"000");
    }

    #[test]
    fn rle_rejects_low_count_byte() {
        // count_byte = 28 → invalid.
        let raw = b"0*\x1c";
        assert!(decode_payload(raw).is_err());
    }

    #[test]
    fn read_packet_skips_leading_plus() {
        let mut b = BiCursor::new(b"+$qC#b4");
        let payload = read_packet(&mut b, true).unwrap();
        assert_eq!(std::str::from_utf8(&payload).unwrap(), "qC");
    }

    #[test]
    fn read_packet_skips_leading_minus_and_etx() {
        let mut b = BiCursor::new(b"-\x03$qC#b4");
        let payload = read_packet(&mut b, true).unwrap();
        assert_eq!(std::str::from_utf8(&payload).unwrap(), "qC");
    }

    #[test]
    fn read_packet_sends_ack_in_ack_mode() {
        let mut b = BiCursor::new(b"$qC#b4");
        let _ = read_packet(&mut b, false).unwrap();
        assert_eq!(b.outbox, b"+");
    }

    #[test]
    fn read_packet_no_ack_in_noack_mode() {
        let mut b = BiCursor::new(b"$qC#b4");
        let _ = read_packet(&mut b, true).unwrap();
        assert!(b.outbox.is_empty());
    }

    #[test]
    fn write_packet_noack_mode_does_not_read_ack() {
        // Empty input: write_packet must NOT attempt to read in noack mode.
        let mut b = BiCursor::new(b"");
        write_packet(&mut b, "OK", true).unwrap();
        // Verify the framed packet was written.
        assert_eq!(b.outbox, b"$OK#9a");
    }

    #[test]
    fn read_packet_rejects_oversized_payload() {
        // A packet whose payload exceeds MAX_INBOUND_PAYLOAD must be rejected
        // with ProtocolViolation rather than growing the buffer unboundedly.
        // Build "$" + (MAX_INBOUND_PAYLOAD + 1) 'a' bytes + (no '#').
        let mut input = vec![b'$'];
        input.extend(std::iter::repeat(b'a').take(MAX_INBOUND_PAYLOAD + 2));
        let mut b = BiCursor::new(&input);
        let result = read_packet(&mut b, true);
        assert!(matches!(result, Err(StubError::ProtocolViolation)));
    }
}
