//! Throwaway semantic/structure study. No production temporal mean is claimed.
use std::{collections::VecDeque, hint::black_box, time::Instant};
const WINDOWS: [u64; 4] = [60_000, 300_000, 3_600_000, 86_400_000];
const RETAIN: u64 = WINDOWS[3];
const FRESH: u64 = 60_000;
const WIDTH: u64 = 10_000;
#[derive(Clone, Copy, Debug, Default)]
struct Sample {
    at: u64,
    value: Option<f64>,
}
#[derive(Clone, Copy, Debug, Default)]
struct Mean {
    value: f64,
    weight: u64,
}
impl Mean {
    fn add(&mut self, x: f64, weight: u64) {
        if weight == 0 {
            return;
        }
        let total = self.weight + weight;
        self.value = if self.weight == 0 {
            x
        } else {
            // Convex combination avoids value * milliseconds overflow.
            let y = self.value * (self.weight as f64 / total as f64)
                + x * (weight as f64 / total as f64);
            y.clamp(self.value.min(x), self.value.max(x))
        };
        self.weight = total;
    }
    fn get(self) -> Option<f64> {
        (self.weight > 0).then_some(self.value)
    }
}
#[derive(Clone, Copy, Debug, Default)]
struct Answer {
    samples: Mean,
    time: Mean,
    limited: bool,
}
trait Store {
    fn push(&mut self, s: Sample) -> Result<(), &'static str>;
    fn query(&self, now: u64) -> [Answer; 4];
    fn heap(&self) -> usize;
    fn retained(&self) -> usize;
}
#[derive(Clone)]
enum Rows {
    Deque(VecDeque<Sample>),
    Ring {
        slots: Vec<Sample>,
        head: usize,
        len: usize,
    },
}
impl Rows {
    fn len(&self) -> usize {
        match self {
            Self::Deque(d) => d.len(),
            Self::Ring { len, .. } => *len,
        }
    }
    fn get(&self, i: usize) -> Sample {
        match self {
            Self::Deque(d) => d[i],
            Self::Ring { slots, head, .. } => slots[(head + i) % slots.len()],
        }
    }
    fn set_last(&mut self, s: Sample) {
        let i = self.len() - 1;
        match self {
            Self::Deque(d) => d[i] = s,
            Self::Ring { slots, head, .. } => {
                let j = (*head + i) % slots.len();
                slots[j] = s;
            }
        }
    }
    fn pop(&mut self) {
        match self {
            Self::Deque(d) => {
                d.pop_front();
            }
            Self::Ring { slots, head, len } => {
                *head = (*head + 1) % slots.len();
                *len -= 1;
            }
        }
    }
    fn append(&mut self, s: Sample) -> bool {
        match self {
            Self::Deque(d) => {
                d.push_back(s);
                false
            }
            Self::Ring { slots, head, len } => {
                let lost = *len == slots.len();
                if lost {
                    *head = (*head + 1) % slots.len();
                    *len -= 1;
                }
                let index = (*head + *len) % slots.len();
                slots[index] = s;
                *len += 1;
                lost
            }
        }
    }
}
#[derive(Clone)]
struct Exact {
    rows: Rows,
    lost_until: u64,
}
impl Exact {
    fn deque() -> Self {
        Self {
            rows: Rows::Deque(VecDeque::new()),
            lost_until: 0,
        }
    }
    fn ring(n: usize) -> Self {
        assert!(n > 1);
        Self {
            rows: Rows::Ring {
                slots: vec![Sample::default(); n],
                head: 0,
                len: 0,
            },
            lost_until: 0,
        }
    }
}
fn cleaned(s: Sample) -> Sample {
    Sample {
        at: s.at,
        value: s.value.filter(|v| v.is_finite()),
    }
}
fn add_segment(out: &mut [Answer; 4], from: u64, to: u64, value: f64, now: u64) {
    for (i, window) in WINDOWS.iter().enumerate() {
        let duration = to
            .min(now)
            .saturating_sub(from.max(now.saturating_sub(*window)));
        out[i].time.add(value, duration);
    }
}
impl Store for Exact {
    fn push(&mut self, s: Sample) -> Result<(), &'static str> {
        let s = cleaned(s);
        if self.rows.len() > 0 {
            let last = self.rows.get(self.rows.len() - 1);
            if s.at < last.at {
                return Err("out_of_order");
            }
            if s.at == last.at {
                self.rows.set_last(s);
                return Ok(());
            }
        }
        while self.rows.len() > 1 && self.rows.get(1).at < s.at.saturating_sub(RETAIN) {
            self.rows.pop();
        }
        if self.rows.append(s) {
            self.lost_until = self.rows.get(0).at;
        }
        Ok(())
    }
    fn query(&self, now: u64) -> [Answer; 4] {
        let mut out = [Answer::default(); 4];
        for (i, w) in WINDOWS.iter().enumerate() {
            out[i].limited = self.lost_until > now.saturating_sub(*w);
        }
        for j in 0..self.rows.len() {
            let s = self.rows.get(j);
            if s.at > now {
                continue;
            }
            if let Some(v) = s.value {
                for (i, w) in WINDOWS.iter().enumerate() {
                    if s.at >= now.saturating_sub(*w) && s.at < now {
                        out[i].samples.add(v, 1);
                    }
                }
                let to = if j + 1 < self.rows.len() {
                    self.rows.get(j + 1).at
                } else {
                    now
                };
                add_segment(&mut out, s.at, to.min(s.at.saturating_add(FRESH)), v, now);
            }
        }
        out
    }
    fn heap(&self) -> usize {
        match &self.rows {
            Rows::Deque(d) => d.capacity() * std::mem::size_of::<Sample>(),
            Rows::Ring { slots, .. } => slots.capacity() * std::mem::size_of::<Sample>(),
        }
    }
    fn retained(&self) -> usize {
        self.rows.len()
    }
}
#[derive(Clone, Copy, Debug, Default)]
struct Bucket {
    tag: Option<u64>,
    samples: Mean,
    time: Mean,
}
struct Buckets {
    slots: Vec<Bucket>,
    last: Option<Sample>,
}
impl Buckets {
    fn new() -> Self {
        Self {
            slots: vec![Bucket::default(); (RETAIN / WIDTH + 2) as usize],
            last: None,
        }
    }
    fn bucket(&mut self, key: u64) -> &mut Bucket {
        let n = self.slots.len();
        let b = &mut self.slots[key as usize % n];
        if b.tag != Some(key) {
            *b = Bucket {
                tag: Some(key),
                ..Bucket::default()
            };
        }
        b
    }
    fn close(&mut self, s: Sample, to: u64) {
        if let Some(v) = s.value {
            let end = to.min(s.at.saturating_add(FRESH));
            let mut at = s.at;
            // Long gaps do not create empty buckets.
            while at < end {
                let key = at / WIDTH;
                let next = end.min((key + 1) * WIDTH);
                self.bucket(key).time.add(v, next - at);
                at = next;
            }
        }
    }
}
impl Store for Buckets {
    fn push(&mut self, s: Sample) -> Result<(), &'static str> {
        let s = cleaned(s);
        if let Some(last) = self.last {
            if s.at < last.at {
                return Err("out_of_order");
            }
            if s.at != last.at {
                self.close(last, s.at);
                if let Some(v) = last.value {
                    self.bucket(last.at / WIDTH).samples.add(v, 1);
                }
            }
        }
        self.last = Some(s);
        Ok(())
    }
    fn query(&self, now: u64) -> [Answer; 4] {
        let mut out = [Answer::default(); 4];
        for b in &self.slots {
            let Some(key) = b.tag else { continue };
            let a = key * WIDTH;
            let z = a + WIDTH;
            for (i, w) in WINDOWS.iter().enumerate() {
                let lo = now.saturating_sub(*w);
                let overlap = z.min(now).saturating_sub(a.max(lo));
                if overlap == 0 {
                    continue;
                }
                // Sample membership of the left/right partial bucket is approximate.
                if b.samples.weight > 0 {
                    out[i].samples.add(b.samples.value, b.samples.weight);
                }
                let covered = ((b.time.weight as u128 * overlap as u128) / WIDTH as u128) as u64;
                out[i].time.add(b.time.value, covered);
            }
        }
        if let Some(s) = self.last
            && let Some(v) = s.value
        {
            for (i, w) in WINDOWS.iter().enumerate() {
                if s.at >= now.saturating_sub(*w) && s.at < now {
                    out[i].samples.add(v, 1);
                }
            }
            add_segment(&mut out, s.at, now.min(s.at.saturating_add(FRESH)), v, now);
        }
        out
    }
    fn heap(&self) -> usize {
        self.slots.capacity() * std::mem::size_of::<Bucket>()
    }
    fn retained(&self) -> usize {
        self.slots.iter().filter(|b| b.tag.is_some()).count()
    }
}
// Independent offline oracle: collect elementary intersections, then use scaled
// compensated sums. Does not call Store query, Mean::add, or add_segment.
fn oracle(events: &[Sample], now: u64) -> [Answer; 4] {
    let mut out = [Answer::default(); 4];
    for (i, w) in WINDOWS.iter().enumerate() {
        let lo = now.saturating_sub(*w);
        let mut samples = Vec::new();
        let mut spans = Vec::new();
        for (j, s) in events.iter().enumerate() {
            if s.at >= now {
                break;
            }
            let Some(v) = s.value.filter(|v| v.is_finite()) else {
                continue;
            };
            if s.at >= lo {
                samples.push((v, 1));
            }
            let end = events
                .get(j + 1)
                .map_or(now, |n| n.at)
                .min(now)
                .min(s.at.saturating_add(FRESH));
            let length = end.saturating_sub(s.at.max(lo));
            if length > 0 {
                spans.push((v, length));
            }
        }
        fn sum(values: &[(f64, u64)]) -> Mean {
            let weight = values.iter().map(|v| v.1).sum::<u64>();
            if weight == 0 {
                return Mean::default();
            }
            let scale = values.iter().map(|v| v.0.abs()).fold(0.0, f64::max);
            if scale == 0.0 {
                return Mean { value: 0.0, weight };
            }
            let (mut total, mut correction) = (0.0, 0.0);
            for &(v, n) in values {
                let y = (v / scale) * (n as f64 / weight as f64) - correction;
                let t = total + y;
                correction = (t - total) - y;
                total = t;
            }
            Mean {
                value: scale * total.clamp(-1.0, 1.0),
                weight,
            }
        }
        out[i].samples = sum(&samples);
        out[i].time = sum(&spans);
    }
    out
}
fn generate(kind: &str) -> Vec<Sample> {
    let mut data = Vec::new();
    let mut at = 0;
    let mut i = 0;
    while at <= RETAIN {
        let value = if i % 193 == 192 {
            None
        } else {
            Some(100.0 + (i % 101) as f64 * 2.5)
        };
        data.push(Sample { at, value });
        at += match kind {
            "sparse" => 30_000,
            "burst" => 100,
            _ => [500, 1500, 3000, 2000, 1000][i % 5],
        };
        i += 1;
        if kind == "burst" && i >= 100_000 {
            break;
        }
    }
    data
}
fn build(kind: &str) -> Box<dyn Store> {
    match kind {
        "deque" => Box::new(Exact::deque()),
        "ring" => Box::new(Exact::ring(65_536)),
        "ring_small" => Box::new(Exact::ring(2048)),
        "buckets_10s" => Box::new(Buckets::new()),
        _ => panic!("unknown backend"),
    }
}
fn abs_error(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    match (a, b) {
        (Some(a), Some(b)) => Some((a - b).abs()),
        (None, None) => Some(0.0),
        _ => None,
    }
}
fn json_num(n: Option<f64>) -> String {
    n.filter(|n| n.is_finite())
        .map_or("null".into(), |n| format!("{n:.12}"))
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let backend = args.get(1).map_or("deque", String::as_str);
    let workload = args.get(2).map_or("irregular", String::as_str);
    assert!(["irregular", "sparse", "burst"].contains(&workload));
    let data = generate(workload);
    let now = data.last().unwrap().at + 1234;
    let reference = oracle(&data, now);
    let mut ingest = Vec::new();
    let mut query = Vec::new();
    let mut measured = build(backend);
    for round in 0..8 {
        let mut store = build(backend);
        let start = Instant::now();
        for s in &data {
            store.push(black_box(*s)).unwrap();
        }
        let ing = start.elapsed().as_nanos() as f64 / data.len() as f64;
        let start = Instant::now();
        for _ in 0..128 {
            black_box(store.query(black_box(now)));
        }
        let qry = start.elapsed().as_nanos() as f64 / 128.0;
        if round > 0 {
            ingest.push(ing);
            query.push(qry);
        }
        measured = store;
    }
    ingest.sort_by(f64::total_cmp);
    query.sort_by(f64::total_cmp);
    let answers = measured.query(now);
    println!(
        "{{\"status\":\"prototype_not_product\",\"backend\":\"{backend}\",\"workload\":\"{workload}\",\"rounds\":7,\"input_events\":{},\"span_ms\":{now},\"retained_items\":{},\"heap_capacity_bytes\":{},\"sample_layout_bytes\":{},\"bucket_layout_bytes\":{},\"ingest_ns_per_event_median\":{:.3},\"query_4windows_ns_median\":{:.3},\"ingest_ns_rounds\":{:?},\"query_ns_rounds\":{:?},\"windows\":[",
        data.len(),
        measured.retained(),
        measured.heap(),
        std::mem::size_of::<Sample>(),
        std::mem::size_of::<Bucket>(),
        ingest[3],
        query[3],
        ingest,
        query
    );
    for i in 0..4 {
        let a = answers[i];
        let r = reference[i];
        println!(
            "{}{{\"window_ms\":{},\"sample_mean\":{},\"oracle_sample_mean\":{},\"sample_abs_error\":{},\"time_mean\":{},\"oracle_time_mean\":{},\"time_abs_error\":{},\"covered_ms\":{},\"oracle_covered_ms\":{},\"unknown_ms\":{},\"limited\":{}}}",
            if i == 0 { "" } else { "," },
            WINDOWS[i],
            json_num(a.samples.get()),
            json_num(r.samples.get()),
            json_num(abs_error(a.samples.get(), r.samples.get())),
            json_num(a.time.get()),
            json_num(r.time.get()),
            json_num(abs_error(a.time.get(), r.time.get())),
            a.time.weight,
            r.time.weight,
            WINDOWS[i].min(now).saturating_sub(a.time.weight),
            a.limited
        );
    }
    println!("]}}");
}
#[cfg(test)]
mod tests {
    use super::*;
    fn s(at: u64, value: f64) -> Sample {
        Sample {
            at,
            value: Some(value),
        }
    }
    fn close(a: Option<f64>, b: Option<f64>) {
        match (a, b) {
            (Some(a), Some(b)) => assert!(
                (a - b).abs() <= a.abs().max(b.abs()).max(1.0) * 1e-10,
                "{a} vs {b}"
            ),
            (None, None) => (),
            _ => panic!("{a:?} vs {b:?}"),
        }
    }
    fn exacts(data: &[Sample], now: u64) {
        let r = oracle(data, now);
        for mut x in [Exact::deque(), Exact::ring(data.len() + 2)] {
            for sample in data {
                x.push(*sample).unwrap();
            }
            let actual = x.query(now);
            for i in 0..4 {
                close(actual[i].samples.get(), r[i].samples.get());
                close(actual[i].time.get(), r[i].time.get());
                assert_eq!(actual[i].time.weight, r[i].time.weight);
                assert!(!actual[i].limited);
            }
        }
    }
    #[test]
    fn irregular_samples_and_time_are_different() {
        let data = [s(0, 0.0), s(9000, 100.0)];
        let r = oracle(&data, 10000);
        assert_eq!(r[0].samples.get(), Some(50.0));
        assert_eq!(r[0].time.get(), Some(10.0));
        exacts(&data, 10000);
    }
    #[test]
    fn expiry_without_new_events_and_half_open_boundaries() {
        let mut x = Exact::deque();
        x.push(s(0, 3.0)).unwrap();
        assert_eq!(x.query(60_000)[0].time.weight, 60_000);
        assert_eq!(x.query(120_000)[0].time.get(), None);
        let mut y = Exact::deque();
        y.push(s(60_000, 7.0)).unwrap();
        assert_eq!(y.query(60_000)[0].samples.get(), None);
        assert_eq!(y.query(120_000)[0].samples.get(), Some(7.0));
    }
    #[test]
    fn all_windows_against_independent_oracle() {
        for kind in ["sparse", "irregular", "burst"] {
            let data = generate(kind);
            exacts(&data, data.last().unwrap().at + 1234);
        }
    }
    #[test]
    fn missing_is_unknown_but_zero_is_covered() {
        let data = [
            s(0, 0.0),
            Sample {
                at: 10000,
                value: None,
            },
            s(20000, 0.0),
        ];
        exacts(&data, 30000);
        let r = oracle(&data, 30000);
        assert_eq!(r[0].time.get(), Some(0.0));
        assert_eq!(r[0].time.weight, 20000);
    }
    #[test]
    fn invalid_numbers_close_coverage() {
        let data = [
            s(0, 1.0),
            s(10000, f64::NAN),
            s(20000, f64::INFINITY),
            s(30000, 3.0),
        ];
        exacts(&data, 40000);
        assert_eq!(oracle(&data, 40000)[0].time.weight, 20000);
    }
    #[test]
    fn duplicate_replacement_and_old_event_rejection() {
        for kind in ["deque", "ring", "buckets_10s"] {
            let mut x = build(kind);
            x.push(s(0, 1.0)).unwrap();
            x.push(s(0, 2.0)).unwrap();
            assert!(x.push(s(0, 3.0)).is_ok());
            x.push(s(10000, 4.0)).unwrap();
            assert_eq!(x.push(s(9999, 9.0)), Err("out_of_order"));
            let q = x.query(20000);
            close(q[0].samples.get(), Some(3.5));
            close(q[0].time.get(), Some(3.5));
        }
    }
    #[test]
    fn restart_marker_does_not_count_downtime() {
        let data = [
            s(0, 5.0),
            Sample {
                at: 10000,
                value: None,
            },
            s(50000, 5.0),
        ];
        let mut x = Exact::deque();
        for p in &data {
            x.push(*p).unwrap();
        }
        assert_eq!(x.query(60000)[0].time.weight, 20000);
        exacts(&data, 60000);
    }
    #[test]
    fn bounded_ring_reports_loss_until_lost_history_leaves_window() {
        let mut x = Exact::ring(2);
        for at in [0, 10000, 20000, 30000] {
            x.push(s(at, 1.0)).unwrap();
        }
        let q = x.query(40000);
        assert!(q[0].limited);
        assert_eq!(q[0].time.weight, 20000);
        assert!(!x.query(100000)[0].limited);
    }
    #[test]
    fn bucket_aligned_windows_exact_time_but_partial_edge_is_approximate() {
        let mut b = Buckets::new();
        for p in [s(0, 0.0), s(9000, 100.0), s(10000, 100.0)] {
            b.push(p).unwrap();
        }
        let aligned = b.query(20000);
        close(aligned[0].time.get(), Some(55.0));
        let q = b.query(65000);
        let r = oracle(&[s(0, 0.0), s(9000, 100.0), s(10000, 100.0)], 65000);
        assert!(abs_error(q[0].time.get(), r[0].time.get()).unwrap() > 0.1);
        assert_eq!(b.slots.len(), 8642);
    }
    #[test]
    fn large_finite_values_do_not_overflow_and_drift_remains_bounded() {
        exacts(
            &[
                s(0, f64::MAX / 2.0),
                s(10000, -f64::MAX / 2.0),
                s(20000, 1.0),
            ],
            30000,
        );
        let data = (0..10000)
            .map(|i| s(i * 1000, 1e10 + (i % 2) as f64))
            .collect::<Vec<_>>();
        exacts(&data, 10_000_000);
    }
    #[test]
    fn every_backend_preserves_zero_missing_and_expiry() {
        for kind in ["deque", "ring", "buckets_10s"] {
            let mut x = build(kind);
            for p in [
                s(0, 0.0),
                s(10000, f64::NAN),
                s(20000, f64::INFINITY),
                s(30000, 3.0),
            ] {
                x.push(p).unwrap();
            }
            let q = x.query(40000);
            assert_eq!(q[0].time.weight, 20000);
            close(q[0].time.get(), Some(1.5));
            assert!(x.query(200000)[0].time.get().is_none());
        }
    }
    #[test]
    fn bucket_aligned_four_windows_match_oracle() {
        let data = generate("irregular");
        let now = RETAIN;
        let r = oracle(&data, now);
        let mut x = Buckets::new();
        for s in &data {
            x.push(*s).unwrap();
        }
        let q = x.query(now);
        for i in 0..4 {
            close(q[i].samples.get(), r[i].samples.get());
            close(q[i].time.get(), r[i].time.get());
            assert_eq!(q[i].time.weight, r[i].time.weight);
        }
    }
}
