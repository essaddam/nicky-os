//! La charge ×2 de l'étude, rejouée sur le schéma cible avec Turso.
//!
//! Le jeu vient de `dashboard-workload-shape.json` (passé en argument, jamais
//! copié dans le dépôt) : nombre de relevés par point et par heure sur sept
//! jours, ×2 répliques, 114 compteurs, bornes de calendrier. Les valeurs sont
//! fictives, comme dans l'étude.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{Value as Json, json};
use turso::{Connection, Value};

use crate::schema::{ENERGY_UPSERT, OP_ENERGY_HOUR, OP_SAMPLE, SAMPLE_INSERT, TARGET, TOTAL_UPSERT, hour_key, key, local_day_month};
use crate::util::{R, block_on, connect, emit, exec, file_size, int, memory, ms, one, open, quantiles, real, remove_db, rows};

const HOUR: u64 = 3_600_000;
const DAY: u64 = 24 * HOUR;
/// Taille d'une transaction de chargement (comme l'essai redb de l'étude).
const LOAD_BATCH: usize = 500;

#[derive(Deserialize)]
struct Series {
    id: u64,
    hours: Vec<u64>,
    power: bool,
    event: bool,
}

#[derive(Deserialize)]
pub struct Shape {
    replicas: u64,
    series: Vec<Series>,
    base_series: u64,
    choices: HashMap<String, u64>,
    raw_fixture_epoch_ms: u64,
    raw_fixture_end_ms: u64,
    energy_meters: u64,
    energy_now_ms: u64,
    chart_days: Vec<u64>,
    chart_months: Vec<u64>,
    periods: BTreeMap<String, Vec<u64>>,
}

pub fn shape(path: &str) -> R<Shape> {
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}

fn point_name(point: u64) -> String {
    format!("fixture:{point}/value")
}

fn i(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

/// Le prix et la consommation fictifs de l'étude, par heure et compteur.
fn energy_amount(hour_ms: u64, meter: u64) -> (f64, f64) {
    let h = hour_ms / HOUR;
    let kwh = 0.02 + (meter % 11) as f64 * 0.003 + (h % 24) as f64 / 10_000.0;
    (kwh, kwh * if h % 24 < 8 { 0.16 } else { 0.25 })
}

/// Charge : le catalogue, puis les relevés dans l'ordre du temps (tous les
/// points entremêlés, comme en production), par transactions durables de 500
/// opérations (échantillon + entrée d'outbox + séquence), puis la fenêtre
/// locale d'énergie (deux mois calendaires × 114 compteurs).
pub fn load(s: &Shape, path: &str) -> R<()> {
    remove_db(path);
    let (_db, c) = open(path)?;
    exec(&c, TARGET)?;
    exec(&c, "PRAGMA synchronous = FULL")?;
    exec(&c, "BEGIN")?;
    for replica in 0..s.replicas {
        for series in &s.series {
            let family = if series.power {
                "power"
            } else if series.event {
                "event"
            } else {
                "other"
            };
            block_on(c.execute(
                "INSERT INTO catalog (id, point, family) VALUES (?1, ?2, ?3)",
                (i(series.id + replica * s.base_series), point_name(series.id + replica * s.base_series), family),
            ))?;
        }
    }
    exec(&c, "COMMIT")?;

    let started = Instant::now();
    let mut seq: i64 = 1;
    let mut in_tx = 0usize;
    let mut tx_times = Vec::new();
    let mut tx_started = Instant::now();
    let mut hour_rows: Vec<(u64, u64, bool)> = Vec::new();
    let hours = s.series.first().map_or(0, |x| x.hours.len());
    for hour in 0..hours {
        hour_rows.clear();
        for replica in 0..s.replicas {
            for series in &s.series {
                let count = series.hours[hour];
                for j in 0..count {
                    let ts = s.raw_fixture_epoch_ms + hour as u64 * HOUR + j * HOUR / count.max(1);
                    hour_rows.push((ts, series.id + replica * s.base_series, series.event));
                }
            }
        }
        hour_rows.sort_by_key(|r| r.0);
        for &(ts, point, event) in &hour_rows {
            if in_tx == 0 {
                tx_started = Instant::now();
                exec(&c, "BEGIN")?;
            }
            let added = if event {
                block_on(c.execute(
                    "INSERT INTO samples (id, point_id, ts, seq, kind, txt) VALUES ((?1 << 44) + ?2, ?1, ?2, ?3, 't', 'short_release')
                     ON CONFLICT (id) DO NOTHING",
                    (i(point), i(ts), seq),
                ))?
            } else {
                block_on(c.execute(SAMPLE_INSERT, (i(point), i(ts), seq, 100.0 + (seq % 2000) as f64 / 100.0)))?
            };
            if added == 0 {
                return Err(format!("point {point} : deux relevés à {ts} ms").into());
            }
            crate::journal::append(&c, seq, OP_SAMPLE, key(i(point), i(ts)))?;
            seq += 1;
            in_tx += 1;
            if in_tx == LOAD_BATCH {
                block_on(c.execute("UPDATE meta SET next_seq = ?1", (seq,)))?;
                exec(&c, "COMMIT")?;
                tx_times.push(ms(tx_started));
                in_tx = 0;
            }
        }
    }
    if in_tx > 0 {
        block_on(c.execute("UPDATE meta SET next_seq = ?1", (seq,)))?;
        exec(&c, "COMMIT")?;
        tx_times.push(ms(tx_started));
        in_tx = 0;
    }
    let samples_ms = ms(started);
    let samples = seq - 1;
    emit(&json!({
        "phase": "load_samples", "rows": samples, "points": s.base_series * s.replicas,
        "batch": LOAD_BATCH, "synchronous": "FULL", "elapsed_ms": samples_ms,
        "ops_per_s": samples as f64 / (samples_ms / 1000.0),
        "commit": quantiles(&mut tx_times),
        "db_bytes": file_size(path), "wal_bytes": file_size(&format!("{path}-wal")),
        "memory": memory(),
    }));

    // Fenêtre locale d'énergie : depuis le début du mois précédent, heure par
    // heure (UPSERT, séquence, outbox), totaux de jour et de mois écrits à la
    // fin (la charge est un import : le chemin vivant les tient à chaque
    // apport, voir `burst`), plus douze mois de totaux mensuels déjà là.
    let started = Instant::now();
    let from = s.periods["previous_month"][0];
    let offset = local_offset(s);
    exec(&c, "BEGIN")?;
    for meter in 0..s.energy_meters {
        block_on(c.execute(
            "INSERT INTO meter_state (id, meter, last_kwh, last_ts, first_ts) VALUES (?1, ?2, 1000.0, ?3, ?4)",
            (i(meter), meter_name(meter), i(s.energy_now_ms), i(from)),
        ))?;
    }
    exec(&c, "COMMIT")?;
    let mut energy_rows = 0u64;
    let mut totals: BTreeMap<(&str, i64, u64), (f64, f64)> = BTreeMap::new();
    let mut hour = from;
    while hour < s.energy_now_ms {
        for meter in 0..s.energy_meters {
            if in_tx == 0 {
                exec(&c, "BEGIN")?;
            }
            let (kwh, cost) = energy_amount(hour, meter);
            block_on(c.execute(ENERGY_UPSERT, (i(meter), i(hour), kwh, cost, seq)))?;
            let id = hour_key(i(meter), i(hour));
            crate::journal::append(&c, seq, OP_ENERGY_HOUR, id)?;
            let (day, month) = local_day_month(i(hour), offset);
            for k in [("day", day, meter), ("month", month, meter)] {
                let e = totals.entry(k).or_default();
                e.0 += kwh;
                e.1 += cost;
            }
            seq += 1;
            in_tx += 1;
            energy_rows += 1;
            if in_tx == LOAD_BATCH {
                block_on(c.execute("UPDATE meta SET next_seq = ?1", (seq,)))?;
                exec(&c, "COMMIT")?;
                in_tx = 0;
            }
        }
        hour += HOUR;
    }
    if in_tx == 0 {
        exec(&c, "BEGIN")?;
    }
    for (&(span, start, meter), &(kwh, cost)) in &totals {
        block_on(c.execute(TOTAL_UPSERT, (span, start, i(meter), kwh, cost)))?;
    }
    for &start in s.chart_months.iter().filter(|m| **m < from) {
        for meter in 0..s.energy_meters {
            let (kwh, cost) = energy_amount(start, meter);
            block_on(c.execute(TOTAL_UPSERT, ("month", i(start), i(meter), kwh * 720.0, cost * 720.0)))?;
        }
    }
    block_on(c.execute("UPDATE meta SET next_seq = ?1, first_energy_hour = ?2", (seq, i(from))))?;
    exec(&c, "COMMIT")?;
    emit(&json!({
        "phase": "load_energy_window", "rows": energy_rows, "meters": s.energy_meters,
        "totals": totals.len(), "window_from": from, "elapsed_ms": ms(started),
        "db_bytes": file_size(path), "wal_bytes": file_size(&format!("{path}-wal")),
        "memory": memory(),
    }));
    let t = Instant::now();
    rows(&c, "PRAGMA wal_checkpoint(TRUNCATE)", ())?;
    emit(&json!({
        "phase": "load_done", "checkpoint_ms": ms(t), "next_seq": seq,
        "db_bytes": file_size(path), "wal_bytes": file_size(&format!("{path}-wal")),
        "memory": memory(),
    }));
    Ok(())
}

#[derive(Default)]
struct Shape_ {
    scanned: u64,
    emitted: u64,
    bytes: usize,
}

fn sample_json(r: &[Value]) -> Json {
    let value = match (&r[1], &r[2], &r[3]) {
        (Value::Text(k), _, Value::Text(t)) if k == "t" => Json::from(t.clone()),
        (_, Value::Real(f), _) => Json::from(*f),
        (_, Value::Integer(n), _) => Json::from(*n),
        _ => Json::Null,
    };
    json!([int(&r[0]), value])
}

/// `moli-history::Store::series`, porté tel quel sur le schéma cible : les
/// mêmes requêtes (identifiant, valeur d'ancrage, compte, bords, puis brut ou
/// seaux en SQL), sérialisé en JSON.
fn series(c: &Connection, point: &str, from: u64, to: u64, max_points: usize) -> R<Shape_> {
    let Some(id) = rows(c, "SELECT id FROM catalog WHERE point = ?1", (point,))?.first().map(|r| int(&r[0])) else {
        return Ok(Shape_::default());
    };
    // Les requêtes de `Store::series`, la condition `point_id = ? AND ts …`
    // devenue une plage de rowid (voir `schema::key`).
    let (f, t) = (i(from), i(to));
    let (start, a, b) = (key(id, 0), key(id, f), key(id, t));
    let before = rows(c, "SELECT ts, kind, num, txt FROM samples WHERE id >= ?1 AND id < ?2 ORDER BY id DESC LIMIT 1", (start, a))?;
    let counts = rows(c, "SELECT COUNT(*), COUNT(num) FROM samples WHERE id BETWEEN ?1 AND ?2", (a, b))?;
    let (total, numeric) = (int(&counts[0][0]), int(&counts[0][1]));
    let first = rows(c, "SELECT ts, kind, num, txt FROM samples WHERE id BETWEEN ?1 AND ?2 ORDER BY id ASC LIMIT 1", (a, b))?;
    let last = rows(c, "SELECT ts, kind, num, txt FROM samples WHERE id BETWEEN ?1 AND ?2 ORDER BY id DESC LIMIT 1", (a, b))?;
    let max = i64::try_from(max_points.max(1)).unwrap_or(i64::MAX);
    let (raw, buckets): (Vec<Json>, Vec<Json>) = if total <= max {
        let r = rows(c, "SELECT ts, kind, num, txt FROM samples WHERE id BETWEEN ?1 AND ?2 ORDER BY id", (a, b))?;
        (r.iter().map(|x| sample_json(x)).collect(), Vec::new())
    } else if numeric > 0 {
        let width = ((t - f) / max).max(1);
        let r = rows(
            c,
            "SELECT MIN(ts), AVG(num), MIN(num), MAX(num), COUNT(*) FROM samples
             WHERE id BETWEEN ?1 AND ?2 AND num IS NOT NULL
             GROUP BY (ts - ?3) / ?4 ORDER BY 1",
            (a, b, f, width),
        )?;
        (
            Vec::new(),
            r.iter()
                .map(|b| json!({"ts": int(&b[0]), "avg": real(&b[1]), "min": real(&b[2]), "max": real(&b[3]), "count": int(&b[4])}))
                .collect(),
        )
    } else {
        let mut r = rows(c, "SELECT ts, kind, num, txt FROM samples WHERE id BETWEEN ?1 AND ?2 ORDER BY id DESC LIMIT ?3", (a, b, max))?;
        r.reverse();
        (r.iter().map(|x| sample_json(x)).collect(), Vec::new())
    };
    let emitted = (raw.len() + buckets.len()) as u64;
    let body = json!({
        "point": point, "from": from, "to": to, "total": total,
        "before": before.first().map(|x| sample_json(x)),
        "first": first.first().map(|x| sample_json(x)),
        "last": last.first().map(|x| sample_json(x)),
        "raw": raw, "buckets": buckets,
    });
    Ok(Shape_ { scanned: u64::try_from(total).unwrap_or(0), emitted, bytes: serde_json::to_vec(&body)?.len() })
}

fn meter_name(meter: u64) -> String {
    format!("meter-{meter:03}")
}

/// Décalage du fuseau aux bornes de l'étude (minuit local en ms UTC).
fn local_offset(s: &Shape) -> i64 {
    let d = i(DAY);
    (d - i(s.periods["today"][0]) % d) % d
}

/// Lignes `(compteur, instant, kWh, coût)` regroupées selon `bounds`, puis la
/// réponse JSON de `/api/energy/series` (méta des compteurs comprise), comme
/// l'étude.
fn energy_body(s: &Shape, bounds: &[u64], r: &[Vec<Value>]) -> R<Shape_> {
    let meters = s.energy_meters as usize;
    let mut sums = vec![(0.0f64, 0.0f64, 0u32); (bounds.len() - 1) * meters];
    for row in r {
        let m = usize::try_from(int(&row[0])).unwrap_or(0).min(meters - 1);
        let at = u64::try_from(int(&row[1])).unwrap_or(0);
        let b = bounds.partition_point(|x| *x <= at).saturating_sub(1).min(bounds.len() - 2);
        let cell = &mut sums[b * meters + m];
        cell.0 += real(&row[2]);
        cell.1 += real(&row[3]);
        cell.2 += 1;
    }
    let buckets: Vec<Json> = (0..bounds.len() - 1)
        .map(|b| {
            let m: serde_json::Map<String, Json> = (0..meters)
                .filter(|m| sums[b * meters + m].2 > 0)
                .map(|m| {
                    let v = sums[b * meters + m];
                    (meter_name(m as u64), json!({"kwh": (v.0 * 1000.0).round() / 1000.0, "cost": (v.1 * 1000.0).round() / 1000.0}))
                })
                .collect();
            json!({"start": bounds[b], "meters": m})
        })
        .collect();
    let metadata: Vec<Json> = (0..s.energy_meters)
        .map(|m| json!({"id": meter_name(m), "name": format!("Appareil {m:03}"), "role": if m < 2 {"grid"} else if m == 2 {"total"} else if m < 23 {"circuit"} else {"appliance"}, "point": format!("fixture:meter-{m:03}/energy"), "power": format!("fixture:meter-{m:03}/power")}))
        .collect();
    let body = json!({"timezone": "Europe/Paris", "currency": "EUR", "meters": metadata, "buckets": buckets});
    Ok(Shape_ { scanned: r.len() as u64, emitted: buckets.len() as u64, bytes: serde_json::to_vec(&body)?.len() })
}

/// Heures `[from, to)` : une plage de rowid (voir `schema::hour_key`).
fn hours(c: &Connection, from: u64, to: u64) -> R<Vec<Vec<Value>>> {
    rows(
        c,
        "SELECT meter_id, hour, kwh, cost FROM energy_hour WHERE id >= ?1 AND id < ?2 ORDER BY id",
        (hour_key(0, i(from)), hour_key(0, i(to))),
    )
}

/// Graphique depuis les heures.
fn energy(c: &Connection, s: &Shape, bounds: &[u64]) -> R<Shape_> {
    energy_body(s, bounds, &hours(c, bounds[0], *bounds.last().unwrap())?)
}

/// Graphique depuis les totaux de jour ou de mois (bornes = débuts de période).
fn energy_totals(c: &Connection, s: &Shape, span: &str, bounds: &[u64]) -> R<Shape_> {
    let r = rows(
        c,
        "SELECT meter_id, start, kwh, cost FROM energy_total WHERE span = ?1 AND start >= ?2 AND start < ?3 ORDER BY start",
        (span, i(bounds[0]), i(*bounds.last().unwrap())),
    )?;
    energy_body(s, bounds, &r)
}

/// Option « fenêtre locale » : `Energy::summary()` inchangé, qui charge les
/// heures depuis le début du mois précédent dans un Vec, puis additionne
/// aujourd'hui, hier, le mois et le mois précédent.
fn summary(c: &Connection, s: &Shape) -> R<(Shape_, Json)> {
    struct HourRow {
        meter: i64,
        hour: u64,
        kwh: f64,
        cost: Option<f64>,
    }
    let from = s.periods["previous_month"][0].min(s.periods["yesterday"][0]);
    let hours: Vec<HourRow> = hours(c, from, s.energy_now_ms + DAY)?
        .into_iter()
        .map(|row| HourRow {
            meter: int(&row[0]),
            hour: u64::try_from(int(&row[1])).unwrap_or(0),
            kwh: real(&row[2]),
            cost: match row[3] {
                Value::Null => None,
                ref v => Some(real(v)),
            },
        })
        .collect();
    let mut out = serde_json::Map::new();
    for name in ["today", "yesterday", "month", "previous_month"] {
        let b = &s.periods[name];
        let mut per_meter: BTreeMap<String, (f64, f64)> = BTreeMap::new();
        for h in hours.iter().filter(|h| h.hour >= b[0] && h.hour < b[1]) {
            let e = per_meter.entry(meter_name(h.meter as u64)).or_default();
            e.0 += h.kwh;
            e.1 += h.cost.unwrap_or(0.0);
        }
        out.insert(name.into(), json!(per_meter));
    }
    let bytes = serde_json::to_vec(&out)?.len();
    Ok((Shape_ { scanned: hours.len() as u64, emitted: 4, bytes }, Json::Object(out)))
}

/// Option « accumulateurs » : les quatre totaux lus tels quels.
fn summary_from_totals(c: &Connection, s: &Shape) -> R<(Shape_, Json)> {
    let mut out = serde_json::Map::new();
    let mut scanned = 0;
    for (name, span) in [("today", "day"), ("yesterday", "day"), ("month", "month"), ("previous_month", "month")] {
        let r = rows(
            c,
            "SELECT meter_id, kwh, cost FROM energy_total WHERE span = ?1 AND start = ?2",
            (span, i(s.periods[name][0])),
        )?;
        scanned += r.len() as u64;
        let per_meter: BTreeMap<String, (f64, f64)> =
            r.iter().map(|x| (meter_name(int(&x[0]) as u64), (real(&x[1]), real(&x[2])))).collect();
        out.insert(name.into(), json!(per_meter));
    }
    let bytes = serde_json::to_vec(&out)?.len();
    Ok((Shape_ { scanned, emitted: 4, bytes }, Json::Object(out)))
}

/// Deux réponses de `summary` : mêmes périodes, mêmes compteurs, mêmes
/// valeurs à 1e-9 près (l'ordre des additions peut différer).
fn same_totals(a: &Json, b: &Json) -> bool {
    let (Some(a), Some(b)) = (a.as_object(), b.as_object()) else { return false };
    a.len() == b.len()
        && a.iter().all(|(period, x)| {
            let (Some(x), Some(y)) = (x.as_object(), b.get(period).and_then(Json::as_object)) else { return false };
            x.len() == y.len()
                && x.iter().all(|(meter, v)| {
                    y.get(meter).is_some_and(|w| {
                        (0..2).all(|k| (v[k].as_f64().unwrap_or(f64::NAN) - w[k].as_f64().unwrap_or(f64::NAN)).abs() < 1e-9)
                    })
                })
        })
}

/// Un lot d'export du contrat : `n` opérations consécutives depuis `after`,
/// échantillons et heures d'énergie, autonomes (point ou compteur nommé),
/// encodées en NDJSON.
pub(crate) fn export_ndjson(c: &Connection, after: i64, n: i64) -> R<Vec<u8>> {
    crate::journal::body(c, after, n)
}

fn export_batch(c: &Connection, after: i64, n: i64) -> R<Shape_> {
    let body = export_ndjson(c, after, n)?;
    let count = body.iter().filter(|b| **b == b'\n').count() as u64;
    Ok(Shape_ { scanned: count, emitted: count, bytes: body.len() })
}

fn bench(name: &str, n: usize, mut action: impl FnMut() -> R<Shape_>) -> R<()> {
    let t = Instant::now();
    let first = action()?;
    let first_ms = ms(t);
    let mut times = Vec::with_capacity(n);
    let mut last = first;
    for _ in 0..n {
        let t = Instant::now();
        last = action()?;
        times.push(ms(t));
    }
    let mut out = quantiles(&mut times);
    out["name"] = name.into();
    out["first_ms"] = first_ms.into();
    out["rows_scanned"] = last.scanned.into();
    out["values_returned"] = last.emitted.into();
    out["json_bytes"] = last.bytes.into();
    emit(&out);
    Ok(())
}

fn hourly(s: &Shape, hours: u64) -> Vec<u64> {
    (0..=hours).map(|k| s.energy_now_ms - (hours - k) * HOUR).collect()
}

/// Les lectures du dashboard, mêmes cas et mêmes répétitions que l'étude.
pub fn reads(s: &Shape, path: &str) -> R<()> {
    let (_db, c) = open(path)?;
    let end = s.raw_fixture_end_ms;
    let cases: [(&str, u64, u64, usize); 6] = [
        ("plug_24h_stable", s.choices["stable_power"], 24, 300),
        ("plug_24h_median", s.choices["median_power"], 24, 300),
        ("plug_24h_busy", s.choices["busy_power"], 24, 300),
        ("plug_7d_busy", s.choices["busy_power"], 168, 2000),
        ("plug_24h_worst_point", s.choices["busiest_any"], 24, 300),
        ("security_720h_events", s.series.iter().filter(|x| x.event).max_by_key(|x| x.hours.iter().sum::<u64>()).map_or(1, |x| x.id), 720, 5000),
    ];
    for (name, point, hours, max) in cases {
        let p = point_name(point);
        bench(name, 100, || series(&c, &p, end - hours * HOUR, end, max))?;
    }
    let mut power: Vec<&Series> = s.series.iter().filter(|x| x.power).collect();
    power.sort_by_key(|x| std::cmp::Reverse(x.hours.iter().sum::<u64>()));
    let chosen: Vec<String> = power.iter().take(20).map(|x| point_name(x.id)).collect();
    bench("dashboard_20_sparklines", 50, || {
        let mut out = Shape_::default();
        for p in &chosen {
            let r = series(&c, p, end - DAY, end, 90)?;
            out.scanned += r.scanned;
            out.emitted += r.emitted;
            out.bytes += r.bytes;
        }
        Ok(out)
    })?;
    bench("energy_24h_hourly", 50, || energy(&c, s, &hourly(s, 24)))?;
    bench("energy_168h_hourly", 30, || energy(&c, s, &hourly(s, 168)))?;
    bench("energy_30d_daily_from_hours", 30, || energy(&c, s, &s.chart_days))?;
    bench("energy_30d_daily_from_totals", 100, || energy_totals(&c, s, "day", &s.chart_days))?;
    bench("energy_12m_monthly_from_totals", 100, || energy_totals(&c, s, "month", &s.chart_months))?;
    bench("energy_summary_two_month_window", 30, || summary(&c, s).map(|x| x.0))?;
    bench("energy_summary_from_totals", 100, || summary_from_totals(&c, s).map(|x| x.0))?;
    let (window, totals) = (summary(&c, s)?.1, summary_from_totals(&c, s)?.1);
    let totals_pass = same_totals(&window, &totals) && window["today"].as_object().is_some_and(|m| !m.is_empty());
    emit(&json!({"test": "summary_window_equals_totals", "pass": totals_pass}));
    if !totals_pass { return Err("les résumés d'énergie divergent".into()) }
    let next = one(&c, "SELECT next_seq FROM meta", ())?;
    let first_energy = one(&c, "SELECT min(seq) FROM outbox WHERE kind = 2", ())?;
    let sample_rows = first_energy - 1;
    let energy_rows = next - first_energy;
    if sample_rows < 5_000 || energy_rows < 5_000 {
        return Err("le jeu doit couvrir un lot complet de chaque famille".into());
    }
    let mut offset = 0i64;
    bench("export_samples_batch_5000_ops", 20, || {
        offset = (offset + 104_729) % (sample_rows - 4_999);
        export_batch(&c, offset, 5_000)
    })?;
    let mut offset = 0i64;
    bench("export_energy_batch_5000_ops", 20, || {
        offset = (offset + 7_919) % (energy_rows - 4_999);
        export_batch(&c, first_energy - 1 + offset, 5_000)
    })?;
    bench("export_mixed_batch_5000_ops", 20, || export_batch(&c, first_energy - 2_501, 5_000))?;
    let count_types = |after: i64| -> R<(usize, usize)> {
        let body = export_ndjson(&c, after, 5_000)?;
        let (mut samples, mut energy) = (0, 0);
        for line in body.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
            let operation: Json = serde_json::from_slice(line)?;
            match operation["type"].as_str() {
                Some("sample") => samples += 1,
                Some("energy_hour") => energy += 1,
                _ => return Err("opération d'export inconnue".into()),
            }
        }
        Ok((samples, energy))
    };
    let sample_batch = count_types(0)?;
    let energy_batch = count_types(first_energy - 1)?;
    let mixed_batch = count_types(first_energy - 2_501)?;
    let coverage = sample_batch == (5_000, 0) && energy_batch == (0, 5_000) && mixed_batch == (2_500, 2_500);
    emit(&json!({"test": "export_batches_cover_samples_and_energy", "pass": coverage,
                 "samples_only": sample_batch, "energy_only": energy_batch, "mixed": mixed_batch}));
    if !coverage { return Err("couverture des lots d'export incorrecte".into()) }
    emit(&json!({"phase": "reads_done", "memory": memory()}));
    Ok(())
}

/// Cinq lecteurs (courbe 24 h du point actif, 300 points, 100 fois chacun,
/// 50 ms entre deux) pendant qu'un rédacteur valide `rate` opérations par
/// seconde (lots toutes les 100 ms, durables), comme l'étude.
pub fn burst(s: &Shape, path: &str, rate: u64) -> R<()> {
    let (db, c) = open(path)?;
    let stop = Arc::new(AtomicBool::new(false));
    let written = Arc::new(AtomicU64::new(0));
    let end = s.raw_fixture_end_ms;
    let busy = point_name(s.choices["busy_power"]);
    let points = s.base_series * s.replicas;
    let mut seq = one(&c, "SELECT next_seq FROM meta", ())?;
    let first_seq = seq;
    drop(c);
    let writer = {
        let (db, stop, written) = (db.clone(), stop.clone(), written.clone());
        let busy_id = i(s.choices["busy_power"]);
        let meters = s.energy_meters;
        let hour = i(s.energy_now_ms - HOUR);
        let offset = local_offset(s);
        thread::spawn(move || -> R<Vec<f64>> {
            let w = connect(&db)?;
            exec(&w, "PRAGMA synchronous = FULL")?;
            let per_tick = rate.div_ceil(10).max(1);
            let tick = Duration::from_secs_f64(per_tick as f64 / rate as f64);
            let mut commits = Vec::new();
            while !stop.load(Ordering::Relaxed) {
                let t = Instant::now();
                exec(&w, "BEGIN IMMEDIATE")?;
                for k in 0..per_tick {
                    if seq % 10 == 0 {
                        // Le chemin vivant : l'heure, puis le même apport sur
                        // les totaux du jour et du mois, dans la transaction.
                        let meter = i(seq as u64 % meters);
                        block_on(w.execute(ENERGY_UPSERT, (meter, hour, 0.001, Some(0.0002), seq)))?;
                        let id = hour_key(meter, hour);
                        let (day, month) = local_day_month(hour, offset);
                        block_on(w.execute(TOTAL_UPSERT, ("day", day, meter, 0.001, Some(0.0002))))?;
                        block_on(w.execute(TOTAL_UPSERT, ("month", month, meter, 0.001, Some(0.0002))))?;
                        crate::journal::append(&w, seq, OP_ENERGY_HOUR, id)?;
                        block_on(w.execute(
                            "UPDATE meter_state SET last_kwh = last_kwh + 0.001, last_ts = ?2 WHERE id = ?1",
                            (meter, i(end)),
                        ))?;
                    } else {
                        // Après la fin du jeu, une milliseconde par opération :
                        // jamais deux fois la même clé.
                        let point = if k % 10 == 1 { busy_id } else { 1 + seq % i(points) };
                        let ts = i(end) + seq - first_seq;
                        block_on(w.execute(SAMPLE_INSERT, (point, ts, seq, 200.0)))?;
                        crate::journal::append(&w, seq, OP_SAMPLE, key(point, ts))?;
                    }
                    seq += 1;
                }
                block_on(w.execute("UPDATE meta SET next_seq = ?1", (seq,)))?;
                exec(&w, "COMMIT")?;
                commits.push(ms(t));
                written.fetch_add(per_tick, Ordering::Relaxed);
                if let Some(rest) = tick.checked_sub(t.elapsed()) {
                    thread::sleep(rest);
                }
            }
            Ok(commits)
        })
    };
    let started = Instant::now();
    let readers: Vec<_> = (0..5)
        .map(|_| {
            let (db, p) = (db.clone(), busy.clone());
            thread::spawn(move || -> R<Vec<f64>> {
                let c = connect(&db)?;
                let mut times = Vec::new();
                for _ in 0..100 {
                    let t = Instant::now();
                    series(&c, &p, end - DAY, end, 300)?;
                    times.push(ms(t));
                    thread::sleep(Duration::from_millis(50));
                }
                Ok(times)
            })
        })
        .collect();
    let mut times = Vec::new();
    for r in readers {
        times.extend(r.join().map_err(|_| "reader panicked")??);
    }
    stop.store(true, Ordering::Relaxed);
    let mut commits = writer.join().map_err(|_| "writer panicked")??;
    let elapsed = started.elapsed().as_secs_f64();
    let n = written.load(Ordering::Relaxed);
    let mut out = quantiles(&mut times);
    out["name"] = format!("five_readers_with_{rate}_writes_per_s").into();
    out["writes_committed"] = n.into();
    out["achieved_writes_per_s"] = (n as f64 / elapsed).into();
    out["elapsed_s"] = elapsed.into();
    out["commit"] = quantiles(&mut commits);
    out["memory"] = memory();
    emit(&out);
    Ok(())
}

/// Purge d'un jour de relevés (1/7 du récent) une fois l'archive confirmée :
/// par point, en gardant la valeur d'ancrage juste avant la limite, une
/// transaction par point ; puis nettoyage de l'outbox par tranches.
pub fn purge(s: &Shape, path: &str) -> R<()> {
    let (_db, c) = open(path)?;
    let next = one(&c, "SELECT next_seq FROM meta", ())?;
    let published = next - 1;
    let acknowledged_at = i(s.raw_fixture_end_ms);
    let ack_started = Instant::now();
    let mut upto = 0;
    while upto < published {
        upto = (upto + 5_000).min(published);
        crate::journal::acknowledge(&c, 1, upto, upto, acknowledged_at)?;
    }
    let ack_ms = ms(ack_started);
    let pending_after_ack = one(&c, "SELECT count(*) FROM outbox", ())?;
    let protected_deleted = crate::journal::prune(&c, acknowledged_at, 10_000)?;
    let retained = one(&c, "SELECT count(*) FROM export_journal", ())?;
    let cutoff = i(s.raw_fixture_end_ms - 6 * DAY);
    let before_bytes = file_size(path);
    let points: Vec<i64> = rows(&c, "SELECT id FROM catalog ORDER BY id", ())?.iter().map(|r| int(&r[0])).collect();
    let started = Instant::now();
    let mut deleted = 0u64;
    let mut anchored = 0i64;
    let mut tx_times = Vec::new();
    for &p in &points {
        let t = Instant::now();
        exec(&c, "BEGIN IMMEDIATE")?;
        let anchor = rows(&c, "SELECT id FROM samples WHERE id >= ?1 AND id < ?2 ORDER BY id DESC LIMIT 1", (key(p, 0), key(p, cutoff)))?;
        if let Some(anchor) = anchor.first().map(|r| int(&r[0])) {
            anchored += 1;
            deleted += block_on(c.execute(
                "DELETE FROM samples WHERE id >= ?1 AND id < ?2 AND seq <= ?3",
                (key(p, 0), anchor, published),
            ))?;
        }
        exec(&c, "COMMIT")?;
        tx_times.push(ms(t));
    }
    let samples_ms = ms(started);
    // Ancrage : chaque point qui avait une valeur avant la limite en garde
    // exactement une, la dernière.
    let left = rows(
        &c,
        "SELECT (SELECT count(*) FROM samples s WHERE s.id >= (c.id << 44) AND s.id < (c.id << 44) + ?1) FROM catalog c",
        (cutoff,),
    )?;
    let with_anchor = left.iter().filter(|r| int(&r[0]) == 1).count() as i64;
    let with_extra = left.iter().filter(|r| int(&r[0]) > 1).count();
    let started = Instant::now();
    let mut journal_times = Vec::new();
    let mut journal_deleted = 0u64;
    loop {
        let t = Instant::now();
        let deleted = crate::journal::prune(&c, acknowledged_at + crate::journal::REPLAY_WINDOW_MS + 1, 10_000)?;
        if deleted == 0 { break }
        journal_deleted += deleted;
        journal_times.push(ms(t));
    }
    let journal_ms = ms(started);
    let floor = one(&c, "SELECT replay_floor FROM archive_ack", ())?;
    let journal_pass = pending_after_ack == 0 && protected_deleted == 0 && retained == published
        && journal_deleted as i64 == published && floor == published;
    let anchors_pass = with_anchor == anchored && with_extra == 0;
    let t = Instant::now();
    rows(&c, "PRAGMA wal_checkpoint(TRUNCATE)", ())?;
    emit(&json!({
        "phase": "purge",
        "samples_deleted": deleted, "points": points.len(), "samples_ms": samples_ms,
        "per_point_tx": quantiles(&mut tx_times),
        "anchors": {"points_with_data_before_cutoff": anchored, "kept_exactly_one": with_anchor, "kept_more": with_extra, "pass": anchors_pass},
        "outbox_deleted": published, "outbox_ms": ack_ms,
        "journal_deleted": journal_deleted, "journal_ms": journal_ms, "journal_chunk_10k": quantiles(&mut journal_times),
        "journal_retention": {"pass": journal_pass, "pending_after_ack": pending_after_ack,
            "retained_after_ack": retained, "deleted_before_window": protected_deleted, "replay_floor": floor},
        "checkpoint_ms": ms(t),
        "db_bytes_before": before_bytes, "db_bytes_after": file_size(path),
        "memory": memory(),
    }));
    if !anchors_pass || !journal_pass { return Err("ancrages ou rétention du journal incorrects".into()) }
    Ok(())
}

/// Réouverture : ouverture, première lecture, première courbe ; puis
/// `integrity_check` complet sur le fichier ×2.
pub fn reopen(s: &Shape, path: &str) -> R<()> {
    let t = Instant::now();
    let (_db, c) = open(path)?;
    let open_ms = ms(t);
    let next = one(&c, "SELECT next_seq FROM meta", ())?;
    let first_query_ms = ms(t);
    let curve = series(&c, &point_name(s.choices["busy_power"]), s.raw_fixture_end_ms - DAY, s.raw_fixture_end_ms, 300)?;
    let first_curve_ms = ms(t);
    let mem = memory();
    let t = Instant::now();
    let integrity = rows(&c, "PRAGMA integrity_check", ())?;
    let integrity = match integrity.first().and_then(|r| r.first()) {
        Some(Value::Text(value)) => value.clone(),
        other => format!("{other:?}"),
    };
    emit(&json!({
        "phase": "reopen", "open_ms": open_ms, "first_query_ms": first_query_ms, "first_curve_ms": first_curve_ms,
        "curve_rows": curve.scanned, "next_seq": next, "memory_after_first_curve": mem,
        "integrity": integrity, "integrity_ms": ms(t),
        "db_bytes": file_size(path), "memory": memory(),
    }));
    if integrity != "ok" { return Err("vérification d'intégrité incorrecte".into()) }
    Ok(())
}

#[cfg(test)]
mod review_regressions {
    use super::*;
    use crate::schema::TARGET;

    fn fixture(label: &str) -> R<(turso::Database, Connection, String)> {
        let path = format!("/tmp/moli-bench-{label}-{}.db", std::process::id());
        remove_db(&path);
        let (db, c) = open(&path)?;
        exec(&c, TARGET)?;
        exec(&c, "INSERT INTO meter_state (id, meter) VALUES (0, 'meter-test')")?;
        Ok((db, c, path))
    }

    fn write_hour(c: &Connection, seq: i64, kwh: f64) -> R<()> {
        exec(c, "BEGIN IMMEDIATE")?;
        block_on(c.execute(ENERGY_UPSERT, (0i64, 0i64, kwh, Some(kwh * 0.2), seq)))?;
        crate::journal::append(c, seq, OP_ENERGY_HOUR, 0)?;
        block_on(c.execute("UPDATE meta SET next_seq = ?1", (seq + 1,)))?;
        exec(c, "COMMIT")
    }

    #[test]
    fn retry_keeps_the_original_energy_operation() -> R<()> {
        let (db, c, path) = fixture("retry")?;
        write_hour(&c, 1, 1.0)?;
        let first = export_ndjson(&c, 0, 1)?;
        write_hour(&c, 2, 1.0)?;
        drop(c);
        drop(db);
        let (_db, c) = open(&path)?;
        assert_eq!(first, export_ndjson(&c, 0, 1)?, "le même lot change après une mise à jour et une réouverture");
        Ok(())
    }

    #[test]
    fn published_operations_can_replay_after_outbox_is_drained() -> R<()> {
        let (_db, c, _path) = fixture("restore")?;
        write_hour(&c, 1, 1.0)?;
        let first = export_ndjson(&c, 0, 1)?;
        write_hour(&c, 2, 1.0)?;
        let second = export_ndjson(&c, 1, 1)?;
        exec(&c, "DELETE FROM outbox; DELETE FROM energy_hour")?;
        let mut expected = first;
        expected.extend(second);
        assert_eq!(expected, export_ndjson(&c, 0, 2)?, "les tables courantes ne reconstruisent pas le journal publié");
        Ok(())
    }
}
