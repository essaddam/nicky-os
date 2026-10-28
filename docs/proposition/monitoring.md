# Monitoring — Sarah Home : ce que consomme chaque équipement, chaque processus, chaque module ; et des modules dans le cœur ou isolés

Date : 23 octobre 2026. Statut : **proposition, aucune modification du code**. Base analysée : `main` au
commit `a835893`, et les sources de `rumqttd` 0.20.0, `tokio` 1.54.1 et `rustix` 1.1.5, lues là où la
proposition s'appuie sur elles.

Proposition à part. Elle complète l'[analyse V7](isolation-reactivite-runtime-v7.md) (le courtier, § 6 ;
l'isolation des modules, § 9.4 ; un fil par pilote, § 20.1), l'[étude de haute disponibilité
v2](haute-disponibilite-v2.md) (le pont maison) et la [critique des workflows](critique-workflow.md) (la
zone de travail). Ta seconde idée, redémarrer le moteur sans rien perdre, a son propre document :
[Redémarrage à chaud](redemarrage-a-chaud.md).

Ta demande :

1. **Connaître la mémoire utilisée par les équipements, le CPU de l'ensemble, le CPU par module.**
2. **La mémoire de chaque module** : celui qui gère le pont, les modules API, et les autres.
3. **Le nombre de messages MQTT par module**, et le reste : mémoire, CPU…
4. Hors monitoring : **qu'un module tourne dans le cœur quand il a été validé, ou à part** ; et qu'on
   puisse **le relancer isolé, ou dans le cœur**, comme on le souhaite.

Les questions, reformulées pour être tranchables (« comment pourrait-on… ») :

| # | Comment pourrait-on… | § |
|---|---|---|
| 1 | savoir ce que consomme chaque équipement, chaque processus et chaque module, **sans que la mesure coûte plus que ce qu'elle mesure** ? | 4, 5, 8 |
| 2 | attribuer à un module **la mémoire et le CPU qu'il prend quand il partage le processus du cœur** ? | 5, 6 |
| 3 | compter les messages MQTT **de chaque module**, alors que les modules du cœur partagent un seul lien avec le courtier ? | 7 |
| 4 | voir venir une dérive (une fuite, une boucle, une rafale) **avant qu'elle ne touche la maison**, sans réveiller personne pour rien ? | 3, 10 |
| 5 | décider **où tourne un module**, et le faire passer du cœur à l'isolé, ou l'inverse, **sans redémarrer le cœur ni perdre un ordre** ? | 11 |

Méthode, et techniques employées, nommées au fil du texte :

- **lecture du code** : ce que Sarah Home mesure déjà, preuves `fichier:ligne` (§ 2, annexe B) ;
- **premiers principes** : quelles mesures brutes donnent toutes les autres (§ 4) ;
- **remue-méninges inversé** : « comment rendre le monitoring nuisible ? » ; chaque réponse, retournée,
  devient une règle (§ 3) ;
- **questions en étoile** : qui mesure, quoi, où, quand, comment, pour qui (§ 4) ;
- **analyse morphologique** : les façons de mesurer la mémoire d'un module qui partage le tas du cœur,
  côte à côte (§ 6) ;
- **analogies** : Erlang, dont chaque processus a son tas et se mesure exactement ; Vector, qui
  étiquette chaque allocation de son composant ; Zigbee2MQTT, qui publie sa propre santé ; Home Assistant,
  qui n'attribue rien par intégration (§ 6, § 7, annexe A) ;
- **méthode Disney** (rêveur, réaliste, critique) sur le passage d'un module du cœur à l'isolé (§ 11.6) ;
- **convergence à critères visibles** : exactitude, coût au repos, mémoire, `unsafe`, effort (§ 12) ;
- **une vérification dans un conteneur** : les fichiers `/proc` et le montage des cgroups, dans un
  conteneur Docker ordinaire (alpine, noyau 6.8), pour savoir ce qu'on peut lire sans privilège.

Vocabulaire :

- un **équipement** est une machine (celle de Sarah Home, la secours, un ordinateur de la maison, le
  routeur) ou un appareil qui sait dire sa propre charge (Frigate, Zigbee2MQTT) ;
- un **processus** est un programme qui tourne avec sa propre mémoire : le cœur (`moli-os serve`), un
  module isolé (`moli-os module run <id>`, V7 § 9.4.2), un voisin (Zigbee2MQTT, `moli-archive`, Wyoming) ;
- un **module** est une brique de Sarah Home qui a son code et sa place dans le contrat MQTT (V7 § 3) :
  un module api-to-mqtt (une intégration, ce que tu appelles les modules API), le hub, le courtier, le
  moteur des workflows, le pont maison, le module WebSocket, la base, Sarah ;
- **dans le cœur** : le module tourne dans le processus du cœur, sur son fil et son runtime (V7 § 20.1) ;
  aujourd'hui, sur le runtime partagé ;
- **isolé** : le module tourne dans son propre processus et parle au courtier par l'écouteur local (V7
  § 9.4.2). À ne pas confondre avec le mode « isolé » d'une machine dans l'étude de haute disponibilité
  v2 (§ 4), qui désigne une machine coupée de ses pairs ;
- **le pont** a deux sens dans les études : le **pont maison**, qui copie la mémoire vers les secours
  (HA v2 § 6.4), et les **ponts radio** (Zigbee2MQTT, demain Matter). Les deux sont mesurés ici ;
- l'**observatoire** est le module qui relève les mesures, les range et les sert. Il porte un nom à part
  pour ne pas le confondre avec le **superviseur**, qui relance les pilotes (`supervisor.rs`) ;
- l'**astreinte** est la personne qui reçoit les alertes techniques, le propriétaire, et pas la famille
  (ARCHITECTURE, « l'astreinte lira ce journal ») ;
- le **gardien** est le petit processus qui reste quand le cœur redémarre : il lance les modules isolés
  et garde ce qui doit survivre. Il est décrit dans le document [Redémarrage à chaud](redemarrage-a-chaud.md).

## 1. Le verdict en bref

- **Trois étages, une source de vérité chacun.** L'**équipement** se lit dans `/proc` et `/sys`, ce que
  fait déjà `moli-host` toutes les 15 s. Le **processus** se lit dans `/proc/<pid>`, exactement, sans une
  ligne de code dans le programme mesuré. Le **module** se lit selon sa place : isolé, c'est son
  processus ; dans le cœur, c'est son fil, ses compteurs et, pour la mémoire seulement, une estimation ou
  une étiquette.
- **Le CPU par module est exact dès qu'un module a son fil.** C'est la couche 1 de la V7 (un fil et un
  runtime par pilote, § 20.1). Le noyau compte alors le temps de chaque fil, en nanosecondes
  (`/proc/self/task/<tid>/schedstat`), et `tokio` 1.54 donne, sans option instable sur une machine
  64 bits, le temps occupé et le nombre de réveils de chaque runtime. Tant que tous les pilotes partagent
  le runtime, comme aujourd'hui, `TaskMonitor` (décision D8, jamais faite) donne une approximation.
- **La mémoire par module est la seule mesure difficile.** Un tas partagé ne sait pas à qui appartient
  un octet, et un octet alloué par un module est souvent libéré par un autre fil : le courtier garde les
  messages dans ses segments. Je propose quatre niveaux :
  - **toujours**, la mémoire de ce que le kit tient pour le module : files, voies, caches ;
  - **toujours**, la part que ces comptes n'expliquent pas, entre ce total et la mémoire anonyme du
    cœur ;
  - **en option de construction**, une étiquette d'un octet par allocation, comme Vector, à mesurer sur
    le Pi ;
  - **la vérité de terrain, à la demande** : relancer le module isolé et lire sa mémoire exacte.
- **Les messages MQTT par module, sans rien ajouter au chemin des messages.**
  - Isolé : le courtier compte sur sa connexion. `rumqttd` 0.20 tient déjà un compteur d'entrée par
    connexion, mais ne l'expose pas, et sa table par sujet fuit. Notre version le corrige.
  - Dans le cœur : le kit compte, par une addition atomique par message.
  - Zigbee2MQTT publie déjà ses propres comptes sur `bridge/health`, que `moli-z2m` reçoit sans le lire.
  - Montrer **la pire rafale**, pas seulement le débit : la V7 a montré que c'est elle qui fait décrocher
    (§ 17.1.2).
- **La mesure ne réveille rien et n'écrit presque rien.**
  - Un relevé par minute ; toutes les 10 s seulement quand quelqu'un regarde la page Système.
  - Les modules ne sont jamais réveillés : on les lit de l'extérieur.
  - En mémoire, ≈ 342 Kio de courbes pour 40 modules, 24 h au pas de 5 min.
  - Sur la carte : des agrégats de 15 min dans la famille `diagnostic` de l'historique, 7 jours, jamais
    archivés.
- **Où tourne un module : dans le cœur une fois validé, isolé sinon.** C'est ta règle. Elle tient sur les
  critères de la V7 (§ 9.4.2), avec des preuves pour « validé » : relu, vérifié par le catalogue, 7 jours
  isolé sans incident et dans son budget.
  - **Isolé** : un module nouveau, écrit par un agent, en code tiers ou en FFI, déjà planté, ou au-dessus
    de son budget.
  - **Dans le cœur, toujours** : le hub, le courtier, le moteur, l'API, et les modules qui partagent la
    passerelle HTTP de l'API.
  - **Rentrer dans le cœur** demande un humain. **Retourner à l'isolé** peut être automatique : c'est le
    sens sûr.
- **Passer d'une place à l'autre se fait à chaud, sans redémarrer le cœur**, parce que le même code est
  dans le binaire (V7 § 9.4.2).
  - Le hub retient les ordres dans leurs voies.
  - L'ancienne instance finit et rend un petit état (≤ 4 Kio), puis la nouvelle démarre dans l'autre
    mode.
  - Les appareils restent « en bascule », jamais « hors ligne », et aucun ordre n'est rejoué après son
    échéance.
- **Un module isolé est lancé par le gardien, pas par le cœur** : il survit ainsi au redémarrage du
  cœur. C'est le lien avec ta seconde idée, et le sujet du document
  [Redémarrage à chaud](redemarrage-a-chaud.md).

## 2. Ce qui existe aujourd'hui

| Mesure | Où | Rythme | Limite |
|---|---|---|---|
| CPU de la machine, charge, mémoire, échange, températures, réseau, durée de marche, cœurs | `moli-host` (`crates/moli-host/src/lib.rs`) : points `cpu`, `load`, `ram`, `ram_used`, `ram_total`, `swap`, `cpu_temperature`, `disk_temperature`, `net_down`, `net_up`, `uptime`, `cores` | 15 s (`POLL`, `lib.rs:21`) | la machine de Moli seulement ; « disques et conteneurs » sont laissés à un script de l'hôte (`data/infra.json`) |
| Les ordinateurs de la maison | l'agent Moli (`moli-host/src/agents.rs`) : CPU, mémoire, GPU, disque, réseau | au rapport de l'agent | seulement les machines qui ont l'agent |
| Routeurs | ASUSWRT (`cpu`, `memory`, `memory_total`, `memory_used`, `memory_free`), Netgear (`cpu`, `memory`) | au sondage du pilote | — |
| Frigate | `cpu_load` (`moli-frigate/src/lib.rs:246`, `:287`) | — | — |
| Le processus de Moli | `Stats` : `uptime_ms`, `devices`, `points`, `drivers`, `drivers_running`, `rss_bytes` ; ce dernier vient de `VmRSS` dans `/proc/self/status` (`moli-runtime/src/stats.rs:14-20`) | à la demande (`/api/health`, `/api/system`) | `VmRSS` compte aussi les pages du binaire ; aucune ventilation |
| Les pilotes | statut (`starting`, `running`, `waiting`, `backoff`, `stopped`), appareils, points, hors ligne, caméras (`moli-api/src/system.rs:95`, `:135-163`) | à la demande | ni CPU, ni mémoire, ni messages |
| Face à Home Assistant | `data/perf.jsonl`, écrit par un script de l'hôte (mémoire anonyme du cgroup, CPU sur 30 s) ; les 20 dernières lignes servies par `/api/system` (`system.rs:103`, `:166-176`) | à chaque déploiement | hors du dépôt ; pas une mesure continue |
| Décidé, pas fait | D8 : « `tokio-metrics` `TaskMonitor` par tâche supervisée, part de CPU et `0 poll` affichés sur la page Système » (ARCHITECTURE) | — | `tokio-metrics` n'est pas dans les dépendances du dépôt |
| Le superviseur | `spawn_driver` (`supervisor.rs:22-68`) : `tokio::spawn` sur le runtime partagé (`:31`), relance de 1 s à 60 s (`:16-19`, `:64`) | — | tous les pilotes partagent le runtime : un temps CPU par pilote n'existe pas encore |

Trois conclusions :

- **L'étage de l'équipement est déjà là**, pour la machine de Moli, les PC équipés de l'agent, deux
  routeurs et Frigate. Il manque la pression (le temps où l'on attend), les écritures sur la carte, le
  bridage du Pi et la santé de Zigbee2MQTT.
- **L'étage du processus se réduit à `VmRSS`**, une mesure qui ne correspond pas au budget : celui-ci
  compte la mémoire anonyme (PERFORMANCE.md).
- **L'étage du module n'existe pas.** Rien ne dit aujourd'hui ce que coûte Tuya plutôt que Hue.

## 3. Remue-méninges inversé : comment rendre le monitoring nuisible ?

La question à l'envers fait sortir les règles plus vite que la question à l'endroit. Chaque idée pour
**nuire** devient une règle, en la retournant :

| # | Pour nuire, on pourrait… | La règle qui en sort |
|---|---|---|
| N1 | relever tout chaque seconde, et réveiller chaque module pour qu'il se mesure | un relevé par minute ; 10 s seulement quand la page Système est ouverte ; les modules se lisent de l'extérieur, jamais réveillés |
| N2 | écrire chaque relevé sur la carte SD | le fin en mémoire, 24 h ; en base, seulement des agrégats de 15 min, 7 jours, famille `diagnostic`, jamais archivés |
| N3 | prendre plus de mémoire que les modules mesurés | un budget fixe : ≈ 342 Kio pour 40 modules ; des entiers de 16 bits, pas des flottants de 64 |
| N4 | afficher `VmRSS` et l'appeler « mémoire » | la mémoire anonyme, comme le budget ; `Pss` quand des processus partagent le binaire |
| N5 | attribuer à un module ce qu'un autre a libéré, et voir une fuite qui n'existe pas | une mémoire par module est mesurée, étiquetée ou estimée, et l'écran dit laquelle |
| N6 | montrer un module « à 0 % » parce qu'il est mort | un module mort est mort, pas sobre : statut d'abord, mesures ensuite |
| N7 | moyenner sur 5 min et rater la rafale de 5 000 messages qui fait décrocher le moteur | la pire rafale sur 100 ms est une mesure à part entière (V7 § 17.1.2) |
| N8 | alerter la famille à 3 h du matin parce qu'un nuage a répondu lentement | les alertes vont à l'astreinte, groupées par incident ; rien aux heures calmes, sauf si l'alarme, une serrure ou un détecteur est touché |
| N9 | laisser un compteur déborder et repartir de zéro, pour faire croire à une chute | des compteurs de 64 bits qui ne font que monter ; les débits se calculent par différence |
| N10 | perdre toutes les courbes à chaque redémarrage | elles passent au cœur suivant par le relais du gardien (document Redémarrage à chaud) |
| N11 | laisser un agent changer la place d'un module | lire : oui ; changer : une demande, qu'un humain valide (la garde, 202) |
| N12 | mesurer avec une sonde qui exige `root` ou le socket Docker | uniquement ce qu'un processus non privilégié lit : `/proc`, `/sys`, son propre cgroup |
| N13 | montrer cent courbes, et aucune réponse à « est-ce que la maison tient ? » | une ligne en tête : la maison tient, ou non, et pourquoi ; le détail ensuite |
| N14 | taire une perte de messages, comme `rumqttd` 0.20 au-delà de ~1 400 messages de retard (V7 § 6.1) | toute perte signalée par le courtier déclenche une alerte, dès la première |

## 4. Trois étages, et ce qu'on mesure à chacun

<!-- FIGURE:etages -->

**Premiers principes.** Toutes les mesures se déduisent de cinq grandeurs brutes, que le noyau ou nos
propres compteurs donnent sans calcul :

1. **un temps passé sur le processeur**, en nanosecondes, par fil ou par processus ;
2. **une mémoire résidente**, en octets, par processus, anonyme d'un côté, fichier de l'autre ;
3. **des octets alloués**, par module, quand le module partage le tas du cœur ;
4. **des compteurs qui ne font que monter** : messages, octets, appels, erreurs, relances ;
5. **l'heure du relevé**.

Les pourcentages, les débits, les tendances et les alertes se calculent ensuite, par différence entre
deux relevés. **Questions en étoile** sur chaque mesure : qui la produit, qui la lit, où elle vit,
combien de temps, à quel rythme, pour qui. Les tableaux qui suivent y répondent.

### 4.1 L'équipement

La machine de Sarah Home et la secours :

| Mesure | Source | Déjà lue ? | Rythme | Pourquoi |
|---|---|---|---|---|
| Processeur, total et par cœur | `/proc/stat` (lignes `cpu` et `cpuN`) | le total, oui | 15 s | — |
| Charge | `/proc/loadavg` | oui (`load`) | 15 s | — |
| Mémoire totale, disponible, échange | `/proc/meminfo` | oui | 15 s | — |
| **Pression** : la part du temps où une tâche attendait le processeur, la mémoire ou le disque | `/proc/pressure/cpu`, `memory`, `io` (`some avg10`, `avg60`) | non | 15 s | le meilleur signal de saturation : il dit « on attend », pas « on est occupé ». Le noyau peut la désactiver par défaut (`CONFIG_PSI_DEFAULT_DISABLED`, réactivée par `psi=1` au démarrage) : à vérifier sur le Pi, par `ls /proc/pressure` |
| **Octets écrits sur la carte**, par jour | `/proc/diskstats` (secteurs écrits × 512) | non | 15 min | l'usure de la carte SD est le vrai coût du Pi (V7 § 21.3) ; l'étude HA v2 compte les écritures (§ 10) : la mesure les vérifie |
| Températures | `/sys/class/hwmon` | oui | 15 s | — |
| **Bridage du Pi** : sous-tension, fréquence plafonnée, bridage thermique, en cours et depuis l'allumage | la propriété `get_throttled` du micrologiciel : les 16 bits bas disent l'état en cours, les 16 bits hauts ce qui s'est produit depuis l'allumage | non | 15 s | un Pi mal alimenté ralentit sans rien dire ; le fichier où le noyau du Pi l'expose est à vérifier sur la carte |
| Espace libre du dossier de données | `statvfs` | oui (`/api/system`, `free_bytes`) | à la demande | — |
| Réseau | `/proc/net/dev` | oui | 15 s | — |
| Un saut de l'horloge murale | le détecteur du hub (mémoire v2 § 6.5) | avec la mémoire v2 | à chaque saut | une horloge qui saute fausse les instants rangés |

Les autres équipements, qui disent eux-mêmes leur charge :

| Équipement | Ce qu'il dit de lui | Par | État |
|---|---|---|---|
| Routeur ASUSWRT | processeur, mémoire totale, utilisée, libre | `moli-asuswrt` | lu |
| Routeur Netgear | processeur, mémoire | `moli-netgear` | lu |
| Frigate | charge du processeur | `moli-frigate` | lu |
| Ordinateurs de la maison | processeur, mémoire, GPU, disque, réseau | l'agent Moli | lu |
| **Zigbee2MQTT** | la charge et la mémoire de sa machine ; sa propre mémoire et sa durée de marche ; ses messages MQTT publiés, reçus, en file ; **par appareil** : messages, messages par seconde, départs du réseau, changements d'adresse | `zigbee2mqtt/bridge/health`, toutes les 10 min par défaut (`health.interval`) | **reçu, pas lu** : `moli-z2m` s'abonne à `{base}/#` (`lib.rs:110`) mais n'en fait rien |
| La machine de secours | la même chose que la machine principale | son propre `moli-host`, publié sur `sys/` (HA v2) | avec la haute disponibilité |

### 4.2 Le processus

Pour le cœur, chaque module isolé, et le gardien :

| Mesure | Source (Linux) | Rythme | Remarque |
|---|---|---|---|
| Temps processeur | `/proc/<pid>/stat` (`utime`, `stime`, en tics de 10 ms) ; plus fin, `/proc/<pid>/schedstat` (nanosecondes sur le processeur) | 60 s, 10 s regardé | — |
| Mémoire résidente, anonyme, fichier, pic | `/proc/<pid>/status` : `VmRSS`, `RssAnon`, `RssFile`, `VmHWM` | 60 s | `RssAnon` est la mesure du budget ; `VmRSS`, que lit `stats.rs` aujourd'hui, compte aussi le binaire |
| La juste part des pages partagées | `/proc/<pid>/smaps_rollup` : `Pss`, `Pss_Anon` | 5 min | les modules isolés partagent le même binaire : `Rss` compterait son code autant de fois qu'il y a de processus. Plus coûteux à lire (le noyau parcourt les tables de pages) |
| Mémoire anonyme du conteneur | `/sys/fs/cgroup/memory.stat` (`anon`) | 60 s | **la définition même du budget** (PERFORMANCE.md), lisible depuis le conteneur : le budget devient une mesure continue, plus seulement une mesure de déploiement |
| Fils, descripteurs | `Threads:` dans `status` ; les entrées de `/proc/<pid>/fd` | 60 s | une fuite de connexions se voit là d'abord |
| Disque | `/proc/<pid>/io` (`read_bytes`, `write_bytes`) | 60 s | qui écrit sur la carte |
| Relances, dernier code de sortie, dernier signal | celui qui l'a lancé : le gardien | à chaque fin | — |

Ce que j'ai vérifié dans un conteneur Docker ordinaire (alpine 3.20, noyau 6.8, sans privilège) :
`smaps_rollup`, le `schedstat` d'un fil, `io` et `/proc/pressure` répondent. Le système de fichiers des
cgroups est monté **en lecture seule** : `mkdir /sys/fs/cgroup/essai` répond *Read-only file system*.
Dans Docker, on ne peut donc pas créer un cgroup par module isolé. On mesure son processus par `/proc`, et
sa limite de mémoire passe par `setrlimit` (V7 § 9.4.2 prévoyait « cgroup ou `RLIMIT_AS` »). Sous
systemd, le gardien peut recevoir un sous-arbre de cgroups (`Delegate=yes`) et donner à chaque module
isolé le sien, avec sa propre limite.

### 4.3 Le module

| Mesure | Dans le cœur | Isolé |
|---|---|---|
| Processeur | **son fil** : `/proc/self/task/<tid>/schedstat`, en nanosecondes ; **son runtime** : temps occupé et réveils (§ 5) | son processus (§ 4.2) |
| Mémoire | estimée, ou étiquetée (§ 6) | **exacte** : `Pss_Anon` |
| Messages MQTT | comptés par le kit (§ 7) | comptés par le courtier, sur sa connexion (§ 7) |
| Appels d'API | comptés par `moli-net`, la couche HTTP que partagent les pilotes | idem, dans son processus |
| Ordres et issues | comptés par le kit : reçus, issues par sorte (V7 § 5.6), durée de l'ordre à l'issue | idem |
| Santé | statut, relances, dernière erreur (le superviseur) | idem, plus le code de sortie |

### 4.4 Ce que chaque module montre en plus

Le CPU, la mémoire et les messages valent pour tous. Chaque module a, en plus, ce qui dit s'il fait bien
son travail :

| Module | Où il tourne | Ses mesures propres |
|---|---|---|
| **Hub** | dans le cœur, toujours | valeurs entrées par seconde, valeurs refusées par leur spécification, occupation du tampon d'entrée et fusions (V7 § 20.5), demandes retenues par la garde, époque, rang `s` |
| **Courtier** (`moli-courtier`) | dans le cœur, toujours | connexions, publications par seconde, échecs, **pertes signalées**, mémoire des segments par filtre (V7 § 6.4) |
| **Moteur des workflows** | dans le cœur, toujours | exécutions en cours, attentes, file (n au plus, critique § 3.9), occupation de la zone de travail par rapport à son plafond, réveils |
| **Pont maison** (HA v2 § 6.4) | dans le cœur | occupation de la file (sur 10 000 sorties), retard de chaque secours en rang et en secondes, âge du dernier battement, tronçons servis, états complets envoyés |
| **API et écrans** (WebSocket, REST, MCP) | dans le cœur, toujours | écrans connectés, file par écran, instantanés servis, requêtes par seconde, p95 |
| **Base d'historique** | dans le cœur | lots par seconde, `fsync` par seconde, retard `seq`, lignes de l'outbox en attente, taille du fichier |
| **Sarah** (l'assistant) | isolé conseillé | appels au modèle, jetons en entrée et en sortie, coût estimé, durée, « trop long », réponses coupées à leur borne (critique § 3.9) |
| **Modules api-to-mqtt** (les intégrations) | dans le cœur si validés, isolés sinon | appels d'API (nombre ; erreurs : délai, refus, panne, transport ; réponses 429), durée p50 et p95, connexions ouvertes, **connexions au cloud par jour**, valeurs publiées, doublons écartés, ordres et issues, disjoncteur ouvert (V7 § 9.2) |
| **Ponts radio** : Zigbee2MQTT (voisin, client extérieur) | son propre processus | par `bridge/health` : sa mémoire, ses messages, sa file ; par appareil : messages, départs du réseau |
| **Gardien** (document Redémarrage à chaud) | son propre processus | redémarrages du cœur, durée du dernier, taille et âge du relais en mémoire |

Les connexions au cloud par jour méritent une ligne à part. Un module qui se reconnecte en boucle peut
faire bloquer le compte de la maison par le fabricant. C'est une panne qui ne se voit ni au CPU ni à la
mémoire.

## 5. Le CPU d'un module dans le cœur

**Aujourd'hui, une approximation.** Tous les pilotes partagent le runtime (`supervisor.rs:31`). Le noyau
ne voit que les fils de `tokio`, pas les pilotes. `TaskMonitor` de `tokio-metrics` (0.5.2), décidé par D8,
instrumente une tâche et donne, entre autres, la durée moyenne d'un `poll`, la part des `poll` lents, la
durée moyenne d'attente et d'inactivité. La somme des durées de `poll` d'un pilote est une bonne
approximation de son CPU, avec deux défauts :

- un appel bloquant fait dans un `poll` compte comme du calcul ;
- un fil que le système suspend au milieu d'un `poll` compte aussi.

**Avec un fil par module (V7 § 20.1), une mesure exacte.** Chaque module a son fil, nommé
`pilote-<instance>`, et son runtime `current_thread`. Deux mesures en sortent :

- **le noyau** compte le temps de ce fil sur le processeur, en nanosecondes ; l'observatoire le lit de
  l'extérieur, sans réveiller le module ;
- **`tokio` 1.54** donne pour ce runtime le temps occupé (`worker_total_busy_duration`) et le nombre de
  fois où il s'est endormi puis réveillé (`worker_park_unpark_count`). Dans les sources de `tokio`
  1.54.1, ces deux mesures sont sous `cfg_64bit_metrics!` : stables, sans `tokio_unstable`, sur une
  machine 64 bits comme le Pi Zero 2 W sous un système 64 bits. Le reste des métriques par worker
  demande `tokio_unstable` (`runtime/metrics/runtime.rs:104-249`).

Les réveils sont la mesure qui prouve la règle « une brique inactive ne coûte rien » (PERFORMANCE.md) :
un module au repos ne se réveille pas. D8 le visait avec « `0 poll` » ; le fil par module le donne
directement.

**Le CPU qu'un module fait dépenser aux autres.** Un module qui publie beaucoup fait travailler le
courtier (3,3 µs de CPU du routeur par message, V7 § 11.4.5) et le hub. Ce coût n'est pas sur son fil.
L'écran le montre à part, comme **coût induit** : messages × coût unitaire mesuré. Il n'est jamais ajouté
à son CPU propre.

Bloc technique — lire le CPU d'un module dans le cœur (proposé) :

```rust
/// Le temps passé sur le processeur par le fil d'un module, en nanosecondes.
/// `/proc/self/task/<tid>/schedstat` : « temps sur le processeur, temps d'attente, tranches ».
/// Lu par l'observatoire : le module n'est pas réveillé pour ça.
fn cpu_ns(tid: i32) -> Option<u64> {
    let ligne = std::fs::read_to_string(format!("/proc/self/task/{tid}/schedstat")).ok()?;
    ligne.split_whitespace().next()?.parse().ok()
}

// Au lancement du fil d'un module (V7 § 20.1), le kit note son numéro de fil
// (`rustix::thread::gettid()`, sans `unsafe`) et garde le `Handle` de son runtime.
// À chaque relevé, l'observatoire lit, pour ce runtime `current_thread` (un seul worker, le n° 0) :
let m = handle.metrics();
let occupe = m.worker_total_busy_duration(0);  // stable en 64 bits (tokio 1.54)
let reveils = m.worker_park_unpark_count(0);   // idem : un module au repos ne se réveille pas
```

## 6. La mémoire d'un module dans le cœur : la seule mesure difficile

**Pourquoi c'est difficile.** Dans le cœur, tous les modules partagent un tas et un allocateur : celui de
musl, que la V7 garde (§ 11.4.6). Un octet ne sait pas qui l'a demandé. Pire, il change souvent de main :
un module construit un message, et c'est le fil du courtier qui le libère, quand le segment qui le garde
tourne. Un compteur par fil se trompe donc dans les deux sens : le module semble fuir, le courtier
semble rendre de la mémoire qu'il n'a jamais prise.

**Analogie : Erlang.** Chaque processus Erlang a son propre tas. `process_info(Pid, memory)` donne « la
taille en octets du processus », pile, tas et structures internes comprises, et `reductions` le travail
qu'il a fait. La mesure est simple parce que les tas sont séparés, pas parce que l'instrument est plus
fin. La leçon vaut ici : **la mémoire d'un module n'est exacte que s'il
a sa propre mémoire**, c'est-à-dire s'il est isolé.

**Analyse morphologique** : les façons de mesurer, côte à côte.

| Option | Comment | Exactitude | Coût | Verdict |
|---|---|---|---|---|
| A. **La mémoire tenue par le kit** | le kit additionne ce qu'il tient pour le module : files, voies, caches d'appareils (longueur × taille) | partielle : ne voit pas une fuite hors du kit | presque rien | **toujours** |
| B. Un compteur par fil, sans étiquette | l'allocateur ajoute à son fil ce qu'il alloue, retire ce qu'il libère | fausse dès qu'un octet change de fil (ci-dessus) | une ou deux nanosecondes par allocation | **seulement comme débit** : les octets alloués par seconde, qui disent qu'un module « brasse » |
| C. **Une étiquette par allocation**, à la façon de Vector | un octet placé après chaque allocation porte le numéro du module qui l'a faite ; la libération le relit, quel que soit le fil qui libère | exacte, même quand un autre fil libère | un octet, arrondi à l'alignement, par allocation vivante ; quelques nanosecondes par allocation ; Vector annonce environ 20 % de débit en moins sur ses tests, et rien quand c'est éteint | **en option de construction**, après mesure sur le Pi |
| D. Une arène par module (`jemalloc`, `thread.arena`) | chaque fil alloue dans son arène ; les statistiques sont tenues par arène | exacte | un autre allocateur : la V7 garde celui de musl, `mimalloc` y coûtait +0,5 à +2,6 Mio (§ 11.4.6) | non |
| E. **Isoler pour mesurer** | relancer le module dans son processus et lire `Pss_Anon` | exacte | 577 Kio de plus pendant la mesure (V3, Linux musl), et une bascule (§ 11.4) | **la vérité de terrain, à la demande** |

**Ce que je propose.**

1. **Toujours, la mémoire tenue par le kit (A) et le débit d'allocation (B).** Pour chaque module, l'écran
   affiche « estimée » à côté du chiffre.
2. **Toujours, la part non expliquée.** C'est la mémoire anonyme du cœur, lue dans le cgroup comme le
   budget, moins ce que les comptes expliquent : les modules (A) et les structures du hub, dont la V7
   donne la taille (§ 11.4). Tant qu'elle reste plate, rien à dire. Si elle monte, un module fuit hors de
   ce que le kit voit, et l'observatoire propose l'étape 4.
3. **En option de construction, l'étiquette (C)**, telle que Vector l'a faite (v0.27.0, décembre 2022,
   `--allocation-tracing`) : un octet de numéro de module après chaque allocation.
   - L'interrupteur est fixé **à la construction** du binaire, pas pendant la marche. Le code de Vector
     lit son interrupteur à chaque allocation et à chaque libération : si on l'allume en route, un bloc
     alloué avant sera libéré comme s'il portait une étiquette qu'il n'a pas.
   - Passer au binaire de mesure coûte un redémarrage, qui ne perd rien avec le document Redémarrage à
     chaud.
   - L'écran affiche alors « étiquetée ».
   - Une limite, comme chez Vector : la mémoire est comptée **à qui l'a allouée**, même si un autre la
     garde. Un message construit par un module et gardé par le courtier reste compté au module. C'est la
     bonne réponse à « qu'est-ce que ce module fait garder ? ».
4. **À la demande, isoler pour mesurer (E).** « Mesurer exactement pendant 24 h » relance le module isolé,
   lit sa mémoire exacte, puis le remet à sa place. L'écran affiche « mesurée ».

**Une exception à une règle du dépôt.** Un allocateur global s'écrit avec `unsafe`
(`unsafe impl GlobalAlloc`), et l'espace de travail l'interdit (`unsafe_code = "forbid"`,
`Cargo.toml:110`). Deux voies :

- une crate à nous d'une soixantaine de lignes, seule exception nommée, relue et passée sous Miri ;
- la crate `tracking-allocator` (0.4.0), qui porte l'`unsafe` à notre place.

Comme l'option ne vit que dans le binaire de mesure, le binaire de production reste sans `unsafe`.

Bloc technique — l'étiquette d'un octet, dans le binaire de mesure seulement (esquisse) :

```rust
// moli-alloc : la seule crate qui contient `unsafe`, compilée avec la fonction « mesure-memoire ».
// Un octet après chaque allocation porte le numéro du module qui l'a faite (0 : le cœur lui-même).
thread_local! { static MODULE: Cell<u8> = const { Cell::new(0) }; }  // posé au lancement du fil du module
static VIVANTS: [AtomicI64; 256] = [const { AtomicI64::new(0) }; 256];

unsafe impl GlobalAlloc for Etiquete {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let (env, ici) = enveloppe(l);                        // l étendu d'un octet, puis aligné
        let p = unsafe { System.alloc(env) };
        if !p.is_null() {
            let m = MODULE.with(Cell::get);
            unsafe { p.add(ici).write(m) };
            VIVANTS[usize::from(m)].fetch_add(env.size() as i64, Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        let (env, ici) = enveloppe(l);
        let m = unsafe { p.add(ici).read() };                 // qui l'a faite, pas qui la libère
        VIVANTS[usize::from(m)].fetch_sub(env.size() as i64, Relaxed);
        unsafe { System.dealloc(p, env) };
    }
    // `realloc` garde l'étiquette d'origine ; la version par défaut passerait par alloc et dealloc.
}
```

## 7. Les messages MQTT, module par module

<!-- FIGURE:compter -->

**Les modules isolés : le courtier compte, sur leur connexion.** Chacun a sa connexion à l'écouteur local
(V7 § 9.4.2). Ce que `rumqttd` 0.20.0 fait, lu dans ses sources :

- un compteur d'entrée **par connexion** existe : chaque publication reçue passe par
  `IncomingMeter::register_publish` (`router/routing.rs:641-642`), qui tient un total en nombre et en
  octets, **et une table par sujet**, la fuite relevée par la V7 (« chaque connexion garde un compteur
  par sujet publié, 73 o », § 6.1) ;
- **la sortie n'est pas comptée** : la ligne qui ajouterait la taille est commentée
  (`routing.rs:702-703`) ;
- **rien de cela n'est exposé** : `send_meters` ne publie que le compteur du routeur (connexions,
  abonnements, publications et échecs, ces deux derniers remis à zéro à chaque lecture) et un compteur
  par filtre d'abonnement (`routing.rs:1152-1160`, `router/mod.rs:272-279`, `:307-312`) ;
- l'alerte de perte `CursorJump { filter, lost }` existe (`router/alertlog.rs:8-10`), mais la V7 a mesuré
  que son `lost` reste à 0.

Dans `moli-courtier`, notre version : garder les totaux par connexion, dans les deux sens, en nombre et en
octets ; supprimer la table par sujet ; exposer ces compteurs au hub ; corriger `lost`. Ce sont des
endroits que la V7 touche déjà pour les ACL et le signal de perte (§ 6.1).

**Les modules du cœur : le kit compte.** Ils partagent un seul lien avec le courtier (« les modules du
binaire partagent un seul lien abonné à `cmd/#` ; le kit répartit par module », V7 § 6.4). Le courtier ne
peut donc pas les distinguer. Le kit le peut : chaque publication et chaque ordre réparti passent par lui.
Coût : une addition atomique `Relaxed` par message, de l'ordre de la nanoseconde, à 239 messages par
seconde à la pointe des deux maisons (V7).

**Les clients extérieurs** (Zigbee2MQTT, appareils MQTT) : leur connexion, comme un module isolé, et ce
qu'ils disent d'eux-mêmes (`bridge/health` pour Zigbee2MQTT, § 4.1).

**Par famille de sujets.** `in/`, `val/`, `cmd/`, `req/`, `res/`, `evt/`, `wf/`, `sys/`, `repl/` :
`rumqttd` tient déjà un compteur par filtre d'abonnement (`SubscriptionMeter`, nombre et taille). Notre
version l'expose tel quel.

**La rafale, plus que le débit.** La V7 l'a mesuré : « ce n'est pas le débit qui fait décrocher le moteur
actuel, c'est la profondeur d'une rafale instantanée » (§ 17.1.2). Le kit garde donc, pour chaque module,
**le plus grand nombre de messages publiés en 100 ms** depuis le dernier relevé. Un pont qui republie tout
à sa reconnexion se voit tout de suite.

Bloc technique — ce que le kit compte pour chaque module (proposé) :

```rust
/// Des compteurs qui ne font que monter, écrits par le module, lus par l'observatoire.
/// 64 bits : jamais de retour à zéro ; les débits se calculent par différence entre deux relevés.
#[derive(Debug, Default)]
pub struct Compteurs {
    pub publies: AtomicU64,        // in/, avail/, res/, $vu
    pub octets_publies: AtomicU64,
    pub recus: AtomicU64,          // ordres reçus sur cmd/
    pub ecartes: AtomicU64,        // doublons écartés par le kit (V7 § 5.2)
    pub appels: AtomicU64,         // appels d'API, comptés par moli-net
    pub erreurs: [AtomicU64; 4],   // délai, refus (4xx et 429), panne (5xx), transport
    pub connexions: AtomicU64,     // connexions ouvertes vers l'appareil ou le cloud
    pub issues: [AtomicU64; 9],    // les neuf issues d'un ordre (V7 § 5.6)
    pub rafale_max: AtomicU32,     // le plus de messages publiés en 100 ms depuis le dernier relevé
}
```

## 8. Ce que la mesure coûte

Le budget de Sarah Home est de 0,5 % de CPU au repos et de 32 Mio de mémoire anonyme (PERFORMANCE.md).
La mesure doit tenir dans une petite part de ce budget.

| Poste | Coût | Nature |
|---|---|---|
| Relevé à 60 s : une quarantaine de petits fichiers de `/proc` (40 fils, 5 processus) | de 10 à 30 µs par fichier dans le conteneur de production, 8 à 10 fois plus sur un Pi (V7 § 21.3) : ≈ 9 ms par minute sur le Pi, **0,015 % d'un cœur** | estimé ; à mesurer (exp. M1) |
| Le même relevé à 10 s, page Système ouverte | ≈ 0,09 % sur le Pi | estimé ; seulement quand quelqu'un regarde |
| `smaps_rollup`, toutes les 5 min, pour 5 processus | quelques millisecondes | estimé |
| Compteurs du kit | une addition atomique par message | invisible au rythme de la maison |
| Étiquette mémoire (option C) | quelques nanosecondes par allocation ; la mémoire, à mesurer | exp. M3 |
| Courbes en mémoire | ≈ 342 Kio pour 40 modules (tableau ci-dessous) | calculé |
| Écritures sur la carte | les agrégats de 15 min : 40 modules × 6 courbes × 96 = **23 040 lignes par jour**, dans les lots de la base (D13 : 100 ms ou 500 opérations au plus), famille `diagnostic` (7 jours, jamais archivée) | calculé ; le volume en octets est à mesurer |

Les courbes, rangées en entiers de 16 bits : le CPU en millièmes d'un cœur, la mémoire en pages de
4 Kio (jusqu'à 256 Mio), les messages en dixièmes par seconde (jusqu'à 6 553 par seconde).

| Étage | Pas | Durée | Courbes par module | Octets par courbe | Pour 40 modules |
|---|---|---|---|---|---|
| Direct | 10 s | 30 min | 3 : CPU, mémoire, messages | 180 × 2 o = 360 o | 42 Kio |
| Jour | 5 min | 24 h | 6, moyenne et pointe | 288 × 2 × 2 o = 1 152 o | 270 Kio |
| Semaine | 15 min | 7 jours | 6 | en base | 0 |
| La machine | 10 s et 5 min | 30 min et 24 h | 20 | 1 512 o | 30 Kio |
| **Total en mémoire** | | | | | **≈ 342 Kio** |

**La mesure ne réveille rien.** L'observatoire a une seule minuterie, toutes les 60 s, ou 10 s quand un
écran regarde la page Système. Il lit les fichiers du noyau et des compteurs atomiques, sans jamais
réveiller un module. Un module au repos reste à zéro réveil.

## 9. Ce qu'on voit, et qui le voit

<!-- FIGURE:systeme -->

**La page Système** (`#/systeme`, `web/src/maison/pages/Systeme.svelte`) garde sa carte vivante et son
duel avec Home Assistant. Elle gagne trois choses :

- **une ligne en tête** : « la maison tient » ou non, et pourquoi (le budget, la pression, une alerte en
  cours) ;
- **la liste des modules** : place (cœur ou isolé), état, CPU, mémoire avec sa nature (mesurée,
  étiquetée, estimée), messages, pire rafale, appels et erreurs, relances sur 24 h, et une barre par
  rapport au budget ;
- **la fiche d'un module** : ses courbes, ses dernières erreurs, ses connexions, et trois gestes :
  « Relancer », « Relancer isolé » ou « Relancer dans le cœur », « Mesurer exactement (24 h) ».

**Les surfaces**, en REST et en MCP, comme tout le reste (règle 8) :

- `GET /api/system/modules`, la liste ;
- `GET /api/system/modules/{id}/courbes?etage=jour`, les courbes ;
- `POST /api/system/modules/{id}/relancer` avec `{"mode": "coeur" | "isole" | "inchange"}` ;
- en MCP, `systeme_modules` en lecture et `relancer_module`.

**Qui peut quoi.**

- **Lire** : tout le foyer, et les agents.
- **Relancer un module dans le même mode, l'isoler, le mesurer** : comme une commande du foyer. Un agent
  passe par la garde : sa demande devient une approbation qu'un humain valide (202).
- **Rentrer un module dans le cœur** : une session PIN, comme valider un automatisme, parce que c'est
  donner sa confiance à un code.
- Tous les textes passent par `t('systeme.…')` et `tr!("systeme.…")`, en français et en anglais
  (traduction.md).

## 10. Les alertes

Une alerte va à **l'astreinte** : le propriétaire, par une notice et par son bot Telegram, le canal qui
existe déjà. Jamais à un invité ni à un enfant. Elle est groupée par incident : un message qui dit ce qui
se passe et quoi faire, pas un message par relevé. Aux heures calmes, elle attend le matin, sauf si un
module important est touché : l'alarme, une serrure, un détecteur de fumée.

| Règle | Seuil de départ (à régler) | Ce qui suit |
|---|---|---|
| Une perte de messages signalée par le courtier | dès la première | alerte tout de suite |
| Un module relancé 3 fois en 10 min | — | alerte, avec la dernière erreur |
| La mémoire d'un module au-dessus de son budget | 15 min au-dessus | alerte ; à **deux fois** le budget, il est isolé à sa prochaine relance (§ 11.2) |
| Une mémoire qui monte sans redescendre | un nouveau maximum chaque heure, 6 h de suite | alerte « fuite probable » ; l'observatoire propose « Mesurer exactement » |
| La part non expliquée du cœur qui monte | idem | idem, sans nom de module : la fuite est hors du kit |
| Le CPU d'un module au-dessus de son budget | 15 min | alerte |
| La pression processeur ou mémoire de la machine | `some avg60` au-dessus de 20 % pendant 5 min | alerte |
| La mémoire disponible de la machine | sous 15 % | alerte |
| Le bridage du Pi | en cours ; ou une sous-tension depuis l'allumage | alerte |
| Les écritures sur la carte | au-dessus du budget du jour (à fixer avec l'étude HA v2 : l'instantané horaire y fait 5,7 Mo par jour) | alerte |
| Une rafale au-dessus du budget du module | — | alerte, avec le module et l'heure |
| Le budget de la maison | mémoire anonyme au-dessus de 32 Mio ; CPU au repos au-dessus de 0,5 % sur 30 min, aucun écran ouvert | alerte : c'est la règle de PERFORMANCE.md, mesurée en continu |

Le budget de chaque module se déclare dans son paquet, à côté de son manifeste. L'observatoire le
mesure : au bout de 7 jours, il propose un budget mesuré, qu'un humain accepte.

Bloc technique — le budget d'un module, dans son paquet (proposé) :

```toml
# integrations/<id>/integration.toml
[budget]
memoire = "6MiB"      # au-delà 15 min : alerte ; deux fois au-delà : isolé à la prochaine relance
cpu = "1%"            # d'un cœur, en moyenne sur 5 min
messages = "20/s"     # publiés, en moyenne sur 5 min
rafale = 500          # messages publiés en 100 ms, au plus
```

## 11. Dans le cœur ou isolé : où tourne un module

### 11.1 Ce que chaque place arrête, et ce qu'elle coûte

La V7 l'a établi (§ 9.4.1), en bref :

| Défaillance d'un module | Dans le cœur, sur son fil | Isolé, dans son processus |
|---|---|---|
| Panique | arrêtée : `catch_unwind` du superviseur (`supervisor.rs:38`) | arrêtée |
| Appel bloquant, boucle sans attente, calcul long | arrêtés pour les autres (V2 C1, V4) | arrêtés |
| `abort`, débordement de pile, bibliothèque C qui écrit hors de sa mémoire | **non** : tout le processus tombe | **arrêtés** |
| Fuite de mémoire | **non** : elle mange le budget de tous | **arrêtée**, avec une limite de mémoire |
| Coût | **19 Kio** par fil (Linux musl) | **577 Kio** par processus minimal (V3), trente fois plus ; avec MQTT au centre, c'est le seul coût, plus un passage par TCP local au lieu d'un lien en mémoire (V7 § 9.4.2) |

Isoler un module ne change rien à la confiance : notre code est de confiance dans les deux places (V7
§ 6.2). On paie de la mémoire et un passage par TCP local. Sur un Pi de 512 Mo, on isole les modules qui
le méritent, pas tous.

### 11.2 La règle : dans le cœur quand il est validé

| Situation | Place par défaut | Pourquoi |
|---|---|---|
| Un module nouveau, ou écrit par un agent (la forge, D6), pas encore validé | **isolé** | la règle 9 le charge après validation ; isolé, il ne peut abattre que lui |
| Une nouvelle version d'un module déjà validé | **isolée**, le temps de faire ses preuves | une version validée n'est pas la suivante |
| Code tiers, FFI, SDK de fabricant, décodeur | **isolé, toujours** | un `abort` emporterait le cœur (V7 § 9.4.2) |
| Gros consommateur de mémoire : analyse d'images, modèle local | **isolé, avec un plafond** | V7 § 9.4.2 |
| Sarah (l'assistant) | **isolé conseillé** | elle attend un modèle pendant des secondes ; isolée, sa réponse survit à un redémarrage du cœur (critique § 3.11) |
| Un module validé : relu, vérifié par le catalogue, 7 jours isolé sans incident et dans son budget | **dans le cœur** | 19 Kio au lieu de 577, aucun passage par TCP |
| Un module du cœur qui a planté le processus, dépassé deux fois son budget, ou fui | **retourne à l'isolé** | le sens sûr ; voir plus bas |
| Le hub, le courtier, le moteur, l'API et les écrans ; les modules qui montent une passerelle HTTP partagée avec l'API (Cast, Android TV, téléphones, ordinateurs, Netatmo) | **dans le cœur, toujours** | ils sont le cœur, ou ils en partagent la porte (V7 § 9.4.2, crible G4) |

Un module isolé garde ses besoins de service (le coffre, l'attente d'un humain, les images, les saisies
privées). Il y accède par la socket locale authentifiée de la V7, jamais par le courtier (§ 9.4.2).

**Rentrer dans le cœur est un geste humain, en sortir peut être automatique.** C'est la même logique que
le plancher de la critique des workflows : le sens risqué demande une personne, le sens sûr peut se faire
seul. L'observatoire propose (« ce module tient depuis 7 jours dans son budget : le rentrer dans le
cœur ? »), un humain valide avec son PIN. Le retour à l'isolé, lui, peut être automatique dans trois cas :

- **le cœur a planté et le dernier souffle désigne le module.** Une panique nomme son fil. Le message de
  débordement de pile aussi (« thread '…' has overflowed its stack »), quand la plateforme le produit :
  sur musl et hors x86, un débordement peut finir en simple faute de segmentation, à vérifier sur la
  carte. Le gardien garde les dernières lignes de la sortie d'erreur du cœur pour les lire. Sans dernier
  souffle, le doute profite à la maison : les modules entrés dans le cœur depuis moins de 7 jours
  retournent à l'isolé, et l'astreinte décide ;
- **sa mémoire dépasse deux fois son budget** ;
- **il a été relancé 3 fois en 10 min.**

### 11.3 Ce que « validé » veut dire

Une liste de preuves, gardées avec la version exacte du module (l'empreinte du binaire) :

1. **relu** : un humain l'a validé, avec son PIN (règle 9) ;
2. **vérifié** : `moli-os catalogue check` passe, fixtures rejouées ;
3. **observé** : 7 jours isolé, sans panique ni relance anormale ;
4. **dans son budget** : mémoire, CPU, messages, rafale ;
5. **stable** : aucune mémoire qui monte sans redescendre sur la période ;
6. **sans `unsafe` ni FFI**, ou alors il reste isolé quoi qu'il arrive.

Une nouvelle version repart de l'étape 3 : elle tourne isolée pendant que l'ancienne a cédé sa place. Le
document Redémarrage à chaud montre qu'une version neuve d'un module peut même tourner isolée à côté d'un
cœur resté à l'ancienne version, tant que le contrat `moli/v1/` ne change pas.

### 11.4 Passer de l'un à l'autre, à chaud

<!-- FIGURE:bascule -->

**Pas besoin de redémarrer le cœur.** Le même module est compilé dans le binaire et tourne aussi dans un
petit exécutable (V7 § 9.4.2). Changer de place, c'est arrêter une instance et démarrer l'autre :

1. **Retenir.** Le hub marque le module « en bascule ».
   - Ses ordres attendent dans leurs voies, avec leur échéance inchangée.
   - Ses appareils gardent leur valeur et leur qualité.
   - La santé suspend leur échéance « muet » le temps de la bascule, 10 s au plus. Aucun automatisme ne
     voit un appareil « hors ligne ».
2. **Finir.** L'ancienne instance cesse de prendre des ordres.
   - Elle finit l'ordre en vol en 1 s au plus ; au-delà, son issue est « incertaine » (V7 § 20.7).
   - Elle vide son tampon de sortie : valeurs, occurrences, issues.
3. **Passer.** Elle rend son **état de module**, 4 Kio au plus (des curseurs, des abonnements, ce qu'elle
   veut retrouver), et s'arrête : son fil est rejoint, ou son processus sort. **Jamais un secret** : les
   secrets restent au coffre, et chaque instance les relit.
4. **Lancer.** La nouvelle instance démarre dans l'autre mode, avec cet état.
5. **Reprendre.** Elle se reconnecte à ses appareils, republie sa disponibilité et ses valeurs (`$vu`). Le
   hub libère les ordres en attente dont l'échéance court encore. Ceux qui sont échus sont refusés, jamais
   rejoués : c'est la règle de toujours.

**Jamais deux instances à la fois.** Deux instances seraient deux écrivains pour les mêmes appareils, et
certains modules prennent un identifiant MQTT fixe : deux instances « se chassent du courtier » (HA v2
§ 2.4, pour Bambu, Frigate et Zigbee2MQTT).

**La durée** est celle de la reconnexion du module à ses appareils : d'une fraction de seconde pour un
appareil local à quelques secondes pour un cloud. L'écran l'annonce avant le geste : « environ 2 s sans
ordre pour 4 appareils ».

Bloc technique — la place d'un module et sa bascule (proposé) :

```rust
/// Où tourne un module. La valeur par défaut vient de `moli.toml` (V7 § 9.4.2) ;
/// la page Système la change, et la configuration vivante (`data/bricks.json`, D3) la garde.
pub enum Place {
    Coeur,                       // son fil et son runtime, dans le processus du cœur (V7 § 20.1)
    Isole { memoire_max: u64 },  // son processus, lancé par le gardien ; limite par cgroup ou setrlimit
}

/// Ce qu'une instance passe à la suivante, dans une place ou dans l'autre. 4 Kio au plus.
/// Jamais un secret : ils restent au coffre ; chaque instance les relit.
pub struct EtatDeModule {
    pub format: u16,             // la version du format propre à ce module
    pub octets: Vec<u8>,         // curseurs, abonnements, sessions ouvertes : ce qu'il veut retrouver
}

/// Les étapes, dans l'ordre. Le hub tient les voies du module pendant toute la bascule.
pub enum Bascule {
    Retenir,    // ordres en attente dans leurs voies ; appareils « en bascule », pas « hors ligne »
    Finir,      // l'ordre en vol aboutit en 1 s, sinon issue « incertaine » ; le tampon de sortie se vide
    Passer,     // l'EtatDeModule est rendu ; fil rejoint ou processus sorti
    Lancer,     // la nouvelle instance démarre dans l'autre place, avec l'état
    Reprendre,  // disponibilité et valeurs republiées ; ordres encore valables libérés, échus refusés
}
```

Bloc technique — la place par défaut, dans `moli.toml` (la forme de la V7, § 9.4.2) :

```toml
[[module]]
id = "cloud-x"
kind = "cloud-x"
isolation = "processus"     # « dans le cœur » : "thread", la valeur par défaut d'un module validé
memoire_max = "48MiB"       # isolé seulement : la limite de son processus
```

### 11.5 Qui lance un module isolé

**Le gardien, pas le cœur.** Si le cœur lançait ses modules isolés, ils seraient ses enfants. Dans un
conteneur, la mort du processus principal arrête tous les autres, et ils mourraient avec lui. Lancés par
le gardien, ils survivent au redémarrage du cœur. Ils gardent leurs connexions aux appareils et leurs
tampons, et se reconnectent au courtier dès qu'il revient. C'est ce qui rend possible ta seconde idée,
décrite dans [Redémarrage à chaud](redemarrage-a-chaud.md).

Le gardien lance un module avec son compte auprès du courtier. Il lui passe un **jeton par lancement**,
par un tube hérité, jamais dans l'environnement ni sur la ligne de commande : les deux se lisent dans
`/proc`. C'est la règle des secrets du dépôt (« par un tube, jamais au clavier ni affiché »). Le gardien
le relance s'il meurt, avec la même attente croissante que le superviseur, de 1 s à 60 s.

### 11.6 La bascule, vue par le rêveur, le réaliste et le critique

**Méthode Disney.**

- **Le rêveur** : chaque module change de place d'un geste, à tout moment, sans que personne ne le
  remarque. On essaie la version neuve d'un module isolée, à côté de l'ancienne, et l'on bascule quand
  elle a fait ses preuves.
- **Le réaliste** :
  - les modules qui partagent la passerelle HTTP de l'API ne sortent pas du cœur ;
  - un processus coûte 577 Kio ;
  - une reconnexion au cloud prend quelques secondes et compte dans les limites du fabricant ;
  - deux instances du même module ne doivent jamais tourner ensemble, donc « à côté » veut dire « l'une
    après l'autre ».
- **Le critique** :
  - une bascule qui échoue au milieu laisse un module nulle part. Il faut donc un retour automatique à
    l'ancienne place si la nouvelle instance n'est pas prête en 30 s ;
  - une bascule pendant une rafale prolonge la rafale. Elle attend donc que le tampon d'entrée soit
    vide ;
  - une bascule aux heures calmes réveille des appareils. Le geste les signale et propose d'attendre.

Ce qui en reste, dans le § 11.4 : arrêt puis départ, jamais en parallèle ; retour à l'ancienne place
après 30 s sans nouvelle instance prête ; pas de bascule pendant une rafale.

## 12. Les choix, à critères visibles

**La mémoire d'un module du cœur** (✓ bon, ~ moyen, ✗ mauvais) :

| Option | Exacte | Coût au repos | Mémoire en plus | Sans `unsafe` en production | Effort | Retenue |
|---|---|---|---|---|---|---|
| A. Tenue par le kit | ~ | ✓ | ✓ | ✓ | ✓ | **toujours** |
| B. Compteur par fil | ✗ (débit : ✓) | ✓ | ✓ | ✗ | ~ | comme débit, dans le binaire de mesure |
| C. Étiquette d'un octet | ✓ | ~ | ~ à mesurer | ✓ (option de construction) | ~ | **option de construction** |
| D. Une arène par module | ✓ | ~ | ✗ | ✓ | ✗ | non |
| E. Isoler pour mesurer | ✓ | ✓ | ~ (577 Kio, le temps de la mesure) | ✓ | ✓ (avec la bascule) | **à la demande** |

**Où ranger les courbes** :

| Option | Écritures | Survit à un redémarrage | Mémoire | Retenue |
|---|---|---|---|---|
| En mémoire seulement | 0 | non | ✓ | — |
| En base, chaque relevé | ✗ (40 × 6 par minute) | oui | ✓ | non |
| Dans l'instantané horaire (HA v2) | ~ (il quintuplerait : de 233 Kio à plus d'1 Mio) | oui | ✓ | non |
| **24 h en mémoire, passées par le relais ; agrégats de 15 min en base** | ✓ (23 040 lignes par jour, en lots) | oui (à chaud : tout ; à froid : les agrégats) | ✓ (≈ 342 Kio) | **oui** |

**Le rythme** : fixe à 10 s (✗ au repos), fixe à 60 s (✓ au repos, ~ pour diagnostiquer en direct),
**adaptatif : 60 s, 10 s quand on regarde** (✓ et ✓). Retenu : adaptatif.

**Changer de place** : redémarrer le cœur (✗ : tout s'arrête), **arrêt puis départ, à chaud** (✓),
deux instances en parallèle (✗ : deux écrivains). Retenu : arrêt puis départ.

**Qui lance un module isolé** : le cœur (✗ : il meurt avec lui), **le gardien** (✓ : un seul mécanisme,
sous Docker et sous systemd), une unité systemd par module (~ : parfait sur le Pi, impossible sous
Docker, et il faudrait piloter systemd depuis le cœur). Retenu : le gardien ; sous systemd, il reçoit un
sous-arbre de cgroups pour donner à chaque module isolé sa limite et ses comptes.

## 13. Le plan

Chaque marche est utile seule, dans l'ordre :

1. **Lire ce que le noyau donne déjà** (aucun changement d'architecture) :
   - `RssAnon` et `VmHWM` à la place de `VmRSS` ;
   - la mémoire anonyme du cgroup, pour suivre le budget en continu ;
   - `/proc/self/io` ;
   - la pression, les écritures sur la carte, le bridage du Pi ;
   - `bridge/health` de Zigbee2MQTT.
2. **Un fil par module** (V7 § 20.1). Le CPU par module devient exact : le temps noyau de chaque fil, le
   temps occupé et les réveils de son runtime. Avant cela, `TaskMonitor` (D8) donne une approximation.
3. **Les compteurs du kit et de `moli-net`** : messages, octets, pire rafale, appels, erreurs, issues.
4. **L'observatoire** :
   - ses courbes en mémoire ;
   - ses agrégats dans la famille `diagnostic` ;
   - `/api/system/modules` ;
   - la page Système, et l'outil MCP.
5. **Les compteurs par connexion dans `moli-courtier`**, avec la V7 (§ 6.1).
6. **Le gardien, et la place d'un module** : `moli-os module run`, la bascule à chaud, l'état de module, la
   rétrogradation automatique (document Redémarrage à chaud, pour le gardien).
7. **Les alertes à l'astreinte**, et les budgets dans les paquets.
8. **L'étiquette mémoire**, en option de construction, après la mesure M3 sur le Pi.

## 14. Les expériences

| # | Expérience | Réussi si |
|---|---|---|
| M1 | Le relevé complet (40 fils, 5 processus) sur un Pi Zero 2 W, à 60 s puis à 10 s | moins de 0,05 % d'un cœur à 60 s, moins de 0,2 % à 10 s |
| M2 | Un module au repos pendant une heure | zéro réveil de son runtime ; le CPU de son fil n'avance pas |
| M3 | Le binaire de mesure (étiquette) contre le binaire de production, même maison simulée, sur le Pi | mémoire en plus et débit en moins mesurés ; la somme des étiquettes égale la mémoire anonyme du cœur à 5 % près |
| M4 | Une fuite injectée dans un module du cœur (1 Kio par minute, hors du kit) | la part non expliquée monte ; l'alerte « fuite probable » part en moins de 6 h ; « Mesurer exactement » désigne le bon module |
| M5 | Une rafale de 5 000 messages d'un pont qui se reconnecte | la pire rafale du module la montre ; l'alerte de rafale part |
| M6 | Le courtier perd des messages (abonné en retard de plus de 1 400 messages) | une alerte, dès la première perte, avec le nombre perdu (et plus `lost = 0`) |
| M7 | Bascule d'un module du cœur vers l'isolé, puis retour, pendant que des ordres arrivent | aucun ordre perdu ni rejoué après son échéance ; aucun appareil « hors ligne » ; durée de la reconnexion du module, plus 0,1 s au plus |
| M8 | Un module isolé plante en boucle (3 fois en 10 min) | une alerte groupée ; il reste isolé ; la maison continue |
| M9 | Un `abort` dans un module du cœur | le cœur redémarre ; le dernier souffle désigne le module, ou les modules entrés depuis moins de 7 jours sont isolés ; une alerte |
| M10 | Un agent demande `relancer_module` vers le cœur | la demande est retenue ; un humain la valide avec son PIN |

## 15. Décidé ici, et à trancher

Décidé ici, parce que ta demande, le code ou une étude antérieure le tranche :

| Sujet | Décision | Pourquoi |
|---|---|---|
| La mémoire mesurée | la mémoire anonyme, comme le budget ; `Pss` entre processus qui partagent le binaire | PERFORMANCE.md ; § 4.2 |
| Le CPU par module | le temps noyau du fil de chaque module, dès que la V7 lui donne son fil | § 5 |
| Les messages par module | le courtier pour une connexion à soi, le kit pour les modules du cœur | V7 § 6.4 ; § 7 |
| La rafale | une mesure à part entière | V7 § 17.1.2 |
| Le rythme | 60 s, 10 s quand on regarde ; aucun module réveillé | budget au repos |
| Les courbes | 24 h en mémoire, agrégats de 15 min en base (`diagnostic`, 7 jours) | § 8, § 12 |
| La place d'un module | dans le cœur une fois validé ; isolé sinon | ta demande ; V7 § 9.4.2 |
| Changer de place | à chaud, arrêt puis départ, sans redémarrer le cœur | § 11.4 |
| Qui lance un module isolé | le gardien | § 11.5 |

À trancher :

| Question | Ma recommandation |
|---|---|
| La durée d'observation avant d'entrer dans le cœur | 7 jours sans incident et dans le budget |
| Le retour automatique à l'isolé sur incident | oui, dans les trois cas du § 11.2 ; l'entrée dans le cœur jamais automatique |
| L'étiquette mémoire | option de construction (binaire de mesure), après M3 ; toujours présente seulement si M3 la trouve invisible |
| L'exception à `unsafe_code = "forbid"` | une crate à nous, d'une soixantaine de lignes, relue et passée sous Miri, plutôt qu'une dépendance |
| Le budget d'un module | déclaré dans son paquet par son auteur ; mesuré et proposé par l'observatoire après 7 jours |
| Qui reçoit les alertes, et comment | le propriétaire, par notice et par son bot Telegram, groupées, rien aux heures calmes sauf l'alarme, les serrures et les détecteurs |
| La page Système | lecture pour tout le foyer ; « rentrer dans le cœur » avec un PIN |
| La pression sur le Pi | l'activer (`psi=1`) si le noyau la désactive par défaut, après mesure de son coût |

## Annexe A — Sources

- systemd, le dépôt de descripteurs (pour le gardien et les cgroups délégués) :
  <https://systemd.io/FILE_DESCRIPTOR_STORE/>
- `tokio-metrics` 0.5.2, `TaskMonitor` : <https://docs.rs/tokio-metrics/latest/tokio_metrics/struct.TaskMonitor.html>,
  et la crate : <https://docs.rs/tokio-metrics/latest/tokio_metrics/> (« some metrics require `tokio_unstable` » pour
  `RuntimeMonitor`)
- Vector, la mémoire par composant : <https://vector.dev/blog/tracking-allocations/> (v0.27.0, 14 décembre 2022,
  `--allocation-tracing`, « ~20% reduction in throughput », aucun suivi de la propriété entre composants) ; son
  allocateur : <https://github.com/vectordotdev/vector/blob/master/src/internal_telemetry/allocations/allocator/tracing_allocator.rs>
  (un octet de groupe après l'objet, l'interrupteur `TRACK_ALLOCATIONS` lu à l'allocation et à la libération)
- `tracking-allocator` : <https://docs.rs/tracking-allocator>
- Zigbee2MQTT, `bridge/health` : <https://www.zigbee2mqtt.io/guide/usage/health.html> (`health.interval` : 10 min par
  défaut ; `reset_on_check` : faux)
- Erlang, `process_info/2` (`memory` : « the size in bytes of the process. This includes call stack, heap, and
  internal structures ») : <https://www.erlang.org/doc/apps/erts/erlang.html>
- Home Assistant, l'intégration Profiler (aucune attribution par intégration annoncée) :
  <https://www.home-assistant.io/integrations/profiler/>
- La pression (PSI) et `psi=1` : le correctif du texte d'aide du noyau, <https://lkml.rescloud.iu.edu/1812.0/00299.html>
- Le bridage du Pi (`get_throttled` : « lower 16-bits are the live value, upper 16-bits are the history ») : le
  correctif du noyau,
  <https://android-kvm.googlesource.com/linux/+/74d1e007915fab590f8be9dc647b19511260210c%5E>
- Le message de débordement de pile de Rust, et le cas de musl et des architectures sans sondes de pile :
  <https://users.rust-lang.org/t/is-rust-guaranteed-to-detect-stack-overflows/52593>

## Annexe B — Preuves dans le code

| Fait | Où |
|---|---|
| `moli-host` relève toutes les 15 s | `crates/moli-host/src/lib.rs:21` |
| ses points : `cpu`, `load`, `ram`, `ram_used`, `ram_total`, `swap`, températures, réseau, `uptime`, `cores` | `crates/moli-host/src/lib.rs`, `fn device` |
| `rss_bytes` vient de `VmRSS` | `crates/moli-runtime/src/stats.rs:14-20` |
| `/api/system` : `stats`, `drivers`, `perf` (les 20 dernières lignes de `perf.jsonl`) | `crates/moli-api/src/system.rs:94-103`, `:166-176` |
| `/api/health` : `stats`, `guard`, `drivers`, `journal_failures` | `crates/moli-api/src/rest.rs:22-34` |
| le superviseur lance chaque pilote par `tokio::spawn`, relance de 1 s à 60 s, capture les paniques | `crates/moli-runtime/src/supervisor.rs:16-19`, `:31`, `:38`, `:64` (`spawn_driver` : `:22-68`) |
| `moli-z2m` s'abonne à tout son arbre, `bridge/health` compris, sans le lire | `crates/moli-z2m/src/lib.rs:110` |
| Frigate publie `cpu_load` | `crates/moli-frigate/src/lib.rs:246`, `:287` |
| l'espace de travail interdit `unsafe` | `Cargo.toml:110` |
| `rustix` : fonctions `fs` et `std` seulement aujourd'hui | `Cargo.toml:86` |
| `rumqttd` 0.20.0 : compteur d'entrée par connexion, avec une table par sujet | `src/router/routing.rs:641-642`, `src/router/mod.rs` (`IncomingMeter`) |
| `rumqttd` 0.20.0 : la sortie n'est pas comptée | `src/router/routing.rs:702-703` |
| `rumqttd` 0.20.0 : seuls le routeur et les filtres sont publiés | `src/router/routing.rs:1152-1160`, `src/router/mod.rs:272-279` |
| `rumqttd` 0.20.0 : l'alerte de perte | `src/router/alertlog.rs:8-10` |
| `tokio` 1.54.1 : temps occupé et réveils stables en 64 bits | `src/runtime/metrics/runtime.rs:104-249` |
| `rustix` 1.1.5 : `gettid`, `clock_gettime` avec `ThreadCPUTime`, sans `unsafe` | `src/thread/id.rs:59`, `src/time/clock.rs:60`, `src/clockid.rs:63` |
