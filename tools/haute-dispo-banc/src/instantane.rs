//! L'instantané d'arrêt : toute la mémoire, figée à un rang précis, sur disque, relisible plus tard,
//! par une version plus récente ou par une autre machine.
//!
//! Format (proposé) : un en-tête de 64 octets, puis des sections `(genre u16, longueur u32,
//! crc32 u32, octets)`. Toute échéance y est en heure murale : l'horloge monotone repart de zéro à
//! chaque démarrage (mémoire v2 § 6.5). Les secrets n'y sont pas : ils sont au coffre, et un secret
//! « mémoire seule » ne touche jamais le disque.
//!
//! Écriture comme `StateFile` : fichier voisin, `fsync`, renommage, `fsync` du dossier.

use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::time::Instant;

use crate::copie::{rapports, temps_actif};
use crate::flux::{Actif, empreinte, ranger_vif};
use crate::maison::{Maison, generer};
use crate::outils::{Alea, Temps, resultat};
use crate::valeurs::Etat;

pub const MAGIQUE: &[u8; 4] = b"SHI1";
pub const VERSION: u16 = 1;

/// Pourquoi l'instantané a été pris.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Raison {
    /// Arrêt propre (SIGTERM, onduleur) : la reprise se passe du journal.
    Arret = 1,
    /// Au quart d'heure (V7 § 20.6) : la reprise rejoue le journal après lui.
    Periodique = 2,
    /// Un point de restauration nommé (avant une mise à jour).
    Point = 3,
    /// Le passage de main au secours (bascule planifiée).
    Releve = 4,
}

pub mod genre {
    pub const ANNUAIRE: u16 = 1;
    pub const VALEURS: u16 = 2;
    pub const SANTE: u16 = 3;
    pub const ECHEANCES: u16 = 4;
    pub const EXECUTIONS: u16 = 5;
    pub const ATTENTES: u16 = 6;
    pub const HORAIRES: u16 = 7;
    pub const MINUTERIES: u16 = 8;
    pub const AGENDAS: u16 = 9;
}

/// Ce que le moteur et le hub rangent en plus des valeurs : des fiches de taille fixe (mémoire v2
/// § 8.4, § 8.5, § 10.4, § 10.8, § 10.9). Le banc les remplit au hasard ; leurs champs monotones
/// (réveil d'une exécution, échéance d'une attente, fin d'une minuterie) sont convertis.
pub struct Fiches {
    pub executions: Vec<[u8; 64]>,
    pub attentes: Vec<[u8; 24]>,
    pub horaires: Vec<[u8; 25]>,
    pub minuteries: Vec<[u8; 24]>,
    pub agendas: Vec<[u8; 24]>,
}

impl Fiches {
    pub fn tirer(echelle: u32, t: Temps) -> Fiches {
        let mut a = Alea::new(31);
        let e = echelle as usize;
        let mut f = Fiches { executions: vec![[0; 64]; 400 * e], attentes: vec![[0; 24]; 100 * e], horaires: vec![[0; 25]; 400 * e], minuteries: vec![[0; 24]; 20 * e], agendas: vec![[0; 24]; 150 * e] };
        for x in f.executions.iter_mut() {
            x.iter_mut().for_each(|b| *b = a.suivant() as u8);
            // réveil (octets 8..12) : dans 1 à 3 600 s, en secondes monotones.
            x[8..12].copy_from_slice(&(t.mono + 1 + a.sous(3_600) as u32).to_le_bytes());
        }
        for x in f.attentes.iter_mut() {
            x.iter_mut().for_each(|b| *b = a.suivant() as u8);
            x[0..4].copy_from_slice(&(t.mono + 1 + a.sous(600) as u32).to_le_bytes());
        }
        for x in f.horaires.iter_mut() {
            x.iter_mut().for_each(|b| *b = a.suivant() as u8);
        }
        for x in f.minuteries.iter_mut() {
            x.iter_mut().for_each(|b| *b = a.suivant() as u8);
            x[0..4].copy_from_slice(&(t.mono + 1 + a.sous(7_200) as u32).to_le_bytes());
        }
        for x in f.agendas.iter_mut() {
            x.iter_mut().for_each(|b| *b = a.suivant() as u8);
        }
        f
    }
}

/// Une échéance monotone en heure murale (s), et l'inverse (mémoire v2 § 6.5).
fn vers_mur(t: Temps, mono: u32) -> u32 {
    if mono == u32::MAX { u32::MAX } else { (t.mur_ms / 1_000) as u32 + mono.saturating_sub(t.mono) }
}
fn vers_mono(t: Temps, mur: u32) -> u32 {
    if mur == u32::MAX { u32::MAX } else { t.mono + mur.saturating_sub((t.mur_ms / 1_000) as u32) }
}

fn section(sortie: &mut Vec<u8>, genre: u16, octets: &[u8]) {
    sortie.extend_from_slice(&genre.to_le_bytes());
    sortie.extend_from_slice(&(octets.len() as u32).to_le_bytes());
    sortie.extend_from_slice(&crc32fast::hash(octets).to_le_bytes());
    sortie.extend_from_slice(octets);
}

fn colonne_mur(sortie: &mut Vec<u8>, t: Temps, x: &mut [u8], debut: usize) {
    let m = u32::from_le_bytes(x[debut..debut + 4].try_into().unwrap());
    x[debut..debut + 4].copy_from_slice(&vers_mur(t, m).to_le_bytes());
    sortie.extend_from_slice(x);
}

/// Fige la mémoire au rang `s`, à l'instant `t`.
pub fn figer(e: &Etat, f: &Fiches, s: u64, raison: Raison, t: Temps) -> Vec<u8> {
    let mut b = Vec::with_capacity(e.vifs.len() * 100 + f.executions.len() * 64);
    b.extend_from_slice(MAGIQUE);
    b.extend_from_slice(&VERSION.to_le_bytes());
    b.push(raison as u8);
    b.push(0);
    b.extend_from_slice(&s.to_le_bytes());
    b.extend_from_slice(&t.mur_ms.to_le_bytes());
    b.extend_from_slice(&1u64.to_le_bytes()); // époque
    b.resize(60, 0); // version du programme, empreinte de l'annuaire… (réservé)
    let crc = crc32fast::hash(&b);
    b.extend_from_slice(&crc.to_le_bytes());
    let mut x = Vec::with_capacity(e.vifs.len() * 52);
    for n in 0..e.annuaire.len() {
        let nom = e.annuaire.nom(n as u32).as_bytes();
        x.extend_from_slice(&(nom.len() as u16).to_le_bytes());
        x.extend_from_slice(nom);
    }
    section(&mut b, genre::ANNUAIRE, &x);
    x.clear();
    for n in 0..e.vifs.len() {
        ranger_vif(e, n, &mut x);
    }
    section(&mut b, genre::VALEURS, &x);
    x.clear();
    for s in &e.sante {
        x.extend_from_slice(&s.contact_s.to_le_bytes());
        x.extend_from_slice(&s.change_s.to_le_bytes());
        x.extend_from_slice(&s.appris_s.to_le_bytes());
        x.push(s.vie);
    }
    section(&mut b, genre::SANTE, &x);
    x.clear();
    // Les échéances, en heure murale : appareils, puis points à fraîcheur exigée.
    for &m in e.echeance.iter().chain(&e.perime) {
        x.extend_from_slice(&vers_mur(t, m).to_le_bytes());
    }
    section(&mut b, genre::ECHEANCES, &x);
    for (g, fiches, debut) in [
        (genre::EXECUTIONS, f.executions.iter().map(|y| y.to_vec()).collect::<Vec<_>>(), Some(8)),
        (genre::ATTENTES, f.attentes.iter().map(|y| y.to_vec()).collect(), Some(0)),
        (genre::HORAIRES, f.horaires.iter().map(|y| y.to_vec()).collect(), None),
        (genre::MINUTERIES, f.minuteries.iter().map(|y| y.to_vec()).collect(), Some(0)),
        (genre::AGENDAS, f.agendas.iter().map(|y| y.to_vec()).collect(), None),
    ] {
        x.clear();
        for mut y in fiches {
            match debut {
                Some(d) => colonne_mur(&mut x, t, &mut y, d),
                None => x.extend_from_slice(&y),
            }
        }
        section(&mut b, g, &x);
    }
    b
}

#[derive(Debug, PartialEq, Eq)]
pub enum Refus {
    PasUnInstantane,
    VersionInconnue(u16),
    EnTeteAbime,
    SectionAbimee(u16),
    Tronque,
}

/// Ce qu'on relit : l'en-tête et les sections, vérifiées.
pub struct Relu<'a> {
    pub raison: u8,
    pub s: u64,
    pub mur_ms: u64,
    pub sections: Vec<(u16, &'a [u8])>,
}

pub fn relire(b: &[u8]) -> Result<Relu<'_>, Refus> {
    if b.len() < 64 || &b[0..4] != MAGIQUE {
        return Err(Refus::PasUnInstantane);
    }
    let v = u16::from_le_bytes([b[4], b[5]]);
    if v != VERSION {
        // Une version plus récente du logiciel lit les anciennes ; jamais l'inverse.
        return Err(Refus::VersionInconnue(v));
    }
    if crc32fast::hash(&b[0..60]) != u32::from_le_bytes(b[60..64].try_into().unwrap()) {
        return Err(Refus::EnTeteAbime);
    }
    let mut sections = Vec::new();
    let mut k = 64;
    while k < b.len() {
        if k + 10 > b.len() {
            return Err(Refus::Tronque);
        }
        let g = u16::from_le_bytes([b[k], b[k + 1]]);
        let n = u32::from_le_bytes(b[k + 2..k + 6].try_into().unwrap()) as usize;
        let crc = u32::from_le_bytes(b[k + 6..k + 10].try_into().unwrap());
        let d = b.get(k + 10..k + 10 + n).ok_or(Refus::Tronque)?;
        if crc32fast::hash(d) != crc {
            return Err(Refus::SectionAbimee(g));
        }
        sections.push((g, d));
        k += 10 + n;
    }
    Ok(Relu { raison: b[6], s: u64::from_le_bytes(b[8..16].try_into().unwrap()), mur_ms: u64::from_le_bytes(b[16..24].try_into().unwrap()), sections })
}

/// Rebâtit la mémoire des valeurs depuis un instantané, au démarrage `t` (horloge monotone neuve).
pub fn reprendre(m: &Maison, r: &Relu, t: Temps) -> Etat {
    let mut e = Etat::depuis(m);
    for &(g, d) in &r.sections {
        match g {
            genre::VALEURS => {
                for (n, c) in d.chunks_exact(38).enumerate() {
                    let v = &mut e.vifs[n];
                    v.valeur = i64::from_le_bytes(c[0..8].try_into().unwrap());
                    v.precedente = i64::from_le_bytes(c[8..16].try_into().unwrap());
                    v.observe_ms = u64::from_le_bytes(c[16..24].try_into().unwrap());
                    v.change_ms = u64::from_le_bytes(c[24..32].try_into().unwrap());
                    v.recu_s = u32::from_le_bytes(c[32..36].try_into().unwrap());
                    v.genres = c[36];
                    v.qualite = c[37];
                }
            }
            genre::SANTE => {
                for (k, c) in d.chunks_exact(13).enumerate() {
                    let s = &mut e.sante[k];
                    s.contact_s = u32::from_le_bytes(c[0..4].try_into().unwrap());
                    s.change_s = u32::from_le_bytes(c[4..8].try_into().unwrap());
                    s.appris_s = u32::from_le_bytes(c[8..12].try_into().unwrap());
                    s.vie = c[12];
                }
            }
            genre::ECHEANCES => {
                let nd = e.echeance.len();
                for (k, c) in d.chunks_exact(4).enumerate() {
                    let mono = vers_mono(t, u32::from_le_bytes(c.try_into().unwrap()));
                    if k < nd { e.echeance[k] = mono } else { e.perime[k - nd] = mono }
                }
            }
            _ => {}
        }
    }
    e
}

/// Écrit comme `StateFile` : voisin, fsync, renommage, fsync du dossier.
pub fn ecrire(dossier: &Path, octets: &[u8]) -> std::io::Result<()> {
    let tmp = dossier.join("instantane.tmp");
    let mut f = File::create(&tmp)?;
    f.write_all(octets)?;
    f.sync_all()?;
    fs::rename(&tmp, dossier.join("instantane.bin"))?;
    File::open(dossier)?.sync_all()
}

/// La forme d'aujourd'hui, pour comparer : `state.json` (state_cache.rs), un objet par point.
fn json_aujourdhui(e: &Etat) -> Vec<u8> {
    let mut b = Vec::with_capacity(e.vifs.len() * 90);
    b.push(b'{');
    for n in 0..e.vifs.len() {
        if n > 0 {
            b.push(b',');
        }
        let v = &e.vifs[n];
        let _ = write!(b, r#""{}":{{"value":{},"ts":{}}}"#, e.annuaire.nom(n as u32), v.valeur, v.observe_ms);
    }
    b.push(b'}');
    b
}

pub fn banc(echelle: u32) {
    let sec = "instantane";
    let m = generer(echelle);
    let mut actif = Actif::new(&m);
    for r in rapports(&m, 3_600, 7) {
        actif.rapport(r.n, r.valeur, temps_actif(r.t_ms));
    }
    let t = temps_actif(3_600_000);
    let f = Fiches::tirer(echelle, t);
    let r = |cle: &str, v: f64, u: &str| resultat(sec, echelle, cle, v, u);
    let tours = 21;
    let mediane = |mut v: Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    };
    let mut octets = Vec::new();
    let figer_us = mediane((0..tours).map(|_| {
        let d = Instant::now();
        octets = figer(&actif.etat, &f, actif.s, Raison::Arret, t);
        d.elapsed().as_secs_f64() * 1e6
    }).collect());
    r("octets", octets.len() as f64, "o");
    for (g, nom) in [(genre::ANNUAIRE, "annuaire"), (genre::VALEURS, "valeurs"), (genre::EXECUTIONS, "executions"), (genre::HORAIRES, "horaires")] {
        let rel = relire(&octets).expect("instantané");
        let n = rel.sections.iter().find(|x| x.0 == g).map_or(0, |x| x.1.len());
        r(&format!("octets_{nom}"), n as f64, "o");
    }
    r("figer", figer_us, "µs");
    let dossier = std::env::temp_dir().join(format!("hd-instantane-{echelle}"));
    let _ = fs::create_dir_all(&dossier);
    let ecrire_ms = mediane((0..tours).map(|_| {
        let d = Instant::now();
        ecrire(&dossier, &octets).expect("écriture");
        d.elapsed().as_secs_f64() * 1e3
    }).collect());
    r("ecrire_fsync", ecrire_ms, "ms");
    let t2 = Temps { mono: 7, mur_ms: t.mur_ms + 300_000 };
    let mut repris = None;
    let relire_us = mediane((0..tours).map(|_| {
        let d = Instant::now();
        let lu = fs::read(dossier.join("instantane.bin")).expect("lecture");
        let rel = relire(&lu).expect("instantané");
        repris = Some(reprendre(&m, &rel, t2));
        d.elapsed().as_secs_f64() * 1e6
    }).collect());
    r("relire_et_reprendre", relire_us, "µs");
    r("repris_identique", f64::from(u8::from(repris.as_ref().is_some_and(|e| empreinte(e) == empreinte(&actif.etat)))), "booléen");
    let lz4 = lz4_flex::compress_prepend_size(&octets);
    r("octets_lz4", lz4.len() as f64, "o");
    let json = json_aujourdhui(&actif.etat);
    r("octets_json_valeurs_aujourdhui", json.len() as f64, "o");
    let json_ms = mediane((0..tours).map(|_| {
        let d = Instant::now();
        let j = json_aujourdhui(&actif.etat);
        ecrire(&dossier, &j).expect("écriture");
        d.elapsed().as_secs_f64() * 1e3
    }).collect());
    r("json_valeurs_ecrire_fsync", json_ms, "ms");
    let _ = fs::remove_dir_all(&dossier);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monde() -> (Maison, Actif, Fiches, Temps) {
        let m = generer(1);
        let mut actif = Actif::new(&m);
        for r in rapports(&m, 900, 3) {
            actif.rapport(r.n, r.valeur, temps_actif(r.t_ms));
        }
        let t = temps_actif(900_000);
        let f = Fiches::tirer(1, t);
        (m, actif, f, t)
    }

    #[test]
    fn un_instantane_se_reprend_a_l_identique_sur_une_horloge_neuve() {
        let (m, actif, f, t) = monde();
        let b = figer(&actif.etat, &f, actif.s, Raison::Arret, t);
        let r = relire(&b).expect("instantané");
        assert_eq!((r.raison, r.s, r.mur_ms), (Raison::Arret as u8, actif.s, t.mur_ms));
        // Redémarrage 5 min plus tard : l'horloge monotone repart de 7 s.
        let t2 = Temps { mono: 7, mur_ms: t.mur_ms + 300_000 };
        let e = reprendre(&m, &r, t2);
        assert_eq!(empreinte(&e), empreinte(&actif.etat));
        // Une échéance encore à venir le reste, décalée de ce qui s'est écoulé.
        for (d, &avant) in actif.etat.echeance.iter().enumerate() {
            if avant != u32::MAX && avant > t.mono + 300 {
                assert_eq!(e.echeance[d], t2.mono + (avant - t.mono) - 300, "appareil {d}");
            }
        }
    }

    #[test]
    fn un_instantane_abime_ou_d_une_version_inconnue_est_refuse() {
        let (_, actif, f, t) = monde();
        let b = figer(&actif.etat, &f, actif.s, Raison::Periodique, t);
        let mut abime = b.clone();
        let k = abime.len() - 100;
        abime[k] ^= 1;
        assert!(matches!(relire(&abime), Err(Refus::SectionAbimee(_))));
        let mut futur = b.clone();
        futur[4] = 2;
        assert_eq!(relire(&futur).err(), Some(Refus::VersionInconnue(2)));
        assert_eq!(relire(&b[..b.len() - 1]).err(), Some(Refus::Tronque));
    }
}
