//! Requested live allocation bytes, distinct from resident memory/allocator overhead.
#[cfg(not(feature = "plain-timing"))]
use std::alloc::{GlobalAlloc, Layout, System};
#[cfg(not(feature = "plain-timing"))]
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(not(feature = "plain-timing"))]
pub struct Meter;
#[cfg(not(feature = "plain-timing"))]
static LIVE: AtomicUsize = AtomicUsize::new(0);
#[cfg(not(feature = "plain-timing"))]
static PEAK: AtomicUsize = AtomicUsize::new(0);
#[cfg(not(feature = "plain-timing"))]
static TOTAL: AtomicUsize = AtomicUsize::new(0);

#[cfg(not(feature = "plain-timing"))]
fn add(size: usize) {
    TOTAL.fetch_add(size, Ordering::Relaxed);
    let live = LIVE.fetch_add(size, Ordering::Relaxed) + size;
    PEAK.fetch_max(live, Ordering::Relaxed);
}

// SAFETY: every allocation forwards the exact layout/pointer to System. Counters
// allocate nothing, and never affect allocator results or ownership.
#[cfg(not(feature = "plain-timing"))]
unsafe impl GlobalAlloc for Meter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: caller supplies a valid allocation layout.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            add(layout.size());
        }
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: caller supplies a valid allocation layout.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            add(layout.size());
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        // SAFETY: pointer and original layout are forwarded unchanged.
        unsafe { System.dealloc(pointer, layout) };
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // SAFETY: caller's valid pointer/layout/size are forwarded unchanged.
        let next = unsafe { System.realloc(pointer, layout, size) };
        if !next.is_null() {
            LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
            add(size);
        }
        next
    }
}

pub fn live() -> usize {
    #[cfg(not(feature = "plain-timing"))]
    {
        LIVE.load(Ordering::Relaxed)
    }
    #[cfg(feature = "plain-timing")]
    {
        0
    }
}
pub fn total() -> usize {
    #[cfg(not(feature = "plain-timing"))]
    {
        TOTAL.load(Ordering::Relaxed)
    }
    #[cfg(feature = "plain-timing")]
    {
        0
    }
}
pub fn peak() -> usize {
    #[cfg(not(feature = "plain-timing"))]
    {
        PEAK.load(Ordering::Relaxed)
    }
    #[cfg(feature = "plain-timing")]
    {
        0
    }
}
pub fn reset_peak() {
    #[cfg(not(feature = "plain-timing"))]
    {
        PEAK.store(live(), Ordering::Relaxed);
    }
}
