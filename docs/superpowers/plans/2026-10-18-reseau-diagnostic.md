# N0 : diagnostic explicite et unités de trafic — plan d'implémentation

> Exécution dans la session existante, sans sous-agent ni nouveau worker.
> Skill d'exécution : `superpowers:executing-plans`, étape par étape après relecture du
> présent plan par le relecteur architecte désigné. Le contrat est déjà approuvé côté
> architecture ; ce document n'ajoute aucun arbitrage du propriétaire.

**But :** permettre aux futurs clients réseau de déclarer leur diagnostic technique,
avec des unités de trafic explicites, sans reclasser les points existants.

**Architecture :** ajouter `Semantic::Diagnostic` (serde `diagnostic`) dans le domaine,
le reconnaître explicitement avant l'unité dans B1, puis autoriser `B`, `B/s`, `Mbit/s`
dans les profils en gardant `Unit::Other`. Aucun pilote n'utilise encore cette extension.

**Pile :** Rust, serde JSON/TOML, tests du domaine/profils et stockage SQLite existant.
Aucune dépendance ajoutée. Base publiée : `34b03b2338bb64a8f84261b778a2ec3a2e3f62e3`.
Branche : `integ-reseau-diagnostic`, dans un worktree natif distinct du parent Réseau.

**Contrat :** [réseau, N0](../../components/reseau.md#n0--diagnostic-et-unités-avant-les-clients),
[matrice de qualification](../../components/reseau-qualification.md), ticket N0
`10e20827-53e2-4737-890f-a5992045b55e` et sa précision architecte.

## Contraintes globales

- `diagnostic` décrit la santé technique de l'équipement lui-même ; une mesure ambiante
  garde sa sémantique physique, même produite par un équipement réseau.
- Aucun changement à `Semantic::infer`, aux pilotes existants, à `battery_voltage`, aux
  durées, filtres, planchers, consentements, purges ou schémas persistants.
- Aucun endpoint, accès matériel, scan, migration, commande, secret ou changement de présence.
- Les unités sont des identifiants, pas des textes à traduire ; pas de nouveau texte visible.
- Sources et tests ne contiennent que des données synthétiques neutres.
- Pas de compilation, test lourd ou build sans créneau explicite de l'orchestrateur.
  Linux, image `moli-os-dev`, 1 CPU, 1 job, tests séquentiels et cache sans concurrence.
- Commit public français sur la branche du lot, aucune fusion, publication ni déploiement
  par l'auteur. Soumission indépendante au relecteur ; le parent et les autres lots restent ouverts.

## Audit des consommateurs et points de revue

1. Sérialisation : `Semantic` utilise serde `snake_case`, les `PointSpec` circulent dans
   `Device`/`DeviceView` sans conversion spécifique. Prouver le nouveau mot et tous les
   anciens mots avec des fixtures JSON en mémoire ; ne changer aucun ancien identifiant.
2. Classement : le seul match exhaustif de `Semantic` est `policy::family` ; les matches
   de `moli-demo/src/sim.rs` ont des replis et ne nécessitent aucun ajout. Prouver que
   `diagnostic` avec `°C` reste diagnostic et qu'une vraie température reste climat.
3. Conservation : une reclassification vers une famille plus courte ne doit abaisser
   aucun plancher existant, y compris après réouverture. Tester avec la vraie `Policy`.
4. Profils : autoriser seulement les trois nouveaux symboles exacts, vérifier valeur et
   unité au décodage ; les fautes et sémantiques inconnues restent refusées.
5. API/web : REST expose la sérialisation du domaine ; le MCP compact conserve ses
   champs actuels sans ajouter de sémantique. Le web utilise un repli générique pour une
   sémantique inconnue et affiche l'unité telle quelle. Aucun changement API/web prévu.
   Les tests serde du point couvrent le contrat transporté ; la suite complète couvre
   les routes existantes. Aucun libellé fr/en à créer en l'absence de texte nouveau.

## Lot unique : vocabulaire et contrat de conservation

**Fichiers de production à modifier :**

- `crates/moli-core/src/spec.rs` : enum `Semantic`, commentaire du nouveau cas.
- `crates/moli-history/src/policy.rs` : `family(semantic, unit, kind, key) -> Family`.
- `crates/moli-profile/src/profile.rs` : liste `EXTRA_UNITS` seulement.

**Tests, dans les modules existants :** les trois fichiers précédents et
`crates/moli-history/src/retention.rs` (test seulement, aucun code de garde modifié).

**Docs :** `docs/components/coeur.md`, `profils.md`, `historique-energie.md` et
`reseau.md` pour décrire l'extension effective et sa règle d'usage ; pas de nouvelle
entrée de catalogue ni de paquet d'intégration factice.

**Interfaces préservées :** signatures de `family`, `Family::of`, `Semantic::infer`,
`Unit::parse`, `StorageConfig::compile`, `Profile::parse`/`compile` et `CompiledPoint::read`.
La seule extension du contrat sérialisé est la valeur explicite `diagnostic`.

- [ ] **1. Écrire les tests avant le code de production.**

  Tous les nouveaux tests portent le préfixe `n0_` pour un contrôle ciblé de même graphe
  Cargo que le workspace. Ils utilisent `serde_json::from_str::<Semantic>("\"diagnostic\"")`
  dans les assertions, ce qui permet de compiler la phase rouge contre l'enum actuelle.

  - Cœur, `n0_diagnostic_round_trips_in_a_point_spec` : désérialiser un `PointSpec`
    numérique diagnostic en `B/s`, accès lecture seule ; JSON restitué identique, unité
    `Unit::Other("B/s")`. Refaire avec `B` et `Mbit/s` ; `diagnostics` reste refusé.
  - Cœur, `n0_existing_semantics_units_and_inference_are_unchanged` : table de toutes
    les anciennes variantes et leur mot serde, aller-retour JSON ; `temperature`/`°C`
    garde `Temperature`, `ram_usage`/`%` et `rx_bytes`/`B` gardent `Other` par inférence.
    Les unités nouvelles restent `Other`, jamais `Energy`, `WattHour` ni `KiloWattHour`.
  - B1, `n0_diagnostics_win_over_units_without_changing_policy_precedence` : catégorie
    diagnostic explicite avec `B`, `B/s`, `Mbit/s`, `%`, `s`, `°C`, sans unité, et type
    texte/binaire/enum ; `Family::of` reste diagnostic. Configuration diagnostic 7 jours,
    climat 90, défaut 30 et exception du point 60 : durées attendues 7/90/30/60 ;
    `include` continue de gagner sur `exclude`. Batterie/tension de pile et température
    ambiante gardent leur classement existant.
  - Profils, `n0_network_units_remain_other_without_an_explicit_semantic` : profil JSON
    synthétique de lecture seule pour chaque symbole `B`, `B/s`, `Mbit/s` ; compilation
    et lecture de `123.5` exactes, unité conservée et sémantique inférée `Other`.
  - Profils, `n0_diagnostic_profiles_decode_without_writes` : même profil avec catégorie
    explicite diagnostic, pas d'écriture, accès `write = false`, valeur `123.5` et `null`
    correctement décodées. `Mb/s`, `B/sec`, `byte`, `diagnostics` restent refusés.
  - Conservation, `n0_reclassification_keeps_its_persisted_floor_after_reopen` : avec le
    `Bench` existant, enregistrer un point technique d'abord sans catégorie (plancher
    30 jours), publier son `Device` diagnostic en `B/s` et conserver sa famille via
    `Family::of`. Configuration diagnostic 7 jours ; une passe ne supprime rien avant
    validation, plan proposé 30 → 7, refus des origines non humaines. Réouvrir la base,
    vérifier le plancher de 30 et la garde, puis validation UI du plan concret et coupe
    exacte annoncée. Aucune méthode de production ni hook d'ordonnancement ajouté.

- [ ] **2. Observer la phase rouge dans le créneau alloué.**

  Commande Linux : `cargo test --workspace --locked --no-fail-fast n0_ -- --test-threads=1`.
  Attendu : tests de compatibilité ancienne verts ; tests nouveaux en échec sur le mot
  `diagnostic` et/ou les unités inconnues. Pas d'erreur de compilation ni panne d'outil.
  Conserver journal/code, base, diff et empreintes ; aucune modification pendant ce passage.

- [ ] **3. Implémenter le minimum.**

  Ajouter `Semantic::Diagnostic` avec sa règle d'usage ; ajouter cette variante à la
  branche diagnostic de `family`, avant toute inférence par unité. Ajouter les trois
  symboles exacts à `EXTRA_UNITS`. Aucun autre classement, inférence ou pilote modifié.

- [ ] **4. Mettre à jour les quatre docs du composant.**

  Documenter catégorie explicite, unités acceptées et préservation des mesures ambiantes,
  des politiques et de la garde. Le tableau des familles décrit désormais le diagnostic
  technique explicite. Les clients réseau restent « à implémenter ».

- [ ] **5. Vérifier la phase verte et le garde-fou final, dans le créneau alloué.**

  Rejouer exactement la commande ciblée de l'étape 2 : tous les `n0_` passent.
  Puis `sh scripts/check.sh` : format, Clippy strict, workspace entier, catalogue et fuites
  verts sur les sources définitives. Aucun changement web prévu ; si l'audit ou la revue
  impose finalement un changement web, demander l'ajustement du périmètre de validation
  et exécuter `cd web && npm ci && npm run build`, puis `node scripts/i18n-check.mjs`.
  Pas de changement de code après le début du garde-fou. Preuves dans un dossier neuf
  hors dépôt : commande/image/base/diff, empreintes avant/après, journaux et codes.

- [ ] **6. Commiter les octets vérifiés et soumettre N0 au relecteur.**

  Comparer les blobs du commit aux empreintes contrôlées, vérifier le worktree propre,
  puis `backlog_submit_for_review` sur N0. Parent et N1–N4 restent ouverts ; aucun
  `backlog_review`, push, merge, déploiement ou changement d'appareil par l'auteur.

## Relecture du plan

Couverture vérifiée contre le contrat N0 : classification, unités, compatibilité des
anciens mots/configurations, filtres/priorités, garde persistante, affichage et transport.
Le plan ne demande aucune modification de schéma ou des règles de garde. Exécution
native dans cette session après approbation du relecteur, sans délégation d'implémentation.
