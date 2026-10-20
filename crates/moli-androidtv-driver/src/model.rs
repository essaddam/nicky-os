use moli_androidtv::{cast, remote};
use moli_core::{Access, Device, DeviceId, InstanceId, Kind, PointSpec, Semantic, Unit};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Volume {
    pub level: u64,
    pub max: u64,
    pub muted: bool,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Observation {
    pub online: bool,
    pub epoch: u64,
    pub cast_epoch: u64,
    pub power: Option<bool>,
    pub app: Option<String>,
    pub volume: Option<Volume>,
    pub playing: cast::Playing,
    pub refusal: Option<(u64, String)>,
    pub refusal_count: u64,
}

impl Observation {
    pub(crate) fn remote(&mut self, event: remote::Event) {
        match event {
            remote::Event::Ready => self.online = true,
            remote::Event::Lost => {
                self.online = false;
                self.epoch = self.epoch.wrapping_add(1);
                self.power = None;
                self.app = None;
                self.volume = None;
            }
            remote::Event::Power(on) => self.power = Some(on),
            remote::Event::App(app) => {
                self.app = (app.len() <= 256 && !app.chars().any(char::is_control)).then_some(app);
            }
            remote::Event::Volume { level, max, muted } => {
                self.volume = (max > 0 && max <= 1000 && level <= max).then_some(Volume {
                    level,
                    max,
                    muted,
                });
            }
            remote::Event::Refused(link) => {
                self.refusal_count = self.refusal_count.wrapping_add(1);
                self.refusal = Some((self.refusal_count, link));
            }
        }
    }
    pub(crate) fn cast(&mut self, event: cast::Event) {
        match event {
            cast::Event::Playing(playing) => {
                if self.playing.state.is_some() && playing.state.is_none() {
                    self.cast_epoch = self.cast_epoch.wrapping_add(1);
                }
                self.playing = playing;
            }
            cast::Event::Lost => {
                self.playing = cast::Playing::default();
                self.cast_epoch = self.cast_epoch.wrapping_add(1);
            }
        }
    }
}

pub(crate) const KEYS: [(&str, u32); 24] = [
    ("CursorUp", 19),
    ("CursorDown", 20),
    ("CursorLeft", 21),
    ("CursorRight", 22),
    ("Confirm", 23),
    ("Back", 4),
    ("Home", 3),
    ("Options", 82),
    ("Info", 165),
    ("PlayPause", 85),
    ("Play", 126),
    ("Pause", 127),
    ("Stop", 86),
    ("Next", 87),
    ("Previous", 88),
    ("FastForward", 90),
    ("Rewind", 89),
    ("VolumeUp", 24),
    ("VolumeDown", 25),
    ("Mute", 164),
    ("ChannelStepUp", 166),
    ("ChannelStepDown", 167),
    ("Source", 178),
    ("WatchTV", 170),
];

fn point(key: &str, label: String, kind: Kind, read: bool, write: bool) -> PointSpec {
    PointSpec {
        key: key.into(),
        label: label.into(),
        kind,
        access: Access { read, write },
        unit: None,
        semantic: if key == "power" {
            Semantic::OnOff
        } else {
            Semantic::infer(key, None)
        },
    }
}

fn control_points(max: Option<u64>) -> Vec<PointSpec> {
    vec![
        point(
            "power",
            moli_i18n::tr!("pilotes.philips.allumee"),
            Kind::Binary,
            true,
            true,
        ),
        point(
            "volume",
            moli_i18n::tr!("pilotes.philips.volume"),
            Kind::Numeric {
                min: Some(0.0),
                max: max.and_then(|n| u32::try_from(n).ok()).map(f64::from),
                step: Some(1.0),
            },
            true,
            true,
        ),
        point(
            "mute",
            moli_i18n::tr!("pilotes.philips.sourdine"),
            Kind::Binary,
            true,
            true,
        ),
        point(
            "app",
            moli_i18n::tr!("pilotes.philips.application"),
            Kind::Text,
            true,
            true,
        ),
        point(
            "text",
            moli_i18n::tr!("pilotes.philips.texte"),
            Kind::Text,
            false,
            true,
        ),
        point(
            "key",
            moli_i18n::tr!("pilotes.philips.touche"),
            Kind::Enum {
                values: KEYS.iter().map(|(name, _)| (*name).into()).collect(),
            },
            false,
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
            false,
            true,
        ),
        point(
            "play_media",
            moli_i18n::tr!("pilotes.androidtv.contenu"),
            Kind::Text,
            false,
            true,
        ),
    ]
}

pub(crate) fn device(instance: &InstanceId, max: Option<u64>) -> Device {
    let mut points = control_points(max);
    for (key, label) in [
        ("media_app", "pilotes.philips.appli_en_lecture"),
        ("media_state", "pilotes.philips.lecture"),
        ("media_title", "pilotes.philips.titre"),
        ("media_subtitle", "pilotes.philips.serie_artiste"),
        ("media_image", "pilotes.philips.image"),
    ] {
        points.push(point(key, moli_i18n::tr(label), Kind::Text, true, false));
    }
    for (key, label) in [
        ("media_duration", "pilotes.philips.duree"),
        ("media_position", "pilotes.philips.position"),
    ] {
        let mut spec = point(
            key,
            moli_i18n::tr(label),
            Kind::Numeric {
                min: Some(0.0),
                max: None,
                step: Some(1.0),
            },
            true,
            false,
        );
        spec.unit = Some(Unit::Second);
        points.push(spec);
    }
    Device {
        id: DeviceId::new(instance, "tv"),
        instance: instance.clone(),
        native_name: moli_i18n::tr!("pilotes.androidtv.television").into(),
        manufacturer: None,
        model: Some("Android TV Remote v2".into()),
        description: None,
        native_room: None,
        members: Vec::new(),
        points,
    }
}
