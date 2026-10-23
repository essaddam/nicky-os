# Valeurs, appareils et stockage, V2 : le type décide, le temps résume

> **Version 2, remplacée par la [V3](valeurs-appareils-stockage-v3.md)** (21 octobre 2026). La V3
> mesure ce que la V2 estimait : le format de l'archive devient le premier levier, les heures
> creuses et la coupure de bande à l'heure sont abandonnées, les transitions de sûreté suivent
> D13. Ce texte reste en historique.

> **Proposition V2, non livrée.** 21 octobre 2026, références au code vérifiées sur `bd1714d`.
> Elle remplace la [V1](valeurs-appareils-stockage.md) du même jour, gardée comme historique. Aucun
> code de production n'a été modifié. Les chiffres mesurés viennent de deux séries datées,
> produites par [`tools/storage-writes`](../../tools/storage-writes/README.md) :
>
> - [`mesure-ecritures-2026-10-21T114647Z`](../reports/mesure-ecritures-2026-10-21T114647Z.md) :
>   ce que coûte chaque façon d'écrire ;
> - [`profil-historique-2026-10-21T151507Z`](../reports/profil-historique-2026-10-21T151507Z.md) :
>   ce que contient l'historique et ce que pèserait chaque forme à long terme.
>
> Page d'explication illustrée : [valeurs-appareils-stockage-v2.html](valeurs-appareils-stockage-v2.html).

## Ce qui change depuis la V1

| Sujet | V1 | V2 | Pourquoi |
| --- | --- | --- | --- |
| Long terme | agrégats horaires et journaliers gardés sans fin dans `moli.db` (Turso) | dans l'archive Parquet seulement ; `moli.db` ne garde que le récent | décision du propriétaire, confirmée le 21 octobre : le récent sur Turso, le long terme sur Parquet ; D13 dit déjà « le cœur ne garde que des fenêtres chaudes » ([ARCHITECTURE.md:199](../ARCHITECTURE.md)) |
| Format du long terme | non examiné | `archive_v1` est générique : nom du point et JSON complet sur chaque ligne, sans dictionnaire, en Snappy. La V2 propose un Parquet typé par nature | l'étude mesurait 7,2 à 9,6 octets par relevé en colonnes typées ; `archive_v1` en est loin (§ 2) |
| Vieillissement | des heures pour tout | des paliers par nature (§ 6). Aujourd'hui, l'archive ne retire rien | un tiers des séries écrit tous les relevés du jour, et 34 séries en écrivent 78 % |
| Mesures | heures denses | relevé à bande → heures creuses → jours creux ; segments à tolérance en option | une grille horaire dense écrit 36 288 lignes par jour, les heures creuses 4 827 |
| États (porte) | durées par heure | transitions gardées telles quelles, jamais lissées ; durées par jour en plus | une ouverture de porte est un fait, pas une mesure |
| Cumuls (énergie) | delta réparti, périodes additives | mêmes règles, plus un élagage explicite : deltas par heure, puis par jour, puis par mois, index de fin gardé | un cumul ne se moyenne jamais |
| Intervalle minimal | le dernier gagne ; extrêmes dans un agrégat horaire à part | la bande : le relevé écrit porte le minimum, le maximum et la moyenne de ce qu'il a absorbé | un seul mécanisme au lieu de trois |
| Énumérations, textes | dictionnaires dans `moli.db` | des codes de bout en bout : journal d'export, archive, résumés | 9,1 % des changements sont des textes courts, presque tous des énumérations |
| Double écriture, nuit | V1 § 13.7 à 13.9 | inchangées ; le vieillissement du long terme revient à l'archive (E1) | — |

## En bref

1. **Quatre étages, chacun sa technique.**
   - La mémoire du hub est la vérité vive.
   - `maison.db` garde l'état courant.
   - `moli.db` garde le récent, sur Turso, dans les fenêtres de D13 : 7 à 31 jours.
   - L'archive garde le long terme, en Parquet.
   - Un étage ne garde jamais ce qui revient au suivant.
2. **Le type décide de tout** (§ 4) : l'encodage, le filtre, le vieillissement, et ce qu'on ne fait
   jamais.
3. **Tout est segment** (§ 5) : un intervalle `[début, fin)` et une charge utile qui dépend de la
   nature. Vieillir, c'est fusionner des voisins selon la règle de la nature : c'est ta proposition
   « valeur moyenne, date de début et de fin », généralisée.
4. **Mesures** : relevé à bande, puis heures creuses, puis jours creux.
   - Le relevé à bande garde, pour l'intervalle qu'il ferme, le minimum, le maximum et la moyenne
     de ce qu'il a absorbé.
   - Une heure creuse n'est écrite que si la valeur a changé.
5. **États** (porte, serrure, mode) : les transitions sont gardées à l'identique, jamais lissées. Des
   durées par jour s'y ajoutent pour les statistiques.
6. **Cumuls** (énergie, eau) : élagués, jamais moyennés. On garde des deltas par heure, puis par
   jour, puis par mois, avec l'index de fin et les remises à zéro.
7. **Énumérations et textes** : des codes d'un octet, de bout en bout.
   - Un texte répété n'est stocké qu'une fois.
   - Effacer un texte l'efface partout, sans réécrire l'archive.
8. **Le long terme en Parquet typé**, avec un jeu de fichiers par nature, rangé par mois.
   - Pour la charge ×1, les heures creuses pèsent 11,7 à 15,2 Mo par an.
   - Les relevés bruts pèseraient 430 à 573 Mo par an (série profil, octets de l'étude).
9. **`moli.db` ne garde que le récent** : environ 70 à 90 Mo en régime établi (*estimation*).
10. **La double écriture et la nuit de la V1 restent.** On écrit ≈ 0,32 Go par jour contre ≈ 3,1 Go
    pour le chemin de D13 (série écritures).
11. **Le chantier existe déjà en tickets** : C2 (Turso), D1 (lectures fusionnées), E1 (vieillissement
    de l'archive). La V2 donne à E1 ses règles.

## 1. Le besoin

La V1 reste valable pour les valeurs, les instants, la fraîcheur, les équipements et les
conditions (§ 13). La V2 répond à six demandes :

- garder le long terme dans la base optimisée dont on a parlé : l'archive Parquet ;
- garder des données qu'on agrège petit à petit, suivant le type ;
- conserver « une valeur moyenne, une date de début et une date de fin », toujours selon la valeur ;
- une ouverture ou une fermeture de porte se garde : on ne la lisse pas ;
- l'énergie n'a pas de moyenne : c'est un cumul, on l'élague ;
- les énumérations évitent de stocker des textes coûteux.

**Questions de cadrage (« Comment pourrions-nous… »)**

1. … garder des années d'historique en ne payant que ce qui change ?
2. … faire vieillir chaque valeur selon sa nature, sans jamais perdre ce qui ne se résume pas ?
3. … que le long terme vive dans le format fait pour lui, et le récent dans le sien ?
4. … ne jamais stocker deux fois le même texte ?

## 2. Ce qui existe (constats du 21 octobre, code à `bd1714d`)

| Sujet | Aujourd'hui | Conséquence |
| --- | --- | --- |
| Cœur | SQLite (`history.db`, `energy.db`). Turso (`moli.db`, C2) est en développement, pas intégré | la V2 vise la cible ; rien n'est à migrer deux fois si elle est décidée avant C2 |
| Archive | `moli-archive` est fusionné : binaire à part, HTTP, image `FROM scratch`, d'abord sur la même machine ([ARCHITECTURE.md:200](../ARCHITECTURE.md)). Aucun crate du cœur ne l'appelle encore : pas de client (D1) | le long terme existe, mais rien ne l'alimente |
| Format `archive_v1` | une seule forme de ligne : `entity` (nom du point en texte), `ts`, `seq`, `kind` (texte), `payload` (le JSON complet de l'opération), `num`, `cost`, `covered_ms` ([parquet.rs:27](../../crates/moli-archive/src/parquet.rs#L27)). Snappy, dictionnaire désactivé, groupes de 256 lignes au plus ([parquet.rs:190](../../crates/moli-archive/src/parquet.rs#L190)) | chaque ligne répète le nom du point et l'opération entière : loin des 7,2 à 9,6 octets par relevé que l'étude a mesurés en colonnes typées et zstd |
| Rangement | fichiers `{groupe}/year=…/month=…/part-…` ([parts.rs:118](../../crates/moli-archive/src/parts.rs#L118)) ; groupes `numeric`, `events`, `energy`, `stats`, `corrections`, `other` ([manifest.rs:77](../../crates/moli-archive/src/manifest.rs#L77)) | les états (`b`, `t`) vont dans `events`, mêlés aux événements ([store.rs:294](../../crates/moli-archive/src/store.rs#L294)) |
| Résumés | heures, jours et mois calculés par l'archive depuis le brut, avec `moli-rollup` | ils s'ajoutent au brut : rien n'est retiré |
| Vieillissement | aucun : ni rétention, ni sous-échantillonnage, ni suppression. « La purge du détail relève d'E1 » ([archive-protocol.md](../components/archive-protocol.md)) | l'archive grossit au rythme du brut, sans fin |
| Horizons par famille | écrits dans D13 ([ARCHITECTURE.md:282](../ARCHITECTURE.md)), codés nulle part. Le cœur ne connaît que `history_days` = 30 ([config.rs:153](../../crates/moli-os/src/config.rs#L153)) | à coder dans B1 (local) et E1 (archive) |
| États et énumérations | valeur en texte (`t`) ; durées par état indexées par ce texte. Plus de 64 états distincts dans une période font échouer la publication ([lib.rs:236](../../crates/moli-rollup/src/lib.rs#L236)). L'archive se met alors en panne ([store.rs:338](../../crates/moli-archive/src/store.rs#L338), [http.rs:398](../../crates/moli-archive/src/http.rs#L398)) et rejoue le lot à chaque redémarrage | une énumération bavarde peut bloquer l'archive |
| Énergie | l'archive reçoit des heures déjà dérivées des index par le cœur ; seul `Sum` est interdit sur la famille `energy` ([protocol.rs:277](../../crates/moli-archive/src/protocol.rs#L277)) | une moyenne d'index reste acceptée |
| Segments | aucun, ni dans l'archive ni dans `moli-rollup` | — |
| Lectures longues | `POST /query` : brut, heure, jour ou mois, 5 000 points au plus, 2 s, réponse en JSON brut ([manifest.rs:163](../../crates/moli-archive/src/manifest.rs#L163)) | la base existe pour D1 |
| Sauvegarde | `FILES` compte 23 fichiers, dont le nouveau `durations.json`, mais toujours sans `people.json`, `settings.json` ni `assistant.json` ([backup.rs:22](../../crates/moli-os/src/backup.rs#L22)) | chaque nouveau fichier d'état allonge la liste ; `maison.db` la supprime (V1 § 13.6) |

Les tickets ne portent pas les mêmes numéros partout. D13 nomme C1 la bascule du cœur et C3
l'archive ([ARCHITECTURE.md:187](../ARCHITECTURE.md)). Le plan, le code et `compose.yaml`
nomment C1 l'archive. Ce document suit le plan : C1 archive, C2 récent sur Turso, D1 lectures
fusionnées, E1 vieillissement de l'archive.

## 3. Quatre étages

| Étage | Où | Contient | Horizon | Technique | Pourquoi |
| --- | --- | --- | --- | --- | --- |
| Vif | mémoire du hub | valeur, trois instants, qualité | maintenant | structures en mémoire | une décision n'attend jamais le disque |
| État courant | `maison.db` (Turso) | une ligne par point | la dernière valeur | table écrite par lots de 60 s | démarrage sans l'historique (V1 § 13.7) |
| Récent | `moli.db` (Turso) | journal du jour, relevés rangés, heures creuses de la fenêtre, énergie de D13 | fenêtres de D13 : 7 à 31 jours, heures d'énergie 2 mois | lignes, rowid calculé | courbes rapides, export, purge par plages |
| Long terme | archive Parquet (`moli-archive`) | paliers par nature (§ 6) | des mois à toujours | colonnes typées, compressées, fichiers immuables | 7 à 10 octets par ligne (étude) |

- **Un étage ne garde jamais ce qui revient au suivant.** `moli.db` n'a plus de table « pour
  toujours » ; l'archive n'a pas d'état courant.
- **Les passages** : de la mémoire aux deux bases par lots (V1 § 13.7) ; du récent à l'archive par
  l'export (opérations compactes, lots de 5 000) ; d'un palier à l'autre par la compaction de
  l'archive (E1, § 11).
- **Une lecture longue** prend le récent dans `moli.db` et le reste dans l'archive (D1). Si
  l'archive est injoignable, on rend la partie locale, marquée comme telle
  ([ARCHITECTURE.md:291](../ARCHITECTURE.md)).
- **L'archive est en option** ([compose.yaml](../../compose.yaml)). Sans elle, il n'y a pas de long
  terme : le cœur garde ses fenêtres, et rien au-delà. Dès qu'on veut garder plus que les
  fenêtres de D13, l'archive devient nécessaire. Ce choix fait partie de la décision 1.

## 4. Le type décide

Chaque point déclare sa **nature**. Elle fixe tout le reste.

| Nature | Exemples | Encodage | Filtre à l'entrée de l'historique | Récent (`moli.db`) | Long terme (archive) | Jamais |
| --- | --- | --- | --- | --- | --- | --- |
| Mesure | température, humidité, luminosité, puissance | entier mis à l'échelle de la source | la bande (§ 5.1) | relevés à bande | heures creuses, puis jours creux ; segments à tolérance en option | éclaircir au hasard, moyenne simple |
| État | porte, fenêtre, serrure, alarme, mode, programme | code de dictionnaire (1 octet) | aucun ; anti-rebond au bord s'il est prouvé | transitions | transitions telles quelles, puis durées et nombre de transitions par jour | lisser, moyenner, échantillonner |
| Index cumulé | Linky, eau, gaz, production | entier mis à l'échelle | intervalle minimal seulement ; le dernier relevé avant une remise à zéro est toujours gardé | relevés, heures d'énergie (D13) | deltas par heure, puis par jour, puis par mois, avec l'index de fin | moyenne, « fin − début », durée par état |
| Occurrence | bouton, sonnette, passage | code du geste | aucun | événements | événements, puis nombre par jour | — |
| Texte, média | titre en cours, application | code de contenu | session : début, fin, contenu | sessions | sessions, selon la vie privée | écrire le texte à chaque changement |
| Diagnostic | qualité de lien, batterie | entier | la bande | relevés (7 j) | rien, comme le dit D13 | archiver le détail |
| Résumé de période | énergie du jour | — | non historisé : recalculé | — | — | stocker un total déjà tenu ailleurs |

- La nature se déclare dans le profil ou le pilote, jamais par l'unité (V1 § 7.5).
- Une valeur calculée prend la nature de ce qu'elle calcule ; un index n'alimente qu'un delta.
- Les familles de D13 (`states`, `power`, `climate`, `energy`, `diagnostic`, `other`) donnent les
  horizons. La nature donne la forme. Les deux se combinent au § 6.

## 5. Tout est segment

Un segment est un intervalle `[début, fin)` et une charge utile. Les deux règles d'une nature
disent ce que porte un segment et comment deux segments voisins se fusionnent. Les heures, jours
et mois ne sont que des fusions alignées sur le calendrier local, aux jours de 23 ou 25 heures,
comme `moli-rollup` le fait déjà.

| Nature | Charge utile | Fusion de deux voisins | Ce qu'on perd |
| --- | --- | --- | --- |
| Mesure | valeur tenue ; avec bande : moyenne pondérée par le temps, min, max, nombre de rapports | moyenne pondérée par les durées, min des min, max des max, nombres additionnés | la forme de la courbe dans le segment ; jamais ses extrêmes ni sa moyenne |
| État | code | seulement si le code est le même ; sinon les deux restent | rien |
| Index | delta, index de fin, remise à zéro éventuelle | deltas additionnés, index de fin du second, remises gardées | le moment exact d'un relevé intermédiaire, jamais une quantité |
| Occurrence | nombre (par code) | nombres additionnés | l'instant de chaque appui, au-delà de l'horizon du détail |
| Texte, média | code de contenu | seulement si c'est le même contenu | rien |

### 5.1 La mesure : le relevé à bande

C'est ta proposition « valeur moyenne, date de début et de fin », appliquée dès l'entrée de
l'historique.

```text
pour chaque rapport (t, v) d'une mesure p :
    b ← bande[p] ; b.aire += b.tenue × (t − b.dernier_t)      -- la valeur précédente a tenu jusqu'à t
    si |v − b.écrite| ≥ bande_morte(p) et t − b.début ≥ intervalle_min(p) :
        écrire (p, t, v, b.n > 1 ? (b.aire / (t − b.début), b.min, b.max, b.n) : rien)
        bande[p] ← (début = t, écrite = v, tenue = v, min = max = v, aire = 0, n = 1, dernier_t = t)
    sinon :                                                  -- absorbé : rien n'est écrit
        b.min ← min(b.min, v) ; b.max ← max(b.max, v) ; b.n += 1 ; b.tenue ← v ; b.dernier_t ← t
à chaque heure close h, pour chaque bande ouverte avec n > 1 :
    écrire (p, h, b.tenue, résumé de [b.début, h)) ; rouvrir la bande à h
```

- **Ce que dit un relevé écrit** : « à partir de t, la valeur est v ; depuis le relevé précédent,
  elle a varié entre min et max, avec cette moyenne ». Le relevé qui ferme un intervalle en porte
  le résumé : l'écriture reste en ajout seul.
- Un intervalle sans rapport absorbé n'a pas de résumé : ses colonnes restent vides.
- **Ce que ça remplace** : la bande morte, l'intervalle minimal « le dernier gagne » et
  l'accumulateur horaire séparé de la V1 (§ 6, § 12.3, § 12.5). Les extrêmes et la moyenne restent
  exacts, et les heures se déduisent exactement des intervalles, puisque la bande est coupée à
  chaque heure.
- **Coût** : une ligne seulement quand la valeur sort de la bande, plus au plus une coupure par
  heure pour un point bavard. Les 34 séries qui font 78,2 % des relevés (série profil) sont
  justement celles que la bande résume le plus.

### 5.2 L'état : les transitions, sans perte

Une porte s'ouvre à 08:03 et se ferme à 08:05 : on garde exactement ces deux transitions.

- **Forme** : `(point, début, code)`. La fin est le début de la transition suivante, et la
  disponibilité dit si la source vivait.
- On ne lisse jamais un état, et on ne le ré-échantillonne pas.
- Pour les statistiques, des **durées par jour** s'y ajoutent (`point, jour, code, ms,
  transitions`) : « porte ouverte 2 h 10 hier, 14 ouvertures ».
- Avec des codes, la limite des 64 états de `moli-rollup` devient une alerte sur la taille du
  dictionnaire, et ne bloque plus jamais l'archive (§ 7).

### 5.3 L'index cumulé : l'élagage

On ne moyenne jamais un cumul : **on l'élague**.

- On ne garde que des deltas de plus en plus larges : par heure, puis par jour, puis par mois.
- L'élagage ne perd aucune quantité. Les remises à zéro et les trous sont traités avant, par les
  règles de `meter.rs` (V1 § 7.5) : un delta n'est jamais négatif.
- L'**index de fin** de chaque période reste, pour comparer au compteur.
- Les heures d'énergie que le cœur envoie déjà à l'archive sont le premier palier.

### 5.4 Les segments à tolérance (option)

Pour garder plus de détail que des heures pendant des mois, on fusionne les intervalles voisins
tant que `max − min ≤ 2ε` (ε : la tolérance du point).

- **Garantie** : toute valeur réelle du segment est dans `[min, max]`.
- C'est l'algorithme des historiens industriels (compression par bande, PI). Sa durée est variable :
  long la nuit, court quand la valeur bouge.
- **Gain inconnu ici** : le profil n'a que des comptes, pas les valeurs. Il se mesure sur un export
  anonymisé avec valeurs (§ 17) avant de décider s'il prend place entre le détail et les heures.

## 6. Vieillir petit à petit : les paliers

Pour chaque nature et chaque famille, ce que garde chaque étage. Les horizons marqués D13 sont
décidés, les autres sont proposés. « Toujours » se décide avec le propriétaire, pour la vie
privée autant que pour la place.

| Nature (famille D13) | Récent, `moli.db` | Archive, palier 1 | Palier 2 | Palier 3 |
| --- | --- | --- | --- | --- |
| Mesure, puissance (`power`) | relevés à bande, 7 j | relevés à bande, 30 j (détail D13) | heures creuses, 10 ans (D13) | jours creux, toujours |
| Mesure, climat (`climate`) | 14 j | 90 j | heures creuses, 5 ans (D13) | jours creux, toujours |
| États de sûreté : portes, fenêtres, serrures, alarme (`states`) | transitions, 31 j | transitions, toujours | durées par jour, toujours | — |
| Autres états, occurrences (`states`) | 31 j | transitions et événements, 365 j (D13) | durées, transitions et nombres par jour, toujours | — |
| Index cumulé (`energy`) | relevés, heures 2 mois, totaux 62 j et 13 mois (D13) | deltas par heure et index de fin, 10 ans (D13) | jours, toujours | mois, toujours |
| Diagnostic | 7 j | rien (D13) | — | — |
| Autres mesures (`other`) | 7 j | 30 j | heures creuses, 5 ans | jours creux, toujours |
| Texte, média | sessions de la fenêtre | sessions, selon la vie privée : court et effaçable | durées et nombres par contenu | — |

**Ce que ça pèse**, charge ×1, série profil :

| Forme | Lignes par jour | Mo par an en Parquet |
| --- | --- | --- |
| Relevés bruts | 164 126 | 430 à 573 |
| Heures denses : chaque point, chaque heure | 36 288 | 88 à 115 |
| **Heures creuses** : seulement si la valeur a changé | 4 827 | **11,7 à 15,2** |
| Jours denses | 1 512 | 3,7 à 4,8 |
| **Jours creux** | 506 | **1,2 à 1,6** |

- Dix ans d'heures creuses tiennent dans 120 à 150 Mo, quand une seule année de bruts en pèse 430 à
  573.
- La grille horaire dense de la V1 aurait écrit 7,5 fois plus de lignes que les heures creuses, et
  presque toutes identiques : 1 006 séries sur 1 512 n'ont pas changé de la journée.
- Les fenêtres de détail tournent sans grossir. Trente jours de bruts pèsent 35 à 47 Mo, et
  90 jours 106 à 142 Mo (*estimation* : lignes × octets de l'étude).
- Ces octets supposent le Parquet typé du § 8. En `archive_v1`, chaque ligne pèse bien plus, et
  reste à mesurer (§ 17).

**Qui le fait : l'archive (E1)**, par sa compaction. Les fichiers Parquet sont immuables : vieillir
un mois revient à réécrire ses paliers puis à basculer le manifeste. C'est déjà le modèle de
l'archive : révisions, `current.json`, ramasse-miettes.

```text
chaque nuit, pour chaque (nature, mois) dont l'âge dépasse l'horizon de son palier :
    lire les lignes du palier courant
    écrire le palier suivant (fusion selon la nature, § 5)
    vérifier : mêmes totaux (deltas, durées, nombres), mêmes min et max, couverture identique
    publier une révision du manifeste, puis retirer l'ancien palier au ramasse-miettes
jamais : retirer un palier avant que le suivant soit écrit, vérifié et publié
raccourcir un horizon demande la validation humaine de B1, comme en local
```

## 7. Énumérations et textes : des codes de bout en bout

- **Dictionnaire en ajout seul**, par énumération : clé stable → code (V1 § 4.3). Le code circule
  partout : relevés de `moli.db`, opérations compactes, archive, durées par état.
- **Un code nouveau voyage avant sa première valeur.** Une opération `dictionnaire` part dans le
  même lot d'export, comme le catalogue. L'archive peut donc toujours décoder.
- **Ce que ça change** :
  - 9,1 % des changements sont des textes courts, presque tous des énumérations (série profil) ;
  - un octet au lieu d'un mot ;
  - plus de valeur `t` pour une énumération dans l'archive ;
  - des durées par état indexées par un entier ;
  - un nombre d'états connu d'avance : la limite des 64 états devient une alerte sur le
    dictionnaire, jamais une panne de publication.
- **Textes de contenu** (titres, applications) : un dictionnaire `contenu(id, titre, type,
  application)` et des sessions `(appareil, début, fin, contenu)`. Un titre revu dix fois n'est
  stocké qu'une fois.
- **Effacer pour la vie privée** : on efface l'entrée du dictionnaire. Toutes les sessions gardent
  leur forme, sans le titre, et aucun fichier de l'archive n'est réécrit.
- **Texte libre** (messages) : borné à 4 000 caractères au bord ; non historisé, ou gardé peu de
  temps.

## 8. Le long terme en Parquet typé (`archive_v2`)

Un jeu de fichiers par nature et par mois, chaque fichier trié par (point, début) :

| Fichier | Colonnes | Palier |
| --- | --- | --- |
| `mesures` | point (INT32), début (INT64), valeur (INT32 mis à l'échelle, ou DOUBLE), moyenne, min, max (facultatifs), n | 1 : relevés à bande |
| `mesures_heures`, `mesures_jours` | point, début, moyenne, min, max, couvert_ms, changements | 2 et 3 : creux |
| `etats` | point, début, code (INT16) | transitions |
| `etats_jours` | point, jour, code, ms, transitions | durées |
| `index` | point (compteur), heure, delta (INT64 mis à l'échelle), index de fin, coût, origine | 1 : heures d'énergie |
| `index_jours`, `index_mois` | point, période, delta, index de fin, coût | 2 et 3 |
| `occurrences`, `occurrences_jours` | point, instant ou jour, code, nombre | — |
| `sessions` | point, début, fin, contenu (INT32) | — |
| `catalogue`, `dictionnaires` | points, natures, unités, échelles ; codes → clés | — |

- **Écriture** :
  - compression zstd ;
  - dictionnaire activé ;
  - groupes de dizaines de milliers de lignes ;
  - statistiques sur `point` et `début`, pour sauter les groupes inutiles ;
  - aucune colonne JSON.
- **Le NDJSON reçu** reste où il est aujourd'hui : dans le journal de réception, jusqu'à sa
  publication. Il sert à l'idempotence, pas au stockage.
- **Pourquoi pas Turso pour le long terme** : une ligne SQLite coûte environ 21 à 39 octets, plus
  son b-arbre ; une ligne Parquet typée, 7 à 10. Les fichiers immuables se sauvegardent et se
  déplacent sur un VPS sans transformation.
- **À soumettre à la revue de D13 et de l'archive** :
  - `archive_v1` → `archive_v2` se convertit une fois, en relisant les fichiers existants ;
  - `deny_unknown_fields` impose de mettre l'archive à jour avant le cœur
    ([archive-protocol.md](../components/archive-protocol.md)) ;
  - les réponses de `POST /query` se construisent depuis les colonnes, et non plus depuis le JSON
    stocké.

## 9. Le récent dans `moli.db` : seulement le récent

- **Contenu** :
  - les opérations compactes, qui servent de journal du jour et d'export (V1 § 13.8) ;
  - les relevés à bande, rangés par point à l'heure close ;
  - les heures creuses de la fenêtre, pour les courbes de 7 à 31 jours ;
  - l'énergie de D13 : heures sur 2 mois, totaux de jours et de mois ;
  - le catalogue, les dictionnaires, la garde B1, le progrès de la nuit.
- **Retiré par rapport à la V1** : `agg_day`, `state_day`, et `agg_hour` au-delà de la fenêtre.
  Le long terme est dans l'archive.
- **Purge** : chaque nuit, par famille, aux horizons de D13. Les heures creuses suivent leurs
  relevés.
- **Taille en régime établi** : environ 70 à 90 Mo pour la charge ×1 (*estimation*), contre
  « des centaines de Mo » dans la V1.
  - Relevés à bande, pondérés par les fenêtres : ≈ 1,6 million de lignes, soit ≈ 40 Mo.
  - Journal compact gardé 7 jours après publication : ≈ 28 Mo.
  - Heures creuses sur 31 jours : ≈ 4 Mo.

## 10. État courant et double écriture

Inchangés depuis la V1 (§ 13.7) :

- `maison.db` garde une ligne complète par point et suffit au démarrage ;
- `moli.db` reçoit l'historique en parallèle ;
- au démarrage, une réconciliation silencieuse garde la valeur la plus récente ;
- aucune transaction commune aux deux fichiers.

Coût mesuré (série écritures) :

- `point_state` par lots de 60 s : ≈ 65 Mo écrits par jour, contre ≈ 117 Mo pour `state.json`
  aujourd'hui (*estimation*) ;
- avec l'historique : ≈ 0,32 Go écrits par jour, contre ≈ 3,1 Go pour le chemin de D13.

## 11. La nuit et la compaction

| Où | Quand | Quoi |
| --- | --- | --- |
| Cœur, au fil de l'eau | lots de 60 s et de 2 s | état courant, relevés à bande, opérations compactes |
| Cœur, à l'heure close | une transaction | rangement de l'heure par point, heures creuses locales |
| Cœur, la nuit (V1 § 13.9) | 03 h 30 – 06 h 00 | vérifier les heures, purger les fenêtres de D13, sauvegarder, point de contrôle |
| Archive, à la réception | lot par lot | publication des lignes reçues dans le palier 1 de leur nature |
| Archive, la nuit (E1) | après la nuit du cœur | paliers suivants (§ 6), vérification, manifeste, ramasse-miettes |

- Le cœur ne calcule plus d'agrégat de long terme. Ses heures creuses ne servent qu'aux courbes
  locales et partent avec la purge.
- Les tranches de nuit restent courtes et reprenables, des deux côtés (V1 § 13.9).

## 12. Algorithmes

| Algorithme | Où | Coût | Garantie |
| --- | --- | --- | --- |
| Relevé à bande (§ 5.1) | rédacteur de l'historique | O(1) par rapport | extrêmes et moyenne exacts par intervalle |
| Heure creuse (§ 5.1) | rangement à l'heure close | une ligne par point qui a bougé | heure exacte, puisque la bande est coupée à l'heure |
| Transitions (§ 5.2) | rédacteur | une ligne par changement | sans perte |
| Élagage d'un index (§ 5.3) | énergie, puis archive | une ligne par période | totaux exacts, jamais de delta négatif |
| Fusion de paliers (§ 6) | archive (E1) | lecture et écriture d'un mois par nature | totaux, min, max et couverture vérifiés avant retrait |
| Dictionnaire (§ 7) | bord et rédacteur | un code par valeur, une entrée par clé nouvelle | décodable partout, effaçable en un point |
| Segments à tolérance (§ 5.4) | archive, en option | une passe par mois | toute valeur dans `[min, max]` |

Fusion de deux segments de mesure voisins, la même opération à tous les paliers :

```text
fusion(a, b) où a.fin = b.début :
    durée ← (a.fin − a.début) + (b.fin − b.début)
    moyenne ← (a.moyenne × durée(a) + b.moyenne × durée(b)) / durée      -- pondérée par le temps
    min ← min(a.min, b.min) ; max ← max(a.max, b.max) ; n ← a.n + b.n
    couvert ← a.couvert + b.couvert                                      -- un trou n'est pas zéro
```

## 13. Ce qui ne change pas depuis la V1

| Sujet | V1 |
| --- | --- |
| Valeur canonique au bord, échelle de la source, dictionnaires | § 4 |
| Trois instants : observé, reçu, changé | § 5 |
| Fraîcheur, qualité, pannes en cascade | § 8 |
| Équipements logiques par liaisons | § 9 |
| Conditions avec durée, échéancier unique | § 10, § 12.7 |
| Familles particulières : Linky, notes scolaires, médias | § 11 |
| Deux bases, règle des noms stables | § 13.1 à 13.6 |
| Double écriture, coût réel d'une écriture, nuit | § 13.7 à 13.9 |
| État métier en tables | § 14.7 |

## 14. Changements de base

### 14.1 `moli.db` : le récent

```sql
CREATE TABLE samples (            -- relevés à bande, rangés par point à l'heure close
    id   INTEGER PRIMARY KEY,     -- (point << 44) + ts en ms
    seq  INTEGER NOT NULL,
    num  ANY,                     -- valeur à partir de ts (code pour un état)
    txt  TEXT,
    q    INTEGER,                 -- marqueur rare
    moy  ANY, lo ANY, hi ANY,     -- résumé de l'intervalle que ce relevé ferme ; NULL sans bande
    n    INTEGER                  -- rapports absorbés dans cet intervalle
) STRICT;
CREATE TABLE hours (              -- heures creuses de la fenêtre seulement
    id      INTEGER PRIMARY KEY,  -- (point << 24) + heures depuis 1970
    moy ANY, lo ANY, hi ANY,
    covered INTEGER NOT NULL,     -- ms couvertes par une source fraîche
    changes INTEGER NOT NULL
) STRICT;
-- ops : comme la V1 § 14.6, avec les mêmes colonnes de bande (moy, lo, hi, n)
-- inchangés : catalog, dict_entry, meter_state, energy_hour, energy_total, archive_ack, meta,
--             progress, garde B1
-- retirés : agg_day, state_day, et agg_hour au-delà de la fenêtre
```

Les colonnes de bande coûtent un octet d'en-tête chacune quand elles sont vides : environ
4 octets de plus par relevé sans bande. C'est à vérifier au banc. Une table à part pour les
seuls résumés est l'autre option.

### 14.2 L'archive : `archive_v2`

Les colonnes du § 8. Les lignes de résumé d'aujourd'hui (`hour`, `day`, `month` dans `stats`)
deviennent les paliers 2 et 3 de chaque nature. Le brut reçu devient le palier 1. Un palier n'en
remplace un autre qu'à son horizon.

### 14.3 `maison.db`

Inchangée depuis la V1 (§ 14.4) : `point_state` complet, appareils, équipements, échéances.

## 15. Ce qu'on ne fait pas

- Lisser, moyenner ou ré-échantillonner un état.
- Moyenner un index, ou calculer une période par « index de fin − index de début ».
- Garder le long terme dans `moli.db`.
- Écrire une grille horaire dense pour des points qui ne changent pas.
- Écrire le nom du point ou l'opération JSON sur chaque ligne du long terme.
- Retirer un palier avant que le suivant soit écrit, vérifié et publié.
- Stocker le texte d'une énumération ou un titre répété.
- Laisser une valeur inattendue (65ᵉ état) mettre l'archive en panne.
- Et tout ce que la V1 écarte déjà (V1 § 15).

## 16. Décisions à prendre

1. **Quatre étages** : le long terme dans l'archive seulement, `moli.db` limitée au récent
   (proposé) ? Et l'archive, aujourd'hui en option, lancée par défaut dès qu'on veut un long
   terme ?
2. **`archive_v2` typé** : codes, dictionnaire, zstd, sans JSON par ligne. À présenter à la revue de
   D13 et de l'archive ?
3. **Paliers et horizons par nature** (table du § 6) : faut-il garder pour toujours les transitions
   de sûreté et les jours creux ?
4. **Relevé à bande** à la place de l'intervalle minimal « le dernier gagne » (proposé) ? Colonnes
   de bande dans `samples` ou table à part ?
5. **Segments à tolérance** : un palier entre le détail et les heures, à décider après la mesure du
   § 17 ?
6. **Énumérations codées dans le protocole d'export** : opération `dictionnaire`, valeur codée. Avec
   `deny_unknown_fields`, l'archive doit être mise à jour d'abord.
7. **Qui fait vieillir le long terme** : l'archive, par E1 (proposé), ou le cœur ?
8. **Numérotation des tickets** : aligner D13 sur le plan (C1 archive, C2 récent) ?
9. Les décisions encore ouvertes de la V1 (§ 16) restent valables.

## 17. Mesures et expériences

- **Profil de l'historique** : fait, série `2026-10-21T151507Z` ([rapport](../reports/profil-historique-2026-10-21T151507Z.md)).
- **Écritures** : fait, série `2026-10-21T114647Z` ([rapport](../reports/mesure-ecritures-2026-10-21T114647Z.md)).
- **`archive_v1` contre `archive_v2`** : octets par ligne à la charge ×1, avec l'archive réelle. On
  peut partir de `tools/archive-tests`, qui mesure aujourd'hui la mémoire, le binaire et l'image,
  et y ajouter un relevé de taille.
- **Bande et segments à tolérance** : sur un export anonymisé **avec valeurs**, que le propriétaire
  produit en local depuis la copie privée. On mesure le nombre de lignes et l'erreur, point par
  point. Un scénario s'ajoutera à `tools/storage-writes` quand cet export existera, avec son
  rapport daté.
- **E1** : durée d'un mois de paliers sur un Pi Zero 2 W, et vérification des totaux avant retrait.
- **Durée par état codée** : rejouer une énumération à plus de 64 valeurs, pour vérifier qu'elle
  ne met plus l'archive en panne.

## 18. Méthode

- Premiers principes : ce que chaque nature doit permettre (lire, décider, consolider), avant la
  technique de stockage.
- Analyse morphologique : pour chaque nature, la forme, le filtre, les paliers, et ce qu'on ne fait
  jamais (§ 4 à 6).
- Analogies :
  - RRDtool : archives consolidées par fonction ;
  - historiens industriels (PI) : exception puis compression par bande ;
  - TimescaleDB et InfluxDB : agrégats continus, compression par série, rétention par palier ;
  - Gorilla : horodatages en différences, valeurs compressées par colonnes ;
  - arbres LSM : écrire en ajout, ranger plus tard.
- Mesure plutôt qu'estimation, avec des outils versionnés et des séries datées (§ 17).
- Inversion : comment rater (§ 15).
