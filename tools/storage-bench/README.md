# Banc du stockage récent sur Turso (ADR D13)

Outil hors workspace : il ne fait partie ni de `cargo test --workspace` ni du
`Cargo.lock` racine, et `scripts/check.sh` l'ignore. Il prouve, à la version
figée de Turso (`=0.8.3`), ce que l'ADR D13 de `docs/ARCHITECTURE.md` suppose,
et mesure la charge ×2 de l'étude de dimensionnement sur le schéma cible.

```
sh tools/storage-bench/run.sh <chemin>/dashboard-workload-shape.json [répétitions=3]
```

Docker et Python 3 (bibliothèque standard) sont requis ; Rust et Node restent
dans les conteneurs. Cible `linux/arm64` par défaut (`PLATFORM=…` pour
une autre). Durée : une dizaine de minutes par répétition sur une machine ARM à
4 cœurs, plus la compilation. `MOLI_IMAGE=1` mesure en plus le binaire actuel de
Moli (étapes du `Dockerfile` racine rejouées sans BuildKit, une vingtaine de
minutes et environ 3 Go de cache) pour situer le delta dans le budget d'image.
Les résultats vont dans `tools/storage-bench/results/<date>/` (ignoré par git) :
un fichier JSON Lines par étape, `sizes.txt`, `host-load.txt` et `summary.txt`.
Les volumes Docker `moli-storage-bench*` gardent les caches de compilation et la
base d'essai ; `docker volume rm` les efface.

Le crate est figé à `turso = "=0.8.3"` sans fonctions par défaut (ni allocateur
global `mimalloc`, ni plein texte). `roaring` (0.11.3) et `uuid` (1.26.1) sont
épinglés dans `Cargo.lock` : leurs dernières versions demandent Rust 1.89 et
1.90, le dépôt annonce 1.88 ; l'étape « minimum Rust » le vérifie.

## Le fichier de forme

`dashboard-workload-shape.json` vient de l'étude privée : pour chaque point
anonymisé, le nombre de relevés par heure sur sept jours, ×2 répliques, plus
les bornes de calendrier de l'énergie (114 compteurs). Il est monté en
lecture seule dans le conteneur et n'entre jamais dans le dépôt ; toutes les
valeurs écrites sont fictives.

## Ce qui est lancé

| Étape | Commande | Ce qu'elle prouve ou mesure |
|---|---|---|
| Compatibilité | `compat` | tables rowid à clé calculée et index secondaires : `UPDATE`, `DELETE`, UPSERT, `RETURNING`, `CHECK`, `STRICT`, doublon à la même milliseconde ignoré ; 20 000 opérations comparées à SQLite puis `integrity_check` de SQLite sur le fichier Turso ; transactions ; sans dossier temporaire, un rédacteur et `VACUUM INTO` échouent (d'où `TURSO_TMPDIR` et `temp_store = MEMORY`) ; lecteurs pendant une écriture ; MVCC (information) ; `synchronous`, WAL, point de contrôle ; les fichiers actuels (`WITHOUT ROWID`) ouverts par Turso ; un fichier Turso lu et modifié par SQLite et réciproquement ; bascule puis retour arrière à l'identique ; séquence jamais réutilisée ; SQL des agents sans autorisateur ; second processus ; `VACUUM INTO` ; io_uring refusé et Turso qui fonctionne quand même |
| Crash | `crash` | 31 `kill -9` à des instants aléatoires pendant des transactions multi-tables et des points de contrôle : séquence annoncée validée ≤ séquence relue, outbox contiguë, liens échantillon ↔ outbox, heures = totaux de jour et de mois = état du compteur, `integrity_check` ; au dernier tour, SQLite relit d'abord le fichier laissé par le crash |
| fsync | `fsync-probe` sous `strace -c` | nombre de fsync pour 200 validations selon `PRAGMA synchronous` |
| Charge ×2 | `load` | 2 322 732 relevés sur 3 024 points dans l'ordre du temps, lots durables de 500 (échantillon + outbox + journal immuable + séquence), puis la fenêtre d'énergie de deux mois civils (UPSERT, outbox), ses totaux de jour et de mois, et douze mois de totaux mensuels |
| Lectures | `reads` | les parcours du dashboard de l'étude (fiche prise 24 h / 7 j, point le plus bavard, 20 petites courbes, énergie 24 h / 168 h / 30 j / 12 mois), plus SecurityPanel 720 h et trois lots d'export de 5 000 opérations (relevés, énergie, mixte) ; `summary()` des deux façons (fenêtre d'heures, totaux) et contrôle qu'elles rendent les mêmes totaux |
| Rafale | `burst` | 5 lecteurs pendant 4 op/s puis 974 op/s durables (une opération d'énergie sur dix : heure + totaux du jour et du mois + état du compteur) |
| Réouverture | `reopen` | ouverture, première courbe, `integrity_check` complet |
| Purge | `purge` | publication et vidage de l'outbox ; un jour de relevés par point avec ancrage ; journal conservé avant sept jours, puis purgé par tranches |

Le lanceur exécute aussi les régressions Python du verdict global et les tests
Rust du banc. Une compatibilité fausse, un invariant de crash faux, un échec
MSRV, une intégrité incorrecte, un ancrage faux ou une série incomplète rendent
un code non nul. Le résumé est validé avant d'être publié ; aucun tube vers
`tee` ne masque son statut.

## Journal et reprise de l'archive

Chaque écriture métier capture son opération autonome dans `export_journal`,
dans la transaction de la donnée, de l'outbox et de la séquence. Le contenu
persisté fournit l'export NDJSON, dans l'ordre des séquences. Les lots mesurés
couvrent séparément 5 000 relevés, 5 000 heures et un lot mixte de 2 500 de chaque.
La compatibilité vérifie le réessai après modification d'une heure et réouverture,
puis le rejeu après restauration, vidage de l'outbox et purge d'un import ancien.

L'outbox d'attente est vidée à publication ; le journal reste sept jours après
publication. Une restauration réarme cette fenêtre pour les opérations à
republier. La purge ne retire qu'un préfixe contigu et conserve son plancher
`replay_floor` ; une restauration antérieure est refusée explicitement. Le banc
mesure l'accusé et le vidage de l'outbox, puis simule l'expiration des sept jours
pour mesurer séparément le nettoyage du journal.

Les essais tournent dans l'image `FROM scratch` (binaire statique musl seul),
sous le profil seccomp par défaut de Docker, limités à 1 CPU et 128 Mio comme
l'étude, avec la base sur un volume Docker et `TURSO_TMPDIR` sur ce volume
(l'image n'a pas de `/tmp`).

## Limites connues

- `kill -9` tue le processus, pas la machine : le cache du système survit.
  La durabilité face à une coupure de courant repose sur les fsync comptés
  ci-dessus, pas sur une coupure simulée.
- Pas de requête HTTP, de Hub ni de pilotes : ce sont les coûts du stockage,
  sérialisation JSON comprise, comme dans l'étude.
- La machine de mesure n'est pas un Pi : comparer les chiffres entre eux et à
  la référence redb de l'étude, pas à un objectif absolu sur le matériel cible.
- Les conteneurs voisins partagent la VM : `host-load.txt` note les charges de
  la VM et de l'hôte au début de chaque tour et à la fin, à lire avec la dispersion.
- Les totaux de jour et de mois prennent un décalage horaire fixe tiré des
  bornes de l'étude (valeurs fictives) ; Moli prend son fuseau.
