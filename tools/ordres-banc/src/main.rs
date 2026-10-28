//! Banc de la proposition « les ordres des appareils » de Sarah Home
//! (docs/proposition/ordres-appareils.md, analyse).
//! Usage : banc-ordres <section> [échelle]
//! Sections : catalogue, demandes, clients, soiree. Une section et une échelle par processus.

// Un banc : le vocabulaire complet (raisons, origines, états) reste, même quand une section ne s'en sert pas.
#![allow(dead_code)]

// Repris tels quels du banc mémoire : la maison (496 appareils, 2 264 valeurs à ×1), la valeur
// d'aujourd'hui et les outils de mesure ; du banc du nettoyage : le numéro à génération et son
// registre.
#[allow(dead_code, unused_imports)]
#[path = "../../memoire-banc/src/outils.rs"]
mod outils;
#[allow(dead_code, unused_imports)]
#[path = "../../memoire-banc/src/maison.rs"]
mod maison;
#[allow(dead_code, unused_imports)]
#[path = "../../memoire-banc/src/valeurs.rs"]
mod valeurs;
#[allow(dead_code)]
#[path = "../../nettoyage-banc/src/numeros.rs"]
mod numeros;

mod aujourdhui;
mod banc;
mod catalogue;
mod demandes;
mod droits;
#[cfg(test)]
mod essais;
#[cfg(test)]
mod essais_droits;
mod hub;
mod maison_ordres;
mod module;
mod moteur;
mod soiree;

#[global_allocator]
static COMPTEUR: outils::Compteur = outils::Compteur;

fn main() {
    let mut args = std::env::args().skip(1);
    let section = args.next().unwrap_or_else(|| "soiree".into());
    let echelle: u32 = args.next().and_then(|e| e.parse().ok()).unwrap_or(1);
    match section.as_str() {
        "catalogue" => banc::catalogue(echelle),
        "demandes" => banc::demandes(echelle),
        "clients" => banc::clients(echelle),
        "soiree" => banc::soiree(),
        autre => panic!("section inconnue : {autre}"),
    }
    if let Some(rss) = outils::rss_kio() {
        outils::resultat(&section, echelle, "rss_fin", rss as f64, "Kio");
    }
}
