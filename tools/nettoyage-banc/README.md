# Banc du nettoyage des équipements

Le banc de la proposition [« nettoyage des équipements »](../../docs/proposition/nettoyage-equipements.md)
(+ [`.html`](../../docs/proposition/nettoyage-equipements.html)) : sortir un appareil, et réutiliser
sa place. Il sert à trois choses :

- **vérifier** le registre proposé (le numéro = une place et sa génération), le lot, la succession et
  le déménagement, sur une refonte rejouée minute par minute, avec deux copies de secours et un client
  à cache ;
- **mesurer** le coût du passage (lot, déménagement) à ×1, ×10 et ×100, la lecture par numéro, et
  vingt ans de la vie d'une maison sous trois règles ;
- **reproduire** les chiffres et les schémas du document.

Crate hors workspace (`[workspace]` vide), sans dépendance aux crates de Sarah Home : seulement
`hashbrown` 0.17, déjà dans le `Cargo.lock` du dépôt. Il reprend tels quels, par inclusion de
fichiers (`#[path]`), la maison, les fiches de la mémoire v2 et les outils du
[banc mémoire](../memoire-banc/).

## Contenu

| Fichier | Rôle |
| --- | --- |
| `src/numeros.rs` | le numéro (place 24 bits, génération 7 bits), le registre : statuts, plages libres, tiroir des noms, lot, déménagement, annuaire de l'état complet ; 6 tests |
| `src/memoire.rs` | les colonnes de la mémoire v2 rangées par place ; les sorties critiques (déclaration, valeur, statut, lot, déménagement) ; l'état complet ; l'empreinte exacte et l'empreinte par les noms |
| `src/moteur.rs` | le moteur réduit : programme compilé depuis les noms, exécutions, attentes revérifiées, file des exécutions finies ; le client à cache |
| `src/refonte.rs` | la refonte rejouée sur trois jours et ses 12 tests |
| `src/banc.rs` | les mesures : `lecture`, `lot` (et déménagement), `vie` (vingt ans) |
| `scripts/linux.sh` | la série en Linux musl, chaque mesure dans trois conteneurs neufs de 2 CPU et 128 Mio |
| `scripts/mutations.py` | 11 fautes injectées ; chacune doit faire échouer au moins un test |
| `sorties/linux/` | la série publiée, citée par le document |
| `rendu/figures.py` | les 12 schémas en relief (avec `tools/haute-dispo-banc/rendu/relief.py`) ; `rendu/gabarit.html`, la page |

## Lancer

```
# Les 18 tests (quelques secondes) :
cargo test --release --offline --manifest-path tools/nettoyage-banc/Cargo.toml

# Une section, en natif :
cargo run --release --offline --manifest-path tools/nettoyage-banc/Cargo.toml -- refonte
cargo run --release --offline --manifest-path tools/nettoyage-banc/Cargo.toml -- lot 10

# La série complète, comme la production (Docker ou Colima ; quelques minutes) :
sh tools/nettoyage-banc/scripts/linux.sh
sh tools/nettoyage-banc/scripts/linux.sh lecture   # une section seulement

# Les fautes injectées :
python3 -I tools/nettoyage-banc/scripts/mutations.py

# Les schémas, puis la page :
python3 -I tools/nettoyage-banc/rendu/figures.py
python3 -I tools/haute-dispo-banc/rendu/build.py docs/proposition/nettoyage-equipements.md \
  docs/proposition/nettoyage-equipements.html tools/nettoyage-banc/rendu/gabarit.html \
  tools/nettoyage-banc/rendu/fig nettoyage-equipements
```

## Pièges

- `gen` est un mot réservé en édition 2024 : ne pas en faire un nom de variable.
- Un test vert ne prouve rien s'il ne sait pas échouer : rejouer `scripts/mutations.py` après tout
  changement du registre, de la mémoire ou du moteur.
- Les durées varient du simple au double sur une machine chargée : la série garde la médiane de trois
  conteneurs ; rejouer une section seule si sa charge l'a faussée (la lecture l'a été une fois).
