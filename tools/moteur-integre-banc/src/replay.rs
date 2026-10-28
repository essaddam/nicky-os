//! Catalogue-layout adapters only. The real Hub retains its production layout.
//! Fields mirror catalogue-banc; this is not a replacement runtime implementation.
use anyhow::{Result, ensure};
use moli_core::{DeviceId, Value};
use std::mem::size_of;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Numeric32 {
    min: u64,
    max: u64,
    step: u64,
    anchor: u64,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
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
#[derive(Clone, Copy, Default)]
struct Row96 {
    base: Row64,
    numeric: Numeric32,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Hot32 {
    payload: u64,
    observed_ms: u64,
    received_ms: u64,
    changed_ms: u64,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
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

enum Rows {
    Uniform(Vec<Row96>),
    Compact(Vec<Row64>, Vec<Numeric32>),
    Columns(Vec<Hot32>, Vec<Cold32>, Vec<Numeric32>),
}
pub struct Replay {
    handles: Vec<DeviceId>,
    rows: Rows,
}

impl Replay {
    pub fn new(layout: &str, devices: usize) -> Result<Self> {
        ensure!(
            size_of::<Row96>() == 96
                && size_of::<Row64>() == 64
                && size_of::<Hot32>() == 32
                && size_of::<Cold32>() == 32
                && size_of::<Numeric32>() == 32
        );
        let mut bases = Vec::with_capacity(devices * 4);
        let mut extras = Vec::with_capacity(devices);
        for index in 0..devices {
            extras.push(Numeric32 {
                min: 0,
                max: i64::MAX.unsigned_abs(),
                step: 1,
                anchor: 0,
            });
            for slot in 0..4 {
                bases.push(Row64 {
                    id: u64::try_from(index * 4 + slot + 1)?,
                    parent: u64::try_from(index)?,
                    payload: if slot == 0 { u64::try_from(index)? } else { 0 },
                    schema: u32::try_from(slot)?,
                    aux: if slot == 0 {
                        u32::try_from(index)?
                    } else {
                        u32::MAX
                    },
                    flags: if slot == 0 { 1 } else { 2 },
                    row_kind: 2,
                    value_kind: if slot == 0 { 1 } else { 0 },
                    ..Row64::default()
                });
            }
        }
        let rows = match layout {
            "uniform96" => Rows::Uniform(
                bases
                    .into_iter()
                    .map(|base| Row96 {
                        numeric: if base.aux == u32::MAX {
                            Numeric32::default()
                        } else {
                            extras[usize::try_from(base.aux).expect("bounded index")]
                        },
                        base,
                    })
                    .collect(),
            ),
            "compact64" => Rows::Compact(bases, extras),
            "columns" => {
                let hot = bases
                    .iter()
                    .map(|base| Hot32 {
                        payload: base.payload,
                        ..Hot32::default()
                    })
                    .collect();
                let cold = bases
                    .into_iter()
                    .map(|base| Cold32 {
                        id: base.id,
                        parent: base.parent,
                        schema: base.schema,
                        aux: base.aux,
                        revision: base.revision,
                        flags: base.flags,
                        row_kind: base.row_kind,
                        value_kind: base.value_kind,
                    })
                    .collect();
                Rows::Columns(hot, cold, extras)
            }
            _ => anyhow::bail!("--layout must be uniform96, compact64 or columns"),
        };
        Ok(Self {
            handles: (0..devices).map(super::id).collect(),
            rows,
        })
    }
    pub fn bytes(&self) -> usize {
        let strings: usize = self.handles.iter().map(|id| id.as_str().len()).sum();
        strings
            + self.handles.capacity() * size_of::<DeviceId>()
            + match &self.rows {
                Rows::Uniform(rows) => rows.capacity() * size_of::<Row96>(),
                Rows::Compact(rows, extras) => {
                    rows.capacity() * size_of::<Row64>()
                        + extras.capacity() * size_of::<Numeric32>()
                }
                Rows::Columns(hot, cold, extras) => {
                    hot.capacity() * size_of::<Hot32>()
                        + cold.capacity() * size_of::<Cold32>()
                        + extras.capacity() * size_of::<Numeric32>()
                }
            }
    }
    pub fn batch(&mut self, round: usize) -> Result<Vec<(DeviceId, &'static str, Value)>> {
        let base = u64::try_from(round + 1)? * 1_000_000;
        let mut result = Vec::with_capacity(self.handles.len() + 1);
        for index in 0..self.handles.len() {
            let payload = base + u64::try_from(index)?;
            let slot = index * 4;
            let readback = match &mut self.rows {
                Rows::Uniform(rows) => {
                    rows[slot].base.payload = payload;
                    std::hint::black_box(&rows[slot]).base.payload
                }
                Rows::Compact(rows, _) => {
                    rows[slot].payload = payload;
                    std::hint::black_box(&rows[slot]).payload
                }
                Rows::Columns(hot, _, _) => {
                    hot[slot].payload = payload;
                    std::hint::black_box(&hot[slot]).payload
                }
            };
            result.push((
                self.handles[index].clone(),
                "value",
                Value::Int(i64::try_from(readback)?),
            ));
        }
        result.push((
            self.handles[0].clone(),
            "state",
            Value::Bool(round.is_multiple_of(2)),
        ));
        Ok(result)
    }
}
