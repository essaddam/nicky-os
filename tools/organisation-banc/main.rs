//! Standalone synthetic organization representations; no production imports or device I/O.
use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::{HashMap, HashSet};
use std::hint::black_box;
use std::mem::size_of;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

struct Meter;
static LIVE: AtomicUsize = AtomicUsize::new(0);
#[global_allocator]
static ALLOCATOR: Meter = Meter;
unsafe impl GlobalAlloc for Meter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe {
            System.dealloc(ptr, layout);
        }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new: usize) -> *mut u8 {
        let result = unsafe { System.realloc(ptr, layout, new) };
        if !result.is_null() {
            LIVE.fetch_add(new, Ordering::Relaxed);
            LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        }
        result
    }
}
fn live() -> usize {
    LIVE.load(Ordering::Relaxed)
}

// Canonical Row96 layout copied from catalogue-banc / alias-banc. Numeric space is
// reserved for every kind, including containers; no kind gets a cheaper allocation.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Numeric32 {
    min: u64,
    max: u64,
    step: u64,
    anchor: u64,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Row64 {
    id: u64,
    parent: u64,
    payload: u64,
    observed_ms: u64,
    received_ms: u64,
    changed_ms: u64,
    schema: u32,
    aux: u32,
    revision: u32,
    flags: u16,
    row_kind: u8,
    value_kind: u8,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct OrgNode96 {
    base: Row64,
    numeric: Numeric32,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct StableEdge24 {
    from: u64,
    to: u64,
    relation: u16,
    flags: u16,
    revision: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct CachedEdge16 {
    from: u32,
    to: u32,
    relation: u16,
    flags: u16,
    revision: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct Target12 {
    to: u32,
    relation: u16,
    flags: u16,
    revision: u32,
}

const FAMILY: u8 = 1;
const HOUSE: u8 = 2;
const FLOOR: u8 = 3;
const ROOM: u8 = 4;
const ZONE: u8 = 5;
const DEVICE: u8 = 6;
const VALUE: u8 = 7;
const ACTION: u8 = 8;
const CONTAINERS: usize = 39;

struct Fixture {
    nodes: Vec<OrgNode96>,
    edges: Vec<StableEdge24>,
    zones: Vec<u32>,
    devices: Vec<u32>,
}
fn slot(n: usize) -> u32 {
    u32::try_from(n).expect("slot exceeds u32")
}
fn node(id: u64, parent: u64, kind: u8) -> OrgNode96 {
    OrgNode96 {
        base: Row64 {
            id,
            parent,
            payload: id.wrapping_mul(17),
            observed_ms: id + 10,
            received_ms: id + 11,
            changed_ms: id + 9,
            schema: u32::from(kind),
            aux: (id % 113) as u32,
            revision: (id % 97) as u32,
            flags: (id % 7) as u16,
            row_kind: kind,
            value_kind: u8::from(kind == VALUE),
        },
        numeric: Numeric32 {
            min: id % 3,
            max: id % 101 + 100,
            step: 1,
            anchor: id % 13,
        },
    }
}
fn fixture(
    devices: usize,
    values: usize,
    actions: usize,
    scale: usize,
) -> Result<Fixture, &'static str> {
    if devices == 0 || scale == 0 {
        return Err("devices and scale must be positive");
    }
    let per_world = CONTAINERS
        .checked_add(devices)
        .and_then(|n| n.checked_add(values))
        .and_then(|n| n.checked_add(actions))
        .ok_or("node count overflow")?;
    let count = per_world.checked_mul(scale).ok_or("node count overflow")?;
    let edge_count = devices
        .checked_add(24)
        .and_then(|n| n.checked_mul(2))
        .and_then(|n| n.checked_mul(scale))
        .ok_or("edge count overflow")?;
    if count > u32::MAX as usize || edge_count > u32::MAX as usize {
        return Err("u32 capacity exceeded");
    }
    let mut f = Fixture {
        nodes: Vec::with_capacity(count),
        edges: Vec::with_capacity(edge_count),
        zones: Vec::with_capacity(8 * scale),
        devices: Vec::with_capacity(devices * scale),
    };
    for world in 0..scale {
        let start = f.nodes.len();
        let id = |local: usize| (world * per_world + local + 1) as u64;
        f.nodes.push(node(id(0), 0, FAMILY));
        for house in 0..2 {
            f.nodes.push(node(id(1 + house), id(0), HOUSE));
        }
        for floor in 0..4 {
            f.nodes.push(node(id(3 + floor), id(1 + floor / 2), FLOOR));
        }
        for room in 0..24 {
            f.nodes.push(node(id(7 + room), id(3 + room / 6), ROOM));
        }
        for zone in 0..8 {
            f.zones.push(slot(start + 31 + zone));
            f.nodes.push(node(id(31 + zone), id(1 + zone / 4), ZONE));
        }
        for device in 0..devices {
            let house = device % 2;
            let room = house * 12 + (device / 2) % 12;
            f.devices.push(slot(start + CONTAINERS + device));
            f.nodes
                .push(node(id(CONTAINERS + device), id(7 + room), DEVICE));
        }
        for value in 0..values {
            f.nodes.push(node(
                id(CONTAINERS + devices + value),
                id(CONTAINERS + value % devices),
                VALUE,
            ));
        }
        // Inventory service/action contracts are aggregate counts, not one action per entity.
        for action in 0..actions {
            let parent = if action % 3 == 0 { 0 } else { 1 + action % 2 };
            f.nodes.push(node(
                id(CONTAINERS + devices + values + action),
                id(parent),
                ACTION,
            ));
        }
        for member in 0..24 + devices {
            let (local, house, ordinal) = if member < 24 {
                (7 + member, member / 12, member % 12)
            } else {
                let d = member - 24;
                (CONTAINERS + d, d % 2, d / 2)
            };
            for offset in [0, 1] {
                let from = id(31 + house * 4 + (ordinal + offset) % 4);
                let to = id(local);
                f.edges.push(StableEdge24 {
                    from,
                    to,
                    relation: 1 + (to % 3) as u16,
                    flags: ((from + to) % 11) as u16,
                    revision: ((from * 13 + to) % 997) as u32,
                });
            }
        }
    }
    f.edges.sort_unstable();
    validate(&f)?;
    Ok(f)
}

fn validate(f: &Fixture) -> Result<(), &'static str> {
    let mut ids = HashMap::with_capacity(f.nodes.len());
    for n in &f.nodes {
        if n.base.id == 0 || ids.insert(n.base.id, n).is_some() {
            return Err("duplicate or zero ID");
        }
    }
    for n in &f.nodes {
        let parent = ids.get(&n.base.parent).map(|p| p.base.row_kind);
        if !matches!(
            (n.base.row_kind, parent),
            (FAMILY, None)
                | (HOUSE, Some(FAMILY))
                | (FLOOR | ZONE, Some(HOUSE))
                | (ROOM, Some(FLOOR))
                | (DEVICE, Some(ROOM))
                | (VALUE, Some(DEVICE))
                | (ACTION, Some(FAMILY | HOUSE))
        ) || (n.base.row_kind == FAMILY && n.base.parent != 0)
        {
            return Err("invalid physical parent");
        }
    }
    let house = |mut id: u64| -> Option<u64> {
        for _ in 0..5 {
            let n = ids.get(&id)?;
            if n.base.row_kind == HOUSE {
                return Some(id);
            }
            id = n.base.parent;
        }
        None
    };
    let mut pairs = HashSet::with_capacity(f.edges.len());
    for e in &f.edges {
        let from = ids.get(&e.from).ok_or("dangling source")?;
        let to = ids.get(&e.to).ok_or("dangling target")?;
        if from.base.row_kind != ZONE || !matches!(to.base.row_kind, ROOM | DEVICE) {
            return Err("invalid membership kinds");
        }
        if house(e.from) != house(e.to) {
            return Err("cross-house membership");
        }
        if !pairs.insert((e.from, e.to)) {
            return Err("duplicate membership");
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
enum Variant {
    Nested,
    Flat,
    Csr,
}
impl Variant {
    fn parse(s: &str) -> Result<Self, &'static str> {
        match s {
            "nested_ids" => Ok(Self::Nested),
            "flat_cached" => Ok(Self::Flat),
            "csr_cached" => Ok(Self::Csr),
            _ => Err("unknown variant"),
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Nested => "nested_ids",
            Self::Flat => "flat_cached",
            Self::Csr => "csr_cached",
        }
    }
}
enum Forward {
    Nested(Vec<Vec<StableEdge24>>),
    Flat {
        offsets: Vec<u32>,
        edges: Vec<CachedEdge16>,
    },
    Csr {
        offsets: Vec<u32>,
        edges: Vec<Target12>,
    },
}
struct Reverse {
    offsets: Vec<u32>,
    zones: Vec<u32>,
}
struct Graph {
    nodes: Vec<OrgNode96>,
    identity: HashMap<u64, u32>,
    forward: Forward,
    reverse: Reverse,
    identity_bytes: usize,
}
fn bytes<T>(v: &Vec<T>) -> usize {
    v.capacity() * size_of::<T>()
}
fn offsets(counts: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(counts.len() + 1);
    out.push(0);
    for count in counts {
        out.push(out.last().unwrap() + count);
    }
    out
}
impl Graph {
    fn build(f: &Fixture, variant: Variant) -> Self {
        let nodes = f.nodes.clone();
        let before = live();
        let identity: HashMap<_, _> = nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.base.id, slot(i)))
            .collect();
        let identity_bytes = live() - before;
        let mut counts = vec![0_u32; nodes.len()];
        for e in &f.edges {
            counts[identity[&e.from] as usize] += 1;
        }
        let forward = match variant {
            Variant::Nested => {
                let mut rows: Vec<_> = counts
                    .iter()
                    .map(|c| Vec::with_capacity(*c as usize))
                    .collect();
                for e in &f.edges {
                    rows[identity[&e.from] as usize].push(*e);
                }
                Forward::Nested(rows)
            }
            Variant::Flat => Forward::Flat {
                offsets: offsets(&counts),
                edges: f
                    .edges
                    .iter()
                    .map(|e| CachedEdge16 {
                        from: identity[&e.from],
                        to: identity[&e.to],
                        relation: e.relation,
                        flags: e.flags,
                        revision: e.revision,
                    })
                    .collect(),
            },
            Variant::Csr => Forward::Csr {
                offsets: offsets(&counts),
                edges: f
                    .edges
                    .iter()
                    .map(|e| Target12 {
                        to: identity[&e.to],
                        relation: e.relation,
                        flags: e.flags,
                        revision: e.revision,
                    })
                    .collect(),
            },
        };
        counts.fill(0);
        for e in &f.edges {
            counts[identity[&e.to] as usize] += 1;
        }
        let reverse_offsets = offsets(&counts);
        let mut cursors = reverse_offsets[..nodes.len()].to_vec();
        let mut zones = vec![0; f.edges.len()];
        for e in &f.edges {
            let to = identity[&e.to] as usize;
            zones[cursors[to] as usize] = identity[&e.from];
            cursors[to] += 1;
        }
        Self {
            nodes,
            identity,
            forward,
            reverse: Reverse {
                offsets: reverse_offsets,
                zones,
            },
            identity_bytes,
        }
    }
    fn forward_bytes(&self) -> usize {
        match &self.forward {
            Forward::Nested(v) => bytes(v) + v.iter().map(bytes).sum::<usize>(),
            Forward::Flat { offsets, edges } => bytes(offsets) + bytes(edges),
            Forward::Csr { offsets, edges } => bytes(offsets) + bytes(edges),
        }
    }
    fn reverse_bytes(&self) -> usize {
        bytes(&self.reverse.offsets) + bytes(&self.reverse.zones)
    }
    fn visit(&self, source: u32, mut consume: impl FnMut(StableEdge24)) {
        let i = source as usize;
        let id = self.nodes[i].base.id;
        match &self.forward {
            Forward::Nested(rows) => {
                for edge in &rows[i] {
                    // Baseline resolves retained stable IDs through the common identity map.
                    let target = self.identity[&edge.to] as usize;
                    consume(StableEdge24 {
                        to: self.nodes[target].base.id,
                        ..*edge
                    });
                }
            }
            Forward::Flat { offsets, edges } => {
                for e in &edges[offsets[i] as usize..offsets[i + 1] as usize] {
                    consume(StableEdge24 {
                        from: self.nodes[e.from as usize].base.id,
                        to: self.nodes[e.to as usize].base.id,
                        relation: e.relation,
                        flags: e.flags,
                        revision: e.revision,
                    });
                }
            }
            Forward::Csr { offsets, edges } => {
                for e in &edges[offsets[i] as usize..offsets[i + 1] as usize] {
                    consume(StableEdge24 {
                        from: id,
                        to: self.nodes[e.to as usize].base.id,
                        relation: e.relation,
                        flags: e.flags,
                        revision: e.revision,
                    });
                }
            }
        }
    }
    fn zone_sum(&self, source: u32) -> u64 {
        let mut sum = 0_u64;
        self.visit(source, |e| sum = sum.wrapping_add(edge_hash(e)));
        sum
    }
    fn reverse_sum(&self, member: u32) -> u64 {
        let i = member as usize;
        self.reverse.zones[self.reverse.offsets[i] as usize..self.reverse.offsets[i + 1] as usize]
            .iter()
            .fold(0_u64, |s, z| {
                s.wrapping_add(mix(self.nodes[*z as usize].base.id))
            })
    }
    fn node_sum(&self) -> u64 {
        self.nodes.iter().fold(0_u64, |s, n| {
            let b = n.base;
            let words = [
                b.id,
                b.parent,
                b.payload,
                b.observed_ms,
                b.received_ms,
                b.changed_ms,
                u64::from(b.schema),
                u64::from(b.aux),
                u64::from(b.revision),
                u64::from(b.flags),
                u64::from(b.row_kind),
                u64::from(b.value_kind),
                n.numeric.min,
                n.numeric.max,
                n.numeric.step,
                n.numeric.anchor,
            ];
            words.iter().fold(s, |acc, w| acc.wrapping_add(mix(*w)))
        })
    }
}
fn mix(mut x: u64) -> u64 {
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 27;
    x.wrapping_mul(0x94d0_49bb_1331_11eb) ^ (x >> 31)
}
fn edge_hash(e: StableEdge24) -> u64 {
    mix(e.from).wrapping_add(mix(e.to).rotate_left(7))
        ^ u64::from(e.relation)
        ^ (u64::from(e.flags) << 16)
        ^ (u64::from(e.revision) << 32)
}
fn random_queries(list: &[u32], count: usize) -> Vec<u32> {
    let mut state = 0x1234_5678_9abc_def0_u64;
    (0..count)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            list[state as usize % list.len()]
        })
        .collect()
}
fn timed(mut run: impl FnMut() -> u64) -> (u128, u64) {
    black_box(run());
    let mut times = [0_u128; 7];
    let mut expected = None;
    for time in &mut times {
        let start = Instant::now();
        let checksum = black_box(run());
        *time = start.elapsed().as_nanos();
        if let Some(previous) = expected {
            assert_eq!(checksum, previous);
        }
        expected = Some(checksum);
    }
    times.sort_unstable();
    (times[3], expected.unwrap())
}
fn rss_anon() -> usize {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|line| line.starts_with("RssAnon:"))
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|v| v.parse::<usize>().ok())
                .map(|kb| kb * 1024)
        })
        .unwrap_or(0)
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(2);
    }
}
fn run() -> Result<(), &'static str> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 7 || args[1] != "--bench" {
        return Err(
            "usage: --bench nested_ids|flat_cached|csr_cached DEVICES VALUES ACTIONS SCALE",
        );
    }
    let variant = Variant::parse(&args[2])?;
    let number = |i: usize| args[i].parse::<usize>().map_err(|_| "invalid count");
    let (devices, values, actions, scale) = (number(3)?, number(4)?, number(5)?, number(6)?);
    let f = fixture(devices, values, actions, scale)?;
    let query_count = (1_000_000 * f.zones.len() / f.edges.len()).clamp(4096, 65536);
    let forward_queries = random_queries(&f.zones, query_count);
    let reverse_queries = random_queries(&f.devices, 500_000);
    let scan_repeats = (1_000_000 / f.edges.len()).clamp(1, 65536);
    // Everything above is an excluded, identical workload fixture. Only Graph is measured.
    let rss_before = rss_anon();
    let before = live();
    let graph = Graph::build(&f, variant);
    black_box(graph.node_sum());
    let nodes_bytes = bytes(&graph.nodes);
    let forward_bytes = graph.forward_bytes();
    let reverse_bytes = graph.reverse_bytes();
    let requested_heap = live() - before;
    assert_eq!(
        requested_heap,
        nodes_bytes + graph.identity_bytes + forward_bytes + reverse_bytes
    );
    let rss_after = rss_anon();
    let (forward_ns, forward_checksum) = timed(|| {
        forward_queries.iter().fold(0_u64, |s, z| {
            s.wrapping_add(black_box(&graph).zone_sum(black_box(*z)))
        })
    });
    let (reverse_ns, reverse_checksum) = timed(|| {
        reverse_queries.iter().fold(0_u64, |s, d| {
            s.wrapping_add(black_box(&graph).reverse_sum(black_box(*d)))
        })
    });
    let (scan_ns, scan_checksum) = timed(|| {
        let mut sum = 0_u64;
        for _ in 0..scan_repeats {
            for zone in &f.zones {
                sum = sum.wrapping_add(black_box(&graph).zone_sum(black_box(*zone)));
            }
        }
        sum
    });
    let checksum = graph.node_sum().wrapping_add(
        f.zones
            .iter()
            .fold(0_u64, |s, z| s.wrapping_add(graph.zone_sum(*z))),
    );
    let forward_visited = forward_queries
        .iter()
        .map(|z| match &graph.forward {
            Forward::Nested(rows) => rows[*z as usize].len(),
            Forward::Flat { offsets, .. } | Forward::Csr { offsets, .. } => {
                (offsets[*z as usize + 1] - offsets[*z as usize]) as usize
            }
        })
        .sum::<usize>();
    let empty_headers_bytes = match &graph.forward {
        Forward::Nested(rows) => {
            rows.iter().filter(|r| r.is_empty()).count() * size_of::<Vec<StableEdge24>>()
        }
        _ => 0,
    };
    println!(
        concat!(
            "{{\"variant\":\"{}\",\"scale\":{},\"nodes\":{},\"devices\":{},\"values\":{},\"actions\":{},\"memberships\":{},",
            "\"rounds\":7,\"node_size\":{},\"stable_edge_size\":{},\"cached_edge_size\":{},\"target_size\":{},\"checksum\":{},",
            "\"requested_heap\":{},\"nodes_bytes\":{},\"identity_index_bytes\":{},\"forward_bytes\":{},\"reverse_bytes\":{},\"empty_headers_bytes\":{},",
            "\"rss_anon_before\":{},\"rss_anon_after\":{},\"forward_ops\":{},\"forward_visited_edges\":{},\"forward_median_ns\":{},\"forward_checksum\":{},",
            "\"reverse_ops\":{},\"reverse_median_ns\":{},\"reverse_checksum\":{},\"scan_repeats\":{},\"scan_visited_edges\":{},\"scan_median_ns\":{},\"scan_checksum\":{}}}"
        ),
        variant.name(),
        scale,
        graph.nodes.len(),
        devices * scale,
        values * scale,
        actions * scale,
        f.edges.len(),
        size_of::<OrgNode96>(),
        size_of::<StableEdge24>(),
        size_of::<CachedEdge16>(),
        size_of::<Target12>(),
        checksum,
        requested_heap,
        nodes_bytes,
        graph.identity_bytes,
        forward_bytes,
        reverse_bytes,
        empty_headers_bytes,
        rss_before,
        rss_after,
        forward_queries.len(),
        forward_visited,
        forward_ns,
        forward_checksum,
        reverse_queries.len(),
        reverse_ns,
        reverse_checksum,
        scan_repeats,
        scan_repeats * f.edges.len(),
        scan_ns,
        scan_checksum
    );
    black_box(graph);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn small() -> Fixture {
        fixture(10, 25, 7, 1).unwrap()
    }
    fn reconstructed(graph: &Graph) -> Vec<StableEdge24> {
        let mut out = Vec::new();
        for i in 0..graph.nodes.len() {
            graph.visit(slot(i), |e| out.push(e));
        }
        out.sort_unstable();
        out
    }
    #[test]
    fn exact_layouts() {
        assert_eq!(
            (
                size_of::<OrgNode96>(),
                size_of::<StableEdge24>(),
                size_of::<CachedEdge16>(),
                size_of::<Target12>()
            ),
            (96, 24, 16, 12)
        );
    }
    #[test]
    fn expected_counts() {
        let f = small();
        assert_eq!(f.nodes.len(), 39 + 10 + 25 + 7);
        assert_eq!(f.edges.len(), (24 + 10) * 2);
        for (kind, count) in [(FAMILY, 1), (HOUSE, 2), (FLOOR, 4), (ROOM, 24), (ZONE, 8)] {
            assert_eq!(
                f.nodes.iter().filter(|n| n.base.row_kind == kind).count(),
                count
            );
        }
    }
    #[test]
    fn duplicate_ids_rejected() {
        let mut f = small();
        f.nodes[1].base.id = f.nodes[0].base.id;
        assert_eq!(validate(&f), Err("duplicate or zero ID"));
    }
    #[test]
    fn dangling_target_rejected() {
        let mut f = small();
        f.edges[0].to = u64::MAX;
        assert_eq!(validate(&f), Err("dangling target"));
    }
    #[test]
    fn dangling_source_rejected() {
        let mut f = small();
        f.edges[0].from = u64::MAX;
        assert_eq!(validate(&f), Err("dangling source"));
    }
    #[test]
    fn cross_house_rejected() {
        let mut f = small();
        let source_house = f
            .nodes
            .iter()
            .find(|n| n.base.id == f.edges[0].from)
            .unwrap()
            .base
            .parent;
        let house_id = f
            .nodes
            .iter()
            .find(|n| n.base.row_kind == HOUSE && n.base.id != source_house)
            .unwrap()
            .base
            .id;
        let floor = f
            .nodes
            .iter()
            .find(|n| n.base.row_kind == FLOOR && n.base.parent == house_id)
            .unwrap()
            .base
            .id;
        f.edges[0].to = f
            .nodes
            .iter()
            .find(|n| n.base.row_kind == ROOM && n.base.parent == floor)
            .unwrap()
            .base
            .id;
        assert_eq!(validate(&f), Err("cross-house membership"));
    }
    #[test]
    fn duplicate_membership_rejected() {
        let mut f = small();
        f.edges.push(f.edges[0]);
        assert_eq!(validate(&f), Err("duplicate membership"));
    }
    #[test]
    fn overlap_preserves_physical_parent_and_single_state() {
        let f = small();
        let device = f.devices[0] as usize;
        let before = f.nodes[device];
        let links: Vec<_> = f.edges.iter().filter(|e| e.to == before.base.id).collect();
        assert_eq!(links.len(), 2);
        assert_ne!(links[0].from, links[1].from);
        for v in [Variant::Nested, Variant::Flat, Variant::Csr] {
            let g = Graph::build(&f, v);
            assert_eq!(g.nodes[device], before);
            assert_eq!(g.nodes, f.nodes);
            assert_eq!(
                g.nodes.iter().filter(|n| n.base.row_kind == VALUE).count(),
                25
            );
        }
    }
    #[test]
    fn exact_edges_and_metadata_all_variants() {
        let f = small();
        assert!(
            f.edges
                .windows(2)
                .any(|w| (w[0].flags, w[0].revision) != (w[1].flags, w[1].revision))
        );
        for v in [Variant::Nested, Variant::Flat, Variant::Csr] {
            assert_eq!(reconstructed(&Graph::build(&f, v)), f.edges);
        }
    }
    #[test]
    fn reverse_matches_forward_all_variants() {
        let f = small();
        for v in [Variant::Nested, Variant::Flat, Variant::Csr] {
            let g = Graph::build(&f, v);
            for (i, n) in f.nodes.iter().enumerate() {
                let expected = f
                    .edges
                    .iter()
                    .filter(|e| e.to == n.base.id)
                    .fold(0_u64, |s, e| s.wrapping_add(mix(e.from)));
                assert_eq!(g.reverse_sum(slot(i)), expected);
            }
        }
    }
    #[test]
    fn replicated_identities_unique_and_parent_links_valid() {
        let f = fixture(7, 19, 5, 4).unwrap();
        assert_eq!(f.nodes.len(), (39 + 7 + 19 + 5) * 4);
        assert_eq!(
            f.nodes
                .iter()
                .map(|n| n.base.id)
                .collect::<HashSet<_>>()
                .len(),
            f.nodes.len()
        );
        validate(&f).unwrap();
    }
    #[test]
    fn contracts_attach_to_family_or_house() {
        let f = small();
        for n in f.nodes.iter().filter(|n| n.base.row_kind == ACTION) {
            assert!(matches!(
                f.nodes
                    .iter()
                    .find(|p| p.base.id == n.base.parent)
                    .unwrap()
                    .base
                    .row_kind,
                FAMILY | HOUSE
            ));
        }
    }
    #[test]
    fn empty_containers_return_no_members() {
        let f = small();
        for v in [Variant::Nested, Variant::Flat, Variant::Csr] {
            let g = Graph::build(&f, v);
            assert_eq!(g.zone_sum(0), 0);
            assert_eq!(g.reverse_sum(0), 0);
        }
    }
    #[test]
    fn zero_devices_and_zero_scale_rejected() {
        assert!(fixture(0, 0, 0, 1).is_err());
        assert!(fixture(1, 1, 1, 0).is_err());
    }
    #[test]
    fn no_values_or_actions_is_valid() {
        let f = fixture(1, 0, 0, 1).unwrap();
        assert_eq!(f.nodes.len(), 40);
        validate(&f).unwrap();
    }
    #[test]
    fn count_overflow_rejected() {
        assert!(fixture(usize::MAX, 1, 1, 1).is_err());
        assert!(fixture(1, 1, 1, usize::MAX).is_err());
    }
    #[test]
    fn invalid_parent_and_membership_kind_rejected() {
        let mut f = small();
        f.nodes[7].base.parent = 1;
        assert_eq!(validate(&f), Err("invalid physical parent"));
        let mut f = small();
        f.edges[0].to = 1;
        assert_eq!(validate(&f), Err("invalid membership kinds"));
    }
    #[test]
    fn query_checksums_identical() {
        let f = small();
        let queries = random_queries(&f.zones, 1000);
        let sums: Vec<_> = [Variant::Nested, Variant::Flat, Variant::Csr]
            .iter()
            .map(|v| {
                let g = Graph::build(&f, *v);
                queries
                    .iter()
                    .fold(0_u64, |s, z| s.wrapping_add(g.zone_sum(*z)))
            })
            .collect();
        assert_eq!(sums[0], sums[1]);
        assert_eq!(sums[1], sums[2]);
    }
}
