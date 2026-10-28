//! Isolated, std-only catalogue layout experiment; not the production catalogue.
//! rustc --edition=2021 -C opt-level=3 main.rs -o /tmp/catalogue-banc
//! rustc --edition=2021 --test main.rs -o /tmp/catalogue-banc-tests
//! See --help for the input contract. No device or network access.
use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::HashMap;
use std::hint::black_box;
#[cfg(test)]
use std::mem::align_of;
use std::mem::size_of;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

struct CountingAllocator;
static LIVE: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe {
            System.dealloc(pointer, layout);
        }
    }
    unsafe fn realloc(&self, pointer: *mut u8, old: Layout, new_size: usize) -> *mut u8 {
        let result = unsafe { System.realloc(pointer, old, new_size) };
        if !result.is_null() {
            if new_size >= old.size() {
                LIVE.fetch_add(new_size - old.size(), Ordering::Relaxed);
            } else {
                LIVE.fetch_sub(old.size() - new_size, Ordering::Relaxed);
            }
        }
        result
    }
}
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;
const READ: u16 = 1;
const HAS_SET: u16 = 2;
const NUMERIC: u16 = 4;
const BOUNDS_PRESENT: u16 = 8;
const NONE: u32 = u32::MAX;
const ROUNDS: usize = 7;
const OPS: usize = 1_048_576;

// Local installation namespace only. A global identity also needs an installation
// identifier (e.g. 128 bits). Durable high-water storage is outside this benchmark.
struct LocalIdAllocator {
    next: u64,
}
impl LocalIdAllocator {
    fn new() -> Self {
        Self { next: 1 }
    }
    fn issue(&mut self) -> Result<u64, String> {
        let following = self.next.checked_add(1).ok_or("local ID space exhausted")?;
        if self.next == 0 {
            return Err("zero is not an ID".into());
        }
        let id = self.next;
        self.next = following;
        Ok(id)
    }
    fn restore_high_water(&mut self, last_issued: u64) -> Result<(), String> {
        let next = last_issued
            .checked_add(1)
            .ok_or("local ID space exhausted")?;
        if next < self.next {
            return Err("ID high-water mark cannot decrease".into());
        }
        self.next = next;
        Ok(())
    }
}

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
struct Row96 {
    base: Row64,
    numeric: Numeric32,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Hot32 {
    payload: u64,
    observed_ms: u64,
    received_ms: u64,
    changed_ms: u64,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Cold32 {
    id: u64,
    parent: u64,
    schema: u32,
    aux: u32,
    revision: u32,
    flags: u16,
    row_kind: u8,
    value_kind: u8,
}

fn parse(input: &str) -> Result<Vec<Row96>, String> {
    let mut rows = Vec::new();
    let mut ids = LocalIdAllocator::new();
    ids.restore_high_water(0)?;
    for (line, text) in input.lines().enumerate() {
        if text.trim().is_empty() || text.trim_start().starts_with('#') {
            continue;
        }
        let columns: Vec<&str> = text.split('\t').collect();
        if columns.len() != 12 {
            return Err(format!(
                "line {}: expected 12 tab-separated integers",
                line + 1
            ));
        }
        let mut fields = [0_u64; 12];
        for (out, column) in fields.iter_mut().zip(columns) {
            *out = column
                .parse()
                .map_err(|_| format!("line {}: invalid unsigned integer", line + 1))?;
        }
        let [
            kind,
            parent,
            value_kind,
            flags,
            payload,
            observed_ms,
            received_ms,
            changed_ms,
            min,
            max,
            step,
            anchor,
        ] = fields;
        if kind > 2 || value_kind > 5 || flags > u64::from(u16::MAX) || parent > u64::from(NONE) {
            return Err(format!("line {}: field out of range", line + 1));
        }
        let id = ids.issue()?;
        let row = Row96 {
            base: Row64 {
                id,
                parent: if parent == u64::from(NONE) {
                    0
                } else {
                    parent + 1
                },
                payload,
                observed_ms,
                received_ms,
                changed_ms,
                schema: 0,
                aux: NONE,
                revision: 0,
                flags: flags as u16,
                row_kind: kind as u8,
                value_kind: value_kind as u8,
            },
            numeric: Numeric32 {
                min,
                max,
                step,
                anchor,
            },
        };
        validate_row(&row)?;
        // Parent before child gives an acyclic forest; records may be parents too.
        if row.base.parent != 0 {
            let parent_slot =
                usize::try_from(row.base.parent - 1).map_err(|_| "parent overflow")?;
            let parent_row: &Row96 = rows.get(parent_slot).ok_or("unknown or forward parent")?;
            if parent_row.base.row_kind == 2 {
                return Err("an action cannot be a parent".into());
            }
        }
        rows.push(row);
    }
    if rows.is_empty() {
        return Err("empty workload".into());
    }
    if rows.len() >= NONE as usize {
        return Err("too many rows for u32 slots".into());
    }
    Ok(rows)
}

fn validate_row(row: &Row96) -> Result<(), String> {
    let b = row.base;
    if b.row_kind == 0
        && (!matches!(b.value_kind, 0..=2)
            || (b.value_kind == 0 && b.payload != 0)
            || b.flags & (HAS_SET | NUMERIC) != 0)
    {
        return Err(
            "device health must be unknown, bool or integer, without has_set or numeric metadata"
                .into(),
        );
    }
    if b.row_kind == 2
        && (b.value_kind != 0
            || b.payload != 0
            || b.observed_ms != 0
            || b.received_ms != 0
            || b.changed_ms != 0
            || b.flags & (READ | HAS_SET | NUMERIC) != 0)
    {
        return Err("action must not have value state or has_set".into());
    }
    if b.flags & NUMERIC == 0 && row.numeric != Numeric32::default() {
        return Err("numeric metadata on nonnumeric row would be lost".into());
    }
    if b.flags & BOUNDS_PRESENT != 0 && b.flags & NUMERIC == 0 {
        return Err("bounds require numeric metadata".into());
    }
    if b.flags & NUMERIC != 0 && !matches!(b.value_kind, 0 | 2 | 3) {
        return Err("numeric metadata requires unknown, integer or float value".into());
    }
    Ok(())
}

fn replicate(input: &[Row96], scale: usize) -> Result<Vec<Row96>, String> {
    let count = input.len().checked_mul(scale).ok_or("scale overflow")?;
    if scale == 0 || count >= NONE as usize {
        return Err("scale must fit nonzero u32 slots".into());
    }
    let mut rows = Vec::with_capacity(count);
    for world in 0..scale {
        let offset = (world * input.len()) as u64;
        for row in input {
            let mut row = *row;
            row.base.id = row.base.id.checked_add(offset).ok_or("ID overflow")?;
            if row.base.parent != 0 {
                row.base.parent = row
                    .base
                    .parent
                    .checked_add(offset)
                    .ok_or("parent overflow")?;
            }
            rows.push(row);
        }
    }
    Ok(rows)
}

struct Common {
    by_id: HashMap<u64, u32>,
    offsets: Vec<u32>,
    children: Vec<u32>,
    index_bytes: usize,
}
impl Common {
    fn new(rows: &[Row96]) -> Result<Self, String> {
        let before = LIVE.load(Ordering::Relaxed);
        let mut by_id = HashMap::with_capacity(rows.len());
        for (slot, row) in rows.iter().enumerate() {
            if row.base.id == 0 || by_id.insert(row.base.id, slot as u32).is_some() {
                return Err("duplicate or zero ID".into());
            }
        }
        // The benchmark executable is single-threaded. Unit tests may run in
        // parallel and their unrelated frees can lower the process-wide counter.
        let index_bytes = LIVE.load(Ordering::Relaxed).saturating_sub(before);
        let mut offsets = vec![0_u32; rows.len() + 1];
        for row in rows {
            if row.base.parent != 0 {
                let parent = *by_id.get(&row.base.parent).ok_or("unknown parent ID")? as usize;
                if rows[parent].base.row_kind == 2 {
                    return Err("action parent".into());
                }
                offsets[parent + 1] += 1;
            }
        }
        for slot in 1..offsets.len() {
            offsets[slot] += offsets[slot - 1];
        }
        let mut children = vec![0_u32; *offsets.last().unwrap() as usize];
        let mut cursor = offsets[..rows.len()].to_vec();
        for (slot, row) in rows.iter().enumerate() {
            if row.base.parent != 0 {
                let parent = by_id[&row.base.parent] as usize;
                children[cursor[parent] as usize] = slot as u32;
                cursor[parent] += 1;
            }
        }
        Ok(Self {
            by_id,
            offsets,
            children,
            index_bytes,
        })
    }
    fn adjacency_bytes(&self) -> usize {
        (self.offsets.capacity() + self.children.capacity()) * size_of::<u32>()
    }
}

trait Store: Sized {
    const NAME: &'static str;
    fn build(rows: &[Row96]) -> Result<Self, String>;
    fn common(&self) -> &Common;
    fn len(&self) -> usize;
    fn scan(&self, slot: usize) -> (u64, u16);
    fn scalar(&self, slot: usize) -> Hot32;
    fn numeric(&self, slot: usize) -> Numeric32;
    fn decode(&self, slot: usize) -> Row96;
    fn update(&mut self, slot: usize, payload: u64, received: u64, observation: Option<u64>);
    fn fixed_bytes(&self) -> usize;
    fn numeric_bytes(&self) -> usize;
    fn snapshot(&self) -> usize;
}

fn update_hot(
    hot: &mut Hot32,
    value_kind: u8,
    payload: u64,
    received: u64,
    observation: Option<u64>,
) {
    // Float equality follows Rust's f64 PartialEq: signed zeros are equal and
    // NaNs are unequal. Incoming payload bits are retained even when equal.
    // Text/record payloads must be immutable, versioned handles: equal handles
    // mean equal content. This experiment does not implement a mutable arena.
    let changed = if value_kind == 3 {
        f64::from_bits(hot.payload) != f64::from_bits(payload)
    } else {
        hot.payload != payload
    };
    if changed {
        hot.changed_ms = received;
    }
    hot.payload = payload;
    hot.received_ms = received;
    if let Some(observed) = observation {
        hot.observed_ms = observed;
    }
}
fn hot(base: Row64) -> Hot32 {
    Hot32 {
        payload: base.payload,
        observed_ms: base.observed_ms,
        received_ms: base.received_ms,
        changed_ms: base.changed_ms,
    }
}
fn update_base(base: &mut Row64, payload: u64, received: u64, observation: Option<u64>) {
    let mut value = hot(*base);
    update_hot(&mut value, base.value_kind, payload, received, observation);
    base.payload = value.payload;
    base.observed_ms = value.observed_ms;
    base.received_ms = value.received_ms;
    base.changed_ms = value.changed_ms;
}

struct Uniform96 {
    rows: Vec<Row96>,
    common: Common,
}
impl Store for Uniform96 {
    const NAME: &'static str = "uniform96";
    fn build(rows: &[Row96]) -> Result<Self, String> {
        Ok(Self {
            rows: rows.to_vec(),
            common: Common::new(rows)?,
        })
    }
    fn common(&self) -> &Common {
        &self.common
    }
    fn len(&self) -> usize {
        self.rows.len()
    }
    fn scan(&self, slot: usize) -> (u64, u16) {
        let b = self.rows[slot].base;
        (b.payload, b.flags)
    }
    fn scalar(&self, slot: usize) -> Hot32 {
        hot(self.rows[slot].base)
    }
    fn numeric(&self, slot: usize) -> Numeric32 {
        self.rows[slot].numeric
    }
    fn decode(&self, slot: usize) -> Row96 {
        self.rows[slot]
    }
    fn update(&mut self, slot: usize, payload: u64, received: u64, observation: Option<u64>) {
        update_base(&mut self.rows[slot].base, payload, received, observation);
    }
    fn fixed_bytes(&self) -> usize {
        self.rows.capacity() * size_of::<Row96>()
    }
    fn numeric_bytes(&self) -> usize {
        0
    }
    fn snapshot(&self) -> usize {
        let copy = black_box(self.rows.clone());
        black_box(&copy);
        copy.len() * size_of::<Row96>()
    }
}

struct Uniform64 {
    rows: Vec<Row64>,
    numbers: Vec<Numeric32>,
    common: Common,
}
impl Store for Uniform64 {
    const NAME: &'static str = "uniform64";
    fn build(input: &[Row96]) -> Result<Self, String> {
        let mut rows = Vec::with_capacity(input.len());
        let mut numbers =
            Vec::with_capacity(input.iter().filter(|r| r.base.flags & NUMERIC != 0).count());
        for row in input {
            let mut base = row.base;
            if base.flags & NUMERIC != 0 {
                base.aux = numbers.len() as u32;
                numbers.push(row.numeric);
            }
            rows.push(base);
        }
        Ok(Self {
            rows,
            numbers,
            common: Common::new(input)?,
        })
    }
    fn common(&self) -> &Common {
        &self.common
    }
    fn len(&self) -> usize {
        self.rows.len()
    }
    fn scan(&self, slot: usize) -> (u64, u16) {
        let b = self.rows[slot];
        (b.payload, b.flags)
    }
    fn scalar(&self, slot: usize) -> Hot32 {
        hot(self.rows[slot])
    }
    fn numeric(&self, slot: usize) -> Numeric32 {
        let b = self.rows[slot];
        if b.flags & NUMERIC == 0 {
            Numeric32::default()
        } else {
            self.numbers[b.aux as usize]
        }
    }
    fn decode(&self, slot: usize) -> Row96 {
        let mut base = self.rows[slot];
        base.aux = NONE;
        Row96 {
            base,
            numeric: self.numeric(slot),
        }
    }
    fn update(&mut self, slot: usize, payload: u64, received: u64, observation: Option<u64>) {
        update_base(&mut self.rows[slot], payload, received, observation);
    }
    fn fixed_bytes(&self) -> usize {
        self.rows.capacity() * size_of::<Row64>()
    }
    fn numeric_bytes(&self) -> usize {
        self.numbers.capacity() * size_of::<Numeric32>()
    }
    fn snapshot(&self) -> usize {
        let copy = black_box((self.rows.clone(), self.numbers.clone()));
        black_box(&copy);
        copy.0.len() * size_of::<Row64>() + copy.1.len() * size_of::<Numeric32>()
    }
}

struct Split {
    hot: Vec<Hot32>,
    cold: Vec<Cold32>,
    numbers: Vec<Numeric32>,
    common: Common,
}
impl Store for Split {
    const NAME: &'static str = "split";
    fn build(input: &[Row96]) -> Result<Self, String> {
        let mut hot_rows = Vec::with_capacity(input.len());
        let mut cold = Vec::with_capacity(input.len());
        let mut numbers =
            Vec::with_capacity(input.iter().filter(|r| r.base.flags & NUMERIC != 0).count());
        for row in input {
            let b = row.base;
            let aux = if b.flags & NUMERIC != 0 {
                let n = numbers.len() as u32;
                numbers.push(row.numeric);
                n
            } else {
                NONE
            };
            hot_rows.push(hot(b));
            cold.push(Cold32 {
                id: b.id,
                parent: b.parent,
                schema: b.schema,
                aux,
                revision: b.revision,
                flags: b.flags,
                row_kind: b.row_kind,
                value_kind: b.value_kind,
            });
        }
        Ok(Self {
            hot: hot_rows,
            cold,
            numbers,
            common: Common::new(input)?,
        })
    }
    fn common(&self) -> &Common {
        &self.common
    }
    fn len(&self) -> usize {
        self.hot.len()
    }
    fn scan(&self, slot: usize) -> (u64, u16) {
        (self.hot[slot].payload, self.cold[slot].flags)
    }
    fn scalar(&self, slot: usize) -> Hot32 {
        self.hot[slot]
    }
    fn numeric(&self, slot: usize) -> Numeric32 {
        let c = self.cold[slot];
        if c.flags & NUMERIC == 0 {
            Numeric32::default()
        } else {
            self.numbers[c.aux as usize]
        }
    }
    fn decode(&self, slot: usize) -> Row96 {
        let h = self.hot[slot];
        let c = self.cold[slot];
        Row96 {
            base: Row64 {
                id: c.id,
                parent: c.parent,
                payload: h.payload,
                observed_ms: h.observed_ms,
                received_ms: h.received_ms,
                changed_ms: h.changed_ms,
                schema: c.schema,
                aux: NONE,
                revision: c.revision,
                flags: c.flags,
                row_kind: c.row_kind,
                value_kind: c.value_kind,
            },
            numeric: self.numeric(slot),
        }
    }
    fn update(&mut self, slot: usize, payload: u64, received: u64, observation: Option<u64>) {
        update_hot(
            &mut self.hot[slot],
            self.cold[slot].value_kind,
            payload,
            received,
            observation,
        );
    }
    fn fixed_bytes(&self) -> usize {
        self.hot.capacity() * size_of::<Hot32>() + self.cold.capacity() * size_of::<Cold32>()
    }
    fn numeric_bytes(&self) -> usize {
        self.numbers.capacity() * size_of::<Numeric32>()
    }
    fn snapshot(&self) -> usize {
        let copy = black_box((self.hot.clone(), self.cold.clone(), self.numbers.clone()));
        black_box(&copy);
        (copy.0.len() + copy.1.len()) * 32 + copy.2.len() * size_of::<Numeric32>()
    }
}

fn fold(hash: u64, value: u64) -> u64 {
    (hash ^ value).wrapping_mul(0x100000001b3)
}
fn row_hash(mut hash: u64, row: Row96) -> u64 {
    let b = row.base;
    for value in [
        b.id,
        b.parent,
        b.payload,
        b.observed_ms,
        b.received_ms,
        b.changed_ms,
        u64::from(b.schema),
        u64::from(b.revision),
        u64::from(b.flags),
        u64::from(b.row_kind),
        u64::from(b.value_kind),
        row.numeric.min,
        row.numeric.max,
        row.numeric.step,
        row.numeric.anchor,
    ] {
        hash = fold(hash, value);
    }
    hash
}
fn checksum(rows: impl Iterator<Item = Row96>) -> u64 {
    rows.fold(0xcbf29ce484222325, row_hash)
}

fn random_slots(eligible: &[usize]) -> Vec<usize> {
    if eligible.is_empty() {
        return Vec::new();
    }
    let mut seed = 0x71321a66421d9c15_u64;
    (0..OPS)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            eligible[(seed as usize) % eligible.len()]
        })
        .collect()
}
fn measure(mut body: impl FnMut() -> u64, operations: usize) -> f64 {
    if operations == 0 {
        return 0.0;
    }
    black_box(body()); // Untimed warmup, including page faults and instruction cache.
    let mut times = [0.0_f64; ROUNDS];
    for time in &mut times {
        let start = Instant::now();
        black_box(body());
        *time = start.elapsed().as_nanos() as f64 / operations as f64;
    }
    times.sort_by(f64::total_cmp);
    times[ROUNDS / 2]
}
fn rss() -> (Option<u64>, Option<u64>) {
    let Ok(text) = std::fs::read_to_string("/proc/self/status") else {
        return (None, None);
    };
    let read = |prefix: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(prefix))
            .and_then(|value| value.split_whitespace().next())
            .and_then(|v| v.parse::<u64>().ok())
            .map(|v| v * 1024)
    };
    (read("VmRSS:"), read("RssAnon:"))
}
fn json_option(value: Option<u64>) -> String {
    value.map_or_else(|| "null".into(), |v| v.to_string())
}
fn json_delta(before: Option<u64>, after: Option<u64>) -> String {
    before.zip(after).map_or_else(
        || "null".into(),
        |(b, a)| (i128::from(a) - i128::from(b)).to_string(),
    )
}

fn bench<S: Store>(input: &[Row96], scale: usize, arena_per_world: usize) -> Result<(), String> {
    let expected = checksum(input.iter().copied());
    let rss_before = rss();
    let baseline = LIVE.load(Ordering::Relaxed);
    let mut store = S::build(input)?;
    let arena_size = arena_per_world
        .checked_mul(scale)
        .ok_or("arena scale overflow")?;
    // A synthetic, fully touched byte pool with the measured export's byte count.
    // This charges retained variable-data bytes without claiming a schema engine.
    let payload_pool = vec![0x5a_u8; arena_size];
    black_box(&payload_pool);
    let allocated = LIVE.load(Ordering::Relaxed) - baseline;
    let rss_after = rss();
    let actual = checksum((0..store.len()).map(|slot| store.decode(slot)));
    if actual != expected {
        return Err("canonical checksum mismatch".into());
    }
    for (slot, row) in input.iter().enumerate() {
        if store.decode(slot) != *row {
            return Err("exact decoded row mismatch".into());
        }
    }
    let n = store.len();
    let all: Vec<usize> = (0..n).collect();
    let scalar: Vec<usize> = input
        .iter()
        .enumerate()
        .filter(|(_, r)| r.base.row_kind != 2 && matches!(r.base.value_kind, 1..=3))
        .map(|(slot, _)| slot)
        .collect();
    let numeric: Vec<usize> = input
        .iter()
        .enumerate()
        .filter(|(_, r)| r.base.flags & NUMERIC != 0)
        .map(|(slot, _)| slot)
        .collect();
    let all_slots = random_slots(&all);
    let scalar_slots = random_slots(&scalar);
    let numeric_slots = random_slots(&numeric);
    let scan_repeats = OPS.div_ceil(n);
    let scan_ns = measure(
        || {
            let mut sum = 0_u64;
            for _ in 0..scan_repeats {
                for slot in 0..n {
                    let (payload, flags) = black_box(&store).scan(slot);
                    sum = sum.wrapping_add(payload ^ u64::from(flags));
                }
            }
            sum
        },
        scan_repeats * n,
    );
    let scalar_ns = measure(
        || {
            let mut sum = 0_u64;
            for &slot in &scalar_slots {
                let h = black_box(&store).scalar(black_box(slot));
                sum = sum.wrapping_add(h.payload ^ h.observed_ms ^ h.received_ms ^ h.changed_ms);
            }
            sum
        },
        scalar_slots.len(),
    );
    let numeric_ns = measure(
        || {
            let mut sum = 0_u64;
            for &slot in &numeric_slots {
                let s = black_box(&store);
                let h = s.scalar(black_box(slot));
                let q = s.numeric(slot);
                let (_, flags) = s.scan(slot);
                sum = sum.wrapping_add(
                    h.payload
                        ^ h.observed_ms
                        ^ h.received_ms
                        ^ h.changed_ms
                        ^ q.min
                        ^ q.max
                        ^ q.step
                        ^ q.anchor
                        ^ u64::from(flags),
                );
            }
            sum
        },
        numeric_slots.len(),
    );
    let lookup_ns = measure(
        || {
            let mut sum = 0_u64;
            for &slot in &all_slots {
                let id = black_box(input[slot].base.id);
                sum = sum.wrapping_add(u64::from(
                    *black_box(&store).common().by_id.get(&id).unwrap(),
                ));
            }
            sum
        },
        all_slots.len(),
    );
    let adjacency_ns = measure(
        || {
            let mut sum = 0_u64;
            for &slot in &all_slots {
                let c = black_box(&store).common();
                for &child in &c.children[c.offsets[slot] as usize..c.offsets[slot + 1] as usize] {
                    sum = sum.wrapping_add(u64::from(child));
                }
            }
            sum
        },
        all_slots.len(),
    );
    let equal_ns = measure(
        || {
            for (i, &slot) in scalar_slots.iter().enumerate() {
                let payload = input[slot].base.payload;
                black_box(&mut store).update(
                    black_box(slot),
                    black_box(payload),
                    black_box(2_000_000 + i as u64),
                    None,
                );
            }
            store
                .scalar(scalar_slots.first().copied().unwrap_or(0))
                .received_ms
        },
        scalar_slots.len(),
    );
    // Restore before the change benchmark; each visit toggles one bit, guaranteeing change.
    for &slot in &scalar {
        let h = input[slot].base;
        store.update(slot, h.payload, h.received_ms, Some(h.observed_ms));
    }
    let change_ns = measure(
        || {
            for (i, &slot) in scalar_slots.iter().enumerate() {
                let payload = store.scalar(slot).payload ^ 1;
                black_box(&mut store).update(
                    black_box(slot),
                    black_box(payload),
                    black_box(4_000_000 + i as u64),
                    None,
                );
            }
            store
                .scalar(scalar_slots.first().copied().unwrap_or(0))
                .changed_ms
        },
        scalar_slots.len(),
    );
    // Snapshot includes row arrays + numeric arrays, excludes the unchanged common indexes.
    let snapshot_repeats = (OPS / n).clamp(1, 128);
    let snapshot_ns = measure(
        || {
            let mut bytes = 0_u64;
            for _ in 0..snapshot_repeats {
                bytes = bytes.wrapping_add(black_box(&store).snapshot() as u64);
            }
            bytes
        },
        snapshot_repeats,
    );
    let fixed = store.fixed_bytes();
    let num = store.numeric_bytes();
    let index = store.common().index_bytes;
    let adjacency = store.common().adjacency_bytes();
    if fixed + num + index + adjacency + payload_pool.capacity() != allocated {
        return Err("allocation breakdown does not match live heap delta".into());
    }
    println!(
        "{{\"layout\":\"{}\",\"scale\":{},\"n\":{},\"numeric_rows\":{},\"checksum\":\"{:016x}\",\"parity\":true,\"rounds\":{},\"random_ops_per_round\":{},\"allocations_requested_bytes\":{},\"fixed_bytes\":{},\"numeric_bytes\":{},\"index_bytes\":{},\"adjacency_bytes\":{},\"payload_pool_bytes\":{},\"rss_before_bytes\":{},\"rss_after_bytes\":{},\"rss_delta_bytes\":{},\"rss_anon_before_bytes\":{},\"rss_anon_after_bytes\":{},\"rss_anon_delta_bytes\":{},\"scan_ns_per_row\":{:.4},\"scalar_ns_per_op\":{:.4},\"numeric_ns_per_op\":{:.4},\"lookup_ns_per_op\":{:.4},\"adjacency_ns_per_parent\":{:.4},\"equal_update_ns_per_op\":{:.4},\"changed_update_ns_per_op\":{:.4},\"snapshot_ns_per_copy\":{:.4},\"snapshot_bytes\":{},\"numeric_timing_available\":{},\"scalar_timing_available\":{}}}",
        S::NAME,
        scale,
        n,
        numeric.len(),
        actual,
        ROUNDS,
        OPS,
        allocated,
        fixed,
        num,
        index,
        adjacency,
        payload_pool.capacity(),
        json_option(rss_before.0),
        json_option(rss_after.0),
        json_delta(rss_before.0, rss_after.0),
        json_option(rss_before.1),
        json_option(rss_after.1),
        json_delta(rss_before.1, rss_after.1),
        scan_ns,
        scalar_ns,
        numeric_ns,
        lookup_ns,
        adjacency_ns,
        equal_ns,
        change_ns,
        snapshot_ns,
        fixed + num,
        !numeric.is_empty(),
        !scalar.is_empty()
    );
    Ok(())
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 2 && args[1] == "--help" {
        println!(
            "--bench INPUT.tsv uniform96|uniform64|split SCALE [ARENA_BYTES_PER_WORLD]\n\
            Input: 12 tab-separated unsigned decimal integers per row; # comments ignored.\n\
            kind(0=device,1=value,2=action), parent zero-based slot(u32::MAX=none),\n\
            value_kind(0=null,1=bool,2=i64,3=f64,4=text,5=record), flags(READ=1,HAS_SET=2,NUMERIC=4,BOUNDS=8),\n\
            payload bits/ref, observed_ms, received_ms, changed_ms, min_bits, max_bits, step_bits, anchor_bits.\n\
            IDs are global sequential u64, schema=0 shared stub. Zero observed_ms means unknown.\n\
            All 12 input fields are preserved exactly; optional synthetic byte pool charges variable payload/schema storage.\n\
            Heap is requested LIVE allocation bytes, including hash buckets; excludes allocator overhead.\n\
            RSS is optional absolute /proc data; delta includes page granularity and allocator retention.\n\
            Each invocation measures one layout. Run each in a fresh process/container.\n\
            Random slots/input parsed before timing; heap baseline excludes input and random slot buffers.\n\
            Timings: median of 7 rounds after warmup. Snapshot allocates/copies/drops rows+numeric, excludes indexes/pool.\n\
            No deletion, schema interpreter, I/O, validation, locks, journaling, or production API measured."
        );
        return Ok(());
    }
    if !(args.len() == 5 || args.len() == 6) || args[1] != "--bench" {
        return Err(
            "usage: --bench INPUT.tsv uniform96|uniform64|split SCALE [ARENA_BYTES_PER_WORLD]"
                .into(),
        );
    }
    let source = std::fs::read_to_string(&args[2]).map_err(|e| format!("input: {e}"))?;
    let parsed = parse(&source)?;
    let scale = args[4].parse::<usize>().map_err(|_| "invalid scale")?;
    let arena = args
        .get(5)
        .map_or(Ok(0), |v| v.parse::<usize>())
        .map_err(|_| "invalid arena size")?;
    let rows = replicate(&parsed, scale)?;
    match args[3].as_str() {
        "uniform96" => bench::<Uniform96>(&rows, scale, arena),
        "uniform64" => bench::<Uniform64>(&rows, scale, arena),
        "split" => bench::<Split>(&rows, scale, arena),
        _ => Err("unknown layout".into()),
    }
}
fn main() {
    if let Err(error) = run() {
        eprintln!("catalogue-banc: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> Vec<Row96> {
        parse("0\t4294967295\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\n\
            1\t0\t2\t15\t18446744073709551609\t0\t20\t10\t0\t4636737291354636288\t4607182418800017408\t0\n\
            1\t0\t5\t1\t9\t0\t20\t10\t0\t0\t0\t0\n\
            1\t2\t1\t1\t1\t0\t20\t10\t0\t0\t0\t0\n\
            2\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0").unwrap()
    }
    #[test]
    fn exact_sizes_alignment() {
        assert_eq!(
            (
                size_of::<Row96>(),
                size_of::<Row64>(),
                size_of::<Hot32>(),
                size_of::<Cold32>(),
                size_of::<Numeric32>()
            ),
            (96, 64, 32, 32, 32)
        );
        assert_eq!(
            (
                align_of::<Row96>(),
                align_of::<Row64>(),
                align_of::<Cold32>()
            ),
            (8, 8, 8)
        );
    }
    #[test]
    fn duplicate_index_ids_rejected() {
        let mut r = sample();
        r[1].base.id = r[0].base.id;
        assert!(Common::new(&r).is_err());
    }
    #[test]
    fn unknown_parent_rejected() {
        let mut r = sample();
        r[1].base.parent = 999;
        assert!(Common::new(&r).is_err());
    }
    #[test]
    fn record_parent_is_valid_and_csr_exact() {
        let r = sample();
        let c = Common::new(&r).unwrap();
        assert_eq!(
            &c.children[c.offsets[2] as usize..c.offsets[3] as usize],
            &[3]
        );
        assert_eq!(
            &c.children[c.offsets[0] as usize..c.offsets[1] as usize],
            &[1, 2, 4]
        );
    }
    #[test]
    fn replicated_ids_parents_are_disjoint() {
        let r = sample();
        let copies = replicate(&r, 10).unwrap();
        let c = Common::new(&copies).unwrap();
        assert_eq!(c.by_id.len(), 50);
        for world in 0..10 {
            assert_eq!(copies[world * 5 + 3].base.parent, (world * 5 + 3) as u64);
        }
        assert!(replicate(&r, 0).is_err());
    }
    fn timestamps<S: Store>() {
        let r = sample();
        let mut s = S::build(&r).unwrap();
        s.update(1, r[1].base.payload, 30, None);
        let b = s.decode(1).base;
        assert_eq!((b.observed_ms, b.received_ms, b.changed_ms), (0, 30, 10));
        s.update(1, 123, 40, Some(35));
        let b = s.decode(1).base;
        assert_eq!((b.observed_ms, b.received_ms, b.changed_ms), (35, 40, 40));
        s.update(1, 123, 45, None);
        assert_eq!(s.decode(1).base.changed_ms, 40);
    }
    #[test]
    fn update_timestamp_semantics_all_layouts() {
        timestamps::<Uniform96>();
        timestamps::<Uniform64>();
        timestamps::<Split>();
    }
    #[test]
    fn readonly_retains_absent_has_set() {
        let r = sample();
        for row in [
            Uniform96::build(&r).unwrap().decode(3),
            Uniform64::build(&r).unwrap().decode(3),
            Split::build(&r).unwrap().decode(3),
        ] {
            assert_eq!(row.base.flags & READ, READ);
            assert_eq!(row.base.flags & HAS_SET, 0);
        }
    }
    #[test]
    fn action_has_no_fake_state_or_set() {
        let r = sample();
        assert_eq!(r[4].base.flags & (READ | HAS_SET), 0);
        let mut invalid = r[4];
        invalid.base.flags = HAS_SET;
        assert!(validate_row(&invalid).is_err());
        invalid = r[4];
        invalid.base.payload = 1;
        assert!(validate_row(&invalid).is_err());
    }
    #[test]
    fn exact_integer_and_float_payload_bits() {
        let mut r = sample();
        assert_eq!(r[1].base.payload as i64, -7);
        r[1].base.value_kind = 3;
        r[1].base.payload = 0x8000000000000000;
        r[1].numeric.anchor = 0x7ff8000000000042;
        for row in [
            Uniform96::build(&r).unwrap().decode(1),
            Uniform64::build(&r).unwrap().decode(1),
            Split::build(&r).unwrap().decode(1),
        ] {
            assert_eq!(row.base.payload, (-0.0_f64).to_bits());
            assert_eq!(row.numeric.anchor, 0x7ff8000000000042);
        }
    }
    #[test]
    fn external_metadata_and_full_decode_equal() {
        let r = replicate(&sample(), 3).unwrap();
        let a = Uniform96::build(&r).unwrap();
        let b = Uniform64::build(&r).unwrap();
        let c = Split::build(&r).unwrap();
        for (slot, row) in r.iter().enumerate() {
            assert_eq!(a.decode(slot), *row);
            assert_eq!(b.decode(slot), *row);
            assert_eq!(c.decode(slot), *row);
        }
        assert_eq!(b.numbers.len(), 3);
        assert_eq!(c.numbers.len(), 3);
        assert_eq!(
            checksum(r.iter().copied()),
            checksum((0..r.len()).map(|i| c.decode(i)))
        );
    }
    #[test]
    fn parser_rejects_invalid_and_forward_parents() {
        assert!(parse("1\t0\t1\t1\t1\t0\t1\t1\t0\t0\t0\t0").is_err());
        assert!(parse("0\t4294967295\t0\t0\t0\t0\t0\t0\t1\t0\t0\t0").is_err());
        assert!(parse("0\t4294967295\t0\t0\t0\t0\t0\t0\t0\t0\t0").is_err());
    }
    #[test]
    fn numeric_flag_without_bounds_keeps_quartet() {
        let mut r = sample();
        r[1].base.flags &= !BOUNDS_PRESENT;
        let a = Uniform64::build(&r).unwrap();
        assert_eq!(a.decode(1).numeric, r[1].numeric);
    }
    #[test]
    fn ids_are_independent_of_storage_slots() {
        let mut r = sample();
        for row in &mut r {
            row.base.id += 1000;
            if row.base.parent != 0 {
                row.base.parent += 1000;
            }
        }
        let c = Common::new(&r).unwrap();
        assert_eq!(c.by_id.get(&1004), Some(&3));
        assert_eq!(
            &c.children[c.offsets[2] as usize..c.offsets[3] as usize],
            &[3]
        );
    }
    #[test]
    fn local_id_high_water_prevents_reuse_after_removal() {
        let mut ids = LocalIdAllocator::new();
        let first = ids.issue().unwrap();
        let second = ids.issue().unwrap();
        let mut live = HashMap::from([(first, 0), (second, 1)]);
        live.remove(&second);
        let third = ids.issue().unwrap();
        assert!(third > second);
        assert!(ids.restore_high_water(first).is_err());
        let mut restored = LocalIdAllocator::new();
        restored.restore_high_water(third).unwrap();
        assert_eq!(restored.issue().unwrap(), third + 1);
    }
    #[test]
    fn local_id_exhaustion_and_zero_are_rejected() {
        assert!(LocalIdAllocator { next: u64::MAX }.issue().is_err());
        assert!(LocalIdAllocator { next: 0 }.issue().is_err());
        assert!(
            LocalIdAllocator::new()
                .restore_high_water(u64::MAX)
                .is_err()
        );
    }
    fn equipment_health<S: Store>() {
        let mut rows = sample();
        rows[0].base.value_kind = 1;
        rows[0].base.flags = READ;
        rows[0].base.payload = 1;
        rows[0].base.observed_ms = 5;
        rows[0].base.received_ms = 10;
        rows[0].base.changed_ms = 8;
        validate_row(&rows[0]).unwrap();
        let mut store = S::build(&rows).unwrap();
        store.update(0, 1, 20, None);
        assert_eq!(
            store.scalar(0),
            Hot32 {
                payload: 1,
                observed_ms: 5,
                received_ms: 20,
                changed_ms: 8
            }
        );
        store.update(0, 0, 30, Some(25));
        assert_eq!(
            store.scalar(0),
            Hot32 {
                payload: 0,
                observed_ms: 25,
                received_ms: 30,
                changed_ms: 30
            }
        );
        rows[0].base.value_kind = 2;
        rows[0].base.payload = 7;
        validate_row(&rows[0]).unwrap();
        rows[0].base.flags |= HAS_SET;
        assert!(validate_row(&rows[0]).is_err());
    }
    #[test]
    fn equipment_health_updates_all_layouts() {
        equipment_health::<Uniform96>();
        equipment_health::<Uniform64>();
        equipment_health::<Split>();
    }
    fn signed_zero<S: Store>() {
        let mut rows = sample();
        rows[1].base.value_kind = 3;
        rows[1].base.payload = (-0.0_f64).to_bits();
        let mut store = S::build(&rows).unwrap();
        store.update(1, 0.0_f64.to_bits(), 30, None);
        assert_eq!(
            store.scalar(1),
            Hot32 {
                payload: 0.0_f64.to_bits(),
                observed_ms: 0,
                received_ms: 30,
                changed_ms: 10
            }
        );
        store.update(1, (-0.0_f64).to_bits(), 40, None);
        assert_eq!(store.scalar(1).payload, (-0.0_f64).to_bits());
        assert_eq!(store.scalar(1).changed_ms, 10);
        store.update(1, 1.0_f64.to_bits(), 50, None);
        assert_eq!(store.scalar(1).changed_ms, 50);
    }
    #[test]
    fn signed_zero_is_equal_but_incoming_bits_retained_all_layouts() {
        signed_zero::<Uniform96>();
        signed_zero::<Uniform64>();
        signed_zero::<Split>();
    }
    fn immutable_handles<S: Store>() {
        for kind in [4, 5] {
            let mut rows = sample();
            rows[2].base.value_kind = kind;
            let mut store = S::build(&rows).unwrap();
            store.update(2, 9, 30, None);
            assert_eq!(store.scalar(2).changed_ms, 10);
            store.update(2, 10, 40, None);
            assert_eq!(store.scalar(2).changed_ms, 40);
            assert_eq!(store.scalar(2).payload, 10);
        }
    }
    #[test]
    fn immutable_text_record_handle_changes_advance_date_all_layouts() {
        immutable_handles::<Uniform96>();
        immutable_handles::<Uniform64>();
        immutable_handles::<Split>();
    }
}
