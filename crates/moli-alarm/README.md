# Moteur pur d’alarme

`step(state, event, now, config, profiles)` reçoit tout ce dont il a besoin et
renvoie un état et des effets typés. Le crate ne dépend que de la bibliothèque
standard : aucune entrée/sortie, horloge, tâche asynchrone ou commande physique.

Référence : spécification validée le 18 octobre 2026 sur la branche
`integ-alarme`, `docs/components/alarme.md`, notamment les sections 4, 5, 7, 9,
12 et 16. Cette livraison est le lot 2/9 ; elle ne remplace pas Alarmo dans une maison.

Les responsabilités sont réparties entre les demandes et autorisations
(`engine/commands`), les observations et politiques des capteurs (`coverage`,
`engine/sensors`), les transitions et sorties (`engine/transitions`), la
configuration (`configuration`) et les données de reprise (`restore`).

Le futur propriétaire doit valider la configuration contre ses descripteurs,
sérialiser les événements, fournir le temps et les droits actuels, persister
l’état métier puis exécuter les effets. `State::coverage` calcule les vues de
couverture hors de la table des transitions. `State::saved` produit uniquement
des données ; il ne lit ni n’écrit de fichier. Le traitement des codes secrets,
les transports, la persistance réelle, le mode ombre et les surfaces restent
aux lots suivants.

Chaque changement de génération réémet les minuteries encore actives avec leur
nouveau jeton et leur échéance absolue d’origine. Les anciens jetons sont ignorés.
Un arrêt de sirène arrivé à échéance reste obligatoire même si le compteur de
génération est épuisé. Aucune mise en marche n’est émise si son échéance bornée
ne peut pas être représentée. Une exclusion forcée ne survit pas au passage à
une nouvelle période d’armement.

Les tests gardent des états d’entrée indépendants du moteur, une matrice des
18 phases/modes × 16 familles d’événements et des variantes de politique, ainsi
que des contrats de reprise, de permissions et de réémission des minuteries.
Ils utilisent des identités fictives et ne touchent aucun appareil.

Validation native du crate :

```sh
RUSTUP_TOOLCHAIN=1.99.1 CARGO_TARGET_DIR="$PWD/target" CARGO_BUILD_JOBS=2 cargo test -j 2 --locked -p moli-alarm
RUSTUP_TOOLCHAIN=1.99.1 CARGO_TARGET_DIR="$PWD/target" CARGO_BUILD_JOBS=2 cargo clippy -j 2 --locked -p moli-alarm --all-targets -- -D warnings
```

Le format du workspace est contrôlé avec `cargo fmt --all -- --check`.
Le contrôle complet du projet reste réservé au tour de fusion.
