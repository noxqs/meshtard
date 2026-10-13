//! A global allocator that counts live bytes and refuses requests over the fuzzing
//! limits: one allocation over [`MAX_SINGLE`], or more than [`MAX_LIVE`] in use at
//! once. A refused request makes the process abort ("memory allocation of N bytes
//! failed"), which the fuzzer reports as an oversized allocation; `try_reserve` and
//! other fallible paths see an ordinary allocation failure.
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Largest single allocation a reader may request.
pub const MAX_SINGLE: usize = 256 << 20;
/// Most memory in use at once.
pub const MAX_LIVE: usize = 1 << 30;

pub struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static ENFORCE: AtomicBool = AtomicBool::new(true);

/// Bytes in use now.
pub fn live() -> usize {
    LIVE.load(Ordering::Relaxed)
}

/// The most bytes in use since the last [`reset_peak`].
pub fn peak() -> usize {
    PEAK.load(Ordering::Relaxed)
}

pub fn reset_peak() {
    PEAK.store(live(), Ordering::Relaxed);
}

/// Turns the limits on or off (seed generation exports whole models).
pub fn enforce(on: bool) {
    ENFORCE.store(on, Ordering::Relaxed);
}

fn admit(size: usize) -> bool {
    if !ENFORCE.load(Ordering::Relaxed) {
        return true;
    }
    size <= MAX_SINGLE && live().saturating_add(size) <= MAX_LIVE
}

fn added(size: usize) {
    let now = LIVE.fetch_add(size, Ordering::Relaxed) + size;
    PEAK.fetch_max(now, Ordering::Relaxed);
}

// SAFETY: forwards to `System`, only counting sizes and refusing (null) requests over
// the limits, which `GlobalAlloc` allows.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if !admit(layout.size()) {
            return std::ptr::null_mut();
        }
        // SAFETY: same contract as the caller's.
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            added(layout.size());
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if !admit(layout.size()) {
            return std::ptr::null_mut();
        }
        // SAFETY: same contract as the caller's.
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            added(layout.size());
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: same contract as the caller's.
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if new_size > layout.size() && !admit(new_size - layout.size()) {
            return std::ptr::null_mut();
        }
        if ENFORCE.load(Ordering::Relaxed) && new_size > MAX_SINGLE {
            return std::ptr::null_mut();
        }
        // SAFETY: same contract as the caller's.
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
            added(new_size);
        }
        p
    }
}
