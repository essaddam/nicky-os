//! Le moteur de workflows proposé, en entier : la référence que le scénario vérifie
//! (docs/proposition/memoire.md, § 8 et § 9).
//!
//! - Le programme : des fiches de taille fixe rangées par numéro (plans, nœuds, liens, règles),
//!   un index par point pour les déclencheurs simples et les « attendre que », un réseau figé
//!   pour les déclencheurs composés. Il est rebâti entier à chaque validation.
//! - Les exécutions : des fiches de 64 o à génération, qui gardent leur cause pour toute la suite
//!   et n'avancent que vers l'avant. Le curseur est un rang dans l'automatisme : il survit à une
//!   recompilation.
//! - Les attentes des déclencheurs à durée : une case par déclencheur, revérifiée à l'échéance par
//!   l'instant « changé » (ou « vraie depuis » pour une condition composée), sans désarmement.
//! - La vérité des conditions composées : à part du programme, reconstruite d'un coup.
//! - L'échéancier : des colonnes d'échéances, et le réveil au plus proche.
//!
//! v2 (docs/proposition/memoire-v2.md) :
//! - deux horloges : la monotone pour toutes les durées (attentes, exécutions, santé, fraîcheur),
//!   la murale pour le calendrier et ce qu'on montre ; un instantané range les échéances en heure
//!   murale, le redémarrage les reconvertit (l'horloge monotone repart de zéro) ;
//! - une attente de déclencheur ne copie plus la cause (24 o au lieu de 52) : elle la relit dans la
//!   fiche du point, que la revérification par l'instant « changé » garantit inchangée ;
//! - une valeur trop vieille (fraîcheur exigée) devient mauvaise à son échéance, et ses feuilles
//!   sont relues ;
//! - les déclencheurs horaires (heure, soleil, période) sont des fiches du moteur du temps
//!   (calendrier.rs), rattrapées ou manquées selon leur retard.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use serde_json::Value as Json;

use crate::calendrier::{self, Calendrier, Fiche as FicheCal, Sortie};
use crate::decisions::Execution64;
use crate::outils::Temps;
use crate::valeurs::{Etat, qualite, vie};
use crate::workflows::{Automation, DayNight, Mode, Op, Rule, Step, SunEvent, minutes_of};

pub mod op {
    pub const QUAND_ETAT: u8 = 1;
    pub const QUAND_COMPOSE: u8 = 2;
    pub const AUTRE_DECLENCHEUR: u8 = 3;
    /// Heure, soleil, période : une fiche du moteur du temps.
    pub const HORAIRE: u8 = 4;
    pub const SI: u8 = 10;
    pub const REGLER: u8 = 11;
    pub const LIRE: u8 = 12;
    pub const ATTENDRE: u8 = 14;
    pub const ATTENDRE_QUE: u8 = 15;
    pub const PREVENIR: u8 = 16;
    pub const AUTRE: u8 = 20;
}
pub mod port {
    pub const SORTIE: u8 = 0;
    pub const OUI: u8 = 1;
    pub const NON: u8 = 2;
    pub const FAIT: u8 = 3;
    pub const TROP_LONG: u8 = 4;
}
mod drapeau {
    pub const A_VERS: u8 = 1;
    pub const A_DEPUIS: u8 = 2;
    pub const TOUTES: u8 = 4;
    /// « régler le point qui a déclenché » : la cible est la cause.
    pub const CIBLE_CAUSE: u8 = 8;
}
mod genre {
    pub const ETAT: u8 = 1;
    pub const CAUSE_POINT: u8 = 4;
    pub const CAUSE_VALEUR: u8 = 5;
}
pub mod etat_ex {
    pub const LIBRE: u8 = 0;
    pub const PRETE: u8 = 1;
    pub const DUREE: u8 = 2;
    pub const ATTEND_QUE: u8 = 3;
    pub const ISSUE: u8 = 4;
}

/// Un nœud : 32 o.
#[derive(Clone, Copy, Default, Debug)]
#[repr(C)]
pub struct Noeud {
    pub op: u8,
    pub drapeaux: u8,
    pub nb_liens: u16,
    pub a: u32,
    pub b: u32,
    pub c: u32,
    pub liens: u32,
    /// « depuis » d'un déclencheur d'état (la valeur d'avant attendue).
    pub w: u32,
    pub v: i64,
}

/// Un lien : 8 o. La cible est un rang dans l'automatisme, pas une place globale.
#[derive(Clone, Copy, Default, Debug)]
#[repr(C)]
pub struct Lien {
    pub port: u8,
    pub _r: [u8; 3],
    pub cible: u32,
}

/// Une règle : 16 o.
#[derive(Clone, Copy, Default, Debug)]
#[repr(C)]
pub struct Regle {
    pub genre: u8,
    pub op: u8,
    pub _r: u16,
    pub point: u32,
    pub valeur: i64,
}

/// Un automatisme compilé, rangé à son numéro (fixe, jamais redonné).
#[derive(Clone, Copy, Default, Debug)]
#[repr(C)]
pub struct Plan {
    pub premier: u32,
    pub nb: u16,
    pub mode: u8,
    pub actif: u8,
}

/// Le programme : figé, partagé, rebâti entier à chaque validation.
#[derive(Default)]
pub struct Programme {
    pub plans: Vec<Plan>,
    pub empreintes: Vec<u64>,
    pub noeuds: Vec<Noeud>,
    pub noeud_auto: Vec<u32>,
    /// Le nom de chaque nœud, pour l'écran et le journal.
    pub ids: Vec<Box<str>>,
    pub liens: Vec<Lien>,
    pub regles: Vec<Regle>,
    pub textes: Vec<Box<str>>,
    pub noms: Vec<Box<str>>,
    /// Déclencheurs simples : point → nœuds (places globales).
    pub index_debut: Vec<u32>,
    pub index: Vec<u32>,
    /// « Attendre que » : point → nœuds.
    pub attente_debut: Vec<u32>,
    pub attente_index: Vec<u32>,
    /// Le réseau des déclencheurs composés, figé : feuilles, conditions, index par point.
    pub f_point: Vec<u32>,
    pub f_op: Vec<u8>,
    pub f_valeur: Vec<i64>,
    pub f_cond: Vec<u32>,
    pub c_premiere: Vec<u32>,
    pub c_nb: Vec<u16>,
    pub c_tous: Vec<bool>,
    pub c_noeud: Vec<u32>,
    pub f_debut: Vec<u32>,
    pub f_index: Vec<u32>,
    /// Cases d'attente : une par déclencheur à durée → (automatisme, rang du déclencheur).
    pub attente_de: Vec<(u32, u32)>,
    /// Les déclencheurs horaires : une fiche calendaire chacun → (automatisme, rang du déclencheur).
    pub fiches_cal: Vec<FicheCal>,
    pub horaires: Vec<(u32, u32)>,
}

impl Programme {
    /// Les chaînes : noms des nœuds et des automatismes, gabarits des messages.
    pub fn octets_textes(&self) -> usize {
        self.textes.iter().chain(self.noms.iter()).chain(self.ids.iter()).map(|t| t.len() + 16).sum()
    }

    pub fn octets(&self) -> usize {
        use std::mem::size_of;
        self.plans.capacity() * size_of::<Plan>()
            + self.empreintes.capacity() * 8
            + self.noeuds.capacity() * size_of::<Noeud>()
            + self.noeud_auto.capacity() * 4
            + self.ids.iter().map(|t| t.len() + 16).sum::<usize>()
            + self.liens.capacity() * size_of::<Lien>()
            + self.regles.capacity() * size_of::<Regle>()
            + self.textes.iter().map(|t| t.len() + 16).sum::<usize>()
            + self.noms.iter().map(|t| t.len() + 16).sum::<usize>()
            + (self.index_debut.capacity() + self.index.capacity() + self.attente_debut.capacity() + self.attente_index.capacity()) * 4
            + (self.f_point.capacity() + self.f_cond.capacity() + self.c_premiere.capacity() + self.c_noeud.capacity()) * 4
            + self.f_op.capacity() + self.f_valeur.capacity() * 8 + self.c_nb.capacity() * 2 + self.c_tous.capacity()
            + (self.f_debut.capacity() + self.f_index.capacity()) * 4
            + self.attente_de.capacity() * 8
            + self.fiches_cal.capacity() * std::mem::size_of::<FicheCal>()
            + self.horaires.capacity() * 8
    }
}

/// Le nom du point virtuel d'une plage horaire, tenu par le hub (mémoire § 9.4).
pub fn plage_nom(apres: u32, avant: u32, jours: u8) -> String {
    format!("hub:horloge/plage_{apres}_{avant}_{jours}")
}
pub const NUIT: &str = "hub:horloge/nuit";

fn canon(j: &Json, echelle: i8) -> i64 {
    match j {
        Json::Bool(b) => i64::from(*b),
        Json::Number(n) => {
            let x = n.as_f64().unwrap_or(0.0) * 10f64.powi(-i32::from(echelle));
            x.round() as i64
        }
        _ => 0,
    }
}

fn comparer(v: i64, op: u8, r: i64) -> bool {
    match op {
        0 => v == r,
        1 => v != r,
        2 => v > r,
        3 => v >= r,
        4 => v < r,
        _ => v <= r,
    }
}

fn empreinte(a: &Automation) -> u64 {
    let nodes: Vec<Json> = a.graph.nodes.iter().map(|n| serde_json::json!({ "id": n.id, "step": n.step })).collect();
    let canonique = serde_json::json!({ "mode": a.mode, "nodes": nodes, "edges": a.graph.edges, "enabled": a.enabled });
    let mut h = DefaultHasher::new();
    canonique.to_string().hash(&mut h);
    h.finish()
}

/// Les plages horaires que demandent les automatismes : le hub en fait des points.
pub fn plages_requises(defs: &[Automation]) -> Vec<(u32, u32, u8)> {
    let mut v = Vec::new();
    let mut voir = |r: &Rule| {
        if let Rule::Time { after, before, days } = r {
            let apres = after.as_deref().and_then(minutes_of).unwrap_or(0);
            let avant = before.as_deref().and_then(minutes_of).unwrap_or(1_440);
            let jours = days.iter().fold(0u8, |m, d| m | (1 << (d % 7)));
            if !v.contains(&(apres, avant, jours)) {
                v.push((apres, avant, jours));
            }
        }
    };
    for a in defs {
        for n in &a.graph.nodes {
            match &n.step {
                Step::If { rules, .. } | Step::WhenRules { rules, .. } => rules.iter().for_each(&mut voir),
                Step::WaitFor { rule, .. } => voir(rule),
                _ => {}
            }
        }
    }
    v
}

/// Les jours du modèle (lundi = 1 … dimanche = 7) en masque : lundi = bit 1 … dimanche = bit 0.
pub fn masque(days: &[u8]) -> u8 {
    days.iter().fold(0u8, |m, d| m | (1 << (d % 7)))
}

/// « HH:MM » ou « HH:MM:SS » → seconde du jour.
pub fn secondes_de(at: &str) -> Option<u32> {
    let mut x = at.trim().split(':');
    let (h, m) = (x.next()?.parse::<u32>().ok()?, x.next()?.parse::<u32>().ok()?);
    let s = x.next().map_or(Some(0), |s| s.parse::<u32>().ok())?;
    (h < 24 && m < 60 && s < 60 && x.next().is_none()).then_some(h * 3_600 + m * 60 + s)
}

pub fn compiler(defs: &[Automation], etat: &Etat) -> Programme {
    let numero = |nom: &str| etat.annuaire.numero(nom).unwrap_or_else(|| panic!("point inconnu : {nom}"));
    let regle = |r: &Rule| -> Regle {
        match r {
            Rule::State { point, op, value } => {
                let n = numero(point);
                Regle { genre: genre::ETAT, op: *op as u8, point: n, valeur: canon(value, etat.defs[n as usize].echelle), ..Regle::default() }
            }
            Rule::Time { after, before, days } => {
                let apres = after.as_deref().and_then(minutes_of).unwrap_or(0);
                let avant = before.as_deref().and_then(minutes_of).unwrap_or(1_440);
                Regle { genre: genre::ETAT, op: 0, point: numero(&plage_nom(apres, avant, masque(days))), valeur: 1, ..Regle::default() }
            }
            Rule::Sun { is } => Regle { genre: genre::ETAT, op: 0, point: numero(NUIT), valeur: i64::from(*is == DayNight::Night), ..Regle::default() },
            Rule::Cause { point: Some(p), .. } => Regle { genre: genre::CAUSE_POINT, point: numero(p), ..Regle::default() },
            Rule::Cause { point: None, op, value } => Regle {
                genre: genre::CAUSE_VALEUR,
                op: *op as u8,
                valeur: value.as_ref().map_or(0, |v| canon(v, 0)),
                ..Regle::default()
            },
        }
    };
    let mut p = Programme::default();
    let mut simples: Vec<(u32, u32)> = Vec::new();
    let mut attendre_que: Vec<(u32, u32)> = Vec::new();
    for (ai, a) in defs.iter().enumerate() {
        let premier = p.noeuds.len() as u32;
        p.empreintes.push(empreinte(a));
        p.noms.push(a.name.clone().into());
        let mode = match a.mode {
            Mode::Restart => 0,
            Mode::Single => 1,
            Mode::Queued => 2,
        };
        if !a.enabled {
            p.plans.push(Plan { premier, nb: 0, mode, actif: 0 });
            continue;
        }
        let rang: HashMap<&str, u32> = a.graph.nodes.iter().enumerate().map(|(k, n)| (n.id.as_str(), k as u32)).collect();
        for n in &a.graph.nodes {
            let g = p.noeuds.len() as u32;
            let mut nd = Noeud::default();
            match &n.step {
                Step::WhenState { point, to, from, for_s } => {
                    nd.op = op::QUAND_ETAT;
                    nd.a = numero(point);
                    nd.b = *for_s as u32;
                    if let Some(to) = to {
                        nd.drapeaux |= drapeau::A_VERS;
                        nd.v = canon(to, etat.defs[nd.a as usize].echelle);
                    }
                    if let Some(from) = from {
                        nd.drapeaux |= drapeau::A_DEPUIS;
                        nd.w = canon(from, etat.defs[nd.a as usize].echelle) as u32;
                    }
                    if *for_s > 0 {
                        nd.c = p.attente_de.len() as u32;
                        p.attente_de.push((ai as u32, g - premier));
                    }
                    simples.push((nd.a, g));
                }
                Step::WhenRules { rules, all, for_s } => {
                    nd.op = op::QUAND_COMPOSE;
                    let c = p.c_nb.len() as u32;
                    nd.a = c;
                    nd.b = *for_s as u32;
                    p.c_premiere.push(p.f_point.len() as u32);
                    p.c_nb.push(rules.len() as u16);
                    p.c_tous.push(*all);
                    p.c_noeud.push(g);
                    for r in rules {
                        let rg = regle(r);
                        assert_eq!(rg.genre, genre::ETAT, "une condition composée ne porte que sur des valeurs");
                        p.f_point.push(rg.point);
                        p.f_op.push(rg.op);
                        p.f_valeur.push(rg.valeur);
                        p.f_cond.push(c);
                    }
                    if *for_s > 0 {
                        nd.c = p.attente_de.len() as u32;
                        p.attente_de.push((ai as u32, g - premier));
                    }
                }
                Step::If { rules, all } => {
                    nd.op = op::SI;
                    nd.a = p.regles.len() as u32;
                    nd.b = rules.len() as u32;
                    if *all {
                        nd.drapeaux |= drapeau::TOUTES;
                    }
                    for r in rules {
                        p.regles.push(regle(r));
                    }
                }
                Step::Set { point, value } => {
                    nd.op = op::REGLER;
                    if point == "@declencheur" {
                        nd.drapeaux |= drapeau::CIBLE_CAUSE;
                        nd.v = canon(value, 0);
                    } else {
                        nd.a = numero(point);
                        nd.v = canon(value, etat.defs[nd.a as usize].echelle);
                    }
                }
                Step::Wait { seconds } => {
                    nd.op = op::ATTENDRE;
                    nd.b = *seconds as u32;
                }
                Step::ReadValue { point } => {
                    nd.op = op::LIRE;
                    nd.a = numero(point);
                }
                Step::WaitFor { rule, timeout_s } => {
                    nd.op = op::ATTENDRE_QUE;
                    nd.a = p.regles.len() as u32;
                    nd.b = *timeout_s as u32;
                    let rg = regle(rule);
                    if rg.genre == genre::ETAT {
                        attendre_que.push((rg.point, g));
                    }
                    p.regles.push(rg);
                }
                Step::Notify { message, .. } => {
                    nd.op = op::PREVENIR;
                    nd.a = p.textes.len() as u32;
                    p.textes.push(message.clone().into());
                }
                Step::AtTime { at, days } => {
                    nd.op = op::HORAIRE;
                    nd.c = p.horaires.len() as u32;
                    p.fiches_cal.push(FicheCal { genre: calendrier::genre::HEURE, jours: masque(days), a: secondes_de(at).unwrap_or(0), ..FicheCal::default() });
                    p.horaires.push((ai as u32, g - premier));
                }
                Step::AtSun { event, offset_min, days } => {
                    nd.op = op::HORAIRE;
                    nd.c = p.horaires.len() as u32;
                    p.fiches_cal.push(FicheCal {
                        genre: calendrier::genre::SOLEIL,
                        jours: masque(days),
                        coucher: u8::from(*event == SunEvent::Set),
                        decalage_s: offset_min * 60,
                        ..FicheCal::default()
                    });
                    p.horaires.push((ai as u32, g - premier));
                }
                Step::WhenCalendar { agenda, end, offset_min } => {
                    nd.op = op::HORAIRE;
                    nd.c = p.horaires.len() as u32;
                    p.fiches_cal.push(FicheCal {
                        genre: calendrier::genre::AGENDA,
                        a: *agenda,
                        coucher: u8::from(*end),
                        decalage_s: offset_min * 60,
                        ..FicheCal::default()
                    });
                    p.horaires.push((ai as u32, g - premier));
                }
                Step::Every { minutes } => {
                    nd.op = op::HORAIRE;
                    nd.c = p.horaires.len() as u32;
                    p.fiches_cal.push(FicheCal { genre: calendrier::genre::PERIODE, a: minutes * 60, ..FicheCal::default() });
                    p.horaires.push((ai as u32, g - premier));
                }
                s if s.is_trigger() => nd.op = op::AUTRE_DECLENCHEUR,
                _ => nd.op = op::AUTRE,
            }
            nd.liens = p.liens.len() as u32;
            for e in a.graph.edges.iter().filter(|e| e.from == n.id) {
                let port = match e.port.as_str() {
                    "yes" => port::OUI,
                    "no" => port::NON,
                    "ok" => port::FAIT,
                    "timeout" => port::TROP_LONG,
                    _ => port::SORTIE,
                };
                p.liens.push(Lien { port, cible: rang[e.to.as_str()], ..Lien::default() });
            }
            nd.nb_liens = (p.liens.len() as u32 - nd.liens) as u16;
            p.noeuds.push(nd);
            p.noeud_auto.push(ai as u32);
            p.ids.push(n.id.clone().into());
        }
        p.plans.push(Plan { premier, nb: (p.noeuds.len() as u32 - premier) as u16, mode, actif: 1 });
    }
    let nb_points = etat.annuaire.len();
    let csr = |paires: &[(u32, u32)]| {
        let mut debut = vec![0u32; nb_points + 1];
        for &(pt, _) in paires {
            debut[pt as usize + 1] += 1;
        }
        for i in 0..nb_points {
            debut[i + 1] += debut[i];
        }
        let mut cur = debut.clone();
        let mut index = vec![0u32; paires.len()];
        for &(pt, x) in paires {
            index[cur[pt as usize] as usize] = x;
            cur[pt as usize] += 1;
        }
        (debut, index)
    };
    (p.index_debut, p.index) = csr(&simples);
    (p.attente_debut, p.attente_index) = csr(&attendre_que);
    let feuilles: Vec<(u32, u32)> = p.f_point.iter().enumerate().map(|(f, &pt)| (pt, f as u32)).collect();
    (p.f_debut, p.f_index) = csr(&feuilles);
    p
}

/// L'état qui a lancé une exécution. Pour un déclencheur horaire, `point` vaut `HORLOGE`.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Cause {
    pub point: u32,
    pub declencheur: u32,
    pub instant_ms: u64,
    pub valeur: i64,
    pub precedente: i64,
}

/// La cause d'un déclencheur horaire : l'horloge, pas un point.
pub const HORLOGE: u32 = u32::MAX;
/// La cause d'une occurrence d'agenda : ce bit, plus le numéro de l'agenda ; la valeur porte le
/// numéro de l'événement.
pub const AGENDA: u32 = 0x8000_0000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ordre {
    /// Heure murale (s), pour l'écran et le journal.
    pub t: u32,
    pub automatisme: u32,
    pub point: u32,
    pub valeur: i64,
    pub case: u32,
    pub generation: u16,
    pub demande: u32,
}

/// Où est la maison : le fuseau et la position pour le soleil.
#[derive(Clone, Debug)]
pub struct Lieu {
    pub fuseau: String,
    pub lat: f64,
    pub lon: f64,
}

/// Ce qui est rangé pour survivre à un redémarrage : les fiches, rien d'autre (le programme se
/// recompile, la vérité des conditions se reconstruit). Les échéances y sont en heure murale (s) :
/// l'horloge monotone repart de zéro au démarrage.
#[derive(Clone)]
pub struct Instantane {
    pub ex: Vec<Execution64>,
    pub libres: Vec<u32>,
    pub en_cours: Vec<u32>,
    pub at_echeance: Vec<u32>,
    pub at_depuis: Vec<u64>,
    pub at_parti: Vec<u64>,
    pub at_point: Vec<u32>,
    pub c_parti: Vec<u64>,
    pub cal_derniere: Vec<u64>,
    pub cal_prochaine: Vec<u64>,
    pub cal_derniere_evt: Vec<u32>,
    pub cal_prochaine_evt: Vec<u32>,
    /// Les événements des agendas : de la mémoire du hub, rangée avec le reste (24 o chacun).
    pub cal_evenements: Vec<calendrier::Evenement>,
    pub prochaine_demande: u32,
}

pub struct Moteur {
    pub prog: Programme,
    pub ex: Vec<Execution64>,
    pub libres: Vec<u32>,
    pub en_cours: Vec<u32>,
    // Les attentes des déclencheurs à durée, en colonnes, par case : 24 o chacune.
    /// L'échéance, en secondes monotones.
    pub at_echeance: Vec<u32>,
    /// L'instant « changé » (ou « vraie depuis ») quand l'attente s'est armée : sa version.
    pub at_depuis: Vec<u64>,
    /// La version pour laquelle le déclencheur est déjà parti : jamais deux fois pour la même.
    pub at_parti: Vec<u64>,
    /// Le point dont la fiche donne la cause au départ (la feuille, pour une condition composée).
    pub at_point: Vec<u32>,
    // La vérité des conditions composées, à part de leur structure.
    pub f_vrai: Vec<bool>,
    pub c_vraies: Vec<u16>,
    pub c_vrai: Vec<bool>,
    pub c_parti: Vec<u64>,
    /// Les déclencheurs horaires : le moteur du temps, ses fiches dans l'ordre de `prog.horaires`.
    pub cal: Calendrier,
    pub lieu: Lieu,
    pub prochaine_demande: u32,
    pub ordres: Vec<Ordre>,
    pub notices: Vec<(u32, u32, String)>,
    pub journal: Vec<(u32, String)>,
    /// Les nœuds par lesquels passe chaque exécution : (automatisme, case, génération, nœud).
    pub chemins: Vec<(u32, u32, u16, String)>,
}

fn calendrier_de(prog: &Programme, lieu: &Lieu) -> Calendrier {
    let mut c = Calendrier::new(&lieu.fuseau, lieu.lat, lieu.lon);
    for f in &prog.fiches_cal {
        c.ajouter(*f);
    }
    c
}

/// Une échéance murale (ms), en secondes monotones, d'après l'heure présente.
fn vers_mono(t: Temps, mur_ms: u64) -> u32 {
    if mur_ms == u64::MAX {
        return u32::MAX;
    }
    t.mono.saturating_add(mur_ms.saturating_sub(t.mur_ms).div_ceil(1_000) as u32)
}

impl Moteur {
    pub fn new(prog: Programme, etat: &Etat, lieu: &Lieu) -> Self {
        let nb_auto = prog.plans.len();
        let nb_at = prog.attente_de.len();
        let nb_c = prog.c_nb.len();
        let cal = calendrier_de(&prog, lieu);
        let mut m = Moteur {
            ex: Vec::new(),
            libres: Vec::new(),
            en_cours: vec![u32::MAX; nb_auto],
            at_echeance: vec![u32::MAX; nb_at],
            at_depuis: vec![0; nb_at],
            at_parti: vec![u64::MAX; nb_at],
            at_point: vec![0; nb_at],
            f_vrai: vec![false; prog.f_point.len()],
            c_vraies: vec![0; nb_c],
            c_vrai: vec![false; nb_c],
            c_parti: vec![u64::MAX; nb_c],
            cal,
            lieu: lieu.clone(),
            prochaine_demande: 1,
            ordres: Vec::new(),
            notices: Vec::new(),
            journal: Vec::new(),
            chemins: Vec::new(),
            prog,
        };
        m.reconstruire(etat);
        m
    }

    /// Arme les déclencheurs horaires (au démarrage, quand l'heure devient sûre).
    pub fn armer(&mut self, etat: &Etat, t: Temps) {
        let valeur = |p: u32| etat.vifs.get(p as usize).map(|v| v.valeur);
        self.cal.armer_tout(t.mur_ms, &valeur);
    }

    fn noter(&mut self, t: u32, s: String) {
        self.journal.push((t, s));
    }

    fn nom_auto(&self, a: u32) -> &str {
        &self.prog.noms[a as usize]
    }

    /// La vérité des conditions composées, recalculée d'un coup depuis la mémoire des valeurs.
    pub fn reconstruire(&mut self, etat: &Etat) {
        let p = &self.prog;
        self.f_vrai = (0..p.f_point.len()).map(|f| feuille_vraie(p, etat, f)).collect();
        self.c_vraies = vec![0; p.c_nb.len()];
        for (f, &v) in self.f_vrai.iter().enumerate() {
            if v {
                self.c_vraies[p.f_cond[f] as usize] += 1;
            }
        }
        self.c_vrai = (0..p.c_nb.len()).map(|c| cond_vraie(p, &self.c_vraies, c)).collect();
    }

    /// Depuis quand une condition composée est vraie : pour un « et », le plus grand des instants
    /// « changé » de ses feuilles ; pour un « ou », le plus petit parmi ses feuilles vraies.
    pub fn depuis(&self, etat: &Etat, c: u32) -> u64 {
        let p = &self.prog;
        let d = p.c_premiere[c as usize] as usize;
        let n = p.c_nb[c as usize] as usize;
        let changes = (d..d + n).filter(|&f| p.c_tous[c as usize] || self.f_vrai[f]).map(|f| etat.vifs[p.f_point[f] as usize].change_ms);
        if p.c_tous[c as usize] { changes.max().unwrap_or(0) } else { changes.min().unwrap_or(0) }
    }

    /// Relit les feuilles d'un point ; note la vérité d'avant de chaque condition touchée, et la
    /// dernière feuille devenue vraie (la cause d'une bascule).
    fn relire(&mut self, etat: &Etat, n: u32, avant: &mut Vec<(u32, bool)>, derniere: &mut HashMap<u32, u32>) {
        let (d, f_) = (self.prog.f_debut[n as usize] as usize, self.prog.f_debut[n as usize + 1] as usize);
        for k in d..f_ {
            let f = self.prog.f_index[k] as usize;
            let t = feuille_vraie(&self.prog, etat, f);
            if t == self.f_vrai[f] {
                continue;
            }
            let c = self.prog.f_cond[f];
            if !avant.iter().any(|&(x, _)| x == c) {
                avant.push((c, self.c_vrai[c as usize]));
            }
            self.f_vrai[f] = t;
            if t {
                self.c_vraies[c as usize] += 1;
                derniere.insert(c, f as u32);
            } else {
                self.c_vraies[c as usize] -= 1;
            }
            self.c_vrai[c as usize] = cond_vraie(&self.prog, &self.c_vraies, c as usize);
        }
    }

    /// Lit les bascules après une relecture : fausse → vraie déclenche, vraie → fausse se note.
    fn bascules(&mut self, etat: &Etat, t: Temps, avant: Vec<(u32, bool)>, derniere: &HashMap<u32, u32>, pourquoi: &str) {
        for (c, etait) in avant {
            if !etait && self.c_vrai[c as usize] {
                self.bascule_vraie(etat, t, c, derniere[&c]);
            } else if etait && !self.c_vrai[c as usize] {
                let g = self.prog.c_noeud[c as usize];
                let a = self.prog.noeud_auto[g as usize];
                let nom = self.nom_auto(a).to_string();
                self.noter(t.mur_s(), format!("{nom} : condition composée fausse{pourquoi}"));
            }
        }
    }

    /// Un message : ses changements appliqués ensemble, puis les bascules lues.
    pub fn message(&mut self, etat: &mut Etat, t: Temps, changements: &[(u32, i64)]) {
        let mut changes: Vec<(u32, i64, i64)> = Vec::new();
        let mut a_relire: Vec<u32> = Vec::new();
        for &(n, v) in changements {
            let d = etat.defs[n as usize].appareil;
            let revient = etat.sante[d as usize].vie == vie::MUET;
            let perimee = etat.vifs[n as usize].qualite == qualite::MAUVAISE;
            let prec = etat.vifs[n as usize].valeur;
            if etat.rapport(n, v, t.mur_ms, t.mono) {
                changes.push((n, prec, v));
                a_relire.push(n);
            } else if perimee {
                a_relire.push(n);
            }
            if revient {
                let nom = etat.annuaire.nom(n).to_string();
                self.noter(t.mur_s(), format!("appareil revenu : {nom}"));
                a_relire.extend(etat.points_de(d));
            }
        }
        a_relire.sort_unstable();
        a_relire.dedup();
        let mut avant = Vec::new();
        let mut derniere = HashMap::new();
        for &n in &a_relire {
            self.relire(etat, n, &mut avant, &mut derniere);
        }
        self.bascules(etat, t, avant, &derniere, "");
        for &(n, prec, v) in &changes {
            let (d, f_) = (self.prog.index_debut[n as usize] as usize, self.prog.index_debut[n as usize + 1] as usize);
            for k in d..f_ {
                let g = self.prog.index[k];
                let nd = self.prog.noeuds[g as usize];
                let (val, q) = etat.lire(n);
                let vers = nd.drapeaux & drapeau::A_VERS == 0 || nd.v == val;
                let depuis = nd.drapeaux & drapeau::A_DEPUIS == 0 || i64::from(nd.w) == prec;
                if !vers || !depuis || q == qualite::MAUVAISE {
                    continue;
                }
                let a = self.prog.noeud_auto[g as usize];
                let rang = g - self.prog.plans[a as usize].premier;
                if nd.b == 0 {
                    let cause = Cause { point: n, declencheur: rang, instant_ms: etat.vifs[n as usize].change_ms, valeur: v, precedente: prec };
                    self.declencher(etat, t, a, rang, cause);
                } else {
                    // L'attente ne copie pas la cause : elle relira la fiche du point au départ.
                    let k = nd.c as usize;
                    self.at_depuis[k] = etat.vifs[n as usize].change_ms;
                    self.at_echeance[k] = t.mono + nd.b;
                    self.at_point[k] = n;
                    let nom = self.nom_auto(a).to_string();
                    self.noter(t.mur_s(), format!("{nom} : attente armée jusqu'à {}", heure(t.mur_s() + nd.b)));
                }
            }
        }
        for &(n, _, _) in &changes {
            let (d, f_) = (self.prog.attente_debut[n as usize] as usize, self.prog.attente_debut[n as usize + 1] as usize);
            for k in d..f_ {
                let g = self.prog.attente_index[k];
                let a = self.prog.noeud_auto[g as usize];
                let rang = g - self.prog.plans[a as usize].premier;
                for c in 0..self.ex.len() as u32 {
                    let e = self.ex[c as usize];
                    if e.etat == etat_ex::ATTEND_QUE && e.automatisme == a && e.curseur == rang {
                        let nd = self.prog.noeuds[g as usize];
                        let r = self.prog.regles[nd.a as usize];
                        if tenir(&r, etat, &e) {
                            self.reprendre(etat, t, c, port::FAIT);
                        }
                    }
                }
            }
        }
    }

    fn bascule_vraie(&mut self, etat: &Etat, t: Temps, c: u32, f: u32) {
        let g = self.prog.c_noeud[c as usize];
        let nd = self.prog.noeuds[g as usize];
        let a = self.prog.noeud_auto[g as usize];
        let rang = g - self.prog.plans[a as usize].premier;
        let depuis = self.depuis(etat, c);
        let pt = self.prog.f_point[f as usize];
        let vif = etat.vifs[pt as usize];
        let nom = self.nom_auto(a).to_string();
        self.noter(t.mur_s(), format!("{nom} : condition composée vraie depuis {}", heure((depuis / 1_000) as u32)));
        if nd.b == 0 {
            if self.c_parti[c as usize] != depuis {
                self.c_parti[c as usize] = depuis;
                let cause = Cause { point: pt, declencheur: rang, instant_ms: vif.change_ms, valeur: vif.valeur, precedente: vif.precedente };
                self.declencher(etat, t, a, rang, cause);
            }
            return;
        }
        let k = nd.c as usize;
        if self.at_parti[k] == depuis {
            self.noter(t.mur_s(), format!("{nom} : déjà parti pour cette vérité"));
            return;
        }
        // La durée se compte depuis l'instant de la vérité, pas depuis l'armement ; l'échéance,
        // elle, vit en horloge monotone.
        let ecoule = (t.mur_ms.saturating_sub(depuis) / 1_000) as u32;
        self.at_depuis[k] = depuis;
        self.at_echeance[k] = t.mono + nd.b.saturating_sub(ecoule);
        self.at_point[k] = pt;
        self.noter(t.mur_s(), format!("{nom} : attente armée jusqu'à {}", heure(t.mur_s() + nd.b.saturating_sub(ecoule))));
        if self.at_echeance[k] <= t.mono {
            self.at_echeance[k] = u32::MAX;
            self.verifier_attente(etat, t, k);
        }
    }

    fn verifier_attente(&mut self, etat: &Etat, t: Temps, k: usize) {
        let (a, rang) = self.prog.attente_de[k];
        let g = self.prog.plans[a as usize].premier + rang;
        let nd = self.prog.noeuds[g as usize];
        let ok = match nd.op {
            op::QUAND_ETAT => {
                let (v, q) = etat.lire(nd.a);
                etat.vifs[nd.a as usize].change_ms == self.at_depuis[k]
                    && q != qualite::MAUVAISE
                    && (nd.drapeaux & drapeau::A_VERS == 0 || v == nd.v)
            }
            op::QUAND_COMPOSE => self.c_vrai[nd.a as usize] && self.depuis(etat, nd.a) == self.at_depuis[k],
            _ => false,
        };
        let nom = self.nom_auto(a).to_string();
        if ok && self.at_parti[k] != self.at_depuis[k] {
            self.at_parti[k] = self.at_depuis[k];
            // La cause se relit dans la fiche du point : la revérification garantit qu'elle n'a pas bougé.
            let pt = self.at_point[k];
            let vif = etat.vifs[pt as usize];
            let cause = Cause { point: pt, declencheur: rang, instant_ms: vif.change_ms, valeur: vif.valeur, precedente: vif.precedente };
            self.declencher(etat, t, a, rang, cause);
        } else {
            self.noter(t.mur_s(), format!("{nom} : attente abandonnée, la valeur a bougé ou n'est plus sûre"));
        }
    }

    fn declencher(&mut self, etat: &Etat, t: Temps, a: u32, rang: u32, cause: Cause) {
        let plan = self.prog.plans[a as usize];
        let nom = self.nom_auto(a).to_string();
        if plan.actif == 0 {
            return;
        }
        let en = self.en_cours[a as usize];
        if en != u32::MAX {
            if plan.mode == 0 {
                self.finir(t, en, "relancée par un nouveau déclenchement");
            } else {
                self.noter(t.mur_s(), format!("{nom} : déclenchement ignoré, déjà en cours"));
                return;
            }
        }
        let case = match self.libres.pop() {
            Some(c) => c,
            None => {
                self.ex.push(Execution64::default());
                (self.ex.len() - 1) as u32
            }
        };
        let generation = self.ex[case as usize].generation.wrapping_add(1);
        self.ex[case as usize] = Execution64 {
            automatisme: a,
            curseur: rang,
            reveil_s: u32::MAX,
            depuis_s: t.mono,
            generation,
            etat: etat_ex::PRETE,
            cause_point: cause.point,
            cause_declencheur: cause.declencheur,
            cause_instant_ms: cause.instant_ms,
            cause_valeur: cause.valeur,
            cause_precedente: cause.precedente,
            ..Execution64::default()
        };
        self.en_cours[a as usize] = case;
        let cause_txt = decrire(etat, cause.point, cause.valeur, cause.precedente, cause.instant_ms);
        self.noter(t.mur_s(), format!("{nom} : départ (case {case}, génération {generation}) ; cause : {cause_txt}"));
        self.passer(case);
        self.reprendre(etat, t, case, port::SORTIE);
    }

    fn passer(&mut self, case: u32) {
        let e = self.ex[case as usize];
        let plan = self.prog.plans[e.automatisme as usize];
        let id = self.prog.ids[(plan.premier + e.curseur) as usize].to_string();
        self.chemins.push((e.automatisme, case, e.generation, id));
    }

    /// Suit le lien du port depuis le nœud courant, puis avance.
    fn reprendre(&mut self, etat: &Etat, t: Temps, case: u32, p: u8) {
        if self.aller(case, p) {
            self.avancer(etat, t, case);
        } else {
            self.finir(t, case, "terminée");
        }
    }

    fn aller(&mut self, case: u32, p: u8) -> bool {
        let e = self.ex[case as usize];
        let plan = self.prog.plans[e.automatisme as usize];
        let nd = self.prog.noeuds[(plan.premier + e.curseur) as usize];
        let liens = &self.prog.liens[nd.liens as usize..nd.liens as usize + nd.nb_liens as usize];
        match liens.iter().find(|l| l.port == p) {
            Some(l) => {
                let x = &mut self.ex[case as usize];
                x.curseur = l.cible;
                x.etat = etat_ex::PRETE;
                x.reveil_s = u32::MAX;
                self.passer(case);
                true
            }
            None => false,
        }
    }

    /// Avance une exécution jusqu'à ce qu'elle attende ou finisse. Jamais en arrière.
    fn avancer(&mut self, etat: &Etat, t: Temps, case: u32) {
        for _ in 0..100 {
            let e = self.ex[case as usize];
            let plan = self.prog.plans[e.automatisme as usize];
            let nd = self.prog.noeuds[(plan.premier + e.curseur) as usize];
            let p = match nd.op {
                op::SI => {
                    let regles = &self.prog.regles[nd.a as usize..(nd.a + nd.b) as usize];
                    let oui = if nd.drapeaux & drapeau::TOUTES != 0 {
                        regles.iter().all(|r| tenir(r, etat, &e))
                    } else {
                        regles.iter().any(|r| tenir(r, etat, &e))
                    };
                    if oui { port::OUI } else { port::NON }
                }
                op::REGLER => {
                    let point = if nd.drapeaux & drapeau::CIBLE_CAUSE != 0 { e.cause_point } else { nd.a };
                    let demande = self.prochaine_demande;
                    self.prochaine_demande += 1;
                    self.ordres.push(Ordre { t: t.mur_s(), automatisme: e.automatisme, point, valeur: nd.v, case, generation: e.generation, demande });
                    let x = &mut self.ex[case as usize];
                    x.etat = etat_ex::ISSUE;
                    x.demande = demande;
                    x.reveil_s = t.mono + 5;
                    return;
                }
                op::ATTENDRE => {
                    let x = &mut self.ex[case as usize];
                    x.etat = etat_ex::DUREE;
                    x.reveil_s = t.mono + nd.b;
                    return;
                }
                op::LIRE => {
                    // Une donnée produite en route : gardée par l'exécution, même si la valeur bouge ensuite.
                    self.ex[case as usize].registre = etat.lire(nd.a).0;
                    port::SORTIE
                }
                op::ATTENDRE_QUE => {
                    let r = self.prog.regles[nd.a as usize];
                    if tenir(&r, etat, &e) {
                        port::FAIT
                    } else {
                        let x = &mut self.ex[case as usize];
                        x.etat = etat_ex::ATTEND_QUE;
                        x.reveil_s = t.mono + nd.b;
                        return;
                    }
                }
                op::PREVENIR => {
                    let gabarit = self.prog.textes[nd.a as usize].to_string();
                    let cause = decrire(etat, e.cause_point, e.cause_valeur, e.cause_precedente, e.cause_instant_ms);
                    let texte = gabarit.replace("{{declencheur}}", &cause).replace("{{lu}}", &e.registre.to_string());
                    self.notices.push((t.mur_s(), e.automatisme, texte));
                    port::SORTIE
                }
                _ => port::SORTIE,
            };
            if !self.aller(case, p) {
                self.finir(t, case, "terminée");
                return;
            }
        }
        self.finir(t, case, "arrêtée : plus de 100 pas");
    }

    fn finir(&mut self, t: Temps, case: u32, pourquoi: &str) {
        let e = self.ex[case as usize];
        if e.etat == etat_ex::LIBRE {
            return;
        }
        let x = &mut self.ex[case as usize];
        x.etat = etat_ex::LIBRE;
        x.reveil_s = u32::MAX;
        if self.en_cours[e.automatisme as usize] == case {
            self.en_cours[e.automatisme as usize] = u32::MAX;
        }
        self.libres.push(case);
        let nom = self.nom_auto(e.automatisme).to_string();
        self.noter(t.mur_s(), format!("{nom} : {pourquoi} (case {case}, génération {})", e.generation));
    }

    /// L'issue d'un ordre arrive (sur `res/`). Une issue destinée à l'occupante d'avant est écartée.
    pub fn issue(&mut self, etat: &Etat, t: Temps, case: u32, generation: u16, demande: u32) {
        let e = self.ex[case as usize];
        if e.etat == etat_ex::ISSUE && e.generation == generation && e.demande == demande {
            self.reprendre(etat, t, case, port::SORTIE);
        } else {
            self.noter(t.mur_s(), format!("issue tardive écartée (demande {demande}, case {case}, génération {generation})"));
        }
    }

    /// Le passage de l'échéancier : appareils muets, valeurs trop vieilles, attentes des
    /// déclencheurs, exécutions, déclencheurs horaires. Seules les échéances échues comptent.
    pub fn passage(&mut self, etat: &mut Etat, t: Temps) {
        for d in 0..etat.sante.len() {
            if etat.delai[d] != 0 && etat.echeance[d] < t.mono && etat.sante[d].vie == vie::VIVANT {
                etat.sante[d].vie = vie::MUET;
                etat.echeance[d] = u32::MAX;
                let pts: Vec<u32> = etat.points_de(d as u32).collect();
                let nom = etat.annuaire.nom(pts[0]).to_string();
                self.noter(t.mur_s(), format!("appareil muet : {nom}"));
                let (mut avant, mut derniere) = (Vec::new(), HashMap::new());
                for n in pts {
                    self.relire(etat, n, &mut avant, &mut derniere);
                }
                self.bascules(etat, t, avant, &derniere, " (valeur plus sûre)");
            }
        }
        for n in 0..etat.fraicheur.len() {
            if etat.fraicheur[n] != 0 && etat.perime[n] < t.mono && etat.vifs[n].qualite != qualite::MAUVAISE {
                etat.vifs[n].qualite = qualite::MAUVAISE;
                etat.perime[n] = u32::MAX;
                let nom = etat.annuaire.nom(n as u32).to_string();
                self.noter(t.mur_s(), format!("valeur trop vieille : {nom}"));
                let (mut avant, mut derniere) = (Vec::new(), HashMap::new());
                self.relire(etat, n as u32, &mut avant, &mut derniere);
                self.bascules(etat, t, avant, &derniere, " (valeur trop vieille)");
            }
        }
        for k in 0..self.at_echeance.len() {
            if self.at_echeance[k] <= t.mono {
                self.at_echeance[k] = u32::MAX;
                self.verifier_attente(etat, t, k);
            }
        }
        for c in 0..self.ex.len() as u32 {
            let e = self.ex[c as usize];
            if e.etat == etat_ex::LIBRE || e.reveil_s > t.mono {
                continue;
            }
            match e.etat {
                etat_ex::DUREE => self.reprendre(etat, t, c, port::SORTIE),
                etat_ex::ATTEND_QUE => self.reprendre(etat, t, c, port::TROP_LONG),
                etat_ex::ISSUE => {
                    let nom = self.nom_auto(e.automatisme).to_string();
                    self.noter(t.mur_s(), format!("{nom} : issue absente à l'échéance de l'ordre, on continue"));
                    self.reprendre(etat, t, c, port::SORTIE);
                }
                _ => {}
            }
        }
        let sorties = {
            let valeur = |p: u32| etat.vifs.get(p as usize).map(|v| v.valeur);
            self.cal.passage(t.mur_ms, &valeur)
        };
        self.horaires(etat, t, sorties);
    }

    fn horaires(&mut self, etat: &Etat, t: Temps, sorties: Vec<Sortie>) {
        for s in sorties {
            match s {
                Sortie::Part { fiche, occurrence_ms, retard_ms } => {
                    let (a, rang) = self.prog.horaires[fiche as usize];
                    let cause = Cause { point: HORLOGE, declencheur: rang, instant_ms: occurrence_ms, valeur: (retard_ms / 1_000) as i64, precedente: 0 };
                    self.declencher(etat, t, a, rang, cause);
                }
                Sortie::Manquee { fiche, occurrence_ms, nombre } => {
                    let (a, _) = self.prog.horaires[fiche as usize];
                    let nom = self.nom_auto(a).to_string();
                    self.noter(t.mur_s(), format!("{nom} : occurrence de {} manquée ({nombre}), retard au-delà de 2 min", heure((occurrence_ms / 1_000) as u32)));
                }
                Sortie::Evenement { fiche, occurrence_ms, agenda, numero, .. } => {
                    let (a, rang) = self.prog.horaires[fiche as usize];
                    let cause = Cause { point: AGENDA | agenda, declencheur: rang, instant_ms: occurrence_ms, valeur: i64::from(numero), precedente: 0 };
                    self.declencher(etat, t, a, rang, cause);
                }
                Sortie::Niveau { .. } => {}
            }
        }
    }

    /// Le module d'agenda a relu un agenda : les rappels se réarment, ceux qui sont dus partent.
    pub fn agenda(&mut self, etat: &Etat, t: Temps, agenda: u32, evenements: Vec<calendrier::Evenement>) {
        let sorties = self.cal.agenda(agenda, evenements, t.mur_ms);
        self.horaires(etat, t, sorties);
    }

    /// Le réveil au plus proche, en secondes monotones : la plus petite des échéances.
    pub fn prochain_reveil(&self, etat: &Etat, t: Temps) -> u32 {
        let appareils = (0..etat.sante.len())
            .filter(|&d| etat.delai[d] != 0 && etat.sante[d].vie == vie::VIVANT)
            .map(|d| etat.echeance[d].saturating_add(1))
            .min()
            .unwrap_or(u32::MAX);
        let fraicheur = (0..etat.fraicheur.len())
            .filter(|&n| etat.fraicheur[n] != 0 && etat.vifs[n].qualite != qualite::MAUVAISE)
            .map(|n| etat.perime[n].saturating_add(1))
            .min()
            .unwrap_or(u32::MAX);
        let attentes = self.at_echeance.iter().copied().min().unwrap_or(u32::MAX);
        let exec = self.ex.iter().filter(|e| e.etat != etat_ex::LIBRE).map(|e| e.reveil_s).min().unwrap_or(u32::MAX);
        let horaires = vers_mono(t, self.cal.prochain());
        appareils.min(fraicheur).min(attentes).min(exec).min(horaires)
    }

    /// Une nouvelle validation : tout le programme rebâti. Les exécutions d'un automatisme dont la
    /// logique a changé s'arrêtent ; les autres continuent, par (automatisme, rang).
    pub fn recompiler(&mut self, etat: &Etat, defs: &[Automation], t: Temps) {
        let nouveau = compiler(defs, etat);
        let change = |a: usize| a >= self.prog.empreintes.len() || self.prog.empreintes[a] != nouveau.empreintes[a];
        let changes: Vec<usize> = (0..nouveau.plans.len()).filter(|&a| change(a)).collect();
        for &a in &changes {
            for c in 0..self.ex.len() as u32 {
                let e = self.ex[c as usize];
                if e.etat != etat_ex::LIBRE && e.automatisme as usize == a {
                    self.finir(t, c, "arrêtée : logique modifiée");
                }
            }
        }
        let ancien: HashMap<(u32, u32), usize> = self.prog.attente_de.iter().enumerate().map(|(k, &x)| (x, k)).collect();
        let n = nouveau.attente_de.len();
        let (mut e, mut d, mut p, mut pt) = (vec![u32::MAX; n], vec![0; n], vec![u64::MAX; n], vec![0; n]);
        for (k, x) in nouveau.attente_de.iter().enumerate() {
            if changes.contains(&(x.0 as usize)) {
                continue;
            }
            if let Some(&o) = ancien.get(x) {
                (e[k], d[k], p[k], pt[k]) = (self.at_echeance[o], self.at_depuis[o], self.at_parti[o], self.at_point[o]);
            }
        }
        let rang_cond = |prog: &Programme, c: usize| {
            let g = prog.c_noeud[c];
            let a = prog.noeud_auto[g as usize];
            (a, g - prog.plans[a as usize].premier)
        };
        let anciennes: HashMap<(u32, u32), usize> = (0..self.prog.c_nb.len()).map(|c| (rang_cond(&self.prog, c), c)).collect();
        let mut c_parti = vec![u64::MAX; nouveau.c_nb.len()];
        for (c, cp) in c_parti.iter_mut().enumerate() {
            let x = rang_cond(&nouveau, c);
            if !changes.contains(&(x.0 as usize))
                && let Some(&o) = anciennes.get(&x)
            {
                *cp = self.c_parti[o];
            }
        }
        // Les déclencheurs horaires gardent leur dernière occurrence, par (automatisme, rang).
        let anciens_h: HashMap<(u32, u32), usize> = self.prog.horaires.iter().enumerate().map(|(k, &x)| (x, k)).collect();
        let mut cal = calendrier_de(&nouveau, &self.lieu);
        cal.horloge_sure = self.cal.horloge_sure;
        cal.evenements = std::mem::take(&mut self.cal.evenements);
        for (k, x) in nouveau.horaires.iter().enumerate() {
            if !changes.contains(&(x.0 as usize))
                && let Some(&o) = anciens_h.get(x)
            {
                cal.derniere[k] = self.cal.derniere[o];
                cal.prochaine[k] = self.cal.prochaine[o];
                cal.derniere_evt[k] = self.cal.derniere_evt[o];
                cal.prochaine_evt[k] = self.cal.prochaine_evt[o];
            }
        }
        (self.at_echeance, self.at_depuis, self.at_parti, self.at_point, self.c_parti) = (e, d, p, pt, c_parti);
        self.en_cours.resize(nouveau.plans.len(), u32::MAX);
        self.prog = nouveau;
        self.cal = cal;
        let valeur = |p: u32| etat.vifs.get(p as usize).map(|v| v.valeur);
        self.cal.armer_manquantes(t.mur_ms, &valeur);
        self.reconstruire(etat);
        self.noter(t.mur_s(), format!("programme recompilé ({} nœuds) ; {} automatisme(s) modifié(s)", self.prog.noeuds.len(), changes.len()));
    }

    /// L'instantané : les échéances monotones passent en heure murale.
    pub fn instantane(&self, t: Temps) -> Instantane {
        let mur = |mono: u32| if mono == u32::MAX { u32::MAX } else { t.mur_s() + mono.saturating_sub(t.mono) };
        let mut ex = self.ex.clone();
        for e in &mut ex {
            e.reveil_s = mur(e.reveil_s);
        }
        Instantane {
            ex,
            libres: self.libres.clone(),
            en_cours: self.en_cours.clone(),
            at_echeance: self.at_echeance.iter().map(|&x| mur(x)).collect(),
            at_depuis: self.at_depuis.clone(),
            at_parti: self.at_parti.clone(),
            at_point: self.at_point.clone(),
            c_parti: self.c_parti.clone(),
            cal_derniere: self.cal.derniere.clone(),
            cal_prochaine: self.cal.prochaine.clone(),
            cal_derniere_evt: self.cal.derniere_evt.clone(),
            cal_prochaine_evt: self.cal.prochaine_evt.clone(),
            cal_evenements: self.cal.evenements.clone(),
            prochaine_demande: self.prochaine_demande,
        }
    }

    /// Le redémarrage : programme recompilé, vérité reconstruite, fiches relues, échéances murales
    /// reconverties. Une exécution qui attendait reprend si son échéance n'est pas dépassée de plus
    /// de 2 min, sinon elle s'arrête ; un déclencheur horaire reprend après sa dernière occurrence.
    pub fn redemarrer(i: Instantane, etat: &Etat, defs: &[Automation], t: Temps, lieu: &Lieu, journal: Vec<(u32, String)>) -> Self {
        let mut m = Moteur::new(compiler(defs, etat), etat, lieu);
        m.journal = journal;
        let mono = |mur: u32| if mur == u32::MAX { u32::MAX } else { t.mono + mur.saturating_sub(t.mur_s()) };
        (m.libres, m.en_cours) = (i.libres, i.en_cours);
        (m.at_depuis, m.at_parti, m.at_point, m.c_parti) = (i.at_depuis, i.at_parti, i.at_point, i.c_parti);
        m.at_echeance = i.at_echeance.iter().map(|&x| mono(x)).collect();
        m.prochaine_demande = i.prochaine_demande;
        m.cal.derniere = i.cal_derniere;
        m.cal.prochaine = i.cal_prochaine;
        m.cal.derniere_evt = i.cal_derniere_evt;
        m.cal.prochaine_evt = i.cal_prochaine_evt;
        m.cal.evenements = i.cal_evenements;
        m.noter(t.mur_s(), "redémarrage : programme recompilé, vérité des conditions reconstruite".into());
        m.ex = i.ex;
        for c in 0..m.ex.len() as u32 {
            let e = m.ex[c as usize];
            if e.etat == etat_ex::LIBRE {
                continue;
            }
            let nom = m.nom_auto(e.automatisme).to_string();
            if e.reveil_s != u32::MAX && e.reveil_s.saturating_add(120) < t.mur_s() {
                let retard = t.mur_s() - e.reveil_s;
                m.ex[c as usize].reveil_s = t.mono;
                m.finir(t, c, &format!("interrompue : échéance dépassée de {retard} s au redémarrage"));
            } else {
                m.noter(t.mur_s(), format!("{nom} : reprise, réveil à {}", heure(e.reveil_s)));
                m.ex[c as usize].reveil_s = mono(e.reveil_s);
            }
        }
        let valeur = |p: u32| etat.vifs.get(p as usize).map(|v| v.valeur);
        m.cal.armer_manquantes(t.mur_ms, &valeur);
        m
    }
}

fn feuille_vraie(p: &Programme, etat: &Etat, f: usize) -> bool {
    let (v, q) = etat.lire(p.f_point[f]);
    q != qualite::MAUVAISE && comparer(v, p.f_op[f], p.f_valeur[f])
}

fn cond_vraie(p: &Programme, vraies: &[u16], c: usize) -> bool {
    if p.c_tous[c] { vraies[c] == p.c_nb[c] } else { vraies[c] > 0 }
}

fn tenir(r: &Regle, etat: &Etat, e: &Execution64) -> bool {
    match r.genre {
        genre::ETAT => {
            let (v, q) = etat.lire(r.point);
            q != qualite::MAUVAISE && comparer(v, r.op, r.valeur)
        }
        genre::CAUSE_POINT => e.cause_point == r.point,
        genre::CAUSE_VALEUR => comparer(e.cause_valeur, r.op, r.valeur),
        _ => false,
    }
}

/// « 21 h 51 min 00 » à partir de secondes d'horloge murale (le scénario est en UTC).
pub fn heure(s: u32) -> String {
    if s == u32::MAX {
        return "jamais".into();
    }
    let j = s % 86_400;
    format!("{:02}:{:02}:{:02}", j / 3_600, j % 3_600 / 60, j % 60)
}

fn decrire(etat: &Etat, point: u32, valeur: i64, precedente: i64, instant_ms: u64) -> String {
    if point != HORLOGE && point & AGENDA != 0 {
        return format!("agenda {}, événement {valeur}, rappel de {}", point & !AGENDA, heure((instant_ms / 1_000) as u32));
    }
    if point == HORLOGE {
        let retard = if valeur > 0 { format!(", {valeur} s de retard") } else { String::new() };
        return format!("l'horloge, {}{retard}", heure((instant_ms / 1_000) as u32));
    }
    format!("{} = {valeur} (avant : {precedente}) à {}", etat.annuaire.nom(point), heure((instant_ms / 1_000) as u32))
}

#[allow(dead_code)]
fn _op_inutilise(_: Op) {}
