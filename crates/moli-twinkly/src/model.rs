use std::time::Duration;

use moli_core::{Access, Kind, PointSpec, Semantic, Unit, Value};
use serde_json::Value as Json;

pub(crate) const MAX_JSON: usize = 8 * 1024;
const INVALID: &str = "pilotes.twinkly.reponse_invalide";
const REFUSED: &str = "pilotes.twinkly.refus";
const SESSION: &str = "pilotes.twinkly.session_invalide";
const VALUE: &str = "pilotes.twinkly.valeur_invalide";
const UNSUPPORTED: &str = "pilotes.twinkly.capacite_absente";
const RESUME: &str = "pilotes.twinkly.mode_reprise_inconnu";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Off,
    Color,
    Movie,
    Playlist,
    Other,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Capabilities {
    pub brightness: bool,
    pub color: bool,
}

pub(crate) struct Info {
    pub native: String,
    pub profile: String,
    pub name: String,
    pub model: String,
}

pub(crate) struct Login {
    pub token: String,
    pub response: String,
    pub lifetime: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Mode(Mode),
    Brightness(u8),
    Color([u8; 3]),
}

pub(crate) fn object(bytes: &[u8]) -> Result<Json, &'static str> {
    if bytes.len() > MAX_JSON {
        return Err(INVALID);
    }
    // Never return a serde error: it may quote a device-supplied secret.
    let raw: Json = serde_json::from_slice(bytes).map_err(|_| INVALID)?;
    if !raw.is_object() {
        return Err(INVALID);
    }
    match raw.get("code").and_then(Json::as_u64) {
        Some(1000) => Ok(raw),
        Some(_) => Err(REFUSED),
        None => Err(INVALID),
    }
}

pub(crate) fn text<'a>(raw: &'a Json, key: &str, max: usize) -> Result<&'a str, &'static str> {
    let value = raw.get(key).and_then(Json::as_str).ok_or(INVALID)?;
    if value.is_empty() || value.chars().count() > max || value.chars().any(char::is_control) {
        return Err(INVALID);
    }
    Ok(value)
}

/// Canonical identity; never derived from the name, address or LED count.
pub(crate) fn identity(mac: &str) -> Result<String, &'static str> {
    if mac.len() != 17 {
        return Err(INVALID);
    }
    let parts: Vec<_> = mac.split(':').collect();
    if parts.len() != 6
        || parts
            .iter()
            .any(|p| p.len() != 2 || !p.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(INVALID);
    }
    let native = parts.concat().to_ascii_lowercase();
    let first = u8::from_str_radix(parts[0], 16).map_err(|_| INVALID)?;
    if native == "000000000000" || first & 1 != 0 {
        return Err(INVALID);
    }
    Ok(native)
}

pub(crate) fn info(bytes: &[u8]) -> Result<Info, &'static str> {
    let raw = object(bytes)?;
    Ok(Info {
        native: identity(text(&raw, "mac", 17)?)?,
        profile: text(&raw, "led_profile", 16)?.to_owned(),
        name: text(&raw, "device_name", 64)?.to_owned(),
        model: text(&raw, "product_code", 64)?.to_owned(),
    })
}

fn token_shape(token: &str) -> bool {
    if !(12..=128).contains(&token.len()) || !token.len().is_multiple_of(4) {
        return false;
    }
    let padding = token.bytes().rev().take_while(|&b| b == b'=').count();
    padding <= 2
        && token.as_bytes()[..token.len() - padding]
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'+' || *b == b'/')
}

pub(crate) fn login(bytes: &[u8]) -> Result<Login, &'static str> {
    let raw = object(bytes)?;
    let token = text(&raw, "authentication_token", 128)?;
    let response = text(&raw, "challenge-response", 40)?;
    let lifetime = raw
        .get("authentication_token_expires_in")
        .and_then(Json::as_u64)
        .filter(|s| (1..=86_400).contains(s))
        .ok_or(SESSION)?;
    if !token_shape(token)
        || response.len() != 40
        || !response.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(SESSION);
    }
    Ok(Login {
        token: token.to_owned(),
        response: response.to_owned(),
        lifetime: Duration::from_secs(lifetime),
    })
}

pub(crate) fn static_color(version: &str, profile: &str) -> bool {
    if profile != "RGB" || version.len() > 24 {
        return false;
    }
    let numbers: Option<Vec<u16>> = version
        .split('.')
        .map(|n| {
            if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
                None
            } else {
                n.parse().ok()
            }
        })
        .collect();
    matches!(numbers.as_deref(), Some([major, minor, patch]) if (*major, *minor, *patch) >= (2, 7, 1))
}

pub(crate) fn brightness(bytes: &[u8]) -> Result<u8, &'static str> {
    let raw = object(bytes)?;
    let value = raw
        .get("value")
        .and_then(Json::as_u64)
        .and_then(|n| u8::try_from(n).ok())
        .filter(|n| *n <= 100)
        .ok_or(INVALID)?;
    match text(&raw, "mode", 16)? {
        "enabled" => Ok(value),
        "disabled" => Ok(100),
        _ => Err(INVALID),
    }
}

pub(crate) fn mode(bytes: &[u8]) -> Result<Mode, &'static str> {
    let raw = object(bytes)?;
    match text(&raw, "mode", 16)? {
        "off" => Ok(Mode::Off),
        "color" => Ok(Mode::Color),
        "movie" => Ok(Mode::Movie),
        "playlist" => Ok(Mode::Playlist),
        "demo" | "effect" | "rt" | "music" => Ok(Mode::Other),
        _ => Err(INVALID),
    }
}

fn rgb(value: &Value) -> Result<[u8; 3], &'static str> {
    let Value::Text(hex) = value else {
        return Err(VALUE);
    };
    if hex.len() != 7
        || !hex.starts_with('#')
        || !hex.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
    {
        return Err(VALUE);
    }
    Ok([
        u8::from_str_radix(&hex[1..3], 16).map_err(|_| VALUE)?,
        u8::from_str_radix(&hex[3..5], 16).map_err(|_| VALUE)?,
        u8::from_str_radix(&hex[5..7], 16).map_err(|_| VALUE)?,
    ])
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn percent(value: &Value) -> Result<u8, &'static str> {
    let n = value.as_f64().ok_or(VALUE)?;
    if !n.is_finite() || !(0.0..=100.0).contains(&n) || n.fract() != 0.0 {
        return Err(VALUE);
    }
    // Finite, integral and checked against the u8 range above.
    Ok(n as u8)
}

pub(crate) fn plan(
    key: &str,
    value: &Value,
    current: Mode,
    resume: Option<Mode>,
    caps: Capabilities,
) -> Result<Vec<Step>, &'static str> {
    match (key, value) {
        ("on", Value::Bool(false)) if current == Mode::Off => Ok(Vec::new()),
        ("on", Value::Bool(false)) => Ok(vec![Step::Mode(Mode::Off)]),
        ("on", Value::Bool(true)) if current != Mode::Off => Ok(Vec::new()),
        ("on", Value::Bool(true)) => match resume {
            Some(Mode::Color) if caps.color => Ok(vec![Step::Mode(Mode::Color)]),
            Some(mode @ (Mode::Movie | Mode::Playlist)) => Ok(vec![Step::Mode(mode)]),
            _ => Err(RESUME),
        },
        ("brightness", _) if caps.brightness => {
            let n = percent(value)?;
            if n == 0 {
                Ok(vec![Step::Mode(Mode::Off)])
            } else {
                Ok(vec![Step::Brightness(n)])
            }
        }
        ("color", _) if caps.color => {
            let mut steps = vec![Step::Color(rgb(value)?)];
            if current != Mode::Color {
                steps.push(Step::Mode(Mode::Color));
            }
            Ok(steps)
        }
        ("brightness" | "color", _) => Err(UNSUPPORTED),
        _ => Err(VALUE),
    }
}

pub(crate) fn points(caps: Capabilities) -> Vec<PointSpec> {
    let mut points = vec![PointSpec {
        key: "on".into(),
        label: moli_i18n::tr!("pilotes.twinkly.marche").into(),
        kind: Kind::Binary,
        access: Access {
            read: true,
            write: true,
        },
        unit: None,
        semantic: Semantic::OnOff,
    }];
    if caps.brightness {
        points.push(PointSpec {
            key: "brightness".into(),
            label: moli_i18n::tr!("pilotes.twinkly.luminosite").into(),
            kind: Kind::Numeric {
                min: Some(0.0),
                max: Some(100.0),
                step: Some(1.0),
            },
            access: Access {
                read: true,
                write: true,
            },
            unit: Some(Unit::Percent),
            semantic: Semantic::Brightness,
        });
    }
    if caps.color {
        points.push(PointSpec {
            key: "color".into(),
            label: moli_i18n::tr!("pilotes.twinkly.couleur").into(),
            kind: Kind::Text,
            access: Access {
                read: true,
                write: true,
            },
            unit: None,
            semantic: Semantic::Color,
        });
    }
    points
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
