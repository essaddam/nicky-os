//! What the bridge says, as Moli's points: pure, tested on the fixtures.
//!
//! Everything here comes from the network and is treated as hostile: the
//! list is bounded, names are trimmed and bounded, numbers are read only
//! from the known tables (an unknown state is `undefined`, never a guess).

use moli_core::{Access, Kind, PointSpec, Semantic, Unit, Value};
use ring::digest::{SHA256, digest};
use serde::Deserialize;

/// More locks than any house has behind one bridge: the rest is ignored.
pub const MAX_LOCKS: usize = 32;
const MAX_NAME: usize = 64;

/// The smart locks the bridge reports (`deviceType`, Bridge HTTP API
/// 1.13.2, § 4.1). The Opener (2) has other states and actions: not driven
/// here. An unknown type is left out, never guessed.
const LOCK_TYPES: [(u8, &str); 3] = [(0, "Smart Lock"), (3, "Smart Door"), (4, "Smart Lock 3.0")];

/// A smart lock's `state`, by number (Bridge HTTP API, « lock states »).
pub const STATES: [(u8, &str); 10] = [
    (0, "uncalibrated"),
    (1, "locked"),
    (2, "unlocking"),
    (3, "unlocked"),
    (4, "locking"),
    (5, "unlatched"),
    (6, "unlocked_lock_n_go"),
    (7, "unlatching"),
    (254, "motor_blocked"),
    (255, "undefined"),
];

/// `lockAction` numbers.
pub const UNLOCK: u8 = 1;
pub const LOCK: u8 = 2;
pub const UNLATCH: u8 = 3;

/// One entry of `/list`.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub nuki_id: u32,
    /// Absent before API 1.13 (Smart Lock 1.0/2.0 only): 0.
    #[serde(default)]
    pub device_type: u8,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub last_known_state: Option<Last>,
}

/// What the bridge last heard from the lock.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Last {
    #[serde(default)]
    pub state: Option<u8>,
    #[serde(default)]
    pub battery_critical: Option<bool>,
    #[serde(default)]
    pub battery_charging: Option<bool>,
    #[serde(default)]
    pub battery_charge_state: Option<u8>,
    #[serde(default)]
    pub keypad_battery_critical: Option<bool>,
    #[serde(default)]
    pub doorsensor_state: Option<u8>,
}

/// A lock Moli drives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lock {
    /// Stable: the lock's Nuki id, as the app shows it (hexadecimal).
    pub native: String,
    pub nuki_id: u32,
    pub device_type: u8,
    pub name: String,
    pub model: &'static str,
}

/// The locks of a `/list` answer, at most [`MAX_LOCKS`], with what each said.
pub fn locks(body: &[u8]) -> anyhow::Result<Vec<(Lock, Option<Last>)>> {
    let entries: Vec<serde_json::Value> = serde_json::from_slice(body)?;
    Ok(entries
        .into_iter()
        .filter_map(|e| serde_json::from_value::<Entry>(e).ok())
        .filter_map(|e| {
            let model = LOCK_TYPES
                .iter()
                .find(|(t, _)| *t == e.device_type)
                .map(|(_, m)| *m)?;
            let native = format!("{:X}", e.nuki_id);
            let name: String = e
                .name
                .chars()
                .filter(|c| !c.is_control())
                .take(MAX_NAME)
                .collect();
            let name = if name.trim().is_empty() {
                format!("Nuki {native}")
            } else {
                name.trim().to_owned()
            };
            Some((
                Lock {
                    native,
                    nuki_id: e.nuki_id,
                    device_type: e.device_type,
                    name,
                    model,
                },
                e.last_known_state,
            ))
        })
        .take(MAX_LOCKS)
        .collect())
}

fn spec(key: &str, label: String, kind: Kind, access: Access, semantic: Semantic) -> PointSpec {
    PointSpec {
        key: key.into(),
        label: label.into(),
        kind,
        access,
        unit: None,
        semantic,
    }
}

const READ: Access = Access {
    read: true,
    write: false,
};

/// A lock's points: the bolt, its state, the latch, and what the lock
/// reports of itself (battery, door sensor, keypad) when it has them.
#[must_use]
pub fn points(last: Option<&Last>) -> Vec<PointSpec> {
    let mut points = vec![
        spec(
            "locked",
            moli_i18n::tr!("pilotes.nuki.verrouillee"),
            Kind::Binary,
            Access {
                read: true,
                write: true,
            },
            Semantic::Lock,
        ),
        spec(
            "state",
            moli_i18n::tr!("pilotes.nuki.etat"),
            Kind::Enum {
                values: STATES.iter().map(|(_, s)| (*s).into()).collect(),
            },
            READ,
            Semantic::Other,
        ),
        // Write-only: every write pulls the latch (the door opens).
        spec(
            "unlatch",
            moli_i18n::tr!("pilotes.nuki.ouvrir"),
            Kind::Binary,
            Access {
                read: false,
                write: true,
            },
            Semantic::Lock,
        ),
        spec(
            "battery_low",
            moli_i18n::tr!("pilotes.nuki.batterie_faible"),
            Kind::Binary,
            READ,
            Semantic::BatteryLow,
        ),
    ];
    let Some(last) = last else { return points };
    if last.battery_charge_state.is_some() {
        points.push(PointSpec {
            unit: Some(Unit::Percent),
            ..spec(
                "battery",
                moli_i18n::tr!("pilotes.nuki.batterie"),
                Kind::Numeric {
                    min: Some(0.0),
                    max: Some(100.0),
                    step: Some(1.0),
                },
                READ,
                Semantic::Battery,
            )
        });
    }
    if last.battery_charging.is_some() {
        points.push(spec(
            "charging",
            moli_i18n::tr!("pilotes.nuki.en_charge"),
            Kind::Binary,
            READ,
            Semantic::Other,
        ));
    }
    // 1: no door sensor paired (deactivated).
    if last.doorsensor_state.is_some_and(|s| s != 1) {
        points.push(spec(
            "door",
            moli_i18n::tr!("pilotes.nuki.porte"),
            Kind::Binary,
            READ,
            Semantic::Contact,
        ));
    }
    if last.keypad_battery_critical.is_some() {
        points.push(spec(
            "keypad_battery_low",
            moli_i18n::tr!("pilotes.nuki.clavier_pile_faible"),
            Kind::Binary,
            READ,
            Semantic::BatteryLow,
        ));
    }
    points
}

/// The values of what the bridge last heard. `locked` is true only when
/// the lock says locked: moving or blocked, it is not.
#[must_use]
pub fn values(last: &Last) -> Vec<(&'static str, Value)> {
    let state = last
        .state
        .and_then(|n| STATES.iter().find(|(s, _)| *s == n))
        .map_or("undefined", |(_, name)| *name);
    let locked = match last.state {
        Some(1) => Value::Bool(true),
        Some(2..=7) => Value::Bool(false),
        _ => Value::Null,
    };
    let mut values = vec![
        ("state", Value::from(state)),
        ("locked", locked),
        (
            "battery_low",
            last.battery_critical.map_or(Value::Null, Value::Bool),
        ),
    ];
    if let Some(pct) = last.battery_charge_state {
        values.push(("battery", Value::Int(i64::from(pct.min(100)))));
    }
    if let Some(charging) = last.battery_charging {
        values.push(("charging", Value::Bool(charging)));
    }
    if let Some(door) = last.doorsensor_state.filter(|s| *s != 1) {
        // A contact is true when closed.
        let closed = match door {
            2 => Value::Bool(true),
            3 => Value::Bool(false),
            _ => Value::Null,
        };
        values.push(("door", closed));
    }
    if let Some(low) = last.keypad_battery_critical {
        values.push(("keypad_battery_low", Value::Bool(low)));
    }
    values
}

/// The `lockAction` an order asks for, or why it is refused.
pub fn action(key: &str, value: &Value) -> Result<u8, String> {
    match (key, value) {
        ("locked", Value::Bool(true)) => Ok(LOCK),
        ("locked", Value::Bool(false)) => Ok(UNLOCK),
        ("unlatch", Value::Bool(true)) => Ok(UNLATCH),
        ("unlatch", _) => Err(moli_i18n::tr!("pilotes.nuki.ouvrir_vrai")),
        _ => Err(moli_i18n::tr!("pilotes.nuki.lecture_seule", key = key)),
    }
}

/// The hashed-token authentication (the token itself never travels):
/// `ts` (UTC, to the second), `rnr` (random, never twice within a minute)
/// and `hash` = SHA-256 of `ts,rnr,token`. The bridge refuses a `ts` more
/// than a minute off its own clock.
#[must_use]
pub fn auth(token: &str, ts: &str, rnr: u16) -> String {
    let hash = digest(&SHA256, format!("{ts},{rnr},{token}").as_bytes());
    format!("ts={ts}&rnr={rnr}&hash={}", hex(hash.as_ref()))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// Whether a `lockAction` answer says it was done.
#[must_use]
pub fn succeeded(body: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(body)
        .is_ok_and(|j| j["success"] == serde_json::Value::Bool(true))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = include_str!("../../../integrations/nuki/fixtures/list.json");

    #[test]
    fn the_list_gives_the_locks_and_their_values() {
        let locks = locks(LIST.as_bytes()).unwrap();
        // The opener is not a lock: left out.
        assert_eq!(locks.len(), 2);
        let (lock, last) = &locks[0];
        assert_eq!(lock.native, "1A2B3C4D");
        assert_eq!(lock.model, "Smart Lock 3.0");
        assert_eq!(lock.name, "Porte de test");
        let last = last.as_ref().unwrap();
        let keys: Vec<String> = points(Some(last))
            .iter()
            .map(|p| p.key.to_string())
            .collect();
        assert_eq!(
            keys,
            [
                "locked",
                "state",
                "unlatch",
                "battery_low",
                "battery",
                "charging",
                "door",
                "keypad_battery_low"
            ]
        );
        let v = values(last);
        let get = |k: &str| v.iter().find(|(key, _)| *key == k).map(|(_, v)| v.clone());
        assert_eq!(get("locked"), Some(Value::Bool(true)));
        assert_eq!(get("state"), Some(Value::from("locked")));
        assert_eq!(get("battery"), Some(Value::Int(82)));
        assert_eq!(get("battery_low"), Some(Value::Bool(false)));
        assert_eq!(get("door"), Some(Value::Bool(true)));
        // The older lock: no battery level, no door sensor (deactivated).
        let (old, last) = &locks[1];
        assert_eq!(old.model, "Smart Lock");
        let keys: Vec<String> = points(last.as_ref())
            .iter()
            .map(|p| p.key.to_string())
            .collect();
        assert_eq!(keys, ["locked", "state", "unlatch", "battery_low"]);
        let v = values(last.as_ref().unwrap());
        assert!(v.contains(&("locked", Value::Bool(false))));
        assert!(v.contains(&("state", Value::from("unlatched"))));
    }

    #[test]
    fn hostile_answers_are_bounded_and_never_guessed() {
        let many: Vec<String> = (0..100)
            .map(|i| {
                format!(
                    r#"{{"nukiId":{i},"deviceType":4,"name":"{}"}}"#,
                    "x".repeat(500)
                )
            })
            .collect();
        let locks = locks(format!("[{}]", many.join(",")).as_bytes()).unwrap();
        assert_eq!(locks.len(), MAX_LOCKS);
        assert!(locks.iter().all(|(l, _)| l.name.chars().count() == 64));
        // Garbage entries are skipped, not fatal; a non-list is an error.
        assert_eq!(
            locks_of(r#"[{"nukiId":"x"},{"nukiId":1,"deviceType":0,"name":"\u0007"}]"#)[0].name,
            "Nuki 1"
        );
        assert!(super::locks(b"{}").is_err());
        // An unknown state is undefined, and the bolt unknown.
        let last = Last {
            state: Some(42),
            ..Last::default()
        };
        let v = values(&last);
        assert!(v.contains(&("state", Value::from("undefined"))));
        assert!(v.contains(&("locked", Value::Null)));
        let blocked = Last {
            state: Some(254),
            battery_charge_state: Some(250),
            ..Last::default()
        };
        assert!(values(&blocked).contains(&("locked", Value::Null)));
        assert!(values(&blocked).contains(&("battery", Value::Int(100))));
    }

    fn locks_of(json: &str) -> Vec<Lock> {
        super::locks(json.as_bytes())
            .unwrap()
            .into_iter()
            .map(|(l, _)| l)
            .collect()
    }

    #[test]
    fn orders_map_to_lock_actions() {
        assert_eq!(action("locked", &Value::Bool(true)), Ok(LOCK));
        assert_eq!(action("locked", &Value::Bool(false)), Ok(UNLOCK));
        assert_eq!(action("unlatch", &Value::Bool(true)), Ok(UNLATCH));
        assert!(action("unlatch", &Value::Bool(false)).is_err());
        assert!(action("battery", &Value::Int(1)).is_err());
        // The guard reads these points as a door's lock.
        let p = points(None);
        let locked = p.iter().find(|p| &*p.key == "locked").unwrap();
        let unlatch = p.iter().find(|p| &*p.key == "unlatch").unwrap();
        assert!(!locked.opens(Some(&Value::Bool(true))));
        assert!(locked.opens(Some(&Value::Bool(false))));
        assert!(unlatch.opens(Some(&Value::Bool(true))));
    }

    #[test]
    fn the_hashed_token_is_the_documented_one() {
        // Bridge HTTP API 1.13.2, § 3.2.1: ts=2019-03-05T01:06:53Z,
        // rnr=4711, token 123456.
        assert_eq!(
            auth("123456", "2019-03-05T01:06:53Z", 4711),
            "ts=2019-03-05T01:06:53Z&rnr=4711&hash=f52eb5ce382e356c4239f8fb4d0a87402bb95b7b3124f0762b806ad7d0d01cb6"
        );
        assert!(succeeded(br#"{"success": true, "batteryCritical": false}"#));
        assert!(!succeeded(br#"{"success": false}"#));
        assert!(!succeeded(b"<html>"));
    }
}
