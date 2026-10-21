# AUTO-K1 — Points calculés : plan d’implémentation

> **Exécution :** `superpowers:executing-plans`, directement dans la session ; revue finale Database. Les corrections de revue AUTO-H5 passent avant ce lot.

**Objectif :** publier min, max et moyenne de sources numériques fraîches, avec des définitions déclaratives validées et une propagation déterministe bornée.

**Architecture :** étendre les définitions typées et le propriétaire `dependencies.json` d’AUTO-H5. Réutiliser sans duplication `VersionedDefinition<T>`, éditions CAS, aperçu des consommateurs, verrou de publication et session PIN. Un index de calculs et un seul ordonnanceur de calcul suivent les sources et leurs expirations ; aucun calcul dans un agent.

**Technologies :** Rust, Serde, Tokio et Svelte, sans nouvelle dépendance.

**Spécification :** `docs/components/automatismes-primitives.md`, AUTO-K1 et contrats rectifiés ; `docs/components/plannings.md` pour le socle H5.

## Contraintes et décisions

- Branche du ticket `auto-c1-agregats`, base H5 corrigée `5406d8f`, worktree séparé. Le code canonique est AUTO-K1. Aucun merge, push, contrôle complet ni appareil réel.
- Une définition `AggregateSpec` déclare opération `min|max|mean`, sources triées sans doublons, unité de sortie et fraîcheur en secondes (1..604800).
- 32 sources par calcul ; 128 définitions au total dans le propriétaire partagé, 4 Mio de fichier, profondeur de calcul 16 au plus. Chaque propagation visite au plus 128 calculs et 8192 arêtes/lectures.
- Ne considérer que des points numériques lisibles, des échantillons finis, non futurs, strictement antérieurs à leur expiration et provenant de sources en ligne. Vérifier les unités à la publication puis à chaque lecture ; les conversions connues énergie, tension et durée utilisent des rapports explicites, les autres exigent l’égalité.
- Ensemble vide : `null`, jamais zéro. Moyenne à échelle bornée pour ne pas déborder avec des valeurs finies extrêmes. Horodatage de sortie : plus ancien des échantillons valides utilisés, sans renouveler la date des sources.
- Valeurs dérivées hors cache métier : recomputation à partir des échantillons du hub, dont les horodatages restaurés restent inchangés. Les observations vivantes gardent leur âge monotone ; un saut d’horloge ne renouvelle ni ne ressuscite une observation. Les versions seules sont durables dans le fichier existant.
- Ordre topologique stable ; cycles directs/indirects et profondeur excessive refusés avant activation. Le brouillon reste inerte, même s’il propose une nouvelle topologie.
- L’aperçu lie aussi les versions courantes des sources exécutables et les calculs/graphes concernés en aval. Une modification de ce contexte exige un nouvel aperçu.
- Signal interne ciblé pour tout rafraîchissement de source, même si sa valeur n’a pas changé ; ne pas fabriquer un événement de changement d’état. Traiter aussi disponibilité et changement/disparition de description.
- Coalescer les états dans une file bornée de calculs sales ; calculer une propagation depuis une vue cohérente, puis publier les valeurs modifiées. Aucun balayage global à chaque événement ordinaire et aucune tâche par source.
- Expiration sans événement avec l’ordonnanceur de calcul ; recul/avance d’horloge provoquent une relecture bornée. Une brique sans calcul courant n’entretient aucune minuterie de calcul.

## Points de revue

- Retour d’une source après expiration avec exactement la même valeur : elle redevient utilisable.
- Source restaurée ancienne : la lecture du cache ne la rajeunit pas.
- Publication changeant une unité ou fermant un cycle : aucun courant invalide ne devient visible.
- Changement d’une source calculée entre aperçu et confirmation : conflit, même si le graphe conserve son identité.
- Propagation en chaîne : pas de fraîcheur inventée par un calcul intermédiaire ni de doublon pour une valeur égale.

## 1. Domaine pur et contrats numériques

Fichiers : `moli-core/src/aggregate.rs`, exports, `Kind::Computed`, identité de dépendance et tests.

- [x] Tests rouges : filtrage des absences/nuls/non-finis/hors ligne/périmés/futurs, frontière exacte, moyenne extrême et conversions d’unités.
- [x] Définition normalisée bornée, résultats horodatés et fonction pure d’agrégation.
- [x] Graphe pur : ordre stable, cycles, profondeur et bornes ; tests fan-out/chaîne.

## 2. Réutilisation des versions et propriétaire de projection

Fichiers : `moli-runtime/src/dependencies/`, hub et nouveau module `computed`.

- [x] Étendre le type de définition et le schéma compatible H5 ; aucune seconde enveloppe de versions.
- [x] Prévalidation de la topologie et des unités avant publication durable ; aperçu des sources versionnées et consommateurs transitifs.
- [x] Initialisation des calculs depuis le courant restauré, projections `value` en lecture seule et métadonnées d’unité atomiques.
- [x] Index inverse, file sale bornée et propagation stable ; branchement sur rafraîchissements de sources et disponibilité.
- [x] Échéances de fraîcheur dans un ordonnanceur global indépendant des écritures ; tests de retour identique, disparition, expiration et restauration.

## 3. Tranche de configuration et surfaces

Fichiers : `moli-helpers`, REST/MCP/CLI, éditeur web, locales fr/en et documentation.

- [x] Configuration `kind=computed` initialisant toujours un brouillon ; autres aides compatibles.
- [x] Routes communes de dépendances avec identité `computed`, MCP et CLI compatibles H5 ; aucune validation déclarative de confiance.
- [x] Éditeur utile des sources, opération, unité et fraîcheur, affichant courant/brouillon et portée avant PIN ; réutilisation des appels de versions et contrôle d’aperçu affiché.
- [x] Documentation automatismes et schéma OpenAPI à jour ; comportement de reprise et limites explicites.

## 4. Vérification et livraison

- [x] fmt, Clippy workspace/all-targets stricts ; tests Rust ciblés -j2, H5 inclus. Historique intégral avec TMPDIR isolé ; tout nouvel échec instable relève d’un ticket distinct.
- [x] npm ci/test/build, i18n, catalogue et fuites ; mesure simulée des bornes sans extrapolation physique.
- [ ] Sources figées, empreintes et journaux des phases ; commit en français sur la branche du ticket puis soumission à Database. Contrôle complet au tour d’intégration après approbation.
