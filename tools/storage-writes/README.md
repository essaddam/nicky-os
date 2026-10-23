# Écritures, profil et formes du stockage

Trois outils de mesure de la proposition « Valeurs, appareils et stockage » :

- la [V3](../../docs/proposition/valeurs-appareils-stockage-v3.md), qui fait foi ;
- la [V1](../../docs/proposition/valeurs-appareils-stockage.md) et la
  [V2](../../docs/proposition/valeurs-appareils-stockage-v2.md), gardées en historique.

| Outil | Ce qu'il mesure | Rapport daté | Dépendances |
| --- | --- | --- | --- |
| `measure.py` | les octets écrits sur disque par jour, en rejouant une vraie journée de relevés sur SQLite : rangés par point dès l'écriture (comme D13), ou écrits à la suite puis rangés ; plusieurs rythmes de validation ; état courant ; lectures | `docs/reports/mesure-ecritures-<série>.md` | bibliothèque standard |
| `profile.py` | ce que contient l'historique (séries silencieuses ou bavardes, types de valeurs) et les lignes par jour de chaque forme à long terme | `docs/reports/profil-historique-<série>.md` | bibliothèque standard |
| `parquet_forms.py` | ce que pèse chaque forme en Parquet : format actuel de l'archive contre colonnes typées, brut contre relevés à bande, heures et jours denses contre creux | `docs/reports/formes-parquet-<série>.md` | **pyarrow** |

```
python3 -I tools/storage-writes/measure.py <chemin>/dashboard-workload-shape.json
python3 -I tools/storage-writes/profile.py <chemin>/dashboard-workload-shape.json <chemin>/production-load.json
python3 -I tools/storage-writes/parquet_forms.py <chemin>/dashboard-workload-shape.json
```

Il faut Python 3.8 ou plus récent, sans Docker ni Rust. `parquet_forms.py` demande en plus
`pyarrow`. La mesure des écritures prend quelques minutes sur un ordinateur portable, les deux
autres moins d'une minute. Les fichiers d'essai vont dans un dossier temporaire, effacé à la fin.

## Ce que les outils produisent

Chaque lancement écrit un nouveau rapport daté dans `docs/reports/`. La série est l'instant UTC
du calcul ; le nom suit la forme des autres rapports du dossier. Chaque rapport contient :

- les empreintes SHA-256 de l'outil et des entrées, le commit, les versions de Python et des
  bibliothèques, la machine ;
- la charge, la méthode, les résultats et les limites ;
- les données brutes en JSON.

On ne corrige jamais un rapport à la main : on relance l'outil, qui écrit une nouvelle série. Un
document qui cite des chiffres cite la série qui les a produits. Si un outil change, son
empreinte change aussi : le rapport indique toujours quelle version l'a écrit.

## Rapports produits

| Série | Rapport | Cité par |
| --- | --- | --- |
| `2026-10-21T114647Z` | [mesure-ecritures-2026-10-21T114647Z.md](../../docs/reports/mesure-ecritures-2026-10-21T114647Z.md) | V3 : « En bref », § 11 ; V1 : § 2, § 12.1, § 13.7, § 13.8, § 14.8 ; V2 : § 10 ; pages HTML, sections « Double écriture » |
| `2026-10-21T151507Z` | [profil-historique-2026-10-21T151507Z.md](../../docs/reports/profil-historique-2026-10-21T151507Z.md) | V3 : § 6.1, § 9 ; V2 : « Ce qui change », § 5.1, § 6, § 7 ; pages HTML |
| `2026-10-21T200551Z` | [formes-parquet-2026-10-21T200551Z.md](../../docs/reports/formes-parquet-2026-10-21T200551Z.md) | V3 : « Ce que la réanalyse a changé », « En bref », § 2, § 6.1, § 6.4, § 7, § 8.1 ; page HTML V3, sections « Réanalyse », « Paliers » et « Format de l'archive » |

## Les entrées

- `dashboard-workload-shape.json` est le même fichier de forme que pour `tools/storage-bench` :
  pour chaque point anonymisé, le nombre de relevés par heure sur sept jours.
- `production-load.json` contient les agrégats de la même étude : mélange des types de valeurs,
  octets par ligne mesurés en Parquet sur des valeurs réelles.
- Les deux viennent de l'étude privée et n'entrent jamais dans le dépôt. Les rapports n'en
  gardent que le nom, la taille ou l'empreinte, et des agrégats. Toutes les valeurs écrites par
  les outils sont fictives.

## Ce que mesure `measure.py`

| Partie | Scénarios | Question |
| --- | --- | --- |
| 1. Chemin d'écriture complet de l'historique | D13 tel que décidé (relevé, outbox, journal NDJSON, `meta`), en lots de 100 ms au plus puis de 2 s ; proposé (une opération compacte écrite à la suite, rangée par point à l'heure close ou la nuit), en lots de 100 ms au plus, de 2 s et de 10 s | combien d'octets et de validations par jour ? |
| 2. `samples` rangé par point, seul | lots de 2 s, 10 s et 60 s | des lots plus longs suffisent-ils ? |
| 3. État courant | `point_state` vidé toutes les 2 s, 60 s ou 15 min ; taille estimée de `state.json` | que coûte la double écriture ? |
| 4. Lectures | courbe de 24 h du point le plus bavard : plage de `samples`, journal du jour entier, journal d'une heure | ranger à l'heure close ou la nuit ? |

**Méthode**

- La charge rejouée vient de l'entrée : les jours 1 à 5 sont pré-remplis, le jour 6 est rejoué.
  Dans une heure, les relevés d'une série sont régulièrement espacés, avec une phase tirée au
  hasard par série. Les graines sont fixes.
- SQLite tourne en WAL, avec des pages de 4 Kio et `synchronous = OFF`. On compte des octets et
  des validations, pas des fsync : en `FULL`, chaque validation ajoute un fsync.
- Les points de contrôle automatiques sont coupés. Un `TRUNCATE` manuel part dès que le WAL dépasse
  64 Mo.
- Octets écrits = octets ajoutés au WAL + pages distinctes que chaque point de contrôle recopie dans
  la base. Le pré-remplissage n'est pas compté.
- Les schémas sont ceux de `tools/storage-bench/src/schema.rs` pour D13 et ceux du § 14 de la V1
  pour le reste.
- Relancer l'outil redonne les mêmes octets et les mêmes validations avec la même version de
  SQLite. Les temps de lecture varient d'une série à l'autre.

## Ce que calcule `profile.py`

- Les séries de la journée rejouée, par classe : silencieuses, puis de 1 à plus de 1 440 relevés
  par jour, avec leur part des relevés.
- Les lignes par jour de chaque forme : relevés bruts, heures denses ou creuses, jours denses ou
  creux ; et leur poids par an avec les octets par ligne de l'étude.

## Ce que mesure `parquet_forms.py`

- **Comptes réels, valeurs synthétiques.** Les natures sont tirées au sort selon le mélange de
  l'étude (86,6 % de mesures, 9,1 % d'énumérations, 3,5 % de booléens). Les séries bavardes
  deviennent des puissances bruitées (±3 W, sauts d'allumage), les autres des mesures lentes
  (±0,02 °C, dérives), les états changent à la plupart de leurs rapports. Graine fixe.
- **Formes écrites en Parquet, sur les sept jours** :
  - `archive_v1` : le schéma de `crates/moli-archive/src/parquet.rs` (`entity`, `ts`, `seq`,
    `kind`, `payload` JSON, `num`, `cost`, `covered_ms`), avec ses réglages (Snappy, dictionnaire
    coupé, groupes de 256 lignes), pour le brut puis pour des heures résumées ; puis le même schéma
    avec zstd, dictionnaire et grands groupes ;
  - typé : brut (mesures et états), relevés à bande (seuil de 5 unités), heures denses et creuses
    (moyenne pondérée, min, max, couverture, changements ; durées par état), jours denses et creux ;
    zstd, dictionnaire, groupes de 65 536 lignes.
- **Ce que ça dit et ne dit pas** : les octets des formes `archive_v1` et des heures ou jours
  dépendent surtout de la forme ; ceux des relevés à bande dépendent du bruit des capteurs, que
  seules des valeurs réelles donneront.

## Limites et suite

- SQLite remplace Turso dans `measure.py` : le format des pages est le même, mais Turso peut
  différer sur les points de contrôle.
- `tools/storage-bench` mesure D13 sur Turso 0.8.3 ; il relève la taille du fichier WAL, pas les
  octets écrits. La suite, prévue au § 15 de la V3 :
  - porter les scénarios de `measure.py` dans ce banc ;
  - relancer `measure.py` sur le Pi Zero 2 W de D10, avec sa carte microSD ;
  - relancer `parquet_forms.py` sur un export anonymisé **avec valeurs**, pour la bande et les
    segments à tolérance.
- La charge est celle d'une maison entièrement passée de Home Assistant à Moli, sans bande morte ni
  intervalle minimal.
- La machine de mesure n'est pas un Pi : les lignes d'un même rapport se comparent entre elles.
