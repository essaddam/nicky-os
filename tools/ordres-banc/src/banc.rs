//! Les mesures : le catalogue (ce que la maison sait demander), une journée de demandes, ce que
//! l'assistant lit, ce qu'un écran charge ; la forme d'aujourd'hui à côté de la proposition, à ×1,
//! ×10 et ×100.
//!
//! v2 (revue Codex I15 à I19) : le témoin d'aujourd'hui suit la convention des pilotes réels (un
//! seul point `control` en écriture seule pour les appuis d'un appareil, aucun état pour un point en
//! écriture seule) ; les deux validations sont chronométrées sur le même corpus, dont les décisions
//! sont vérifiées identiques d'abord ; les gains d'un écran sont séparés (partage, encodage,
//! filtrage) à information égale ; chaque taille vient de `size_of` ou du compteur d'allocations, et
//! sa provenance est écrite ; la journée mesure le temps processeur de toute la boucle.

use std::collections::BTreeMap;
use std::hint::black_box;
use std::sync::Arc;

use crate::aujourdhui::{self as a, Access, Device, Kind, PointSpec, Semantic, Unit};
use crate::catalogue::{Arg, effet, humain, nature};
use crate::demandes::{Demande, Ref, etat, origine};
use crate::hub::{Cible, Hub, Principal, Req, T, genre, sorte};
use crate::maison::{Maison, Nature, generer};
use crate::maison_ordres::{declarations, dico_mode, est_rts};
use crate::module::{Kit, Monde, Rep};
use crate::numeros::Numero;
use crate::outils::{Alea, ns, poids, resultat};
use crate::valeurs::{Sample, Value};

/// Le temps processeur de ce processus (utilisateur + système), en microsecondes (`getrusage`).
pub fn cpu_us() -> u64 {
    #[cfg(target_os = "macos")]
    type Micro = i32;
    #[cfg(not(target_os = "macos"))]
    type Micro = i64;
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Timeval {
        sec: i64,
        usec: Micro,
    }
    #[repr(C)]
    struct Rusage {
        utime: Timeval,
        stime: Timeval,
        autres: [i64; 14],
    }
    unsafe extern "C" {
        fn getrusage(who: i32, usage: *mut Rusage) -> i32;
    }
    let mut r = std::mem::MaybeUninit::<Rusage>::zeroed();
    // SAFETY : `getrusage(RUSAGE_SELF)` remplit la structure, de la disposition de la libc.
    let r = unsafe {
        getrusage(0, r.as_mut_ptr());
        r.assume_init()
    };
    let us = |t: Timeval| t.sec as u64 * 1_000_000 + t.usec as u64;
    us(r.utime) + us(r.stime)
}

/// L'intention d'aujourd'hui pour un ordre : le point écrivable et la valeur (la convention des
/// pilotes réels : une consigne écrit sa valeur ; les appuis d'un appareil partagent un point
/// `control` en écriture seule ; un texte a son point).
fn vers_aujourdhui(hub: &Hub, o: Numero, args: &[Arg]) -> (String, Value) {
    let nom = hub.ordres.nom(o).unwrap_or("");
    let (app, cle) = nom.rsplit_once('/').unwrap_or(("", ""));
    let f = hub.catalogue.fiches[o.place()].forme;
    let ps = hub.catalogue.params_de(f);
    match (ps.first(), args.first()) {
        (Some(p), Some(Arg::N(v))) => {
            // La valeur qu'il règle, ou, s'il n'en règle aucune, son propre point en écriture seule.
            let point = if p.regle != crate::catalogue::AUCUNE_CLE { format!("{app}/{}", hub.catalogue.nom_cle(p.regle)) } else { format!("{app}/{cle}") };
            let val = match p.nature {
                nature::BINAIRE if *v == 0 || *v == 1 => Value::Bool(*v == 1),
                nature::ENUM => Value::Text(hub.catalogue.codes(p.dico).iter().find(|x| i64::from(x.0) == *v).map_or_else(|| Arc::from(format!("code_{v}")), |x| Arc::from(&*x.1))),
                _ => Value::Float(*v as f64 * 10f64.powi(i32::from(p.echelle))),
            };
            (point, val)
        }
        (Some(p), Some(Arg::T(t))) if p.nature == nature::TEXTE => (format!("{app}/{cle}"), Value::Text(Arc::from(&**t))),
        _ => (format!("{app}/control"), Value::Text(Arc::from(cle))),
    }
}

/// La maison d'aujourd'hui, à la convention des pilotes : un `Device` par appareil, un `PointSpec`
/// par valeur (écrivable si un ordre la règle, avec les bornes de cet ordre) ; un ordre à paramètre
/// qui ne règle aucune valeur, son point en écriture seule ; les appuis d'un appareil en un seul
/// point `control` (une énumération) en écriture seule ; un texte, son point.
fn aujourdhui(m: &Maison, hub: &Hub) -> BTreeMap<Arc<str>, Arc<Device>> {
    let mut map = BTreeMap::new();
    for (d, app) in m.appareils.iter().enumerate() {
        let na = hub.appareils.numero(&app.nom).expect("appareil");
        // Les valeurs qu'un ordre règle, et les bornes de cet ordre.
        let mut regles: BTreeMap<String, Kind> = BTreeMap::new();
        let mut seuls: Vec<(String, Kind)> = Vec::new();
        let mut controles: Vec<Arc<str>> = Vec::new();
        let mut textes: Vec<String> = Vec::new();
        for &o in hub.ordres_de(na.place()) {
            let o = Numero(o);
            let cle = hub.ordres.nom(o).unwrap_or("").rsplit('/').next().unwrap_or("").to_string();
            let f = hub.catalogue.fiches[o.place()].forme;
            let kind_de = |p: &crate::catalogue::Param| match p.nature {
                nature::BINAIRE => Kind::Binary,
                nature::ENUM => Kind::Enum { values: hub.catalogue.codes(p.dico).iter().filter(|x| x.0 >= p.min && x.0 <= p.max).map(|x| Arc::from(&*x.1)).collect() },
                _ => {
                    let e = 10f64.powi(i32::from(p.echelle));
                    Kind::Numeric { min: Some(f64::from(p.min) * e), max: Some(f64::from(p.max) * e), step: Some(f64::from(p.pas.max(1)) * e) }
                }
            };
            match hub.catalogue.params_de(f).first() {
                Some(p) if p.nature == nature::TEXTE => textes.push(cle),
                Some(p) if p.regle != crate::catalogue::AUCUNE_CLE => {
                    regles.insert(hub.catalogue.nom_cle(p.regle).to_string(), kind_de(p));
                }
                Some(p) => seuls.push((cle, kind_de(p))),
                None => controles.push(cle.as_str().into()),
            }
        }
        let mut points = Vec::new();
        let cles = crate::maison_ordres::cles_de(m, d);
        for (k, p) in m.points[app.premier as usize..(app.premier + app.nb) as usize].iter().enumerate() {
            let cle = cles[k].clone();
            let ecrit = regles.remove(&cle);
            let kind = ecrit.clone().unwrap_or(match p.nature {
                Nature::Mesure => Kind::Numeric { min: Some(0.0), max: Some(254.0), step: Some(1.0) },
                Nature::Binaire => Kind::Binary,
                Nature::Enum => Kind::Enum { values: dico_mode().iter().map(|x| Arc::from(&*x.1)).collect() },
                Nature::Texte | Nature::Vide => Kind::Text,
            });
            points.push(PointSpec {
                key: cle.as_str().into(),
                label: format!("Libellé {cle}").into(),
                kind,
                access: Access { read: true, write: ecrit.is_some() },
                unit: (p.nature == Nature::Mesure).then_some(Unit::Celsius),
                semantic: Semantic::Measure,
            });
        }
        for (cle, kind) in seuls {
            points.push(PointSpec { key: cle.as_str().into(), label: "Commande".into(), kind, access: Access { read: false, write: true }, unit: None, semantic: Semantic::Control });
        }
        if !controles.is_empty() {
            points.push(PointSpec { key: "control".into(), label: "Commande".into(), kind: Kind::Enum { values: controles }, access: Access { read: false, write: true }, unit: None, semantic: Semantic::Control });
        }
        for cle in textes {
            points.push(PointSpec { key: cle.as_str().into(), label: "Message".into(), kind: Kind::Text, access: Access { read: false, write: true }, unit: None, semantic: Semantic::Other });
        }
        let id: Arc<str> = app.nom.as_str().into();
        let dev = Device {
            id: Arc::clone(&id),
            instance: app.nom.split(':').next().unwrap_or("").into(),
            native_name: format!("Appareil {d}").into(),
            manufacturer: Some("Fabricant".into()),
            model: Some("Modèle".into()),
            description: None,
            native_room: Some("Salon".into()),
            members: Vec::new(),
            points,
        };
        map.insert(id, Arc::new(dev));
    }
    // Les appareils virtuels d'aujourd'hui sont des appareils comme les autres, avec `members`.
    for p in 0..hub.apps.len() {
        if hub.apps[p].genre == genre::VIRTUEL {
            let n = hub.appareils.numero_de(p);
            let nom = hub.appareils.nom(n).unwrap_or("");
            let membres = hub.compo(n).iter().filter(|x| x.sorte == sorte::APPAREIL).map(|x| Arc::from(hub.appareils.nom(Numero(x.numero)).unwrap_or(""))).collect();
            let points = if hub.apps[p].nb_ordres > 0 {
                vec![PointSpec { key: "on".into(), label: "Allumé".into(), kind: Kind::Binary, access: Access { read: true, write: true }, unit: None, semantic: Semantic::Other }]
            } else {
                Vec::new()
            };
            let id: Arc<str> = nom.into();
            map.insert(Arc::clone(&id), Arc::new(Device { id, instance: "lumieres".into(), native_name: nom.into(), manufacturer: None, model: Some("room".into()), description: None, native_room: None, members: membres, points }));
        }
    }
    map
}

/// L'état d'aujourd'hui : un échantillon par point **lisible** (un point en écriture seule n'a pas
/// d'état : le test du volet RTS l'exige, overkiz model.rs:604).
fn etats_de(devices: &BTreeMap<Arc<str>, Arc<Device>>) -> BTreeMap<Arc<str>, BTreeMap<Arc<str>, Sample>> {
    let mut etats = BTreeMap::new();
    for (id, d) in devices {
        let mut s = BTreeMap::new();
        for (k, p) in d.points.iter().enumerate().filter(|(_, p)| p.access.read) {
            let v = match p.kind {
                Kind::Binary => Value::Bool(k % 2 == 0),
                Kind::Numeric { .. } => Value::Float(21.5),
                Kind::Enum { .. } => Value::Text("auto".into()),
                _ => Value::Null,
            };
            s.insert(Arc::clone(&p.key), Sample { value: v, ts: 1_792_778_400_000 });
        }
        etats.insert(Arc::clone(id), s);
    }
    etats
}

pub fn hub_de(m: &Maison) -> Hub {
    let mut hub = Hub::default();
    hub.catalogue.dicos.push(dico_mode());
    let t = T { mono_ms: 1_000, mur_ms: 1_792_713_600_000 };
    for d in declarations(m) {
        hub.declarer(t, &d).expect("déclaration");
        hub.sorties.clear();
    }
    hub.sorties.shrink_to_fit();
    hub
}

/// Les libellés d'un écran, par clé (aujourd'hui : un libellé par point).
pub fn libelle(cle: &str) -> String {
    format!("Libellé {cle}")
}

pub fn catalogue(e: u32) {
    let m = generer(e);
    let (hub, octets_hub) = poids(|| hub_de(&m));
    let sec = "catalogue";
    let nb_virtuels = hub.apps.iter().filter(|x| x.genre == genre::VIRTUEL).count();
    resultat(sec, e, "appareils", hub.appareils.len() as f64, "");
    resultat(sec, e, "appareils_virtuels", nb_virtuels as f64, "");
    resultat(sec, e, "valeurs", hub.points.len() as f64, "");
    resultat(sec, e, "ordres", hub.ordres.len() as f64, "");
    resultat(sec, e, "formes", hub.catalogue.nb_formes() as f64, "");
    resultat(sec, e, "parametres", hub.catalogue.params.len() as f64, "");
    resultat(sec, e, "membres", hub.membres.len() as f64, "");
    resultat(sec, e, "cles", hub.catalogue.cles.len() as f64, "");
    // Les tailles réelles des fiches (`size_of`), vérifiées aussi à la compilation.
    resultat(sec, e, "taille_app_o", std::mem::size_of::<crate::hub::App>() as f64, "o");
    resultat(sec, e, "taille_fiche_ordre_o", std::mem::size_of::<crate::catalogue::Fiche>() as f64, "o");
    resultat(sec, e, "taille_forme_o", std::mem::size_of::<crate::catalogue::Forme>() as f64, "o");
    resultat(sec, e, "taille_param_o", std::mem::size_of::<crate::catalogue::Param>() as f64, "o");
    resultat(sec, e, "taille_demande_o", std::mem::size_of::<Demande>() as f64, "o");
    resultat(sec, e, "taille_recu_o", std::mem::size_of::<crate::demandes::Recu>() as f64, "o");
    resultat(sec, e, "taille_membre_o", std::mem::size_of::<crate::hub::Membre>() as f64, "o");
    resultat(sec, e, "taille_ligne_o", std::mem::size_of::<crate::hub::Ligne>() as f64, "o");
    // La mémoire proposée, poste par poste, mesurée par le compteur d'allocations (un clone alloue
    // les mêmes tables ; les vecteurs y sont ajustés à leur longueur).
    let (_, registre_ordres) = poids(|| hub.ordres.clone());
    let (_, registre_points) = poids(|| hub.points.clone());
    let (_, registre_appareils) = poids(|| hub.appareils.clone());
    let (_, catalogue) = poids(|| hub.catalogue.clone());
    let appareils = registre_appareils + hub.octets_appareils();
    resultat(sec, e, "propose_registre_ordres_o", registre_ordres as f64, "o");
    resultat(sec, e, "propose_catalogue_o", catalogue as f64, "o");
    resultat(sec, e, "propose_structure_ordres_o", (std::mem::size_of::<crate::catalogue::Forme>() * hub.catalogue.formes.len() + std::mem::size_of::<crate::catalogue::Param>() * hub.catalogue.params.len() + 4 * hub.catalogue.attendus.len() + std::mem::size_of::<crate::catalogue::Fiche>() * hub.catalogue.fiches.len()) as f64, "o");
    resultat(sec, e, "propose_appareils_listes_compositions_o", appareils as f64, "o");
    resultat(sec, e, "propose_annuaire_valeurs_o", registre_points as f64, "o");
    // La Def de la mémoire v2 (16 o par valeur, v2 § 5.1) : une donnée de la v2, pas une mesure de ce banc.
    let def_v2 = 16 * hub.points.len();
    resultat(sec, e, "memoire_v2_def_valeurs_o", def_v2 as f64, "o");
    resultat(sec, e, "propose_description_totale_o", (registre_ordres + catalogue + appareils + registre_points + def_v2) as f64, "o");
    resultat(sec, e, "hub_du_banc_total_o", octets_hub as f64, "o");
    // La même maison sous la forme d'aujourd'hui : à ×100, elle ne tient plus dans 128 Mio avec le
    // reste ; le banc ne la construit qu'à ×1 et ×10.
    let mut alea = Alea::new(11);
    let corpus = corpus_de(&hub, &mut alea);
    mesures_proposees(&hub, e, &corpus);
    if e >= 100 {
        return;
    }
    let (devices, octets_auj) = poids(|| aujourdhui(&m, &hub));
    resultat(sec, e, "aujourdhui_description_o", octets_auj as f64, "o");
    let ecrits = devices.values().flat_map(|d| d.points.iter()).filter(|p| p.access.write).count();
    let ecriture_seule = devices.values().flat_map(|d| d.points.iter()).filter(|p| !p.access.read).count();
    let rts = m.appareils.iter().enumerate().filter(|(d, x)| est_rts(x.classe, *d)).count();
    resultat(sec, e, "aujourdhui_points", devices.values().map(|d| d.points.len()).sum::<usize>() as f64, "");
    resultat(sec, e, "aujourdhui_points_ecrivables", ecrits as f64, "");
    resultat(sec, e, "aujourdhui_points_en_ecriture_seule", ecriture_seule as f64, "");
    resultat(sec, e, "volets_rts", rts as f64, "");
    // La validation : le même corpus des deux côtés, les mêmes décisions d'abord, puis le temps.
    let mut identiques = 0usize;
    let mut acceptees = 0usize;
    let intentions: Vec<(String, Value)> = corpus.iter().map(|(_, o, _, args, _)| vers_aujourdhui(&hub, *o, args)).collect();
    for ((_, o, f, args, _), (point, v)) in corpus.iter().zip(&intentions) {
        let propose = hub.ordres.vivant(*o) && hub.catalogue.valider(*f, args).is_ok();
        let auj = a::prepare(&devices, point, v).is_ok();
        identiques += usize::from(propose == auj);
        acceptees += usize::from(propose);
    }
    assert_eq!(identiques, corpus.len(), "les deux validations décident pareil sur le corpus commun");
    resultat(sec, e, "corpus_intentions", corpus.len() as f64, "");
    resultat(sec, e, "corpus_acceptees", acceptees as f64, "");
    resultat(sec, e, "corpus_decisions_identiques", identiques as f64, "");
    let t_auj = ns(intentions.len(), 101, || {
        let mut ok = 0u64;
        for (point, v) in &intentions {
            ok += u64::from(a::prepare(&devices, point, v).is_ok());
        }
        ok
    });
    resultat(sec, e, "valider_aujourdhui_ns", t_auj, "ns");
    // L'assistant : la copie de la maison à chaque requête, et l'inventaire qu'il lit.
    let etats = etats_de(&devices);
    let (vues, copie) = poids(|| a::snapshot(&devices, &etats));
    resultat(sec, e, "assistant_copie_de_la_maison_o", copie as f64, "o");
    let t_copie = ns(1, 11, || a::snapshot(&devices, &etats).len() as u64);
    resultat(sec, e, "assistant_copie_us", t_copie / 1_000.0, "µs");
    let inv = a::inventaire(&vues);
    let prop = inventaire_propose(&hub);
    resultat(sec, e, "assistant_inventaire_aujourdhui_car", inv.chars().count() as f64, "car");
    resultat(sec, e, "assistant_inventaire_propose_car", prop.chars().count() as f64, "car");
    // Estimation : un jeton pour quatre caractères (aucun tokeniseur hors ligne).
    resultat(sec, e, "assistant_inventaire_aujourdhui_jetons_estimes", (inv.chars().count() / 4) as f64, "jetons");
    resultat(sec, e, "assistant_inventaire_propose_jetons_estimes", (prop.chars().count() / 4) as f64, "jetons");
    let t_inv = ns(1, 11, || inventaire_propose(&hub).len() as u64);
    resultat(sec, e, "assistant_inventaire_propose_us", t_inv / 1_000.0, "µs");
    black_box(vues);
}

type Intention = (String, Numero, u16, Vec<Arg>, bool);

/// Le corpus commun : 1 024 ordres tirés, chacun avec des arguments valides (dans les bornes, au
/// pas, au dictionnaire) et, pour un paramètre numérique, hors de ses bornes. Les cas qu'aujourd'hui
/// ne sait pas exprimer (le pas, la longueur d'un texte, le nombre d'arguments) n'y sont pas.
fn corpus_de(hub: &Hub, alea: &mut Alea) -> Vec<Intention> {
    let mut v = Vec::with_capacity(2_048);
    while v.len() < 1_024 {
        let o = hub.ordres.numero_de(alea.sous(hub.ordres.len()));
        if !hub.ordres.vivant(o) {
            continue;
        }
        let nom = hub.ordres.nom(o).unwrap_or("").to_string();
        let f = hub.catalogue.fiches[o.place()].forme;
        let ps = hub.catalogue.params_de(f).to_vec();
        let valides: Vec<Arg> = ps
            .iter()
            .map(|p| match p.nature {
                nature::ENUM => Arg::N(i64::from(hub.catalogue.codes(p.dico).first().map_or(p.min, |x| x.0))),
                nature::TEXTE | nature::OBJET => Arg::T("x".into()),
                _ => {
                    let pas = i64::from(p.pas.max(1));
                    let n = (i64::from(p.max) - i64::from(p.min)) / pas;
                    Arg::N(i64::from(p.min) + pas * alea.sous(n as usize + 1) as i64)
                }
            })
            .collect();
        v.push((nom.clone(), o, f, valides.clone(), true));
        if let Some(p) = ps.first()
            && matches!(p.nature, nature::MESURE | nature::BINAIRE)
        {
            let mut hors = valides;
            hors[0] = Arg::N(i64::from(p.max) + i64::from(p.pas.max(1)));
            v.push((nom, o, f, hors, false));
        }
    }
    v
}

/// Ce qui ne dépend que de la mémoire proposée : valider, lire, interroger, servir une page.
fn mesures_proposees(hub: &Hub, e: u32, corpus: &[Intention]) {
    let sec = "catalogue";
    let t_nom = ns(corpus.len(), 101, || {
        let mut ok = 0u64;
        for (nom, _, _, args, _) in corpus {
            if let Some(o) = hub.ordres.numero(nom).filter(|&o| hub.ordres.vivant(o)) {
                ok += u64::from(hub.catalogue.valider(hub.catalogue.fiches[o.place()].forme, args).is_ok());
            }
        }
        ok
    });
    let t_num = ns(corpus.len(), 101, || {
        let mut ok = 0u64;
        for (_, o, _, args, _) in corpus {
            if hub.ordres.vivant(*o) {
                ok += u64::from(hub.catalogue.valider(hub.catalogue.fiches[o.place()].forme, args).is_ok());
            }
        }
        ok
    });
    resultat(sec, e, "valider_par_nom_ns", t_nom, "ns");
    resultat(sec, e, "valider_par_numero_ns", t_num, "ns");
    // Lire la dernière commande d'un ordre, et « la valeur suit-elle la dernière commande ? ».
    let t_fiche = ns(corpus.len(), 201, || {
        let mut s = 0i64;
        for (_, o, _, _, _) in corpus {
            s += hub.catalogue.fiches[o.place()].valeur;
        }
        s as u64
    });
    let t_vient = ns(corpus.len(), 201, || {
        let mut s = 0u64;
        for (_, o, _, _, _) in corpus {
            s += u64::from(crate::moteur::vient_de(hub, *o));
        }
        s
    });
    resultat(sec, e, "lire_derniere_commande_ns", t_fiche, "ns");
    resultat(sec, e, "vient_de_ns", t_vient, "ns");
    // La page d'un appareil virtuel : ses valeurs, ses ordres, ce qu'il emprunte.
    let virtuels: Vec<Numero> = (0..hub.apps.len()).filter(|&p| hub.apps[p].genre == genre::VIRTUEL).map(|p| hub.appareils.numero_de(p)).collect();
    let t_vue = ns(virtuels.len().max(1), 21, || virtuels.iter().map(|&v| hub.vue(v).len()).sum::<usize>() as u64);
    let t_aussi = ns(64, 5, || {
        let mut s = 0;
        for k in 0..64 {
            s += hub.aussi_dans(sorte::VALEUR, hub.points.numero_de(k * 7 % hub.points.len())).len();
        }
        s as u64
    });
    resultat(sec, e, "page_virtuel_ns", t_vue, "ns");
    resultat(sec, e, "aussi_dans_parcours_ns", t_aussi, "ns");
    // Ce qui voyage sur `meta/` : la structure écrite à chaque ordre (V7 § 7.4), ou une forme par numéro.
    let mut meta_inline = 0usize;
    let mut meta_formes = 0usize;
    for p in 0..hub.apps.len() {
        let n = hub.appareils.numero_de(p);
        let Some(nom) = hub.appareils.nom(n) else { continue };
        let mut x = format!("{{\"nom\":\"{nom}\",\"points\":{{");
        let mut y = x.clone();
        for &pn in hub.points_de(p) {
            let cle = hub.catalogue.nom_cle(hub.cle_point[Numero(pn).place()]);
            let v = format!("\"{cle}\":{{\"n\":{pn},\"kind\":\"numeric\",\"unit\":\"°C\",\"echelle\":2,\"fraicheur_s\":600}},");
            x.push_str(&v);
            y.push_str(&v);
        }
        y.push_str("},\"ordres\":{");
        for &on in hub.ordres_de(p) {
            let cle = hub.ordres.nom(Numero(on)).unwrap_or("").rsplit('/').next().unwrap_or("");
            let f = hub.catalogue.fiches[Numero(on).place()].forme;
            let j = hub.catalogue.json_forme(f);
            x.push_str(&format!("\"{cle}\":{{\"n\":{on},\"ecrit\":true,{}}},", &j[1..j.len() - 1]));
            y.push_str(&format!("\"{cle}\":{{\"n\":{on},\"f\":{f}}},"));
        }
        x.push_str("}}");
        y.push_str("}}");
        meta_inline += x.len();
        meta_formes += y.len();
    }
    let formes_json: usize = (1..=hub.catalogue.nb_formes()).map(|f| hub.catalogue.json_forme(f as u16).len()).sum();
    resultat(sec, e, "meta_structure_dans_chaque_ordre_o", meta_inline as f64, "o");
    resultat(sec, e, "meta_avec_formes_o", (meta_formes + formes_json) as f64, "o");
    resultat(sec, e, "meta_formes_seules_o", formes_json as f64, "o");
}

/// L'inventaire proposé : les valeurs et les ordres à part, chaque structure une fois, par sa forme,
/// en unités humaines (« 5–30 °C », les noms d'une énumération), lu dans les colonnes, sans copier
/// la maison.
pub fn inventaire_propose(hub: &Hub) -> String {
    use std::fmt::Write as _;
    let unite = |u: u8| ["", " °C", " %", " W"].get(usize::from(u)).copied().unwrap_or("");
    let mut out = String::from("## Formes\n");
    for f in 1..=hub.catalogue.nb_formes() {
        let x = hub.catalogue.formes[f];
        let ps: Vec<String> = hub
            .catalogue
            .params_de(f as u16)
            .iter()
            .map(|p| match p.nature {
                nature::BINAIRE => "oui/non".to_string(),
                nature::MESURE => format!("{}–{}{}", humain(p.min, p.echelle), humain(p.max, p.echelle), unite(p.unite)),
                nature::ENUM => hub.catalogue.codes(p.dico).iter().filter(|c| c.0 >= p.min && c.0 <= p.max).take(30).map(|c| c.1.to_string()).collect::<Vec<_>>().join("|"),
                _ => format!("texte ≤ {}", p.pas),
            })
            .collect();
        let _ = writeln!(out, "F{f} : {}{}", if ps.is_empty() { "sans paramètre".into() } else { ps.join(", ") }, if x.effet == effet::APPUI { ", appui" } else { "" });
    }
    out.push_str("## Salon\n");
    for p in 0..hub.apps.len() {
        let n = hub.appareils.numero_de(p);
        let Some(nom) = hub.appareils.nom(n) else { continue };
        let _ = write!(out, "- {nom}");
        let mut vals = Vec::new();
        for &pn in hub.points_de(p).iter().take(6) {
            let q = Numero(pn).place();
            let d = hub.defs[q];
            let v = hub.vifs[q].valeur;
            let txt = match d.nature {
                nature::ENUM => hub.catalogue.codes(d.dico).iter().find(|c| i64::from(c.0) == v).map_or("?".to_string(), |c| c.1.to_string()),
                nature::MESURE => format!("{}{}", humain(v as i32, d.echelle), unite(d.unite)),
                _ => v.to_string(),
            };
            vals.push(format!("{}={txt}", hub.catalogue.nom_cle(hub.cle_point[q])));
        }
        if !vals.is_empty() {
            let _ = write!(out, " · {}", vals.join(", "));
        }
        let ords: Vec<String> = hub.ordres_de(p).iter().map(|&on| format!("{}·F{}", hub.ordres.nom(Numero(on)).unwrap_or("").rsplit('/').next().unwrap_or(""), hub.catalogue.fiches[Numero(on).place()].forme)).collect();
        if !ords.is_empty() {
            let _ = write!(out, " ✎ {}", ords.join(", "));
        }
        out.push('\n');
    }
    out
}

/// Les principaux de la journée : un écran (humain), un agent, le moteur, huit appelants de l'API.
fn principaux_journee(hub: &mut Hub) -> (u16, u16, u16, Vec<u16>) {
    let tout = crate::droits::Personne { pieces: crate::droits::TOUTES_PIECES, fin_mur_ms: u64::MAX, role: crate::droits::role::PROPRIETAIRE, ..Default::default() };
    hub.ajouter_personne("Bob", tout).expect("personne");
    let ecran = hub.ajouter_principal(Principal::humain("écran du salon", 1)).expect("principal");
    let agent = hub.ajouter_principal(Principal::agent("agent", origine::MCP, 2)).expect("principal");
    let moteur = hub.ajouter_principal(Principal::interne("moteur", origine::AUTOMATISME)).expect("principal");
    let api = (0..8).map(|k| hub.ajouter_principal(Principal::agent(&format!("script {k}"), origine::API, 10 + k)).expect("principal")).collect();
    (ecran, agent, moteur, api)
}

/// Une journée de demandes : 3 000 × échelle, des rafales de scènes, toutes les formes de retour.
pub fn demandes(e: u32) {
    let sec = "demandes";
    let m = generer(e);
    let mut hub = hub_de(&m);
    hub.pieces_protegees = vec![3];
    let (ecran, agent, moteur, api) = principaux_journee(&mut hub);
    let mut monde = Monde::default();
    let mut kit = Kit::default();
    // Comment répondent les appareils du banc : par classe.
    for (d, app) in m.appareils.iter().enumerate() {
        let n = hub.appareils.numero(&app.nom).expect("appareil").place() as u32;
        let r = match app.classe {
            crate::maison::Classe::Action if d % 20 >= 14 => Rep::Pont { ms: 600 },
            crate::maison::Classe::Action | crate::maison::Classe::Wifi => {
                if d % 37 == 0 {
                    Rep::Muet
                } else {
                    Rep::Etat { ms: 300 }
                }
            }
            crate::maison::Classe::Api => Rep::Mouvement { accuse_ms: 400, pas_ms: 10_000, inter: vec![50] },
            _ => Rep::Accuse { accuse_ms: 500, etat_ms: 60_000, valeur: None },
        };
        monde.reps.insert(n, r);
    }
    drop(m);
    let ordres: Vec<Numero> = (0..hub.ordres.len()).map(|p| hub.ordres.numero_de(p)).collect();
    let nb = 3_000 * e as usize;
    let mut alea = Alea::new(0x0D0D + u64::from(e));
    let debut_mur = 1_792_713_600_000u64;
    let mut arrivees: Vec<u64> = (0..nb * 7 / 10).map(|_| alea.sous(86_400_000) as u64).collect();
    while arrivees.len() < nb {
        let t0 = alea.sous(86_400_000) as u64;
        for k in 0..40 {
            arrivees.push(t0 + k * 25);
        }
    }
    arrivees.truncate(nb);
    arrivees.sort_unstable();
    let mut sorties = 0usize;
    let mut octets = 0usize;
    let mut refuses = 0usize;
    let mut doubles = 0usize;
    let mut t = T { mono_ms: 1_000, mur_ms: debut_mur };
    let mut k = 0usize;
    let fin = debut_mur + 86_400_000 + 300_000;
    let mut seqs: BTreeMap<u16, u64> = BTreeMap::new();
    let mut reveils = 0u64;
    let cpu0 = cpu_us();
    let mur0 = std::time::Instant::now();
    while t.mur_ms < fin {
        let prochaine = arrivees.get(k).map_or(u64::MAX, |x| debut_mur + x);
        // Le réveil au plus proche : une arrivée, une réponse d'appareil, ou, tant qu'une demande est
        // en cours, son échéance ; sinon rien ne tourne.
        let suivant = prochaine
            .min(monde.prochain().unwrap_or(u64::MAX))
            .min(hub.prochaine(t).unwrap_or(u64::MAX))
            .min(kit.prochaine(t).unwrap_or(u64::MAX))
            .min(fin)
            .max(t.mur_ms + 1);
        reveils += 1;
        t = T { mono_ms: t.mono_ms + (suivant - t.mur_ms), mur_ms: suivant };
        while k < arrivees.len() && debut_mur + arrivees[k] <= t.mur_ms {
            let o = ordres[alea.sous(ordres.len())];
            let f = hub.catalogue.fiches[o.place()].forme;
            let mut args: Vec<Arg> = hub
                .catalogue
                .params_de(f)
                .iter()
                .map(|p| match p.nature {
                    nature::ENUM => Arg::N(i64::from(hub.catalogue.codes(p.dico).first().map_or(p.min, |x| x.0))),
                    nature::TEXTE | nature::OBJET => Arg::T("x".into()),
                    _ => {
                        let pas = i64::from(p.pas.max(1));
                        Arg::N(i64::from(p.min) + pas * alea.sous(((i64::from(p.max) - i64::from(p.min)) / pas) as usize + 1) as i64)
                    }
                })
                .collect();
            if alea.sous(50) == 0 && !args.is_empty() {
                args[0] = Arg::N(1_000_000);
            }
            let de = match alea.sous(100) {
                0..60 => moteur,
                60..95 => ecran,
                95..98 => agent,
                _ => api[alea.sous(api.len())],
            };
            let seq = if de == moteur {
                0
            } else {
                let s = seqs.entry(de).or_insert(0);
                *s += 1;
                *s
            };
            let r = Req::nouvelle(de, seq, Cible::Numero(o), args, t);
            // Une demande externe sur dix est renvoyée avec la même clé (le transport, un appelant
            // qui retente) : elle ne part qu'une fois.
            if seq != 0 && alea.sous(10) == 0 {
                let _ = hub.demander(t, Req { tentative: 2, ..r.clone() });
                doubles += 1;
            }
            if hub.demander(t, r).is_err() {
                refuses += 1;
            }
            k += 1;
        }
        for c in hub.remettre(t) {
            kit.recevoir(&mut hub, &mut monde, t, c);
        }
        for q in monde.echus(t.mur_ms) {
            kit.evenement(&mut hub, &mut monde, t, q);
        }
        for c in hub.remettre(t) {
            kit.recevoir(&mut hub, &mut monde, t, c);
        }
        kit.tic(&mut hub, &mut monde, t);
        hub.tic(t);
        let annulees = std::mem::take(&mut hub.annulations);
        if !annulees.is_empty() {
            kit.annuler(&mut hub, t, &annulees);
        }
        // Les retenues : un humain en décide une sur deux, une minute après.
        let retenues: Vec<(Ref, Numero, u64, u32)> =
            hub.demandes.refs().filter(|x| hub.demandes.fiches[x.case()].etat == etat::RETENUE).map(|x| (x, Numero(hub.demandes.fiches[x.case()].ordre), hub.demandes.fiches[x.case()].recue_ms, hub.cartes[&x].revision)).collect();
        for (x, o, recue, revision) in retenues {
            if t.mur_ms >= recue + 60_000 && x.generation() % 2 == 0 {
                let _ = hub.decider(t, x, o, revision, true, ecran);
            }
        }
        for p in std::mem::take(&mut hub.sorties) {
            sorties += 1;
            octets += Hub::octets_sortie(&p);
        }
        hub.res.clear();
        hub.notices.clear();
    }
    let cpu = cpu_us() - cpu0;
    let mur = mur0.elapsed();
    resultat(sec, e, "demandes", nb as f64, "");
    resultat(sec, e, "renvois_meme_cle", doubles as f64, "");
    resultat(sec, e, "refusees", refuses as f64, "");
    resultat(sec, e, "envois", kit.envois.len() as f64, "");
    resultat(sec, e, "pic_demandes_en_cours", hub.demandes.pic as f64, "");
    resultat(sec, e, "cpu_journee_ms", cpu as f64 / 1e3, "ms");
    resultat(sec, e, "cpu_par_demande_us", cpu as f64 / nb as f64, "µs");
    resultat(sec, e, "mur_journee_ms", mur.as_secs_f64() * 1e3, "ms");
    resultat(sec, e, "reveils", reveils as f64, "");
    resultat(sec, e, "sorties_critiques_par_jour", sorties as f64, "");
    resultat(sec, e, "sorties_critiques_par_jour_o", octets as f64, "o");
    resultat(sec, e, "memoire_demandes_o", hub.demandes.octets() as f64, "o");
    resultat(sec, e, "memoire_recus_o", hub.recus.octets() as f64, "o");
    resultat(sec, e, "memoire_recus_borne_64_principaux_o", (64 * (8 + std::mem::size_of::<crate::demandes::Fenetre>())) as f64, "o");
    let (journal, octets_journal) = poids(|| std::collections::VecDeque::<crate::hub::Ligne>::with_capacity(crate::hub::JOURNAL));
    black_box(&journal);
    resultat(sec, e, "memoire_journal_o", octets_journal as f64, "o");
    let mut b = Vec::new();
    hub.catalogue.ranger(&mut b);
    let catalogue_etat = b.len();
    b.clear();
    hub.demandes.ranger(&mut b, &|x| x);
    let demandes_etat = b.len();
    let image = hub.ranger(t).len();
    resultat(sec, e, "etat_complet_catalogue_o", catalogue_etat as f64, "o");
    resultat(sec, e, "etat_complet_demandes_o", demandes_etat as f64, "o");
    resultat(sec, e, "etat_complet_total_o", image as f64, "o");
    // Une demande, seule : prendre une case, la conclure ; retrouver une issue par son numéro.
    let mut table = crate::demandes::Demandes::default();
    let t_case = ns(1_000, 101, || {
        let mut s = 0u64;
        for k in 0..1_000u32 {
            let r = table.prendre(Demande { etat: etat::EN_FILE, ordre: k, ..Demande::default() }, 1);
            s += r.0;
            table.liberer(r);
        }
        s
    });
    let refs: Vec<Ref> = (0..256u32).map(|k| table.prendre(Demande { etat: etat::EN_FILE, ordre: k, ..Demande::default() }, 1)).collect();
    let t_issue = ns(refs.len(), 201, || refs.iter().map(|&r| u64::from(table.get(r).is_some_and(|d| d.issue == 0))).sum());
    resultat(sec, e, "prendre_et_liberer_une_case_ns", t_case, "ns");
    resultat(sec, e, "retrouver_une_demande_ns", t_issue, "ns");
    // v3 : les droits. Les tailles ; la décision seule, sur un mélange d'autorités (tablette anonyme,
    // PIN d'un enfant, voix reconnue, agent mandaté) et de classes ; la décision du hub pour un ordre de
    // la maison, ses pièces comprises (celles des cibles d'un groupe).
    {
        use crate::droits::{self as dr, Autorite, Contexte, Personne, Profil, classe, preuve, role};
        resultat(sec, e, "taille_profil_o", std::mem::size_of::<Profil>() as f64, "o");
        resultat(sec, e, "taille_personne_o", std::mem::size_of::<Personne>() as f64, "o");
        resultat(sec, e, "taille_session_o", std::mem::size_of::<dr::Session>() as f64, "o");
        resultat(sec, e, "taille_mandat_o", std::mem::size_of::<dr::Mandat>() as f64, "o");
        resultat(sec, e, "taille_table_o", std::mem::size_of::<dr::Table>() as f64, "o");
        resultat(sec, e, "taille_principal_o", std::mem::size_of::<Principal>() as f64, "o");
        resultat(sec, e, "taille_bail_o", std::mem::size_of::<crate::moteur::Bail>() as f64, "o");
        let table = dr::table_par_defaut();
        let ecran_p = Profil { pieces: dr::TOUTES_PIECES, fin_mur_ms: u64::MAX, oui: classe::TOUTES & !classe::bit(classe::DESARMER), drapeaux: dr::drapeau_p::DECIDE, preuve_max: preuve::PERSONNELLE, ..Profil::default() };
        let voix_p = Profil { oui: crate::hub::CLASSES_AGENT, proposer: crate::hub::PROPOSE_AGENT, drapeaux: dr::drapeau_p::GARDE_PIECES, preuve_max: preuve::INDICE, ..ecran_p };
        let enfant = Personne { pieces: dr::pieces(&[0, 3]), fin_mur_ms: u64::MAX, role: role::ENFANT, ..Personne::default() };
        let proprio = Personne { pieces: dr::TOUTES_PIECES, fin_mur_ms: u64::MAX, role: role::PROPRIETAIRE, ..Personne::default() };
        let auts = [
            Autorite { profil: Some(&ecran_p), personne: None, preuve: preuve::AUCUNE, mandat: None, systeme: false },
            Autorite { profil: Some(&ecran_p), personne: Some(&enfant), preuve: preuve::PERSONNELLE, mandat: None, systeme: false },
            Autorite { profil: Some(&voix_p), personne: Some(&proprio), preuve: preuve::INDICE, mandat: None, systeme: false },
            Autorite { profil: Some(&voix_p), personne: Some(&proprio), preuve: preuve::PERSONNELLE, mandat: None, systeme: false },
        ];
        let ctx = Contexte { protegees: dr::pieces(&[3, 4]), droits_frais: true };
        let t_dec = ns(1_000, 101, || {
            let mut s = 0u64;
            for k in 0..1_000usize {
                s += u64::from(dr::decider(&table, 1, black_box(&auts[k % 4]), (k % 9) as u8, 1 << (k % 8), ctx));
            }
            s
        });
        resultat(sec, e, "droits_decider_ns", t_dec, "ns");
        // La table, case par case : le schéma de la v3 la lit dans la série, comme les mesures.
        if e == 1 {
            for (ro, ligne) in table.iter().enumerate() {
                for (c, &x) in ligne.iter().enumerate() {
                    resultat(sec, e, &format!("droits_table_r{ro}_c{c}"), f64::from(x), "");
                }
            }
        }
        let a = crate::hub::Aut { de: ecran, pour: 1, preuve: preuve::PERSONNELLE, mandat: 0, systeme: false };
        let echantillon: Vec<Numero> = ordres.iter().copied().step_by((ordres.len() / 1_000).max(1)).collect();
        let t_niv = ns(echantillon.len(), 51, || echantillon.iter().map(|&o| u64::from(hub.niveau(t, &a, o))).sum());
        resultat(sec, e, "droits_niveau_hub_ns", t_niv, "ns");
    }
    // 50 retenues, mesurées : un hub à part, un agent, 50 demandes retenues (fiches, cartes, reçus).
    if e == 1 {
        let m = generer(1);
        let mut h = hub_de(&m);
        drop(m);
        h.pieces_protegees = (0..12).collect();
        let ag = h.ajouter_principal(Principal::agent("agent", origine::MCP, 2)).expect("principal");
        let cibles: Vec<Numero> = (0..h.ordres.len()).map(|p| h.ordres.numero_de(p)).filter(|o| h.catalogue.params_de(h.catalogue.fiches[o.place()].forme).len() == 1).take(50).collect();
        let t0 = T { mono_ms: 1_000, mur_ms: debut_mur };
        // Ce qui reste en mémoire : les fiches, les cartes, les paramètres, le reçu ; les sorties, elles,
        // partent sur MQTT (leur tampon est rendu).
        let (_, octets_ret) = poids(|| {
            for (k, o) in cibles.iter().enumerate() {
                let f = h.catalogue.fiches[o.place()].forme;
                let p = h.catalogue.params_de(f)[0];
                let _ = h.demander(t0, Req::nouvelle(ag, k as u64 + 1, Cible::Numero(*o), vec![Arg::N(i64::from(p.min))], t0));
            }
            h.sorties = Vec::new();
            h.res = Vec::new();
        });
        let n = h.demandes.refs().filter(|x| h.demandes.fiches[x.case()].etat == etat::RETENUE).count();
        resultat(sec, e, "propose_retenues", n as f64, "");
        resultat(sec, e, "propose_retenues_50_o", octets_ret as f64, "o");
    }
    // Aujourd'hui, au mieux : une demande en vol, le journal de 500 lignes, 50 retenues.
    let dev = Arc::new(Device { id: "z2m:0x00158d00a1b2c3d4".into(), instance: "z2m".into(), native_name: "Lampe".into(), manufacturer: None, model: None, description: None, native_room: None, members: Vec::new(), points: Vec::new() });
    let (vol, octets_vol) = poids(|| a::en_vol(&dev, "z2m:0x00158d00a1b2c3d4/brightness", Value::Float(128.0), Some("Couloir"), true));
    black_box(&vol);
    drop(vol);
    resultat(sec, e, "aujourdhui_une_demande_en_vol_o", (octets_vol + std::mem::size_of::<a::EnVol>()) as f64, "o");
    let (journal, octets_journal) = poids(|| {
        (0..500u64)
            .map(|k| {
                Arc::new(a::JournalEntry {
                    id: k,
                    ts: 1_792_778_400_000 + k,
                    origin: a::Origin::Automation,
                    actor: Some("Extinction auto".into()),
                    action: a::Action::Command { point: Arc::from(format!("z2m:0x00158d00a1b2{k:04x}/brightness")), value: Value::Float(128.0) },
                    outcome: if k % 10 == 0 { a::Outcome::Err("driver did not answer in time".into()) } else { a::Outcome::Ok },
                    duration_ms: 412,
                    correlation: Some("0123456789abcdef0123456789abcdef".into()),
                })
            })
            .collect::<Vec<_>>()
    });
    black_box(&journal);
    resultat(sec, e, "aujourdhui_journal_500_o", octets_journal as f64, "o");
    let (retenues, octets_ret) = poids(|| {
        (0..50u64)
            .map(|k| {
                Arc::new(a::ApprovalRequest {
                    id: k,
                    ts: 0,
                    expires: 600_000,
                    point: Arc::from(format!("nuki:porte_{k}/unlatch")),
                    value: Value::Null,
                    origin: a::Origin::Mcp,
                    actor: Some("Agent".into()),
                    reason: "porte : décision d'une personne".into(),
                    device_name: "Porte d'entrée".into(),
                    native_name: "Nuki Smart Lock".into(),
                    rooms: vec!["Entrée".into()],
                    request_id: None,
                    correlation: Some("0123456789abcdef0123456789abcdef".into()),
                })
            })
            .collect::<Vec<_>>()
    });
    black_box(&retenues);
    resultat(sec, e, "aujourdhui_retenues_50_o", octets_ret as f64, "o");
}

/// Les clients : ce qu'un écran charge pour (re)démarrer, à information égale, par l'API d'aujourd'hui
/// ou par MQTT ; le gain séparé en partage (JSON proposé), encodage (binaire) et filtrage (deux pièces).
pub fn clients(e: u32) {
    let sec = "clients";
    let m = generer(e);
    let hub = hub_de(&m);
    drop(m);
    if e < 100 {
        let m = generer(e);
        let devices = aujourdhui(&m, &hub);
        drop(m);
        let etats = etats_de(&devices);
        let json = a::snapshot_json(&a::snapshot(&devices, &etats));
        resultat(sec, e, "aujourdhui_api_snapshot_json_o", json.len() as f64, "o");
        drop(json);
        let t_json = ns(1, 11, || a::snapshot_json(&a::snapshot(&devices, &etats)).len() as u64);
        resultat(sec, e, "aujourdhui_api_snapshot_us", t_json / 1_000.0, "µs");
    }
    let tout = |_: u16| true;
    let deux = |p: u16| p < 2;
    let json = hub.ranger_client_json(&tout, &libelle, 0).len();
    resultat(sec, e, "propose_meme_contenu_json_o", json as f64, "o");
    let bin = hub.ranger_client(&tout, &libelle, 0).len();
    resultat(sec, e, "mqtt_etat_client_complet_o", bin as f64, "o");
    resultat(sec, e, "mqtt_etat_client_morceaux_4kio", bin.div_ceil(4_096) as f64, "");
    let invite = hub.ranger_client(&deux, &libelle, 0).len();
    resultat(sec, e, "mqtt_etat_client_deux_pieces_o", invite as f64, "o");
    let tours = if e >= 100 { 3 } else { 11 };
    let t_tout = ns(1, tours, || hub.ranger_client(&tout, &libelle, 0).len() as u64);
    let t_invite = ns(1, tours, || hub.ranger_client(&deux, &libelle, 0).len() as u64);
    resultat(sec, e, "mqtt_etat_client_complet_us", t_tout / 1_000.0, "µs");
    resultat(sec, e, "mqtt_etat_client_deux_pieces_us", t_invite / 1_000.0, "µs");
}

/// La soirée, en chiffres.
pub fn soiree() {
    let c0 = std::time::Instant::now();
    let s = crate::soiree::jouer();
    let ms = c0.elapsed().as_secs_f64() * 1_000.0;
    let b = &s.bilan;
    let sec = "soiree";
    resultat(sec, 1, "duree_ms", ms, "ms");
    resultat(sec, 1, "demandes_suivies", b.issues.len() as f64, "");
    resultat(sec, 1, "envois", b.envois.len() as f64, "");
    resultat(sec, 1, "sorties_critiques", b.sorties as f64, "");
    resultat(sec, 1, "sorties_critiques_o", b.octets_sorties as f64, "o");
    resultat(sec, 1, "instantane_o", b.instantane_octets as f64, "o");
    resultat(sec, 1, "ecran_formes_demandees", f64::from(b.ecran_demandes_meta), "");
    resultat(sec, 1, "ecran_perime_formes_demandees", f64::from(b.ecran2_demandes_meta), "");
    let identiques = b.empreintes.iter().filter(|(m, a, c, d)| if m == "avant la bascule" { a == c && Some(*a) == *d } else { Some(*a) == *d }).count();
    resultat(sec, 1, "releves_identiques", identiques as f64, "");
    resultat(sec, 1, "releves", b.empreintes.len() as f64, "");
    resultat(sec, 1, "validations_identiques", b.validations_identiques.0 as f64, "");
    resultat(sec, 1, "validations", b.validations_identiques.1 as f64, "");
    resultat(sec, 1, "ordres_du_catalogue", b.catalogue.1 as f64, "");
    resultat(sec, 1, "formes_du_catalogue", b.catalogue.2 as f64, "");
    resultat(sec, 1, "ecran_piece_trous", f64::from(b.ecran_piece.0), "");
    resultat(sec, 1, "ecran_piece_rechargements", f64::from(b.ecran_piece.1), "");
    resultat(sec, 1, "secrets_trouves_hors_canal_prive", b.secrets_hors_canal as f64, "");
}

