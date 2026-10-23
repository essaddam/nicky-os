# Valeurs, appareils et stockage, V3 : la proposition consolidée

> **Proposition V3, non livrée.** 21 octobre 2026, références au code vérifiées sur `bd1714d`.
> Elle remplace la [V1](valeurs-appareils-stockage.md) et la [V2](valeurs-appareils-stockage-v2.md)
> du même jour, gardées comme historique. Aucun code de production n'a été modifié. Tous les
> chiffres mesurés viennent de trois séries datées, produites par
> [`tools/storage-writes`](../../tools/storage-writes/README.md) :
>
> - [`mesure-ecritures-2026-10-21T114647Z`](../reports/mesure-ecritures-2026-10-21T114647Z.md) :
>   ce que coûte chaque façon d'écrire ;
> - [`profil-historique-2026-10-21T151507Z`](../reports/profil-historique-2026-10-21T151507Z.md) :
>   qui écrit l'historique, et combien ;
> - [`formes-parquet-2026-10-21T200551Z`](../reports/formes-parquet-2026-10-21T200551Z.md) : ce
>   que pèse chaque forme en Parquet, mesuré et non plus estimé.
>
> Page d'explication illustrée : [valeurs-appareils-stockage-v3.html](valeurs-appareils-stockage-v3.html).

## Ce que la réanalyse a changé

J'ai relu la V2 en cherchant ce qu'elle affirmait sans preuve, ce qu'elle compliquait sans besoin,
et ce qui contredisait une décision prise. Chaque ligne donne la raison, mesurée ou lue dans le
code.

| Sujet | V2 | V3 | Pourquoi |
| --- | --- | --- | --- |
| Le premier levier | les paliers (heures creuses, jours) | **le format de l'archive** : colonnes typées, codes, dictionnaire, zstd | mesuré : le format actuel coûte 6,7 fois plus que le typé pour le brut, et **38 fois plus pour les heures** (série formes). Aucun palier ne rattrape ça |
| Heures « creuses » | n'écrire une heure que si la valeur a changé | **les heures denses que `moli-rollup` calcule déjà** | mesuré : 5,5 fois moins de lignes, mais seulement 17 contre 12 Mo par an. Parquet compresse presque gratuitement une heure sans changement. La forme creuse aurait exigé l'historique de disponibilité pour lire un trou |
| Coupure de la bande à chaque heure | une ligne de plus par point bavard et par heure, pour des heures « exactes » | **supprimée** : l'archive calcule les heures depuis les lignes à bande ; une heure à cheval sur un intervalle prend sa part au prorata, avec une erreur bornée par `[min, max]` de l'intervalle | la bande est la précision déclarée du point : une heure exacte « à la bande près » est exacte |
| Agrégats locaux (`hours`, `agg_hour`) dans `moli.db` | gardés pour les courbes de 7 à 31 jours | **supprimés** | 31 jours de lignes à bande se lisent en quelques millisecondes (fiche 7 jours : 18 ms au banc D13, avant la bande). Les heures viennent de l'archive, ou se calculent à la volée en secours |
| Transitions de sûreté « pour toujours » | toujours | **365 jours de détail (D13), puis durées et nombres par jour** ; « toujours » devient un choix explicite du propriétaire | vie privée : les ouvertures de porte sur dix ans sont un journal de présence. D13 fixe 365 jours ; on ne le dépasse pas en silence |
| Codes dans le protocole d'export | une opération `dictionnaire` et des valeurs codées | **le dictionnaire voyage dans le catalogue** (déjà révisionné, `PUT /catalog`) ; la valeur voyage en code. Variante minimale : clé texte et dictionnaire Parquet | les octets ne sont pas la raison des codes : le dictionnaire Parquet les obtient aussi. Les vraies raisons sont la limite des 64 états et l'identité stable d'une valeur |
| Lectures longues | « D1 » | **un algorithme** : heures et jours par l'archive, brut et bande en local, secours local sans archive | une courbe de 90 jours doit avoir une seule réponse |
| Bande | gain affirmé | **gain mesuré, sur des valeurs synthétiques** : 4,6 fois moins de lignes de mesure ; à confirmer sur des valeurs réelles | une affirmation devient une mesure, avec sa limite |

Ce qui ne change pas depuis la V1 et la V2 est listé au § 13.

## En bref

1. **Quatre étages, chacun sa technique** : la mémoire du hub, `maison.db` (état courant),
   `moli.db` (le récent, Turso, 7 à 31 jours), l'archive (le long terme, Parquet). Un étage ne
   garde jamais ce qui revient au suivant.
2. **Le format de l'archive d'abord.** Le format actuel répète le nom du point et l'opération en
   JSON sur chaque ligne. Typé, codé et compressé en zstd, le long terme pèse 6,7 à 38 fois moins
   (mesuré).
3. **Le type décide** : la nature déclarée d'un point fixe son encodage, son filtre, ses paliers,
   et ce qu'on ne fait jamais.
4. **Tout est segment** : un intervalle `[début, fin)` et une charge utile selon la nature.
   Vieillir, c'est fusionner des voisins.
5. **Mesures** : le relevé à bande au cœur (il résume ce qu'il a absorbé : moyenne, min, max),
   puis les heures de `moli-rollup`, puis les jours. 4,6 fois moins de lignes dans le modèle
   mesuré.
6. **États** (porte, serrure, mode) : transitions gardées telles quelles, jamais lissées ; durées
   par jour en plus.
7. **Cumuls** (énergie, eau) : élagués, jamais moyennés. Deltas par heure, puis jour, puis mois,
   index de fin gardé.
8. **Énumérations et textes** : des codes partout. Un titre n'est stocké qu'une fois et s'efface
   en un point.
9. **Double écriture et nuit** (V1) : ≈ 0,32 Go écrits par jour contre ≈ 3,1 Go pour le chemin
   de D13 (mesuré).
10. **Le long terme, pour la charge ×1 et le format typé** : 17 Mo par an d'heures, 4 Mo de
    jours, 175 Mo par an de détail à bande, 368 Mo de brut (mesuré, valeurs synthétiques). En
    format actuel : 650 Mo d'heures et 2,5 Go de brut.

## 1. Le besoin, en une page

Ce que tu as demandé au fil de la journée, dans l'ordre où la proposition y répond :

| Demande | Réponse | § |
| --- | --- | --- |
| Valeur reçue ≠ stockée ≠ affichée ; énumérations en codes | valeur canonique au bord, code stocké, libellé à l'affichage | 4 |
| Dernière mise à jour ≠ dernier changement | trois instants : observé, reçu, changé | 4 |
| Anti-rebond par valeur | au bord, là où le rebond est prouvé | 4 |
| Combien de temps on garde, combien de temps la valeur reste utilisable, lisser ou éclaircir | paliers par nature, fraîcheur attendue, jamais d'éclaircissement | 6, 7 |
| « Luminosité < 200 pendant 10 min » | condition avec durée, échéancier unique (V1 § 10) | 13 |
| Appareils physiques et logiques, panne, mode dégradé | équipements par liaisons, cascade de pannes (V1 § 8, 9) | 13 |
| Pas de double stockage, liaison visible | une copie par donnée ; recouvrement borné à une ligne par point | 5, 11 |
| Valeurs cumulées à l'agrégation | un cumul s'élague, jamais ne se moyenne | 6.3 |
| Algorithmes expliqués et améliorés | § 6, 8, 10, 12 | — |
| Historique dans une base, fonctionnement direct dans une autre | `moli.db` et `maison.db`, un rédacteur chacune | 5 |
| Turso : embarqué, multithread | deux bases, lecteurs sur leurs connexions ; vecteurs en BLOB (V1 § 13.5) | 13 |
| Double écriture et travaux de nuit | oui, à trois conditions (V1 § 13.7 à 13.9) | 11 |
| Le long terme dans la base optimisée dont on a parlé | l'archive Parquet, en format typé | 8 |
| Agréger petit à petit selon le type ; valeur moyenne avec début et fin | segments et paliers par nature | 6, 7 |
| Porte : on garde, on ne lisse pas | transitions sans perte | 6.2 |
| Énergie : pas de moyenne, on élague | deltas élagués, index de fin | 6.3 |
| Énumérations : pas de textes coûteux | codes de bout en bout | 9 |
| Outils dans `tools/`, datés, reliés aux rapports | `tools/storage-writes`, trois séries datées | 15 |

## 2. Ce qui existe (code à `bd1714d`)

| Sujet | Aujourd'hui | Conséquence |
| --- | --- | --- |
| Cœur | SQLite (`history.db`, `energy.db`). Turso (`moli.db`, C2) en développement, pas intégré | la proposition vise la cible ; décidée avant C2, rien n'est migré deux fois |
| État courant | `state.json` réécrit en entier chaque minute ; valeurs relues réémises comme des changements au démarrage ([hub.rs:2144](../../crates/moli-runtime/src/hub.rs#L2144)) | ≈ 117 Mo écrits par jour (*estimation*), un relevé en double par point à chaque redémarrage |
| Archive | `moli-archive` fusionné : binaire à part, HTTP, en option dans `compose.yaml`. Aucun crate du cœur ne l'appelle | le long terme existe, mais rien ne l'alimente ; sans archive, pas de long terme |
| Format `archive_v1` | une seule forme de ligne : `entity` (nom du point en texte), `ts`, `seq`, `kind` (texte), `payload` (JSON complet de l'opération ou du résumé), `num`, `cost`, `covered_ms` ([parquet.rs:27](../../crates/moli-archive/src/parquet.rs#L27)). Snappy, dictionnaire désactivé, groupes de 256 lignes ([parquet.rs:190](../../crates/moli-archive/src/parquet.rs#L190)) | **mesuré** : 40,7 octets par relevé brut et 59,3 par heure résumée, contre 6,1 et 1,3 en colonnes typées |
| Résumés | heures, jours et mois calculés par l'archive depuis le brut, avec `moli-rollup` (moyenne pondérée, min, max, somme, nombre, durées par état, couverture, qualité, origine) | le bon calcul existe ; il est rangé dans le mauvais format |
| Vieillissement | aucun : rien n'est retiré. « La purge du détail relève d'E1 » ([archive-protocol.md](../components/archive-protocol.md)) | l'archive grossit au rythme du brut |
| Horizons par famille | écrits dans D13 ([ARCHITECTURE.md:282](../ARCHITECTURE.md)), codés nulle part ; le cœur ne connaît que `history_days` = 30 | à coder dans B1 et E1 |
| États, énumérations | valeur en texte ; plus de 64 états distincts dans une période font échouer la publication, l'archive se met en panne et rejoue le lot à chaque redémarrage ([lib.rs:236](../../crates/moli-rollup/src/lib.rs#L236), [store.rs:338](../../crates/moli-archive/src/store.rs#L338)) | une énumération bavarde peut bloquer l'archive |
| Énergie | l'archive reçoit des heures dérivées des index par `meter.rs` ; seule `Sum` est interdite sur un index ([protocol.rs:277](../../crates/moli-archive/src/protocol.rs#L277)) | une moyenne d'index reste acceptée |
| Lectures longues | `POST /query` : brut, heure, jour ou mois ; réponse en JSON brut ([manifest.rs:163](../../crates/moli-archive/src/manifest.rs#L163)) ; D1 pas fait | la base de la lecture fusionnée existe |
| Tickets | D13 nomme C1 la bascule du cœur et C3 l'archive ; le plan, le code et `compose.yaml` nomment C1 l'archive | ce document suit le plan : C1 archive, C2 récent sur Turso, D1 lectures, E1 vieillissement |

## 3. Quatre étages

| Étage | Où | Contient | Horizon | Technique |
| --- | --- | --- | --- | --- |
| Vif | mémoire du hub | valeur, trois instants, qualité, bandes ouvertes | maintenant | structures en mémoire : une décision n'attend jamais le disque |
| État courant | `maison.db` (Turso) | une ligne complète par point ; appareils, équipements, état métier, échéances | la dernière valeur | table écrite par lots de 60 s ; suffit au démarrage |
| Récent | `moli.db` (Turso) | journal du jour, relevés à bande rangés par point, énergie de D13, catalogue, dictionnaires, garde B1 | fenêtres de D13 : 7 à 31 jours ; heures d'énergie 2 mois | lignes, rowid calculé ; courbes, export, purge par plages |
| Long terme | archive Parquet (`moli-archive`) | paliers par nature (§ 7) | des mois à toujours | colonnes typées, zstd, fichiers immuables |

- **Un étage ne garde jamais ce qui revient au suivant.** `moli.db` n'a plus d'agrégat ; l'archive
  n'a pas d'état courant.
- **Les passages** : de la mémoire aux deux bases par lots (§ 11) ; du récent à l'archive par
  l'export, en lots de 5 000 opérations ; d'un palier à l'autre par la compaction de l'archive
  (E1, § 7).
- **L'archive est en option aujourd'hui** ([compose.yaml](../../compose.yaml)). Sans elle, il n'y a
  pas de long terme. Dès qu'on veut garder plus que les fenêtres de D13, elle devient nécessaire :
  c'est la décision 1.

## 4. La valeur : canonique, datée, qualifiée

Repris de la V1 (§ 4, 5, 8), inchangé :

| Sujet | Règle |
| --- | --- |
| Canonique au bord | le pilote ou le profil traduit une fois : sens (« ouvert » vaut vrai pour tout contact), échelle de la source (21,57 °C → `2157`), unité, code d'énumération, texte borné à 4 000 caractères, `Kind` contrôlé |
| Stocké | le code, jamais le libellé ; le libellé vient de la sémantique et de la langue à l'affichage |
| Trois instants | observé (mesure selon la source), reçu (dernier rapport, même identique), changé (dernier changement) |
| Qualité | bonne, incertaine (en retard, ou relue après redémarrage), mauvaise (perdue, intégration en panne) ; un automatisme ne déclenche jamais sur une valeur mauvaise |
| Fraîcheur | un rapport attendu par appareil (Linky ≈ 3 min, contact sur pile 24 h) ; une échéance par appareil dans l'échéancier unique |
| Anti-rebond | au bord, seulement là où le rebond est prouvé |
| Nature | déclarée par le profil ou le pilote, jamais déduite de l'unité |

## 5. Le type décide

| Nature | Exemples | Encodage | Filtre à l'entrée de l'historique | Récent (`moli.db`) | Long terme (archive) | Jamais |
| --- | --- | --- | --- | --- | --- | --- |
| Mesure | température, humidité, luminosité, puissance | entier mis à l'échelle | la bande (§ 6.1) | relevés à bande | relevés à bande, puis heures, puis jours (`moli-rollup`) | éclaircir au hasard, moyenne simple |
| État | porte, fenêtre, serrure, alarme, mode, programme | code (1 octet) | aucun | transitions | transitions telles quelles, puis durées et nombre de transitions par jour | lisser, moyenner, échantillonner |
| Index cumulé | Linky, eau, gaz, production | entier mis à l'échelle | intervalle minimal seulement ; le dernier relevé avant une remise à zéro est toujours gardé | relevés, heures et totaux d'énergie (D13) | deltas par heure, puis jour, puis mois, avec l'index de fin | moyenne, « fin − début », durée par état |
| Occurrence | bouton, sonnette, passage | code du geste | aucun | événements | événements, puis nombre par jour | — |
| Texte, média | titre en cours, application | code de contenu | session : début, fin, contenu | sessions | sessions courtes et effaçables, puis durées par contenu | écrire le texte à chaque changement |
| Diagnostic | qualité de lien, batterie | entier | la bande | relevés, 7 jours | rien (D13) | archiver le détail |
| Résumé de période | énergie du jour | — | non historisé, recalculé | — | — | stocker un total tenu ailleurs |

Les familles de D13 (`states`, `power`, `climate`, `energy`, `diagnostic`, `other`) donnent les
horizons ; la nature donne la forme. Les deux se combinent au § 7.

## 6. Tout est segment

Un segment est un intervalle `[début, fin)` et une charge utile. Pour chaque nature, une règle dit
ce que porte un segment et comment deux voisins se fusionnent. Les heures, jours et mois ne sont
que des fusions alignées sur le calendrier local, aux jours de 23 ou 25 heures, comme
`moli-rollup` le fait déjà.

| Nature | Charge utile | Fusion de deux voisins | Ce qu'on perd |
| --- | --- | --- | --- |
| Mesure | valeur tenue ; avec bande : moyenne pondérée par le temps, min, max, rapports absorbés | moyenne pondérée par les durées, min des min, max des max, nombres additionnés | la forme de la courbe dans le segment ; jamais ses extrêmes ni sa moyenne |
| État | code | seulement si le code est le même | rien |
| Index | delta, index de fin, remise à zéro éventuelle | deltas additionnés, index de fin du second, remises gardées | l'instant d'un relevé intermédiaire, jamais une quantité |
| Occurrence | nombre par code | nombres additionnés | l'instant de chaque appui, au-delà du détail |
| Texte, média | code de contenu | seulement si c'est le même contenu | rien |

### 6.1 La mesure : le relevé à bande

C'est la « valeur moyenne avec date de début et de fin », appliquée dès l'entrée de l'historique,
au cœur.

```text
pour chaque rapport (t, v) d'une mesure p :
    b ← bande[p]
    b.aire += b.tenue × (t − b.dernier_t) ; b.dernier_t ← t     -- la valeur précédente a tenu jusqu'à t
    si |v − b.écrite| ≥ bande(p) et t − b.début ≥ intervalle_min(p) :
        résumé ← b.n > 1 ? (b.aire / (t − b.début), b.min, b.max, b.n) : rien
        écrire (p, t, v, résumé)                                 -- la ligne ferme [b.début, t)
        b ← (début = t, écrite = v, tenue = v, min = v, max = v, aire = 0, n = 1)
    sinon :                                                      -- absorbé : rien n'est écrit
        b.tenue ← v ; b.min ← min(b.min, v) ; b.max ← max(b.max, v) ; b.n += 1
à l'arrêt propre : écrire chaque bande ouverte avec n > 1 (même valeur, résumé), puis la rouvrir au démarrage
```

- **Ce que dit une ligne** : « à partir de t, la valeur est v ; depuis la ligne précédente, elle a
  varié entre min et max, avec cette moyenne, en n rapports ». Une ligne sans rapport absorbé n'a
  pas de résumé : ses colonnes restent vides.
- **Un seul mécanisme** remplace la bande morte, l'intervalle minimal « le dernier gagne » et
  l'accumulateur horaire de la V1 : moyenne, min et max restent exacts par intervalle.
- **Plus de coupure à l'heure** (V2). L'archive calcule les heures depuis ces lignes (§ 6.4).
- **Ce qu'on perd en cas de coupure** : le résumé de la bande ouverte, jamais la valeur, qui est
  dans `maison.db`.
- **Mesuré** (série formes, valeurs synthétiques, bande de 5 W ou 0,05 °C) : 226 983 lignes de
  mesure sur 1 039 955, soit 4,6 fois moins. Le gain réel dépend du bruit des capteurs : il se
  mesure sur un export avec valeurs (§ 15).
- Les 34 séries qui font 78 % des relevés (série profil) sont celles que la bande résume le plus.

### 6.2 L'état : les transitions, sans perte

Une porte s'ouvre à 08:03 et se ferme à 08:05 : on garde exactement ces deux transitions,
`(point, instant, code)`. La fin d'un segment est le début du suivant ; la disponibilité de
l'appareil dit si la source vivait. On ne lisse jamais un état, on ne le ré-échantillonne pas.
Pour les statistiques, des durées par jour s'y ajoutent : « porte ouverte 2 h 10 hier,
14 ouvertures ». Avec des codes, la limite des 64 états devient une alerte sur la taille du
dictionnaire, jamais une panne de l'archive.

### 6.3 L'index cumulé : l'élagage

On ne moyenne jamais un cumul : **on l'élague**. On ne garde que des deltas de plus en plus
larges, par heure, puis par jour, puis par mois, avec l'index de fin de chaque période pour
comparer au compteur. L'élagage ne perd aucune quantité : remises à zéro, trous et changements de
compteur sont traités avant, par les règles de `meter.rs` (V1 § 7.5), et un delta n'est jamais
négatif. Les heures d'énergie que le cœur tient déjà (D13) sont le premier palier. Un index aux
relevés rares a un delta réparti au prorata des heures (`meter.rs`) : l'index de fin d'une période
est alors le dernier relevé réel, avec son instant, jamais une interpolation.

### 6.4 Les heures et les jours : `moli-rollup`, nourri par les segments

`moli-rollup` calcule déjà, par heure, la moyenne pondérée par le temps, le min, le max, la
couverture, le nombre de changements, l'ancre et l'origine ; puis les jours et mois locaux par
fusion. Il manque une seule chose : lire une ligne à bande comme un intervalle fermé.

```text
pour une ligne à bande (t_i, v_i, résumé_i) qui ferme [t_{i−1}, t_i) :
    sans résumé : la valeur v_{i−1} a tenu sur tout l'intervalle (règle actuelle)
    avec résumé : l'intervalle vaut moyenne_i × durée, min_i, max_i
    heure à cheval sur l'intervalle : chaque heure prend sa part de durée au prorata, le même
        min et le même max ; l'erreur sur une heure est bornée par max_i − min_i,
        lui-même borné par la bande du point
```

Les heures restent **denses** : une heure sans changement existe, avec sa couverture. C'est ce que
`moli-rollup` produit, et Parquet la compresse presque gratuitement (17 contre 12 Mo par an pour
la forme creuse, série formes). On ne crée pas de forme creuse.

### 6.5 Les segments à tolérance (option)

Pour garder plus de détail que des heures pendant des mois, on fusionne les lignes à bande
voisines tant que `max − min ≤ 2ε`. Toute valeur réelle reste dans `[min, max]`. C'est la
compression des historiens industriels. Sa place entre le détail et les heures se décide après
la mesure sur des valeurs réelles (§ 15).

## 7. Vieillir petit à petit : les paliers

Pour chaque nature et chaque famille, ce que garde chaque étage. Les horizons marqués D13 sont
décidés ; les autres sont proposés. « Toujours » est un choix explicite du propriétaire, pour la
vie privée autant que pour la place.

| Nature (famille D13) | Récent, `moli.db` | Archive, palier 1 | Palier 2 | Palier 3 |
| --- | --- | --- | --- | --- |
| Mesure, puissance (`power`) | bande, 7 j (D13) | bande, 30 j (D13) | heures, 10 ans (D13) | jours : au-delà, proposé |
| Mesure, climat (`climate`) | bande, 14 j (D13) | bande, 90 j (D13) | heures, 5 ans (D13) | jours : au-delà, proposé |
| États de sûreté : portes, fenêtres, serrures, alarme (`states`) | transitions, 31 j (D13) | transitions, 365 j (D13) | durées et nombres par jour : au-delà, proposé | « toujours » seulement sur choix explicite |
| Autres états, occurrences (`states`) | 31 j (D13) | transitions et événements, 365 j (D13) | durées, transitions et nombres par jour, proposé | — |
| Index cumulé (`energy`) | relevés, heures 2 mois, totaux 62 j et 13 mois (D13) | deltas par heure et index de fin, 10 ans (D13) | jours, proposé | mois, proposé |
| Diagnostic | 7 j (D13) | rien (D13) | — | — |
| Autres mesures (`other`) | 7 j (D13) | bande, 30 j (D13) | heures, 5 ans, proposé | jours, proposé |
| Texte, média | sessions de la fenêtre | sessions courtes et effaçables, proposé | durées et nombres par contenu, proposé | — |

**Ce que pèse chaque forme**, charge ×1, format typé, mesuré sur des valeurs synthétiques
(série formes) :

| Forme | Lignes par semaine | Octets par ligne | Mo par an |
| --- | --- | --- | --- |
| Brut typé (mesures + états) | 1 161 366 | 6,1 | 368 |
| Détail à bande (mesures à bande + états) | 348 394 | 9,7 | 175 |
| Heures denses (`moli-rollup`) | 256 239 | 1,3 | 17 |
| Heures creuses (écartées) | 46 523 | 5,0 | 12 |
| Jours denses | 11 828 | 6,0 | 4 |

- Dix ans d'heures tiennent dans ≈ 170 Mo ; 30 jours de détail à bande dans ≈ 15 Mo, 90 jours dans
  ≈ 45 Mo. Les fenêtres de détail tournent sans grossir.
- Ces chiffres supposent le format typé du § 8. En format actuel, les mêmes heures pèsent 650 Mo
  par an et le brut 2,5 Go (mesuré).

**Qui fait vieillir : l'archive (E1)**, par sa compaction. Les fichiers Parquet sont immuables :
vieillir un mois revient à écrire ses paliers suivants puis à basculer le manifeste. C'est déjà le
modèle de l'archive : révisions, `current.json`, ramasse-miettes.

```text
chaque nuit, pour chaque (nature, mois) dont l'âge dépasse l'horizon de son palier :
    lire le palier courant ; écrire le suivant (fusion selon la nature, § 6)
    vérifier : mêmes totaux (deltas, durées, nombres), mêmes min et max, couverture identique
    publier une révision du manifeste ; retirer l'ancien palier au ramasse-miettes
jamais : retirer un palier avant que le suivant soit écrit, vérifié et publié
raccourcir un horizon demande la validation humaine de B1, comme en local
```

## 8. Le long terme : `archive_v2`, typé

### 8.1 Mesuré : le format compte plus que tout le reste

| Forme | Octets par ligne | Mo par an |
| --- | --- | --- |
| `archive_v1` brut, réglages actuels (Snappy, sans dictionnaire, groupes de 256 lignes) | 40,7 | 2 459 |
| `archive_v1` brut, mêmes colonnes, zstd + dictionnaire + grands groupes | 17,0 | 1 029 |
| Typé brut | 6,1 | 368 |
| `archive_v1` heures résumées, réglages actuels | 59,3 | 650 |
| Typé heures | 1,3 | 17 |

Les réglages seuls divisent par 2,4 ; la forme fait le reste. Une heure résumée en format actuel
pèse plus que dix relevés bruts typés : la colonne `payload` répète le résumé en JSON, avec le
nom du point, la qualité et l'origine en toutes lettres.

### 8.2 Le jeu de fichiers

Un fichier par nature et par mois UTC, trié par (point, début). Les groupes actuels de l'archive
(`numeric`, `events`, `energy`, `stats`, `corrections`, `other`) restent les dossiers ; seules les
colonnes changent.

| Fichier (groupe) | Colonnes | Palier |
| --- | --- | --- |
| `mesures` (`numeric`) | point INT32, début INT64, valeur INT32 mis à l'échelle (ou DOUBLE), moyenne, min, max, n (facultatifs) | 1 : relevés à bande |
| `mesures_heures`, `mesures_jours` (`stats`) | point, début, moyenne, min, max, couvert_ms, changements, qualité INT8, origine INT8 | 2 et 3 : `moli-rollup` |
| `etats` (`events`) | point, début, code INT16 | transitions |
| `etats_jours` (`stats`) | point, jour, code, ms, transitions | durées |
| `occurrences`, `occurrences_jours` | point, instant ou jour, code, nombre | — |
| `index_heures` (`energy`) | compteur, heure, delta INT64 mis à l'échelle, index de fin, coût, origine | 1 : `energy_hour` |
| `index_jours`, `index_mois` | compteur, période, delta, index de fin, coût | 2 et 3 |
| `sessions` | point, début, fin, contenu INT32 | — |
| `catalogue`, `dictionnaires` | points, natures, unités, échelles ; codes → clés | — |

- **Écriture** : zstd, dictionnaire activé, groupes de dizaines de milliers de lignes,
  statistiques sur `point` et `début` pour sauter les groupes inutiles, aucune colonne JSON.
- **Le NDJSON reçu** reste dans le journal de réception jusqu'à sa publication : il sert à
  l'idempotence, pas au stockage.
- **Les lectures** (`POST /query`) se construisent depuis les colonnes et non plus depuis le JSON
  stocké. Le contrat de réponse ne change pas.

### 8.3 Le protocole d'export, delta minimal

Le protocole refuse les champs inconnus : l'archive est mise à jour avant le cœur
([archive-protocol.md](../components/archive-protocol.md)).

| Changement | Détail |
| --- | --- |
| `sample` | quatre champs facultatifs : `lo`, `hi`, `mean`, `n`, le résumé de l'intervalle que la ligne ferme |
| `kind` | `e` : énumération, `num` porte le code |
| catalogue | par point : `nature`, `scale`, et pour une énumération la liste des clés, dont l'indice est le code ; déjà révisionné par `PUT /catalog` |
| ordre | un code inconnu du catalogue reste un entier stockable, décodé à la révision suivante : aucune règle d'ordre à ajouter |
| `energy_hour` | inchangé |

**Variante minimale**, si on ne veut pas toucher au protocole tout de suite : la valeur voyage en
clé texte (`t`), et l'archive active le dictionnaire Parquet, qui stocke chaque clé une fois par
groupe de lignes. Les octets sont les mêmes ; on garde alors la limite des 64 états en texte.

### 8.4 Migration

`archive_v1` → `archive_v2` se convertit une fois, par la compaction : chaque part est relue,
réécrite dans les nouveaux fichiers, puis retirée au ramasse-miettes après publication de la
révision. Le journal de réception et les reçus ne changent pas.

## 9. Énumérations et textes : des codes de bout en bout

- **Dictionnaire en ajout seul**, par énumération : clé stable → code (V1 § 4.3). Un code n'est
  jamais réutilisé ni renuméroté. Le code circule partout : `moli.db`, opérations d'export,
  archive, durées par état.
- **Pourquoi des codes, et pas seulement un dictionnaire Parquet** : la limite des 64 états devient
  une alerte sur le dictionnaire ; un libellé qui change ne casse pas l'historique ; une
  comparaison est entière ; une entrée s'efface en un point.
- **Textes de contenu** (titres, applications) : un dictionnaire `contenu(id, titre, type,
  application)` et des sessions `(appareil, début, fin, contenu)`. Un titre revu dix fois n'est
  stocké qu'une fois. Effacer l'entrée efface le titre de toutes les sessions, sans réécrire un
  fichier de l'archive.
- **Texte libre** : borné à 4 000 caractères au bord ; non historisé, ou gardé peu de temps.
- 9,1 % des changements sont des textes courts, presque tous des énumérations (série profil).

## 10. Le récent dans `moli.db`

- **Contenu** : les opérations compactes (journal du jour et export, V1 § 13.8) ; les relevés à
  bande, rangés par point à l'heure close ; l'énergie de D13 (heures 2 mois, totaux de jours et
  de mois) ; le catalogue, les dictionnaires, la garde B1, `archive_ack`, `meta`, le progrès de la
  nuit.
- **Retiré** : tout agrégat (`agg_hour`, `agg_day`, `state_day`, `hours`). Une courbe locale à la
  résolution heure se calcule à la volée depuis les lignes à bande, et seulement en secours (§ 12).
- **Purge** : chaque nuit, par famille, aux horizons de D13, jamais au-delà de `published_upto` pour
  une famille archivée.
- **Taille en régime établi** : ≈ 50 à 80 Mo pour la charge ×1 (*estimation* : lignes à bande
  dans les fenêtres, journal gardé 7 jours après publication, énergie).

```sql
CREATE TABLE samples (            -- relevés à bande, rangés par point à l'heure close
    id   INTEGER PRIMARY KEY,     -- (point << 44) + ts en ms
    seq  INTEGER NOT NULL,
    num  ANY,                     -- valeur à partir de ts (code pour un état)
    txt  TEXT,
    q    INTEGER,                 -- marqueur rare
    moy  ANY, lo ANY, hi ANY,     -- résumé de l'intervalle que ce relevé ferme ; NULL sans bande
    n    INTEGER                  -- rapports absorbés
) STRICT;
-- ops : opérations compactes (V1 § 14.6), avec les mêmes colonnes de bande
-- inchangés de D13 : catalog, meter_state, energy_hour, energy_total, archive_ack, meta, garde B1
-- ajoutés : dict_entry (V1 § 14.2), progress (V1 § 14.6)
```

Les colonnes de bande coûtent un octet d'en-tête chacune quand elles sont vides : ≈ 4 octets par
relevé sans bande, à vérifier au banc. Une table à part est l'autre option (décision 4).

## 11. État courant, double écriture, nuit

Inchangés depuis la V1 (§ 13.7 à 13.9), mesurés (série écritures) :

- `maison.db` garde une ligne complète par point et suffit au démarrage ; `moli.db` reçoit
  l'historique en parallèle ; au démarrage, une réconciliation silencieuse garde la valeur la
  plus récente ; aucune transaction commune aux deux fichiers.
- `point_state` par lots de 60 s : ≈ 65 Mo écrits par jour, contre ≈ 117 Mo pour `state.json`
  aujourd'hui (*estimation*).
- Historique en journal compact rangé à l'heure close, lots de 2 s : ≈ 258 Mo par jour, contre
  ≈ 3,1 Go pour le chemin de D13 tel que décidé.

| Où | Quand | Quoi |
| --- | --- | --- |
| Cœur, au fil de l'eau | lots de 60 s et de 2 s | état courant ; relevés à bande et opérations compactes |
| Cœur, à l'heure close | une transaction | rangement de l'heure par point |
| Cœur, la nuit (03 h 30 – 06 h 00) | tranches courtes, reprenables | purge des fenêtres de D13, sauvegarde, point de contrôle |
| Archive, à la réception | lot par lot | palier 1 de chaque nature ; heures par `moli-rollup` |
| Archive, la nuit (E1) | après la nuit du cœur | paliers suivants, vérification, manifeste, ramasse-miettes |

Le cœur ne calcule plus aucun agrégat : ses nuits raccourcissent.

## 12. Lire une courbe (D1)

```text
courbe(p, [de, à], résolution) :
    résolution heure, jour ou mois :
        archive pour [de, min(à, published_upto)]             -- heures denses de moli-rollup
        puis lignes à bande locales pour le reste, résumées à la volée
        puis la valeur vive en mémoire comme dernier point
    résolution brute :
        moli.db pour la fenêtre locale ; archive au-delà (détail à bande s'il est encore là,
        sinon le palier suivant, marqué comme tel)
    archive injoignable : la partie locale seule, marquée "archive": "unavailable" (D13)
état : segments début–fin, jamais des points ; index : deltas, jamais une courbe d'index moyennée
```

Une courbe ne retarde jamais d'un lot, et une seule réponse couvre toutes les durées.

## 13. Ce qui ne change pas depuis la V1 et la V2

| Sujet | Où |
| --- | --- |
| Valeur canonique au bord, échelle de la source, dictionnaires | V1 § 4 |
| Trois instants ; fraîcheur, qualité, pannes en cascade | V1 § 5, § 8 |
| Filtres : anti-rebond au bord seulement | V1 § 6 |
| Valeurs cumulées : règles de `meter.rs`, « inclus dans », jamais de moyenne | V1 § 7.5 |
| Équipements logiques par liaisons, mode dégradé | V1 § 9 |
| Conditions avec durée, échéancier unique | V1 § 10, § 12.7 |
| Familles particulières : Linky, notes scolaires, médias | V1 § 11 |
| Deux bases, règle des noms stables, Turso embarqué et multithread, vecteurs en BLOB | V1 § 13.1 à 13.6 |
| Double écriture, coût réel d'une écriture, nuit | V1 § 13.7 à 13.9 |
| État métier en tables, politiques dans `[storage]` | V1 § 14.5, § 14.7 |
| Quatre étages, le type décide, segments, paliers, codes | V2 § 3 à 7, corrigés ici |

## 14. Ce qu'on ne fait pas

- Garder le long terme dans `moli.db`, ou un agrégat dans le cœur.
- Écrire le nom du point ou l'opération en JSON sur chaque ligne du long terme.
- Lisser, moyenner ou ré-échantillonner un état.
- Moyenner un index, ou calculer une période par « index de fin − index de début ».
- Inventer une forme « creuse » que Parquet rend inutile.
- Couper une bande à l'heure pour une exactitude que la bande ne promet pas.
- Garder dix ans de transitions de porte sans une décision explicite.
- Retirer un palier avant que le suivant soit écrit, vérifié et publié.
- Stocker le texte d'une énumération ou un titre répété.
- Laisser un 65ᵉ état mettre l'archive en panne.
- Et tout ce que la V1 écarte déjà (V1 § 15).

## 15. Mesures : faites et à faire

| Mesure | État | Série ou suite |
| --- | --- | --- |
| Octets écrits par jour (double écriture, journal, lots) | faite | [`2026-10-21T114647Z`](../reports/mesure-ecritures-2026-10-21T114647Z.md) |
| Profil de l'historique (qui écrit, types de valeurs) | faite | [`2026-10-21T151507Z`](../reports/profil-historique-2026-10-21T151507Z.md) |
| Formes en Parquet : `archive_v1` contre typé, brut, bande, heures, jours | faite, valeurs synthétiques | [`2026-10-21T200551Z`](../reports/formes-parquet-2026-10-21T200551Z.md) |
| Bande et segments à tolérance sur des valeurs réelles | à faire | export anonymisé **avec valeurs**, produit en local depuis la copie privée ; même outil, nouvelle série |
| Octets écrits sur Turso, puis sur Pi Zero 2 W | à faire | porter les scénarios dans `tools/storage-bench` ; relancer `measure.py` sur le Pi |
| E1 : durée d'un mois de paliers sur Pi, totaux vérifiés avant retrait | à faire | — |
| Une énumération à plus de 64 valeurs ne bloque plus l'archive | à faire | test de `summaries.rs` avec codes |
| Colonnes de bande dans `samples` : coût des NULL | à faire | banc ×2 |

## 16. Décisions à prendre

1. **Quatre étages** : le long terme dans l'archive seulement, `moli.db` limitée au récent, archive
   lancée par défaut dès qu'on veut un long terme (proposé) ?
2. **`archive_v2` typé** (codes, dictionnaire, zstd, sans JSON par ligne), avec le delta de
   protocole du § 8.3 : à présenter à la revue de D13 et de l'archive ? C'est le premier levier.
3. **Heures denses de `moli-rollup`** comme palier, sans forme creuse (proposé) ?
4. **Relevé à bande** au cœur, à la place de la bande morte et de l'intervalle minimal « le
   dernier gagne » (proposé) ? Colonnes dans `samples`, ou table à part ?
5. **Paliers et horizons** (§ 7) : les « proposés » ; et « toujours », pour quelles natures, sur
   décision explicite ?
6. **Codes de bout en bout** (proposé) ou variante minimale (clé texte + dictionnaire Parquet) ?
7. **Vieillissement par l'archive** (E1, proposé) ou par le cœur ?
8. **Lectures D1** : heure et jour par l'archive, brut en local, secours local (proposé) ?
9. **Segments à tolérance** : un palier de plus, après la mesure sur valeurs réelles ?
10. **Tickets** : aligner D13 sur le plan (C1 archive, C2 récent) ?
11. Les décisions encore ouvertes de la V1 (§ 16) : anti-rebond, `seq`, politiques dans
    `[storage]`, noms des bases et place des vecteurs, reprise des durées, équipements, notes
    scolaires, sous-compteurs, MVCC, lots de 60 s et de 2 s, fenêtre de nuit.

## 17. Méthode

- Réanalyse : chaque affirmation de la V2 confrontée à une mesure ou au code ; celles qui ne
  tenaient pas sont corrigées en tête de document, avec leur raison.
- Premiers principes : ce que chaque nature doit permettre avant la technique de stockage.
- Analyse morphologique par nature : forme, filtre, paliers, et ce qu'on ne fait jamais.
- Analogies : RRDtool (archives consolidées), historiens industriels (compression par bande),
  TimescaleDB et InfluxDB (rétention par palier, compression par série), Parquet (dictionnaire,
  répétitions), arbres LSM (écrire en ajout, ranger plus tard).
- Mesure plutôt qu'estimation : trois outils versionnés, trois séries datées, un rapport par
  série.
- Inversion : comment rater (§ 14).
