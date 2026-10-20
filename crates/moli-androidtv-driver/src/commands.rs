use moli_androidtv::{cast, remote};
use moli_core::Value;
use moli_media_target::LaunchTarget;

use crate::model::Observation;

#[derive(Clone, Debug)]
pub(crate) enum Plan {
    Remote(remote::Order),
    Cast(cast::Order),
    App { links: Vec<String> },
    Content(LaunchTarget),
    Volume(u64),
    Noop,
}

pub(crate) fn plan(key: &str, value: &Value, observed: &Observation) -> Result<Plan, String> {
    let invalid = || moli_i18n::tr!("pilotes.androidtv.commande_invalide");
    match (key, value) {
        ("power", Value::Bool(on)) => Ok(Plan::Remote(remote::Order::Key(if *on {
            224
        } else {
            223
        }))),
        ("mute", Value::Bool(muted)) => {
            let volume = observed
                .volume
                .ok_or_else(|| moli_i18n::tr!("pilotes.androidtv.observation_absente"))?;
            Ok(if volume.muted == *muted {
                Plan::Noop
            } else {
                Plan::Remote(remote::Order::Key(164))
            })
        }
        ("volume", _) => {
            let volume = observed
                .volume
                .ok_or_else(|| moli_i18n::tr!("pilotes.androidtv.observation_absente"))?;
            let number = value.as_f64().ok_or_else(invalid)?;
            if !number.is_finite() || !(0.0..=1000.0).contains(&number) || number.fract() != 0.0 {
                return Err(invalid());
            }
            let target = number.to_string().parse::<u64>().map_err(|_| invalid())?;
            if target > volume.max {
                return Err(moli_i18n::tr!("pilotes.androidtv.volume_hors_plage"));
            }
            Ok(if target == volume.level {
                Plan::Noop
            } else {
                Plan::Volume(target)
            })
        }
        ("key", Value::Text(name)) => crate::model::KEYS
            .iter()
            .find(|(key, _)| key == &name.as_ref())
            .map(|(_, code)| Plan::Remote(remote::Order::Key(*code)))
            .ok_or_else(invalid),
        ("text", Value::Text(text)) if text.len() <= 4096 => {
            Ok(Plan::Remote(remote::Order::Text(text.to_string())))
        }
        ("app", Value::Text(name)) if name.len() <= 256 && !name.chars().any(char::is_control) => {
            let package = remote::package_of(name)
                .map(str::to_owned)
                .or_else(|| {
                    (name.contains('.')
                        && name.split('.').all(|part| {
                            !part.is_empty()
                                && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                        }))
                    .then(|| name.to_string())
                })
                .ok_or_else(invalid)?;
            Ok(Plan::App {
                links: remote::app_links(&package),
            })
        }
        ("media_control", Value::Text(action)) => Ok(Plan::Cast(match action.as_ref() {
            "play" => cast::Order::Play,
            "pause" => cast::Order::Pause,
            "stop" => cast::Order::Stop,
            _ => return Err(invalid()),
        })),
        ("play_media", Value::Text(raw)) => {
            let request = moli_media_target::ContentRequest::parse(raw).map_err(|_| invalid())?;
            let target = moli_media_target::resolve(&request)
                .map_err(|_| moli_i18n::tr!("pilotes.androidtv.cible_refusee"))?;
            Ok(if request.kind == moli_media_target::ContentKind::App {
                Plan::App {
                    links: target
                        .expected_packages
                        .iter()
                        .map(|package| format!("market://launch?id={package}"))
                        .collect(),
                }
            } else {
                Plan::Content(target)
            })
        }
        _ => Err(invalid()),
    }
}
