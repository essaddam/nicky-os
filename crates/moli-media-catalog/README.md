# Contrats purs de catalogue média

Tranche A de R1 : types, validation pure et dix contrats synthétiques.
Ce paquet n'est pas une recherche de catalogue utilisable ni une cible de lancement.

## Présent dans ce lot

- Identités source et fournisseur séparées, neuf types de contenu, parents,
  provenance, fraîcheur, offres et entitlement explicite.
- Requête France/français, page et états de sources typés.
- Intention et capacités déclaratives de lecteurs fictifs ; aucun booléen de
  qualification opérationnelle accepté dans les objets.
- Dix contrats dans `tests/contracts.rs`, sur des données entièrement synthétiques.

Le workspace inclut automatiquement `crates/*`. Aucune dépendance de ce paquet
n'est ajoutée au binaire, à l'API ou aux pilotes.

## Validation et limites

`SearchQuery::parse` borne l'entrée avant décodage, refuse les champs inconnus
ou dupliqués, vérifie texte/page/filtres et normalise l'alias `fr` en `fr-FR`.

`SearchPage::validate` contrôle les bornes de page/réponse, identités, parents
déclarés, offres, états de sources et provenance/fraîcheur. Les URL restent des
données non fiables : syntaxe HTTPS restreinte, aucun accès ni rendu, aucune
preuve d'origine officielle ou de résolution publique.

`check_selection` distingue offre incohérente, capacités incompatibles, référence
fournisseur absente, offre expirée/future et preuve périmée. Il conserve les droits
inconnus et refuse toujours `PlayExact` faute de preuve d'adaptateur. Ces fonctions
ne modifient pas les objets reçus et n'émettent aucune commande.

Même le futur résultat `RequiresAdapterEvidence` ne pourra prouver ni une
intention conservée, ni un droit d'accès, ni une lecture : il restera une demande
de preuve à l'adaptateur. Les données désérialisées demeurent non fiables.

## Contrats préparés

| Contrat | Comportement attendu |
|---|---|
| homonyms_keep_source_identity | Homonymes distincts, doublon d'identité refusé |
| source_id_never_becomes_provider_id | Aucun ID fournisseur construit depuis la clé de métadonnées |
| series_season_episode_remain_distinct | Parents distincts, saison non lançable, parent absent refusé |
| channel_number_and_replay_identity | Numéro d'affichage séparé, portail/collection/diffusion non substitués |
| offer_unknown_is_not_entitlement | Abonnement/location distincts, droit inconnu conservé |
| expired_or_future_offer_is_not_current | Limites temporelles et fraîcheur respectées, inconnue conservée |
| intent_and_evidence_do_not_upgrade | Aucun repli d'intention ni confiance créée par un champ client |
| missing_source_differs_from_no_match | Indisponibilité, quota et absence de résultat distincts |
| boundaries_and_untrusted_links | Bornes de requête/page et références hostiles refusées |
| same_reference_two_readers | Même référence, décisions distinctes selon le lecteur fictif |

Les fixtures décrivent Netflix, Crunchyroll, Free TV/OQEE, Prime Video, Disney+,
Molotov et Apple TV ; elles ne contiennent aucun identifiant réel, compte ou
réponse de fournisseur. Leur inclusion est limitée au binaire de test.

Les contrats de futures pagination avec état, cache, Caller, annulation réseau,
quotas réseau et mémoire totale n'appartiennent pas à cette tranche pure.
Un lien Netflix `watch` réécrit en `title` n'est pas exercé ici : aucun résolveur
n'est importé et aucune conservation de son intention n'est déclarée prouvée.

## Contrôle du lot

La préparation conteneur du membre et du lock a précédé le premier passage
fonctionnel groupé. Celui-ci a exécuté les dix fonctions et observé dix échecs
d'assertion sur l'ancienne implémentation, avant les correctifs purs.

Depuis le worktree, la commande native est :
`RUSTUP_TOOLCHAIN=1.99.1 CARGO_TARGET_DIR="$PWD/target" cargo test -j 2 --locked -p moli-media-catalog`.
Le rapport de validation conserve la commande et le résumé des tests.
Une erreur de compilation ou de fixture n'est pas un ROUGE métier.
Les fonctions regroupent plusieurs assertions : un échec ne qualifie que les
assertions effectivement atteintes. La soumission en revue repose sur le format,
Clippy du workspace et les contrats ciblés verts. Le garde-fou global
`sh scripts/check.sh` reste distinct : il est exécuté une seule fois, au tour
de fusion après approbation, avec la toolchain et le target du worktree.

Les résultats détaillés et la comparaison ROUGE/VERT sont conservés dans les
preuves de validation du lot. Les chiffres de mémoire/latence du produit et la
couverture complète hors ligne d'un registre restent distincts et non mesurés ici.

Voir [le contrat de composant](../../docs/components/medias.md).
