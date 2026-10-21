use crate::{discovery::Candidate, transport::Event};
use moli_androidtv::cast::{self, ReceiverOrder, ReceiverVolume};
use moli_core::{Access, Device, DeviceId, InstanceId, Kind, PointSpec, Semantic, Unit, Value};
#[derive(Clone, Debug, Default)]
pub(crate) struct Observed {
    pub online: bool,
    pub epoch: u64,
    pub media_epoch: u64,
    pub playing: cast::Playing,
    pub volume: ReceiverVolume,
}
impl Observed {
    pub fn apply(&mut self, event: Event) {
        match event {
            Event::Ready => self.online = true,
            Event::Lost | Event::Media(cast::Event::Lost) => {
                self.online = false;
                self.epoch = self.epoch.wrapping_add(1);
                self.media_epoch = self.media_epoch.wrapping_add(1);
                self.playing = cast::Playing::default();
                self.volume = ReceiverVolume::default();
            }
            Event::Volume(volume) => {
                if volume.level.is_some() {
                    self.volume.level = volume.level;
                }
                if volume.muted.is_some() {
                    self.volume.muted = volume.muted;
                }
            }
            Event::Media(cast::Event::Playing(playing)) => {
                if self.playing.state.is_some()
                    && (playing.state.is_none() || self.playing.app != playing.app)
                {
                    self.media_epoch = self.media_epoch.wrapping_add(1);
                }
                self.playing = playing;
            }
        }
    }
}
fn point(key: &str, label: String, kind: Kind, write: bool) -> PointSpec {
    PointSpec {
        key: key.into(),
        label: label.into(),
        kind,
        access: Access {
            read: key != "media_control",
            write,
        },
        unit: None,
        semantic: Semantic::infer(key, None),
    }
}
pub(crate) fn device(instance: &InstanceId, candidate: &Candidate) -> Device {
    let mut points = vec![
        point(
            "volume",
            moli_i18n::tr!("pilotes.philips.volume"),
            Kind::Numeric {
                min: Some(0.0),
                max: Some(100.0),
                step: Some(1.0),
            },
            true,
        ),
        point(
            "mute",
            moli_i18n::tr!("pilotes.philips.sourdine"),
            Kind::Binary,
            true,
        ),
        point(
            "media_control",
            moli_i18n::tr!("pilotes.philips.lecture_commande"),
            Kind::Enum {
                values: ["play", "pause", "stop"]
                    .into_iter()
                    .map(Into::into)
                    .collect(),
            },
            true,
        ),
    ];
    points[0].unit = Some(Unit::Percent);
    for (key, label) in [
        ("media_app", "pilotes.philips.appli_en_lecture"),
        ("media_state", "pilotes.philips.lecture"),
        ("media_title", "pilotes.philips.titre"),
        ("media_subtitle", "pilotes.philips.serie_artiste"),
        ("media_image", "pilotes.philips.image"),
    ] {
        points.push(point(key, moli_i18n::tr(label), Kind::Text, false));
    }
    for (key, label) in [
        ("media_duration", "pilotes.philips.duree"),
        ("media_position", "pilotes.philips.position"),
    ] {
        let mut p = point(
            key,
            moli_i18n::tr(label),
            Kind::Numeric {
                min: Some(0.0),
                max: None,
                step: Some(1.0),
            },
            false,
        );
        p.unit = Some(Unit::Second);
        points.push(p);
    }
    Device {
        id: DeviceId::new(instance, &candidate.id),
        instance: instance.clone(),
        native_name: candidate.name.as_str().into(),
        manufacturer: None,
        model: Some("Google Cast".into()),
        description: None,
        native_room: None,
        members: Vec::new(),
        points,
    }
}
pub(crate) fn plan(key: &str, value: &Value) -> Result<ReceiverOrder, String> {
    let invalid = || moli_i18n::tr!("pilotes.cast.commande");
    match (key, value) {
        ("volume", value) => {
            let level = value.as_f64().ok_or_else(invalid)?;
            if !level.is_finite() || !(0.0..=100.0).contains(&level) {
                return Err(invalid());
            }
            Ok(ReceiverOrder::Volume(level / 100.0))
        }
        ("mute", Value::Bool(muted)) => Ok(ReceiverOrder::Mute(*muted)),
        ("media_control", Value::Text(action)) => Ok(ReceiverOrder::Media(match action.as_ref() {
            "play" => cast::Order::Play,
            "pause" => cast::Order::Pause,
            "stop" => cast::Order::Stop,
            _ => return Err(invalid()),
        })),
        _ => Err(invalid()),
    }
}

#[cfg(test)]
mod tests {
    use super::{Event, Observed, ReceiverVolume};
    #[test]
    fn partial_volume_observations_preserve_other_known_fields_until_disconnect() {
        let mut state = Observed::default();
        state.apply(Event::Volume(ReceiverVolume {
            level: Some(0.3),
            muted: Some(false),
        }));
        state.apply(Event::Volume(ReceiverVolume {
            level: None,
            muted: Some(true),
        }));
        assert_eq!(
            state.volume,
            ReceiverVolume {
                level: Some(0.3),
                muted: Some(true)
            }
        );
        state.apply(Event::Lost);
        assert_eq!(state.volume, ReceiverVolume::default());
    }
}
