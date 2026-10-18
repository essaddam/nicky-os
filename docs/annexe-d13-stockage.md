# Annexe D13 : les chiffres du stockage sur Turso

Preuves de [l'ADR D13](ARCHITECTURE.md#d13--stockage--le-récent-sur-turso-le-long-terme-dans-un-service-darchive-17-octobre-2026).
Aucune donnée de production : le banc rejoue la **forme** de la charge ×2 de l'étude de
dimensionnement (nombre de relevés par point anonymisé et par heure sur sept jours, deux
répliques, 114 compteurs, bornes de calendrier) avec des valeurs fictives. Le fichier de forme
reste hors du dépôt ; il est monté en lecture seule.

**Série retenue : `20261018T100333Z`, avec journal immuable et reprise bornée.**

## Rejouer

```
MOLI_IMAGE=1 sh tools/storage-bench/run.sh <chemin>/dashboard-workload-shape.json 3
```

Docker et Python 3 (bibliothèque standard) ; `results/<date>/summary.txt` rassemble ce qui suit. Détail des étapes :
[`tools/storage-bench/README.md`](../tools/storage-bench/README.md).

**Environnement de la série citée** : Turso 0.8.3, Rust 1.99.1 (image `rust:1-alpine`),
binaires statiques musl linux/arm64 au profil de release de Moli ; essais dans l'image
`FROM scratch` du banc, profil seccomp par défaut, `--cpus 1 --memory 128m`, base sur un
volume Docker ext4 ; Docker 29.2.1 dans une VM colima (Linux 6.8, 4 vCPU, 8 Gio) sur un
portable Apple M4. D'autres conteneurs tournaient dans la même VM : sa charge au début de
chaque tour est notée plus bas. Trois tours ; « min / médiane / max » porte sur ces tours.

## Essais fonctionnels

Série `20261018T100333Z`, Turso 0.8.3 : **18 essais de compatibilité, aucun échec**.
Les observations MVCC et SQL libre sont informatives ; les règles de D13
restent un rédacteur, des lecteurs séparés et des outils structurés.

| Besoin | Preuve exécutée et résultat |
|---|---|
| DML rowid et index | `UPDATE`, `DELETE`, UPSERT additif, `RETURNING`, `CHECK`, `STRICT`, doublon d'instant et clé calculée : conformes |
| Équivalence SQLite | 20 000 opérations, 400 transactions dont 57 annulées ; 3 157 lignes comparées sans divergence ; fichiers intègres |
| Transactions | validation, annulation, `SAVEPOINT`, instruction rejetée sans validation de la transaction |
| Dossier temporaire | `temp_store = MEMORY` pour le rédacteur ; `TURSO_TMPDIR` sur le volume requis aussi pour `VACUUM INTO` |
| Lecteurs concurrents | 2 000 lignes non validées invisibles, instantané conservé jusqu'à la fin du lecteur |
| Durabilité | WAL, `FULL` par défaut et checkpoint `TRUNCATE` ; 214 / 14 / 14 fsync pour 200 validations en FULL / NORMAL / OFF, création comprise |
| Anciens fichiers `WITHOUT ROWID` | écritures refusées et plages mal lues ; migration par copie obligatoire |
| Fichier dans les deux sens | Turso lu et modifié par SQLite, puis relu et réécrit par Turso ; intégrité correcte |
| Migration et retour arrière | 100 200 opérations, séquences et journaux compris : 9,33 s ; retour inverse 0,13 s ; données identiques |
| Frontières de migration | 10 cas : 0, 1, 4 999, 5 000, 5 001 relevés, avec 0 ou 1 heure ; opérations, journal et séquence vérifiés |
| Séquence persistante | le rowid ordinaire se réutilise ; le compteur transactionnel donne `1, 2, 3` après purge et réouverture |
| SQL des agents | autorisateur absent ; les PRAGMA, transactions, instructions multiples et allocations non bornées justifient l'outil structuré |
| Second processus | écrivain refusé ; lecteur Turso possible ; SQLite concurrent obtient `database is locked` |
| Sauvegarde | `VACUUM INTO` : 2 000 lignes, copie vérifiée intègre par SQLite |
| Seccomp par défaut | `io_uring_setup` refusé, 1 000 écritures réussies avec le backend par défaut dans `scratch` |
| Réessai immuable | seq1 garde kWh=1 et coût inconnu après seq2/kWh=2 et réouverture ; octets du lot seq1 identiques |
| Restauration | seq1..3 rejouées à l'identique après vidage de l'outbox, purge des tables et restauration ; import ancien préservé ; nouvelle fenêtre après republication ; restauration expirée refusée |
| Rust minimum annoncé | `cargo check --release --locked` réussi en 1.88 |

**31 `kill -9`** pendant des transactions multi-tables et checkpoints : aucun
invariant violé. Dernier arrêt : 47 034 opérations annoncées validées,
47 034 relues ; journal immuable et outbox contiennent chacun cette
séquence contiguë. Heures, totaux et état des compteurs restent cohérents.
SQLite récupère d'abord le dernier fichier, puis Turso le rouvre correctement.
Réouverture après crash : p95 1,92 ms,
maximum 2,01 ms sur 30 reprises.

Le lanceur exécute aussi **5 tests Rust** et **8 tests Python**. Les régressions
ont été observées en échec avant correction. Un verdict obligatoire faux, un
échec MSRV, une intégrité incorrecte, un ancrage faux, une rétention incorrecte
ou une série incomplète produisent un code non nul ; le résumé est validé avant
publication et aucun tube vers `tee` ne masque son statut.
Un lancement sur `/proc`, volontairement non inscriptible, produit 18 verdicts
faux et le code **1** ; ce même défaut était masqué par le code 0 avant correction.

## Tailles (linux/arm64, musl, octets)

| Sonde | Symboles conservés (`strip=debuginfo`) | Strip complet | Strip complet + gzip -9 |
|---|---:|---:|---:|
| Sans moteur | 530 384 | 397 480 | 188 538 |
| SQLite | 1 876 200 | 1 526 792 | 796 386 |
| Turso | 13 249 464 | 10 373 824 | 4 822 656 |
| SQLite + Turso | 14 514 720 | 11 437 584 | 5 418 214 |
| `turso_core` minimal | 12 178 200 | 9 706 088 | 4 580 213 |
| Moli actuel | 23 053 336 | 17 723 296 | 7 989 296 |

Turso à la place de SQLite : **+11,37 Mo**.
Projection additive depuis Moli actuel : **34,43 Mo**,
**35,69 Mo** avec les deux moteurs pendant le retour arrière.
C1 doit mesurer son image réelle ; le budget propriétaire reste **40 Mo**,
symboles conservés. `docker image inspect` rapporte 6 557 108 octets pour
l'image `scratch` du **banc**, qui est distincte de l'image future du cœur.

## Charge ×2

2 322 732 relevés sur 3 024 points, puis 127 452 heures
pour 114 compteurs. Chaque opération écrit sa donnée, l'outbox et le contenu
immuable du journal dans la transaction durable (`FULL`). La durée de la
transaction inclut ses écritures et sa validation.

| Mesure | Min / médiane / max |
|---|---:|
| Chargement des relevés (s) | 213,38 / 245,40 / 251,49 |
| Débit de chargement (op/s) | 9235,72 / 9465,06 / 10885,19 |
| Transaction de 500 opérations, p95 (ms) | 90,82 / 110,69 / 112,31 |
| Fenêtre d’énergie chargée (s) | 9,26 / 10,06 / 10,49 |
| Pic RSS à la fin du chargement (Mio) | 19,23 / 19,23 / 19,29 |
| Fichier après checkpoint (Mo) | 465,14 / 465,14 / 465,14 |

**Coût du journal immuable** sur ce jeu : le fichier passe de 151,74 Mo
(ancienne série `20261018T080709Z`, sans ce journal) à 465,14 Mo,
soit **+313,40 Mo**. Le chargement précédent prenait
141,80–161,20 s ; cette série prend 213,38–251,49 s.
Ces octets supplémentaires rendent les réessais et la restauration possibles,
même après remplacement ou purge des données métier. Le journal reste sept
jours après publication ; il se purge ensuite par préfixes contigus.

Avant lancement : plus de cinq minutes de relevés hôte sous 6, sans contrôle
Moli concurrent. Charges (moyennes **1 / 5 / 15 minutes**) relevées aux bornes
ci-dessous ; les autres services de la VM restent présents.

| Moment | VM Docker | Hôte |
|---|---|---|
| tour 1 | 1,47 / 1,08 / 0,85 | 3,30 / 3,78 / 3,73 |
| tour 2 | 1,76 / 1,52 / 1,11 | 2,66 / 3,08 / 3,41 |
| tour 3 | 2,05 / 1,79 / 1,33 | 5,22 / 3,57 / 3,45 |
| fin | 1,60 / 1,63 / 1,38 | 2,94 / 3,64 / 3,56 |

Les séries antérieures interrompues ou surchargées sont exclues de la dispersion.

## Lectures du tableau de bord (p95, ms : min / médiane / max sur les tours)

P95 sur 30 à 100 répétitions par tour (20 par famille d'export), après une
première exécution séparée, production du JSON comprise ; aucun HTTP ni rendu.
Pour l'export, les contenus préencodés sont concaténés depuis le journal.
« Lignes » désigne les lignes parcourues, avant réduction en points.

| Parcours | P95 min / médiane / max (ms) | Lignes | JSON (octets) |
|---|---:|---:|---:|
| Fiche prise stable, 24 h | 0,15 / 0,19 / 0,28 | 0 | 158 |
| Fiche prise médiane, 24 h | 0,13 / 0,13 / 0,18 | 6 | 329 |
| Fiche prise active, 24 h | 1,51 / 1,88 / 2,08 | 2 991 | 24 513 |
| Fiche prise active, 7 j | 9,35 / 9,71 / 10,77 | 21 657 | 162 225 |
| Point le plus bavard, 24 h | 5,05 / 5,33 / 8,23 | 14 817 | 24 862 |
| Sécurité, requête sur 720 h | 5,22 / 5,67 / 5,76 | 43 211 | 160 197 |
| 20 petites courbes, 24 h | 6,78 / 6,84 / 6,91 | 7 754 | 44 203 |
| Énergie 24 h, heures | 4,37 / 4,92 / 4,95 | 2 736 | 121 778 |
| Énergie 7 j, heures | 30,37 / 31,31 / 31,83 | 19 152 | 764 132 |
| Énergie 30 j, recalcul depuis les heures | 28,23 / 29,93 / 30,83 | 80 940 | 148 201 |
| Énergie 30 j, totaux journaliers retenus | 6,38 / 6,73 / 7,12 | 3 420 | 148 201 |
| Énergie 12 mois, totaux mensuels retenus | 2,64 / 2,93 / 3,21 | 1 368 | 69 827 |
| Résumé, option deux mois d’heures écartée | 49,30 / 53,25 / 53,31 | 127 452 | 22 086 |
| Résumé, accumulateurs retenus | 0,48 / 0,49 / 0,52 | 456 | 22 086 |
| Export, 5 000 relevés | 3,97 / 4,76 / 5,85 | 5 000 | 575 432 |
| Export, 5 000 heures | 2,71 / 3,19 / 3,43 | 5 000 | 581 623 |
| Export, 2 500 relevés + 2 500 heures | 2,34 / 2,72 / 3,33 | 5 000 | 578 206 |

Égalité du résumé heures / totaux : **3/3**. Couverture de l'export : **3/3**
pour chacun des lots, respectivement 5 000 / 0, 0 / 5 000 et 2 500 / 2 500
relevés / heures. Les lots sont lus dans le journal immuable, dans l'ordre des
séquences, sans jointure aux tables métier.

Pic RSS du processus de comparaison : **53,21 / 53,33 / 54,32 Mio**, options écartées
comprises. Le budget du cœur porte sur **32 Mio anonymes du cgroup** : C1 doit
le mesurer sur les seuls chemins retenus et le cœur intégré avant bascule.

**Réserve de performance : énergie 7 jours**, à **30,37–31,83 ms** au p95,
avant HTTP. Le repère de 25 ms reste à prouver en C1/C2. Le résumé retenu lit
456 lignes à 0,48–0,52 ms, contre 127 452 heures pour l'option écartée.

Référence **redb de l'étude existante**, sans nouveau passage : fiche active
24 h 0,31 ms, 7 j 1,97 ms, 20 petites courbes 0,87 ms ; énergie 24 h 0,65 ms,
7 j 3,66 ms, 30 j depuis les jours 1,70 ms, 12 mois depuis les mois 1,18 ms.
Les pipelines et le JSON diffèrent ; cette comparaison situe le coût observé.

## Écritures concurrentes, réouverture, purge

Cinq lecteurs (100 lectures de la fiche active 24 h chacun, pause de 50 ms),
un rédacteur durable ; une opération d'énergie sur dix tient aussi les totaux
et l'état du compteur. Toutes les opérations capturent leur contenu immuable.

| Mesure (min / médiane / max) | 4 op/s visées | 974 op/s visées |
|---|---:|---:|
| Débit observé (op/s) | 3,97 / 3,98 / 3,99 | 949,24 / 963,75 / 965,17 |
| Lecture p95 (ms) | 8,15 / 12,56 / 14,94 | 9,00 / 9,13 / 11,90 |
| Lecture p99 (ms) | 9,88 / 14,82 / 16,51 | 9,93 / 10,97 / 13,94 |
| Transaction p95 (ms) | 7,52 / 8,73 / 9,46 | 26,05 / 34,90 / 85,83 |
| Pic RSS (Mio) | 18,10 / 18,15 / 20,13 | 20,34 / 21,92 / 22,38 |

La rafale atteint **97,46–99,09 %** du débit visé ;
la tenue intégrale et durable de 974 op/s reste à établir dans le cœur intégré.

| Réouverture et purge (min / médiane / max) | Mesure |
|---|---:|
| Ouverture (ms) | 1,40 / 1,84 / 2,95 |
| Ouverture + première courbe (ms) | 3,17 / 3,68 / 4,60 |
| Vérification complète d’intégrité (s) | 1,84 / 2,48 / 2,73 |
| Purge des relevés (s) | 1,84 / 2,01 / 2,17 |
| Transaction par point, p95 (ms) | 1,12 / 1,20 / 1,26 |
| Transaction par point, maximum (ms) | 85,29 / 102,76 / 103,58 |
| Accusés + vidage de l’outbox (s) | 7,64 / 9,06 / 9,73 |
| Journal après expiration des 7 jours (s) | 6,63 / 6,77 / 6,89 |

316 636 relevés purgés ; une ancre conservée pour les 2 870 points concernés,
**3/3**. Après publication, aucune entrée d'outbox en attente et aucune ligne
du journal supprimée avant sa fenêtre. Après expiration simulée, le journal
est purgé et `replay_floor` atteint la dernière séquence, **3/3**. Intégrité
correcte après chaque réouverture. Le fichier conserve sa taille après purge :
les pages sont réutilisables ; la purge ne compacte pas le fichier.

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
