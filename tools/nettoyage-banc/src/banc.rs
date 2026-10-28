//! Les mesures : lire par numéro (quatre formes), le passage du lot et du déménagement, et vingt ans
//! de la vie d'une maison sous trois règles (jamais libérer, des lots, des lots et un déménagement à
//! chaque fin de refonte).

use std::hint::black_box;
use std::time::Instant;

use crate::maison::{self, Classe, Maison, Nature};
use crate::memoire::{Memoire, Sortie};
use crate::numeros::{Numero, statut};
use crate::outils::{Alea, ns, poids, resultat};
use crate::refonte::{JOUR, T0, temps};
use crate::valeurs::Vif;

fn cles(m: &Maison, d: usize) -> Vec<(&str, Nature)> {
    let a = &m.appareils[d];
    m.points[a.premier as usize..(a.premier + a.nb) as usize]
        .iter()
        .map(|p| (p.nom.split_once('/').expect("appareil/clé").1, p.nature))
        .collect()
}

fn declarer(m: &mut Memoire, nom: &str, cles: &[(&str, Nature)], classe: Classe) -> Numero {
    let d = m.preparer(nom, cles, classe);
    let a = d.appareil;
    m.appliquer(&Sortie::Declaration(d)).expect("déclaration");
    a
}

fn maison_declaree(maison: &Maison) -> (Memoire, Vec<Numero>) {
    let mut m = Memoire::default();
    let nums = (0..maison.appareils.len()).map(|d| declarer(&mut m, &maison.appareils[d].nom, &cles(maison, d), maison.appareils[d].classe)).collect();
    (m, nums)
}

fn mediane_us(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// Lire une valeur au hasard : le numéro est l'adresse (v2) ; place et génération dans le numéro ;
/// la même, génération vérifiée ; un numéro fixe qui passe par une table de places.
pub fn lecture(echelle: u32) {
    let sec = "lecture";
    let nb = 2_264 * echelle as usize;
    let vifs: Vec<Vif> = (0..nb).map(|k| Vif { valeur: k as i64, ..Vif::default() }).collect();
    let mut a = Alea::new(7);
    let gens: Vec<u8> = (0..nb).map(|_| a.sous(3) as u8).collect();
    let mut table: Vec<u32> = (0..nb as u32).collect();
    for i in (1..nb).rev() {
        table.swap(i, a.sous(i + 1));
    }
    const N: usize = 4_096;
    let ordre: Vec<u32> = (0..N).map(|_| a.sous(nb) as u32).collect();
    let numeros: Vec<Numero> = ordre.iter().map(|&p| Numero::new(p, gens[p as usize])).collect();
    let direct = ns(N, 31, || {
        let mut x = 0i64;
        for &p in &ordre {
            x = x.wrapping_add(vifs[black_box(p) as usize].valeur);
        }
        x as u64
    });
    let place = ns(N, 31, || {
        let mut x = 0i64;
        for &n in &numeros {
            x = x.wrapping_add(vifs[black_box(n).place()].valeur);
        }
        x as u64
    });
    let verifiee = ns(N, 31, || {
        let mut x = 0i64;
        for &n in &numeros {
            let n = black_box(n);
            let p = n.place();
            if gens[p] == n.generation() {
                x = x.wrapping_add(vifs[p].valeur);
            }
        }
        x as u64
    });
    let indirection = ns(N, 31, || {
        let mut x = 0i64;
        for &k in &ordre {
            x = x.wrapping_add(vifs[table[black_box(k) as usize] as usize].valeur);
        }
        x as u64
    });
    resultat(sec, echelle, "numero_adresse_v2", direct, "ns");
    resultat(sec, echelle, "place_et_generation", place, "ns");
    resultat(sec, echelle, "place_et_generation_verifiee", verifiee, "ns");
    resultat(sec, echelle, "numero_fixe_par_une_table", indirection, "ns");
}

/// Les formes d'appareil de la maison ×1 (clés, natures, classe), pour en déclarer de neufs.
fn formes() -> Vec<(Vec<(String, Nature)>, Classe)> {
    let m = maison::generer(1);
    (0..m.appareils.len())
        .map(|d| (cles(&m, d).into_iter().map(|(c, n)| (c.to_owned(), n)).collect(), m.appareils[d].classe))
        .collect()
}

/// Le passage : 30 % des appareils retirés, puis le lot ; ensuite le déménagement de ce qui reste ;
/// enfin des appareils neufs dans les places libérées. Une copie neuve par essai, une seule à la fois.
pub fn lot(echelle: u32) {
    let sec = "lot";
    let essais = if echelle >= 100 { 3 } else { 9 };
    let formes = formes();
    let (mut m, nums) = {
        let maison = maison::generer(echelle);
        maison_declaree(&maison)
    };
    let mut a = Alea::new(31);
    let t = temps(T0 + JOUR);
    for (k, &n) in nums.iter().enumerate() {
        for p in m.points_de(n) {
            m.appliquer(&Sortie::Valeur { n: p, v: (k % 700) as i64, t }).expect("valeur");
        }
    }
    let mut retires = 0u32;
    for &n in &nums {
        if a.sous(10) < 3 {
            m.appliquer(&Sortie::Statut { a: n, statut: statut::RETIRE, t_ms: T0 }).expect("retrait");
            retires += 1;
        }
    }
    drop(nums);
    // Les octets sont comptés sur des copies : leurs capacités sont exactes.
    let m = {
        let c = m.clone();
        drop(m);
        c
    };
    resultat(sec, echelle, "points", m.points.len() as f64, "");
    resultat(sec, echelle, "appareils_retires", f64::from(retires), "");
    resultat(sec, echelle, "memoire_avant_lot", m.octets() as f64, "o");
    resultat(sec, echelle, "noms_avant_lot", m.points.octets_noms() as f64, "o");
    resultat(sec, echelle, "etat_complet_avant_lot", m.ranger().len() as f64, "o");

    let mut durees = Vec::new();
    let mut apres = None;
    for _ in 0..essais {
        drop(apres.take());
        let mut c = m.clone();
        let d = Instant::now();
        let l = c.choisir_lot(t.mur_ms, 0);
        c.appliquer(&Sortie::Lot(l.clone())).expect("lot");
        durees.push(d.elapsed().as_secs_f64() * 1e6);
        apres = Some((c, l));
    }
    drop(m);
    let (m, l) = apres.expect("un essai");
    resultat(sec, echelle, "lot_duree", mediane_us(durees), "µs");
    resultat(sec, echelle, "lot_points", l.points.len() as f64, "");
    resultat(sec, echelle, "lot_message", l.octets() as f64, "o");
    resultat(sec, echelle, "memoire_apres_lot", m.octets() as f64, "o");
    resultat(sec, echelle, "noms_apres_lot", m.points.octets_noms() as f64, "o");
    resultat(sec, echelle, "etat_complet_apres_lot", m.ranger().len() as f64, "o");
    resultat(sec, echelle, "plages_libres", m.points.libres.len() as f64, "");

    // Le déménagement : la maison a rétréci de 30 %, les trous sont partout.
    let colonnes = m.vifs.len();
    let mut durees = Vec::new();
    let mut fin = None;
    for _ in 0..essais {
        drop(fin.take());
        let mut c = m.clone();
        let d = Instant::now();
        let dm = c.demenager();
        c.appliquer(&Sortie::Demenagement(dm.clone())).expect("déménagement");
        durees.push(d.elapsed().as_secs_f64() * 1e6);
        fin = Some((c, dm));
    }
    let (c, dm) = fin.expect("un essai");
    resultat(sec, echelle, "demenagement_duree", mediane_us(durees), "µs");
    resultat(sec, echelle, "demenagement_mouvements", (dm.points.len() + dm.appareils.len()) as f64, "");
    resultat(sec, echelle, "demenagement_message", dm.octets() as f64, "o");
    resultat(sec, echelle, "colonnes_avant_demenagement", colonnes as f64, "places");
    resultat(sec, echelle, "colonnes_apres_demenagement", c.vifs.len() as f64, "places");
    resultat(sec, echelle, "memoire_apres_demenagement", c.octets() as f64, "o");
    resultat(sec, echelle, "etat_complet_apres_demenagement", c.ranger().len() as f64, "o");
    drop((c, dm));

    // Après le lot, des appareils neufs reprennent les places (la moitié de ce qui est parti).
    let mut m = m;
    let avant = m.points.len();
    let mut d_us = Vec::new();
    for k in 0..retires / 2 {
        let (cles, classe) = &formes[a.sous(formes.len())];
        let cles: Vec<(&str, Nature)> = cles.iter().map(|(c, n)| (c.as_str(), *n)).collect();
        let d = Instant::now();
        declarer(&mut m, &format!("neuf:{k}"), &cles, *classe);
        d_us.push(d.elapsed().as_secs_f64() * 1e6);
    }
    resultat(sec, echelle, "declaration_dans_une_place_libre", mediane_us(d_us), "µs");
    resultat(sec, echelle, "places_ajoutees_au_bout", (m.points.len() - avant) as f64, "");
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Regle {
    Jamais,
    Lots,
    LotsEtDemenagement,
}

/// Vingt ans : chaque mois, 0,5 % des appareils remplacés, 0,3 % retirés, 0,3 % ajoutés ; une
/// grande refonte (la moitié des appareils remplacés) tous les cinq ans ; un lot par mois, délai de
/// grâce 30 jours ; le déménagement le mois qui suit chaque grande refonte.
fn vingt_ans(echelle: u32, regle: Regle, sec: &str) -> Memoire {
    let maison = maison::generer(echelle);
    let (mut m, nums) = maison_declaree(&maison);
    let mut vivants: Vec<Numero> = nums;
    let mut a = Alea::new(2_026);
    let (mut acc_r, mut acc_x, mut acc_a) = (0.0f64, 0.0f64, 0.0f64);
    let mut neuf = 0u32;
    let mut declares = maison.points.len();
    let nom_de = |k: u32| format!("v:{k:07}");
    for mois in 0..240u64 {
        let t_ms = T0 + mois * 30 * JOUR;
        let refonte = mois % 60 == 30;
        let n = vivants.len() as f64;
        acc_r += if refonte { 0.5 } else { 0.005 } * n;
        acc_x += 0.003 * n;
        acc_a += 0.003 * n;
        let remplaces = acc_r.floor() as usize;
        acc_r -= remplaces as f64;
        let retires = acc_x.floor() as usize;
        acc_x -= retires as f64;
        let ajoutes = acc_a.floor() as usize;
        acc_a -= ajoutes as f64;
        for _ in 0..remplaces + retires {
            let i = a.sous(vivants.len());
            let ancien = vivants.swap_remove(i);
            m.appliquer(&Sortie::Statut { a: ancien, statut: statut::RETIRE, t_ms }).expect("retrait");
        }
        for _ in 0..remplaces + ajoutes {
            let modele = a.sous(maison.appareils.len());
            vivants.push(declarer(&mut m, &nom_de(neuf), &cles(&maison, modele), maison.appareils[modele].classe));
            declares += maison.appareils[modele].nb as usize;
            neuf += 1;
        }
        if regle != Regle::Jamais {
            let l = m.choisir_lot(t_ms, 30 * JOUR);
            m.appliquer(&Sortie::Lot(l)).expect("lot");
        }
        if regle == Regle::LotsEtDemenagement && mois % 60 == 31 {
            let dm = m.demenager();
            m.appliquer(&Sortie::Demenagement(dm)).expect("déménagement");
            // Les numéros des vivants ont pu changer : on les relit par la place de leur nom.
            vivants = (0..m.appareils.len())
                .filter(|&p| m.appareils.statut[p] == statut::EN_SERVICE)
                .map(|p| m.appareils.numero_de(p))
                .collect();
        }
        if mois % 12 == 11 {
            let an = mois / 12 + 1;
            resultat(sec, echelle, &format!("an{an:02}_places"), m.points.len() as f64, "");
            resultat(sec, echelle, &format!("an{an:02}_colonnes"), m.vifs.len() as f64, "");
            resultat(sec, echelle, &format!("an{an:02}_en_service"), m.points.statut.iter().filter(|&&s| s == statut::EN_SERVICE).count() as f64, "");
        }
    }
    resultat(sec, echelle, "points_declares_en_vingt_ans", declares as f64, "");
    m
}

pub fn vie(echelle: u32) {
    for (regle, sec) in [(Regle::Jamais, "vie_jamais"), (Regle::Lots, "vie_lots"), (Regle::LotsEtDemenagement, "vie_demenagement")] {
        let (m, octets) = poids(|| vingt_ans(echelle, regle, sec));
        resultat(sec, echelle, "memoire", octets as f64, "o");
        resultat(sec, echelle, "places", m.points.len() as f64, "");
        resultat(sec, echelle, "colonnes", m.vifs.len() as f64, "");
        resultat(sec, echelle, "noms_gardes", m.points.nommes() as f64, "");
        resultat(sec, echelle, "noms_octets", m.points.octets_noms() as f64, "o");
        resultat(sec, echelle, "etat_complet", m.ranger().len() as f64, "o");
        resultat(sec, echelle, "places_condamnees", m.points.statut.iter().filter(|&&s| s == statut::CONDAMNE).count() as f64, "");
        black_box(&m);
    }
}
