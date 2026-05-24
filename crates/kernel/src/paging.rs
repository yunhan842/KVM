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
const HUGE: u64 = 1 << 7;
const PAGE_2MIB: u64 = 0x20_0000;

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
