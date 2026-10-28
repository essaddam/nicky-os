//! Le flux de sortie du hub actif, et la réplique qui l'applique.
//!
//! L'actif publie ce qu'il a décidé, avec SES instants et un rang `s` sans trou (V7 § 7.3), plus le
//! numéro du point `i` (mémoire v2 § 4.4) :
//! - `val/` : une valeur qui change ;
//! - `vu/` : un rapport identique, qui ne fait avancer que « observé » et « reçu » (le `$vu` des
//!   modules, V7 § 5.2, repris dans le flux du hub) ;
//! - `wf/` (v2) : l'état d'une exécution de workflow (sa cause, son départ, son réveil), dans le MÊME
//!   rang `s` : un seul ordre pour tout ce qui est critique, que le récepteur rétablit.
//!
//! La réplique applique ces sorties dans l'ordre de `s` et ne date rien elle-même : c'est ce qui rend
//! la copie exacte. Seules ses échéances (santé, fraîcheur) sont à elle, en horloge monotone locale.
//! Quand un trou dépasse la file en mémoire de l'actif, la réplique reçoit l'**état complet** (v2) :
//! toute la mémoire répliquée au rang `s0`, construite en mémoire, envoyée par MQTT, sans disque.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use crate::maison::Maison;
use crate::outils::Temps;
use crate::valeurs::{Etat, vie};

/// Une publication du hub actif : `val/`, `vu/` ou `wf/`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Sortie {
    /// Le rang dans le flux du hub, sans trou.
    pub s: u64,
    /// Le numéro du point ; pour `wf/`, la case de l'exécution.
    pub i: u32,
    /// La valeur ; pour `wf/`, le point qui a causé l'exécution.
    pub v: i64,
    /// Observé ; pour `wf/`, le réveil (heure murale, ms).
    pub o_ms: u64,
    /// Reçu ; pour `wf/`, le départ.
    pub r_ms: u64,
    /// Changé ; pour `wf/`, l'instant de la cause.
    pub c_ms: u64,
    /// La qualité ; pour `wf/`, l'état de l'exécution.
    pub q: u8,
    /// Un rapport identique : seuls « observé » et « reçu » avancent.
    pub vu: bool,
    /// L'état d'une exécution de workflow (v2).
    pub wf: bool,
}

/// Taille d'une sortie rangée en binaire (file en mémoire, tronçons, état complet).
pub const OCTETS_BINAIRE: usize = 46;

/// La fiche d'une exécution, telle que la réplique la tient (le reste des 64 o de la mémoire v2 se
/// recalcule ou n'a pas besoin de voyager pour ce banc).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Fiche {
    pub cause: u32,
    pub cause_ms: u64,
    pub depart_ms: u64,
    pub reveil_ms: u64,
    pub etat: u8,
}

/// Les cases d'exécution du banc : une par automatisme (mémoire v2 § 8.4).
pub const CASES: usize = 400;

impl Sortie {
    pub fn sujet(&self, nom: &str) -> String {
        if self.wf {
            format!("moli/v1/wf/{}", self.i)
        } else if self.vu {
            format!("moli/v1/vu/{nom}")
        } else {
            format!("moli/v1/val/{nom}")
        }
    }

    /// La charge JSON telle que le hub la publie (V7 § 7.3, plus `i` de la mémoire v2 § 4.4).
    pub fn charge(&self, b: &mut Vec<u8>) {
        use std::io::Write;
        b.clear();
        if self.wf {
            let _ = write!(b, r#"{{"x":{},"o":{},"r":{},"c":{},"s":{},"i":{},"e":{}}}"#, self.v, self.o_ms, self.r_ms, self.c_ms, self.s, self.i, self.q);
        } else if self.vu {
            let _ = write!(b, r#"{{"o":{},"r":{},"s":{},"i":{}}}"#, self.o_ms, self.r_ms, self.s, self.i);
        } else {
            let _ = write!(b, r#"{{"v":{},"o":{},"r":{},"c":{},"s":{},"i":{}"#, self.v, self.o_ms, self.r_ms, self.c_ms, self.s, self.i);
            match self.q {
                0 => {}
                1 => b.extend_from_slice(br#","q":"u""#),
                _ => b.extend_from_slice(br#","q":"m""#),
            }
            b.push(b'}');
        }
    }

    /// Relit une charge JSON du hub. Rend `None` si elle n'en est pas une.
    pub fn lire(charge: &[u8]) -> Option<Sortie> {
        let mut so = Sortie { vu: true, ..Sortie::default() };
        let (mut a_s, mut a_i) = (false, false);
        let mut k = 0;
        while k < charge.len() {
            if charge[k] != b'"' {
                k += 1;
                continue;
            }
            let cle = *charge.get(k + 1)?;
            if charge.get(k + 2) != Some(&b'"') || charge.get(k + 3) != Some(&b':') {
                return None;
            }
            k += 4;
            if cle == b'q' {
                so.q = match charge.get(k + 1)? {
                    b'u' => 1,
                    _ => 2,
                };
                k += 3;
                continue;
            }
            let neg = charge.get(k) == Some(&b'-');
            if neg {
                k += 1;
            }
            let mut x: u64 = 0;
            let debut = k;
            while k < charge.len() && charge[k].is_ascii_digit() {
                x = x.checked_mul(10)?.checked_add(u64::from(charge[k] - b'0'))?;
                k += 1;
            }
            if k == debut {
                return None;
            }
            let signe = |x: u64| if neg { -(x as i64) } else { x as i64 };
            match cle {
                b'v' => {
                    so.v = signe(x);
                    so.vu = false;
                }
                b'x' => {
                    so.v = signe(x);
                    so.vu = false;
                    so.wf = true;
                }
                b'e' => so.q = u8::try_from(x).ok()?,
                b'o' => so.o_ms = x,
                b'r' => so.r_ms = x,
                b'c' => so.c_ms = x,
                b's' => (so.s, a_s) = (x, true),
                b'i' => (so.i, a_i) = (u32::try_from(x).ok()?, true),
                _ => return None,
            }
        }
        (a_s && a_i).then_some(so)
    }

    /// 46 octets, sans en-tête : la forme de la file en mémoire et des tronçons de rattrapage.
    pub fn ranger(&self, b: &mut Vec<u8>) {
        b.extend_from_slice(&self.s.to_le_bytes());
        b.extend_from_slice(&self.i.to_le_bytes());
        b.extend_from_slice(&self.v.to_le_bytes());
        b.extend_from_slice(&self.o_ms.to_le_bytes());
        b.extend_from_slice(&self.r_ms.to_le_bytes());
        b.extend_from_slice(&self.c_ms.to_le_bytes());
        b.push(self.q);
        b.push(if self.wf { 2 } else { u8::from(self.vu) });
    }

    pub fn relire(b: &[u8]) -> Sortie {
        let u = |a: usize| u64::from_le_bytes(b[a..a + 8].try_into().unwrap());
        Sortie {
            s: u(0),
            i: u32::from_le_bytes(b[8..12].try_into().unwrap()),
            v: u(12) as i64,
            o_ms: u(20),
            r_ms: u(28),
            c_ms: u(36),
            q: b[44],
            vu: b[45] == 1,
            wf: b[45] == 2,
        }
    }

    /// Octets d'un PUBLISH MQTT 3.1.1 en QoS 0 (en-tête fixe, longueur restante, sujet, charge).
    pub fn octets_mqtt(sujet: usize, charge: usize) -> usize {
        let reste = 2 + sujet + charge;
        1 + if reste < 128 { 1 } else if reste < 16_384 { 2 } else { 3 } + reste
    }
}

/// Le hub actif, réduit à ce qui compte ici : la mémoire des valeurs de la v2, les cases
/// d'exécution du moteur, le rang `s` et la suite de ses sorties.
pub struct Actif {
    pub etat: Etat,
    pub moteur: Vec<Fiche>,
    pub s: u64,
    pub journal: Vec<Sortie>,
}

impl Actif {
    pub fn new(m: &Maison) -> Self {
        Actif { etat: Etat::depuis(m), moteur: vec![Fiche::default(); CASES], s: 0, journal: Vec::new() }
    }

    /// Un rapport d'un module : le hub le date avec SON horloge (« une seule horloge de référence
    /// pour toute la maison », V7 § 10.1), l'écrit et le publie.
    pub fn rapport(&mut self, n: u32, valeur: i64, t: Temps) -> Sortie {
        let change = self.etat.rapport(n, valeur, t.mur_ms, t.mono);
        self.s += 1;
        let v = self.etat.vifs[n as usize];
        let so = Sortie { s: self.s, i: n, v: v.valeur, o_ms: v.observe_ms, r_ms: t.mur_ms, c_ms: v.change_ms, q: v.qualite, vu: !change, wf: false };
        self.journal.push(so);
        so
    }

    /// Une exécution démarre et attend 10 min (le moteur la publie, dans le même rang que les valeurs).
    pub fn execution(&mut self, case: u32, cause: u32, cause_ms: u64, t: Temps) -> Sortie {
        self.s += 1;
        let f = Fiche { cause, cause_ms, depart_ms: t.mur_ms, reveil_ms: t.mur_ms + 600_000, etat: 1 };
        self.moteur[case as usize] = f;
        let so = Sortie { s: self.s, i: case, v: i64::from(cause), o_ms: f.reveil_ms, r_ms: f.depart_ms, c_ms: cause_ms, q: f.etat, vu: false, wf: true };
        self.journal.push(so);
        so
    }

    /// Le tronçon `[de, a]` (rangs inclus), pour combler un trou.
    pub fn troncon(&self, de: u64, a: u64) -> &[Sortie] {
        let a = a.min(self.s);
        if de == 0 || de > a {
            return &[];
        }
        &self.journal[(de - 1) as usize..a as usize]
    }
}

/// La réplique : la même mémoire, tenue par les sorties de l'actif.
pub struct Replique {
    pub etat: Etat,
    pub moteur: Vec<Fiche>,
    /// Le dernier rang appliqué.
    pub dernier: u64,
    /// Les sorties arrivées en avance, en attendant le trou.
    pub tampon: BTreeMap<u64, Sortie>,
    pub doublons: u64,
    pub appliquees: u64,
}

/// L'instant `mur_ms` de l'actif, en secondes de l'horloge monotone locale (les deux horloges murales
/// sont tenues par NTP ; l'écart, quelques ms, ne compte pas à la seconde).
pub fn mono_local(t: Temps, mur_ms: u64) -> u32 {
    t.mono.saturating_sub((t.mur_ms.saturating_sub(mur_ms) / 1_000) as u32)
}

impl Replique {
    pub fn new(m: &Maison) -> Self {
        Replique { etat: Etat::depuis(m), moteur: vec![Fiche::default(); CASES], dernier: 0, tampon: BTreeMap::new(), doublons: 0, appliquees: 0 }
    }

    /// Reçoit une sortie. Rend le trou `[de, a]` à demander, s'il s'en ouvre un.
    pub fn recevoir(&mut self, so: Sortie, t: Temps) -> Option<(u64, u64)> {
        if so.s <= self.dernier || self.tampon.contains_key(&so.s) {
            self.doublons += 1;
            return None;
        }
        if so.s != self.dernier + 1 {
            let de = self.dernier + 1;
            let deja = self.tampon.keys().next().copied();
            self.tampon.insert(so.s, so);
            // Un trou n'est demandé qu'une fois : à la première sortie arrivée en avance.
            return if deja.is_none() { Some((de, so.s - 1)) } else { None };
        }
        self.appliquer(so, t);
        while let Some(x) = self.tampon.remove(&(self.dernier + 1)) {
            self.appliquer(x, t);
        }
        None
    }

    fn appliquer(&mut self, so: Sortie, t: Temps) {
        self.dernier = so.s;
        self.appliquees += 1;
        if so.wf {
            self.moteur[so.i as usize] = Fiche { cause: so.v as u32, cause_ms: so.c_ms, depart_ms: so.r_ms, reveil_ms: so.o_ms, etat: so.q };
            return;
        }
        let i = so.i as usize;
        let e = &mut self.etat;
        let v = &mut e.vifs[i];
        if !so.vu {
            // La précédente se déduit : aucune sortie n'a manqué, `s` le garantit.
            v.precedente = v.valeur;
            v.valeur = so.v;
            v.change_ms = so.c_ms;
        }
        v.observe_ms = so.o_ms;
        v.recu_s = (so.r_ms / 1_000) as u32;
        v.qualite = so.q;
        let d = e.defs[i].appareil as usize;
        e.sante[d].contact_s = v.recu_s;
        e.sante[d].vie = vie::VIVANT;
        let recu = mono_local(t, so.r_ms);
        if e.delai[d] != 0 {
            e.echeance[d] = recu.saturating_add(e.delai[d]);
        }
        if e.fraicheur[i] != 0 {
            e.perime[i] = recu.saturating_add(e.fraicheur[i]);
        }
    }

    /// Prend un état complet au rang `s0` (v2) : la mémoire répliquée entière remplace la sienne ;
    /// les sorties gardées au-delà de `s0` s'appliquent ensuite, dans l'ordre.
    pub fn prendre_etat(&mut self, b: &[u8], t: Temps) -> Option<u64> {
        let s0 = lire_etat(b, &mut self.etat, &mut self.moteur, t)?;
        self.dernier = s0;
        self.tampon.retain(|&s, _| s > s0);
        while let Some(x) = self.tampon.remove(&(self.dernier + 1)) {
            self.appliquer(x, t);
        }
        Some(s0)
    }
}

/// Les champs répliqués d'un point, dans un ordre fixe : 38 octets (sans l'octet de remplissage de
/// la fiche, dont le contenu n'est pas défini).
pub fn ranger_vif(e: &Etat, n: usize, b: &mut Vec<u8>) {
    let v = &e.vifs[n];
    b.extend_from_slice(&v.valeur.to_le_bytes());
    b.extend_from_slice(&v.precedente.to_le_bytes());
    b.extend_from_slice(&v.observe_ms.to_le_bytes());
    b.extend_from_slice(&v.change_ms.to_le_bytes());
    b.extend_from_slice(&v.recu_s.to_le_bytes());
    b.push(v.genres);
    b.push(v.qualite);
}

fn ranger_fiche(f: &Fiche, b: &mut Vec<u8>) {
    b.extend_from_slice(&f.cause.to_le_bytes());
    b.extend_from_slice(&f.cause_ms.to_le_bytes());
    b.extend_from_slice(&f.depart_ms.to_le_bytes());
    b.extend_from_slice(&f.reveil_ms.to_le_bytes());
    b.push(f.etat);
}

/// L'état complet au rang `s0` : en-tête (s0, nombre de points, d'appareils, de cases), puis les
/// fiches de valeur (38 o), le contact de chaque appareil (5 o), les cases d'exécution (29 o).
pub fn etat_complet(e: &Etat, moteur: &[Fiche], s0: u64) -> Vec<u8> {
    let mut b = Vec::with_capacity(20 + e.vifs.len() * 38 + e.sante.len() * 5 + moteur.len() * 29);
    b.extend_from_slice(&s0.to_le_bytes());
    b.extend_from_slice(&(e.vifs.len() as u32).to_le_bytes());
    b.extend_from_slice(&(e.sante.len() as u32).to_le_bytes());
    b.extend_from_slice(&(moteur.len() as u32).to_le_bytes());
    for n in 0..e.vifs.len() {
        ranger_vif(e, n, &mut b);
    }
    for s in &e.sante {
        b.extend_from_slice(&s.contact_s.to_le_bytes());
        b.push(s.vie);
    }
    for f in moteur {
        ranger_fiche(f, &mut b);
    }
    b
}

/// Relit un état complet dans une mémoire ; refait les échéances monotones locales. Rend `s0`.
pub fn lire_etat(b: &[u8], e: &mut Etat, moteur: &mut [Fiche], t: Temps) -> Option<u64> {
    let u32_ = |a: usize| -> Option<u32> { Some(u32::from_le_bytes(b.get(a..a + 4)?.try_into().ok()?)) };
    let u64_ = |a: usize| -> Option<u64> { Some(u64::from_le_bytes(b.get(a..a + 8)?.try_into().ok()?)) };
    let s0 = u64_(0)?;
    let (np, na, nc) = (u32_(8)? as usize, u32_(12)? as usize, u32_(16)? as usize);
    if np != e.vifs.len() || na != e.sante.len() || nc != moteur.len() || b.len() != 20 + np * 38 + na * 5 + nc * 29 {
        return None;
    }
    let mut k = 20;
    for n in 0..np {
        let v = &mut e.vifs[n];
        v.valeur = u64_(k)? as i64;
        v.precedente = u64_(k + 8)? as i64;
        v.observe_ms = u64_(k + 16)?;
        v.change_ms = u64_(k + 24)?;
        v.recu_s = u32_(k + 32)?;
        v.genres = b[k + 36];
        v.qualite = b[k + 37];
        k += 38;
    }
    for d in 0..na {
        e.sante[d].contact_s = u32_(k)?;
        e.sante[d].vie = b[k + 4];
        k += 5;
    }
    for f in moteur.iter_mut() {
        *f = Fiche { cause: u32_(k)?, cause_ms: u64_(k + 4)?, depart_ms: u64_(k + 12)?, reveil_ms: u64_(k + 20)?, etat: b[k + 28] };
        k += 29;
    }
    // Les échéances monotones sont à cette machine : elles se refont depuis « reçu », en heure murale.
    for n in 0..np {
        let i = e.defs[n].appareil as usize;
        let recu = mono_local(t, u64::from(e.vifs[n].recu_s) * 1_000);
        if e.delai[i] != 0 {
            e.echeance[i] = e.echeance[i].max(recu.saturating_add(e.delai[i]));
        }
        if e.fraicheur[n] != 0 {
            e.perime[n] = recu.saturating_add(e.fraicheur[n]);
        }
    }
    Some(s0)
}

/// L'empreinte de l'état répliqué, par blocs de `BLOC` points : valeurs, instants, qualité, puis un
/// bloc pour le contact de chaque appareil, puis un bloc pour les cases d'exécution. Les échéances
/// monotones sont exclues : elles appartiennent à chaque machine. Un bloc qui diffère se recopie seul.
pub const BLOC: usize = 256;

pub fn empreintes_moteur(e: &Etat, moteur: &[Fiche]) -> Vec<u32> {
    let mut sortie = Vec::with_capacity(e.vifs.len() / BLOC + 3);
    let mut b = Vec::with_capacity(BLOC * 38);
    for debut in (0..e.vifs.len()).step_by(BLOC) {
        b.clear();
        for n in debut..(debut + BLOC).min(e.vifs.len()) {
            ranger_vif(e, n, &mut b);
        }
        sortie.push(crc32fast::hash(&b));
    }
    b.clear();
    for s in &e.sante {
        b.extend_from_slice(&s.contact_s.to_le_bytes());
        b.push(s.vie);
    }
    sortie.push(crc32fast::hash(&b));
    b.clear();
    for f in moteur {
        ranger_fiche(f, &mut b);
    }
    sortie.push(crc32fast::hash(&b));
    sortie
}

/// Les empreintes de la seule mémoire des valeurs (sans moteur).
pub fn empreintes(e: &Etat) -> Vec<u32> {
    let mut v = empreintes_moteur(e, &[]);
    v.pop();
    v
}

/// L'empreinte de tout l'état répliqué (SHA-256 des empreintes de blocs).
pub fn empreinte_moteur(e: &Etat, moteur: &[Fiche]) -> [u8; 32] {
    let mut h = Sha256::new();
    for x in empreintes_moteur(e, moteur) {
        h.update(x.to_le_bytes());
    }
    h.finalize().into()
}

/// L'empreinte de la seule mémoire des valeurs.
pub fn empreinte(e: &Etat) -> [u8; 32] {
    let mut h = Sha256::new();
    for x in empreintes(e) {
        h.update(x.to_le_bytes());
    }
    h.finalize().into()
}

/// Les fiches qui diffèrent entre deux états : (valeur ou précédente, instant « changé », autre).
pub fn differences(a: &Etat, b: &Etat) -> (u32, u32, u32) {
    let (mut val, mut chg, mut autre) = (0, 0, 0);
    for (x, y) in a.vifs.iter().zip(&b.vifs) {
        if x.valeur != y.valeur || x.precedente != y.precedente {
            val += 1;
        } else if x.change_ms != y.change_ms {
            chg += 1;
        } else if x.observe_ms != y.observe_ms || x.recu_s != y.recu_s || x.qualite != y.qualite {
            autre += 1;
        }
    }
    (val, chg, autre)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn une_charge_se_relit_a_l_identique() {
        let mut b = Vec::new();
        for so in [
            Sortie { s: 918_273, i: 1234, v: -2150, o_ms: 1_792_675_624_123, r_ms: 1_792_675_624_125, c_ms: 1_792_675_624_123, ..Sortie::default() },
            Sortie { s: 7, i: 3, v: 1, o_ms: 10, r_ms: 11, c_ms: 9, q: 2, ..Sortie::default() },
            Sortie { s: 8, i: 4, o_ms: 12, r_ms: 13, vu: true, ..Sortie::default() },
            Sortie { s: 9, i: 17, v: 1234, o_ms: 600_100, r_ms: 100, c_ms: 90, q: 1, wf: true, ..Sortie::default() },
        ] {
            so.charge(&mut b);
            assert_eq!(Sortie::lire(&b), Some(so), "{}", String::from_utf8_lossy(&b));
            let mut r = Vec::new();
            so.ranger(&mut r);
            assert_eq!(r.len(), OCTETS_BINAIRE);
            assert_eq!(Sortie::relire(&r), so);
        }
        // La charge de la mémoire v2 (§ 4.4) : 124 octets avec son sujet, en QoS 0.
        let so = Sortie { s: 918_273, i: 1234, v: 2150, o_ms: 1_792_675_624_123, r_ms: 1_792_675_624_125, c_ms: 1_792_675_624_123, ..Sortie::default() };
        so.charge(&mut b);
        assert_eq!(Sortie::octets_mqtt("moli/v1/val/salon_thermo/temperature".len(), b.len()), 124);
    }

    fn temps(k: u64) -> Temps {
        Temps { mono: 100 + k as u32, mur_ms: 1_800_000_000_000 + k * 1_000 }
    }

    #[test]
    fn la_replique_comble_un_trou_et_ecarte_les_doublons() {
        let m = crate::maison::generer(1);
        let mut actif = Actif::new(&m);
        let mut rep = Replique::new(&m);
        let mut sorties = Vec::new();
        for k in 0..50u64 {
            sorties.push(actif.rapport((k * 7 % 2264) as u32, k as i64, temps(k)));
            if k % 10 == 3 {
                sorties.push(actif.execution((k % 400) as u32, (k * 7 % 2264) as u32, temps(k).mur_ms, temps(k)));
            }
        }
        let n = sorties.len();
        for so in &sorties[..10] {
            assert_eq!(rep.recevoir(*so, temps(10)), None);
        }
        // 10 à 19 perdues : la première arrivée en avance ouvre le trou, une seule fois.
        assert_eq!(rep.recevoir(sorties[20], temps(21)), Some((11, 20)));
        assert_eq!(rep.recevoir(sorties[21], temps(22)), None);
        for so in actif.troncon(11, 20) {
            rep.recevoir(*so, temps(23));
        }
        for so in &sorties[22..] {
            rep.recevoir(*so, temps(60));
        }
        rep.recevoir(sorties[5], temps(60));
        assert_eq!(rep.dernier, n as u64);
        assert_eq!(rep.doublons, 1);
        assert_eq!(empreinte_moteur(&rep.etat, &rep.moteur), empreinte_moteur(&actif.etat, &actif.moteur));
    }

    #[test]
    fn un_etat_complet_remplace_un_trou_trop_long_sans_disque() {
        let m = crate::maison::generer(1);
        let mut actif = Actif::new(&m);
        let mut rep = Replique::new(&m);
        let mut sorties = Vec::new();
        for k in 0..400u64 {
            sorties.push(actif.rapport((k * 13 % 2264) as u32, (k % 50) as i64, temps(k)));
            if k % 9 == 0 {
                sorties.push(actif.execution((k % 400) as u32, (k * 13 % 2264) as u32, temps(k).mur_ms, temps(k)));
            }
        }
        // La réplique a vu les 30 premières, puis plus rien jusqu'aux dernières.
        for so in &sorties[..30] {
            rep.recevoir(*so, temps(30));
        }
        let fin = sorties.len();
        for so in &sorties[fin - 5..] {
            rep.recevoir(*so, temps(500));
        }
        // L'actif envoie son état complet au rang de son avant-dernière sortie gardée.
        let s0 = sorties[fin - 6].s;
        let mut miroir = Replique::new(&m);
        for so in &sorties[..fin - 5] {
            miroir.recevoir(*so, temps(500));
        }
        let b = etat_complet(&miroir.etat, &miroir.moteur, s0);
        assert_eq!(rep.prendre_etat(&b, temps(500)), Some(s0));
        assert_eq!(rep.dernier, fin as u64);
        assert_eq!(empreinte_moteur(&rep.etat, &rep.moteur), empreinte_moteur(&actif.etat, &actif.moteur));
        // Un état abîmé est refusé.
        assert_eq!(rep.prendre_etat(&b[..b.len() - 1], temps(500)), None);
    }
}
