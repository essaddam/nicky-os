//! Le moteur du temps (mémoire v2, § 10) : des fiches calendaires de taille fixe, rangées par
//! numéro ; une colonne d'échéances en heure murale (ms) ; le réveil au plus proche. Jamais de
//! balayage à la seconde : la prochaine occurrence d'une règle se calcule, une fois.
//!
//! Les règles :
//! - une occurrence à une heure civile qui n'existe pas (passage à l'heure d'été) a lieu au saut ;
//! - une heure civile qui existe deux fois (retour à l'heure d'hiver) : une occurrence a lieu une
//!   seule fois, à la première ; un niveau (plage, planning) vaut pour les deux ;
//! - une période se compte en temps réel (alignée sur l'UTC) : aucun changement d'heure ne la touche ;
//! - une occurrence en retard (panne, saut d'horloge en avant) part si elle a 2 min de retard au
//!   plus, sinon elle est « manquée » ; plusieurs manquées comptent pour une seule alerte ;
//! - jamais deux fois la même occurrence : la dernière partie est gardée (saut d'horloge en
//!   arrière, redémarrage) ;
//! - tant que l'horloge n'est pas sûre, rien de calendaire ne s'arme ; les durées, si ;
//! - un agenda est une table de fiches d'événements (début, fin, numéro ; ni titre ni participant) :
//!   « 10 min avant le début » est une occurrence par événement ; un rappel part tant que son
//!   événement n'a pas commencé, même en retard (événement ajouté tard, panne) ; deux événements à la
//!   même heure font deux rappels ; un événement déplacé ou annulé réarme.

use jiff::civil::{Date, Time};
use jiff::tz::{AmbiguousOffset, TimeZone};
use jiff::{SignedDuration, Timestamp};

/// Le retard au-delà duquel une occurrence est manquée plutôt que rattrapée (décision du 23/10).
pub const RATTRAPAGE_MS: u64 = 120_000;

pub mod genre {
    /// À une heure civile (seconde du jour), certains jours.
    pub const HEURE: u8 = 1;
    /// Au lever ou au coucher du soleil, avec un décalage.
    pub const SOLEIL: u8 = 2;
    /// Toutes les N secondes, en temps réel.
    pub const PERIODE: u8 = 3;
    /// À l'instant (ms) contenu dans une valeur : une fois.
    pub const INSTANT_VALEUR: u8 = 4;
    /// Chaque jour, à l'heure (seconde du jour) contenue dans une valeur.
    pub const HEURE_VALEUR: u8 = 5;
    /// Avant ou après le début (ou la fin) de chaque événement d'un agenda.
    pub const AGENDA: u8 = 6;
    /// Niveau : entre deux heures civiles, certains jours (la partie après minuit appartient à la veille).
    pub const PLAGE: u8 = 10;
    /// Niveau : entre le coucher et le lever du soleil.
    pub const NUIT: u8 = 11;
    /// Niveau : un événement de l'agenda est en cours (dès le décalage avant son début).
    pub const AGENDA_EN_COURS: u8 = 12;
}

/// Un événement d'agenda : 24 octets, sans titre ni participant (comme le pilote ICS d'aujourd'hui).
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct Evenement {
    pub debut_ms: u64,
    pub fin_ms: u64,
    pub agenda: u32,
    /// Le numéro de l'événement dans son agenda, stable d'une lecture à l'autre.
    pub numero: u32,
}

/// Une fiche calendaire : 24 octets.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct Fiche {
    pub genre: u8,
    /// Les jours : lundi = bit 1 … samedi = bit 6, dimanche = bit 0 ; 0 : tous les jours.
    pub jours: u8,
    /// Soleil : 0 au lever, 1 au coucher. Agenda : 0 au début, 1 à la fin.
    pub coucher: u8,
    pub _r: u8,
    /// Heure ou début de plage (seconde du jour) ; période (s) ; point source d'une valeur ; agenda.
    pub a: u32,
    /// Fin de plage (seconde du jour, 86 400 au plus).
    pub b: u32,
    /// Décalage, en secondes : soleil, valeur, agenda (« 10 min avant » : -600).
    pub decalage_s: i32,
    /// Ce que l'occurrence déclenche, numéroté par le propriétaire de la fiche.
    pub cible: u32,
    /// Niveau : le point virtuel tenu par la fiche.
    pub point: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sortie {
    /// Une occurrence part (avec son retard, 0 à l'heure).
    Part { fiche: u32, occurrence_ms: u64, retard_ms: u64 },
    /// Des occurrences sont passées sans partir : la première, et combien.
    Manquee { fiche: u32, occurrence_ms: u64, nombre: u32 },
    /// Un niveau change de valeur.
    Niveau { fiche: u32, point: u32, valeur: i64 },
    /// Une occurrence d'agenda part : l'événement qui la porte.
    Evenement { fiche: u32, occurrence_ms: u64, retard_ms: u64, agenda: u32, numero: u32 },
}

pub struct Calendrier {
    pub tz: TimeZone,
    pub lat: f64,
    pub lon: f64,
    pub fiches: Vec<Fiche>,
    /// La prochaine échéance de chaque fiche, en heure murale (ms) ; u64::MAX : aucune.
    pub prochaine: Vec<u64>,
    /// La dernière occurrence partie (ms) ; 0 : aucune. Rangée pour le redémarrage.
    pub derniere: Vec<u64>,
    /// La valeur courante d'un niveau (-1 : pas encore calculée).
    pub niveau: Vec<i8>,
    /// Faux tant que l'heure murale n'est pas sûre (pas encore synchronisée au démarrage).
    pub horloge_sure: bool,
    /// Agenda : l'événement de la prochaine occurrence et celui de la dernière partie.
    pub prochaine_evt: Vec<u32>,
    pub derniere_evt: Vec<u32>,
    /// Les événements des agendas, tenus par le hub (le module d'agenda les remplace à chaque lecture).
    pub evenements: Vec<Evenement>,
}

fn ts(ms: u64) -> Timestamp {
    Timestamp::from_millisecond(ms as i64).expect("instant valide")
}

fn ms(t: Timestamp) -> u64 {
    t.as_millisecond().max(0) as u64
}

fn jour_ok(jours: u8, d: Date) -> bool {
    let j = d.weekday().to_monday_one_offset() as u8 % 7;
    jours == 0 || jours & (1 << j) != 0
}

/// Lever et coucher du soleil d'une date (approximation de la NOAA, comme `moli-automation/src/sun.rs`).
#[allow(clippy::many_single_char_names)]
pub fn soleil(date: Date, latitude: f64, longitude: f64) -> Option<(u64, u64)> {
    let n = f64::from(date.day_of_year());
    let g = 2.0 * std::f64::consts::PI / 365.0 * (n - 1.0);
    let eqtime = 229.18
        * (0.000_075 + 0.001_868 * g.cos() - 0.032_077 * g.sin() - 0.014_615 * (2.0 * g).cos() - 0.040_849 * (2.0 * g).sin());
    let decl = 0.006_918 - 0.399_912 * g.cos() + 0.070_257 * g.sin() - 0.006_758 * (2.0 * g).cos() + 0.000_907 * (2.0 * g).sin()
        - 0.002_697 * (3.0 * g).cos()
        + 0.001_48 * (3.0 * g).sin();
    let lat = latitude.to_radians();
    let cos_ha = 90.833_f64.to_radians().cos() / (lat.cos() * decl.cos()) - lat.tan() * decl.tan();
    if !(-1.0..=1.0).contains(&cos_ha) {
        return None;
    }
    let ha = cos_ha.acos().to_degrees();
    let minuit = date.to_zoned(TimeZone::UTC).ok()?.timestamp();
    let a = |minutes: f64| minuit.checked_add(SignedDuration::from_secs((minutes * 60.0).round() as i64)).ok().map(ms);
    Some((a(720.0 - 4.0 * (longitude + ha) - eqtime)?, a(720.0 - 4.0 * (longitude - ha) - eqtime)?))
}

impl Calendrier {
    pub fn new(fuseau: &str, lat: f64, lon: f64) -> Self {
        Calendrier {
            tz: TimeZone::get(fuseau).expect("fuseau connu"),
            lat,
            lon,
            fiches: Vec::new(),
            prochaine: Vec::new(),
            derniere: Vec::new(),
            niveau: Vec::new(),
            horloge_sure: true,
            prochaine_evt: Vec::new(),
            derniere_evt: Vec::new(),
            evenements: Vec::new(),
        }
    }

    pub fn ajouter(&mut self, f: Fiche) -> u32 {
        self.fiches.push(f);
        self.prochaine.push(u64::MAX);
        self.derniere.push(0);
        self.niveau.push(-1);
        self.prochaine_evt.push(0);
        self.derniere_evt.push(0);
        (self.fiches.len() - 1) as u32
    }

    fn date_locale(&self, t_ms: u64) -> Date {
        ts(t_ms).to_zoned(self.tz.clone()).date()
    }

    fn seconde_locale(&self, t_ms: u64) -> (Date, u32) {
        let z = ts(t_ms).to_zoned(self.tz.clone());
        (z.date(), z.hour() as u32 * 3_600 + z.minute() as u32 * 60 + z.second() as u32)
    }

    /// L'instant d'une heure civile pour une occurrence : une heure qui n'existe pas a lieu au saut ;
    /// une heure qui existe deux fois a lieu à la première.
    pub fn instant_occurrence(&self, d: Date, seconde: u32) -> Option<u64> {
        let (d, s) = if seconde >= 86_400 { (d.tomorrow().ok()?, seconde - 86_400) } else { (d, seconde) };
        let civil = d.to_datetime(Time::midnight()).checked_add(SignedDuration::from_secs(i64::from(s))).ok()?;
        let amb = self.tz.to_ambiguous_timestamp(civil);
        let t = match amb.offset() {
            AmbiguousOffset::Unambiguous { .. } => amb.unambiguous().ok()?,
            AmbiguousOffset::Fold { .. } => amb.earlier().ok()?,
            AmbiguousOffset::Gap { .. } => {
                let avant = amb.earlier().ok()?;
                self.tz.following(avant).next()?.timestamp()
            }
        };
        Some(ms(t))
    }

    /// Les instants où une heure civile est franchie, pour un niveau : aucune dans un saut (le saut
    /// lui-même est une limite), les deux dans une heure répétée.
    fn instants_limite(&self, d: Date, seconde: u32) -> Vec<u64> {
        let (d, s) = if seconde >= 86_400 {
            match d.tomorrow() {
                Ok(d) => (d, seconde - 86_400),
                Err(_) => return vec![],
            }
        } else {
            (d, seconde)
        };
        let Ok(civil) = d.to_datetime(Time::midnight()).checked_add(SignedDuration::from_secs(i64::from(s))) else {
            return vec![];
        };
        let amb = self.tz.to_ambiguous_timestamp(civil);
        match amb.offset() {
            AmbiguousOffset::Unambiguous { .. } => amb.unambiguous().ok().map(ms).into_iter().collect(),
            AmbiguousOffset::Fold { .. } => [amb.earlier().ok(), amb.later().ok()].into_iter().flatten().map(ms).collect(),
            AmbiguousOffset::Gap { .. } => vec![],
        }
    }

    /// La prochaine occurrence strictement après `apres_ms`.
    pub fn suivante(&self, f: &Fiche, apres_ms: u64, valeur: &dyn Fn(u32) -> Option<i64>) -> Option<u64> {
        match f.genre {
            genre::PERIODE => {
                let p = u64::from(f.a.max(1)) * 1_000;
                Some((apres_ms / p + 1) * p)
            }
            genre::INSTANT_VALEUR => {
                let v = valeur(f.a)? + i64::from(f.decalage_s) * 1_000;
                (v > 0 && v as u64 > apres_ms).then_some(v as u64)
            }
            genre::HEURE | genre::HEURE_VALEUR | genre::SOLEIL => {
                let seconde = match f.genre {
                    genre::HEURE_VALEUR => u32::try_from(valeur(f.a)?).ok().filter(|s| *s < 86_400)?,
                    _ => f.a,
                };
                let mut d = self.date_locale(apres_ms).yesterday().ok()?;
                for _ in 0..10 {
                    if jour_ok(f.jours, d) {
                        let t = if f.genre == genre::SOLEIL {
                            soleil(d, self.lat, self.lon).map(|(l, c)| {
                                let base = if f.coucher == 1 { c } else { l };
                                (base as i64 + i64::from(f.decalage_s) * 1_000) as u64
                            })
                        } else {
                            self.instant_occurrence(d, seconde).map(|t| (t as i64 + i64::from(f.decalage_s) * 1_000) as u64)
                        };
                        if let Some(t) = t
                            && t > apres_ms
                        {
                            return Some(t);
                        }
                    }
                    d = d.tomorrow().ok()?;
                }
                None
            }
            _ => None,
        }
    }

    fn borne(f: &Fiche, e: &Evenement) -> u64 {
        if f.coucher == 1 { e.fin_ms } else { e.debut_ms }
    }

    /// La prochaine occurrence d'agenda après `apres` (instant, numéro), parmi celles qui ont encore
    /// un objet à `maintenant` : 2 min de retard au plus, ou un événement qui n'a pas commencé.
    pub fn suivante_agenda(&self, f: &Fiche, apres: (u64, u32), maintenant_ms: u64) -> Option<(u64, u32)> {
        self.evenements
            .iter()
            .filter(|e| e.agenda == f.a)
            .filter_map(|e| {
                let b = Self::borne(f, e);
                let i = (b as i64 + i64::from(f.decalage_s) * 1_000).max(0) as u64;
                ((i, e.numero) > apres && (i + RATTRAPAGE_MS >= maintenant_ms || b > maintenant_ms)).then_some((i, e.numero))
            })
            .min()
    }

    /// Le module d'agenda a relu un agenda : ses événements sont remplacés, ses fiches réarmées.
    pub fn agenda(&mut self, agenda: u32, evenements: Vec<Evenement>, maintenant_ms: u64) -> Vec<Sortie> {
        self.evenements.retain(|e| e.agenda != agenda);
        self.evenements.extend(evenements.into_iter().filter(|e| e.agenda == agenda));
        if !self.horloge_sure {
            return Vec::new();
        }
        let ks: Vec<usize> =
            (0..self.fiches.len()).filter(|&k| matches!(self.fiches[k].genre, genre::AGENDA | genre::AGENDA_EN_COURS) && self.fiches[k].a == agenda).collect();
        let mut s: Vec<Sortie> = ks.into_iter().filter_map(|k| self.armer(k, maintenant_ms, &|_| None)).collect();
        s.extend(self.passage(maintenant_ms, &|_| None));
        s
    }

    /// La valeur d'un niveau à un instant.
    pub fn actif(&self, f: &Fiche, t_ms: u64) -> bool {
        match f.genre {
            genre::PLAGE => {
                let (d, s) = self.seconde_locale(t_ms);
                if f.a < f.b {
                    jour_ok(f.jours, d) && f.a <= s && s < f.b
                } else {
                    (jour_ok(f.jours, d) && s >= f.a) || (d.yesterday().is_ok_and(|v| jour_ok(f.jours, v)) && s < f.b)
                }
            }
            genre::NUIT => {
                let d = self.date_locale(t_ms);
                soleil(d, self.lat, self.lon).is_some_and(|(lever, coucher)| t_ms < lever || t_ms >= coucher)
            }
            genre::AGENDA_EN_COURS => self.evenements.iter().any(|e| {
                e.agenda == f.a && (e.debut_ms as i64 + i64::from(f.decalage_s) * 1_000) as u64 <= t_ms && t_ms < e.fin_ms
            }),
            _ => false,
        }
    }

    /// La prochaine limite d'un niveau : une limite de plage ou de soleil, ou un changement d'heure.
    pub fn limite_suivante(&self, f: &Fiche, apres_ms: u64) -> u64 {
        if f.genre == genre::AGENDA_EN_COURS {
            return self
                .evenements
                .iter()
                .filter(|e| e.agenda == f.a)
                .flat_map(|e| [(e.debut_ms as i64 + i64::from(f.decalage_s) * 1_000) as u64, e.fin_ms])
                .filter(|t| *t > apres_ms)
                .min()
                .unwrap_or(u64::MAX);
        }
        let mut meilleur = self.tz.following(ts(apres_ms)).next().map_or(u64::MAX, |t| ms(t.timestamp()));
        let Ok(mut d) = self.date_locale(apres_ms).yesterday() else {
            return meilleur;
        };
        for _ in 0..10 {
            let candidats: Vec<u64> = match f.genre {
                genre::PLAGE => {
                    let fin = if f.a < f.b { f.b } else { f.b + 86_400 };
                    let mut v = self.instants_limite(d, f.a);
                    v.extend(self.instants_limite(d, fin));
                    v
                }
                genre::NUIT => soleil(d, self.lat, self.lon).map(|(l, c)| vec![l, c]).unwrap_or_default(),
                _ => vec![],
            };
            for t in candidats {
                if t > apres_ms {
                    meilleur = meilleur.min(t);
                }
            }
            match d.tomorrow() {
                Ok(x) => d = x,
                Err(_) => break,
            }
        }
        meilleur
    }

    /// Arme une fiche : un niveau prend sa valeur et sa prochaine limite ; une occurrence reprend
    /// après la dernière partie (ou maintenant, si aucune n'est partie) : c'est ce qui rattrape une
    /// occurrence tombée pendant une panne, et ce qui interdit de partir deux fois.
    pub fn armer(&mut self, k: usize, maintenant_ms: u64, valeur: &dyn Fn(u32) -> Option<i64>) -> Option<Sortie> {
        let f = self.fiches[k];
        if f.genre >= genre::PLAGE {
            let v = i8::from(self.actif(&f, maintenant_ms));
            self.prochaine[k] = self.limite_suivante(&f, maintenant_ms);
            if v != self.niveau[k] {
                self.niveau[k] = v;
                return Some(Sortie::Niveau { fiche: k as u32, point: f.point, valeur: i64::from(v) });
            }
            return None;
        }
        if f.genre == genre::AGENDA {
            let apres = (self.derniere[k], self.derniere_evt[k]);
            (self.prochaine[k], self.prochaine_evt[k]) = self.suivante_agenda(&f, apres, maintenant_ms).unwrap_or((u64::MAX, 0));
            return None;
        }
        let apres = if self.derniere[k] > 0 { self.derniere[k] } else { maintenant_ms };
        self.prochaine[k] = self.suivante(&f, apres, valeur).unwrap_or(u64::MAX);
        None
    }

    pub fn armer_tout(&mut self, maintenant_ms: u64, valeur: &dyn Fn(u32) -> Option<i64>) -> Vec<Sortie> {
        if !self.horloge_sure {
            return Vec::new();
        }
        (0..self.fiches.len()).filter_map(|k| self.armer(k, maintenant_ms, valeur)).collect()
    }

    /// Arme les niveaux et les fiches qui n'ont pas d'échéance ; garde celles qui en ont une (rangée
    /// avant une panne, reprise d'une recompilation) : leur retard décidera au passage.
    pub fn armer_manquantes(&mut self, maintenant_ms: u64, valeur: &dyn Fn(u32) -> Option<i64>) -> Vec<Sortie> {
        if !self.horloge_sure {
            return Vec::new();
        }
        let a_armer: Vec<usize> =
            (0..self.fiches.len()).filter(|&k| self.fiches[k].genre >= genre::PLAGE || self.prochaine[k] == u64::MAX).collect();
        a_armer.into_iter().filter_map(|k| self.armer(k, maintenant_ms, valeur)).collect()
    }

    /// Une valeur qui porte une heure a changé : seules les fiches de ce point se réarment.
    pub fn valeur_changee(&mut self, point: u32, maintenant_ms: u64, valeur: &dyn Fn(u32) -> Option<i64>) {
        if !self.horloge_sure {
            return;
        }
        for k in 0..self.fiches.len() {
            let f = self.fiches[k];
            if matches!(f.genre, genre::INSTANT_VALEUR | genre::HEURE_VALEUR) && f.a == point {
                self.prochaine[k] = self.suivante(&f, maintenant_ms.max(self.derniere[k]), valeur).unwrap_or(u64::MAX);
            }
        }
    }

    /// Le réveil au plus proche (heure murale).
    pub fn prochain(&self) -> u64 {
        if !self.horloge_sure {
            return u64::MAX;
        }
        self.prochaine.iter().copied().min().unwrap_or(u64::MAX)
    }

    /// Le passage : les fiches échues seulement.
    pub fn passage(&mut self, maintenant_ms: u64, valeur: &dyn Fn(u32) -> Option<i64>) -> Vec<Sortie> {
        let mut sorties = Vec::new();
        if !self.horloge_sure {
            return sorties;
        }
        for k in 0..self.fiches.len() {
            let p = self.prochaine[k];
            if p > maintenant_ms {
                continue;
            }
            let f = self.fiches[k];
            if f.genre >= genre::PLAGE {
                if let Some(s) = self.armer(k, maintenant_ms, valeur) {
                    sorties.push(s);
                }
                continue;
            }
            if f.genre == genre::AGENDA {
                // Chaque événement est sa propre occurrence : toutes celles qui sont échues partent ou
                // sont manquées, dans l'ordre ; deux événements à la même heure font deux départs.
                while self.prochaine[k] <= maintenant_ms {
                    let (i, num) = (self.prochaine[k], self.prochaine_evt[k]);
                    let a_objet = self.evenements.iter().any(|e| {
                        e.agenda == f.a && e.numero == num && (i + RATTRAPAGE_MS >= maintenant_ms || Self::borne(&f, e) > maintenant_ms)
                    });
                    if a_objet {
                        sorties.push(Sortie::Evenement { fiche: k as u32, occurrence_ms: i, retard_ms: maintenant_ms - i, agenda: f.a, numero: num });
                    } else {
                        sorties.push(Sortie::Manquee { fiche: k as u32, occurrence_ms: i, nombre: 1 });
                    }
                    (self.derniere[k], self.derniere_evt[k]) = (i, num);
                    (self.prochaine[k], self.prochaine_evt[k]) = self.suivante_agenda(&f, (i, num), maintenant_ms).unwrap_or((u64::MAX, 0));
                }
                continue;
            }
            // La plus récente occurrence échue ; celles d'avant sont manquées.
            let (mut occ, mut avant) = (p, 0u32);
            while let Some(n) = self.suivante(&f, occ, valeur).filter(|n| *n <= maintenant_ms) {
                occ = n;
                avant += 1;
                if avant >= 1_000_000 {
                    break;
                }
            }
            let retard = maintenant_ms - occ;
            let manquees = avant + u32::from(retard > RATTRAPAGE_MS);
            if manquees > 0 {
                sorties.push(Sortie::Manquee { fiche: k as u32, occurrence_ms: p, nombre: manquees });
            }
            if retard <= RATTRAPAGE_MS {
                sorties.push(Sortie::Part { fiche: k as u32, occurrence_ms: occ, retard_ms: retard });
            }
            self.derniere[k] = occ;
            self.prochaine[k] = self.suivante(&f, maintenant_ms.max(occ), valeur).unwrap_or(u64::MAX);
        }
        sorties
    }

    /// L'heure murale a sauté (réglage, NTP), ou elle devient sûre. Les niveaux se recalculent ; une
    /// occurrence déjà armée garde son échéance : en avant, le passage la trouve échue et son retard
    /// décide (partie ou manquée) ; en arrière, elle reste après la dernière partie, rien ne repart.
    /// Une fiche jamais armée (horloge pas sûre) s'arme depuis maintenant : rien ne se rattrape d'une
    /// période où l'heure n'était pas sûre.
    pub fn saut(&mut self, maintenant_ms: u64, valeur: &dyn Fn(u32) -> Option<i64>) -> Vec<Sortie> {
        let mut s = self.armer_manquantes(maintenant_ms, valeur);
        s.extend(self.passage(maintenant_ms, valeur));
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(s: &str) -> u64 {
        ms(s.parse::<Timestamp>().unwrap())
    }
    fn paris() -> Calendrier {
        Calendrier::new("Europe/Paris", 48.85, 2.35)
    }
    fn aucune(_: u32) -> Option<i64> {
        None
    }
    fn heure(sec: u32) -> Fiche {
        Fiche { genre: genre::HEURE, a: sec, ..Fiche::default() }
    }
    /// Joue le calendrier de `debut` à `fin`, réveil au plus proche, et rend ce qui est parti.
    fn jouer(c: &mut Calendrier, debut: u64, fin: u64) -> Vec<Sortie> {
        let mut v = c.armer_tout(debut, &aucune);
        loop {
            let t = c.prochain();
            if t > fin {
                break;
            }
            v.extend(c.passage(t, &aucune));
        }
        v
    }
    fn parties(v: &[Sortie]) -> Vec<u64> {
        v.iter().filter_map(|s| if let Sortie::Part { occurrence_ms, .. } = s { Some(*occurrence_ms) } else { None }).collect()
    }

    #[test]
    fn retour_a_l_heure_d_hiver_une_seule_fois_a_la_premiere() {
        // 25 octobre 2026 : à 3 h (heure d'été), il est 2 h (heure d'hiver) ; 2 h 30 existe deux fois.
        let mut c = paris();
        c.ajouter(heure(2 * 3_600 + 30 * 60));
        let v = jouer(&mut c, utc("2026-10-24T22:00:00Z"), utc("2026-10-25T23:00:00Z"));
        assert_eq!(parties(&v), vec![utc("2026-10-25T00:30:00Z")], "une fois, à la première 2 h 30 (UTC+2)");
    }

    #[test]
    fn passage_a_l_heure_d_ete_au_saut() {
        // 28 mars 2027 : à 2 h, il est 3 h ; 2 h 30 n'existe pas : l'occurrence a lieu au saut (3 h, UTC+2).
        let mut c = paris();
        c.ajouter(heure(2 * 3_600 + 30 * 60));
        let v = jouer(&mut c, utc("2027-03-27T22:00:00Z"), utc("2027-03-28T22:00:00Z"));
        assert_eq!(parties(&v), vec![utc("2027-03-28T01:00:00Z")]);
    }

    #[test]
    fn une_periode_compte_le_temps_reel() {
        // Toutes les 15 min entre 0 h et 2 h UTC le 25 octobre : 2 h réelles, 8 occurrences, alors que
        // l'heure locale passe deux fois par 2 h–3 h.
        let mut c = paris();
        c.ajouter(Fiche { genre: genre::PERIODE, a: 900, ..Fiche::default() });
        let v = jouer(&mut c, utc("2026-10-25T00:00:00Z"), utc("2026-10-25T02:00:00Z"));
        assert_eq!(parties(&v).len(), 8);
    }

    #[test]
    fn une_plage_vaut_pour_les_deux_copies_de_l_heure_repetee() {
        let mut c = paris();
        c.ajouter(Fiche { genre: genre::PLAGE, a: 2 * 3_600 + 30 * 60, b: 2 * 3_600 + 45 * 60, point: 7, ..Fiche::default() });
        let v = jouer(&mut c, utc("2026-10-24T22:00:00Z"), utc("2026-10-25T03:00:00Z"));
        let niveaux: Vec<(u64, i64)> = v.iter().filter_map(|s| if let Sortie::Niveau { valeur, .. } = s { Some((0, *valeur)) } else { None }).collect();
        // 0 au départ, puis 1, 0, 1, 0 : deux fois active.
        assert_eq!(niveaux.iter().map(|x| x.1).collect::<Vec<_>>(), vec![0, 1, 0, 1, 0]);
        assert!(c.actif(&c.fiches[0], utc("2026-10-25T00:35:00Z")));
        assert!(c.actif(&c.fiches[0], utc("2026-10-25T01:35:00Z")));
        assert!(!c.actif(&c.fiches[0], utc("2026-10-25T01:00:00Z")));
    }

    #[test]
    fn une_plage_dans_le_saut_ne_s_ouvre_jamais_celle_qui_l_enjambe_s_ouvre_au_saut() {
        let mut c = paris();
        c.ajouter(Fiche { genre: genre::PLAGE, a: 2 * 3_600 + 15 * 60, b: 2 * 3_600 + 45 * 60, point: 1, ..Fiche::default() });
        c.ajouter(Fiche { genre: genre::PLAGE, a: 2 * 3_600 + 30 * 60, b: 3 * 3_600 + 30 * 60, point: 2, ..Fiche::default() });
        let v = jouer(&mut c, utc("2027-03-27T22:00:00Z"), utc("2027-03-28T03:00:00Z"));
        let de = |p: u32| -> Vec<i64> { v.iter().filter_map(|s| if let Sortie::Niveau { point, valeur, .. } = s { (*point == p).then_some(*valeur) } else { None }).collect() };
        assert_eq!(de(1), vec![0]);
        assert_eq!(de(2), vec![0, 1, 0]);
        assert!(c.actif(&c.fiches[1], utc("2027-03-28T01:00:00Z")), "ouverte au saut, 3 h heure d'été");
    }

    #[test]
    fn une_plage_de_nuit_appartient_au_jour_ou_elle_commence() {
        // « Le lundi, de 22 h à 6 h » : la nuit du lundi au mardi, comme un planning hebdomadaire
        // (`moli-core/src/weekly.rs`) ; pas le lundi de 0 h à 6 h, comme la règle horaire d'aujourd'hui
        // (`moli-automation/src/run.rs`, `day_ok`).
        let mut c = paris();
        c.ajouter(Fiche { genre: genre::PLAGE, jours: 1 << 1, a: 22 * 3_600, b: 6 * 3_600, point: 3, ..Fiche::default() });
        let f = c.fiches[0];
        // Le 26 octobre 2026 est un lundi ; Paris est à UTC+1.
        assert!(c.actif(&f, utc("2026-10-26T22:30:00Z")), "lundi 23 h 30");
        assert!(c.actif(&f, utc("2026-10-27T02:00:00Z")), "mardi 3 h");
        assert!(!c.actif(&f, utc("2026-10-26T02:00:00Z")), "lundi 3 h : c'est la nuit du dimanche");
        assert!(!c.actif(&f, utc("2026-10-27T22:30:00Z")), "mardi 23 h 30");
    }

    #[test]
    fn le_coucher_du_soleil_suit_les_jours_et_le_decalage() {
        let mut c = paris();
        c.ajouter(Fiche { genre: genre::SOLEIL, coucher: 1, decalage_s: -30 * 60, ..Fiche::default() });
        let v = jouer(&mut c, utc("2026-10-23T00:00:00Z"), utc("2026-10-27T00:00:00Z"));
        let p = parties(&v);
        assert_eq!(p.len(), 4, "une fois par jour, changement d'heure compris");
        // Chaque occurrence est le coucher du jour moins 30 min, au millième près.
        let mut d = Date::new(2026, 10, 23).unwrap();
        for t in &p {
            let (_, coucher) = soleil(d, 48.85, 2.35).unwrap();
            assert_eq!(*t, coucher - 30 * 60 * 1_000);
            d = d.tomorrow().unwrap();
        }
        // En UTC, le coucher avance de 1 à 3 min par jour ; en heure locale, il recule d'une heure le
        // 25 octobre : le soleil, lui, ne change pas d'heure.
        let local = |t: u64| {
            let z = ts(t).to_zoned(TimeZone::get("Europe/Paris").unwrap());
            z.hour() as i64 * 60 + z.minute() as i64
        };
        for w in p.windows(2) {
            assert!((24 * 3_600 * 1_000 - 4 * 60 * 1_000..24 * 3_600 * 1_000).contains(&(w[1] - w[0])), "{}", w[1] - w[0]);
        }
        assert!((55..=65).contains(&(local(p[1]) - local(p[2]))), "{} → {}", local(p[1]), local(p[2]));
    }

    #[test]
    fn rattrapee_si_moins_de_2_min_de_retard_sinon_manquee() {
        let mut c = paris();
        c.ajouter(heure(7 * 3_600));
        c.ajouter(heure(7 * 3_600 + 3 * 60));
        let debut = utc("2026-10-28T05:00:00Z"); // 6 h locales
        c.armer_tout(debut, &aucune);
        // Panne de 6 h 58 à 7 h 04 : au redémarrage, la fiche 7 h (4 min de retard) est manquée,
        // celle de 7 h 03 (1 min de retard) part.
        let reprise = utc("2026-10-28T06:04:00Z");
        let v = c.passage(reprise, &aucune);
        assert!(v.contains(&Sortie::Manquee { fiche: 0, occurrence_ms: utc("2026-10-28T06:00:00Z"), nombre: 1 }));
        assert!(v.contains(&Sortie::Part { fiche: 1, occurrence_ms: utc("2026-10-28T06:03:00Z"), retard_ms: 60_000 }));
        assert_eq!(c.prochaine[0], utc("2026-10-29T06:00:00Z"));
    }

    #[test]
    fn une_longue_panne_ne_fait_qu_une_alerte() {
        let mut c = paris();
        c.ajouter(Fiche { genre: genre::PERIODE, a: 300, ..Fiche::default() });
        c.armer_tout(utc("2026-10-28T06:00:00Z"), &aucune);
        let v = c.passage(utc("2026-10-28T07:01:00Z"), &aucune);
        // 6 h 05 … 7 h 00 : 12 occurrences échues ; la dernière (1 min de retard) part, 11 manquées.
        assert_eq!(v, vec![
            Sortie::Manquee { fiche: 0, occurrence_ms: utc("2026-10-28T06:05:00Z"), nombre: 11 },
            Sortie::Part { fiche: 0, occurrence_ms: utc("2026-10-28T07:00:00Z"), retard_ms: 60_000 },
        ]);
    }

    #[test]
    fn un_saut_d_horloge_en_arriere_ne_fait_pas_partir_deux_fois() {
        let mut c = paris();
        c.ajouter(heure(7 * 3_600));
        c.armer_tout(utc("2026-10-28T05:30:00Z"), &aucune);
        assert_eq!(parties(&c.passage(utc("2026-10-28T06:00:00Z"), &aucune)).len(), 1);
        // L'horloge recule de 10 min (correction NTP) : 7 h ne repart pas.
        let v = c.saut(utc("2026-10-28T05:52:00Z"), &aucune);
        assert!(parties(&v).is_empty());
        let v = jouer(&mut c, utc("2026-10-28T05:52:00Z"), utc("2026-10-28T23:00:00Z"));
        assert!(parties(&v).is_empty(), "{v:?}");
    }

    #[test]
    fn un_saut_d_horloge_en_avant_rattrape_ou_manque() {
        let mut c = paris();
        c.ajouter(heure(7 * 3_600));
        c.ajouter(heure(7 * 3_600 + 59 * 60));
        c.armer_tout(utc("2026-10-28T05:30:00Z"), &aucune);
        // L'horloge saute de 6 h 30 à 8 h : 7 h a une heure de retard (manquée), 7 h 59 une minute (part).
        let v = c.saut(utc("2026-10-28T07:00:00Z"), &aucune);
        assert!(v.contains(&Sortie::Manquee { fiche: 0, occurrence_ms: utc("2026-10-28T06:00:00Z"), nombre: 1 }));
        assert!(v.contains(&Sortie::Part { fiche: 1, occurrence_ms: utc("2026-10-28T06:59:00Z"), retard_ms: 60_000 }));
    }

    #[test]
    fn rien_de_calendaire_tant_que_l_horloge_n_est_pas_sure() {
        // Un Pi sans pile démarre à l'heure de son dernier arrêt, puis NTP le remet à l'heure.
        let mut c = paris();
        c.ajouter(heure(7 * 3_600));
        c.horloge_sure = false;
        assert!(c.armer_tout(utc("2026-10-20T06:00:00Z"), &aucune).is_empty());
        assert_eq!(c.prochain(), u64::MAX);
        // NTP : il est en fait 9 h le 28. Rien ne part, et rien n'est compté manqué, pour une période
        // où l'heure n'était pas sûre.
        c.horloge_sure = true;
        let v = c.saut(utc("2026-10-28T08:00:00Z"), &aucune);
        assert!(v.is_empty(), "{v:?}");
        assert_eq!(c.prochaine[0], utc("2026-10-29T06:00:00Z"));
    }

    #[test]
    fn une_heure_portee_par_une_valeur_se_rearme_quand_elle_change() {
        let mut c = paris();
        c.ajouter(Fiche { genre: genre::INSTANT_VALEUR, a: 42, ..Fiche::default() });
        let reveil = std::cell::Cell::new(utc("2026-10-28T06:10:00Z") as i64);
        let valeur = |p: u32| (p == 42).then(|| reveil.get());
        c.armer_tout(utc("2026-10-28T05:00:00Z"), &valeur);
        assert_eq!(c.prochain(), utc("2026-10-28T06:10:00Z"));
        reveil.set(utc("2026-10-28T06:20:00Z") as i64);
        c.valeur_changee(42, utc("2026-10-28T05:30:00Z"), &valeur);
        assert_eq!(c.prochain(), utc("2026-10-28T06:20:00Z"));
        let v = c.passage(utc("2026-10-28T06:20:00Z"), &valeur);
        assert_eq!(parties(&v), vec![utc("2026-10-28T06:20:00Z")]);
        assert_eq!(c.prochain(), u64::MAX, "une fois");
        reveil.set(utc("2026-10-28T05:00:00Z") as i64);
        c.valeur_changee(42, utc("2026-10-28T06:30:00Z"), &valeur);
        assert_eq!(c.prochain(), u64::MAX, "une heure passée ne part pas");
    }

    fn evt(debut: &str, fin: &str, numero: u32) -> Evenement {
        Evenement { debut_ms: utc(debut), fin_ms: utc(fin), agenda: 1, numero }
    }

    #[test]
    fn dix_minutes_avant_chaque_evenement() {
        let mut c = paris();
        c.ajouter(Fiche { genre: genre::AGENDA, a: 1, decalage_s: -600, ..Fiche::default() });
        c.horloge_sure = true;
        c.agenda(1, vec![evt("2026-10-28T13:30:00Z", "2026-10-28T14:00:00Z", 1), evt("2026-10-28T15:00:00Z", "2026-10-28T16:00:00Z", 2)], utc("2026-10-28T08:00:00Z"));
        let mut v = Vec::new();
        loop {
            let t = c.prochain();
            if t > utc("2026-10-28T23:00:00Z") {
                break;
            }
            v.extend(c.passage(t, &aucune));
        }
        let rappels: Vec<(u64, u32)> = v.iter().filter_map(|s| if let Sortie::Evenement { occurrence_ms, numero, .. } = s { Some((*occurrence_ms, *numero)) } else { None }).collect();
        assert_eq!(rappels, vec![(utc("2026-10-28T13:20:00Z"), 1), (utc("2026-10-28T14:50:00Z"), 2)]);
    }

    #[test]
    fn deux_evenements_a_la_meme_heure_font_deux_rappels() {
        let mut c = paris();
        c.ajouter(Fiche { genre: genre::AGENDA, a: 1, decalage_s: -600, ..Fiche::default() });
        c.agenda(1, vec![evt("2026-10-28T13:30:00Z", "2026-10-28T14:00:00Z", 1), evt("2026-10-28T13:30:00Z", "2026-10-28T13:45:00Z", 2)], utc("2026-10-28T08:00:00Z"));
        let v = c.passage(utc("2026-10-28T13:20:00Z"), &aucune);
        assert_eq!(v.len(), 2, "{v:?}");
    }

    #[test]
    fn un_evenement_deplace_ou_annule_rearme_le_rappel() {
        let mut c = paris();
        c.ajouter(Fiche { genre: genre::AGENDA, a: 1, decalage_s: -600, ..Fiche::default() });
        c.agenda(1, vec![evt("2026-10-28T15:00:00Z", "2026-10-28T16:00:00Z", 1)], utc("2026-10-28T08:00:00Z"));
        assert_eq!(c.prochain(), utc("2026-10-28T14:50:00Z"));
        // À 14 h, l'événement passe à 17 h : le rappel suit.
        c.agenda(1, vec![evt("2026-10-28T17:00:00Z", "2026-10-28T18:00:00Z", 1)], utc("2026-10-28T14:00:00Z"));
        assert_eq!(c.prochain(), utc("2026-10-28T16:50:00Z"));
        // Puis il est annulé : plus de rappel.
        c.agenda(1, vec![], utc("2026-10-28T15:00:00Z"));
        assert_eq!(c.prochain(), u64::MAX);
    }

    #[test]
    fn un_rappel_tardif_part_tant_que_l_evenement_n_a_pas_commence() {
        let mut c = paris();
        c.ajouter(Fiche { genre: genre::AGENDA, a: 1, decalage_s: -600, ..Fiche::default() });
        // Un rendez-vous ajouté à 14 h 56 pour 15 h : son rappel de 14 h 50 part tout de suite.
        let v = c.agenda(1, vec![evt("2026-10-28T15:00:00Z", "2026-10-28T15:30:00Z", 1)], utc("2026-10-28T14:56:00Z"));
        assert!(matches!(v.as_slice(), [Sortie::Evenement { retard_ms: 360_000, .. }]), "{v:?}");
        // Un rendez-vous déjà commencé n'a plus de rappel.
        let v = c.agenda(1, vec![evt("2026-10-28T14:40:00Z", "2026-10-28T15:30:00Z", 2)], utc("2026-10-28T14:56:00Z"));
        assert!(v.is_empty(), "{v:?}");
    }

    #[test]
    fn un_niveau_en_rendez_vous() {
        let mut c = paris();
        c.ajouter(Fiche { genre: genre::AGENDA_EN_COURS, a: 1, point: 5, ..Fiche::default() });
        c.agenda(1, vec![evt("2026-10-28T13:30:00Z", "2026-10-28T14:00:00Z", 1)], utc("2026-10-28T08:00:00Z"));
        let mut v = Vec::new();
        loop {
            let t = c.prochain();
            if t > utc("2026-10-28T23:00:00Z") {
                break;
            }
            v.extend(c.passage(t, &aucune));
        }
        let niveaux: Vec<i64> = v.iter().filter_map(|s| if let Sortie::Niveau { valeur, .. } = s { Some(*valeur) } else { None }).collect();
        assert_eq!(niveaux, vec![1, 0]);
    }

    #[test]
    fn la_seconde_et_la_milliseconde_sont_tenues() {
        let mut c = paris();
        c.ajouter(heure(7 * 3_600 + 30)); // 7 h 00 min 30 s
        c.ajouter(Fiche { genre: genre::PERIODE, a: 1, ..Fiche::default() }); // chaque seconde
        c.ajouter(Fiche { genre: genre::INSTANT_VALEUR, a: 9, ..Fiche::default() });
        let precis = utc("2026-10-28T06:00:30.250Z") as i64;
        let valeur = |p: u32| (p == 9).then_some(precis);
        let mut v = c.armer_tout(utc("2026-10-28T06:00:25Z"), &valeur);
        loop {
            let t = c.prochain();
            if t > utc("2026-10-28T06:00:35Z") {
                break;
            }
            v.extend(c.passage(t, &valeur));
        }
        let de = |f: u32| -> Vec<u64> { v.iter().filter_map(|s| if let Sortie::Part { fiche, occurrence_ms, .. } = s { (*fiche == f).then_some(*occurrence_ms) } else { None }).collect() };
        assert_eq!(de(0), vec![utc("2026-10-28T06:00:30Z")]);
        assert_eq!(de(1).len(), 10, "26 s … 35 s");
        assert_eq!(de(2), vec![precis as u64], "à la milliseconde");
    }
}
