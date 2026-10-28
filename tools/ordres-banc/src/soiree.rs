//! La vérification de bout en bout : une soirée d'ordres, de 18 h à 23 h, puis une refonte un mois
//! plus tard. Un actif, sa copie de secours (le flux critique, dans l'ordre de `s`, avec ses
//! accusés), une copie reprise par l'état complet à 19 h 45, un écran qui garde les formes en cache
//! par numéro, un écran de pièce qui suit le rang de sa pièce, et des principaux : un écran, un
//! agent, l'assistant, un script, un invité, l'agent d'un gardien. Sur la maison ×1 du banc, plus
//! quinze appareils nommés, dont deux virtuels.
//!
//! v2 : chaque constat de la revue Codex qui se rejoue a sa scène (B1, B2, B5, I1 à I8, I11, I13,
//! I20) ; les autres ont leurs essais (`essais.rs`).

use std::collections::{BTreeMap, BTreeSet};

use crate::catalogue::{Arg, Fiche, FormeDecl, binaire, code, drapeau, effet, mesure, retour, texte, valider_decl};
use crate::demandes::{Ref, etat, issue, origine};
use crate::droits::Personne;
use crate::hub::{AppDecl, Cible, Hub, Principal, Publiee, Req, Sortie, T, role, sorte};
use crate::module::{Kit, Monde, Quoi, Rep};
use crate::moteur::{DeclDef, Def, Moteur, PasDef, genre, op, resultat};
use crate::numeros::Numero;

/// L'heure murale d'un instant de la soirée (24 octobre 2026, heure de Paris), en ms.
pub fn h(hh: u64, mm: u64, ss: u64) -> u64 {
    1_792_778_400_000 + ((hh - 18) * 3_600 + mm * 60 + ss) * 1_000
}
pub const JOUR_MS: u64 = 86_400_000;

/// Les principaux de la soirée, dans l'ordre de leur numéro (la configuration des comptes).
pub const P_HUB: u16 = 0;
pub const P_MOTEUR: u16 = 1;
pub const P_ECRAN: u16 = 2;
pub const P_MCP: u16 = 3;
pub const P_ASSISTANT: u16 = 4;
pub const P_API: u16 = 5;
pub const P_INVITE: u16 = 6;
pub const P_GARDIEN: u16 = 7;
/// Un second écran, pour les scènes de la v2 : chaque principal garde ses séquences croissantes.
pub const P_ECRAN2: u16 = 8;

pub fn principaux() -> Vec<Principal> {
    vec![
        Principal::interne("hub", origine::SYSTEME),
        Principal::interne("moteur", origine::AUTOMATISME),
        Principal::humain("écran du salon", 1),
        Principal::agent("agent", origine::MCP, 2),
        Principal::agent("assistant", origine::ASSISTANT, 3),
        Principal::agent("script de la sonnette", origine::API, 4),
        // Un invité : le bureau seulement, jusqu'à 19 h.
        Principal::invite("invité", 5, vec![6], h(19, 0, 0)),
        // L'agent d'un gardien, mandaté par lui : l'entrée seulement, jusqu'à 18 h 40 (sa fiche).
        Principal::agent("agent du gardien", origine::MCP, 6).pour(6),
        Principal::humain("écran de la cuisine", 1),
    ]
}

/// Les personnes de la maison du banc (v3) : leur rang est leur numéro.
pub fn personnes() -> Vec<(String, Personne)> {
    use crate::droits::{TOUTES_PIECES, pieces, role};
    let p = |r: u8, ps: u64, fin: u64| Personne { pieces: ps, debut_mur_ms: 0, fin_mur_ms: fin, role: r, ..Personne::default() };
    vec![
        ("Bob".into(), p(role::PROPRIETAIRE, TOUTES_PIECES, u64::MAX)),
        ("Alice".into(), p(role::ADULTE, TOUTES_PIECES, u64::MAX)),
        ("Charlie".into(), p(role::ENFANT, pieces(&[3]), u64::MAX)),
        ("Dan".into(), p(role::ENFANT, pieces(&[4]), u64::MAX)),
        // Un invité : le bureau seulement, jusqu'à 19 h.
        ("invité".into(), p(role::INVITE, pieces(&[6]), h(19, 0, 0))),
        // Un gardien : l'entrée seulement, jusqu'à 18 h 40.
        ("gardien".into(), p(role::SERVICE, pieces(&[5]), h(18, 40, 0))),
    ]
}

/// Les deux textes témoins : ils ne doivent jamais sortir du canal privé.
pub const SECRET_ANNONCE: &str = "SECRET-ANNONCE-42";
pub const SECRET_TELEGRAM: &str = "SECRET-TELEGRAM-7";

pub struct Machine {
    pub hub: Hub,
    pub moteur: Moteur,
    pub kit: Kit,
    /// L'heure murale de son démarrage : son horloge monotone en part.
    pub debut_ms: u64,
}

impl Machine {
    pub fn t(&self, mur: u64) -> T {
        T { mono_ms: mur - self.debut_ms + 1_000, mur_ms: mur }
    }
}

/// La dernière commande, telle qu'un écran l'affiche (la preuve, elle, se juge chez le hub à chaque
/// valeur : un écran la lit dans son état, ou la recalcule avec les mêmes formes).
pub fn derniere(f: &Fiche) -> (i64, u64, Ref, u32, u16, u8, u8, u16) {
    // La partie publique : un écran ne reçoit jamais qui l'a demandée (v3).
    (f.valeur, f.instant_ms, f.demande, f.qui, f.forme, f.issue, f.origine, f.actives)
}

/// Un écran : il garde les formes et les dictionnaires en cache par numéro, et demande
/// `meta/forme/<f>` (ou `meta/dico/<d>`) pour ce qu'il ne connaît pas ; il ne devine jamais.
#[derive(Clone, Default)]
pub struct Ecran {
    pub formes: BTreeMap<u16, FormeDecl>,
    pub dicos: BTreeMap<u16, Vec<i32>>,
    pub ordres: BTreeMap<u32, (String, u16)>,
    pub fiches: BTreeMap<u32, Fiche>,
    pub demandes_meta: u32,
}

impl Ecran {
    fn connaitre(&mut self, f: u16, source: &Hub) {
        if !self.formes.contains_key(&f) {
            self.demandes_meta += 1;
            let d = source.catalogue.decl(f);
            for p in &d.params {
                if p.nature == crate::catalogue::nature::ENUM {
                    self.dicos.entry(p.dico).or_insert_with(|| source.catalogue.codes(p.dico).iter().map(|x| x.0).collect());
                }
            }
            self.formes.insert(f, d);
        }
    }
    pub fn appliquer(&mut self, p: &Publiee, source: &Hub) {
        match &p.sortie {
            Sortie::Appareil { nom, ordres, .. } => {
                for (n, cle, f) in ordres {
                    self.ordres.insert(n.0, (format!("{nom}/{cle}"), *f));
                    self.connaitre(*f, source);
                }
            }
            Sortie::Trace { ordre, fiche, .. } => {
                if *fiche != Fiche::default() {
                    self.fiches.insert(ordre.0, *fiche);
                }
            }
            Sortie::Lot { ordres, .. } => {
                let places: BTreeSet<u32> = ordres.iter().copied().collect();
                self.ordres.retain(|n, _| !places.contains(&(Numero(*n).place() as u32)));
                self.fiches.retain(|n, _| !places.contains(&(Numero(*n).place() as u32)));
            }
            _ => {}
        }
    }
    /// L'écran refuse avant d'envoyer, avec la même fonction, la même forme et les mêmes
    /// dictionnaires que le hub.
    pub fn valider(&self, ordre: Numero, args: &[Arg]) -> Result<(), u8> {
        let (_, f) = self.ordres.get(&ordre.0).ok_or(crate::catalogue::raison::INCONNU)?;
        valider_decl(&self.formes[f], args, &|d, v| self.dicos.get(&d).is_some_and(|c| c.contains(&v)))
    }
}

/// Un écran de pièce (un invité, une tablette au mur) : il ne voit que ses pièces, et vérifie la
/// continuité du rang de chacune ; un trou le fait se recharger (revue I13).
#[derive(Clone, Default)]
pub struct EcranPiece {
    pub pieces: Vec<u16>,
    pub epoque: u32,
    pub rangs: BTreeMap<u16, u64>,
    pub fiches: BTreeMap<u32, Fiche>,
    pub trous: u32,
    pub rechargements: u32,
    /// Une perte simulée : la prochaine sortie de cette pièce n'arrive pas.
    pub perdre: Option<u16>,
}

impl EcranPiece {
    fn voit(&self, piece: u16) -> bool {
        piece == crate::hub::PIECE_GLOBALE || self.pieces.contains(&piece)
    }

    /// Charge son état par MQTT : l'en-tête dit le rang de chaque pièce ; les fiches viennent du hub.
    pub fn charger(&mut self, hub: &Hub) {
        let pieces = self.pieces.clone();
        let b = hub.ranger_client(&|p| pieces.contains(&p), &crate::banc::libelle, 0);
        let lire16 = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
        let lire64 = |i: usize| u64::from_le_bytes(b[i..i + 8].try_into().expect("8 o"));
        self.epoque = u32::from_le_bytes(b[0..4].try_into().expect("4 o"));
        let n = usize::from(lire16(12));
        self.rangs.clear();
        for k in 0..n {
            let i = 14 + k * 10;
            self.rangs.insert(lire16(i), lire64(i + 2));
        }
        self.fiches.clear();
        for a in 0..hub.apps.len() {
            if hub.appareils.nom(hub.appareils.numero_de(a)).is_some() && pieces.contains(&hub.apps[a].piece) {
                for &o in hub.ordres_de(a) {
                    let f = hub.catalogue.fiches[Numero(o).place()];
                    if f != Fiche::default() {
                        self.fiches.insert(o, f);
                    }
                }
            }
        }
        self.rechargements += 1;
    }

    pub fn appliquer(&mut self, p: &Publiee, source: &Hub) {
        if !self.voit(p.piece) {
            return;
        }
        if self.perdre == Some(p.piece) {
            self.perdre = None;
            return;
        }
        if p.e != self.epoque {
            // Une époque nouvelle : l'écran se recharge (ce n'est pas un trou).
            self.charger(source);
            return;
        }
        let vu = self.rangs.get(&p.piece).copied().unwrap_or(0);
        if p.sp <= vu {
            return; // déjà dans l'état chargé
        }
        if p.sp != vu + 1 {
            self.trous += 1;
            self.charger(source);
            return;
        }
        self.rangs.insert(p.piece, p.sp);
        if let Sortie::Trace { ordre, fiche, .. } = &p.sortie
            && *fiche != Fiche::default()
        {
            self.fiches.insert(ordre.0, *fiche);
        }
    }
}

/// Ce que la soirée a produit, pour les tests.
#[derive(Default)]
pub struct Bilan {
    /// Les issues rendues à chaque séquence, dans l'ordre (issue, raison, suite).
    pub issues: BTreeMap<u64, Vec<(u8, u8, bool)>>,
    /// L'heure murale de chacune.
    pub instants: BTreeMap<u64, Vec<u64>>,
    /// Pour un groupe : combien de cibles ont obéi, sur combien (la dernière issue).
    pub groupes: BTreeMap<u64, (u8, u8)>,
    /// Le numéro de demande de chaque séquence (la première fois), et tous ceux qu'elle a eus.
    pub refs: BTreeMap<u64, Ref>,
    pub toutes_refs: BTreeMap<u64, BTreeSet<Ref>>,
    /// La mère de chaque fille de groupe.
    pub parents: BTreeMap<Ref, Ref>,
    /// Après le lot, les fiches des ordres libérés sont à zéro.
    pub fiches_liberees_a_zero: bool,
    /// Les envois vers un appareil, tous kits confondus (actif puis secours).
    pub envois: Vec<(String, Ref)>,
    pub notices: Vec<String>,
    pub moteur: Vec<(u64, String, String)>,
    /// Empreintes relevées : (moment, actif, copie, copie par état complet).
    pub empreintes: Vec<(String, u64, u64, Option<u64>)>,
    /// La fiche de chaque ordre chez l'actif et chez l'écran, à un relevé.
    pub fiches_ecran: Vec<(String, usize, usize)>,
    pub vues: BTreeMap<String, Vec<String>>,
    pub a_revoir: Vec<(String, Vec<(String, String)>)>,
    pub ecran_demandes_meta: u32,
    pub ecran2_demandes_meta: u32,
    /// Ce que l'écran à vieux cache décide de 62 °C après la redéclaration.
    pub ecran2_valide_62: Option<Result<(), u8>>,
    pub validations_identiques: (usize, usize),
    pub sorties: usize,
    pub octets_sorties: usize,
    pub instantane_octets: usize,
    pub instantane_identique: bool,
    pub refus_epoque: u32,
    pub par_numero_perime: Option<(u8, u8)>,
    pub vient_de: Vec<(String, u8)>,
    pub derniere_rts: Option<(i64, u8, u8, u64)>,
    pub nouvelle_fiche: Option<Fiche>,
    pub aussi_dans: Vec<String>,
    pub ordres_rts: (usize, usize),
    pub catalogue: (usize, usize, usize),
    // v2
    pub qui_invite: u32,
    pub pour_invite: u16,
    pub decision_trop_tard: Option<Result<Ref, &'static str>>,
    pub decision_carte_perimee: Option<Result<Ref, &'static str>>,
    pub decision_gardien: Option<Result<Ref, &'static str>>,
    pub fiche_chauffe_eau: Option<Fiche>,
    pub ecran_piece: (u32, u32),
    pub ecran_piece_identique: bool,
    pub secrets_hors_canal: usize,
    pub prives_lus: Vec<String>,
    pub prives_restants: usize,
    pub copie_tenait_le_prive: bool,
    pub redeclaration: Option<(Vec<Numero>, Vec<Numero>, bool)>,
    pub temoin_decroche_a_la_panne: bool,
}

pub struct Soiree {
    pub m: Vec<Machine>,
    pub actif: usize,
    pub copie: Option<usize>,
    /// La copie reprise par l'état complet, à 19 h 45, et l'heure de son démarrage.
    pub c: Option<(Hub, u64)>,
    pub ecran: Ecran,
    pub ecran2: Option<Ecran>,
    pub ecran_piece: EcranPiece,
    pub monde: Monde,
    pub mur: u64,
    /// Les valeurs arrivées quand aucun module n'écoutait (la bascule, le redémarrage) : le
    /// module les relit à son démarrage, par des lectures fraîches.
    pub en_attente: Vec<Quoi>,
    pub en_panne: bool,
    /// Le pont vers le secours est coupé : plus rien ne lui arrive, plus aucun accusé.
    pub pont_coupe: bool,
    /// Ce que le témoin garde : l'actif y a inscrit que le secours avait décroché.
    pub temoin_decroche: bool,
    pub bilan: Bilan,
    pub defs: Vec<Def>,
}

fn regler(p: crate::catalogue::ParamDecl, cle: &str, tol: u16) -> FormeDecl {
    FormeDecl::regler(p, cle, tol)
}

fn sans(effet_: u8, retour_: u8) -> FormeDecl {
    FormeDecl::sans_param(effet_, retour_)
}

fn attend(effet_: u8, retour_: u8, cle: &str, attendus: &[i32], drapeaux: u8, suivi_s: u16) -> FormeDecl {
    FormeDecl { confirme: cle.to_string(), attendus: attendus.to_vec(), drapeaux, suivi_s, ..sans(effet_, retour_) }
}

/// Le dictionnaire des touches de la télé.
pub const DICO_TOUCHES: u16 = 2;

/// Les quinze appareils nommés de la soirée (pièces : 0 salon, 1 couloir, 2 cuisine, 3 chambre et
/// 4 cellier protégées, 5 entrée, 6 bureau).
pub fn appareils() -> Vec<AppDecl> {
    let mut position = regler(mesure(0, 100, 1, 0, 2), "position", 1);
    position.retour = retour::ACCUSE;
    position.drapeaux |= drapeau::A_L_ARRET;
    position.suivi_s = 120;
    let mut consigne = regler(mesure(40, 65, 1, 0, 1), "consigne", 0);
    consigne.retour = retour::ACCUSE;
    consigne.suivi_s = 180;
    let mut v = vec![
        AppDecl::physique(
            "z2m:salon_lampe",
            0,
            &["on", "brightness", "color_temp", "linkquality"],
            vec![("on", regler(binaire(), "on", 0)), ("brightness", regler(mesure(0, 254, 1, 0, 0), "brightness", 2)), ("color_temp", regler(mesure(153, 500, 1, 0, 0), "color_temp", 5))],
        ),
        AppDecl::physique("z2m:couloir_mouvement", 1, &["occupancy", "battery"], vec![]),
        AppDecl::physique("hue:couloir", 1, &["on", "brightness"], vec![("on", regler(binaire(), "on", 0)), ("brightness", regler(mesure(0, 100, 1, 0, 2), "brightness", 1))]),
        AppDecl::physique(
            "overkiz:volet_salon",
            0,
            &[],
            vec![("ouvrir", sans(effet::APPUI, retour::PONT)), ("fermer", sans(effet::APPUI, retour::PONT)), ("stop", sans(effet::APPUI, retour::PONT)), ("favori", sans(effet::APPUI, retour::PONT))],
        ),
        AppDecl::physique(
            "overkiz:volet_chambre",
            3,
            &["position", "moving"],
            vec![
                ("position", position.clone()),
                ("ouvrir", attend(effet::APPUI, retour::ACCUSE, "position", &[100], drapeau::A_L_ARRET, 120)),
                ("fermer", attend(effet::APPUI, retour::ACCUSE, "position", &[0], drapeau::A_L_ARRET, 120)),
                ("stop", sans(effet::APPUI, retour::AUCUN)),
            ],
        ),
        AppDecl::physique(
            "roborock:aspirateur",
            2,
            &["state", "battery", "error"],
            vec![
                ("demarrer", attend(effet::APPUI, retour::ACCUSE, "state", &[1, 5], 0, 60)),
                ("pause", attend(effet::APPUI, retour::ACCUSE, "state", &[10], 0, 60)),
                ("base", attend(effet::APPUI, retour::ACCUSE, "state", &[6, 8], 0, 60)),
            ],
        ),
        AppDecl::physique(
            "nuki:porte",
            5,
            &["lock"],
            vec![
                ("verrouiller", attend(effet::CONSIGNE, retour::ETAT, "lock", &[1], 0, 0)),
                ("deverrouiller", attend(effet::CONSIGNE, retour::ETAT, "lock", &[2], drapeau::OUVRE, 0)),
                ("tirer", attend(effet::APPUI, retour::ETAT, "lock", &[3], drapeau::OUVRE, 0)),
            ],
        ),
        AppDecl::physique("ariston:chauffe_eau", 4, &["temperature", "consigne"], vec![("consigne", consigne)]),
        AppDecl::physique(
            "sonos:salon",
            0,
            &["volume", "playing"],
            vec![
                ("volume", regler(mesure(0, 100, 1, 0, 2), "volume", 0)),
                ("volume_plus", sans(effet::APPUI, retour::PONT)),
                ("annonce", FormeDecl { params: vec![texte(200)], drapeaux: drapeau::TEXTE_PRIVE, ..sans(effet::APPUI, retour::PONT) }),
            ],
        ),
        AppDecl::physique(
            "samsung:tele",
            0,
            &["power"],
            vec![
                ("allumer", attend(effet::CONSIGNE, retour::ETAT, "power", &[1], drapeau::REVEILLABLE, 45)),
                ("touche", FormeDecl { params: vec![code(DICO_TOUCHES, 1, 40)], ..sans(effet::APPUI, retour::AUCUN) }),
            ],
        ),
        AppDecl::physique("telegram:famille", 6, &[], vec![("message", FormeDecl { params: vec![texte(1_000)], drapeaux: drapeau::TEXTE_PRIVE, ..sans(effet::APPUI, retour::PONT) })]),
        AppDecl::physique("helpers:sonnette", 5, &[], vec![("appuyer", FormeDecl { drapeaux: drapeau::PROPRIO_HUB, ..sans(effet::APPUI, retour::PONT) })]),
        AppDecl::physique(
            "bambu:imprimante",
            6,
            &["state"],
            vec![
                ("pause", attend(effet::APPUI, retour::ETAT, "state", &[2], drapeau::CONTROLE, 0)),
                ("reprise", attend(effet::APPUI, retour::ETAT, "state", &[1], drapeau::CONTROLE, 0)),
            ],
        ),
    ];
    for k in 1..=4 {
        v.push(AppDecl::physique(&format!("hue:sejour_{k}"), if k == 4 { 3 } else { 0 }, &["on"], vec![("on", regler(binaire(), "on", 0))]));
    }
    // Deux appareils virtuels : un groupe qui commande quatre ampoules, une « entrée » qui montre
    // des valeurs et des ordres d'autres appareils sans rien posséder.
    let mut groupe = regler(binaire(), "", 0);
    groupe.drapeaux |= drapeau::GROUPE;
    let mut membres = Vec::new();
    for k in 1..=4 {
        membres.push((sorte::ORDRE, format!("hue:sejour_{k}/on"), role::CIBLE, "on".to_string()));
        membres.push((sorte::APPAREIL, format!("hue:sejour_{k}"), role::MEMBRE, String::new()));
        membres.push((sorte::VALEUR, format!("hue:sejour_{k}/on"), role::AFFICHE, String::new()));
    }
    v.push(AppDecl { nom: "lumieres:sejour".into(), piece: 0, points: Vec::new(), defs: Vec::new(), ordres: vec![("on".into(), groupe)], membres });
    v.push(AppDecl {
        nom: "virtuel:entree".into(),
        piece: 5,
        points: Vec::new(),
        defs: Vec::new(),
        ordres: Vec::new(),
        membres: vec![
            (sorte::VALEUR, "nuki:porte/lock".into(), role::AFFICHE, String::new()),
            (sorte::ORDRE, "nuki:porte/verrouiller".into(), role::AFFICHE, String::new()),
            (sorte::ORDRE, "overkiz:volet_salon/ouvrir".into(), role::AFFICHE, String::new()),
            (sorte::ORDRE, "overkiz:volet_salon/fermer".into(), role::AFFICHE, String::new()),
            (sorte::VALEUR, "z2m:salon_lampe/on".into(), role::AFFICHE, String::new()),
        ],
    });
    v
}

/// Comment chaque appareil nommé répond.
fn comportements(hub: &Hub, monde: &mut Monde) {
    let rep = |n: &str| hub.appareils.numero(n).map(|x| x.place() as u32).expect("appareil");
    for (n, r) in [
        ("z2m:salon_lampe", Rep::Etat { ms: 300 }),
        ("hue:couloir", Rep::Etat { ms: 200 }),
        ("overkiz:volet_salon", Rep::Pont { ms: 600 }),
        ("overkiz:volet_chambre", Rep::Mouvement { accuse_ms: 400, pas_ms: 10_000, inter: vec![50, 30] }),
        ("roborock:aspirateur", Rep::Accuse { accuse_ms: 200, etat_ms: 2_000, valeur: None }),
        ("nuki:porte", Rep::Etat { ms: 1_000 }),
        ("ariston:chauffe_eau", Rep::Accuse { accuse_ms: 500, etat_ms: 150_000, valeur: None }),
        ("sonos:salon", Rep::Etat { ms: 400 }),
        ("samsung:tele", Rep::Reveil { ms: 20_000 }),
        ("telegram:famille", Rep::Pont { ms: 300 }),
        ("bambu:imprimante", Rep::Etat { ms: 1_500 }),
        ("hue:sejour_1", Rep::Etat { ms: 200 }),
        ("hue:sejour_2", Rep::Etat { ms: 200 }),
        ("hue:sejour_3", Rep::Etat { ms: 200 }),
        ("hue:sejour_4", Rep::Muet),
    ] {
        monde.reps.insert(rep(n), r);
    }
}

/// Les automatismes de la soirée, par les noms (comme un humain les a validés).
pub fn automatismes() -> Vec<Def> {
    let commander = |o: &str, a: Vec<Arg>| PasDef::Commander(o.into(), a);
    vec![
        Def { nom: "Couloir".into(), declencheur: DeclDef::Valeur("z2m:couloir_mouvement/occupancy".into(), Some(1)), pas: vec![PasDef::Bail("hue:couloir/on".into(), vec![Arg::N(1)], 120)] },
        Def { nom: "Départ".into(), declencheur: DeclDef::Main, pas: vec![commander("roborock:aspirateur/demarrer", vec![])] },
        Def { nom: "Chauffe-eau".into(), declencheur: DeclDef::Main, pas: vec![commander("ariston:chauffe_eau/consigne", vec![Arg::N(55)])] },
        // Un volet RTS n'a aucune valeur : seul un déclencheur sur son ordre peut y réagir.
        Def {
            nom: "Volets du soir".into(),
            declencheur: DeclDef::Ordre("overkiz:volet_salon/fermer".into(), 1 << issue::EXECUTE),
            pas: vec![commander("overkiz:volet_chambre/fermer", vec![])],
        },
        // Éteindre seulement une lumière dont la valeur suit la dernière commande de la maison : pas
        // celle qu'un humain a allumée, ni celle qu'un interrupteur a rallumée.
        Def {
            nom: "Extinction auto".into(),
            declencheur: DeclDef::Main,
            pas: vec![
                PasDef::Si(vec![
                    (genre::VIENT_DE, "z2m:salon_lampe/on".into(), op::EGAL, i64::from(resultat::OUI)),
                    (genre::ORIGINE, "z2m:salon_lampe/on".into(), 0, 1 << origine::AUTOMATISME),
                ]),
                commander("z2m:salon_lampe/on", vec![Arg::N(0)]),
            ],
        },
        Def { nom: "Lumière du soir".into(), declencheur: DeclDef::Main, pas: vec![commander("z2m:salon_lampe/on", vec![Arg::N(1)])] },
        Def {
            nom: "Alerte serrure".into(),
            declencheur: DeclDef::Ordre("nuki:porte/verrouiller".into(), 1 << issue::ENVOYE | 1 << issue::INCERTAIN | 1 << issue::ECHEC),
            pas: vec![PasDef::Prevenir("la porte n'a pas confirmé le verrouillage".into())],
        },
        Def {
            nom: "Nuit".into(),
            declencheur: DeclDef::Main,
            pas: vec![
                PasDef::Si(vec![(genre::EN_COURS, "roborock:aspirateur/demarrer".into(), op::EGAL | op::NON, 0)]),
                commander("lumieres:sejour/on", vec![Arg::N(0)]),
                commander("overkiz:volet_salon/fermer", vec![]),
            ],
        },
        Def { nom: "Eau chaude max".into(), declencheur: DeclDef::Main, pas: vec![commander("ariston:chauffe_eau/consigne", vec![Arg::N(62)])] },
    ]
}

fn configurer(hub: &mut Hub) {
    hub.pieces_protegees = vec![3, 4];
    for (nom, x) in personnes() {
        hub.ajouter_personne(&nom, x).expect("personne");
    }
    hub.principaux = principaux();
}

impl Soiree {
    pub fn new() -> Self {
        let mut a = Machine { hub: Hub::default(), moteur: Moteur { actif: true, de: P_MOTEUR, responsable_defaut: "Bob".into(), ..Moteur::default() }, kit: Kit::default(), debut_ms: h(17, 0, 0) };
        configurer(&mut a.hub);
        a.hub.catalogue.dicos.push(crate::maison_ordres::dico_mode());
        a.hub.catalogue.dicos.push((1..=40).map(|c| (c, format!("touche_{c}").into_boxed_str())).collect());
        // Un appui ne part qu'avec l'accusé du secours : il est en phase dès le départ.
        a.hub.secours_en_phase = true;
        let mut b = Machine { hub: Hub { actif: false, ..Hub::default() }, moteur: Moteur { de: P_MOTEUR, responsable_defaut: "Bob".into(), ..Moteur::default() }, kit: Kit::default(), debut_ms: h(17, 30, 0) };
        configurer(&mut b.hub);
        b.hub.catalogue.dicos = a.hub.catalogue.dicos.clone();
        Soiree {
            m: vec![a, b],
            actif: 0,
            copie: Some(1),
            c: None,
            ecran: Ecran::default(),
            ecran2: None,
            ecran_piece: EcranPiece { pieces: vec![0], ..EcranPiece::default() },
            monde: Monde::default(),
            mur: h(18, 0, 0),
            en_attente: Vec::new(),
            en_panne: false,
            pont_coupe: false,
            temoin_decroche: false,
            bilan: Bilan::default(),
            defs: automatismes(),
        }
    }

    fn t(&self) -> T {
        self.m[self.actif].t(self.mur)
    }

    pub fn declarer(&mut self, decls: &[AppDecl]) {
        let t = self.t();
        for d in decls {
            self.m[self.actif].hub.declarer(t, d).unwrap_or_else(|e| panic!("{} : {e}", d.nom));
        }
        self.propager();
        self.recompiler();
    }

    pub fn recompiler(&mut self) {
        for k in 0..self.m.len() {
            let Machine { hub, moteur, .. } = &mut self.m[k];
            moteur.defs = self.defs.clone();
            moteur.compiler(hub);
        }
    }

    /// Ce que l'actif a produit : ses ordres vont au kit, ses sorties au moteur, à la copie (qui
    /// accuse), à la copie par état complet et aux écrans, dans l'ordre de `s`.
    pub fn propager(&mut self) {
        loop {
            let t = self.t();
            let a = self.actif;
            let annulees = std::mem::take(&mut self.m[a].hub.annulations);
            if !annulees.is_empty() {
                let Machine { hub, kit, .. } = &mut self.m[a];
                kit.annuler(hub, t, &annulees);
            }
            let cmds = self.m[a].hub.remettre(t);
            for c in cmds {
                let Machine { hub, kit, .. } = &mut self.m[a];
                kit.recevoir(hub, &mut self.monde, t, c);
            }
            self.temoin_decroche |= self.m[a].hub.decroche;
            let sorties = std::mem::take(&mut self.m[a].hub.sorties);
            if sorties.is_empty() && self.m[a].hub.cmd.is_empty() {
                break;
            }
            for p in sorties {
                self.bilan.sorties += 1;
                self.bilan.octets_sorties += Hub::octets_sortie(&p);
                if !matches!(p.sortie, Sortie::Prive { .. }) {
                    let vu = format!("{:?}", p.sortie);
                    if vu.contains(SECRET_ANNONCE) || vu.contains(SECRET_TELEGRAM) {
                        self.bilan.secrets_hors_canal += 1;
                    }
                }
                {
                    let Machine { hub, moteur, .. } = &mut self.m[a];
                    match &p.sortie {
                        Sortie::Valeur { point, valeur, .. } => moteur.changement(hub, t, *point, *valeur),
                        Sortie::Trace { d, demande, ordre, .. } => {
                            if demande.parent != Ref(0) {
                                self.bilan.parents.insert(*d, demande.parent);
                            }
                            let i = if matches!(demande.etat, etat::LIBRE | etat::SUIVIE | etat::RETENUE) { demande.issue } else { issue::EN_COURS };
                            moteur.trace(hub, t, *d, *ordre, i, demande.etat == etat::LIBRE);
                        }
                        _ => {}
                    }
                }
                if let Some(c) = self.copie
                    && !self.pont_coupe
                {
                    let tc = self.m[c].t(self.mur);
                    self.m[c].hub.appliquer(&p, tc).expect("la copie applique la sortie");
                    self.m[c].moteur.appliquer(&p.sortie);
                    if matches!(p.sortie, Sortie::Prive { .. }) {
                        self.bilan.copie_tenait_le_prive = true;
                    }
                    // La copie accuse ce rang : un appui qui l'attendait peut partir.
                    self.m[a].hub.acquitter(p.s);
                }
                if let Some((c, debut)) = &mut self.c {
                    let tc = T { mono_ms: self.mur - *debut + 1_000, mur_ms: self.mur };
                    if p.e != c.epoque {
                        // Une époque nouvelle : la copie se refait par l'état complet du nouvel actif,
                        // qui contient déjà toutes les sorties de ce lot.
                        let source = &self.m[a].hub;
                        let image = source.ranger(self.m[a].t(self.mur));
                        let mut neuve = Hub::relire(&image, T { mono_ms: 1_000, mur_ms: self.mur }, principaux()).expect("état complet");
                        neuve.pieces_protegees = vec![3, 4];
                        *c = neuve;
                        *debut = self.mur;
                    } else if p.s > c.rang {
                        c.appliquer(&p, tc).expect("la copie par état complet applique la sortie");
                    }
                }
                let source = &self.m[a].hub;
                self.ecran.appliquer(&p, source);
                if let Some(e) = &mut self.ecran2 {
                    e.appliquer(&p, source);
                }
                self.ecran_piece.appliquer(&p, source);
            }
        }
        self.recolter();
    }

    fn recolter(&mut self) {
        for k in 0..self.m.len() {
            let res = std::mem::take(&mut self.m[k].hub.res);
            for r in res {
                self.bilan.issues.entry(r.seq).or_default().push((r.issue, r.raison, r.suite));
                self.bilan.instants.entry(r.seq).or_default().push(self.mur);
                if r.m > 0 {
                    self.bilan.groupes.insert(r.seq, (r.n, r.m));
                }
                if r.d != Ref(0) {
                    self.bilan.refs.entry(r.seq).or_insert(r.d);
                    self.bilan.toutes_refs.entry(r.seq).or_default().insert(r.d);
                }
            }
            let notices = std::mem::take(&mut self.m[k].hub.notices);
            self.bilan.notices.extend(notices);
        }
    }

    /// Le temps passe : les appareils répondent, les échéances tombent.
    pub fn jusqua(&mut self, fin: u64) {
        while self.mur < fin {
            let m = &self.m[self.actif];
            let t0 = m.t(self.mur);
            let suivant = self
                .monde
                .prochain()
                .unwrap_or(u64::MAX)
                .min(m.hub.prochaine(t0).unwrap_or(u64::MAX))
                .min(m.kit.prochaine(t0).unwrap_or(u64::MAX))
                .min(m.moteur.prochaine().unwrap_or(u64::MAX))
                .min(fin)
                .min(self.mur + 1_000);
            self.mur = suivant.max(self.mur + 1);
            let t = self.t();
            let a = self.actif;
            for q in self.monde.echus(self.mur) {
                if self.en_panne {
                    if matches!(q, Quoi::Valeur { .. }) {
                        self.en_attente.push(q);
                    }
                    continue;
                }
                let Machine { hub, kit, .. } = &mut self.m[a];
                kit.evenement(hub, &mut self.monde, t, q);
                self.propager();
            }
            if self.en_panne {
                continue;
            }
            {
                let Machine { hub, kit, moteur, .. } = &mut self.m[a];
                kit.tic(hub, &mut self.monde, t);
                hub.tic(t);
                moteur.echeances(hub, t);
            }
            self.propager();
        }
    }

    pub fn demander(&mut self, seq: u64, ordre: &str, args: Vec<Arg>, de: u16) -> Result<Ref, (u8, u8)> {
        self.demander_req(seq, ordre, args, de, |r| r)
    }

    pub fn demander_req(&mut self, seq: u64, ordre: &str, args: Vec<Arg>, de: u16, f: impl FnOnce(Req) -> Req) -> Result<Ref, (u8, u8)> {
        let t = self.t();
        // Le client émet sur la session de l'époque qu'il suit (revue v3, M7).
        let epoque = self.m[self.actif].hub.epoque;
        let r = f(Req { emise_epoque: epoque, ..Req::nouvelle(de, seq, Cible::Nom(ordre.into()), args, t) });
        let res = self.m[self.actif].hub.demander(t, r);
        if let Ok(x) = res {
            self.bilan.refs.entry(seq).or_insert(x);
            self.bilan.toutes_refs.entry(seq).or_default().insert(x);
        }
        self.propager();
        res
    }

    /// L'écran décide d'une retenue : il nomme l'ordre et la révision de la carte qu'il montre.
    pub fn decider_avec(&mut self, seq: u64, ordre: &str, revision_vue: Option<u32>, oui: bool) -> Result<Ref, &'static str> {
        let t = self.t();
        let a = self.actif;
        let x = *self.bilan.refs.get(&seq).ok_or("demande inconnue")?;
        let o = self.m[a].hub.ordre(ordre);
        let f = revision_vue.unwrap_or_else(|| self.m[a].hub.cartes.get(&x).map_or(0, |c| c.revision));
        let r = self.m[a].hub.decider(t, x, o, f, oui, P_ECRAN);
        self.propager();
        r
    }

    pub fn decider(&mut self, seq: u64, ordre: &str, oui: bool) -> Result<Ref, &'static str> {
        self.decider_avec(seq, ordre, None, oui)
    }

    pub fn lancer(&mut self, nom: &str) {
        let t = self.t();
        let Machine { hub, moteur, .. } = &mut self.m[self.actif];
        moteur.lancer_nom(hub, t, nom);
        self.propager();
    }

    /// Un geste physique : un interrupteur, un mouvement. La valeur arrive par le module, lue à
    /// l'instant.
    pub fn geste(&mut self, point: &str, v: i64) {
        let p = self.m[self.actif].hub.point(point);
        self.monde.prevoir(self.mur, Quoi::Valeur { point: p, v, mouvement: false, lu_ms: self.mur });
        self.jusqua(self.mur + 1);
    }

    pub fn relever(&mut self, moment: &str) {
        let a = self.m[self.actif].hub.empreinte();
        let b = self.copie.map_or(0, |c| self.m[c].hub.empreinte());
        let c = self.c.as_ref().map(|(h, _)| h.empreinte());
        self.bilan.empreintes.push((moment.into(), a, b, c));
        let hub = &self.m[self.actif].hub;
        let mut egales = 0;
        let mut total = 0;
        for (n, f) in &self.ecran.fiches {
            if hub.ordres.nom(Numero(*n)).is_some() {
                total += 1;
                egales += usize::from(derniere(&hub.catalogue.fiches[Numero(*n).place()]) == derniere(f));
            }
        }
        self.bilan.fiches_ecran.push((moment.into(), egales, total));
    }

    pub fn vue(&mut self, moment: &str, appareil: &str) {
        let hub = &self.m[self.actif].hub;
        let a = hub.appareils.numero(appareil).expect("appareil");
        let v = hub.vue(a).into_iter().map(|(s, _, nom)| format!("{}:{nom}", ["", "valeur", "ordre", "appareil"][usize::from(s)])).collect();
        self.bilan.vues.insert(format!("{moment} {appareil}"), v);
    }

    fn noter_vient_de(&mut self, moment: &str, ordre: &str) {
        let hub = &self.m[self.actif].hub;
        let o = hub.ordre(ordre);
        self.bilan.vient_de.push((moment.into(), crate::moteur::vient_de(hub, o)));
    }

    /// Le module d'une machine qui démarre relit l'état de ses appareils : des lectures fraîches.
    fn relire_les_appareils(&mut self) {
        let a = self.actif;
        let t = self.t();
        for q in std::mem::take(&mut self.en_attente) {
            let q = match q {
                Quoi::Valeur { point, v, mouvement, .. } => Quoi::Valeur { point, v, mouvement, lu_ms: self.mur },
                x => x,
            };
            let Machine { hub, kit, .. } = &mut self.m[a];
            kit.evenement(hub, &mut self.monde, t, q);
            self.propager();
        }
    }

    /// L'actif s'arrête net ; trois secondes plus tard (le bail du témoin), la copie devient l'actif.
    /// La reprise est sûre si le témoin ne dit pas que le secours avait décroché.
    pub fn bascule(&mut self) {
        let a = self.actif;
        let b = self.copie.expect("une copie");
        let envois: Vec<(String, Ref)> = self.m[a].kit.envois.iter().map(|r| ("A".to_string(), *r)).collect();
        self.bilan.envois.extend(envois);
        self.bilan.prives_lus.extend(self.m[a].kit.prives_lus.clone());
        self.bilan.temoin_decroche_a_la_panne = self.temoin_decroche;
        self.en_panne = true;
        self.jusqua(self.mur + 3_000);
        self.en_panne = false;
        self.actif = b;
        self.copie = None;
        self.pont_coupe = false;
        let t = self.t();
        {
            let Machine { hub, moteur, .. } = &mut self.m[b];
            hub.promouvoir(t, !self.temoin_decroche);
            moteur.actif = true;
        }
        self.m[b].kit = Kit::default();
        self.recompiler();
        self.propager();
        self.relire_les_appareils();
    }

    /// L'arrêt figé, l'instantané, le redémarrage : une horloge monotone neuve, et rien d'autre que
    /// les octets de l'instantané et la configuration (revue I20).
    pub fn redemarrer(&mut self) {
        let a = self.actif;
        let t = self.t();
        self.m[a].hub.arreter(t);
        self.propager();
        self.jusqua(self.mur + 1_000);
        let t = self.t();
        let image = self.m[a].hub.ranger(t);
        let image_moteur = self.m[a].moteur.ranger();
        let avant = self.m[a].hub.empreinte();
        let journal = std::mem::take(&mut self.m[a].moteur.journal);
        self.bilan.moteur.extend(journal);
        let envois: Vec<(String, Ref)> = self.m[a].kit.envois.iter().map(|r| ("B".to_string(), *r)).collect();
        self.bilan.envois.extend(envois);
        self.bilan.prives_lus.extend(self.m[a].kit.prives_lus.clone());
        self.bilan.instantane_octets = image.len() + image_moteur.len();
        let texte = String::from_utf8_lossy(&image);
        if texte.contains(SECRET_ANNONCE) || texte.contains(SECRET_TELEGRAM) {
            self.bilan.secrets_hors_canal += 1;
        }
        self.en_panne = true;
        self.jusqua(self.mur + 4_000);
        self.en_panne = false;
        let debut = self.mur;
        let t2 = T { mono_ms: 1_000, mur_ms: self.mur };
        let mut hub = Hub::relire(&image, t2, principaux()).expect("l'instantané se relit");
        hub.pieces_protegees = vec![3, 4];
        hub.redemarre(t2, true);
        self.bilan.instantane_identique = hub.empreinte() == avant;
        let mut moteur = Moteur::relire(&image_moteur, P_MOTEUR).expect("le moteur se relit");
        moteur.responsable_defaut = "Bob".into();
        moteur.defs = self.defs.clone();
        moteur.compiler(&mut hub);
        self.m[a] = Machine { hub, moteur, kit: Kit::default(), debut_ms: debut };
        self.relire_les_appareils();
    }

    pub fn retirer(&mut self, appareil: &str) {
        let t = self.t();
        let a = self.actif;
        let n = self.m[a].hub.appareils.numero(appareil).expect("appareil");
        {
            let Machine { hub, moteur, .. } = &mut self.m[a];
            moteur.retirer(hub, n);
            hub.retirer(t, n);
        }
        self.propager();
        self.recompiler();
    }

    pub fn lot(&mut self) {
        let t = self.t();
        self.m[self.actif].hub.lot(t, 30 * JOUR_MS);
        self.propager();
        self.recompiler();
    }

    /// Ce que les tests lisent à la fin.
    pub fn terminer(&mut self) {
        let a = self.actif;
        let envois: Vec<(String, Ref)> = self.m[a].kit.envois.iter().map(|r| ("B'".to_string(), *r)).collect();
        self.bilan.envois.extend(envois);
        self.bilan.prives_lus.extend(self.m[a].kit.prives_lus.clone());
        for m in &self.m {
            for args in &m.kit.args_envoyes {
                if format!("{args:?}").contains("SECRET-") {
                    self.bilan.secrets_hors_canal += 1;
                }
            }
        }
        let client = self.m[a].hub.ranger_client(&|_| true, &crate::banc::libelle, 0);
        if String::from_utf8_lossy(&client).contains("SECRET-") {
            self.bilan.secrets_hors_canal += 1;
        }
        self.bilan.prives_restants = self.m.iter().map(|m| m.hub.prives.len()).sum::<usize>() + self.c.as_ref().map_or(0, |(h, _)| h.prives.len());
        let fin: Vec<(u64, String, String)> = self.m.iter().flat_map(|m| m.moteur.journal.clone()).collect();
        self.bilan.moteur.extend(fin);
        self.bilan.moteur.sort_by_key(|x| x.0);
        self.bilan.ecran2_demandes_meta = self.ecran2.as_ref().map_or(0, |e| e.demandes_meta);
        self.bilan.refus_epoque = self.m.iter().map(|m| m.kit.refus_epoque).sum();
        self.bilan.ecran_piece = (self.ecran_piece.trous, self.ecran_piece.rechargements);
        let hub = &self.m[a].hub;
        self.bilan.ecran_piece_identique = self.ecran_piece.fiches.iter().all(|(o, f)| hub.ordres.nom(Numero(*o)).is_none() || derniere(&hub.catalogue.fiches[Numero(*o).place()]) == derniere(f)) && self.ecran_piece.fiches.len() > 5;
    }
}

/// La soirée entière.
pub fn jouer() -> Soiree {
    let mut s = Soiree::new();
    let m = crate::maison::generer(1);
    // 18:00 — la maison se déclare : 496 appareils du banc, puis les quinze de la soirée.
    let mut decls = crate::maison_ordres::declarations(&m);
    decls.extend(appareils());
    // L'écran de pièce démarre avant : il charge l'état (vide), puis suit le flux de sa pièce.
    s.ecran_piece.charger(&s.m[0].hub);
    s.declarer(&decls);
    comportements(&s.m[0].hub, &mut s.monde);
    {
        let h0 = &s.m[0].hub;
        s.bilan.catalogue = (h0.points.len(), h0.ordres.len(), h0.catalogue.nb_formes());
        let rts = h0.appareils.numero("overkiz:volet_salon").expect("rts");
        let app = h0.apps[rts.place()];
        s.bilan.ordres_rts = (usize::from(app.nb_points), usize::from(app.nb_ordres));
    }
    s.bilan.ecran_demandes_meta = s.ecran.demandes_meta;
    s.vue("18:00", "lumieres:sejour");
    s.vue("18:00", "virtuel:entree");
    {
        let h0 = &s.m[0].hub;
        let p = h0.point("z2m:salon_lampe/on");
        s.bilan.aussi_dans = h0.aussi_dans(sorte::VALEUR, p).into_iter().map(|a| h0.appareils.nom(a).unwrap_or("").to_string()).collect();
        // Un écran et un agent valident avec la même forme que le hub : sur des valeurs tirées.
        let mut identiques = 0;
        let mut total = 0;
        let mut alea = crate::outils::Alea::new(7);
        for o in 0..h0.ordres.len() {
            let n = h0.ordres.numero_de(o);
            let f = h0.catalogue.fiches[o].forme;
            for _ in 0..4 {
                let args: Vec<Arg> = h0
                    .catalogue
                    .params_de(f)
                    .iter()
                    .map(|p| {
                        if matches!(p.nature, crate::catalogue::nature::TEXTE | crate::catalogue::nature::OBJET) {
                            Arg::T("x".repeat(alea.sous(1_200)).into())
                        } else {
                            Arg::N(i64::from(p.min) - 5 + alea.sous((p.max - p.min + 11) as usize) as i64)
                        }
                    })
                    .collect();
                total += 1;
                identiques += usize::from(s.ecran.valider(n, &args) == h0.catalogue.valider(f, &args));
            }
        }
        s.bilan.validations_identiques = (identiques, total);
    }
    // 18:05 — la lampe du salon, depuis l'écran ; une luminosité hors de sa forme ; une bonne.
    s.mur = h(18, 5, 0);
    let _ = s.demander(1, "z2m:salon_lampe/on", vec![Arg::N(1)], P_ECRAN);
    s.jusqua(h(18, 5, 30));
    let _ = s.demander(2, "z2m:salon_lampe/brightness", vec![Arg::N(300)], P_ECRAN);
    s.jusqua(h(18, 6, 0));
    let _ = s.demander(3, "z2m:salon_lampe/brightness", vec![Arg::N(128)], P_ECRAN);
    // 18:10 — un mouvement au couloir : la veilleuse, pendant 2 min (un bail).
    s.jusqua(h(18, 10, 0));
    s.geste("z2m:couloir_mouvement/occupancy", 1);
    s.jusqua(h(18, 11, 0));
    s.geste("z2m:couloir_mouvement/occupancy", 0);
    // 18:20 — l'assistant ouvre le volet RTS : exécuté par la box, sans aucune valeur.
    s.jusqua(h(18, 20, 0));
    let _ = s.demander(4, "overkiz:volet_salon/ouvrir", vec![], P_ASSISTANT);
    s.jusqua(h(18, 21, 0));
    {
        let hub = &s.m[s.actif].hub;
        let f = hub.catalogue.fiches[hub.ordre("overkiz:volet_salon/ouvrir").place()];
        s.bilan.derniere_rts = Some((f.valeur, f.issue, f.origine, f.instant_ms));
    }
    // 18:22 — l'invité (le bureau seulement) : la porte d'entrée et le groupe du séjour lui sont
    // refusés ; arrêter l'imprimante du bureau, il le propose et le propriétaire l'approuve (une
    // machine : la v2 la laissait passer, revue v3 D2). Ce qu'il prétend être n'y change rien (B1).
    s.jusqua(h(18, 22, 0));
    let _ = s.demander(40, "nuki:porte/deverrouiller", vec![], P_INVITE);
    let _ = s.demander(41, "lumieres:sejour/on", vec![Arg::N(1)], P_INVITE);
    let _ = s.demander_req(42, "bambu:imprimante/pause", vec![], P_INVITE, |r| Req { qui: 1, ..r });
    s.jusqua(h(18, 22, 30));
    {
        let hub = &s.m[s.actif].hub;
        let f = hub.catalogue.fiches[hub.ordre("bambu:imprimante/pause").place()];
        s.bilan.qui_invite = f.qui;
        s.bilan.pour_invite = f.pour;
    }
    let _ = s.decider(42, "bambu:imprimante/pause", true);
    s.jusqua(h(18, 23, 0));
    // 18:30 — un agent veut tirer le pêne : retenu ; une pause d'impression : retenue aussi.
    s.jusqua(h(18, 30, 0));
    let _ = s.demander(5, "nuki:porte/tirer", vec![], P_MCP);
    s.jusqua(h(18, 31, 0));
    let _ = s.demander(6, "bambu:imprimante/pause", vec![], P_MCP);
    // 18:33 — l'humain décide de la première, d'abord sur la mauvaise carte.
    s.jusqua(h(18, 33, 0));
    let mauvaise = s.decider(5, "nuki:porte/verrouiller", true);
    s.bilan.notices.push(format!("décision sur la mauvaise carte : {}", mauvaise.err().unwrap_or("acceptée")));
    let _ = s.decider(5, "nuki:porte/tirer", true);
    // 18:35 — l'agent du gardien (l'entrée, jusqu'à 18 h 40) veut tirer le pêne : retenu. Son
    // permis finit avant la décision : la retenue tombe, et l'approbation de 18 h 50 ne vaut plus (I1).
    s.jusqua(h(18, 35, 0));
    let _ = s.demander(43, "nuki:porte/tirer", vec![], P_GARDIEN);
    // 18:40 — le groupe du séjour : quatre ampoules, l'une coupée au mur.
    s.jusqua(h(18, 40, 0));
    let _ = s.demander(7, "lumieres:sejour/on", vec![Arg::N(1)], P_ECRAN);
    // 18:42 — le même ordre par un agent : une ampoule est dans une pièce protégée ; retenu, puis
    // approuvé à 18 h 43 : la mère se répartit, ses filles ne sont pas retenues de nouveau (I7).
    s.jusqua(h(18, 42, 0));
    let _ = s.demander(30, "lumieres:sejour/on", vec![Arg::N(1)], P_MCP);
    s.jusqua(h(18, 43, 0));
    let _ = s.decider(30, "lumieres:sejour/on", true);
    // 18:45 — trois « volume + », puis trois consignes de volume en 150 ms. L'écran de la pièce
    // perd un message du salon : il le voit au suivant et se recharge (I13).
    s.jusqua(h(18, 45, 0));
    s.ecran_piece.perdre = Some(0);
    for (k, (o, a)) in [
        ("sonos:salon/volume_plus", vec![]),
        ("sonos:salon/volume_plus", vec![]),
        ("sonos:salon/volume_plus", vec![]),
        ("sonos:salon/volume", vec![Arg::N(20)]),
        ("sonos:salon/volume", vec![Arg::N(25)]),
        ("sonos:salon/volume", vec![Arg::N(30)]),
    ]
    .into_iter()
    .enumerate()
    {
        let _ = s.demander(8 + k as u64, o, a, P_ECRAN);
        s.jusqua(s.mur + 50);
    }
    // 18:46 — deux textes privés : une annonce, un message de l'assistant (B5).
    s.jusqua(h(18, 46, 0));
    let _ = s.demander(50, "sonos:salon/annonce", vec![Arg::T(SECRET_ANNONCE.into())], P_ECRAN2);
    s.jusqua(h(18, 47, 0));
    let _ = s.demander(51, "telegram:famille/message", vec![Arg::T(SECRET_TELEGRAM.into())], P_ASSISTANT);
    // 18:50 — la sonnette, par un script qui retente avec la même clé ; l'agent du gardien est
    // approuvé trop tard.
    s.jusqua(h(18, 50, 0));
    let _ = s.demander(14, "helpers:sonnette/appuyer", vec![], P_API);
    let _ = s.demander_req(14, "helpers:sonnette/appuyer", vec![], P_API, |r| Req { tentative: 2, ..r });
    s.bilan.decision_gardien = Some(s.decider(43, "nuki:porte/tirer", true));
    // 18:52 — une demande arrivée après son échéance (I2).
    s.jusqua(h(18, 52, 0));
    let t = s.t();
    let _ = s.demander_req(44, "z2m:salon_lampe/brightness", vec![Arg::N(100)], P_ECRAN2, |r| Req { echeance_mur_ms: t.mur_ms - 1, ..r });
    // 19:00 — Départ : l'aspirateur, accepté puis confirmé par son état.
    s.jusqua(h(19, 0, 0));
    s.lancer("Départ");
    // 19:10 — le chauffe-eau : accepté, puis la suite, 150 s plus tard. 19:11 — une autre consigne
    // depuis l'écran : la suite tardive de la première ne la remplace pas dans la fiche (I6).
    s.jusqua(h(19, 10, 0));
    s.lancer("Chauffe-eau");
    s.jusqua(h(19, 11, 0));
    let _ = s.demander(46, "ariston:chauffe_eau/consigne", vec![Arg::N(54)], P_ECRAN2);
    s.jusqua(h(19, 12, 31));
    {
        let hub = &s.m[s.actif].hub;
        s.bilan.fiche_chauffe_eau = Some(hub.catalogue.fiches[hub.ordre("ariston:chauffe_eau/consigne").place()]);
    }
    // 19:20 — la télé endort ses ordres : envoyé, puis la suite quand elle reparaît.
    s.jusqua(h(19, 20, 0));
    let _ = s.demander(16, "samsung:tele/allumer", vec![], P_ECRAN);
    // 19:30 — une pause de l'aspirateur, et son module meurt avec l'ordre.
    s.jusqua(h(19, 30, 0));
    let _ = s.demander(17, "roborock:aspirateur/pause", vec![], P_ECRAN);
    s.jusqua(s.mur + 100);
    {
        let a = s.actif;
        let rob = s.m[a].hub.appareils.numero("roborock:aspirateur").expect("robot").place() as u32;
        let t = s.t();
        let Machine { hub, kit, .. } = &mut s.m[a];
        kit.mourir(&[rob]);
        hub.module_mort(t, &[rob]);
        let mut morts = BTreeSet::new();
        morts.insert(rob);
        s.monde.oublier(&morts, hub);
        s.propager();
    }
    // 19:35 — une issue tardive, pour une demande dont la case sert maintenant à une autre.
    s.jusqua(h(19, 35, 0));
    {
        let a = s.actif;
        if let Ok(r) = s.demander(18, "z2m:salon_lampe/color_temp", vec![Arg::N(300)], P_ECRAN) {
            let g = r.generation() - 1;
            let t = s.t();
            s.m[a].hub.issue_module(t, Ref::new(r.case() as u16, r.epoque(), g), issue::ECHEC, 0);
            s.propager();
        }
    }
    // 19:45 — une copie arrive par l'état complet, puis suit le flux.
    s.jusqua(h(19, 45, 0));
    {
        let t = s.t();
        let image = s.m[s.actif].hub.ranger(t);
        let mut c = Hub::relire(&image, T { mono_ms: 1_000, mur_ms: t.mur_ms }, principaux()).expect("état complet");
        c.pieces_protegees = vec![3, 4];
        s.c = Some((c, t.mur_ms));
    }
    // 20:00 — Extinction auto : la lampe a été allumée par un humain, elle reste.
    s.jusqua(h(20, 0, 0));
    s.lancer("Extinction auto");
    // 20:10 — quelqu'un éteint et rallume à l'interrupteur : la valeur ne suit plus la commande (I5).
    s.jusqua(h(20, 10, 0));
    s.geste("z2m:salon_lampe/on", 0);
    s.jusqua(h(20, 10, 5));
    s.geste("z2m:salon_lampe/on", 1);
    s.jusqua(h(20, 12, 0));
    s.noter_vient_de("20:12 après l'interrupteur", "z2m:salon_lampe/on");
    s.jusqua(h(20, 13, 0));
    s.geste("z2m:salon_lampe/on", 0);
    // 20:15 — la maison rallume la lampe éteinte : c'est sa commande que la valeur suit.
    s.jusqua(h(20, 15, 0));
    s.lancer("Lumière du soir");
    s.jusqua(h(20, 16, 0));
    s.noter_vient_de("20:16 après la maison", "z2m:salon_lampe/on");
    // 20:40 — Extinction auto : cette fois, la lampe est éteinte.
    s.jusqua(h(20, 40, 0));
    s.lancer("Extinction auto");
    // 20:41 — une teinte déjà là : confirmée par une lecture, sans variation : on ne sait pas.
    s.jusqua(h(20, 41, 0));
    let _ = s.demander(52, "z2m:salon_lampe/color_temp", vec![Arg::N(300)], P_ECRAN2);
    s.jusqua(h(20, 41, 30));
    s.noter_vient_de("20:41 une valeur déjà là", "z2m:salon_lampe/color_temp");
    // 20:42 — une luminosité, et un geste à la main pendant la latence : deux variations avant la
    // preuve : on ne sait pas non plus.
    s.jusqua(h(20, 42, 0));
    let _ = s.demander(53, "z2m:salon_lampe/brightness", vec![Arg::N(150)], P_ECRAN2);
    s.jusqua(h(20, 42, 0) + 50);
    s.geste("z2m:salon_lampe/brightness", 50);
    s.jusqua(h(20, 43, 0));
    s.noter_vient_de("20:43 une variation avant la preuve", "z2m:salon_lampe/brightness");
    // 20:45 — on ferme le volet RTS : « Volets du soir » réagit à l'ordre, faute de valeur.
    s.jusqua(h(20, 45, 0));
    let _ = s.demander(19, "overkiz:volet_salon/fermer", vec![], P_ECRAN);
    // 20:50 — la serrure ne répond pas : envoyé, sans confirmation ; l'alerte part sur l'issue.
    s.jusqua(h(20, 50, 0));
    let nuki = s.m[s.actif].hub.appareils.numero("nuki:porte").expect("nuki").place() as u32;
    s.monde.reps.insert(nuki, Rep::Muet);
    let _ = s.demander(20, "nuki:porte/verrouiller", vec![], P_ECRAN);
    s.jusqua(h(20, 51, 0));
    s.monde.reps.insert(nuki, Rep::Etat { ms: 1_000 });
    // 20:59 — avant la bascule : un bail, une retenue, un suivi, un groupe à demi conclu, un ordre
    // en vol, un ordre en file.
    s.jusqua(h(20, 59, 30));
    s.geste("z2m:couloir_mouvement/occupancy", 1);
    s.jusqua(h(20, 59, 31));
    s.geste("z2m:couloir_mouvement/occupancy", 0);
    s.jusqua(h(20, 59, 40));
    let _ = s.demander(36, "bambu:imprimante/pause", vec![], P_MCP);
    s.jusqua(h(20, 59, 50));
    let _ = s.demander(21, "nuki:porte/tirer", vec![], P_MCP);
    s.jusqua(h(20, 59, 55));
    let _ = s.demander(22, "ariston:chauffe_eau/consigne", vec![Arg::N(50)], P_ECRAN);
    s.jusqua(h(20, 59, 58));
    let _ = s.demander(47, "lumieres:sejour/on", vec![Arg::N(0)], P_ECRAN2);
    s.jusqua(h(21, 0, 0) - 300);
    let _ = s.demander(23, "overkiz:volet_chambre/position", vec![Arg::N(30)], P_ECRAN);
    s.jusqua(h(21, 0, 0) - 200);
    let _ = s.demander(24, "overkiz:volet_chambre/position", vec![Arg::N(60)], P_ECRAN);
    s.jusqua(h(21, 0, 0) - 100);
    let _ = s.demander(25, "z2m:salon_lampe/brightness", vec![Arg::N(200)], P_ECRAN);
    s.jusqua(h(21, 0, 0) - 50);
    s.relever("avant la bascule");
    // Le pont vers le secours se coupe ; un « volume + » demandé 30 ms avant la panne attend
    // l'accusé qui ne viendra pas : il ne part pas (B2).
    s.jusqua(h(21, 0, 0) - 40);
    s.pont_coupe = true;
    s.jusqua(h(21, 0, 0) - 30);
    let _ = s.demander(49, "sonos:salon/volume_plus", vec![], P_ECRAN2);
    // 21:00 — l'actif s'arrête net ; la copie devient l'actif.
    s.jusqua(h(21, 0, 0));
    s.bascule();
    // 21:00:10 — une lecture d'avant la bascule arrive en retard : elle ne confirme rien (I4).
    s.jusqua(h(21, 0, 10));
    {
        let p = s.m[s.actif].hub.point("overkiz:volet_chambre/position");
        s.monde.prevoir(s.mur, Quoi::Valeur { point: p, v: 60, mouvement: false, lu_ms: h(21, 0, 0) - 200 });
    }
    // 21:05 — la retenue de 20:59 est décidée sur le nouvel actif ; la sonnette retentée.
    s.jusqua(h(21, 5, 0));
    let _ = s.decider(21, "nuki:porte/tirer", true);
    s.jusqua(h(21, 6, 0));
    let _ = s.demander_req(14, "helpers:sonnette/appuyer", vec![], P_API, |r| Req { tentative: 3, premiere_mur_ms: h(18, 50, 0), ..r });
    // 21:06:30 — l'écran retente son « volume + » : le nouvel actif ne le connaît pas, la reprise est
    // sûre : il part, une fois.
    s.jusqua(h(21, 6, 30));
    let avant = s.m[s.actif].hub.epoque - 1;
    let _ = s.demander_req(49, "sonos:salon/volume_plus", vec![], P_ECRAN2, |r| Req { tentative: 2, premiere_mur_ms: h(21, 0, 0) - 30, emise_epoque: avant, ..r });
    // 21:09:40 — l'écran approuve la retenue de 20:59:40 à l'instant même de son échéance : la
    // décision la voit expirée, sans attendre le balayage (I2).
    s.jusqua(h(21, 9, 40) - 1);
    s.mur = h(21, 9, 40);
    s.bilan.decision_trop_tard = Some(s.decider(36, "bambu:imprimante/pause", true));
    s.jusqua(h(21, 10, 0));
    s.relever("après la bascule");
    // 21:30 — Nuit, sur le nouvel actif.
    s.jusqua(h(21, 30, 0));
    s.lancer("Nuit");
    // 21:58 — avant le redémarrage : une retenue, un suivi, un ordre vers la télé, un bail.
    s.jusqua(h(21, 58, 0));
    let _ = s.demander(27, "bambu:imprimante/reprise", vec![], P_MCP);
    s.jusqua(h(21, 58, 2));
    s.geste("z2m:couloir_mouvement/occupancy", 1);
    s.jusqua(h(21, 58, 3));
    s.geste("z2m:couloir_mouvement/occupancy", 0);
    s.jusqua(h(21, 59, 30));
    let _ = s.demander(28, "ariston:chauffe_eau/consigne", vec![Arg::N(52)], P_ECRAN);
    s.jusqua(h(21, 59, 59));
    let _ = s.demander(29, "samsung:tele/allumer", vec![], P_ECRAN);
    s.jusqua(h(22, 0, 0));
    s.redemarrer();
    // 22:03 — la retenue de 21:58, décidée après le redémarrage.
    s.jusqua(h(22, 3, 0));
    let _ = s.decider(27, "bambu:imprimante/reprise", true);
    s.jusqua(h(22, 10, 0));
    // 22:20 — le volet RTS est retiré.
    s.jusqua(h(22, 20, 0));
    let ancien_rts = s.m[s.actif].hub.ordre("overkiz:volet_salon/ouvrir");
    s.retirer("overkiz:volet_salon");
    let _ = s.demander(31, "overkiz:volet_salon/ouvrir", vec![], P_ECRAN);
    s.bilan.a_revoir.push(("22:20".into(), s.m[s.actif].moteur.a_revoir()));
    s.vue("22:20", "virtuel:entree");
    s.jusqua(h(22, 30, 0));
    // Un mois plus tard — le lot libère ses places ; un volet io neuf les prend.
    s.mur += 30 * JOUR_MS;
    // Un écran resté éteint pendant le lot revient avec son cache d'avant.
    let ecran_perime = s.ecran.clone();
    let places_rts: Vec<usize> = {
        let hub = &s.m[s.actif].hub;
        let a = hub.appareils.numero("overkiz:volet_salon").expect("rts");
        hub.ordres_de(a.place()).iter().map(|&o| Numero(o).place()).collect()
    };
    s.lot();
    s.bilan.fiches_liberees_a_zero = places_rts.iter().all(|&p| s.m[s.actif].hub.catalogue.fiches[p] == Fiche::default());
    s.ecran2 = Some(Ecran { demandes_meta: 0, ..ecran_perime });
    let neuf = AppDecl::physique(
        "overkiz:volet_salon_io",
        0,
        &["position", "moving"],
        vec![
            ("position", {
                let mut p = regler(mesure(0, 100, 1, 0, 2), "position", 1);
                p.retour = retour::ACCUSE;
                p.drapeaux |= drapeau::A_L_ARRET;
                p.suivi_s = 120;
                p
            }),
            ("ouvrir", attend(effet::APPUI, retour::ACCUSE, "position", &[100], drapeau::A_L_ARRET, 120)),
            ("fermer", attend(effet::APPUI, retour::ACCUSE, "position", &[0], drapeau::A_L_ARRET, 120)),
            ("stop", sans(effet::APPUI, retour::AUCUN)),
        ],
    );
    s.declarer(&[neuf]);
    {
        let a = s.actif;
        let hub = &s.m[a].hub;
        let n = hub.ordre("overkiz:volet_salon_io/position");
        s.bilan.nouvelle_fiche = Some(hub.catalogue.fiches[n.place()]);
        let t = s.t();
        // Un client à vieux cache commande par l'ancien numéro : refusé, jamais le volet neuf.
        let r = s.m[a].hub.demander(t, Req::nouvelle(P_ECRAN, 32, Cible::Numero(ancien_rts), vec![], t));
        s.bilan.par_numero_perime = r.err();
        s.propager();
    }
    s.vue("lot", "virtuel:entree");
    s.jusqua(s.mur + 5 * 60_000);
    // Un agent demande 63 °C, puis 55 °C, au chauffe-eau (pièce protégée) : retenus.
    let _ = s.demander(33, "ariston:chauffe_eau/consigne", vec![Arg::N(63)], P_MCP);
    let _ = s.demander(45, "ariston:chauffe_eau/consigne", vec![Arg::N(55)], P_MCP);
    let carte_45 = s.m[s.actif].hub.cartes.get(&s.bilan.refs[&45]).map(|c| c.revision);
    s.jusqua(s.mur + 5 * 60_000);
    // Le chauffe-eau se redéclare : 60 °C au plus.
    let mut ariston = appareils().into_iter().find(|d| d.nom == "ariston:chauffe_eau").expect("ariston");
    ariston.ordres[0].1.params[0].max = 60;
    s.declarer(&[ariston]);
    s.bilan.a_revoir.push(("redéclaration".into(), s.m[s.actif].moteur.a_revoir()));
    {
        let o = s.m[s.actif].hub.ordre("ariston:chauffe_eau/consigne");
        s.bilan.ecran2_valide_62 = s.ecran2.as_ref().map(|e| e.valider(o, &[Arg::N(62)]));
    }
    s.jusqua(s.mur + 2 * 60_000);
    // 63 °C : la forme a changé, revalidée, refusée. 55 °C : la forme a changé, mais 55 tient : une
    // nouvelle carte ; un écran qui approuve encore l'ancienne carte est refusé ; la nouvelle passe (I3).
    let _ = s.decider(33, "ariston:chauffe_eau/consigne", true);
    let _ = s.decider_avec(45, "ariston:chauffe_eau/consigne", carte_45, true);
    s.bilan.decision_carte_perimee = Some(s.decider_avec(45, "ariston:chauffe_eau/consigne", carte_45, true));
    let _ = s.decider(45, "ariston:chauffe_eau/consigne", true);
    s.jusqua(s.mur + 3 * 60_000);
    let _ = s.demander(34, "ariston:chauffe_eau/consigne", vec![Arg::N(62)], P_ECRAN);
    let _ = s.demander(35, "ariston:chauffe_eau/consigne", vec![Arg::N(58)], P_ECRAN);
    s.jusqua(s.mur + 4 * 60_000);
    // La lampe du salon se redéclare : une valeur en plus au milieu, une en moins. Les numéros
    // suivent les clés ; la luminosité est toujours prouvée par la luminosité (I11).
    {
        let avant: Vec<Numero> = ["on", "brightness", "color_temp", "linkquality"].iter().map(|k| s.m[s.actif].hub.point(&format!("z2m:salon_lampe/{k}"))).collect();
        let mut lampe = appareils().into_iter().find(|d| d.nom == "z2m:salon_lampe").expect("lampe");
        lampe.points = ["on", "energy", "brightness", "color_temp"].iter().map(|k| k.to_string()).collect();
        s.declarer(&[lampe]);
        let hub = &s.m[s.actif].hub;
        let apres: Vec<Numero> = ["on", "brightness", "color_temp", "energy"].iter().map(|k| hub.point(&format!("z2m:salon_lampe/{k}"))).collect();
        let retire = !hub.points.vivant(avant[3]);
        s.bilan.redeclaration = Some((avant, apres, retire));
    }
    let _ = s.demander(48, "z2m:salon_lampe/brightness", vec![Arg::N(180)], P_ECRAN2);
    s.jusqua(s.mur + 60_000);
    s.relever("fin");
    s.terminer();
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::raison;
    use std::sync::OnceLock;

    fn soiree() -> &'static Soiree {
        static S: OnceLock<Soiree> = OnceLock::new();
        S.get_or_init(jouer)
    }

    fn issues(seq: u64) -> Vec<(u8, u8, bool)> {
        soiree().bilan.issues.get(&seq).cloned().unwrap_or_default()
    }

    fn fin(seq: u64) -> u8 {
        issues(seq).last().map_or(0, |x| x.0)
    }

    fn envois_de(seq: u64) -> usize {
        let b = &soiree().bilan;
        let r = b.refs[&seq];
        b.envois.iter().filter(|(_, x)| *x == r).count()
    }

    #[test]
    fn chacun_connait_l_appareil_l_ordre_et_sa_structure() {
        let b = &soiree().bilan;
        assert_eq!(b.validations_identiques.0, b.validations_identiques.1, "l'écran valide comme le hub, avec la même forme");
        assert!(b.validations_identiques.1 > 3_000);
        let e = soiree().bilan.empreintes.iter().find(|e| e.0 == "avant la bascule").expect("relevé");
        assert_eq!(e.1, e.2, "la copie de secours a la même mémoire des ordres");
        assert_eq!(Some(e.1), e.3, "la copie reprise par l'état complet aussi");
        let (_, egales, total) = b.fiches_ecran.iter().find(|x| x.0 == "avant la bascule").cloned().expect("relevé");
        assert_eq!(egales, total, "l'écran lit la même dernière commande que le hub");
        assert!(total > 10);
        assert_eq!(b.ecran_demandes_meta as usize, b.catalogue.2, "l'écran a demandé chaque forme une fois");
    }

    #[test]
    fn les_valeurs_et_les_ordres_ne_se_melangent_pas() {
        let b = &soiree().bilan;
        assert_eq!(b.ordres_rts, (0, 4), "un volet RTS n'a aucune valeur, et quatre ordres");
        let (points, ordres, formes) = b.catalogue;
        assert_eq!(points, 2_264 + 24, "les ordres ne prennent aucune place de valeur");
        assert!(ordres > 600 && formes < 40, "{ordres} ordres, {formes} formes");
    }

    #[test]
    fn une_valeur_hors_de_sa_forme_n_atteint_aucun_module() {
        assert_eq!(issues(2), vec![(issue::REFUSE, raison::BORNES, false)]);
        assert_eq!(issues(34), vec![(issue::REFUSE, raison::BORNES, false)], "62 °C, après la redéclaration à 60 °C au plus");
        assert_eq!(fin(35), issue::CONFIRME);
    }

    #[test]
    fn chaque_issue_tombe_sur_sa_demande() {
        assert_eq!(fin(1), issue::CONFIRME);
        assert_eq!(fin(3), issue::CONFIRME);
        assert_eq!(fin(4), issue::EXECUTE);
        assert!(soiree().bilan.notices.iter().any(|n| n.starts_with("issue tardive écartée")), "{:?}", soiree().bilan.notices);
        assert_eq!(fin(18), issue::CONFIRME, "la demande qui a pris la case n'a pas reçu l'issue de l'ancienne");
        assert_eq!(issues(18).len(), 1);
        let m = &soiree().bilan.moteur;
        assert!(m.iter().any(|(_, a, x)| a == "Départ" && x.starts_with("issue accepte")), "{m:?}");
    }

    #[test]
    fn les_consignes_se_fusionnent_les_appuis_jamais() {
        for seq in [8, 9, 10] {
            assert_eq!(fin(seq), issue::EXECUTE, "volume + n°{seq}");
        }
        assert_eq!(issues(11), vec![(issue::NON_ENVOYE, raison::REMPLACE, false)]);
        assert_eq!(issues(12), vec![(issue::NON_ENVOYE, raison::REMPLACE, false)]);
        assert_eq!(fin(13), issue::CONFIRME);
    }

    #[test]
    fn une_retenue_n_approuve_que_ce_qu_elle_montre() {
        let b = &soiree().bilan;
        assert!(b.notices.iter().any(|n| n == "décision sur la mauvaise carte : cette demande porte sur un autre ordre que celui montré"));
        assert_eq!(issues(5), vec![(issue::RETENUE, raison::GARDE, false), (issue::CONFIRME, 0, false)]);
        assert_eq!(issues(6), vec![(issue::RETENUE, raison::GARDE, false), (issue::REFUSE, raison::EXPIREE, false)]);
        assert_eq!(issues(21), vec![(issue::RETENUE, raison::GARDE, false), (issue::CONFIRME, 0, false)], "retenue avant la bascule, décidée après");
        assert_eq!(issues(27), vec![(issue::RETENUE, raison::GARDE, false), (issue::CONFIRME, 0, false)], "retenue avant le redémarrage, décidée après");
        assert_eq!(issues(33), vec![(issue::RETENUE, raison::GARDE, false), (issue::REFUSE, raison::BORNES, false)], "approuvée, mais la forme a changé : revalidée, refusée");
        assert_eq!(issues(36), vec![(issue::RETENUE, raison::GARDE, false), (issue::REFUSE, raison::EXPIREE, false)]);
        let quand = b.instants[&36][1];
        assert!(quand >= h(21, 9, 40) && quand < h(21, 9, 41), "la retenue expire à son heure chez le nouvel actif : {quand}");
    }

    /// B1 : le principal vient du sujet ; chaque cible est autorisée, filles comprises.
    #[test]
    fn un_invite_ne_commande_que_ses_pieces() {
        let b = &soiree().bilan;
        assert_eq!(issues(40), vec![(issue::REFUSE, raison::DROITS, false)], "la porte d'entrée n'est pas dans ses pièces");
        assert_eq!(issues(41), vec![(issue::REFUSE, raison::DROITS, false)], "le groupe atteint le salon et la chambre");
        assert_eq!(issues(42).first(), Some(&(issue::RETENUE, raison::GARDE, false)), "arrêter une machine : il le propose");
        assert_eq!(fin(42), issue::CONFIRME, "le propriétaire l'approuve");
        assert_eq!((b.qui_invite, b.pour_invite), (5, 5), "la demande porte l'invité, pas la personne qu'il prétend être");
    }

    /// I1 : un permis qui finit fait tomber la retenue ; l'approbation ne la ranime pas.
    #[test]
    fn une_retenue_ne_garde_pas_une_autorisation_echue() {
        let b = &soiree().bilan;
        assert_eq!(issues(43), vec![(issue::RETENUE, raison::GARDE, false), (issue::REFUSE, raison::DROITS, false)]);
        let quand = b.instants[&43][1];
        assert!(quand >= h(18, 40, 0) && quand < h(18, 40, 1), "à la fin du permis : {quand}");
        assert!(b.decision_gardien.as_ref().is_some_and(Result::is_err), "approuvée à 18 h 50 : trop tard");
        assert_eq!(envois_de(43), 0, "le pêne n'a jamais été tiré");
    }

    /// I2 : l'échéance de l'appelant, et celle de la retenue au moment même de la décision.
    #[test]
    fn une_echeance_se_verifie_a_chaque_frontiere() {
        assert_eq!(issues(44), vec![(issue::REFUSE, raison::ECHUE, false)], "arrivée après son échéance");
        assert_eq!(soiree().bilan.decision_trop_tard, Some(Err("retenue expirée")), "approuvée à l'instant de son échéance");
    }

    /// I3 : une approbation ne vaut que pour la forme qu'elle a montrée.
    #[test]
    fn une_carte_ne_s_approuve_que_sous_sa_forme() {
        assert_eq!(
            issues(45)[..2],
            [(issue::RETENUE, raison::GARDE, false), (issue::RETENUE, raison::FORME_CHANGEE, false)],
            "la forme a changé, 55 tient : une nouvelle carte"
        );
        assert_eq!(soiree().bilan.decision_carte_perimee, Some(Err("la carte montrée n'est plus la bonne")));
        assert_eq!(fin(45), issue::CONFIRME, "la nouvelle carte, approuvée, passe");
    }

    #[test]
    fn aucun_ordre_n_est_rejoue() {
        let b = &soiree().bilan;
        let mut vus = BTreeSet::new();
        for (machine, r) in &b.envois {
            assert!(vus.insert(*r), "la demande {r:?} est partie deux fois ({machine})");
        }
        assert_eq!(issues(17), vec![(issue::INCERTAIN, raison::MODULE, false), (issue::INCERTAIN, raison::MODULE, true)], "le module est mort avec l'ordre");
        assert_eq!(issues(25), vec![(issue::INCERTAIN, raison::BASCULE, false), (issue::CONFIRME, 0, true)], "en vol à la bascule : incertain, puis confirmé par le hub, qui connaît la forme");
        assert_eq!(issues(23), vec![(issue::INCERTAIN, raison::BASCULE, false), (issue::CONFIRME, 0, true)], "parti juste avant la bascule : incertain, puis confirmé à l'arrêt par le hub");
        let quand = b.instants[&23][1];
        assert!(quand >= h(21, 0, 30), "confirmé à l'arrêt, pas au passage par la cible en mouvement : {}", (quand - h(21, 0, 0)) / 1_000);
        assert_eq!(issues(22), vec![(issue::ACCEPTE, 0, false), (issue::CONFIRME, 0, true)], "la suite du chauffe-eau arrive au nouvel actif");
        assert_eq!(issues(29), vec![(issue::INCERTAIN, raison::ARRET, false), (issue::CONFIRME, 0, true)], "en vol à l'arrêt figé");
        assert_eq!(issues(28), vec![(issue::ACCEPTE, 0, false), (issue::CONFIRME, 0, true)], "suivi à travers le redémarrage");
        assert_eq!(b.refus_epoque, 0);
    }

    /// I4 : une lecture commencée avant la reprise ne confirme rien.
    #[test]
    fn une_lecture_d_avant_la_reprise_ne_confirme_pas() {
        assert_eq!(issues(24), vec![(issue::INCERTAIN, raison::BASCULE, false), (issue::INCERTAIN, raison::BASCULE, true)], "en file à la bascule : jamais parti, jamais renvoyé, et la vieille lecture de 60 ne le confirme pas");
    }

    /// B2 : un appui en attente de l'accusé ne part pas ; retenté après une reprise sûre, il part une fois.
    #[test]
    fn un_appui_n_est_ni_perdu_ni_double_par_la_bascule() {
        let b = &soiree().bilan;
        assert!(!b.temoin_decroche_a_la_panne, "l'actif est tombé dans les 200 ms : il n'avait pas inscrit le décrochage");
        assert_eq!(fin(49), issue::EXECUTE, "retenté sur le nouvel actif : il part");
        let refs = &b.toutes_refs[&49];
        assert_eq!(refs.len(), 2, "une demande chez l'ancien actif, une chez le nouveau");
        assert_eq!(b.envois.iter().filter(|(_, r)| refs.contains(r)).count(), 1, "partie une seule fois");
        assert!(!b.envois.iter().any(|(m, r)| m == "A" && refs.contains(r)), "jamais par l'ancien actif, qui attendait l'accusé");
    }

    #[test]
    fn la_derniere_commande_se_lit_partout_pareil() {
        let b = &soiree().bilan;
        for (moment, egales, total) in &b.fiches_ecran {
            assert_eq!(egales, total, "{moment}");
        }
        let (valeur, i, o, quand) = b.derniere_rts.expect("relevé");
        assert_eq!((valeur, i, o), (0, issue::EXECUTE, origine::ASSISTANT), "« Ouvrir : exécuté par la box, par l'assistant »");
        assert!(quand >= h(18, 20, 0) && quand < h(18, 21, 0));
    }

    /// I6 : une suite tardive ne remplace pas la dernière commande ; « en cours » se compte à part.
    #[test]
    fn la_derniere_commande_est_la_derniere_admise() {
        let b = &soiree().bilan;
        let f = b.fiche_chauffe_eau.expect("relevé");
        assert_eq!((f.demande, f.valeur, f.issue, f.actives), (b.refs[&46], 54, issue::ACCEPTE, 1), "la suite de 55 °C est arrivée ; la fiche reste sur 54 °C, encore suivie");
        assert_eq!(fin(46), issue::CONFIRME);
    }

    #[test]
    fn un_appui_retente_ne_part_qu_une_fois() {
        let s = soiree();
        assert_eq!(issues(14), vec![(issue::EXECUTE, 0, false); 3], "le même appui, la même issue, avant et après la bascule");
        let hub = &s.m[s.actif].hub;
        let n = hub.ordre("helpers:sonnette/appuyer");
        let appuis = s.m.iter().flat_map(|m| m.hub.journal.iter()).filter(|l| l.ordre == n.0 && l.action == 1 && l.issue == issue::EN_COURS).count();
        assert!(appuis <= 2, "un seul appui (la ligne de départ, vue par l'actif et sa copie) : {appuis}");
    }

    #[test]
    fn un_bail_se_retablit_apres_la_bascule_et_le_redemarrage() {
        let m = &soiree().bilan.moteur;
        let retablis: Vec<&(u64, String, String)> = m.iter().filter(|(_, o, x)| o == "hue:couloir/on" && x.starts_with("bail rétabli à 0")).collect();
        assert_eq!(retablis.len(), 3, "{m:?}");
        assert!(retablis.iter().any(|(t, _, _)| *t == h(21, 1, 30) / 1_000), "rétabli par le nouvel actif : {retablis:?}");
        // Le bail s'ouvre au succès de son ordre (revue v3, W3) : son terme suit, 2 s de retard.
        assert!(retablis.iter().any(|(t, _, x)| *t == h(22, 0, 5) / 1_000 && x.contains("(2 s de retard)")), "rétabli en retard, après le redémarrage : {retablis:?}");
    }

    /// I5 : « la valeur suit-elle la commande ? » dit oui, non, ou qu'on ne sait pas.
    #[test]
    fn les_workflows_interrogent_les_ordres() {
        let b = &soiree().bilan;
        let m = &b.moteur;
        let ext: Vec<&(u64, String, String)> = m.iter().filter(|(_, a, _)| a == "Extinction auto").collect();
        assert!(ext.iter().any(|(t, _, x)| *t == h(20, 0, 0) / 1_000 && x == "conditions non réunies"), "allumée par un humain : {ext:?}");
        assert!(ext.iter().any(|(t, _, x)| *t / 60 == h(20, 40, 0) / 60_000 && x.starts_with("issue confirme")), "allumée par la maison : {ext:?}");
        assert_eq!(
            b.vient_de,
            vec![
                ("20:12 après l'interrupteur".to_string(), resultat::NON),
                ("20:16 après la maison".to_string(), resultat::OUI),
                ("20:41 une valeur déjà là".to_string(), resultat::INCONNU),
                ("20:43 une variation avant la preuve".to_string(), resultat::INCONNU),
            ]
        );
        assert!(m.iter().any(|(_, a, x)| a == "Volets du soir" && x.starts_with("part : ordre overkiz:volet_salon/fermer")), "{m:?}");
        assert!(b.notices.iter().any(|n| n == "Alerte serrure : la porte n'a pas confirmé le verrouillage"), "{:?}", b.notices);
    }

    #[test]
    fn un_appareil_virtuel_compose_sans_posseder() {
        let b = &soiree().bilan;
        let sejour = &b.vues["18:00 lumieres:sejour"];
        assert_eq!(sejour.iter().filter(|x| x.starts_with("valeur:hue:sejour_")).count(), 4, "{sejour:?}");
        assert_eq!(sejour.iter().filter(|x| x.starts_with("ordre:lumieres:sejour/on")).count(), 1);
        assert_eq!(issues(7), vec![(issue::ECHEC, raison::PARTIEL, false)], "3 ampoules sur 4");
        assert_eq!(b.groupes[&7], (3, 4));
        assert_eq!(b.aussi_dans, vec!["virtuel:entree".to_string()]);
        let entree = &b.vues["18:00 virtuel:entree"];
        assert_eq!(entree.len(), 5, "{entree:?}");
    }

    /// I7 : un groupe approuvé passe par le même répartiteur ; ses filles ne sont pas retenues.
    #[test]
    fn un_groupe_approuve_se_repartit() {
        let b = &soiree().bilan;
        assert_eq!(issues(30), vec![(issue::RETENUE, raison::GARDE, false), (issue::ECHEC, raison::PARTIEL, false)]);
        assert_eq!(b.groupes[&30], (3, 4));
        let mere = b.refs[&30];
        let filles: BTreeSet<Ref> = b.parents.iter().filter(|(_, m)| **m == mere).map(|(f, _)| *f).collect();
        assert_eq!(filles.len(), 4, "une fille par cible");
        assert_eq!(b.envois.iter().filter(|(_, r)| filles.contains(r)).count(), 4, "chacune envoyée, aucune retenue de nouveau");
    }

    /// I8 : la progression d'un groupe passe la bascule.
    #[test]
    fn un_groupe_coupe_par_la_bascule_garde_son_compte() {
        let b = &soiree().bilan;
        assert_eq!(fin(47), issue::ECHEC);
        assert_eq!(b.groupes[&47], (3, 4), "trois ampoules éteintes avant la bascule, comptées par la copie");
    }

    #[test]
    fn un_appareil_retire_n_est_plus_commande_et_sa_place_ne_garde_rien() {
        let b = &soiree().bilan;
        assert_eq!(issues(31), vec![(issue::REFUSE, raison::RETIRE, false)]);
        let (_, revoir) = b.a_revoir.iter().find(|x| x.0 == "22:20").expect("relevé");
        let noms: BTreeSet<&str> = revoir.iter().map(|(a, _)| a.as_str()).collect();
        assert_eq!(noms, ["Nuit", "Volets du soir"].into_iter().collect());
        assert!(b.vues["22:20 virtuel:entree"].iter().any(|x| x.ends_with("(retiré)")));
        assert!(b.fiches_liberees_a_zero, "le lot remet à zéro les fiches des ordres libérés");
        assert_eq!(b.nouvelle_fiche.map(|f| (f.issue, f.valeur, f.instant_ms)), Some((0, 0, 0)), "le volet neuf ne montre rien de l'ancien");
        assert_eq!(b.par_numero_perime, Some((issue::REFUSE, raison::INCONNU)), "l'ancien numéro ne commande pas l'appareil neuf");
        let apres = &b.vues["lot virtuel:entree"];
        assert!(apres.iter().filter(|x| x.contains("(membre retiré)")).count() == 2 && !apres.iter().any(|x| x.contains("volet_salon_io")), "{apres:?}");
    }

    #[test]
    fn une_structure_qui_change_met_l_automatisme_a_revoir() {
        let b = &soiree().bilan;
        let (_, revoir) = b.a_revoir.iter().find(|x| x.0 == "redéclaration").expect("relevé");
        assert!(revoir.iter().any(|(a, r)| a == "Eau chaude max" && r.contains("bornes")), "{revoir:?}");
        assert_eq!(b.ecran2_demandes_meta, 1, "l'écran à vieux cache n'a demandé que la forme nouvelle du chauffe-eau");
        assert_eq!(b.ecran2_valide_62, Some(Err(raison::BORNES)), "et il refuse 62 °C avec elle, comme le hub");
    }

    /// I11 : une redéclaration garde les numéros par clé ; la preuve suit la clé.
    #[test]
    fn une_redeclaration_garde_les_numeros_par_cle() {
        let (avant, apres, retire) = soiree().bilan.redeclaration.clone().expect("relevé");
        assert_eq!(avant[..3], apres[..3], "on, brightness, color_temp gardent leur numéro");
        assert!(!avant.contains(&apres[3]), "energy est neuve");
        assert!(retire, "linkquality, qui n'est plus déclarée, est retirée");
        assert_eq!(fin(48), issue::CONFIRME, "la luminosité est prouvée par la luminosité, pas par la valeur qui a pris son rang");
    }

    #[test]
    fn la_copie_et_l_etat_complet_finissent_identiques() {
        let b = &soiree().bilan;
        for (moment, a, copie, c) in &b.empreintes {
            if moment != "avant la bascule" {
                assert_eq!(Some(*a), *c, "{moment} : la copie par état complet suit l'actif");
            } else {
                assert_eq!(a, copie, "{moment}");
            }
        }
        assert!(b.instantane_identique, "l'instantané relu est la mémoire d'avant l'arrêt");
        assert!(b.instantane_octets > 10_000);
    }

    /// B5 : un texte privé ne sort que par le canal privé, et n'y reste pas.
    #[test]
    fn un_texte_prive_ne_sort_que_par_le_canal_prive() {
        let b = &soiree().bilan;
        assert_eq!(b.secrets_hors_canal, 0, "ni sur ord/, ni sur cmd/, ni dans l'état d'un client, ni dans l'instantané");
        assert!(b.prives_lus.iter().any(|x| x == SECRET_ANNONCE) && b.prives_lus.iter().any(|x| x == SECRET_TELEGRAM), "le module de l'appareil l'a lu : {:?}", b.prives_lus);
        assert!(b.copie_tenait_le_prive, "le pair l'a reçu par le canal privé");
        assert_eq!(b.prives_restants, 0, "effacé partout à la conclusion");
        assert_eq!(fin(50), issue::EXECUTE);
        assert_eq!(fin(51), issue::EXECUTE);
    }

    /// I13 : un écran de pièce voit le message qu'il a perdu, et seulement celui-là.
    #[test]
    fn un_ecran_de_piece_voit_ses_trous() {
        let b = &soiree().bilan;
        assert_eq!(b.ecran_piece.0, 1, "un trou, celui du message perdu ; aucune autre pièce ne fait de faux trou");
        assert_eq!(b.ecran_piece.1, 4, "quatre chargements : le premier, après le trou, après la bascule, après le redémarrage");
        assert!(b.ecran_piece_identique, "rechargé, il lit les mêmes fiches que le hub");
    }

    #[test]
    fn le_journal_dit_pourquoi() {
        let b = &soiree().bilan;
        assert!(b.notices.iter().any(|n| n.contains("aucune suite dans la fenêtre de suivi")), "{:?}", b.notices);
        assert!(b.moteur.iter().any(|(_, a, x)| a == "Nuit" && x == "part : à la main"));
    }
}

#[cfg(test)]
mod montrer {
    #[test]
    #[ignore]
    fn montrer() {
        let s = super::jouer();
        let b = &s.bilan;
        let base = super::h(18, 0, 0) / 1_000;
        for (id, v) in &b.issues {
            println!(
                "demande {id}: {:?}",
                v.iter()
                    .map(|(i, r, su)| format!(
                        "{}{}{}",
                        crate::demandes::issue::NOMS[usize::from(*i)],
                        if *r != 0 { format!("({})", crate::catalogue::raison::NOMS[usize::from(*r)]) } else { String::new() },
                        if *su { " [suite]" } else { "" }
                    ))
                    .collect::<Vec<_>>()
            );
        }
        for n in &b.notices {
            println!("avis: {n}");
        }
        for (t, a, x) in &b.moteur {
            let d = t.saturating_sub(base);
            println!("moteur {:02}:{:02}:{:02} {a} — {x}", 18 + d / 3600, d / 60 % 60, d % 60);
        }
        for e in &b.empreintes {
            println!("empreinte {:?}", e);
        }
        println!("groupes {:?}", b.groupes);
        println!("fiches écran {:?}", b.fiches_ecran);
        println!("vues {:?}", b.vues);
        println!("à revoir {:?}", b.a_revoir);
        println!("catalogue {:?} rts {:?} formes demandées {} / {}", b.catalogue, b.ordres_rts, b.ecran_demandes_meta, b.ecran2_demandes_meta);
        println!("sorties {} ({} o) instantané {} o identique {}", b.sorties, b.octets_sorties, b.instantane_octets, b.instantane_identique);
        println!("envois {} validations {:?} vient_de {:?}", b.envois.len(), b.validations_identiques, b.vient_de);
        println!("écran de pièce {:?} identique {}", b.ecran_piece, b.ecran_piece_identique);
        println!("fiche chauffe-eau {:?}", b.fiche_chauffe_eau);
        println!("décisions : trop tard {:?}, carte périmée {:?}, gardien {:?}", b.decision_trop_tard, b.decision_carte_perimee, b.decision_gardien);
    }
}
