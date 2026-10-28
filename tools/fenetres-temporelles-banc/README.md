# Fenêtres temporelles : preuves produit et étude de structures

Ce banc ne raccorde aucune nouvelle fonctionnalité au Hub. Il n'ouvre aucune
connexion, ne lit aucune installation et n'utilise que des valeurs synthétiques.
Les quatre fenêtres de moyenne numérique glissante **1 min, 5 min, 1 h et 24 h**
sont un **prototype autonome**, pas une capacité livrée de `computed`.

## Ce qui existe et ce qui est étudié

| Chemin | Contrat réel |
|---|---|
| `moli-core::AggregateSpec` | min/max/moyenne des dernières observations de plusieurs sources ; `max_age_s` borne leur fraîcheur, pas une fenêtre de moyenne temporelle |
| `moli-history::Store::series` | changements SQLite, ancre avant fenêtre ; brut ou AVG/min/max/count de changements ; borne supérieure inclusive (`BETWEEN`) |
| MCP `summarize` | moyenne des changements, buckets pesés par count et ancre pesée par 1 ; ni moyenne de tous les polls ni intégrale temporelle |
| `moli-rollup::hourly` / `group` | calcul pur livré de moyenne pondérée par durée, couverture/fraîcheur, heures UTC puis jours/mois locaux ; pas API de moyenne glissante arbitraire ni branchement `computed` |
| `moli-core::DurationLedger` | temps dans un état cible et couverture ; pas moyenne numérique |
| `main.rs` | trois structures proposées, deux capacités d'anneau ; aucun chemin produit modifié |

`production.rs` appelle les **vraies API publiques**, avec une base temporaire
synthétique supprimée à la fin. Exemple vérifié par assertions : trois polls
`0, 0, 100`, dont seulement deux changements enregistrés, valent respectivement
33,333… de moyenne de polls, 50 de moyenne de changements et 10 de moyenne
pondérée par durée lorsque 0 tient 90 % de l'heure. Un autre test vérifie l'ancre
de l'heure suivante, les zéros couverts, les valeurs absentes, les doublons, le
refus du désordre et des bornes non horaires, les unités compatibles/incompatibles
par `AggregateSpec`, la reprise de `DurationLedger` et les jours locaux de 23/25 h.
La moyenne temporelle du rollup est chronométrée sur 86 400 changements / 24 h.

## Contrat proposé pour la comparaison de structures

Les instants sont des millisecondes UTC d'une chronologie écoulée croissante. La
requête doit porter sur un instant supérieur ou égal au dernier événement reçu.
Une fenêtre est `[max(0, maintenant - durée), maintenant)` ; un événement à la
borne droite ne contribue pas encore. La moyenne arithmétique porte sur les
observations numériques finies dans cette fenêtre. La moyenne temporelle porte
sur la valeur maintenue de chaque observation jusqu'au prochain événement, à la
limite de **60 secondes de fraîcheur**, avec intersection exacte de la fenêtre.
Une ancre antérieure à la fenêtre peut donc contribuer à la moyenne temporelle.
Les quatre fenêtres partagent la même mémoire de rétention de 24 h.

La durée connue accompagne chaque moyenne temporelle. `None`, NaN et infini
ferment la couverture ; ils ne deviennent pas zéro. Zéro reste une valeur
couverte. Les moyennes vides valent `null`. La durée inconnue vaut la durée de la
fenêtre depuis l'origine zéro moins la durée connue. Les moyennes ne divisent
jamais par cette durée inconnue. La méthode de moyenne est une combinaison
convexe pour éviter le débordement `valeur × durée`.

Un événement désordonné est refusé. Un doublon au même instant remplace la dernière
observation et ne compte qu'une fois. **C'est un choix du prototype différent du
rollup livré**, qui compte toutes les lignes comme changements et maintient la
dernière valeur à cet instant. Aucune déduplication distribuée n'est démontrée.

Une reprise est représentée dans le test par une fermeture explicite à la date du
checkpoint ; le temps d'arrêt reste inconnu, jusqu'à une nouvelle observation.
Le prototype n'implémente ni fichier de checkpoint, ni sérialisation, ni garantie
de reprise après panne. Le test produit vérifie le contrat pur de reprise de
`DurationLedger`, sans tester le rédacteur durable du runtime ni une panne réelle.
L'ingestion suppose une seule unité déjà normalisée par flux ; les conversions
réelles sont testées séparément via l'API produit. Aucune addition de dimensions
incompatibles n'est proposée.

Une fenêtre UTC écoulée de 24 h reste 24 h. Elle ne désigne pas une journée civile
locale : le binaire produit montre directement des jours Europe/Paris de 23 h au
29 mars 2026 et 25 h au 25 octobre 2026, via `jiff` et `moli-rollup::group`.
Le prototype ne gère pas les sauts de l'horloge murale ; un raccordement au moteur
devrait réutiliser son ancre monotone et préciser la reprise.

## Variantes et compromis

| CLI | Structure | Exactitude / borne |
|---|---|---|
| `deque` | `VecDeque<Sample>` purgée en conservant une ancre | exacte sur les deux moyennes ; mémoire proportionnelle au nombre d'observations sur 24 h, donc sans borne en nombre sans admission de débit |
| `ring` | anneau préalloué de 65 536 observations | exacte tant qu'il ne déborde pas ; `limited=true` pour les fenêtres touchées, couverture perdue devenue inconnue |
| `ring_small` | même anneau, 2 048 observations | provoque volontairement la perte ; aucune exactitude affirmée quand `limited=true` |
| `buckets_10s` | anneau de 8 642 seaux fixes de 10 s, plus observation ouverte | mémoire indépendante du débit ; exact pour l'intégrale des seaux complets, approximation aux frontières partielles |

Les requêtes exactes parcourent les observations une seule fois et mettent à jour
les quatre fenêtres. Les seaux retiennent moyenne/compte d'observations et
moyenne/durée couverte des segments fermés. Aux frontières, toute la population
d'observations d'un seau intersecté est incluse ; la couverture est proratisée à
l'intersection, supposant sa répartition uniforme dans le seau. L'observation
ouverte est intégrée exactement. Les erreurs de moyenne ET de couverture sont
mesurées par fenêtre face à l'oracle, pas masquées. Aucun maximum universel d'erreur
sur la valeur n'est promis sans bornes sur ses variations ; l'erreur de couverture
est limitée aux seaux partiels. Les seaux ne constituent pas un sketch probabiliste.

L'oracle est un calcul hors ligne indépendant : intersections élémentaires depuis
le corpus complet, somme compensée des valeurs normalisées par amplitude et poids.
Il ne fait appel ni aux accumulateurs des structures, ni à leur fonction de
requête. Les tests incluent irrégularité, les quatre fenêtres, expiration sans
nouvelle lecture, bornes, absence/zéro, doublons/désordre, reprise explicite,
débordement et disparition du signal de perte, valeurs finies extrêmes et dérive.

## Reproduire

À la racine, avec l'image de développement du projet. Rien n'est installé sur
l'hôte. `rustc` suffit au premier binaire ; le second utilise Cargo et le code du
dépôt. Les dépendances doivent être disponibles dans le cache de construction pour
exécuter avec `--network none` / `--offline`.

```sh
docker run --rm --network none -v "$PWD:/work:ro" -w /work moli-os-dev:latest sh -c 'rustc --edition=2024 -D warnings --test tools/fenetres-temporelles-banc/main.rs -o /tmp/fenetres-tests && /tmp/fenetres-tests'
```

La crate autonome conserve son propre Cargo.lock ; `target` est ignoré. Compiler
les deux binaires dans le conteneur avec le même profil produit : `opt-level=s`,
`lto=fat`, `codegen-units=1`, défini dans `Cargo.toml`. Conserver dans les métadonnées
la commande, le compilateur, le digest de l'image et les limites de mesure réelles.
Les exécutables Linux sont lancés **dans Docker**, même si le runner Python est
lancé sur macOS. Les noms de volume et d'image ci-dessous sont des exemples à
adapter à la campagne réellement construite.

```sh
docker run --rm --network none -v "$PWD:/work" -v moli-target:/target -w /work -e CARGO_TARGET_DIR=/target moli-os-dev:latest cargo test --offline --manifest-path tools/fenetres-temporelles-banc/Cargo.toml --bins
docker run --rm --network none -v "$PWD:/work" -v moli-target:/target -w /work -e CARGO_TARGET_DIR=/target moli-os-dev:latest cargo build --offline --release --manifest-path tools/fenetres-temporelles-banc/Cargo.toml --bins
python3 tools/fenetres-temporelles-banc/executer.py \
  --structures 'docker run --rm --network none --cpus 1 --memory 512m -v moli-target:/target moli-os-dev:latest /target/release/structures' \
  --production 'docker run --rm --network none --cpus 1 --memory 512m -v moli-target:/target moli-os-dev:latest /target/release/production' \
  --metadata /tmp/fenetres-compilation-metadata.json \
  --output /tmp/fenetres-campagne
```

`--structures` et `--production` sont des chaînes de commande complètes, découpées
par `shlex.split`, puis exécutées avec `subprocess.run(..., shell=False)`. Les
arguments backend/charge sont ajoutés à la commande structures. Aucun interpréteur
shell implicite, aucune résolution du binaire Linux comme chemin macOS. Pour un
runner dans le même conteneur, une commande directe `/target/release/structures`
convient aussi.

`--metadata` désigne un fichier JSON **fourni par le responsable de campagne**,
repris sans inférer l'OS de mesure depuis l'hôte Python. Il doit documenter la
compilation et l'environnement effectif : Linux/architecture du conteneur,
compilateur, image et digest, profil/flags, source mesurée, volume des binaires,
CPU/mémoire imposés. Le runner archive le fichier et son empreinte ; il n'invente
aucune de ces informations et n'atteste pas à lui seul que le binaire fourni a
été compilé depuis les sources photographiées.

Le runner exécute **trois processus indépendants par backend/charge**, soit
`resultats-bruts.jsonl` de **36 mesures** et `resultats.json` de **12 synthèses**.
Chaque synthèse prend la médiane des trois médianes de processus ; chacun de ces
processus effectue déjà sept passes internes après échauffement. Les médianes de
chaque processus restent dans la synthèse et toutes les passes internes restent
dans le brut. Le binaire produit est aussi répété trois fois :
`production-brut.jsonl` et `production.json`.

Chaque répétition doit reproduire exactement **tous les champs hors chronos**,
donc valeurs, oracle, erreurs, couverture, signal de perte et mémoire structurelle.
Une différence interrompt la campagne. Les sources sont empreintées avant le
premier processus, avant/après chaque invocation et en fin de campagne. Le dossier
`sources/` garde les octets initiaux (`Cargo.lock` inclus s'il est présent) ;
`campagne.json` garde les deux empreintes, les contrôles et les commandes exactes.
Une mutation crée `source-race.json` et interrompt la campagne sans synthèse finale.
Garder aussi les preuves de compilation et sorties des tests : les empreintes du
runner ne remplacent pas le lien entre compilation et binaire mesuré.

Trois charges déterministes : `sparse` toutes les 30 s sur 24 h ; `irregular`
intervalles 0,5/1,5/3/2/1 s sur 24 h ; `burst` 100 000 observations toutes les
100 ms (environ 2 h 47). Les valeurs varient de 100 à 350 et un événement sur 193
ferme la couverture. Sept passes mesurées suivent une passe d'échauffement ;
construction et génération sont hors chronomètre d'ingestion. Les requêtes sont
128 lectures du même instant final après ingestion ; elles mesurent un état chaud,
pas un ordonnanceur ni les mises à jour concurrentes. Les échantillons de durée
sont conservés triés, avec leur médiane. Les fenêtres expirent par calcul de
requête ; aucune minuterie autonome n'est mesurée.

`heap_capacity_bytes` est une **estimation structurelle exacte des capacités des
buffers × taille réelle sur la cible**. Elle exclut l'en-tête du stockage, les
métadonnées de l'allocateur, la fragmentation, les entrées préparées, l'oracle et
le processus : ce n'est ni un RSS ni un comptage d'allocations. L'anneau réserve sa
capacité même sur flux peu actif ; la deque grandit. L'oracle et l'assertion de
qualité sont hors chronomètre. Chaque requête renvoie 4 fenêtres pour 1 point ;
ne pas extrapoler directement ces chiffres à toute une maison ou au moteur complet.

Aucune recommandation de structure ne peut s'appuyer seulement sur le temps : il
faut aussi vérifier erreur, couverture, signal `limited`, débit admissible et
mémoire. Le raccordement à `computed`, la sélection par point des fenêtres, les
limites globales, les unités par définition, la durabilité, la concurrence et les
API/écrans restent des travaux distincts.
