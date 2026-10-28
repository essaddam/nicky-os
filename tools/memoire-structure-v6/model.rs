//! Single-writer, synthetic reference model. No device execution, pointers, or unsafe.
#![allow(dead_code)]
use crate::layouts::*;
use std::collections::{HashMap, HashSet};
use std::mem::size_of;

pub type Result<T> = std::result::Result<T, &'static str>;
pub const FAMILY: u8 = 1;
pub const HOUSE: u8 = 2;
pub const FLOOR: u8 = 3;
pub const ROOM: u8 = 4;
pub const ZONE: u8 = 5;
pub const PHYSICAL: u8 = 6;
pub const VIRTUAL: u8 = 7;
pub const VALUE: u8 = 8;
pub const ACTION: u8 = 9;
pub const CALC: u8 = 10;
pub const GEOZONE: u8 = 11;
pub const NULL: u8 = 0;
pub const INTEGER: u8 = 1;
pub const COMPOSITE: u8 = 2;
pub const WRITE: u16 = 1;
pub const ACTIVE_GET: u16 = 2;
pub const HAS_OBSERVATION: u16 = 0x0010;
pub const HAS_PREVIOUS: u16 = 0x0040;
pub const RESERVED_FLAGS: u16 = 0x0080;
pub const PREVIOUS_KIND_MASK: u16 = 0xF000;
pub const PREVIOUS_KIND_SHIFT: u32 = 12;
const LIVE: u8 = 1;
const ROW: u8 = 1;
const ALIAS: u8 = 2;
pub const CHILD: u16 = 1;
pub const MEMBER: u16 = 2;

#[derive(Debug, PartialEq)]
pub struct Intent {
    pub owner_id: u64,
    pub point_id: u64,
    pub payload: u64,
    pub active_get: bool,
}

#[derive(Default)]
pub struct Bank {
    pub(crate) rows: Vec<Row96>,
    pub(crate) aliases: Vec<Alias32>,
    pub(crate) slots: Vec<GlobalSlot16>,
    pub(crate) metadata: Vec<Meta32>,
    pub(crate) index: HashMap<u64, Handle8>,
    pub(crate) edges: Vec<Edge24>,
    pub epoch: u64,
    pub pool: Pool,
}

impl Bank {
    pub fn new() -> Self {
        Self {
            epoch: 1,
            ..Self::default()
        }
    }

    fn checked_slot(&self, handle: Handle8) -> Result<&GlobalSlot16> {
        let slot = self
            .slots
            .get(handle.slot as usize)
            .ok_or("slot out of bounds")?;
        if slot.flags & LIVE == 0 || slot.generation != handle.generation {
            return Err("stale generation");
        }
        Ok(slot)
    }
    pub fn handle(&self, id: u64) -> Result<Handle8> {
        self.index.get(&id).copied().ok_or("unknown id")
    }
    fn row(&self, id: u64) -> Result<&Row96> {
        let slot = self.checked_slot(self.handle(id)?)?;
        if slot.arena != ROW {
            return Err("expected canonical row");
        }
        let row = self
            .rows
            .get(slot.physical_index as usize)
            .ok_or("invalid row index")?;
        if row.base.id != id {
            return Err("row identity mismatch");
        }
        Ok(row)
    }
    pub fn read_at(&self, id: u64, epoch: u64) -> Result<&Row96> {
        if epoch != self.epoch {
            return Err("stale epoch");
        }
        let slot = self.checked_slot(self.handle(id)?)?;
        if slot.arena == ROW {
            return self.row(id);
        }
        let alias = self
            .aliases
            .get(slot.physical_index as usize)
            .ok_or("invalid alias index")?;
        if alias.alias_id != id {
            return Err("alias identity mismatch");
        }
        let source = self.checked_slot(Handle8 {
            slot: alias.source_slot,
            generation: alias.source_generation,
        })?;
        if source.arena != ROW {
            return Err("alias source is not canonical");
        }
        let row = self
            .rows
            .get(source.physical_index as usize)
            .ok_or("invalid source index")?;
        if row.base.id != alias.source_id {
            return Err("source identity mismatch");
        }
        Ok(row)
    }
    fn allocate(&mut self, id: u64, arena: u8, physical_index: usize) -> Result<Handle8> {
        if id == 0 || self.index.contains_key(&id) {
            return Err("zero or duplicate id");
        }
        let physical_index = u32::try_from(physical_index).map_err(|_| "arena full")?;
        let handle = if let Some(position) = self
            .slots
            .iter()
            .position(|s| s.flags & LIVE == 0 && s.generation < u32::MAX)
        {
            let slot = &mut self.slots[position];
            slot.physical_index = physical_index;
            slot.arena = arena;
            slot.flags = LIVE;
            self.metadata[slot.meta_index as usize] = Meta32::default();
            Handle8 {
                slot: position as u32,
                generation: slot.generation,
            }
        } else {
            let position = u32::try_from(self.slots.len()).map_err(|_| "slot arena full")?;
            let meta_index = u32::try_from(self.metadata.len()).map_err(|_| "metadata full")?;
            self.metadata.push(Meta32::default());
            self.slots.push(GlobalSlot16 {
                physical_index,
                generation: 1,
                meta_index,
                arena,
                flags: LIVE,
                reserved: 0,
            });
            Handle8 {
                slot: position,
                generation: 1,
            }
        };
        self.index.insert(id, handle);
        Ok(handle)
    }
    fn parent_valid(&self, id: u64, kind: u8, parent: u64) -> Result<()> {
        if kind == FAMILY {
            return if parent == 0 {
                Ok(())
            } else {
                Err("family must be root")
            };
        }
        let parent_kind = self.row(parent)?.base.row_kind;
        let valid = match kind {
            HOUSE => parent_kind == FAMILY,
            FLOOR | ZONE | GEOZONE => parent_kind == HOUSE,
            ROOM => matches!(parent_kind, HOUSE | FLOOR),
            PHYSICAL | VIRTUAL => matches!(parent_kind, HOUSE | ROOM),
            VALUE | ACTION | CALC => matches!(parent_kind, PHYSICAL | VIRTUAL),
            _ => false,
        };
        if !valid {
            return Err("invalid parent kind");
        }
        let mut current = parent;
        let mut seen = HashSet::new();
        while current != 0 {
            if current == id || !seen.insert(current) {
                return Err("parent cycle");
            }
            current = self.row(current)?.base.parent;
        }
        Ok(())
    }
    pub fn insert(&mut self, id: u64, kind: u8, parent: u64, flags: u16) -> Result<Handle8> {
        if flags & (HAS_OBSERVATION | HAS_PREVIOUS | PREVIOUS_KIND_MASK) != 0 {
            return Err("observation flags are managed internally");
        }
        self.parent_valid(id, kind, parent)?;
        let handle = self.allocate(id, ROW, self.rows.len())?;
        self.rows.push(Row96 {
            base: Row64 {
                id,
                parent,
                row_kind: kind,
                flags,
                ..Row64::default()
            },
            ..Row96::default()
        });
        if parent != 0 {
            self.edges.push(Edge24 {
                from: parent,
                to: id,
                relation: CHILD,
                ..Edge24::default()
            });
        }
        Ok(handle)
    }
    pub fn alias(&mut self, id: u64, owner: u64, source: u64) -> Result<Handle8> {
        if self.row(owner)?.base.row_kind != VIRTUAL {
            return Err("alias owner must be virtual");
        }
        let resolved = *self.read_at(source, self.epoch)?;
        let source_id = resolved.base.id;
        if source_id == id {
            return Err("alias loop");
        }
        let source_kind = resolved.base.row_kind;
        if !matches!(source_kind, VALUE | ACTION | CALC) {
            return Err("invalid alias source");
        }
        if source_kind != CALC && self.row(resolved.base.parent)?.base.row_kind != PHYSICAL {
            return Err("source must be physical or calculation");
        }
        let source_handle = self.handle(source_id)?;
        let handle = self.allocate(id, ALIAS, self.aliases.len())?;
        self.aliases.push(Alias32 {
            alias_id: id,
            owner_id: owner,
            source_id,
            source_slot: source_handle.slot,
            source_generation: source_handle.generation,
        });
        self.edges.push(Edge24 {
            from: owner,
            to: id,
            relation: CHILD,
            ..Edge24::default()
        });
        Ok(handle)
    }
    pub fn rename(&mut self, id: u64, label: u32) -> Result<()> {
        let meta = self.checked_slot(self.handle(id)?)?.meta_index;
        self.metadata[meta as usize].label = label;
        Ok(())
    }
    /// The previous distinct observed value, resolved through aliases like current state.
    pub fn previous(&self, id: u64) -> Result<Option<(u8, u64)>> {
        let row = &self.read_at(id, self.epoch)?.base;
        Ok((row.flags & HAS_PREVIOUS != 0).then_some((
            ((row.flags & PREVIOUS_KIND_MASK) >> PREVIOUS_KIND_SHIFT) as u8,
            row.previous_payload,
        )))
    }
    pub fn observe(
        &mut self,
        id: u64,
        kind: u8,
        payload: u64,
        source_changed_at: u64,
        received: u64,
    ) -> Result<()> {
        if !matches!(kind, NULL | INTEGER | COMPOSITE) {
            return Err("unknown value kind");
        }
        let row = self.row(id)?.base; // Direct observations never target aliases.
        let equal = if kind == COMPOSITE {
            let (_, fields) =
                parse_composite(self.pool.get(unpack_handle(payload), COMPOSITE as u16)?)?;
            self.same_composite(&row, &fields)?
        } else {
            row.value_kind == kind && row.payload == payload
        };
        let next =
            Self::observation_transition(row, kind, payload, equal, source_changed_at, received)?;
        let index = self.checked_slot(self.handle(id)?)?.physical_index as usize;
        self.rows[index].base = next;
        Ok(())
    }
    /// Pure preparation: every fallible check finishes before row/pool publication.
    fn observation_transition(
        mut row: Row64,
        kind: u8,
        payload: u64,
        equal: bool,
        source_changed_at: u64,
        received: u64,
    ) -> Result<Row64> {
        let revision = row.revision.checked_add(1).ok_or("revision exhausted")?;
        let observed = row.flags & HAS_OBSERVATION != 0;
        if !observed || !equal {
            if observed {
                row.previous_payload = row.payload;
                row.flags = (row.flags & !PREVIOUS_KIND_MASK)
                    | HAS_PREVIOUS
                    | (u16::from(row.value_kind) << PREVIOUS_KIND_SHIFT);
            }
            row.changed_ms = source_changed_at;
            row.value_kind = kind;
            row.payload = payload;
        }
        row.flags |= HAS_OBSERVATION;
        row.received_ms = received;
        row.revision = revision;
        Ok(row)
    }
    fn same_composite(&self, row: &Row64, fields: &[(u32, u64)]) -> Result<bool> {
        if row.flags & HAS_OBSERVATION == 0 || row.value_kind != COMPOSITE {
            return Ok(false);
        }
        let (_, mut current) = parse_composite(
            self.pool
                .get(unpack_handle(row.payload), COMPOSITE as u16)?,
        )?;
        let mut next = fields.to_vec();
        // Keys are unique: wire order and envelope version do not change field semantics.
        current.sort_unstable_by_key(|(key, _)| *key);
        next.sort_unstable_by_key(|(key, _)| *key);
        Ok(current == next)
    }
    pub fn intent(&self, id: u64, payload: u64, active_get: bool) -> Result<Intent> {
        let row = &self.read_at(id, self.epoch)?.base;
        let required = if active_get { ACTIVE_GET } else { WRITE };
        if row.flags & required == 0 {
            return Err("operation not available");
        }
        if self.row(row.parent)?.base.row_kind != PHYSICAL {
            return Err("no physical route");
        }
        Ok(Intent {
            owner_id: row.parent,
            point_id: row.id,
            payload,
            active_get,
        })
    }
    pub fn remove(&mut self, id: u64) -> Result<()> {
        let handle = self.handle(id)?;
        let slot = *self.checked_slot(handle)?;
        let generation = slot
            .generation
            .checked_add(1)
            .ok_or("generation exhausted; retire slot")?;
        if self
            .edges
            .iter()
            .any(|edge| edge.from == id && edge.relation == CHILD)
        {
            return Err("has children");
        }
        let displaced = if slot.arena == ROW {
            self.rows.swap_remove(slot.physical_index as usize);
            self.rows
                .get(slot.physical_index as usize)
                .map(|row| row.base.id)
        } else {
            self.aliases.swap_remove(slot.physical_index as usize);
            self.aliases
                .get(slot.physical_index as usize)
                .map(|alias| alias.alias_id)
        };
        if let Some(displaced) = displaced {
            let moved = self.handle(displaced)?;
            self.slots[moved.slot as usize].physical_index = slot.physical_index;
        }
        self.slots[handle.slot as usize].flags = 0;
        self.slots[handle.slot as usize].generation = generation;
        self.index.remove(&id);
        self.edges.retain(|edge| edge.from != id && edge.to != id);
        Ok(())
    }
    pub fn compact(&mut self) {
        self.rows.reverse();
        for (position, row) in self.rows.iter().enumerate() {
            self.slots[self.index[&row.base.id].slot as usize].physical_index = position as u32;
        }
        self.rows.shrink_to_fit();
        self.aliases.shrink_to_fit();
    }
    pub fn next_epoch(&mut self) -> Result<()> {
        self.epoch = self.epoch.checked_add(1).ok_or("epoch exhausted")?;
        Ok(())
    }
    pub fn house(&self, id: u64) -> Result<u64> {
        let mut current = id;
        let mut seen = HashSet::new();
        while current != 0 && seen.insert(current) {
            let row = &self.row(current)?.base;
            if row.row_kind == HOUSE {
                return Ok(row.id);
            }
            current = row.parent;
        }
        Err("not in a house")
    }
    pub fn membership(&mut self, zone: u64, target: u64) -> Result<()> {
        if self.row(zone)?.base.row_kind != ZONE {
            return Err("expected functional zone");
        }
        if !matches!(self.row(target)?.base.row_kind, ROOM | PHYSICAL | VIRTUAL) {
            return Err("invalid zone member");
        }
        if self.house(zone)? != self.house(target)? {
            return Err("cross-house membership");
        }
        if self
            .edges
            .iter()
            .any(|edge| edge.from == zone && edge.to == target && edge.relation == MEMBER)
        {
            return Err("duplicate membership");
        }
        self.edges.push(Edge24 {
            from: zone,
            to: target,
            relation: MEMBER,
            ..Edge24::default()
        });
        Ok(())
    }
    pub fn reparent(&mut self, id: u64, parent: u64) -> Result<()> {
        let row = *self.row(id)?;
        self.parent_valid(id, row.base.row_kind, parent)?;
        let index = self.checked_slot(self.handle(id)?)?.physical_index as usize;
        self.rows[index].base.parent = parent;
        if self
            .edges
            .iter()
            .filter(|edge| edge.relation == MEMBER)
            .any(|edge| self.house(edge.from).ok() != self.house(edge.to).ok())
        {
            self.rows[index].base.parent = row.base.parent;
            return Err("move invalidates zone membership");
        }
        self.edges
            .retain(|edge| !(edge.to == id && edge.relation == CHILD));
        if parent != 0 {
            self.edges.push(Edge24 {
                from: parent,
                to: id,
                relation: CHILD,
                ..Edge24::default()
            });
        }
        Ok(())
    }
    pub fn csr(&self) -> Result<Csr<'_>> {
        Csr::build(self)
    }
    pub fn composite(
        &mut self,
        id: u64,
        expected: u32,
        bytes: &[u8],
        source_changed_at: u64,
        received: u64,
    ) -> Result<()> {
        let (version, fields) = parse_composite(bytes)?;
        let row = self.row(id)?.base;
        if row.revision != expected
            || version != expected.checked_add(1).ok_or("revision exhausted")?
        {
            return Err("composite version conflict");
        }
        let equal = self.same_composite(&row, &fields)?;
        let mut next =
            Self::observation_transition(row, COMPOSITE, 0, equal, source_changed_at, received)?;
        let position = self.checked_slot(self.handle(id)?)?.physical_index as usize;
        if !equal {
            let reference = self.pool.put(bytes, COMPOSITE as u16)?;
            next.payload = u64::from(reference.slot) | (u64::from(reference.generation) << 32);
        }
        // The retained blob version dates its last content publication. Row revision
        // counts all accepted reports, including reports with identical fields.
        self.rows[position].base = next;
        Ok(())
    }
    pub fn memory(&self, csr: &Csr<'_>) -> String {
        let mut entries = Vec::new();
        macro_rules! vector {
            ($name:expr,$v:expr,$ty:ty) => {entries.push(format!("\"{}\":{{\"len\":{},\"capacity\":{},\"element_bytes\":{},\"used_bytes\":{},\"capacity_bytes\":{}}}",
                $name,$v.len(),$v.capacity(),size_of::<$ty>(),$v.len()*size_of::<$ty>(),$v.capacity()*size_of::<$ty>()));};
        }
        vector!("rows", self.rows, Row96);
        vector!("aliases", self.aliases, Alias32);
        vector!("global_slots", self.slots, GlobalSlot16);
        vector!("metadata", self.metadata, Meta32);
        vector!("durable_edges", self.edges, Edge24);
        vector!("forward_offsets", csr.forward_offsets, u32);
        vector!("forward_targets", csr.forward, Target12);
        vector!("reverse_offsets", csr.reverse_offsets, u32);
        vector!("reverse_edge_indices", csr.reverse, u32);
        vector!("pool_entries", self.pool.entries, PoolEntry16);
        vector!("pool_bytes", self.pool.bytes, u8);
        format!(
            "{{\"scope\":\"Vec element storage only; allocator overhead, HashMap buckets/control bytes, stacks and temporary builders excluded\",\"vectors\":{{{}}},\"index\":{{\"len\":{},\"capacity\":{},\"key_value_payload_bytes\":{},\"allocation_bytes\":null}}}}",
            entries.join(","),
            self.index.len(),
            self.index.capacity(),
            self.index.len() * (size_of::<u64>() + size_of::<Handle8>())
        )
    }
}

pub struct Csr<'a> {
    bank: &'a Bank,
    pub forward_offsets: Vec<u32>,
    pub forward: Vec<Target12>,
    pub reverse_offsets: Vec<u32>,
    pub reverse: Vec<u32>,
}
impl<'a> Csr<'a> {
    fn build(bank: &'a Bank) -> Result<Self> {
        let nodes = bank.slots.len();
        let mut forward_offsets = vec![0u32; nodes + 1];
        let mut reverse_offsets = vec![0u32; nodes + 1];
        // Keep durable Edge24 alongside both derived indexes. Reverse entries point
        // into durable_edges, not into sorted forward targets.
        for edge in &bank.edges {
            let from = bank.handle(edge.from)?.slot as usize;
            let to = bank.handle(edge.to)?.slot as usize;
            forward_offsets[from + 1] = forward_offsets[from + 1]
                .checked_add(1)
                .ok_or("CSR overflow")?;
            reverse_offsets[to + 1] = reverse_offsets[to + 1]
                .checked_add(1)
                .ok_or("CSR overflow")?;
        }
        for index in 1..=nodes {
            forward_offsets[index] = forward_offsets[index]
                .checked_add(forward_offsets[index - 1])
                .ok_or("CSR overflow")?;
            reverse_offsets[index] = reverse_offsets[index]
                .checked_add(reverse_offsets[index - 1])
                .ok_or("CSR overflow")?;
        }
        let mut forward = vec![Target12::default(); bank.edges.len()];
        let mut reverse = vec![0u32; bank.edges.len()];
        let mut f = forward_offsets.clone();
        let mut r = reverse_offsets.clone();
        for (index, edge) in bank.edges.iter().enumerate() {
            let from = bank.handle(edge.from)?.slot as usize;
            let to = bank.handle(edge.to)?.slot as usize;
            forward[f[from] as usize] = Target12 {
                to: to as u32,
                relation: edge.relation,
                flags: edge.flags,
                revision: edge.revision,
            };
            reverse[r[to] as usize] = u32::try_from(index).map_err(|_| "edge index overflow")?;
            f[from] += 1;
            r[to] += 1;
        }
        Ok(Self {
            bank,
            forward_offsets,
            forward,
            reverse_offsets,
            reverse,
        })
    }
    pub fn targets(&self, handle: Handle8) -> Result<&[Target12]> {
        self.bank.checked_slot(handle)?;
        let index = handle.slot as usize;
        Ok(&self.forward
            [self.forward_offsets[index] as usize..self.forward_offsets[index + 1] as usize])
    }
}

#[derive(Default)]
pub struct Pool {
    pub entries: Vec<PoolEntry16>,
    pub bytes: Vec<u8>,
}
impl Pool {
    pub const MAX_ENTRY: usize = 4096;
    pub const MAX_TOTAL: usize = 1_048_576;
    pub const MAX_ENTRIES: usize = 65_536;
    pub fn put(&mut self, bytes: &[u8], kind: u16) -> Result<Handle8> {
        let end = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .ok_or("pool overflow")?;
        if bytes.len() > Self::MAX_ENTRY
            || end > Self::MAX_TOTAL
            || self.entries.len() >= Self::MAX_ENTRIES
        {
            return Err("pool bound exceeded");
        }
        let slot = u32::try_from(self.entries.len()).map_err(|_| "pool entry overflow")?;
        self.entries.push(PoolEntry16 {
            offset: self.bytes.len() as u32,
            len: bytes.len() as u32,
            generation: 1,
            kind,
            flags: 0,
        });
        self.bytes.extend_from_slice(bytes);
        Ok(Handle8 {
            slot,
            generation: 1,
        })
    }
    pub fn get(&self, reference: Handle8, kind: u16) -> Result<&[u8]> {
        let entry = self
            .entries
            .get(reference.slot as usize)
            .ok_or("pool ref out of bounds")?;
        if entry.generation != reference.generation || entry.kind != kind {
            return Err("pool reference mismatch");
        }
        let start = entry.offset as usize;
        let end = start
            .checked_add(entry.len as usize)
            .ok_or("pool range overflow")?;
        self.bytes.get(start..end).ok_or("pool range out of bounds")
    }
}

/// Wire example: little endian version:u32, count:u16, reserved:u16,
/// then count (field_key:u32, integer_bits:u64). No native layout cast.
pub fn parse_composite(bytes: &[u8]) -> Result<(u32, Vec<(u32, u64)>)> {
    if bytes.len() < 8 || bytes.len() > Pool::MAX_ENTRY {
        return Err("invalid composite size");
    }
    let version = u32::from_le_bytes(bytes[0..4].try_into().map_err(|_| "version")?);
    let count = u16::from_le_bytes(bytes[4..6].try_into().map_err(|_| "count")?) as usize;
    if count > 64 || bytes[6..8] != [0, 0] || bytes.len() != 8 + count * 12 {
        return Err("invalid composite header");
    }
    let mut values = Vec::with_capacity(count);
    let mut keys = HashSet::new();
    for record in bytes[8..].chunks_exact(12) {
        let key = u32::from_le_bytes(record[0..4].try_into().map_err(|_| "key")?);
        let value = u64::from_le_bytes(record[4..12].try_into().map_err(|_| "value")?);
        if !keys.insert(key) {
            return Err("duplicate composite key");
        }
        values.push((key, value));
    }
    Ok((version, values))
}

fn unpack_handle(payload: u64) -> Handle8 {
    Handle8 {
        slot: payload as u32,
        generation: (payload >> 32) as u32,
    }
}
