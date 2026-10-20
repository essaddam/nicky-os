# Contrats chaleur

Tests partagés inclus uniquement par les modules `#[cfg(test)]` des pilotes Ariston
et Voltalis. Les comptes et appareils sont fictifs ; aucun accès cloud ou physique.

- `caller.rs` : boucle réelle, vrai hub et réponses cloud en mémoire ; abandon pendant
  renouvellement de session, échéance, résultat après émission, absence de rejeu.
- `fixture.rs` : vérifie que les paniques de workers et erreurs de boucle font échouer
  explicitement le montage après jonction des tâches.
- `transport.rs` : vrai TCP loopback, TLS vérifié avec certificat éphémère en mémoire,
  vrai client Hyper ; abandon/échéance avant TCP, après TCP, après TLS et avant remise
  à HTTP ; témoins sans abandon, réponse après réception complète et départ de
  l'appelant, rejet d'un certificat non fiable. Les tâches client et serveur sont
  détenues et jointes. Le signal de fin de l'ordre remplace une temporisation supposée.
- `failures.rs` : expiration de confirmation sous quota, trois HTTP 500 comptés.
- `crates/moli-voltalis/src/tests/p1/guard.rs` : vraie garde du hub, consigne/préréglage
  sous `Control`, approbation humaine selon l'origine et le contexte, droits existants
  UI/automatisme validé et chauffe-eau en lecture seule.

Le support opt-in `moli-net/contract-test` est activé par les dépendances de
**développement** des pilotes. Le contexte est local à la tâche, sans registre global ;
les seuls hôtes fixes des deux pilotes sont redirigés vers loopback. Les observations
suspendent les étapes réseau et ne décident jamais si un ordre est encore autorisé.
L'autorisation appartient à `SendPermit` et `web_if`. Sans contexte, le réseau public
et ses autorités restent inchangés ; sans feature, ces observations ne sont pas compilées.

```sh
RUSTUP_TOOLCHAIN=1.99.1 CARGO_TARGET_DIR="$PWD/target" cargo test -j 4 --locked -p moli-ariston -p moli-voltalis -p moli-runtime -p moli-automation
```

Les tests de fenêtres du ballon utilisent le JSON de la recette et le prédicat réel
`in_window`, aux quatre bornes et juste avant. Le test de redémarrage couvre les trois
modes avec relais initialement ouvert et fermé. La qualification réelle reste soumise
au jalon humain H1 ; Home Assistant reste propriétaire avant validation de la bascule.

## Garde-fou natif

Sur macOS, les quatre tests ci-dessous lient des adresses `127.0.0.x` réservées à
Linux. Le garde-fou natif exclut exactement ces cas, exécutés séparément dans
`moli-os-dev` sur une copie des mêmes sources. Il ne modifie pas `scripts/check.sh`.

```sh
RUSTUP_TOOLCHAIN=1.99.1 CARGO_TARGET_DIR="$PWD/target" CARGO_BUILD_JOBS=4 sh -c '
cargo() {
    if test "$1" = test; then
        command cargo "$@" -- --skip tests::a_device_that_changed_address_is_found_and_followed --skip tests::the_device_whose_address_another_one_took_is_found_elsewhere --skip tests::another_device_at_its_configured_address_is_not_taken_before_a_long_search --skip tests::an_address_a_person_changed_is_obeyed
    else
        command cargo "$@"
    fi
}
. scripts/check.sh
'
```

Dans la copie Linux :

```sh
cargo test -j 4 --locked -p moli-profile --lib -- --exact tests::a_device_that_changed_address_is_found_and_followed tests::the_device_whose_address_another_one_took_is_found_elsewhere tests::another_device_at_its_configured_address_is_not_taken_before_a_long_search tests::an_address_a_person_changed_is_obeyed --test-threads=1
```
