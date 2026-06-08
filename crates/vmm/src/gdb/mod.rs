//! GDB remote-serial-protocol stub for the MiniKVM VMM.
//!
//! Slice 7 — see docs/superpowers/specs/2026-06-07-minikvm-month3plus-gdb-stub-design.md.

pub mod errors;
mod breakpoints;
mod packets;
mod regs;
pub(crate) mod rsp;

pub use errors::StubError;

// Stub struct + serve entry + run_session are added in Task 4.
