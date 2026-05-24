#![no_std]
#![no_main]

mod idt;
mod io;
mod paging;
mod serial;

use core::fmt::Write;
use core::panic::PanicInfo;

use serial::Serial;

#[no_mangle]
#[link_section = ".text.boot"]
pub extern "C" fn _start() -> ! {
    Serial::init();
    let mut com = Serial;
    let _ = writeln!(com, "[guest] kernel entered");
    unsafe { paging::init_identity_map(); }
    let _ = writeln!(com, "[guest] paging enabled");
    unsafe { idt::init(); }
    let _ = writeln!(com, "[guest] idt loaded");
    let _ = writeln!(com, "hello from the kernel (long mode)");
    loop {
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack));
        }
    }
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
