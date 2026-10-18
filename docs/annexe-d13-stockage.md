# Annexe D13 : les chiffres du stockage sur Turso

Preuves de [l'ADR D13](ARCHITECTURE.md#d13--stockage--le-récent-sur-turso-le-long-terme-dans-un-service-darchive-17-octobre-2026).
Aucune donnée de production : le banc rejoue la **forme** de la charge ×2 de l'étude de
dimensionnement (nombre de relevés par point anonymisé et par heure sur sept jours, deux
répliques, 114 compteurs, bornes de calendrier) avec des valeurs fictives. Le fichier de forme
reste hors du dépôt ; il est monté en lecture seule.

## Rejouer

```
MOLI_IMAGE=1 sh tools/storage-bench/run.sh <chemin>/dashboard-workload-shape.json 3
```

Docker seul ; `results/<date>/summary.txt` rassemble ce qui suit. Détail des étapes :
[`tools/storage-bench/README.md`](../tools/storage-bench/README.md).

**Environnement de la série citée** : Turso 0.8.3, Rust 1.99.1 (image `rust:1-alpine`),
binaires statiques musl linux/arm64 au profil de release de Moli ; essais dans l'image
`FROM scratch` du banc, profil seccomp par défaut, `--cpus 1 --memory 128m`, base sur un
volume Docker ext4 ; Docker 29.2.1 dans une VM colima (Linux 6.8, 4 vCPU, 8 Gio) sur un
portable Apple M4. D'autres conteneurs tournaient dans la même VM : sa charge au début de
chaque tour est notée plus bas. Trois tours ; « min / médiane / max » porte sur ces tours.

## Essais fonctionnels

Série `20261018T080709Z`, à la version figée ; 15 essais de compatibilité,
aucun échec. Les deux observations MVCC et SQL libre sont informatives : elles
ne valident ni MVCC en production ni la sûreté du SQL libre.

| Besoin | Preuve exécutée et résultat |
|---|---|
| DML sur tables rowid et index | `UPDATE`, `DELETE`, UPSERT additif, `RETURNING`, `CHECK`, `STRICT`, clé calculée et doublon d'instant : conformes |
| Équivalence avec SQLite | 20 000 opérations, 400 transactions dont 57 annulées ; 3 157 lignes comparées, aucune divergence, `integrity_check` correct des deux côtés |
| Transactions | validation, annulation, `SAVEPOINT` ; une instruction rejetée ne valide pas la transaction |
| Dossier temporaire | sans dossier, `BEGIN IMMEDIATE` échoue ; `temp_store = MEMORY` le permet, mais `VACUUM INTO` exige toujours `TURSO_TMPDIR` sur le volume |
| Lecteurs concurrents | les 2 000 lignes non validées restent invisibles ; instantané conservé jusqu'à la fin de la transaction du lecteur |
| Durabilité | `synchronous = FULL` par défaut, WAL et checkpoint `TRUNCATE` ; 213 fsync pour 200 validations en `FULL`, 13 en `NORMAL` et `OFF` (création comprise) |
| Fichiers actuels `WITHOUT ROWID` | écritures refusées et plages de clé mal lues ; fichiers intacts, copie de migration obligatoire |
| Compatibilité de fichier | fichier Turso lu, modifié et vérifié par SQLite, puis relu et réécrit par Turso : conforme |
| Bascule et retour arrière | 100 200 opérations copiées, outbox et totaux compris, en 5,22 s ; ajout inverse en 0,107 s ; historique et énergie identiques |
| Séquence persistante | rowid ordinaire réutilisé après suppression ; le compteur transactionnel donne bien `1, 2, 3` |
| SQL libre des agents | autorisateur absent ; connexion en lecture seule acceptant PRAGMA, transaction, plusieurs instructions et allocation de 10 Mo : outil structuré requis |
| Second processus | ouverture en écriture refusée, lecteur Turso en lecture seule à 100 lignes ; SQLite concurrent obtient `database is locked` |
| Sauvegarde | `VACUUM INTO` recopie 2 000 lignes, copie vérifiée intègre par SQLite |
| Backend et seccomp Docker | `io_uring_setup` refusé (`EPERM`) ; 1 000 écritures réussies avec le backend par défaut, dans `scratch` |
| Rust minimum annoncé | `cargo check --release --locked` du banc réussi en 1.88 |

**31 `kill -9`** pendant des transactions multi-tables et des checkpoints :
aucun invariant violé. Au dernier arrêt, 64 976 opérations avaient été annoncées
validées, 65 004 sont relues (une validation peut précéder son annonce), dont
52 004 échantillons et 13 000 opérations d'énergie. L'outbox est contiguë ;
échantillons, heures, totaux et état des compteurs restent cohérents. SQLite
récupère d'abord le dernier fichier, puis Turso le rouvre correctement.
Réouverture après crash : p95 1,42 ms, maximum 1,45 ms sur 30 reprises.

## Tailles (linux/arm64, musl, octets)

| Sonde | Symboles conservés (`strip=debuginfo`) | Strip complet | Strip complet + gzip -9 |
|---|---:|---:|---:|
| Sans moteur | 530 384 | 397 480 | 188 538 |
| SQLite | 1 876 200 | 1 526 792 | 796 386 |
| Turso | 13 249 464 | 10 373 824 | 4 822 656 |
| SQLite + Turso | 14 514 720 | 11 437 584 | 5 418 214 |
| `turso_core` minimal | 12 178 200 | 9 706 088 | 4 580 213 |
| Moli actuel | 23 053 336 | 17 723 296 | 7 989 300 |

Turso à la place de SQLite ajoute **11,37 Mo** ; les deux moteurs ajoutent
**12,64 Mo**. Projection additive depuis Moli actuel : **34,43 Mo** sur Turso,
**35,69 Mo** pendant le retour arrière (deux moteurs). Ce sont des projections,
pas une image de Moli déjà basculée ; C1 doit mesurer son image réelle. Le budget
de **40 Mo**, symboles conservés, est celui validé par le propriétaire.
`docker image inspect` rapporte 6 535 654 octets pour l'image `scratch` du
**banc** : cette taille ne doit pas être confondue avec l'image future du cœur.

## Charge ×2

2 322 732 relevés sur 3 024 points, puis 127 452 heures pour 114 compteurs.
Transactions de 500 opérations, donnée + outbox + séquence, durables (`FULL`).
La durée d'une transaction comprend ses écritures et sa validation.

| Mesure | Min / médiane / max |
|---|---:|
| Chargement des relevés (s) | 141,80 / 145,69 / 161,20 |
| Débit de chargement (op/s) | 14409,05 / 15943,14 / 16379,82 |
| Transaction de 500 opérations, p95 (ms) | 64,73 / 72,57 / 82,77 |
| Fenêtre d’énergie chargée (s) | 5,70 / 5,97 / 6,05 |
| Pic RSS à la fin du chargement (Mio) | 18,55 / 18,55 / 18,55 |
| Fichier après checkpoint (Mo) | 151,74 / 151,74 / 151,74 |

Charge disponible (moyennes **1 / 5 / 15 minutes**, VM de 4 vCPU et hôte de mesure).
Les autres services de la VM restaient présents. Les relevés ne décrivent pas
une machine isolée ; les échantillons intermédiaires disponibles de l'hôte
restent entre 3,26 et 4,96 sur une minute.

| Moment | VM Docker | Hôte |
|---|---|---|
| Début tour 1 | 1,78 / 1,02 / 0,61 | 4,96 / 4,27 / 4,19 |
| Début tour 2 | 1,31 / 1,26 / 0,79 | 3,93 / 4,20 / 4,18 |
| Début tour 3 | 1,72 / 1,56 / 1,00 | 3,88 / 4,11 / 4,14 |
| Fin | 2,52 / 2,03 / 1,29 | 3,48 / 3,97 / 4,08 |

Les passages précédents, interrompus ou fortement chargés, ne sont pas inclus
(dont une charge hôte passée de 9,83 à 32,02 avant l'interruption d'un tour).

## Lectures du tableau de bord (p95, ms : min / médiane / max sur les tours)

Le p95 porte sur 30 à 100 répétitions par tour (20 pour l'export), après une
première exécution séparée. Il inclut la sérialisation JSON, sans HTTP ni rendu
web. « Lignes » désigne les lignes parcourues, avant réduction en points.

| Parcours | P95 min / médiane / max (ms) | Lignes | JSON (octets) |
|---|---:|---:|---:|
| Fiche prise stable, 24 h | 0,16 / 0,17 / 0,28 | 0 | 158 |
| Fiche prise médiane, 24 h | 0,16 / 0,21 / 0,29 | 6 | 329 |
| Fiche prise active, 24 h | 1,52 / 1,65 / 1,81 | 2 991 | 24 513 |
| Fiche prise active, 7 j | 10,25 / 10,54 / 10,65 | 21 657 | 162 225 |
| Point le plus bavard, 24 h | 5,20 / 5,34 / 5,44 | 14 817 | 24 862 |
| Sécurité, requête sur 720 h | 5,79 / 5,80 / 5,99 | 43 211 | 160 197 |
| 20 petites courbes, 24 h | 6,44 / 6,75 / 6,98 | 7 754 | 44 203 |
| Énergie 24 h, heures | 4,58 / 4,68 / 4,80 | 2 736 | 121 778 |
| Énergie 7 j, heures | 31,11 / 32,31 / 33,99 | 19 152 | 764 132 |
| Énergie 30 j, recalcul depuis les heures | 31,79 / 32,17 / 35,62 | 80 940 | 148 201 |
| Énergie 30 j, totaux journaliers retenus | 7,10 / 9,34 / 10,13 | 3 420 | 148 201 |
| Énergie 12 mois, totaux mensuels retenus | 2,99 / 3,04 / 3,07 | 1 368 | 69 827 |
| Résumé, option deux mois d’heures écartée | 50,23 / 53,58 / 54,50 | 127 452 | 22 086 |
| Résumé, accumulateurs retenus | 0,47 / 0,52 / 0,92 | 456 | 22 086 |
| Export, lot de 5 000 opérations | 12,41 / 12,77 / 14,49 | 5 000 | 375 432 |

Égalité du résumé depuis les heures et depuis les totaux : **3/3**.
Pic RSS du processus de lectures : **52,09 / 52,62 / 52,77 Mio**. Ce processus
exécute aussi les deux comparatifs écartés (30 jours depuis les heures et
résumé depuis deux mois d'heures). Le banc relève le RSS, tandis que le budget
du cœur porte sur la mémoire anonyme du cgroup : C1 doit mesurer celle-ci
avec les chemins retenus et le reste du cœur, avant la bascule.

**Réserve de performance : énergie 7 jours.** Ce parcours reste à
31,11–33,99 ms au p95, au-delà des 25 ms visées, avant même HTTP. C1/C2 doivent
réduire ce coût et prouver le budget sur le cœur intégré et le matériel cible.
L'option deux mois d'heures est écartée au profit des accumulateurs (456 lignes
contre 127 452 pour le résumé), dont la réponse identique est vérifiée.

Référence **redb de l'étude existante**, conservée sans nouveau passage :
fiche active 24 h 0,31 ms, 7 j 1,97 ms, 20 petites courbes 0,87 ms ; énergie
24 h 0,65 ms, 7 j 3,66 ms, 30 j depuis les jours 1,70 ms, 12 mois depuis les
mois 1,18 ms. Les pipelines et le JSON diffèrent : cette comparaison situe le
coût de Turso et de SQL ; elle ne rouvre pas le choix du moteur.

## Écritures concurrentes, réouverture, purge

Cinq lecteurs (100 lectures de la fiche active 24 h chacun, pause de 50 ms),
un rédacteur durable ; une opération d'énergie sur dix met aussi à jour les
totaux du jour et du mois et l'état du compteur dans la transaction.

| Mesure (min / médiane / max) | 4 op/s visées | 974 op/s visées |
|---|---:|---:|
| Débit observé (op/s) | 3,96 / 3,99 / 3,99 | 955,82 / 958,58 / 965,61 |
| Lecture p95 (ms) | 7,18 / 14,34 / 14,46 | 8,54 / 10,78 / 12,19 |
| Lecture p99 (ms) | 8,74 / 16,70 / 35,56 | 14,89 / 16,34 / 16,95 |
| Transaction p95 (ms) | 4,43 / 6,15 / 45,07 | 20,78 / 25,68 / 34,85 |
| Pic RSS (Mio) | 17,92 / 18,20 / 20,00 | 20,55 / 20,63 / 21,38 |

La rafale atteint **98,1 à 99,1 %** du débit visé : elle reste proche de 974 op/s,
mais ce banc ne prouve pas que 974 op/s soient soutenues intégralement sous
cette limite CPU et avec ces lecteurs. C'est une réserve de capacité à
revérifier lors de l'intégration.

| Réouverture et purge (min / médiane / max) | Mesure |
|---|---:|
| Ouverture (ms) | 1,07 / 2,04 / 3,45 |
| Ouverture + première courbe (ms) | 3,11 / 4,23 / 5,46 |
| Vérification complète d’intégrité (s) | 1,05 / 1,14 / 1,61 |
| Purge des relevés (s) | 1,60 / 2,04 / 2,32 |
| Transaction par point, p95 (ms) | 0,96 / 1,13 / 1,86 |
| Transaction par point, maximum (ms) | 88,32 / 96,73 / 105,26 |
| Vidage de l’outbox (s) | 1,45 / 1,58 / 1,80 |

316 636 relevés purgés sur 3 024 points ; exactement une ancre conservée pour
chacun des 2 870 points ayant une valeur avant la limite, **3/3**. Intégrité
correcte après chaque réouverture. Le fichier reste vers 152 Mo après purge :
les pages libérées sont réutilisables ; la purge ne compacte pas le fichier.

## Mesures exploratoires (hors commande unique)

Faites pendant la conception sur la machine de développement (macOS, Apple Silicon, base
sur le disque interne), même binaire et même charge ×2, pour choisir entre deux formes ; à
comparer entre elles seulement.

| Lecture (p95, ms) | rowid ordinaire + index `(point_id, ts)` | rowid = `point_id · 2^44 + ts` |
|---|---|---|
| fiche prise 24 h, point actif (2 991 lignes) | 12,46 | 2,66 |
| fiche prise 7 jours, point actif (21 657 lignes) | 108,99 | 18,07 |
| point le plus bavard 24 h (14 817 lignes) | 26,05 | 9,46 |
| 20 petites courbes | 35,05 | 11,68 |

L'exécuteur des futures Turso compte aussi : céder la main au système à chaque entrée /
sortie en attente (`yield`) coûtait trois fois plus que repolluer aussitôt (fiche 7 jours :
307 → 86 ms au p50 avant le changement de clé). Le banc repolle aussitôt, comme tokio après un
réveil, et ne cède la main qu'après 64 attentes de suite.

## Ce qui n'est pas mesuré

- Le matériel cible (D10) : la VM de mesure tourne sur un hôte ARM de bureau. Une première
  mesure sur Pi est un critère d'invalidation de l'ADR, pas une formalité.
- Le budget complet du cœur (32 Mio anonymes, API à 25 ms) et son image après
  intégration : le banc mesure le stockage et son RSS. Les réserves chiffrées
  ci-dessus doivent être levées en C1/C2 avant la bascule.
- Sécurité sur 30 jours pleins : la requête porte bien sur 720 h, mais le jeu
  de forme ne contient que sept jours de relevés ; sa rétention est fixée à
  31 jours dans l'ADR, son coût sur 30 jours reste à mesurer.
- La coupure de courant : `kill -9` tue le processus, pas la machine ; la durabilité tient aux
  fsync comptés plus haut.
- Le service d'archive (C3) : ni HTTP, ni Parquet ici ; seul le coût côté cœur d'un lot
  d'export est mesuré.
