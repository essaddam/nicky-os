//! Pure validation over descriptors supplied by the caller, not device discovery.
use std::collections::BTreeMap;

use crate::coverage::PointSemantic;
use crate::{Config, Mode};

#[derive(Clone, Debug, PartialEq, Eq)]
// Independent descriptor/configuration switches, not alternative machine states.
#[allow(clippy::struct_excessive_bools)]
pub struct PointDescriptor {
    pub semantic: PointSemantic,
    pub readable: bool,
    pub writable: bool,
    pub binary: bool,
    pub private_keypad_input: bool,
}

pub type PointCatalog = BTreeMap<String, PointDescriptor>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigError {
    UnknownPoint,
    UnsupportedSensor,
    DisabledMode,
    EntryTooLong,
    InvalidSiren,
    MissingPrivateInput,
    InvalidSirenLimit,
    DuplicatePoint,
    ForbiddenAutomation,
}

/// Validate descriptors without discovering devices or touching their state.
pub fn validate(config: &Config, catalog: &PointCatalog) -> Result<(), ConfigError> {
    if !(1..=180).contains(&config.siren_max_s) {
        return Err(ConfigError::InvalidSirenLimit);
    }
    let mut seen = std::collections::BTreeSet::new();
    for sensor in &config.sensors {
        if !seen.insert(sensor.id.0.as_str()) {
            return Err(ConfigError::DuplicatePoint);
        }
        let point = catalog.get(&sensor.id.0).ok_or(ConfigError::UnknownPoint)?;
        if !point.readable
            || !matches!(
                point.semantic,
                PointSemantic::Contact
                    | PointSemantic::Occupancy
                    | PointSemantic::Lock
                    | PointSemantic::Tamper
            )
        {
            return Err(ConfigError::UnsupportedSensor);
        }
        for mode in sensor.modes.iter().chain(&sensor.auto_bypass) {
            let timing = config.modes.get(mode).ok_or(ConfigError::DisabledMode)?;
            if sensor
                .entry_s
                .is_some_and(|seconds| seconds > timing.entry_s)
            {
                return Err(ConfigError::EntryTooLong);
            }
        }
    }
    for siren in &config.sirens {
        if !seen.insert(siren.as_str()) {
            return Err(ConfigError::DuplicatePoint);
        }
        let point = catalog.get(siren).ok_or(ConfigError::UnknownPoint)?;
        if !point.writable || !point.binary {
            return Err(ConfigError::InvalidSiren);
        }
    }
    for keypad in &config.keypads {
        if !seen.insert(keypad.as_str()) {
            return Err(ConfigError::DuplicatePoint);
        }
        if !catalog
            .get(keypad)
            .ok_or(ConfigError::UnknownPoint)?
            .private_keypad_input
        {
            return Err(ConfigError::MissingPrivateInput);
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomationAction {
    Arm(Mode),
    Disarm,
    SkipDelay,
    Force,
}

/// The graph/editor adapter must reject every action other than arming.
pub fn validate_automation(action: AutomationAction) -> Result<(), ConfigError> {
    if matches!(action, AutomationAction::Arm(_)) {
        Ok(())
    } else {
        Err(ConfigError::ForbiddenAutomation)
    }
}
