mod config;
mod serial;
mod stats;
mod vcpu;
mod vm;

use anyhow::{anyhow, Result};
use std::io;

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

    let mut vcpu = vcpu::create_vcpu(&vm_fd)?;
    log("[host] created vCPU 0");

    let stdout = io::stdout();
    let mut serial = serial::Serial::new(stdout.lock());
    let mut stats = stats::Stats::default();

    let start = std::time::Instant::now();
    vcpu::run(&mut vcpu, &mut serial, &mut stats)?;
    let elapsed = start.elapsed();

    if cfg.trace {
        println!("{}", stats.summary());
        println!("[host] runtime: {elapsed:?}");
    }
    Ok(())
}
