//! Isolated std-only pure alias benchmark. No production/device access.
use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::HashMap;
use std::hint::black_box;
use std::mem::size_of;
use std::ptr::NonNull;
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
const DELETED: u16 = 1 << 15;
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
        let [kind, parent, value_kind, flags, payload, observed_ms, received_ms, changed_ms, min, max, step, anchor] =
            fields;
        if kind > 2 || value_kind > 5 || flags >= u64::from(DELETED) || parent > u64::from(NONE) {
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

// IDs are separate from storage slots and from copied physical state.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct Prefix {
    alias_id: u64,
    owner_id: u64,
    source_id: u64,
}
#[repr(C)]
struct Handle {
    prefix: Prefix,
    slot: u32,
    generation: u32,
}
// Prior full-row alias envelope: reference only, no copied state. The unused
// bytes remain physically allocated so this baseline measures the 96-byte stride.
#[repr(C)]
struct RowAlias96 {
    link: Handle,
    reserved: [u8; 64],
}
#[repr(C)]
struct Mirror {
    handle: Handle,
    row: Row96,
}
#[repr(C)]
struct Pointer {
    prefix: Prefix,
    target: NonNull<Row96>,
}
#[repr(C)]
struct IdLookup {
    prefix: Prefix,
    reserved: u64,
}

// Box::into_raw transfers the frozen Box<[Row96]> to this private owner. Keeping
// the raw owner avoids reborrowing/retagging a Box while element pointers exist.
// It is never resized, moved out, or freed before all bindings are destroyed.
struct Frozen {
    pointer: NonNull<Row96>,
    len: usize,
}
impl Frozen {
    fn new(rows: &[Row96]) -> Self {
        let boxed = rows.to_vec().into_boxed_slice();
        let len = boxed.len();
        let pointer = NonNull::new(Box::into_raw(boxed).cast::<Row96>()).unwrap();
        Self { pointer, len }
    }
    fn as_slice(&self) -> &[Row96] {
        // SAFETY: allocation holds len initialized rows and outlives this borrow.
        // Mutations require exclusive access to the entire Bank/Owner.
        unsafe { std::slice::from_raw_parts(self.pointer.as_ptr(), self.len) }
    }
    fn get_mut(&mut self, slot: usize) -> &mut Row96 {
        assert!(slot < self.len);
        // SAFETY: bound checked; &mut Owner excludes every Bank::read reference.
        unsafe { &mut *self.pointer.as_ptr().add(slot) }
    }
}
impl Drop for Frozen {
    fn drop(&mut self) {
        // SAFETY: exactly the Box allocation transferred in new, reconstructed
        // once with its original length. All bindings have already been dropped.
        unsafe {
            drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                self.pointer.as_ptr(),
                self.len,
            )));
        }
    }
}
enum Owner {
    Moving(Vec<Row96>),
    Frozen(Frozen),
}
impl Owner {
    fn new(rows: &[Row96], frozen: bool) -> Self {
        if frozen {
            Self::Frozen(Frozen::new(rows))
        } else {
            Self::Moving(rows.to_vec())
        }
    }
    fn rows(&self) -> &[Row96] {
        match self {
            Self::Moving(r) => r,
            Self::Frozen(r) => r.as_slice(),
        }
    }
    fn get_mut(&mut self, slot: usize) -> &mut Row96 {
        match self {
            Self::Moving(r) => &mut r[slot],
            Self::Frozen(r) => r.get_mut(slot),
        }
    }
    fn pointer(&self, slot: usize) -> NonNull<Row96> {
        assert!(slot < self.rows().len());
        match self {
            Self::Frozen(r) => {
                // SAFETY: checked within the frozen allocation; no dereference.
                unsafe { NonNull::new_unchecked(r.pointer.as_ptr().add(slot)) }
            }
            Self::Moving(_) => panic!("only frozen owners may supply pointers"),
        }
    }
    fn bytes(&self) -> usize {
        match self {
            Self::Moving(r) => r.capacity() * size_of::<Row96>(),
            Self::Frozen(r) => r.len * size_of::<Row96>(),
        }
    }
}
trait Binding: Sized {
    const NAME: &'static str;
    const FROZEN: bool = false;
    fn new(prefix: Prefix, slot: u32, owner: &Owner) -> Self;
    fn prefix(&self) -> Prefix;
    fn read<'a>(
        &'a self,
        owner: &'a Owner,
        generations: &[u32],
        index: &HashMap<u64, u64>,
    ) -> Option<&'a Row96>;
    fn refresh(&mut self, _row: Row96) {}
}
fn live(row: &Row96, expected: u64) -> bool {
    row.base.id == expected && row.base.flags & DELETED == 0
}
impl Binding for Handle {
    const NAME: &'static str = "handle";
    fn new(prefix: Prefix, slot: u32, _: &Owner) -> Self {
        Self {
            prefix,
            slot,
            generation: 1,
        }
    }
    fn prefix(&self) -> Prefix {
        self.prefix
    }
    fn read<'a>(
        &'a self,
        owner: &'a Owner,
        generations: &[u32],
        _: &HashMap<u64, u64>,
    ) -> Option<&'a Row96> {
        if generations.get(self.slot as usize)? != &self.generation {
            return None;
        }
        owner
            .rows()
            .get(self.slot as usize)
            .filter(|r| live(r, self.prefix.source_id))
    }
}
impl Binding for RowAlias96 {
    const NAME: &'static str = "row_alias";
    fn new(prefix: Prefix, slot: u32, owner: &Owner) -> Self {
        Self {
            link: Handle::new(prefix, slot, owner),
            reserved: [0; 64],
        }
    }
    fn prefix(&self) -> Prefix {
        self.link.prefix()
    }
    fn read<'a>(
        &'a self,
        owner: &'a Owner,
        generations: &[u32],
        index: &HashMap<u64, u64>,
    ) -> Option<&'a Row96> {
        self.link.read(owner, generations, index)
    }
}
impl Binding for Mirror {
    const NAME: &'static str = "mirror";
    fn new(prefix: Prefix, slot: u32, owner: &Owner) -> Self {
        Self {
            handle: Handle::new(prefix, slot, owner),
            row: owner.rows()[slot as usize],
        }
    }
    fn prefix(&self) -> Prefix {
        self.handle.prefix
    }
    fn read<'a>(
        &'a self,
        _: &'a Owner,
        generations: &[u32],
        _: &HashMap<u64, u64>,
    ) -> Option<&'a Row96> {
        if generations.get(self.handle.slot as usize)? != &self.handle.generation {
            return None;
        }
        live(&self.row, self.handle.prefix.source_id).then_some(&self.row)
    }
    fn refresh(&mut self, row: Row96) {
        self.row = row;
    }
}
impl Binding for Pointer {
    const NAME: &'static str = "pointer";
    const FROZEN: bool = true;
    fn new(prefix: Prefix, slot: u32, owner: &Owner) -> Self {
        Self {
            prefix,
            target: owner.pointer(slot as usize),
        }
    }
    fn prefix(&self) -> Prefix {
        self.prefix
    }
    fn read<'a>(&'a self, _: &'a Owner, _: &[u32], _: &HashMap<u64, u64>) -> Option<&'a Row96> {
        // SAFETY: only Bank::build creates bindings, always targeting its frozen
        // owner. Owner is private and dropped after bindings; no resize/free.
        // Reference is tied to &Bank. Every mutation needs &mut Bank, so there
        // are no retained shared references during writes, nor concurrent writers.
        let row = unsafe { self.target.as_ref() };
        live(row, self.prefix.source_id).then_some(row)
    }
}
impl Binding for IdLookup {
    const NAME: &'static str = "id_lookup";
    fn new(prefix: Prefix, _: u32, _: &Owner) -> Self {
        Self {
            prefix,
            reserved: 0,
        }
    }
    fn prefix(&self) -> Prefix {
        self.prefix
    }
    fn read<'a>(
        &'a self,
        owner: &'a Owner,
        _: &[u32],
        index: &HashMap<u64, u64>,
    ) -> Option<&'a Row96> {
        owner
            .rows()
            .get((*index.get(&self.prefix.source_id)? >> 2) as usize)
            .filter(|r| live(r, self.prefix.source_id))
    }
}

// Bindings precede Owner in declaration order and are therefore dropped first.
struct Bank<B: Binding> {
    bindings: Vec<B>,
    owner: Owner,
    equipment: Vec<Row96>,
    generations: Vec<u32>,
    offsets: Vec<u32>,
    reverse: Vec<u32>,
    index: HashMap<u64, u64>,
    index_bytes: usize,
    sources: Vec<u32>,
}
fn identity_index<B: Binding>(
    rows: &[Row96],
    equipment: &[Row96],
    bindings: &[B],
) -> Result<HashMap<u64, u64>, String> {
    let count = rows
        .len()
        .checked_add(equipment.len())
        .and_then(|n| n.checked_add(bindings.len()))
        .ok_or("index size overflow")?;
    let mut index = HashMap::with_capacity(count);
    let mut add = |id: u64, slot: usize, kind: u64| -> Result<(), String> {
        if id == 0 || index.insert(id, ((slot as u64) << 2) | kind).is_some() {
            return Err("duplicate or zero ID".into());
        }
        Ok(())
    };
    for (slot, row) in rows.iter().enumerate() {
        add(row.base.id, slot, 0)?;
    }
    for (slot, row) in equipment.iter().enumerate() {
        add(row.base.id, slot, 1)?;
    }
    for (slot, binding) in bindings.iter().enumerate() {
        add(binding.prefix().alias_id, slot, 2)?;
    }
    Ok(index)
}
fn source_slots(rows: &[Row96]) -> Vec<u32> {
    rows.iter()
        .enumerate()
        .filter(|(_, r)| {
            r.base.row_kind == 1
                && (r.base.parent == 0 || rows[(r.base.parent - 1) as usize].base.row_kind == 0)
        })
        .map(|(i, _)| i as u32)
        .collect()
}
impl<B: Binding> Bank<B> {
    fn build(rows: &[Row96], fanout: usize) -> Result<Self, String> {
        let sources = source_slots(rows);
        let count = sources
            .len()
            .checked_mul(fanout)
            .ok_or("alias count overflow")?;
        if count == 0 || count >= NONE as usize {
            return Err("alias count must fit nonzero u32 slots".into());
        }
        let owner = Owner::new(rows, B::FROZEN);
        let equipment_count = count.div_ceil(4);
        let mut equipment = Vec::with_capacity(equipment_count);
        let first_equipment = rows.len() as u64 + 1;
        let first_alias = first_equipment
            .checked_add(equipment_count as u64)
            .ok_or("ID overflow")?;
        first_alias.checked_add(count as u64).ok_or("ID overflow")?;
        for i in 0..equipment_count {
            equipment.push(Row96 {
                base: Row64 {
                    id: first_equipment + i as u64,
                    aux: NONE,
                    ..Row64::default()
                },
                numeric: Numeric32::default(),
            });
        }
        let mut bindings = Vec::with_capacity(count);
        let mut offsets = vec![0u32; rows.len() + 1];
        let mut reverse = Vec::with_capacity(count);
        for &slot in &sources {
            for _ in 0..fanout {
                let i = bindings.len();
                bindings.push(B::new(
                    Prefix {
                        alias_id: first_alias + i as u64,
                        owner_id: first_equipment + (i / 4) as u64,
                        source_id: rows[slot as usize].base.id,
                    },
                    slot,
                    &owner,
                ));
                reverse.push(i as u32);
            }
            offsets[slot as usize + 1] = fanout as u32;
        }
        for i in 1..offsets.len() {
            offsets[i] += offsets[i - 1];
        }
        let before = LIVE.load(Ordering::Relaxed);
        // Every variant can resolve external canonical, equipment and alias IDs.
        // Packed EntryRef uses the low two bits for kind and the rest for slot.
        let index = identity_index(rows, &equipment, &bindings)?;
        let index_bytes = LIVE.load(Ordering::Relaxed).saturating_sub(before);
        Ok(Self {
            bindings,
            owner,
            equipment,
            generations: vec![1; rows.len()],
            offsets,
            reverse,
            index,
            index_bytes,
            sources,
        })
    }
    fn read(&self, alias: usize) -> Option<&Row96> {
        self.bindings
            .get(alias)?
            .read(&self.owner, &self.generations, &self.index)
    }
    fn update(
        &mut self,
        slot: usize,
        payload: u64,
        received: u64,
        observation: Option<u64>,
    ) -> u64 {
        let row = self.owner.get_mut(slot);
        update_base(&mut row.base, payload, received, observation);
        let copy = *row;
        let mut notified = 0u64;
        for &alias in &self.reverse[self.offsets[slot] as usize..self.offsets[slot + 1] as usize] {
            let binding = &mut self.bindings[alias as usize];
            binding.refresh(copy);
            // Equal notification work for all variants; no actual bus/queues.
            notified = notified.wrapping_add(black_box(binding.prefix().alias_id));
        }
        notified
    }
    #[cfg(test)]
    fn remove(&mut self, slot: usize) -> Result<(), String> {
        let next = self.generations[slot]
            .checked_add(1)
            .ok_or("generation exhausted; retire slot")?;
        self.generations[slot] = next;
        self.owner.get_mut(slot).base.flags |= DELETED;
        Ok(())
    }
    fn verify(&self) -> Result<u64, String> {
        let mut hash = 0xcbf29ce484222325;
        for (alias, binding) in self.bindings.iter().enumerate() {
            let p = binding.prefix();
            let row = self.read(alias).ok_or("stale alias")?;
            // Sequential fixture IDs permit this independent observation check.
            let expected = self
                .owner
                .rows()
                .get((p.source_id - 1) as usize)
                .ok_or("unknown source")?;
            if row != expected {
                return Err("alias differs from canonical state".into());
            }
            if p.owner_id == row.base.parent || p.alias_id == row.base.id {
                return Err("alias identity conflated with physical state".into());
            }
            hash = row_hash(fold(fold(hash, p.alias_id), p.owner_id), *row);
        }
        Ok(hash)
    }
    fn breakdown(&self) -> [usize; 8] {
        [
            self.owner.bytes(),
            self.equipment.capacity() * size_of::<Row96>(),
            self.bindings.capacity() * size_of::<B>(),
            self.generations.capacity() * 4,
            (self.offsets.capacity() + self.reverse.capacity()) * 4,
            self.index_bytes,
            self.sources.capacity() * 4,
            0,
        ]
    }
}
fn update_base(b: &mut Row64, payload: u64, received: u64, observation: Option<u64>) {
    let changed = if b.value_kind == 3 {
        f64::from_bits(b.payload) != f64::from_bits(payload)
    } else {
        b.payload != payload
    };
    if changed {
        b.changed_ms = received;
    }
    b.payload = payload;
    b.received_ms = received;
    if let Some(observed) = observation {
        b.observed_ms = observed;
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
        u64::from(b.aux),
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
fn measure(mut f: impl FnMut() -> u64, count: usize) -> f64 {
    if count == 0 {
        return 0.0;
    }
    black_box(f());
    let mut samples = [0.0; ROUNDS];
    for sample in &mut samples {
        let start = Instant::now();
        black_box(f());
        *sample = start.elapsed().as_nanos() as f64 / count as f64;
    }
    samples.sort_by(f64::total_cmp);
    samples[ROUNDS / 2]
}
fn random_slots(candidates: &[usize], count: usize) -> Vec<usize> {
    if candidates.is_empty() {
        return Vec::new();
    }
    let mut seed = 0x9e3779b97f4a7c15u64;
    (0..count)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            candidates[seed as usize % candidates.len()]
        })
        .collect()
}
fn rss() -> (Option<usize>, Option<usize>) {
    let Ok(text) = std::fs::read_to_string("/proc/self/status") else {
        return (None, None);
    };
    let field = |name: &str| {
        text.lines().find_map(|line| {
            line.strip_prefix(name)
                .and_then(|v| v.split_whitespace().next())
                .and_then(|v| v.parse::<usize>().ok())
                .map(|v| v * 1024)
        })
    };
    (field("VmRSS:"), field("RssAnon:"))
}
fn json_option(value: Option<usize>) -> String {
    value.map_or("null".into(), |v| v.to_string())
}
fn bench<B: Binding>(
    rows: &[Row96],
    scale: usize,
    fanout: usize,
    pool_per_world: usize,
) -> Result<(), String> {
    let rss_before = rss();
    let before = LIVE.load(Ordering::Relaxed);
    let mut bank = Bank::<B>::build(rows, fanout)?;
    let pool_len = pool_per_world
        .checked_mul(scale)
        .ok_or("pool size overflow")?;
    let mut pool = vec![0u8; pool_len];
    for (i, byte) in pool.iter_mut().enumerate() {
        *byte = (i % 251) as u8;
    }
    black_box(&pool);
    let allocated = LIVE.load(Ordering::Relaxed) - before;
    let rss_after = rss();
    let mut breakdown = bank.breakdown();
    breakdown[7] = pool.capacity();
    if breakdown.iter().sum::<usize>() != allocated {
        return Err(format!(
            "live heap differs from breakdown: {allocated} versus {breakdown:?}"
        ));
    }
    let initial = bank.verify()?;
    let scalar: Vec<usize> = (0..bank.bindings.len())
        .filter(|&i| matches!(bank.read(i).unwrap().base.value_kind, 1..=3))
        .collect();
    let numeric: Vec<usize> = (0..bank.bindings.len())
        .filter(|&i| bank.read(i).unwrap().base.flags & NUMERIC != 0)
        .collect();
    let scalar_slots = random_slots(&scalar, OPS);
    let numeric_slots = random_slots(&numeric, OPS);
    let scalar_ns = measure(
        || {
            let mut sum = 0u64;
            for &i in &scalar_slots {
                let r = black_box(&bank).read(black_box(i)).unwrap();
                let b = &r.base;
                sum = sum.wrapping_add(b.payload ^ b.observed_ms ^ b.received_ms ^ b.changed_ms);
            }
            sum
        },
        scalar_slots.len(),
    );
    let numeric_ns = measure(
        || {
            let mut sum = 0u64;
            for &i in &numeric_slots {
                let r = black_box(&bank).read(black_box(i)).unwrap();
                let n = &r.numeric;
                sum = sum.wrapping_add(r.base.payload ^ n.min ^ n.max ^ n.step ^ n.anchor);
            }
            sum
        },
        numeric_slots.len(),
    );
    let update_count = (OPS / fanout).clamp(4096, OPS);
    let candidates: Vec<usize> = bank
        .sources
        .iter()
        .copied()
        .map(|v| v as usize)
        .filter(|&i| matches!(rows[i].base.value_kind, 1..=3))
        .collect();
    let update_slots = random_slots(&candidates, update_count);
    let mut tick = 10_000_000u64;
    let update_ns = measure(
        || {
            let mut sum = 0u64;
            for &slot in &update_slots {
                let b = bank.owner.rows()[slot].base;
                // Finite f64 0/1, bool 0/1 and integer bit toggles: actual changes.
                let payload = if b.value_kind == 3 {
                    if f64::from_bits(b.payload) == 1.0 {
                        0.0f64.to_bits()
                    } else {
                        1.0f64.to_bits()
                    }
                } else {
                    b.payload ^ 1
                };
                tick += 1;
                sum = sum.wrapping_add(black_box(&mut bank).update(
                    black_box(slot),
                    black_box(payload),
                    black_box(tick),
                    None,
                ));
            }
            sum
        },
        update_slots.len(),
    );
    let final_checksum = bank.verify()?;
    println!("{{\"variant\":\"{}\",\"scale\":{},\"fanout\":{},\"canonical_rows\":{},\"source_rows\":{},\"virtual_equipment_rows\":{},\"aliases\":{},\"row_bytes\":{},\"binding_bytes\":{},\"initial_checksum\":\"{:016x}\",\"final_checksum\":\"{:016x}\",\"parity\":true,\"rounds\":{},\"read_ops_per_round\":{},\"update_ops_per_round\":{},\"allocations_requested_bytes\":{},\"canonical_bytes\":{},\"virtual_equipment_bytes\":{},\"bindings_bytes\":{},\"generations_bytes\":{},\"reverse_index_bytes\":{},\"identity_index_bytes\":{},\"source_slots_bytes\":{},\"payload_pool_bytes\":{},\"rss_before_bytes\":{},\"rss_after_bytes\":{},\"rss_anon_before_bytes\":{},\"rss_anon_after_bytes\":{},\"scalar_ns_per_op\":{:.4},\"numeric_ns_per_op\":{:.4},\"source_update_notify_ns_per_op\":{:.4},\"scalar_timing_available\":{},\"numeric_timing_available\":{}}}",
        B::NAME,scale,fanout,rows.len(),bank.sources.len(),bank.equipment.len(),bank.bindings.len(),size_of::<Row96>(),size_of::<B>(),initial,final_checksum,ROUNDS,OPS,update_slots.len(),allocated,
        breakdown[0],breakdown[1],breakdown[2],breakdown[3],breakdown[4],breakdown[5],breakdown[6],breakdown[7],json_option(rss_before.0),json_option(rss_after.0),json_option(rss_before.1),json_option(rss_after.1),scalar_ns,numeric_ns,update_ns,!scalar.is_empty(),!numeric.is_empty());
    Ok(())
}
fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 2 && args[1] == "--help" {
        println!("--bench INPUT.tsv row_alias|mirror|handle|pointer|id_lookup SCALE FANOUT [POOL_BYTES_PER_WORLD]\nSame 12-column TSV contract as tools/catalogue-banc; bit15 reserved as tombstone.\nPrincipal kind=1 states only; nested record children remain canonical without aliases.\nOne virtual equipment Row96 per four aliases. Alias IDs never replace source IDs.\nPure aliases only; calculated states must own separate canonical rows.\nOne warmup + medians of 7 rounds, 1048576 random reads per round.\nUpdates include identical reverse CSR notification iteration plus mirror value copies.\nRequested LIVE heap includes every persistent array/map/pool, excludes input and timing buffers.\nPointer owner is a frozen Box allocation, single-threaded exclusive mutations; no production API.");
        return Ok(());
    }
    if !(args.len() == 6 || args.len() == 7) || args[1] != "--bench" {
        return Err("usage: --bench INPUT.tsv row_alias|mirror|handle|pointer|id_lookup SCALE FANOUT [POOL_BYTES_PER_WORLD]".into());
    }
    let text = std::fs::read_to_string(&args[2]).map_err(|e| e.to_string())?;
    let input = parse(&text)?;
    let scale = args[4].parse::<usize>().map_err(|_| "invalid scale")?;
    let fanout = args[5].parse::<usize>().map_err(|_| "invalid fanout")?;
    let pool = args
        .get(6)
        .map_or(Ok(0), |s| s.parse::<usize>())
        .map_err(|_| "invalid pool")?;
    let rows = replicate(&input, scale)?;
    match args[3].as_str() {
        "row_alias" => bench::<RowAlias96>(&rows, scale, fanout, pool),
        "mirror" => bench::<Mirror>(&rows, scale, fanout, pool),
        "handle" => bench::<Handle>(&rows, scale, fanout, pool),
        "pointer" => bench::<Pointer>(&rows, scale, fanout, pool),
        "id_lookup" => bench::<IdLookup>(&rows, scale, fanout, pool),
        _ => Err("unknown variant".into()),
    }
}
fn main() {
    if let Err(error) = run() {
        eprintln!("alias-banc: {error}");
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
    fn layouts_96_32_128() {
        assert_eq!(
            (
                size_of::<Row96>(),
                size_of::<Prefix>(),
                size_of::<Handle>(),
                size_of::<Pointer>(),
                size_of::<IdLookup>(),
                size_of::<Mirror>(),
                size_of::<RowAlias96>()
            ),
            (96, 24, 32, 32, 32, 128, 96)
        );
    }
    #[test]
    fn full_row_alias_envelope_shares_canonical_state_without_copy() {
        coherence::<RowAlias96>();
        zero_dates::<RowAlias96>();
        let mut bank = Bank::<RowAlias96>::build(&sample(), 2).unwrap();
        let mut handle = Bank::<Handle>::build(&sample(), 2).unwrap();
        assert!(std::ptr::eq(bank.read(0).unwrap(), bank.read(1).unwrap()));
        assert_eq!(bank.verify().unwrap(), handle.verify().unwrap());
        bank.update(1, 123, 40, Some(35));
        handle.update(1, 123, 40, Some(35));
        assert_eq!(bank.verify().unwrap(), handle.verify().unwrap());
        assert_eq!(bank.bindings.len() * size_of::<RowAlias96>(), 384);
        bank.remove(1).unwrap();
        assert!(bank.read(0).is_none());
    }
    #[test]
    fn principal_states_only_and_deterministic_alias_identity() {
        let rows = replicate(&sample(), 3).unwrap();
        assert_eq!(source_slots(&rows), vec![1, 2, 6, 7, 11, 12]);
        let bank = Bank::<Handle>::build(&rows, 3).unwrap();
        assert_eq!(
            (
                bank.owner.rows().len(),
                bank.equipment.len(),
                bank.bindings.len()
            ),
            (15, 5, 18)
        );
        assert_eq!(bank.bindings[0].prefix.alias_id, 21);
        assert_eq!(bank.bindings[0].prefix.source_id, 2);
        assert_eq!(bank.bindings[3].prefix.source_id, 3);
        assert_eq!(bank.index.len(), 38);
    }
    fn coherence<B: Binding>() {
        let mut bank = Bank::<B>::build(&sample(), 5).unwrap();
        let parent = bank.owner.rows()[1].base.parent;
        let notified = bank.update(1, 123, 40, Some(35));
        assert!(notified > 0);
        for alias in 0..5 {
            let r = bank.read(alias).unwrap();
            assert_eq!(
                (
                    r.base.payload,
                    r.base.observed_ms,
                    r.base.received_ms,
                    r.base.changed_ms
                ),
                (123, 35, 40, 40)
            );
            assert_eq!(r.base.parent, parent);
        }
        assert_eq!(bank.read(5).unwrap().base.payload, 9);
        bank.verify().unwrap();
    }
    #[test]
    fn all_variants_share_one_source_and_mirrors_copy_full_state() {
        coherence::<Mirror>();
        coherence::<Handle>();
        coherence::<Pointer>();
        coherence::<IdLookup>();
    }
    #[test]
    fn shared_variants_do_not_own_duplicate_96_state() {
        let bank = Bank::<Handle>::build(&sample(), 2).unwrap();
        assert!(std::ptr::eq(bank.read(0).unwrap(), bank.read(1).unwrap()));
        assert_eq!(bank.bindings.len() * size_of::<Handle>(), 128);
        let pointer = Bank::<Pointer>::build(&sample(), 2).unwrap();
        assert!(std::ptr::eq(
            pointer.read(0).unwrap(),
            pointer.read(1).unwrap()
        ));
        let mirror = Bank::<Mirror>::build(&sample(), 2).unwrap();
        assert!(!std::ptr::eq(
            mirror.read(0).unwrap(),
            mirror.read(1).unwrap()
        ));
    }
    #[test]
    fn virtual_owner_does_not_move_physical_parent_or_alias_source_id() {
        let bank = Bank::<Mirror>::build(&sample(), 8).unwrap();
        assert_ne!(
            bank.bindings[0].prefix().owner_id,
            bank.bindings[4].prefix().owner_id
        );
        for alias in 0..8 {
            assert_eq!(bank.bindings[alias].prefix().source_id, 2);
            assert_eq!(bank.read(alias).unwrap().base.id, 2);
            assert_eq!(bank.read(alias).unwrap().base.parent, 1);
        }
    }
    #[test]
    fn stale_handles_rejected_after_removal_and_slot_reuse() {
        let mut bank = Bank::<Handle>::build(&sample(), 2).unwrap();
        bank.remove(1).unwrap();
        assert!(bank.read(0).is_none());
        bank.owner.get_mut(1).base.flags &= !DELETED;
        bank.owner.get_mut(1).base.id = 999;
        assert!(bank.read(0).is_none());
        // ID check alone rejects ABA even if generation were accidentally reset.
        bank.generations[1] = 1;
        assert!(bank.read(0).is_none());
    }
    #[test]
    fn generation_exhaustion_retires_without_wrapping_or_partial_mutation() {
        let mut bank = Bank::<Handle>::build(&sample(), 2).unwrap();
        bank.generations[1] = u32::MAX;
        assert!(bank.remove(1).is_err());
        assert_eq!(bank.generations[1], u32::MAX);
        assert_eq!(bank.owner.rows()[1].base.flags & DELETED, 0);
    }
    #[test]
    fn growing_source_vec_preserves_cached_slots_and_ids() {
        let mut bank = Bank::<Handle>::build(&sample(), 2).unwrap();
        let Owner::Moving(rows) = &mut bank.owner else {
            panic!("movable handles");
        };
        for i in 0..10000 {
            rows.push(Row96 {
                base: Row64 {
                    id: 100 + i,
                    ..Row64::default()
                },
                ..Row96::default()
            });
        }
        assert_eq!(bank.read(0).unwrap().base.payload, (-7i64) as u64);
        bank.update(1, 321, 100, None);
        assert_eq!(bank.read(1).unwrap().base.payload, 321);
    }
    #[test]
    fn frozen_pointer_addresses_survive_owner_moves_and_exclusive_updates() {
        let bank = Bank::<Pointer>::build(&sample(), 2).unwrap();
        let address = bank.bindings[0].target;
        let mut moved = Box::new(bank);
        moved.update(1, 88, 99, None);
        assert_eq!(moved.bindings[0].target, address);
        assert_eq!(moved.read(0).unwrap().base.payload, 88);
        moved.remove(1).unwrap();
        assert!(moved.read(0).is_none()); // memory remains allocated, tombstone checked
    }
    #[test]
    fn aliases_preserve_readonly_has_set_bounds_and_unknown_dates() {
        let bank = Bank::<Handle>::build(&sample(), 2).unwrap();
        assert_eq!(bank.read(0).unwrap().base.flags, 15);
        assert_eq!(bank.read(0).unwrap().numeric, sample()[1].numeric);
        let readonly = bank.read(2).unwrap();
        assert_eq!(readonly.base.flags & READ, READ);
        assert_eq!(readonly.base.flags & HAS_SET, 0);
        assert_eq!(readonly.base.observed_ms, 0);
    }
    #[test]
    fn calculated_value_owns_independent_canonical_row() {
        let mut rows = sample();
        // This result is materialized: it is not a pure alias binding.
        let mut calculated = rows[1];
        calculated.base.id = 6;
        calculated.base.payload = 246;
        rows.push(calculated);
        let mut bank = Bank::<Handle>::build(&rows, 2).unwrap();
        bank.update(1, 123, 100, None);
        assert_eq!(bank.owner.rows()[5].base.payload, 246);
        assert!(!std::ptr::eq(&bank.owner.rows()[1], &bank.owner.rows()[5]));
        bank.update(5, 1000, 200, None);
        assert_eq!(bank.read(0).unwrap().base.payload, 123);
    }
    fn zero_dates<B: Binding>() {
        let mut rows = sample();
        rows[1].base.value_kind = 3;
        rows[1].base.payload = (-0.0f64).to_bits();
        let mut bank = Bank::<B>::build(&rows, 2).unwrap();
        bank.update(1, 0.0f64.to_bits(), 30, None);
        assert_eq!(
            (
                bank.read(0).unwrap().base.payload,
                bank.read(0).unwrap().base.changed_ms
            ),
            (0.0f64.to_bits(), 10)
        );
        bank.update(1, 0.0f64.to_bits(), 40, Some(35));
        assert_eq!(
            (
                bank.read(1).unwrap().base.observed_ms,
                bank.read(1).unwrap().base.received_ms,
                bank.read(1).unwrap().base.changed_ms
            ),
            (35, 40, 10)
        );
        bank.update(1, 1.0f64.to_bits(), 50, None);
        assert_eq!(bank.read(1).unwrap().base.changed_ms, 50);
    }
    #[test]
    fn signed_zero_and_equal_receptions_preserve_change_date() {
        zero_dates::<Mirror>();
        zero_dates::<Handle>();
        zero_dates::<Pointer>();
        zero_dates::<IdLookup>();
    }
    #[test]
    fn virtual_equipment_has_unknown_health_and_no_invented_permissions() {
        let bank = Bank::<Handle>::build(&sample(), 2).unwrap();
        let b = bank.equipment[0].base;
        assert_eq!(
            (
                b.row_kind,
                b.value_kind,
                b.payload,
                b.flags,
                b.observed_ms,
                b.received_ms,
                b.changed_ms
            ),
            (0, 0, 0, 0, 0, 0, 0)
        );
        validate_row(&bank.equipment[0]).unwrap();
    }
    #[test]
    fn identity_registry_rejects_duplicates_across_every_kind() {
        let bank = Bank::<Handle>::build(&sample(), 2).unwrap();
        let mut equipment = bank.equipment.clone();
        equipment[0].base.id = 2;
        assert!(identity_index(bank.owner.rows(), &equipment, &bank.bindings).is_err());
        let mut rows = sample();
        rows[2].base.id = 2;
        assert!(Bank::<Handle>::build(&rows, 2).is_err());
    }
    #[test]
    fn parser_rejects_forward_parent_and_reserved_tombstone() {
        assert!(parse("1\t0\t1\t1\t1\t0\t1\t1\t0\t0\t0\t0").is_err());
        assert!(parse("0\t4294967295\t0\t32768\t0\t0\t0\t0\t0\t0\t0\t0").is_err());
        assert!(Bank::<Handle>::build(&sample(), 0).is_err());
    }
    #[test]
    fn all_variant_checksums_agree_before_and_after_updates() {
        let rows = replicate(&sample(), 3).unwrap();
        let mut a = Bank::<Mirror>::build(&rows, 4).unwrap();
        let mut b = Bank::<Handle>::build(&rows, 4).unwrap();
        let mut c = Bank::<Pointer>::build(&rows, 4).unwrap();
        let mut d = Bank::<IdLookup>::build(&rows, 4).unwrap();
        assert_eq!(
            [a.verify().unwrap(); 3],
            [
                b.verify().unwrap(),
                c.verify().unwrap(),
                d.verify().unwrap()
            ]
        );
        for (i, slot) in [1, 6, 11, 2, 7, 12].into_iter().enumerate() {
            a.update(slot, i as u64, 100 + i as u64, None);
            b.update(slot, i as u64, 100 + i as u64, None);
            c.update(slot, i as u64, 100 + i as u64, None);
            d.update(slot, i as u64, 100 + i as u64, None);
        }
        assert_eq!(
            [a.verify().unwrap(); 3],
            [
                b.verify().unwrap(),
                c.verify().unwrap(),
                d.verify().unwrap()
            ]
        );
    }
}
