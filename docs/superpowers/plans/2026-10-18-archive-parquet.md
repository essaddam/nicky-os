# Service d’archive Parquet — plan d’implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans. Exécution dans cette session, sans nouveau worker, puis revue du relecteur affecté.

**Goal:** Implémenter le service séparé de D13, local puis déplaçable, avec réception durable et publication Parquet vérifiable.

**Architecture:** Un seul travail de données à la fois, dans un volume propre au service. Un journal temporaire de lots et un index de reçus de taille fixe avec checksum assurent la reprise ; les fichiers Parquet et manifestes immuables assurent les lectures. Aucune seconde base relationnelle n’est nécessaire pour les seuls reçus.

**Tech Stack:** Rust, HTTP Axum, gzip, SHA-256, Parquet bas niveau sans Arrow, Snappy, calcul pur `moli-rollup`.

**Spec:** `docs/ARCHITECTURE.md`, D13 ; ticket C1 `e9f57b42-704d-4d6c-84a4-691c2bd40098`.

## Global Constraints

- Lots D13 : ≤ 5 000 opérations, ≤ 1 Mio gzip, ≤ 8 Mio décompressés ; SHA-256 des octets NDJSON originaux décompressés.
- `durable_upto`, `published_upto`, `generation` distincts ; époque stable hors restauration, révision de publication séparée.
- Un service par installation ; identité validée et conservée, aucun disque partagé avec le cœur.
- Un travail à la fois ; mesurer sous 64 puis 128 Mio ; image ≤ 25 Mo ; aucun relèvement implicite.
- Rust déclaré 1.88 ; pas de relèvement de MSRV ; image séparée `FROM scratch`.
- Aucun changement du cœur C2, branchement D1, sauvegarde D2, purge E1 ou déploiement réel.
- Tous les contrôles lourds sont soumis au créneau de l’orchestrateur. `sh scripts/check.sh` avant commit.

## Review Focus

- Erreur disque après une écriture partielle : arrêter les mutations jusqu’à réouverture et reconstruire les accusés depuis les seuls enregistrements validés.
- Catalogue tardif ou règle changée : préserver les reçus bruts, reconstruire les résumés et publier ensemble le catalogue et ses résultats.
- Même horodatage : conserver les événements distincts ; seule une opération énergie remplace la version précédente de la même heure.
- Import très ancien, fuseau et passage de mois : partitionner selon UTC ; regrouper les jours/mois selon le fuseau déclaré, avec couverture explicite.
- Lecteur ou future sauvegarde en cours : conserver les fichiers de son manifeste malgré publication et compactage.

## Fichiers et interfaces

- `crates/moli-archive/src/protocol.rs` : `Operation`, `Status`, `Catalog`, `Query`, validation et décodage borné.
- `receipt.rs` : index de reçus fixe, journal temporaire, fsync et récupération ; `receive(first,last,bytes)`.
- `parquet.rs` : lignes triées, écriture par petits groupes, lecteur en flux et vérification des fichiers.
- `manifest.rs` : `Manifest`, `Part`, `Snapshot`, publication atomique et protection des générations.
- `store.rs` : orchestration `Archive::open/receive/publish/query/set_catalog/compact`, sans réseau.
- `summaries.rs` : calcul horaire puis jour/mois avec A3, énergie résolue avant sommation.
- `http.rs`, `main.rs` : authentification avant lecture du corps, admission bornée, HTTP et CLI.
- `tests/durability.rs`, `tests/http.rs` : même sémantique sur stockage et transport, interruptions, corrections, concurrence et entrées invalides.
- `docs/components/archive-protocol.md` : contrat catalogue/lecture D1, avec bornes et couverture.
- `docker/archive.Dockerfile`, procédure de mesure et documentation d’exploitation.

### 1. Réception durable et contrat

- [ ] Écrire les tests : doublon exact : statut courant, contenu différent → conflit, trou → conflit, gzip trop gros, entier hors domaine, nombre non fini et type incohérent refusés.
- [ ] Exécuter les tests rouges ciblés sous Linux à un job après feu vert.
- [ ] Implémenter `Operation::validate` et `ReceiptLog::receive`. Index fixe de 64 octets contenant bornes, hash, date de publication et CRC de l’enregistrement ; journal fsync avant index fsync. Vérifier l’index contigu à l’ouverture ; tronquer uniquement sa dernière écriture incomplète.
- [ ] Vérifier les mêmes tests verts, plus réouverture après chaque frontière d’écriture.

### 2. Publication Parquet et corrections

- [ ] Écrire les tests : aucune ligne visible avant manifeste ; failpoints de publication ; manifeste corrompu/fichier absent ; correction énergie et coût tardif sans doublon ; événements simultanés conservés.
- [ ] Implémenter staging, fsync fichier et répertoire, vérification complète avant manifeste versionné et pointeur atomique. Le manifeste fait autorité pour `published_upto` ; le journal fait autorité pour `durable_upto`.
- [ ] Implémenter fusion triée en flux par famille/mois ; résoudre l’énergie par séquence maximale et conserver les reçus d’origine.
- [ ] Vérifier reprise et absence de perte/double compte à chaque interruption.

### 3. Résumés et lectures bornées

- [ ] Écrire les tests : catalogue absent/tardif, changement de règle, fraîcheur, durée, ancre inter-mois, DST, limite globale de sortie et borne de travail.
- [ ] Implémenter les résumés A3 heure/jour/mois et la réponse figée sur un `Snapshot`, avec qualité/couverture, origine et révision du catalogue.
- [ ] Vérifier que le compactage et la collecte respectent les générations épinglées, y compris lors d’une publication concurrente.

### 4. Transport, déplacement et budgets

- [ ] Écrire les tests HTTP authentifiés : même suite métier, mauvais jeton, mauvais encodage, taille et concurrence bornées ; le client n’a pas accès au volume du service.
- [ ] Implémenter le binaire avec bouclage par défaut, jeton par fichier secret et une seule tâche lourde ; documenter l’époque à changer lors d’une restauration, pas lors d’un déplacement.
- [ ] Exécuter dans deux conteneurs sans volume partagé ; mesurer mémoire 64 puis 128 Mio et taille réelle binaire/image. Refuser la clôture si un budget échoue.
- [ ] Mettre à jour historique/exploitation, exécuter `sh scripts/check.sh`, commiter en français puis soumettre au relecteur affecté, sans push ni intégration.

## Arbitrages du plan

- Revue architecte msg-0mvdtalrc-e16fe687 : extraction pure moli-rollup, aucun moteur SQL, un service par installation.
- Index : au plus sept jours de reçus publiés et 8 Mio ; reçus publiés évinçables plus tôt sous quota, non publiés jamais. Doublon : statut courant.
- Verrou distinct de l’index remplacé atomiquement ; CRC invalide final tronqué, corruption intérieure refusée. Lot sans reçu supprimé.
- Règle absente : famille other publiable, 30 jours par défaut B1, résumés inconnus explicites. Purge en E1.
