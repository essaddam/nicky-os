# Banc intégré du moteur existant

Ce programme lie les **vraies crates** `moli-core`, `moli-runtime` et
`moli-automation`. Il construit un vrai `Hub`, un pilote supervisé dont le
transport est exclusivement en mémoire, puis des calculs et un automatisme
validés. Il ne copie ni évaluateur de calculs, ni garde, ni moteur de graphes.
Aucune commande ne sort vers un appareil ou un service réseau.

Les trois dispositions mémoire sont des **adaptateurs de rejeu devant le même
Hub**, pas trois implémentations du Hub. Elles reprennent les structures du banc
`catalogue-banc` : lignes de 96 octets, lignes de 64 octets avec extras numériques,
colonnes chaud/froid de 32 + 32 octets avec extras numériques. Chaque appareil
possède quatre lignes, dont une numérique. Les identités sont résolues par des
handles `DeviceId` communs aux trois adaptateurs. Les dispositions internes du
produit restent celles des crates liées. Ce banc permet de comparer le coût
d'un adaptateur lorsqu'il alimente le moteur réel ; il ne démontre pas le gain
d'une réécriture du modèle interne de Moli.

## Exécuter

Depuis la racine du dépôt, avec l'image de développement déjà construite :

```sh
docker run --rm --network none \
  -v "$PWD:/src" -v moli-cargo:/usr/local/cargo/registry -v moli-target:/target \
  -e CARGO_TARGET_DIR=/target/moteur-integre moli-os-dev:latest \
  sh -c 'cargo build --release --offline --manifest-path tools/moteur-integre-banc/Cargo.toml && mkdir -p /target/moteur-integre/bin && cp /target/moteur-integre/release/moteur-integre-banc /target/moteur-integre/bin/counted && cargo build --release --offline --features plain-timing --manifest-path tools/moteur-integre-banc/Cargo.toml && cp /target/moteur-integre/release/moteur-integre-banc /target/moteur-integre/bin/timing'
```

Le lock dédié est inclus. À la toute première génération seulement, le lock du
dépôt sert de base ; Cargo le réduit et ajoute le package du banc. Ensuite,
ajouter `--locked` aux commandes de construction. Le profil release reprend
`opt-level = "s"`, LTO `fat`, une unité de génération et unwind du produit.

Un essai, contenant les assertions et une mesure JSON :

```sh
docker run --rm --network none -v moli-target:/target moli-os-dev:latest \
  /target/moteur-integre/bin/counted --devices 206 --rounds 10 --layout uniform96
```

La campagne peut être orchestrée par Python sur l'hôte, sans Python dans l'image :

```sh
python3 tools/moteur-integre-banc/executer.py \
  --counted-command 'docker run --rm --network none -v moli-target:/target moli-os-dev:latest /target/moteur-integre/bin/counted' \
  --timing-command 'docker run --rm --network none -v moli-target:/target moli-os-dev:latest /target/moteur-integre/bin/timing' \
  --sizes 206,290,496,2060,2900,4960 --rounds 10 --repetitions 3 \
  --output tools/moteur-integre-banc/sorties/campagne-01
```

Soit **108 processus indépendants**, chacun avec son Hub neuf. La péremption
réelle prend environ une seconde par processus. Les tailles sont des appareils
synthétiques : le modèle possède `4 × appareils + 3` points et trois définitions
calculées, même à ×10. Il ne franchit pas la limite produit de 128 définitions.
`--rounds` accepte 1 à 20 : le garde-fou réel des automatismes coupe au-delà de
20 départs par minute. Il reste activé pendant la mesure.

`brut.jsonl` conserve chaque résultat et `resultats.json` donne médiane/min/max
des processus par taille, disposition et mode. `campagne.json` conserve commandes,
arguments et SHA-256 des sources Rust liées et des fichiers Cargo. L'ordre des
dispositions tourne entre répétitions. Le runner refuse d'écraser une sortie
existante et échoue dès qu'un oracle ou un processus échoue. Une archive durable
doit aussi conserver les sources, le binaire, l'identifiant de l'image et la
toolchain ; les hashes du manifeste ne remplacent pas ces fichiers.

## Oracle exécuté à chaque processus

- Découverte par `DriverCtx`, état initial, disponibilité et statut supervisé.
- Un calcul brouillon reste nul. Une publication sans autorité humaine échoue.
- Moyenne de deux sources, maximum transitif de cette moyenne et d'une troisième
  source ; comparaison à chaque ronde (tolérance de quatre epsilon relatifs
  pour la moyenne mise à l'échelle en `f64`, égalité pour les autres valeurs).
- Péremption autonome d'un calcul à une seconde ; une observation brute identique
  le réactive sans fabriquer d'événement de changement brut.
- Un graphe enregistré par un agent reste un brouillon, puis la validation humaine
  active le vrai moteur. Chaque changement du déclencheur bascule exactement
  l'état cible par une commande passant par le Hub ; fin de run `Done` attendue.
- Setter et action en pièce synthétique protégée : aucun effet avant approbation,
  exécution après approbation. L'action autonome n'ajoute pas d'état persistant.
  Une action autonome supplémentaire est envoyée à chaque ronde : le compteur
  d'effets vaut exactement `1 + rondes`, sans état d'action à chaque contrôle.
- Refus d'écriture sur valeur en lecture seule et refus d'un mauvais type.
- Erreur du transport factice propagée ; retard de 80 ms face à une échéance
  de 40 ms : retour `Timeout`, puis barrière de transport prouvant l'absence
  d'effet tardif. Ces délais concernent seulement le transport synthétique.
- Deux pulses identiques produisent deux occurrences aux identités distinctes.
- Présence du journal des commandes ; collecte du bus sans perte autorisée.
- Nombre exact de changements numériques bruts par ronde ; **chaque valeur
  finale**, tous les états cibles et le nombre d'effets sont contrôlés. Le checksum
  supplémentaire sert à comparer les sorties des dispositions et modes.

Le graphe utilise seulement les changements d'état, pas les occurrences ; ces
dernières sont exercées jusqu'au bus du Hub. Le journal est conservé en mémoire,
borné à 128 entrées. Aucune persistance métier ou base SQLite n'est configurée.

## Lire les mesures

Le binaire `counted` utilise un allocateur compté : octets demandés vivants,
pic exact de ces octets pendant la charge et octets alloués cumulés. Il ajoute
des opérations atomiques à chaque allocation : **ses temps ne servent pas à
classer les dispositions**. Le binaire `timing` possède les mêmes chemins métier,
avec l'allocateur système sans compteur ; `measurement_mode = timing_plain` et
les champs d'allocations valent zéro, ce qui signifie **non mesuré**.

`adapter_live_requested_bytes` isole la différence d'allocations au chargement de
l'adaptateur. `adapter_payload_and_handles_bytes` calcule séparément les capacités
des tableaux, handles et identités ; il exclut l'en-tête d'allocation des chaînes.
`baseline_live_requested_bytes` est relevé après cet adaptateur, avant le Hub.
`hub_setup_net_requested_bytes` est la différence jusqu'au modèle chargé : elle
inclut le Hub, l'automatisme, le faux transport et la comptabilité du banc.
Ce n'est pas une attribution pure à un seul composant. Les valeurs `populated`,
`final` et `peak` sont celles du **processus entier**, adaptateur compris. Les
allocations système non Rust, la fragmentation et la taille des pages ne font
pas partie de ces compteurs.

Sous Linux, `loaded_rss_bytes`/`rss_bytes` et `loaded_anonymous_bytes`/
`steady_anonymous_bytes` donnent les pages résidentes avant/après la charge.
`process_peak_rss_bytes` vient de `VmHWM`, donc couvre le processus depuis son
démarrage. `sampled_peak_anonymous_bytes` est seulement un maximum **échantillonné**
au chargement et après chaque batch, pouvant manquer un pic transitoire. Hors
Linux, ces valeurs sont nulles. Un pic d'octets demandés n'est pas un pic RSS.

Les latences sont mesurées avec `Instant`, en nanosecondes :

- `batch_ack_latency_ns` : mise à jour de l'adaptateur, matérialisation du batch,
  canal vers le pilote et publications brutes jusqu'à son accusé de réception ;
- `propagation_and_automation_latency_ns` : depuis le début du même batch jusqu'au
  résultat calculé et à la fin de l'automatisme, avec attente **événementielle** ;
- `setter_round_trip_latency_ns` : commande valide via garde/routage/pilote/retour.
- `action_round_trip_latency_ns` : action autonome valide par le même chemin,
  avec retour du transport, sans publication d'état.

Le débit couvre la boucle complète : adaptateur, publications, calculs, automate,
setter, action autonome, collecte du bus, assertions et échantillonnage mémoire. Ce n'est pas le
débit maximal isolé du Hub. Les p50/p95 sont des percentiles par processus sur le
nombre de rondes demandé, sans interpolation ; avec dix échantillons, le p95
est le maximum. Les agrégats de campagne sont des médianes de ces percentiles,
pas des percentiles fusionnés. L'initialisation et les scénarios de sûreté sont
hors de cette fenêtre chronométrée.

## Portée et limites

Ce banc couvre un chemin intégré significatif du moteur existant. Il ne couvre
pas tout le produit : pas d'API HTTP/MCP, d'historique/énergie SQLite, de transport
MQTT/HTTP réel, de sauvegarde/reprise, de pilote natif, de panne superviseur, de
scène, de timer ou de calendrier. Les tests du workspace complètent cette preuve.
Les tailles et données sont synthétiques et ne décrivent aucune installation.
Le runtime Tokio possède un seul thread pour rendre les séquences reproductibles ;
ce banc ne constitue pas une mesure de contention multithread ou de concurrence
réseau. Les identités et temps exacts varient entre processus, mais les états,
effets et valeurs métier contrôlés sont déterministes.

`raw_observations` et `raw_observations_per_second` comptent uniquement les
observations numériques `/value` vérifiées, soit appareils × rondes. Chaque ronde
ajoute un état déclencheur, puis les commandes, leurs retours et les calculs : ces
échanges participent au temps total mais ne gonflent pas ce compteur numérique.
Ce débit ne désigne donc pas le total des messages du moteur.
