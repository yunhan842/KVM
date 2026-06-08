//! Packet inventory and dispatch. Reply column is payload-only — framing
//! happens in rsp::write_packet.

use std::borrow::Cow;

use crate::gdb::errors::StubError;

/// What dispatch returns to `run_session` for one inbound packet.
pub enum DispatchAction {
    /// Stay in Stopped. Write `Reply(...)` as the next outbound packet.
    /// `Reply(Cow::Borrowed(""))` is the standard empty packet (`$#00`).
    Reply(Cow<'static, str>),
    /// Transition Stopped → Running. `singlestep=true` for `s`, false for `c`.
    Resume { singlestep: bool },
    /// Receive of `D` (clean detach). Run §11.2 cleanup + return Ok.
    Detach,
    /// Receive of `k` (hard kill). process::exit(0).
    Kill,
    /// Indicates the handler already wrote its reply (used for the
    /// QStartNoAckMode special case, where the cutover spans dispatch+write).
    AlreadyReplied,
}

/// Hardcoded qSupported reply, per §6.5.
pub const QSUPPORTED_REPLY: &str =
    "PacketSize=4000;qXfer:features:read+;swbreak+;QStartNoAckMode+";

/// Pure helper used by `Stub::dispatch` for packets that don't need
/// access to vcpu / mem / breakpoints / stream state.
///
/// Returns Some(DispatchAction) if this packet is fully handled here,
/// None if the caller must run a stateful handler (g/G/m/M/Z0/z0/c/s/D/k/QStartNoAckMode).
pub fn dispatch_stateless(pkt: &[u8]) -> Option<DispatchAction> {
    let s = std::str::from_utf8(pkt).ok()?;

    // Exact-match queries.
    let reply: Option<&'static str> = match s {
        "?" => Some("T05thread:1;"),
        "qAttached" => Some("1"),
        "qC" => Some("QC1"),
        "qfThreadInfo" => Some("m1"),
        "qsThreadInfo" => Some("l"),
        "qSymbol::" => Some("OK"),
        "qOffsets" => Some(""),
        "vCont?" => Some(""),
        // All Hg/Hc tids: only literal 0, 1, -1 OK; everything else E22.
        // Hg with -1 is meaningless on a single-thread target → E22.
        "Hg0" | "Hg1" | "Hc0" | "Hc1" | "Hc-1" => Some("OK"),
        _ => None,
    };
    if let Some(r) = reply {
        return Some(DispatchAction::Reply(Cow::Borrowed(r)));
    }

    // Prefix-match queries.
    if s.starts_with("qSupported") {
        return Some(DispatchAction::Reply(Cow::Borrowed(QSUPPORTED_REPLY)));
    }
    if s.starts_with("Hg") || s.starts_with("Hc") {
        // Anything not exact-matched above → E22.
        return Some(DispatchAction::Reply(Cow::Borrowed("E22")));
    }
    if s.starts_with('v') {
        // ALL unknown v-prefixed packets reply empty: vCont*, vMustReplyEmpty,
        // vCtrlC, vKill, vRun, vAttach, vFile, ...
        return Some(DispatchAction::Reply(Cow::Borrowed("")));
    }
    if s.starts_with('k') {
        return Some(DispatchAction::Kill);
    }
    if s.starts_with('D') {
        return Some(DispatchAction::Detach);
    }
    if s.starts_with('p') || s.starts_with('P') {
        // Empty fallback → gdb uses g/G.
        return Some(DispatchAction::Reply(Cow::Borrowed("")));
    }
    if s.starts_with("Z1") || s.starts_with("z1")
        || s.starts_with("Z2") || s.starts_with("z2")
        || s.starts_with("Z3") || s.starts_with("z3")
        || s.starts_with("Z4") || s.starts_with("z4")
    {
        return Some(DispatchAction::Reply(Cow::Borrowed("")));
    }

    None
}

/// Parse an optional hex u64 suffix from a `c`/`s` packet (after the opcode byte).
/// Returns `Ok(None)` if no suffix, `Ok(Some(addr))` if valid hex, `Err(StubError::ProtocolViolation)` otherwise.
pub fn parse_optional_resume_addr(pkt: &[u8]) -> Result<Option<u64>, StubError> {
    if pkt.len() <= 1 {
        return Ok(None);
    }
    let tail = std::str::from_utf8(&pkt[1..]).map_err(|_| StubError::ProtocolViolation)?;
    if tail.is_empty() {
        return Ok(None);
    }
    let addr = u64::from_str_radix(tail, 16).map_err(|_| StubError::ProtocolViolation)?;
    Ok(Some(addr))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply_str(action: DispatchAction) -> Option<String> {
        match action {
            DispatchAction::Reply(s) => Some(s.into_owned()),
            _ => None,
        }
    }

    #[test]
    fn qsupported_always_returns_hardcoded() {
        let r = dispatch_stateless(b"qSupported:multiprocess+;swbreak+;hwbreak+").unwrap();
        assert_eq!(reply_str(r).unwrap(), QSUPPORTED_REPLY);
    }

    #[test]
    fn question_returns_initial_stop_reason() {
        let r = dispatch_stateless(b"?").unwrap();
        assert_eq!(reply_str(r).unwrap(), "T05thread:1;");
    }

    #[test]
    fn qattached_returns_one() {
        let r = dispatch_stateless(b"qAttached").unwrap();
        assert_eq!(reply_str(r).unwrap(), "1");
    }

    #[test]
    fn thread_queries() {
        assert_eq!(reply_str(dispatch_stateless(b"qC").unwrap()).unwrap(), "QC1");
        assert_eq!(reply_str(dispatch_stateless(b"qfThreadInfo").unwrap()).unwrap(), "m1");
        assert_eq!(reply_str(dispatch_stateless(b"qsThreadInfo").unwrap()).unwrap(), "l");
        assert_eq!(reply_str(dispatch_stateless(b"qSymbol::").unwrap()).unwrap(), "OK");
        assert_eq!(reply_str(dispatch_stateless(b"qOffsets").unwrap()).unwrap(), "");
    }

    #[test]
    fn unknown_v_packets_reply_empty() {
        for pkt in [b"vCont".as_slice(), b"vCont?", b"vMustReplyEmpty", b"vCtrlC", b"vKill", b"vRun;", b"vAttach;1"] {
            let r = dispatch_stateless(pkt).unwrap();
            assert_eq!(reply_str(r).unwrap(), "", "pkt = {:?}", std::str::from_utf8(pkt).unwrap());
        }
    }

    #[test]
    fn h_thread_select() {
        for pkt in [b"Hg0".as_slice(), b"Hg1", b"Hc0", b"Hc1", b"Hc-1"] {
            assert_eq!(reply_str(dispatch_stateless(pkt).unwrap()).unwrap(), "OK", "pkt = {:?}", pkt);
        }
        for pkt in [b"Hg-1".as_slice(), b"Hc2", b"Hg7"] {
            assert_eq!(reply_str(dispatch_stateless(pkt).unwrap()).unwrap(), "E22", "pkt = {:?}", pkt);
        }
    }

    #[test]
    fn single_register_packets_empty() {
        assert_eq!(reply_str(dispatch_stateless(b"p0").unwrap()).unwrap(), "");
        assert_eq!(reply_str(dispatch_stateless(b"P0=00").unwrap()).unwrap(), "");
    }

    #[test]
    fn unknown_packet_falls_through_to_none() {
        // Caller will treat None as "stateful handler needed or unknown".
        assert!(dispatch_stateless(b"Zxxxxxx").is_none());
    }

    #[test]
    fn parse_optional_addr_none() {
        assert_eq!(parse_optional_resume_addr(b"c").unwrap(), None);
        assert_eq!(parse_optional_resume_addr(b"s").unwrap(), None);
    }

    #[test]
    fn parse_optional_addr_hex() {
        assert_eq!(parse_optional_resume_addr(b"c1000").unwrap(), Some(0x1000));
        assert_eq!(parse_optional_resume_addr(b"s2000").unwrap(), Some(0x2000));
    }

    #[test]
    fn parse_optional_addr_garbage_rejected() {
        assert!(parse_optional_resume_addr(b"czz").is_err());
    }
}
