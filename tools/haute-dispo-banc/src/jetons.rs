//! Les jetons qui tournent : un compte cloud, deux machines, un refresh token qui change à chaque
//! renouvellement (Netatmo le fait ; RFC 9700 § 4.14.2, janvier 2025).
//!
//! Le fournisseur suit la RFC : chaque renouvellement rend un nouveau refresh token et invalide le
//! précédent ; s'il revoit un jeton déjà utilisé, il ne sait pas qui l'a présenté et révoque le jeton
//! actif. Le banc joue quatre façons de tenir un compte à deux machines, sur 30 jours, avec un
//! renouvellement toutes les 3 h (la durée d'un jeton d'accès Netatmo).

use std::time::Instant;

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};

use crate::outils::{Alea, resultat};

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Refus {
    /// Le jeton n'est pas le dernier : `invalid_grant`.
    Invalide,
    /// Un jeton déjà utilisé est revenu : toute la famille est révoquée, un humain doit réautoriser.
    Revoque,
}

/// Le fournisseur, selon la RFC 9700 § 4.14.2.
pub struct Fournisseur {
    actif: u64,
    utilises: Vec<u64>,
    revoque: bool,
}

impl Fournisseur {
    pub fn new() -> Self {
        Fournisseur { actif: 1, utilises: Vec::new(), revoque: false }
    }
    pub fn renouveler(&mut self, rt: u64) -> Result<u64, Refus> {
        if self.revoque {
            return Err(Refus::Revoque);
        }
        if rt == self.actif {
            self.utilises.push(rt);
            self.actif += 1;
            return Ok(self.actif);
        }
        if self.utilises.contains(&rt) {
            self.revoque = true;
            return Err(Refus::Revoque);
        }
        Err(Refus::Invalide)
    }
}

const JOUR_MS: u64 = 86_400_000;
const PERIODE_MS: u64 = 3 * 3_600_000;

/// A. Les deux machines renouvellent chacune de leur côté : jours avant la première révocation.
pub fn deux_renouveleurs(a: &mut Alea) -> Option<f64> {
    let mut f = Fournisseur::new();
    // Chacune garde sa copie du jeton ; leurs horloges de renouvellement sont décalées.
    let mut rts = [1u64, 1u64];
    let phases = [0u64, a.sous(PERIODE_MS as usize) as u64];
    let mut evts: Vec<(u64, usize)> = (1..=240u64).flat_map(|k| [(k * PERIODE_MS + phases[0], 0), (k * PERIODE_MS + phases[1], 1)]).collect();
    evts.sort_unstable();
    for (t, m) in evts {
        match f.renouveler(rts[m]) {
            Ok(n) => rts[m] = n,
            Err(_) => return Some(t as f64 / JOUR_MS as f64),
        }
    }
    None
}

/// B. L'actif seul renouvelle et copie le nouveau jeton au secours, qui le reçoit `delai_ms` plus
/// tard. Une bascule tombe à un instant tiré au hasard : le secours renouvelle avec sa copie.
/// Rend vrai si le secours réussit.
pub fn un_renouveleur(a: &mut Alea, delai_ms: u64, hors_ligne: &[(u64, u64)]) -> bool {
    let mut f = Fournisseur::new();
    let mut actif = 1u64;
    // (instant d'arrivée chez le secours, jeton)
    let mut copies: Vec<(u64, u64)> = vec![(0, 1)];
    let bascule = a.sous((30 * JOUR_MS) as usize) as u64;
    let mut t = PERIODE_MS;
    while t < bascule {
        actif = f.renouveler(actif).expect("l'actif seul renouvelle");
        // La copie part aussitôt ; un secours hors ligne la reçoit à son retour (le journal).
        let mut arrivee = t + delai_ms;
        for &(debut, fin) in hors_ligne {
            if arrivee >= debut && arrivee < fin {
                arrivee = fin;
            }
        }
        copies.push((arrivee, actif));
        t += PERIODE_MS;
    }
    let copie = copies.iter().filter(|c| c.0 <= bascule).max_by_key(|c| c.1).map_or(1, |c| c.1);
    f.renouveler(copie).is_ok()
}

/// D. Un jeton par machine : deux autorisations, deux familles, aucune copie.
pub fn par_machine(a: &mut Alea) -> bool {
    let (mut f1, mut f2) = (Fournisseur::new(), Fournisseur::new());
    let (mut r1, mut r2) = (1, 1);
    for _ in 0..240 {
        if a.sous(2) == 0 {
            r1 = f1.renouveler(r1).unwrap();
        } else {
            r2 = f2.renouveler(r2).unwrap();
        }
    }
    f1.renouveler(r1).is_ok() && f2.renouveler(r2).is_ok()
}

pub fn banc(echelle: u32) {
    let sec = "jetons";
    let r = |cle: &str, v: f64, u: &str| resultat(sec, echelle, cle, v, u);
    let mut a = Alea::new(41);
    let essais = 10_000;
    // A.
    let jours: Vec<f64> = (0..1_000).filter_map(|_| deux_renouveleurs(&mut a)).collect();
    r("a_deux_renouveleurs_revoques", jours.len() as f64, "sur 1 000");
    r("a_jours_avant_revocation_max", jours.iter().copied().fold(0.0, f64::max), "jours");
    // B, réseau local : la copie arrive en 5 ms.
    let ok = (0..essais).filter(|_| un_renouveleur(&mut a, 5, &[])).count();
    r("b_local_echecs", (essais - ok) as f64, "sur 10 000 bascules");
    // B, secours distant : 200 ms de lien, et hors ligne 1 h par jour (une coupure de la box).
    let hors_ligne: Vec<(u64, u64)> = (0..30).map(|j| (j * JOUR_MS + 3 * 3_600_000, j * JOUR_MS + 4 * 3_600_000)).collect();
    let ok = (0..essais).filter(|_| un_renouveleur(&mut a, 200, &hors_ligne)).count();
    r("c_distant_hors_ligne_1h_jour_echecs", (essais - ok) as f64, "sur 10 000 bascules");
    // La part théorique : un renouvellement sur 8 par jour tombe dans l'heure hors ligne.
    // D.
    let ok = (0..1_000).filter(|_| par_machine(&mut a)).count();
    r("d_par_machine_echecs", (1_000 - ok) as f64, "sur 1 000");
    // Le coût de sceller un jeton pour le secours (2 Kio, ChaCha20-Poly1305 comme le coffre).
    let cle = ChaCha20Poly1305::new(&[7u8; 32].into());
    let jeton = vec![0x5au8; 2_048];
    let nonce = Nonce::from_slice(&[1u8; 12]);
    let tours = 2_001;
    let d = Instant::now();
    let mut scelle = Vec::new();
    for _ in 0..tours {
        scelle = cle.encrypt(nonce, jeton.as_slice()).expect("scellement");
    }
    r("sceller_2kio", d.elapsed().as_secs_f64() * 1e6 / tours as f64, "µs");
    let d = Instant::now();
    for _ in 0..tours {
        assert_eq!(cle.decrypt(nonce, scelle.as_slice()).expect("ouverture").len(), 2_048);
    }
    r("ouvrir_2kio", d.elapsed().as_secs_f64() * 1e6 / tours as f64, "µs");
    r("octets_scelles_2kio", scelle.len() as f64, "o");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_fournisseur_revoque_quand_un_vieux_jeton_revient() {
        let mut f = Fournisseur::new();
        let n = f.renouveler(1).unwrap();
        assert_eq!(f.renouveler(1), Err(Refus::Revoque), "le jeton 1 revient : révocation");
        assert_eq!(f.renouveler(n), Err(Refus::Revoque), "même le bon jeton ne passe plus");
    }

    #[test]
    fn deux_renouveleurs_se_font_toujours_revoquer() {
        let mut a = Alea::new(1);
        for _ in 0..50 {
            let j = deux_renouveleurs(&mut a).expect("révoqués");
            assert!(j <= 0.25 + 1e-9, "au plus tard au second renouvellement, 6 h ({j} j)");
        }
    }

    #[test]
    fn un_seul_renouveleur_tient_si_la_copie_arrive() {
        let mut a = Alea::new(2);
        assert!((0..2_000).all(|_| un_renouveleur(&mut a, 0, &[])));
        // Un secours hors ligne tout le mois n'a que le premier jeton : il échoue.
        let mut b = Alea::new(3);
        let echecs = (0..2_000).filter(|_| !un_renouveleur(&mut b, 0, &[(0, 31 * JOUR_MS)])).count();
        assert!(echecs > 1_900);
    }
}
