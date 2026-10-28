//! Les ordres et les appareils virtuels de la maison du banc (la forme des deux maisons, ×1 :
//! 496 appareils, 2 264 valeurs), par classe d'appareil. Hypothèse, à remplacer par le relevé des
//! deux maisons quand il existera :
//!
//! - lampes (70 % des appareils « à l'action ») : allumer, luminosité, teinte, en six modèles de
//!   bornes ; volets RTS (15 %) : ouvrir, fermer, stop, favori ; télécommandes (15 %) : aucun ordre ;
//! - prises Wi-Fi : allumer ;
//! - volets io (appareils interrogés) : position, ouvrir, fermer, stop ;
//! - vannes thermostatiques (5 % des capteurs Zigbee) : consigne ;
//! - capteurs sur pile : aucun ordre ;
//! - un groupe de lumières pour 8 lampes, et un appareil virtuel « pièce » pour 10 capteurs, qui
//!   montre ses valeurs et l'ordre de son groupe.
//!
//! Les valeurs sont celles de la maison du banc mémoire, les mêmes des deux côtés de chaque
//! comparaison (un volet RTS y garde les valeurs génériques du générateur, qu'un vrai n'a pas).

use crate::catalogue::{FormeDecl, binaire, drapeau, effet, mesure, nature, retour};
use crate::hub::{AppDecl, DefPoint, role, sorte};
use crate::maison::{Classe, Maison, Nature};

/// Le dictionnaire des énumérations génériques de la maison du banc.
pub const DICO_MODE: u16 = 1;

pub fn dico_mode() -> Vec<(i32, Box<str>)> {
    vec![(1, "auto".into()), (2, "manuel".into()), (3, "absent".into())]
}

/// La définition d'une valeur de la maison du banc (ce que la mémoire v2 range dans `Def`).
pub fn def_de(n: Nature) -> DefPoint {
    match n {
        Nature::Mesure => DefPoint { nature: nature::MESURE, unite: 1, ..DefPoint::default() },
        Nature::Binaire => DefPoint { nature: nature::BINAIRE, ..DefPoint::default() },
        Nature::Enum => DefPoint { nature: nature::ENUM, dico: DICO_MODE, ..DefPoint::default() },
        Nature::Texte | Nature::Vide => DefPoint { nature: nature::TEXTE, ..DefPoint::default() },
    }
}

/// La classe d'une lampe, d'un volet RTS ou d'une télécommande, parmi les appareils « à l'action ».
pub fn est_lampe(classe: Classe, d: usize) -> bool {
    classe == Classe::Action && d % 20 < 14
}
pub fn est_rts(classe: Classe, d: usize) -> bool {
    classe == Classe::Action && (14..17).contains(&(d % 20))
}

/// Les ordres d'un appareil de la maison du banc, par la clé des valeurs qu'ils règlent.
fn ordres_de(classe: Classe, d: usize, cles: &[String]) -> Vec<(String, FormeDecl)> {
    let a = |k: &str| cles.iter().any(|c| c == k);
    match classe {
        Classe::Action if est_lampe(classe, d) => {
            let lum_max = if d % 2 == 0 { 254 } else { 100 };
            let (tmin, tmax) = [(153, 454), (153, 500), (250, 454), (153, 370), (200, 500), (158, 500)][d % 6];
            let mut v = vec![("on".to_string(), FormeDecl::regler(binaire(), if a("on") { "on" } else { "" }, 0))];
            if a("brightness") {
                v.push(("brightness".into(), FormeDecl::regler(mesure(0, lum_max, 1, 0, 0), "brightness", 2)));
            }
            if a("color_temp") {
                v.push(("color_temp".into(), FormeDecl::regler(mesure(tmin, tmax, 1, 0, 0), "color_temp", 5)));
            }
            v
        }
        Classe::Action if est_rts(classe, d) => ["ouvrir", "fermer", "stop", "favori"].iter().map(|k| (k.to_string(), FormeDecl::sans_param(effet::APPUI, retour::PONT))).collect(),
        Classe::Action | Classe::Pile => Vec::new(),
        Classe::Wifi => vec![("on".into(), FormeDecl::regler(binaire(), if a("on") { "on" } else { "" }, 0))],
        Classe::Api => {
            let cle = if a("core:ClosureState") { "core:ClosureState" } else { "" };
            let mut position = FormeDecl::regler(mesure(0, 100, 1, 0, 2), cle, 1);
            position.drapeaux |= drapeau::A_L_ARRET;
            position.suivi_s = 120;
            position.retour = retour::ACCUSE;
            let fin = |x: i32| FormeDecl {
                attendus: vec![x],
                confirme: cle.to_string(),
                drapeaux: drapeau::A_L_ARRET,
                suivi_s: 120,
                ..FormeDecl::sans_param(effet::APPUI, retour::ACCUSE)
            };
            vec![
                ("position".into(), position),
                ("ouvrir".into(), fin(100)),
                ("fermer".into(), fin(0)),
                ("stop".into(), FormeDecl::sans_param(effet::APPUI, retour::AUCUN)),
            ]
        }
        Classe::Zigbee if d % 20 == 0 => vec![("consigne".into(), FormeDecl::regler(mesure(50, 300, 5, -1, 1), if a("temperature") { "temperature" } else { "" }, 0))],
        Classe::Zigbee => Vec::new(),
    }
}

/// Les clés des valeurs d'un appareil de la maison (deux fois la même clé : la seconde est suffixée).
pub fn cles_de(m: &Maison, d: usize) -> Vec<String> {
    let a = &m.appareils[d];
    let points = &m.points[a.premier as usize..(a.premier + a.nb) as usize];
    let mut vues = std::collections::HashSet::new();
    points
        .iter()
        .enumerate()
        .map(|(k, p)| {
            let c = p.nom.rsplit('/').next().unwrap_or("").to_string();
            if vues.insert(c.clone()) { c } else { format!("{c}_{k}") }
        })
        .collect()
}

/// Les déclarations de toute la maison : les appareils physiques, puis les virtuels.
pub fn declarations(m: &Maison) -> Vec<AppDecl> {
    let mut v = Vec::with_capacity(m.appareils.len() * 11 / 10);
    let mut lampes = Vec::new();
    let mut capteurs = Vec::new();
    for (d, a) in m.appareils.iter().enumerate() {
        let cles = cles_de(m, d);
        let defs = m.points[a.premier as usize..(a.premier + a.nb) as usize].iter().map(|p| def_de(p.nature)).collect();
        let ordres = ordres_de(a.classe, d, &cles);
        if est_lampe(a.classe, d) && !ordres.is_empty() {
            lampes.push(a.nom.clone());
        }
        if a.classe == Classe::Zigbee {
            capteurs.push((a.nom.clone(), cles[0].clone()));
        }
        v.push(AppDecl { nom: a.nom.clone(), piece: (d % 12) as u16, points: cles, defs, ordres, membres: Vec::new() });
    }
    // Un groupe de lumières pour 8 lampes : un ordre de groupe, qui cible leurs « on ».
    for (g, groupe) in lampes.chunks(8).enumerate() {
        let mut membres = Vec::new();
        for l in groupe {
            membres.push((sorte::ORDRE, format!("{l}/on"), role::CIBLE, "on".to_string()));
            membres.push((sorte::APPAREIL, l.clone(), role::MEMBRE, String::new()));
        }
        let mut f = FormeDecl::regler(binaire(), "", 0);
        f.drapeaux |= drapeau::GROUPE;
        f.retour = retour::ETAT;
        v.push(AppDecl { nom: format!("lumieres:groupe_{g:04}"), piece: (g % 12) as u16, points: Vec::new(), defs: Vec::new(), ordres: vec![("on".into(), f)], membres });
    }
    // Une « pièce » pour 10 capteurs : elle montre leurs premières valeurs.
    for (k, lot) in capteurs.chunks(10).enumerate() {
        let membres = lot.iter().map(|(n, c)| (sorte::VALEUR, format!("{n}/{c}"), role::AFFICHE, String::new())).collect();
        v.push(AppDecl { nom: format!("virtuel:piece_{k:04}"), piece: (k % 12) as u16, points: Vec::new(), defs: Vec::new(), ordres: Vec::new(), membres });
    }
    v
}
