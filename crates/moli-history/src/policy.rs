//! What the history keeps: which points it records, and for how long.
//!
//! A point's duration comes from its own rule (`[[storage.point]]`), else
//! its family (`[storage.policy.<family>]`), else the general setting
//! (`[server] history_days`). The family comes from what the point means and
//! its unit, never from its type alone. Without any `[storage]` section,
//! every point is recorded and kept `history_days` days, as before.

use std::collections::BTreeMap;

use moli_core::{Kind, PointSpec, Semantic, Unit};
use serde::{Deserialize, Serialize};

/// What a point measures, for its retention.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    /// Power, current, mains voltage.
    Power,
    /// Temperature, humidity, pressure, light, air.
    Climate,
    /// Meters (kWh, Wh): their hours also live in the energy database.
    Energy,
    /// On / off, contacts, presence, buttons, texts.
    States,
    /// Signal, link quality, batteries.
    Diagnostic,
    Other,
}

impl Family {
    pub const ALL: [Self; 6] = [
        Self::Power,
        Self::Climate,
        Self::Energy,
        Self::States,
        Self::Diagnostic,
        Self::Other,
    ];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Power => "power",
            Self::Climate => "climate",
            Self::Energy => "energy",
            Self::States => "states",
            Self::Diagnostic => "diagnostic",
            Self::Other => "other",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.name() == name)
    }

    #[must_use]
    pub fn of(spec: &PointSpec) -> Self {
        family(spec.semantic, spec.unit.as_ref(), &spec.kind, &spec.key)
    }
}

/// A point's family: its meaning first, then its unit; the type only
/// separates states from numbers when neither says anything. `key` only
/// tells a battery's voltage from a measurement in the same unit (a pool's
/// ORP is in millivolts too).
#[must_use]
pub fn family(semantic: Semantic, unit: Option<&Unit>, kind: &Kind, key: &str) -> Family {
    use Semantic as S;
    let millivolts = unit == Some(&Unit::MilliVolt);
    match semantic {
        S::Voltage | S::Other if battery_voltage(key, millivolts) => Family::Diagnostic,
        S::Voltage if millivolts => Family::Other,
        S::Power | S::ApparentPower | S::Current | S::Voltage => Family::Power,
        S::Energy => Family::Energy,
        S::Temperature | S::Humidity | S::Pressure | S::Illuminance => Family::Climate,
        S::Battery | S::BatteryLow | S::Diagnostic | S::SignalStrength => Family::Diagnostic,
        S::OnOff
        | S::Brightness
        | S::ColorTemp
        | S::Color
        | S::Contact
        | S::Occupancy
        | S::WaterLeak
        | S::Smoke
        | S::Tamper
        | S::Config
        | S::Lock
        | S::Control => Family::States,
        S::Other => match unit {
            Some(Unit::Watt | Unit::VoltAmpere | Unit::Ampere | Unit::Volt) => Family::Power,
            Some(Unit::KiloWattHour | Unit::WattHour) => Family::Energy,
            Some(Unit::Celsius | Unit::HectoPascal | Unit::Lux | Unit::Ppm) => Family::Climate,
            Some(_) => Family::Other,
            None => match kind {
                Kind::Numeric { .. }
                | Kind::Button
                | Kind::Counter { .. }
                | Kind::CounterCommand { .. }
                | Kind::Timer { .. }
                | Kind::TimerCommand { .. } => Family::Other,
                Kind::Binary | Kind::Enum { .. } | Kind::Text => Family::States,
            },
        },
    }
}

/// A battery's voltage: named so (`battery_voltage`, `vbat`…), or the
/// plain `voltage` of a battery device, told in millivolts (Zigbee2MQTT).
fn battery_voltage(key: &str, millivolts: bool) -> bool {
    let name = key.rsplit('.').next().unwrap_or(key).to_ascii_lowercase();
    name.contains("batt") || name.contains("vbat") || (millivolts && name == "voltage")
}

/// `[storage]` in `moli.toml`.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageConfig {
    #[serde(default)]
    pub history: HistoryRules,
    /// `[storage.policy.<family>]`
    #[serde(default)]
    pub policy: BTreeMap<String, Keep>,
    /// `[[storage.point]]`, the first matching one wins.
    #[serde(default, rename = "point")]
    pub points: Vec<PointKeep>,
}

/// `[storage.history]`: which points are written at all.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryRules {
    /// Points never written (patterns, `*` for anything).
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Written even when `exclude` names them.
    #[serde(default)]
    pub include: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Keep {
    pub days: u32,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PointKeep {
    /// A point id (`z2m:salon/temperature`) or a pattern (`z2m:*/linkquality`).
    #[serde(rename = "match")]
    pub pattern: String,
    pub days: u32,
}

/// Longest duration a rule may ask for (a century).
pub(crate) const MAX_DAYS: u32 = 36_500;

impl StorageConfig {
    /// The filter and the durations, `general_days` being `[server]
    /// history_days`. Errors are in the house's language.
    pub fn compile(&self, general_days: u32) -> Result<(Filter, Retention), String> {
        let days = |at: &str, days: u32| {
            if (1..=MAX_DAYS).contains(&days) {
                Ok(days)
            } else {
                Err(moli_i18n::tr!(
                    "serveur.config.stockage_jours",
                    ou = at,
                    max = MAX_DAYS
                ))
            }
        };
        let pattern = |at: &str, p: &str| {
            if p.trim().is_empty() {
                Err(moli_i18n::tr!(
                    "serveur.config.stockage_motif_vide",
                    ou = at
                ))
            } else {
                Ok(p.trim().to_owned())
            }
        };
        let mut families = BTreeMap::new();
        for (name, keep) in &self.policy {
            let at = format!("[storage.policy.{name}]");
            let family = Family::parse(name).ok_or_else(|| {
                let known: Vec<_> = Family::ALL.iter().map(|f| f.name()).collect();
                moli_i18n::tr!(
                    "serveur.config.stockage_famille_inconnue",
                    ou = at,
                    familles = known.join(", ")
                )
            })?;
            families.insert(family, days(&at, keep.days)?);
        }
        let points = self
            .points
            .iter()
            .map(|p| {
                let at = format!("[[storage.point]] match = {:?}", p.pattern);
                Ok((pattern(&at, &p.pattern)?, days(&at, p.days)?))
            })
            .collect::<Result<_, String>>()?;
        let list = |name: &str, list: &[String]| {
            let at = format!("[storage.history] {name}");
            list.iter()
                .map(|p| pattern(&at, p))
                .collect::<Result<Vec<_>, _>>()
        };
        let filter = Filter {
            exclude: list("exclude", &self.history.exclude)?,
            include: list("include", &self.history.include)?,
        };
        let retention = Retention {
            days: general_days.max(1),
            families,
            points,
        };
        Ok((filter, retention))
    }
}

/// Which points are written. Excluding stops new writes only: what is
/// already recorded stays, and goes with its retention.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Filter {
    pub exclude: Vec<String>,
    pub include: Vec<String>,
}

impl Filter {
    #[must_use]
    pub fn records(&self, point: &str) -> bool {
        !self.exclude.iter().any(|p| glob(p, point)) || self.include.iter().any(|p| glob(p, point))
    }
}

/// How long each point is kept, in days.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Retention {
    /// The general setting.
    pub days: u32,
    #[serde(default)]
    pub families: BTreeMap<Family, u32>,
    /// (pattern, days), the first matching one wins.
    #[serde(default)]
    pub points: Vec<(String, u32)>,
}

impl Retention {
    /// Everything kept `days` days: Moli before any `[storage]` section.
    #[must_use]
    pub fn uniform(days: u32) -> Self {
        Self {
            days: days.max(1),
            families: BTreeMap::new(),
            points: Vec::new(),
        }
    }

    /// The point's rule, else its family's, else the general setting.
    #[must_use]
    pub fn days(&self, point: &str, family: Family) -> u32 {
        self.points
            .iter()
            .find(|(p, _)| glob(p, point))
            .map_or_else(
                || self.families.get(&family).copied().unwrap_or(self.days),
                |(_, d)| *d,
            )
    }
}

/// `*` matches any run of characters, everything else itself.
#[must_use]
pub fn glob(pattern: &str, text: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or_default();
    let Some(mut rest) = text.strip_prefix(first) else {
        return false;
    };
    let parts: Vec<&str> = parts.collect();
    let Some((last, middle)) = parts.split_last() else {
        return rest.is_empty();
    };
    for part in middle {
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    rest.len() >= last.len() && rest.ends_with(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NUMBER: Kind = Kind::Numeric {
        min: None,
        max: None,
        step: None,
    };

    #[test]
    fn families_come_from_meaning_and_unit() {
        let f = |s, u: Option<Unit>, k: &Kind| family(s, u.as_ref(), k, "value");
        assert_eq!(f(Semantic::Power, Some(Unit::Watt), &NUMBER), Family::Power);
        assert_eq!(f(Semantic::Current, None, &NUMBER), Family::Power);
        assert_eq!(
            f(Semantic::Voltage, Some(Unit::Volt), &NUMBER),
            Family::Power
        );
        // Millivolts alone say nothing: a battery's voltage is named so…
        let named = |s, key| family(s, Some(&Unit::MilliVolt), &NUMBER, key);
        let z2m_battery = Semantic::infer("voltage", Some(&Unit::MilliVolt));
        assert_eq!(named(z2m_battery, "voltage"), Family::Diagnostic);
        assert_eq!(
            named(Semantic::Other, "battery_voltage"),
            Family::Diagnostic
        );
        assert_eq!(
            family(
                Semantic::Voltage,
                Some(&Unit::Volt),
                &NUMBER,
                "battery_voltage"
            ),
            Family::Diagnostic
        );
        // …a pool's ORP, in millivolts too, is a measurement.
        let orp = Semantic::infer("orp", Some(&Unit::MilliVolt));
        assert_eq!(named(orp, "orp"), Family::Other);
        assert_eq!(named(Semantic::Other, "orp"), Family::Other);
        // The mains' voltage, in volts, is power.
        assert_eq!(
            family(Semantic::Voltage, Some(&Unit::Volt), &NUMBER, "voltage"),
            Family::Power
        );
        assert_eq!(
            f(Semantic::Energy, Some(Unit::KiloWattHour), &NUMBER),
            Family::Energy
        );
        assert_eq!(
            f(Semantic::Temperature, Some(Unit::Celsius), &NUMBER),
            Family::Climate
        );
        assert_eq!(
            f(Semantic::Humidity, Some(Unit::Percent), &NUMBER),
            Family::Climate
        );
        assert_eq!(
            f(Semantic::SignalStrength, None, &NUMBER),
            Family::Diagnostic
        );
        assert_eq!(
            f(Semantic::Battery, Some(Unit::Percent), &NUMBER),
            Family::Diagnostic
        );
        assert_eq!(f(Semantic::Contact, None, &Kind::Binary), Family::States);
        assert_eq!(f(Semantic::Occupancy, None, &Kind::Binary), Family::States);
        // A driver that says nothing of the meaning: the unit decides…
        assert_eq!(
            f(Semantic::Other, Some(Unit::WattHour), &NUMBER),
            Family::Energy
        );
        assert_eq!(
            f(Semantic::Other, Some(Unit::Ppm), &NUMBER),
            Family::Climate
        );
        // …a percentage alone could be anything…
        assert_eq!(
            f(Semantic::Other, Some(Unit::Percent), &NUMBER),
            Family::Other
        );
        // …and without unit, a button or a text is an event, a number is not.
        assert_eq!(f(Semantic::Other, None, &Kind::Text), Family::States);
        let action = Kind::Enum {
            values: vec!["single".into()],
        };
        assert_eq!(f(Semantic::Other, None, &action), Family::States);
        assert_eq!(f(Semantic::Other, None, &NUMBER), Family::Other);
        // The number type alone never makes a family: a numeric contact is a state.
        assert_eq!(f(Semantic::Contact, None, &NUMBER), Family::States);
    }

    #[test]
    fn patterns() {
        assert!(glob("z2m:salon/temperature", "z2m:salon/temperature"));
        assert!(!glob("z2m:salon/temperature", "z2m:salon/temperature_2"));
        assert!(glob("z2m:*/linkquality", "z2m:salon/linkquality"));
        assert!(!glob("z2m:*/linkquality", "hue:salon/linkquality"));
        assert!(glob("*", "anything"));
        assert!(glob("*rssi", "tapo:plug/rssi"));
        assert!(glob("tapo:*", "tapo:plug/rssi"));
        assert!(glob("a*b*c", "aXbYc"));
        assert!(!glob("a*b*c", "aXcYb"));
        assert!(!glob("ab*ba", "aba"), "the ends do not overlap");
    }

    fn config(text: &str) -> StorageConfig {
        toml::from_str(text).unwrap()
    }

    #[test]
    fn without_configuration_everything_is_recorded_and_kept_as_before() {
        let (filter, retention) = StorageConfig::default().compile(30).unwrap();
        assert_eq!(retention, Retention::uniform(30));
        for family in Family::ALL {
            assert!(filter.records("z2m:any/point"));
            assert_eq!(retention.days("z2m:any/point", family), 30);
        }
    }

    #[test]
    fn a_locks_state_and_opener_keep_the_states_duration_not_its_battery_duration() {
        let (_, retention) = config(
            r"
            [policy.states]
            days = 31
            [policy.diagnostic]
            days = 3
            ",
        )
        .compile(7)
        .unwrap();
        for key in ["locked", "unlatch"] {
            let family = family(Semantic::Lock, None, &Kind::Binary, key);
            assert_eq!(retention.days(&format!("lock:door/{key}"), family), 31);
        }
        let battery = family(Semantic::Battery, Some(&Unit::Percent), &NUMBER, "battery");
        assert_eq!(retention.days("lock:door/battery", battery), 3);
    }

    #[test]
    fn a_point_wins_over_its_family_which_wins_over_the_general_setting() {
        let (filter, retention) = config(
            r#"
            [history]
            exclude = ["z2m:*/linkquality", "tapo:*"]
            include = ["tapo:plug/power"]
            [policy.climate]
            days = 90
            [policy.diagnostic]
            days = 3
            [[point]]
            match = "z2m:cave/temperature"
            days = 365
            "#,
        )
        .compile(30)
        .unwrap();
        assert_eq!(retention.days("z2m:cave/temperature", Family::Climate), 365);
        assert_eq!(retention.days("z2m:salon/temperature", Family::Climate), 90);
        assert_eq!(retention.days("z2m:salon/battery", Family::Diagnostic), 3);
        assert_eq!(retention.days("z2m:salon/state", Family::States), 30);
        assert!(!filter.records("z2m:salon/linkquality"));
        assert!(!filter.records("tapo:plug/rssi"));
        assert!(filter.records("tapo:plug/power"), "include wins");
        assert!(filter.records("z2m:salon/temperature"));
    }

    #[test]
    fn bad_rules_are_refused_with_a_reason() {
        let err = |text: &str| config(text).compile(30).unwrap_err();
        assert!(err("[policy.climat]\ndays = 3").contains("climate"));
        assert!(err("[policy.climate]\ndays = 0").contains("[storage.policy.climate]"));
        assert!(err("[[point]]\nmatch = \" \"\ndays = 3").contains("[[storage.point]]"));
        assert!(err("[history]\nexclude = [\"\"]").contains("exclude"));
        assert!(toml::from_str::<StorageConfig>("[policy.climate]\nhot_days = 3").is_err());
    }

    #[test]
    fn n0_diagnostics_win_over_units_without_changing_policy_precedence() {
        let diagnostic: Semantic = serde_json::from_str("\"diagnostic\"").unwrap();
        let (filter, retention) = config(
            r#"
            [history]
            exclude = ["net:*/traffic"]
            include = ["net:unit/traffic"]
            [policy.diagnostic]
            days = 7
            [policy.climate]
            days = 90
            [[point]]
            match = "net:special/traffic"
            days = 60
            "#,
        )
        .compile(30)
        .unwrap();
        let units = [
            None,
            Unit::parse("B"),
            Unit::parse("B/s"),
            Unit::parse("Mbit/s"),
            Some(Unit::Percent),
            Some(Unit::Second),
            Some(Unit::Celsius),
        ];
        for unit in units {
            for kind in [
                NUMBER,
                Kind::Text,
                Kind::Binary,
                Kind::Enum {
                    values: vec!["ok".into()],
                },
            ] {
                let spec = PointSpec {
                    key: "traffic".into(),
                    label: "Traffic".into(),
                    kind,
                    access: moli_core::Access {
                        read: true,
                        write: false,
                    },
                    unit: unit.clone(),
                    semantic: diagnostic,
                };
                let classified = Family::of(&spec);
                assert_eq!(classified, Family::Diagnostic);
                assert_eq!(retention.days("net:unit/traffic", classified), 7);
                assert_eq!(retention.days("net:special/traffic", classified), 60);
            }
        }
        assert!(filter.records("net:unit/traffic"));
        assert!(!filter.records("net:other/traffic"));
        let ambient = family(
            Semantic::Temperature,
            Some(&Unit::Celsius),
            &NUMBER,
            "temperature",
        );
        assert_eq!(ambient, Family::Climate);
        assert_eq!(retention.days("net:unit/temperature", ambient), 90);
        let ordinary = family(
            Semantic::Other,
            Unit::parse("B").as_ref(),
            &NUMBER,
            "rx_bytes",
        );
        assert_eq!(ordinary, Family::Other);
        assert_eq!(retention.days("net:unit/rx_bytes", ordinary), 30);
        assert_eq!(
            family(
                Semantic::Voltage,
                Some(&Unit::MilliVolt),
                &NUMBER,
                "voltage"
            ),
            Family::Diagnostic
        );
        assert_eq!(
            family(
                Semantic::Other,
                Some(&Unit::MilliVolt),
                &NUMBER,
                "battery_voltage"
            ),
            Family::Diagnostic
        );
    }
}
