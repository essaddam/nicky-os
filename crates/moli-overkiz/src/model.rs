//! What the Overkiz API says, in Moli's words: the devices the box knows
//! (`/setup/devices`), their states, the orders they take and the events
//! the box reports. Pure: no network, so every rule is tested on fixtures.
//!
//! Only the classes seen in real homes are published: Somfy RTS shutters
//! and gate openers (one-way radio: they never say where they are), and
//! the OGP gate, light and outlet of a Connectivity Kit. Anything else the
//! box knows is ignored, never guessed.

use std::collections::HashMap;
use std::sync::Arc;

use moli_core::{Access, Device, DeviceId, InstanceId, Kind, PointSpec, Semantic, Unit, Value};
use serde_json::{Value as Json, json};

/// `controllableName`s this driver publishes.
pub const SUPPORTED: [&str; 5] = [
    "rts:RollerShutterRTSComponent",
    "rts:GateOpenerRTSComponent",
    "ogp:Gate",
    "ogp:Light",
    "ogp:Outlet",
];

/// The moves of a shutter or a gate, in the order the dashboard shows them
/// (`my`: the favourite position stored in the motor).
pub const MOVES: [&str; 4] = ["open", "stop", "my", "close"];

/// Where a gate or a shutter is, when it says so (never an RTS one).
pub const OPENINGS: [&str; 4] = ["open", "closed", "pedestrian", "unknown"];

const ON_OFF: &str = "core:OnOffState";
const INTENSITY: &str = "core:LightIntensityState";
const OPENING_STATES: [&str; 3] = [
    "core:OpenClosedState",
    "core:OpenClosedPedestrianState",
    "core:OpenClosedUnknownState",
];

/// A name the box gives is a suggestion: bounded, without control characters.
const MAX_NAME: usize = 64;
/// What an error message may bring back (the network is hostile).
const MAX_CAUSE: usize = 120;

/// One order for the box: a command and its parameters on one device.
#[derive(Clone, Debug, PartialEq)]
pub struct Order {
    pub url: String,
    pub command: &'static str,
    pub parameters: Vec<Json>,
}

impl Order {
    /// The body of `POST /exec/apply`.
    #[must_use]
    pub fn body(&self) -> Json {
        json!({
            "label": "Moli",
            "actions": [{
                "deviceURL": self.url,
                "commands": [{ "name": self.command, "parameters": self.parameters }],
            }],
        })
    }
}

#[derive(Debug, Default)]
struct Entry {
    url: String,
    /// The box it belongs to (an account may have several).
    gateway: String,
    commands: Vec<String>,
}

/// The box a `deviceURL` belongs to: `rts://<gateway>/<address>`.
#[must_use]
pub fn gateway(url: &str) -> &str {
    url.split_once("://")
        .map_or("", |(_, rest)| rest.split('/').next().unwrap_or_default())
}

/// Moli device ↔ Overkiz `deviceURL`, and what each device accepts.
#[derive(Debug, Default)]
pub struct Index {
    devices: HashMap<DeviceId, Entry>,
    urls: HashMap<String, DeviceId>,
}

/// A device the box knows, as Moli publishes it.
#[derive(Debug)]
pub struct Found {
    pub device: Device,
    /// It answers (an RTS motor: the box can reach its radio).
    pub online: bool,
    pub values: Vec<(&'static str, Value)>,
}

/// One load of `/setup/devices`.
#[derive(Debug, Default)]
pub struct Load {
    /// Each published device, whether it answers, and its current values.
    pub devices: Vec<Found>,
    pub index: Index,
    /// Classes the box knows and this driver does not publish.
    pub skipped: Vec<String>,
}

fn has(list: &[String], name: &str) -> bool {
    list.iter().any(|n| n == name)
}

fn point(
    key: &str,
    label: &str,
    kind: Kind,
    read: bool,
    unit: Option<Unit>,
    semantic: Semantic,
) -> PointSpec {
    PointSpec {
        key: key.into(),
        label: moli_i18n::tr(label).into(),
        kind,
        access: Access { read, write: true },
        unit,
        semantic,
    }
}

/// Text from the network, made safe to show: no control characters, bounded.
#[must_use]
pub fn clean(text: &str, max: usize) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// The points a device gets from the commands and states it declares.
fn points(commands: &[String], states: &[String]) -> Vec<PointSpec> {
    let mut points = Vec::new();

    if has(commands, "open") && has(commands, "close") {
        let moves = MOVES
            .iter()
            .filter(|m| has(commands, m))
            .map(|m| Arc::from(*m))
            .collect();
        // No `read`: an RTS motor never says where it is, and Moli
        // never pretends to know.
        points.push(point(
            "control",
            "pilotes.overkiz.commande",
            Kind::Enum { values: moves },
            false,
            None,
            Semantic::Other,
        ));
    }
    if OPENING_STATES.iter().any(|s| has(states, s)) {
        let mut opening = point(
            "opening",
            "pilotes.overkiz.ouverture",
            Kind::Enum {
                values: OPENINGS.iter().map(|o| Arc::from(*o)).collect(),
            },
            true,
            None,
            Semantic::Other,
        );
        opening.access.write = false;
        points.push(opening);
    }
    if has(commands, "on") && has(commands, "off") {
        points.push(point(
            "on",
            "pilotes.overkiz.marche",
            Kind::Binary,
            has(states, ON_OFF),
            None,
            Semantic::OnOff,
        ));
    }
    if has(commands, "setIntensity") {
        points.push(point(
            "brightness",
            "pilotes.overkiz.luminosite",
            Kind::Numeric {
                min: Some(0.0),
                max: Some(100.0),
                step: Some(1.0),
            },
            has(states, INTENSITY),
            Some(Unit::Percent),
            Semantic::Brightness,
        ));
    }
    points
}

/// Every device of a `/setup/devices` answer this driver publishes.
#[must_use]
pub fn load(instance: &InstanceId, devices: &Json) -> Load {
    let mut out = Load::default();
    for raw in devices.as_array().into_iter().flatten() {
        let controllable = raw["controllableName"].as_str().unwrap_or_default();
        let Some(url) = raw["deviceURL"].as_str().filter(|u| !u.is_empty()) else {
            continue;
        };
        if !SUPPORTED.contains(&controllable) {
            if !controllable.is_empty() && !out.skipped.iter().any(|s| s == controllable) {
                out.skipped.push(clean(controllable, MAX_NAME));
            }
            continue;
        }
        if raw["enabled"] == Json::Bool(false) {
            continue;
        }
        let definition = &raw["definition"];
        let commands: Vec<String> = definition["commands"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| c["commandName"].as_str().map(str::to_owned))
            .collect();
        let states: Vec<String> = definition["states"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| s["name"].as_str().map(str::to_owned))
            .collect();
        let points = points(&commands, &states);
        if points.is_empty() {
            continue;
        }

        let model = definition["widgetName"]
            .as_str()
            .map_or_else(|| clean(controllable, MAX_NAME), |w| clean(w, MAX_NAME));
        let name = clean(raw["label"].as_str().unwrap_or_default(), MAX_NAME);
        let id = DeviceId::new(instance, url);
        let device = Device {
            id: id.clone(),
            instance: instance.clone(),
            native_name: if name.is_empty() { model.clone() } else { name }.into(),
            manufacturer: Some("Somfy".into()),
            model: Some(model.into()),
            description: None,
            native_room: None,
            members: Vec::new(),
            points,
        };
        let values = decode(raw["states"].as_array().map_or(&[][..], Vec::as_slice));
        let online = raw["available"] != Json::Bool(false);
        out.devices.push(Found {
            device,
            online,
            values,
        });
        out.index.urls.insert(url.to_owned(), id.clone());
        out.index.devices.insert(
            id,
            Entry {
                url: url.to_owned(),
                gateway: gateway(url).to_owned(),
                commands,
            },
        );
    }
    out
}

/// States (`[{name, type, value}]`) as Moli values; unknown ones skipped.
#[must_use]
pub fn decode(states: &[Json]) -> Vec<(&'static str, Value)> {
    states
        .iter()
        .filter_map(|state| {
            let name = state["name"].as_str()?;
            let value = &state["value"];
            match name {
                ON_OFF => match value.as_str()? {
                    "on" => Some(("on", Value::Bool(true))),
                    "off" => Some(("on", Value::Bool(false))),
                    _ => None,
                },
                INTENSITY => {
                    // The cloud may send a number as text (`"42"`).
                    let level = value
                        .as_f64()
                        .or_else(|| value.as_str()?.trim().parse().ok())
                        .filter(|v: &f64| v.is_finite())?;
                    Some(("brightness", Value::Float(level.clamp(0.0, 100.0))))
                }
                n if OPENING_STATES.contains(&n) => {
                    let opening = value.as_str()?;
                    let known = OPENINGS
                        .iter()
                        .find(|o| **o == opening)
                        .unwrap_or(&"unknown");
                    Some(("opening", Value::from(*known)))
                }
                _ => None,
            }
        })
        .collect()
}

impl Index {
    #[must_use]
    pub fn device(&self, url: &str) -> Option<&DeviceId> {
        self.urls.get(url)
    }

    /// The devices behind one box.
    pub fn devices_on<'a>(&'a self, gateway: &'a str) -> impl Iterator<Item = &'a DeviceId> {
        self.devices
            .iter()
            .filter(move |(_, e)| e.gateway == gateway)
            .map(|(id, _)| id)
    }

    /// The order for a write already validated by the hub, if this device
    /// takes it (a move it does not know is refused, never sent).
    #[must_use]
    pub fn encode(&self, device: &DeviceId, key: &str, value: &Value) -> Option<Order> {
        let entry = self.devices.get(device)?;
        let takes = |c: &str| has(&entry.commands, c);
        let (command, parameters): (&'static str, Vec<Json>) = match (key, value) {
            ("control", Value::Text(m)) => {
                let command = MOVES.iter().copied().find(|c| *c == &**m && takes(c))?;
                (command, Vec::new())
            }
            ("on", Value::Bool(true)) if takes("on") => ("on", Vec::new()),
            ("on", Value::Bool(false)) if takes("off") => ("off", Vec::new()),
            ("brightness", v) if takes("setIntensity") => {
                let level = v
                    .as_f64()
                    .filter(|l| l.is_finite())?
                    .round()
                    .clamp(0.0, 100.0);
                // An integer: the box refuses `50.0`.
                #[allow(clippy::cast_possible_truncation)]
                let level = level as i64;
                ("setIntensity", vec![json!(level)])
            }
            _ => return None,
        };
        Some(Order {
            url: entry.url.clone(),
            command,
            parameters,
        })
    }
}

/// Where an execution is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Execution {
    Running,
    Completed,
    Failed,
}

/// One event of `/events/{listener}/fetch`, in what it means for Moli.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    States {
        url: String,
        values: Vec<(&'static str, Value)>,
    },
    Available {
        url: String,
        online: bool,
    },
    Execution {
        id: String,
        state: Execution,
        cause: Option<String>,
    },
    /// The box itself stopped answering its cloud.
    GatewayDown {
        gateway: String,
    },
    /// Devices added, removed or renamed, or the box back: load again.
    Reload,
}

/// The events a fetch returned; unknown ones skipped.
#[must_use]
pub fn events(answer: &Json) -> Vec<Event> {
    answer
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|event| {
            let url = || event["deviceURL"].as_str().map(str::to_owned);
            Some(match event["name"].as_str()? {
                "DeviceStateChangedEvent" => Event::States {
                    url: url()?,
                    values: decode(
                        event["deviceStates"]
                            .as_array()
                            .map_or(&[][..], Vec::as_slice),
                    ),
                },
                "DeviceAvailableEvent" => Event::Available {
                    url: url()?,
                    online: true,
                },
                "DeviceUnavailableEvent" | "DeviceDisabledEvent" => Event::Available {
                    url: url()?,
                    online: false,
                },
                "ExecutionStateChangedEvent" => Event::Execution {
                    id: event["execId"].as_str()?.to_owned(),
                    state: match event["newState"].as_str()? {
                        "COMPLETED" => Execution::Completed,
                        "FAILED" => Execution::Failed,
                        _ => Execution::Running,
                    },
                    cause: event["failureType"].as_str().map(|c| clean(c, MAX_CAUSE)),
                },
                "GatewayDownEvent" => Event::GatewayDown {
                    gateway: event["gatewayId"].as_str()?.to_owned(),
                },
                "DeviceCreatedEvent" | "DeviceRemovedEvent" | "DeviceUpdatedEvent"
                | "GatewayAliveEvent" => Event::Reload,
                _ => return None,
            })
        })
        .collect()
}

/// Why the box (or the cloud) refused a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The bearer this request carried is not (or no longer) valid: in the
    /// cloud, log in again; a local token is revoked. The request did not run.
    Session,
    /// The account itself is refused (credentials, ban, no access to the
    /// box): a human must act.
    Denied,
    /// The event listener expired: register another one.
    Listener,
    /// Quotas, a full queue, maintenance: slow down.
    Busy,
    Other,
}

/// Longest run of letters and digits kept in a message from the network: a
/// longer one may be a token, a key or a session id, never shown anywhere.
const MAX_WORD: usize = 16;

/// A message from the network, safe to show: bounded, cleaned, and every
/// long word (a token the box may quote back) replaced by `…`.
#[must_use]
pub fn redact(text: &str) -> String {
    let mut out = String::new();
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if word.chars().count() > MAX_WORD {
            out.push('…');
        } else {
            out.push_str(word);
        }
        word.clear();
    };
    for c in clean(text, MAX_CAUSE * 4).chars() {
        if c.is_alphanumeric() || "-_.:/+=".contains(c) {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    out.chars().take(MAX_CAUSE).collect()
}

/// What an Overkiz error answer (`{"errorCode", "error"}`) means, and what
/// may be said of it. An authentication refusal is named by its fixed code
/// only: its text may quote the token it refuses.
#[must_use]
pub fn refusal(status: u16, body: &[u8]) -> (Refusal, String) {
    let json: Json = serde_json::from_slice(body).unwrap_or_default();
    let code = json["errorCode"]
        .as_str()
        .filter(|c| {
            !c.is_empty() && c.len() <= 40 && c.bytes().all(|b| b.is_ascii_uppercase() || b == b'_')
        })
        .unwrap_or_default();
    let raw = json["error"].as_str().unwrap_or_default().to_lowercase();
    let says = |part: &str| raw.contains(part);
    let kind = if [
        "too many attempts",
        "bad credentials",
        "no such user",
        "unknown user",
        "access denied to gateway",
    ]
    .iter()
    .any(|p| says(p))
    {
        Refusal::Denied
    } else if [
        "not authenticated",
        "missing authorization token",
        "invalid token",
        "no such token",
        "not such token",
    ]
    .iter()
    .any(|p| says(p))
        || status == 401
    {
        Refusal::Session
    } else if says("event listener") {
        Refusal::Listener
    } else if code == "QUOTA_EXCEEDED"
        || code == "EXEC_QUEUE_FULL"
        || ["too many", "queue is full", "maintenance"]
            .iter()
            .any(|p| says(p))
        || status == 429
        || status >= 500
    {
        Refusal::Busy
    } else {
        Refusal::Other
    };
    let fixed = || {
        if code.is_empty() {
            format!("HTTP {status}")
        } else {
            code.to_owned()
        }
    };
    let message = match kind {
        Refusal::Session | Refusal::Denied => fixed(),
        _ => Some(redact(json["error"].as_str().unwrap_or_default()))
            .filter(|m| !m.is_empty())
            .unwrap_or_else(fixed),
    };
    (kind, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Json {
        let path = format!(
            "{}/../../integrations/overkiz/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    fn loaded() -> Load {
        load(&InstanceId::from("volets"), &fixture("setup-devices.json"))
    }

    fn device<'a>(load: &'a Load, model: &str) -> (&'a Device, bool, &'a [(&'static str, Value)]) {
        let found = load
            .devices
            .iter()
            .find(|f| f.device.model.as_deref() == Some(model))
            .unwrap();
        (&found.device, found.online, &found.values)
    }

    fn keys(d: &Device) -> Vec<&str> {
        d.points.iter().map(|p| &*p.key).collect()
    }

    fn moves(d: &Device) -> Vec<String> {
        match &d.point("control").unwrap().kind {
            Kind::Enum { values } => values.iter().map(ToString::to_string).collect(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn only_the_observed_classes_are_published() {
        let load = loaded();
        assert_eq!(
            load.devices.len(),
            6,
            "shutter ×2, RTS gate, OGP gate, light, outlet"
        );
        assert_eq!(load.skipped, ["internal:PodV3Component"]);
        // A disabled device is not published.
        assert!(
            load.devices
                .iter()
                .all(|f| &*f.device.native_name != "Ancien volet")
        );
    }

    #[test]
    fn an_rts_shutter_moves_but_never_claims_a_position() {
        let load = loaded();
        let (shutter, online, values) = device(&load, "UpDownRollerShutter");
        assert_eq!(shutter.id.as_str(), "volets:rts:__1234-5678-9012_16700001");
        assert_eq!(keys(shutter), ["control"]);
        assert_eq!(moves(shutter), ["open", "stop", "my", "close"]);
        let control = shutter.point("control").unwrap();
        assert!(control.access.write && !control.access.read);
        assert!(online);
        assert!(values.is_empty(), "no state from a one-way motor");
    }

    #[test]
    fn an_unavailable_device_is_published_offline() {
        let load = loaded();
        let offline: Vec<_> = load.devices.iter().filter(|f| !f.online).collect();
        assert_eq!(offline.len(), 1);
        assert_eq!(&*offline[0].device.native_name, "Volet 2");
    }

    #[test]
    fn an_rts_gate_offers_only_its_own_moves() {
        let load = loaded();
        let (gate, ..) = device(&load, "OpenCloseGate");
        assert_eq!(moves(gate), ["open", "stop", "close"]);
        // `identify` exists on the box, never in Moli.
        assert!(
            load.index
                .encode(&gate.id, "control", &Value::from("my"))
                .is_none()
        );
    }

    #[test]
    fn an_ogp_gate_says_where_it_is() {
        let load = loaded();
        let (gate, _, values) = device(&load, "DynamicGate");
        assert_eq!(keys(gate), ["control", "opening"]);
        assert_eq!(moves(gate), ["open", "stop", "my", "close"]);
        assert!(!gate.point("opening").unwrap().access.write);
        assert_eq!(values, &[("opening", Value::from("closed"))]);
    }

    #[test]
    fn a_light_and_an_outlet() {
        let load = loaded();
        let (light, _, values) = device(&load, "DynamicLight");
        assert_eq!(keys(light), ["on", "brightness"]);
        assert_eq!(light.point("on").unwrap().semantic, Semantic::OnOff);
        assert_eq!(
            values,
            &[
                ("on", Value::Bool(true)),
                ("brightness", Value::Float(60.0))
            ]
        );
        let (outlet, _, values) = device(&load, "DynamicOutlet");
        assert_eq!(keys(outlet), ["on"]);
        assert_eq!(values, &[("on", Value::Bool(false))]);
    }

    #[test]
    fn names_from_the_box_are_bounded_and_cleaned() {
        let raw = json!([{
            "deviceURL": "ogp://1234-5678-9012/00000009",
            "controllableName": "ogp:Outlet",
            "label": format!("Prise\u{0007}\n{}", "x".repeat(200)),
            "definition": { "commands": [{"commandName": "on"}, {"commandName": "off"}] },
        }]);
        let load = load(&InstanceId::from("v"), &raw);
        let name = &load.devices[0].device.native_name;
        assert!(name.starts_with("Prisex"));
        assert_eq!(name.chars().count(), MAX_NAME);
    }

    #[test]
    fn orders_are_what_the_box_expects() {
        let load = loaded();
        let (shutter, ..) = device(&load, "UpDownRollerShutter");
        let order = load
            .index
            .encode(&shutter.id, "control", &Value::from("my"))
            .unwrap();
        assert_eq!(
            order.body(),
            json!({"label": "Moli", "actions": [{
                "deviceURL": "rts://1234-5678-9012/16700001",
                "commands": [{"name": "my", "parameters": []}],
            }]})
        );
        let (light, ..) = device(&load, "DynamicLight");
        let dim = load
            .index
            .encode(&light.id, "brightness", &Value::Float(42.6))
            .unwrap();
        assert_eq!(
            (dim.command, dim.parameters),
            ("setIntensity", vec![json!(43)])
        );
        let off = load
            .index
            .encode(&light.id, "on", &Value::Bool(false))
            .unwrap();
        assert_eq!(off.command, "off");
        // Not a point of this device, or not a value it takes: nothing sent.
        assert!(
            load.index
                .encode(&light.id, "control", &Value::from("open"))
                .is_none()
        );
        assert!(
            load.index
                .encode(&shutter.id, "control", &Value::from("identify"))
                .is_none()
        );
        let stranger = DeviceId::new(&InstanceId::from("volets"), "rts://x/1");
        assert!(
            load.index
                .encode(&stranger, "control", &Value::from("open"))
                .is_none()
        );
    }

    #[test]
    fn events_are_understood() {
        let events = events(&fixture("events.json"));
        assert_eq!(
            events,
            [
                Event::Execution {
                    id: "11111111-2222-3333-4444-555555555555".into(),
                    state: Execution::Running,
                    cause: None,
                },
                Event::Execution {
                    id: "11111111-2222-3333-4444-555555555555".into(),
                    state: Execution::Completed,
                    cause: None,
                },
                Event::States {
                    url: "ogp://1234-5678-9012/00000003".into(),
                    values: vec![
                        ("on", Value::Bool(false)),
                        ("brightness", Value::Float(0.0))
                    ],
                },
                Event::Available {
                    url: "rts://1234-5678-9012/16700002".into(),
                    online: true,
                },
                Event::Execution {
                    id: "66666666-7777-8888-9999-000000000000".into(),
                    state: Execution::Failed,
                    cause: Some("NONEXEC_OTHER".into()),
                },
                Event::Reload,
            ]
        );
    }

    #[test]
    fn hostile_states_are_bounded() {
        let values = decode(&[
            json!({"name": INTENSITY, "type": 1, "value": 1e9}),
            json!({"name": "core:OpenClosedState", "type": 3, "value": "ajar"}),
            json!({"name": ON_OFF, "type": 3, "value": "maybe"}),
            json!({"name": INTENSITY, "type": 1, "value": "high"}),
        ]);
        assert_eq!(
            values,
            [
                ("brightness", Value::Float(100.0)),
                ("opening", Value::from("unknown")),
            ]
        );
    }

    #[test]
    fn refusals_are_sorted() {
        let r = |status, body: &str| refusal(status, body.as_bytes()).0;
        let body = |code: &str, error: &str| json!({"errorCode": code, "error": error}).to_string();
        assert_eq!(
            r(401, &body("RESOURCE_ACCESS_DENIED", "Not authenticated")),
            Refusal::Session
        );
        assert_eq!(
            r(400, &body("AUTHENTICATION_ERROR", "Bad credentials")),
            Refusal::Denied
        );
        assert_eq!(
            r(
                400,
                &body(
                    "UNSPECIFIED_ERROR",
                    "Too many attempts with an invalid token, user temporarily blocked"
                )
            ),
            Refusal::Denied
        );
        assert_eq!(
            r(400, &body("UNSPECIFIED_ERROR", "Invalid event listener id")),
            Refusal::Listener
        );
        assert_eq!(
            r(
                400,
                &body("UNSPECIFIED_ERROR", "No registered event listener")
            ),
            Refusal::Listener
        );
        assert_eq!(
            r(400, &body("RESOURCE_ACCESS_DENIED", "Too many executions")),
            Refusal::Busy
        );
        assert_eq!(
            r(
                400,
                &body("EXEC_QUEUE_FULL", "Execution queue is full on gateway")
            ),
            Refusal::Busy
        );
        assert_eq!(r(400, &body("QUOTA_EXCEEDED", "")), Refusal::Busy);
        assert_eq!(r(503, "<html>down for maintenance</html>"), Refusal::Busy);
        assert_eq!(
            r(400, &body("UNSUPPORTED_OPERATION", "No such command : foo")),
            Refusal::Other
        );
        let (_, message) = refusal(
            400,
            body("X", &format!("{}\u{1b}", "ab ".repeat(500))).as_bytes(),
        );
        assert_eq!(message.chars().count(), MAX_CAUSE);
    }

    /// The official refusals of a bearer quote it back (pyoverkiz fixtures
    /// `invalid-token.json`, `not-such-token.json`, HTTP 400).
    #[test]
    fn a_refused_token_is_never_repeated() {
        const MARK: &str = "synthetic-token-123";
        for (code, error) in [
            ("", "Invalid token : synthetic-token-123"),
            (
                "UNSPECIFIED_ERROR",
                "Not such token with UUID: synthetic-token-123",
            ),
            (
                "RESOURCE_ACCESS_DENIED",
                "Not authenticated synthetic-token-123",
            ),
            (
                "AUTHENTICATION_ERROR",
                "Bad credentials synthetic-token-123",
            ),
        ] {
            let body = json!({"errorCode": code, "error": error}).to_string();
            let (kind, message) = refusal(400, body.as_bytes());
            assert!(
                matches!(kind, Refusal::Session | Refusal::Denied),
                "{error}"
            );
            assert!(!message.contains(MARK), "{message}");
            let shown = crate::api::Refused { kind, message }.to_string();
            assert!(!shown.contains(MARK), "{shown}");
        }
        // The bearer refused is a session to renew, not a human's call…
        let session = |e: &str| refusal(400, json!({"error": e}).to_string().as_bytes());
        assert_eq!(session("Invalid token : x").0, Refusal::Session);
        assert_eq!(session("Not such token with UUID: x").0, Refusal::Session);
        assert_eq!(session("Invalid token : x").1, "HTTP 400");
        // … and any other message loses its long words.
        let (_, other) = refusal(
            400,
            json!({"errorCode": "X", "error": "No such command : 0123456789abcdef0123"})
                .to_string()
                .as_bytes(),
        );
        assert_eq!(other, "No such command : …");
    }

    #[test]
    fn the_cloud_sends_numbers_as_text_and_names_its_boxes() {
        let events = events(&fixture("events-cloud.json"));
        assert_eq!(
            events,
            [
                Event::States {
                    url: "ogp://1234-5678-9012/00000003".into(),
                    values: vec![
                        ("on", Value::Bool(true)),
                        ("brightness", Value::Float(42.0))
                    ],
                },
                Event::GatewayDown {
                    gateway: "1234-5678-9012".into(),
                },
                Event::Reload,
            ]
        );
        assert!(decode(&[json!({"name": INTENSITY, "type": 1, "value": "NaN"})]).is_empty());
        assert_eq!(
            decode(&[json!({"name": INTENSITY, "type": 1, "value": " 250 "})]),
            [("brightness", Value::Float(100.0))]
        );
    }

    #[test]
    fn devices_belong_to_their_box() {
        assert_eq!(gateway("rts://1234-5678-9012/16700001"), "1234-5678-9012");
        assert_eq!(gateway("internal://1234-5678-9012/pod/0"), "1234-5678-9012");
        assert_eq!(gateway("nonsense"), "");
        let load = loaded();
        assert_eq!(load.index.devices_on("1234-5678-9012").count(), 6);
        assert_eq!(load.index.devices_on("9999-9999-9999").count(), 0);
    }
}
