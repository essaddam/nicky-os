# Banc isolé des alias purs — V3

Ce programme Rust autonome, sans dépendance, compare cinq façons de présenter une
même valeur canonique dans plusieurs équipements virtuels. Il ne modifie ni le
catalogue de production, ni le banc précédent `tools/catalogue-banc/`. Aucun accès
réseau, appareil ou export privé. Les charges TSV publiques du banc précédent
suffisent.

```sh
docker run --rm \
  -v "$PWD/tools/alias-banc:/work:ro" \
  -v "$PWD/tools/catalogue-banc:/catalogue:ro" \
  -w /work moli-os-dev sh -c '
    rustc --edition=2021 -D warnings -C opt-level=s --test main.rs -o /tmp/alias-tests &&
    /tmp/alias-tests &&
    rustc --edition=2021 -D warnings -C opt-level=s main.rs -o /tmp/alias-banc &&
    /tmp/alias-banc --bench /catalogue/charges/somme-detail.tsv handle 1 4 0
  '
```

Interface :

```text
--bench INPUT.tsv row_alias|mirror|handle|pointer|id_lookup SCALE FANOUT [POOL_BYTES_PER_WORLD]
```

Une invocation = une variante dans un processus neuf. Les scripts de campagne et
le rapport sont séparés du programme. Le profil `opt-level=s` correspond au choix
de taille du projet ; les résultats restent ceux du compilateur et de la machine
indiqués par la campagne.

## Données et identité

L'entrée est exactement le TSV à douze entiers décimaux non signés décrit dans
[`../catalogue-banc/README.md`](../catalogue-banc/README.md) :

```text
kind parent value_kind flags payload observed_ms received_ms changed_ms min max step anchor
```

Les séparateurs sont des tabulations. Les parents sont des indices de ligne à
partir de zéro, `4294967295` signifie absence. Les lignes vides et les commentaires
`#` sont ignorés. Types : équipement 0, valeur 1, action 2 ; valeur inconnue 0,
booléen 1, entier signé 2, flottant 3, texte 4, enregistrement 5. Les valeurs et les
quatre paramètres numériques conservent leurs bits exacts. Les textes et les
enregistrements sont des handles immuables : le banc ne réalise aucun décodage.

Toutes les lignes d'entrée sont conservées en `Row96`, répliquées `SCALE` fois,
avec les parents remappés. Les sources des alias sont seulement les lignes
`kind=1` sans parent, ou dont le parent est un équipement `kind=0`. Les enfants
d'un enregistrement restent présents dans le catalogue canonique, sans devenir
séparément des sources d'alias. Sur `somme-detail.tsv`, cela conserve 12 243 lignes
canoniques et choisit 2 264 sources par monde.

Chaque source reçoit `FANOUT` alias consécutifs, dans l'ordre des lignes. Tous les
alias sont distribués quatre par équipement virtuel : `ceil(alias_count / 4)`
équipements, chacun possédant sa propre `Row96`. Leur santé demeure inconnue ; le
banc ne déduit aucune disponibilité de leurs sources.

Les identifiants `u64` sont uniques dans trois plages séquentielles : lignes
canoniques, équipements virtuels, alias. Un alias possède trois identités,
`alias_id`, `owner_id`, `source_id`, soit un préfixe de 24 octets commun à toutes
les variantes. Sa vue n'est pas une ligne d'état complète. La ligne physique
garde son identifiant, son parent physique, son type, ses droits et ses métadonnées.
Un alias partage même les trois dates ; il ne reçoit pas une date de changement
locale. Une valeur calculée doit disposer d'une ligne canonique distincte : un
test démontre son indépendance, mais aucun moteur de calcul n'est chronométré.

## Variantes comparées

| Variante | Par alias, architecture 64 bits | Lecture | Mise à jour source |
|---|---:|---|---|
| `row_alias` | 96 octets | Handle de 32 octets + réserve de 64 octets ; référence à la source canonique | Écrit seulement la source |
| `mirror` | 128 octets | Préfixe 24 + slot/génération 8 + copie `Row96` 96 | Met à jour la source puis copie ses 96 octets vers chaque miroir |
| `handle` | 32 octets | Préfixe 24 + slot/génération 8 ; accès au vecteur canonique, génération puis ID vérifiés | Écrit seulement la source |
| `pointer` | 32 octets | Préfixe 24 + `NonNull<Row96>` 8 ; ID et marque de vie vérifiés | Écrit seulement la source, allocation figée |
| `id_lookup` | 32 octets | Préfixe 24 + réserve 8 ; résolution `source_id` dans l'index commun à chaque lecture | Écrit seulement la source |

`row_alias` représente l'enveloppe de l'ancienne proposition : un alias occupe
une ligne entière de 96 octets tout en référençant déjà l'état canonique. Ses
64 octets réservés sont de vrais octets alloués ; ils ne contiennent aucun second
état. La comparaison `row_alias` → `handle` mesure donc seulement le gain de
compaction du binding, soit 64 octets par alias. `mirror` constitue une autre
alternative, avec duplication réelle des 96 octets d'état et propagation des
copies : le gain face à `handle` y est de 96 octets par alias.

Chaque variante possède **le même index d'identités** `HashMap<u64, u64>` qui
contient les lignes canoniques, les équipements virtuels et les alias. La valeur
est un slot et un tag de type compactés dans un `u64`. Il sert à retrouver une
identité externe ; `handle` et `pointer` évitent cette résolution dans leur lecture
interne chronométrée. Résoudre l'identifiant externe d'un alias n'est chronométré
dans aucune variante. Le coût mémoire de cet index demeure mesuré partout.

Chaque variante possède aussi le même index inverse CSR, de la source vers ses
alias. **Partager une valeur ne dispense pas de prévenir les abonnés** : toutes
les mises à jour chronométrées traversent cet index et consomment chaque
`alias_id` avec `black_box`. Seul `mirror` ajoute les copies d'état. Il n'y a ni
bus réel, ni allocation d'événement, ni sérialisation dans ce parcours.

Le vecteur canonique des handles peut grandir sans invalider leurs slots. Une
suppression invalide la génération puis laisse un tombstone. Une génération ne
boucle jamais : au débordement, il faut retirer définitivement le slot. La
vérification de l'identifiant rejette également la réutilisation accidentelle d'un
slot avec une autre identité. Le bit 15 des flags est réservé au tombstone par ce
banc et refusé en entrée. Ces mécanismes font l'objet de tests ; le chargement et
le chronométrage ordinaires ne suppriment aucune source.

Les pointeurs reposent sur une allocation initialement créée comme
`Box<[Row96]>`, transférée à un propriétaire privé avec `Box::into_raw` et
reconstituée une seule fois au `Drop`. Cette enveloppe évite de réemprunter une
`Box` exclusive pendant que ses pointeurs existent. L'allocation ne grandit pas,
ne se compacte pas, et reste vivante après la destruction des bindings. Les
lectures ne prêtent des références que pendant l'emprunt de `Bank` ; toute
mutation exige `&mut Bank`. Aucun lecteur concurrent, aucune référence retenue
pendant une mutation, aucun pointeur chargé depuis un fichier. Une suppression
peut marquer la ligne morte, mais ne libère jamais son allocation : la lecture
vérifie le tombstone avant de livrer l'observation. Les commentaires `SAFETY`
encadrent les opérations `unsafe`. `Pin` n'est pas nécessaire ici et ne
remplacerait pas ces invariants.

Ce prototype à propriétaire unique est monothread ; il ne compare ni `Arc`, ni
`RwLock`, ni des modèles à écritures concurrentes. Il ne propose pas d'intégrer
ces pointeurs dans le cœur de production. La persistance d'un cache, les epochs,
la reconstruction des bindings, les schémas complets, les transformations, les
cycles et les alias d'alias ne sont pas implémentés. Les alias générés pointent
tous directement vers une source canonique, sans chaîne.

## Mémoire et chronométrage

L'allocateur global compte les octets des **allocations encore vivantes**, tailles
demandées à `System`, et non un maximum cumulé. Le delta autour de la construction
du banc inclut :

- lignes canoniques et équipements virtuels ;
- bindings, générations, index inverse et liste des slots sources ;
- buckets et octets de contrôle de l'index global d'identités ;
- pool synthétique optionnel, effectivement alloué et touché.

Le programme vérifie que la somme détaillée égale exactement ce delta. L'entrée
parsée, les chaînes de la CLI et les buffers des opérations aléatoires sont hors
de ce budget. Les tailles demandées excluent l'en-tête de l'allocateur, sa
fragmentation et la pile. Les compteurs absolus RSS et RSS anonyme de
`/proc/self/status` sont fournis quand disponibles ; leurs différences reflètent
aussi les pages, la rétention de l'allocateur et le bruit du processus.

Chaque mesure utilise un tour de chauffe puis la médiane de sept tours :

- `scalar_ns_per_op` : 1 048 576 lectures aléatoires d'alias booléens, entiers ou
  flottants, charge utile et trois dates ;
- `numeric_ns_per_op` : 1 048 576 lectures aléatoires d'alias portant des
  métadonnées numériques, charge utile et `min/max/step/anchor` ;
- `source_update_notify_ns_per_op` : mises à jour centrales, parcours des
  notifications d'alias et copies des miroirs. Le nombre d'opérations par tour
  est `clamp(1_048_576 / FANOUT, 4_096, 1_048_576)`. Ce temps est **par changement
  de source**, comprenant le fanout complet, et non par alias.

Le générateur pseudo-aléatoire et l'ordre des modifications sont identiques pour
toutes les variantes. Les valeurs modifiées sont des booléens, entiers ou flottants
finis ; chaque visite change la valeur. La réception avance toujours, la date de
changement n'avance que si la valeur change et une observation absente conserve
l'ancienne date. `+0.0 == -0.0` mais les nouveaux bits sont conservés ; les NaN
restent inégaux. Les tests couvrent aussi des réceptions égales sans changement.

Avant et après les mesures, chaque alias est comparé champ par champ à sa source
canonique. Deux checksums incluent les identités de vue, le type, les flags,
toutes les dates et les métadonnées numériques ; la campagne doit vérifier leur
égalité entre variantes pour un même couple échelle/fanout. L'égalité de checksum
ne suffit pas seule : la vérification exacte de chaque observation est également
exécutée. Les opérations de test et de vérification sont hors chronométrage.

La sortie est une ligne JSON. Les champs `*_bytes` donnent la décomposition,
`binding_bytes` la taille d'un binding, `allocations_requested_bytes` leur somme,
`parity` le succès des vérifications exactes, et les champs `*_timing_available`
indiquent si la charge contient les types nécessaires. Les tests du programme
peuvent être exécutés en parallèle ; leur compteur global ne sert pas à valider
la mémoire d'un test individuel. La mesure de mémoire se fait dans l'exécutable
monothread, avec une seule variante par processus.
