//! What the Worx cloud says, as Moli's points, and Moli's orders as what
//! the mower expects: pure, tested on the fixtures.
//!
//! Everything here comes from the network and is treated as hostile: the
//! list is bounded, names are trimmed and bounded, the broker address and
//! the topics are checked before Moli connects or subscribes to anything,
//! and numbers are read only from the known tables (an unknown state is
//! `unknown`, never a guess).

use moli_core::{Access, Kind, PointSpec, Semantic, Unit, Value};
use serde_json::{Value as Json, json};

/// More mowers than any house has on one account: the rest is ignored.
pub const MAX_MOWERS: usize = 8;
const MAX_NAME: usize = 64;
const MAX_SERIAL: usize = 32;
const MAX_TOPIC: usize = 128;
const MAX_HOST: usize = 128;
/// The only domain a broker address from the cloud may end with (D5: the
/// cloud's answer never takes Moli anywhere else).
pub const BROKER_DOMAIN: &str = ".worxlandroid.com";
/// The longest rain delay the mower accepts, in minutes.
pub const MAX_RAIN_DELAY: i64 = 1440;

/// A mower's `ls`, by number (status table of the Landroid cloud).
pub const STATES: [(i64, &str); 23] = [
    (0, "idle"),
    (1, "home"),
    (2, "start_sequence"),
    (3, "leaving_home"),
    (4, "follow_wire"),
    (5, "searching_home"),
    (6, "searching_wire"),
    (7, "mowing"),
    (8, "lifted"),
    (9, "trapped"),
    (10, "blade_blocked"),
    (11, "debug"),
    (12, "remote_control"),
    (13, "fence_escape"),
    (30, "going_home"),
    (31, "zone_training"),
    (32, "border_cut"),
    (33, "searching_zone"),
    (34, "paused"),
    (103, "searching_zone"),
    (104, "searching_home"),
    (110, "border_crossing"),
    (111, "exploring_lawn"),
];

/// A mower's `le`, by number (error table of the Landroid cloud).
pub const ERRORS: [(i64, &str); 21] = [
    (0, "none"),
    (1, "trapped"),
    (2, "lifted"),
    (3, "wire_missing"),
    (4, "outside_wire"),
    (5, "rain_delay"),
    (6, "close_door_to_mow"),
    (7, "close_door_to_go_home"),
    (8, "blade_motor_blocked"),
    (9, "wheel_motor_blocked"),
    (10, "trapped_timeout"),
    (11, "upside_down"),
    (12, "battery_low"),
    (13, "reverse_wire"),
    (14, "charge_error"),
    (15, "timeout_finding_home"),
    (16, "locked"),
    (17, "battery_temperature"),
    (19, "battery_trunk_open"),
    (20, "wire_sync"),
    (21, "message_number"),
];

/// `control` values and the mower's `cmd` numbers.
pub const COMMANDS: [(&str, i64); 3] = [("start", 1), ("pause", 2), ("home", 3)];
/// `schedule` values and the schedule's `m`.
pub const SCHEDULES: [(&str, i64); 3] = [("off", 0), ("active", 1), ("party", 2)];

/// A mower Moli drives: protocol 0 (the Landroid wire models, L1000 among
/// them). Others (Vision, protocol 1) speak another language: left out.
#[derive(Clone, Debug, PartialEq)]
pub struct Mower {
    /// Stable: the serial number printed on the mower and in the app.
    pub serial: String,
    pub name: String,
    pub online: bool,
    pub firmware: Option<String>,
    /// The account's broker, checked to be Worx's.
    pub endpoint: String,
    pub command_in: String,
    pub command_out: String,
    /// The account, for the MQTT client id.
    pub user_id: String,
    /// The last status the cloud kept (`cfg` and `dat`), if any.
    pub last: Option<Json>,
}

fn serial(value: &Json) -> Option<String> {
    let s = value.as_str()?;
    (!s.is_empty() && s.len() <= MAX_SERIAL && s.bytes().all(|b| b.is_ascii_alphanumeric()))
        .then(|| s.to_owned())
}

/// The account number, as a plain token (a number in today's answers).
fn user_id(value: &Json) -> Option<String> {
    match value {
        Json::Number(n) => n.as_u64().map(|n| n.to_string()),
        other => serial(other),
    }
}

/// A broker address from the cloud: a host name under [`BROKER_DOMAIN`].
#[must_use]
pub fn broker(value: &Json) -> Option<String> {
    let host = value.as_str()?;
    (host.len() <= MAX_HOST
        && host.ends_with(BROKER_DOMAIN)
        && host.len() > BROKER_DOMAIN.len()
        && !host.starts_with(['.', '-'])
        && !host.contains("..")
        && host
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-'))
    .then(|| host.to_owned())
}

/// A topic from the cloud: printable, no wildcard (subscribing to `#`
/// would let one answer pull every mower's traffic), bounded.
#[must_use]
pub fn topic(value: &Json) -> Option<String> {
    let t = value.as_str()?;
    (!t.is_empty()
        && t.len() <= MAX_TOPIC
        && t.bytes()
            .all(|b| b.is_ascii_graphic() && b != b'#' && b != b'+'))
    .then(|| t.to_owned())
}

fn name(value: &Json, serial: &str) -> String {
    let name: String = value
        .as_str()
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME)
        .collect();
    let name = name.trim();
    if name.is_empty() {
        format!("Landroid {serial}")
    } else {
        name.to_owned()
    }
}

/// One entry of `product-items`, if Moli can drive it.
fn mower(entry: &Json) -> Option<Mower> {
    if entry["protocol"].as_i64() != Some(0) {
        return None;
    }
    let serial = serial(&entry["serial_number"])?;
    let payload = &entry["last_status"]["payload"];
    Some(Mower {
        name: name(&entry["name"], &serial),
        online: entry["online"].as_bool().unwrap_or(false),
        firmware: entry["firmware_version"]
            .as_str()
            .filter(|f| f.len() <= 16 && f.bytes().all(|b| b.is_ascii_graphic()))
            .map(str::to_owned),
        endpoint: broker(&entry["mqtt_endpoint"])?,
        command_in: topic(&entry["mqtt_topics"]["command_in"])?,
        command_out: topic(&entry["mqtt_topics"]["command_out"])?,
        user_id: user_id(&entry["user_id"])?,
        last: payload.is_object().then(|| payload.clone()),
        serial,
    })
}

/// The mowers of a `product-items` answer Moli can drive, at most
/// [`MAX_MOWERS`], each serial once. Entries that are not a protocol 0
/// mower, or whose broker or topics fail the checks, are left out.
pub fn mowers(body: &[u8]) -> anyhow::Result<Vec<Mower>> {
    let entries: Vec<Json> = serde_json::from_slice(body)?;
    let mut out: Vec<Mower> = Vec::new();
    for mower in entries.iter().filter_map(mower) {
        if out.len() == MAX_MOWERS {
            break;
        }
        if !out.iter().any(|m| m.serial == mower.serial) {
            out.push(mower);
        }
    }
    Ok(out)
}

fn spec(
    key: &str,
    label: String,
    kind: Kind,
    access: Access,
    unit: Option<Unit>,
    semantic: Semantic,
) -> PointSpec {
    PointSpec {
        key: key.into(),
        label: label.into(),
        kind,
        access,
        unit,
        semantic,
    }
}

const READ: Access = Access {
    read: true,
    write: false,
};

fn number(min: Option<f64>, max: Option<f64>) -> Kind {
    Kind::Numeric {
        min,
        max,
        step: None,
    }
}

fn names<T>(table: &[(T, &str)]) -> Kind {
    let mut values: Vec<std::sync::Arc<str>> = Vec::new();
    for (_, name) in table {
        if !values.iter().any(|v| &**v == *name) {
            values.push((*name).into());
        }
    }
    values.push("unknown".into());
    Kind::Enum { values }
}

/// A mower's points. The same for every protocol 0 mower: what a model
/// does not report stays `null`.
#[must_use]
#[allow(clippy::too_many_lines)] // a table, one entry per point
pub fn points() -> Vec<PointSpec> {
    vec![
        // Write-only: the mower moves, and its state says what it does.
        // `Control`: an agent asking waits for a person.
        spec(
            "control",
            moli_i18n::tr!("pilotes.landroid.commande"),
            Kind::Enum {
                values: COMMANDS.iter().map(|(c, _)| (*c).into()).collect(),
            },
            Access {
                read: false,
                write: true,
            },
            None,
            Semantic::Control,
        ),
        spec(
            "state",
            moli_i18n::tr!("pilotes.landroid.etat"),
            names(&STATES),
            READ,
            None,
            Semantic::Other,
        ),
        spec(
            "error",
            moli_i18n::tr!("pilotes.landroid.erreur"),
            names(&ERRORS),
            READ,
            None,
            Semantic::Other,
        ),
        spec(
            "battery",
            moli_i18n::tr!("pilotes.landroid.batterie"),
            number(Some(0.0), Some(100.0)),
            READ,
            Some(Unit::Percent),
            Semantic::Battery,
        ),
        spec(
            "charging",
            moli_i18n::tr!("pilotes.landroid.en_charge"),
            Kind::Binary,
            READ,
            None,
            Semantic::Other,
        ),
        spec(
            "battery_temperature",
            moli_i18n::tr!("pilotes.landroid.temperature_batterie"),
            number(None, None),
            READ,
            Some(Unit::Celsius),
            Semantic::Temperature,
        ),
        // Waiting after rain only delays mowing: a setting.
        spec(
            "rain_delay",
            moli_i18n::tr!("pilotes.landroid.delai_pluie"),
            Kind::Numeric {
                min: Some(0.0),
                #[allow(clippy::cast_precision_loss)]
                max: Some(MAX_RAIN_DELAY as f64),
                step: Some(1.0),
            },
            Access {
                read: true,
                write: true,
            },
            Some(Unit::Minute),
            Semantic::Config,
        ),
        spec(
            "rain",
            moli_i18n::tr!("pilotes.landroid.pluie"),
            Kind::Binary,
            READ,
            None,
            Semantic::Other,
        ),
        spec(
            "rain_remaining",
            moli_i18n::tr!("pilotes.landroid.pluie_restant"),
            number(Some(0.0), None),
            READ,
            Some(Unit::Minute),
            Semantic::Other,
        ),
        // Switching the schedule on lets the mower leave by itself later:
        // `Control`, like an order.
        spec(
            "schedule",
            moli_i18n::tr!("pilotes.landroid.planning"),
            Kind::Enum {
                values: SCHEDULES.iter().map(|(s, _)| (*s).into()).collect(),
            },
            Access {
                read: true,
                write: true,
            },
            None,
            Semantic::Control,
        ),
        spec(
            "schedule_week",
            moli_i18n::tr!("pilotes.landroid.semaine"),
            Kind::Text,
            READ,
            None,
            Semantic::Other,
        ),
        spec(
            "wifi_signal",
            moli_i18n::tr!("pilotes.landroid.wifi"),
            number(None, None),
            READ,
            Some(Unit::Other("dBm".into())),
            Semantic::SignalStrength,
        ),
        spec(
            "blade_hours",
            moli_i18n::tr!("pilotes.landroid.lame_heures"),
            number(Some(0.0), None),
            READ,
            Some(Unit::Hour),
            Semantic::Other,
        ),
        spec(
            "mowing_hours",
            moli_i18n::tr!("pilotes.landroid.tonte_heures"),
            number(Some(0.0), None),
            READ,
            Some(Unit::Hour),
            Semantic::Other,
        ),
    ]
}

fn lookup(table: &[(i64, &'static str)], code: Option<i64>) -> Value {
    code.map_or(Value::Null, |code| {
        Value::from(
            table
                .iter()
                .find(|(c, _)| *c == code)
                .map_or("unknown", |(_, n)| *n),
        )
    })
}

/// A finite number in `[min, max]`, else nothing.
fn within(value: &Json, min: f64, max: f64) -> Option<f64> {
    value
        .as_f64()
        .filter(|v| v.is_finite() && (min..=max).contains(v))
}

fn int(value: &Json, min: f64, max: f64) -> Value {
    #[allow(clippy::cast_possible_truncation)]
    within(value, min, max).map_or(Value::Null, |v| Value::Int(v.round() as i64))
}

fn float(value: &Json, min: f64, max: f64) -> Value {
    within(value, min, max).map_or(Value::Null, |v| Value::Float((v * 10.0).round() / 10.0))
}

fn flag(value: &Json) -> Value {
    match value.as_i64() {
        Some(0) => Value::Bool(false),
        Some(1) => Value::Bool(true),
        _ => value.as_bool().map_or(Value::Null, Value::Bool),
    }
}

/// Minutes from the counters, as hours.
fn hours(value: &Json) -> Value {
    within(value, 0.0, 1e9).map_or(Value::Null, |m| Value::Float((m / 6.0).round() / 10.0))
}

/// The status's schedule mode, if it is one Moli knows.
fn schedule(sc: &Json) -> Value {
    sc["m"].as_i64().map_or(Value::Null, |m| {
        Value::from(
            SCHEDULES
                .iter()
                .find(|(_, n)| *n == m)
                .map_or("unknown", |(s, _)| *s),
        )
    })
}

/// One slot `["HH:MM", minutes, border]`, if it mows.
fn slot(entry: &Json) -> Option<String> {
    let start = entry[0].as_str()?;
    let ok = start.len() == 5
        && start.as_bytes()[2] == b':'
        && start
            .bytes()
            .enumerate()
            .all(|(i, b)| i == 2 || b.is_ascii_digit());
    let minutes = entry[1].as_i64().filter(|m| (1..=1440).contains(m))?;
    if !ok {
        return None;
    }
    let border = if entry[2].as_i64() == Some(1) {
        moli_i18n::tr!("pilotes.landroid.bordure")
    } else {
        String::new()
    };
    Some(format!("{start} {minutes} min{border}"))
}

/// The week of the schedule in words: `lun. 10:00 300 min, bordure ; …`,
/// Monday first, days without mowing left out. Day 0 is Sunday (`d`, and
/// the second slot `dd` some models have).
fn week(sc: &Json) -> Value {
    let Some(days) = sc["d"].as_array().filter(|d| d.len() == 7) else {
        return Value::Null;
    };
    let labels = moli_i18n::tr!("pilotes.landroid.jours");
    let labels: Vec<&str> = labels.split(',').collect();
    let second = sc["dd"].as_array().filter(|d| d.len() == 7);
    let mut parts = Vec::new();
    for day in [1, 2, 3, 4, 5, 6, 0] {
        let slots: Vec<String> = std::iter::once(&days[day])
            .chain(second.map(|s| &s[day]))
            .filter_map(slot)
            .collect();
        if !slots.is_empty() {
            let label = labels.get(day).copied().unwrap_or("?");
            parts.push(format!("{label} {}", slots.join(" + ")));
        }
    }
    if parts.is_empty() {
        Value::from(moli_i18n::tr!("pilotes.landroid.aucune_tonte").as_str())
    } else {
        Value::from(parts.join(" ; ").as_str())
    }
}

/// The values a status (`cfg` and `dat`) says, every point's key once:
/// what is absent or out of range is `null`.
#[must_use]
pub fn values(status: &Json) -> Vec<(&'static str, Value)> {
    let (cfg, dat) = (&status["cfg"], &status["dat"]);
    let rain = &dat["rain"];
    vec![
        ("state", lookup(&STATES, dat["ls"].as_i64())),
        ("error", lookup(&ERRORS, dat["le"].as_i64())),
        ("battery", int(&dat["bt"]["p"], 0.0, 100.0)),
        ("charging", flag(&dat["bt"]["c"])),
        ("battery_temperature", float(&dat["bt"]["t"], -40.0, 100.0)),
        (
            "rain_delay",
            #[allow(clippy::cast_precision_loss)]
            int(&cfg["rd"], 0.0, MAX_RAIN_DELAY as f64),
        ),
        ("rain", flag(&rain["s"])),
        ("rain_remaining", int(&rain["cnt"], 0.0, 100_000.0)),
        ("schedule", schedule(&cfg["sc"])),
        ("schedule_week", week(&cfg["sc"])),
        ("wifi_signal", int(&dat["rsi"], -150.0, 0.0)),
        ("blade_hours", hours(&dat["st"]["b"])),
        ("mowing_hours", hours(&dat["st"]["wt"])),
    ]
}

/// Whether a status is this mower's: one that names another serial is
/// not (a topic shared by mistake must not mix two mowers).
#[must_use]
pub fn belongs(status: &Json, serial: &str) -> bool {
    status["cfg"].is_object()
        && status["dat"].is_object()
        && status["cfg"]["sn"].as_str().is_none_or(|sn| sn == serial)
}

/// What a write asks of the mower, before its envelope. `schedule` needs
/// the last schedule seen: the mower takes the whole `sc` back, so Moli
/// sends it as it is with only the mode changed.
pub fn order(key: &str, value: &Value, schedule: Option<&Json>) -> Result<Json, String> {
    match (key, value) {
        ("control", Value::Text(c)) => COMMANDS
            .iter()
            .find(|(name, _)| **name == **c)
            .map(|(_, cmd)| json!({ "cmd": cmd }))
            .ok_or_else(|| moli_i18n::tr!("pilotes.landroid.valeur_inconnue", value = c)),
        ("rain_delay", Value::Int(minutes)) if (0..=MAX_RAIN_DELAY).contains(minutes) => {
            Ok(json!({ "rd": minutes }))
        }
        ("rain_delay", Value::Float(minutes))
            if minutes.fract() == 0.0 && (0.0..=1440.0).contains(minutes) =>
        {
            #[allow(clippy::cast_possible_truncation)]
            Ok(json!({ "rd": *minutes as i64 }))
        }
        ("schedule", Value::Text(mode)) => {
            let m = SCHEDULES
                .iter()
                .find(|(name, _)| **name == **mode)
                .map(|(_, m)| *m)
                .ok_or_else(|| moli_i18n::tr!("pilotes.landroid.valeur_inconnue", value = mode))?;
            let Some(Json::Object(sc)) = schedule else {
                return Err(moli_i18n::tr!("pilotes.landroid.planning_inconnu"));
            };
            let mut sc = sc.clone();
            sc.insert("m".into(), json!(m));
            if sc.contains_key("distm") {
                sc.insert("distm".into(), json!(0));
            }
            Ok(json!({ "sc": sc }))
        }
        (key, _) => Err(moli_i18n::tr!("pilotes.landroid.ne_s_ecrit_pas", key = key)),
    }
}

/// The envelope of a protocol 0 message: a message number, the serial,
/// and the mower's (local) time and date, the order merged in.
#[must_use]
pub fn envelope(order: &Json, serial: &str, id: u16, now: &jiff::civil::DateTime) -> Vec<u8> {
    let mut message = json!({
        "id": id,
        "sn": serial,
        "tm": now.strftime("%H:%M:%S").to_string(),
        "dt": now.strftime("%d/%m/%Y").to_string(),
    });
    if let (Json::Object(m), Json::Object(o)) = (&mut message, order) {
        for (k, v) in o {
            m.insert(k.clone(), v.clone());
        }
    }
    message.to_string().into_bytes()
}

/// The MQTT user name that carries the access token to Worx's broker (an
/// AWS IoT custom authorizer): the token's header and claims as `jwt`, its
/// signature apart, in standard base64 and percent-encoded. `None`: not a
/// JWT.
#[must_use]
pub fn username(token: &str) -> Option<String> {
    let standard = token.replace('_', "/").replace('-', "+");
    let parts: Vec<&str> = standard.split('.').collect();
    let [header, claims, signature] = parts.as_slice() else {
        return None;
    };
    if [header, claims, signature].iter().any(|p| p.is_empty()) {
        return None;
    }
    Some(format!(
        "bot?jwt={}.{}&x-amz-customauthorizer-name=''&x-amz-customauthorizer-signature={}",
        quote(header),
        quote(claims),
        quote(signature)
    ))
}

/// Percent-encoding as Python's `urllib.parse.quote` (`/` kept).
fn quote(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
                char::from(b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ITEMS: &[u8] =
        include_bytes!("../../../integrations/landroid/fixtures/product-items.json");
    const MOWING: &[u8] =
        include_bytes!("../../../integrations/landroid/fixtures/status-mowing.json");

    fn value<'a>(values: &'a [(&str, Value)], key: &str) -> &'a Value {
        &values.iter().find(|(k, _)| *k == key).unwrap().1
    }

    #[test]
    fn the_account_lists_the_wire_mowers_only() {
        let mowers = mowers(ITEMS).unwrap();
        // The Vision (protocol 1) and the one with a foreign broker and a
        // wildcard topic are left out.
        assert_eq!(mowers.len(), 1);
        let m = &mowers[0];
        assert_eq!(m.serial, "20000000000000000001");
        assert_eq!(m.name, "Tondeuse");
        assert!(m.online);
        assert_eq!(m.endpoint, "iot.eu-west-1.worxlandroid.com");
        assert_eq!(m.command_in, "PRM100/AABBCCDDEEFF/commandIn");
        assert_eq!(m.command_out, "PRM100/AABBCCDDEEFF/commandOut");
        assert_eq!(m.user_id, "12345");
        assert_eq!(m.firmware.as_deref(), Some("3.30"));
        let last = values(m.last.as_ref().unwrap());
        assert_eq!(value(&last, "state"), &Value::from("home"));
        assert_eq!(value(&last, "charging"), &Value::Bool(true));
        assert_eq!(value(&last, "battery"), &Value::Int(87));
        assert_eq!(value(&last, "rain_delay"), &Value::Int(60));
    }

    #[test]
    fn hostile_answers_are_bounded() {
        let many: Vec<String> = (0..100)
            .map(|i| {
                format!(
                    r#"{{"serial_number":"S{i}","protocol":0,"user_id":1,"name":"{}","mqtt_endpoint":"iot.worxlandroid.com","mqtt_topics":{{"command_in":"a/{i}/in","command_out":"a/{i}/out"}}}}"#,
                    "x".repeat(500)
                )
            })
            .collect();
        let listed = mowers(format!("[{}]", many.join(",")).as_bytes()).unwrap();
        assert_eq!(listed.len(), MAX_MOWERS);
        assert!(listed.iter().all(|m| m.name.chars().count() == MAX_NAME));

        for bad in [
            "worxlandroid.com.example.org",
            "example.org",
            ".worxlandroid.com",
            "a..worxlandroid.com",
            "IOT.worxlandroid.com",
            "iot.worxlandroid.com:8443",
            "10.0.0.1",
        ] {
            assert_eq!(broker(&Json::from(bad)), None, "{bad}");
        }
        for bad in ["a/#", "a/+/b", "", "a b", &"a".repeat(200)] {
            assert_eq!(topic(&Json::from(bad)), None, "{bad:?}");
        }
        for bad in ["../x", "a b", &"1".repeat(33)] {
            assert_eq!(serial(&Json::from(bad)), None, "{bad:?}");
        }
        // The same serial twice is one mower.
        let twice = br#"[
            {"serial_number":"S1","protocol":0,"user_id":1,"mqtt_endpoint":"iot.worxlandroid.com","mqtt_topics":{"command_in":"a/in","command_out":"a/out"}},
            {"serial_number":"S1","protocol":0,"user_id":1,"mqtt_endpoint":"iot.worxlandroid.com","mqtt_topics":{"command_in":"b/in","command_out":"b/out"}}
        ]"#;
        let listed = mowers(twice).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "Landroid S1");
        assert!(mowers(b"{}").is_err());
    }

    #[test]
    fn a_status_becomes_points() {
        let status: Json = serde_json::from_slice(MOWING).unwrap();
        let values = values(&status);
        let keys: Vec<&str> = values.iter().map(|(k, _)| *k).collect();
        let readable: Vec<String> = points()
            .into_iter()
            .filter(|p| p.access.read)
            .map(|p| p.key.to_string())
            .collect();
        assert_eq!(keys, readable, "every readable point, once");
        assert_eq!(value(&values, "state"), &Value::from("mowing"));
        assert_eq!(value(&values, "error"), &Value::from("none"));
        assert_eq!(value(&values, "battery"), &Value::Int(74));
        assert_eq!(value(&values, "charging"), &Value::Bool(false));
        assert_eq!(value(&values, "battery_temperature"), &Value::Float(24.0));
        assert_eq!(value(&values, "rain"), &Value::Bool(false));
        assert_eq!(value(&values, "rain_remaining"), &Value::Int(0));
        assert_eq!(value(&values, "schedule"), &Value::from("active"));
        assert_eq!(value(&values, "wifi_signal"), &Value::Int(-67));
        assert_eq!(value(&values, "blade_hours"), &Value::Float(500.3));
        assert_eq!(value(&values, "mowing_hours"), &Value::Float(600.3));
        let Value::Text(week) = value(&values, "schedule_week") else {
            panic!("week");
        };
        assert!(week.starts_with("lun. 10:00 300 min"), "{week}");
        assert!(week.contains("mer. 10:00 120 min"), "{week}");
        assert!(week.contains("ven. 14:30 90 min"), "{week}");
        assert!(!week.contains("dim."), "{week}");
        assert!(belongs(&status, "20000000000000000001"));
        assert!(!belongs(&status, "20000000000000000002"));
        assert!(!belongs(&json!({"dat": {}}), "x"));
    }

    #[test]
    fn unknown_or_absurd_numbers_are_not_guessed() {
        let status = json!({
            "cfg": {"rd": 99999, "sc": {"m": 7, "d": [["25:00", 10, 0]]}},
            "dat": {"ls": 77, "le": 99, "bt": {"p": 300, "c": 5, "t": "hot"}, "rsi": 12}
        });
        let values = values(&status);
        assert_eq!(value(&values, "state"), &Value::from("unknown"));
        assert_eq!(value(&values, "error"), &Value::from("unknown"));
        assert_eq!(value(&values, "schedule"), &Value::from("unknown"));
        for key in [
            "battery",
            "charging",
            "battery_temperature",
            "rain_delay",
            "rain",
            "schedule_week",
            "wifi_signal",
        ] {
            assert_eq!(value(&values, key), &Value::Null, "{key}");
        }
        let empty = json!({"cfg": {"sc": {"d": [["00:00",0,0],["00:00",0,0],["00:00",0,0],["00:00",0,0],["00:00",0,0],["00:00",0,0],["00:00",0,0]]}}, "dat": {}});
        assert_eq!(
            value(&super::values(&empty), "schedule_week"),
            &Value::from("aucune tonte prévue")
        );
    }

    #[test]
    fn orders_are_what_the_mower_expects() {
        assert_eq!(
            order("control", &Value::from("start"), None).unwrap(),
            json!({"cmd": 1})
        );
        assert_eq!(
            order("control", &Value::from("pause"), None).unwrap(),
            json!({"cmd": 2})
        );
        assert_eq!(
            order("control", &Value::from("home"), None).unwrap(),
            json!({"cmd": 3})
        );
        assert!(order("control", &Value::from("unlock"), None).is_err());
        assert_eq!(
            order("rain_delay", &Value::Int(90), None).unwrap(),
            json!({"rd": 90})
        );
        assert_eq!(
            order("rain_delay", &Value::Float(30.0), None).unwrap(),
            json!({"rd": 30})
        );
        assert!(order("rain_delay", &Value::Int(2000), None).is_err());
        assert!(order("rain_delay", &Value::Float(1.5), None).is_err());
        assert!(order("state", &Value::from("home"), None).is_err());

        // The schedule goes back whole, only its mode changed.
        let sc = json!({"m": 1, "p": 0, "distm": 3, "d": [["10:00", 60, 0]]});
        assert_eq!(
            order("schedule", &Value::from("party"), Some(&sc)).unwrap(),
            json!({"sc": {"m": 2, "p": 0, "distm": 0, "d": [["10:00", 60, 0]]}})
        );
        assert_eq!(
            order("schedule", &Value::from("off"), Some(&sc)).unwrap()["sc"]["m"],
            json!(0)
        );
        assert!(order("schedule", &Value::from("active"), None).is_err());
    }

    #[test]
    fn the_envelope_carries_the_serial_and_the_mower_time() {
        let now: jiff::civil::DateTime = "2026-10-01T09:05:07".parse().unwrap();
        let raw = envelope(&json!({"cmd": 3}), "SERIAL1", 2048, &now);
        let message: Json = serde_json::from_slice(&raw).unwrap();
        assert_eq!(
            message,
            json!({"id": 2048, "sn": "SERIAL1", "tm": "09:05:07", "dt": "01/10/2026", "cmd": 3})
        );
    }

    #[test]
    fn the_token_rides_in_the_user_name() {
        // header.claims.signature, base64url with `-` and `_`.
        let name = username("eyJh_bGc.eyJz-dWI.c2ln_bmF0-dXJl").unwrap();
        assert_eq!(
            name,
            "bot?jwt=eyJh/bGc.eyJz%2BdWI&x-amz-customauthorizer-name=''&x-amz-customauthorizer-signature=c2ln/bmF0%2BdXJl"
        );
        assert_eq!(username("not-a-jwt"), None);
        assert_eq!(username("a..c"), None);
    }
}
