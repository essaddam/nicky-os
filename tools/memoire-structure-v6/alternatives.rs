//! Materialized alternative canonical storage, independent from Bank's Row96 arena.
#![allow(dead_code)]
use crate::layouts::*;
use crate::model::Result;
const NO_NUMERIC: u32 = u32::MAX;

pub struct Annex64 {
    pub rows: Vec<Row64>,
    pub numeric: Vec<Numeric32>,
    /// One reference per canonical row, including rows with no numeric metadata.
    pub numeric_refs: Vec<u32>,
}
impl Annex64 {
    pub fn new(rows: &[Row96], numeric_schema: &[bool]) -> Result<Self> {
        if rows.len() != numeric_schema.len() {
            return Err("numeric schema length mismatch");
        }
        let mut result = Self {
            rows: Vec::with_capacity(rows.len()),
            numeric: Vec::new(),
            numeric_refs: Vec::with_capacity(rows.len()),
        };
        for (row, is_numeric) in rows.iter().zip(numeric_schema) {
            result.rows.push(row.base);
            result.numeric_refs.push(if *is_numeric {
                let index =
                    u32::try_from(result.numeric.len()).map_err(|_| "numeric arena full")?;
                if index == NO_NUMERIC {
                    return Err("numeric sentinel exhausted");
                }
                result.numeric.push(row.numeric);
                index
            } else {
                NO_NUMERIC
            });
        }
        Ok(result)
    }
    pub fn get(&self, index: usize) -> Result<(Row64, Option<Numeric32>)> {
        let row = *self.rows.get(index).ok_or("row out of bounds")?;
        let reference = *self
            .numeric_refs
            .get(index)
            .ok_or("numeric ref out of bounds")?;
        let numeric = if reference == NO_NUMERIC {
            None
        } else {
            Some(
                *self
                    .numeric
                    .get(reference as usize)
                    .ok_or("numeric index out of bounds")?,
            )
        };
        Ok((row, numeric))
    }
}
pub struct HotCold {
    pub hot: Vec<Hot32>,
    pub cold: Vec<Cold32>,
    pub numeric: Vec<Numeric32>,
    pub numeric_refs: Vec<u32>,
}
impl HotCold {
    pub fn new(annex: Annex64) -> Self {
        let mut result = Self {
            hot: Vec::with_capacity(annex.rows.len()),
            cold: Vec::with_capacity(annex.rows.len()),
            numeric: annex.numeric,
            numeric_refs: annex.numeric_refs,
        };
        for row in annex.rows {
            result.hot.push(Hot32 {
                payload: row.payload,
                previous_payload: row.previous_payload,
                received_ms: row.received_ms,
                changed_ms: row.changed_ms,
            });
            result.cold.push(Cold32 {
                id: row.id,
                parent: row.parent,
                schema: row.schema,
                aux: row.aux,
                revision: row.revision,
                flags: row.flags,
                row_kind: row.row_kind,
                value_kind: row.value_kind,
            });
        }
        result
    }
    pub fn base(&self, index: usize) -> Result<Row64> {
        let hot = self.hot.get(index).ok_or("hot row out of bounds")?;
        let cold = self.cold.get(index).ok_or("cold row out of bounds")?;
        Ok(Row64 {
            id: cold.id,
            parent: cold.parent,
            payload: hot.payload,
            previous_payload: hot.previous_payload,
            received_ms: hot.received_ms,
            changed_ms: hot.changed_ms,
            schema: cold.schema,
            aux: cold.aux,
            revision: cold.revision,
            flags: cold.flags,
            row_kind: cold.row_kind,
            value_kind: cold.value_kind,
        })
    }
}
