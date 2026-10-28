# V6 : preuve de structures, isolée du produit

Ce dossier est un **prototype synthétique**, compilé directement avec `rustc`, sans
dépendance. Il n'importe aucun code du moteur et ne contient aucune donnée de maison.
Il ne contacte ni réseau ni appareil. Ce n'est ni un moteur livré, ni un nouveau banc
de latence, ni une preuve de mémoire RSS. Les campagnes V2–V5 restent les preuves
de performance de leurs implémentations respectives, figées avant la conservation
de la précédente valeur : elles ne comparent pas cette fonction nouvelle.

`main.rs --layouts` émet taille, alignement et **tous** les offsets des 18 structures.
Les formes Row64/Row96/Numeric32/Hot32/Cold32 reprennent le banc catalogue ; Alias32
reprend le banc orientations (les champs de cache sont nommés `source_slot` et
`source_generation`) ; Edge24/Target12 reprennent le banc organisation.
Schema64, Transform64 et Calc48 font respectivement 64, 64 et 48 octets sans `packed`.
Les layouts C en RAM ne sont pas un format de fichier ni un protocole inter-machine.

Le modèle `Bank` stocke les lignes canoniques Row96 et les alias dans deux Vec.
L'identité u64 ne dépend ni du nom, ni de l'offset dans un Vec. Un slot global 16
octets situe chaque nœud ; ses métadonnées 32 octets sont séparées. Le HashMap associe
l'identité au couple slot/génération. Renommer ne touche que la métadonnée ; compacter
ne change que les indices physiques. Aucune adresse de Vec n'est gardée.

Lire un alias vérifie l'epoch du lecteur, sa propre génération, la génération et
l'identité de la source. Une source alias est aplatie à la création ; le stockage
n'a jamais de chaîne d'alias. Une suppression suivie d'un réemploi invalide les caches.
Générations et epochs ne rebouclent pas : leur épuisement est refusé. Les IDs sont
fournis par l'appelant : un allocateur durable d'ID reste à développer pour le produit.

Un alias n'a pas de copie d'état : il partage la valeur courante, la précédente et les
deux dates de sa source. Une observation canonique devient visible par tous ses alias.
Un calcul possède sa ligne et son état propres ; ici le test injecte son résultat, **aucun ordonnanceur de calcul n'est implémenté**. `intent` renvoie seulement
un destinataire physique et un point canonique, sans muter l'observation. Il distingue
écriture et lecture active. Validation métier complète, garde, contrôle d'origine,
pilote, délais, journal et confirmation d'état doivent rester dans le Hub du produit.

La topologie impose famille → maison → étage facultatif → pièce ; équipements
physiques/virtuels sous maison ou pièce ; valeurs/actions/calculs sous équipement.
Une zone fonctionnelle appartient à une maison et référence pièces/équipements par
arêtes, sans copier leurs lignes. Plusieurs zones peuvent se recouvrir. La géozone
est une ressource distincte, refusée comme zone fonctionnelle. Les liens de zone
entre maisons, parents absents, cycles et déplacements qui invalident une zone sont
refusés. Ce modèle n'est pas un catalogue complet des relations métier.

Edge24 est la source durable **en RAM** des liens parent/enfant et appartenances ;
« durable » ne signifie pas persisté sur disque. Le CSR avant ajoute Target12 et ses
offsets u32 ; le CSR inverse ajoute des indices d'arêtes u32 et ses propres offsets.
Les trois coûts sont cumulés. Le CSR emprunte Bank immuablement : Rust interdit les
mutations tant qu'il est utilisé. Les mutations ordinaires requièrent `&mut Bank`.
Ce n'est pas une stratégie de concurrence pour un moteur multi-lecteur.

Les blobs ont un descripteur PoolEntry16, génération, type et bornes contrôlées :
4 Kio par valeur, 1 Mio d'octets et 65 536 descripteurs pour le pool synthétique
(les valeurs vides consomment aussi un descripteur). Le composite d'exemple est décodé
en little endian explicitement, jamais par transmutation : version u32, nombre u16,
réservé u16 nul, puis couples clé u32 / bits de valeur u64 ; 64 champs au plus,
aucune clé dupliquée. Une mise à jour valide intégralement taille et version avant
de publier un nouveau descripteur et une nouvelle version ensemble sous `&mut`.
Le pool est append-only et conserve les anciennes versions ; récupération,
déduplication et persistance restent hors périmètre. Le format illustre une preuve
de remplacement atomique, pas toutes les valeurs composites du futur produit.

`alternatives.rs` matérialise aussi Row64 + annexe Numeric32 + une référence u32
par ligne canonique, et Hot32 + Cold32 avec la même annexe et les mêmes références.
Le masque des schémas numériques est explicite : une valeur numérique encore Null
peut avoir des limites ; le masque ne se déduit pas de la dernière valeur observée.
Le test reconstruit tous les champs des lignes. La représentation active de Bank
reste Row96 ; aucune latence comparative n'est annoncée pour ces alternatives.

## Valeur courante, précédente et deux dates

Row96 garde **deux payloads et deux dates**, sans changer taille ni offsets :

```text
Row64 [0..64[ :
  [ 0..16[ id + parent
  [16..24[ payload          [24..32[ previous_payload
  [32..40[ received_ms      [40..48[ changed_ms
  [48..64[ schema + aux + revision + flags + row_kind + value_kind
Numeric32 [64..96[ : min + max + step + anchor
Hot32 = payload + previous_payload + received_ms + changed_ms
```

`received_ms` date la dernière réception locale, même identique ; ce n'est pas la
preuve de l'instant de mesure physique. `changed_ms` date le dernier changement
effectif de type ou valeur. La date source est passée transitoirement à `observe` :
sa dernière occurrence n'est pas gardée dans la ligne. Le choix d'une date quand la
source n'en fournit pas reste un contrat explicite à définir par adaptateur.
Les dates source et qualités utiles aux fenêtres temporelles demandent une ressource
externe proposée, avec ses propres limites et son budget.

`previous_payload` garde la dernière valeur **distincte** avant changement. La première
observation n'a pas de précédente ; un paquet identique conserve la précédente.
Exemple, avec dates source et réception égales uniquement pour l'illustration :

| Reçu | Courante | Précédente | received_ms | changed_ms |
|---|---|---|---:|---:|
| 20 à t=10 | 20 | absente | 10 | 10 |
| 21 à t=20 | 21 | 20 | 20 | 20 |
| 21 à t=30 | 21 | 20 | 30 | 20 |
| 22 à t=40 | 22 | 21 | 40 | 40 |

`WRITE=0x0001` et `ACTIVE_GET=0x0002` restent indépendants.
`HAS_OBSERVATION=0x0010` et `HAS_PREVIOUS=0x0040` sont implémentés ; le type précédent
occupe les quatre bits hauts de `flags` (`0xF000`, décalage 12). Un `Null` précédent
présent se distingue donc d'une précédente absente. `READ=0x0004`,
`AVAILABLE=0x0008`, `ACCESS_UNRESOLVED=0x0020` et les présences numériques
`MIN/MAX/STEP/ANCHOR=0x0100/0x0200/0x0400/0x0800` restent proposés.
`0x0080` est réservé.

La précédente scalaire garde ses bits ; texte, binaire et composite gardent un handle
du pool. Le pool append-only conserve les anciens blobs sans récupération : les
contenus variables peuvent coûter plus malgré la ligne fixe inchangée à 96 octets.
Les budgets de contenu existants ne mesurent pas ce surcoût de rétention : le pool
opaque hérité mêle plusieurs contenus et ne permet pas de l’isoler. Les totaux sont
donc des **budgets de base, hors contenu variable précédent**, sans borne démontrée
pour cette rétention sur tous les scénarios. Une seule
précédente valeur ne fournit aucun historique complet, ni date de la précédente.
Ni une action ni un lieu ne crée de fausse valeur observée ; leurs champs restent
alloués mais absents tant qu'aucune observation ne leur correspond explicitement.

## Comptage : budget de base, hors contenu variable précédent

`--demo` émet des nœuds/liens synthétiques, les résolutions d'alias et les capacités
réelles des Vec du modèle actif. `used_bytes = len × size_of`,
`capacity_bytes = capacity × size_of`. Ce sont les octets d'éléments réservés, sans
surcoût d'allocateur, fragmentation, piles, objets Vec/Bank ou allocations temporaires.
Le HashMap expose len/capacité et les **seuls octets de clés/valeurs utiles** ; ses
octets de buckets, contrôle, capacité libre et alignement ne sont pas mesurés.
`allocation_bytes: null` indique cette absence de mesure. Aucun total RSS n'est inféré.

Pour N canoniques, M schémas numériques, A alias, S slots alloués et E arêtes :

| Partie | Octets utiles, sans capacité excédentaire |
|---|---:|
| Canonique Row96 | 96 N |
| Alternative Row64/Hot32+Cold32 + Numeric32 + références | 68 N + 32 M |
| Alias | 32 A |
| Slots globaux + métadonnées | 48 S |
| Arêtes sources + CSR avant + CSR inverse | 40 E + 8 (S + 1) |
| Clés u64 + valeurs Handle8 utiles du HashMap | 16 (N + A), **pas son allocation** |
| Descripteurs et octets du pool | 16 P + B |

Sans trous, S = N + A : une ligne Row96 atteint déjà **144 octets** et un alias
**80 octets** avec slot et métadonnée, avant index, relations, pools et autres tables.
Après suppressions, les slots et métadonnées libres restent réservés. Schema64,
Transform64, Component32, Calc48 et Route32 ont des layouts testés mais **aucune
instance en Vec dans ce Bank minimal**. Ils s'ajoutent selon leurs nombres réels :
64 Q + 64 T + 32 C + 48 K + 32 R. Les textes, plans et règles référencés par Meta32
ne sont pas implicitement gratuits : leurs contenus et index restent à budgéter.

## Reproduire

Depuis la racine du dépôt, avec l'image de développement déjà disponible :

```sh
docker run --rm --network none -v "$PWD/tools/memoire-structure-v6:/bench:ro" moli-os-dev:latest sh -c 'rustc --edition=2024 -D warnings --test /bench/main.rs -o /tmp/structure-tests && /tmp/structure-tests'
docker run --rm --network none -v "$PWD/tools/memoire-structure-v6:/bench:ro" moli-os-dev:latest sh -c 'rustc --edition=2024 -D warnings -O /bench/main.rs -o /tmp/structure && /tmp/structure --layouts'
docker run --rm --network none -v "$PWD/tools/memoire-structure-v6:/bench:ro" moli-os-dev:latest sh -c 'rustc --edition=2024 -D warnings -O /bench/main.rs -o /tmp/structure && /tmp/structure --demo'
```

Les fichiers `layouts.json` et `demo.json` proviennent de ces deux sorties, sur
linux/aarch64, pointeurs 64 bits, little endian. La commande de tests couvre tailles,
offsets sensibles, alternatives, alias, identités, slots, epochs, topologie, zones,
CSR, null/zéro, accès, deux dates, précédente typée et composites malformés. Aucun secret,
flux MQTT réel ni état HA brut ne sert de fixture.

## Dimensionnement et conservation

Depuis la racine, avec Python standard et Docker :

```sh
python3 tools/memoire-structure-v6/executer.py
python3 tools/memoire-structure-v6/rapport.py
python3 tools/memoire-structure-v6/verifier.py
python3 tools/memoire-structure-v6/archiver.py
```

Le lanceur fige les sources pendant la compilation et les tests structurels. Leur
nombre est enregistré dans `sorties/metadata.json` (`test_count`) et leur résultat
figure dans le journal de tests de la capture. Il mesure aussi
23 réservations de `HashMap<u64,Handle8>`, jusqu’à 1 613 600 entrées. Le probe a son
allocateur de comptage : octets demandés, sans mesure RSS ni latence. Les tables plus
grandes sont extrapolées selon les buckets de cette toolchain.

`budget.py` calcule 12 scénarios et 3 variantes depuis les formes anonymisées A/B/somme.
Hypothèses et exclusions figurent dans `sorties/budgets.json`. Les alternatives V6
comptent une référence numérique de 4 octets par canonique : `aux` reste disponible
pour un composant typé. Ce coût diffère de celui du microbanc V2.

Les mesures V2–V5 sont importées avec leur périmètre propre ; leurs totaux ne sont
pas additionnés. Alias, zones, calculs et métadonnées ajoutés sont synthétiques.
Les données HA brutes et conversations privées restent hors de ce dossier.

Le rapport est `docs/proposition/analyse/catalogue-memoire-v6.html`. L’archive conserve
sources, résultats, entrées synthétiques publiques, mesures importées et empreintes.
Un changement de source ou d’entrée crée une capture distincte. Les fichiers sont
à replacer à la racine d’une copie du dépôt pour conserver les chemins des outils.

## Mesurer le travail des opérations

Le chapitre « Ce que l’organisation mémoire accélère, mesures à l’appui » du rapport
s’appuie sur [`operations-banc/`](operations-banc/README.md). Ce banc importe le vrai
`Bank` structurel V6, avec les deux dates et la précédente valeur ; il compare recherche
linéaire/indexée, relevé identique/changement, observation canonique seule/avec copie
de snapshots Row96 et scan des
arêtes/CSR. Les temps identique/changement sont deux travaux différents, présentés en
valeurs absolues, incluant observation, relecture et digest. Le détail conserve les échantillons et les allocations réellement
comptées, avec les limites de chaque opération.

Les tailles A/B/somme sont des projections de formes anonymisées. Les canoniques sont
matérialisés à la taille annoncée, mais seules 32 sources avec 64 alias chacune sont
instrumentées pour les comparaisons de fanout : les alias projetés ne sont pas tous
construits. Ni les notifications, ni les transports, ni le Hub complet ne sont mesurés.
Construction et allocations initiales des index/CSR restent hors des chronomètres ;
leurs coûts mémoire restent visibles et doivent être budgétés. Le même Bank de
2 048 alias et le même buffer de 2 048 snapshots sont alloués dans chaque variante :
ces durées ne comparent pas la mémoire de deux catalogues complets. Les snapshots
gardent l’ID source ; les lectures d’alias sont mesurées dans une opération séparée.
Le remplissage massif dédié de la fixture ne qualifie pas la vitesse de `insert`. Les chronomètres ne
mesurent ni les défauts de cache matériel ni un débit moteur intégré.

Le rapport exige `operations-banc/sorties/resultats.json` avant sa génération : il
n’invente aucun chiffre quand cette capture est absente. Sources, protocole et
reproduction sont liés depuis le chapitre ; les mesures V2–V5 restent inchangées.

## Alignement : 96, 128 et chaud/froid

Le dossier [`alignement-banc`](alignement-banc/README.md) conserve la nouvelle
comparaison de tailles et de vitesse. Il importe les définitions du prototype
sans modifier le stockage du Hub. Les six variantes séquentielles gardent la
même information ; cinq formes atomiques étudient séparément les écritures
concurrentes. Les résultats ne sont pas ceux d’un Hub migré.

```sh
python3 tools/memoire-structure-v6/alignement-banc/executer.py --image rust:1-alpine
python3 tools/memoire-structure-v6/alignement-banc/verifier.py
python3 tools/memoire-structure-v6/rapport.py
```

Le Docker actif doit disposer d’au moins quatre CPU. Le lanceur fixe l’image par
son ID, les affinités 0–3 et la limite de 768 Mio ; `--docker-context` permet un
environnement isolé. Il ne lance aucun conteneur existant. La capture conserve
les caches annoncés par l’hôte et les informations réellement exposées par Linux.
La vue de cache 64 octets du HTML est une illustration, pas une mesure sur x86.

La piste Row96 + Meta32 dans 128 octets alignés conserve le socle utile de
144 octets avec le slot, si la copie séparée de Meta32 canonique est retirée.
La variante de remplissage garde Meta32 à côté et porte ce socle à 176 octets.
Les mesures ne justifient aucun passage global automatique à 128 octets.

## Parcours visuel et mouvements mémoire

Les cinq premiers chapitres de V6 expliquent une petite maison fictive. La fixture
`exemple-pedagogique.json` conserve ses identités, parents, alias, liens, routes,
transformations et états initiaux. `pedagogie.py` génère les schémas et la simulation
locale des mesures MQTT/API, commande virtuelle, refus, mesure identique et deux
actions successives. Les boutons permettent de lire les zones sollicitées et de
comparer les valeurs, dates et compteurs avant/après chaque étape.

Le compteur de temporaires décrit des unités logiques, **pas** des allocations
Rust ou des octets transférés dans les caches CPU. Les publications MQTT virtuelles
et la normalisation en pourcentage restent des propositions. Une acceptation de
commande dans la file locale du pilote ne prouve ni livraison au broker ni effet
physique. La moyenne de l’exemple est une moyenne des dernières valeurs courantes de deux
capteurs ; la précédente valeur distincte ne remplace pas une fenêtre. Les
fenêtres temporelles restent dans le banc V5 dédié.

Pour vérifier la fixture et générer les tests des parcours :

```sh
python3 tools/memoire-structure-v6/verifier_pedagogie.py
```

Ouvrir ensuite `sorties/parcours-tests.html` dans un navigateur. La page affiche
le résultat de 12 parcours (6 cas × 2 modes de notification), avec contrôle de
chaque étape, des alias sans état, des écritures du calcul, des deux occurrences
d’action et du retour arrière. `parcours-tests.js` contient les assertions et
expose `runMemoryLessonChecks()` ; il peut aussi être injecté dans le rapport
généré pour vérifier exactement son code embarqué. Aucune connexion n’est faite.
Le résultat enregistré dans `sorties/parcours-validation.json`, s’il existe,
concerne cette simulation JavaScript : il ne remplace pas les tests Rust du Bank.
