# AUTO-H5 — Planning hebdomadaire : plan d’implémentation

> **Exécution :** compétence `superpowers:executing-plans`, directement dans cette session. La revue finale appartient à Rust ; aucune revue intermédiaire n’est attendue.

**Objectif :** éditer des plannings hebdomadaires durables, dont seule la version explicitement validée par un humain alimente les graphes.

**Architecture :** identités stables et enveloppe générique brouillon/courant dans le domaine pur ; un propriétaire runtime écrit un fichier métier borné via `StateFile`. Un index des graphes concernés et un verrou commun de publication rendent la confirmation exacte malgré les éditions concurrentes. Le planning publie un état booléen en lecture seule, calculé dans son fuseau.

**Technologies :** Rust, Serde, Tokio, Jiff et Svelte ; aucune dépendance nouvelle.

**Spécification :** `docs/components/automatismes-primitives.md`, contrat rectifié et AUTO-H5. Le cadrage du premier lot est soumis avec son code au relecteur Rust.

## Contraintes

- Base `ecf16e9`, branche `auto-h5-planning`, candidat H3 immuable ; aucun merge ni push.
- Aucun LLM ni appareil réel. Les valeurs des autres aides restent sans version de définition.
- Brouillon initial et toute édition inertes ; publication avec session PIN existante, version attendue et liste des graphes concernés.
- Identités des dépendances couvertes par l’empreinte du graphe ; résolution de leur version validée courante, sans figer son numéro dans l’empreinte.
- Un fichier `dependencies.json`, schéma versionné, 128 définitions, 4 Mio ; 256 créneaux après découpage, au plus deux versions par définition.
- Jours lundi=1 à dimanche=7 ; minutes 0..1440, intervalles semi-ouverts ; traversées de minuit découpées, chevauchements rejetés.
- État durable écrit en temporaire, fsync, rename, fsync du répertoire avant acquittement. Issue incertaine : fermer mutations et projections jusqu’à reprise.
- Textes fr/en ; REST, MCP, CLI, éditeur, OpenAPI et sauvegarde cohérents.

## Points de revue

- Une simple origine UI sans session ne publie aucune définition.
- Un graphe ajouté entre aperçu et confirmation invalide la confirmation.
- Une heure sautée n’invente aucun instant ; les deux copies d’une heure répétée suivent le même prédicat civil.
- La reprise projette l’état actuel sans rejouer les transitions manquées ni un événement déjà consommé.
- Un demandeur parti après engagement n’annule pas l’écriture ; une erreur après rename ne permet aucun nouvel essai aveugle.

## 1. Contrat commun additif — commit distinct

Fichiers : `crates/moli-core/src/dependency.rs`, exports et tests de contrat.

- [ ] Tests rouges : édition concurrente, brouillon inerte, publication de la version exacte, version maximale, restauration invalide.
- [ ] Enveloppe `VersionedDefinition<T>` à deux têtes, `DefinitionVersion<T>`, préconditions de publication et identités typées réutilisables.
- [ ] Exécuter les contrats purs et Clippy ; commit séparé du planning.

## 2. Planning, propriétaire durable et projection

Fichiers : domaine `weekly.rs`, runtime `dependencies/`, point `Kind::Schedule`, `moli-helpers` et intégration du démarrage.

- [ ] Tests rouges : dimanche/minuit, chevauchements, limite après découpage, DST, redémarrage et coupures.
- [ ] Normalisation pure ; validation de fuseau et recherche du prochain changement incluant les transitions de fuseau dans Jiff.
- [ ] Propriétaire global paresseux, chargement validé, éditions CAS, publication durable et journalisée ; un ordonnanceur global borné.
- [ ] Projection sans cache périodique ni commande directe ; le brouillon ne publie aucun état.

## 3. Dépendances des graphes et confirmation humaine

Fichiers : `moli-automation` (index et vérification), `moli-runtime` (verrou de définition et instantané des consommateurs).

- [ ] Tests rouges : brouillon sans effet, graphe sans version courante inexécutable, mutation concurrente des consommateurs.
- [ ] Index par identité ; graphe validé utilisant le courant ; verrou partagé pour save/approve/restore/delete et publication.
- [ ] Aperçu signé par empreinte des versions et consommateurs ; tout changement pertinent exige un nouvel aperçu.

## 4. Surfaces et vérification finale

Fichiers : API dépendances/MCP/OpenAPI, CLI, sauvegarde, éditeur web, locales, documentation automatismes.

- [ ] Contrats API pour brouillon/CAS/aperçu/publication PIN et refus invité ; CLI et MCP ne contournent aucune validation.
- [ ] Éditeur avec fuseau, créneaux, versions, conflits et graphes concernés avant confirmation ; tests de conversion des horaires.
- [ ] fmt, Clippy workspace strict, suites Rust ciblées (-j2), npm ci/test/build, i18n, catalogue et fuites.
- [ ] Commit final et soumission à Rust avec preuves ; contrôle complet au tour d’intégration selon la file existante.
