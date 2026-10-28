//! La refonte, rejouée minute par minute sur trois jours, dans une maison ×1 (496 appareils, 2 264
//! points) qui vit (20 nouvelles par minute) et 56 automatismes :
//!
//! - J1 09:30 : des exécutions partent pour 20 h vers des appareils qui seront retirés ;
//! - J1 09:55 : des attentes « pendant 10 min » s'arment sur des appareils qui vont être retirés ou
//!   devenir absents ;
//! - J1 10:00 : 5 appareils deviennent absents (pont redémarré) ; la refonte retire 150 appareils,
//!   dont 75 remplacés par un successeur (succession : définitions récrites par les noms) ;
//! - J1 10:20 : les absents reviennent ; J1 11:00 : les successeurs déclenchent ;
//! - J1 23:00 : un client se déconnecte en gardant son cache de noms ;
//! - J2 11:00 : un retrait tardif ; J2 12:00 : le lot (délai de grâce 24 h) ;
//! - J2 13:00 : 80 appareils neufs reprennent les places libérées ; J2 13:30 : le client revient ;
//! - J3 01:00 : fin de refonte, le déménagement, avec une exécution et une attente en cours.
//!
//! L'actif publie chaque sortie critique dans l'ordre du rang `s`. Une copie les applique toutes ;
//! une autre manque la fenêtre du lot (J2 11:00 à 14:00) et reprend par l'état complet.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use crate::maison::{self, Classe, Nature};
use crate::memoire::{Demenagement, Lot, Memoire, Sortie};
use crate::moteur::{Automatisme, Client, Moteur, compiler};
use crate::numeros::{Numero, statut};
use crate::outils::{Alea, Temps, resultat};
use crate::valeurs::qualite;

pub const MINUTE: u64 = 60_000;
pub const HEURE: u64 = 60 * MINUTE;
pub const JOUR: u64 = 24 * HEURE;
/// Le 1er novembre 2026, minuit UTC.
pub const T0: u64 = 1_793_491_200_000;
const RAPPORTS_PAR_MINUTE: usize = 20;

pub fn temps(mur_ms: u64) -> Temps {
    Temps { mono: ((mur_ms - T0) / 1_000) as u32, mur_ms }
}

/// L'actif : sa mémoire, son moteur, les définitions par les noms, le flux de ses sorties critiques,
/// et les contrôles faits au fil de l'eau.
#[derive(Default)]
pub struct Actif {
    pub m: Memoire,
    pub moteur: Moteur,
    pub defs: Vec<Automatisme>,
    pub flux: Vec<Sortie>,
    /// Tous les numéros de point jamais donnés : aucun ne doit l'être deux fois.
    pub emis: HashSet<u32>,
    pub doublons: u32,
    pub client: Client,
    pub client_connecte: bool,
    pub client_erreurs: u32,
    pub ordres_vers_inerte: u32,
    pub ordres_mauvais_nom: u32,
}

impl Actif {
    fn publier(&mut self, so: Sortie) {
        self.m.appliquer(&so).expect("l'actif applique ce qu'il publie");
        if let Sortie::Valeur { n, .. } = &so {
            if self.client_connecte {
                self.verifier_client(*n);
            }
        }
        self.flux.push(so);
    }

    /// Le client range la nouvelle sous le nom de son cache (ou de `meta/`) ; le vrai nom est celui du
    /// registre à cet instant.
    fn verifier_client(&mut self, n: Numero) {
        let m = &self.m;
        let vrai = m.points.nom(n).map(str::to_owned);
        let vu = self.client.nom(n, |k| m.points.nom(k).map(str::to_owned));
        if vrai.as_deref() != Some(vu.as_str()) {
            self.client_erreurs += 1;
        }
    }

    pub fn declarer(&mut self, nom: &str, cles: &[(&str, Nature)], classe: Classe) -> (Numero, Vec<Numero>) {
        let d = self.m.preparer(nom, cles, classe);
        for (n, _, _) in &d.points {
            if !self.emis.insert(n.0) {
                self.doublons += 1;
            }
        }
        let r = (d.appareil, d.points.iter().map(|p| p.0).collect());
        self.publier(Sortie::Declaration(d));
        r
    }

    /// Une nouvelle d'un point en service ; le moteur voit le changement.
    pub fn rapport(&mut self, n: Numero, v: i64, t: Temps) {
        if !self.m.points.vivant(n) || self.m.points.statut[n.place()] != statut::EN_SERVICE {
            return;
        }
        let avant = self.m.vifs[n.place()].valeur;
        self.publier(Sortie::Valeur { n, v, t });
        if avant != v {
            self.moteur.changement(&self.m, n, v, t);
        }
    }

    pub fn statut(&mut self, a: Numero, s: u8, t: Temps) {
        self.publier(Sortie::Statut { a, statut: s, t_ms: t.mur_ms });
        self.moteur.retrait(&self.m);
    }

    pub fn recompiler(&mut self) {
        self.moteur.prog = compiler(&self.defs, &self.m);
    }

    pub fn lot(&mut self, t: Temps, grace_ms: u64) -> Lot {
        let l = self.m.choisir_lot(t.mur_ms, grace_ms);
        if !l.est_vide() {
            self.publier(Sortie::Lot(l.clone()));
            self.recompiler();
        }
        l
    }

    pub fn demenager(&mut self) -> Demenagement {
        let dm = self.m.demenager();
        self.publier(Sortie::Demenagement(dm.clone()));
        self.moteur.renumeroter(&dm);
        self.recompiler();
        dm
    }

    pub fn tic(&mut self, t: Temps) {
        let avant = self.moteur.ordres.len();
        self.moteur.tic(&self.m, t);
        for o in &self.moteur.ordres[avant..] {
            if !self.m.points.vivant(o.cible) {
                self.ordres_vers_inerte += 1;
            }
            if o.nom != self.defs[o.auto as usize].cible {
                self.ordres_mauvais_nom += 1;
            }
        }
    }
}

/// Ce que la refonte a donné, lu par les tests et par le banc.
#[derive(Debug, Default)]
pub struct Bilan {
    pub sorties: usize,
    pub empreinte_actif: u64,
    pub empreinte_copie: u64,
    pub empreinte_copie_reprise: u64,
    pub copie_erreurs: u32,
    pub ordres: usize,
    pub ordres_vers_inerte: u32,
    pub ordres_mauvais_nom: u32,
    pub sans_cible: u32,
    pub attentes_parties_absent_retire: u32,
    pub a_revoir: Vec<u32>,
    pub a_revoir_attendu: Vec<u32>,
    pub succession_autos: u32,
    pub succession_ordres: u32,
    pub absents_memes_numeros: bool,
    pub lot_points: usize,
    pub lot_appareils: usize,
    pub lot_octets: usize,
    pub lot_us: f64,
    pub noms_oublies: bool,
    pub tardif_garde: bool,
    pub neufs_points: usize,
    pub neufs_dans_les_places_liberees: usize,
    pub neufs_generation_1: usize,
    pub neufs_valeur_heritee: usize,
    pub doublons: u32,
    pub client_erreurs: u32,
    pub client_demandes_meta: u32,
    pub causes_mauvais_nom: u32,
    pub causes_retirees: u32,
    pub dm_mouvements: usize,
    pub dm_octets: usize,
    pub dm_us: f64,
    pub noms_avant_demenagement: u64,
    pub noms_apres_demenagement: u64,
    pub colonnes_avant: usize,
    pub colonnes_apres: usize,
    pub nommes_apres: usize,
    pub dm_ordre_suivi: bool,
    pub dm_attente_suivie: bool,
}

struct Jeu {
    act: Actif,
    t_ms: u64,
    alea: Alea,
    vivants: Vec<(Numero, u8)>,
    /// Les automatismes dont le déclencheur ne bouge que quand le scénario le dit.
    calmes: Vec<u32>,
}

impl Jeu {
    /// Les points qui rapportent : en service, sauf les déclencheurs calmes.
    fn rafraichir(&mut self) {
        let m = &self.act.m;
        let calmes: Vec<&str> = self.calmes.iter().map(|&k| self.act.defs[k as usize].declencheur.as_str()).collect();
        self.vivants = (0..m.points.len())
            .filter(|&p| m.points.statut[p] == statut::EN_SERVICE)
            .map(|p| m.points.numero_de(p))
            .filter(|&n| !calmes.contains(&m.points.nom(n).unwrap_or("")))
            .map(|n| (n, m.defs[n.place()].nature))
            .collect();
    }

    /// La maison vit jusqu'à `fin` : chaque minute, des nouvelles au hasard, puis le réveil du moteur.
    fn avancer(&mut self, fin: u64) {
        while self.t_ms < fin {
            self.t_ms += MINUTE;
            let t = temps(self.t_ms);
            for _ in 0..RAPPORTS_PAR_MINUTE {
                let (n, nature) = self.vivants[self.alea.sous(self.vivants.len())];
                let v = match nature {
                    x if x == Nature::Binaire as u8 => self.alea.sous(2) as i64,
                    x if x == Nature::Mesure as u8 => 1_800 + self.alea.sous(700) as i64,
                    _ => self.alea.sous(3) as i64,
                };
                self.act.rapport(n, v, t);
            }
            self.act.tic(t);
        }
    }

    fn maintenant(&self) -> Temps {
        temps(self.t_ms)
    }

    /// Fait passer un point à 0 puis à 1 : son déclencheur part (ou arme son attente).
    fn declencher(&mut self, nom: &str) {
        let n = self.act.m.points.numero(nom).expect("point connu");
        let t = self.maintenant();
        self.act.rapport(n, 0, t);
        self.act.rapport(n, 1, t);
    }
}

fn melanger(v: &mut [usize], a: &mut Alea) {
    for i in (1..v.len()).rev() {
        v.swap(i, a.sous(i + 1));
    }
}

pub fn jouer() -> Bilan {
    let maison = maison::generer(1);
    let mut b = Bilan::default();
    let mut j = Jeu { act: Actif::default(), t_ms: T0, alea: Alea::new(11), vivants: Vec::new(), calmes: Vec::new() };
    let cles_de = |d: usize| -> Vec<(&str, Nature)> {
        let a = &maison.appareils[d];
        maison.points[a.premier as usize..(a.premier + a.nb) as usize]
            .iter()
            .map(|p| (p.nom.split_once('/').expect("appareil/clé").1, p.nature))
            .collect()
    };
    let num: Vec<(Numero, Vec<Numero>)> =
        (0..maison.appareils.len()).map(|d| j.act.declarer(&maison.appareils[d].nom, &cles_de(d), maison.appareils[d].classe)).collect();

    // Qui sera remplacé (S), retiré (R), absent un moment (G), retiré tard (L) ; le reste est stable.
    let mut ordre: Vec<usize> = (0..maison.appareils.len()).collect();
    melanger(&mut ordre, &mut Alea::new(0x4E45_5454));
    let (s, r, g, tardif) = (ordre[..75].to_vec(), ordre[75..150].to_vec(), ordre[150..155].to_vec(), ordre[155]);
    let mut stables = ordre[156..].to_vec();
    stables.sort_unstable();
    let binaire = |d: usize| -> Option<String> {
        let a = &maison.appareils[d];
        maison.points[a.premier as usize..(a.premier + a.nb) as usize].iter().find(|p| p.nature == Nature::Binaire).map(|p| p.nom.clone())
    };
    let premier = |d: usize| maison.points[maison.appareils[d].premier as usize].nom.clone();
    let avec_binaire = |v: &[usize]| v.iter().copied().filter(|&d| binaire(d).is_some()).collect::<Vec<_>>();
    let (sb, rb, gb) = (avec_binaire(&s), avec_binaire(&r), avec_binaire(&g));
    let stb = avec_binaire(&stables);

    let mut defs = Vec::new();
    let groupe = |defs: &mut Vec<Automatisme>, declencheur: String, cible: String, pendant_s: u32, attendre_s: u32| {
        defs.push(Automatisme { declencheur, vers: 1, pendant_s, attendre_s, cible, ordre: 1 });
        defs.len() as u32 - 1
    };
    // A : déclencheur sur un appareil remplacé, cible stable.
    let a_autos: Vec<u32> = (0..15).map(|k| groupe(&mut defs, binaire(sb[k]).unwrap(), premier(stables[k]), 0, 0)).collect();
    // B : déclencheur stable, cible remplacée.
    let b_autos: Vec<u32> = (0..10).map(|k| groupe(&mut defs, binaire(stb[k]).unwrap(), premier(s[k]), 0, 60)).collect();
    // C : déclencheur stable, cible retirée, attendre 20 h : des exécutions vivent à travers le retrait.
    let c_autos: Vec<u32> = (0..10).map(|k| groupe(&mut defs, binaire(stb[10 + k]).unwrap(), premier(r[k]), 0, 20 * 3_600)).collect();
    // D : déclencheur sur un appareil retiré, pendant 10 min.
    let d_autos: Vec<u32> = (0..6).map(|k| groupe(&mut defs, binaire(rb[k]).unwrap(), premier(stables[20 + k]), 600, 0)).collect();
    // G : déclencheur sur un appareil qui sera absent, pendant 10 min.
    let g_autos: Vec<u32> = (0..gb.len().min(3)).map(|k| groupe(&mut defs, binaire(gb[k]).unwrap(), premier(stables[30 + k]), 600, 0)).collect();
    // E : témoins stables.
    for k in 0..10 {
        groupe(&mut defs, binaire(stb[20 + k]).unwrap(), premier(stables[40 + k]), (k as u32 % 2) * 600, [0, 60, 3_600, 3 * 3_600][k % 4]);
    }
    // F : pour la fin de refonte, une exécution de 3 h et une attente de 10 min sur des appareils
    // remplacés : leurs successeurs, déclarés au bout, déménageront.
    let f_exec = groupe(&mut defs, binaire(stb[30]).unwrap(), premier(s[20]), 0, 3 * 3_600);
    let f_attente = groupe(&mut defs, binaire(sb[15]).unwrap(), premier(stables[60]), 600, 0);
    j.calmes = vec![f_exec, f_attente];
    j.act.defs = defs;
    j.act.recompiler();
    assert!(j.act.moteur.prog.a_revoir.is_empty(), "tout se compile au départ");
    j.act.client_connecte = true;
    j.rafraichir();

    // Jour 0 : la maison vit.
    j.avancer(T0 + JOUR + 9 * HEURE + 30 * MINUTE);
    // J1 09:30 : les C partent pour 20 h.
    for &k in &c_autos {
        let d = j.act.defs[k as usize].declencheur.clone();
        j.declencher(&d);
    }
    j.avancer(T0 + JOUR + 9 * HEURE + 55 * MINUTE);
    // J1 09:55 : les attentes D et G s'arment (échéance 10:05).
    for &k in d_autos.iter().chain(&g_autos) {
        let d = j.act.defs[k as usize].declencheur.clone();
        j.declencher(&d);
    }
    j.avancer(T0 + JOUR + 10 * HEURE);

    // J1 10:00 : absents, puis la refonte.
    let t = j.maintenant();
    let mut avant_g: Vec<(String, Option<Numero>)> = Vec::new();
    for &d in &g {
        for (c, _) in cles_de(d) {
            let nom = format!("{}/{c}", maison.appareils[d].nom);
            let n = j.act.m.points.numero(&nom);
            avant_g.push((nom, n));
        }
    }
    for &d in &g {
        j.act.statut(num[d].0, statut::ABSENT, t);
    }
    let mut renommer: HashMap<String, String> = HashMap::new();
    for &d in &s {
        let nom = format!("{}~2", maison.appareils[d].nom);
        j.act.declarer(&nom, &cles_de(d), maison.appareils[d].classe);
        for (c, _) in cles_de(d) {
            renommer.insert(format!("{}/{c}", maison.appareils[d].nom), format!("{nom}/{c}"));
        }
    }
    for a in &mut j.act.defs {
        if let Some(x) = renommer.get(&a.declencheur) {
            a.declencheur = x.clone();
        }
        if let Some(x) = renommer.get(&a.cible) {
            a.cible = x.clone();
        }
    }
    for &d in s.iter().chain(&r) {
        j.act.statut(num[d].0, statut::RETIRE, t);
    }
    j.act.recompiler();
    b.a_revoir = j.act.moteur.prog.a_revoir.clone();
    b.a_revoir_attendu = c_autos.iter().chain(&d_autos).copied().collect();
    b.a_revoir_attendu.sort_unstable();
    j.rafraichir();
    let lancees_avant = j.act.moteur.lancees.len();
    j.avancer(T0 + JOUR + 10 * HEURE + 20 * MINUTE);
    b.attentes_parties_absent_retire =
        j.act.moteur.lancees[lancees_avant..].iter().filter(|(_, a, _, _)| d_autos.contains(a) || g_autos.contains(a)).count() as u32;

    // J1 10:20 : les absents reviennent, avec les mêmes numéros.
    let t = j.maintenant();
    for &d in &g {
        j.act.statut(num[d].0, statut::EN_SERVICE, t);
    }
    j.act.recompiler();
    b.absents_memes_numeros = avant_g.iter().all(|(nom, n)| n.is_some() && j.act.m.points.numero(nom) == *n && j.act.m.points.vivant(n.unwrap()));
    j.rafraichir();

    // J1 11:00 : les successeurs déclenchent (A) ; les B commandent leurs successeurs.
    j.avancer(T0 + JOUR + 11 * HEURE);
    let ordres_avant = j.act.moteur.ordres.len();
    for &k in a_autos.iter().chain(&b_autos) {
        let d = j.act.defs[k as usize].declencheur.clone();
        j.declencher(&d);
    }
    j.avancer(T0 + JOUR + 23 * HEURE);
    let succ: HashSet<u32> = j.act.moteur.ordres[ordres_avant..]
        .iter()
        .filter(|o| o.nom == j.act.defs[o.auto as usize].cible)
        .map(|o| o.auto)
        .collect();
    b.succession_autos = (a_autos.len() + b_autos.len()) as u32;
    b.succession_ordres = a_autos.iter().chain(&b_autos).filter(|k| succ.contains(k)).count() as u32;

    // J1 23:00 : le client se déconnecte et garde son cache.
    j.act.client_connecte = false;
    j.avancer(T0 + 2 * JOUR + 11 * HEURE);
    let rang_coupure = j.act.flux.len();

    // J2 11:00 : un retrait tardif.
    let t = j.maintenant();
    j.act.statut(num[tardif].0, statut::RETIRE, t);
    j.act.recompiler();
    j.rafraichir();
    j.avancer(T0 + 2 * JOUR + 12 * HEURE);

    // J2 12:00 : le lot, délai de grâce 24 h.
    let t = j.maintenant();
    let liberees_attendues: HashSet<u32> =
        s.iter().chain(&r).flat_map(|&d| num[d].1.iter().map(|n| n.place() as u32)).collect();
    let debut = Instant::now();
    let lot = j.act.lot(t, JOUR);
    b.lot_us = debut.elapsed().as_secs_f64() * 1e6;
    b.lot_points = lot.points.len();
    b.lot_appareils = lot.appareils.len();
    b.lot_octets = lot.octets();
    b.noms_oublies = s.iter().chain(&r).all(|&d| cles_de(d).iter().all(|(c, _)| j.act.m.points.numero(&format!("{}/{c}", maison.appareils[d].nom)).is_none()));
    b.tardif_garde = num[tardif].1.iter().all(|n| j.act.m.points.statut[n.place()] == statut::RETIRE)
        && j.act.m.appareils.statut[num[tardif].0.place()] == statut::RETIRE;
    j.rafraichir();
    j.avancer(T0 + 2 * JOUR + 13 * HEURE);

    // J2 13:00 : 80 appareils neufs, aux formes d'appareils stables.
    for k in 0..80 {
        let modele = stables[(k * 7) % stables.len()];
        let (_, pts) = j.act.declarer(&format!("neuf:{k:03}"), &cles_de(modele), maison.appareils[modele].classe);
        b.neufs_points += pts.len();
        b.neufs_dans_les_places_liberees += pts.iter().filter(|n| liberees_attendues.contains(&(n.place() as u32))).count();
        b.neufs_generation_1 += pts.iter().filter(|n| n.generation() == 1).count();
        // Avant sa première nouvelle, un appareil neuf ne sait rien : jamais la valeur de l'ancien occupant.
        b.neufs_valeur_heritee += pts
            .iter()
            .filter(|&&n| {
                let v = j.act.m.vifs[n.place()];
                v.change_ms != 0 || v.valeur != 0 || j.act.m.lire(n).is_none_or(|(_, q)| q != qualite::INCERTAINE)
            })
            .count();
    }
    j.rafraichir();
    j.avancer(T0 + 2 * JOUR + 13 * HEURE + 30 * MINUTE);

    // J2 13:30 : le client revient avec son cache ; il reçoit les valeurs gardées de chaque point.
    j.act.client_connecte = true;
    for (n, _) in j.vivants.clone() {
        j.act.verifier_client(n);
    }
    j.avancer(T0 + 2 * JOUR + 14 * HEURE);
    let etat_reprise = j.act.m.ranger();
    let rang_reprise = j.act.flux.len();

    // J3 00:30 et 00:55 : une exécution de 3 h et une attente de 10 min, vers des appareils qui déménageront.
    j.avancer(T0 + 3 * JOUR + 30 * MINUTE);
    let d = j.act.defs[f_exec as usize].declencheur.clone();
    j.declencher(&d);
    j.avancer(T0 + 3 * JOUR + 55 * MINUTE);
    let d = j.act.defs[f_attente as usize].declencheur.clone();
    j.declencher(&d);
    j.avancer(T0 + 3 * JOUR + HEURE);

    // J3 01:00 : fin de refonte, le déménagement.
    b.colonnes_avant = j.act.m.vifs.len();
    b.noms_avant_demenagement = j.act.m.empreinte_noms();
    let debut = Instant::now();
    let dm = j.act.demenager();
    b.dm_us = debut.elapsed().as_secs_f64() * 1e6;
    b.noms_apres_demenagement = j.act.m.empreinte_noms();
    b.dm_mouvements = dm.points.len() + dm.appareils.len();
    b.dm_octets = dm.octets();
    b.colonnes_apres = j.act.m.vifs.len();
    b.nommes_apres = j.act.m.points.nommes();
    j.rafraichir();
    let lancees_avant = j.act.moteur.lancees.len();
    j.avancer(T0 + 3 * JOUR + 4 * HEURE);
    // L'attente armée à 00:55 part à 01:05 ; l'exécution partie à 00:30 commande à 03:30.
    b.dm_attente_suivie =
        j.act.moteur.lancees[lancees_avant..].iter().any(|(_, a, _, t)| *a == f_attente && *t == T0 + 3 * JOUR + HEURE + 5 * MINUTE);
    b.dm_ordre_suivi = j.act.moteur.ordres.iter().any(|o| {
        o.auto == f_exec && o.t_ms == T0 + 3 * JOUR + 3 * HEURE + 30 * MINUTE && o.nom == j.act.defs[f_exec as usize].cible
    });

    // Les causes des exécutions finies, relues maintenant comme l'écran les montre.
    for &(id, cause) in &j.act.moteur.finies {
        match j.act.m.points.nom(cause) {
            None => b.causes_retirees += 1,
            Some(nom) if nom != j.act.moteur.lancees[id as usize].2 => b.causes_mauvais_nom += 1,
            Some(_) => {}
        }
    }

    // Les copies : l'une applique tout, dans l'ordre ; l'autre manque la fenêtre du lot et reprend
    // par l'état complet.
    let mut copie = Memoire::default();
    for so in &j.act.flux {
        if copie.appliquer(so).is_err() {
            b.copie_erreurs += 1;
        }
    }
    let mut reprise = Memoire::default();
    for so in &j.act.flux[..rang_coupure] {
        if reprise.appliquer(so).is_err() {
            b.copie_erreurs += 1;
        }
    }
    let mut reprise = Memoire::relire(&etat_reprise).expect("état complet lisible");
    for so in &j.act.flux[rang_reprise..] {
        if reprise.appliquer(so).is_err() {
            b.copie_erreurs += 1;
        }
    }
    b.sorties = j.act.flux.len();
    b.empreinte_actif = j.act.m.empreinte();
    b.empreinte_copie = copie.empreinte();
    b.empreinte_copie_reprise = reprise.empreinte();
    b.ordres = j.act.moteur.ordres.len();
    b.ordres_vers_inerte = j.act.ordres_vers_inerte;
    b.ordres_mauvais_nom = j.act.ordres_mauvais_nom;
    b.sans_cible = j.act.moteur.sans_cible;
    b.doublons = j.act.doublons;
    b.client_erreurs = j.act.client_erreurs;
    b.client_demandes_meta = j.act.client.demandes_meta;
    b
}

/// La refonte, en chiffres : `RESULTAT|refonte|1|…`.
pub fn banc() {
    let b = jouer();
    let sec = "refonte";
    for (cle, v, u) in [
        ("sorties", b.sorties as f64, ""),
        ("ordres", b.ordres as f64, ""),
        ("ordres_vers_un_numero_inerte", f64::from(b.ordres_vers_inerte), ""),
        ("executions_arrivees_sans_cible", f64::from(b.sans_cible), ""),
        ("attentes_parties_sur_absent_ou_retire", f64::from(b.attentes_parties_absent_retire), ""),
        ("automatismes_a_revoir", b.a_revoir.len() as f64, ""),
        ("succession_automatismes_qui_commandent", f64::from(b.succession_ordres), ""),
        ("succession_automatismes", f64::from(b.succession_autos), ""),
        ("lot_points", b.lot_points as f64, ""),
        ("lot_appareils", b.lot_appareils as f64, ""),
        ("lot_message", b.lot_octets as f64, "o"),
        ("lot_duree", b.lot_us, "µs"),
        ("neufs_points", b.neufs_points as f64, ""),
        ("neufs_dans_les_places_liberees", b.neufs_dans_les_places_liberees as f64, ""),
        ("neufs_avec_la_valeur_de_l_ancien", b.neufs_valeur_heritee as f64, ""),
        ("numeros_redonnes", f64::from(b.doublons), ""),
        ("client_erreurs", f64::from(b.client_erreurs), ""),
        ("client_demandes_meta", f64::from(b.client_demandes_meta), ""),
        ("causes_au_nom_d_un_autre", f64::from(b.causes_mauvais_nom), ""),
        ("causes_d_un_appareil_libere", f64::from(b.causes_retirees), ""),
        ("demenagement_mouvements", b.dm_mouvements as f64, ""),
        ("demenagement_message", b.dm_octets as f64, "o"),
        ("demenagement_duree", b.dm_us, "µs"),
        ("colonnes_avant_demenagement", b.colonnes_avant as f64, "places"),
        ("colonnes_apres_demenagement", b.colonnes_apres as f64, "places"),
        ("copies_conformes", f64::from(u8::from(b.empreinte_copie == b.empreinte_actif && b.empreinte_copie_reprise == b.empreinte_actif)), ""),
    ] {
        resultat(sec, 1, cle, v, u);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    fn bilan() -> &'static Bilan {
        static B: OnceLock<Bilan> = OnceLock::new();
        B.get_or_init(jouer)
    }

    #[test]
    fn les_copies_restent_conformes() {
        let b = bilan();
        assert_eq!(b.copie_erreurs, 0);
        assert_eq!(b.empreinte_copie, b.empreinte_actif, "la copie qui applique tout, dans l'ordre");
        assert_eq!(b.empreinte_copie_reprise, b.empreinte_actif, "la copie qui a manqué le lot, reprise par l'état complet");
    }

    #[test]
    fn aucun_ordre_ne_part_vers_un_appareil_retire() {
        let b = bilan();
        assert!(b.ordres > 100, "la maison a vécu");
        assert!(b.sans_cible > 0, "des exécutions sont arrivées au bout après le retrait de leur cible");
        assert_eq!(b.ordres_vers_inerte, 0);
        assert_eq!(b.ordres_mauvais_nom, 0);
    }

    #[test]
    fn une_attente_ne_part_pas_sur_un_appareil_absent_ou_retire() {
        assert_eq!(bilan().attentes_parties_absent_retire, 0);
    }

    #[test]
    fn les_automatismes_d_un_appareil_retire_sont_a_revoir() {
        let b = bilan();
        assert_eq!(b.a_revoir, b.a_revoir_attendu);
    }

    #[test]
    fn la_succession_reprend_les_automatismes() {
        let b = bilan();
        assert_eq!(b.succession_ordres, b.succession_autos);
    }

    #[test]
    fn un_appareil_absent_revient_avec_ses_numeros() {
        assert!(bilan().absents_memes_numeros);
    }

    #[test]
    fn le_lot_respecte_le_delai_de_grace_et_oublie_les_noms() {
        let b = bilan();
        assert_eq!(b.lot_appareils, 150);
        assert!(b.noms_oublies);
        assert!(b.tardif_garde, "retiré une heure avant le lot : gardé");
    }

    #[test]
    fn les_places_liberees_servent_a_la_generation_suivante() {
        let b = bilan();
        assert!(b.neufs_points > 300);
        assert_eq!(b.neufs_dans_les_places_liberees, b.neufs_points);
        assert_eq!(b.neufs_generation_1, b.neufs_points);
        assert_eq!(b.doublons, 0, "aucun numéro entier donné deux fois");
    }

    #[test]
    fn un_appareil_neuf_ne_lit_jamais_la_valeur_de_l_ancien_occupant() {
        assert_eq!(bilan().neufs_valeur_heritee, 0);
    }

    #[test]
    fn le_client_en_cache_ne_confond_jamais_deux_appareils() {
        let b = bilan();
        assert_eq!(b.client_erreurs, 0);
        assert!(b.client_demandes_meta > 0);
    }

    #[test]
    fn la_cause_d_une_execution_ne_prend_jamais_un_autre_nom() {
        let b = bilan();
        assert_eq!(b.causes_mauvais_nom, 0);
        assert!(b.causes_retirees > 0, "des causes d'appareils libérés : l'écran dit « appareil retiré »");
    }

    #[test]
    fn le_demenagement_garde_tout_par_les_noms() {
        let b = bilan();
        assert_eq!(b.noms_avant_demenagement, b.noms_apres_demenagement);
        assert!(b.colonnes_apres < b.colonnes_avant);
        assert_eq!(b.colonnes_apres, b.nommes_apres, "plus aucun trou");
        assert!(b.dm_ordre_suivi, "l'exécution en cours commande le bon appareil après le déménagement");
        assert!(b.dm_attente_suivie, "l'attente en cours part après le déménagement");
    }
}
