//! Exact requested allocation for a reserved std HashMap<u64, Handle8>.
use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
#[repr(C)]
#[derive(Clone, Copy)]
struct Handle8 {
    slot: u32,
    generation: u32,
}
struct Meter;
static LIVE: AtomicUsize = AtomicUsize::new(0);
#[global_allocator]
static ALLOC: Meter = Meter;
unsafe impl GlobalAlloc for Meter {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            LIVE.fetch_add(l.size(), Ordering::Relaxed);
        }
        p
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(l) };
        if !p.is_null() {
            LIVE.fetch_add(l.size(), Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size(), Ordering::Relaxed);
        unsafe {
            System.dealloc(p, l);
        }
    }
}
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    for arg in args {
        let n: usize = arg.parse().expect("nonnegative count");
        assert!(
            n <= 2_000_000,
            "probe bound; larger tables must be extrapolated"
        );
        let before = LIVE.load(Ordering::Relaxed);
        let map: HashMap<u64, Handle8> = HashMap::with_capacity(n);
        let requested = LIVE.load(Ordering::Relaxed) - before;
        let capacity = map.capacity();
        drop(map);
        let after = LIVE.load(Ordering::Relaxed);
        assert_eq!(before, after, "map allocation must be released");
        println!(
            "{{\"entries_requested\":{n},\"capacity\":{capacity},\"requested_bytes\":{requested},\"header_bytes\":{},\"entry_layout_bytes\":{},\"scope\":\"reserved HashMap allocation; no RSS claim\"}}",
            std::mem::size_of::<HashMap<u64, Handle8>>(),
            std::mem::size_of::<(u64, Handle8)>()
        );
    }
}
