//! Create the vCPU in 16-bit real mode and run it until HLT, dispatching exits.

use anyhow::{bail, Context, Result};
use kvm_ioctls::{VcpuExit, VcpuFd, VmFd};
use std::io::Write;

use crate::serial::{Serial, COM1_PORT};
use crate::stats::Stats;
use crate::vm::GUEST_LOAD_ADDR;

/// Create vCPU 0 and set initial 16-bit real-mode register state.
pub fn create_vcpu(vm_fd: &VmFd) -> Result<VcpuFd> {
    let vcpu = vm_fd.create_vcpu(0).context("KVM_CREATE_VCPU failed")?;

    // Real mode with flat segments based at 0 so linear addr == offset.
    // (CR0.PE is 0 at reset, which get_sregs reflects — we leave it real mode.)
    let mut sregs = vcpu.get_sregs().context("KVM_GET_SREGS failed")?;
    for seg in [&mut sregs.cs, &mut sregs.ds, &mut sregs.es, &mut sregs.ss] {
        seg.base = 0;
        seg.selector = 0;
    }
    vcpu.set_sregs(&sregs).context("KVM_SET_SREGS failed")?;

    let mut regs = vcpu.get_regs().context("KVM_GET_REGS failed")?;
    regs.rip = GUEST_LOAD_ADDR; // start at the loaded blob
    regs.rflags = 0x2; // bit 1 is reserved-and-must-be-1; DF=0 so lodsb counts up
    // The blob uses no stack (no push/call), so RSP is left at its reset value.
    vcpu.set_regs(&regs).context("KVM_SET_REGS failed")?;

    Ok(vcpu)
}

/// Run the vCPU until HLT, forwarding serial output and counting exits.
pub fn run<W: Write>(vcpu: &mut VcpuFd, serial: &mut Serial<W>, stats: &mut Stats) -> Result<()> {
    loop {
        match vcpu.run().context("KVM_RUN failed")? {
            VcpuExit::IoOut(port, data) => {
                stats.record_io();
                if port == COM1_PORT {
                    serial.write_bytes(data);
                }
            }
            VcpuExit::IoIn(_, _) => {
                stats.record_io();
            }
            VcpuExit::MmioRead(_, _) | VcpuExit::MmioWrite(_, _) => {
                stats.record_mmio();
            }
            VcpuExit::Hlt => {
                stats.record_hlt();
                break;
            }
            other => bail!("unexpected VM exit: {other:?}"),
        }
    }
    Ok(())
}
