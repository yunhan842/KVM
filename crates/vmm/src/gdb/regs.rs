//! Register encode/decode + target.xml constant.
//!
//! Slice-7 register set (24 regs): 16 GPRs + RIP (17×8B = 136B)
//! + EFLAGS (4B) + 6 segs (6×4B = 24B) = 164 bytes / 328 hex chars.
//!
//! Encoding is per-byte little-endian hex. Reference:
//! hex_le_u64(0xDEADBEEFCAFEBABE) == "bebafecaefbeadde"

use kvm_bindings::{kvm_regs, kvm_sregs};

use crate::gdb::errors::StubError;

pub const TARGET_XML: &str = include_str!("target.xml");

/// Total bytes encoded in the `g` packet (= 164). Half of `g`'s hex-char count.
pub const G_PACKET_BYTES: usize = 17 * 8 + 1 * 4 + 6 * 4;
pub const G_PACKET_HEX_CHARS: usize = G_PACKET_BYTES * 2;

pub fn hex_le_u64(v: u64) -> String {
    v.to_le_bytes().iter().map(|b| format!("{:02x}", b)).collect()
}

pub fn hex_le_u32(v: u32) -> String {
    v.to_le_bytes().iter().map(|b| format!("{:02x}", b)).collect()
}

fn parse_hex_le_u64(s: &str) -> Result<u64, StubError> {
    if s.len() != 16 { return Err(StubError::ProtocolViolation); }
    let mut bytes = [0u8; 8];
    for i in 0..8 {
        bytes[i] = u8::from_str_radix(&s[i*2..i*2+2], 16)
            .map_err(|_| StubError::ProtocolViolation)?;
    }
    Ok(u64::from_le_bytes(bytes))
}

fn parse_hex_le_u32(s: &str) -> Result<u32, StubError> {
    if s.len() != 8 { return Err(StubError::ProtocolViolation); }
    let mut bytes = [0u8; 4];
    for i in 0..4 {
        bytes[i] = u8::from_str_radix(&s[i*2..i*2+2], 16)
            .map_err(|_| StubError::ProtocolViolation)?;
    }
    Ok(u32::from_le_bytes(bytes))
}

/// Encode `g` packet payload from a kvm_regs + kvm_sregs snapshot.
/// Result is exactly `G_PACKET_HEX_CHARS` characters.
pub fn encode_regs(regs: &kvm_regs, sregs: &kvm_sregs) -> String {
    let mut s = String::with_capacity(G_PACKET_HEX_CHARS);
    // GPRs in declared order.
    s.push_str(&hex_le_u64(regs.rax));
    s.push_str(&hex_le_u64(regs.rbx));
    s.push_str(&hex_le_u64(regs.rcx));
    s.push_str(&hex_le_u64(regs.rdx));
    s.push_str(&hex_le_u64(regs.rsi));
    s.push_str(&hex_le_u64(regs.rdi));
    s.push_str(&hex_le_u64(regs.rbp));
    s.push_str(&hex_le_u64(regs.rsp));
    s.push_str(&hex_le_u64(regs.r8));
    s.push_str(&hex_le_u64(regs.r9));
    s.push_str(&hex_le_u64(regs.r10));
    s.push_str(&hex_le_u64(regs.r11));
    s.push_str(&hex_le_u64(regs.r12));
    s.push_str(&hex_le_u64(regs.r13));
    s.push_str(&hex_le_u64(regs.r14));
    s.push_str(&hex_le_u64(regs.r15));
    s.push_str(&hex_le_u64(regs.rip));
    // EFLAGS — low 32 bits only; high 32 are AMD64-reserved zero.
    s.push_str(&hex_le_u32(regs.rflags as u32));
    // Segment selectors — full kvm_segment carries base/limit/type but we
    // only round-trip the 16-bit selector zero-extended to 32 bits.
    s.push_str(&hex_le_u32(sregs.cs.selector as u32));
    s.push_str(&hex_le_u32(sregs.ss.selector as u32));
    s.push_str(&hex_le_u32(sregs.ds.selector as u32));
    s.push_str(&hex_le_u32(sregs.es.selector as u32));
    s.push_str(&hex_le_u32(sregs.fs.selector as u32));
    s.push_str(&hex_le_u32(sregs.gs.selector as u32));
    debug_assert_eq!(s.len(), G_PACKET_HEX_CHARS);
    s
}

/// Decode `G` packet payload into `regs` + `sregs`. Caller is responsible
/// for calling `vcpu.set_regs(...)` / `vcpu.set_sregs(...)` on success.
///
/// Segment selectors are SILENTLY IGNORED — see spec §8.3. We read them
/// from the hex and discard; the caller's existing sregs values are kept.
pub fn decode_regs(hex: &str, regs: &mut kvm_regs, sregs: &mut kvm_sregs) -> Result<(), StubError> {
    if hex.len() != G_PACKET_HEX_CHARS {
        return Err(StubError::ProtocolViolation);
    }
    let mut o = 0;
    let take64 = |o: &mut usize| -> Result<u64, StubError> {
        let v = parse_hex_le_u64(&hex[*o..*o+16])?;
        *o += 16;
        Ok(v)
    };
    regs.rax = take64(&mut o)?;
    regs.rbx = take64(&mut o)?;
    regs.rcx = take64(&mut o)?;
    regs.rdx = take64(&mut o)?;
    regs.rsi = take64(&mut o)?;
    regs.rdi = take64(&mut o)?;
    regs.rbp = take64(&mut o)?;
    regs.rsp = take64(&mut o)?;
    regs.r8  = take64(&mut o)?;
    regs.r9  = take64(&mut o)?;
    regs.r10 = take64(&mut o)?;
    regs.r11 = take64(&mut o)?;
    regs.r12 = take64(&mut o)?;
    regs.r13 = take64(&mut o)?;
    regs.r14 = take64(&mut o)?;
    regs.r15 = take64(&mut o)?;
    regs.rip = take64(&mut o)?;
    // EFLAGS: preserve upper 32 (architecturally reserved zero today, but
    // future-proof — see spec §8.3).
    let take32 = |o: &mut usize| -> Result<u32, StubError> {
        let v = parse_hex_le_u32(&hex[*o..*o+8])?;
        *o += 8;
        Ok(v)
    };
    let low_eflags = take32(&mut o)? as u64;
    regs.rflags = (regs.rflags & !0xFFFF_FFFFu64) | low_eflags;
    // Segments: read and discard. sregs is unchanged.
    let _ = take32(&mut o)?; // cs
    let _ = take32(&mut o)?; // ss
    let _ = take32(&mut o)?; // ds
    let _ = take32(&mut o)?; // es
    let _ = take32(&mut o)?; // fs
    let _ = take32(&mut o)?; // gs
    debug_assert_eq!(o, G_PACKET_HEX_CHARS);
    let _ = sregs; // explicit: unused. Keeps the signature symmetric for future slices.
    Ok(())
}

/// Compute a single chunk of the qXfer:features:read:target.xml stream.
///
/// Returns the reply payload starting with 'm' (more) or 'l' (last).
pub fn target_xml_chunk(offset: usize, length: usize) -> String {
    let blob = TARGET_XML.as_bytes();
    if offset >= blob.len() {
        // Transfer complete — payload is exactly the byte 'l'.
        return "l".to_string();
    }
    let end = (offset + length).min(blob.len());
    let slice = &blob[offset..end];
    let prefix = if end >= blob.len() { 'l' } else { 'm' };
    let mut s = String::with_capacity(1 + slice.len());
    s.push(prefix);
    s.push_str(std::str::from_utf8(slice).expect("target.xml is valid UTF-8"));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zero_regs() -> kvm_regs { unsafe { std::mem::zeroed() } }
    fn zero_sregs() -> kvm_sregs { unsafe { std::mem::zeroed() } }

    #[test]
    fn hex_le_u64_reference() {
        assert_eq!(hex_le_u64(0xDEADBEEFCAFEBABE), "bebafecaefbeadde");
    }

    #[test]
    fn hex_le_u32_reference() {
        assert_eq!(hex_le_u32(0xDEADBEEF), "efbeadde");
    }

    #[test]
    fn g_packet_size_is_164_bytes() {
        let s = encode_regs(&zero_regs(), &zero_sregs());
        assert_eq!(s.len(), 328);
        assert_eq!(s.len(), G_PACKET_HEX_CHARS);
    }

    #[test]
    fn g_then_G_round_trip() {
        let mut r = zero_regs();
        r.rax = 0xDEADBEEFCAFEBABE;
        r.rip = 0x800000;
        r.rflags = 0x202;
        let mut s = zero_sregs();
        s.cs.selector = 0x38;
        s.ss.selector = 0x30;

        let hex = encode_regs(&r, &s);
        let mut r2 = zero_regs();
        let mut s2 = zero_sregs();
        decode_regs(&hex, &mut r2, &mut s2).unwrap();
        assert_eq!(r2.rax, r.rax);
        assert_eq!(r2.rip, r.rip);
        assert_eq!(r2.rflags & 0xFFFF_FFFF, r.rflags & 0xFFFF_FFFF);
        // s2.cs.selector is NOT updated — selectors are no-op'd on G.
        assert_eq!(s2.cs.selector, 0);
    }

    #[test]
    fn encode_pins_each_register_to_its_offset() {
        // Distinct sentinel per register so a wrong offset/size is visible.
        let mut r = zero_regs();
        r.rax = 0x0000000000000001;
        r.rbx = 0x0000000000000002;
        r.rcx = 0x0000000000000003;
        r.rdx = 0x0000000000000004;
        r.rsi = 0x0000000000000005;
        r.rdi = 0x0000000000000006;
        r.rbp = 0x0000000000000007;
        r.rsp = 0x0000000000000008;
        r.r8  = 0x0000000000000009;
        r.r9  = 0x000000000000000a;
        r.r10 = 0x000000000000000b;
        r.r11 = 0x000000000000000c;
        r.r12 = 0x000000000000000d;
        r.r13 = 0x000000000000000e;
        r.r14 = 0x000000000000000f;
        r.r15 = 0x0000000000000010;
        r.rip = 0x0000000000000011;
        r.rflags = 0x00000202;
        let mut s = zero_sregs();
        s.cs.selector = 0x0038;
        s.ss.selector = 0x0030;
        s.ds.selector = 0x0028;
        s.es.selector = 0x0020;
        s.fs.selector = 0x0018;
        s.gs.selector = 0x0010;

        let hex = encode_regs(&r, &s);
        // Each u64 GPR occupies 16 hex chars, little-endian.
        // rax=1 → "0100000000000000" at chars [0..16].
        assert_eq!(&hex[0..16],   "0100000000000000", "rax @0");
        assert_eq!(&hex[16..32],  "0200000000000000", "rbx @8");
        assert_eq!(&hex[32..48],  "0300000000000000", "rcx @16");
        assert_eq!(&hex[48..64],  "0400000000000000", "rdx @24");
        assert_eq!(&hex[64..80],  "0500000000000000", "rsi @32");
        assert_eq!(&hex[80..96],  "0600000000000000", "rdi @40");
        assert_eq!(&hex[96..112], "0700000000000000", "rbp @48");
        assert_eq!(&hex[112..128],"0800000000000000", "rsp @56");
        assert_eq!(&hex[128..144],"0900000000000000", "r8 @64");
        assert_eq!(&hex[144..160],"0a00000000000000", "r9 @72");
        assert_eq!(&hex[160..176],"0b00000000000000", "r10 @80");
        assert_eq!(&hex[176..192],"0c00000000000000", "r11 @88");
        assert_eq!(&hex[192..208],"0d00000000000000", "r12 @96");
        assert_eq!(&hex[208..224],"0e00000000000000", "r13 @104");
        assert_eq!(&hex[224..240],"0f00000000000000", "r14 @112");
        assert_eq!(&hex[240..256],"1000000000000000", "r15 @120");
        assert_eq!(&hex[256..272],"1100000000000000", "rip @128");
        // eflags u32 → 8 hex chars LE. 0x202 → "02020000".
        assert_eq!(&hex[272..280],"02020000", "eflags @136");
        // segment selectors u32 LE, zero-extended.
        assert_eq!(&hex[280..288],"38000000", "cs @140");
        assert_eq!(&hex[288..296],"30000000", "ss @144");
        assert_eq!(&hex[296..304],"28000000", "ds @148");
        assert_eq!(&hex[304..312],"20000000", "es @152");
        assert_eq!(&hex[312..320],"18000000", "fs @156");
        assert_eq!(&hex[320..328],"10000000", "gs @160");
    }

    #[test]
    fn g_decode_rejects_wrong_length() {
        let mut r = zero_regs();
        let mut s = zero_sregs();
        assert!(decode_regs("abcd", &mut r, &mut s).is_err());
    }

    #[test]
    fn target_xml_chunk_offsets() {
        let blob = TARGET_XML.as_bytes();
        let len = blob.len();

        // offset 0, length larger than blob → returns 'l' + full xml.
        let chunk = target_xml_chunk(0, len + 100);
        assert!(chunk.starts_with('l'));
        assert_eq!(&chunk[1..], TARGET_XML);

        // offset 0, length smaller than blob → 'm' + partial.
        let chunk = target_xml_chunk(0, 100);
        assert!(chunk.starts_with('m'));
        assert_eq!(chunk.len(), 101);

        // offset = blob.len() → exactly "l".
        let chunk = target_xml_chunk(len, 100);
        assert_eq!(chunk, "l");

        // offset > blob.len() → exactly "l".
        let chunk = target_xml_chunk(len + 1, 100);
        assert_eq!(chunk, "l");
    }

    #[test]
    fn target_xml_has_architecture_and_24_regs() {
        assert!(TARGET_XML.contains("<architecture>i386:x86-64</architecture>"));
        assert_eq!(TARGET_XML.matches("<reg ").count(), 24);
    }

    #[test]
    fn target_xml_bitsize_sum_equals_g_packet_bytes() {
        // Every <reg bitsize="N"> contributes N/8 bytes to the g packet.
        let mut total_bits: usize = 0;
        for line in TARGET_XML.lines() {
            if let Some(idx) = line.find("bitsize=\"") {
                let rest = &line[idx + 9..];
                let end = rest.find('"').unwrap();
                total_bits += rest[..end].parse::<usize>().unwrap();
            }
        }
        assert_eq!(total_bits / 8, G_PACKET_BYTES);
    }
}
