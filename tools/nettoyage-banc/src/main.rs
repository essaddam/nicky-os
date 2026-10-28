//! Banc de la proposition « nettoyage des équipements » de Sarah Home
//! (docs/proposition/nettoyage-equipements.md).
//! Usage : banc-nettoyage <section> [échelle]
//! Sections : lecture, lot, vie, refonte. Une section et une échelle par processus.

// Repris tels quels du banc mémoire : la maison (496 appareils, 2 264 points à ×1), les fiches de la
// mémoire v2 (`Vif` 40 o, `Def` 16 o, `Sante` 20 o) et les outils de mesure.
#[allow(dead_code, unused_imports)]
#[path = "../../memoire-banc/src/outils.rs"]
mod outils;
#[allow(dead_code, unused_imports)]
#[path = "../../memoire-banc/src/maison.rs"]
mod maison;
#[allow(dead_code, unused_imports)]
#[path = "../../memoire-banc/src/valeurs.rs"]
mod valeurs;

mod banc;
mod memoire;
mod moteur;
mod numeros;
mod refonte;

#[global_allocator]
static COMPTEUR: outils::Compteur = outils::Compteur;

fn main() {
    let mut args = std::env::args().skip(1);
    let section = args.next().unwrap_or_else(|| "refonte".into());
    let echelle: u32 = args.next().and_then(|e| e.parse().ok()).unwrap_or(1);
    match section.as_str() {
        "lecture" => banc::lecture(echelle),
        "lot" => banc::lot(echelle),
        "vie" => banc::vie(echelle),
        "refonte" => refonte::banc(),
        autre => panic!("section inconnue : {autre}"),
    }
    if let Some(rss) = outils::rss_kio() {
        outils::resultat(&section, echelle, "rss_fin", rss as f64, "Kio");
    }
}
