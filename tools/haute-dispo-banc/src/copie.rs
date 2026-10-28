//! Copier la mémoire par les sorties de l'actif, ou par ses entrées : la question de fond du pont.
//!
//! Une heure de la maison (la cadence de la mémoire v2 : ≈ 1,96 rapport par seconde à ×1) est jouée
//! sur l'actif. Deux secours la suivent par un pont qui est coupé 30 s au milieu de l'heure :
//!
//! - **A, par les sorties** : il reçoit `val/` et `vu/` avec le rang `s` et les instants de l'actif ;
//!   au retour du pont, le premier rang reçu montre le trou, que le journal de l'actif comble ;
//! - **B, par les entrées** : il reçoit les mêmes messages des modules (`in/`) et les traite lui-même,
//!   avec son horloge (NTP à 23 ms près) et le délai du pont ; ce qui passe pendant la coupure est
//!   perdu, aucun rang ne le dit, et un module ne garde pas de journal.
//!
//! À la fin, on compare les deux mémoires à celle de l'actif, et on rejoue la question des attentes
//! de la v2 (§ 8.5) : une attente « depuis 10 min », armée sur l'actif, se revérifie après la bascule
//! par `changé == depuis`. Sur B, l'instant « changé » n'est pas celui de l'actif.

use crate::flux::{Actif, Replique, Sortie, differences, empreinte};
use crate::maison::{Maison, generer};
use crate::outils::{Alea, Temps, ns, resultat};

/// Un rapport d'un module : à l'instant `t_ms` (depuis le début de la période), le point `n` vaut
/// `valeur` ; `module` est la classe de l'appareil (un module par classe).
#[derive(Clone, Copy, Debug)]
pub struct Rapport {
    pub t_ms: u64,
    pub n: u32,
    pub valeur: i64,
}

/// L'heure murale du début des mesures (le 23 octobre 2026, 18:00:00 UTC).
pub const DEBUT_MS: u64 = 1_792_778_400_000;

/// `secondes` de la maison : chaque appareil émet à son rythme (± 20 %), comme l'heure simulée du
/// banc mémoire ; un rapport porte sur un des points de l'appareil, une fois sur deux sans changement.
pub fn rapports(m: &Maison, secondes: u64, graine: u64) -> Vec<Rapport> {
    let mut a = Alea::new(graine);
    let mut valeurs: Vec<i64> = (0..m.points.len()).map(|n| 1_800 + (n % 700) as i64).collect();
    let mut sortie = Vec::new();
    for app in &m.appareils {
        let i = u64::from(app.classe.intervalle_s());
        let mut t = a.sous(i as usize) as u64;
        while t < secondes {
            let n = app.premier + a.sous(app.nb as usize) as u32;
            if a.sous(2) == 0 {
                valeurs[n as usize] += a.sous(41) as i64 - 20;
            }
            sortie.push(Rapport { t_ms: t * 1_000 + a.sous(1_000) as u64, n, valeur: valeurs[n as usize] });
            let j = i / 5;
            t += (i - j + a.sous((2 * j + 1) as usize) as u64).max(1);
        }
    }
    sortie.sort_by_key(|r| r.t_ms);
    sortie
}

pub fn temps_actif(t_ms: u64) -> Temps {
    Temps { mono: 5_000 + (t_ms / 1_000) as u32, mur_ms: DEBUT_MS + t_ms }
}

/// Le bilan d'une heure, pour les deux copies.
pub struct Bilan {
    pub sorties: usize,
    pub vu: usize,
    pub octets_mqtt: usize,
    pub a_perdues_au_pont: usize,
    pub a_trous: usize,
    pub a_identique: bool,
    pub a_differences: (u32, u32, u32),
    pub b_perdues: usize,
    pub b_differences: (u32, u32, u32),
    /// La médiane de l'écart de « changé » sur les valeurs justes.
    pub b_ecart_change_ms: f64,
    /// Les valeurs justes dont « changé » est faux de plus d'une seconde.
    pub b_faux_changements: usize,
    pub attentes: usize,
    pub a_attentes_tenues: usize,
    pub b_attentes_tenues: usize,
}

/// Joue l'heure. `coupure` : la fenêtre (ms) pendant laquelle le pont est coupé. `decalage_ms` :
/// l'écart de l'horloge murale de B. Le délai du pont : 2 ms, plus 0 à 3 ms.
pub fn jouer(m: &Maison, secondes: u64, coupure: (u64, u64), decalage_ms: u64) -> Bilan {
    let rs = rapports(m, secondes, 7);
    let mut actif = Actif::new(m);
    let mut a = Replique::new(m);
    let mut b = Actif::new(m); // B traite les entrées comme un hub : c'est ce qu'on lui demanderait
    let mut alea = Alea::new(11);
    let mut tampon = Vec::new();
    let (mut octets, mut vu, mut a_perdues, mut a_trous, mut b_perdues) = (0, 0, 0, 0, 0);
    let mut sorties_coupees: Vec<Sortie> = Vec::new();
    for r in &rs {
        let so = actif.rapport(r.n, r.valeur, temps_actif(r.t_ms));
        so.charge(&mut tampon);
        octets += Sortie::octets_mqtt(so.sujet(m.points[r.n as usize].nom.as_str()).len(), tampon.len());
        vu += usize::from(so.vu);
        let delai = 2 + alea.sous(4) as u64;
        let arrivee = r.t_ms + delai;
        if arrivee >= coupure.0 && arrivee < coupure.1 {
            // Le pont est coupé : A perd la sortie (session propre), B perd l'entrée.
            a_perdues += 1;
            b_perdues += 1;
            sorties_coupees.push(so);
            continue;
        }
        // A : le premier rang après la coupure montre le trou ; le journal de l'actif le comble.
        let t_a = Temps { mono: 90_000 + (arrivee / 1_000) as u32, mur_ms: DEBUT_MS + arrivee };
        if let Some((de, jusqu_a)) = a.recevoir(so, t_a) {
            a_trous += 1;
            for x in actif.troncon(de, jusqu_a) {
                a.recevoir(*x, t_a);
            }
        }
        // B : il date l'entrée avec son horloge, à l'arrivée.
        let mur_b = DEBUT_MS + arrivee + decalage_ms;
        b.rapport(r.n, r.valeur, Temps { mono: 90_000 + (arrivee / 1_000) as u32, mur_ms: mur_b });
    }
    let _ = sorties_coupees;
    let b_differences = differences(&actif.etat, &b.etat);
    // L'écart de l'instant « changé », pour les valeurs justes : le plus souvent le délai du pont
    // plus l'écart des horloges ; parfois des minutes, quand B a vu un « changement » que l'actif
    // n'avait pas (sa valeur d'avant était fausse, perdue pendant la coupure).
    let mut ecarts: Vec<u64> = actif
        .etat
        .vifs
        .iter()
        .zip(&b.etat.vifs)
        .filter(|(x, y)| x.valeur == y.valeur && x.change_ms != y.change_ms)
        .map(|(x, y)| x.change_ms.abs_diff(y.change_ms))
        .collect();
    ecarts.sort_unstable();
    // Les attentes de la v2 : un déclencheur « depuis 10 min » sur chaque point lu par une règle qui
    // a changé dans les 10 dernières minutes. Armée sur l'actif avec `depuis = changé`, elle se
    // revérifie sur le secours après la bascule.
    let fin = DEBUT_MS + secondes * 1_000;
    let lus: Vec<u32> = m.lus_binaires.iter().chain(&m.lus_mesures).copied().collect();
    let (mut attentes, mut tenues_a, mut tenues_b) = (0, 0, 0);
    for &n in &lus {
        let v = actif.etat.vifs[n as usize];
        if v.change_ms > DEBUT_MS && v.change_ms + 600_000 > fin {
            attentes += 1;
            tenues_a += usize::from(a.etat.vifs[n as usize].change_ms == v.change_ms);
            tenues_b += usize::from(b.etat.vifs[n as usize].change_ms == v.change_ms);
        }
    }
    Bilan {
        sorties: rs.len(),
        vu,
        octets_mqtt: octets,
        a_perdues_au_pont: a_perdues,
        a_trous,
        a_identique: empreinte(&a.etat) == empreinte(&actif.etat),
        a_differences: differences(&actif.etat, &a.etat),
        b_perdues,
        b_differences,
        b_ecart_change_ms: ecarts.get(ecarts.len() / 2).map_or(0.0, |&x| x as f64),
        b_faux_changements: ecarts.iter().filter(|&&x| x > 1_000).count(),
        attentes,
        a_attentes_tenues: tenues_a,
        b_attentes_tenues: tenues_b,
    }
}

pub fn banc(echelle: u32) {
    let m = generer(echelle);
    let sec = "copie";
    // Une heure ; le pont coupé 30 s à la 25e minute ; l'horloge de B à 23 ms de celle de l'actif.
    let bilan = jouer(&m, 3_600, (1_500_000, 1_530_000), 23);
    let r = |cle: &str, v: f64, u: &str| resultat(sec, echelle, cle, v, u);
    r("sorties_heure", bilan.sorties as f64, "messages");
    r("vu_heure", bilan.vu as f64, "messages");
    r("octets_mqtt_heure", bilan.octets_mqtt as f64, "o");
    r("octets_mqtt_jour", (bilan.octets_mqtt * 24) as f64, "o");
    r("octets_journal_jour", (bilan.sorties * 24 * crate::flux::OCTETS_BINAIRE) as f64, "o");
    r("a_perdues_au_pont", bilan.a_perdues_au_pont as f64, "messages");
    r("a_trous_combles", bilan.a_trous as f64, "trous");
    r("a_identique", f64::from(u8::from(bilan.a_identique)), "booléen");
    r("a_fiches_differentes", f64::from(bilan.a_differences.0 + bilan.a_differences.1 + bilan.a_differences.2), "fiches");
    r("b_perdues", bilan.b_perdues as f64, "messages");
    r("b_valeur_differente", f64::from(bilan.b_differences.0), "fiches");
    r("b_change_different", f64::from(bilan.b_differences.1), "fiches");
    r("b_autre_different", f64::from(bilan.b_differences.2), "fiches");
    r("b_ecart_change_ms_median", bilan.b_ecart_change_ms, "ms");
    r("b_faux_changements", bilan.b_faux_changements as f64, "fiches");
    r("attentes", bilan.attentes as f64, "attentes");
    r("a_attentes_tenues", bilan.a_attentes_tenues as f64, "attentes");
    r("b_attentes_tenues", bilan.b_attentes_tenues as f64, "attentes");

    // Les coûts : appliquer une sortie sur la réplique ; l'empreinte de toute la mémoire répliquée.
    let rs = rapports(&m, 3_600, 7);
    let mut actif = Actif::new(&m);
    let sorties: Vec<Sortie> = rs.iter().map(|x| actif.rapport(x.n, x.valeur, temps_actif(x.t_ms))).collect();
    let t = temps_actif(3_600_000);
    let appliquer = ns(sorties.len(), 21, || {
        let mut rep = Replique::new(&m);
        for so in &sorties {
            rep.recevoir(*so, t);
        }
        rep.dernier
    });
    r("appliquer_une_sortie", appliquer, "ns");
    let mut charge = Vec::new();
    let mut tout = Vec::with_capacity(sorties.len() * 90);
    let lire = ns(sorties.len(), 21, || {
        let mut x = 0;
        for so in &sorties {
            so.charge(&mut charge);
            tout.clear();
            tout.extend_from_slice(&charge);
            x += Sortie::lire(&tout).map_or(0, |s| s.s);
        }
        x
    });
    r("ecrire_et_relire_une_charge", lire, "ns");
    let e = ns(1, 21, || empreinte(&actif.etat)[0] as u64);
    r("empreinte", e / 1_000.0, "µs");
    r("empreinte_blocs", crate::flux::empreintes(&actif.etat).len() as f64, "blocs");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_copie_par_les_sorties_est_exacte_celle_par_les_entrees_ne_l_est_pas() {
        let m = generer(1);
        let b = jouer(&m, 3_600, (1_500_000, 1_530_000), 23);
        assert!(b.a_perdues_au_pont > 0, "la coupure doit perdre des sorties");
        assert_eq!(b.a_trous, 1, "un seul trou, comblé une fois");
        assert!(b.a_identique, "A doit être identique à l'actif : {:?}", b.a_differences);
        assert_eq!(b.a_attentes_tenues, b.attentes);
        // B : les valeurs perdues pendant la coupure restent fausses, et ses instants « changé » sont
        // les siens : les attentes armées sur l'actif ne se revérifient plus.
        assert!(b.b_differences.0 > 0);
        assert!(b.b_differences.1 > 100);
        assert!(b.b_attentes_tenues < b.attentes / 10);
    }

    #[test]
    fn sans_coupure_ni_decalage_b_garde_quand_meme_le_delai_du_pont() {
        let m = generer(1);
        let b = jouer(&m, 600, (u64::MAX, u64::MAX), 0);
        assert!(b.a_identique);
        assert_eq!(b.b_differences.0, 0, "sans perte, les valeurs sont justes");
        assert!(b.b_differences.1 > 0, "mais « changé » porte le délai du pont");
    }
}
