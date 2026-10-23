//! Les minuteries, aides tenues par le hub (mémoire v2, § 10.6) : l'échéance vit en horloge
//! monotone, qu'aucun réglage de l'heure ne touche ; pour survivre au redémarrage, on range
//! l'échéance murale, et l'on refuse une reprise impossible (il resterait plus que la durée :
//! l'horloge murale a reculé). La règle est celle de `moli-runtime/src/timers.rs` aujourd'hui.

use crate::outils::Temps;

pub mod etat {
    pub const REPOS: u8 = 0;
    pub const EN_COURS: u8 = 1;
    pub const ANNULEE: u8 = 2;
    pub const FINIE: u8 = 3;
    /// Au redémarrage, l'échéance rangée n'a plus de sens : la minuterie ne repart pas seule.
    pub const HORLOGE_INVALIDE: u8 = 4;
}

/// Ce qui est rangé d'une minuterie : 24 octets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct Rangee {
    pub echeance_mur_ms: u64,
    pub duree_s: u32,
    pub generation: u32,
    pub etat: u8,
    pub _r: [u8; 7],
}

/// Les minuteries, en colonnes, par numéro.
#[derive(Default)]
pub struct Minuteries {
    pub duree_s: Vec<u32>,
    /// L'échéance vivante, en secondes monotones (u32::MAX : aucune).
    pub echeance: Vec<u32>,
    pub echeance_mur_ms: Vec<u64>,
    pub generation: Vec<u32>,
    pub etat: Vec<u8>,
}

impl Minuteries {
    pub fn ajouter(&mut self) -> u32 {
        self.duree_s.push(0);
        self.echeance.push(u32::MAX);
        self.echeance_mur_ms.push(0);
        self.generation.push(0);
        self.etat.push(etat::REPOS);
        (self.etat.len() - 1) as u32
    }

    pub fn demarrer(&mut self, k: u32, duree_s: u32, t: Temps) {
        let k = k as usize;
        self.duree_s[k] = duree_s;
        self.echeance[k] = t.mono + duree_s;
        self.echeance_mur_ms[k] = t.mur_ms + u64::from(duree_s) * 1_000;
        self.generation[k] += 1;
        self.etat[k] = etat::EN_COURS;
    }

    pub fn annuler(&mut self, k: u32) {
        let k = k as usize;
        if self.etat[k] == etat::EN_COURS {
            self.etat[k] = etat::ANNULEE;
            self.echeance[k] = u32::MAX;
        }
    }

    /// Le passage : les minuteries échues finissent. Rend leurs numéros.
    pub fn passage(&mut self, t: Temps) -> Vec<u32> {
        let mut finies = Vec::new();
        for k in 0..self.etat.len() {
            if self.etat[k] == etat::EN_COURS && self.echeance[k] <= t.mono {
                self.etat[k] = etat::FINIE;
                self.echeance[k] = u32::MAX;
                finies.push(k as u32);
            }
        }
        finies
    }

    pub fn prochain(&self) -> u32 {
        self.echeance.iter().copied().min().unwrap_or(u32::MAX)
    }

    pub fn ranger(&self) -> Vec<Rangee> {
        (0..self.etat.len())
            .map(|k| Rangee {
                echeance_mur_ms: self.echeance_mur_ms[k],
                duree_s: self.duree_s[k],
                generation: self.generation[k],
                etat: self.etat[k],
                _r: [0; 7],
            })
            .collect()
    }

    /// Au redémarrage : le reste se recalcule sur l'heure murale, et se refuse s'il dépasse la durée.
    pub fn restaurer(rangees: &[Rangee], t: Temps) -> Self {
        let mut m = Minuteries::default();
        for r in rangees {
            let k = m.ajouter() as usize;
            m.duree_s[k] = r.duree_s;
            m.generation[k] = r.generation;
            m.echeance_mur_ms[k] = r.echeance_mur_ms;
            m.etat[k] = r.etat;
            if r.etat == etat::EN_COURS {
                let reste_ms = r.echeance_mur_ms.saturating_sub(t.mur_ms);
                if reste_ms > u64::from(r.duree_s) * 1_000 {
                    m.etat[k] = etat::HORLOGE_INVALIDE;
                } else {
                    m.echeance[k] = t.mono + reste_ms.div_ceil(1_000) as u32;
                }
            }
        }
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MUR: u64 = 1_792_778_400_000;

    #[test]
    fn l_echeance_suit_l_horloge_monotone() {
        let mut m = Minuteries::default();
        let k = m.ajouter();
        m.demarrer(k, 600, Temps { mono: 1_000, mur_ms: MUR });
        // L'heure murale saute d'une heure en avant : la minuterie ne finit pas plus tôt.
        assert!(m.passage(Temps { mono: 1_300, mur_ms: MUR + 3_900_000 }).is_empty());
        assert_eq!(m.passage(Temps { mono: 1_600, mur_ms: MUR + 4_200_000 }), vec![k]);
    }

    #[test]
    fn elle_reprend_apres_un_redemarrage_avec_ce_qui_reste() {
        let mut m = Minuteries::default();
        let k = m.ajouter();
        m.demarrer(k, 600, Temps { mono: 1_000, mur_ms: MUR });
        let rangees = m.ranger();
        // Redémarrage 7 min plus tard : l'horloge monotone repart de zéro.
        let mut r = Minuteries::restaurer(&rangees, Temps { mono: 20, mur_ms: MUR + 420_000 });
        assert_eq!(r.echeance[0], 200, "il reste 3 min");
        assert!(r.passage(Temps { mono: 199, mur_ms: MUR + 599_000 }).is_empty());
        assert_eq!(r.passage(Temps { mono: 200, mur_ms: MUR + 600_000 }), vec![k]);
    }

    #[test]
    fn une_heure_murale_qui_a_recule_rend_la_reprise_impossible() {
        let mut m = Minuteries::default();
        let k = m.ajouter();
        m.demarrer(k, 600, Temps { mono: 1_000, mur_ms: MUR });
        // Au redémarrage, l'heure murale a reculé de 2 h (pile vide, pas encore NTP) : il resterait
        // 2 h 10 pour une minuterie de 10 min. Elle ne repart pas seule.
        let r = Minuteries::restaurer(&m.ranger(), Temps { mono: 20, mur_ms: MUR - 7_200_000 });
        assert_eq!(r.etat[k as usize], etat::HORLOGE_INVALIDE);
        assert_eq!(r.prochain(), u32::MAX);
    }

    #[test]
    fn une_minuterie_annulee_ne_finit_pas() {
        let mut m = Minuteries::default();
        let k = m.ajouter();
        m.demarrer(k, 60, Temps { mono: 0, mur_ms: MUR });
        m.annuler(k);
        assert!(m.passage(Temps { mono: 120, mur_ms: MUR + 120_000 }).is_empty());
        assert_eq!(m.etat[k as usize], etat::ANNULEE);
    }
}
