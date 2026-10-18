//! Incremental hour rebuilds from raw rows, then days from hours and months from days.
//! All affected points share each scan; only one point/hour accumulator is live.
use crate::{
    Archive,
    manifest::{Manifest, Part},
    merge::ExternalMerge,
    parquet::{self, Neighbors, Record, Selection},
    parts::RecordSink,
    protocol::{CatalogUpdate, HOUR_MS, Operation, Resolution, Source},
};
use anyhow::{Context, Result, ensure};
use jiff::tz::TimeZone;
use moli_core::Value;
use moli_rollup::{
    Aggregate, Aggregation, Hour, Origin, Period, PeriodAccumulator, Quality, Rollup, Stat,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Debug, Default, Clone, Copy)]
struct Costs {
    sum: f64,
    known: u64,
    observed: u64,
}
#[derive(Debug, Serialize, Deserialize)]
struct Summary {
    point: String,
    source: Source,
    catalog_revision: i64,
    stat: Stat,
    known_cost_sum: f64,
    cost_hours_known: u64,
    hours_observed: u64,
    coverage: String,
}
fn prefix(source: Source) -> &'static str {
    match source {
        Source::Samples => "s:",
        Source::Energy => "e:",
    }
}
fn raw_group(group: &str, source: Source) -> bool {
    match source {
        Source::Samples => ["numeric", "events", "other"].contains(&group),
        Source::Energy => ["energy", "corrections"].contains(&group),
    }
}
fn floor_hour(ts: i64) -> i64 {
    ts / HOUR_MS * HOUR_MS
}
fn after_hour(ts: i64) -> Result<i64> {
    floor_hour(ts)
        .checked_add(HOUR_MS)
        .context("timestamp_overflow")
}
fn rule(m: &Manifest, id: &str, source: Source) -> Option<Rollup> {
    match source {
        Source::Samples => {
            let p = m.catalog.points.iter().find(|p| p.id == id)?;
            Some(Rollup {
                aggregation: p.aggregation?,
                freshness_ms: p.freshness_ms,
                origin: p.origin,
            })
        }
        Source::Energy => Some(Rollup {
            aggregation: Aggregation::Sum,
            freshness_ms: Some(0),
            origin: m
                .catalog
                .meters
                .iter()
                .find(|p| p.id == id)
                .map_or(Origin::Estimated, |p| p.origin),
        }),
    }
}
fn number(aggregate: Option<&Aggregate>) -> Option<f64> {
    match aggregate {
        Some(Aggregate::TimeWeighted { mean, .. }) => Some(*mean),
        Some(Aggregate::Sum { total }) => Some(*total),
        _ => None,
    }
}
fn summary_record(
    m: &Manifest,
    id: &str,
    source: Source,
    resolution: Resolution,
    stat: Stat,
    cost: Costs,
) -> Result<Record> {
    ensure!(cost.sum.is_finite(), "cost_sum_overflow");
    let covered_ms = i64::try_from(stat.covered_ms)?;
    let ts = i64::try_from(stat.start)?;
    let num = number(stat.aggregate.as_ref());
    let summary = Summary {
        point: id.into(),
        source,
        catalog_revision: m.catalog_revision,
        stat,
        known_cost_sum: cost.sum,
        cost_hours_known: cost.known,
        hours_observed: cost.observed,
        coverage: if source == Source::Energy {
            "unknown"
        } else {
            "measured"
        }
        .into(),
    };
    Ok(Record {
        entity: format!("{}{id}", prefix(source)),
        ts,
        seq: m.revision,
        kind: format!("{resolution:?}").to_ascii_lowercase(),
        payload: serde_json::to_string(&summary)?,
        num,
        cost: None,
        covered_ms,
    })
}
fn empty(start: i64, end: i64, origin: Origin) -> Result<Stat> {
    Ok(Stat {
        start: u64::try_from(start)?,
        end: u64::try_from(end)?,
        aggregate: None,
        covered_ms: 0,
        changes: 0,
        last: None,
        quality: Quality::Empty,
        origin,
    })
}
fn bounds(ts: i64, period: Period, tz: &TimeZone) -> Result<(i64, i64)> {
    let (a, b) = moli_rollup::period_bounds(u64::try_from(ts)?, period, tz)?;
    Ok((i64::try_from(a)?, i64::try_from(b)?))
}
#[allow(clippy::cast_possible_truncation)] // Protocol validates integer f64s within the exact integer range.
fn value(row: &Record) -> Result<Value> {
    match serde_json::from_str::<Operation>(&row.payload)? {
        Operation::Sample { kind, num, txt, .. } => Ok(match kind.as_str() {
            "b" => Value::Bool(num.and_then(|n| n.as_f64()) == Some(1.0)),
            "i" => {
                let n = num.context("missing_integer")?;
                Value::Int(
                    n.as_i64()
                        .unwrap_or_else(|| n.as_f64().expect("validated integer") as i64),
                )
            }
            "f" => Value::Float(num.and_then(|n| n.as_f64()).context("missing_float")?),
            "t" => Value::Text(txt.context("missing_text")?.into()),
            _ => Value::Null,
        }),
        Operation::EnergyHour { kwh, .. } => Ok(Value::Float(kwh)),
    }
}
fn selected(
    parts: &[Part],
    windows: &Selection,
    resolution: Option<Resolution>,
    source: Source,
) -> Vec<Part> {
    parts
        .iter()
        .filter(|p| {
            let group = if let Some(r) = resolution {
                p.group == "stats" && p.resolution == Some(r)
            } else {
                raw_group(&p.group, source)
            };
            group
                && windows
                    .range(p.bounds.first.clone()..=p.bounds.last.clone())
                    .any(|(_, (a, b))| p.bounds.start < *b && p.bounds.end > *a)
        })
        .cloned()
        .collect()
}
impl Archive {
    pub fn set_catalog(&mut self, update: CatalogUpdate) -> Result<crate::protocol::Status> {
        ensure!(!self.poisoned, "archive_requires_reopen");
        update.catalog.validate()?;
        ensure!(
            update.expected_revision == self.current.revision,
            "catalog_revision_conflict"
        );
        if serde_json::to_vec(&update.catalog)? == serde_json::to_vec(&self.current.catalog)? {
            return Ok(self.status());
        }
        let result = (|| {
            let mut next = (*self.current).clone();
            next.revision = next.revision.checked_add(1).context("revision_overflow")?;
            next.catalog_revision = next.revision;
            next.catalog = update.catalog;
            self.update_summaries(&mut next, None)?;
            self.compact_manifest(&mut next, false)?;
            self.publish_manifest(next)?;
            self.log
                .published(self.current.published_upto, self.current.revision)?;
            Ok(self.status())
        })();
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
    pub(crate) fn update_summaries(
        &self,
        m: &mut Manifest,
        operations: Option<&[Operation]>,
    ) -> Result<()> {
        for source in [Source::Energy, Source::Samples] {
            let mut wanted = self.affected_points(m, operations, source)?;
            if wanted.is_empty() {
                continue;
            }
            let mut raw: Vec<_> = m
                .parts
                .iter()
                .filter(|p| {
                    raw_group(&p.group, source)
                        && wanted
                            .range(p.bounds.first.clone()..=p.bounds.last.clone())
                            .next()
                            .is_some()
                })
                .cloned()
                .collect();
            raw.sort_by_key(|p| std::cmp::Reverse(p.bounds.max_seq));
            self.verify_parts(&raw)?;
            if source == Source::Samples {
                let paths: Vec<_> = raw.iter().map(|p| self.root.join(&p.file)).collect();
                parquet::neighbors(&paths, &mut wanted)?;
                let mut expanded = false;
                for window in wanted.values_mut() {
                    if let Some(anchor) = &window.anchor {
                        let start = floor_hour(anchor.ts);
                        if start < window.start {
                            window.start = start;
                            window.anchor = None;
                            expanded = true;
                        }
                    }
                }
                if expanded {
                    parquet::neighbors(&paths, &mut wanted)?;
                }
            }
            let windows: Selection = wanted
                .iter()
                .map(|(id, w)| {
                    Ok((
                        id.clone(),
                        (w.start, after_hour(w.next.unwrap_or(w.last_new))?),
                    ))
                })
                .collect::<Result<_>>()?;
            m.parts
                .extend(self.rebuild_hours(m, source, &wanted, &windows)?);
            let tz = TimeZone::get(&m.catalog.timezone)?;
            let day_windows: Selection = windows
                .iter()
                .map(|(id, (a, b))| {
                    Ok((
                        format!("{}{id}", prefix(source)),
                        (
                            bounds(*a, Period::Day, &tz)?.0,
                            bounds(b - 1, Period::Day, &tz)?.1,
                        ),
                    ))
                })
                .collect::<Result<_>>()?;
            self.calendar_summaries(m, &day_windows, source, Resolution::Day, &tz)?;
            let month_windows: Selection = day_windows
                .iter()
                .map(|(id, (a, b))| {
                    Ok((
                        id.clone(),
                        (
                            bounds(*a, Period::Month, &tz)?.0,
                            bounds(b - 1, Period::Month, &tz)?.1,
                        ),
                    ))
                })
                .collect::<Result<_>>()?;
            self.calendar_summaries(m, &month_windows, source, Resolution::Month, &tz)?;
        }
        Ok(())
    }
    fn affected_points(
        &self,
        m: &Manifest,
        operations: Option<&[Operation]>,
        source: Source,
    ) -> Result<BTreeMap<String, Neighbors>> {
        let mut wanted: BTreeMap<String, Neighbors> = BTreeMap::new();
        let mut include = |id: &str, ts: i64| {
            if rule(m, id, source).is_none() {
                return;
            }
            wanted
                .entry(id.into())
                .and_modify(|w| {
                    w.start = w.start.min(floor_hour(ts));
                    w.last_new = w.last_new.max(ts);
                })
                .or_insert(Neighbors {
                    start: floor_hour(ts),
                    last_new: ts,
                    anchor: None,
                    next: None,
                });
        };
        if let Some(operations) = operations {
            for op in operations {
                match (source, op) {
                    (Source::Samples, Operation::Sample { point, ts, .. }) => include(point, *ts),
                    (Source::Energy, Operation::EnergyHour { meter, hour, .. }) => {
                        include(meter, *hour);
                    }
                    _ => {}
                }
            }
        } else {
            let parts: Vec<_> = m
                .parts
                .iter()
                .filter(|p| raw_group(&p.group, source))
                .cloned()
                .collect();
            self.verify_parts(&parts)?;
            for p in parts {
                let mut reader = parquet::PartReader::new(&self.root.join(&p.file))?;
                while let Some(row) = reader.next_record()? {
                    include(&row.entity, row.ts);
                }
            }
        }
        Ok(wanted)
    }
    fn rebuild_hours(
        &self,
        m: &Manifest,
        source: Source,
        wanted: &BTreeMap<String, Neighbors>,
        windows: &Selection,
    ) -> Result<Vec<Part>> {
        let inputs = selected(&m.parts, windows, None, source);
        self.verify_parts(&inputs)?;
        let merge = ExternalMerge::new(
            &self.root,
            &inputs,
            Some(&Arc::new(windows.clone())),
            source == Source::Energy,
            "hours",
        )?;
        let mut cursor = Cursor::new(merge)?;
        let tag = match source {
            Source::Samples => "samples",
            Source::Energy => "energy",
        };
        let mut sink = RecordSink::new(self, "stats", tag, Some(Resolution::Hour), m.revision, 0);
        for (id, window) in windows {
            point_hours(
                m,
                source,
                id,
                *window,
                wanted.get(id).context("missing_point_window")?,
                &mut cursor,
                &mut sink,
            )?;
        }
        drop(cursor);
        sink.finish()
    }
    fn calendar_summaries(
        &self,
        m: &mut Manifest,
        windows: &Selection,
        source: Source,
        resolution: Resolution,
        tz: &TimeZone,
    ) -> Result<()> {
        let (base, period) = match resolution {
            Resolution::Day => (Resolution::Hour, Period::Day),
            Resolution::Month => (Resolution::Day, Period::Month),
            _ => anyhow::bail!("invalid_calendar_resolution"),
        };
        let inputs = selected(&m.parts, windows, Some(base), source);
        self.verify_parts(&inputs)?;
        let mut merge = ExternalMerge::new(
            &self.root,
            &inputs,
            Some(&Arc::new(windows.clone())),
            true,
            "calendar",
        )?;
        let mut next = next_summary(&mut merge, m.catalog_revision)?;
        let tag = match source {
            Source::Samples => "samples",
            Source::Energy => "energy",
        };
        let mut sink = RecordSink::new(self, "stats", tag, Some(resolution), m.revision, 0);
        for (entity, (from, to)) in windows {
            let id = entity
                .strip_prefix(prefix(source))
                .context("invalid_summary_namespace")?;
            let origin = rule(m, id, source).context("missing_rollup_rule")?.origin;
            let mut cursor = *from;
            while cursor < *to {
                let end = bounds(cursor, period, tz)?.1;
                let mut acc = PeriodAccumulator::new(period, tz.clone(), 64);
                let mut cost = Costs::default();
                while cursor < end {
                    let base_end = if base == Resolution::Hour {
                        cursor + HOUR_MS
                    } else {
                        bounds(cursor, Period::Day, tz)?.1
                    };
                    let stat = if next
                        .as_ref()
                        .is_some_and(|(r, _)| r.entity == *entity && r.ts == cursor)
                    {
                        let (_, s) = next.take().expect("checked summary");
                        ensure!(
                            s.source == source
                                && s.stat.start == u64::try_from(cursor)?
                                && s.stat.end == u64::try_from(base_end)?,
                            "invalid_summary_bounds"
                        );
                        cost.sum += s.known_cost_sum;
                        cost.known += s.cost_hours_known;
                        cost.observed += s.hours_observed;
                        next = next_summary(&mut merge, m.catalog_revision)?;
                        s.stat
                    } else {
                        empty(cursor, base_end, origin)?
                    };
                    ensure!(
                        acc.push_span(&stat)?.is_none(),
                        "unexpected_period_boundary"
                    );
                    cursor = base_end;
                }
                if let Some(stat) = acc.finish()
                    && (stat.aggregate.is_some() || stat.changes > 0)
                {
                    sink.push(&summary_record(m, id, source, resolution, stat, cost)?)?;
                }
            }
        }
        drop(merge);
        m.parts.extend(sink.finish()?);
        Ok(())
    }
}
fn next_summary(merge: &mut ExternalMerge, revision: i64) -> Result<Option<(Record, Summary)>> {
    for row in merge.by_ref() {
        let row = row?;
        let summary: Summary = serde_json::from_str(&row.payload)?;
        if summary.catalog_revision == revision {
            return Ok(Some((row, summary)));
        }
    }
    Ok(None)
}

struct Cursor {
    merge: ExternalMerge,
    next: Option<Record>,
}
impl Cursor {
    fn new(mut merge: ExternalMerge) -> Result<Self> {
        let next = merge.next().transpose()?;
        Ok(Self { merge, next })
    }
    fn pop(&mut self) -> Result<Record> {
        let row = self.next.take().context("missing_cursor_row")?;
        self.next = self.merge.next().transpose()?;
        Ok(row)
    }
}
fn save_hour(
    sink: &mut RecordSink<'_>,
    m: &Manifest,
    id: &str,
    source: Source,
    stat: Stat,
    cost: Costs,
) -> Result<Option<(u64, Value)>> {
    let carry = stat.last.clone();
    if stat.aggregate.is_some() || stat.changes > 0 {
        sink.push(&summary_record(
            m,
            id,
            source,
            Resolution::Hour,
            stat,
            cost,
        )?)?;
    }
    Ok(carry)
}
fn point_hours(
    m: &Manifest,
    source: Source,
    id: &str,
    (start, end): (i64, i64),
    neighbors: &Neighbors,
    rows: &mut Cursor,
    sink: &mut RecordSink<'_>,
) -> Result<()> {
    let rule = rule(m, id, source).context("missing_rollup_rule")?;
    let anchor = neighbors
        .anchor
        .as_ref()
        .map(|r| -> Result<_> { Ok((u64::try_from(r.ts)?, value(r)?)) })
        .transpose()?;
    let mut cursor = start;
    let mut hour = Hour::new(&rule, u64::try_from(cursor)?, anchor, 64)?;
    let mut cost = Costs::default();
    while rows.next.as_ref().is_some_and(|r| r.entity == id) {
        let row = rows.pop()?;
        while row.ts >= cursor + HOUR_MS {
            let carry = save_hour(sink, m, id, source, hour.finish()?, cost)?;
            cursor += HOUR_MS;
            hour = Hour::new(&rule, u64::try_from(cursor)?, carry, 64)?;
            cost = Costs::default();
        }
        hour.push(u64::try_from(row.ts)?, value(&row)?)?;
        if source == Source::Energy {
            cost.observed += 1;
            if let Some(value) = row.cost {
                cost.sum += value;
                cost.known += 1;
            }
        }
    }
    while cursor < end {
        let carry = save_hour(sink, m, id, source, hour.finish()?, cost)?;
        cursor += HOUR_MS;
        if cursor >= end {
            break;
        }
        hour = Hour::new(&rule, u64::try_from(cursor)?, carry, 64)?;
        cost = Costs::default();
    }
    Ok(())
}
