//! Synthetic, offline microbenchmarks. No Hub, credentials or household input.
use moli_core::{
    Access, Device, DeviceId, InstanceId, Kind, PointSpec, Sample, Semantic, Unit, Value,
};
use serde_json::{Value as Json, json};
use std::hint::black_box;
use std::sync::Arc;
use std::time::Instant;

mod meter;
#[cfg(feature = "compteurs")]
#[global_allocator]
static ALLOCATOR: meter::Meter = meter::Meter;

const TEXT_SIZES: [usize; 4] = [16, 64, 256, 4096];
const OPERATIONS: [&str; 5] = [
    "create_drop",
    "clone_drop",
    "replace_changed",
    "replace_identical_without_check",
    "skip_identical_by_bytes",
];

#[derive(Clone, Copy)]
struct Measurement {
    ns: f64,
    calls: f64,
    bytes: f64,
    peak: usize,
    live: isize,
}

struct Settings {
    iterations: usize,
    repetitions: usize,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            iterations: 50_000,
            repetitions: 5,
        }
    }
}

fn measure(operation: &str, a: &str, b: &str, iterations: usize) -> Measurement {
    let mut current = Value::from(a);
    let before = meter::counters();
    meter::reset_peak();
    let started = Instant::now();
    for index in 0..iterations {
        match operation {
            "create_drop" => drop(black_box(Value::from(black_box(a)))),
            "clone_drop" => drop(black_box(black_box(&current).clone())),
            "replace_changed" => {
                current = Value::from(black_box(if index % 2 == 0 { b } else { a }));
                black_box(&current);
            }
            "replace_identical_without_check" => {
                current = Value::from(black_box(a));
                black_box(&current);
            }
            "skip_identical_by_bytes" => {
                if let Value::Text(text) = black_box(&current)
                    && text.as_ref() != black_box(a)
                {
                    current = Value::from(black_box(a));
                }
                black_box(&current);
            }
            _ => unreachable!("fixed operation list"),
        }
    }
    let ns = started.elapsed().as_nanos() as f64 / iterations as f64;
    let after = meter::counters();
    let result = Measurement {
        ns,
        calls: (after.calls - before.calls) as f64 / iterations as f64,
        bytes: (after.total - before.total) as f64 / iterations as f64,
        peak: after.peak.saturating_sub(before.live),
        live: after.live as isize - before.live as isize,
    };
    drop(black_box(current));
    result
}

fn text_bench(settings: &Settings) {
    println!(
        "{}",
        json!({
            "kind":"text_protocol", "architecture":std::env::consts::ARCH,
            "os":std::env::consts::OS, "iterations":settings.iterations,
            "repetitions":settings.repetitions, "allocation_meter":cfg!(feature="compteurs"),
            "value_bytes":std::mem::size_of::<Value>(), "sample_bytes":std::mem::size_of::<Sample>(),
            "input":"synthetic UTF-8; repeated two-byte code points; no private data",
            "scope":"moli_core::Value; excludes Hub, transport, allocator bookkeeping and RSS"
        })
    );
    for size in TEXT_SIZES {
        let a = "é".repeat(size / 2);
        let b = "à".repeat(size / 2);
        for operation in OPERATIONS {
            black_box(measure(operation, &a, &b, 1000));
            let mut samples = Vec::with_capacity(settings.repetitions);
            for _ in 0..settings.repetitions {
                samples.push(measure(operation, &a, &b, settings.iterations));
            }
            samples.sort_by(|left, right| left.ns.total_cmp(&right.ns));
            let median = samples[settings.repetitions / 2];
            println!(
                "{}",
                json!({
                    "kind":"text_measurement", "utf8_bytes":size, "operation":operation,
                    "median_ns_per_op":median.ns, "min_ns_per_op":samples[0].ns,
                    "max_ns_per_op":samples[settings.repetitions-1].ns,
                    "allocation_calls_per_op":cfg!(feature="compteurs").then_some(median.calls),
                    "requested_allocation_bytes_per_op":cfg!(feature="compteurs").then_some(median.bytes),
                    "peak_requested_bytes_above_start":cfg!(feature="compteurs").then_some(median.peak),
                    "live_requested_bytes_delta":cfg!(feature="compteurs").then_some(median.live)
                })
            );
        }
    }
}

// Layout prototypes only. These types do not implement storage, serde or UTF-8 validation.
#[allow(dead_code)]
enum Candidate<const N: usize> {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    EnumCode(u8),
    Inline { len: u8, bytes: [u8; N] },
    Shared(Arc<str>),
}
#[allow(dead_code)]
enum TextEnum<const N: usize> {
    Inline { len: u8, bytes: [u8; N] },
    Shared(Arc<str>),
}
#[allow(dead_code)]
enum Nested<const N: usize> {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    EnumCode(u8),
    Text(TextEnum<N>),
}

fn layout<const N: usize>() -> Json {
    json!({"inline_bytes":N,"candidate_size":std::mem::size_of::<Candidate<N>>(),
        "candidate_alignment":std::mem::align_of::<Candidate<N>>(),
        "text_enum_size":std::mem::size_of::<TextEnum<N>>(),
        "nested_value_size":std::mem::size_of::<Nested<N>>(),
        "nested_alignment":std::mem::align_of::<Nested<N>>()})
}
fn layouts() {
    println!(
        "{}",
        json!({"kind":"core_layout","architecture":std::env::consts::ARCH,
        "pointer_bytes":std::mem::size_of::<usize>(),"arc_str_bytes":std::mem::size_of::<Arc<str>>(),
        "device_bytes":std::mem::size_of::<Device>(),"point_spec_bytes":std::mem::size_of::<PointSpec>(),
        "kind_bytes":std::mem::size_of::<Kind>(),"value_bytes":std::mem::size_of::<Value>(),
        "sample_bytes":std::mem::size_of::<Sample>(),
        "prototypes":[layout::<15>(),layout::<16>(),layout::<24>(),layout::<32>()]})
    );
}

fn point(humidity: bool) -> PointSpec {
    PointSpec {
        key: if humidity { "humidity" } else { "temperature" }.into(),
        label: if humidity { "Humidite" } else { "Temperature" }.into(),
        kind: Kind::Numeric {
            min: None,
            max: None,
            step: None,
        },
        access: Access {
            read: true,
            write: false,
        },
        unit: Some(if humidity {
            Unit::Percent
        } else {
            Unit::Celsius
        }),
        semantic: if humidity {
            Semantic::Humidity
        } else {
            Semantic::Temperature
        },
    }
}
fn device(instance: &InstanceId, native: &str, points: Vec<PointSpec>) -> Device {
    Device {
        id: DeviceId::new(instance, native),
        instance: instance.clone(),
        native_name: native.into(),
        manufacturer: None,
        model: None,
        description: None,
        native_room: None,
        members: Vec::new(),
        points,
    }
}
fn fixture(separate: bool) -> Vec<Device> {
    let instance = InstanceId::from("fixture");
    let mut devices = Vec::with_capacity(if separate { 34 } else { 17 });
    for room in 0..17 {
        if separate {
            for humidity in [false, true] {
                let native = format!("room-{room:02}-{}", if humidity { "h" } else { "t" });
                devices.push(device(&instance, &native, vec![point(humidity)]));
            }
        } else {
            let native = format!("room-{room:02}");
            devices.push(device(&instance, &native, vec![point(false), point(true)]));
        }
    }
    devices
}
fn virtuals(settings: &Settings) {
    for separate in [false, true] {
        let before = meter::counters();
        meter::reset_peak();
        let devices = fixture(separate);
        let after = meter::counters();
        // Serialization is outside the construction allocation measurement.
        let json_bytes = serde_json::to_vec(&devices)
            .expect("synthetic descriptor")
            .len();
        let mut times = Vec::with_capacity(settings.repetitions);
        for _ in 0..settings.repetitions {
            let started = Instant::now();
            for _ in 0..settings.iterations {
                let mut count = 0;
                for device in black_box(&devices) {
                    for point in &device.points {
                        count += point.key.len();
                    }
                }
                black_box(count);
            }
            times.push(started.elapsed().as_nanos() as f64 / settings.iterations as f64);
        }
        times.sort_by(f64::total_cmp);
        println!(
            "{}",
            json!({"kind":"fixture_schema_only", "separate_devices":separate,
            "devices":devices.len(), "points":devices.iter().map(|d|d.points.len()).sum::<usize>(),
            "outer_device_vec_bytes":devices.capacity()*std::mem::size_of::<Device>(),
            "point_vec_capacity_bytes":devices.iter().map(|d|d.points.capacity()*std::mem::size_of::<PointSpec>()).sum::<usize>(),
            "requested_live_bytes":cfg!(feature="compteurs").then_some(after.live-before.live),
            "requested_total_bytes_during_build":cfg!(feature="compteurs").then_some(after.total-before.total),
            "requested_peak_bytes_during_build":cfg!(feature="compteurs").then_some(after.peak-before.live),
            "allocation_calls_during_build":cfg!(feature="compteurs").then_some(after.calls-before.calls),
            "serialized_descriptor_bytes":json_bytes,"median_ns_scan_34_point_keys":times[settings.repetitions/2],
            "min_ns_scan_34_point_keys":times[0],"max_ns_scan_34_point_keys":times[settings.repetitions-1],
            "allocation_meter":cfg!(feature="compteurs"),
            "excluded":"Hub,indexes,availability,values,snapshots,driver,commands,allocator_overhead,RSS"})
        );
    }
}

fn verify() {
    for size in TEXT_SIZES {
        let a = "é".repeat(size / 2);
        let b = "à".repeat(size / 2);
        let value = Value::from(a.as_str());
        assert_eq!(value.to_json(), json!(a));
        for operation in OPERATIONS {
            let result = measure(operation, &a, &b, 100);
            if cfg!(feature = "compteurs") {
                let allocates = matches!(
                    operation,
                    "create_drop" | "replace_changed" | "replace_identical_without_check"
                );
                assert_eq!(result.calls, if allocates { 1.0 } else { 0.0 });
                assert_eq!(result.live, 0);
                if allocates {
                    assert!(result.bytes >= size as f64);
                } else {
                    assert_eq!(result.bytes, 0.0);
                }
            }
        }
    }
    let unified = fixture(false);
    let separate = fixture(true);
    assert_eq!(unified.len(), 17);
    assert_eq!(separate.len(), 34);
    for devices in [&unified, &separate] {
        assert_eq!(
            devices
                .iter()
                .map(|device| device.points.len())
                .sum::<usize>(),
            34
        );
        assert_eq!(
            devices
                .iter()
                .flat_map(|device| &device.points)
                .filter(|point| point.semantic == Semantic::Temperature)
                .count(),
            17
        );
        assert_eq!(
            devices
                .iter()
                .flat_map(|device| &device.points)
                .filter(|point| point.semantic == Semantic::Humidity)
                .count(),
            17
        );
    }
    println!(
        "{}",
        json!({"verification":"ok","allocation_meter":cfg!(feature="compteurs"),
        "allocation_invariants_checked":cfg!(feature="compteurs"),"scope":"synthetic UTF-8 round trip and 17/34 device fixtures; no performance threshold"})
    );
}

fn arguments() -> Result<(String, Settings), String> {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "verifier".into());
    let mut settings = Settings::default();
    while let Some(flag) = args.next() {
        let raw = args.next().ok_or("Valeur manquante pour le paramètre")?;
        let value: usize = raw.parse().map_err(|_| "Entier positif attendu")?;
        match flag.as_str() {
            "--iterations" if (1..=5_000_000).contains(&value) => settings.iterations = value,
            "--repetitions" if (1..=31).contains(&value) && value % 2 == 1 => {
                settings.repetitions = value
            }
            _ => {
                return Err(
                    "Paramètre inconnu ou hors limites ; répétitions impaires requises".into(),
                );
            }
        }
    }
    Ok((command, settings))
}
fn main() {
    let (command, settings) = arguments().unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(2)
    });
    match command.as_str() {
        "textes" => text_bench(&settings),
        "disposition" => layouts(),
        "virtuels" => virtuals(&settings),
        "verifier" => verify(),
        _ => {
            eprintln!("Commande attendue : textes, disposition, virtuels ou verifier");
            std::process::exit(2);
        }
    }
}
