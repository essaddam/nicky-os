//! Trouver ce qui a expiré. Le numéro donne la place d'une fiche ; il ne dit pas lesquelles sont
//! périmées. Quatre façons de le savoir, jouées sur une heure simulée de la maison.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::hint::black_box;
use std::time::Instant;

use crate::maison::{Classe, generer};
use crate::outils::{Alea, ns, poids, resultat};
use crate::valeurs::{Etat, Sante, vie};

/// A. Le prototype de la V7 : l'échéance se calcule au passage, depuis la fiche de santé.
pub fn passage_calcule(sante: &mut [Sante], maintenant: u32, sortie: &mut Vec<u32>) {
    for (d, s) in sante.iter_mut().enumerate() {
        if s.rythme_s != 0 && s.vie == vie::VIVANT {
            let delai = s.rythme_s.saturating_mul(u32::from(s.pertes) + 1);
            if maintenant > s.contact_s.saturating_add(delai) {
                s.vie = vie::MUET;
                sortie.push(d as u32);
            }
        }
    }
}

/// B. Une colonne d'échéances de 4 octets, écrite à chaque nouvelle ; le passage compare
/// seulement, et rend la prochaine échéance.
pub fn passage_colonne(ech: &mut [u32], maintenant: u32, sortie: &mut Vec<u32>) -> u32 {
    let mut prochaine = u32::MAX;
    for (i, e) in ech.iter_mut().enumerate() {
        if *e < maintenant {
            sortie.push(i as u32);
            *e = u32::MAX;
        } else {
            prochaine = prochaine.min(*e);
        }
    }
    prochaine
}

/// B'. La même colonne, par blocs de 16 : le minimum d'un bloc se calcule en quelques
/// instructions vectorielles ; on ne descend dans le bloc que s'il contient une échéance passée.
pub fn passage_blocs(ech: &mut [u32], maintenant: u32, sortie: &mut Vec<u32>) -> u32 {
    let mut prochaine = u32::MAX;
    for (b, bloc) in ech.chunks_mut(16).enumerate() {
        let mini = bloc.iter().copied().fold(u32::MAX, u32::min);
        if mini < maintenant {
            for (i, e) in bloc.iter_mut().enumerate() {
                if *e < maintenant {
                    sortie.push((b * 16 + i) as u32);
                    *e = u32::MAX;
                } else {
                    prochaine = prochaine.min(*e);
                }
            }
        } else {
            prochaine = prochaine.min(mini);
        }
    }
    prochaine
}

/// C. Une file de priorité : chaque nouvelle empile une échéance ; les entrées dépassées par une
/// nouvelle plus récente sont écartées au passage grâce à une génération par appareil.
pub struct Tas {
    pub tas: BinaryHeap<Reverse<(u32, u32, u32)>>,
    pub generation: Vec<u32>,
}

impl Tas {
    pub fn nouvelle(&mut self, d: u32, echeance: u32) {
        let g = &mut self.generation[d as usize];
        *g = g.wrapping_add(1);
        self.tas.push(Reverse((echeance, d, *g)));
    }
    pub fn passage(&mut self, maintenant: u32, sortie: &mut Vec<u32>) -> u32 {
        while let Some(&Reverse((e, d, g))) = self.tas.peek() {
            if e >= maintenant {
                return e;
            }
            self.tas.pop();
            if self.generation[d as usize] == g {
                sortie.push(d);
            }
        }
        u32::MAX
    }
}

/// Une heure de nouvelles : (seconde, appareil), triées. 2 % des appareils qui émettent se taisent
/// à la 20e minute : ils doivent être déclarés muets à leur échéance, et pas un autre.
fn heure_simulee(echelle: u32) -> (crate::maison::Maison, Vec<(u32, u32)>, Vec<u32>) {
    let m = generer(echelle);
    let mut a = Alea::new(99);
    let mut nouvelles = Vec::new();
    let mut muets_attendus = Vec::new();
    for (d, app) in m.appareils.iter().enumerate() {
        let i = app.classe.intervalle_s();
        let se_tait = app.classe.rythme_s() != 0 && a.sous(50) == 0;
        let mut t = a.sous(i as usize) as u32;
        let mut dernier = 0;
        while t < 3_600 {
            if se_tait && t >= 1_200 {
                break;
            }
            nouvelles.push((t, d as u32));
            dernier = t;
            // ± 20 % autour de l'intervalle, jamais au-delà du rythme déclaré.
            let j = i / 5;
            let pas = i - j + a.sous((2 * j + 1) as usize) as u32;
            let plafond = match app.classe.rythme_s() {
                0 => u32::MAX,
                r => r,
            };
            t += pas.min(plafond).max(1);
        }
        if se_tait {
            let delai = app.classe.rythme_s() * (u32::from(app.classe.pertes()) + 1);
            if dernier + delai < 3_600 || app.classe == Classe::Pile {
                muets_attendus.push(d as u32);
            }
        }
    }
    nouvelles.sort_unstable();
    (m, nouvelles, muets_attendus)
}

pub fn banc(echelle: u32) {
    let sec = "echeances";
    let (m, nouvelles, _) = heure_simulee(echelle);
    resultat(sec, echelle, "nouvelles_par_heure", nouvelles.len() as f64, "");
    let nb_app = m.appareils.len();

    // État de départ commun : contact à 0, toutes les échéances à leur délai.
    let etat0 = Etat::depuis(&m);

    // ---- A : calcul au passage, un passage par seconde ----
    let mut sante = etat0.sante.clone();
    let mut muets_a = Vec::new();
    let debut = Instant::now();
    let mut k = 0;
    let mut cout_nouvelles_a = 0u128;
    for s in 0..3_600u32 {
        let d0 = Instant::now();
        while k < nouvelles.len() && nouvelles[k].0 == s {
            let d = nouvelles[k].1 as usize;
            sante[d].contact_s = s;
            sante[d].vie = vie::VIVANT;
            k += 1;
        }
        cout_nouvelles_a += d0.elapsed().as_nanos();
        passage_calcule(&mut sante, s, &mut muets_a);
    }
    let total_a = debut.elapsed().as_nanos();

    // ---- B : colonne d'échéances, un passage par seconde ----
    let mut ech = etat0.echeance.clone();
    let delai = etat0.delai.clone();
    let mut muets_b = Vec::new();
    let debut = Instant::now();
    let mut k = 0;
    let mut cout_nouvelles_b = 0u128;
    for s in 0..3_600u32 {
        let d0 = Instant::now();
        while k < nouvelles.len() && nouvelles[k].0 == s {
            let d = nouvelles[k].1 as usize;
            if delai[d] != 0 {
                ech[d] = s + delai[d];
            }
            k += 1;
        }
        cout_nouvelles_b += d0.elapsed().as_nanos();
        passage_colonne(&mut ech, s, &mut muets_b);
    }
    let total_b = debut.elapsed().as_nanos();

    // ---- B' + réveil au plus proche : on ne passe que quand une échéance peut tomber ----
    let mut ech = etat0.echeance.clone();
    let mut muets_r = Vec::new();
    let mut prochaine = 0u32;
    let mut passages = 0u32;
    let debut = Instant::now();
    let mut k = 0;
    for s in 0..3_600u32 {
        while k < nouvelles.len() && nouvelles[k].0 == s {
            let d = nouvelles[k].1 as usize;
            if delai[d] != 0 {
                ech[d] = s + delai[d];
                // Une échéance plus proche que le prochain réveil l'avance (un appareil qui revient).
                prochaine = prochaine.min(ech[d]);
            }
            k += 1;
        }
        if s > prochaine {
            prochaine = passage_blocs(&mut ech, s, &mut muets_r);
            passages += 1;
        }
    }
    let total_r = debut.elapsed().as_nanos();

    // ---- C : file de priorité ----
    let (mut tas, o_tas_vide) = poids(|| Tas { tas: BinaryHeap::new(), generation: vec![0; nb_app] });
    for (d, &dl) in delai.iter().enumerate() {
        if dl != 0 {
            tas.nouvelle(d as u32, dl);
        }
    }
    let mut muets_c = Vec::new();
    let mut plus_grand_tas = 0;
    let debut = Instant::now();
    let mut k = 0;
    let mut cout_nouvelles_c = 0u128;
    for s in 0..3_600u32 {
        let d0 = Instant::now();
        while k < nouvelles.len() && nouvelles[k].0 == s {
            let d = nouvelles[k].1 as usize;
            if delai[d] != 0 {
                tas.nouvelle(d as u32, s + delai[d]);
            }
            k += 1;
        }
        cout_nouvelles_c += d0.elapsed().as_nanos();
        tas.passage(s, &mut muets_c);
        plus_grand_tas = plus_grand_tas.max(tas.tas.len());
    }
    let total_c = debut.elapsed().as_nanos();
    black_box(o_tas_vide);

    // Les quatre doivent déclarer les mêmes appareils muets.
    let trier = |v: &mut Vec<u32>| {
        v.sort_unstable();
        v.dedup();
    };
    let (mut a1, mut b1, mut r1, mut c1) = (muets_a, muets_b, muets_r, muets_c);
    for v in [&mut a1, &mut b1, &mut r1, &mut c1] {
        trier(v);
    }
    let accord = a1 == b1 && b1 == r1 && r1 == c1;
    resultat(sec, echelle, "muets_detectes", a1.len() as f64, "");
    resultat(sec, echelle, "accord_des_quatre_methodes", f64::from(u8::from(accord)), "");
    assert!(accord, "les méthodes ne déclarent pas les mêmes muets");

    let nb_n = nouvelles.len() as f64;
    resultat(sec, echelle, "A_heure_totale", total_a as f64 / 1e6, "ms");
    resultat(sec, echelle, "B_heure_totale", total_b as f64 / 1e6, "ms");
    resultat(sec, echelle, "R_heure_totale", total_r as f64 / 1e6, "ms");
    resultat(sec, echelle, "C_heure_totale", total_c as f64 / 1e6, "ms");
    resultat(sec, echelle, "A_par_nouvelle", cout_nouvelles_a as f64 / nb_n, "ns");
    resultat(sec, echelle, "B_par_nouvelle", cout_nouvelles_b as f64 / nb_n, "ns");
    resultat(sec, echelle, "C_par_nouvelle", cout_nouvelles_c as f64 / nb_n, "ns");
    resultat(sec, echelle, "R_passages_par_heure", f64::from(passages), "");
    resultat(sec, echelle, "C_plus_grand_tas", plus_grand_tas as f64, "entrees");
    resultat(sec, echelle, "C_memoire_tas", (plus_grand_tas * 12 + nb_app * 4) as f64, "o");
    resultat(sec, echelle, "B_memoire_colonne", (nb_app * 8) as f64, "o");

    // Le coût d'une nouvelle seule : toutes les nouvelles de l'heure d'affilée.
    let mut ech = etat0.echeance.clone();
    let t = ns(nouvelles.len(), 31, || {
        for &(s, d) in &nouvelles {
            let d = d as usize;
            if delai[d] != 0 {
                ech[d] = s + delai[d];
            }
        }
        u64::from(ech[0])
    });
    resultat(sec, echelle, "B_nouvelle_seule", t, "ns");
    let mut sante = etat0.sante.clone();
    let t = ns(nouvelles.len(), 31, || {
        for &(s, d) in &nouvelles {
            let x = &mut sante[d as usize];
            x.contact_s = s;
            x.vie = vie::VIVANT;
        }
        u64::from(sante[0].contact_s)
    });
    resultat(sec, echelle, "A_nouvelle_seule", t, "ns");
    let t = ns(nouvelles.len(), 31, || {
        let mut tas = Tas { tas: BinaryHeap::with_capacity(nouvelles.len()), generation: vec![0; nb_app] };
        for &(s, d) in &nouvelles {
            if delai[d as usize] != 0 {
                tas.nouvelle(d, s + delai[d as usize]);
            }
        }
        tas.tas.len() as u64
    });
    resultat(sec, echelle, "C_nouvelle_seule", t, "ns");

    // Un passage seul, sans rien d'expiré (le cas de toutes les secondes) : appareils, puis points.
    let mut ech = vec![u32::MAX - 1; nb_app];
    let mut rien = Vec::new();
    let t = ns(1, 201, || u64::from(passage_colonne(&mut ech, 1_000, &mut rien)));
    resultat(sec, echelle, "passage_colonne_appareils", t, "ns");
    let t = ns(1, 201, || u64::from(passage_blocs(&mut ech, 1_000, &mut rien)));
    resultat(sec, echelle, "passage_blocs_appareils", t, "ns");
    let mut sante = etat0.sante.clone();
    for s in &mut sante {
        s.contact_s = 1_000_000;
    }
    let t = ns(1, 201, || {
        passage_calcule(&mut sante, 1_000, &mut rien);
        0
    });
    resultat(sec, echelle, "passage_calcule_appareils", t, "ns");
    let mut perime = vec![u32::MAX - 1; m.points.len()];
    let t = ns(1, 201, || u64::from(passage_blocs(&mut perime, 1_000, &mut rien)));
    resultat(sec, echelle, "passage_blocs_points", t, "ns");
    assert!(rien.is_empty());
}

/// Le coût d'un passage selon la taille : de la maison à un million de points.
pub fn banc_echelle() {
    let sec = "passage";
    for n in [2_264usize, 22_640, 226_400, 2_264_000] {
        let mut col = vec![u32::MAX - 1; n];
        let mut rien = Vec::new();
        let t = ns(1, 51, || u64::from(passage_blocs(&mut col, 1_000, &mut rien)));
        resultat(sec, n as u32, "passage_blocs", t / 1_000.0, "us");
        let t = ns(1, 51, || u64::from(passage_colonne(&mut col, 1_000, &mut rien)));
        resultat(sec, n as u32, "passage_colonne", t / 1_000.0, "us");
    }
}
