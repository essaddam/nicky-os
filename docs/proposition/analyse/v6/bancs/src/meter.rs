//! Requested allocation sizes only: allocator bookkeeping and RSS are excluded.
#[cfg(feature = "compteurs")]
use std::alloc::{GlobalAlloc, Layout, System};
#[cfg(feature = "compteurs")]
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(feature = "compteurs")]
pub struct Meter;
#[cfg(feature = "compteurs")]
static LIVE: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "compteurs")]
static PEAK: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "compteurs")]
static TOTAL: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "compteurs")]
static CALLS: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, Default)]
pub struct Counters {
    pub live: usize,
    pub peak: usize,
    pub total: usize,
    pub calls: usize,
}

#[cfg(feature = "compteurs")]
fn add(bytes: usize) {
    CALLS.fetch_add(1, Ordering::Relaxed);
    TOTAL.fetch_add(bytes, Ordering::Relaxed);
    let live = LIVE.fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK.fetch_max(live, Ordering::Relaxed);
}

// SAFETY: all pointers/layouts are forwarded unchanged to the system allocator;
// the counters allocate nothing and never change ownership.
#[cfg(feature = "compteurs")]
unsafe impl GlobalAlloc for Meter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's allocation contract is forwarded unchanged.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            add(layout.size());
        }
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's allocation contract is forwarded unchanged.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            add(layout.size());
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        // SAFETY: the original pointer and its layout are forwarded unchanged.
        unsafe { System.dealloc(pointer, layout) };
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // SAFETY: the original pointer/layout and requested size are forwarded.
        let next = unsafe { System.realloc(pointer, layout, size) };
        if !next.is_null() {
            LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
            add(size);
        }
        next
    }
}

pub fn counters() -> Counters {
    #[cfg(feature = "compteurs")]
    {
        Counters {
            live: LIVE.load(Ordering::Relaxed),
            peak: PEAK.load(Ordering::Relaxed),
            total: TOTAL.load(Ordering::Relaxed),
            calls: CALLS.load(Ordering::Relaxed),
        }
    }
    #[cfg(not(feature = "compteurs"))]
    {
        Counters::default()
    }
}

pub fn reset_peak() {
    #[cfg(feature = "compteurs")]
    PEAK.store(LIVE.load(Ordering::Relaxed), Ordering::Relaxed);
}
