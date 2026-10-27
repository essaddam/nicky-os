# Sonde MQTT live vers Hub

## POC V3 : description froide et valeurs différentielles

Les anciennes passes 9–17 sont historiques : leur arbre par hachage était un
banc synthétique, pas la topologie de la maison. Le mode `--virtual-tree`
utilise maintenant `src/house.rs` pour projeter le registre HA et statestream.
Ce POC est distinct du moteur Hub/K1 de référence, conservé sans ce drapeau.
Il ne constitue pas un nouveau pilote Apple TV ni un moteur Row64 livré.

`export_registry.py --mqtt`, exécuté sur l'hôte HA, publie les métadonnées
sélectionnées sur `moli-probe/source/registry`. Il utilise les credentials sur
l'hôte uniquement, sans les exporter. Le secret MQTT local est lu depuis
`MQTT_PASSWORD`, jamais depuis un argument ; à défaut, saisie terminal masquée.
Une référence de coffre non résolue est refusée. Aucun secret n'est écrit.

### Contrat pour les interfaces

Racine : `moli-probe/maison/<id-stable-de-l-installation>/`.
L'ID vient de `core.uuid`, indépendamment du nom et de l'ordre des ponts.
Les exemples sans identité amont restent provisoires, sous `pont/<instance>`.
Pour fédérer plusieurs sources dans une même maison Moli, le rattachement au
HouseId devra être explicite ; une connexion MQTT ne définit pas une maison.

| Topic retained | Contenu |
| --- | --- |
| `config` | Description V3 : IDs, parents, appareils, canaux, noms, unités, rôles, catalogues et contrats de commandes disponibles. |
| `canal/<id>/state` | Valeur, qualité, réception et référence de description du seul canal concerné. Aucun nom ni unité répété. |
| `noeud/<id>/state` | Valeurs des génériques, couverture connue/inconnue ; les membres et les libellés restent dans la description. |
| `systeme/state` | Volumes/débit, temps moyens de transformation, calcul des génériques, préparation des publications et écho MQTT. |

La souscription à `moli-probe/maison/#` reconstruit la maison : maison → étage →
pièce si un étage est déclaré, sinon maison → pièce. Les éléments sans
rattachement sont explicites. Un appareil porte plusieurs canaux ; la pièce de
l'entité prime sur celle de l'appareil. Aucun étage ni zone n'est inventé.
Les clés sont les identités du registre encodées, jamais un nom ni un rang dans
un tri. Les IDs locaux sont qualifiés par l'identité de la maison.

MQTT ne calcule pas de delta JSON. La projection publie le seul canal changé
(valeur/qualité), et les génériques concernés. La description repart à son
initialisation ou lors d'un changement réel de métadonnées. Le client accepte
les états avant la description et les résout ensuite ; il ne connaît pas HA.
Un rechargement du registre conserve les valeurs, qualités et dates déjà
reçues des attributs dynamiques et le catalogue média, pour la même identité
native. Un renommage ou déplacement recale leurs références ; un retrait ou
remplacement ne transmet aucune ancienne valeur au nouvel objet.

### Stockage froid et mémoire mesurée

En mode réel, `metadata.rs` sauvegarde la description dans un fichier temporaire
privé, mode 0600, hors dépôt, puis libère les chaînes et champs de noms d’affichage du modèle actif.
Les contrats de commande ne sont pas conservés comme un second document JSON
permanent. Les identités, unités, sémantiques, index de routage et valeurs utiles
aux calculs restent en mémoire. La description est relue à froid pour son envoi
ou une modification. Les tests couvrent aussi le stockage en mémoire.

`hot_label_bytes = 0` ne signifie PAS zéro mémoire de métadonnées ni zéro RSS.
La file d'envoi et la sérialisation ont un pic transitoire ; le cache de pages du
système reste à compter. En production, un stockage froid doit être sur disque,
pas une tmpfs si l'objectif est de réduire la RAM totale. Les budgets Row64 et
les layouts de l'étude ne sont pas ceux de cette projection JSON de POC.
Le broker externe conserve aussi les retained : cette preuve ne démontre pas une économie de RAM globale. La cible du broker embarqué devra servir une vue de la donnée canonique (descripteur partagé ou fichier indexé) avec un tampon borné, sans recopier durablement tous les payloads sérialisés. La passerelle web Rust implémente maintenant l’envoi des descriptions par blocs de 8 Kio en mode disque. La projection JSON et le Hub restent cependant deux représentations distinctes dans ce banc.

### Lumières et gestes du POC

La découverte MQTT des lumières est rapprochée par `unique_id` dans le périmètre
MQTT/light ; un identifiant homonyme de capteur n'autorise pas une commande.
Le manifeste fournit les paramètres acceptés : ON/OFF, intensité et température
du blanc lorsque déclarées. Les templates arbitraires et les topics invalides
sont refusés. Un retrait de découverte retire le contrat.

Les lumières d'une pièce partagent une tuile : clic pour basculer, clic droit ou
appui de 550 ms pour la liste individuelle. Un luminaire seul se bascule au clic ;
son clic droit/appui prolongé ouvre les réglages. L'appui long n'émet pas le clic
court à sa libération. Les bornes des curseurs viennent des sources.

Les commandes humaines du POC utilisent les topics virtuels déclarés. La
passerelle conserve le routage natif côté serveur et transmet les ordres en
QoS 1 **non retained**, sans reconnexion ni rejeu. Le retour d'état, et non le simple acquittement MQTT,
confirme l'effet. Les gestes/payloads sont vérifiés en navigateur avec un
transport simulé : aucun ordre physique de test n'est envoyé. Ce POC n'installe
pas la garde Moli : le futur routage des commandes virtuelles devra passer par
le Hub, avec origine externe et contrôle de garde, jamais en se déclarant Ui.
Les capacités absentes ne sont pas inventées ; les autres intégrations restent
en observation tant qu'aucun contrat de commande utilisable n'est exposé.

### Prises, sorties et couleur

Les contrats MQTT `switch` sont liés dans leur propre périmètre
`mqtt/switch/unique_id`, indépendamment des lumières. Chaque sortie conserve
son nom, son état et son topic de commande déclaré, y compris un payload
ON/OFF JSON fourni par la source. Les entités de configuration ou diagnostic
restent hors des commandes principales. Plusieurs sorties d’un appareil ne
prouvent pas qu’il s’agit physiquement d’une multiprise : l’interface parle de
sorties et ne fabrique pas cette classification. Une mesure sans lien explicite
avec une sortie reste une mesure de l’appareil.

Les couleurs JSON utilisent seulement les modèles déclarés : XY, HS ou RGB.
La passerelle refuse les modèles mélangés, les champs supplémentaires et les
coordonnées hors bornes. Les tableaux HA connus sont découpés en canaux
numériques nommés ; aucun objet arbitraire n’entre dans le chemin chaud. Le
sélecteur conserve un brouillon et exige un envoi explicite ; une couleur reçue
reste distincte d’une couleur choisie. Ce découpage JSON de POC n’est pas une
implémentation de couleur compacte dans le layout mémoire cible.

Références : [MQTT switch](https://www.home-assistant.io/integrations/switch.mqtt/),
[MQTT light JSON](https://www.home-assistant.io/integrations/light.mqtt/#json-schema).

### Runtime web et tunnel sécurisé

`web-gateway --port 1414 --description-storage memory` sert le POC, le
MQTT local en boucle locale sur 1415 et `/mqtt`. Le binaire est construit par
`cargo build --manifest-path tools/mqtt-live-probe/Cargo.toml --bin web-gateway`.
Le serveur Node `web-runtime/server.mjs` reste un ancien banc de comparaison ;
il est arrêté lorsque la passerelle Rust utilise le port 1414. Le navigateur utilise la même origine, en WSS derrière un tunnel
HTTPS. Un ticket signé de cinq minutes, obtenu avant chaque connexion,
autorise le WebSocket ; les identifiants MQTT restent obligatoires, même en
loopback. Le ticket ne remplace pas cette authentification. Aucun secret ne
passe dans une URL ni dans le manifeste. Les credentials restent en mémoire.

La passerelle ne dépend pas des réécritures Host/Origin du proxy. Le transport
réassemble le flux MQTT fragmenté et émet des blocs WebSocket de 8 Kio avec
contre-pression : les gros manifestes retained restent complets. Les diagnostics
détaillés sont accessibles par le socket Unix privé ; `/api/status` expose
uniquement la disponibilité et le port.

Les éditeurs suivent les paramètres déclarés : enum, booléen, nombre borné,
entier et texte borné. Les valeurs sont revalidées côté serveur et le payload
reste limité à 256 octets UTF-8. Une saisie reste un brouillon jusqu’à l’envoi ;
un nouveau retour du canal, après conversion déclarée, confirme la valeur.
L’intensité HA sur 0–255 est convertie vers la borne native, lorsque les
métadonnées identifient explicitement ce retour. Volets, chauffe-eau et lecteurs
restent en observation si aucun contrat de retour n’est raccordé : statestream
ne crée aucun service de commande.

### Descriptions initiales : RAM ou disque

`--description-storage memory` conserve une copie partagée de chaque description
retained en RAM. `--description-storage disk` garde un index en RAM et les
payloads des topics `config` dans des fichiers privés, transmis par blocs de
8 Kio. Les états différentiels restent en RAM dans les deux modes. Une lecture
client ne réécrit aucun fichier. Les valeurs identiques ne réécrivent pas non
plus la description ; seule une véritable modification de métadonnées produit
une nouvelle version. Les anciens fichiers sont supprimés après leur dernier
lecteur. Un arrêt brutal peut laisser un ancien répertoire privé à nettoyer.
Le cache de pages du système reste distinct du RSS de la passerelle.

Le stockage des descriptions du serveur web est indépendant de celui du Hub :
le lancement de ce banc conserve actuellement `--metadata-memory` dans le
processus de projection, pour isoler cette comparaison. Le mode disque ne
signifie donc pas que toutes les métadonnées de toute la chaîne quittent la RAM.

Le broker intégré est un sous-ensemble borné MQTT 3.1.1 pour ce POC : sessions
propres, publications entrantes QoS 0/1, abonnements et sorties QoS 0,
retained et authentification. QoS 2, wills et sessions persistantes sont refusés.
Les bornes sont 32 clients, 256 filtres par client, 4 Mio par paquet, 16 384
retained et 32 Mio de payloads ; les versions de descriptions sur disque sont
bornées à 16 Mio, lecteurs compris. Un client lent est déconnecté pour relire
l’instantané plutôt que de laisser croître sa file. Les commandes sont bornées
à 256 octets, quatre exécutions simultanées, file 128 et échéance cinq secondes.

La coque PWA et ses icônes sont décrites dans `web-runtime/public/README.md`.
Elle met en cache les fichiers statiques, sans identifiants ni états privés.
Le client envoie les commandes en QoS 0 sans file hors connexion ; la passerelle
les transmet au broker natif en QoS 1 sans reconnexion automatique. Le retour
d’état demeure la seule confirmation physique.

### Charge synthétique et workflows sans appareils

`python3 tools/mqtt-live-probe/run-load.py` lance un banc isolé : dix copies
de chacun des deux exports HA privés déjà disponibles, soit vingt maisons.
Les registres et messages statestream sont rendus synthétiques en RAM puis
transmis par MQTT TCP local vers les vrais House, Hub, broker et moteur
d'automatismes. Deux graphes par copie ne commandent qu'un pilote simulé ;
les notifications restent locales au Hub du banc. Aucun secret ni accès à un
broker réel n'est utilisé, et le service 1414 n'est pas redémarré.

Le binaire expose aussi `--synthetic-load --source <export-A> --source <export-B>
--copies 10 --seconds 31 --rate 8 --layout shared|per-house --storage memory|disk`.
Le nombre de copies est borné à dix par source, la durée à 120 secondes et le
débit à 256 messages/s/maison. Les plafonds du broker existant restent inchangés.
Le générateur et ses abonnés sont dans un autre processus que le worker mesuré.
Les identités stables empêchent les collisions entre copies ; les payloads ne
reproduisent pas les noms et valeurs privées.

Les résultats sont dans le [complément HTML V6](../../docs/proposition/analyse/v6/charge.html),
avec [protocole](../../docs/proposition/analyse/v6/charge-protocole.md) et JSON
agrégés. La reprise automatique conserve les résultats du même binaire et des
mêmes exports hors ligne, échecs compris. Le test initial corrigé reste archivé.
Le statut de chaque scénario compte : un script terminé ne prouve pas que tous
les scénarios sont validés. Les initialisations, limites et pertes sont conservées.

`python3 tools/mqtt-live-probe/snapshot-load.py` prépare deux graphes mémoire
macOS privés avec MallocStackLogging et les outils Apple. Il surveille un budget
RSS de 1 Gio, échantillonné toutes les 0,5 seconde : cette garde peut arrêter
le worker, mais n'est pas une limite mémoire cgroup. Le profilage modifie le coût
d'exécution ; ses chiffres ne remplacent pas les mesures non instrumentées.
Les memgraphs et sorties brutes restent hors dépôt ; seuls reçu et analyse
agrégée sont publiables. Le snapshot ne prouve pas, à lui seul, une fuite.

### Catalogues enum et états codés

Les listes explicites `options`, `operation_list`, `hvac_modes` et `source_list`
priment sur toute inférence numérique. Une option native « 1 » reste une enum.
Le document initial contient `catalogues` avec code, identifiant natif, libellé,
activité et version. Chaque canal référence `enumeration: {catalogue, version}`.
Son état contient `valeur` (code u8 ou null), `encodage: "enum"`, référence/version
et qualité, sans liste ni sélection textuelle répétée. La description est émise
avant les premiers codes de son catalogue ; le client attend la bonne version.

Les tables immuables partagent les ensembles compatibles. Un réordonnancement
ne recode rien ; un retrait conserve un emplacement inactif et une nouvelle
valeur ne récupère jamais le code d’une autre. Une valeur inconnue reste null
avec une qualité explicite. Le pont peut retenir temporairement un lexème en
attente de schéma (512 entrées, 128 Kio, 30 secondes), hors du cœur. Les codes
ne participent jamais aux génériques numériques.

`app_id` devient une sélection du catalogue d’applications. `app_name` enrichit
le libellé froid associé à l’identifiant courant et n’est plus un état chaud.
Un identifiant nouveau complète la description avant d’être publié ; son propre
identifiant sert de libellé tant qu’aucun nom connu n’a été reçu. Un snapshot
API atomique peut fournir `applications`, `app_id` et `app_name` ensemble.

L’entrée API normalisée `moli-probe/source/<instance>/snapshot` accepte un
objet `{entity_id, state, attributes}` de la source correspondante après son
registre, borné à 256 Kio. Elle ne constitue pas un raccord d’actions HA.
`MOLI_ENUM_CATALOGUE_DIR`, configuré par la passerelle dans son répertoire privé,
conserve les identités et associations de catalogues entre redémarrages. Le cache
ne contient aucun état courant ; les sauvegardes identiques ne réécrivent rien.
Sans ce cache, un usage autonome est limité à la stabilité de la session.

L’audit `enum-audit <states-A.json> <states-B.json>` chiffre les tables sans
exporter les valeurs privées. `mqtt-live-probe --offline-enum-audit <states…>`
rejoue les snapshots dans la vraie projection sans connexion MQTT ni écriture.
Les résultats des deux exports décrivent leurs snapshots, pas le débit ni la
fraîcheur des brokers réels. Les catalogues TV complets absents des exports ne
sont pas inventés à partir des seuls noms de sources.

### Génériques et médias

Température/humidité : moyenne par sémantique et unité, avec préférence pour le
capteur de pièce déclaré. Une température de chauffe-eau, une consigne, une
calibration ou un diagnostic ne devient pas la température ambiante. Ouvertures,
lumières, présences et chauffages actifs : nombre avec couverture explicite.
`heat` est un mode, pas une activité de chauffe. Un inconnu n'est jamais zéro.

Les médias exposent aussi `app_id`, `app_name`, titre, artiste, album, position,
durée, volume et capacités disponibles. `source_list` reste un catalogue borné
dans la description froide. Une application signalée par la source n'est pas
une preuve de l'application au premier plan. Voir la section Apple TV du rapport
V6-KIMI pour les mesures de longueurs et la proposition de représentation.
`media_position_updated_at` est affiché avec sa date et son âge ; aucune
progression ni fraîcheur globale ne sont inventées. Une date de dernière
modification ancienne peut aussi correspondre à un appareil resté éteint.

### Limites et vérification

Registre ≤ 4 Mio, 8 192 entités, 16 384 canaux, 2 048 appareils ; texte/scalaire
≤ 4 Kio, catalogue ≤ 128 chaînes de 256 octets. Seuls les attributs métier
explicitement admis sont projetés. Les valeurs retained gardent un âge inconnu.
La file coalesce les états par topic et attend la disponibilité du client MQTT ;
la boucle réseau continue indépendamment des ticks applicatifs. Les temps sont
mesurés avec une horloge monotone. La préparation inclut le manifeste initial ;
l'aller-retour MQTT ne mesure pas un appareil. Le suivi des RTT est borné à 64
entrées : ses échantillons ne représentent pas un décompte fiable des pertes.

`retained-audit --host <hôte>` relit indépendamment la projection et vérifie les
liens, libellés, canaux et métriques. `--snapshot <fichier>` est une option
explicite pour une copie privée 0600 hors dépôt ; les sorties standard restent
anonymisées. `media-audit <hôte>` relève uniquement les types et longueurs de
champs médias, jamais leurs valeurs. Les deux lisent `MQTT_PASSWORD`.

Le broker A a été relu sur des données réelles. Sur B, le registre est visible
avec le compte HA local, mais pas avec `test-mqtt` depuis la connexion de test :
la projection complète de B reste bloquée par cet accès. Aucune ACL, aucun
mode de protection ni appareil n'a été modifié. Reconnexion, retrait complet
d'objets, reprise persistante et budget de production restent à qualifier.

## Mesures historiques des passes 1–17

Les chiffres ci-dessous décrivent les versions précédentes ; le contrat actuel est celui de la section V3 ci-dessus.

Sonde Rust qui utilise les vraies structures de Moli pour relier un ou plusieurs brokers externes au Hub : `rumqttc`, catalogue Z2M, `Entry::decode`, `DriverCtx::set_state`, moteur K1 et analyse anonymisée de la découverte Home Assistant. Elle ne publie aucun ordre (`/set`, `/get`, `press`, commandes HA), n'écrit aucun fichier et n'exporte ni topic, ni nom, ni valeur, ni secret dans son résultat. Les comptages de découverte sont des agrégats : familles de topics whitelistées, domaines et suffixes HA, formes de segments (IEEE/MAC/numérique/UUID/mot), jamais les chaînes elles-mêmes.

## Tests hors ligne

```sh
RUSTUP_TOOLCHAIN=1.99.1 cargo test --offline --manifest-path tools/mqtt-live-probe/Cargo.toml -- --test-threads=1
RUSTUP_TOOLCHAIN=1.99.1 cargo test --offline --manifest-path tools/mqtt-live-probe/Cargo.toml arithmetic_paths_are_bounded_and_allocation_counted -- --ignored --test-threads=1
RUSTUP_TOOLCHAIN=1.99.1 cargo clippy --offline --manifest-path tools/mqtt-live-probe/Cargo.toml --all-targets -- -D warnings
```

Le premier test d'allocation est isolé parce que le compteur global du processus se mélange avec les autres tests lorsqu'ils tournent en parallèle.

État du 26 octobre 2026 :

- 33 tests fonctionnels + 2 tests d'alarme réussis ;
- le test d'allocations isolé est réussi ;
- Clippy strict réussi ;
- banc arithmétique en build debug : recalcul de 32 sources ≈ 2,344 µs, accumulateur par delta ≈ 19 ns, aucun octet alloué dans les deux chemins. Cette mesure n'est ni release, ni Hub, ni transport : elle sert à choisir ce qu'il faut mesurer ensuite, pas à justifier un nouveau moteur.

## Broker réel, en lecture seule

```sh
target/mqtt-live-probe/debug/mqtt-live-probe \
  --host <hôte-a> [--host <hôte-b>] --port <port> --username <utilisateur> \
  --seconds 30 --bridges 2 [--all-topics]
```

Le mot de passe est demandé dans le terminal sans être affiché ; il n'apparaît ni dans les arguments, ni dans le résultat. Chaque pont a un client ID distinct et partage le même Hub local pour vérifier les collisions (`z2m-0`, `z2m-1`) ; avec `--host` répété, chaque pont vise un broker différent. `--all-topics` souscrit `#` et `$SYS/#` (audit explicite) au lieu des seuls préfixes `zigbee2mqtt/#` et `homeassistant/#`. Le rapport indique les appareils/points du Hub partagé, les SubAck acceptés/refusés, les retained réappliqués, les timings parse/decode/apply, les octets alloués, la découverte HA (entités, groupes d'appareils, domaines, suffixes, profondeurs, formes), les métadonnées d'alarme sans codes et les formes de payloads sans valeurs.

## Passages réels du 26 octobre 2026, lecture seule

- **Passe 1-2** (périmètre ciblé, deux clients sur un Hub partagé, même broker) : 138 appareils et 1 164 points partagés ; 68 entrées de catalogue Z2M par pont ; 82 contrôles d'agrégat K1 (20 sources °C) sans écart ; 164 abonnements acceptés, aucun refus ; 7 049 retained, aucune réapplication identique.
- **Passe 3** (`--all-topics`, deux clients, même broker) : 692 entités HA ≠ appareils ; 74 topics d'état partagés ; familles `homeassistant` 7 326, `zigbee2mqtt` 392, `$SYS` 172, autres 44 ; aucune configuration d'alarme MQTT.
- **Passe 4** (ajout domaines/suffixes HA) : 805 messages `/state` et 6 008 autres suffixes sous `homeassistant/<mot>/…` non-domaines — motif à identifier.
- **Passe 5** (deux brokers DISTINCTS, un Hub partagé) : 96 appareils sans collision (68 + 26 + 2 agrégats), client IDs uniques, 0 erreur de transport ; broker B : 27 groupes d'appareils, 267 entités, famille `alarmo` présente (3 messages), absente du broker A.
- **Passe 6-7** (formes de segments + whitelist étendue, mono-broker A) : le motif mystère est résolu — `mqtt_statestream` (`base_topic: homeassistant/statestream`, attributs et horodatages activés, confirmé par la configuration lue en SSH) publie chaque entité incluse et chacun de ses attributs en topic retained : 7 310 des 8 002 messages `homeassistant`, 821 `/state`, 6 489 attributs, profondeur 5. Vérifié identique sur le broker B. 74 groupes d'appareils HA (dont 68 entrées Z2M), 91 contrôles d'agrégat sans écart, 1 796 valeurs appliquées → 175 états changés sans fenêtre fixe, aucun abonnement refusé.
- **Passe 8** (`--republish moli-probe`, mono-broker A) : l'agrégat K1 (moyenne °C) est republié sur `moli-probe/aggregate/state` uniquement sur changement — 1 publication, 1 retour, 0 sans réponse, aller-retour ≈ 4,5 ms ; `catalog_kinds` du catalogue réel : numeric 333, enum 124, binary 113, text 11 ; formes appliquées au Hub : int 783, text 313, bool 301, float 170, null 150. La chaîne « message → mémoire → calcul → virtuel publié » est prouvée de bout en bout.
- **Passe 9** (`--virtual-tree --republish-retain`) : arbre de 15 virtuels (maison + 2 étages + 4 pièces + 8 zones, partition déterministe par hash du PointId) installés dans le Hub partagé (84 appareils) et republiés retained sous `moli-probe/maison/…` ; 13 publications, 13 retours, 0 perdu ; 1 984 contrôles de parité sans écart.
- **Passe 10** (alias + histogramme de textes) : 4 alias détectés par capacité (`contact`→porte, `occupancy`→présence, `brightness`→lumière, `power`+`state`→prise) ; longueurs de texte mesurées — Hub : 100 % ≤ 16 o (641 ≤ 8 o) ; broker/statestream : majorité 17–64 o (3 071), 2 payloads > 4 000 o. `alias_republished` à 0 : les états retained des appareils-alias avaient été livrés avant la construction du catalogue.
- **Passe 11** (garde pré-catalogue) : course infirmée sur ce broker (`pre_catalog_messages: 0` — le catalogue arrive en premier), mais le buffer borné (512) + rejeu reste en place et testé hors ligne.
- **Passe 12-13** (`--self-test-commands`) : ligne `SUMMARY` en fin de rapport ; chemin de commande retour prouvé de bout en bout par auto-test — 2 commandes reçues via le broker, 1 acceptée (lumière), 1 refusée (porte, lecture seule), validation bornée en µs ; alias republié quand sa source parle (1 changement observé).
- **Passe 14** (`--virtual-mirror` + arbre + agrégats extra) : **un virtuel par appareil physique** — 64 topics `moli-probe/appareil/NNN/state` (numérotés, jamais nommés), 327 republications d'état en 60 s ; agrégats métier ajoutés : `reseau/linkquality-moyenne`, `reseau/linkquality-min` (maillon le plus faible du maillage), `energie/batterie-min`, `energie/batterie-moyenne` — 19 virtuels au total (15 arbre + 4 extra), parité sans écart ; auto-test commandes confirmé (1 acceptée, 1 refusée lecture seule). Limite documentée : « nombre d'appareils en ligne » n'est pas exprimable en K1 (Min/Max/Mean sur points numériques) — c'est l'extension Count/Sum identifiée, pas un contournement.
- **Passe 15** (`--publish-config`) : le document `moli-probe/config` retained (35 986 octets) porte toute la structure — nœuds avec noms lisibles, topics, unités, sources ; appareils avec nom d'affichage, points (clé, genre, unité), alimentation ; rôles avec topics d'état, de commande et contrats bornés. Les noms vivent sur le broker de la maison, jamais dans la sortie de la sonde.

## Interface web POC

`docs/proposition/analyse/poc-interface-mqtt.html` se connecte au broker en MQTT sur WebSocket (port 1884) : à l'ouverture, la config retained puis tous les états retained arrivent d'un coup ; ensuite seulement les différentiels. Vérifié dans un vrai navigateur : cartes par pièce avec icônes teintées, tuiles d'appareils, rangées d'actions, panneau de détail, commandes publiées et journalisées — zéro erreur console ; mauvais mot de passe refusé proprement (« Not authorized »). La hiérarchie garde des IDs stables dans les topics ; les noms lisibles viennent des états auto-décrits et de la config — un nom n'est pas une clé. L'aspect suit les maquettes « tableaux de bord » du projet (fond clair, cartes blanches, bleu #4b74f2, crème #fff2dc pour l'actif).

## Deux brokers en même temps (passe 17)

`--host A --host B --bridges 2` : deux brokers distincts sur un Hub partagé — 134 appareils, aucune collision, parité K1 sans écart sur les deux. Anomalie réelle trouvée : le broker B n'a renvoyé aucune des publications du pont (12 publiées, 0 écho, 0 commande auto-test reçue, 0 erreur de transport) alors que le broker A renvoie tout (32/32). À vérifier côté broker B (ACL ou filtrage des écrits du compte) ; la leçon est déjà dans le rapport : le pont doit mesurer par broker si ses écrits reviennent.

Enseignements : un comptage MQTT n'est pas un inventaire de la maison ; un topic n'est pas une identité (74 topics partagés) ; un pont qui lit `homeassistant/statestream/#` atteint toutes les entités HA, y compris celles sans découverte MQTT ; les textes libres (titres, applications) dominent les payloads et doivent être bornés, jamais transformés en énumérations.
