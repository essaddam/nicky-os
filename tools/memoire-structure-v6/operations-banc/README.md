# V6 : microbanc d’opérations sur la vraie Row96

Ce banc importe directement `../layouts.rs` et `../model.rs`. Il ne change pas le
produit et n’utilise aucune donnée privée, appareil ou connexion réseau. Les valeurs,
identités, dates, liens et requêtes sont synthétiques. Seules les dimensions des
arènes dérivent du manifeste public `tools/catalogue-banc/charges/manifest.json`.
Les lignes n’interprètent pas tout le modèle HA : c’est une projection de tailles,
pas un replay des exports ni une mesure du moteur complet.

Lancer depuis la racine :

```sh
python3 tools/memoire-structure-v6/operations-banc/executer.py
```

Le script résout l’image existante `moli-os-dev:latest` et utilise ensuite son
identifiant immuable pour toutes les commandes ; il fige ses sources dans
un dossier temporaire et compile uniquement dans Docker, sans téléchargement :
réseau `none`, quota CPU `1`, plafond mémoire `512m`. Compilation identique au
prototype : `--edition=2024 -D warnings -C opt-level=s -C codegen-units=1`.
Il exécute les vérifications sémantiques avant les chronomètres, puis lance trois
processus indépendants. Chaque processus mesure trois rounds après une chauffe de
`min(iterations,256)` opérations par variante. Les neuf temps par variante restent
conservés dans `sorties/resultats.json` et `sorties/echantillons.jsonl`.

Le résultat comprend les empreintes des sources/imports/manifeste, l’identité de
l’image, la toolchain et l’environnement. Un changement concurrent des sources
annule la publication des résultats. Les anciens résultats sont remplacés par le
lanceur ; le rapport parent les archive avec ses autres preuves.

## Les variantes réellement mesurées

| Opération | Comparaison | Travail commun et limites |
|---|---|---|
| `lookup` | `linear_scan` / `bank_read_at` | Même Row96 canonique et même digest courant/précédent/types/dates. Même petit jeu déterministe de 16 à 128 requêtes, répété par la variante rapide ; tous les IDs existent. Le scan linéaire est borné pour ne pas rendre la campagne quadratique. |
| `observe` | `identical` / `changed_previous` | Vraie `Bank.observe`, suivie d’un `Bank.read_at` et du digest. Deux comportements différents : temps absolus, aucune affirmation d’équivalence de travail. |
| `fanout_update` | `canonical_alias` / `materialized_row96_views` | Une observation canonique, puis une lecture/digest. La variante vues copie en plus la Row96 entière dans 1/4/16/64 snapshots de vue. Les Alias32 ne copient aucun état. Aucune notification, souscription ou lecture d’alias dans cette opération. |
| `alias_read` | `bank_read_at` | Coût séparé de la résolution et lecture d’un alias, avec les contrôles réels de génération et d’identité. |
| `zone_members` | `edge24_scan` / `csr_targets` | Même liste/somme des slots des membres directs, au plus 256. Les IDs canoniques synthétiques sont denses : le scan convertit `ID-1` en slot, ce qui évite d’ajouter une recherche HashMap seulement au scan. Pas de développement des pièces, déduplication ou lecture d’état. |

La copie de vue est un **snapshot Row96 de 96 octets** : elle conserve l’identité
de la source. Ce n’est pas un second catalogue complet avec des identités propres,
ni une preuve contre toutes les représentations possibles de valeurs dupliquées.
La chauffe et le jeu de sources restent petits et chauds même dans une grande
arène. Les résultats ne représentent donc pas une charge aléatoire complète de
la maison ni une fréquence matérielle de défauts de cache.

## Dimensions et mémoire

Les huit scénarios reprennent A, B et leur somme, formes principal et détail ;
la somme-détail est aussi exécutée à ×10 et ×100. Le nombre canonique suit la
formule du budget V6 :

```
(rows_manifeste + 39 + ceil(source_entities/4) + 32 + 2*devices) * échelle
```

Une seule topologie synthétique est construite dans chaque arène : cinq
conteneurs, les équipements physiques dimensionnés, puis des valeurs entières.
Le `projected_aliases` vient de `source_entities*échelle`, mais le Bank mesuré
matérialise seulement **32 sources réparties ×64 Alias32 = 2048 alias**. Les
fanouts 1/4/16/64 sélectionnent un sous-ensemble de ces alias, sans reconstruire
l’index. Le buffer de snapshots de vue contient séparément 2048 Row96 (196608
octets utiles). Il n’est pas multiplié par tous les canoniques de l’arène.

Le montage bulk, toutes les réservations et la construction HashMap/CSR restent
hors chronométrage. Le constructeur initialise directement les champs `pub(crate)`
du prototype, puis vérifie identités, slots, alias et listes de membres. Le
`Bank.insert` actuel scanne les slots libres à chaque insertion : le débit de
création massive par l’API actuelle **n’est pas qualifié** par ce banc.

`memory.vectors` donne les nombres d’éléments, capacités, tailles et octets utiles/
réservés réels, CSR compris. `requested_bank_allocation_bytes` mesure les octets
d’allocation demandés pour les Vec et le HashMap du Bank avant la construction du
CSR ; ces nombres ne s’additionnent donc pas sans vérifier leurs recouvrements.
Les snapshots de vue sont indiqués séparément dans `materialized_views`.
Le petit buffer des requêtes contient 16–128 `u64` selon l’arène, soit 128–1024
octets, hors Bank. Aucun RSS n’est déduit : en-têtes, piles, métadonnées de
l’allocateur, fragmentation et résidence des pages ne sont pas mesurés.

Un `GlobalAlloc` de comptage mesure réellement les allocations réussies et les
octets demandés par opération chronométrée. Les compteurs atomiques peuvent
perturber les chemins qui allouent. Les kernels actuels devraient ne rien allouer,
mais le résultat JSON est la preuve, pas cette attente. Les allocations de montage,
CSR, requêtes, sorties JSON et validation sont exclues des compteurs par opération.

## Vérifications et interprétation

Avant les mesures, un petit Bank vérifie l’égalité intégrale des snapshots et des
lectures alias sur la séquence 21→22→22→23→Null→Null→0, précédent distinct, types et
horodatages compris. Il vérifie les refus d’observation sur alias sans mutation et
l’égalité des membres entre Edge24 et CSR. Chaque scénario vérifie ensuite tous
les slots/identités et les listes de membres avant sa chauffe.

Les temps sont les nanosecondes **écoulées `Instant` par opération**, pas des
cycles CPU ni le temps CPU du processus. `median_ns` est la médiane des neuf
rounds, `min_ns`/`max_ns` montrent leur dispersion ; `p95_sample_ns` est seulement
un percentile des neuf rounds, **pas un p95 de requêtes** ni un intervalle de
confiance. La VM, le scheduling et les caches ne sont pas isolés. Les tailles
mesurées n’autorisent ni nouvelles limites produit ni promesse de latence réseau.
