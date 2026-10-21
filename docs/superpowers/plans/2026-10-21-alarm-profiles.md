# Codes et profils d’alarme — plan d’implémentation

> Exécution directe par l’auteur avec superpowers:executing-plans ; tests d’abord, sans délégation supplémentaire.

**Objectif :** livrer le service privé de profils et de vérification des codes du lot 3/9, réutilisable par le pilote 4/9 et les surfaces 7/9.

**Architecture :** `moli-alarm` reste pur. Une crate `moli-alarm-access` porte profils, HMAC, coffre et verrous ; `Household` fournit les personnes et la suppression synchronisée de leurs profils. Aucun endpoint, pilote actif ou effet physique nouveau.

**Pile :** Rust, ring HMAC-SHA256, serde, Tokio, coffre existant du runtime.

**Spécification :** `docs/components/alarme.md`, § 6/12/20, validée ; disponible sur `integ-alarme` avant son intégration documentaire. Ticket `c6547d42`.

## Contraintes

- Code : 4 à 8 chiffres ; aucun Serialize/Debug du contenu, aucune valeur réelle de code utilisée par l’auteur.
- `alarm/code_key` et les HMAC restent au coffre ; profils publics sans code ni empreinte, fichier privé 0600 et écriture atomique.
- Propriétaire en session humaine pour toute gestion, contrôle réévalué après attente. Les routes appartiennent au lot 7/9.
- Désarmer toujours avec code, PIN ou non ; le moteur pur décide des commandes sans code et des sources autorisées.
- 5 refus en 10 minutes : verrou 15 minutes, doublement jusqu’à 24 heures, par source ; état dans `alarm.json`, réponse publique uniforme.
- Aucun effet matériel avant H1. Tests natifs Rust 1.99.1/-j2 ; complet seulement à la fusion après revue.

## Fichiers et frontières

- `crates/moli-alarm-access/src/model.rs` : métadonnées, code opaque emprunté, droits, source et limites.
- `crypto.rs` : clé et HMAC privés, vérification constante par ring et unicité.
- `lockouts.rs` : refus/fenêtres/escalade, horloge fournie, capacités bornées.
- `storage.rs` : JSON privé borné, temporaire/fsync/rename/fsync répertoire ; erreur après renommage fermée.
- `lib.rs`, `service.rs`, `authorization.rs` : coffre, transactions, gestion, reçus liés à la demande interne et adaptateur `moli_alarm::Profiles` avec revalidation vivante.
- `crates/moli-api/src/alarm_access.rs`, `people.rs` : annuaire humain/invité et garde de suppression ; pas de routes.
- Tests séparés : modèles/crypto, service et redémarrage, politiques du moteur, liaison réelle People.

## Points de revue et tests

- [x] Transactions coffre/profils interrompues : un HMAC orphelin ne donne aucun droit ; incertitude après renommage ferme le service.
- [x] Deux gestions concurrentes avec le même code : un seul profil admis ; code refusé sans echo.
- [x] Révocation et retrait de personne : les reçus déjà préparés deviennent invalides ; aucune création ne se glisse entre révocation et suppression.
- [x] Horloge reculée, addition saturée, capacité de sources : aucun contournement du verrou ni éviction d’une source verrouillée.
- [x] État existant corrompu ou clé absente : refus, jamais de régénération silencieuse ni remise à zéro.

## Séquence

1. [x] Baseline du moteur pur ; tests rouges des codes, droits et verrous, puis implémentation minimale.
2. [x] Tests rouges des fichiers/coffre, redémarrages et concurrence ; service transactionnel, fichiers privés et reçus vivants.
3. [x] Tests rouges des personnes/propriétaires/suppression ; adaptateur Household et crochet atomique de retrait.
4. [x] Tests de politique via le moteur pur : armer sans code, code fourni obligatoire à vérifier, changement/désarmement toujours codés ; preuve d’absence de code dans toutes les projections.
5. [x] Fmt/Clippy workspace, ciblage des crates touchées, documentation et soumission à la revue Rust ; aucun complet anticipé.

## Éléments de vérification

Les tests normaux et dégradés vivent dans `tests/service.rs`, `tests/degraded.rs`,
`tests/concurrency.rs` et `tests/engine.rs`. Les tests de liaison réelle sont dans
`moli-api/src/alarm_access.rs` ; la réservation du namespace est testée dans
`moli-os/src/config.rs`.

La fermeture sur erreur d’écriture est provoquée par un chemin devenu répertoire ;
la même branche traite les erreurs de synchronisation après renommage. Cette dernière
panne n’est pas injectée séparément. La reprise du moteur et ses sauvegardes appartiennent
au lot 4/9 ; les routes et le branchement au journal sont le lot 7/9.

Validation avant soumission : fmt et Clippy workspace stricts ; 189 tests ciblés
(`moli-alarm-access`, `moli-api`, `moli-os`) sans exclusion ; i18n et fuites propres.
Les 22 tests du moteur pur étaient verts avant le travail ; ses sources sont inchangées.
Le contrôle complet et la fusion restent soumis à l’approbation indépendante.

## Correction de revue ALARM3-R1-001

- L’affichage n’appelle plus le validateur d’un nouveau brouillon. Il projette une
  intersection devenue vide sur `[t, t)` sans modifier les droits enregistrés ni
  bloquer la liste des profils, y compris les profils révoqués.
- La publication de l’état durable et la levée de la barrière transitoire précèdent
  la lecture du séjour pour l’affichage. Les vraies erreurs de stockage ferment
  toujours le service.
- `tests/stay_changes.rs` couvre le témoin stable, le séjour disjoint, le profil
  révoqué, le déplacement entre les lectures pendant publication et le passage
  membre → invité sans fin définie. Les nouveaux brouillons invalides restent refusés.
- Régression reproduite avant correction : un témoin vert et trois échecs. Après
  correction : 163 tests ciblés accès/API sans filtre, fmt et Clippy workspace verts,
  traductions et fuites propres. Les scénarios originaux de la sonde Rust restent
  inchangés et passent en contre-épreuve auteur ; leur avis indépendant reste requis.
