//! What a point *is*: its kind, access, unit and meaning.

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::Value;

/// Shape of the values a point accepts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Kind {
    Timer {
        field: crate::TimerField,
    },
    TimerCommand {
        operation: crate::TimerOperation,
        default_seconds: u32,
    },
    /// Business state, never assigned through a client command.
    Counter {
        spec: crate::CounterSpec,
    },
    /// An explicit operation at null; it never publishes an occurrence.
    CounterCommand {
        operation: crate::CounterOperation,
    },
    /// An explicit press with no retained value.
    Button,
    Binary,
    Numeric {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        step: Option<f64>,
    },
    Enum {
        values: Vec<Arc<str>>,
    },
    Text,
}

impl Kind {
    #[must_use]
    pub fn is_numeric(&self) -> bool {
        matches!(
            self,
            Self::Numeric { .. }
                | Self::Counter { .. }
                | Self::Timer {
                    field: crate::TimerField::Remaining
                        | crate::TimerField::Generation
                        | crate::TimerField::Deadline
                }
        )
    }
    /// Commands whose identical contents may represent distinct intentions.
    #[must_use]
    pub fn is_operation(&self) -> bool {
        matches!(
            self,
            Self::Button | Self::CounterCommand { .. } | Self::TimerCommand { .. }
        )
    }

    /// Occurrences are never restored as sampled state.
    #[must_use]
    pub fn is_occurrence(&self) -> bool {
        matches!(
            self,
            Self::Button
                | Self::Timer {
                    field: crate::TimerField::Expired
                }
        )
    }

    /// Business-owned state cannot be restored from the periodic device cache.
    #[must_use]
    pub fn uses_state_cache(&self) -> bool {
        !self.is_operation() && !matches!(self, Self::Counter { .. } | Self::Timer { .. })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct Access {
    pub read: bool,
    pub write: bool,
}

/// Physical unit. Serialized as its symbol (`"W"`, `"kWh"`, `"°C"`…).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Unit {
    Watt,
    KiloWattHour,
    WattHour,
    VoltAmpere,
    Volt,
    MilliVolt,
    Ampere,
    Celsius,
    Percent,
    Lux,
    HectoPascal,
    Decibel,
    Ppm,
    Second,
    Minute,
    Hour,
    Other(Arc<str>),
}

impl Unit {
    const TABLE: [(Unit, &'static str); 16] = [
        (Unit::Watt, "W"),
        (Unit::KiloWattHour, "kWh"),
        (Unit::WattHour, "Wh"),
        (Unit::VoltAmpere, "VA"),
        (Unit::Volt, "V"),
        (Unit::MilliVolt, "mV"),
        (Unit::Ampere, "A"),
        (Unit::Celsius, "°C"),
        (Unit::Percent, "%"),
        (Unit::Lux, "lx"),
        (Unit::HectoPascal, "hPa"),
        (Unit::Decibel, "dB"),
        (Unit::Ppm, "ppm"),
        (Unit::Second, "s"),
        (Unit::Minute, "min"),
        (Unit::Hour, "h"),
    ];

    /// Parses a unit symbol. Empty → `None`; unknown symbols are preserved.
    #[must_use]
    pub fn parse(symbol: &str) -> Option<Self> {
        let symbol = symbol.trim();
        if symbol.is_empty() {
            return None;
        }
        let symbol = match symbol {
            "seconds" | "sec" => "s",
            "minutes" => "min",
            "hours" => "h",
            other => other,
        };
        Some(
            Self::TABLE
                .iter()
                .find(|(_, s)| *s == symbol)
                .map_or_else(|| Self::Other(symbol.into()), |(u, _)| u.clone()),
        )
    }

    #[must_use]
    pub fn symbol(&self) -> &str {
        match self {
            Self::Other(s) => s,
            known => Self::TABLE
                .iter()
                .find(|(u, _)| u == known)
                .map_or("", |(_, s)| s),
        }
    }
}

impl fmt::Display for Unit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.symbol())
    }
}

impl Serialize for Unit {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.symbol())
    }
}

impl<'de> Deserialize<'de> for Unit {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Self::parse(&s).ok_or_else(|| serde::de::Error::custom("empty unit"))
    }
}

/// What a point means, independent of the brand that produced it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Semantic {
    OnOff,
    Brightness,
    ColorTemp,
    Color,
    Power,
    ApparentPower,
    Energy,
    Voltage,
    Current,
    Temperature,
    Humidity,
    Pressure,
    Illuminance,
    Battery,
    BatteryLow,
    SignalStrength,
    Contact,
    Occupancy,
    WaterLeak,
    Smoke,
    Tamper,
    Config,
    /// An order to a machine at work (pause or cancel a print): an agent
    /// asking gets a human's approval first.
    Control,
    /// A door's lock. Readable, it is the bolt (`true` = locked): writing
    /// `true` locks, any other write opens. Write-only, it is an opener
    /// (unlatch, buzzer): every write opens. Opening is a person's call:
    /// an agent asks, and so does the dashboard without a code.
    Lock,
    /// The equipment's own technical health (load, traffic, chip temperature),
    /// explicitly declared by its driver. Ambient measurements keep their
    /// physical meaning; this category is never inferred from a key or unit.
    Diagnostic,
    Other,
}

impl PointSpec {
    /// Whether writing `value` here may open a door (see [`Semantic::Lock`]).
    /// `None`: a value not known yet (a toggle), so assumed to open.
    #[must_use]
    pub fn opens(&self, value: Option<&Value>) -> bool {
        self.semantic == Semantic::Lock && !(self.access.read && value == Some(&Value::Bool(true)))
    }
}

impl Semantic {
    /// Brand-agnostic inference from a property name and its unit.
    /// Unit evidence wins over naming, naming over nothing.
    #[must_use]
    pub fn infer(key: &str, unit: Option<&Unit>) -> Self {
        let name = key.rsplit('.').next().unwrap_or(key).to_ascii_lowercase();
        let name = name.trim_end_matches(|c: char| c == '_' || c.is_ascii_digit());
        let name = name.strip_suffix("_l").unwrap_or(name);
        match unit {
            Some(Unit::Watt) => return Self::Power,
            Some(Unit::VoltAmpere) => return Self::ApparentPower,
            Some(Unit::KiloWattHour | Unit::WattHour) => return Self::Energy,
            Some(Unit::Volt | Unit::MilliVolt) if name != "battery" => return Self::Voltage,
            Some(Unit::Ampere) => return Self::Current,
            Some(Unit::Celsius) => return Self::Temperature,
            Some(Unit::Lux) => return Self::Illuminance,
            Some(Unit::HectoPascal) => return Self::Pressure,
            _ => {}
        }
        match name {
            "state" => Self::OnOff,
            "brightness" => Self::Brightness,
            "color_temp" => Self::ColorTemp,
            "color" | "x" | "y" | "hue" | "saturation" => Self::Color,
            "power" => Self::Power,
            "energy" => Self::Energy,
            "temperature" => Self::Temperature,
            "humidity" => Self::Humidity,
            "battery" => Self::Battery,
            "battery_low" => Self::BatteryLow,
            "linkquality" | "rssi" => Self::SignalStrength,
            "contact" => Self::Contact,
            "occupancy" | "presence" | "motion" => Self::Occupancy,
            "water_leak" => Self::WaterLeak,
            "smoke" => Self::Smoke,
            "tamper" => Self::Tamper,
            _ => Self::Other,
        }
    }
}

/// Full description of one point of a device.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PointSpec {
    /// Stable key inside the device (`state_l1`, `color.x`).
    pub key: Arc<str>,
    /// Human label suggested by the driver.
    pub label: Arc<str>,
    pub kind: Kind,
    pub access: Access,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<Unit>,
    pub semantic: Semantic,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ValueError {
    #[error("expected a button press (null)")]
    ExpectedPress,
    #[error("expected a boolean")]
    ExpectedBool,
    #[error("expected a number")]
    ExpectedNumber,
    #[error("expected text")]
    ExpectedText,
    #[error("{value} is out of range [{min:?}, {max:?}]")]
    OutOfRange {
        value: f64,
        min: Option<f64>,
        max: Option<f64>,
    },
    #[error("{0:?} is not one of the allowed values")]
    NotAllowed(String),
    #[error("text longer than {MAX_TEXT} characters")]
    TooLong,
}

/// Validates a value against a point's kind and returns its canonical form.
/// Numbers are canonicalized to `Float`; enum values must match exactly.
/// Longest text a point accepts: a value is a setting, never a document.
pub const MAX_TEXT: usize = 4_000;

pub fn validate(kind: &Kind, value: &Value) -> Result<Value, ValueError> {
    if let Value::Text(t) = value
        && t.chars().count() > MAX_TEXT
    {
        return Err(ValueError::TooLong);
    }
    if matches!(kind, Kind::Timer { .. } | Kind::TimerCommand { .. }) {
        return valid_timer(kind, value)
            .then(|| value.clone())
            .ok_or(ValueError::ExpectedNumber);
    }
    match (kind, value) {
        (Kind::CounterCommand { .. } | Kind::Button, Value::Null) => Ok(Value::Null),
        (Kind::CounterCommand { .. } | Kind::Button, _) => Err(ValueError::ExpectedPress),
        (Kind::Counter { spec }, Value::Int(value)) if spec.is_valid() && spec.contains(*value) => {
            Ok(Value::Int(*value))
        }
        (Kind::Counter { .. } | Kind::Timer { .. } | Kind::TimerCommand { .. }, _) => {
            Err(ValueError::ExpectedNumber)
        }
        (Kind::Binary, Value::Bool(_)) | (Kind::Text, Value::Text(_)) => Ok(value.clone()),
        (Kind::Binary, _) => Err(ValueError::ExpectedBool),
        (Kind::Numeric { min, max, .. }, v) => {
            let n = v
                .as_f64()
                .filter(|n| n.is_finite())
                .ok_or(ValueError::ExpectedNumber)?;
            let below = min.is_some_and(|m| n < m);
            let above = max.is_some_and(|m| n > m);
            if below || above {
                return Err(ValueError::OutOfRange {
                    value: n,
                    min: *min,
                    max: *max,
                });
            }
            Ok(Value::Float(n))
        }
        (Kind::Enum { values }, Value::Text(t)) => values
            .iter()
            .any(|v| v == t)
            .then(|| value.clone())
            .ok_or_else(|| ValueError::NotAllowed(t.to_string())),
        (Kind::Enum { .. } | Kind::Text, _) => Err(ValueError::ExpectedText),
    }
}

fn valid_timer(kind: &Kind, value: &Value) -> bool {
    use crate::{MAX_TIMER_SECONDS, TimerField, TimerOperation};
    match (kind, value) {
        (
            Kind::TimerCommand {
                operation: TimerOperation::Start,
                default_seconds,
            },
            Value::Int(seconds),
        ) => {
            (1..=MAX_TIMER_SECONDS).contains(default_seconds)
                && (1..=i64::from(MAX_TIMER_SECONDS)).contains(seconds)
        }
        (
            Kind::TimerCommand {
                operation: TimerOperation::Cancel,
                default_seconds,
            },
            Value::Null,
        ) => (1..=MAX_TIMER_SECONDS).contains(default_seconds),
        (
            Kind::Timer {
                field: TimerField::Expired | TimerField::Deadline,
            },
            Value::Null,
        ) => true,
        (
            Kind::Timer {
                field: TimerField::State,
            },
            Value::Text(state),
        ) => matches!(
            state.as_ref(),
            "idle" | "running" | "cancelled" | "finished"
        ),
        (
            Kind::Timer {
                field: TimerField::Remaining,
            },
            Value::Int(seconds),
        ) => (0..=i64::from(MAX_TIMER_SECONDS)).contains(seconds),
        (
            Kind::Timer {
                field: TimerField::Generation | TimerField::Deadline,
            },
            Value::Int(number),
        ) => (0..=crate::MAX_COUNTER_VALUE).contains(number),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_symbols_round_trip() {
        for (unit, symbol) in &Unit::TABLE {
            assert_eq!(Unit::parse(symbol).as_ref(), Some(unit));
            assert_eq!(unit.symbol(), *symbol);
        }
        assert_eq!(Unit::parse("µg/m³"), Some(Unit::Other("µg/m³".into())));
        assert_eq!(Unit::parse("  "), None);
        assert_eq!(serde_json::to_string(&Unit::Celsius).unwrap(), "\"°C\"");
    }

    #[test]
    fn semantic_inference() {
        assert_eq!(Semantic::infer("state", None), Semantic::OnOff);
        assert_eq!(Semantic::infer("state_l1", None), Semantic::OnOff);
        assert_eq!(
            Semantic::infer("SINSTS", Some(&Unit::VoltAmpere)),
            Semantic::ApparentPower
        );
        assert_eq!(
            Semantic::infer("EAST", Some(&Unit::WattHour)),
            Semantic::Energy
        );
        assert_eq!(
            Semantic::infer("battery", Some(&Unit::Percent)),
            Semantic::Battery
        );
        assert_eq!(
            Semantic::infer("linkquality", None),
            Semantic::SignalStrength
        );
        assert_eq!(Semantic::infer("color.x", None), Semantic::Color);
        assert_eq!(Semantic::infer("whatever", None), Semantic::Other);
    }

    #[test]
    fn validation() {
        let numeric = Kind::Numeric {
            min: Some(0.0),
            max: Some(254.0),
            step: None,
        };
        assert_eq!(validate(&numeric, &Value::Int(10)), Ok(Value::Float(10.0)));
        assert!(matches!(
            validate(&numeric, &Value::Int(300)),
            Err(ValueError::OutOfRange { .. })
        ));
        assert_eq!(
            validate(&numeric, &Value::Bool(true)),
            Err(ValueError::ExpectedNumber)
        );
        assert_eq!(
            validate(&numeric, &Value::Float(f64::NAN)),
            Err(ValueError::ExpectedNumber)
        );

        assert_eq!(
            validate(&Kind::Binary, &Value::Bool(false)),
            Ok(Value::Bool(false))
        );
        assert_eq!(
            validate(&Kind::Binary, &Value::from("ON")),
            Err(ValueError::ExpectedBool)
        );

        let modes = Kind::Enum {
            values: vec!["off".into(), "on".into()],
        };
        assert!(validate(&modes, &Value::from("on")).is_ok());
        assert_eq!(
            validate(&modes, &Value::from("maybe")),
            Err(ValueError::NotAllowed("maybe".into()))
        );
        assert_eq!(
            validate(&Kind::Text, &Value::Int(1)),
            Err(ValueError::ExpectedText)
        );
    }

    #[test]
    fn n0_diagnostic_round_trips_in_a_point_spec() {
        for unit in ["B", "B/s", "Mbit/s"] {
            let fixture = serde_json::json!({
                "key": "traffic", "label": "Traffic", "kind": { "type": "numeric" },
                "access": { "read": true, "write": false },
                "unit": unit, "semantic": "diagnostic"
            });
            let spec: PointSpec = serde_json::from_value(fixture.clone()).unwrap();
            assert_eq!(spec.unit, Some(Unit::Other(unit.into())));
            assert_eq!(serde_json::to_value(&spec).unwrap(), fixture);
        }
        assert!(serde_json::from_str::<Semantic>("\"diagnostics\"").is_err());
    }

    #[test]
    fn n0_existing_semantics_units_and_inference_are_unchanged() {
        let existing = [
            (Semantic::OnOff, "on_off"),
            (Semantic::Brightness, "brightness"),
            (Semantic::ColorTemp, "color_temp"),
            (Semantic::Color, "color"),
            (Semantic::Power, "power"),
            (Semantic::ApparentPower, "apparent_power"),
            (Semantic::Energy, "energy"),
            (Semantic::Voltage, "voltage"),
            (Semantic::Current, "current"),
            (Semantic::Temperature, "temperature"),
            (Semantic::Humidity, "humidity"),
            (Semantic::Pressure, "pressure"),
            (Semantic::Illuminance, "illuminance"),
            (Semantic::Battery, "battery"),
            (Semantic::BatteryLow, "battery_low"),
            (Semantic::SignalStrength, "signal_strength"),
            (Semantic::Contact, "contact"),
            (Semantic::Occupancy, "occupancy"),
            (Semantic::WaterLeak, "water_leak"),
            (Semantic::Smoke, "smoke"),
            (Semantic::Tamper, "tamper"),
            (Semantic::Config, "config"),
            (Semantic::Control, "control"),
            (Semantic::Lock, "lock"),
            (Semantic::Other, "other"),
        ];
        for (semantic, name) in existing {
            let json = serde_json::Value::from(name);
            assert_eq!(serde_json::to_value(semantic).unwrap(), json);
            assert_eq!(serde_json::from_value::<Semantic>(json).unwrap(), semantic);
        }
        for unit in ["B", "B/s", "Mbit/s"] {
            let parsed = Unit::parse(unit).unwrap();
            assert_eq!(parsed, Unit::Other(unit.into()));
            assert_eq!(serde_json::to_value(&parsed).unwrap(), unit);
            assert_eq!(Semantic::infer("rx_bytes", Some(&parsed)), Semantic::Other);
        }
        assert_eq!(
            Semantic::infer("temperature", Some(&Unit::Celsius)),
            Semantic::Temperature
        );
        assert_eq!(
            Semantic::infer("ram_usage", Some(&Unit::Percent)),
            Semantic::Other
        );
    }
}
