# Écritures du stockage : série 2026-10-21T114647Z

> Rapport généré par [`tools/storage-writes/measure.py`](../../tools/storage-writes/measure.py) : ne pas
> le modifier à la main, relancer l'outil. Ses chiffres fondent les § 13.7 à 13.9 de la proposition
> [Valeurs, appareils et stockage](../proposition/valeurs-appareils-stockage.md#137-double-écriture--létat-courant-complet-dans-maisondb) et la section « Double écriture et nuit » de sa
> [page HTML](../proposition/valeurs-appareils-stockage.html#double-ecriture). Contexte :
> [inventaire de la persistance](inventaire-persistance-2026-10-20T060728Z.md),
> [annexe D13](../annexe-d13-stockage.md).

## Série

| Champ | Valeur |
| --- | --- |
| Série | `2026-10-21T114647Z` : instant UTC de la mesure |
| Dépôt | `d0ce806` ; outil modifié ou non committé |
| Outil | `tools/storage-writes/measure.py`, SHA-256 `41e65082d52a34c397df6f9fd623fc080352f01d6fc660ad6ba736a1fccc3c75` |
| Entrée | `dashboard-workload-shape.json` de l'étude privée, hors dépôt : SHA-256 `ad7d65a0171c9ed5a1851ec31f7c594d0ad7b5a5f579de678cb62ff10a8abb85`, 872 933 octets |
| Environnement | Python 3.14.5, SQLite 3.53.4, Darwin 27.2.0 arm64, Apple M4 |
| Durée | 132 s |
| Commande | `python3 -I tools/storage-writes/measure.py <dashboard-workload-shape.json>` |

## Charge rejouée

- 1 512 séries anonymisées, comptes horaires réels sur 168 heures : 1 161 366 relevés
  en sept jours, soit 1,92 par seconde en moyenne (charge ×1 du banc D13). Les
  10 séries les plus bavardes font 42 % des relevés, les 50 premières 86 %.
- Jours 1 à 5 pré-remplis (827 038 relevés), jour 6 rejoué : 164 126 relevés,
  1 440 minutes sur 1 440 avec au moins un rapport.
- Dans une heure, les relevés d'une série sont régulièrement espacés, avec une phase tirée par série
  (graine 42). Les valeurs sont fictives : des entiers de 2 octets, comme un décimal mis à l'échelle.
- Données utiles du jour : ≈ 3,4 Mo, à ≈ 21 octets par relevé amaigri (*estimation*
  du § 14.1).

## Méthode

- SQLite en WAL, pages de 4 Kio, `synchronous = OFF` : on compte des octets, pas des fsync. En
  `FULL`, comme dans D13, chaque validation ajoute un fsync.
- Point de contrôle automatique coupé ; `TRUNCATE` manuel quand le WAL dépasse 64 Mo.
- Écrit par jour = octets ajoutés au WAL + pages distinctes que chaque point de contrôle recopie dans
  la base. Le pré-remplissage n'est pas compté.
- Schémas : ceux du banc D13 (`tools/storage-bench/src/schema.rs`) et ceux du § 14 de la proposition.

## 1. Chemin d'écriture complet de l'historique

| Scénario | Validations par jour | WAL (Mo) | Points de contrôle (Mo) | Écrit par jour (Mo) | Pages par validation |
| --- | --- | --- | --- | --- | --- |
| D13 tel que décidé : relevé + outbox + journal NDJSON + `meta`, lots ≤ 100 ms | 150 311 | 2 948,9 | 149,4 | 3 098,3 | 4,76 |
| D13 avec des lots de 2 s | 42 638 | 1 635,5 | 115,6 | 1 751,1 | 9,31 |
| Proposé : opération compacte en ajout, lots ≤ 100 ms, rangée la nuit | 150 313 | 641,2 | 17,3 | 658,6 | 1,04 |
| Proposé : opération compacte en ajout, lots de 2 s, rangée à l'heure close | 42 663 | 234,6 | 23,1 | 257,7 | 1,33 |
| Proposé : lots de 2 s, rangée la nuit | 42 640 | 200,3 | 17,2 | 217,5 | 1,14 |
| Proposé : lots de 10 s, rangée la nuit | 8 723 | 61,5 | 17,1 | 78,6 | 1,71 |

## 2. `samples` rangé par point, seul

Le rangement de D13 sans l'export : chaque point présent dans un lot réécrit sa propre page.

| Scénario | Validations par jour | WAL (Mo) | Points de contrôle (Mo) | Écrit par jour (Mo) | Pages par validation |
| --- | --- | --- | --- | --- | --- |
| `samples` rangé par point, lots de 2 s | 42 557 | 853,7 | 26,9 | 880,5 | 4,87 |
| `samples` rangé par point, lots de 10 s | 8 640 | 815,9 | 26,5 | 842,4 | 22,92 |
| `samples` rangé par point, lots de 60 s | 1 440 | 482,8 | 19,1 | 501,9 | 81,38 |

## 3. État courant

| Scénario | Validations par jour | WAL (Mo) | Points de contrôle (Mo) | Écrit par jour (Mo) | Pages par validation |
| --- | --- | --- | --- | --- | --- |
| `point_state`, lots de 2 s | 42 557 | 493,9 | 0,6 | 494,6 | 2,82 |
| `point_state`, lots de 60 s | 1 440 | 64,8 | 0,1 | 64,9 | 10,92 |
| `point_state`, lots de 15 min | 96 | 5,8 | 0,1 | 5,9 | 14,61 |

`state.json` aujourd'hui (*estimation*) : un fichier de même forme pour 1 512 points pèse
81 411 octets. Réécrit à chaque minute qui a reçu un rapport (1 440 par jour), il
écrit ≈ 117,2 Mo par jour, avec deux fsync par réécriture (fichier et dossier).

## 4. Lectures

Courbe des dernières 24 h du point le plus bavard, médiane de 31 lectures sur la machine de mesure :

| Lecture | Lignes rendues | Médiane (ms) |
| --- | --- | --- |
| plage de `samples` rangé par point | 14 136 | 9,77 |
| balayage du journal du jour entier | 14 136 | 19,48 |
| balayage du journal d'une heure | 454 | 0,46 |

## Contrôles

- Chaque scénario de l'historique retrouve tous ses relevés après la journée : pré-remplissage +
  jour dans `samples`, sans doublon.
- Relancer l'outil réécrit un rapport identique pour les octets et les validations (graines fixes,
  même version de SQLite) ; les lectures varient d'une série à l'autre.

## Limites

- SQLite remplace Turso : même format de pages, mais Turso peut différer sur le point de contrôle.
  À confirmer avec `tools/storage-bench`, puis sur le Pi Zero 2 W de D10 et sa carte microSD.
- La charge est celle d'une maison entièrement passée de Home Assistant à Moli, sans bande morte
  ni intervalle minimal ; les horodatages sont réguliers dans l'heure.
- La machine de mesure n'est pas un Pi : comparer les lignes entre elles.

## Données brutes

```json
{
 "serie": "2026-10-21T114647Z",
 "meta": {
  "commit": "d0ce806",
  "outil_etat": "modifié ou non committé",
  "outil_sha256": "41e65082d52a34c397df6f9fd623fc080352f01d6fc660ad6ba736a1fccc3c75",
  "entree": "dashboard-workload-shape.json",
  "entree_sha256": "ad7d65a0171c9ed5a1851ec31f7c594d0ad7b5a5f579de678cb62ff10a8abb85",
  "entree_octets": 872933,
  "env": {
   "python": "3.14.5",
   "sqlite": "3.53.4",
   "systeme": "Darwin 27.2.0 arm64",
   "processeur": "Apple M4"
  },
  "duree_s": 132
 },
 "charge": {
  "series": 1512,
  "heures": 168,
  "releves_7j": 1161366,
  "releves_par_s": 1.92,
  "part_10": 0.425,
  "part_50": 0.865,
  "prerempli": 827038,
  "jour": 164126,
  "minutes_avec_rapport": 1440
 },
 "resultats": {
  "chemin": {
   "d13-100ms": {
    "validations": 150311,
    "wal_mo": 2948.9,
    "checkpoint_mo": 149.4,
    "total_mo": 3098.3,
    "pages_par_validation": 4.76
   },
   "d13-2s": {
    "validations": 42638,
    "wal_mo": 1635.5,
    "checkpoint_mo": 115.6,
    "total_mo": 1751.1,
    "pages_par_validation": 9.31
   },
   "prop-100ms-nuit": {
    "validations": 150313,
    "wal_mo": 641.2,
    "checkpoint_mo": 17.3,
    "total_mo": 658.6,
    "pages_par_validation": 1.04
   },
   "prop-2s-heure": {
    "validations": 42663,
    "wal_mo": 234.6,
    "checkpoint_mo": 23.1,
    "total_mo": 257.7,
    "pages_par_validation": 1.33
   },
   "prop-2s-nuit": {
    "validations": 42640,
    "wal_mo": 200.3,
    "checkpoint_mo": 17.2,
    "total_mo": 217.5,
    "pages_par_validation": 1.14
   },
   "prop-10s-nuit": {
    "validations": 8723,
    "wal_mo": 61.5,
    "checkpoint_mo": 17.1,
    "total_mo": 78.6,
    "pages_par_validation": 1.71
   }
  },
  "rangement_par_point": {
   "point-2 s": {
    "validations": 42557,
    "wal_mo": 853.7,
    "checkpoint_mo": 26.9,
    "total_mo": 880.5,
    "pages_par_validation": 4.87
   },
   "point-10 s": {
    "validations": 8640,
    "wal_mo": 815.9,
    "checkpoint_mo": 26.5,
    "total_mo": 842.4,
    "pages_par_validation": 22.92
   },
   "point-60 s": {
    "validations": 1440,
    "wal_mo": 482.8,
    "checkpoint_mo": 19.1,
    "total_mo": 501.9,
    "pages_par_validation": 81.38
   }
  },
  "etat": {
   "point_state-2 s": {
    "validations": 42557,
    "wal_mo": 493.9,
    "checkpoint_mo": 0.6,
    "total_mo": 494.6,
    "pages_par_validation": 2.82
   },
   "point_state-60 s": {
    "validations": 1440,
    "wal_mo": 64.8,
    "checkpoint_mo": 0.1,
    "total_mo": 64.9,
    "pages_par_validation": 10.92
   },
   "point_state-15 min": {
    "validations": 96,
    "wal_mo": 5.8,
    "checkpoint_mo": 0.1,
    "total_mo": 5.9,
    "pages_par_validation": 14.61
   },
   "state_json": {
    "points": 1512,
    "octets": 81411,
    "reecritures": 1440,
    "mo_par_jour": 117.2
   }
  },
  "lecture": {
   "plage_samples": [
    9.77,
    14136
   ],
   "journal_du_jour": [
    19.48,
    14136
   ],
   "journal_d_une_heure": [
    0.46,
    454
   ]
  }
 }
}
```
