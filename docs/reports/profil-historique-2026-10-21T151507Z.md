# Profil de l'historique : série 2026-10-21T151507Z

> Rapport généré par [`tools/storage-writes/profile.py`](../../tools/storage-writes/profile.py) : ne
> pas le modifier à la main, relancer l'outil. Ses chiffres fondent la
> [V2 de la proposition](../proposition/valeurs-appareils-stockage-v2.md) (paliers par type,
> segments). Mesure voisine des écritures :
> [mesure-ecritures-2026-10-21T114647Z](mesure-ecritures-2026-10-21T114647Z.md).

## Série

| Champ | Valeur |
| --- | --- |
| Série | `2026-10-21T151507Z` : instant UTC du calcul |
| Dépôt | `bd1714d` ; outil modifié ou non committé |
| Outil | `tools/storage-writes/profile.py`, SHA-256 `e5bf00e7bfb2a5567bb9e86617b83316ad142887b1eb4055c218e939ebe2c08f` |
| Entrées | `dashboard-workload-shape.json`, SHA-256 `ad7d65a0171c9ed5a1851ec31f7c594d0ad7b5a5f579de678cb62ff10a8abb85`; `production-load.json`, SHA-256 `404179f82d5c050697c25b5cb6393fb184c26a607a8287a7dbdb50ee3a001d19` (étude privée, hors dépôt) |
| Environnement | Python 3.14.5, Darwin 27.2.0 arm64 |
| Commande | `python3 -I tools/storage-writes/profile.py <dashboard-workload-shape.json> <production-load.json>` |

## Qui écrit l'historique

Journée rejouée par la mesure des écritures (jour 6) : 164 126 relevés pour
1 512 séries. Les comptes comprennent les mises à jour d'attributs seuls : ils majorent
les vrais changements de valeur.

| Séries | Nombre | Relevés du jour | Part des relevés |
| --- | --- | --- | --- |
| silencieuses | 1 006 | 0 | 0,0 % |
| 1 à 24 par jour | 338 | 2 078 | 1,3 % |
| 25 à 96 | 44 | 2 146 | 1,3 % |
| 97 à 288 | 60 | 13 047 | 7,9 % |
| 289 à 1 440 | 30 | 18 544 | 11,3 % |
| plus de 1 440 (plus d'une par minute) | 34 | 128 311 | 78,2 % |

## Types de valeurs et octets par ligne (étude)

| Type de valeur (étude) | Part des changements |
| --- | --- |
| number | 86,6 % |
| text_32 | 9,1 % |
| bool | 3,5 % |
| null | 0,5 % |
| text_256 | 0,3 % |

Sur 901 423 changements en sept jours. `text_32` : textes courts, surtout des
énumérations (« on », « heat », « playing ») ; `text_256` : textes longs (titres, messages).
En Parquet, sur des valeurs réelles : 7,2 à 9,6 octets par
relevé brut, 6,6 à 8,7 octets par ligne de statistiques
(moyenne, minimum, maximum).

## Ce que pèse chaque forme à long terme

Lignes par an pour la charge ×1 ; le poids applique les octets par ligne mesurés en Parquet sur des
valeurs réelles (bas et haut de l'étude). Une heure creuse ou un jour creux non écrit veut dire
« la valeur n'a pas changé » : la disponibilité de l'appareil dit si la source vivait.

| Forme | Lignes par jour | Lignes par an | Mo par an en Parquet (mesuré) |
| --- | --- | --- | --- |
| Relevés bruts | 164 126 | 59 905 990 | 429,8 à 573,1 |
| Heures denses : chaque point, chaque heure | 36 288 | 13 245 120 | 87,9 à 114,6 |
| Heures creuses : seulement les heures où le point a changé | 4 827 | 1 761 855 | 11,7 à 15,2 |
| Jours denses : chaque point, chaque jour | 1 512 | 551 880 | 3,7 à 4,8 |
| Jours creux : seulement les jours où le point a changé | 506 | 184 690 | 1,2 à 1,6 |

## Limites

- Les comptes viennent de Home Assistant, sans bande morte : une forme creuse sur Moli serait plus
  petite encore.
- Les octets par ligne ont été mesurés par l'étude, en colonnes typées et zstd. Le format actuel
  de l'archive (`archive_v1`) en est loin : nom du point et JSON complet de l'opération sur chaque
  ligne, sans dictionnaire, en Snappy, par groupes de 256 lignes. Son coût réel reste à mesurer.
- Les segments à tolérance demandent les valeurs elles-mêmes, que ce profil n'a pas : leur gain se
  mesure sur un export anonymisé avec valeurs.

## Données brutes

```json
{
 "serie": "2026-10-21T151507Z",
 "meta": {
  "commit": "bd1714d",
  "outil_etat": "modifié ou non committé",
  "outil_sha256": "e5bf00e7bfb2a5567bb9e86617b83316ad142887b1eb4055c218e939ebe2c08f",
  "entrees": [
   [
    "dashboard-workload-shape.json",
    "ad7d65a0171c9ed5a1851ec31f7c594d0ad7b5a5f579de678cb62ff10a8abb85"
   ],
   [
    "production-load.json",
    "404179f82d5c050697c25b5cb6393fb184c26a607a8287a7dbdb50ee3a001d19"
   ]
  ],
  "env": {
   "python": "3.14.5",
   "sqlite": "3.53.4",
   "systeme": "Darwin 27.2.0 arm64",
   "processeur": "Apple M4"
  }
 },
 "profil": {
  "series": 1512,
  "releves_jour": 164126,
  "heures_denses": 36288,
  "heures_creuses": 4827,
  "jours_denses": 1512,
  "jours_creux": 506,
  "classes": [
   {
    "classe": "silencieuses",
    "series": 1006,
    "releves": 0,
    "part": 0.0
   },
   {
    "classe": "1 à 24 par jour",
    "series": 338,
    "releves": 2078,
    "part": 0.013
   },
   {
    "classe": "25 à 96",
    "series": 44,
    "releves": 2146,
    "part": 0.013
   },
   {
    "classe": "97 à 288",
    "series": 60,
    "releves": 13047,
    "part": 0.079
   },
   {
    "classe": "289 à 1 440",
    "series": 30,
    "releves": 18544,
    "part": 0.113
   },
   {
    "classe": "plus de 1 440 (plus d'une par minute)",
    "series": 34,
    "releves": 128311,
    "part": 0.782
   }
  ]
 },
 "etude": {
  "melange": {
   "number": 0.866,
   "text_32": 0.091,
   "bool": 0.035,
   "null": 0.005,
   "text_256": 0.003
  },
  "changements_7j": 901423,
  "octets_brut": [
   7.1752,
   9.567
  ],
  "octets_stats": [
   6.6356,
   8.6534
  ]
 }
}
```
