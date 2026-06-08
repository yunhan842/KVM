//! Create the vCPU in 16-bit real mode and run it until HLT, dispatching exits.

use anyhow::{bail, Context, Result};
use kvm_bindings::{kvm_debug_exit_arch, KVM_MAX_CPUID_ENTRIES};
use kvm_ioctls::{Kvm, VcpuExit, VcpuFd, VmFd};
use std::io::Write;

use crate::serial::{Uart, COM1_BASE, COM1_LAST};
use crate::stats::Stats;
use crate::vm::GUEST_LOAD_ADDR;

/// Create vCPU 0 and set initial 16-bit real-mode register state.
pub fn create_vcpu(kvm: &Kvm, vm_fd: &VmFd) -> Result<VcpuFd> {
    let vcpu = vm_fd.create_vcpu(0).context("KVM_CREATE_VCPU failed")?;

    // Long mode requires the guest CPUID to advertise long-mode support (the LM bit,
    // CPUID.80000001h:EDX[29]). KVM sets no CPUID by default, so copy the host's
    // KVM-supported CPUID onto the vCPU. Without this, the guest's `wrmsr EFER.LME`
    // raises #GP and (with no IDT) triple-faults.
    let cpuid = kvm
        .get_supported_cpuid(KVM_MAX_CPUID_ENTRIES)
        .context("KVM_GET_SUPPORTED_CPUID failed")?;
    vcpu.set_cpuid2(&cpuid).context("KVM_SET_CPUID2 failed")?;

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
pub fn run<W: Write>(vcpu: &mut VcpuFd, uart: &mut Uart<W>, stats: &mut Stats) -> Result<()> {
    loop {
        match vcpu.run().context("KVM_RUN failed")? {
            VcpuExit::IoOut(port, data) => {
                stats.record_io();
                if (COM1_BASE..=COM1_LAST).contains(&port) {
                    for &b in data.iter() {
                        uart.write_reg(port, b, stats);
                    }
                }
            }
            VcpuExit::IoIn(port, data) => {
                stats.record_io();
                if (COM1_BASE..=COM1_LAST).contains(&port) {
                    let v = uart.read_reg(port);
                    for b in data.iter_mut() {
                        *b = v;
                    }
                }
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

/// Reason `run_until_event` returned control to the caller.
pub enum StopReason {
    /// SW BP (#BP, exception 3) or single-step (#DB, exception 1) trap.
    Debug(kvm_debug_exit_arch),
    /// Signal interruption (slice-8 path; unreachable in slice 7 but wired).
    Intr,
    /// Guest executed `hlt`.
    Hlt,
}

/// Run the vCPU until the next Debug exit, Hlt, or signal-interrupt.
/// Forwards IoIn/IoOut/Mmio internally exactly like `run()`. Unlike `run()`,
/// it does NOT `?`-bail on EINTR — it returns `StopReason::Intr` (so the gdb
/// stub can treat a Ctrl-C interrupt as a stop, in slice 8).
pub fn run_until_event<W: Write>(
    vcpu: &mut VcpuFd,
    uart: &mut Uart<W>,
    stats: &mut Stats,
) -> Result<StopReason> {
    loop {
        match vcpu.run() {
            Ok(VcpuExit::IoOut(port, data)) => {
                stats.record_io();
                if (COM1_BASE..=COM1_LAST).contains(&port) {
                    for &b in data.iter() {
                        uart.write_reg(port, b, stats);
                    }
                }
            }
            Ok(VcpuExit::IoIn(port, data)) => {
                stats.record_io();
                if (COM1_BASE..=COM1_LAST).contains(&port) {
                    let v = uart.read_reg(port);
                    for b in data.iter_mut() {
                        *b = v;
                    }
                }
            }
            Ok(VcpuExit::MmioRead(_, _)) | Ok(VcpuExit::MmioWrite(_, _)) => {
                stats.record_mmio();
            }
            Ok(VcpuExit::Hlt) => {
                stats.record_hlt();
                return Ok(StopReason::Hlt);
            }
            Ok(VcpuExit::Debug(arch)) => {
                return Ok(StopReason::Debug(arch));
            }
            Ok(VcpuExit::Intr) => {
                return Ok(StopReason::Intr);
            }
            Err(e) if e.errno() == libc::EINTR => {
                return Ok(StopReason::Intr);
            }
            Err(e) => return Err(e).context("KVM_RUN failed"),
            Ok(other) => bail!("unexpected VM exit: {other:?}"),
        }
    }
}
