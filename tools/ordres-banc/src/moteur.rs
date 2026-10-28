//! Le moteur réduit : ce qu'un workflow demande aux valeurs et aux ordres.
//!
//! Le programme cite les valeurs par numéro de point et les ordres par numéro d'ordre (mémoire v2
//! § 8.2, règle 1, étendue aux ordres) : une condition lit une valeur, ou la fiche d'un ordre (sa
//! dernière commande), sans rien copier. Un déclencheur se réveille par l'index de son point, ou
//! par l'index de son ordre quand une trace d'ordre arrive. Une action est une demande : elle cite
//! un ordre, jamais une valeur.

use crate::catalogue::{AUCUNE_CLE, Arg, preuve};
use crate::demandes::{Ref, issue};
use crate::droits::{self, Mandat, classe, drapeau_m};
use crate::hub::{AutoriteMoteur, Cible, Hub, Req, Sortie, T};
use crate::numeros::{Fnv, Numero, get16, get32, get64, put16, put32, put64, take};

/// Au plus 20 départs par minute et par automatisme, comme aujourd'hui (`docs/components/automatismes.md`) :
/// deux automatismes qui se déclenchent l'un l'autre ne tournent pas sans fin (revue v3, W6).
pub const DEPARTS_PAR_MINUTE: usize = 20;

/// Ce qu'une règle lit.
pub mod genre {
    /// Une valeur : `valeur(point) op x`.
    pub const VALEUR: u8 = 1;
    /// La dernière commande d'un ordre : son premier paramètre `op x`.
    pub const DERNIERE: u8 = 2;
    /// La dernière issue d'un ordre : parmi un masque d'issues.
    pub const ISSUE: u8 = 3;
    /// Qui a donné la dernière commande : parmi un masque d'origines.
    pub const ORIGINE: u8 = 4;
    /// La dernière commande date de moins de `x` secondes.
    pub const DEPUIS: u8 = 5;
    /// Une demande de cet ordre vit (retenue, en file, suivie) : `actives` de sa fiche, à part de
    /// l'issue de la dernière (revue I6).
    pub const EN_COURS: u8 = 6;
    /// La valeur que l'ordre prouve suit-elle sa dernière commande ? `oui`, `non` ou `inconnu`
    /// (`resultat::*`), comparé à la valeur de la règle (revue I5) : une compatibilité, pas une
    /// causalité.
    pub const VIENT_DE: u8 = 7;
}

/// Ce que répond `VIENT_DE`.
pub mod resultat {
    /// Rien ne la relie à la dernière commande : jamais commandé, la commande a échoué, ou la valeur
    /// a changé depuis la preuve (un geste à la main, une autre application).
    pub const NON: u8 = 0;
    /// La dernière commande est confirmée, et la première variation de la valeur après son envoi est
    /// celle qu'elle demandait ; rien n'a changé depuis.
    pub const OUI: u8 = 1;
    /// On ne sait pas : pas de preuve (exécuté, accepté, envoyé, incertain, en cours), une valeur
    /// déjà là avant la commande, ou plusieurs variations avant la preuve.
    pub const INCONNU: u8 = 2;
}

pub mod op {
    pub const EGAL: u8 = 1;
    pub const DIFFERENT: u8 = 2;
    pub const SUP: u8 = 3;
    pub const INF: u8 = 4;
    /// Faux devient vrai, vrai devient faux.
    pub const NON: u8 = 0x80;
}

/// Une règle compilée : 16 o, comme celles de la mémoire v2. Le numéro est celui d'un point ou d'un
/// ordre, selon le genre.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct Regle {
    pub valeur: i64,
    pub numero: u32,
    pub genre: u8,
    pub op: u8,
    pub _r: u16,
}

/// Un bail « pendant » : ouvert par un ordre, fermé par un ordre qui rétablit la valeur d'avant,
/// lue au moment de l'ouverture (une valeur, un ordre : les deux, chacun à sa place). 32 o.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct Bail {
    /// La valeur à rétablir, lue dans le point que l'ordre règle.
    pub valeur: i64,
    /// Son terme, en heure murale (ms) dans l'instantané et sur `wf/` ; chaque machine le
    /// reconvertit dans sa propre horloge monotone.
    pub terme_mur_ms: u64,
    /// La demande qui l'a ouvert : le bail ne possède l'état que tant qu'elle reste la dernière
    /// commande de l'ordre et que la valeur la suit (revue v3, W3).
    pub demande: u64,
    /// L'ordre qui ouvre et qui rétablit.
    pub ordre: u32,
    /// L'exécution qui l'a ouvert (case | génération).
    pub execution: u32,
    pub generation: u16,
    /// Le mandat sous lequel il rétablit (l'automatisme + 1).
    pub mandat: u16,
    pub etat: u8,
    pub _r: [u8; 3],
}

const _: () = assert!(std::mem::size_of::<Bail>() == 40);

/// Une action compilée.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pas {
    /// Une demande à un ordre ; l'exécution attend son issue.
    Commander { ordre: Numero, args: Vec<Arg> },
    /// Une demande, puis la valeur d'avant rétablie au terme, même en retard (mémoire v2 § 11).
    Bail { ordre: Numero, args: Vec<Arg>, duree_s: u32 },
    /// Continuer seulement si toutes les règles tiennent.
    Si { regles: Vec<Regle> },
    Prevenir { texte: String },
}

/// Un déclencheur compilé.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Declencheur {
    /// Une valeur change (vers une valeur donnée, ou toute valeur).
    Valeur { point: Numero, vers: Option<i64> },
    /// Une trace d'ordre arrive avec une issue du masque (`EN_COURS` : l'ordre part).
    Ordre { ordre: Numero, issues: u16 },
    /// Le scénario le lance (une heure, un bouton) : le moteur du temps est celui de la mémoire v2.
    Main,
}

/// Un automatisme, tel qu'un humain l'a validé : par les noms.
#[derive(Clone, Debug)]
pub struct Def {
    pub nom: String,
    pub declencheur: DeclDef,
    pub pas: Vec<PasDef>,
}

#[derive(Clone, Debug)]
pub enum DeclDef {
    Valeur(String, Option<i64>),
    Ordre(String, u16),
    Main,
}

#[derive(Clone, Debug)]
pub enum PasDef {
    Commander(String, Vec<Arg>),
    Bail(String, Vec<Arg>, u32),
    /// (genre, nom du point ou de l'ordre, op, valeur)
    Si(Vec<(u8, String, u8, i64)>),
    Prevenir(String),
}

/// Un automatisme compilé ; `a_revoir` dit pourquoi il ne part plus (sa cible, sa structure, son
/// mandat). Son mandat (v3) : ce que son graphe commande, et qui en répond ; son empreinte : une
/// exécution relue ne reprend que dans le même programme (revue v3, W4).
#[derive(Clone, Debug)]
pub struct Auto {
    pub nom: String,
    pub declencheur: Declencheur,
    pub pas: Vec<Pas>,
    pub a_revoir: Option<String>,
    pub mandat: Mandat,
    pub empreinte: u32,
}

/// Qui répond d'un automatisme : la personne qui l'a validé (par son nom), s'il est adopté par le
/// foyer (il survit à son départ), et si son déclencheur ouvert à tous a été accepté en connaissance
/// de cause pour un mandat sensible.
#[derive(Clone, Debug, Default)]
pub struct Responsabilite {
    pub responsable: String,
    pub foyer: bool,
    pub declencheur_ouvert: bool,
}

pub mod etat_ex {
    pub const LIBRE: u8 = 0;
    pub const EN_COURS: u8 = 1;
    pub const ISSUE: u8 = 2;
}

/// Une exécution, réduite à ce que les ordres exigent : son pas, la demande dont elle attend
/// l'issue, sa génération (la mémoire v2 en garde 64 o, cause comprise ; 72 o avec la v3). Son
/// autorité (v3) : lancée par une personne (`appele`), elle garde sa connexion, la personne et sa
/// preuve ; sinon, le mandat de l'automatisme. Un bail attend le succès de son ordre pour s'ouvrir.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Execution {
    pub auto: u32,
    pub curseur: u32,
    pub demande: u64,
    pub empreinte: u32,
    pub generation: u16,
    pub etat: u8,
    pub appele: bool,
    pub de: u16,
    pub pour: u16,
    pub preuve: u8,
    pub bail: bool,
    pub bail_duree_s: u32,
    pub bail_avant: i64,
}

#[derive(Clone, Default)]
pub struct Moteur {
    pub defs: Vec<Def>,
    pub autos: Vec<Auto>,
    /// L'index par point et l'index par ordre : `debut[n]..debut[n+1]` dans `index`.
    pub debut_p: Vec<u32>,
    pub index_p: Vec<u32>,
    pub debut_o: Vec<u32>,
    pub index_o: Vec<u32>,
    pub ex: Vec<Execution>,
    pub baux: Vec<Bail>,
    /// Ce que le moteur a fait, pour les tests : (heure murale s, automatisme, ce qui s'est passé).
    pub journal: Vec<(u64, String, String)>,
    pub actif: bool,
    /// Son principal : le moteur demande en son nom, sans reçu (interne).
    pub de: u16,
    /// Qui répond de chaque automatisme (par son nom) ; à défaut, `responsable_defaut`.
    pub responsabilites: std::collections::HashMap<String, Responsabilite>,
    pub responsable_defaut: String,
    /// Les départs de la dernière minute, par automatisme (revue v3, W6).
    pub departs: Vec<std::collections::VecDeque<u64>>,
    /// La dernière transition d'ordre qui a lancé chaque automatisme : (demande, issue).
    pub dernier: Vec<(u64, u8)>,
}

impl Moteur {
    /// Recompiler tout le programme depuis les définitions (à chaque validation, retrait, lot,
    /// redéclaration) : des noms vers les numéros, contre le catalogue.
    pub fn compiler(&mut self, hub: &mut Hub) {
        self.autos.clear();
        for d in &self.defs {
            let mut revoir = None;
            let declencheur = match &d.declencheur {
                DeclDef::Valeur(n, vers) => match hub.points.numero(n).filter(|&p| hub.points.vivant(p)) {
                    Some(p) => Declencheur::Valeur { point: p, vers: *vers },
                    None => {
                        revoir = Some(format!("valeur {n} retirée ou inconnue"));
                        Declencheur::Main
                    }
                },
                DeclDef::Ordre(n, m) => match hub.ordres.numero(n).filter(|&o| hub.ordres.vivant(o)) {
                    Some(o) => Declencheur::Ordre { ordre: o, issues: *m },
                    None => {
                        revoir = Some(format!("ordre {n} retiré ou inconnu"));
                        Declencheur::Main
                    }
                },
                DeclDef::Main => Declencheur::Main,
            };
            let mut pas = Vec::new();
            for p in &d.pas {
                match p {
                    PasDef::Commander(n, args) | PasDef::Bail(n, args, _) => {
                        let Some(o) = hub.ordres.numero(n).filter(|&o| hub.ordres.vivant(o)) else {
                            revoir.get_or_insert(format!("ordre {n} retiré ou inconnu"));
                            continue;
                        };
                        // La structure de l'ordre a pu changer (redéclaration) : la valeur est revérifiée.
                        let f = hub.catalogue.fiches[o.place()].forme;
                        if let Err(r) = hub.catalogue.valider(f, args) {
                            revoir.get_or_insert(format!("{n} : {}", crate::catalogue::raison::NOMS[usize::from(r)]));
                            continue;
                        }
                        pas.push(match p {
                            PasDef::Bail(_, _, s) => Pas::Bail { ordre: o, args: args.clone(), duree_s: *s },
                            _ => Pas::Commander { ordre: o, args: args.clone() },
                        });
                    }
                    PasDef::Si(rs) => {
                        let mut regles = Vec::new();
                        for (g, n, o, v) in rs {
                            let num = if *g == genre::VALEUR { hub.points.numero(n) } else { hub.ordres.numero(n) };
                            match num {
                                Some(x) => regles.push(Regle { valeur: *v, numero: x.0, genre: *g, op: *o, _r: 0 }),
                                None => {
                                    revoir.get_or_insert(format!("règle sur {n} inconnu"));
                                }
                            }
                        }
                        pas.push(Pas::Si { regles });
                    }
                    PasDef::Prevenir(t) => pas.push(Pas::Prevenir { texte: t.clone() }),
                }
            }
            // Le mandat (v3) : le manifeste de ce que le graphe commande, et qui en répond. Le
            // responsable doit pouvoir l'accorder lui-même ; un déclencheur que d'autres peuvent
            // provoquer ne lance pas une action sensible, sauf décision explicite (revue v3, W1, W2).
            let resp = self.responsabilites.get(&d.nom).cloned().unwrap_or_else(|| Responsabilite { responsable: self.responsable_defaut.clone(), ..Responsabilite::default() });
            let mut mandat = Mandat::default();
            for p in &pas {
                if let Pas::Commander { ordre, .. } | Pas::Bail { ordre, .. } = p {
                    let f = hub.catalogue.fiches[ordre.place()].forme;
                    mandat.classes |= classe::bit(hub.catalogue.formes[usize::from(f)].classe());
                    mandat.pieces |= hub.bits_pieces(*ordre);
                }
            }
            let id = hub.personne(&resp.responsable);
            mandat.responsable = id;
            if resp.foyer {
                mandat.drapeaux |= drapeau_m::FOYER;
            }
            let ouvert = match &declencheur {
                Declencheur::Ordre { .. } => true,
                Declencheur::Valeur { point, .. } => hub.reglee_par_un_ordre(*point),
                Declencheur::Main => false,
            };
            let accorde = match hub.personnes.get(usize::from(id)).filter(|_| id != 0) {
                None => Err("aucun responsable".to_string()),
                Some(p) if resp.foyer && p.role != droits::role::PROPRIETAIRE => Err("seul un propriétaire fait adopter un automatisme par le foyer".to_string()),
                Some(p) if !droits::peut_mandater(&hub.table, p.debut_mur_ms, p, mandat.classes, mandat.pieces) => Err(format!("{} ne peut pas accorder ce mandat", resp.responsable)),
                Some(_) if ouvert && mandat.classes & classe::SENSIBLES != 0 && !resp.declencheur_ouvert => Err("déclencheur ouvert à d'autres pour une action sensible".to_string()),
                Some(_) => Ok(()),
            };
            match accorde {
                Ok(()) => mandat.drapeaux |= drapeau_m::VIVANT,
                Err(e) => {
                    revoir.get_or_insert(format!("mandat refusé : {e}"));
                }
            }
            let empreinte = empreinte_def(d);
            self.autos.push(Auto { nom: d.nom.clone(), declencheur, pas, a_revoir: revoir, mandat, empreinte });
        }
        hub.mandater(self.autos.iter().map(|a| a.mandat).collect());
        self.departs.resize(self.autos.len(), std::collections::VecDeque::new());
        self.dernier.resize(self.autos.len(), (0, 0));
        self.reprendre();
        self.indexer(hub);
    }

    /// Une exécution vivante ne reprend que dans le programme où elle est née : si son automatisme a
    /// changé de place, elle le retrouve par son empreinte ; s'il a changé de logique, elle s'arrête
    /// (revue v3, W4 ; mémoire v2 § 8.3).
    fn reprendre(&mut self) {
        for c in 0..self.ex.len() {
            let x = self.ex[c];
            if x.etat == etat_ex::LIBRE {
                continue;
            }
            if self.autos.get(x.auto as usize).is_some_and(|a| a.empreinte == x.empreinte) {
                continue;
            }
            match self.autos.iter().position(|a| a.empreinte == x.empreinte) {
                Some(k) => self.ex[c].auto = k as u32,
                None => {
                    self.journal.push((0, format!("exécution {c}"), "programme changé : arrêtée".into()));
                    self.ex[c].etat = etat_ex::LIBRE;
                }
            }
        }
    }

    fn indexer(&mut self, hub: &Hub) {
        let (np, no) = (hub.points.len(), hub.ordres.len());
        let mut par_p: Vec<Vec<u32>> = vec![Vec::new(); np];
        let mut par_o: Vec<Vec<u32>> = vec![Vec::new(); no];
        for (k, a) in self.autos.iter().enumerate() {
            if a.a_revoir.is_some() {
                continue;
            }
            match a.declencheur {
                Declencheur::Valeur { point, .. } => par_p[point.place()].push(k as u32),
                Declencheur::Ordre { ordre, .. } => par_o[ordre.place()].push(k as u32),
                Declencheur::Main => {}
            }
        }
        (self.debut_p, self.index_p) = csr(&par_p);
        (self.debut_o, self.index_o) = csr(&par_o);
    }

    pub fn a_revoir(&self) -> Vec<(String, String)> {
        self.autos.iter().filter_map(|a| a.a_revoir.clone().map(|r| (a.nom.clone(), r))).collect()
    }

    /// Une règle, lue dans la mémoire : une valeur, ou la fiche d'un ordre.
    pub fn tient(hub: &Hub, t: T, r: &Regle) -> bool {
        let n = Numero(r.numero);
        let brut = match r.genre {
            genre::VALEUR => hub.points.vivant(n) && comparer(hub.vifs[n.place()].valeur, r.op, r.valeur),
            _ if !hub.ordres.vivant(n) => false,
            genre::DERNIERE => {
                let f = &hub.catalogue.fiches[n.place()];
                f.issue != 0 && comparer(f.valeur, r.op, r.valeur)
            }
            genre::ISSUE => 1u64 << hub.catalogue.fiches[n.place()].issue & r.valeur as u64 != 0,
            genre::ORIGINE => 1u64 << hub.catalogue.fiches[n.place()].origine & r.valeur as u64 != 0,
            genre::DEPUIS => {
                let f = &hub.catalogue.fiches[n.place()];
                f.issue != 0 && t.mur_ms.saturating_sub(f.instant_ms) <= r.valeur as u64 * 1_000
            }
            genre::EN_COURS => hub.catalogue.fiches[n.place()].actives > 0,
            genre::VIENT_DE => i64::from(vient_de(hub, n)) == r.valeur,
            _ => false,
        };
        brut != (r.op & op::NON != 0)
    }

    /// Une valeur a changé : seuls les déclencheurs de son point se réveillent.
    pub fn changement(&mut self, hub: &mut Hub, t: T, point: Numero, valeur: i64) {
        if !self.actif || point.place() + 1 >= self.debut_p.len() {
            return;
        }
        let (d, f) = (self.debut_p[point.place()] as usize, self.debut_p[point.place() + 1] as usize);
        for k in self.index_p[d..f].to_vec() {
            if let Declencheur::Valeur { point: p, vers } = self.autos[k as usize].declencheur
                && p == point
                && vers.is_none_or(|v| v == valeur)
            {
                self.lancer(hub, t, k, &format!("valeur {}", hub.points.nom(point).unwrap_or("")), None);
            }
        }
    }

    /// Une trace d'ordre est arrivée : les issues des exécutions qui attendaient, puis les
    /// déclencheurs de cet ordre.
    pub fn trace(&mut self, hub: &mut Hub, t: T, d: Ref, o: Numero, i: u8, conclue: bool) {
        if !self.actif {
            return;
        }
        if conclue || matches!(i, issue::ACCEPTE | issue::ENVOYE | issue::INCERTAIN) {
            for c in 0..self.ex.len() {
                let x = self.ex[c];
                if x.etat == etat_ex::ISSUE && x.demande == d.0 {
                    let nom = self.autos[x.auto as usize].nom.clone();
                    self.journal.push((t.mur_ms / 1_000, nom.clone(), format!("issue {} de {}", issue::NOMS[usize::from(i)], hub.nom_ordre(o))));
                    if x.bail {
                        // Le bail ne s'ouvre qu'une fois son ordre réussi (revue v3, W3).
                        if matches!(i, issue::CONFIRME | issue::EXECUTE | issue::ACCEPTE | issue::ENVOYE) {
                            self.ouvrir_bail(hub, t, c, o, d);
                        } else {
                            self.journal.push((t.mur_ms / 1_000, nom, "bail non ouvert : l'ordre n'a pas réussi".into()));
                        }
                        self.ex[c].bail = false;
                    }
                    self.ex[c].etat = etat_ex::EN_COURS;
                    self.ex[c].curseur += 1;
                    self.avancer(hub, t, c);
                }
            }
        }
        if o.place() + 1 >= self.debut_o.len() {
            return;
        }
        let (a, b) = (self.debut_o[o.place()] as usize, self.debut_o[o.place() + 1] as usize);
        for k in self.index_o[a..b].to_vec() {
            if let Declencheur::Ordre { ordre, issues } = self.autos[k as usize].declencheur
                && ordre == o
                && issues & (1 << i) != 0
                && self.dernier[k as usize] != (d.0, i)
            {
                // Une transition d'une demande identifiée, une fois (revue v3, W6).
                self.dernier[k as usize] = (d.0, i);
                self.lancer(hub, t, k, &format!("ordre {} : {}", hub.nom_ordre(o), issue::NOMS[usize::from(i)]), None);
            }
        }
    }

    /// Le scénario lance un automatisme sans personne (une heure, un bouton physique) : son mandat.
    pub fn lancer_nom(&mut self, hub: &mut Hub, t: T, nom: &str) {
        self.lancer_avec(hub, t, nom, None);
    }

    /// Une personne lance un automatisme (une routine, une scène) depuis sa connexion : chaque ordre
    /// garde ses droits et ceux de sa connexion, dans la limite du mandat (revue v3, W1).
    pub fn lancer_par(&mut self, hub: &mut Hub, t: T, nom: &str, de: u16, indice: u16) {
        let (pour, preuve_) = hub.personne_de(t, de, indice);
        self.lancer_avec(hub, t, nom, Some((de, pour, preuve_)));
    }

    fn lancer_avec(&mut self, hub: &mut Hub, t: T, nom: &str, appel: Option<(u16, u16, u8)>) {
        let k = self.autos.iter().position(|a| a.nom == nom).expect("automatisme");
        if self.autos[k].a_revoir.is_some() {
            let r = self.autos[k].a_revoir.clone().unwrap_or_default();
            self.journal.push((t.mur_ms / 1_000, nom.to_string(), format!("à revoir, ne part pas : {r}")));
            return;
        }
        self.lancer(hub, t, k as u32, if appel.is_some() { "lancé par une personne" } else { "à la main" }, appel);
    }

    fn lancer(&mut self, hub: &mut Hub, t: T, k: u32, cause: &str, appel: Option<(u16, u16, u8)>) {
        let nom = self.autos[k as usize].nom.clone();
        let fenetre = &mut self.departs[k as usize];
        while fenetre.front().is_some_and(|&d| d + 60_000 <= t.mur_ms) {
            fenetre.pop_front();
        }
        if fenetre.len() >= DEPARTS_PAR_MINUTE {
            self.journal.push((t.mur_ms / 1_000, nom, "quota : 20 départs par minute".into()));
            return;
        }
        fenetre.push_back(t.mur_ms);
        let c = match self.ex.iter().position(|x| x.etat == etat_ex::LIBRE) {
            Some(c) => c,
            None => {
                self.ex.push(Execution::default());
                self.ex.len() - 1
            }
        };
        let g = self.ex[c].generation.wrapping_add(1).max(1);
        let (appele, de, pour, preuve_) = match appel {
            Some((de, pour, p)) => (true, de, pour, p),
            None => (false, self.de, 0, droits::preuve::AUCUNE),
        };
        self.ex[c] = Execution { auto: k, curseur: 0, demande: 0, empreinte: self.autos[k as usize].empreinte, generation: g, etat: etat_ex::EN_COURS, appele, de, pour, preuve: preuve_, ..Execution::default() };
        self.journal.push((t.mur_ms / 1_000, nom, format!("part : {cause}")));
        self.sortir(hub, c);
        self.avancer(hub, t, c);
    }

    /// L'autorité d'une exécution, pour chacune de ses demandes.
    fn autorite(&self, c: usize) -> AutoriteMoteur {
        let x = self.ex[c];
        AutoriteMoteur { appele: x.appele, de: x.de, pour: x.pour, preuve: x.preuve, mandat: x.auto as u16 + 1 }
    }

    fn avancer(&mut self, hub: &mut Hub, t: T, c: usize) {
        loop {
            let x = self.ex[c];
            let a = &self.autos[x.auto as usize];
            let Some(p) = a.pas.get(x.curseur as usize).cloned() else {
                self.ex[c].etat = etat_ex::LIBRE;
                self.sortir(hub, c);
                return;
            };
            let nom = a.nom.clone();
            let qui = c as u32 | u32::from(x.generation) << 16;
            match p {
                Pas::Si { regles } => {
                    if !regles.iter().all(|r| Self::tient(hub, t, r)) {
                        self.journal.push((t.mur_ms / 1_000, nom, "conditions non réunies".into()));
                        self.ex[c].etat = etat_ex::LIBRE;
                        self.sortir(hub, c);
                        return;
                    }
                    self.ex[c].curseur += 1;
                }
                Pas::Prevenir { texte } => {
                    hub.notices.push(format!("{nom} : {texte}"));
                    self.journal.push((t.mur_ms / 1_000, nom, format!("prévient : {texte}")));
                    self.ex[c].curseur += 1;
                }
                Pas::Commander { ordre, ref args } | Pas::Bail { ordre, ref args, .. } => {
                    let args = args.clone();
                    if let Pas::Bail { duree_s, .. } = &p {
                        // La valeur d'avant se lit dans la valeur que l'ordre règle (par sa clé) ; le bail
                        // s'ouvrira au succès de l'ordre (revue v3, W3).
                        let f = hub.catalogue.fiches[ordre.place()].forme;
                        let cle = hub.catalogue.params_de(f).first().map_or(AUCUNE_CLE, |q| q.regle);
                        self.ex[c].bail = true;
                        self.ex[c].bail_avant = hub.point_de(ordre, cle).map_or(0, |p| hub.vifs[p.place()].valeur);
                        self.ex[c].bail_duree_s = *duree_s;
                    }
                    let autorite = Some(self.autorite(c));
                    let r = hub.demander(t, Req { qui, autorite, ..Req::nouvelle(self.de, 0, Cible::Numero(ordre), args, t) });
                    match r {
                        Ok(d) if hub.demandes.get(d).is_some() => {
                            self.ex[c].etat = etat_ex::ISSUE;
                            self.ex[c].demande = d.0;
                            self.sortir(hub, c);
                            return;
                        }
                        Ok(_) => self.ex[c].curseur += 1,
                        Err((_, raison)) => {
                            self.journal.push((t.mur_ms / 1_000, nom.clone(), format!("ordre refusé ({})", crate::catalogue::raison::NOMS[usize::from(raison)])));
                            // Un automatisme autonome dont le mandat ne passe plus (son responsable a
                            // perdu ses droits) : à revoir, il ne part plus (revue v3, W2).
                            if raison == crate::catalogue::raison::DROITS && !x.appele {
                                self.autos[x.auto as usize].a_revoir.get_or_insert(format!("{nom} : mandat refusé à l'exécution"));
                            }
                            self.ex[c].etat = etat_ex::LIBRE;
                            self.sortir(hub, c);
                            return;
                        }
                    }
                }
            }
        }
    }

    /// Chaque transition d'une exécution part sur `wf/ex/<c>` au rang `s`, au pair seulement (HA v2
    /// § 7 ; revue v3, W5) : le secours qui reprend continue une exécution entre deux étapes.
    fn sortir(&self, hub: &mut Hub, c: usize) {
        if hub.actif {
            let x = self.ex[c];
            hub.sortir(Sortie::Execution { c: c as u32, ex: (x.etat != etat_ex::LIBRE).then_some(x) });
        }
    }

    /// Un bail s'ouvre : la demande réussie, la valeur d'avant, son terme.
    fn ouvrir_bail(&mut self, hub: &mut Hub, t: T, c: usize, o: Numero, d: Ref) {
        let x = self.ex[c];
        let qui = c as u32 | u32::from(x.generation) << 16;
        let b = Bail { valeur: x.bail_avant, terme_mur_ms: t.mur_ms + u64::from(x.bail_duree_s) * 1_000, demande: d.0, ordre: o.0, execution: qui, generation: 1, mandat: x.auto as u16 + 1, etat: 1, _r: [0; 3] };
        let k = match self.baux.iter().position(|b| b.etat == 0) {
            Some(k) => k,
            None => {
                self.baux.push(Bail::default());
                self.baux.len() - 1
            }
        };
        let g = self.baux[k].generation.wrapping_add(1).max(1);
        self.baux[k] = Bail { generation: g, ..b };
        hub.sortir(Sortie::Bail { b: k as u32, bail: Some(self.baux[k]) });
    }

    /// Le prochain terme d'un bail, en heure murale : le moteur se réveille là.
    pub fn prochaine(&self) -> Option<u64> {
        self.baux.iter().filter(|b| b.etat == 1).map(|b| b.terme_mur_ms).min()
    }

    /// Les baux arrivés à terme : l'ordre rétablit la valeur d'avant, même en retard.
    pub fn echeances(&mut self, hub: &mut Hub, t: T) {
        if !self.actif {
            return;
        }
        for k in 0..self.baux.len() {
            let b = self.baux[k];
            if b.etat == 1 && b.terme_mur_ms <= t.mur_ms {
                self.baux[k].etat = 0;
                hub.sortir(Sortie::Bail { b: k as u32, bail: None });
                let o = Numero(b.ordre);
                // Le bail ne possède l'état que si sa commande est encore la dernière et que la valeur la
                // suit : un humain qui a changé l'état entre-temps l'emporte (revue v3, W3).
                if hub.catalogue.fiches[o.place()].demande.0 != b.demande || vient_de(hub, o) == resultat::NON {
                    self.journal.push((t.mur_ms / 1_000, hub.nom_ordre(o), "bail : l'état a changé de main, rien n'est rétabli".into()));
                    continue;
                }
                let retard = (t.mur_ms - b.terme_mur_ms) / 1_000;
                self.journal.push((t.mur_ms / 1_000, hub.nom_ordre(o), format!("bail rétabli à {} ({retard} s de retard)", b.valeur)));
                // Une nouvelle commande, sous le mandat de l'automatisme, revérifiée à son terme.
                let autorite = Some(AutoriteMoteur { appele: false, de: self.de, pour: 0, preuve: 0, mandat: b.mandat });
                if let Err((_, r)) = hub.demander(t, Req { qui: b.execution, autorite, ..Req::nouvelle(self.de, 0, Cible::Numero(o), vec![Arg::N(b.valeur)], t) }) {
                    self.journal.push((t.mur_ms / 1_000, hub.nom_ordre(o), format!("bail non rétabli ({})", crate::catalogue::raison::NOMS[usize::from(r)])));
                }
            }
        }
    }

    /// Une copie applique `wf/bail/<b>` et `wf/ex/<c>`.
    pub fn appliquer(&mut self, s: &Sortie) {
        match s {
            Sortie::Bail { b, bail } => {
                let k = *b as usize;
                if k >= self.baux.len() {
                    self.baux.resize(k + 1, Bail::default());
                }
                match bail {
                    Some(x) => self.baux[k] = *x,
                    None => self.baux[k].etat = 0,
                }
            }
            Sortie::Execution { c, ex } => {
                let k = *c as usize;
                if k >= self.ex.len() {
                    self.ex.resize(k + 1, Execution::default());
                }
                match ex {
                    Some(x) => self.ex[k] = *x,
                    None => self.ex[k].etat = etat_ex::LIBRE,
                }
            }
            _ => {}
        }
    }

    /// Un appareil retiré : ses baux se terminent sans rien rétablir.
    pub fn retirer(&mut self, hub: &mut Hub, a: Numero) {
        for k in 0..self.baux.len() {
            let b = self.baux[k];
            if b.etat == 1 && hub.app_de_ordre(Numero(b.ordre)) == a.place() {
                self.baux[k].etat = 0;
                hub.sortir(Sortie::Bail { b: k as u32, bail: None });
            }
        }
    }
}

fn comparer(a: i64, op: u8, b: i64) -> bool {
    match op & !op::NON {
        op::EGAL => a == b,
        op::DIFFERENT => a != b,
        op::SUP => a > b,
        op::INF => a < b,
        _ => false,
    }
}

/// La valeur que l'ordre prouve suit-elle sa dernière commande (revue I5) ? Lu dans la fiche de
/// l'ordre, que le hub tient à chaque variation de la valeur et à chaque issue, par l'ordre des
/// événements (jamais en comparant des horloges).
pub fn vient_de(hub: &Hub, o: Numero) -> u8 {
    let f = hub.catalogue.fiches[o.place()];
    if f.issue == 0 || matches!(f.issue, issue::ECHEC | issue::REFUSE | issue::NON_ENVOYE) || f.preuve & preuve::MODIFIEE != 0 {
        return resultat::NON;
    }
    if f.issue == issue::CONFIRME && f.preuve & preuve::CHANGEMENT != 0 && f.preuve & preuve::PREMIER != 0 {
        return resultat::OUI;
    }
    resultat::INCONNU
}

impl Moteur {
    /// Ce que le moteur garde d'une machine à l'autre : les baux et les exécutions qui attendent une
    /// issue (revue I20 : le redémarrage repart de ces octets, rien d'autre).
    pub fn ranger(&self) -> Vec<u8> {
        let mut b = Vec::new();
        put32(&mut b, self.baux.len() as u32);
        for x in &self.baux {
            put64(&mut b, x.valeur as u64);
            put64(&mut b, x.terme_mur_ms);
            put64(&mut b, x.demande);
            put32(&mut b, x.ordre);
            put32(&mut b, x.execution);
            put16(&mut b, x.generation);
            put16(&mut b, x.mandat);
            b.push(x.etat);
        }
        put32(&mut b, self.ex.len() as u32);
        for x in &self.ex {
            put32(&mut b, x.auto);
            put32(&mut b, x.curseur);
            put64(&mut b, x.demande);
            put32(&mut b, x.empreinte);
            put16(&mut b, x.generation);
            b.extend_from_slice(&[x.etat, u8::from(x.appele), x.preuve, u8::from(x.bail)]);
            put16(&mut b, x.de);
            put16(&mut b, x.pour);
            put32(&mut b, x.bail_duree_s);
            put64(&mut b, x.bail_avant as u64);
        }
        b
    }

    pub fn relire(mut b: &[u8], de: u16) -> Option<Moteur> {
        let b = &mut b;
        let mut m = Moteur { actif: true, de, ..Moteur::default() };
        for _ in 0..get32(b)? {
            let (valeur, terme_mur_ms, demande) = (get64(b)? as i64, get64(b)?, get64(b)?);
            let (ordre, execution) = (get32(b)?, get32(b)?);
            let (generation, mandat) = (get16(b)?, get16(b)?);
            let etat = take(b, 1)?[0];
            m.baux.push(Bail { valeur, terme_mur_ms, demande, ordre, execution, generation, mandat, etat, _r: [0; 3] });
        }
        for _ in 0..get32(b)? {
            let (auto, curseur, demande, empreinte) = (get32(b)?, get32(b)?, get64(b)?, get32(b)?);
            let generation = get16(b)?;
            let q = take(b, 4)?;
            let (de, pour) = (get16(b)?, get16(b)?);
            let (bail_duree_s, bail_avant) = (get32(b)?, get64(b)? as i64);
            m.ex.push(Execution { auto, curseur, demande, empreinte, generation, etat: q[0], appele: q[1] == 1, de, pour, preuve: q[2], bail: q[3] == 1, bail_duree_s, bail_avant });
        }
        Some(m)
    }
}

/// L'empreinte d'un automatisme validé : son nom, son déclencheur et ses pas.
pub fn empreinte_def(d: &Def) -> u32 {
    let mut h = Fnv::default();
    h.octets(format!("{}|{:?}|{:?}", d.nom, d.declencheur, d.pas).as_bytes());
    (h.0 ^ (h.0 >> 32)) as u32
}

fn csr(listes: &[Vec<u32>]) -> (Vec<u32>, Vec<u32>) {
    let mut debut = Vec::with_capacity(listes.len() + 1);
    let mut index = Vec::new();
    for l in listes {
        debut.push(index.len() as u32);
        index.extend_from_slice(l);
    }
    debut.push(index.len() as u32);
    (debut, index)
}
