//! Plusieurs stockages qui reçoivent les mêmes messages. L'état courant (`maison.db`) se refait à
//! l'identique depuis `val/`. L'historique (`moli.db`, D13) non : chaque opération y prend un rang
//! `seq` dans la transaction de son lot, et l'archive exige, pour une plage déjà reçue, exactement
//! les mêmes octets (sinon 409). Deux bases qui dérivent chacune leurs opérations du même flux ne
//! numérotent pas pareil : leurs lots tombent à d'autres instants, leurs heures se closent à leur
//! horloge, et un import fait sur l'une n'existe pas sur l'autre.
//!
//! Le banc dérive les deux bases d'une même heure de la maison, puis essaie la reprise de l'export
//! par le secours après une bascule. Il joue aussi l'autre façon : le secours applique les opérations
//! de l'actif, avec leur rang.

use crate::copie::{rapports, temps_actif};
use crate::flux::{Actif, Sortie};
use crate::maison::generer;
use crate::outils::{Alea, resultat};

/// Une opération de `moli.db` : ce qui part, tel quel, vers l'archive (D13, `export_journal`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op {
    Releve { point: u32, valeur: i64, instant: u64 },
    Heure { compteur: u32, heure: u64 },
    Import { point: u32, instant: u64 },
}

/// Le module base de données : lots de 2 s selon son horloge (`phase_ms`), un relevé par valeur
/// reçue, les heures d'énergie closes à l'heure ronde de son horloge (`decalage_ms`), des imports.
pub fn deriver(arrivees: &[(u64, Sortie)], phase_ms: u64, decalage_ms: u64, imports: &[(u64, Vec<Op>)]) -> Vec<Op> {
    let mut ops = Vec::new();
    let mut k = 0;
    let fin = arrivees.last().map_or(0, |x| x.0);
    let mut lot = phase_ms;
    let mut derniere_heure = (arrivees.first().map_or(0, |x| x.0) + decalage_ms) / 3_600_000;
    while lot <= fin + 2_000 {
        let fin_lot = lot + 2_000;
        while k < arrivees.len() && arrivees[k].0 < fin_lot {
            let so = arrivees[k].1;
            if !so.vu && !so.wf {
                ops.push(Op::Releve { point: so.i, valeur: so.v, instant: so.c_ms });
            }
            k += 1;
        }
        for (quand, lot_import) in imports {
            if *quand >= lot && *quand < fin_lot {
                ops.extend_from_slice(lot_import);
            }
        }
        let heure = (fin_lot + decalage_ms) / 3_600_000;
        if heure > derniere_heure {
            for compteur in 0..10 {
                ops.push(Op::Heure { compteur, heure: derniere_heure });
            }
            derniere_heure = heure;
        }
        lot = fin_lot;
    }
    ops
}

pub struct Bilan {
    pub ops_actif: usize,
    pub ops_secours: usize,
    pub premier_ecart: Option<usize>,
    pub rangs_differents: usize,
    pub reprise_possible: bool,
    pub expedition_identique: bool,
}

pub fn jouer(heures: u64) -> Bilan {
    let m = generer(1);
    let mut actif = Actif::new(&m);
    let rs = rapports(&m, heures * 3_600, 7);
    let mut a = Alea::new(5);
    let debut = crate::copie::DEBUT_MS;
    let mut chez_actif = Vec::new();
    let mut chez_secours = Vec::new();
    for r in &rs {
        let so = actif.rapport(r.n, r.valeur, temps_actif(r.t_ms));
        chez_actif.push((debut + r.t_ms, so));
        chez_secours.push((debut + r.t_ms + 2 + a.sous(4) as u64, so));
    }
    // Un import fait à la main sur l'actif, à la 30e minute (D13 : imports, corrections).
    let import: Vec<Op> = (0..50).map(|k| Op::Import { point: k, instant: debut - 86_400_000 + u64::from(k) * 60_000 }).collect();
    let ops_actif = deriver(&chez_actif, debut % 2_000, 0, &[(debut + 1_800_000, import)]);
    let ops_secours = deriver(&chez_secours, (debut + 700) % 2_000, 23, &[]);
    let commun = ops_actif.len().min(ops_secours.len());
    let premier_ecart = (0..commun).find(|&k| ops_actif[k] != ops_secours[k]);
    let rangs_differents = (0..commun).filter(|&k| ops_actif[k] != ops_secours[k]).count() + ops_actif.len().abs_diff(ops_secours.len());
    // Après une bascule à la 40e minute, l'archive a reçu les rangs 1..=K de l'actif. Le secours
    // reprend l'export à K + 1 : il faut que ses K premières opérations soient celles de l'actif.
    let k = ops_actif.len() * 2 / 3;
    let reprise_possible = ops_secours.len() > k && ops_actif[..=k] == ops_secours[..=k];
    // L'autre façon : le secours applique les opérations de l'actif, rang par rang.
    let expediees: Vec<Op> = ops_actif.clone();
    Bilan { ops_actif: ops_actif.len(), ops_secours: ops_secours.len(), premier_ecart, rangs_differents, reprise_possible, expedition_identique: expediees == ops_actif }
}

pub fn banc(echelle: u32) {
    let sec = "stockage";
    let b = jouer(1);
    let r = |cle: &str, v: f64, u: &str| resultat(sec, echelle, cle, v, u);
    r("ops_actif", b.ops_actif as f64, "opérations");
    r("ops_secours_derivees", b.ops_secours as f64, "opérations");
    r("premier_rang_different", b.premier_ecart.map_or(-1.0, |x| x as f64 + 1.0), "rang");
    r("rangs_differents", b.rangs_differents as f64, "rangs");
    r("reprise_export_si_derivee", f64::from(u8::from(b.reprise_possible)), "booléen");
    r("reprise_export_si_expediee", f64::from(u8::from(b.expedition_identique)), "booléen");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn une_base_derivee_ne_peut_pas_reprendre_l_export_une_base_expediee_si() {
        let b = jouer(1);
        assert!(b.premier_ecart.is_some(), "les deux bases devraient numéroter différemment");
        assert!(!b.reprise_possible);
        assert!(b.expedition_identique);
    }

    #[test]
    fn sans_import_ni_ecart_d_horloge_les_releves_seuls_se_suivent() {
        // Le même flux, sans import, mêmes lots, même horloge : les rangs coïncident.
        let m = generer(1);
        let mut actif = Actif::new(&m);
        let arr: Vec<(u64, Sortie)> = rapports(&m, 600, 7).iter().map(|r| (r.t_ms, actif.rapport(r.n, r.valeur, temps_actif(r.t_ms)))).collect();
        assert_eq!(deriver(&arr, 0, 0, &[]), deriver(&arr, 0, 0, &[]));
    }
}
