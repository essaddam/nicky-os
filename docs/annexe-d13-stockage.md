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

‹FUNC›

## Tailles (linux/arm64, musl, octets)

‹SIZES›

## Charge ×2

‹LOAD›

## Lectures du tableau de bord (p95, ms : min / médiane / max sur les tours)

‹READS›

## Écritures concurrentes, réouverture, purge

‹BURST›

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
- La coupure de courant : `kill -9` tue le processus, pas la machine ; la durabilité tient aux
  fsync comptés plus haut.
- Le service d'archive (C3) : ni HTTP, ni Parquet ici ; seul le coût côté cœur d'un lot
  d'export est mesuré.
