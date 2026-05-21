// TEMPORARY (remove in Task 6): while modules are built incrementally, some pub items
// (e.g. Stats::record_mmio) are unused until the run loop wires them up. The resulting
// dead_code *warning* triggers a rustc 1.95.0 renderer ICE in this toolchain, so we
// silence the lint until everything is wired, then delete this attribute and confirm a
// warning-clean build.
#![allow(dead_code)]

mod config;
mod serial;
mod stats;

fn main() {
    println!("minikvm vmm");
}
