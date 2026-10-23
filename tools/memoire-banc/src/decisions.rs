//! Ce que les décisions du 23 octobre ajoutent, mesuré à part (les autres sections ne changent
//! pas) : la cause complète gardée par l'exécution (64 o), et les déclencheurs sur condition
//! composée par le réseau incrémental, dont l'état se reconstruit d'un coup.

use std::hint::black_box;
use std::mem::size_of;

use crate::conditions::Reseau;
use crate::outils::{ns, poids, resultat};
use crate::workflows::preparer;

/// Une exécution qui garde l'état qui l'a lancée, pour toute la suite : 64 octets, une ligne de
/// cache. La case est réutilisée ; la génération écarte une issue tardive.
#[derive(Clone, Copy, Default, Debug)]
#[repr(C)]
pub struct Execution64 {
    // où elle en est : 24 o
    pub automatisme: u32,
    pub curseur: u32,
    pub reveil_s: u32,
    pub depuis_s: u32,
    pub demande: u32,
    pub generation: u16,
    pub etat: u8,
    pub version: u8,
    // l'état qui l'a lancée : 32 o
    pub cause_point: u32,
    pub cause_declencheur: u32,
    pub cause_instant_ms: u64,
    pub cause_valeur: i64,
    pub cause_precedente: i64,
    // une donnée produite en route (texte de Sarah : son rang dans le tiroir ; une moyenne lue) : 8 o
    pub registre: i64,
}

#[derive(Default)]
pub struct Executions64 {
    pub cases: Vec<Execution64>,
    pub libres: Vec<u32>,
}

impl Executions64 {
    pub fn lancer(&mut self, automatisme: u32, curseur: u32, cause: (u32, u32, u64, i64, i64)) -> (u32, u16) {
        let case = match self.libres.pop() {
            Some(c) => c,
            None => {
                self.cases.push(Execution64::default());
                (self.cases.len() - 1) as u32
            }
        };
        let e = &mut self.cases[case as usize];
        let generation = e.generation.wrapping_add(1);
        *e = Execution64 {
            automatisme, curseur, reveil_s: u32::MAX, generation, etat: 1,
            cause_point: cause.0, cause_declencheur: cause.1, cause_instant_ms: cause.2,
            cause_valeur: cause.3, cause_precedente: cause.4, ..Execution64::default()
        };
        (case, generation)
    }
    pub fn finir(&mut self, case: u32) {
        self.cases[case as usize].etat = 0;
        self.cases[case as usize].reveil_s = u32::MAX;
        self.libres.push(case);
    }
}

pub fn banc(echelle: u32) {
    let sec = "decisions";
    let b = preparer(echelle);
    let p = &b.programme;
    resultat(sec, echelle, "taille_execution64", size_of::<Execution64>() as f64, "o");

    // Les exécutions, au pire une par automatisme.
    let (mut ex, o_ex) = poids(|| {
        let mut ex = Executions64::default();
        for (k, pl) in p.plans.iter().enumerate() {
            ex.lancer(k as u32, 3, (pl.premier, pl.premier, 1_792_675_624_123, 1, 0));
        }
        ex
    });
    resultat(sec, echelle, "memoire_executions64", o_ex as f64, "o");
    let t = ns(p.plans.len(), 51, || {
        for k in 0..p.plans.len() as u32 {
            ex.finir(k);
        }
        for (k, pl) in p.plans.iter().enumerate() {
            ex.lancer(k as u32, 0, (pl.premier, pl.premier, 1_792_675_624_123, 1, 0));
        }
        ex.cases.len() as u64
    });
    resultat(sec, echelle, "finir_et_relancer64", t, "ns");

    // Le réseau : au pire, un déclencheur composé de trois feuilles par automatisme.
    let (mut reseau, o_reseau) = poids(|| Reseau::depuis(p, &b.etat));
    let structure = (reseau.point.capacity() + reseau.parent.capacity() + reseau.debut.capacity() + reseau.index.capacity()) * 4
        + reseau.op.capacity() + reseau.valeur.capacity() * 8 + reseau.nb.capacity() * 2;
    let etat_verite = reseau.vrai.capacity() + reseau.vraies.capacity() * 2 + reseau.parent_vrai.capacity() + reseau.depuis.capacity() * 8;
    resultat(sec, echelle, "reseau_memoire_totale", o_reseau as f64, "o");
    resultat(sec, echelle, "reseau_structure_figee", structure as f64, "o");
    resultat(sec, echelle, "reseau_etat_de_verite", etat_verite as f64, "o");
    resultat(sec, echelle, "reseau_feuilles", reseau.point.len() as f64, "");
    resultat(sec, echelle, "reseau_conditions", reseau.nb.len() as f64, "");
    let t = ns(1, 101, || {
        reseau.reconstruire(&b.etat);
        reseau.parent_vrai.iter().filter(|&&v| v).count() as u64
    });
    resultat(sec, echelle, "reseau_reconstruire", t / 1_000.0, "us");
    black_box(&reseau.depuis);
}
