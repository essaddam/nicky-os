//! Counting allocator runs in its own binary, never in timing measurements.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;
use transformations_banc::{action_index, catalog};

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);

// SAFETY: the allocator delegates all pointer/layout operations unchanged to
// System. Atomic counters track requested layout sizes only, without allocation.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: GlobalAlloc caller provides the valid layout.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: ptr/layout are forwarded exactly as supplied by caller.
        unsafe {
            System.dealloc(ptr, layout);
        }
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, old: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller's existing allocation and new size go unchanged.
        let new = unsafe { System.realloc(ptr, old, new_size) };
        if !new.is_null() {
            LIVE.fetch_add(new_size, Ordering::Relaxed);
            LIVE.fetch_sub(old.size(), Ordering::Relaxed);
        }
        new
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn main() {
    for devices in [1, 100, 1_000] {
        let cat = catalog(devices);
        let before = LIVE.load(Ordering::Relaxed);
        let index = std::hint::black_box(action_index(&cat));
        let live = LIVE.load(Ordering::Relaxed);
        let table = live.checked_sub(before).unwrap();
        let stack = std::mem::size_of_val(&index);
        let capacity = index.capacity();
        drop(index);
        let after = LIVE.load(Ordering::Relaxed);
        assert_eq!(
            before, after,
            "index drop must free its requested heap bytes"
        );
        println!(
            "{}",
            json!({"type":"memory", "name":"experimental_borrowed_action_id_index",
                "devices":devices,"entries":devices,"capacity":capacity,
                "requested_live_heap_bytes":table,"stack_header_bytes":stack,
                "requested_heap_bytes_after_drop_delta":after-before,
                "boundary":"additional ID HashMap only; Arc identity bytes and Entry storage are shared with production catalog; requested allocator layout bytes, excludes allocator metadata/RSS"
            })
        );
    }
}
