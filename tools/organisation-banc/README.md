# Banc isolé : organisation et appartenances

Ce programme Rust `std` seulement compare trois représentations **du même graphe
synthétique**, sans modifier Moli, appeler Home Assistant, lire un inventaire privé,
ouvrir un service réseau ou commander un appareil. Les trois arguments d’inventaire
sont des **comptages agrégés** : appareils, valeurs et contrats d’action/service.
Les actions ne sont pas présentées comme des commandes disponibles sur chaque entité.
Les comptages d’inventaires distincts peuvent être fournis par un lanceur extérieur.

Chaque réplication crée une famille, deux maisons, quatre étages, vingt-quatre pièces
et huit zones logiques : **39 conteneurs synthétiques**, sans prétendre décrire les
pièces d’un inventaire réel. Les appareils sont répartis alternativement entre les
deux maisons puis dans leurs douze pièces. Les valeurs ont un appareil pour parent ;
les contrats d’action ont une famille ou une maison pour parent. Les identifiants
`u64` sont uniques entre réplications, indépendants de noms ou d’adresses ; ils sont
déterministes pour une configuration donnée du banc, sans contrat d’identité durable
entre deux configurations d’inventaire différentes.

Chaque pièce et chaque appareil appartient à **deux zones de sa maison** ; ces zones
se recouvrent. Une appartenance ne déplace pas le parent physique et ne copie aucune
valeur. Tous les nœuds — famille, maison, étage, pièce, zone, appareil, valeur,
contrat — occupent réellement la même structure `OrgNode96` de 96 octets, copiée du
layout `Row96` des autres bancs : identité et parent `u64`, données/descripteurs,
horodatages, schéma, révision, drapeaux, catégories et réserve numérique de 32 octets.
Ce layout expérimental ne définit pas une ABI de stockage pour le produit. Les
champs sont remplis puis lus dans une somme de contrôle ; aucune matrice d’états
factice séparée ne gonfle artificiellement les allocations mesurées.

| Variante | Représentation des appartenances aller |
|---|---|
| `nested_ids` | Un `Vec<StableEdge24>` par nœud, avec IDs stables source et cible ; résolution de la cible par la table d’identité commune. Tous les en-têtes de vecteurs vides sont comptés. |
| `flat_cached` | Liens triés de 16 octets : slots source/cible `u32`, relation `u16`, drapeaux `u16`, révision `u32` ; offsets aller `u32` de longueur N+1. |
| `csr_cached` | Cibles de 12 octets : slot cible et mêmes métadonnées ; source déduite de la plage des offsets aller `u32` de longueur N+1. |

Les trois gardent les mêmes nœuds, la même `HashMap<u64,u32>` d’identité et le même
index inverse CSR membre→slots des zones (offsets N+1 et une entrée `u32` par lien).
L’index inverse ne duplique pas les métadonnées : les liens aller les gardent
intégralement. Relation, drapeaux et révision varient de manière déterministe ; les
tests reconstruisent les `StableEdge24` exacts dans chaque représentation. Un slot
est un cache local au graphe, pas une identité persistante ; sa reconstruction et
son invalidation lors d’une mutation ne sont pas implémentées ou mesurées ici.

## Reproduire dans le conteneur de développement

Depuis la racine, sans toolchain locale et sans binaire ajouté au dépôt :

```sh
docker run --rm -v "$PWD:/work" -w /work moli-os-dev:latest sh -ec '
  rustc --edition 2024 -D warnings -C opt-level=s --test tools/organisation-banc/main.rs -o /tmp/organisation-tests
  /tmp/organisation-tests
  rustc --edition 2024 -D warnings -C opt-level=s tools/organisation-banc/main.rs -o /tmp/organisation-banc
  /tmp/organisation-banc --bench nested_ids 171 1591 90 1
  /tmp/organisation-banc --bench flat_cached 171 1591 90 1
  /tmp/organisation-banc --bench csr_cached 171 1591 90 1
'
```

L’exemple numérique reprend les comptages historiques publics de la documentation.
La CLI est `--bench VARIANTE APPAREILS VALEURS ACTIONS ECHELLE`. Chaque invocation
est un **nouveau processus** ; une série fiable compile une fois dans le conteneur,
puis appelle séparément chaque variante et chaque échelle. Pour comparer deux
inventaires, exécuter exactement cette matrice avec leurs seuls trois comptages.
L’échelle réplique toute l’organisation et tous les nœuds, pas seulement les états.
Appareils et échelle doivent être positifs ; valeurs et actions peuvent être nuls.
Les tailles qui débordent les limites des offsets/slots `u32` sont refusées.

## Mesure et lecture du JSON

Le jeu de référence validé et les listes de requêtes sont construits **avant** le
point zéro ; ils sont identiques et exclus de `requested_heap`. Un allocateur global
qui délègue à `System` compte les tailles d’allocations demandées encore vivantes,
y compris les vrais buckets de `HashMap`, tous les buffers et les capacités de
vecteurs. Les allocations temporaires de construction sont libérées avant lecture.
Le programme vérifie à chaque exécution l’égalité exacte :

```text
requested_heap = nodes_bytes + identity_index_bytes + forward_bytes + reverse_bytes
```

`empty_headers_bytes` est une **partie** de `forward_bytes`, pas une allocation à
additionner. La mémoire demandée ne comprend pas les en-têtes du graphe sur la pile,
les surcoûts internes de l’allocateur, la fragmentation, les pages de code, la mémoire
du moteur réel ni une concurrence de lecteurs/rédacteurs. `rss_anon_before` et
`rss_anon_after` sont des observations Linux de `/proc/self/status` en octets ; elles
incluent le jeu de référence et des pages que l’allocateur peut conserver. Leur
différence n’est donc pas interchangeable avec `requested_heap`. Sur une plateforme
sans ce compteur, la valeur publiée est zéro.

Les champs `*_median_ns` sont les durées totales médianes de **sept tours**, après
un échauffement, avec `black_box`. Il faut les diviser par `*_ops` pour une durée
moyenne par requête ; aucune latence p95 de production n’est déduite de ces boucles.

- `forward_*` : zones tirées avec un PRNG déterministe, lecture des membres et somme
  de contrôle des IDs stables et de toutes les métadonnées. Le nombre de requêtes
  vise environ un million de liens visités, borné entre 4 096 et 65 536 requêtes ;
  à grande échelle le minimum peut dépasser cette cible. `forward_visited_edges`
  donne le travail réellement exécuté par tour.
- `reverse_*` : 500 000 appareils tirés de la même manière, recherche des deux zones
  et somme de contrôle de leurs IDs stables via l’index inverse commun.
- `scan_*` : parcours de tous les liens via les zones, répété pour viser un million
  de liens, avec au moins un parcours et au plus 65 536 ; `scan_visited_edges` est
  le travail par tour.

`checksum` lit tous les champs des nœuds et tous les liens. Cette somme et les trois
sommes de contrôle des requêtes doivent être identiques entre variantes pour un
même inventaire et une même échelle. Les mesures ne couvrent ni les écritures, ni
le coût de reconstruction, ni les autorisations, ni un moteur de stockage. Un gain
de ce microbanc reste à qualifier sur un chemin produit avant toute décision.
