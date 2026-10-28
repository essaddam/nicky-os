//! Banc de l'étude haute disponibilité de Sarah Home (docs/proposition/haute-disponibilite.md).
//! Usage : banc-haute-dispo <section> [échelle]
//! Sections : copie, pont, distant, rattrapage, resync, instantane, jetons, stockage.
//! Une section et une échelle par processus.

// Repris tels quels du banc mémoire : la maison (496 appareils, 2 264 points à ×1), la mémoire des
// valeurs de la v2 (fiches `Vif` de 40 o, santé, échéances, annuaire) et les outils de mesure.
#[allow(dead_code, unused_imports)]
#[path = "../../memoire-banc/src/outils.rs"]
mod outils;
#[allow(dead_code, unused_imports)]
#[path = "../../memoire-banc/src/maison.rs"]
mod maison;
#[allow(dead_code, unused_imports)]
#[path = "../../memoire-banc/src/valeurs.rs"]
mod valeurs;

mod copie;
mod flux;
mod instantane;
mod jetons;
mod pont;
mod relais;
mod stockage;

#[global_allocator]
static COMPTEUR: outils::Compteur = outils::Compteur;

fn main() {
    let mut args = std::env::args().skip(1);
    let section = args.next().unwrap_or_else(|| "copie".into());
    let echelle: u32 = args.next().and_then(|e| e.parse().ok()).unwrap_or(1);
    match section.as_str() {
        "copie" => copie::banc(echelle),
        "pont" => pont::banc_local(echelle),
        "distant" => pont::banc_distant(echelle),
        "rattrapage" => pont::banc_rattrapage(echelle),
        "resync" => pont::banc_resync(echelle),
        "instantane" => instantane::banc(echelle),
        "jetons" => jetons::banc(echelle),
        "stockage" => stockage::banc(echelle),
        autre => panic!("section inconnue : {autre}"),
    }
    if let Some(rss) = outils::rss_kio() {
        outils::resultat(&section, echelle, "rss_fin", rss as f64, "Kio");
    }
}
