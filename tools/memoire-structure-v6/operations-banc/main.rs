//! Synthetic microkernels over the actual V6 Bank; no runtime/device integration.
#[path = "../layouts.rs"]
mod layouts;
#[path = "../model.rs"]
mod model;
use layouts::*;
use model::*;
use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

struct Meter;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static CALLS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);
#[global_allocator]
static ALLOCATOR: Meter = Meter;
unsafe impl GlobalAlloc for Meter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
            CALLS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        ptr
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
            CALLS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) };
    }
}
const SOURCES: usize = 32;
const MAX_FANOUT: usize = 64;
const MEMBERS: u16 = 2;

fn digest(row: &Row64) -> u64 {
    row.payload
        .wrapping_mul(31)
        .wrapping_add(row.previous_payload.wrapping_mul(17))
        .wrapping_add(row.received_ms.wrapping_mul(13))
        .wrapping_add(row.changed_ms.wrapping_mul(7))
        .wrapping_add(u64::from(row.revision))
        .wrapping_add(u64::from(row.flags) << 16)
        .wrapping_add(u64::from(row.value_kind) << 48)
}

/// Bulk fixture construction deliberately bypasses insert's free-slot scan.
/// All globals remain valid and are checked before the chronometers start.
fn fixture(n: usize, devices: usize) -> (Bank, [u64; SOURCES], usize) {
    assert!(devices >= 1 && n > devices + SOURCES + 5);
    let before = LIVE.load(Ordering::Relaxed);
    let mut bank = Bank::new();
    let total = n + SOURCES * MAX_FANOUT;
    bank.rows = Vec::with_capacity(n);
    bank.aliases = Vec::with_capacity(SOURCES * MAX_FANOUT);
    bank.slots = Vec::with_capacity(total);
    bank.metadata = Vec::with_capacity(total);
    bank.index = std::collections::HashMap::with_capacity(total);
    bank.edges = Vec::with_capacity(total - 1 + devices.min(256));
    for index in 0..n {
        let id = index as u64 + 1;
        let (row_kind, parent) = match index {
            0 => (FAMILY, 0),
            1 => (HOUSE, 1),
            2 => (ROOM, 2),
            3 => (VIRTUAL, 3),
            4 => (ZONE, 2),
            i if i < 5 + devices => (PHYSICAL, 3),
            i => (VALUE, 6 + ((i - 5 - devices) % devices) as u64),
        };
        let is_value = row_kind == VALUE;
        bank.rows.push(Row96 {
            base: Row64 {
                id,
                parent,
                payload: if is_value { 10 } else { 0 },
                value_kind: if is_value { INTEGER } else { NULL },
                row_kind,
                flags: if is_value {
                    WRITE | ACTIVE_GET | HAS_OBSERVATION
                } else {
                    0
                },
                received_ms: if is_value { 100 } else { 0 },
                changed_ms: if is_value { 90 } else { 0 },
                revision: if is_value { 1 } else { 0 },
                ..Row64::default()
            },
            ..Row96::default()
        });
        let handle = Handle8 {
            slot: index as u32,
            generation: 1,
        };
        bank.index.insert(id, handle);
        bank.slots.push(GlobalSlot16 {
            physical_index: index as u32,
            generation: 1,
            meta_index: index as u32,
            arena: 1,
            flags: 1,
            reserved: 0,
        });
        bank.metadata.push(Meta32::default());
        if parent != 0 {
            bank.edges.push(Edge24 {
                from: parent,
                to: id,
                relation: CHILD,
                ..Edge24::default()
            });
        }
    }
    let mut sources = [0; SOURCES];
    let first = 5 + devices;
    for (index, source) in sources.iter_mut().enumerate() {
        *source = (first + (n - first) * index / SOURCES) as u64 + 1;
        for _ in 0..MAX_FANOUT {
            let id = bank.slots.len() as u64 + 1;
            let slot = bank.slots.len() as u32;
            bank.index.insert(
                id,
                Handle8 {
                    slot,
                    generation: 1,
                },
            );
            bank.slots.push(GlobalSlot16 {
                physical_index: bank.aliases.len() as u32,
                generation: 1,
                meta_index: slot,
                arena: 2,
                flags: 1,
                reserved: 0,
            });
            bank.metadata.push(Meta32::default());
            bank.aliases.push(Alias32 {
                alias_id: id,
                owner_id: 4,
                source_id: *source,
                source_slot: (*source - 1) as u32,
                source_generation: 1,
            });
            bank.edges.push(Edge24 {
                from: 4,
                to: id,
                relation: CHILD,
                ..Edge24::default()
            });
        }
    }
    for index in 0..devices.min(256) {
        bank.edges.push(Edge24 {
            from: 5,
            to: index as u64 + 6,
            relation: MEMBERS,
            ..Edge24::default()
        });
    }
    let requested = LIVE.load(Ordering::Relaxed) - before;
    (bank, sources, requested)
}

fn linear(bank: &Bank, id: u64) -> &Row96 {
    bank.rows
        .iter()
        .find(|row| row.base.id == id)
        .expect("known canonical id")
}
fn edge_members(bank: &Bank) -> Vec<u32> {
    bank.edges
        .iter()
        .filter(|edge| edge.from == 5 && edge.relation == MEMBERS)
        .map(|edge| bank.handle(edge.to).unwrap().slot)
        .collect()
}
fn csr_members(csr: &Csr<'_>, zone: Handle8) -> Vec<u32> {
    csr.targets(zone)
        .unwrap()
        .iter()
        .filter(|target| target.relation == MEMBERS)
        .map(|target| target.to)
        .collect()
}
fn zone_digest(bank: &Bank) -> u64 {
    bank.edges
        .iter()
        .filter(|edge| edge.from == 5 && edge.relation == MEMBERS)
        // Fixture canonical ids are dense; assert this conversion before timing.
        .fold(0u64, |sum, edge| sum.wrapping_add(edge.to - 1))
}

fn validate(bank: &Bank, sources: &[u64; SOURCES]) {
    assert_eq!(std::mem::size_of::<Row96>(), 96);
    assert_eq!(std::mem::offset_of!(Row64, previous_payload), 24);
    for (id, handle) in &bank.index {
        let row = bank.read_at(*id, bank.epoch).unwrap();
        let slot = bank.slots[handle.slot as usize];
        assert_eq!(slot.generation, handle.generation);
        if slot.arena == 1 {
            assert_eq!(row.base.id, *id);
            assert_eq!(linear_identity(bank, *id), row.base.id);
        }
    }
    for id in sources {
        assert_eq!(
            digest(&linear(bank, *id).base),
            digest(&bank.read_at(*id, bank.epoch).unwrap().base)
        );
    }
    let csr = bank.csr().unwrap();
    assert_eq!(
        edge_members(bank),
        csr_members(&csr, bank.handle(5).unwrap())
    );
    assert_eq!(
        zone_digest(bank),
        csr_members(&csr, bank.handle(5).unwrap())
            .iter()
            .map(|v| u64::from(*v))
            .sum::<u64>()
    );
}
// O(1) verification of dense bulk identity; do not accidentally do O(N²) validation.
fn linear_identity(bank: &Bank, id: u64) -> u64 {
    bank.rows[(id - 1) as usize].base.id
}

fn semantic_tests() {
    let (mut bank, sources, _) = fixture(128, 8);
    validate(&bank, &sources);
    let source = sources[0];
    let alias = bank.aliases[0].alias_id;
    let mut views = [Row96::default(); MAX_FANOUT];
    for (step, (kind, payload)) in [
        (INTEGER, 21),
        (INTEGER, 22),
        (INTEGER, 22),
        (INTEGER, 23),
        (NULL, 0),
        (NULL, 0),
        (INTEGER, 0),
    ]
    .into_iter()
    .enumerate()
    {
        let at = 1000 + step as u64 * 10;
        bank.observe(source, kind, payload, at, at + 3).unwrap();
        let row = *bank.read_at(source, bank.epoch).unwrap();
        views.fill(row);
        for (index, view) in views.iter().enumerate() {
            let alias_id = alias + index as u64;
            assert_eq!(*view, *bank.read_at(alias_id, bank.epoch).unwrap());
            assert_eq!(
                digest(&view.base),
                digest(&bank.read_at(alias_id, bank.epoch).unwrap().base)
            );
            assert_eq!(
                bank.previous(alias_id).unwrap(),
                bank.previous(source).unwrap()
            );
        }
        match step {
            1 | 2 => assert_eq!(bank.previous(source).unwrap(), Some((INTEGER, 21))),
            3 => assert_eq!(bank.previous(source).unwrap(), Some((INTEGER, 22))),
            4 | 5 => assert_eq!(bank.previous(source).unwrap(), Some((INTEGER, 23))),
            6 => assert_eq!(bank.previous(source).unwrap(), Some((NULL, 0))),
            _ => (),
        }
        if step == 2 {
            assert_eq!(row.base.changed_ms, 1010);
        }
        if step == 5 {
            assert_eq!(row.base.changed_ms, 1040);
        }
    }
    let before = bank.rows.clone();
    assert!(bank.observe(alias, INTEGER, 99, 0, 0).is_err());
    assert_eq!(bank.rows, before);
    println!(
        "{{\"kind\":\"semantic_tests\",\"passed\":true,\"cases\":[\"dense_bulk_identity\",\"lookup_same_row\",\"alias_views_full_row_current_previous_types_dates\",\"identical_keeps_distinct_previous\",\"null_previous\",\"zone_edge_csr_equal_list\",\"alias_observation_rejected\"]}}"
    );
}

fn measure<F: FnMut(u64) -> u64>(
    scenario: &str,
    operation: &str,
    variant: &str,
    fanout: usize,
    iterations: usize,
    mut action: F,
) {
    let mut checksum = 0u64;
    for iteration in 0..iterations.min(256) {
        checksum = checksum.wrapping_add(black_box(action(black_box(iteration as u64))));
    }
    for round in 0..3 {
        CALLS.store(0, Ordering::Relaxed);
        BYTES.store(0, Ordering::Relaxed);
        let started = Instant::now();
        for iteration in 0..iterations {
            checksum = checksum.wrapping_add(black_box(action(black_box(iteration as u64))));
        }
        let elapsed = started.elapsed().as_nanos();
        let calls = CALLS.load(Ordering::Relaxed);
        let bytes = BYTES.load(Ordering::Relaxed);
        black_box(checksum);
        println!(
            "{{\"kind\":\"sample\",\"scenario\":\"{scenario}\",\"operation\":\"{operation}\",\"variant\":\"{variant}\",\"fanout\":{fanout},\"round\":{round},\"iterations\":{iterations},\"elapsed_ns\":{elapsed},\"ns_per_op\":{},\"allocations_per_op\":{},\"allocated_bytes_per_op\":{},\"checksum\":{checksum}}}",
            elapsed as f64 / iterations as f64,
            calls as f64 / iterations as f64,
            bytes as f64 / iterations as f64
        );
    }
}

fn run(scenario: &str, n: usize, devices: usize) {
    let (mut bank, sources, requested) = fixture(n, devices);
    validate(&bank, &sources);
    let mut views = vec![Row96::default(); SOURCES * MAX_FANOUT];
    let csr = bank.csr().unwrap();
    println!(
        "{{\"kind\":\"memory\",\"scenario\":\"{scenario}\",\"canonical_rows\":{n},\"physical_nodes\":{devices},\"materialized_aliases\":{},\"instrumented_sources\":{SOURCES},\"requested_bank_allocation_bytes\":{requested},\"materialized_views\":{{\"rows\":{},\"used_bytes\":{},\"capacity_bytes\":{}}},\"memory\":{}}}",
        bank.aliases.len(),
        views.len(),
        views.len() * 96,
        views.capacity() * 96,
        bank.memory(&csr)
    );
    drop(csr);
    let scan_iterations = (8_000_000 / n).clamp(16, 128);
    let queries: Vec<_> = (0..scan_iterations)
        .map(|index| {
            let random = (index as u64)
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            random % n as u64 + 1
        })
        .collect();
    measure(scenario, "lookup", "linear_scan", 0, scan_iterations, |i| {
        digest(&linear(&bank, black_box(queries[i as usize % queries.len()])).base)
    });
    measure(scenario, "lookup", "bank_read_at", 0, 8192, |i| {
        digest(
            &bank
                .read_at(black_box(queries[i as usize % queries.len()]), bank.epoch)
                .unwrap()
                .base,
        )
    });
    let source = sources[0];
    measure(scenario, "observe", "identical", 0, 4096, |i| {
        bank.observe(
            black_box(source),
            INTEGER,
            black_box(10),
            black_box(i + 200),
            black_box(i + 203),
        )
        .unwrap();
        digest(&bank.read_at(source, bank.epoch).unwrap().base)
    });
    let mut tick = 10000u64;
    measure(scenario, "observe", "changed_previous", 0, 4096, |_| {
        tick += 1;
        bank.observe(
            black_box(source),
            INTEGER,
            black_box(20 + tick % 2),
            black_box(tick),
            black_box(tick + 3),
        )
        .unwrap();
        digest(&bank.read_at(source, bank.epoch).unwrap().base)
    });
    for fanout in [1, 4, 16, 64] {
        measure(
            scenario,
            "fanout_update",
            "canonical_alias",
            fanout,
            1024,
            |i| {
                tick += 1;
                let source = sources[i as usize % SOURCES];
                bank.observe(
                    black_box(source),
                    INTEGER,
                    black_box(tick),
                    black_box(tick),
                    black_box(tick + 3),
                )
                .unwrap();
                digest(&bank.read_at(source, bank.epoch).unwrap().base)
            },
        );
        measure(
            scenario,
            "fanout_update",
            "materialized_row96_views",
            fanout,
            1024,
            |i| {
                tick += 1;
                let index = i as usize % SOURCES;
                let source = sources[index];
                bank.observe(
                    black_box(source),
                    INTEGER,
                    black_box(tick),
                    black_box(tick),
                    black_box(tick + 3),
                )
                .unwrap();
                let row = *bank.read_at(source, bank.epoch).unwrap();
                let selected = &mut views[index * MAX_FANOUT..index * MAX_FANOUT + fanout];
                selected.fill(row);
                black_box(selected);
                digest(&row.base)
            },
        );
        measure(scenario, "alias_read", "bank_read_at", fanout, 4096, |i| {
            let alias = &bank.aliases
                [(i as usize % SOURCES) * MAX_FANOUT + (i as usize / SOURCES) % fanout];
            digest(
                &bank
                    .read_at(black_box(alias.alias_id), bank.epoch)
                    .unwrap()
                    .base,
            )
        });
    }
    let csr = bank.csr().unwrap();
    let zone = bank.handle(5).unwrap();
    let scan_iterations = (4_000_000 / bank.edges.len()).clamp(4, 128);
    measure(
        scenario,
        "zone_members",
        "edge24_scan",
        0,
        scan_iterations,
        |_| zone_digest(black_box(&bank)),
    );
    measure(scenario, "zone_members", "csr_targets", 0, 4096, |_| {
        csr.targets(black_box(zone))
            .unwrap()
            .iter()
            .filter(|target| target.relation == MEMBERS)
            .fold(0u64, |sum, target| sum.wrapping_add(u64::from(target.to)))
    });
}
fn main() {
    semantic_tests();
    if std::env::args().nth(1).as_deref() == Some("--self-test") {
        return;
    }
    for argument in std::env::args().skip(1) {
        let fields: Vec<_> = argument.split(':').collect();
        assert_eq!(fields.len(), 3);
        run(
            fields[0],
            fields[1].parse().unwrap(),
            fields[2].parse().unwrap(),
        );
    }
}
