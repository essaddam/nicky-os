//! Banc de l'étude mémoire de Sarah Home (docs/proposition/memoire.md), hors dépôt.
//! Usage : banc-memoire <valeurs|echeances|passage|workflows|conditions|decisions|scenario|precision> [échelle]
//! (pour `scenario`, l'échelle est le nombre de copies de la maison).
//! Une section et une échelle par processus : la mémoire comptée est celle de la mesure seule.

mod calendrier;
mod conditions;
mod decisions;
mod echeances;
mod maison;
mod minuteries;
mod moteur;
mod outils;
mod precision;
mod scenario;
mod valeurs;
mod workflows;

#[global_allocator]
static COMPTEUR: outils::Compteur = outils::Compteur;

fn main() {
    let mut args = std::env::args().skip(1);
    let section = args.next().unwrap_or_else(|| "valeurs".into());
    let echelle: u32 = args.next().and_then(|e| e.parse().ok()).unwrap_or(1);
    match section.as_str() {
        "valeurs" => valeurs::banc(echelle),
        "echeances" => echeances::banc(echelle),
        "passage" => echeances::banc_echelle(),
        "workflows" => workflows::banc(echelle),
        "conditions" => conditions::banc(echelle),
        "decisions" => decisions::banc(echelle),
        "scenario" => scenario::banc(echelle),
        "precision" => precision::banc(echelle),
        autre => panic!("section inconnue : {autre}"),
    }
    if let Some(rss) = outils::rss_kio() {
        outils::resultat(&section, echelle, "rss_fin", rss as f64, "Kio");
    }
}
