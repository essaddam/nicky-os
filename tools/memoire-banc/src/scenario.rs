//! Le scénario de bout en bout : une soirée de la maison, de 18 h à 23 h 15, rejouée contre le
//! moteur de référence (moteur.rs), seconde par seconde là où il se passe quelque chose.
//!
//! Huit automatismes par copie de la maison :
//! - A « Coucher de la maison » (23 nœuds, deux entrées) : quand fenêtres et porte sont fermées la
//!   nuit depuis 10 min, ou au bouton « bonne nuit » ; un « si » de quatre règles (dont la plage
//!   horaire et le mode de la maison) ; la température lue dans le registre ; attendre 5 min ; un
//!   « si » relu après l'attente ; fermer le volet et vérifier sa nouvelle valeur ; armer l'alarme
//!   et vérifier ; passer la maison en mode nuit ; attendre 30 min ; une consigne de chauffage
//!   jamais confirmée ; une fin qui dépend de la cause ;
//! - B « Couloir » (mode relance) : veilleuse la nuit, lumière sinon, pendant 2 min ;
//! - C « Porte ouverte » : de fermée à ouverte depuis 5 min, alerte si la maison est en nuit ;
//! - D « Extinction auto » : une lumière allumée depuis 10 min s'éteint, par sa cause ;
//! - E « Alerte » : l'alarme passée de 2 (armée nuit) à 3 (en alerte) depuis 1 min. Elle revient à 3
//!   depuis 1 : sans nouvelle transition attendue, seule la revérification par l'instant « changé »
//!   empêche l'attente de partir ;
//! - F « Ronde de 22 h 30 » : un déclencheur horaire, à la seconde ;
//! - G « Tâches de nuit » : deux heures qui tombent pendant la panne ; au redémarrage, celle qui a
//!   plus de 2 min de retard est manquée, l'autre est rattrapée ;
//! - H « Rendez-vous » : 10 min avant chaque événement de l'agenda ; un rappel tombe pendant la
//!   panne, un rendez-vous est ajouté au dernier moment, deux ont lieu à la même heure.
//!
//! v2 : deux horloges (la monotone repart de zéro au redémarrage) ; la nuit et la plage 21 h–2 h sont
//! des niveaux tenus par le calendrier du hub ; le thermomètre se tait à 22:40 (valeur trop vieille).
//!
//! Pendant la soirée : un message qui ferait basculer la condition en deux temps, une issue qui
//! arrive après la relance, un bouton pressé pendant l'exécution, une panne de 5 min et le
//! redémarrage, une modification de B pendant qu'il tourne, un capteur qui devient muet puis
//! revient. Le scénario peut être joué en N copies de la maison à la fois.

use std::collections::BTreeMap;
use std::time::Instant;

use serde_json::{Value as Json, json};

use crate::calendrier::{self, Calendrier, Fiche as FicheCal, Sortie};
use crate::maison::{Appareil, Classe, Maison, Nature, Point};
use crate::moteur::{Instantane, Lieu, Moteur, NUIT, compiler, etat_ex, heure, plage_nom, plages_requises};
use crate::outils::Temps;
use crate::valeurs::Etat;
use crate::workflows::Automation;

/// 23 octobre 2026, 18:00:00 (le scénario lit l'heure en UTC).
pub const DEBUT: u32 = 1_792_778_400;
pub const FIN: u32 = DEBUT + 5 * 3_600 + 15 * 60;

pub const fn hms(h: u32, m: u32, s: u32) -> u32 {
    DEBUT + (h - 18) * 3_600 + m * 60 + s
}

/// Les points d'une copie : clé, nom (`{}` : la copie), classe, nature, échelle, valeur à 18 h.
pub const CLES: [(&str, &str, Classe, Nature, i8, i64); 16] = [
    ("fen_cuisine", "z2m:fenetre_cuisine_{}/contact", Classe::Zigbee, Nature::Binaire, 0, 1),
    ("fen_salon", "z2m:fenetre_salon_{}/contact", Classe::Zigbee, Nature::Binaire, 0, 1),
    ("fen_chambre", "z2m:fenetre_chambre_{}/contact", Classe::Zigbee, Nature::Binaire, 0, 0),
    ("fen_bureau", "z2m:fenetre_bureau_{}/contact", Classe::Zigbee, Nature::Binaire, 0, 0),
    ("porte", "z2m:porte_entree_{}/contact", Classe::Zigbee, Nature::Binaire, 0, 1),
    ("mvt_salon", "z2m:mouvement_salon_{}/occupancy", Classe::Zigbee, Nature::Binaire, 0, 0),
    ("mvt_couloir", "z2m:mouvement_couloir_{}/occupancy", Classe::Zigbee, Nature::Binaire, 0, 0),
    ("bouton", "z2m:bouton_chambre_{}/action", Classe::Action, Nature::Binaire, 0, 0),
    ("temp", "z2m:thermometre_salon_{}/temperature", Classe::Zigbee, Nature::Mesure, -2, 2130),
    ("volet", "overkiz:volet_salon_{}/core:ClosureState", Classe::Api, Nature::Mesure, 0, 100),
    ("lum_salon", "hue:lumiere_salon_{}/on", Classe::Action, Nature::Binaire, 0, 0),
    ("lum_couloir", "hue:lumiere_couloir_{}/on", Classe::Action, Nature::Binaire, 0, 0),
    ("veilleuse", "hue:veilleuse_couloir_{}/on", Classe::Action, Nature::Binaire, 0, 0),
    ("alarme", "alarme:maison_{}/mode", Classe::Action, Nature::Enum, 0, 0),
    ("consigne", "voltalis:radiateur_salon_{}/consigne", Classe::Api, Nature::Mesure, -2, 1900),
    ("mode", "hub:mode_maison_{}/mode", Classe::Action, Nature::Enum, 0, 0),
];

fn k(cle: &str) -> usize {
    CLES.iter().position(|c| c.0 == cle).expect("clé connue")
}

fn nom(cle: &str, i: u32) -> String {
    CLES[k(cle)].1.replace("{}", &i.to_string())
}

fn noeuds(n: &[Json]) -> Json {
    Json::Array(n.to_vec())
}

fn automatisme(id: String, name: String, mode: &str, nodes: Json, edges: Json) -> Automation {
    serde_json::from_value(json!({
        "id": id, "name": name, "enabled": true, "mode": mode, "author": "human",
        "graph": { "nodes": nodes, "edges": edges }
    }))
    .expect("automatisme au format du modèle")
}

/// B (ou B modifié), A, C, D, E, F, G pour chaque copie, B avant A : modifier B déplace les nœuds de A.
/// Automatismes par copie de la maison.
pub const PAR_COPIE: u32 = 8;

pub fn automatismes(copies: u32, b_modifie: bool) -> Vec<Automation> {
    let mut v = Vec::new();
    for i in 0..copies {
        let n = |c: &str| nom(c, i);
        // ---------------------------------------------------------------- B, le couloir
        let mut b_noeuds = vec![
            json!({"id": "mouvement", "type": "when_state", "point": n("mvt_couloir"), "to": true}),
            json!({"id": "la_nuit", "type": "if", "all": true, "rules": [{"kind": "state", "point": n("mode"), "op": "eq", "value": 1}]}),
            json!({"id": "veilleuse_on", "type": "set", "point": n("veilleuse"), "value": true}),
            json!({"id": "attendre_veilleuse", "type": "wait", "seconds": 120}),
            json!({"id": "veilleuse_off", "type": "set", "point": n("veilleuse"), "value": false}),
            json!({"id": "couloir_on", "type": "set", "point": n("lum_couloir"), "value": true}),
            json!({"id": "attendre_couloir", "type": "wait", "seconds": 120}),
            json!({"id": "couloir_off", "type": "set", "point": n("lum_couloir"), "value": false}),
        ];
        let mut b_liens = vec![
            json!({"from": "mouvement", "to": "la_nuit"}),
            json!({"from": "la_nuit", "port": "yes", "to": "veilleuse_on"}),
            json!({"from": "la_nuit", "port": "no", "to": "couloir_on"}),
            json!({"from": "veilleuse_on", "to": "attendre_veilleuse"}),
            json!({"from": "attendre_veilleuse", "to": "veilleuse_off"}),
            json!({"from": "couloir_on", "to": "attendre_couloir"}),
            json!({"from": "attendre_couloir", "to": "couloir_off"}),
        ];
        if b_modifie {
            b_noeuds.push(json!({"id": "prevenir", "type": "notify", "message": "Couloir éteint."}));
            b_liens.push(json!({"from": "veilleuse_off", "to": "prevenir"}));
        }
        v.push(automatisme(format!("couloir-{i}"), format!("Couloir {i}"), "restart", noeuds(&b_noeuds), Json::Array(b_liens)));
        // ---------------------------------------------------------------- A, le coucher
        let a_noeuds = [
            json!({"id": "tout_ferme", "type": "when_rules", "all": true, "for_s": 600, "rules": [
                {"kind": "state", "point": n("fen_cuisine"), "op": "eq", "value": true},
                {"kind": "state", "point": n("fen_salon"), "op": "eq", "value": true},
                {"kind": "state", "point": n("fen_chambre"), "op": "eq", "value": true},
                {"kind": "state", "point": n("fen_bureau"), "op": "eq", "value": true},
                {"kind": "state", "point": n("porte"), "op": "eq", "value": true},
                {"kind": "sun", "is": "night"}]}),
            json!({"id": "bouton", "type": "when_state", "point": n("bouton"), "to": true}),
            json!({"id": "verifier", "type": "if", "all": true, "rules": [
                {"kind": "state", "point": n("mvt_salon"), "op": "eq", "value": false},
                {"kind": "state", "point": n("alarme"), "op": "eq", "value": 0},
                {"kind": "time", "after": "21:00", "before": "02:00"},
                {"kind": "state", "point": n("mode"), "op": "ne", "value": 1}]}),
            json!({"id": "lire_temperature", "type": "read_value", "point": n("temp")}),
            json!({"id": "eteindre_salon", "type": "set", "point": n("lum_salon"), "value": false}),
            json!({"id": "attendre_5", "type": "wait", "seconds": 300}),
            json!({"id": "encore_personne", "type": "if", "all": true, "rules": [
                {"kind": "state", "point": n("mvt_salon"), "op": "eq", "value": false}]}),
            json!({"id": "fermer_volet", "type": "set", "point": n("volet"), "value": 0}),
            json!({"id": "verifier_volet", "type": "wait_for", "timeout_s": 120,
                   "rule": {"kind": "state", "point": n("volet"), "op": "eq", "value": 0}}),
            json!({"id": "volet_bloque", "type": "notify", "message": "Volet du salon bloqué."}),
            json!({"id": "armer", "type": "set", "point": n("alarme"), "value": 2}),
            json!({"id": "verifier_alarme", "type": "wait_for", "timeout_s": 30,
                   "rule": {"kind": "state", "point": n("alarme"), "op": "eq", "value": 2}}),
            json!({"id": "alarme_ko", "type": "notify", "message": "L'alarme n'a pas confirmé l'armement : coucher arrêté."}),
            json!({"id": "mode_nuit", "type": "set", "point": n("mode"), "value": 1}),
            json!({"id": "attendre_30", "type": "wait", "seconds": 1800}),
            json!({"id": "chauffage", "type": "set", "point": n("consigne"), "value": 17.0}),
            json!({"id": "verifier_chauffage", "type": "wait_for", "timeout_s": 60,
                   "rule": {"kind": "state", "point": n("consigne"), "op": "eq", "value": 17.0}}),
            json!({"id": "chauffage_ko", "type": "notify", "message": "Chauffage : consigne non confirmée (température au départ : {{lu}})."}),
            json!({"id": "selon_cause", "type": "if", "all": true, "rules": [{"kind": "cause", "point": n("fen_bureau")}]}),
            json!({"id": "fin_fenetre", "type": "notify", "message": "Maison couchée ; dernière fenêtre fermée : {{declencheur}} ; température au départ : {{lu}}."}),
            json!({"id": "fin", "type": "notify", "message": "Maison couchée (départ : {{declencheur}})."}),
            json!({"id": "presence", "type": "notify", "message": "Quelqu'un est au salon : coucher annulé."}),
            json!({"id": "non_reunies", "type": "notify", "message": "Coucher : conditions non réunies ({{declencheur}})."}),
        ];
        let a_liens = json!([
            {"from": "tout_ferme", "to": "verifier"},
            {"from": "bouton", "to": "verifier"},
            {"from": "verifier", "port": "yes", "to": "lire_temperature"},
            {"from": "verifier", "port": "no", "to": "non_reunies"},
            {"from": "lire_temperature", "to": "eteindre_salon"},
            {"from": "eteindre_salon", "to": "attendre_5"},
            {"from": "attendre_5", "to": "encore_personne"},
            {"from": "encore_personne", "port": "yes", "to": "fermer_volet"},
            {"from": "encore_personne", "port": "no", "to": "presence"},
            {"from": "fermer_volet", "to": "verifier_volet"},
            {"from": "verifier_volet", "port": "ok", "to": "armer"},
            {"from": "verifier_volet", "port": "timeout", "to": "volet_bloque"},
            {"from": "volet_bloque", "to": "armer"},
            {"from": "armer", "to": "verifier_alarme"},
            {"from": "verifier_alarme", "port": "ok", "to": "mode_nuit"},
            {"from": "verifier_alarme", "port": "timeout", "to": "alarme_ko"},
            {"from": "mode_nuit", "to": "attendre_30"},
            {"from": "attendre_30", "to": "chauffage"},
            {"from": "chauffage", "to": "verifier_chauffage"},
            {"from": "verifier_chauffage", "port": "ok", "to": "selon_cause"},
            {"from": "verifier_chauffage", "port": "timeout", "to": "chauffage_ko"},
            {"from": "chauffage_ko", "to": "selon_cause"},
            {"from": "selon_cause", "port": "yes", "to": "fin_fenetre"},
            {"from": "selon_cause", "port": "no", "to": "fin"}
        ]);
        v.push(automatisme(format!("coucher-{i}"), format!("Coucher {i}"), "single", noeuds(&a_noeuds), a_liens));
        // ---------------------------------------------------------------- C, la porte
        let c_noeuds = [
            json!({"id": "porte_ouverte", "type": "when_state", "point": n("porte"), "from": true, "to": false, "for_s": 300}),
            json!({"id": "la_nuit", "type": "if", "all": true, "rules": [{"kind": "state", "point": n("mode"), "op": "eq", "value": 1}]}),
            json!({"id": "alerte_nuit", "type": "notify", "message": "Porte d'entrée ouverte depuis 5 min, la nuit ! ({{declencheur}})"}),
            json!({"id": "rappel", "type": "notify", "message": "Porte d'entrée ouverte depuis 5 min ({{declencheur}})."}),
        ];
        let c_liens = json!([
            {"from": "porte_ouverte", "to": "la_nuit"},
            {"from": "la_nuit", "port": "yes", "to": "alerte_nuit"},
            {"from": "la_nuit", "port": "no", "to": "rappel"}
        ]);
        v.push(automatisme(format!("porte-{i}"), format!("Porte {i}"), "single", noeuds(&c_noeuds), c_liens));
        // ---------------------------------------------------------------- D, l'extinction
        let d_noeuds = [
            json!({"id": "allumee", "type": "when_state", "point": n("lum_salon"), "to": true, "for_s": 600}),
            json!({"id": "eteindre", "type": "set", "point": "@declencheur", "value": false}),
        ];
        v.push(automatisme(format!("extinction-{i}"), format!("Extinction {i}"), "restart", noeuds(&d_noeuds), json!([{"from": "allumee", "to": "eteindre"}])));
        // ---------------------------------------------------------------- E, l'alerte
        let e_noeuds = [
            json!({"id": "alerte", "type": "when_state", "point": n("alarme"), "from": 2, "to": 3, "for_s": 60}),
            json!({"id": "prevenir", "type": "notify", "message": "Alarme en alerte depuis 1 min ({{declencheur}})."}),
        ];
        v.push(automatisme(format!("alerte-{i}"), format!("Alerte {i}"), "single", noeuds(&e_noeuds), json!([{"from": "alerte", "to": "prevenir"}])));
        // ---------------------------------------------------------------- F, la ronde
        let f_noeuds = [
            json!({"id": "ronde", "type": "at_time", "at": "22:30:00"}),
            json!({"id": "porte_ouverte", "type": "if", "all": true, "rules": [{"kind": "state", "point": n("porte"), "op": "eq", "value": false}]}),
            json!({"id": "alerte", "type": "notify", "message": "Ronde de 22 h 30 : la porte d'entrée est ouverte."}),
            json!({"id": "calme", "type": "notify", "message": "Ronde de 22 h 30 : tout est fermé."}),
        ];
        let f_liens = json!([
            {"from": "ronde", "to": "porte_ouverte"},
            {"from": "porte_ouverte", "port": "yes", "to": "alerte"},
            {"from": "porte_ouverte", "port": "no", "to": "calme"}
        ]);
        v.push(automatisme(format!("ronde-{i}"), format!("Ronde {i}"), "single", noeuds(&f_noeuds), f_liens));
        // ---------------------------------------------------------------- G, les tâches de nuit
        let g_noeuds = [
            json!({"id": "tache_1", "type": "at_time", "at": "22:21:30"}),
            json!({"id": "tache_2", "type": "at_time", "at": "22:24:00"}),
            json!({"id": "prevenir", "type": "notify", "message": "Tâche de nuit ({{declencheur}})."}),
        ];
        let g_liens = json!([{"from": "tache_1", "to": "prevenir"}, {"from": "tache_2", "to": "prevenir"}]);
        v.push(automatisme(format!("taches-{i}"), format!("Tâches {i}"), "single", noeuds(&g_noeuds), g_liens));
        // ---------------------------------------------------------------- H, les rendez-vous
        let h_noeuds = [
            json!({"id": "rappel", "type": "when_calendar", "agenda": i + 1, "offset_min": -10}),
            json!({"id": "prevenir", "type": "notify", "message": "Rendez-vous dans 10 min ({{declencheur}})."}),
        ];
        v.push(automatisme(format!("rendez-vous-{i}"), format!("Rendez-vous {i}"), "single", noeuds(&h_noeuds), json!([{"from": "rappel", "to": "prevenir"}])));
    }
    v
}

/// Ce qu'on relève d'une exécution à un instant donné.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fiche {
    pub etat: u8,
    pub noeud: String,
    pub registre: i64,
    pub cause: String,
    pub generation: u16,
}

#[derive(Clone, Debug)]
pub struct Releve {
    pub t: u32,
    pub nom: &'static str,
    pub a: Option<Fiche>,
    pub b: Option<Fiche>,
    pub a_condition_vraie: bool,
    pub a_premier: u32,
    /// Le thermomètre : la qualité de sa valeur, et son appareil muet ou non.
    pub temp_qualite: u8,
    pub temp_muet: bool,
}

enum Ev {
    /// Le module d'agenda a lu l'agenda d'une copie (début, fin, numéro de chaque événement).
    Agenda(u32, Vec<(u32, u32, u32)>),
    Valeurs(Vec<(u32, i64)>),
    Rapport(u32),
    Issue { case: u32, generation: u16, demande: u32 },
    Panne,
    Reprise,
    Modifier,
    Retour(u32),
    Relever(&'static str),
}

pub struct Scenario {
    pub copies: u32,
    pub etat: Etat,
    pub moteur: Moteur,
    pub defs: Vec<Automation>,
    /// Par copie, le numéro de chaque point (dans l'ordre de `CLES`).
    pub pts: Vec<Vec<u32>>,
    /// La maison réelle : ce que disent les appareils, même quand le hub n'écoute pas.
    pub vrai: Vec<i64>,
    /// Le calendrier du hub : les niveaux (la nuit, les plages) qu'il tient en points virtuels.
    pub hub: Calendrier,
    pub lieu: Lieu,
    /// L'horloge monotone : secondes depuis le démarrage ; elle repart de zéro au redémarrage.
    boot_mur: u32,
    boot_mono: u32,
    silences: Vec<(u32, u32, u32)>,
    file: BTreeMap<(u32, u64), Ev>,
    seq: u64,
    en_panne: bool,
    sauvegarde: Option<(Etat, Instantane)>,
    ordres_vus: usize,
    pub releves: Vec<Releve>,
    pub cpu_ns: u128,
    pub messages: u64,
    pub passages: u64,
}

const SILENCE: (u32, u32) = (hms(22, 25, 0), hms(23, 5, 0));
/// Le thermomètre se tait à 22:40 : sa valeur devient trop vieille (fraîcheur exigée : 15 min).
const SILENCE_THERMO: u32 = hms(22, 40, 0);
const FRAICHEUR_THERMO: u32 = 900;

impl Scenario {
    pub fn new(copies: u32) -> Self {
        let defs = automatismes(copies, false);
        // La maison : 16 appareils par copie, plus l'horloge du hub et ses points virtuels.
        let mut m = Maison { appareils: Vec::new(), points: Vec::new(), lus_mesures: Vec::new(), lus_binaires: Vec::new() };
        let mut pts = Vec::new();
        for i in 0..copies {
            let mut p = Vec::new();
            for c in CLES {
                let nom_point = c.1.replace("{}", &i.to_string());
                let appareil = nom_point.split('/').next().unwrap().to_string();
                let d = m.appareils.len() as u32;
                p.push(m.points.len() as u32);
                m.appareils.push(Appareil { nom: appareil, classe: c.2, premier: m.points.len() as u32, nb: 1 });
                m.points.push(Point { nom: nom_point, appareil: d, nature: c.3 });
            }
            pts.push(p);
        }
        let horloge = m.appareils.len() as u32;
        let mut virtuels = vec![NUIT.to_string()];
        virtuels.extend(plages_requises(&defs).iter().map(|&(a, b, j)| plage_nom(a, b, j)));
        m.appareils.push(Appareil { nom: "hub:horloge".into(), classe: Classe::Action, premier: m.points.len() as u32, nb: virtuels.len() as u32 });
        for v in &virtuels {
            m.points.push(Point { nom: v.clone(), appareil: horloge, nature: Nature::Binaire });
        }
        let mut etat = Etat::depuis(&m);
        let mut vrai = vec![0i64; etat.vifs.len()];
        for p in &pts {
            for (kk, &n) in p.iter().enumerate() {
                etat.defs[n as usize].echelle = CLES[kk].4;
                vrai[n as usize] = CLES[kk].5;
            }
        }
        let ms = u64::from(DEBUT) * 1_000;
        let boot_mono = 1_000;
        for n in 0..etat.vifs.len() {
            let v = &mut etat.vifs[n];
            v.valeur = vrai[n];
            v.precedente = vrai[n];
            v.change_ms = ms;
            v.observe_ms = ms;
            v.recu_s = DEBUT;
            etat.fraicheur[n] = 0;
            etat.perime[n] = u32::MAX;
        }
        for p in &pts {
            let n = p[k("temp")] as usize;
            etat.fraicheur[n] = FRAICHEUR_THERMO;
            etat.perime[n] = boot_mono + FRAICHEUR_THERMO;
        }
        for d in 0..etat.sante.len() {
            etat.sante[d].contact_s = DEBUT;
            if etat.delai[d] != 0 {
                etat.echeance[d] = boot_mono + etat.delai[d];
            }
        }
        // Le calendrier du hub : la nuit du scénario est une plage fixe (18:45 → 08:05, en UTC), les
        // plages des règles sont des niveaux ; leurs valeurs de 18 h sont posées avant de démarrer.
        let lieu = Lieu { fuseau: "UTC".into(), lat: 48.85, lon: 2.35 };
        let mut hub = Calendrier::new("UTC", lieu.lat, lieu.lon);
        let num = |v: &str| etat.annuaire.numero(v).unwrap();
        hub.ajouter(FicheCal { genre: calendrier::genre::PLAGE, a: 18 * 3_600 + 45 * 60, b: 8 * 3_600 + 5 * 60, point: num(NUIT), ..FicheCal::default() });
        for &(a, b, j) in &plages_requises(&defs) {
            hub.ajouter(FicheCal { genre: calendrier::genre::PLAGE, jours: j, a: a * 60, b: b * 60, point: num(&plage_nom(a, b, j)), ..FicheCal::default() });
        }
        for s in hub.armer_tout(ms, &|_| None) {
            if let Sortie::Niveau { point, valeur, .. } = s {
                etat.vifs[point as usize].valeur = valeur;
                etat.vifs[point as usize].precedente = valeur;
                vrai[point as usize] = valeur;
            }
        }
        let mut moteur = Moteur::new(compiler(&defs, &etat), &etat, &lieu);
        moteur.armer(&etat, Temps { mono: boot_mono, mur_ms: ms });
        let silences = pts
            .iter()
            .flat_map(|p| [(p[k("fen_cuisine")], SILENCE.0, SILENCE.1), (p[k("temp")], SILENCE_THERMO, u32::MAX)])
            .collect();
        let mut s = Scenario {
            copies, etat, moteur, defs, pts, vrai, hub, lieu, boot_mur: DEBUT, boot_mono, silences, file: BTreeMap::new(),
            seq: 0, en_panne: false, sauvegarde: None, ordres_vus: 0, releves: Vec::new(), cpu_ns: 0, messages: 0, passages: 0,
        };
        s.programmer();
        let _ = virtuels;
        s
    }

    /// L'heure du scénario (murale, s) avec l'horloge monotone du démarrage en cours.
    pub fn temps(&self, t: u32) -> Temps {
        Temps { mono: self.boot_mono + t.saturating_sub(self.boot_mur), mur_ms: u64::from(t) * 1_000 }
    }

    fn mur_de(&self, mono: u32) -> u32 {
        if mono == u32::MAX { u32::MAX } else { self.boot_mur + mono.saturating_sub(self.boot_mono) }
    }

    fn planifier(&mut self, t: u32, ev: Ev) {
        self.seq += 1;
        self.file.insert((t, self.seq), ev);
    }

    fn p(&self, i: u32, cle: &str) -> u32 {
        self.pts[i as usize][k(cle)]
    }

    fn programmer(&mut self) {
        // Les appareils qui émettent seuls parlent toutes les 5 min, même sans rien de neuf.
        for i in 0..self.copies {
            for (kk, c) in CLES.iter().enumerate() {
                if c.2 == Classe::Action {
                    continue;
                }
                let n = self.pts[i as usize][kk];
                let mut t = DEBUT + 150;
                while t <= FIN {
                    self.planifier(t, Ev::Rapport(n));
                    t += 300;
                }
            }
        }
        // Le scénario, identique pour chaque copie.
        let script: Vec<(u32, Vec<(&str, i64)>)> = vec![
            (hms(19, 0, 0), vec![("lum_salon", 1)]),
            (hms(19, 30, 0), vec![("lum_salon", 1)]),
            (hms(19, 35, 0), vec![("lum_salon", 0)]),
            (hms(19, 45, 0), vec![("lum_salon", 1)]),
            (hms(20, 30, 0), vec![("bouton", 1)]),
            (hms(20, 30, 5), vec![("bouton", 0)]),
            (hms(21, 0, 0), vec![("mvt_couloir", 1)]),
            (hms(21, 0, 30), vec![("mvt_couloir", 0)]),
            (hms(21, 1, 30), vec![("mvt_couloir", 1)]),
            (hms(21, 5, 0), vec![("mvt_couloir", 0)]),
            (hms(21, 40, 0), vec![("fen_chambre", 1)]),
            // Un seul message : une fenêtre se ferme, une autre s'ouvre. Rien ne doit s'armer.
            (hms(21, 42, 0), vec![("fen_bureau", 1), ("fen_salon", 0)]),
            (hms(21, 43, 0), vec![("fen_bureau", 0)]),
            (hms(21, 44, 0), vec![("fen_salon", 1)]),
            (hms(21, 45, 0), vec![("fen_bureau", 1)]),
            (hms(21, 50, 0), vec![("fen_bureau", 0)]),
            (hms(21, 51, 0), vec![("fen_bureau", 1)]),
            (hms(22, 2, 0), vec![("temp", 2090)]),
            (hms(22, 3, 0), vec![("mvt_salon", 1)]),
            (hms(22, 4, 0), vec![("mvt_salon", 0)]),
            (hms(22, 10, 0), vec![("bouton", 1)]),
            (hms(22, 10, 5), vec![("bouton", 0)]),
            (hms(22, 17, 0), vec![("porte", 0)]),
            (hms(22, 19, 0), vec![("mvt_couloir", 1)]),
            (hms(22, 20, 30), vec![("mvt_couloir", 0)]),
            (hms(22, 30, 0), vec![("mvt_couloir", 1)]),
            (hms(22, 40, 0), vec![("mvt_couloir", 0)]),
            // L'alarme passe en alerte (de 2), est acquittée (1), repasse en alerte (de 1, pas de 2) :
            // l'attente de E, armée à 22:44:00, ne doit pas partir à 22:45:00.
            (hms(22, 44, 0), vec![("alarme", 3)]),
            (hms(22, 44, 20), vec![("alarme", 1)]),
            (hms(22, 44, 40), vec![("alarme", 3)]),
            (hms(22, 47, 0), vec![("alarme", 2)]),
            (hms(22, 50, 0), vec![("porte", 1)]),
            (hms(23, 8, 0), vec![("mvt_couloir", 1)]),
            (hms(23, 11, 0), vec![("alarme", 3)]),
            (hms(23, 12, 0), vec![("mvt_couloir", 0)]),
            (hms(23, 13, 0), vec![("alarme", 2)]),
        ];
        let mut evs: Vec<(u32, Ev)> = Vec::new();
        for (t, valeurs) in &script {
            for i in 0..self.copies {
                let v = valeurs.iter().map(|&(c, x)| (self.p(i, c), x)).collect();
                evs.push((*t, Ev::Valeurs(v)));
            }
        }
        // L'agenda de chaque copie : lu à 18 h, relu à 22:56 avec trois rendez-vous de plus (l'un à
        // 23:00, ajouté au dernier moment ; deux à 23:14, à la même heure).
        let premier = vec![(hms(21, 30, 0), hms(22, 0, 0), 1), (hms(22, 31, 0), hms(22, 45, 0), 2)];
        let mut second = premier.clone();
        second.extend([(hms(23, 0, 0), hms(23, 30, 0), 3), (hms(23, 14, 0), hms(23, 20, 0), 4), (hms(23, 14, 0), hms(23, 45, 0), 5)]);
        for i in 0..self.copies {
            evs.push((DEBUT + 1, Ev::Agenda(i + 1, premier.clone())));
            evs.push((hms(22, 56, 0), Ev::Agenda(i + 1, second.clone())));
        }
        evs.push((hms(22, 3, 30), Ev::Relever("après le mouvement au salon, pendant l'attente de 5 min")));
        evs.push((hms(22, 19, 30), Ev::Relever("juste avant la panne")));
        evs.push((hms(22, 20, 0), Ev::Panne));
        evs.push((hms(22, 25, 0), Ev::Reprise));
        evs.push((hms(22, 25, 0), Ev::Relever("juste après le redémarrage")));
        evs.push((hms(22, 31, 0), Ev::Modifier));
        evs.push((hms(22, 31, 0), Ev::Relever("juste après la modification de B")));
        evs.push((hms(23, 0, 0), Ev::Relever("le thermomètre se tait depuis 20 min")));
        for i in 0..self.copies {
            evs.push((SILENCE.1, Ev::Retour(self.p(i, "fen_cuisine"))));
        }
        evs.sort_by_key(|e| e.0);
        for (t, ev) in evs {
            self.planifier(t, ev);
        }
    }

    fn silencieux(&self, n: u32, t: u32) -> bool {
        self.silences.iter().any(|&(p, de, a)| p == n && t >= de && t < a)
    }

    fn mesurer<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let d = Instant::now();
        let r = f(self);
        self.cpu_ns += d.elapsed().as_nanos();
        r
    }

    fn message(&mut self, t: u32, v: &[(u32, i64)]) {
        let tt = self.temps(t);
        self.mesurer(|s| s.moteur.message(&mut s.etat, tt, v));
        self.messages += 1;
        self.suivre_ordres();
    }

    /// Les ordres émis : leur issue revient une seconde plus tard (sauf le pont lent du premier
    /// ordre du couloir), et l'appareil confirme ensuite sa nouvelle valeur (le volet passe par 50 ;
    /// le radiateur ne confirme jamais).
    fn suivre_ordres(&mut self) {
        while self.ordres_vus < self.moteur.ordres.len() {
            let o = self.moteur.ordres[self.ordres_vus].clone();
            self.ordres_vus += 1;
            let cle = CLES[self.pts.iter().find_map(|p| p.iter().position(|&x| x == o.point)).unwrap()].0;
            let lent = cle == "lum_couloir" && o.t == hms(21, 0, 0);
            let t_issue = o.t + if lent { 100 } else { 1 };
            self.planifier(t_issue, Ev::Issue { case: o.case, generation: o.generation, demande: o.demande });
            match cle {
                "volet" => {
                    self.planifier(o.t + 19, Ev::Valeurs(vec![(o.point, 50)]));
                    self.planifier(o.t + 44, Ev::Valeurs(vec![(o.point, o.valeur)]));
                }
                "consigne" => {}
                _ => self.planifier(t_issue + 1, Ev::Valeurs(vec![(o.point, o.valeur)])),
            }
        }
    }

    fn traiter(&mut self, t: u32, ev: Ev) {
        match ev {
            Ev::Agenda(agenda, evts) => {
                if !self.en_panne {
                    let evts = evts
                        .into_iter()
                        .map(|(d, f, numero)| calendrier::Evenement { debut_ms: u64::from(d) * 1_000, fin_ms: u64::from(f) * 1_000, agenda, numero })
                        .collect();
                    let tt = self.temps(t);
                    self.mesurer(|s| s.moteur.agenda(&s.etat, tt, agenda, evts));
                    self.suivre_ordres();
                }
            }
            Ev::Valeurs(v) => {
                for &(n, x) in &v {
                    self.vrai[n as usize] = x;
                }
                if !self.en_panne {
                    self.message(t, &v);
                }
            }
            Ev::Rapport(n) => {
                if !self.en_panne && !self.silencieux(n, t) {
                    let v = [(n, self.vrai[n as usize])];
                    self.message(t, &v);
                }
            }
            Ev::Issue { case, generation, demande } => {
                if !self.en_panne {
                    let tt = self.temps(t);
                    self.mesurer(|s| s.moteur.issue(&s.etat, tt, case, generation, demande));
                    self.suivre_ordres();
                }
            }
            Ev::Panne => {
                self.sauvegarde = Some((self.etat.clone(), self.moteur.instantane(self.temps(t))));
                self.en_panne = true;
                self.moteur.journal.push((t, "panne : le hub s'arrête (instantané rangé)".into()));
            }
            Ev::Reprise => {
                let (etat, inst) = self.sauvegarde.take().expect("instantané");
                // Le démarrage : l'horloge monotone repart de zéro.
                (self.boot_mur, self.boot_mono) = (t, 10);
                self.etat = etat;
                self.etat.grace(self.boot_mono);
                let tt = self.temps(t);
                let ancien = std::mem::replace(&mut self.moteur, Moteur::new(compiler(&self.defs, &self.etat), &self.etat, &self.lieu));
                let mut m = Moteur::redemarrer(inst, &self.etat, &self.defs, tt, &self.lieu, ancien.journal);
                (m.ordres, m.notices, m.chemins) = (ancien.ordres, ancien.notices, ancien.chemins);
                self.moteur = m;
                self.en_panne = false;
                // Les appareils se reconnectent et disent leur état (le capteur muet, non).
                for i in 0..self.copies {
                    for kk in 0..CLES.len() {
                        let n = self.pts[i as usize][kk];
                        if !self.silencieux(n, t) {
                            let v = [(n, self.vrai[n as usize])];
                            self.message(t, &v);
                        }
                    }
                }
            }
            Ev::Modifier => {
                self.defs = automatismes(self.copies, true);
                let defs = std::mem::take(&mut self.defs);
                let tt = self.temps(t);
                self.mesurer(|s| s.moteur.recompiler(&s.etat, &defs, tt));
                self.defs = defs;
            }
            Ev::Retour(n) => {
                let v = [(n, self.vrai[n as usize])];
                self.message(t, &v);
            }
            Ev::Relever(nom) => {
                let r = self.relever(t, nom);
                self.releves.push(r);
            }
        }
    }

    fn fiche(&self, a: u32) -> Option<Fiche> {
        let m = &self.moteur;
        let c = m.en_cours.get(a as usize).copied().filter(|&c| c != u32::MAX)?;
        let e = m.ex[c as usize];
        let plan = m.prog.plans[a as usize];
        Some(Fiche {
            etat: e.etat,
            noeud: m.prog.ids[(plan.premier + e.curseur) as usize].to_string(),
            registre: e.registre,
            cause: self.etat.annuaire.nom(e.cause_point).to_string(),
            generation: e.generation,
        })
    }

    fn relever(&self, t: u32, nom: &'static str) -> Releve {
        let (a, b) = (1u32, 0u32);
        let m = &self.moteur;
        let c = (0..m.prog.c_noeud.len()).find(|&c| m.prog.noeud_auto[m.prog.c_noeud[c] as usize] == a).unwrap();
        let temp = self.p(0, "temp");
        let d = self.etat.defs[temp as usize].appareil as usize;
        Releve {
            t,
            nom,
            a: self.fiche(a),
            b: self.fiche(b),
            a_condition_vraie: m.c_vrai[c],
            a_premier: m.prog.plans[a as usize].premier,
            temp_qualite: self.etat.lire(temp).1,
            temp_muet: self.etat.sante[d].vie == crate::valeurs::vie::MUET,
        }
    }

    /// Joue la soirée jusqu'à 23:15:00.
    pub fn jouer(&mut self) {
        let mut dernier = 0;
        let mut tours = 0;
        loop {
            let t_file = self.file.keys().next().map_or(u32::MAX, |k| k.0);
            let maintenant = dernier.max(DEBUT);
            let t_reveil = if self.en_panne { u32::MAX } else { self.mur_de(self.moteur.prochain_reveil(&self.etat, self.temps(maintenant))) };
            let t_hub = if self.en_panne { u32::MAX } else { self.hub.prochain().saturating_add(999).checked_div(1_000).map_or(u32::MAX, |x| x as u32) };
            let t = t_file.min(t_reveil).min(t_hub);
            if t > FIN {
                break;
            }
            tours = if t == dernier { tours + 1 } else { 0 };
            assert!(tours < 10_000, "le moteur ne progresse plus à {}", heure(t));
            dernier = t;
            while let Some((&(te, s), _)) = self.file.iter().next() {
                if te != t {
                    break;
                }
                let ev = self.file.remove(&(te, s)).unwrap();
                self.traiter(t, ev);
            }
            if !self.en_panne && self.hub.prochain() <= u64::from(t) * 1_000 {
                let sorties = self.hub.passage(u64::from(t) * 1_000, &|_| None);
                for so in sorties {
                    if let Sortie::Niveau { point, valeur, .. } = so {
                        self.vrai[point as usize] = valeur;
                        self.message(t, &[(point, valeur)]);
                    }
                }
            }
            let tt = self.temps(t);
            if !self.en_panne && self.moteur.prochain_reveil(&self.etat, tt) <= tt.mono {
                self.mesurer(|s| s.moteur.passage(&mut s.etat, tt));
                self.passages += 1;
                self.suivre_ordres();
            }
        }
    }

    /// Les ordres d'une copie : (heure, clé du point, valeur).
    pub fn ordres(&self, i: u32) -> Vec<(String, &'static str, i64)> {
        self.moteur
            .ordres
            .iter()
            .filter_map(|o| {
                let kk = self.pts[i as usize].iter().position(|&x| x == o.point)?;
                Some((heure(o.t), CLES[kk].0, o.valeur))
            })
            .collect()
    }

    /// Les avis d'une copie, les noms de points remis à la copie 0.
    pub fn notices(&self, i: u32) -> Vec<(String, String)> {
        self.moteur
            .notices
            .iter()
            .filter(|(_, a, _)| a / PAR_COPIE == i)
            .map(|(t, _, s)| (heure(*t), s.replace(&format!("_{i}/"), "_0/").replace(&format!("agenda {},", i + 1), "agenda 1,")))
            .collect()
    }

    /// Le chemin d'une exécution de la copie 0 : les nœuds par lesquels elle est passée.
    pub fn chemin(&self, automatisme: u32, depart: &str) -> Vec<String> {
        let mut v: Vec<(u32, u16, String)> = Vec::new();
        let mut actif: Option<(u32, u16)> = None;
        for (a, case, generation, id) in &self.moteur.chemins {
            if *a != automatisme {
                continue;
            }
            if actif.is_none() && id == depart {
                actif = Some((*case, *generation));
            }
            if actif == Some((*case, *generation)) {
                v.push((*case, *generation, id.clone()));
            }
        }
        v.into_iter().map(|x| x.2).collect()
    }

    pub fn journal_a(&self, t: u32) -> Vec<&str> {
        self.moteur.journal.iter().filter(|(x, _)| *x == t).map(|(_, s)| s.as_str()).collect()
    }
}

pub fn libelle_etat(e: u8) -> &'static str {
    match e {
        etat_ex::LIBRE => "libre",
        etat_ex::PRETE => "prête",
        etat_ex::DUREE => "attend une durée",
        etat_ex::ATTEND_QUE => "attend une condition",
        etat_ex::ISSUE => "attend une issue",
        _ => "?",
    }
}

/// Joue le scénario et l'écrit en clair ; en N copies, mesure le moteur.
pub fn banc(copies: u32) {
    use crate::outils::resultat;
    let sec = "scenario";
    let d = Instant::now();
    let mut s = Scenario::new(copies);
    let preparer_ms = d.elapsed().as_secs_f64() * 1e3;
    let d = Instant::now();
    s.jouer();
    let jouer_ms = d.elapsed().as_secs_f64() * 1e3;
    let reference = s.ordres(0);
    let avis = s.notices(0);
    let identiques = (1..copies).all(|i| s.ordres(i) == reference && s.notices(i) == avis);
    resultat(sec, copies, "automatismes", s.defs.len() as f64, "");
    resultat(sec, copies, "noeuds", s.moteur.prog.noeuds.len() as f64, "");
    resultat(sec, copies, "points", s.etat.vifs.len() as f64, "");
    resultat(sec, copies, "messages", s.messages as f64, "");
    resultat(sec, copies, "passages", s.passages as f64, "");
    resultat(sec, copies, "ordres", s.moteur.ordres.len() as f64, "");
    resultat(sec, copies, "ordres_par_copie", reference.len() as f64, "");
    resultat(sec, copies, "copies_identiques", f64::from(u8::from(identiques)), "");
    resultat(sec, copies, "cpu_moteur_toute_la_soiree", s.cpu_ns as f64 / 1e6, "ms");
    resultat(sec, copies, "cpu_par_message", s.cpu_ns as f64 / s.messages as f64, "ns");
    resultat(sec, copies, "preparer", preparer_ms, "ms");
    resultat(sec, copies, "jouer_avec_le_banc", jouer_ms, "ms");
    resultat(sec, copies, "memoire_programme", s.moteur.prog.octets() as f64, "o");
    resultat(sec, copies, "memoire_programme_textes", s.moteur.prog.octets_textes() as f64, "o");
    let (defs, o_defs) = crate::outils::poids(|| automatismes(copies, false));
    resultat(sec, copies, "memoire_definitions_aujourdhui", o_defs as f64, "o");
    std::hint::black_box(defs.len());
    resultat(sec, copies, "memoire_executions", (s.moteur.ex.capacity() * 64) as f64, "o");
    resultat(sec, copies, "executions_max", s.moteur.ex.len() as f64, "");
    resultat(sec, copies, "memoire_attentes", (s.moteur.at_echeance.capacity() * (4 + 8 + 8 + 4)) as f64, "o");
    // Une fiche horaire : 24 o, plus son état (prochaine 8, dernière 8, niveau 1, événements 4 + 4).
    resultat(sec, copies, "memoire_horaires", (s.moteur.cal.fiches.capacity() * (24 + 8 + 8 + 1 + 4 + 4)) as f64, "o");
    resultat(sec, copies, "horaires", s.moteur.cal.fiches.len() as f64, "");
    resultat(sec, copies, "memoire_verite", (s.moteur.f_vrai.capacity() + s.moteur.c_vraies.capacity() * 2 + s.moteur.c_vrai.capacity() + s.moteur.c_parti.capacity() * 8) as f64, "o");
    assert!(identiques, "une copie s'est comportée autrement que la copie 0");
    if copies == 1 {
        println!("\n== Ordres ==");
        for (t, c, v) in &reference {
            println!("{t}  {c:<12} ← {v}");
        }
        println!("\n== Avis ==");
        for (t, x) in &avis {
            println!("{t}  {x}");
        }
        println!("\n== Relevés d'état ==");
        for r in &s.releves {
            println!("{}  {}", heure(r.t), r.nom);
            for (qui, f) in [("A", &r.a), ("B", &r.b)] {
                match f {
                    Some(f) => println!("      {qui} : {} au nœud {}, registre {}, cause {}, génération {}", libelle_etat(f.etat), f.noeud, f.registre, f.cause, f.generation),
                    None => println!("      {qui} : aucune exécution"),
                }
            }
            println!("      condition d'entrée de A vraie : {} ; A commence au nœud global {}", r.a_condition_vraie, r.a_premier);
        }
        println!("\n== Chemin de A, de 22:01 ==");
        println!("{}", s.chemin(1, "tout_ferme").join(" → "));
        println!("\n== Journal ==");
        for (t, x) in &s.moteur.journal {
            println!("{}  {x}", heure(*t));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jouer(copies: u32) -> Scenario {
        let mut s = Scenario::new(copies);
        s.jouer();
        s
    }

    #[test]
    fn les_ordres_de_la_soiree() {
        let s = jouer(1);
        let attendu: Vec<(String, &str, i64)> = [
            ("19:10:00", "lum_salon", 0),   // D : allumée depuis 10 min, éteinte par sa cause
            ("19:55:00", "lum_salon", 0),   // D : la première attente (19:30) est tombée, celle de 19:45 part
            ("21:00:00", "lum_couloir", 1), // B, le jour
            ("21:01:30", "lum_couloir", 1), // B relancé par un nouveau mouvement
            ("21:03:31", "lum_couloir", 0), // B, la fin de la relance (pas 21:02:05)
            ("22:01:00", "lum_salon", 0),   // A : départ à 22:01, condition vraie depuis 21:51
            ("22:06:01", "volet", 0),       // A : après 5 min, personne au salon
            ("22:06:45", "alarme", 2),      // A : le volet a confirmé 0 (pas 50)
            ("22:06:47", "mode", 1),        // A : l'alarme a confirmé
            ("22:19:00", "veilleuse", 1),   // B, la nuit : veilleuse ; interrompu par la panne
            ("22:30:00", "veilleuse", 1),   // B ; arrêté par sa modification à 22:31
            ("22:36:48", "consigne", 1700), // A, après le redémarrage et la recompilation
            ("23:08:00", "veilleuse", 1),   // B modifié
            ("23:10:01", "veilleuse", 0),   // B modifié, jusqu'au bout
        ]
        .iter()
        .map(|&(t, c, v)| (t.to_string(), c, v))
        .collect();
        assert_eq!(s.ordres(0), attendu);
    }

    #[test]
    fn les_avis_et_les_causes() {
        let s = jouer(1);
        let attendu: Vec<(String, String)> = [
            ("20:30:00", "Coucher : conditions non réunies (z2m:bouton_chambre_0/action = 1 (avant : 0) à 20:30:00)."),
            ("21:20:00", "Rendez-vous dans 10 min (agenda 1, événement 1, rappel de 21:20:00)."),
            ("22:25:00", "Porte d'entrée ouverte depuis 5 min, la nuit ! (z2m:porte_entree_0/contact = 0 (avant : 1) à 22:17:00)"),
            ("22:25:00", "Tâche de nuit (l'horloge, 22:24:00, 60 s de retard)."),
            ("22:25:00", "Rendez-vous dans 10 min (agenda 1, événement 2, rappel de 22:21:00)."),
            ("22:30:00", "Ronde de 22 h 30 : la porte d'entrée est ouverte."),
            ("22:37:49", "Chauffage : consigne non confirmée (température au départ : 2130)."),
            ("22:37:49", "Maison couchée ; dernière fenêtre fermée : z2m:fenetre_bureau_0/contact = 1 (avant : 0) à 21:51:00 ; température au départ : 2130."),
            ("22:56:00", "Rendez-vous dans 10 min (agenda 1, événement 3, rappel de 22:50:00)."),
            ("23:04:00", "Rendez-vous dans 10 min (agenda 1, événement 4, rappel de 23:04:00)."),
            ("23:04:00", "Rendez-vous dans 10 min (agenda 1, événement 5, rappel de 23:04:00)."),
            ("23:05:00", "Coucher : conditions non réunies (z2m:fenetre_cuisine_0/contact = 1 (avant : 1) à 18:00:00)."),
            ("23:10:02", "Couloir éteint."),
            ("23:12:00", "Alarme en alerte depuis 1 min (alarme:maison_0/mode = 3 (avant : 2) à 23:11:00)."),
        ]
        .iter()
        .map(|&(t, x)| (t.to_string(), x.to_string()))
        .collect();
        assert_eq!(s.notices(0), attendu);
    }

    #[test]
    fn le_chemin_de_a_avance_seulement() {
        let s = jouer(1);
        let attendu = [
            "tout_ferme", "verifier", "lire_temperature", "eteindre_salon", "attendre_5", "encore_personne",
            "fermer_volet", "verifier_volet", "armer", "verifier_alarme", "mode_nuit", "attendre_30",
            "chauffage", "verifier_chauffage", "chauffage_ko", "selon_cause", "fin_fenetre",
        ];
        assert_eq!(s.chemin(1, "tout_ferme"), attendu);
    }

    #[test]
    fn les_etats_releves() {
        let s = jouer(1);
        let r = |nom: &str| s.releves.iter().find(|r| r.nom.starts_with(nom)).unwrap().clone();
        let bureau = "z2m:fenetre_bureau_0/contact".to_string();
        // Le mouvement de 22:03 ne fait pas revenir A en arrière : il attend toujours ses 5 min.
        let x = r("après le mouvement");
        let a = x.a.unwrap();
        assert_eq!((a.etat, a.noeud.as_str(), a.registre, a.cause.clone()), (etat_ex::DUREE, "attendre_5", 2130, bureau.clone()));
        // Avant la panne : A attend ses 30 min, avec sa cause et son registre (2130, pas 2090) ;
        // B attend sa veilleuse ; la porte ouverte rend la condition d'entrée de A fausse.
        let x = r("juste avant la panne");
        let a = x.a.unwrap();
        assert_eq!((a.etat, a.noeud.as_str(), a.registre, a.cause.clone()), (etat_ex::DUREE, "attendre_30", 2130, bureau.clone()));
        assert_eq!(x.b.unwrap().noeud, "attendre_veilleuse");
        assert!(!x.a_condition_vraie);
        // Après le redémarrage : la fiche de A est intacte ; B a été interrompu.
        let y = r("juste après le redémarrage");
        assert_eq!(y.a.clone().unwrap(), r("juste avant la panne").a.unwrap());
        assert!(y.b.is_none());
        // Après la modification de B : A n'a pas bougé de nœud, mais ses nœuds ont glissé d'une place.
        let z = r("juste après la modification");
        assert_eq!(z.a.clone().unwrap().noeud, "attendre_30");
        assert_eq!(z.a_premier, y.a_premier + 1);
        assert!(z.b.is_none(), "B, modifié, a été arrêté");
        // À la fin : rien ne tourne ; la maison est en nuit, l'alarme armée, la veilleuse éteinte.
        assert!(s.moteur.ex.iter().all(|e| e.etat == etat_ex::LIBRE));
        let v = |c: &str| s.etat.vifs[s.p(0, c) as usize].valeur;
        assert_eq!((v("mode"), v("alarme"), v("veilleuse"), v("volet")), (1, 2, 0, 0));
        // Le thermomètre se tait depuis 22:40. À 23:00, sa valeur est trop vieille (fraîcheur de 15 min
        // dépassée à 22:52:31), donc mauvaise pour une règle, alors que l'appareil n'est pas encore muet
        // (il le sera à 23:07:31, 30 min après son dernier signe de vie).
        let x = r("le thermomètre se tait");
        assert_eq!((x.temp_qualite, x.temp_muet), (crate::valeurs::qualite::MAUVAISE, false));
        assert_eq!(s.etat.lire(s.p(0, "porte")).1, crate::valeurs::qualite::BONNE);
    }

    #[test]
    fn le_journal_dit_pourquoi() {
        let s = jouer(1);
        let a = |h: u32, m: u32, sec: u32, texte: &str| {
            let t = hms(h, m, sec);
            assert!(s.journal_a(t).iter().any(|l| l.contains(texte)), "{} : « {texte} » absent de {:?}", heure(t), s.journal_a(t));
        };
        a(19, 40, 0, "Extinction 0 : attente abandonnée");
        a(21, 0, 5, "issue absente");
        a(21, 1, 30, "relancée par un nouveau déclenchement");
        a(21, 1, 40, "issue tardive écartée");
        a(21, 51, 0, "condition composée vraie depuis 21:51:00");
        a(22, 10, 0, "déclenchement ignoré, déjà en cours");
        a(22, 25, 0, "interrompue : échéance dépassée de 239 s");
        a(22, 25, 0, "Coucher 0 : reprise, réveil à 22:36:48");
        a(22, 31, 0, "arrêtée : logique modifiée");
        a(22, 45, 0, "Alerte 0 : attente abandonnée");
        a(22, 25, 0, "Tâches 0 : occurrence de 22:21:30 manquée (1)");
        a(22, 52, 31, "valeur trop vieille : z2m:thermometre_salon_0/temperature");
        a(22, 55, 1, "appareil muet : z2m:fenetre_cuisine_0/contact");
        a(23, 0, 0, "Coucher 0 : attente abandonnée");
        a(23, 5, 0, "appareil revenu");
        // Le message de 21:42 (une fenêtre fermée, une autre ouverte) n'a rien armé.
        assert!(!s.journal_a(hms(21, 42, 0)).iter().any(|l| l.contains("Coucher")), "{:?}", s.journal_a(hms(21, 42, 0)));
    }

    #[test]
    fn cent_maisons_identiques() {
        let s = jouer(100);
        let reference = s.ordres(0);
        assert_eq!(reference.len(), 14);
        for i in 1..100 {
            assert_eq!(s.ordres(i), reference, "copie {i}");
            assert_eq!(s.notices(i), s.notices(0), "copie {i}");
        }
    }
}
