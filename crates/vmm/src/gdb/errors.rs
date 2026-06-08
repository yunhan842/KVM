//! Error type for the gdb stub.
//!
//! Intended contract (reached at Task 7b): `ProtocolViolation` and
//! `SocketDied` are handled INSIDE `run_session` and never bubble out of
//! `gdb::serve`; only `Fatal` escalates to anyhow.
//!
//! Transitional note (Tasks 4–7a): the `Resume` arm of `run_session` is a
//! placeholder that returns `ProtocolViolation`, so a `c`/`s` packet sent to
//! an intermediate build leaks it out of `serve` as an anyhow error. This is
//! harmless because no gdb session is run against intermediate commits; Task 7b
//! replaces the placeholder with the real Running state and closes the gap.

#[derive(Debug)]
pub enum StubError {
    /// A malformed packet we choose to drop without killing the VMM
    /// (unknown packets are NOT this — they fall through to an empty reply).
    ProtocolViolation,
    /// Socket EOF / EPIPE / ECONNRESET. Triggers the §11.3 cleanup-and-exit path.
    SocketDied,
    /// Genuinely unrecoverable. Bubbles to `serve`'s `anyhow::Error`.
    Fatal(anyhow::Error),
}

impl From<std::io::Error> for StubError {
    fn from(e: std::io::Error) -> Self {
        // Most std::io errors on the gdb socket are "gdb went away."
        // The caller decides whether to escalate; default to SocketDied
        // and let the dispatch loop handle EOF cleanup.
        use std::io::ErrorKind::*;
        match e.kind() {
            BrokenPipe | ConnectionReset | ConnectionAborted | UnexpectedEof => {
                StubError::SocketDied
            }
            _ => StubError::Fatal(anyhow::Error::from(e)),
        }
    }
}

impl From<vm_memory::GuestMemoryError> for StubError {
    fn from(e: vm_memory::GuestMemoryError) -> Self {
        // m/M bounds-check fires BEFORE the vm_memory call, so any error
        // here is genuinely unexpected (corrupted GuestMemoryMmap, etc.).
        StubError::Fatal(anyhow::Error::from(e))
    }
}

impl From<kvm_ioctls::Error> for StubError {
    fn from(e: kvm_ioctls::Error) -> Self {
        StubError::Fatal(anyhow::Error::msg(format!("kvm ioctl: {e}")))
    }
}
