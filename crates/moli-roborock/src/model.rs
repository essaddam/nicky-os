//! What the vacuum says, as Moli's points, and Moli's orders as miIO
//! methods: pure, tested on the fixtures.
//!
//! The vacuum's answers come from the network and are treated as hostile:
//! numbers are read only within their range and from the known tables (an
//! unknown state is `unknown`, never a guess), texts are bounded.

use moli_core::{Access, Kind, PointSpec, Semantic, Unit, Value};
use serde_json::Value as Json;

/// `get_status` `state`, by number (Roborock V1 vacuums).
pub const STATES: [(i64, &str); 23] = [
    (1, "starting"),
    (2, "charger_disconnected"),
    (3, "idle"),
    (4, "remote_control"),
    (5, "cleaning"),
    (6, "returning_home"),
    (7, "manual_mode"),
    (8, "charging"),
    (9, "charging_problem"),
    (10, "paused"),
    (11, "spot_cleaning"),
    (12, "error"),
    (13, "shutting_down"),
    (14, "updating"),
    (15, "docking"),
    (16, "going_to_target"),
    (17, "zoned_cleaning"),
    (18, "segment_cleaning"),
    (22, "emptying_bin"),
    (23, "washing_mop"),
    (26, "going_to_wash_mop"),
    (100, "charging_complete"),
    (101, "offline"),
];

/// States in which the vacuum is out of its dock, working.
const ACTIVE: [i64; 8] = [1, 4, 5, 6, 11, 15, 16, 17];
const SEGMENTS: i64 = 18;

/// `get_status` `error_code`, by number.
pub const ERRORS: [(i64, &str); 28] = [
    (0, "none"),
    (1, "laser_sensor"),
    (2, "collision_sensor"),
    (3, "wheels_over_void"),
    (4, "cliff_sensors_dirty"),
    (5, "main_brush"),
    (6, "side_brush"),
    (7, "main_wheel_stuck"),
    (8, "stuck"),
    (9, "dust_bin_missing"),
    (10, "filter"),
    (11, "magnetic_barrier"),
    (12, "battery_low"),
    (13, "charging_fault"),
    (14, "battery_fault"),
    (15, "wall_sensor_dirty"),
    (16, "not_flat"),
    (17, "side_brush_fault"),
    (18, "suction_fan"),
    (19, "dock_unpowered"),
    (20, "unknown_error"),
    (21, "laser_blocked"),
    (22, "dock_contacts_dirty"),
    (23, "dock_unreachable"),
    (24, "no_go_zone"),
    (26, "wall_sensor_dirty"),
    (27, "vibrarise_jammed"),
    (28, "on_carpet"),
];

/// `control` values and their miIO methods.
pub const COMMANDS: [(&str, &str); 4] = [
    ("start", "app_start"),
    ("pause", "app_pause"),
    ("stop", "app_stop"),
    ("home", "app_charge"),
];

/// The families of models this driver speaks to (miIO `model`).
const MODELS: [&str; 2] = ["roborock.vacuum.", "rockrobo.vacuum."];
const MAX_MODEL: usize = 48;

/// The vacuum's model from `miIO.info`, if it is a Roborock vacuum.
/// Anything else (another miIO device at that address) is not driven.
#[must_use]
pub fn model(info: &Json) -> Option<String> {
    let model = info["model"].as_str()?;
    (model.len() <= MAX_MODEL
        && MODELS.iter().any(|m| model.starts_with(m))
        && model
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'_'))
    .then(|| model.to_owned())
}

/// The firmware from `miIO.info`, bounded, if plain.
#[must_use]
pub fn firmware(info: &Json) -> Option<String> {
    info["fw_ver"]
        .as_str()
        .filter(|f| f.len() <= 24 && f.bytes().all(|b| b.is_ascii_graphic()))
        .map(str::to_owned)
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

fn names(table: &[(i64, &str)]) -> Kind {
    let mut values: Vec<std::sync::Arc<str>> = Vec::new();
    for (_, name) in table {
        if !values.iter().any(|v| &**v == *name) {
            values.push((*name).into());
        }
    }
    values.push("unknown".into());
    Kind::Enum { values }
}

fn number(min: Option<f64>, max: Option<f64>) -> Kind {
    Kind::Numeric {
        min,
        max,
        step: None,
    }
}

/// The vacuum's points.
#[must_use]
pub fn points() -> Vec<PointSpec> {
    vec![
        // Write-only: the vacuum moves, and its state says what it does.
        // `Control`: an agent asking waits for a person.
        spec(
            "control",
            moli_i18n::tr!("pilotes.roborock.commande"),
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
            moli_i18n::tr!("pilotes.roborock.etat"),
            names(&STATES),
            READ,
            None,
            Semantic::Other,
        ),
        spec(
            "error",
            moli_i18n::tr!("pilotes.roborock.erreur"),
            names(&ERRORS),
            READ,
            None,
            Semantic::Other,
        ),
        spec(
            "battery",
            moli_i18n::tr!("pilotes.roborock.batterie"),
            number(Some(0.0), Some(100.0)),
            READ,
            Some(Unit::Percent),
            Semantic::Battery,
        ),
        spec(
            "cleaning",
            moli_i18n::tr!("pilotes.roborock.en_menage"),
            Kind::Binary,
            READ,
            None,
            Semantic::Other,
        ),
        spec(
            "fan_power",
            moli_i18n::tr!("pilotes.roborock.aspiration"),
            number(Some(0.0), Some(110.0)),
            READ,
            None,
            Semantic::Other,
        ),
        spec(
            "clean_time",
            moli_i18n::tr!("pilotes.roborock.duree_menage"),
            number(Some(0.0), None),
            READ,
            Some(Unit::Minute),
            Semantic::Other,
        ),
        spec(
            "clean_area",
            moli_i18n::tr!("pilotes.roborock.surface_menage"),
            number(Some(0.0), None),
            READ,
            Some(Unit::Other("m²".into())),
            Semantic::Other,
        ),
    ]
}

/// A finite number in `[min, max]`, else nothing.
fn within(value: &Json, min: f64, max: f64) -> Option<f64> {
    value
        .as_f64()
        .filter(|v| v.is_finite() && (min..=max).contains(v))
}

#[allow(clippy::cast_possible_truncation)]
fn int(value: &Json, min: f64, max: f64) -> Value {
    within(value, min, max).map_or(Value::Null, |v| Value::Int(v.round() as i64))
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

/// The status of a `get_status` answer (`result[0]`), if there is one.
#[must_use]
pub fn status(result: &Json) -> Option<&Json> {
    let status = result.get(0)?;
    let state = status["state"].as_i64()?;
    let battery = status["battery"].as_u64()?;
    (STATES.iter().any(|(code, _)| *code == state) && battery <= 100).then_some(status)
}

/// Whether the vacuum is out working (polled more often).
#[must_use]
pub fn active(status: &Json) -> bool {
    status["state"]
        .as_i64()
        .is_some_and(|s| ACTIVE.contains(&s) || s == SEGMENTS)
}

/// The values a status says, every readable point's key once: what is
/// absent or out of range is `null`.
#[must_use]
pub fn values(status: &Json) -> Vec<(&'static str, Value)> {
    let state = status["state"].as_i64();
    vec![
        ("state", lookup(&STATES, state)),
        ("error", lookup(&ERRORS, status["error_code"].as_i64())),
        ("battery", int(&status["battery"], 0.0, 100.0)),
        (
            "cleaning",
            state.map_or(Value::Null, |_| Value::Bool(active(status))),
        ),
        ("fan_power", int(&status["fan_power"], 0.0, 110.0)),
        (
            "clean_time",
            within(&status["clean_time"], 0.0, 1e7)
                .map_or(Value::Null, |s| Value::Float((s / 6.0).round() / 10.0)),
        ),
        (
            "clean_area",
            within(&status["clean_area"], 0.0, 1e12)
                .map_or(Value::Null, |mm2| Value::Float((mm2 / 1e5).round() / 10.0)),
        ),
    ]
}

/// The miIO method a write asks for.
pub fn order(key: &str, value: &Value) -> Result<&'static str, String> {
    match (key, value) {
        ("control", Value::Text(c)) => COMMANDS
            .iter()
            .find(|(name, _)| **name == **c)
            .map(|(_, method)| *method)
            .ok_or_else(|| moli_i18n::tr!("pilotes.roborock.valeur_inconnue", value = c)),
        (key, _) => Err(moli_i18n::tr!("pilotes.roborock.ne_s_ecrit_pas", key = key)),
    }
}

/// An order's answer: `["ok"]` is done; anything else is refused, with the
/// device's message bounded and plain.
pub fn outcome(answer: &Json) -> Result<(), String> {
    if answer["result"].get(0).and_then(Json::as_str) == Some("ok") {
        return Ok(());
    }
    let message: String = answer["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || " _-.:".contains(*c))
        .take(80)
        .collect();
    Err(moli_i18n::tr!(
        "pilotes.roborock.refuse",
        why = if message.is_empty() {
            "?".to_owned()
        } else {
            message
        }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const STATUS: &[u8] = include_bytes!("../../../integrations/roborock/fixtures/get_status.json");
    const INFO: &[u8] = include_bytes!("../../../integrations/roborock/fixtures/miio-info.json");
    const OK: &[u8] = include_bytes!("../../../integrations/roborock/fixtures/app_start.json");
    const ERROR: &[u8] = include_bytes!("../../../integrations/roborock/fixtures/error.json");

    fn value<'a>(values: &'a [(&str, Value)], key: &str) -> &'a Value {
        &values.iter().find(|(k, _)| *k == key).unwrap().1
    }

    #[test]
    fn a_status_becomes_points() {
        let answer: Json = serde_json::from_slice(STATUS).unwrap();
        let status = status(&answer["result"]).unwrap();
        let values = values(status);
        let keys: Vec<&str> = values.iter().map(|(k, _)| *k).collect();
        let readable: Vec<String> = points()
            .into_iter()
            .filter(|p| p.access.read)
            .map(|p| p.key.to_string())
            .collect();
        assert_eq!(keys, readable, "every readable point, once");
        assert_eq!(value(&values, "state"), &Value::from("cleaning"));
        assert_eq!(value(&values, "error"), &Value::from("none"));
        assert_eq!(value(&values, "battery"), &Value::Int(83));
        assert_eq!(value(&values, "cleaning"), &Value::Bool(true));
        assert_eq!(value(&values, "fan_power"), &Value::Int(102));
        assert_eq!(value(&values, "clean_time"), &Value::Float(21.0));
        assert_eq!(value(&values, "clean_area"), &Value::Float(21.3));
        assert!(active(status));
    }

    #[test]
    fn unknown_or_absurd_numbers_are_not_guessed() {
        let values = values(&json!({
            "state": 77, "error_code": 99, "battery": 300, "fan_power": -1,
            "clean_time": "long", "clean_area": -5
        }));
        assert_eq!(value(&values, "state"), &Value::from("unknown"));
        assert_eq!(value(&values, "error"), &Value::from("unknown"));
        assert_eq!(value(&values, "cleaning"), &Value::Bool(false));
        for key in ["battery", "fan_power", "clean_time", "clean_area"] {
            assert_eq!(value(&values, key), &Value::Null, "{key}");
        }
        let empty = super::values(&json!({}));
        assert!(empty.iter().all(|(_, v)| *v == Value::Null));
        assert_eq!(status(&json!([])), None);
        assert_eq!(status(&json!(["ok"])), None);
    }

    #[test]
    fn only_roborock_vacuums_are_driven() {
        let info: Json = serde_json::from_slice(INFO).unwrap();
        assert_eq!(
            model(&info["result"]).as_deref(),
            Some("roborock.vacuum.s6")
        );
        assert_eq!(firmware(&info["result"]).as_deref(), Some("3.5.8_002034"));
        for bad in [
            "zhimi.airpurifier.m1",
            "roborock.vacuum.S6",
            "roborock.vacuum.s6 ",
            &format!("roborock.vacuum.{}", "a".repeat(60)),
        ] {
            assert_eq!(model(&json!({ "model": bad })), None, "{bad}");
        }
        assert_eq!(model(&json!({})), None);
    }

    #[test]
    fn orders_and_their_answers() {
        assert_eq!(order("control", &Value::from("start")), Ok("app_start"));
        assert_eq!(order("control", &Value::from("pause")), Ok("app_pause"));
        assert_eq!(order("control", &Value::from("stop")), Ok("app_stop"));
        assert_eq!(order("control", &Value::from("home")), Ok("app_charge"));
        assert!(order("control", &Value::from("app_start")).is_err());
        assert!(order("state", &Value::from("idle")).is_err());

        assert_eq!(outcome(&serde_json::from_slice(OK).unwrap()), Ok(()));
        let refused = outcome(&serde_json::from_slice(ERROR).unwrap()).unwrap_err();
        assert!(refused.contains("invalid params"), "{refused}");
        let hostile = outcome(&json!({"error": {"message": "<b>x</b>\u{1b}[31m"}})).unwrap_err();
        assert!(
            !hostile.contains('<') && !hostile.contains('\u{1b}'),
            "{hostile}"
        );
        assert!(outcome(&json!({"result": ["nope"]})).is_err());
    }
}
