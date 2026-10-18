//! What the Voltalis cloud says, as Moli's points, and the bodies of the
//! orders: pure, tested on the fixtures.
//!
//! Everything here comes from the network and is treated as hostile: lists
//! are bounded, names trimmed and bounded, identifiers must be plain
//! positive numbers (they end up in a path), and an unknown value is shown
//! as it is, never mapped to a guess.

use std::collections::HashMap;

use moli_core::{Access, Device, DeviceId, InstanceId, Kind, PointSpec, Semantic, Unit, Value};
use serde_json::{Value as Json, json};

/// More appliances, or programs, than a house has: the rest is ignored.
pub const MAX_APPLIANCES: usize = 32;
pub const MAX_PROGRAMS: usize = 32;
const MAX_NAME: usize = 64;

/// A heater on the pilot wire (`applianceType`): driven.
const HEATER: &str = "HEATER";
/// A water heater behind a Voltalis relay: read only. The client every
/// reference home ran does not implement a single order for it (its
/// « off » did nothing): nothing documents what the cloud would do.
const WATER_HEATER: &str = "WATER_HEATER";

/// The heater's regimes, as Moli names them: back to its planning
/// (`programme`), held by hand (`manuel`), held off (`arret`).
pub const MODES: [&str; 3] = ["programme", "manuel", "arret"];
const PROGRAMME: &str = "programme";
const MANUEL: &str = "manuel";
const ARRET: &str = "arret";

/// The presets (`programming.mode`), Voltalis name → Moli value.
pub const PRESETS: [(&str, &str); 5] = [
    ("ECO", "eco"),
    ("CONFORT", "confort"),
    ("TEMPERATURE", "temperature"),
    ("HORS_GEL", "hors_gel"),
    ("NORMAL", "normal"),
];

/// A setpoint the cloud accepts (the client every reference home ran).
pub const MIN_SETPOINT: f64 = 7.0;
pub const MAX_SETPOINT: f64 = 24.0;

/// A plain positive number from the cloud, safe in a path.
fn id(value: &Json) -> Option<u64> {
    value.as_u64().filter(|id| *id > 0 && *id < 1 << 53)
}

fn name(value: &Json, fallback: &str) -> String {
    let name: String = value
        .as_str()
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME)
        .collect();
    let name = name.trim();
    if name.is_empty() {
        fallback.to_owned()
    } else {
        name.to_owned()
    }
}

/// The account's site (`/api/account/me`): every other path names it.
pub fn site(me: &Json) -> anyhow::Result<u64> {
    id(&me["defaultSite"]["id"]).ok_or_else(|| anyhow::anyhow!("voltalis: no default site"))
}

/// What an appliance is to Moli.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Heater,
    WaterHeater,
}

/// One managed appliance, as last read.
#[derive(Clone, Debug, PartialEq)]
pub struct Appliance {
    pub id: u64,
    pub name: String,
    pub role: Role,
    pub modulator: String,
    /// The fields an order sends back as they were (the cloud takes the
    /// whole manual setting at once).
    heating_level: Json,
    kind_name: String,
    pub programming: Programming,
}

/// Its current programming.
#[derive(Clone, Debug, PartialEq)]
pub struct Programming {
    /// `MANUAL`, `USER` (a planning), or another program kind.
    pub prog_type: String,
    pub prog_name: String,
    pub is_on: bool,
    pub until_further_notice: bool,
    /// The Voltalis preset (`ECO`, `CONFORT`…).
    pub mode: String,
    pub target: Option<f64>,
}

/// The appliances of a `managed-appliance` answer Moli knows, at most
/// [`MAX_APPLIANCES`]; anything else (another type, no id) is left out.
#[must_use]
pub fn appliances(answer: &Json) -> Vec<Appliance> {
    let Some(list) = answer.as_array() else {
        return Vec::new();
    };
    list.iter()
        .filter_map(appliance)
        .take(MAX_APPLIANCES)
        .collect()
}

fn appliance(json: &Json) -> Option<Appliance> {
    let id = id(&json["id"])?;
    let kind = json["applianceType"].as_str()?;
    let role = match kind {
        HEATER => Role::Heater,
        WATER_HEATER => Role::WaterHeater,
        _ => return None,
    };
    let p = &json["programming"];
    let text = |v: &Json| -> String {
        v.as_str()
            .unwrap_or_default()
            .chars()
            .filter(|c| !c.is_control())
            .take(MAX_NAME)
            .collect()
    };
    Some(Appliance {
        id,
        name: name(&json["name"], &format!("Voltalis {id}")),
        role,
        modulator: text(&json["modulatorType"]),
        heating_level: json["heatingLevel"].clone(),
        kind_name: kind.to_owned(),
        programming: Programming {
            prog_type: text(&p["progType"]),
            prog_name: text(&p["progName"]),
            is_on: p["isOn"].as_bool().unwrap_or(false),
            until_further_notice: p["untilFurtherNotice"].as_bool().unwrap_or(false),
            mode: text(&p["mode"]),
            target: p["temperatureTarget"].as_f64().filter(|t| t.is_finite()),
        },
    })
}

/// Appliance id → its manual setting's id (`manualsetting`): what an order
/// writes to.
#[must_use]
pub fn manual_settings(answer: &Json) -> HashMap<u64, u64> {
    answer
        .as_array()
        .into_iter()
        .flatten()
        .take(MAX_APPLIANCES)
        .filter_map(|m| Some((id(&m["idAppliance"])?, id(&m["id"])?)))
        .collect()
}

/// Appliance id → reachable (`autodiag`: `OK` or `NOK`).
#[must_use]
pub fn diagnostics(answer: &Json) -> HashMap<u64, bool> {
    answer
        .as_array()
        .into_iter()
        .flatten()
        .take(MAX_APPLIANCES)
        .filter_map(|d| Some((id(&d["csApplianceId"])?, d["status"].as_str()? == "OK")))
        .collect()
}

/// A program of the account: one the household made (`USER`, a planning),
/// or a ready-made one (`quicksettings`). Only one runs at a time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pub id: u64,
    pub name: String,
    pub user: bool,
    pub enabled: bool,
}

impl Program {
    /// Stable: the two kinds are two lists, maybe with the same numbers.
    #[must_use]
    pub fn native(&self) -> String {
        if self.user {
            format!("program-{}", self.id)
        } else {
            format!("quick-{}", self.id)
        }
    }
}

/// The programs of a `programming/program` (`user`) or `quicksettings`
/// answer, at most [`MAX_PROGRAMS`].
#[must_use]
pub fn programs(answer: &Json, user: bool) -> Vec<Program> {
    answer
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| {
            let id = id(&p["id"])?;
            Some(Program {
                id,
                name: name(&p["name"], &format!("Programme {id}")),
                user,
                enabled: p["enabled"].as_bool()?,
            })
        })
        .take(MAX_PROGRAMS)
        .collect()
}

impl Appliance {
    /// Stable: the appliance's Voltalis number.
    #[must_use]
    pub fn native(&self) -> String {
        format!("appliance-{}", self.id)
    }

    /// The device Moli shows.
    #[must_use]
    pub fn device(&self, instance: &InstanceId) -> Device {
        let points = match self.role {
            Role::Heater => vec![
                point(
                    "mode",
                    moli_i18n::tr!("pilotes.voltalis.mode"),
                    Kind::Enum {
                        values: MODES.iter().map(|m| (*m).into()).collect(),
                    },
                    true,
                    None,
                    // Off for the winter, or a whole house back to its
                    // planning: a person's call when an agent asks.
                    Semantic::Control,
                ),
                point(
                    "preset",
                    moli_i18n::tr!("pilotes.voltalis.preset"),
                    Kind::Enum {
                        values: PRESETS.iter().map(|(_, m)| (*m).into()).collect(),
                    },
                    true,
                    None,
                    Semantic::Config,
                ),
                setpoint(true),
                heating(),
                program(),
            ],
            Role::WaterHeater => vec![setpoint(false), heating(), program()],
        };
        Device {
            id: DeviceId::new(instance, &self.native()),
            instance: instance.clone(),
            native_name: self.name.as_str().into(),
            manufacturer: Some("Voltalis".into()),
            model: Some(
                format!("{} {}", self.modulator, self.kind_name)
                    .trim()
                    .into(),
            ),
            description: None,
            native_room: None,
            members: Vec::new(),
            points,
        }
    }

    /// The values of its points.
    #[must_use]
    pub fn values(&self) -> Vec<(&'static str, Value)> {
        let p = &self.programming;
        let mut values = vec![
            ("heating", Value::Bool(p.is_on)),
            ("program", Value::from(p.prog_name.as_str())),
            ("setpoint", p.target.map_or(Value::Null, Value::Float)),
        ];
        if self.role == Role::Heater {
            values.push(("mode", Value::from(self.mode())));
            values.push(("preset", Value::from(preset(&p.mode).as_str())));
        }
        values
    }

    /// `programme` unless held by hand: an unknown program kind (a
    /// ready-made program running) is still not a hand on the heater.
    fn mode(&self) -> &'static str {
        match (self.programming.prog_type.as_str(), self.programming.is_on) {
            ("MANUAL", true) => MANUEL,
            ("MANUAL", false) => ARRET,
            _ => PROGRAMME,
        }
    }

    /// The manual setting that makes `key` read `value`, whole (the cloud
    /// takes it at once), from the appliance as last read; `None` for an
    /// order this appliance does not take.
    #[must_use]
    pub fn order(&self, manual: u64, key: &str, value: &Value) -> Option<Json> {
        if self.role != Role::Heater {
            return None;
        }
        let p = &self.programming;
        let mut body = json!({
            "id": manual,
            "enabled": true,
            "idAppliance": self.id,
            "applianceName": self.name,
            "applianceType": self.kind_name,
            "untilFurtherNotice": true,
            "mode": p.mode,
            "heatingLevel": self.heating_level,
            "endDate": Json::Null,
            "temperatureTarget": p.target,
            "isOn": true,
        });
        match (key, value) {
            ("mode", Value::Text(m)) if &**m == PROGRAMME => {
                // Back to its planning: the manual setting stops applying.
                body["enabled"] = false.into();
                body["untilFurtherNotice"] = p.until_further_notice.into();
            }
            ("mode", Value::Text(m)) if &**m == MANUEL => {
                body["mode"] = "TEMPERATURE".into();
            }
            ("mode", Value::Text(m)) if &**m == ARRET => body["isOn"] = false.into(),
            ("preset", Value::Text(m)) => {
                let (voltalis, _) = PRESETS.iter().find(|(_, moli)| *moli == &**m)?;
                body["mode"] = (*voltalis).into();
            }
            ("setpoint", v) => {
                let t = v
                    .as_f64()
                    .filter(|t| (MIN_SETPOINT..=MAX_SETPOINT).contains(t))?;
                body["mode"] = "TEMPERATURE".into();
                body["temperatureTarget"] = t.into();
            }
            _ => return None,
        }
        Some(body)
    }
}

/// A Voltalis preset as Moli names it; an unknown one as it came, in lower
/// case (shown, never guessed).
fn preset(mode: &str) -> String {
    PRESETS
        .iter()
        .find(|(voltalis, _)| *voltalis == mode)
        .map_or_else(|| mode.to_lowercase(), |(_, moli)| (*moli).to_owned())
}

/// The program's device: switched on, it is the one the house runs.
#[must_use]
pub fn program_device(instance: &InstanceId, program: &Program) -> Device {
    Device {
        id: DeviceId::new(instance, &program.native()),
        instance: instance.clone(),
        native_name: program.name.as_str().into(),
        manufacturer: Some("Voltalis".into()),
        model: Some(if program.user {
            moli_i18n::tr!("pilotes.voltalis.programme_maison").into()
        } else {
            moli_i18n::tr!("pilotes.voltalis.programme_pret").into()
        }),
        description: None,
        native_room: None,
        members: Vec::new(),
        // Every heater of the house at once: a person's call when an agent
        // asks.
        points: vec![point(
            "enabled",
            moli_i18n::tr!("pilotes.voltalis.actif"),
            Kind::Binary,
            true,
            None,
            Semantic::Control,
        )],
    }
}

/// The body that switches a program on or off.
#[must_use]
pub fn program_order(program: &Program, enabled: bool) -> Json {
    if program.user {
        json!({ "name": program.name, "enabled": enabled })
    } else {
        json!({ "enabled": enabled })
    }
}

fn point(
    key: &str,
    label: String,
    kind: Kind,
    write: bool,
    unit: Option<Unit>,
    semantic: Semantic,
) -> PointSpec {
    PointSpec {
        key: key.into(),
        label: label.into(),
        kind,
        access: Access { read: true, write },
        unit,
        semantic,
    }
}

fn setpoint(write: bool) -> PointSpec {
    point(
        "setpoint",
        moli_i18n::tr!("pilotes.voltalis.consigne"),
        Kind::Numeric {
            // Bounds for the orders; a water heater's own reads are not bounded.
            min: write.then_some(MIN_SETPOINT),
            max: write.then_some(MAX_SETPOINT),
            step: write.then_some(0.5),
        },
        write,
        Some(Unit::Celsius),
        Semantic::Config,
    )
}

fn heating() -> PointSpec {
    point(
        "heating",
        moli_i18n::tr!("pilotes.voltalis.chauffe"),
        Kind::Binary,
        false,
        None,
        Semantic::Other,
    )
}

fn program() -> PointSpec {
    point(
        "program",
        moli_i18n::tr!("pilotes.voltalis.programme"),
        Kind::Text,
        false,
        None,
        Semantic::Other,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Json {
        let path = format!(
            "{}/../../integrations/voltalis/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn the_fixtures_read() {
        assert_eq!(site(&fixture("account-me.json")).unwrap(), 100_001);
        let list = appliances(&fixture("managed-appliance.json"));
        assert_eq!(list.len(), 3, "the unknown type is left out");
        let heater = &list[0];
        assert_eq!(heater.role, Role::Heater);
        assert_eq!(heater.native(), "appliance-200001");
        let values = heater.values();
        let get = |k: &str| values.iter().find(|(key, _)| *key == k).unwrap().1.clone();
        assert_eq!(get("mode"), Value::from("programme"));
        assert_eq!(get("preset"), Value::from("eco"));
        assert_eq!(get("setpoint"), Value::Float(19.0));
        let off = list[1].values();
        assert!(off.contains(&("mode", Value::from("arret"))));
        let water = &list[2];
        assert_eq!(water.role, Role::WaterHeater);
        let device = water.device(&InstanceId::from("chauffage"));
        assert!(
            device.points.iter().all(|p| !p.access.write),
            "nothing documents an order to a water heater"
        );
        assert_eq!(
            manual_settings(&fixture("manualsetting.json")).get(&200_001),
            Some(&300_001)
        );
        assert_eq!(
            diagnostics(&fixture("autodiag.json")).get(&200_002),
            Some(&false)
        );
        let user = programs(&fixture("programs.json"), true);
        let quick = programs(&fixture("quicksettings.json"), false);
        assert_eq!(user.len(), 2);
        assert_eq!(quick[0].native(), "quick-1");
        assert!(user[0].enabled && quick.iter().all(|p| !p.enabled));
    }

    #[test]
    fn orders_are_whole_manual_settings() {
        let heater = appliances(&fixture("managed-appliance.json")).remove(0);
        let body = heater
            .order(300_001, "setpoint", &Value::Float(20.5))
            .unwrap();
        assert_eq!(body["id"], 300_001);
        assert_eq!(body["idAppliance"], 200_001);
        assert_eq!(body["mode"], "TEMPERATURE");
        assert_eq!(body["temperatureTarget"], 20.5);
        assert_eq!(body["isOn"], true);
        assert_eq!(body["untilFurtherNotice"], true);
        assert_eq!(body["heatingLevel"], heater.heating_level);
        let off = heater
            .order(300_001, "mode", &Value::from("arret"))
            .unwrap();
        assert_eq!(
            (off["isOn"].clone(), off["enabled"].clone()),
            (false.into(), true.into())
        );
        // Turned back on by hand, it heats (the client the reference homes
        // ran kept it off).
        let on = heater
            .order(300_001, "mode", &Value::from("manuel"))
            .unwrap();
        assert_eq!(on["isOn"], true);
        let auto = heater
            .order(300_001, "mode", &Value::from("programme"))
            .unwrap();
        assert_eq!(auto["enabled"], false);
        let eco = heater
            .order(300_001, "preset", &Value::from("hors_gel"))
            .unwrap();
        assert_eq!(eco["mode"], "HORS_GEL");
        assert!(
            heater
                .order(300_001, "setpoint", &Value::Float(30.0))
                .is_none()
        );
        assert!(
            heater
                .order(300_001, "preset", &Value::from("turbo"))
                .is_none()
        );
        assert!(
            heater
                .order(300_001, "heating", &Value::Bool(true))
                .is_none()
        );
    }

    #[test]
    fn hostile_answers() {
        let many: Vec<Json> = (1..=100)
            .map(|i| json!({ "id": i, "name": "x".repeat(500), "applianceType": "HEATER", "programming": {} }))
            .collect();
        let list = appliances(&Json::from(many));
        assert_eq!(list.len(), MAX_APPLIANCES);
        assert_eq!(list[0].name.chars().count(), MAX_NAME);
        for bad in [
            json!(-1),
            json!(0),
            json!("12"),
            json!(1.5),
            json!(1u64 << 60),
        ] {
            assert!(appliances(&json!([{ "id": bad, "applianceType": "HEATER" }])).is_empty());
        }
        assert!(site(&json!({ "defaultSite": { "id": "../x" } })).is_err());
        let unknown = appliances(
            &json!([{ "id": 7, "applianceType": "HEATER", "programming": { "mode": "TURBO", "progType": "QUICK" } }]),
        );
        let values = unknown[0].values();
        assert!(
            values.contains(&("preset", Value::from("turbo"))),
            "shown, not guessed"
        );
        assert!(values.contains(&("mode", Value::from("programme"))));
        assert!(appliances(&json!({ "not": "a list" })).is_empty());
    }
}
