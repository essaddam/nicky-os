//! La mémoire des workflows, à part de celle des valeurs : le programme (compilé, figé, partagé),
//! les exécutions (l'état de chaque flux en cours, à la n8n), les attentes des déclencheurs.
//! La forme d'aujourd'hui est recopiée de moli-automation (model.rs, engine.rs).

use std::collections::HashMap;
use std::hint::black_box;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

use crate::maison::Maison;
use crate::outils::{Alea, ns, poids, resultat};
use crate::valeurs::{Etat, Value, valeur_aujourdhui};

// ---------------------------------------------------------------------------------------------
// Aujourd'hui : moli-automation/src/model.rs, à l'identique (sans l'empreinte).
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Automation {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub mode: Mode,
    pub graph: Graph,
    #[serde(default)]
    pub author: Author,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_version: Option<Version>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default)]
    pub created: u64,
    #[serde(default)]
    pub updated: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Version {
    pub mode: Mode,
    pub graph: Graph,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Restart,
    Single,
    Queued,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Author {
    #[default]
    Human,
    Assistant,
    Agent,
    Import,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Graph {
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub edges: Vec<Edge>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    #[serde(flatten)]
    pub step: Step,
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub from: String,
    #[serde(default = "out")]
    pub port: String,
    pub to: String,
}

fn out() -> String {
    "out".into()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Step {
    WhenOccurrence {
        point: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<Json>,
    },
    WhenState {
        point: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<Json>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<Json>,
        #[serde(default)]
        for_s: u64,
    },
    WhenThreshold {
        point: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        above: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        below: Option<f64>,
        #[serde(default)]
        for_s: u64,
    },
    AtTime {
        at: String,
        #[serde(default)]
        days: Vec<u8>,
    },
    AtSun {
        event: SunEvent,
        #[serde(default)]
        offset_min: i32,
        #[serde(default)]
        days: Vec<u8>,
    },
    Every {
        minutes: u32,
    },
    OnStart,
    Manual,
    If {
        rules: Vec<Rule>,
        #[serde(default = "yes")]
        all: bool,
    },
    Set {
        point: String,
        value: Json,
    },
    Toggle {
        point: String,
    },
    Scene {
        scene: String,
    },
    Wait {
        seconds: u64,
    },
    WaitFor {
        rule: Rule,
        timeout_s: u64,
    },
    Notify {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        message: String,
        #[serde(default = "home_channel")]
        channels: Vec<Channel>,
    },
    Write {
        prompt: String,
    },
    /// PROPOSÉ (la maquette des règles, V7 § 13.1), absent du modèle d'aujourd'hui : lire une
    /// valeur dans le registre de l'exécution (`{{lu}}` ensuite).
    ReadValue {
        point: String,
    },
    /// PROPOSÉ (mémoire v2, § 10.6), absent du modèle d'aujourd'hui : avant ou après le début (ou
    /// la fin, `end`) de chaque événement d'un agenda (« 10 min avant » : `offset_min` = -10).
    WhenCalendar {
        agenda: u32,
        #[serde(default)]
        end: bool,
        #[serde(default)]
        offset_min: i32,
    },
    /// PROPOSÉ (mémoire § 9.5), absent du modèle d'aujourd'hui : « quand une condition composée
    /// devient vraie », éventuellement depuis `for_s`.
    WhenRules {
        rules: Vec<Rule>,
        #[serde(default = "yes")]
        all: bool,
        #[serde(default)]
        for_s: u64,
    },
}

fn yes() -> bool {
    true
}
fn home_channel() -> Vec<Channel> {
    vec![Channel::Maison]
}

impl Step {
    pub fn is_trigger(&self) -> bool {
        matches!(
            self,
            Self::WhenOccurrence { .. }
                | Self::WhenState { .. }
                | Self::WhenThreshold { .. }
                | Self::AtTime { .. }
                | Self::AtSun { .. }
                | Self::Every { .. }
                | Self::OnStart
                | Self::Manual
                | Self::WhenRules { .. }
                | Self::WhenCalendar { .. }
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SunEvent {
    Rise,
    Set,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    Maison,
    Telegram,
    Voix,
    Telephone,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Rule {
    State {
        point: String,
        #[serde(default)]
        op: Op,
        value: Json,
    },
    Time {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before: Option<String>,
        #[serde(default)]
        days: Vec<u8>,
    },
    Sun {
        is: DayNight,
    },
    /// PROPOSÉ (mémoire § 8.4), absent du modèle d'aujourd'hui : une règle sur la cause de
    /// l'exécution (son point, ou sa valeur), gardée pour toute la suite.
    Cause {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        point: Option<String>,
        #[serde(default)]
        op: Op,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<Json>,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    #[default]
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DayNight {
    Day,
    Night,
}

pub fn minutes_of(at: &str) -> Option<u32> {
    let (h, m) = at.trim().split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

/// L'automatisme type de la maquette : quand, si (trois règles), régler, attendre, régler, prévenir.
pub fn automatisme_json(i: usize, m: &Maison, a: &mut Alea) -> Json {
    let nom = |n: u32| m.points[n as usize].nom.clone();
    let declencheur = nom(m.lus_binaires[a.sous(m.lus_binaires.len())]);
    let condition = nom(m.lus_mesures[a.sous(m.lus_mesures.len())]);
    let cible = nom(m.lus_binaires[a.sous(m.lus_binaires.len())]);
    let for_s = if i % 4 == 0 { 600 } else { 0 };
    let graph = json!({
        "nodes": [
            {"id": "declencheur", "type": "when_state", "point": declencheur, "to": true, "for_s": for_s, "x": 80.0, "y": 120.0},
            {"id": "condition", "type": "if", "all": true, "rules": [
                {"kind": "state", "point": condition, "op": "gt", "value": 20},
                {"kind": "time", "after": "22:00", "before": "06:00", "days": [1, 2, 3, 4, 5]},
                {"kind": "sun", "is": "night"}
            ], "x": 320.0, "y": 120.0},
            {"id": "allumer", "type": "set", "point": cible, "value": true, "x": 560.0, "y": 80.0},
            {"id": "attendre", "type": "wait", "seconds": 300, "x": 800.0, "y": 80.0},
            {"id": "eteindre", "type": "set", "point": cible, "value": false, "x": 1040.0, "y": 80.0},
            {"id": "prevenir", "type": "notify", "title": "Couloir",
             "message": format!("Lumière {i} éteinte après 5 minutes sans mouvement."), "channels": ["maison"],
             "x": 1280.0, "y": 80.0}
        ],
        "edges": [
            {"from": "declencheur", "to": "condition"},
            {"from": "condition", "port": "yes", "to": "allumer"},
            {"from": "allumer", "to": "attendre"},
            {"from": "attendre", "to": "eteindre"},
            {"from": "eteindre", "to": "prevenir"}
        ]
    });
    json!({
        "id": format!("couloir-{i}"), "name": format!("Couloir {i} : lumière au passage"),
        "enabled": true, "mode": "restart", "graph": graph, "author": "human",
        "approved": "4f1c2a9be07d3e5a8c61f0b2d9e4a7c3518b6e0f2a9d4c7b1e3f5a8c0d2b4e6f",
        "approved_version": {"mode": "restart", "graph": graph},
        "activation": "0b7d2e9a-41c6-4f3e-9a2d-5c8b1e7f0a34",
        "created": 1_792_000_000_000u64, "updated": 1_792_000_000_000u64
    })
}

/// `engine.rs`, `on_state` : pour chaque changement, tous les automatismes actifs, tous leurs
/// déclencheurs, une comparaison de chaînes.
pub fn sur_changement_aujourdhui(
    live: &[Automation],
    miroir: &mut HashMap<String, Value>,
    point: &str,
    value: &Value,
    sortie: &mut Vec<(u32, u32)>,
) {
    let key = point.to_string();
    let prev = miroir.insert(key.clone(), value.clone());
    if prev.as_ref() == Some(value) {
        return;
    }
    for (ai, a) in live.iter().enumerate() {
        for (ni, t) in a.graph.nodes.iter().enumerate().filter(|(_, n)| n.step.is_trigger()) {
            if let Step::WhenState { point: p, to, from, .. } = &t.step
                && *p == key
            {
                let to_ok = to.as_ref().is_none_or(|to| same(value, to));
                let from_ok = from.as_ref().is_none_or(|f| prev.as_ref().is_some_and(|p| same(p, f)));
                if prev.is_some() && to_ok && from_ok {
                    sortie.push((ai as u32, ni as u32));
                }
            }
        }
    }
}

fn same(v: &Value, j: &Json) -> bool {
    match (v, j) {
        (Value::Bool(a), Json::Bool(b)) => a == b,
        (Value::Int(a), Json::Number(n)) => n.as_i64() == Some(*a),
        (Value::Float(a), Json::Number(n)) => n.as_f64() == Some(*a),
        (Value::Text(a), Json::String(b)) => &**a == b,
        _ => false,
    }
}

// ---------------------------------------------------------------------------------------------
// Proposé : le programme, des tables de fiches de taille fixe, où le numéro est la place.
// ---------------------------------------------------------------------------------------------

pub mod op {
    pub const QUAND_ETAT: u8 = 1;
    pub const QUAND_SEUIL: u8 = 2;
    pub const QUAND_OCCURRENCE: u8 = 3;
    pub const HORAIRE: u8 = 4;
    pub const SI: u8 = 10;
    pub const REGLER: u8 = 11;
    pub const BASCULER: u8 = 12;
    pub const SCENE: u8 = 13;
    pub const ATTENDRE: u8 = 14;
    pub const ATTENDRE_QUE: u8 = 15;
    pub const PREVENIR: u8 = 16;
    pub const ECRIRE: u8 = 17;
}
pub mod port {
    pub const SORTIE: u8 = 0;
    pub const OUI: u8 = 1;
    pub const NON: u8 = 2;
    pub const FAIT: u8 = 3;
    pub const TROP_LONG: u8 = 4;
}
pub mod drapeau {
    pub const A_VERS: u8 = 1;
    pub const A_DEPUIS: u8 = 2;
    pub const TOUTES: u8 = 4;
}

/// Un nœud compilé : 32 octets, deux par ligne de cache.
#[derive(Clone, Copy, Default, Debug)]
#[repr(C)]
pub struct Noeud {
    pub op: u8,
    pub drapeaux: u8,
    pub nb_liens: u16,
    /// point (quand, régler), début des règles (si), texte (prévenir, écrire), scène.
    pub a: u32,
    /// durée en s (for_s, attendre, délai), nombre de règles (si), canaux (prévenir).
    pub b: u32,
    /// case d'attente (déclencheur à durée), règle (attendre que).
    pub c: u32,
    /// début de ses liens dans `liens`.
    pub liens: u32,
    pub _r: u32,
    /// valeur au canon : « vers », valeur à régler, seuil.
    pub v: i64,
}

#[derive(Clone, Copy, Default, Debug)]
#[repr(C)]
pub struct Lien {
    pub port: u8,
    pub _r: [u8; 3],
    pub cible: u32,
}

pub mod genre_regle {
    pub const ETAT: u8 = 1;
    pub const HEURE: u8 = 2;
    pub const SOLEIL: u8 = 3;
}

/// Une règle compilée : 16 octets.
#[derive(Clone, Copy, Default, Debug)]
#[repr(C)]
pub struct Regle {
    pub genre: u8,
    pub op: u8,
    pub jours: u8,
    pub _r: u8,
    pub point: u32,
    /// au canon ; pour une plage horaire, après << 16 | avant (minutes).
    pub valeur: i64,
}

#[derive(Clone, Copy, Default, Debug)]
#[repr(C)]
pub struct Plan {
    pub premier: u32,
    pub nb: u16,
    pub mode: u8,
    pub _r: u8,
    pub definition: u32,
}

pub struct Programme {
    pub plans: Vec<Plan>,
    pub noeuds: Vec<Noeud>,
    pub liens: Vec<Lien>,
    pub regles: Vec<Regle>,
    pub textes: String,
    pub fins: Vec<u32>,
    /// Par numéro de point : où commencent ses déclencheurs dans `index` (un de plus à la fin).
    pub index_debut: Vec<u32>,
    pub index: Vec<u32>,
    pub horaires: Vec<u32>,
    pub nb_attentes: u32,
}

/// Le canon d'une valeur attendue : un booléen vaut 0 ou 1 ; une mesure est mise à l'échelle
/// (21,57 °C → 2157), comme dans la fiche du point.
fn canon(j: &Json) -> i64 {
    match j {
        Json::Bool(b) => i64::from(*b),
        Json::Number(n) => n.as_i64().map_or_else(|| (n.as_f64().unwrap_or(0.0) * 100.0) as i64, |x| x * 100),
        _ => 0,
    }
}

/// Compile les automatismes actifs. Rebâtir tout le programme à chaque validation est le plus
/// simple : le banc dit combien cela coûte.
pub fn compiler(defs: &[Automation], etat: &Etat) -> Programme {
    let nb_points = etat.annuaire.len();
    let mut p = Programme {
        plans: Vec::with_capacity(defs.len()),
        noeuds: Vec::with_capacity(defs.len() * 6),
        liens: Vec::with_capacity(defs.len() * 5),
        regles: Vec::with_capacity(defs.len() * 3),
        textes: String::new(),
        fins: Vec::new(),
        index_debut: Vec::new(),
        index: Vec::new(),
        horaires: Vec::new(),
        nb_attentes: 0,
    };
    let mut declencheurs: Vec<(u32, u32)> = Vec::new(); // (point, nœud)
    let mut local: HashMap<&str, u32> = HashMap::new();
    let numero = |nom: &str| etat.annuaire.numero(nom).expect("point connu");
    for (di, a) in defs.iter().enumerate().filter(|(_, a)| a.enabled) {
        let premier = p.noeuds.len() as u32;
        local.clear();
        for (k, n) in a.graph.nodes.iter().enumerate() {
            local.insert(n.id.as_str(), premier + k as u32);
        }
        for n in &a.graph.nodes {
            let mut nd = Noeud::default();
            match &n.step {
                Step::WhenState { point, to, from, for_s } => {
                    nd.op = op::QUAND_ETAT;
                    nd.a = numero(point);
                    nd.b = *for_s as u32;
                    if let Some(to) = to {
                        nd.drapeaux |= drapeau::A_VERS;
                        nd.v = canon(to);
                    }
                    if from.is_some() {
                        nd.drapeaux |= drapeau::A_DEPUIS;
                    }
                    if *for_s > 0 {
                        nd.c = p.nb_attentes;
                        p.nb_attentes += 1;
                    }
                    declencheurs.push((nd.a, p.noeuds.len() as u32));
                }
                Step::WhenThreshold { point, above, .. } => {
                    nd.op = op::QUAND_SEUIL;
                    nd.a = numero(point);
                    nd.v = (above.unwrap_or(0.0) * 100.0) as i64;
                    declencheurs.push((nd.a, p.noeuds.len() as u32));
                }
                Step::If { rules, all } => {
                    nd.op = op::SI;
                    nd.a = p.regles.len() as u32;
                    nd.b = rules.len() as u32;
                    if *all {
                        nd.drapeaux |= drapeau::TOUTES;
                    }
                    for r in rules {
                        p.regles.push(match r {
                            Rule::State { point, op, value } => Regle {
                                genre: genre_regle::ETAT,
                                op: *op as u8,
                                point: numero(point),
                                valeur: canon(value),
                                ..Regle::default()
                            },
                            Rule::Time { after, before, days } => {
                                let apres = after.as_deref().and_then(minutes_of).unwrap_or(0);
                                let avant = before.as_deref().and_then(minutes_of).unwrap_or(1_440);
                                Regle {
                                    genre: genre_regle::HEURE,
                                    jours: days.iter().fold(0, |m, d| m | (1 << (d % 7))),
                                    valeur: i64::from(apres) << 16 | i64::from(avant),
                                    ..Regle::default()
                                }
                            }
                            Rule::Sun { is } => Regle {
                                genre: genre_regle::SOLEIL,
                                valeur: i64::from(*is == DayNight::Night),
                                ..Regle::default()
                            },
                            Rule::Cause { .. } => Regle::default(),
                        });
                    }
                }
                Step::Set { point, value } => {
                    nd.op = op::REGLER;
                    nd.a = numero(point);
                    nd.v = canon(value);
                }
                Step::Toggle { point } => {
                    nd.op = op::BASCULER;
                    nd.a = numero(point);
                }
                Step::Wait { seconds } => {
                    nd.op = op::ATTENDRE;
                    nd.b = *seconds as u32;
                }
                Step::Notify { message, .. } => {
                    nd.op = op::PREVENIR;
                    p.textes.push_str(message);
                    p.fins.push(p.textes.len() as u32);
                    nd.a = (p.fins.len() - 1) as u32;
                }
                Step::Write { prompt } => {
                    nd.op = op::ECRIRE;
                    p.textes.push_str(prompt);
                    p.fins.push(p.textes.len() as u32);
                    nd.a = (p.fins.len() - 1) as u32;
                }
                Step::WhenOccurrence { point, .. } => {
                    nd.op = op::QUAND_OCCURRENCE;
                    nd.a = numero(point);
                    declencheurs.push((nd.a, p.noeuds.len() as u32));
                }
                Step::AtTime { .. } | Step::AtSun { .. } | Step::Every { .. } | Step::OnStart | Step::Manual => {
                    nd.op = op::HORAIRE;
                    p.horaires.push(p.noeuds.len() as u32);
                }
                Step::Scene { .. } => nd.op = op::SCENE,
                Step::WhenRules { .. } | Step::ReadValue { .. } | Step::WhenCalendar { .. } => nd.op = op::HORAIRE,
                Step::WaitFor { timeout_s, .. } => {
                    nd.op = op::ATTENDRE_QUE;
                    nd.b = *timeout_s as u32;
                }
            }
            // Les liens du nœud, dans l'ordre validé.
            nd.liens = p.liens.len() as u32;
            for e in a.graph.edges.iter().filter(|e| e.from == n.id) {
                let port = match e.port.as_str() {
                    "yes" => port::OUI,
                    "no" => port::NON,
                    "ok" => port::FAIT,
                    "timeout" => port::TROP_LONG,
                    _ => port::SORTIE,
                };
                p.liens.push(Lien { port, cible: local[e.to.as_str()], ..Lien::default() });
            }
            nd.nb_liens = (p.liens.len() as u32 - nd.liens) as u16;
            p.noeuds.push(nd);
        }
        p.plans.push(Plan {
            premier,
            nb: a.graph.nodes.len() as u16,
            mode: a.mode as u8,
            definition: di as u32,
            ..Plan::default()
        });
    }
    // L'index par point, en deux passes (tri par dénombrement) : le numéro du point donne la place.
    let mut debut = vec![0u32; nb_points + 1];
    for &(pt, _) in &declencheurs {
        debut[pt as usize + 1] += 1;
    }
    for i in 0..nb_points {
        debut[i + 1] += debut[i];
    }
    let mut curseur = debut.clone();
    let mut index = vec![0u32; declencheurs.len()];
    for &(pt, nd) in &declencheurs {
        index[curseur[pt as usize] as usize] = nd;
        curseur[pt as usize] += 1;
    }
    p.index_debut = debut;
    p.index = index;
    p.plans.shrink_to_fit();
    p.noeuds.shrink_to_fit();
    p.liens.shrink_to_fit();
    p.regles.shrink_to_fit();
    p.textes.shrink_to_fit();
    p.fins.shrink_to_fit();
    p.horaires.shrink_to_fit();
    p
}

/// Une exécution : 32 octets. La case est réutilisée quand l'exécution finit ; la génération
/// écarte une issue tardive destinée à l'exécution d'avant.
#[derive(Clone, Copy, Default, Debug)]
#[repr(C)]
pub struct Execution {
    pub plan: u32,
    pub curseur: u32,
    pub reveil_s: u32,
    pub depuis_s: u32,
    pub demande: u32,
    pub generation: u16,
    pub etat: u8,
    pub version: u8,
    /// La valeur qui l'a lancée : la « donnée » du déclencheur, au sens de n8n.
    pub cause: i64,
}

#[derive(Default)]
pub struct Executions {
    pub cases: Vec<Execution>,
    pub libres: Vec<u32>,
    /// Les seules données de taille variable : le texte qu'un nœud « écrire » a produit.
    pub textes: HashMap<u32, Box<str>>,
}

impl Executions {
    pub fn lancer(&mut self, plan: u32, curseur: u32, cause: i64, maintenant: u32) -> (u32, u16) {
        let case = match self.libres.pop() {
            Some(c) => c,
            None => {
                self.cases.push(Execution::default());
                (self.cases.len() - 1) as u32
            }
        };
        let e = &mut self.cases[case as usize];
        let generation = e.generation.wrapping_add(1);
        *e = Execution { plan, curseur, reveil_s: u32::MAX, depuis_s: maintenant, generation, etat: 1, cause, ..Execution::default() };
        (case, generation)
    }
    pub fn finir(&mut self, case: u32) {
        self.cases[case as usize].etat = 0;
        self.cases[case as usize].reveil_s = u32::MAX;
        self.textes.remove(&case);
        self.libres.push(case);
    }
}

/// Les attentes des déclencheurs à durée (« ouvert depuis 10 min »), en colonnes : la case d'un
/// déclencheur est fixée par le compilateur (le numéro donne la place, encore), et le programme sait
/// quel nœud elle sert. 24 o (mémoire v2 ; la v1 en comptait 16 sans `parti` ni `point`).
pub struct Attentes {
    /// En secondes de l'horloge monotone.
    pub echeance: Vec<u32>,
    /// L'instant « changé » du point quand l'attente s'est armée : s'il a bougé à l'échéance, la
    /// valeur est partie entre-temps, et l'attente ne vaut plus rien. Aucun désarmement à tenir.
    pub depuis: Vec<u64>,
    /// Le dernier « depuis » parti : une vérité ne fait partir le déclencheur qu'une fois.
    pub parti: Vec<u64>,
    /// Le point (ou la feuille) dont la fiche donne la cause au départ : elle n'est pas recopiée.
    pub point: Vec<u32>,
}

impl Attentes {
    pub fn new(n: u32) -> Self {
        let n = n as usize;
        Self { echeance: vec![u32::MAX; n], depuis: vec![0; n], parti: vec![0; n], point: vec![0; n] }
    }
}

/// Un changement : seuls les déclencheurs du point se réveillent.
#[inline]
pub fn sur_changement(
    p: &Programme,
    etat: &Etat,
    n: u32,
    precedente: i64,
    valeur: i64,
    maintenant: u32,
    attentes: &mut Attentes,
    sortie: &mut Vec<u32>,
) {
    let (d, f) = (p.index_debut[n as usize] as usize, p.index_debut[n as usize + 1] as usize);
    for &t in &p.index[d..f] {
        let nd = &p.noeuds[t as usize];
        match nd.op {
            op::QUAND_ETAT => {
                let vers = nd.drapeaux & drapeau::A_VERS == 0 || nd.v == valeur;
                if vers && precedente != valeur {
                    if nd.b == 0 {
                        sortie.push(t);
                    } else {
                        let c = nd.c as usize;
                        attentes.echeance[c] = maintenant + nd.b;
                        attentes.depuis[c] = etat.vifs[n as usize].change_ms;
                        attentes.point[c] = n;
                    }
                }
            }
            op::QUAND_SEUIL => {
                if precedente <= nd.v && valeur > nd.v {
                    sortie.push(t);
                }
            }
            _ => {}
        }
    }
}

#[inline]
pub fn tenir(r: &Regle, etat: &Etat, minute: u32, jour: u8, nuit: bool) -> bool {
    match r.genre {
        genre_regle::ETAT => {
            let (v, q) = etat.lire(r.point);
            q != crate::valeurs::qualite::MAUVAISE
                && match r.op {
                    0 => v == r.valeur,
                    1 => v != r.valeur,
                    2 => v > r.valeur,
                    3 => v >= r.valeur,
                    4 => v < r.valeur,
                    _ => v <= r.valeur,
                }
        }
        genre_regle::HEURE => {
            let (apres, avant) = ((r.valeur >> 16) as u32, (r.valeur & 0xffff) as u32);
            let dans = if apres <= avant { minute >= apres && minute < avant } else { minute >= apres || minute < avant };
            dans && (r.jours == 0 || r.jours & (1 << jour) != 0)
        }
        genre_regle::SOLEIL => (r.valeur == 1) == nuit,
        _ => false,
    }
}

/// Fait avancer une exécution jusqu'à une attente ou la fin. Rend (nœud d'arrêt, attente en s).
pub fn avancer(p: &Programme, etat: &Etat, depart: u32, minute: u32, jour: u8, nuit: bool, ordres: &mut Vec<(u32, i64)>) -> (u32, u32) {
    let mut cur = depart;
    for _ in 0..100 {
        let nd = &p.noeuds[cur as usize];
        let port = match nd.op {
            op::SI => {
                let regles = &p.regles[nd.a as usize..(nd.a + nd.b) as usize];
                let oui = if nd.drapeaux & drapeau::TOUTES != 0 {
                    regles.iter().all(|r| tenir(r, etat, minute, jour, nuit))
                } else {
                    regles.iter().any(|r| tenir(r, etat, minute, jour, nuit))
                };
                if oui { port::OUI } else { port::NON }
            }
            op::REGLER => {
                ordres.push((nd.a, nd.v));
                port::SORTIE
            }
            op::ATTENDRE => {
                let suite = p.liens[nd.liens as usize..][..nd.nb_liens as usize].first().map_or(u32::MAX, |l| l.cible);
                return (suite, nd.b);
            }
            _ => port::SORTIE,
        };
        match p.liens[nd.liens as usize..][..nd.nb_liens as usize].iter().find(|l| l.port == port) {
            Some(l) => cur = l.cible,
            None => return (u32::MAX, 0),
        }
    }
    (u32::MAX, 0)
}

pub struct Banc {
    pub maison: Maison,
    pub etat: Etat,
    pub defs: Vec<Automation>,
    pub programme: Programme,
    pub octets_json: usize,
    pub o_defs: usize,
    pub o_prog: usize,
    pub ms_compiler: f64,
    pub ms_relire: f64,
}

#[derive(Deserialize)]
struct Fichier {
    automations: Vec<Automation>,
}

pub fn preparer(echelle: u32) -> Banc {
    let maison = crate::maison::generer(echelle);
    let etat = Etat::depuis(&maison);
    let mut a = Alea::new(41);
    let nb = 400 * echelle as usize;
    // Le fichier automations.json, écrit automatisme par automatisme (jamais tout en arbre).
    let mut fichier = String::from("{\"automations\":[");
    for i in 0..nb {
        if i > 0 {
            fichier.push(',');
        }
        fichier.push_str(&automatisme_json(i, &maison, &mut a).to_string());
    }
    fichier.push_str("]}");
    let octets_json = fichier.len();
    let (defs, o_defs) = poids(|| serde_json::from_str::<Fichier>(&fichier).unwrap().automations);
    let debut = Instant::now();
    let (programme, o_prog) = poids(|| compiler(&defs, &etat));
    let ms_compiler = debut.elapsed().as_secs_f64() * 1e3;
    // Relire le fichier, comme au démarrage : le temps seulement ; la copie est rendue aussitôt.
    let debut = Instant::now();
    let relu: Fichier = serde_json::from_str(&fichier).unwrap();
    let ms_relire = debut.elapsed().as_secs_f64() * 1e3;
    black_box(relu.automations.len());
    drop(relu);
    drop(fichier);
    Banc { maison, etat, defs, programme, octets_json, o_defs, o_prog, ms_compiler, ms_relire }
}

pub fn banc(echelle: u32) {
    let sec = "workflows";
    let b = preparer(echelle);
    let nb = b.defs.len();
    resultat(sec, echelle, "automatismes", nb as f64, "");
    resultat(sec, echelle, "noeuds", b.programme.noeuds.len() as f64, "");
    resultat(sec, echelle, "json_par_automatisme", (b.octets_json / nb) as f64, "o");
    resultat(sec, echelle, "memoire_definitions_aujourdhui", b.o_defs as f64, "o");
    // Le moteur garde en plus la liste des actifs (live) : une seconde copie des graphes.
    let (live, o_live) = poids(|| b.defs.iter().filter(|a| a.enabled).cloned().collect::<Vec<_>>());
    resultat(sec, echelle, "memoire_copie_live_aujourdhui", o_live as f64, "o");
    resultat(sec, echelle, "memoire_programme", b.o_prog as f64, "o");
    let p = &b.programme;
    use std::mem::size_of;
    let detail = [
        ("prog_noeuds", p.noeuds.capacity() * size_of::<Noeud>()),
        ("prog_liens", p.liens.capacity() * size_of::<Lien>()),
        ("prog_regles", p.regles.capacity() * size_of::<Regle>()),
        ("prog_plans", p.plans.capacity() * size_of::<Plan>()),
        ("prog_textes", p.textes.capacity() + p.fins.capacity() * 4),
        ("prog_index", (p.index_debut.capacity() + p.index.capacity()) * 4),
    ];
    for (cle, o) in detail {
        resultat(sec, echelle, cle, o as f64, "o");
    }
    resultat(sec, echelle, "taille_noeud", size_of::<Noeud>() as f64, "o");
    resultat(sec, echelle, "taille_regle", size_of::<Regle>() as f64, "o");
    resultat(sec, echelle, "taille_execution", size_of::<Execution>() as f64, "o");
    resultat(sec, echelle, "compiler_tout", b.ms_compiler, "ms");
    resultat(sec, echelle, "relire_le_fichier_json", b.ms_relire, "ms");

    // Un changement de valeur : aujourd'hui (tous les déclencheurs, chaînes) contre l'index.
    let mut miroir: HashMap<String, Value> = b
        .maison
        .points
        .iter()
        .enumerate()
        .map(|(n, pt)| (pt.nom.clone(), valeur_aujourdhui(n, pt.nature)))
        .collect();
    let mut a = Alea::new(3);
    const N: usize = 2_048;
    let quelconques: Vec<u32> = (0..N).map(|_| a.sous(b.maison.points.len()) as u32).collect();
    let declencheurs: Vec<u32> = (0..N).map(|_| b.maison.lus_binaires[a.sous(b.maison.lus_binaires.len())]).collect();
    let mut sortie = Vec::new();
    let mut bascule = false;
    for (nom, liste) in [("quelconque", &quelconques), ("declencheur", &declencheurs)] {
        let t = ns(N, 15, || {
            sortie.clear();
            for &n in liste.iter() {
                bascule = !bascule;
                sur_changement_aujourdhui(&live, &mut miroir, &b.maison.points[n as usize].nom, &Value::Bool(bascule), &mut sortie);
            }
            sortie.len() as u64
        });
        resultat(sec, echelle, &format!("evenement_{nom}_aujourdhui"), t, "ns");
        let mut attentes = Attentes::new(p.nb_attentes);
        let mut s2 = Vec::new();
        let t = ns(N, 101, || {
            s2.clear();
            for (k, &n) in liste.iter().enumerate() {
                let v = (k & 1) as i64;
                sur_changement(p, &b.etat, n, 1 - v, v, 1_000, &mut attentes, &mut s2);
            }
            s2.len() as u64
        });
        resultat(sec, echelle, &format!("evenement_{nom}_index"), t, "ns");
    }

    // Une exécution du départ jusqu'à l'attente de 5 min : un « si » de trois règles, un ordre.
    let mut ordres = Vec::new();
    let departs: Vec<u32> = p.plans.iter().map(|pl| pl.premier).collect();
    let t = ns(departs.len(), 101, || {
        ordres.clear();
        let mut x = 0u64;
        for &d in &departs {
            x += u64::from(avancer(p, &b.etat, d, 23 * 60, 2, true, &mut ordres).1);
        }
        x
    });
    resultat(sec, echelle, "executer_jusqu_a_l_attente", t, "ns");

    // Les exécutions en cours, au pire une par automatisme, et les attentes à durée.
    let (mut ex, o_ex) = poids(|| {
        let mut ex = Executions::default();
        for (k, pl) in p.plans.iter().enumerate() {
            let (c, _) = ex.lancer(k as u32, pl.premier + 3, 1, 1_000);
            ex.cases[c as usize].reveil_s = 1_300;
        }
        ex
    });
    resultat(sec, echelle, "memoire_executions", o_ex as f64, "o");
    let (att, o_att) = poids(|| Attentes::new(p.nb_attentes));
    resultat(sec, echelle, "memoire_attentes", o_att as f64, "o");
    resultat(sec, echelle, "attentes", f64::from(p.nb_attentes), "");
    black_box(&att.echeance);
    let t = ns(1, 201, || {
        let mut n = 0u64;
        for e in &ex.cases {
            n += u64::from(e.reveil_s <= 1_000);
        }
        n
    });
    resultat(sec, echelle, "passage_executions", t, "ns");
    let t = ns(p.plans.len(), 51, || {
        for k in 0..p.plans.len() as u32 {
            ex.finir(k);
        }
        for (k, pl) in p.plans.iter().enumerate() {
            ex.lancer(k as u32, pl.premier, 1, 1_000);
        }
        ex.cases.len() as u64
    });
    resultat(sec, echelle, "finir_et_relancer_une_execution", t, "ns");
}
