//! Stage-2 paging: replace the stub's 2 MiB bootstrap map with a full 64 MiB identity map
//! (32 x 2 MiB huge pages). Tables live in BSS (zeroed by KVM); identity mapping means a
//! table's virtual address equals its physical address, which is what CR3 wants.

use core::ptr::addr_of_mut;

#[repr(C, align(4096))]
struct PageTable([u64; 512]);

static mut PML4: PageTable = PageTable([0; 512]);
static mut PDPT: PageTable = PageTable([0; 512]);
static mut PD: PageTable = PageTable([0; 512]);

const PRESENT: u64 = 1 << 0;
const WRITABLE: u64 = 1 << 1;
const USER: u64 = 1 << 2;
const PCD: u64 = 1 << 4; // page cache-disable (uncacheable) — required for MMIO
const HUGE: u64 = 1 << 7;
const PAGE_2MIB: u64 = 0x20_0000;
const MMIO_PD_INDEX: usize = 32; // PD[32] covers 0x0400_0000..0x0420_0000

/// Build a 64 MiB identity map and load it into CR3.
///
/// Safety: must be called once, with interrupts disabled, while the running code and
/// stack are inside the region this map covers (the boot stub identity-mapped the first
/// 2 MiB, which contains both — and the new map identity-maps that range too, so RIP/RSP
/// stay valid across the CR3 write).
pub unsafe fn init_identity_map() {
    let pml4 = addr_of_mut!(PML4);
    let pdpt = addr_of_mut!(PDPT);
    let pd = addr_of_mut!(PD);

    (*pml4).0[0] = (pdpt as u64) | PRESENT | WRITABLE;
    (*pdpt).0[0] = (pd as u64) | PRESENT | WRITABLE;

    let mut i = 0usize;
    while i < 32 {
        (*pd).0[i] = (i as u64 * PAGE_2MIB) | PRESENT | WRITABLE | HUGE;
        i += 1;
    }

    // Write CR3. Writing CR3 implicitly flushes non-global TLB entries.
    core::arch::asm!(
        "mov cr3, {}",
        in(reg) pml4 as u64,
        options(nostack, preserves_flags),
    );
}

/// Mark the 2 MiB region covered by `PD[index]` user-accessible.
///
/// Page permission is the AND across the walk, so the USER bit must be set on
/// PML4[0], PDPT[0], AND the chosen PD entry. After this returns, exactly that
/// 2 MiB region is reachable from ring 3; other PD entries lack USER and stay
/// supervisor-only — kernel/user memory separation is real, not just CPL-based.
///
/// # Safety
/// Call once for each user region, with interrupts off, after `init_identity_map`.
/// Must NOT be called for an index whose region contains kernel data, or ring 3
/// would gain access to it.
pub unsafe fn map_user_pd_entry(index: usize) {
    let pml4 = addr_of_mut!(PML4);
    let pdpt = addr_of_mut!(PDPT);
    let pd   = addr_of_mut!(PD);

    (*pml4).0[0]      |= USER;
    (*pdpt).0[0]      |= USER;
    (*pd).0[index]    |= USER;

    // CR3 reload: flush stale (supervisor-only) TLB entries for the touched walk
    // levels. The user-region leaf had no prior TLB entry, but PML4[0]/PDPT[0]
    // were cached as supervisor when kernel pages were accessed.
    core::arch::asm!(
        "mov rax, cr3",
        "mov cr3, rax",
        out("rax") _,
        options(nostack, preserves_flags),
    );
}

/// Map the 2 MiB MMIO window (containing the emulated host device) as an
/// uncacheable identity page at PD[32] (GPA 0x0400_0000). Call once after
/// `init_identity_map`, before any device access.
///
/// Uncacheable (PCD) is mandatory: a device register's value changes outside
/// the CPU's knowledge (the host uptime counter ticks every ns), so a cached
/// read would return stale data, and writes could be coalesced/reordered.
///
/// # Safety
/// Must be called once, interrupts off, after `init_identity_map`.
pub unsafe fn map_mmio_page() {
    let pd = addr_of_mut!(PD);
    (*pd).0[MMIO_PD_INDEX] =
        (MMIO_PD_INDEX as u64 * PAGE_2MIB) | PRESENT | WRITABLE | HUGE | PCD;

    // CR3 reload to flush any stale TLB entry for this walk.
    core::arch::asm!(
        "mov rax, cr3",
        "mov cr3, rax",
        out("rax") _,
        options(nostack, preserves_flags),
    );
}
