# Formes de l'historique en Parquet : série 2026-10-21T200551Z

> Rapport généré par [`tools/storage-writes/parquet_forms.py`](../../tools/storage-writes/parquet_forms.py) :
> ne pas le modifier à la main, relancer l'outil. Ses chiffres fondent la
> [V3 de la proposition](../proposition/valeurs-appareils-stockage-v3.md). Rapports voisins :
> [profil de l'historique](profil-historique-2026-10-21T151507Z.md),
> [mesure des écritures](mesure-ecritures-2026-10-21T114647Z.md).

## Série

| Champ | Valeur |
| --- | --- |
| Série | `2026-10-21T200551Z` : instant UTC du calcul |
| Dépôt | `bd1714d` ; outil modifié ou non committé |
| Outil | `tools/storage-writes/parquet_forms.py`, SHA-256 `5cbfa9dd430e1c70e1ea541abe7b60833c2c8f1bac83235198a92713b1df19b6` |
| Entrée | `dashboard-workload-shape.json` de l'étude privée, hors dépôt : SHA-256 `ad7d65a0171c9ed5a1851ec31f7c594d0ad7b5a5f579de678cb62ff10a8abb85` |
| Environnement | Python 3.14.5, pyarrow 23.0.1, Darwin 27.2.0 arm64, Apple M4 |
| Commande | `python3 -I tools/storage-writes/parquet_forms.py <dashboard-workload-shape.json>` |

## Charge et valeurs

- Comptes réels : 1 512 séries, 1 161 366 relevés en sept jours
  (1 311 mesures, 201 états ou booléens, natures tirées au sort selon
  le mélange de l'étude).
- **Valeurs synthétiques** (graine fixe) : puissances bruitées (±3 W, sauts d'allumage) pour les séries
  bavardes, mesures lentes (±0,02 °C, dérives) pour les autres, états à transitions fréquentes.
  Les octets des formes à bande dépendent de ce modèle ; ceux des autres formes surtout de la forme.
- Bande : seuil de 5 unités (5 W, 0,05 °C). Elle garde 226 983 lignes de mesure sur
  1 039 955, soit 4,6 fois moins.

## Octets par forme

Sept jours ; « Mo par an » = taille × 52. Réglages actuels de l'archive : Snappy, dictionnaire coupé,
groupes de 256 lignes. Réglages proposés : zstd, dictionnaire, groupes de 65 536 lignes.

| Forme | Lignes (7 j) | Mo (7 j) | Octets par ligne | Mo par an |
| --- | --- | --- | --- | --- |
| archive_v1 brut, réglages actuels | 1 161 366 | 47,29 | 40,7 | 2 459 |
| archive_v1 brut, zstd + dictionnaire + grands groupes | 1 161 366 | 19,78 | 17,0 | 1 029 |
| typé brut : mesures + états | 1 161 366 | 7,07 | 6,1 | 368 |
| typé à bande : mesures à bande + états | 348 394 | 3,37 | 9,7 | 175 |
| typé heures denses : mesures + durées par état | 256 239 | 0,33 | 1,3 | 17 |
| archive_v1 heures denses (mesures seulement), réglages actuels | 210 688 | 12,50 | 59,3 | 650 |
| typé heures creuses : mesures + durées par état | 46 523 | 0,23 | 5,0 | 12 |
| typé jours denses : mesures + durées par état | 11 828 | 0,07 | 6,0 | 4 |
| typé jours creux : mesures + durées par état | 5 806 | 0,05 | 8,5 | 3 |

## Lecture

- Le format actuel de l'archive coûte surtout par sa forme, pas par ses réglages : la colonne
  `payload` répète l'opération en JSON, `entity` répète le nom du point.
- Une grille dense se compresse bien quand les séries ne changent pas (dictionnaire et répétitions) :
  comparer « denses » et « creuses » en octets, pas seulement en lignes.
- La bande réduit les lignes ; son gain en octets dépend du bruit réel des capteurs, que seules des
  valeurs réelles donneront.

## Données brutes

```json
{
 "serie": "2026-10-21T200551Z",
 "meta": {
  "commit": "bd1714d",
  "outil_etat": "modifié ou non committé",
  "outil_sha256": "5cbfa9dd430e1c70e1ea541abe7b60833c2c8f1bac83235198a92713b1df19b6",
  "entree": "dashboard-workload-shape.json",
  "entree_sha256": "ad7d65a0171c9ed5a1851ec31f7c594d0ad7b5a5f579de678cb62ff10a8abb85",
  "env": {
   "python": "3.14.5",
   "sqlite": "3.53.4",
   "systeme": "Darwin 27.2.0 arm64",
   "processeur": "Apple M4"
  },
  "pyarrow": "23.0.1"
 },
 "charge": {
  "series": 1512,
  "mesures": 1311,
  "etats": 201,
  "releves_7j": 1161366,
  "releves_mesures_7j": 1039955,
  "lignes_bande_7j": 226983,
  "reduction_bande": 4.6
 },
 "resultats": {
  "archive_v1 brut, réglages actuels": {
   "lignes": 1161366,
   "octets": 47286137
  },
  "archive_v1 brut, zstd + dictionnaire + grands groupes": {
   "lignes": 1161366,
   "octets": 19780321
  },
  "typé brut : mesures + états": {
   "lignes": 1161366,
   "octets": 7074792
  },
  "typé à bande : mesures à bande + états": {
   "lignes": 348394,
   "octets": 3367000
  },
  "typé heures denses : mesures + durées par état": {
   "lignes": 256239,
   "octets": 326709
  },
  "archive_v1 heures denses (mesures seulement), réglages actuels": {
   "lignes": 210688,
   "octets": 12503209
  },
  "typé heures creuses : mesures + durées par état": {
   "lignes": 46523,
   "octets": 232593
  },
  "typé jours denses : mesures + durées par état": {
   "lignes": 11828,
   "octets": 70758
  },
  "typé jours creux : mesures + durées par état": {
   "lignes": 5806,
   "octets": 49474
  }
 }
}
```
