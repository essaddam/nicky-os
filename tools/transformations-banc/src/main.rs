use std::hint::black_box;
use std::time::Instant;

use moli_core::{Unit, Value, unit_ratio, validate};
use moli_profile::{Answer, Format, Profile, Source};
use serde_json::json;
use transformations_banc::{
    Affine, Command, PROFILE, PROFILE_JSON, action, action_index, action_indexed, catalog,
    compiled, control, ingress, payload, read_profile,
};

fn measure(
    name: &str,
    iterations: usize,
    samples: usize,
    devices: usize,
    boundary: &str,
    mut f: impl FnMut(usize) -> usize,
) {
    let mut checksum = 0usize;
    for i in 0..iterations.min(1_000) {
        checksum = checksum.wrapping_add(black_box(f(black_box(i))));
    }
    let mut values = Vec::with_capacity(samples);
    for _ in 0..samples {
        let start = Instant::now();
        for i in 0..iterations {
            checksum = checksum.wrapping_add(black_box(f(black_box(i))));
        }
        values.push(start.elapsed().as_secs_f64() * 1e9 / iterations as f64);
    }
    let mut sorted = values.clone();
    sorted.sort_by(f64::total_cmp);
    println!(
        "{}",
        json!({
            "type":"measurement", "name":name, "devices":devices,
            "iterations_per_sample":iterations, "samples":samples,
            "boundary":boundary, "sample_mean_ns_per_operation":values,
            "median_sample_mean_ns_per_operation":sorted[sorted.len()/2],
            "best_sample_mean_ns_per_operation":sorted[0],
            "worst_sample_mean_ns_per_operation":sorted[sorted.len()-1],
            "checksum":checksum
        })
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let iterations: usize = args
        .get(1)
        .map_or(20_000, |s| s.parse().expect("positive iterations"));
    let samples: usize = args
        .get(2)
        .map_or(7, |s| s.parse().expect("positive samples"));
    assert!(iterations > 0 && samples > 0);
    println!(
        "{}",
        json!({"type":"metadata", "schema":1,
            "production_calls":"moli-z2m Catalog/Entry; moli-profile Profile/Answer/CompiledPoint/CompiledWrite; moli-core validate/unit_ratio",
            "transport":"none", "payloads":"synthetic", "release_debug_assertions":cfg!(debug_assertions),
            "timing":"independent batch means; median/best/worst are not per-message latency percentiles",
            "baseline":"profile_recompile_each_message is deliberately naive, not the production implementation",
            "excluded":"MQTT framing/broker/socket, HTTP server/socket, Hub state/guard/approval/journal/dispatch, physical commands"
        })
    );
    let payloads: Vec<_> = (0..32).map(payload).collect();
    for devices in [1, 100, 1_000] {
        let cat = catalog(devices);
        let corpus = devices.max(32);
        let topics: Vec<_> = (0..corpus)
            .map(|i| format!("synthetic/lamp-{}", i % devices))
            .collect();
        measure(
            "mqtt_bytes_route_decode",
            iterations,
            samples,
            devices,
            "topic classification + Catalog::by_name + JSON bytes parse + Entry::decode; 8 output values; no Hub",
            |i| {
                ingress(
                    black_box(&cat),
                    black_box(&topics[i % corpus]),
                    black_box(&payloads[i % 32]),
                )
                .unwrap()
                .len()
            },
        );
        let commands: Vec<_> = (0..corpus)
            .map(|i| {
                serde_json::to_vec(&json!({
            "point":format!("synthetic:synthetic-{:08}/brightness",i%devices),"value":(i*7)%255
        })).unwrap()
            })
            .collect();
        measure(
            "action_api_json_lookup_validate_encode",
            iterations,
            samples,
            devices,
            "scalar API JSON bytes + PointId::split + Catalog::by_id + access/spec validation + Entry::encode + JSON serialization; no Hub",
            |i| {
                action(black_box(&cat), black_box(&commands[i % corpus]))
                    .unwrap()
                    .len()
            },
        );
        let index = action_index(&cat);
        measure(
            "experimental_action_api_json_index_validate_encode",
            iterations,
            samples,
            devices,
            "EXPERIMENT ONLY: same scalar API JSON + identity/spec/access/validate + real Entry::encode + serialization, replacing Catalog::by_id scan with prebuilt borrowed ID index; excludes index construction/Hub",
            |i| {
                action_indexed(black_box(&index), black_box(&commands[i % corpus]))
                    .unwrap()
                    .len()
            },
        );
        measure(
            "experimental_action_index_rebuild",
            (iterations / devices).max(1),
            samples,
            devices,
            "EXPERIMENT ONLY: construct and drop borrowed ID-to-Entry HashMap for all catalog entries; allocation/hash/Arc clones included; replacement must invalidate old index",
            |_| black_box(action_index(black_box(&cat))).len(),
        );
        let entry = cat.by_name("lamp-0").unwrap();
        measure(
            "action_codec_only",
            iterations,
            samples,
            devices,
            "pre-resolved Entry::encode on already validated native scalar; no JSON input/routing/Hub",
            |i| {
                let result = entry.encode(
                    black_box("brightness"),
                    black_box(&Value::Float((i % 255) as f64)),
                );
                black_box(result).as_object().unwrap().len()
            },
        );
    }
    let profile = compiled();
    measure(
        "profile_json_parse_and_compiled_read",
        iterations,
        samples,
        1,
        "Answer::parse JSON bytes + 5 CompiledPoint::read; descriptors compiled once as in production",
        |_| black_box(read_profile(black_box(&profile), black_box(PROFILE_JSON))).len(),
    );
    let answer = Answer::parse(Format::Json, PROFILE_JSON).unwrap();
    measure(
        "profile_compiled_read_only",
        iterations,
        samples,
        1,
        "5 CompiledPoint::read on parsed Answer; excludes JSON bytes parse",
        |_| {
            profile
                .points
                .iter()
                .filter_map(|p| black_box(p.read(black_box(&answer))))
                .count()
        },
    );
    measure(
        "profile_recompile_each_message_naive_baseline",
        (iterations / 100).max(1),
        samples,
        1,
        "deliberately naive authoring baseline: parse TOML + compile + parse JSON + same 5 reads; NOT current production",
        |_| {
            let p = Profile::parse(black_box(PROFILE))
                .unwrap()
                .compile(Source::File)
                .unwrap();
            black_box(read_profile(&p, black_box(PROFILE_JSON))).len()
        },
    );
    let daikin = Profile::parse(moli_profile::builtin("daikin-brp069").unwrap())
        .unwrap()
        .compile(Source::Builtin)
        .unwrap();
    let answers = control();
    let write = daikin
        .writes
        .iter()
        .find(|w| &*w.key == "target_temperature")
        .unwrap();
    let point = daikin
        .points
        .iter()
        .find(|p| &*p.spec.key == "target_temperature")
        .unwrap();
    let commands: Vec<_> = (0..32)
        .map(|i| {
            serde_json::to_vec(&json!({
                "point":"synthetic:unit/target_temperature","value":16.0+(i%29) as f64/2.0
            }))
            .unwrap()
        })
        .collect();
    measure(
        "profile_action_api_json_validate_render",
        iterations,
        samples,
        1,
        "scalar API JSON parse + point/key split + compiled point/write lookup + validate + CompiledWrite::render with cached latest KV answers; no HTTP or Hub",
        |i| {
            let command: Command = serde_json::from_slice(black_box(&commands[i % 32])).unwrap();
            let (_, key) = command.point.split().unwrap();
            let point = daikin.points.iter().find(|p| &*p.spec.key == key).unwrap();
            let write = daikin.writes.iter().find(|w| &*w.key == key).unwrap();
            let value = validate(&point.spec.kind, &command.value).unwrap();
            black_box(write.render(&value, black_box(&answers), |_| None).unwrap()).len()
        },
    );
    measure(
        "profile_action_validate_render_only",
        iterations,
        samples,
        1,
        "cached point/write/answers + real validate + CompiledWrite::render; excludes API JSON input",
        |i| {
            let value = validate(
                &point.spec.kind,
                black_box(&Value::Float(16.0 + (i % 29) as f64 / 2.0)),
            )
            .unwrap();
            black_box(write.render(&value, black_box(&answers), |_| None).unwrap()).len()
        },
    );
    measure(
        "core_unit_ratio_lookup",
        iterations,
        samples,
        1,
        "production compatible-unit ratio selection; conversion arithmetic only after explicit ratio",
        |i| {
            let (from, to) = if i.is_multiple_of(2) {
                (&Unit::WattHour, &Unit::KiloWattHour)
            } else {
                (&Unit::MilliVolt, &Unit::Volt)
            };
            black_box(
                unit_ratio(black_box(Some(from)), black_box(Some(to))).unwrap()
                    * black_box(i as f64),
            )
            .to_bits() as usize
        },
    );
    let affine = Affine::compile(0.0, 254.0, false, true).unwrap();
    measure(
        "experimental_affine_inbound",
        iterations,
        samples,
        1,
        "EXPERIMENT ONLY, precompiled native 0..254 -> percent 0..100; reject out of bounds; no production integration",
        |i| black_box(affine.inbound(black_box((i % 255) as f64)).unwrap()).to_bits() as usize,
    );
    measure(
        "experimental_affine_outbound",
        iterations,
        samples,
        1,
        "EXPERIMENT ONLY, percent 0..100 -> native 0..254; reject out of bounds then round native integer; no production integration",
        |i| black_box(affine.outbound(black_box((i % 101) as f64)).unwrap()).to_bits() as usize,
    );
}
