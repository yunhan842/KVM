//! Bump allocator backing the `alloc` crate.
//!
//! A fixed-size BSS region (`HEAP`) is handed out by advancing a single
//! `next` pointer. `dealloc` is a no-op -- this allocator never frees.
//!
//! Design rationale: see docs/superpowers/specs/2026-05-24-minikvm-month2-heap-design.md
//!
//! Safety: the `unsafe impl Sync` below relies on three invariants:
//!   1. Single vCPU, no SMP (CLAUDE.md locked decision) -- no parallel threads.
//!   2. Interrupts off during normal execution; IDT handlers don't allocate.
//!   3. `dealloc` is a no-op, so reentrancy from a drop is harmless.
//! If a future slice turns IRQs on AND allocates from a handler, revisit
//! this -- migration options are spin::Mutex, AtomicUsize CAS, or
//! interrupt-masked alloc.

// Items are unused until Task 2 calls `init()` from `_start`. The
// `#[global_allocator]` attribute ALONE isn't enough to keep the
// allocator's machinery live across dead-code analysis; rustc 1.95
// also has the diagnostic-rendering ICE flagged in the devlog. Same
// allow workaround as slices 1-2; removed in Task 2.
#![allow(dead_code)]

use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::ptr::addr_of_mut;

pub const HEAP_SIZE: usize = 64 * 1024;

#[repr(C, align(16))]
struct HeapStorage([u8; HEAP_SIZE]);

static mut HEAP: HeapStorage = HeapStorage([0; HEAP_SIZE]);

struct State {
    base: usize, // start of HEAP
    end:  usize, // base + HEAP_SIZE (exclusive)
    next: usize, // next free byte; invariant: base <= next <= end
}

pub struct BumpAllocator {
    state: UnsafeCell<State>,
}

// See the module-level safety note above.
unsafe impl Sync for BumpAllocator {}

#[global_allocator]
static ALLOCATOR: BumpAllocator = BumpAllocator {
    state: UnsafeCell::new(State { base: 0, end: 0, next: 0 }),
};

/// Round `addr` up to the next multiple of `align`.
///
/// `align` must be a power of two (`Layout::align()` guarantees this).
/// Returns `addr` unchanged when it is already aligned.
///
/// Integer overflow: `addr + align - 1` cannot wrap a 64-bit `usize` at
/// our scale (HEAP is a few MiB into 64 MiB guest space; `align` is at
/// most 2^12 for any reasonable type). Documented assumption.
const fn align_up(addr: usize, align: usize) -> usize {
    (addr + align - 1) & !(align - 1)
}

/// Activate the allocator. Must be called from `_start` AFTER the IDT is
/// installed and BEFORE any code that allocates (`Box`, `Vec`, etc.).
///
/// Safety: called exactly once, single-threaded, before any allocation.
pub unsafe fn init() {
    let base = addr_of_mut!(HEAP) as usize;
    // Sound: we are the only thread, no other code accesses ALLOCATOR
    // before this returns.
    let state = &mut *ALLOCATOR.state.get();
    state.base = base;
    state.end  = base + HEAP_SIZE;
    state.next = base;
}

unsafe impl GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let state = &mut *self.state.get();

        let aligned = align_up(state.next, layout.align());
        let new_next = match aligned.checked_add(layout.size()) {
            Some(n) => n,
            None    => return core::ptr::null_mut(), // size overflow (unreachable in practice)
        };
        if new_next > state.end {
            return core::ptr::null_mut(); // OOM
        }

        state.next = new_next;
        aligned as *mut u8
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        // Bump allocator: never frees. Memory used is monotonic over the
        // kernel's lifetime.
    }
}
