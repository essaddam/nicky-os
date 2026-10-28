//! Les structures d'aujourd'hui, recopiées depuis `moli-core` et `moli-runtime` (a835893), pour
//! les mesurer à côté de la proposition : la description d'un appareil (`Device`, `PointSpec`),
//! le chemin d'un ordre (`Hub::prepare`, `CommandRequest`, la ligne de journal), les retenues
//! (`ApprovalRequest`), la copie de toute la maison que lit l'assistant (`Hub::snapshot`) et son
//! inventaire (moli-assistant house.rs). Les répliques sont **au mieux** : ni les canaux de Tokio,
//! ni les tâches, ni la sérialisation ne sont comptés.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::atomic::AtomicU8;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::valeurs::{Sample, Value};

/// `moli_core::Kind` (spec.rs:12), sans les variantes des aides.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Button,
    Binary,
    Numeric { min: Option<f64>, max: Option<f64>, step: Option<f64> },
    Enum { values: Vec<Arc<str>> },
    Text,
}

/// `moli_core::Unit` (spec.rs:128) : une énumération, ou un symbole libre.
#[derive(Clone, Debug, PartialEq)]
pub enum Unit {
    Watt,
    Celsius,
    Percent,
    Other(Arc<str>),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Access {
    pub read: bool,
    pub write: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Semantic {
    Measure,
    Control,
    Lock,
    Other,
}

/// `moli_core::PointSpec` (spec.rs:113) : une valeur **et** un ordre, dans la même fiche.
#[derive(Clone, Debug, PartialEq)]
pub struct PointSpec {
    pub key: Arc<str>,
    pub label: Arc<str>,
    pub kind: Kind,
    pub access: Access,
    pub unit: Option<Unit>,
    pub semantic: Semantic,
}

/// `moli_core::Device` (model.rs:10).
#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub id: Arc<str>,
    pub instance: Arc<str>,
    pub native_name: Arc<str>,
    pub manufacturer: Option<Arc<str>>,
    pub model: Option<Arc<str>>,
    pub description: Option<Arc<str>>,
    pub native_room: Option<Arc<str>>,
    pub members: Vec<Arc<str>>,
    pub points: Vec<PointSpec>,
}

impl Device {
    pub fn point(&self, key: &str) -> Option<&PointSpec> {
        self.points.iter().find(|p| &*p.key == key)
    }
}

/// `Hub::prepare` (hub.rs:1731) : `PointId::split` alloue un `DeviceId`, une recherche dans le
/// `BTreeMap` des appareils, la recherche linéaire du point, l'accès, la validation de la valeur.
pub fn prepare(devices: &BTreeMap<Arc<str>, Arc<Device>>, point: &str, value: &Value) -> Result<Arc<Device>, &'static str> {
    let (d, key) = point.split_once('/').ok_or("unknown point")?;
    let id: Arc<str> = Arc::from(d);
    let device = devices.get(&id).ok_or("unknown point")?;
    let spec = device.point(key).ok_or("unknown point")?;
    if !spec.access.write {
        return Err("point is read-only");
    }
    match (&spec.kind, value) {
        (Kind::Binary, Value::Bool(_)) | (Kind::Button, Value::Null) | (Kind::Text, Value::Text(_)) => {}
        (Kind::Numeric { min, max, .. }, Value::Float(f)) => {
            if min.is_some_and(|m| *f < m) || max.is_some_and(|m| *f > m) {
                return Err("out of range");
            }
        }
        (Kind::Enum { values }, Value::Text(t)) => {
            if !values.iter().any(|v| **v == **t) {
                return Err("not a choice");
            }
        }
        _ => return Err("invalid value"),
    }
    Ok(Arc::clone(device))
}

/// `moli_core::Origin`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Ui,
    Mcp,
    Assistant,
    Automation,
}

/// `moli_core::Outcome` (event.rs:251).
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Ok,
    Err(String),
    Pending(String),
}

/// `Action::Command` (event.rs:196).
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Command { point: Arc<str>, value: Value },
}

/// `moli_core::JournalEntry` (event.rs:261), gardé 500 fois en mémoire (`journal_capacity`).
#[derive(Clone, Debug, PartialEq)]
pub struct JournalEntry {
    pub id: u64,
    pub ts: u64,
    pub origin: Origin,
    pub actor: Option<String>,
    pub action: Action,
    pub outcome: Outcome,
    pub duration_ms: u64,
    pub correlation: Option<String>,
}

/// `moli_core::ApprovalRequest` (event.rs:77), 50 au plus (`MAX_PENDING`).
#[derive(Clone, Debug, PartialEq)]
pub struct ApprovalRequest {
    pub id: u64,
    pub ts: u64,
    pub expires: u64,
    pub point: Arc<str>,
    pub value: Value,
    pub origin: Origin,
    pub actor: Option<String>,
    pub reason: String,
    pub device_name: String,
    pub native_name: String,
    pub rooms: Vec<String>,
    pub request_id: Option<(String, u64)>,
    pub correlation: Option<String>,
}

/// Ce que tient un `oneshot` de Tokio pour la réponse (`Inner<Result<(), CommandError>>` : état,
/// valeur, deux réveils), recopié par sa taille.
pub struct UnCoup {
    _etat: usize,
    _valeur: Option<Result<(), String>>,
    _reveils: [usize; 4],
}

/// `CommandRequest` (driver.rs:66), et ce que l'appelant garde pour écrire la ligne de journal
/// (`Delivery`, `Observer`, la fermeture `JournalLine`, hub.rs:343-431).
pub struct EnVol {
    pub point: Arc<str>,
    pub device: Arc<Device>,
    pub key: Arc<str>,
    pub value: Value,
    pub deadline: Instant,
    pub origin: Origin,
    pub actor: Option<String>,
    pub request_id: Option<(String, u64)>,
    pub correlation: Option<String>,
    pub reply: Arc<Mutex<Option<Arc<UnCoup>>>>,
    pub claim: Arc<AtomicU8>,
    pub journal: Box<dyn FnOnce() -> (Origin, Option<String>, Action, Option<String>) + Send>,
}

/// Une demande en vol, comme aujourd'hui : l'ordre et ce que son appelant garde.
pub fn en_vol(device: &Arc<Device>, point: &str, value: Value, actor: Option<&str>, correlation: bool) -> EnVol {
    let key: Arc<str> = Arc::from(point.rsplit('/').next().unwrap_or(""));
    let point: Arc<str> = Arc::from(point);
    let actor = actor.map(str::to_string);
    let correlation = correlation.then(|| "0123456789abcdef0123456789abcdef".to_string());
    let (a2, p2, v2, c2) = (actor.clone(), Arc::clone(&point), value.clone(), correlation.clone());
    EnVol {
        point,
        device: Arc::clone(device),
        key,
        value,
        deadline: Instant::now(),
        origin: Origin::Ui,
        actor,
        request_id: None,
        correlation,
        reply: Arc::new(Mutex::new(Some(Arc::new(UnCoup { _etat: 0, _valeur: None, _reveils: [0; 4] })))),
        claim: Arc::new(AtomicU8::new(0)),
        journal: Box::new(move || (Origin::Ui, a2, Action::Command { point: p2, value: v2 }, c2)),
    }
}

/// `DeviceView` (hub.rs:298) : ce que `Hub::snapshot` copie pour chaque appareil, à chaque
/// requête de l'assistant (moli-assistant lib.rs:1139).
#[derive(Clone)]
pub struct DeviceView {
    pub device: Arc<Device>,
    pub label: (Option<String>, Option<String>),
    pub online: Option<bool>,
    pub state: BTreeMap<Arc<str>, Sample>,
}

pub fn snapshot(devices: &BTreeMap<Arc<str>, Arc<Device>>, states: &BTreeMap<Arc<str>, BTreeMap<Arc<str>, Sample>>) -> Vec<DeviceView> {
    devices
        .values()
        .map(|d| DeviceView { device: Arc::clone(d), label: (None, Some("Salon".into())), online: Some(true), state: states.get(&d.id).cloned().unwrap_or_default() })
        .collect()
}

/// L'inventaire que lit le modèle, une ligne par appareil (house.rs:115) : jusqu'à 8 points, les
/// écrivables d'abord (✎), leurs choix ou leur plage.
pub fn inventaire(views: &[DeviceView]) -> String {
    let mut out = String::from("## Salon\n");
    for v in views {
        let _ = write!(out, "- {} · {}", v.device.id, v.device.native_name);
        let ecrits = v.device.points.iter().filter(|p| p.access.write);
        let lus = v.device.points.iter().filter(|p| !p.access.write);
        let mut morceaux = Vec::new();
        for p in ecrits.chain(lus).take(8) {
            let valeur = v.state.get(&p.key).map_or_else(|| "?".to_string(), |s| montrer(&s.value));
            let mut t = format!("{}{}={valeur}", p.key, if p.access.write { "✎" } else { "" });
            if let Some(u) = &p.unit {
                let _ = write!(t, " {}", match u {
                    Unit::Watt => "W",
                    Unit::Celsius => "°C",
                    Unit::Percent => "%",
                    Unit::Other(s) => s,
                });
            }
            if p.access.write {
                match &p.kind {
                    Kind::Enum { values } => {
                        let _ = write!(t, " [{}]", values.iter().take(30).map(|v| &**v).collect::<Vec<_>>().join("|"));
                    }
                    Kind::Numeric { min: Some(a), max: Some(b), .. } => {
                        let _ = write!(t, " ({a}–{b})");
                    }
                    _ => {}
                }
            }
            morceaux.push(t);
        }
        if !morceaux.is_empty() {
            let _ = write!(out, " · {}", morceaux.join(", "));
        }
        out.push('\n');
    }
    out
}

fn montrer(v: &Value) -> String {
    match v {
        Value::Null => "?".into(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => format!("{f:.1}"),
        Value::Text(t) => t.chars().take(40).collect(),
    }
}

/// `GET /api/snapshot` aujourd'hui : chaque `DeviceView` en JSON (serde), toute la maison.
pub fn snapshot_json(views: &[DeviceView]) -> String {
    let mut out = String::from("{\"devices\":[");
    for (k, v) in views.iter().enumerate() {
        if k > 0 {
            out.push(',');
        }
        let d = &v.device;
        let _ = write!(out, "{{\"id\":\"{}\",\"instance\":\"{}\",\"native_name\":\"{}\"", d.id, d.instance, d.native_name);
        if let Some(m) = &d.manufacturer {
            let _ = write!(out, ",\"manufacturer\":\"{m}\"");
        }
        if let Some(m) = &d.model {
            let _ = write!(out, ",\"model\":\"{m}\"");
        }
        if let Some(r) = &d.native_room {
            let _ = write!(out, ",\"native_room\":\"{r}\"");
        }
        out.push_str(",\"points\":[");
        for (i, p) in d.points.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            let kind = match &p.kind {
                Kind::Button => "{\"type\":\"button\"}".to_string(),
                Kind::Binary => "{\"type\":\"binary\"}".to_string(),
                Kind::Numeric { min, max, step } => format!("{{\"type\":\"numeric\",\"min\":{},\"max\":{},\"step\":{}}}", min.unwrap_or(0.0), max.unwrap_or(0.0), step.unwrap_or(0.0)),
                Kind::Enum { values } => format!("{{\"type\":\"enum\",\"values\":[{}]}}", values.iter().map(|v| format!("\"{v}\"")).collect::<Vec<_>>().join(",")),
                Kind::Text => "{\"type\":\"text\"}".to_string(),
            };
            let _ = write!(
                out,
                "{{\"key\":\"{}\",\"label\":\"{}\",\"kind\":{kind},\"access\":{{\"read\":{},\"write\":{}}}{},\"semantic\":\"measure\"}}",
                p.key,
                p.label,
                p.access.read,
                p.access.write,
                if p.unit.is_some() { ",\"unit\":\"°C\"" } else { "" }
            );
        }
        out.push_str("],\"label\":{\"room\":\"Salon\"},\"online\":true,\"state\":{");
        for (i, (cle, s)) in v.state.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            let _ = write!(out, "\"{cle}\":{{\"value\":{},\"ts\":{}}}", match &s.value {
                Value::Null => "null".to_string(),
                Value::Bool(b) => b.to_string(),
                Value::Int(i) => i.to_string(),
                Value::Float(f) => f.to_string(),
                Value::Text(t) => format!("\"{t}\""),
            }, s.ts);
        }
        out.push_str("}}");
    }
    out.push_str("],\"drivers\":[],\"approvals\":[]}");
    out
}
