//! Le côté module : le kit (la voie de chaque appareil, le suivi) et des appareils simulés.
//!
//! Le kit reçoit `cmd/` avec le numéro de la demande, celui de l'ordre et sa forme d'admission ; il
//! range sa voie par appareil, envoie **une seule fois**, et rend **une** issue avant l'échéance,
//! bornée par le retour de la forme (V7 § 5.6). Une consigne qui attend est remplacée par la suivante
//! du même ordre ; un appui ne l'est jamais. Après `accepte` ou `envoye`, il suit l'appareil dans la
//! fenêtre de suivi et conclut une fois. Seule une lecture **commencée après l'envoi** prouve un
//! ordre (`Confirmations::observe` aujourd'hui, orders.rs:273 ; revue I4).

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque};

use crate::catalogue::{AUCUNE_CLE, Arg, raison, retour};
use crate::demandes::{Ref, issue};
use crate::hub::{Cmd, Hub, T};
use crate::numeros::Numero;

/// La marge du kit avant l'échéance (`REPLY_MARGIN`, moli-runtime orders.rs:18) et le temps qu'il
/// faut au moins pour envoyer (`MIN_LEFT`, orders.rs:21).
pub const MARGE_MS: u64 = 400;
pub const MIN_MS: u64 = 1_000;
/// Une voie garde 8 ordres en attente au plus (V7 § 20.2).
pub const ATTENTE_MAX: usize = 8;

/// Comment un appareil simulé répond à un ordre.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rep {
    /// La valeur réglée revient après `ms` (une lampe Zigbee, une serrure).
    Etat { ms: u64 },
    /// Le pont dit « émis » après `ms` ; aucune valeur (un volet RTS, Telegram).
    Pont { ms: u64 },
    /// Un accusé après `accuse_ms`, puis la valeur après `etat_ms` (un aspirateur, un cloud). La
    /// valeur vaut le paramètre, ou `valeur` si l'ordre n'en a pas.
    Accuse { accuse_ms: u64, etat_ms: u64, valeur: Option<i64> },
    /// En mouvement : des valeurs intermédiaires, puis l'arrêt sur la cible (un volet io).
    Mouvement { accuse_ms: u64, pas_ms: u64, inter: Vec<i64> },
    /// Endormi : aucun accusé ; il reparaît après `ms` avec la valeur (une télé réveillée).
    Reveil { ms: u64 },
    /// Ne répond jamais (une ampoule coupée au mur).
    Muet,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Quoi {
    Accuse { d: Ref, i: u8 },
    /// Une valeur lue ; `lu_ms` : l'heure murale où sa lecture a commencé.
    Valeur { point: Numero, v: i64, mouvement: bool, lu_ms: u64 },
}

/// Les appareils simulés : ce qu'ils feront, et quand.
#[derive(Default)]
pub struct Monde {
    pub reps: BTreeMap<u32, Rep>,
    file: BinaryHeap<Reverse<(u64, u64)>>,
    quoi: BTreeMap<u64, Quoi>,
    rang: u64,
}

impl Monde {
    pub fn prevoir(&mut self, t_ms: u64, q: Quoi) {
        self.rang += 1;
        self.file.push(Reverse((t_ms, self.rang)));
        self.quoi.insert(self.rang, q);
    }
    pub fn prochain(&self) -> Option<u64> {
        self.file.peek().map(|Reverse((t, _))| *t)
    }
    pub fn echus(&mut self, t_ms: u64) -> Vec<Quoi> {
        let mut v = Vec::new();
        while let Some(Reverse((t, r))) = self.file.peek().copied() {
            if t > t_ms {
                break;
            }
            self.file.pop();
            v.push(self.quoi.remove(&r).expect("prévu"));
        }
        v
    }
    /// Les appareils d'un module mort ne répondent plus à rien de ce qui était prévu.
    pub fn oublier(&mut self, appareils: &BTreeSet<u32>, hub: &Hub) {
        let garder: Vec<(u64, u64)> = self.file.iter().map(|Reverse(x)| *x).collect();
        self.file.clear();
        for (t, r) in garder {
            let a = match &self.quoi[&r] {
                Quoi::Accuse { d, .. } => hub.demandes.get(*d).map(|x| hub.app_de_ordre(Numero(x.ordre)) as u32),
                Quoi::Valeur { point, .. } => hub.app_de.get(point.place()).copied().filter(|&a| a != u32::MAX),
            };
            if a.is_some_and(|a| appareils.contains(&a)) {
                self.quoi.remove(&r);
            } else {
                self.file.push(Reverse((t, r)));
            }
        }
    }
}

#[derive(Clone, Debug)]
struct EnVol {
    cmd: Cmd,
    /// Heure murale de l'envoi : une lecture commencée avant ne prouve rien.
    depuis_mur_ms: u64,
}

#[derive(Clone, Debug)]
struct Suivi {
    d: Ref,
    appareil: u32,
    ordre: Numero,
    forme: u16,
    attendu: i64,
    envoye_mur_ms: u64,
    fin_mono_ms: u64,
}

#[derive(Default, Clone, Debug)]
struct Voie {
    en_vol: Option<EnVol>,
    file: VecDeque<Cmd>,
}

/// Le kit des modules : les voies, les suivis, les disjoncteurs réduits à « module mort ».
#[derive(Default)]
pub struct Kit {
    voies: BTreeMap<u32, Voie>,
    suivis: Vec<Suivi>,
    /// Chaque envoi vers un appareil : la preuve qu'aucun ordre ne part deux fois.
    pub envois: Vec<Ref>,
    /// Les arguments de chaque envoi, tels qu'ils ont voyagé sur `cmd/` (pour vérifier qu'aucun
    /// texte privé n'y passe).
    pub args_envoyes: Vec<Vec<Arg>>,
    /// Les textes privés lus par le canal privé, au moment d'envoyer.
    pub prives_lus: Vec<String>,
    pub morts: BTreeSet<u32>,
    /// La plus grande époque vue : un ordre d'une époque plus ancienne est refusé (jeton de clôture).
    pub epoque: u32,
    pub refus_epoque: u32,
}

impl Kit {
    /// Un ordre arrive sur `cmd/` (après la garde du hub).
    pub fn recevoir(&mut self, hub: &mut Hub, monde: &mut Monde, t: T, c: Cmd) {
        if c.epoque < self.epoque {
            self.refus_epoque += 1;
            return;
        }
        self.epoque = c.epoque;
        if self.morts.contains(&c.appareil) {
            return;
        }
        let appui = hub.effet_appui(c.forme);
        let v = self.voies.entry(c.appareil).or_default();
        // Une consigne qui attend cède à la suivante du même ordre, si rien ne les sépare.
        if !appui && let Some(last) = v.file.back() && last.ordre == c.ordre {
            let vieux = v.file.pop_back().expect("consigne");
            hub.issue_module(t, vieux.d, issue::NON_ENVOYE, raison::REMPLACE);
        }
        if v.file.len() >= ATTENTE_MAX {
            hub.issue_module(t, c.d, issue::NON_ENVOYE, raison::VOIE_PLEINE);
            return;
        }
        let a = c.appareil;
        v.file.push_back(c);
        self.avancer(hub, monde, t, a);
    }

    /// Le hub demande de retirer des voies ce qui n'est pas encore parti (un permis révoqué).
    pub fn annuler(&mut self, hub: &mut Hub, t: T, refs: &[Ref]) {
        for v in self.voies.values_mut() {
            let (garde, ote): (VecDeque<Cmd>, VecDeque<Cmd>) = std::mem::take(&mut v.file).into_iter().partition(|c| !refs.contains(&c.d));
            v.file = garde;
            for c in ote {
                hub.issue_module(t, c.d, issue::NON_ENVOYE, raison::DROITS);
            }
        }
    }

    /// Envoie ce qui peut partir dans la voie d'un appareil : un ordre en vol, jamais en retard. Une
    /// voie vide disparaît (V7 § 20.2 : une file n'existe que pour un appareil qui a un ordre).
    fn avancer(&mut self, hub: &mut Hub, monde: &mut Monde, t: T, a: u32) {
        loop {
            let Some(v) = self.voies.get_mut(&a) else { return };
            if v.en_vol.is_some() {
                return;
            }
            let Some(c) = v.file.pop_front() else {
                self.voies.remove(&a);
                return;
            };
            if hub.demandes.get(c.d).is_none() {
                continue; // l'appelant est parti, ou la demande a déjà conclu
            }
            if t.mono_ms + MIN_MS >= c.echeance_ms.saturating_sub(MARGE_MS) {
                hub.issue_module(t, c.d, issue::NON_ENVOYE, raison::SANS_REPONSE);
                continue;
            }
            // Un texte privé : le module le lit par le canal privé, au moment d'envoyer.
            if c.args.iter().any(|x| matches!(x, Arg::P(_))) {
                let texte = hub.contenu_prive(&[c.appareil], c.d).map(str::to_string);
                self.prives_lus.extend(texte);
            }
            self.envois.push(c.d);
            self.args_envoyes.push(c.args.clone());
            v.en_vol = Some(EnVol { cmd: c.clone(), depuis_mur_ms: t.mur_ms });
            Self::simuler(hub, monde, t, &c);
        }
    }

    /// La prochaine échéance du kit, en heure murale : ce qui est en vol, ce qui est suivi.
    pub fn prochaine(&self, t: T) -> Option<u64> {
        let vol = self.voies.values().filter_map(|v| v.en_vol.as_ref()).map(|e| e.cmd.echeance_ms.saturating_sub(MARGE_MS));
        let suivis = self.suivis.iter().map(|s| s.fin_mono_ms);
        vol.chain(suivis).min().map(|m| t.mur_de(m))
    }

    /// L'appareil simulé reçoit l'ordre et prévoit sa réponse, selon le retour de sa forme. Ses
    /// valeurs sont lues à leur heure (des lectures fraîches).
    fn simuler(hub: &Hub, monde: &mut Monde, t: T, c: &Cmd) {
        let rep = monde.reps.get(&c.appareil).cloned().unwrap_or(Rep::Etat { ms: 300 });
        let forme = hub.catalogue.formes[usize::from(c.forme)];
        let p0 = hub.catalogue.params_de(c.forme).first().copied();
        let premier = match c.args.first() {
            Some(Arg::N(v)) => Some(*v),
            _ => None,
        };
        // La valeur que l'ordre fait changer : celle que règle son paramètre, sinon celle qui le prouve.
        let cle = p0.map_or(forme.confirme, |p| if p.regle != AUCUNE_CLE { p.regle } else { forme.confirme });
        let point = hub.point_de(c.ordre, cle);
        let cible = premier.or_else(|| hub.catalogue.attendus_de(c.forme).first().map(|&a| i64::from(a)));
        let latence = match &rep {
            Rep::Etat { ms } | Rep::Pont { ms } | Rep::Reveil { ms } => *ms,
            Rep::Accuse { accuse_ms, .. } | Rep::Mouvement { accuse_ms, .. } => *accuse_ms,
            Rep::Muet => return,
        };
        let valeur = |monde: &mut Monde, quand: u64, v: i64, mouvement: bool| {
            if let Some(p) = point {
                monde.prevoir(quand, Quoi::Valeur { point: p, v, mouvement, lu_ms: quand });
            }
        };
        match forme.retour {
            retour::PONT => monde.prevoir(t.mur_ms + latence, Quoi::Accuse { d: c.d, i: issue::EXECUTE }),
            retour::AUCUN => {}
            r => {
                if r == retour::ACCUSE {
                    monde.prevoir(t.mur_ms + latence, Quoi::Accuse { d: c.d, i: issue::ACCEPTE });
                }
                let Some(v) = cible else { return };
                match rep {
                    Rep::Accuse { etat_ms, valeur: x, .. } => valeur(monde, t.mur_ms + etat_ms, x.unwrap_or(v), false),
                    Rep::Mouvement { accuse_ms, pas_ms, inter } => {
                        let mut k = 1;
                        for x in inter {
                            valeur(monde, t.mur_ms + accuse_ms + k * pas_ms, x, true);
                            k += 1;
                        }
                        valeur(monde, t.mur_ms + accuse_ms + k * pas_ms, v, false);
                    }
                    _ => valeur(monde, t.mur_ms + latence, v, false),
                }
            }
        }
    }

    /// Ce qu'un appareil a fait : un accusé, ou une valeur (lue brute, avant la déduplication).
    pub fn evenement(&mut self, hub: &mut Hub, monde: &mut Monde, t: T, q: Quoi) {
        match q {
            Quoi::Accuse { d, i } => {
                let a = hub.demandes.get(d).map(|x| hub.app_de_ordre(Numero(x.ordre)) as u32);
                if let Some(a) = a
                    && self.voies.get(&a).and_then(|v| v.en_vol.as_ref()).is_some_and(|e| e.cmd.d == d)
                {
                    let e = self.voies.get_mut(&a).expect("voie").en_vol.take().expect("en vol");
                    self.rendre(hub, t, &e, i);
                    self.avancer(hub, monde, t, a);
                }
            }
            Quoi::Valeur { point, v, mouvement, lu_ms } => {
                hub.valeur(t, point, v, mouvement, lu_ms);
                // L'ordre en vol que cette valeur prouve : son issue, une fois. La voie est celle de
                // l'appareil du point ; la lecture doit avoir commencé après l'envoi.
                let a = hub.app_de.get(point.place()).copied().filter(|&a| a != u32::MAX);
                if let Some(a) = a
                    && self.voies.get(&a).and_then(|x| x.en_vol.as_ref()).is_some_and(|e| lu_ms >= e.depuis_mur_ms && Self::prouve(hub, &e.cmd, point, v, mouvement))
                {
                    let e = self.voies.get_mut(&a).expect("voie").en_vol.take().expect("en vol");
                    let i = if hub.catalogue.formes[usize::from(e.cmd.forme)].retour == retour::ETAT { issue::CONFIRME } else { issue::ACCEPTE };
                    self.rendre(hub, t, &e, i);
                    self.avancer(hub, monde, t, a);
                }
                // Un ordre suivi que cette valeur conclut : sa suite, une fois.
                if let Some(k) = self.suivis.iter().position(|s| {
                    lu_ms >= s.envoye_mur_ms
                        && hub.point_de(s.ordre, hub.catalogue.formes[usize::from(s.forme)].confirme) == Some(point)
                        && hub.catalogue.confirme_par(s.forme, s.attendu, v, mouvement)
                }) {
                    let s = self.suivis.remove(k);
                    hub.suite_module(t, s.d, issue::CONFIRME, 0);
                }
            }
        }
    }

    fn prouve(hub: &Hub, c: &Cmd, point: Numero, v: i64, mouvement: bool) -> bool {
        let forme = hub.catalogue.formes[usize::from(c.forme)];
        if forme.retour != retour::ETAT || hub.point_de(c.ordre, forme.confirme) != Some(point) {
            return false;
        }
        let premier = match c.args.first() {
            Some(Arg::N(x)) => *x,
            _ => 0,
        };
        hub.catalogue.confirme_par(c.forme, premier, v, mouvement)
    }

    /// Rendre l'issue d'un ordre en vol, bornée par le retour de sa forme ; suivre après.
    fn rendre(&mut self, hub: &mut Hub, t: T, e: &EnVol, i: u8) {
        let forme = hub.catalogue.formes[usize::from(e.cmd.forme)];
        let i = match (forme.retour, i) {
            (retour::PONT, issue::CONFIRME) => issue::EXECUTE,
            (retour::AUCUN, _) => issue::ENVOYE,
            (_, x) => x,
        };
        hub.issue_module(t, e.cmd.d, i, 0);
        if matches!(i, issue::ACCEPTE | issue::ENVOYE) && forme.suivi_s > 0 {
            let attendu = match e.cmd.args.first() {
                Some(Arg::N(v)) => *v,
                _ => 0,
            };
            self.suivis.push(Suivi {
                d: e.cmd.d,
                appareil: e.cmd.appareil,
                ordre: e.cmd.ordre,
                forme: e.cmd.forme,
                attendu,
                envoye_mur_ms: e.depuis_mur_ms,
                fin_mono_ms: t.mono_ms + u64::from(forme.suivi_s) * 1_000,
            });
        }
    }

    /// L'échéance de ce qui est en vol : `envoye` si l'appareil n'a rien dit (jamais renvoyé).
    pub fn tic(&mut self, hub: &mut Hub, monde: &mut Monde, t: T) {
        let echues: Vec<u32> = self
            .voies
            .iter()
            .filter(|(_, v)| v.en_vol.as_ref().is_some_and(|e| t.mono_ms + MARGE_MS >= e.cmd.echeance_ms))
            .map(|(a, _)| *a)
            .collect();
        for a in echues {
            let e = self.voies.get_mut(&a).expect("voie").en_vol.take().expect("en vol");
            self.rendre(hub, t, &e, issue::ENVOYE);
            self.avancer(hub, monde, t, a);
        }
        self.suivis.retain(|s| s.fin_mono_ms > t.mono_ms);
    }

    /// Le module meurt : rien de ce qu'il tenait ne répond plus ; le hub le dira « incertain ».
    pub fn mourir(&mut self, appareils: &[u32]) {
        for a in appareils {
            self.morts.insert(*a);
            self.voies.remove(a);
        }
        self.suivis.retain(|s| !appareils.contains(&s.appareil));
    }

    pub fn en_vol(&self) -> usize {
        self.voies.values().filter(|v| v.en_vol.is_some()).count() + self.voies.values().map(|v| v.file.len()).sum::<usize>()
    }
}
