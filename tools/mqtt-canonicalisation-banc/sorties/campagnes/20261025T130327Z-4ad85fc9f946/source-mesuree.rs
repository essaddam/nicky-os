//! Isolated post-JSON canonicalisation model. No MQTT client or physical output.
use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::HashMap;
use std::hint::black_box;
use std::mem::size_of;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

struct CountingAllocator;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static CALLS: AtomicUsize = AtomicUsize::new(0);
static REQUESTED: AtomicUsize = AtomicUsize::new(0);
fn allocated(bytes: usize) {
    LIVE.fetch_add(bytes, Ordering::Relaxed);
    CALLS.fetch_add(1, Ordering::Relaxed);
    REQUESTED.fetch_add(bytes, Ordering::Relaxed);
}
// Counters contain requested sizes, not allocator headers or RSS. No allocator recursion.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            allocated(layout.size());
        }
        p
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            allocated(layout.size());
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe {
            System.dealloc(p, layout);
        }
    }
    unsafe fn realloc(&self, p: *mut u8, old: Layout, new_size: usize) -> *mut u8 {
        let result = unsafe { System.realloc(p, old, new_size) };
        if !result.is_null() {
            LIVE.fetch_sub(old.size(), Ordering::Relaxed);
            allocated(new_size);
        }
        result
    }
}
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;
const FIELDS: usize = 6;
const RETAINED: usize = 5;
const ROUNDS: usize = 7;
const UPDATES: usize = 1_048_576;
const KEYS: [&str; FIELDS] = [
    "temperature",
    "power",
    "state",
    "contact",
    "battery",
    "action",
];
const READ: u16 = 1;
const WRITE: u16 = 2;
const EVENT: u16 = 4;

// Exactly the proposal's Row64 + Numeric32, not a second kind of value store.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
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
#[derive(Clone, Copy, Debug, Default)]
struct Numeric32 {
    min: u64,
    max: u64,
    step: u64,
    anchor: u64,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct Row96 {
    base: Row64,
    numeric: Numeric32,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Binding {
    device_slot: u32,
    field_slot: u16,
    generation: u16,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Native<'a> {
    Int(i64),
    Float(f64),
    Bool(bool),
    Word(&'a str),
    Null,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Canonical {
    Int(i64),
    Float(f64),
    Bool(bool),
    Occurrence,
}
#[derive(Debug)]
enum Codec {
    Scaled,
    Power,
    Binary(String, String),
    Battery,
    Event(String),
}
impl Codec {
    fn decode(&self, value: Native<'_>) -> Result<Canonical, Error> {
        match (self, value) {
            // Do not silently lose integral precision before applying a scale.
            (Self::Scaled, Native::Int(n))
                if (-9_007_199_254_740_992..=9_007_199_254_740_992).contains(&n) =>
            {
                let scaled = n as f64 * 0.1;
                scaled
                    .is_finite()
                    .then_some(Canonical::Float(scaled))
                    .ok_or(Error::Invalid)
            }
            (Self::Power, Native::Float(n)) if n.is_finite() && n >= 0.0 => Ok(Canonical::Float(n)),
            (Self::Binary(on, _), Native::Word(word)) if word == on => Ok(Canonical::Bool(true)),
            (Self::Binary(_, off), Native::Word(word)) if word == off => Ok(Canonical::Bool(false)),
            (Self::Battery, Native::Int(n)) if (0..=100).contains(&n) => Ok(Canonical::Int(n)),
            (Self::Event(allowed), Native::Word(word)) if word == allowed => {
                Ok(Canonical::Occurrence)
            }
            _ => Err(Error::Invalid),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Error {
    Invalid,
    Unknown,
    Stale,
    ReadOnly,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Unit {
    Celsius,
    Watt,
    Percent,
    None,
}
struct Descriptor {
    id: u64,
    device_id: u64,
    native_key: String,
    binding: Binding,
    row_slot: Option<u32>,
    codec: Codec,
    unit: Unit,
    flags: u16,
}
struct Device {
    id: u64,
    native_identity: String,
    generation: u16,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ActionKind {
    SetBool,
    Press,
}
struct ActionContract {
    id: u64,
    device_id: u64,
    native_key: String,
    kind: ActionKind,
}
struct DeviceRoute {
    device_slot: u32,
    fields: HashMap<String, u32>,
    commands: HashMap<String, u32>,
}
struct Bank {
    rows: Vec<Row96>,
    descriptors: Vec<Descriptor>,
    devices: Vec<Device>,
    routes: HashMap<String, DeviceRoute>,
    actions: Vec<ActionContract>,
    epoch: u64,
    event_sequence: u64,
}
#[derive(Clone, Copy, Debug, Default)]
struct Heap {
    rows: usize,
    descriptors: usize,
    devices: usize,
    routes: usize,
    actions: usize,
    total: usize,
}
fn live() -> usize {
    LIVE.load(Ordering::Relaxed)
}
// Meaningful in the single-threaded executable only. Other test-harness threads
// can free their own banks between snapshots; never interpret test heap deltas.
fn live_delta(after: usize, before: usize) -> usize {
    after.saturating_sub(before)
}
// Reduced stand-alone illustration of code_property's namespace classification.
// Production must call moli_core::privacy::{code_property,private_json/private_value}.
fn private_property(key: &str) -> bool {
    key.split('.').chain(std::iter::once(key)).any(|part| {
        let normalized: String = part
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|c| c.to_ascii_lowercase())
            .collect();
        matches!(normalized.as_str(), "code" | "pin" | "keycode")
            || [
                "actioncode",
                "actionpin",
                "pincode",
                "pincodevalue",
                "keypadcode",
                "usercode",
                "mastercode",
                "accesscode",
                "securitycode",
                "alarmcode",
                "lockcode",
                "inputcode",
                "codeinput",
                "passcode",
                "password",
                "unlockdoor",
            ]
            .iter()
            .any(|name| normalized.contains(name))
    })
}
impl Bank {
    fn new(count: usize) -> (Self, Heap) {
        assert!(count > 0 && count <= u32::MAX as usize / FIELDS);
        let baseline = live();
        let mut rows = Vec::with_capacity(count * RETAINED);
        for device in 0..count {
            let device_id = device as u64 * 9 + 1;
            for field in 0..RETAINED {
                rows.push(Row96 {
                    base: Row64 {
                        id: device_id + field as u64 + 1,
                        parent: device_id,
                        revision: 1,
                        flags: READ | if field == 2 { WRITE } else { 0 },
                        row_kind: 1,
                        ..Row64::default()
                    },
                    numeric: match field {
                        0 => Numeric32 {
                            step: 0.1_f64.to_bits(),
                            ..Numeric32::default()
                        },
                        1 => Numeric32 {
                            min: 0.0_f64.to_bits(),
                            ..Numeric32::default()
                        },
                        4 => Numeric32 {
                            max: 100.0_f64.to_bits(),
                            step: 1.0_f64.to_bits(),
                            ..Numeric32::default()
                        },
                        _ => Numeric32::default(),
                    },
                });
            }
        }
        let after_rows = live();
        let mut descriptors = Vec::with_capacity(count * FIELDS);
        for device in 0..count {
            for (field, key) in KEYS.iter().enumerate() {
                assert!(!private_property(key));
                descriptors.push(Descriptor {
                    id: device as u64 * 9 + field as u64 + 2,
                    device_id: device as u64 * 9 + 1,
                    native_key: (*key).into(),
                    binding: Binding {
                        device_slot: device as u32,
                        field_slot: field as u16,
                        generation: 1,
                    },
                    row_slot: (field < RETAINED).then_some((device * RETAINED + field) as u32),
                    codec: match field {
                        0 => Codec::Scaled,
                        1 => Codec::Power,
                        2 => Codec::Binary("ON".into(), "OFF".into()),
                        3 => Codec::Binary("CLOSED".into(), "OPEN".into()),
                        4 => Codec::Battery,
                        _ => Codec::Event("single".into()),
                    },
                    unit: match field {
                        0 => Unit::Celsius,
                        1 => Unit::Watt,
                        4 => Unit::Percent,
                        _ => Unit::None,
                    },
                    flags: READ
                        | if field == 2 {
                            WRITE
                        } else if field == 5 {
                            EVENT
                        } else {
                            0
                        },
                });
            }
        }
        let after_descriptors = live();
        let devices = (0..count)
            .map(|slot| Device {
                id: slot as u64 * 9 + 1,
                native_identity: format!("fixture-native-{slot:06}"),
                generation: 1,
            })
            .collect();
        let after_devices = live();
        let mut actions = Vec::with_capacity(count * 2);
        for device in 0..count {
            for (key, kind) in [
                ("state", ActionKind::SetBool),
                ("identify", ActionKind::Press),
            ] {
                actions.push(ActionContract {
                    id: device as u64 * 9 + 8 + u64::from(kind == ActionKind::Press),
                    device_id: device as u64 * 9 + 1,
                    native_key: key.into(),
                    kind,
                });
            }
        }
        let after_actions = live();
        let mut routes = HashMap::with_capacity(count);
        for device in 0..count {
            let fields = KEYS
                .iter()
                .enumerate()
                .map(|(field, key)| ((*key).to_owned(), (device * FIELDS + field) as u32))
                .collect();
            let commands = [
                ("state".to_owned(), (device * 2) as u32),
                ("identify".to_owned(), (device * 2 + 1) as u32),
            ]
            .into();
            assert!(
                routes
                    .insert(
                        format!("fixture-topic-{device:06}"),
                        DeviceRoute {
                            device_slot: device as u32,
                            fields,
                            commands
                        }
                    )
                    .is_none()
            );
        }
        let end = live();
        let heap = Heap {
            rows: live_delta(after_rows, baseline),
            descriptors: live_delta(after_descriptors, after_rows),
            devices: live_delta(after_devices, after_descriptors),
            actions: live_delta(after_actions, after_devices),
            routes: live_delta(end, after_actions),
            total: live_delta(end, baseline),
        };
        (
            Self {
                rows,
                descriptors,
                devices,
                routes,
                actions,
                epoch: 1,
                event_sequence: 0,
            },
            heap,
        )
    }
    fn resolve(&self, topic: &str, property: &str) -> Result<Binding, Error> {
        let route = self.routes.get(topic).ok_or(Error::Unknown)?;
        let index = *route.fields.get(property).ok_or(Error::Unknown)? as usize;
        let descriptor = &self.descriptors[index];
        if descriptor.binding.device_slot != route.device_slot || descriptor.native_key != property
        {
            return Err(Error::Stale);
        }
        Ok(descriptor.binding)
    }
    fn descriptor(&self, binding: Binding, epoch: u64) -> Result<&Descriptor, Error> {
        if epoch != self.epoch {
            return Err(Error::Stale);
        }
        let device = self
            .devices
            .get(binding.device_slot as usize)
            .ok_or(Error::Stale)?;
        if device.generation != binding.generation || binding.field_slot as usize >= FIELDS {
            return Err(Error::Stale);
        }
        let descriptor =
            &self.descriptors[binding.device_slot as usize * FIELDS + binding.field_slot as usize];
        if descriptor.binding != binding || descriptor.device_id != device.id {
            return Err(Error::Stale);
        }
        if let Some(slot) = descriptor.row_slot {
            let row = self.rows.get(slot as usize).ok_or(Error::Stale)?;
            if row.base.revision != u32::from(binding.generation)
                || row.base.id != descriptor.id
                || row.base.parent != device.id
            {
                return Err(Error::Stale);
            }
        }
        Ok(descriptor)
    }
    fn apply(
        &mut self,
        binding: Binding,
        epoch: u64,
        native: Native<'_>,
        now: u64,
    ) -> Result<Outcome, Error> {
        let descriptor = self.descriptor(binding, epoch)?;
        let value = descriptor.codec.decode(native)?;
        let (id, device_id, slot, flags) = (
            descriptor.id,
            descriptor.device_id,
            descriptor.row_slot,
            descriptor.flags,
        );
        if value == Canonical::Occurrence {
            if flags & EVENT == 0 || slot.is_some() {
                return Err(Error::Invalid);
            }
            self.event_sequence = self.event_sequence.checked_add(1).ok_or(Error::Invalid)?;
            return Ok(Outcome {
                id,
                device_id,
                payload: 0,
                received: now,
                changed: now,
                sequence: self.event_sequence,
            });
        }
        let (payload, kind) = match value {
            Canonical::Bool(b) => (u64::from(b), 1),
            Canonical::Int(n) => (u64::from_ne_bytes(n.to_ne_bytes()), 2),
            Canonical::Float(n) => (n.to_bits(), 3),
            Canonical::Occurrence => unreachable!(),
        };
        let row = &mut self.rows[slot.ok_or(Error::Invalid)? as usize].base;
        if row.value_kind != kind || row.payload != payload {
            row.changed_ms = now;
        }
        row.payload = payload;
        row.value_kind = kind;
        row.observed_ms = now;
        row.received_ms = now;
        Ok(Outcome {
            id,
            device_id,
            payload,
            received: now,
            changed: row.changed_ms,
            sequence: 0,
        })
    }
    fn update(
        &mut self,
        strategy: Strategy,
        input: &Input<'_>,
        now: u64,
    ) -> Result<Outcome, Error> {
        let binding = match strategy {
            Strategy::Allocating => {
                let topic = black_box(input.topic.to_owned());
                let property = black_box(input.property.to_owned());
                self.resolve(&topic, &property)?
            }
            Strategy::Borrowed => self.resolve(input.topic, input.property)?,
            // Prepared by routing + decoded native key/schema validation OUTSIDE this hot path.
            Strategy::Cached => input.binding,
        };
        self.apply(binding, input.epoch, input.value, now)
    }
    fn reset(&mut self) {
        self.event_sequence = 0;
        for row in &mut self.rows {
            row.base.payload = 0;
            row.base.value_kind = 0;
            row.base.observed_ms = 0;
            row.base.received_ms = 0;
            row.base.changed_ms = 0;
        }
    }
    fn command(&self, topic: &str, key: &str, value: Native<'_>) -> Result<Intent<'_>, Error> {
        let route = self.routes.get(topic).ok_or(Error::Unknown)?;
        let index = route.commands.get(key).ok_or_else(|| {
            if route.fields.contains_key(key) {
                Error::ReadOnly
            } else {
                Error::Unknown
            }
        })?;
        let action = &self.actions[*index as usize];
        let wire = match (action.kind, value) {
            (ActionKind::SetBool, Native::Bool(false)) => "OFF",
            (ActionKind::SetBool, Native::Bool(true)) => "ON",
            (ActionKind::Press, Native::Null) => "identify",
            _ => return Err(Error::Invalid),
        };
        // Intent only. A real command must still enter the hub, guard and deadline machinery.
        Ok(Intent {
            action_id: action.id,
            device_id: action.device_id,
            key: &action.native_key,
            wire,
        })
    }
    #[cfg(test)]
    fn rename_topic(&mut self, old: &str, new: String) -> Result<(), Error> {
        if self.routes.contains_key(&new) {
            return Err(Error::Invalid);
        }
        let route = self.routes.remove(old).ok_or(Error::Unknown)?;
        self.routes.insert(new, route);
        self.epoch = self.epoch.checked_add(1).ok_or(Error::Invalid)?;
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Intent<'a> {
    action_id: u64,
    device_id: u64,
    key: &'a str,
    wire: &'static str,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Outcome {
    id: u64,
    device_id: u64,
    payload: u64,
    received: u64,
    changed: u64,
    sequence: u64,
}
#[derive(Clone, Copy, Debug)]
enum Strategy {
    Allocating,
    Borrowed,
    Cached,
}
impl Strategy {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "allocating_lookup" => Some(Self::Allocating),
            "borrowed_lookup" => Some(Self::Borrowed),
            "cached_binding" => Some(Self::Cached),
            _ => None,
        }
    }
}
struct Input<'a> {
    topic: &'a str,
    property: &'a str,
    binding: Binding,
    epoch: u64,
    value: Native<'a>,
}
struct Corpus {
    topics: Vec<String>,
    properties: Vec<String>,
    words: Vec<String>,
}
impl Corpus {
    fn new(count: usize) -> Self {
        Self {
            topics: (0..count)
                .map(|i| format!("fixture-topic-{i:06}"))
                .collect(),
            properties: KEYS.iter().map(|key| (*key).to_owned()).collect(),
            words: ["ON", "OFF", "CLOSED", "OPEN", "single"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        }
    }
    fn inputs<'a>(&'a self, bank: &Bank, count: usize) -> Vec<Input<'a>> {
        (0..count)
            .map(|i| {
                // Visit each device and all six properties without a stride/GCD bias.
                let device = (i / FIELDS) % self.topics.len();
                let field = i % FIELDS;
                let topic = self.topics[device].as_str();
                let property = self.properties[field].as_str();
                let value = match field {
                    0 => Native::Int(215 + (i / FIELDS % 3) as i64),
                    1 => Native::Float(10.0 + (i / FIELDS % 32) as f64 * 0.5),
                    2 => Native::Word(&self.words[(i / FIELDS) % 2]),
                    3 => Native::Word(&self.words[2 + (i / FIELDS) % 2]),
                    4 => Native::Int((i / FIELDS % 101) as i64),
                    _ => Native::Word(&self.words[4]),
                };
                Input {
                    topic,
                    property,
                    binding: bank.resolve(topic, property).expect("fixture routes"),
                    epoch: bank.epoch,
                    value,
                }
            })
            .collect()
    }
}
fn run_pass(bank: &mut Bank, strategy: Strategy, inputs: &[Input<'_>]) -> u64 {
    let mut checksum = 0_u64;
    for (i, input) in inputs.iter().enumerate() {
        let out = black_box(
            bank.update(strategy, black_box(input), i as u64 + 1)
                .expect("valid fixture"),
        );
        checksum = checksum.rotate_left(7)
            ^ out.id
            ^ out.device_id.rotate_left(3)
            ^ out.payload
            ^ out.received.rotate_left(11)
            ^ out.changed.rotate_left(17)
            ^ out.sequence;
    }
    black_box(checksum ^ bank.event_sequence)
}
fn store_checksum(bank: &Bank) -> u64 {
    bank.rows.iter().fold(0_u64, |checksum, row| {
        checksum.rotate_left(3)
            ^ row.base.id
            ^ row.base.payload
            ^ row.base.received_ms
            ^ row.base.changed_ms
    })
}
fn validate_fixture(bank: &Bank) {
    assert_eq!(size_of::<Row96>(), 96);
    assert_eq!(size_of::<Binding>(), 8);
    assert_eq!(
        bank.command("fixture-topic-000000", "temperature", Native::Int(215)),
        Err(Error::ReadOnly)
    );
    let intent = bank
        .command("fixture-topic-000000", "state", Native::Bool(true))
        .expect("set intent");
    assert_eq!(
        (intent.action_id, intent.device_id, intent.key, intent.wire),
        (8, 1, "state", "ON")
    );
    assert_eq!(
        bank.command("fixture-topic-000000", "identify", Native::Null)
            .expect("press intent")
            .action_id,
        9
    );
    assert_eq!(bank.descriptors[0].unit, Unit::Celsius);
    assert!(
        bank.devices[0]
            .native_identity
            .starts_with("fixture-native-")
    );
}
fn median(mut samples: [usize; ROUNDS]) -> usize {
    samples.sort_unstable();
    samples[ROUNDS / 2]
}
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 5 || args[1] != "--bench" {
        eprintln!(
            "usage: mqtt-bench --bench allocating_lookup|borrowed_lookup|cached_binding DEVICES SCALE"
        );
        std::process::exit(2);
    }
    let Some(strategy) = Strategy::parse(&args[2]) else {
        eprintln!("unknown benchmark variant");
        std::process::exit(2);
    };
    let parsed = args[3]
        .parse::<usize>()
        .ok()
        .zip(args[4].parse::<usize>().ok());
    let Some((devices, scale)) = parsed.filter(|(d, s)| *d > 0 && *s > 0) else {
        eprintln!("DEVICES and SCALE must be positive integers");
        std::process::exit(2);
    };
    let Some(count) = devices.checked_mul(scale).filter(|n| *n <= 1_000_000) else {
        eprintln!("scaled device count must not exceed 1000000");
        std::process::exit(2);
    };
    let (mut bank, heap) = Bank::new(count);
    validate_fixture(&bank);
    assert_eq!(
        heap.total,
        heap.rows + heap.descriptors + heap.devices + heap.routes + heap.actions
    );
    let corpus = Corpus::new(count);
    let inputs = corpus.inputs(&bank, UPDATES);
    // Prove identical results on all paths in this process; not part of measured passes.
    let mut expected = None;
    for candidate in [Strategy::Allocating, Strategy::Borrowed, Strategy::Cached] {
        bank.reset();
        let checksum = run_pass(&mut bank, candidate, &inputs) ^ store_checksum(&bank);
        if let Some(value) = expected {
            assert_eq!(value, checksum, "strategy parity");
        }
        expected = Some(checksum);
    }
    bank.reset();
    black_box(run_pass(&mut bank, strategy, &inputs)); // one unmeasured warmup
    let mut nanos = [0; ROUNDS];
    let mut calls = [0; ROUNDS];
    let mut bytes = [0; ROUNDS];
    let mut checksum = 0;
    for round in 0..ROUNDS {
        bank.reset();
        let start_calls = CALLS.load(Ordering::Relaxed);
        let start_requested = REQUESTED.load(Ordering::Relaxed);
        let start = Instant::now();
        checksum = run_pass(&mut bank, strategy, &inputs);
        let elapsed = start.elapsed();
        calls[round] = CALLS.load(Ordering::Relaxed) - start_calls;
        bytes[round] = REQUESTED.load(Ordering::Relaxed) - start_requested;
        nanos[round] = usize::try_from(elapsed.as_nanos()).expect("measurement duration");
        checksum ^= store_checksum(&bank); // verification scan is outside the timer/counters
        assert_eq!(Some(checksum), expected);
    }
    println!(
        "{{\"variant\":\"{}\",\"scale\":{},\"devices\":{},\"total_devices\":{},\"retained_rows\":{},\"mapping_descriptors\":{},\"action_contracts\":{},\"row_bytes\":{},\"binding_bytes\":{},\"updates_per_round\":{},\"timed_rounds\":{},\"checksum\":{},\"parity\":true,\"heap_live_requested_bytes\":{},\"heap_rows_bytes\":{},\"heap_descriptors_bytes\":{},\"heap_devices_bytes\":{},\"heap_routes_bytes\":{},\"heap_action_contracts_bytes\":{},\"prepared_input_buffer_bytes_excluded\":{},\"update_ns_per_op\":{:.3},\"allocation_calls_per_round\":{},\"allocation_calls_timed_total\":{},\"temporary_requested_bytes_per_op\":{:.3},\"requested_bytes_timed_total\":{}}}",
        args[2],
        scale,
        devices,
        count,
        bank.rows.len(),
        bank.descriptors.len(),
        bank.actions.len(),
        size_of::<Row96>(),
        size_of::<Binding>(),
        UPDATES,
        ROUNDS,
        checksum,
        heap.total,
        heap.rows,
        heap.descriptors,
        heap.devices,
        heap.routes,
        heap.actions,
        inputs.capacity() * size_of::<Input<'_>>(),
        median(nanos) as f64 / UPDATES as f64,
        median(calls),
        calls.iter().sum::<usize>(),
        median(bytes) as f64 / UPDATES as f64,
        bytes.iter().sum::<usize>()
    );
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_binary_words_and_unknown_are_distinct() {
        let codec = Codec::Binary("ON".into(), "OFF".into());
        assert_eq!(codec.decode(Native::Word("ON")), Ok(Canonical::Bool(true)));
        assert_eq!(
            codec.decode(Native::Word("OFF")),
            Ok(Canonical::Bool(false))
        );
        assert_eq!(codec.decode(Native::Word("unknown")), Err(Error::Invalid));
        assert_eq!(codec.decode(Native::Null), Err(Error::Invalid));
    }
    #[test]
    fn contact_true_means_closed() {
        let codec = Codec::Binary("CLOSED".into(), "OPEN".into());
        assert_eq!(
            codec.decode(Native::Word("CLOSED")),
            Ok(Canonical::Bool(true))
        );
        assert_eq!(
            codec.decode(Native::Word("OPEN")),
            Ok(Canonical::Bool(false))
        );
    }
    #[test]
    fn scale_and_non_finite_values_are_checked() {
        assert_eq!(
            Codec::Scaled.decode(Native::Int(215)),
            Ok(Canonical::Float(21.5))
        );
        assert_eq!(
            Codec::Power.decode(Native::Float(f64::INFINITY)),
            Err(Error::Invalid)
        );
        assert_eq!(Codec::Battery.decode(Native::Int(101)), Err(Error::Invalid));
    }
    #[test]
    fn native_single_is_an_occurrence_only() {
        assert_eq!(
            Codec::Event("single".into()).decode(Native::Word("single")),
            Ok(Canonical::Occurrence)
        );
        assert_eq!(
            Codec::Event("single".into()).decode(Native::Word("other")),
            Err(Error::Invalid)
        );
    }
    #[test]
    fn retained_store_has_five_row96_and_no_action_state() {
        assert_eq!(size_of::<Row96>(), 96);
        assert_eq!(size_of::<Binding>(), 8);
        let (bank, _) = Bank::new(2);
        assert_eq!(bank.rows.len(), 10);
        assert_eq!(bank.descriptors.len(), 12);
        assert_eq!(bank.actions.len(), 4);
        assert!(bank.descriptors[5].row_slot.is_none());
        assert_eq!(bank.rows.capacity() * size_of::<Row96>(), 960);
    }
    #[test]
    fn all_equipment_channel_and_action_ids_are_unique() {
        let (bank, _) = Bank::new(10);
        let mut ids = std::collections::HashSet::new();
        for id in bank
            .devices
            .iter()
            .map(|d| d.id)
            .chain(bank.descriptors.iter().map(|d| d.id))
            .chain(bank.actions.iter().map(|a| a.id))
        {
            assert!(id != 0 && ids.insert(id));
        }
        assert_eq!(ids.len(), 90);
        assert_eq!(bank.routes.len(), 10);
        for route in bank.routes.values() {
            assert_eq!(route.fields.len(), 6);
        }
    }
    #[test]
    fn topic_rename_preserves_immutable_native_and_canonical_identities() {
        let (mut bank, _) = Bank::new(1);
        let before = bank.resolve("fixture-topic-000000", "state").unwrap();
        let native = bank.devices[0].native_identity.clone();
        bank.rename_topic("fixture-topic-000000", "renamed-fixture".into())
            .unwrap();
        assert_eq!(
            bank.resolve("fixture-topic-000000", "state"),
            Err(Error::Unknown)
        );
        assert_eq!(bank.resolve("renamed-fixture", "state"), Ok(before));
        let descriptor = bank.descriptor(before, bank.epoch).unwrap();
        assert_eq!((descriptor.device_id, descriptor.id), (1, 4));
        assert_eq!(bank.devices[0].native_identity, native);
    }
    #[test]
    fn rename_rejects_old_epoch_even_when_slot_still_exists() {
        let (mut bank, _) = Bank::new(1);
        let binding = bank.resolve("fixture-topic-000000", "state").unwrap();
        bank.rename_topic("fixture-topic-000000", "renamed-fixture".into())
            .unwrap();
        assert!(matches!(bank.descriptor(binding, 1), Err(Error::Stale)));
        assert_eq!(
            bank.apply(binding, 1, Native::Word("ON"), 5),
            Err(Error::Stale)
        );
    }
    #[test]
    fn reused_slot_generation_mismatch_rejects_read() {
        let (mut bank, _) = Bank::new(1);
        let binding = bank.resolve("fixture-topic-000000", "state").unwrap();
        bank.devices[0].generation = 2;
        assert!(matches!(bank.descriptor(binding, 1), Err(Error::Stale)));
        bank.devices[0].generation = 1;
        bank.rows[2].base.revision = 2;
        assert!(matches!(bank.descriptor(binding, 1), Err(Error::Stale)));
    }
    #[test]
    fn unknown_property_does_not_create_a_point_or_value() {
        let (bank, _) = Bank::new(1);
        assert_eq!(
            bank.resolve("fixture-topic-000000", "new_property"),
            Err(Error::Unknown)
        );
        assert_eq!(bank.resolve("unknown-topic", "state"), Err(Error::Unknown));
        assert_eq!(bank.rows.len(), 5);
        assert_eq!(bank.descriptors.len(), 6);
    }
    #[test]
    fn repeated_value_refreshes_received_without_changed() {
        let (mut bank, _) = Bank::new(1);
        let binding = bank.resolve("fixture-topic-000000", "state").unwrap();
        let first = bank.apply(binding, 1, Native::Word("ON"), 10).unwrap();
        let again = bank.apply(binding, 1, Native::Word("ON"), 20).unwrap();
        assert_eq!((first.changed, again.received, again.changed), (10, 20, 10));
        assert_eq!(
            (
                bank.rows[2].base.observed_ms,
                bank.rows[2].base.received_ms,
                bank.rows[2].base.changed_ms
            ),
            (20, 20, 10)
        );
        assert_eq!(
            bank.apply(binding, 1, Native::Word("unknown"), 30),
            Err(Error::Invalid)
        );
        assert_eq!(bank.rows[2].base.received_ms, 20);
    }
    #[test]
    fn two_single_events_have_two_identities_and_no_retained_state() {
        let (mut bank, _) = Bank::new(1);
        let binding = bank.resolve("fixture-topic-000000", "action").unwrap();
        let first = bank.apply(binding, 1, Native::Word("single"), 10).unwrap();
        let second = bank.apply(binding, 1, Native::Word("single"), 11).unwrap();
        assert_eq!(
            (first.id, first.sequence, second.id, second.sequence),
            (7, 1, 7, 2)
        );
        assert!(bank.rows.iter().all(|row| row.base.value_kind == 0));
        assert_eq!(
            bank.command("fixture-topic-000000", "action", Native::Null),
            Err(Error::ReadOnly)
        );
    }
    #[test]
    fn command_contracts_return_intents_without_mutating_state() {
        let (bank, _) = Bank::new(1);
        for key in ["temperature", "power", "contact", "battery", "action"] {
            assert_eq!(
                bank.command("fixture-topic-000000", key, Native::Bool(true)),
                Err(Error::ReadOnly)
            );
        }
        assert_eq!(
            bank.command("fixture-topic-000000", "state", Native::Bool(false)),
            Ok(Intent {
                action_id: 8,
                device_id: 1,
                key: "state",
                wire: "OFF"
            })
        );
        assert_eq!(
            bank.command("fixture-topic-000000", "identify", Native::Null),
            Ok(Intent {
                action_id: 9,
                device_id: 1,
                key: "identify",
                wire: "identify"
            })
        );
        assert_eq!(
            bank.command("fixture-topic-000000", "identify", Native::Bool(true)),
            Err(Error::Invalid)
        );
        assert_eq!(
            bank.command("fixture-topic-000000", "state", Native::Int(1)),
            Err(Error::Invalid)
        );
        assert!(bank.rows.iter().all(|row| row.base.value_kind == 0));
    }
    #[test]
    fn credential_namespace_is_excluded_even_under_composites() {
        for key in [
            "pin_code",
            "pin_code.user",
            "settings.PIN",
            "unlock_door_with_timeout.pin_code",
            "Action-Code",
        ] {
            assert!(private_property(key));
        }
        for key in ["battery", "error_code", "date_code", "contact"] {
            assert!(!private_property(key));
        }
        let (bank, _) = Bank::new(1);
        // Fictional placeholder; no actual credential value is decoded or stored.
        assert_eq!(
            bank.resolve("fixture-topic-000000", "pin_code.user"),
            Err(Error::Unknown)
        );
        assert_eq!(
            bank.command(
                "fixture-topic-000000",
                "pin_code",
                Native::Word("fictional-placeholder")
            ),
            Err(Error::Unknown)
        );
    }
    #[test]
    fn units_and_integer_payload_are_preserved() {
        let (mut bank, _) = Bank::new(1);
        assert_eq!(bank.descriptors[0].unit, Unit::Celsius);
        assert_eq!(bank.descriptors[1].unit, Unit::Watt);
        assert_eq!(bank.descriptors[4].unit, Unit::Percent);
        assert_eq!(f64::from_bits(bank.rows[0].numeric.step), 0.1);
        let binding = bank.resolve("fixture-topic-000000", "battery").unwrap();
        bank.apply(binding, 1, Native::Int(99), 1).unwrap();
        assert_eq!(
            (bank.rows[4].base.value_kind, bank.rows[4].base.payload),
            (2, 99)
        );
        assert_eq!(
            Codec::Scaled.decode(Native::Int(9_007_199_254_740_993)),
            Err(Error::Invalid)
        );
        assert_eq!(
            Codec::Scaled.decode(Native::Int(i64::MAX)),
            Err(Error::Invalid)
        );
        assert_eq!(
            Codec::Power.decode(Native::Float(f64::NAN)),
            Err(Error::Invalid)
        );
    }
    #[test]
    fn strategies_produce_same_outcomes_and_store() {
        let (mut bank, _) = Bank::new(3);
        let corpus = Corpus::new(3);
        let inputs = corpus.inputs(&bank, 120);
        let mut checksums = Vec::new();
        for strategy in [Strategy::Allocating, Strategy::Borrowed, Strategy::Cached] {
            bank.reset();
            checksums.push(run_pass(&mut bank, strategy, &inputs) ^ store_checksum(&bank));
            assert_eq!(bank.event_sequence, 20);
            assert!(bank.rows.iter().all(|row| row.base.received_ms != 0));
        }
        assert_eq!(checksums[0], checksums[1]);
        assert_eq!(checksums[1], checksums[2]);
    }
}
