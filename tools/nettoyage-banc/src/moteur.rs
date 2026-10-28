//! Ce qui cite un numéro hors de la mémoire des valeurs : le programme (compilé depuis les noms),
//! les exécutions (cause, cible), les attentes des déclencheurs à durée, la file des exécutions
//! finies que montre l'écran ; et un client qui garde les noms en cache d'une connexion à l'autre.
//! Le moteur de référence de la mémoire v2, réduit à ce qu'un retrait peut casser.

use std::collections::HashMap;

use crate::memoire::{Demenagement, Memoire};
use crate::numeros::Numero;
use crate::outils::Temps;
use crate::valeurs::qualite;

/// Un automatisme, par les noms : « quand `declencheur` passe à `vers` (pendant `pendant_s`),
/// attendre `attendre_s`, puis mettre `cible` à `ordre` ».
#[derive(Clone, Debug)]
pub struct Automatisme {
    pub declencheur: String,
    pub vers: i64,
    pub pendant_s: u32,
    pub attendre_s: u32,
    pub cible: String,
    pub ordre: i64,
}

#[derive(Clone, Copy, Debug)]
pub struct Regle {
    pub auto: u32,
    pub declencheur: Numero,
    pub vers: i64,
    pub pendant_s: u32,
    pub attendre_s: u32,
    pub cible: Numero,
    pub ordre: i64,
}

#[derive(Default)]
pub struct Programme {
    pub regles: Vec<Regle>,
    /// Les automatismes qui citent un point retiré, libéré ou inconnu : à revoir par un humain.
    pub a_revoir: Vec<u32>,
}

/// Le compilateur ne lit que les noms, à chaque validation, retrait, lot ou déménagement (mémoire v2
/// § 8.3 : 0,45 ms à ×1). Un point qui n'est plus en service ni absent met l'automatisme « à revoir ».
pub fn compiler(defs: &[Automatisme], m: &Memoire) -> Programme {
    let mut prog = Programme::default();
    let trouver = |nom: &str| m.points.numero(nom).filter(|&n| m.points.vivant(n));
    for (i, a) in defs.iter().enumerate() {
        match (trouver(&a.declencheur), trouver(&a.cible)) {
            (Some(declencheur), Some(cible)) => prog.regles.push(Regle {
                auto: i as u32,
                declencheur,
                vers: a.vers,
                pendant_s: a.pendant_s,
                attendre_s: a.attendre_s,
                cible,
                ordre: a.ordre,
            }),
            _ => prog.a_revoir.push(i as u32),
        }
    }
    prog
}

/// Une exécution en cours : elle garde sa cause et sa cible par numéro (la fiche de 64 o de la v2).
#[derive(Clone, Copy, Debug)]
pub struct Execution {
    pub id: u32,
    pub auto: u32,
    pub cause: Numero,
    pub reveil: u32,
    pub cible: Numero,
    pub ordre: i64,
}

/// L'attente d'un déclencheur à durée (24 o en v2) : revérifiée à l'échéance par l'instant « changé ».
#[derive(Clone, Copy, Debug)]
pub struct Attente {
    pub auto: u32,
    pub point: Numero,
    pub depuis_ms: u64,
    pub echeance: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Ordre {
    pub t_ms: u64,
    pub auto: u32,
    pub cible: Numero,
    pub nom: String,
    pub valeur: i64,
}

#[derive(Default)]
pub struct Moteur {
    pub prog: Programme,
    pub ex: Vec<Execution>,
    pub attentes: Vec<Attente>,
    pub ordres: Vec<Ordre>,
    /// Les exécutions finies, comme la file courte que montre l'écran : (id, cause par numéro).
    pub finies: Vec<(u32, Numero)>,
    /// Les exécutions lancées : (id, automatisme, nom de la cause au départ, instant).
    pub lancees: Vec<(u32, u32, String, u64)>,
    /// Exécutions arrivées au bout sans cible en service : aucun ordre n'est parti.
    pub sans_cible: u32,
}

impl Moteur {
    /// Une nouvelle valeur d'un point : les règles de ce point partent, ou arment leur attente.
    pub fn changement(&mut self, m: &Memoire, n: Numero, v: i64, t: Temps) {
        for i in 0..self.prog.regles.len() {
            let r = self.prog.regles[i];
            if r.declencheur != n || r.vers != v {
                continue;
            }
            let change_ms = m.vifs[n.place()].change_ms;
            if r.pendant_s == 0 {
                self.lancer(m, &r, t);
            } else {
                // Une case par déclencheur à durée : la nouvelle transition remplace l'ancienne.
                self.attentes.retain(|a| a.auto != r.auto);
                self.attentes.push(Attente { auto: r.auto, point: n, depuis_ms: change_ms, echeance: t.mono + r.pendant_s });
            }
        }
    }

    fn lancer(&mut self, m: &Memoire, r: &Regle, t: Temps) {
        let id = self.lancees.len() as u32;
        self.lancees.push((id, r.auto, m.points.nom(r.declencheur).unwrap_or("?").to_owned(), t.mur_ms));
        self.ex.push(Execution { id, auto: r.auto, cause: r.declencheur, reveil: t.mono + r.attendre_s, cible: r.cible, ordre: r.ordre });
    }

    /// Le réveil : attentes échues, puis exécutions dont l'heure est venue.
    pub fn tic(&mut self, m: &Memoire, t: Temps) {
        let (dues, gardees): (Vec<Attente>, Vec<Attente>) = self.attentes.drain(..).partition(|a| a.echeance <= t.mono);
        self.attentes = gardees;
        for a in dues {
            // L'instant « changé » n'a pas bougé (mémoire v2 § 8.5), ET la valeur se lit encore : un
            // numéro inerte, un appareil absent ou muet ne fait rien partir.
            let lisible = m.lire(a.point).is_some_and(|(_, q)| q != qualite::MAUVAISE);
            if lisible && m.vifs[a.point.place()].change_ms == a.depuis_ms {
                if let Some(r) = self.prog.regles.iter().find(|r| r.auto == a.auto).copied() {
                    self.lancer(m, &r, t);
                }
            }
        }
        let (dues, gardees): (Vec<Execution>, Vec<Execution>) = self.ex.drain(..).partition(|e| e.reveil <= t.mono);
        self.ex = gardees;
        for e in dues {
            if m.points.vivant(e.cible) {
                let nom = m.points.nom(e.cible).unwrap_or("?").to_owned();
                self.ordres.push(Ordre { t_ms: t.mur_ms, auto: e.auto, cible: e.cible, nom, valeur: e.ordre });
            } else {
                self.sans_cible += 1;
            }
            self.finies.push((e.id, e.cause));
        }
    }

    /// Au retrait : les attentes des points qui ne se lisent plus s'en vont (elles ne partiraient pas).
    pub fn retrait(&mut self, m: &Memoire) {
        self.attentes.retain(|a| m.points.vivant(a.point));
    }

    /// Au déménagement : chaque numéro cité suit son point. Le programme, lui, se recompile.
    pub fn renumeroter(&mut self, dm: &Demenagement) {
        let t = dm.table_points();
        let suivre = |n: &mut Numero| {
            if let Some(&b) = t.get(&n.0) {
                *n = b;
            }
        };
        for e in &mut self.ex {
            suivre(&mut e.cause);
            suivre(&mut e.cible);
        }
        for a in &mut self.attentes {
            suivre(&mut a.point);
        }
        for f in &mut self.finies {
            suivre(&mut f.1);
        }
    }
}

/// Un client (écran, application) qui garde en cache le nom de chaque numéro d'une connexion à
/// l'autre, et ne lit que `val/`. La règle de la mémoire v2 § 4.4 : un numéro inconnu → `meta/`.
#[derive(Default)]
pub struct Client {
    pub cache: HashMap<u32, String>,
    pub demandes_meta: u32,
}

impl Client {
    /// Le nom sous lequel le client range une nouvelle du numéro `n`.
    pub fn nom(&mut self, n: Numero, meta: impl Fn(Numero) -> Option<String>) -> String {
        let cle = n.0;
        if let Some(nom) = self.cache.get(&cle) {
            return nom.clone();
        }
        self.demandes_meta += 1;
        let nom = meta(n).unwrap_or_default();
        self.cache.insert(cle, nom.clone());
        nom
    }
}
