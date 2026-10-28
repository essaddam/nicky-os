# MQTT natif vers identités canoniques : banc isolé

Ce prototype Rust, bibliothèque standard seulement, étudie le besoin
[MQTT-CANON-01](../../docs/proposition/analyse/orientations-v4/besoin-mqtt-canonique.json).
Il ne modifie aucun pilote, ne lit aucune installation et n'ouvre aucune connexion.
Les identités `fixture-native-*` et les topics `fixture-topic-*` sont générés.
Les propriétés et mots natifs sont ordinaires : `temperature`, `power`, `state`,
`contact`, `battery`, `action`, `ON`, `OFF`, `CLOSED`, `OPEN`, `single`.

**La mesure commence après décodage JSON.** Les entrées sont des structures de
scalaires déjà décodés, et non des paquets MQTT ou du texte à parser. Il n'y a
aucun pseudo-parseur JSON. Pour `cached_binding`, le routage, la validation du
schéma et l'association de la propriété native au handle ont **déjà été faits**.
Le résultat de cette variante ne mesure donc jamais un message MQTT complet.

## Contrat commun aux trois variantes

Un appareil possède six déclarations entrantes :

| Propriété native | Conversion | Stockage canonique |
|---|---|---|
| `temperature` | entier signé `215` × `0,1` → flottant fini `21,5 °C` | état numérique |
| `power` | flottant fini positif ou nul | état en W |
| `state` | exactement `ON` → vrai, `OFF` → faux | état booléen, commande distincte autorisée |
| `contact` | exactement `CLOSED` → vrai, `OPEN` → faux | état booléen en lecture seule |
| `battery` | entier entre 0 et 100, conservé entier | état en % |
| `action` | exactement `single` | occurrence transitoire, aucune ligne d'état |

Un mot inconnu, `null`, NaN ou un type inadéquat est refusé ; il ne devient jamais
`false` ou zéro. La conversion avec échelle refuse les entiers qui perdraient
leur précision avant multiplication (`abs(n) > 2^53`) ; ce contrat conservateur
du prototype ne prétend pas définir toutes les conversions numériques du produit.

L'équipement, ses six canaux et ses deux contrats de commande ont neuf IDs `u64`
uniques par appareil. Les cinq valeurs retenues utilisent exactement le layout
`Row64 + Numeric32 = Row96` du [banc d'alias](../alias-banc/main.rs). Chaque ligne
contient notamment l'ID du point, celui de l'équipement, le payload typé, les
instants observé/reçu/changé et une génération. Renouveler une valeur égale
actualise `received_ms` sans déplacer `changed_ms`.

Le canal `action` n'est pas une commande : deux `single` produisent deux identités
`(ID du canal, séquence locale d'occurrence)`. La séquence est locale à cette vie du
processus et n'est pas une identité durable globale. Aucun événement n'est écrit
dans `Row96` et aucun historique ni bus d'événements n'est mesuré.

Les deux contrats sortants sont séparés : setter booléen `state` et bouton
synthétique `identify` (pression représentée par `null`). Le décodeur de commande
retourne seulement une **intention**, avec ID d'action et payload natif. Il ne
publie rien et ne change aucune valeur observée. `identify` est un contrat fictif
du banc, pas une capacité Zigbee2MQTT qualifiée. Les lectures seules et le canal
entrant `action` refusent les commandes. Dans une intégration, toute intention doit
passer par le hub, la garde, les échéances et les règles de confirmation existants.
Le prototype n'implémente aucun moteur d'autorisation.

L'identité native immuable est séparée du topic renommable. Le test de renommage
conserve les IDs canoniques ; il invalide l'époque des anciens handles. Un handle
`(device_slot: u32, field_slot: u16, generation: u16)` occupe **8 octets**, et son
époque `u64` est transportée séparément. Les lectures refusent une mauvaise époque
ou génération. La gestion durable de l'allocation d'IDs, les suppressions, le
recyclage réel des slots et l'épuisement des générations restent hors du banc.
Les IDs sont attribués à la construction de cette fixture ; la stabilité au
redémarrage ou à la redécouverte n'est pas démontrée.

## Trois algorithmes, une même mémoire persistante

| Variante CLI | Travail mesuré pour chaque propriété décodée |
|---|---|
| `allocating_lookup` | alloue une `String` pour le topic et une pour la propriété, recherche dans deux `HashMap` imbriquées, convertit et met à jour |
| `borrowed_lookup` | utilise les mêmes dictionnaires avec `.get(&str)`, convertit et met à jour |
| `cached_binding` | reçoit le handle prévalidé, vérifie époque/génération/identités, convertit et met à jour |

**Les trois stratégies utilisent exactement le même `Bank`.** Toutes gardent
l'index de routage initial par topic et propriété, les chaînes natives possédées,
les descripteurs avec leurs codecs/mots binaires et unités, les contrats d'action,
les IDs canoniques et leurs slots de cache. Le chemin avec cache n'élimine pas
l'index nécessaire à sa compilation. Il n'y a donc aucun gain attendu de RAM
persistante entre variantes dans ce modèle. Il compare le coût des recherches et
la création/destruction de chaînes temporaires ; le tas vivant seul cacherait
cette dernière dépense.

Les domaines privés sont absents du registre : `pin_code` et ses sous-propriétés,
ainsi que les noms équivalents, sont reconnus par une illustration locale de la
liste de `moli_core::privacy::code_property`. Les propriétés inconnues ne créent
aucun point. **Ce n'est pas une validation complète de confidentialité.** La
production doit appeler les fonctions partagées `code_property`, `private_json`
et `private_value`, y compris pour les objets, composites, textes JSON imbriqués,
profondeurs et tailles bornées. Le banc n'accepte que ses scalaires prédéfinis et ne
mesure ni ces filtres complets ni la découverte hostile. Voir
[`catalog.rs`](../../crates/moli-z2m/src/catalog.rs),
[`privacy.rs`](../../crates/moli-core/src/privacy.rs) et
[le chemin des commandes](../../docs/components/coeur.md).

## Rejouer

Depuis la racine du dépôt, avec l'image de développement existante :

```sh
docker run --rm --network none -v "$PWD/tools/mqtt-canonicalisation-banc:/bench:ro" moli-os-dev:latest sh -c 'rustc --edition=2024 -D warnings -C opt-level=s --test /bench/main.rs -o /tmp/mqtt-tests && /tmp/mqtt-tests --test-threads=1'
docker run --rm --network none -v "$PWD/tools/mqtt-canonicalisation-banc:/bench:ro" moli-os-dev:latest sh -c 'rustc --edition=2024 -D warnings -C opt-level=s /bench/main.rs -o /tmp/mqtt-bench && /tmp/mqtt-bench --bench allocating_lookup 100 1 && /tmp/mqtt-bench --bench borrowed_lookup 100 1 && /tmp/mqtt-bench --bench cached_binding 100 1'
```

CLI : `--bench VARIANTE DEVICES SCALE`. Les deux entiers doivent être positifs,
leur produit est borné à un million. `DEVICES` est la base et `SCALE` le facteur ;
les copies ont des identités uniques. La sortie est un objet JSON par invocation.
La compilation, les fixtures et les exécutables restent dans le conteneur.

Deux compilations de la **même source** séparent les mesures :

- par défaut, `measurement_mode: "counted"` mesure le tas et les allocations ;
- avec `rustc --cfg timing_plain`, `measurement_mode: "timing_plain"` conserve
  le wrapper de `System` mais désactive ses compteurs atomiques. Ce mode fournit
  les temps principaux, sans surcoût de comptage par allocation/libération.

Exemple pour les temps et les mêmes invariants :

```sh
docker run --rm --network none -v "$PWD/tools/mqtt-canonicalisation-banc:/bench:ro" moli-os-dev:latest sh -c 'rustc --edition=2024 -D warnings -C opt-level=s --cfg timing_plain --test /bench/main.rs -o /tmp/mqtt-tests-plain && /tmp/mqtt-tests-plain --test-threads=1 && rustc --edition=2024 -D warnings -C opt-level=s --cfg timing_plain /bench/main.rs -o /tmp/mqtt-plain && /tmp/mqtt-plain --bench allocating_lookup 100 1 && /tmp/mqtt-plain --bench borrowed_lookup 100 1 && /tmp/mqtt-plain --bench cached_binding 100 1'
```

**En mode `timing_plain`, les champs `heap_*`, `allocation_calls_*`,
`temporary_requested_bytes_per_op` et `requested_bytes_timed_total` valent zéro
par absence de comptage. Ces zéros ne prouvent ni absence d'allocation ni absence
de mémoire consommée.** Les tailles structurelles et celle du buffer préparé
restent calculées normalement. Les stratégies, le registre, le corpus et les
checksums sont inchangés entre les deux compilations.

Chaque processus crée le registre commun, puis prépare **1 048 576 entrées**,
exclues du tas attribué au registre et du chronométrage. Les entrées empruntent
les chaînes d'un corpus possédé séparément. L'association au handle est précalculée
avec le vrai index du modèle, hors chronométrage. La fixture visite tous les champs
de chaque appareil successivement ; si le nombre d'appareils dépasse la couverture
des entrées, certains appareils sont enregistrés sans recevoir de mise à jour.

Une passe de parité par stratégie et une passe d'échauffement sont exclues des
compteurs. Sept passes sont chronométrées, avec réinitialisation hors mesure avant
chacune. La mesure inclut résolution éventuelle, validation du handle, codec,
écriture canonique et mélange du résultat dans un checksum (`black_box`). Le scan
final du stockage vérifiant la parité est hors chronométrage. En mode `counted`,
les atomiques de comptage ajoutent un coût aux allocations et libérations : la
stratégie qui alloue en subit davantage. Ses temps ne doivent donc pas servir de
comparaison principale des algorithmes. Utiliser les temps `timing_plain` et les
chiffres de mémoire/allocation `counted`, à source et paramètres identiques.

## Lire les champs JSON

| Champ | Sens |
|---|---|
| `measurement_mode` | `counted` pour mémoire/allocations, `timing_plain` pour les temps principaux |
| `devices`, `scale`, `total_devices` | base, facteur, produit |
| `retained_rows`, `mapping_descriptors`, `action_contracts` | respectivement 5, 6, 2 par appareil |
| `row_bytes`, `binding_bytes` | tailles réelles sur la cible compilée |
| `heap_live_requested_bytes` | delta des octets demandés à `System` lors de la construction du seul `Bank` |
| `heap_rows_bytes` | allocation du vecteur des `Row96` |
| `heap_descriptors_bytes` | vecteur de descripteurs, clés natives possédées et mots des codecs |
| `heap_devices_bytes` | registre d'équipements et chaînes d'identité native |
| `heap_routes_bytes` | index par topic, dictionnaires de propriétés/commandes, clés possédées et capacités |
| `heap_action_contracts_bytes` | vecteur de contrats et clés natives possédées |
| `prepared_input_buffer_bytes_excluded` | capacité du vecteur d'entrées ; corpus de chaînes également exclu, pas inclus dans ce nombre |
| `updates_per_round`, `timed_rounds` | 1 048 576 propriétés, 7 passes chronométrées |
| `update_ns_per_op` | médiane des 7 durées divisée par le nombre de propriétés |
| `allocation_calls_per_round` | médiane des appels alloc/alloc_zeroed/realloc pendant une passe |
| `allocation_calls_timed_total` | somme sur les 7 passes chronométrées uniquement |
| `temporary_requested_bytes_per_op` | médiane des octets demandés pendant une passe, divisée par le nombre de propriétés |
| `requested_bytes_timed_total` | somme des octets demandés sur les 7 passes chronométrées |
| `checksum`, `parity` | même checksum de résultats et du stockage pour les trois stratégies dans ce processus |

En mode `counted`, la somme des cinq postes du tas égale
`heap_live_requested_bytes`, vérifiée à l'exécution. Les compteurs sont globaux au
processus : la mesure est mono-thread. En mode `timing_plain`, les compteurs ne
sont pas mis à jour ; leurs lectures hors boucle chronométrée retournent zéro.
Les octets demandés n'incluent pas les en-têtes de l'allocateur, sa fragmentation,
les pages résidentes, la pile ou le binaire. Le coût vivant du registre comprend
les capacités des `HashMap`, mais pas les buffers d'entrée, les arguments CLI ou le
runtime du processus. Ces chiffres ne sont donc ni un RSS ni une projection du
serveur complet.

Les 16 tests couvrent les conversions exactes, les valeurs invalides et non finies,
les unités, les payloads entiers, les identités uniques, le renommage, la péremption
d'époque et de génération, les propriétés inconnues/privées, les instants reçu et
changé, les occurrences non retenues, les commandes en lecture seule, les intentions
sans effet et la parité des stratégies. Ils n'établissent pas un budget de
latence MQTT/JSON complet, une sécurité de production ou une qualification physique.
