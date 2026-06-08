mod config;
mod gdb;
mod serial;
mod stats;
mod vcpu;
mod vm;

use anyhow::{anyhow, Result};
use std::io;

use kvm_ioctls::VcpuFd;
use vm_memory::GuestMemoryMmap;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cfg = config::parse_args(&args).map_err(|e| anyhow!(e))?;

    let log = |msg: &str| {
        if cfg.trace {
            println!("{msg}");
        }
    };

    let blob = std::fs::read(&cfg.guest_path)
        .map_err(|e| anyhow!("failed to read guest image '{}': {e}", cfg.guest_path))?;

    let kvm = vm::open_kvm()?;
    let vm_fd = kvm.create_vm()?;
    // Needed for real-mode guests on Intel CPUs without "unrestricted guest";
    // harmless otherwise. (Dev-log gotcha.)
    vm_fd
        .set_tss_address(0xfffb_d000)
        .map_err(|e| anyhow!("KVM_SET_TSS_ADDR failed: {e}"))?;
    log("[host] created VM");

    let mem = vm::setup_memory(&vm_fd)?;
    log(&format!(
        "[host] mapped {} MiB guest memory",
        vm::MEM_SIZE / (1024 * 1024)
    ));
    vm::load_guest(&mem, &blob)?;

    let mut vcpu = vcpu::create_vcpu(&kvm, &vm_fd)?;
    log("[host] created vCPU 0");

    run_non_gdb_path(&mut vcpu, &mem, &cfg)
}

/// The slice-1–6 main body. Owns the vCPU loop + stats summary lines.
fn run_non_gdb_path(vcpu: &mut VcpuFd, _mem: &GuestMemoryMmap, cfg: &config::Config) -> Result<()> {
    let stdout = io::stdout();
    let mut uart = serial::Uart::new(stdout.lock());
    let mut stats = stats::Stats::new();

    let start = std::time::Instant::now();
    vcpu::run(vcpu, &mut uart, &mut stats)?;
    let elapsed = start.elapsed();

    if cfg.trace {
        println!("{}", stats.summary());
        // Slice-6 benchmark line. eprintln on the failure path is intentional —
        // the absence of the prefix on stdout is what causes the integration
        // test to fail loudly when the benchmark didn't complete.
        if let Some(ns) = stats.avg_syscall_ns() {
            println!("[host] avg syscall latency: {ns} ns");
        } else {
            eprintln!("[host] benchmark incomplete (markers missing)");
        }
        println!("[host] runtime: {elapsed:?}");
    }
    Ok(())
}
