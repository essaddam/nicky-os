//! Helpers: virtual devices the house keeps for itself, like Home
//! Assistant's `input_select` / `input_boolean` / `input_number` / `input_text`
//! (« mode de la maison », « alarme active »…).
//!
//! Each helper is a device of this instance with one writable point. A
//! command simply becomes its new value (guard and journal apply, as for any
//! device); the value survives restarts through the hub's state cache, and a
//! helper never seen before starts at its `initial` value.
//!
//! ```toml
//! [[driver]]
//! id = "maison"
//! kind = "helpers"
//! [[driver.options.helper]]
//! id = "mode_maison"
//! name = "Mode Maison"
//! kind = "select"
//! options = ["Normal", "Absent", "Nuit", "Vacances"]
//! initial = "Normal"
//! ```

use anyhow::{bail, ensure};
use moli_core::{
    Access, CounterOperation, CounterSpec, Device, DeviceId, Kind, MAX_COUNTER_VALUE, PointSpec,
    Semantic, Value, validate,
};
use moli_runtime::{BoxFuture, Driver, DriverCtx};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default, rename = "helper")]
    pub helpers: Vec<Helper>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Helper {
    /// Stable id: the device is `<instance>:<id>`. Never rename it.
    pub id: String,
    pub name: String,
    pub kind: HelperKind,
    /// `select`: the choices.
    #[serde(default)]
    pub options: Vec<String>,
    /// `number`: bounds and step.
    #[serde(default)]
    pub min: Option<Number>,
    #[serde(default)]
    pub max: Option<Number>,
    #[serde(default)]
    pub step: Option<Number>,
    /// The value before anyone set one.
    #[serde(default)]
    pub initial: Option<toml::Value>,
    /// Room, if it belongs to one (most belong to the whole house).
    #[serde(default)]
    pub room: Option<String>,
    /// Timer default duration; commands may choose another bounded duration.
    #[serde(default)]
    pub duration_s: Option<u32>,
}

/// Preserve the lexical integer/float distinction until the helper kind is known.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(untagged)]
pub enum Number {
    Integer(i64),
    Float(f64),
}

impl Number {
    #[allow(clippy::cast_precision_loss)]
    fn as_f64(self) -> f64 {
        match self {
            Self::Integer(value) => value as f64,
            Self::Float(value) => value,
        }
    }
    fn integer(self) -> anyhow::Result<i64> {
        match self {
            Self::Integer(value) => Ok(value),
            Self::Float(_) => bail!("{}", moli_i18n::tr!("pilotes.aides.compteur_entier")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HelperKind {
    Timer,
    Counter,
    Button,
    Select,
    Switch,
    Number,
    Text,
}

impl Helper {
    fn timer_specs(&self) -> Vec<PointSpec> {
        use moli_core::{TimerField, TimerOperation, Unit};
        let default_seconds = self.duration_s.unwrap_or(moli_core::DEFAULT_TIMER_SECONDS);
        let fields = [
            (
                "state",
                TimerField::State,
                moli_i18n::tr!("pilotes.aides.minuteur_etat"),
                None,
            ),
            (
                "remaining",
                TimerField::Remaining,
                moli_i18n::tr!("pilotes.aides.minuteur_restant"),
                Some(Unit::Second),
            ),
            (
                "generation",
                TimerField::Generation,
                moli_i18n::tr!("pilotes.aides.minuteur_generation"),
                None,
            ),
            (
                "deadline",
                TimerField::Deadline,
                moli_i18n::tr!("pilotes.aides.minuteur_echeance"),
                None,
            ),
            (
                "finished",
                TimerField::Expired,
                moli_i18n::tr!("pilotes.aides.minuteur_termine"),
                None,
            ),
        ];
        let mut specs: Vec<_> = fields
            .into_iter()
            .map(|(key, field, label, unit)| PointSpec {
                key: key.into(),
                label: label.into(),
                kind: Kind::Timer { field },
                access: Access {
                    read: field != TimerField::Expired,
                    write: false,
                },
                unit,
                semantic: Semantic::Other,
            })
            .collect();
        for (key, operation, label) in [
            (
                "start",
                TimerOperation::Start,
                moli_i18n::tr!("pilotes.aides.minuteur_demarrer"),
            ),
            (
                "cancel",
                TimerOperation::Cancel,
                moli_i18n::tr!("pilotes.aides.minuteur_annuler"),
            ),
        ] {
            specs.push(PointSpec {
                key: key.into(),
                label: label.into(),
                kind: Kind::TimerCommand {
                    operation,
                    default_seconds,
                },
                access: Access {
                    read: false,
                    write: true,
                },
                unit: None,
                semantic: Semantic::Other,
            });
        }
        specs
    }

    fn counter_spec(&self) -> anyhow::Result<CounterSpec> {
        let initial = match &self.initial {
            None => 0,
            Some(toml::Value::Integer(value)) => *value,
            Some(_) => bail!("{}", moli_i18n::tr!("pilotes.aides.compteur_entier")),
        };
        let spec = CounterSpec {
            initial,
            min: self
                .min
                .map(Number::integer)
                .transpose()?
                .unwrap_or(-MAX_COUNTER_VALUE),
            max: self
                .max
                .map(Number::integer)
                .transpose()?
                .unwrap_or(MAX_COUNTER_VALUE),
            step: self.step.map(Number::integer).transpose()?.unwrap_or(1),
        };
        ensure!(
            spec.is_valid() && self.options.is_empty(),
            "{}",
            moli_i18n::tr!("pilotes.aides.compteur_definition")
        );
        Ok(spec)
    }

    fn specs(&self) -> Vec<PointSpec> {
        if self.kind == HelperKind::Timer {
            return self.timer_specs();
        }
        let value = self.spec();
        if self.kind != HelperKind::Counter {
            return vec![value];
        }
        let definition = self.counter_spec().expect("validated counter definition");
        let mut specs = vec![value];
        for (key, operation, label) in [
            (
                "increment",
                CounterOperation::Increment,
                moli_i18n::tr!("pilotes.aides.compteur_increment", pas = definition.step),
            ),
            (
                "reset",
                CounterOperation::Reset,
                moli_i18n::tr!("pilotes.aides.compteur_reset", valeur = definition.initial),
            ),
        ] {
            specs.push(PointSpec {
                key: key.into(),
                label: label.into(),
                kind: Kind::CounterCommand { operation },
                access: Access {
                    read: false,
                    write: true,
                },
                unit: None,
                semantic: Semantic::Other,
            });
        }
        specs
    }

    /// The point that holds its value.
    fn spec(&self) -> PointSpec {
        let (key, kind, semantic) = match self.kind {
            HelperKind::Timer => (
                "state",
                Kind::Timer {
                    field: moli_core::TimerField::State,
                },
                Semantic::Other,
            ),
            HelperKind::Button => ("press", Kind::Button, Semantic::Other),
            HelperKind::Counter => (
                "value",
                Kind::Counter {
                    spec: self.counter_spec().expect("validated counter definition"),
                },
                Semantic::Other,
            ),
            HelperKind::Select => (
                "state",
                Kind::Enum {
                    values: self.options.iter().map(|o| o.as_str().into()).collect(),
                },
                Semantic::Other,
            ),
            HelperKind::Switch => ("on", Kind::Binary, Semantic::OnOff),
            HelperKind::Number => (
                "value",
                Kind::Numeric {
                    min: self.min.map(Number::as_f64),
                    max: self.max.map(Number::as_f64),
                    step: self.step.map(Number::as_f64),
                },
                Semantic::Other,
            ),
            HelperKind::Text => ("value", Kind::Text, Semantic::Other),
        };
        PointSpec {
            key: key.into(),
            label: self.name.as_str().into(),
            kind,
            access: Access {
                read: self.kind != HelperKind::Button,
                write: !matches!(self.kind, HelperKind::Counter | HelperKind::Timer),
            },
            unit: None,
            semantic,
        }
    }

    /// The starting value, checked against the point.
    fn initial_value(&self) -> anyhow::Result<Value> {
        if self.kind == HelperKind::Timer {
            ensure!(
                self.initial.is_none()
                    && self.options.is_empty()
                    && self.min.is_none()
                    && self.max.is_none()
                    && self.step.is_none()
                    && (1..=moli_core::MAX_TIMER_SECONDS)
                        .contains(&self.duration_s.unwrap_or(moli_core::DEFAULT_TIMER_SECONDS)),
                "{}",
                moli_i18n::tr!("pilotes.aides.minuteur_definition")
            );
            return Ok(Value::Text("idle".into()));
        }
        ensure!(
            self.duration_s.is_none(),
            "{}",
            moli_i18n::tr!("pilotes.aides.minuteur_definition")
        );
        if self.kind == HelperKind::Counter {
            return Ok(Value::Int(self.counter_spec()?.initial));
        }
        if self.kind == HelperKind::Button {
            ensure!(
                self.initial.is_none()
                    && self.options.is_empty()
                    && self.min.is_none()
                    && self.max.is_none()
                    && self.step.is_none(),
                "{}",
                moli_i18n::tr!("pilotes.aides.bouton_sans_valeur")
            );
            return Ok(Value::Null);
        }
        let raw = match (&self.initial, self.kind) {
            (Some(toml::Value::String(s)), _) => Value::Text(s.as_str().into()),
            (Some(toml::Value::Boolean(b)), _) => Value::Bool(*b),
            (Some(toml::Value::Integer(i)), _) => Value::Int(*i),
            (Some(toml::Value::Float(f)), _) => Value::Float(*f),
            (Some(other), _) => bail!("helper {}: initial {other} is not a value", self.id),
            (None, HelperKind::Select) => match self.options.first() {
                Some(first) => Value::Text(first.as_str().into()),
                None => Value::Null,
            },
            (None, HelperKind::Switch) => Value::Bool(false),
            (None, HelperKind::Number) => Value::Float(self.min.map_or(0.0, Number::as_f64)),
            (None, HelperKind::Text) => Value::Text("".into()),
            (None, HelperKind::Button | HelperKind::Counter | HelperKind::Timer) => Value::Null,
        };
        validate(&self.spec().kind, &raw)
            .map_err(|e| anyhow::anyhow!("helper {}: initial value: {e}", self.id))
    }
}

#[derive(Debug)]
pub struct Helpers {
    config: Config,
}

impl Helpers {
    /// Checks the configuration (ids, options, initial values).
    pub fn new(config: Config) -> anyhow::Result<Self> {
        ensure!(
            config
                .helpers
                .iter()
                .filter(|helper| helper.kind == HelperKind::Timer)
                .count()
                <= 128,
            "{}",
            moli_i18n::tr!("pilotes.aides.minuteurs_max")
        );
        ensure!(
            config
                .helpers
                .iter()
                .filter(|helper| helper.kind == HelperKind::Button)
                .count()
                <= 128,
            "{}",
            moli_i18n::tr!("pilotes.aides.boutons_max")
        );
        ensure!(
            config
                .helpers
                .iter()
                .filter(|h| h.kind == HelperKind::Counter)
                .count()
                <= 128,
            "{}",
            moli_i18n::tr!("pilotes.aides.compteurs_max")
        );
        let mut seen = std::collections::HashSet::new();
        for h in &config.helpers {
            ensure!(
                !h.id.is_empty() && !h.id.contains([':', '/']),
                "helper id {:?}: non-empty, without ':' nor '/'",
                h.id
            );
            ensure!(seen.insert(&h.id), "helper id {:?} twice", h.id);
            ensure!(
                h.kind != HelperKind::Select || !h.options.is_empty(),
                "helper {}: a select needs options",
                h.id
            );
            h.initial_value()?;
        }
        Ok(Self { config })
    }
}

impl Driver for Helpers {
    fn kind(&self) -> &'static str {
        "helpers"
    }

    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            for h in &self.config.helpers {
                let id = ctx.device_id(&h.id);
                let spec = h.spec();
                ctx.upsert_device(Device {
                    id: id.clone(),
                    instance: ctx.instance().clone(),
                    native_name: h.name.as_str().into(),
                    manufacturer: Some("Moli".into()),
                    model: Some("Aide".into()),
                    description: None,
                    native_room: h.room.as_deref().map(Into::into),
                    members: Vec::new(),
                    points: h.specs(),
                });
                // Kept across restarts by the state cache; set only once.
                if h.kind == HelperKind::Timer {
                    ctx.initialize_timer(
                        &id,
                        h.duration_s.unwrap_or(moli_core::DEFAULT_TIMER_SECONDS),
                    )
                    .await?;
                } else if h.kind == HelperKind::Counter {
                    ctx.initialize_counter(&id, h.counter_spec()?).await?;
                } else if h.kind != HelperKind::Button && ctx.current(&id, &spec.key).is_none() {
                    ctx.set_state(&id, &spec.key, h.initial_value()?);
                }
                ctx.set_availability(&id, true);
            }
            ctx.ready();
            while let Some(command) = ctx.next_command().await {
                if command
                    .device
                    .point(&command.key)
                    .is_some_and(|point| matches!(point.kind, Kind::TimerCommand { .. }))
                {
                    ctx.timer_command(command).await;
                    continue;
                }
                if command
                    .device
                    .point(&command.key)
                    .is_some_and(|point| matches!(point.kind, Kind::CounterCommand { .. }))
                {
                    ctx.counter_command(command).await;
                    continue;
                }
                if command
                    .device
                    .point(&command.key)
                    .is_some_and(|point| point.kind == Kind::Button)
                {
                    let result = ctx.press_button(&command).await;
                    command.reply_result(result);
                    continue;
                }
                // Already validated against the point by the hub.
                ctx.set_state(&command.device.id, &command.key, command.value.clone());
                command.reply(Ok(()));
            }
            Ok(())
        })
    }
}

/// Builds the device id of a helper (for tests and remaps).
#[must_use]
pub fn helper_device(instance: &moli_core::InstanceId, id: &str) -> DeviceId {
    DeviceId::new(instance, id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    use moli_core::{InstanceId, Origin, PointId};
    use moli_runtime::{Hub, HubOptions, spawn_driver};
    use tokio_util::sync::CancellationToken;

    fn config() -> Config {
        toml::from_str(
            r#"
            [[helper]]
            id = "mode_maison"
            name = "Mode Maison"
            kind = "select"
            options = ["Normal", "Absent", "Nuit", "Vacances"]
            initial = "Normal"
            [[helper]]
            id = "alarme_active"
            name = "Alarme Active"
            kind = "switch"
            initial = true
            [[helper]]
            id = "budget"
            name = "Budget élec mensuel"
            kind = "number"
            min = 50.0
            max = 500.0
            step = 10.0
            initial = 200.0
            "#,
        )
        .unwrap()
    }

    #[test]
    fn bad_configurations_are_refused() {
        let mut c = config();
        c.helpers[0].initial = Some(toml::Value::String("Fête".into()));
        assert!(Helpers::new(c).is_err(), "initial outside the options");
        let mut c = config();
        c.helpers[1].id = "a:b".into();
        assert!(Helpers::new(c).is_err());
        let mut c = config();
        c.helpers[0].options.clear();
        assert!(Helpers::new(c).is_err());
    }

    #[tokio::test]
    async fn helpers_start_at_their_initial_value_and_take_orders() {
        let hub = Hub::new(HubOptions::default()).unwrap();
        let cancel = CancellationToken::new();
        spawn_driver(
            &hub,
            InstanceId::from("maison"),
            Arc::new(Helpers::new(config()).unwrap()),
            cancel.clone(),
        );
        let mode = PointId::from("maison:mode_maison/state".to_owned());
        for _ in 0..100 {
            if hub.state(&mode).is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(
            hub.state(&mode).unwrap().value,
            Value::Text("Normal".into())
        );
        assert_eq!(
            hub.state(&PointId::from("maison:alarme_active/on".to_owned()))
                .unwrap()
                .value,
            Value::Bool(true)
        );
        hub.command(&mode, Value::Text("Nuit".into()), Origin::Ui, None)
            .await
            .unwrap();
        assert_eq!(hub.state(&mode).unwrap().value, Value::Text("Nuit".into()));
        let refused = hub
            .command(&mode, Value::Text("Fête".into()), Origin::Ui, None)
            .await;
        assert!(refused.is_err(), "only the listed options");
        cancel.cancel();
    }
}
