//! Dashboard-compatible TV points; requested applications are not observations.
use moli_core::{Access, Device, DeviceId, Kind, PointSpec, Semantic, Unit};
use moli_runtime::DriverCtx;

pub(crate) fn device(ctx: &DriverCtx, id: &DeviceId) -> Device {
    let mut points = vec![
        spec(
            "power",
            moli_i18n::tr!("pilotes.samsung.allumee"),
            Kind::Binary,
            true,
            Semantic::OnOff,
        ),
        spec(
            "volume",
            moli_i18n::tr!("pilotes.samsung.volume"),
            Kind::Numeric {
                min: Some(0.0),
                max: Some(100.0),
                step: Some(1.0),
            },
            true,
            Semantic::infer("volume", None),
        ),
        spec(
            "mute",
            moli_i18n::tr!("pilotes.samsung.sourdine"),
            Kind::Binary,
            true,
            Semantic::infer("mute", None),
        ),
        spec(
            "app",
            moli_i18n::tr!("pilotes.samsung.application"),
            Kind::Text,
            true,
            Semantic::Other,
        ),
        spec(
            "key",
            moli_i18n::tr!("pilotes.samsung.touche"),
            Kind::Enum {
                values: crate::model::KEYS
                    .iter()
                    .map(|(k, _)| (*k).into())
                    .collect(),
            },
            true,
            Semantic::Other,
        ),
    ];
    points[1].unit = Some(Unit::Percent);
    points[3].access.read = false;
    points[4].access.read = false;
    Device {
        id: id.clone(),
        instance: ctx.instance().clone(),
        native_name: moli_i18n::tr!("pilotes.samsung.tele").into(),
        manufacturer: Some("Samsung".into()),
        model: Some("Tizen".into()),
        description: None,
        native_room: None,
        members: vec![],
        points,
    }
}
fn spec(key: &str, label: String, kind: Kind, write: bool, semantic: Semantic) -> PointSpec {
    PointSpec {
        key: key.into(),
        label: label.into(),
        kind,
        access: Access { read: true, write },
        unit: None,
        semantic,
    }
}
