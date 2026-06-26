//! Open /dev/kvm, create the VM, map guest memory, and load the guest blob.

use anyhow::{Context, Result};
use kvm_bindings::{kvm_pit_config, kvm_userspace_memory_region};
use kvm_ioctls::{Kvm, VmFd};
// `get_host_address` lives on the `GuestMemoryBackend` trait as of vm-memory 0.18
// (it used to be on `GuestMemory`); `Bytes` provides `write_slice`.
use vm_memory::{Bytes, GuestAddress, GuestMemoryBackend, GuestMemoryMmap};

/// Size of the single guest memory region, mapped at guest-physical 0x0.
pub const MEM_SIZE: usize = 64 * 1024 * 1024;
/// Where the guest blob is copied and where execution starts.
pub const GUEST_LOAD_ADDR: u64 = 0x1000;

/// Open /dev/kvm with an actionable error if it isn't usable.
pub fn open_kvm() -> Result<Kvm> {
    Kvm::new().context(
        "failed to open /dev/kvm — is it present (`ls -l /dev/kvm`) and are you in the 'kvm' group? \
         (sudo usermod -aG kvm $USER, then `wsl --shutdown` and reopen)",
    )
}

/// Create the in-kernel interrupt chip (PIC + IOAPIC + LAPIC) and the in-kernel
/// 8254 PIT. MUST be called after `create_vm` and BEFORE `create_vcpu` — KVM
/// creates the per-vCPU LAPIC at vCPU-creation time and requires the irqchip to
/// exist first. With both in-kernel, the PIT's IRQ0 is delivered to the guest
/// automatically on each KVM_RUN; the VMM never injects an interrupt itself.
pub fn setup_irqchip(vm_fd: &VmFd) -> Result<()> {
    vm_fd
        .create_irq_chip()
        .context("KVM_CREATE_IRQCHIP failed")?;
    vm_fd
        .create_pit2(kvm_pit_config::default())
        .context("KVM_CREATE_PIT2 failed")?;
    Ok(())
}

/// Allocate a 64 MiB host-backed region and register it with the VM at GPA 0.
pub fn setup_memory(vm_fd: &VmFd) -> Result<GuestMemoryMmap> {
    let mem = GuestMemoryMmap::<()>::from_ranges(&[(GuestAddress(0), MEM_SIZE)])
        .context("failed to mmap guest memory")?;

    let host_addr = mem
        .get_host_address(GuestAddress(0))
        .context("failed to resolve host address of guest memory")? as u64;

    let region = kvm_userspace_memory_region {
        slot: 0,
        guest_phys_addr: 0,
        memory_size: MEM_SIZE as u64,
        userspace_addr: host_addr,
        flags: 0,
    };
    // Safety: `host_addr` points to a live mmap of `MEM_SIZE` bytes owned by `mem`,
    // which the caller keeps alive for the lifetime of the VM.
    unsafe { vm_fd.set_user_memory_region(region) }
        .context("KVM_SET_USER_MEMORY_REGION failed")?;

    Ok(mem)
}

/// Copy the guest blob into guest memory at GUEST_LOAD_ADDR.
pub fn load_guest(mem: &GuestMemoryMmap, blob: &[u8]) -> Result<()> {
    mem.write_slice(blob, GuestAddress(GUEST_LOAD_ADDR))
        .context("failed to copy guest blob into guest memory")?;
    Ok(())
}
