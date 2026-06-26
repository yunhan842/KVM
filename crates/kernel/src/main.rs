#![no_std]
#![no_main]

extern crate alloc;

mod elf;
mod gdt;
mod heap;
mod idt;
mod interrupts;
mod io;
mod mmio;
mod paging;
mod serial;
mod syscall;
mod user;

use core::fmt::Write;
use core::panic::PanicInfo;

use alloc::{boxed::Box, vec::Vec};

use serial::Serial;

#[no_mangle]
#[link_section = ".text.boot"]
pub extern "C" fn _start() -> ! {
    Serial::init();
    let mut com = Serial;
    let _ = writeln!(com, "[guest] kernel entered");
    unsafe { paging::init_identity_map(); }
    unsafe { paging::map_mmio_page(); }
    let _ = writeln!(com, "[guest] paging enabled");
    unsafe { idt::init(); }
    let _ = writeln!(com, "[guest] idt loaded");
    unsafe { heap::init(); }
    let _ = writeln!(com, "[guest] heap initialized ({} bytes)", heap::HEAP_SIZE);

    let v: Vec<u32> = (0..5).collect();
    let b: Box<u64> = Box::new(0xDEAD_BEEFu64);
    let _ = writeln!(
        com,
        "[guest] heap demo: Vec<u32>={{{},{},{},{},{}}} Box<u64>={:#X}",
        v[0], v[1], v[2], v[3], v[4], *b
    );

    unsafe { gdt::init(); }
    let _ = writeln!(com, "[guest] gdt+tss installed");

    let _ = writeln!(com, "hello from the kernel (long mode)");

    unsafe { syscall::init(); }
    let _ = writeln!(com, "[guest] syscall enabled");

    let uptime = mmio::read_host_uptime_ns();
    let _ = writeln!(com, "[guest] host uptime via MMIO: {} ns", uptime);

    unsafe { interrupts::init(); }
    let _ = writeln!(com, "[guest] interrupts: PIC remapped, PIT at 1000 Hz");

    let entry = unsafe { user::setup() };
    let _ = writeln!(com, "[guest] loaded /bin/hello");
    let _ = writeln!(com, "[guest] entering ring 3");

    // Enable IF right before ring 3 so the first timer tick preempts ring-3
    // user code (clean boot output; interrupted RIPs are user addresses).
    unsafe { interrupts::enable(); }

    // Hand control to ring 3 at the ELF entry point. The C program does
    // write(1, "[user] hello from C userspace\n", 30) and exit(0); sys_exit
    // halts the kernel, so this function never returns.
    unsafe { user::enter_ring3(entry); }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    let mut com = Serial;
    let _ = writeln!(com, "[guest] PANIC");
    loop {
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack));
        }
    }
}

// --- mem intrinsics the compiler may emit calls to (no libc in no_std) ---

#[no_mangle]
pub unsafe extern "C" fn memcpy(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    let mut i = 0;
    while i < n {
        *dest.add(i) = *src.add(i);
        i += 1;
    }
    dest
}

#[no_mangle]
pub unsafe extern "C" fn memset(dest: *mut u8, c: i32, n: usize) -> *mut u8 {
    let mut i = 0;
    while i < n {
        *dest.add(i) = c as u8;
        i += 1;
    }
    dest
}

#[no_mangle]
pub unsafe extern "C" fn memmove(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    if (dest as usize) < (src as usize) {
        let mut i = 0;
        while i < n {
            *dest.add(i) = *src.add(i);
            i += 1;
        }
    } else {
        let mut i = n;
        while i > 0 {
            i -= 1;
            *dest.add(i) = *src.add(i);
        }
    }
    dest
}

#[no_mangle]
pub unsafe extern "C" fn memcmp(a: *const u8, b: *const u8, n: usize) -> i32 {
    let mut i = 0;
    while i < n {
        let (x, y) = (*a.add(i), *b.add(i));
        if x != y {
            return x as i32 - y as i32;
        }
        i += 1;
    }
    0
}
