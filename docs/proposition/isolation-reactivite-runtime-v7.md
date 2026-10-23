# Analyse V7 — Sarah Home, un hub MQTT : toutes les valeurs par un courtier Rust embarqué

Date : 22 octobre 2026. Statut : **proposition d'architecture, aucune modification du code**.
Base analysée : `main` au commit `bd1714d`, et le code de `rumqttd` 0.20.0.

La V7 est le document complet. Elle reprend la [V6](isolation-reactivite-runtime-v6.md) (toutes les
valeurs par un courtier MQTT écrit en Rust, embarqué dans le binaire) et y replace tout ce que la
[V5](isolation-reactivite-runtime-v5.md) établissait et qui reste vrai : le pattern et ses mesures
de production, les quatre étages du stockage, les bus capricieux, la livraison des ordres, les
transports mesurés, la haute disponibilité des radios, la sécurité, les contrats, le coût et le
plan. On lit la V7 seule ; la V5 et la V6 restent les traces de leurs bancs.

Ce que la V7 ajoute :

- **le nom et la marque Sarah Home**, à la place de « Moli OS » ;
- **l'api-to-mqtt dans les deux sens**, lecture et action, avec des exemples de bout en bout : un
  volet RTS et un volet io par la box TaHoma, l'aspirateur Roborock, et le classement de toutes les
  intégrations selon ce que l'appareil sait dire en retour (§ 5.6 à 5.9) ; un crible fait pilote par
  pilote, preuves dans le code, a éprouvé le contrat : 26 écarts, tous réglés (§ 5.1, § 5.10, annexe D) ;
- **une issue d'ordre qui ne ment pas** : neuf issues au lieu de trois, dont « exécuté par le
  pont » pour un appareil sans retour (§ 5.6) ;
- **la mémoire en schémas** : ce que la maison compte (appareils, valeurs, automatismes, nœuds),
  ce que porte une valeur octet par octet (dernier rapport, dernier changement, qualité, santé),
  la mémoire par type de donnée, et tous les postes, avant et après l'optimisation (§ 11).

Méthode :

- lecture du code de Sarah Home et de `rumqttd` 0.20.0, le courtier Rust retenu pour commencer ;
- bancs hors du dépôt, en Linux musl, dans un conteneur limité à 2 CPU et 128 Mio comme la
  production : ceux de la V5 (production, tailles, connexions, état vif), ceux de la V6 (courtier
  embarqué, mémoire de toute la conception) ([annexe A](#annexe-a--bancs)) ;
- tailles des messages proposés calculées octet par octet (§ 7) ;
- un crible des 37 genres de pilotes par huit sous-agents, en lecture seule, chaque affirmation avec
  sa preuve `fichier:ligne`, une quarantaine revérifiées ([annexe D](#annexe-d--crible-des-intégrations)) ;
- les pilotes TaHoma (`moli-overkiz`) et Roborock (`moli-roborock`) lus dans leurs branches
  d'intégration, avec leurs décisions d'ordres.

Vocabulaire :

- **Sarah Home** est le produit. `moli` reste le nom du code : les crates `moli-*` et le préfixe
  des sujets `moli/v1/` sont des identifiants, que la V7 ne renomme pas ;
- le **moteur MQTT** est le courtier embarqué dans Sarah Home ;
- un **module api-to-mqtt** traduit une API (un appareil, un service, Sarah Home elle-même) en
  valeurs MQTT au canon, et des ordres MQTT en appels à cette API : c'est le **sens lecture** et le
  **sens action** ;
- un **lien local** est une connexion au courtier dans le même processus, par un canal en
  mémoire, sans TCP ni chiffrement ;
- un **sujet** est l'adresse d'un message MQTT (`moli/v1/val/salon_lampe/on`) ; la **charge** est
  son contenu ;
- un message **retenu** est gardé par le courtier et remis à tout nouvel abonné ;
- l'**issue** d'un ordre est ce que Sarah Home sait de son sort : confirmé, exécuté par le pont,
  accepté, envoyé sans confirmation… (§ 5.6) ;
- l'**époque** est le numéro qui désigne la seule machine autorisée à agir.

Les renvois des versions précédentes (« V5 § 12 », « § 17 » de la V6) sont remplacés par les
sections de la V7.

## 1. Réponse courte

**Oui, Sarah Home peut faire passer toutes ses valeurs par un courtier MQTT en Rust embarqué, dans
les deux sens, pour toutes ses intégrations. Le prix est faible, à une condition : ne pas l'écrire
naïvement.** Écrite avec les types d'aujourd'hui et un courtier qui garde des retenus, la V7 prend
46,7 Mio à ×3 et fuit à chaque ordre. Écrite avec les types compacts et le courtier réglé, elle
prend **11,8 Mio à ×3** et ne fuit plus.

| Question | Réponse | Preuve |
|---|---|---|
| Où vit le courtier ? | **Dans le binaire** : `rumqttd`, repris et complété. Les modules de Sarah Home passent par des liens en mémoire, sans ACL ; les clients extérieurs (Zigbee2MQTT, appareils MQTT) passent par ACL et quotas | décision du 22 octobre, § 6 |
| Combien coûte un passage par le courtier ? | **26 µs** de latence médiane et **3,3 µs** de CPU par message, contre ~10 µs pour le bus actuel ; 50 000 messages/s sans perte ; une ACL coûte 30 à 114 ns par message extérieur | banc courtier, § 11.4.5 |
| Les deux sens ? | **Lecture** : le module lit l'appareil, met au canon, déduplique, publie sur `in/`. **Action** : le hub publie l'ordre sur `cmd/` après la garde ; le module le passe par la voie de l'appareil, l'envoie une fois, et rend une **issue** sur `res/` | § 5, § 8 |
| Un volet TaHoma ? | **RTS** n'a aucun retour : l'issue est « exécuté par la box », jamais « ouvert ». **io** renvoie sa position : l'issue est « confirmé » quand la position demandée revient | § 5.7 |
| L'aspirateur ? | Une tâche longue : « accepté » quand le robot répond, « confirmé » quand son état passe à « nettoie » ; le retour à la base est une **attente** du workflow, pas l'issue de l'ordre ; les cartes restent hors MQTT | § 5.8 |
| Toutes les intégrations ? | Éprouvées une par une, preuves dans le code. La lecture était couverte ; l'action et les services avaient **26 écarts**, dont sept structurels : appui contre consigne, confirmation sur un autre point, suivi après l'échéance, services hors courtier, secrets, appareils qui appellent Sarah Home, briques propres au hub. Chacun a sa règle dans le contrat et le kit | § 5.1, § 5.3, § 5.9, § 5.10 |
| Toute la maison en mémoire ? | **Oui.** 496 appareils, 2 264 valeurs, 400 automatismes (≈ 2 400 nœuds) : toutes les données de la V7 tiennent en 6,5 Mio à ×1, **11,8 Mio à ×3**, 27 Mio à ×10, une fois optimisées. Un point coûte 96 o au lieu de 719 ; une condition se lit en moins d'une nanoseconde | banc mémoire, § 11 |
| Ce que porte une valeur ? | Elle-même, la précédente, l'instant observé, le **dernier changement**, le **dernier rapport** et la qualité, en 40 o ; l'appareil porte sa santé (dernier contact, rythme attendu, pertes tolérées) en 20 o | § 11.2 |
| Qui tient la vérité ? | **Le hub** : il valide, journalise, publie `val/` et sert l'état ; le courtier ne garde **aucun message retenu** | § 6.3, § 10 |
| Qui donne les ordres ? | Les surfaces et le moteur **demandent** (`req/`), la garde décide, **seul le hub publie l'ordre** (`cmd/`), l'issue revient (`res/`) | § 5.5, § 8 |
| Un module qui plante ? | Il ne plante que lui : son thread pour une panique ou un blocage, son processus pour un `abort`, une pile ou une fuite, avec le même contrat MQTT | § 9 |
| Le workflow ? | Le modèle de la maquette, exécuté sur MQTT : abonné aux valeurs, il demande des ordres et lit leur issue ; baux durables ; une seule copie compilée | § 13 |
| L'historique ? | La base de données est abonnée au courtier et comble ses trous par le rang `s` ; les écrans reçoivent les courbes par blocs sur leur WebSocket, jamais par MQTT | § 12, § 14 |
| Appareil mort ? | Dernier contact, dernier changement, rythme et pertes tolérées : un appareil interrogé toutes les 3 min avec une perte tolérée est déclaré muet à 6 min | § 10.5 |
| La secours ? | Abonnée au courtier actif, elle garde toute la maison en mémoire ; le journal porte le reste (baux, exécutions, époque) ; un témoin décide | § 18 |
| Ce que `rumqttd` fait mal | Retenus tronqués à 200 **sans signal**, pertes **silencieuses** au-delà de ~1 400 messages de retard, caches par sujet jamais vidés, une panique de connexion : contournés par la conception, à corriger dans notre version | bancs, § 6.1, § 19.2 |
| Ce qui reste de l'existant | Le pattern (noyau vif, cloisons, journal) et ses mesures de production, les quatre étages du stockage, les radios à un seul écrivain, la garde : la V7 les garde tels quels | § 9, § 12, § 17, § 18 |

## 2. Les décisions, et ce qu'elles impliquent

Quatre questions ont été posées le 22 octobre. Les réponses, avec les remarques qui les
accompagnaient :

| Question | Réponse | Remarque jointe | Ce que la conception en tire | § |
|---|---|---|---|---|
| Où vit le courtier MQTT en Rust ? | **dans le binaire Sarah Home** | « moteur haute dispo de Sarah Home à prévoir ; séparation pour que le plantage d'un module ne plante pas les autres ; api-to-mqtt » | un courtier embarqué ; chaque module isolé (thread, ou processus pour les modules à risque) ; un moteur de haute disponibilité propre à Sarah Home | 6, 9.4, 18.2 |
| Qui tient la vérité de l'état courant ? | **Sarah Home, le courtier diffuse** | — | le hub reste la vérité (mémoire et journal) ; il est le seul à écrire les valeurs publiées (`val/`) ; le courtier ne garde aucun message retenu : qui veut l'état le demande au hub | 7, 10 |
| Comment passe un ordre ? | **demande, garde, puis Sarah Home** | « api-to-mqtt de Sarah Home : le module filtre, ajuste les énumérations, transforme » | toute surface (écran, REST, MCP, assistant, moteur d'automatismes) publie une **demande** ; la garde décide ; seul le hub publie l'**ordre** au module de l'appareil | 5, 8, 10 |
| Et ce qui manque à `rumqttd` ? (précision du même jour) | **le reprendre et le compléter** : il est en Apache-2.0 | « les modules qu'on gère nous-mêmes sont très fiables, pas besoin d'ACL ; un Zigbee ou un MQTT extérieur nécessite des vérifications ; attention aux performances » | `rumqttd` repris dans `moli-courtier` ; deux niveaux de confiance : aucun contrôle par message pour les modules de Sarah Home, ACL et quotas pour les seuls clients extérieurs | 6.1, 6.2, 19.2 |
| Où passe la frontière de MQTT ? | **valeurs courtes seulement** | « le module api-to-mqtt gère la transformation (énumérations), la déduplication… » | une charge MQTT de Sarah Home tient en quelques dizaines d'octets ; les appels LLM, textes longs, devoirs, médias et fichiers passent ailleurs | 7, 16 |

Le mot « api-to-mqtt » revient dans trois réponses sur quatre. C'est le cœur de la V7 : **un
seul contrat de module**, dans les deux sens.

- **Vers l'intérieur** : le module d'une intégration lit l'API de l'appareil, met les valeurs
  au canon de Sarah Home et les publie. Le canon, c'est l'énumération en code, l'échelle, l'unité, la
  qualité et l'horodatage ; au passage, le module filtre et déduplique.
- **Depuis les surfaces** : l'« api-to-mqtt de Sarah Home » transforme ce qu'un écran, un script ou un
  agent demande en une demande MQTT au canon. Le libellé d'une énumération devient son code, la
  valeur est contrôlée par sa spécification, et un doublon est écarté.

Une phrase de la demande dit aussi que « même Matter devrait passer par MQTT ». Elle peut se lire
« même le moteur » (le moteur d'automatismes). La V7 traite les deux : Matter devient un module
api-to-mqtt (§ 15), et le moteur d'automatismes devient un client du courtier (§ 13).

### 2.1 Ce qui ne change pas

Le changement de centre ne touche à aucune des règles de l'architecture ni aux acquis des V3 à V5 :

| Acquis | Où | Dans la V7 |
|---|---|---|
| Aucune décision n'attend un disque ni un appareil : le noyau vif en mémoire | V3, § 9, § 11 | le hub garde l'état vif ; le courtier n'en est qu'une projection |
| La garde, les approbations, le journal de chaque écriture | ARCHITECTURE, règles 2, 7, 9 | le hub est le seul à écrire un ordre (`cmd/`) ; ACL du courtier ou confinement par le kit des modules |
| Un ordre a une échéance absolue et n'est jamais rejoué | ARCHITECTURE, § 8.4 | l'échéance voyage dans l'ordre MQTT ; sessions propres, aucun ordre retenu |
| Un module (ex-pilote) isolé : thread, voies, disjoncteur | V3, V4, § 9, § 17.2, § 17.4 | inchangé, et le processus séparé devient facile (§ 9.4) |
| Le tampon d'entrée « dernière valeur gagne » | V3 P2 | à l'entrée du hub, sur `in/` |
| Les quatre étages du stockage | stockage V3, § 12.1 | l'archiveur s'abonne au courtier et comble ses trous par le journal |
| La reprise par journal, instantané et époque | V4 S1-S3, § 18.1 | elle porte aussi le courtier, qui ne garde rien : le hub sert l'état à l'activation |

## 3. L'architecture en une page

<!-- FIGURE:archi -->

Le schéma se lit de gauche à droite : les sources, le binaire de Sarah Home et son courtier, puis les
écrans. Le schéma de couverture montre les mêmes modules autour du bus. Sept composants, chacun
avec ce qu'il lit et ce qu'il écrit :

| Composant | Rôle | Lit (sujets) | Écrit (sujets) | Cloison | Remplace |
|---|---|---|---|---|---|
| **Moteur MQTT** | route, retient, borne | tout | rien en propre | ses threads ; bornes par client | le bus d'événements interne (`broadcast` de 1 024) comme canal de diffusion |
| **Modules api-to-mqtt** | une intégration chacun : lire, mettre au canon, dédupliquer ; exécuter les ordres | `cmd/<module>/#` | `in/<module>/#`, `avail/`, `res/` (issue d'un ordre) | un thread et un runtime ; un processus pour un module à risque | les pilotes (`trait Driver`) et leur `DriverCtx` |
| **Hub** | la vérité : valeurs, qualité, trois instants ; garde, approbations, journal, époque | `in/#`, `req/#` | `val/`, `cmd/`, `res/`, `avail/` | le cœur, sur son runtime | inchangé, mais il publie au lieu de diffuser en interne |
| **Moteur d'automatismes** | évalue les règles, lance les actions | `val/<point>` des seuls points qu'il regarde, `evt/#` | `req/automation/…` | son thread | l'abonnement au bus d'événements |
| **Archiveur** | écrit les étages du stockage | `val/#`, `evt/#` ; le journal pour combler un trou | rien sur MQTT | son thread ; ses propres lots | `moli-history` et l'enregistrement actuel |
| **Module WebSocket** | une connexion par écran : direct filtré par les droits, historique par blocs, ordres et leur issue | `val/`, `avail/`, `res/` (de ses écrans) | `req/ui/…` | son runtime ; une file bornée par écran | SSE `/api/events` et l'essentiel de REST pour le tableau de bord |
| **API de Sarah Home → MQTT** | REST, MCP et assistant traduits en demandes au canon | `res/` | `req/<origine>/…` | celle de `moli-api` | `Hub::command` appelé directement |

Trois choses ne passent jamais par MQTT : les appels LLM, les contenus longs ou lourds (textes,
devoirs, images, audio, fichiers, exports), et la réplication vers la machine de secours, qui
suit le journal (§ 16, § 18.2).

### 3.1 Pourquoi ce centre, et ce qu'il coûte

Les mesures de production (§ 17.1) montrent que MQTT au centre **n'apporte rien à la réactivité** :
le hub diffuse déjà en mémoire, en nanosecondes. Le choix de la V7 se justifie par autre chose, et
il a un prix :

| Ce qu'on gagne | Pourquoi |
|---|---|
| **Un seul contrat de module** | une intégration publie des valeurs et reçoit des ordres sur des sujets ; peu importe où elle tourne et dans quel langage |
| **L'isolation par processus devient facile** | le même module tourne dans un thread (lien local) ou dans un processus à part (TCP local, ou socket Unix), sans changer une ligne de son contrat (§ 9.4) |
| **L'archivage et les écrans se branchent sur le bus** | un abonné de plus, sans toucher au hub |
| **Les appareils MQTT natifs entrent sans pilote écrit à la main** | un profil déclaratif suffit pour Tasmota, Shelly ou ESPHome en MQTT (§ 15) |
| **Les outils du monde MQTT** | observer, enregistrer, rejouer un flux avec n'importe quel client MQTT |

| Ce qu'on paie | Ordre de grandeur | § |
|---|---|---|
| Une sérialisation et un routage par message, à chaque saut | des microsecondes par saut, au lieu de nanosecondes (mesuré) | 11.4 |
| Plus de sauts : une valeur passe par `in/` puis `val/` ; un ordre par `req/`, `cmd/`, puis `res/` | quelques sauts de plus par valeur ou par ordre, négligeables devant un aller-retour Zigbee | 8, 11.4 |
| La mémoire du courtier : connexions, retenus, files | mesurée | 11.4 |
| Un composant repris au centre du binaire (`rumqttd`) | il devient le nôtre : ACL ajoutées pour les clients extérieurs, correctif à suivre à chaque version ; une panique de son routeur reste à couvrir | 6, 9.4 |
| La garde ne doit pas devenir contournable | qui peut écrire `cmd/` commande la maison : ACL ou confinement obligatoires | 19.2 |

### 3.2 Ce que la discussion externe apporte, et ce qu'elle ne voit pas

La discussion externe pose de bonnes questions. Ses réponses sont justes en général, mais sur
plusieurs points elles ne s'appliquent pas à Sarah Home telles quelles.

**Ce qu'elle apporte, et que la V7 retient :**

- séparer le temps réel de l'historique ;
- « reçu » n'est pas « exécuté » : QoS 1, puis une confirmation par l'état réel ;
- un identifiant pour les ordres qui ne se répètent pas sans dommage ;
- une échéance courte pour un ordre physique ;
- se méfier des messages retenus (*retained*) ;
- un seul coordinateur Zigbee à la fois, plusieurs routeurs Thread à la fois ;
- clore l'ancien avant d'activer le nouveau ;
- tester chaque reprise avant de l'automatiser.

**Ce qu'elle ne voit pas, et que la V7 corrige :**

| Affirmation externe | Ce qui vaut pour Sarah Home | § |
|---|---|---|
| HTTP classique veut dire interrogation périodique (*polling*) | Sarah Home pousse déjà en **SSE** : une connexion, un instantané, puis les changements. La discussion ne cite SSE qu'en passant ; c'est pourtant la quatrième option, et celle que Sarah Home utilise aujourd'hui ; la V7 passe à un module WebSocket, pour d'autres raisons | 14.2, 14.1 |
| 14 à 64 Kio par connexion WebSocket | chiffres d'une bibliothèque Python ; mesuré ici en Rust (T4) | 14.2.4 |
| Un relevé coûte quelques dizaines d'octets | vrai sur le courtier ; mais Sarah Home découpe un message d'appareil en un événement par valeur, et chacun porte son identité (T1) | 14.2.2 |
| Un cluster MQTT Raft à trois nœuds en Rust (RMQTT) | son journal Raft ne vit qu'en mémoire (vérifié) ; trois nœuds plus Node.js pour les ponts dépassent le budget d'un Pi Zero 2 W (D10) ; pour une maison, un courtier actif/passif attaché à l'époque suffit | 18.1.5, 18.2 |
| Historique dans PostgreSQL ou TimescaleDB | Sarah Home a décidé Turso pour le récent et une archive Parquet pour le long terme (D13, stockage V3). Rien dans la discussion ne justifie d'en changer | 11, 12.4 |
| La sécurité MQTT, c'est TLS et des ACL | juste, mais incomplet pour Sarah Home : un client du courtier qui publie sur `zigbee2mqtt/…/set` commande un appareil **sans passer par la garde ni le journal** de Sarah Home. Le courtier est une porte de service autour de la règle 9 | 19.1, 19.2 |
| Le gros message à craindre, c'est l'historique | chez Sarah Home, les gros messages sont trois autres : l'instantané SSE, `bridge/devices` et une réponse d'historique brute. Ils sont mesurés (T2, T3) | 14.2.3, 12.4 |

## 4. Les modules et les API

### 4.1 Cinq couches

<!-- FIGURE:modules -->

Les 57 crates se rangent en cinq couches. Les dépendances ont été lues par `cargo metadata`, pas
déduites des noms :

| Couche | Crates | Dépend de | Règle |
|---|---|---|---|
| Binaire | `moli-os` | tout | assemble, configure, démarre ; aucune logique métier |
| Surfaces | `moli-api` (REST, SSE, MCP, sessions, bornes) | `moli-runtime`, les briques, six crates de pilotes pour leurs parcours d'appairage ou d'autorisation | tout ordre passe par `Hub::command` |
| Briques | `moli-automation`, `moli-history` (+ `moli-rollup`), `moli-energy`, `moli-helpers`, `moli-lights`, `moli-presence`, `moli-ambiance`, `moli-alarm-access`, `moli-reservations`, `moli-assistant` | `moli-runtime`, `moli-core` | s'abonnent au bus ; ne parlent jamais à un pilote |
| Pilotes | 32 crates de pilotes d'appareils et de services, plus `moli-profile` (profils déclaratifs) ; dix briques publient aussi leurs points par un pilote interne | `moli-runtime`, `moli-core`, `moli-net` et leur bibliothèque de protocole | n'écrivent que par le hub ; n'atteignent que l'hôte déclaré |
| Hub | `moli-runtime` | `moli-core`, `moli-i18n` | état, bus, ordres à échéance, garde, journal, coffre, superviseur |
| Fondations | `moli-core`, `moli-net`, `moli-i18n`, `moli-rollup`, `moli-alarm`, `moli-audio`, `moli-media-*` | rien, ou `moli-core` | `moli-core`, `moli-rollup` et `moli-alarm` sont purs : aucune entrée-sortie |
| Service à part | `moli-archive` | `moli-core`, `moli-i18n`, `moli-rollup` | binaire séparé, HTTP ; lu par D1 quand D1 existera |

La règle qui compte pour l'isolation est celle de la ligne « Pilotes » : un pilote ne voit que le
hub. C'est elle qui rend possible un thread par pilote (§ 9) sans rien changer aux briques.

### 4.2 Rust par API

Chaque surface externe, le code qui la sert, ce qu'elle appelle dans le hub, et ses bornes :

| API | Transport | Code Rust | Ce qu'elle appelle | Bornes |
|---|---|---|---|---|
| Flux temps réel | SSE `GET /api/events` | `moli-api::sse::events` | `Hub::subscribe` (diffusion de 1 024 événements), `Hub::snapshot` | 8 flux par appelant, 64 en tout (`sse.rs:30-32`) ; un abonné en retard reçoit un instantané (`sse.rs:128`) ; non compressé |
| Ordres | REST `POST /api/command` | `moli-api::rest::command` | `Hub::command_with_request_id` → garde → `CommandRequest` vers le pilote | valeur contrôlée par son `PointSpec` ; échéance 5 s ; `request_id` pour les seules opérations |
| Appareils | REST `GET /api/devices`, `/{id}`, `/{id}/snapshot` | `moli-api::rest` | `Hub::snapshot` | vue invitée filtrée par pièces |
| Historique | REST `GET /api/history` | `moli-api::rest::history` | `moli_history::…::series` | 1 min à 1 an ; 10 à 2 000 points, 300 par défaut (`rest.rs:676-793`) ; compressé |
| Énergie | REST `/api/energy`, `/api/energy/series` | `moli-api::rest` | `moli-energy` | — |
| Automatismes | REST `/api/automations…` et un flux SSE `live` | `moli-api::automations` | `moli-automation` | flux comptés avec ceux de `/api/events` |
| Assistant, voix | REST `/api/assistant…`, `/api/voice` | `moli-api::assistant` | `moli-assistant` (LLM sous la garde) | origine `assistant`, retenue comme un agent |
| Agents | MCP `POST /mcp` (HTTP « streamable », `rmcp`) | `moli-api::mcp`, 22 outils | les mêmes fonctions que REST, origine `mcp` | retenue en pièce protégée, en heures calmes |
| Zigbee | client MQTT | `moli-z2m::run` | `DriverCtx::set_state`, `set_availability`, `upsert_device` ; `next_command` | 4 Mio entrant, 256 Kio sortant, file de 64, lectures à 2 par seconde (`lib.rs:79-99`) |
| Autres pilotes | HTTP(S), WebSocket, TCP, UPnP | `moli-<pilote>::run`, `moli-net` | `DriverCtx` | délais et tailles bornés par pilote (règle 10) |
| Archive | HTTP (service à part) | `moli-archive` : `POST /query`, `PUT /catalog` | aucun appel du cœur aujourd'hui | jeton, 128 Mio, lecture seule du disque |

Le tableau montre une propriété utile pour la suite : **une seule surface pousse en continu**
(`/api/events`), et **une seule reçoit des ordres** (`/api/command`, et `send_command` en MCP, qui
rejoint le même `Hub::command`). C'est ce qui rend la question « SSE ou WebSocket » simple à
trancher (§ 14.2.5).

### 4.3 Les contrats Rust au centre

Bloc technique — ce que voient un pilote et une surface (`moli-runtime`, extraits des signatures publiques) :

```rust
// Un pilote : une boucle supervisée, relancée avec attente croissante si elle sort en erreur.
pub trait Driver: Send + Sync + 'static {
    fn kind(&self) -> &'static str;
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>>;
}

// Un ordre, tel que le pilote le reçoit : il porte son échéance absolue.
pub struct CommandRequest {
    pub point: PointId,
    pub device: Arc<Device>,
    pub key: Arc<str>,
    pub value: Value,
    pub deadline: Instant,          // 5 s après l'acceptation par la garde
    pub metadata: CommandMetadata,  // origine, request_id, corrélation
}
impl CommandRequest {
    pub fn abandoned(&self) -> bool;                    // l'appelant est parti : ne pas envoyer
    pub fn reply(self, result: Result<(), String>);     // la réponse que l'observateur journalise
}

// Ce que le pilote publie, et d'où il tire ses ordres.
impl DriverCtx {
    pub fn ready(&self);
    pub fn upsert_device(&self, device: Device);
    pub fn set_state(&self, device: &DeviceId, key: &str, value: Value);
    pub fn set_availability(&self, device: &DeviceId, online: bool);
    pub fn pulse_state(&self, device: &DeviceId, key: &str, value: Value); // un appui : jamais fusionné
    pub async fn next_command(&mut self) -> Option<CommandRequest>;
}

// Ce que voient les surfaces.
impl Hub {
    pub fn snapshot(&self) -> Snapshot;                       // tout l'état, pour l'instantané SSE
    pub fn subscribe(&self) -> broadcast::Receiver<Event>;    // le bus d'événements
    pub async fn command(&self, point: &PointId, value: Value, origin: Origin,
                         actor: Option<String>) -> Result<(), CommandError>;
}
```

Deux détails du hub comptent pour la livraison des ordres (§ 8.4) :

- **chaque ordre accepté a un observateur** (`hub.rs:357-430`) qui écrit une seule ligne de
  journal : la réponse du pilote, « abandonné » si l'appelant est parti avant l'envoi, ou « issue
  inconnue » si tout s'arrête avant la réponse ;
- **`request_id` est déjà une clé d'idempotence**, réservée aux points de type opération (un
  appui, un compteur) : une demande d'approbation en double rend la première, et un compteur ne
  compte pas deux fois le même identifiant (`hub.rs:1037-1056`, `hub.rs:1219-1233`). Une
  consigne (allumer, régler) n'en a pas besoin : la poser deux fois revient au même.

## 5. Les modules api-to-mqtt

<!-- FIGURE:module -->

### 5.1 Le contrat

Un module api-to-mqtt fait pour son intégration ce que Zigbee2MQTT fait pour Zigbee, avec les
règles de Sarah Home. Ce contrat a été **éprouvé sur toutes les intégrations du dépôt** par un
crible fait pilote par pilote, preuves `fichier:ligne` à l'appui (§ 5.9, annexe D). Le crible a
trouvé 26 écarts entre une première version du contrat et le code, dont sept structurels ; chacun
a ici sa règle, marquée de son numéro (G1 à G26, liste au § 5.10).

**La méta d'un point : ce que le module déclare**

| Champ | Ce qu'il dit | Pourquoi | Écart |
|---|---|---|---|
| `kind`, `unit`, `echelle`, `min`, `max` | type, unité, échelle du canon, bornes de **validité** ; les bornes d'**ordre** sont à part | refuser au bord une valeur absurde, sans refuser une lecture légitime | G24 |
| `codes` | le dictionnaire d'une énumération : `{code, cle, libelle}`. Le code natif de l'appareil quand il en a un (Roborock : 23 états, 28 erreurs) ; un code de repli « autre » ; un code n'est **jamais réattribué**, la clé n'est jamais traduite, aucune prose traduite ne sert de valeur | une règle et un historique ne changent pas de sens quand un libellé change | G11 |
| `ecrit`, `effet` | un point écrivable est une **consigne** (idempotente : la dernière en attente gagne) ou un **appui** (chaque appui compte, dans l'ordre) | sans cela, la voie fusionnerait trois « volume + » ou passerait par-dessus un « stop » | G2 |
| `retour` | **par code** pour une énumération : `etat` (le point qui confirme, les valeurs attendues, la tolérance, et « à l'arrêt » pour une valeur en mouvement), `pont`, `accuse` ou `aucun` | `start` d'un robot se confirme sur `state` ; `stop` d'un volet io n'a pas de cible | G3 |
| `suivi_s` | la durée du suivi après l'échéance, 180 s au plus | Ariston 180 s, TaHoma 120 s, Landroid 60 s, réveil d'une télé 45 s | G1 |
| `reveillable` | un ordre part vers l'appareil même hors ligne (Wake-on-LAN) | une télé qui dort n'est pas une télé en panne | G14 |
| `sensible` | `secret` (refusé par le kit), `position`, `texte_prive` | rien de secret sur le courtier ni au journal ; une diffusion bornée pour le reste | G7 |
| `proprietaire` | `hub` pour les aides, minuteries, compteurs, calculs et scènes : aucun `cmd/`, le hub applique l'ordre lui-même | éviter un écho de cinq sauts pour écrire sa propre vérité | G18 |
| `volatile` | une valeur qui varie sans cesse (`remaining` d'une minuterie, `uptime`) : ni archive, ni déclencheur ; une durée croissante est quantifiée | la déduplication n'y économise rien | G18, G26 |
| `source` | `niveau` pour une occurrence dérivée d'un niveau interrogé (la sonnette Reolink, relevée chaque seconde), avec sa résolution | deux sonneries dans la même fenêtre se confondent : il faut le dire | G19 |
| `acces` | pour une image ou un document : `piece_protegee`, `tableau_de_bord` | la garde des médias n'est pas celle des ordres | G9 |
| `semantic`, `piece`, `membres`, `modele` | ce que la garde lit : `Control`, `Lock`, la pièce native, des membres plafonnés | la garde reste dans le hub, et elle a besoin de ces faits | G9 |
| `sante` | rythme, pertes tolérées, et `veille` pour un silence normal (télé en veille, ampoule coupée au mur, capteur sur pile) ; aucun rythme appris pour un appareil dont le silence **est** le signal (la présence) | ne pas déclarer muet ce qui dort | G5, G6 |
| `objet` | un « objet court » : JSON de 3 Kio au plus, schéma déclaré, sans prose, lu et écrit d'un bloc (le planning d'une tondeuse, `play_media`) ; à défaut, éclater en points | des valeurs composées existent | G12 |

La méta tient dans un budget : les libellés traduits vivent dans les catalogues de traduction, pas
dans `meta/` (une méta compacte pèse ≈ 2,1 Kio au lieu de 5,2). Un appareil se redéclare quand sa
spécification change (G9, G10).

**Le sens lecture**

| Étape | Règle | Écart |
|---|---|---|
| Lire | sondage ou poussée, délais et tailles bornés (règle 10). Ce qui **appelle** Sarah Home (l'appli des téléphones, l'agent d'un PC) passe par une passerelle HTTP **dans le processus du hub** : l'API vérifie jeton, droits et quotas, le module ne reçoit que des rapports bornés | G8 |
| Décoder | entrée hostile ; une réponse illisible n'est jamais une panique (V1 E8) | — |
| Mettre au canon | échelle de la source (21,57 °C → `2157`), unité, sens, **énumération en code** ; une valeur hors de sa validité est refusée au bord ; un texte est borné **en octets** (3,5 Kio enveloppe comprise, 1 Kio pour un corps de notification), tronqué et marqué par le kit | G10 |
| Valeur inconnue | `null` veut dire inconnu, avec la qualité mauvaise par défaut (incertaine si le module le dit) ; une valeur inférée porte la qualité incertaine ; jamais une valeur fausse pour dire « je ne sais pas » : une télé injoignable n'est pas « éteinte » | G5 |
| Filtrer | anti-rebond seulement là où le rebond est prouvé (stockage V3) | — |
| Dédupliquer | même valeur canonique : rien n'est publié. Mais **le module juge une confirmation sur sa lecture brute, avant la déduplication** : une valeur demandée égale à la valeur courante ne repasse jamais sur `in/` | G1 |
| Fraîcheur | un `$vu` ne compte que si la source dit du neuf : un horodatage source plus récent (Netatmo, iopool, Tempo et Open-Meteo rendent la dernière mesure, pas une nouvelle). Il porte la liste des points qu'il rafraîchit quand l'appareil a plusieurs sources ou rythmes, et sa fenêtre n'excède jamais le rythme | G6 |
| Pause imposée | un cloud qui impose une pause (Ariston 900 s, Voltalis 600 s, Landroid 1 h) : le module publie `pause_jusqu_a` sur `sys/`, qui suspend l'échéance de silence de ses appareils | G6 |
| Publier | `in/<instance>/<appareil>/<point>`, QoS 0. `<instance>` est l'instance du module (un pont Hue, un profil), pas son genre | G25 |
| Occurrence | sur `evt/`, QoS 1, jamais fusionnée, avec son instant observé et une identité (`source:rang`) qui écarte un doublon QoS 1 ; le module dérive le front montant d'un niveau | G19 |
| Garder privé | un code tapé sur un clavier ou une serrure, un mot de passe, un code temporaire ne deviennent **jamais** une valeur : ni `in/`, ni `evt/`, ni méta, ni journal, ni historique. Ils vont au hub par le canal de services ; seul le fait est publié (« code saisi au clavier », avec l'identité du clavier) | G7, G23 |
| Disponibilité, santé | publiées pour chaque appareil et pour le module, avec une raison en code ; un bus ouvert met ses appareils hors ligne ; une sonde illisible rend l'appareil **indisponible**, jamais « absent » | G5 |
| Attendre un humain | appairage, code d'une télé, accord OAuth, jeton refusé : `sys/` « en attente d'un humain », avec un code de raison ; l'écran montre la procédure. Ce n'est jamais une valeur, ni un statut de santé déguisé | G4 |

**Le sens action**

| Étape | Règle | Écart |
|---|---|---|
| Recevoir un ordre | `cmd/<instance>/<appareil>/<point>`, écrit par le hub seul, après la garde ; l'échéance absolue est dans la charge. Un ordre ne porte **jamais** de secret ni de texte privé : un flux humain avec secret (`sign_in` d'Alexa, accord OAuth) reste une route REST du module. Un ordre « à permis » (invité, scène, exécution d'automatisme) a pour échéance min(5 s, fin de sa fenêtre), et `cmd/<instance>/$annule` le retire avant l'envoi | G7, G21 |
| La voie | sa clé est choisie par le module : l'appareil, la box, le compte, la station. Un ordre en vol. **Les consignes consécutives d'un même point se fusionnent, jamais par-dessus un appui** ; un appui n'est jamais remplacé. Une consigne écartée rend `non_envoye` (cause `remplace`), une voie pleine aussi (cause `voie_pleine`). Appelant et échéance revérifiés juste avant l'envoi | G2, G15 |
| Traduire | code → valeur de l'appareil ; jamais de bascule (`TOGGLE`) à la place d'un état explicite, même en repli | G13 |
| Envoyer | **une seule fois par ordre.** Un ordre composé (lecture-modification-écriture, plusieurs appels) garde la voie jusqu'à sa dernière étape et relit son prérequis après toute écriture antérieure ; un échec après la première écriture rend `incertain`. Une consigne émulée (le volume, touche à touche) reste en vol jusqu'à l'écho de l'état. Un appareil `reveillable` hors ligne reçoit l'ordre : `envoye`, puis la suite quand il reparaît. Le refus prédictif vise un bus ou un module ouvert, jamais un appareil qui dort | G13, G14 |
| Rendre l'issue | une seule, avant l'échéance, parmi les neuf du § 5.6, **bornée par le retour du code**. Pour `refuse`, `echec` et `non_envoye` : une raison `r` (code d'un vocabulaire fermé par domaine) et un détail `d` (200 o au plus, jamais un secret ; aucun détail pour un code faux). Un ordre réparti sur plusieurs cibles rend `echec`, cause `partiel`, avec `n/m` | G16, G17 |
| Suivre | dans la fenêtre `suivi_s`, le module conclut **une fois**, succès comme échec : le hub publie la suite sur `evt/<appareil>/<point>` avec l'identifiant de la demande, et écrit une seconde ligne de journal. À l'expiration sans conclusion, le hub conclut lui-même (`envoye`, ou il garde `incertain`) et prévient la maison | G1 |
| Un module qui meurt | un ordre pris et non rendu vaut `incertain` (cause `module`), jamais `non_envoye` ; un ordre détruit sans réponse aussi | G20 |
| Un appareil qui tire ses ordres | l'agent d'un PC : une boîte propre au module, avec une durée de vie par type d'ordre (5 à 35 s pour arrêter, redémarrer, mettre en veille ; 15 min pour une session à distance, devenue un appui à décision humaine). L'issue est `accepte` à la remise, puis la suite à l'accusé. Rien n'est rejoué depuis `cmd/` | G8 |
| Relire après écrire | un module qui sonde relit le point juste après une écriture, au lieu d'attendre son prochain relevé : c'est ce qui rend « confirmé » possible avant l'échéance | § 5.9 |

**Ce qui n'est pas une valeur : le canal de services**

Le kit parle au hub par deux chemins : **le courtier**, pour les valeurs, les ordres et les issues ;
**le canal de services**, pour tout le reste. Dans le binaire, ce canal est un appel direct ; pour un
module en processus à part, une socket locale authentifiée, **jamais le courtier**.

| Service | Règle | Aujourd'hui (`DriverCtx`) | Écart |
|---|---|---|---|
| Le coffre | lire, ranger avec acquittement durable (un jeton à usage unique est rangé **avant** son usage), oublier ; 24 genres de pilotes le lisent, 15 y écrivent | `secret`, `store_secret`, `forget_secret`, `scoped_secrets` | G4 |
| L'attente d'un humain | `sys/`, un code ; 27 genres de pilotes s'en servent | `wait_for`, `ready` | G4 |
| Images, documents, médias, voix | fournis à la demande, sous la politique d'accès de la méta | `provide_snapshots`, `provide_printing`, `media_publisher`, `voice_brain` | G9 |
| Saisies privées | en mémoire, vers le seul consommateur (l'alarme) | `submit_keypad_input` | G7, G23 |
| Fenêtres d'accès des invités | un registre éphémère (4 096 fenêtres, une heure au plus), en mémoire, fermé quand le module s'arrête ; jamais sur `in/`, où il serait journalisé et repris au démarrage, le contraire de l'exigence | `replace_reservations` | G22 |
| Ce que le hub connaît | appareils, clés, valeur courante, adresse retrouvée | `known_*`, `current`, `remember_address`, `moved` | G4 |
| Notices | un code sur `evt/`, le texte par le service des messages | `media_publisher().tell` | G1 |

Deux règles en découlent :

- un module qui monte une **passerelle HTTP partagée avec l'API** (Cast, Android TV, téléphones,
  ordinateurs, Netatmo) **reste dans le processus du hub** ;
- un module en processus à part garde le même contrat MQTT et parle au canal de services par sa
  socket : c'est à cette condition seulement que « le contrat ne change pas » (§ 9.4.2).

**Les données sensibles**

| Donnée | Règle | Écart |
|---|---|---|
| Codes, PIN, jetons, URL d'autorisation | jamais sur un sujet `moli/v1/…`, jamais dans une ligne de journal ; un point `sensible: secret` est refusé par le kit | G7 |
| Position des personnes (latitude, longitude, Wi-Fi vu) | `sensible: position` : sur `val/` pour la garde et les écrans qui en ont le droit ; ni archive longue, ni outil MCP sans droit | G7 |
| Texte d'une notification ou d'une annonce | `sensible: texte_prive` : l'ordre porte une référence, le texte passe par le canal de services ; le journal n'en garde qu'un code | G7 |
| Ordres de l'alarme qui portent un code (désarmer, changer de mode, exclure) | jamais des points : ils entrent par le canal privé de l'alarme, avec l'origine `clavier` ; `arm` a un `suivi_s` (la sortie dure 60 s) | G23 |

**Une saisie privée et le courtier.** Le message brut d'un clavier Zigbee porte le code
(`action_code`). Aujourd'hui, il traverse un Mosquitto à part, puis le pilote le détourne vers le
canal privé (`moli-z2m/src/keypad.rs`). Avec Zigbee2MQTT client du courtier de Sarah Home, il
entrerait dans notre courtier, et dans le journal de son filtre, avant d'être détourné : ce serait
une **régression d'une protection existante**. La V7 retient donc :

- un **crochet d'entrée dans le routeur** du courtier, pour les sujets des claviers que le catalogue
  de Zigbee2MQTT déclare : le test de `moli_core::privacy` s'applique **avant** le journal ; le code
  part directement au canal privé, et seul le message dépouillé entre dans le courtier ;
- seul le module `z2m` lit l'arbre `zigbee2mqtt/` ; un message de clavier n'est jamais retenu ;
- tant que ce crochet n'existe pas, **Zigbee2MQTT garde son propre courtier**, comme aujourd'hui
  (question 9 du § 23.2.2).

**Les clients extérieurs qui dépendent des retenus.** Zigbee2MQTT s'appuie sur des messages retenus
(son catalogue `bridge/devices`, les disponibilités) que la V7 supprime partout ailleurs et que
`rumqttd` tronque. Leur arbre garde une exception **bornée** (quelques centaines de retenus,
plafonnés en taille), ou le module s'y abonne par sujets exacts ; Frigate cesse de publier ses images
retenues (G10).

### 5.2 Dédupliquer sans perdre la fraîcheur

Le stockage V3 distingue trois instants : **observé**, **reçu** (le dernier rapport, même
identique) et **changé**. Publier chaque rapport identique pour tenir « reçu » remplirait le
courtier de doublons ; ne rien publier ferait croire qu'un capteur muet est à jour.

Le module ne publie donc une valeur **que si elle change**. Pour la fraîcheur, il publie au plus
une fois par appareil et par fenêtre (une minute, par exemple) un message court `vu` :

```text
moli/v1/in/<module>/<appareil>/$vu    {"r":1792675624125}
```

Le hub met à jour l'instant « reçu » de tous les points de l'appareil, et l'échéance de fraîcheur
attendue (le compteur Linky toutes les 3 minutes, un contact sur pile toutes les 24 h) continue
de juger la qualité. Une prise qui publie sa puissance chaque seconde, sans changement, coûte
alors un message par minute au lieu de 60.

### 5.3 Le kit des modules

Le kit est ce que voit un module. Il cache le courtier, confine les sujets et porte les règles du
contrat : un module ne peut ni publier hors de son arbre, ni rendre deux issues, ni rendre une issue
plus forte que le retour déclaré, ni publier une saisie privée ou un point `sensible: secret`. La
voie est dans le kit, une fois pour toutes : aujourd'hui, 17 des 27 pilotes qui envoient des ordres
ne revérifient ni l'appelant ni l'échéance entre la prise de l'ordre et l'envoi (crible, § 5).

Bloc technique — le kit des modules, complet (esquisse, pas du code livré) :

```rust
/// Un module api-to-mqtt : une boucle supervisée, relancée avec attente croissante,
/// sur son thread et son runtime ; ou dans son processus, avec le même code.
pub trait Module: Send + Sync + 'static {
    /// Le genre (« hue », « profile ») ; l'instance (un pont, un profil) est `ctx.instance()`,
    /// et c'est elle qui forme les sujets in/<instance>/… (G25).
    fn genre(&self) -> &'static str;
    fn run<'a>(&'a self, ctx: &'a mut ModuleCtx) -> BoxFuture<'a, anyhow::Result<()>>;
}

/// La méta d'un point, déclarée par le module (§ 5.1, § 7.4).
pub struct PointDecl {
    pub cle: &'static str,
    pub nature: Nature,                     // mesure, binaire, énumération, texte, objet court
    pub codes: Vec<Code>,                   // {code, cle, libelle} ; jamais réattribués (G11)
    pub validite: Option<Bornes>,           // bornes de validité de la lecture
    pub ordre: Option<Ecriture>,            // None : lecture seule
    pub sensible: Option<Sensible>,         // Secret (refusé), Position, TextePrive (G7)
    pub proprietaire_hub: bool,             // aides, minuteries, compteurs, scènes (G18)
    pub volatile: bool,                     // ni archive ni déclencheur (G18, G26)
    pub source_niveau: Option<Duration>,    // occurrence dérivée d'un niveau, sa résolution (G19)
}

pub struct Ecriture {
    pub effet: Effet,                       // Consigne | Appui (G2)
    pub retour: RetourParCode,              // par code d'énumération, ou un seul pour le point (G3)
    pub bornes: Option<Bornes>,             // bornes d'ordre, distinctes de la validité (G24)
    pub suivi: Duration,                    // suivi_s, 180 s au plus (G1)
    pub reveillable: bool,                  // Wake-on-LAN : l'ordre part même hors ligne (G14)
}

impl ModuleCtx {
    // ---- Déclarer : le hub publie la méta sur meta/ ----
    /// Un appareil, ses points, sa santé (rythme, pertes, veille), la clé de sa voie
    /// (appareil, box, compte, station : G15) et ce que lit la garde (semantic, pièce, membres).
    /// Redéclarer un appareil connu met sa méta à jour.
    pub fn declarer(&self, appareil: Appareil);
    /// L'appareil a quitté son pont : hors ligne, puis retiré du catalogue.
    pub fn retirer(&self, appareil: &DeviceId);

    // ---- Le sens lecture ----
    /// Une valeur au canon, ou Valeur::Inconnue (G5). Bornée en octets ; dédupliquée.
    pub fn valeur(&self, appareil: &DeviceId, point: &str, v: Valeur, qualite: Qualite, observe: Instant);
    /// Une occurrence : jamais dédupliquée, jamais fusionnée ; son identité écarte un doublon (G19).
    pub fn evenement(&self, appareil: &DeviceId, point: &str, v: Valeur, observe: Instant, id: IdOccurrence);
    /// L'appareil a dit du neuf sans rien changer : seulement si l'horodatage de la source avance ;
    /// `points` limite le rafraîchissement aux points concernés (G6).
    pub fn vu(&self, appareil: &DeviceId, source: Instant, points: Option<&[&str]>);
    pub fn disponibilite(&self, appareil: &DeviceId, en_ligne: bool, raison: CodeRaison);
    /// Une pause imposée par la source : l'échéance de silence est suspendue jusque-là (G6).
    pub fn pause_jusqu_a(&self, fin: Instant, raison: CodeRaison);

    // ---- Le sens action ----
    /// Le prochain ordre, déjà passé par la garde, avec son échéance absolue.
    pub async fn prochain_ordre(&mut self) -> Option<Ordre>;

    /// Le canal de services : appel direct dans le binaire, socket locale authentifiée
    /// pour un module en processus à part ; jamais le courtier (G4).
    pub fn services(&self) -> &Services;
}

impl Services {
    pub fn secret(&self, nom: &str) -> Option<Secret>;
    /// Acquitté quand le coffre est écrit sur disque : un jeton à usage unique est rangé avant usage.
    pub async fn ranger_secret(&self, nom: &str, s: &Secret) -> io::Result<()>;
    pub async fn oublier_secret(&self, nom: &str) -> io::Result<bool>;
    /// Une étape humaine bloque le module : sys/, un code ; levée par `humain_fait`.
    pub fn attendre_humain(&self, raison: CodeRaison);
    pub fn humain_fait(&self);
    /// Un code tapé sur un clavier : remis au seul consommateur (l'alarme), jamais publié.
    /// `SaisiePrivee` n'est ni sérialisable ni clonable, son Debug est masqué ; file de 16.
    pub fn saisie_privee(&self, clavier: &DeviceId, saisie: SaisiePrivee) -> bool;
    /// Images, documents, médias et voix, fournis à la demande (G9).
    pub fn fournir_images(&self, appareil: &DeviceId, source: Arc<dyn SourceImage>);
    pub fn fournir_document(&self, appareil: &DeviceId, sortie: Arc<dyn SortieDocument>);
    pub fn media(&self, appareil: &DeviceId, extension: &str, octets: Bytes) -> Option<RefMedia>;
    /// Les fenêtres d'accès des invités : en mémoire, fermées quand le module s'arrête (G22).
    pub fn fenetres(&self, fenetres: Vec<Fenetre>, expire: Duration);
    /// Ce que le hub connaît : appareils, clés, valeur courante, adresse retrouvée.
    pub fn connus(&self) -> Vec<DeviceId>;
    pub fn actuel(&self, appareil: &DeviceId, point: &str) -> Option<Valeur>;
    pub async fn adresse_retrouvee(&self, appareil: Option<&DeviceId>, adresse: Adresse);
    /// Une notice à la maison : un code sur evt/, le texte par le service des messages.
    pub fn prevenir(&self, appareil: &DeviceId, notice: CodeNotice);
}

impl Ordre {
    pub fn echeance(&self) -> Instant;          // min(5 s, fin de la fenêtre) pour un ordre à permis (G21)
    /// L'appelant est parti, l'échéance est passée, ou `$annule` est arrivé : ne pas envoyer.
    pub fn abandonne(&self) -> bool;
    /// Juste avant l'envoi : revérifie tout, puis marque l'ordre « parti ».
    /// Sans permis, l'issue ne peut être que NonEnvoye ; après, un échec devient Incertain.
    pub fn permis_envoi(&self) -> Option<PermisEnvoi>;
    /// Une seule issue, avant l'échéance, bornée par le retour du code (§ 5.6) ; une raison
    /// pour Refuse, Echec et NonEnvoye (G16). Rend un suivi pour Accepte, Envoye et Incertain.
    pub fn repondre(self, issue: Issue, raison: Option<Raison>) -> Option<Suivi>;
}
// Un Ordre détruit sans réponse rend Incertain, cause « module » (G20).

impl Suivi {
    /// La suite, dans la fenêtre suivi_s : publiée une fois sur evt/, succès comme échec,
    /// avec l'identifiant de la demande ; le hub écrit une seconde ligne de journal (G1).
    pub fn conclure(self, issue: Issue, raison: Option<Raison>);
}
// À l'expiration sans conclusion, le hub conclut lui-même (Envoye, ou il garde Incertain) et
// prévient la maison, une fois. Les ordres arrivent par un seul lien abonné à moli/v1/cmd/#,
// partagé par tous les modules du binaire, que le kit répartit par instance (§ 11.4.3).
```

Pour la migration, `DriverCtx` devient une façade du kit. Chaque capacité d'aujourd'hui
(`crates/moli-runtime/src/driver.rs`) a sa place, sur le courtier ou en dehors :

| `DriverCtx` aujourd'hui | Le kit de la V7 | Par le courtier ? |
|---|---|---|
| `set_state`, `pulse_state` | `valeur`, `evenement` | oui : `in/`, puis `val/` et `evt/` par le hub |
| `set_availability` | `disponibilite`, avec une raison ; `pause_jusqu_a` | oui : `avail/`, `sys/`, par le hub |
| `upsert_device`, `remove_device` | `declarer`, `retirer` | oui : `meta/`, par le hub |
| `known_devices`, `known_point_keys`, `current`, `known_address`, `remember_address`, `moved` | `services().connus`, `actuel`, `adresse_retrouvee` | **non** : le canal de services |
| `next_command`, `reply`, `send_permit`, `begin_send`, `emitted`, `abandoned`, `member_permit` | `prochain_ordre`, `repondre`, `permis_envoi`, `abandonne`, ordre à permis | oui : `cmd/`, puis `res/` ; `$annule` |
| `moli_runtime::orders` : `OrderLine`, `Confirmations` | la voie du kit, `Suivi::conclure` | oui : `evt/` pour la suite |
| `submit_keypad_input`, `reject_keypad_input` | `services().saisie_privee` | **jamais** : en mémoire, vers l'alarme |
| `wait_for`, `ready` | `attendre_humain`, `humain_fait` ; jamais pour dire « injoignable » (`disponibilite`) | oui : `sys/`, un code |
| `secret`, `store_secret`, `forget_secret`, `scoped_secrets` | `secret`, `ranger_secret`, `oublier_secret` | **jamais** : le coffre |
| `provide_snapshots`, `provide_printing`, `media_publisher`, `voice_brain` | `fournir_images`, `fournir_document`, `media`, le module voix | **jamais** : à la demande, par HTTP ou le module WebSocket |
| `media_publisher().tell` (notices) | `prevenir` | un code sur `evt/` ; le texte hors MQTT |
| `replace_reservations` | `services().fenetres` | **jamais** : en mémoire |
| `initialize_timer`, `…_counter`, `…_temporal`, `…_computed`, `…_duration`, `…_schedule`, `press_button` | le hub lui-même : `proprietaire = hub` | oui : `val/` et `res/`, sans `cmd/` |

Les 37 genres de pilotes passent sur le courtier par cette façade, sans être réécrits, puis gagnent
un par un la voie du kit, le retour déclaré et l'issue honnête (§ 22.2). Le crible a relevé, en
plus, des écarts de code à corriger au portage, que le contrat interdit déjà (§ 22.3).

### 5.4 Les profils déclaratifs, côté MQTT

D2 prévoyait d'étendre les profils déclaratifs « aux écritures, puis à MQTT et à WebSocket ». Avec
le courtier au centre, un appareil qui parle déjà MQTT n'a plus besoin de pilote écrit à la main.

Bloc technique — un profil déclaratif pour une prise Tasmota (esquisse de format) :

```toml
[profile]
id = "tasmota-prise"
transport = "mqtt"
base = "tasmota/{nom}"                     # sujets de l'appareil, bornés par son ACL

[[point]]
key = "on"
kind = "binary"
from = { topic = "stat/{base}/POWER", value = { ON = true, OFF = false } }
write = { topic = "cmnd/{base}/POWER", value = { true = "ON", false = "OFF" } }

[[point]]
key = "power"
kind = "numeric"
unit = "W"
scale = 1                                  # entier au canon, 1 décimale
from = { topic = "tele/{base}/SENSOR", json = "ENERGY.Power" }

[[point]]
key = "mode"
kind = "enum"
from = { topic = "stat/{base}/RESULT", json = "Mode" }
enum = { "Auto" = 1, "Manuel" = 2, "Absent" = 3 }   # libellé de l'appareil → code de Sarah Home
```

### 5.5 L'api-to-mqtt de Sarah Home : ce que les surfaces demandent

Un écran, un script REST, un agent MCP, l'assistant ou le moteur d'automatismes ne parlent jamais
à un appareil. Ils **demandent**. L'api-to-mqtt de Sarah Home met la demande au canon, avec la même
transformation qu'un module :

- le libellé d'une énumération devient son code (« Absent » → `3`) ;
- la valeur est contrôlée par la spécification du point (type, plage, longueur) ;
- un appui porte sa clé d'idempotence (`request_id`), qui écarte un doublon ;
- l'origine (`ui`, `household`, `api`, `mcp`, `assistant`, `automation`) et l'acteur
  accompagnent la demande.

Elle publie ensuite la demande sur `req/<origine>`, et attend l'issue sur `res/<origine>`. Ces deux
sujets sont **stables** : l'identifiant de la demande voyage dans la charge, jamais dans le sujet.
Un sujet par demande ferait grossir sans fin les caches du courtier (§ 11.4). La garde reste dans le
hub (§ 10).

Bloc technique — une demande et son issue, telles que les publie l'api-to-mqtt de Sarah Home :

```text
moli/v1/req/ui
{"id":"01J9Z8K3M4N5P6Q7R8S9T0V1W2","p":"salon_lampe/on","v":true,"a":"ui:session-3f2a","e":1792675629123}

moli/v1/res/ui
{"id":"01J9Z8K3M4N5P6Q7R8S9T0V1W2","i":"confirme","ms":412}
```

`e` est l'échéance voulue par l'appelant ; le hub la ramène à 5 s au plus après sa décision, comme
aujourd'hui. Si la garde retient la demande pour approbation,
l'issue le dit (`"i":"retenue"`), et l'approbation part plus tard avec une nouvelle échéance,
comme aujourd'hui.

### 5.6 Les deux sens, et une issue qui ne ment pas

Un module api-to-mqtt travaille dans les deux sens :

- **la lecture** : le module lit l'appareil (sondage ou poussée), met la valeur au canon, la
  déduplique et la publie sur `in/` ; le hub la valide, la journalise et la publie sur `val/` ;
- **l'action** : le hub publie l'ordre sur `cmd/` après la garde ; le module le passe par la voie,
  le traduit, l'envoie **une fois**, et rend une **issue** ; le hub la publie sur `res/<origine>`
  pour celui qui a demandé.

La lecture est la même pour toutes les intégrations. Ce qui change, c'est **ce que l'appareil sait
dire après un ordre**. Une lampe Zigbee renvoie son état. Un volet RTS ne renvoie rien : la box dit
seulement qu'elle a émis la trame radio. Un aspirateur répond « ok », puis met des secondes à
changer d'état. Aujourd'hui, le hub ne connaît que trois issues (`Ok`, `Err`, `Pending` :
`moli-core`, `event.rs:254`), et `reply(Ok)` veut dire « remis à l'appareil », jamais « fait »
(`driver.rs:224`). Les pilotes en savent pourtant davantage : celui de TaHoma distingue « accepté »
et « peut-être parti », celui de Roborock « peut-être envoyé, non confirmé ». La livraison des
ordres (§ 8.4) proposait trois issues ; la V7 en donne neuf, pour que chaque intégration dise
**exactement** ce qu'elle sait, ni plus ni moins.

| Issue | Code sur `res/` | Ce qu'elle veut dire | Ce que dit l'écran | Qui la rend (crible, § 5.9) |
|---|---|---|---|---|
| **Confirmé** | `confirme` | l'état attendu est revenu de l'appareil avant l'échéance | « Allumée » | Zigbee, Hue, Tuya local, Twinkly, Frigate, aides ; TaHoma io pour ouvrir et fermer |
| **Exécuté** | `execute` | le pont (une box), ou l'appareil lui-même, dit avoir exécuté l'ordre ; aucun état ne revient pour le prouver | « Ouvrir : exécuté par la box (pas de retour) » | TaHoma RTS ; « stop » et « favori » d'un volet io ; une touche accusée (télé Philips) ; un message Telegram |
| **Accepté** | `accepte` | l'appareil a pris l'ordre ; l'effet viendra plus tard ; le module le suit | « Démarrage demandé… » | Roborock (« ok »), Nuki, Ariston, Voltalis, une annonce Sonos, un mouvement io long |
| **Envoyé, non confirmé** | `envoye` | parti ; aucune confirmation avant l'échéance, ou aucune possible | « Envoyé, pas de confirmation » | Wake-on-LAN ; touches Samsung (aucun accusé) ; `cancel` d'une imprimante (jamais relu) ; push des téléphones ; Landroid (accusé du courtier cloud, pas de la tondeuse) ; Zigbee sans réponse |
| **Peut-être parti** | `incertain` | l'ordre a pu partir : liaison perdue, réponse absente, module mort avec l'ordre, échec après la première écriture d'un ordre composé ; **jamais renvoyé** | « Peut-être parti : à vérifier » | TaHoma, Roborock, tout module qui meurt |
| **Non envoyé** | `non_envoye` | rien n'est parti : voie pleine, consigne remplacée, bus ouvert, échéance passée, appelant parti | « Non envoyé : box hors ligne » | tous les modules |
| **Échec** | `echec` | l'appareil ou le pont a répondu non, avec sa raison en code ; ou une partie des cibles seulement (`partiel`, `n/m`) | « Échec : moteur bloqué » | TaHoma `FAILED`, erreur du robot, envoi en série (Telegram) |
| **Refusé** | `refuse` | la garde a dit non : origine, pièce, heure, code ; ou l'alarme (capteurs ouverts, code requis) | « Refusé : la nuit, pas depuis l'assistant » | le hub, l'alarme |
| **Retenue** | `retenue` | la garde attend une approbation ; l'approbation repartira avec une nouvelle échéance | « En attente d'approbation » | le hub |

Quatre règles tiennent l'ensemble :

- **le module déclare, pour chaque point écrivable et pour chaque code, ce que l'appareil dit en
  retour** (`retour`, § 5.1), et si le point est une consigne ou un appui (`effet`). Le hub, les
  écrans et le moteur de workflow le savent avant même d'envoyer : un automatisme qui « attend que
  le volet soit ouvert » sur un volet RTS est refusé à la validation ;
- **une issue ne dépasse jamais ce que le retour permet** : un retour `pont` plafonne à `execute`.
  Aucun état n'est supposé : un volet RTS n'a pas de position, il a une **dernière commande** et
  son issue ;
- **le module juge la confirmation sur sa lecture brute**, avant la déduplication, avec la tolérance
  du code ; une valeur en mouvement ne se confirme qu'à l'arrêt : passer par la cible en route
  n'est pas y arriver ;
- **une issue est rendue une fois, avant l'échéance** ; ce qui arrive après (le mouvement io
  terminé, la consigne enfin relue par un cloud, une erreur du robot, un succès qu'une box n'a pas
  pu dire à temps) est publié **une fois** sur `evt/<appareil>/<point>`, avec l'identifiant de la
  demande (`{"id":"01JA…W2","i":1,"tard":true}`), dans la fenêtre `suivi_s`. Le moteur de workflow
  peut l'attendre comme n'importe quelle occurrence.

Bloc technique — l'issue, sa raison et le retour par code, tels que le kit les expose (esquisse) :

```rust
/// L'issue d'un ordre, publiée sur res/<origine> : ce que Sarah Home sait, ni plus ni moins.
#[repr(u8)]
pub enum Issue {
    Confirme = 1,    // l'état attendu est revenu, avant l'échéance
    Execute = 2,     // le pont ou l'appareil dit « fait » ; aucun état ne le prouve
    Accepte = 3,     // pris ; l'effet viendra plus tard ; suivi
    Envoye = 4,      // parti ; aucune confirmation avant l'échéance, ou aucune possible
    Incertain = 5,   // a pu partir ; jamais renvoyé
    NonEnvoye = 6,   // rien n'est parti
    Echec = 7,       // l'appareil ou le pont a dit non
    Refuse = 8,      // la garde (ou l'alarme) a dit non
    Retenue = 9,     // la garde attend une approbation
}

/// La raison d'une issue négative (G16) : un code d'un vocabulaire fermé par domaine,
/// et un détail court ; jamais un secret, jamais de détail pour un code faux.
pub struct Raison { pub r: CodeRaison, pub d: Option<Court<200>> }

/// Ce que l'appareil sait dire après un ordre : déclaré par code d'énumération (G3).
pub enum Retour {
    /// L'état revient : sur ce point, parmi ces valeurs, à cette tolérance ; à l'arrêt si en mouvement.
    Etat { point: PointIdx, attendu: Vec<Valeur>, tol: u32, a_l_arret: bool },
    Pont,       // le pont dit « émis », rien de plus : RTS
    Accuse,     // un accusé tout de suite, l'état plus tard : aspirateur, serrure
    Aucun,      // rien : l'issue s'arrête à « envoyé »
}
pub struct RetourParCode(Vec<(Code, Retour)>);   // un seul élément pour une consigne simple
```

Chaque issue est aussi tracée pour tous sur `evt/<appareil>/<point>` : `{"v":1,"i":2,"o":"ui"}`
(la commande, son issue, son origine), en QoS 1. C'est ce qui permet à tous les écrans d'afficher
« Ouvrir : exécuté par la box, 14 h 02 » sur un volet qui n'a pas d'état.

### 5.7 Exemple : ouvrir un volet par la box TaHoma

La box TaHoma (Somfy, plateforme Overkiz) est le cas d'école : elle pilote des volets **RTS**, une
radio à sens unique, et des volets **io-homecontrol**, une radio à deux sens. Le pilote
`moli-overkiz` existe (branche d'intégration, 45 tests) ; il ne publie que cinq classes, celles
observées : `rts:RollerShutterRTSComponent`, `rts:GateOpenerRTSComponent`, `ogp:Gate`, `ogp:Light`,
`ogp:Outlet` (`model.rs:17-23`). Aucune classe io, aucune position n'existe dans le dépôt.

Le pilote est déjà la forme du contrat pour le RTS : une voie, une émission, jamais de position
supposée. Il lui manque trois choses, que le crible a relevées (annexe D) :

- il rend **le même `Ok`** pour « la box a émis la trame » (`COMPLETED`) et pour « la box travaille
  encore » (accepté) : la V7 les sépare en `execute` et `accepte` ;
- il ne dit **jamais un succès tardif**, seulement un échec (`follow.rs:204-208,238-240`) : un ordre
  passé par un cloud lent, resté sans identifiant d'exécution, rend une erreur à la famille alors que
  le volet s'ouvre. La V7 publie la suite, succès comme échec ;
- la voie générique doit savoir que `control` est un **appui** : « ouvrir » puis « stop » ne se
  fusionnent ni ne s'inversent jamais. La file du pilote le garantit aujourd'hui ; la méta le dit.

<!-- FIGURE:volet -->

#### 5.7.1 Le sens lecture

| Étape | Ce que fait le module `overkiz` |
|---|---|
| Se connecter | API locale de la box en HTTPS sur le port 8443 : le certificat est **épinglé avant** tout envoi du jeton porteur ; ou le cloud (OAuth), si la box n'a pas d'API locale (le Connectivity Kit n'en a pas). La session cloud, partagée entre redémarrages, et l'attente d'un humain après trois refus passent par le canal de services |
| Déclarer | `/setup/devices` au démarrage : chaque appareil et ses points, publiés par le hub sur `meta/`. Un volet RTS a un seul point, `control` (ouvrir, stop, favori, fermer : les commandes que la box déclare), **en écriture seule**, `effet` appui, retour `pont`. Sémantique `Other` pour le portail (sinon la famille devrait taper le code), `Control` pour les autres machines |
| Relever | un auditeur d'événements (`/events/{id}/fetch`), relevé toutes les **10 s** au repos (30 s par le cloud, qui compte les appels), **toutes les secondes** pendant le suivi d'un ordre (2 s par le cloud) |
| Mettre au canon | trois états seulement aujourd'hui (`core:OnOffState`, `core:LightIntensityState`, `core:OpenClosed*State`) ; les nombres que le cloud envoie en texte (`"42"`) deviennent des entiers ; une commande devient un code |
| Santé | la box est le battement : un relevé réussi publie `$vu` pour chacun de ses appareils. Rythme 10 s, deux pertes tolérées : la box est déclarée **muette à 30 s** (§ 10.5). Une pause imposée (quota, file pleine, maintenance : 60 s) passe par `pause_jusqu_a`, et ne rend pas la box muette. Un appareil RTS n'émet jamais rien : sa disponibilité est celle de sa box |
| Panne de box | `GatewayDownEvent` met hors ligne **les seuls appareils de cette box** (`deviceURL` = `rts://<box>/<adresse>`) ; auditeur expiré : il se réenregistre et tout est relu |
| Ne jamais rejouer | un relevé vide le tampon de la box : il n'est jamais rejoué ; sa perte relance la liaison et relit tout |

Bloc technique — la méta d'un volet RTS et d'un volet io, publiée une fois par le hub (§ 7.4) :

```json
{"nom": "Volet du salon (RTS)", "semantic": "control", "voie": "box:1234-5678-9012",
 "sante": {"rythme_s": 0},
 "points": {
   "control": {"kind": "enum", "codes": [[1, "open"], [2, "stop"], [3, "my"], [4, "close"]],
               "ecrit": {"effet": "appui", "retour": "pont", "suivi_s": 120}, "lu": false}}}

{"nom": "Volet de la chambre (io)", "semantic": "control", "voie": "box:1234-5678-9012",
 "sante": {"rythme_s": 0},
 "points": {
   "control":   {"kind": "enum", "codes": [[1, "open"], [2, "stop"], [3, "my"], [4, "close"]],
                 "ecrit": {"effet": "appui", "suivi_s": 120, "retour": {
                   "1": {"etat": {"point": "position", "attendu": [100], "tol": 2, "a_l_arret": true}},
                   "2": "pont", "3": "pont",
                   "4": {"etat": {"point": "position", "attendu": [0], "tol": 2, "a_l_arret": true}}}}},
   "position":  {"kind": "numeric", "unit": "%", "echelle": 0, "min": 0, "max": 100,
                 "ecrit": {"effet": "consigne", "suivi_s": 120,
                           "retour": {"etat": {"point": "position", "tol": 2, "a_l_arret": true}}}},
   "mouvement": {"kind": "binary"}}}
```

Les codes sont des paires `[code, clé]` ; les libellés traduits ne voyagent pas dans la méta. Un
volet n'émet qu'à l'action : il n'est jamais déclaré muet, sa disponibilité est celle de sa box.

#### 5.7.2 Le sens action : « ouvrir le volet du salon »

1. **L'écran** envoie `ouvrir` au module WebSocket. L'api-to-mqtt de Sarah Home vérifie que le point
   est écrivable et que `open` est dans son dictionnaire, met la clé en code (`open` → `1`), et
   publie la demande sur `req/ui`.
2. **Le hub** applique la garde (origine, pièce, heure), écrit la ligne de journal, et publie
   l'ordre sur `cmd/overkiz/salon_volet/control`, avec son échéance absolue (5 s au plus).
3. **Le module** passe l'ordre par la voie de la box : **une seule file pour toute la box** (la clé
   de voie est la box), 8 ordres au plus derrière celui en vol, 16 appelants au plus en attente
   d'une réponse ; au-delà, `non_envoye` (cause `voie_pleine`). `control` est un appui : rien n'est
   fusionné. Juste avant l'envoi, le permis revérifie l'appelant, la pause et l'échéance.
4. Il traduit le code en commande Overkiz et appelle **`/exec/apply` une seule fois**, sans
   réessai, borné par l'échéance. La box répond par un identifiant d'exécution, puis émet la trame
   radio. Le volet RTS s'ouvre ; il ne dit rien.
5. Le relevé passe à une seconde et suit l'exécution : `TRANSMITTED`, `IN_PROGRESS`, puis
   `COMPLETED`. Un changement d'état peut arriver **avant** la réponse d'`/exec/apply` : il est
   gardé, puis rattaché.
6. **L'issue** part 400 ms avant l'échéance du hub au plus tard (§ 5.6) :

| Ce que la box a dit | Issue | Ce que voit la famille |
|---|---|---|
| `COMPLETED`, volet RTS | `execute` | « Ouvrir : exécuté par la box » ; jamais « ouvert » |
| `COMPLETED`, volet io, position à 100 ± 2 et mouvement arrêté | `confirme` | « Ouvert » |
| `FAILED` | `echec`, avec la raison de la box en code | « Échec : … » |
| toujours en cours à l'échéance − 400 ms | `accepte` ; le suivi dure `suivi_s` (120 s) ; la suite est publiée une fois, succès comme échec | « Ouverture en cours… », puis « Ouvert » |
| aucun identifiant d'exécution à l'échéance (cloud lent) | `incertain` (cause `sans_reponse`) ; l'exécution reste suivie par sa clé, et sa fin est publiée | « Peut-être parti… », puis la suite |
| la liaison s'est perdue pendant l'envoi | `incertain` (cause `liaison`) ; **jamais renvoyé** | « Peut-être parti : à vérifier » |
| jamais parti : file pleine, box en pause ou hors ligne, appelant parti | `non_envoye`, avec sa cause | « Non envoyé : box hors ligne » |

Bloc technique — ouvrir un volet RTS, message par message :

```text
→ req/ui                               {"id":"01JA…W2","p":"salon_volet/control","v":1,"a":"ui:session-3f2a","e":1792675629123}
→ cmd/overkiz/salon_volet/control      {"id":"01JA…W2","v":1,"e":1792675629123}
   module → box   POST /exec/apply     {"label":"Sarah Home","actions":[{"deviceURL":"rts://1234-5678-9012/16756006",
                                         "commands":[{"name":"open","parameters":[]}]}]}
   box → module                        {"execId":"0f6c…"}
   relevés (1 s)                       ExecutionStateChangedEvent TRANSMITTED → IN_PROGRESS → COMPLETED
→ res/ui                               {"id":"01JA…W2","i":2,"ms":930}                 (2 = execute)
→ evt/salon_volet/control              {"v":1,"i":2,"o":"ui"}                           (pour tous les écrans)
   aucun val/ : un volet RTS n'a pas d'état
```

Les ordres d'une même box partent dans l'ordre, un par un. Ce n'est pas une prudence : des ordres
envoyés en tâches indépendantes arrivent inversés (le cloud ouvre une connexion par requête), et
« ouvrir » puis « stop » deviennent « stop » puis « ouvrir ».

#### 5.7.3 Un volet io : la position revient

io-homecontrol est une radio à deux sens : le moteur dit où il est. Le pilote ne connaît aujourd'hui
**aucune** classe io ; les ajouter demande cinq changements de code et précise le contrat :

| | Volet RTS | Volet io |
|---|---|---|
| Points | `control` (écriture, appui) | `control` (écriture, appui), `position` (lecture et consigne), `mouvement` (lecture) |
| Retour déclaré | `pont` pour tous les codes | **par code** : `open` et `close` → `etat` sur `position` (100 et 0, à 2 près, à l'arrêt) ; `stop` et `my` → `pont`, car le moteur s'arrête où il est et la position favorite est dans le moteur |
| Mise au canon | aucune valeur lue | la box donne `core:ClosureState`, de 0 (ouvert) à 100 (fermé) ; le canon des écrans est l'inverse : le module écrit `100 − closure`, en entier |
| Ordre « 50 % » | impossible : seuls ouvrir, stop, favori, fermer | une consigne `position` : `setClosure(50)`, après la même inversion ; les consignes successives se fusionnent, jamais par-dessus un `stop` en attente |
| Meilleure issue | `execute` | `confirme` à l'arrêt, position atteinte ; le plus souvent `accepte` puis la suite tardive, car le trajet dure une vingtaine de secondes |
| Ce qu'un automatisme peut attendre | rien : « attendre que le volet soit ouvert » est refusé à la validation | `wait_for position = 100`, avec un délai |

Les cinq changements de code : admettre les classes io (`SUPPORTED`, `model.rs:17-23`, par liste ou
par capacité : un état `core:ClosureState` et une commande `setClosure`) ; un point `position` et un
point `mouvement` (`model.rs:142-201`) ; lire `core:ClosureState` (`model.rs:277-309`) ; écrire
`setClosure` (`Index::encode`, `model.rs:328-357`) ; donner au suivi (`Tracker`) une cible
`(point, valeur, tolérance)` et un chemin de succès tardif (`follow.rs:36-45`).

**Non vérifié :** `COMPLETED` veut-il dire, pour un moteur io, « position atteinte » ou « ordre
accusé » ? Aucune box réelle n'a été vue (fixtures reconstruites). La qualification sur la vraie box
doit le mesurer avant de figer l'issue d'un volet io.

Un volet io met une vingtaine de secondes à s'ouvrir, plus que l'échéance de 5 s. L'issue est donc
le plus souvent `accepte`, puis la position arrive par le sens lecture, comme n'importe quelle
valeur, et la suite tardive conclut l'ordre. C'est voulu : **l'issue dit le sort de l'ordre, la
valeur dit l'état de la maison.**

### 5.8 Exemple : l'aspirateur Roborock, une tâche longue

Le pilote `moli-roborock` parle au robot en local, en miIO chiffré sur UDP 54321, sans compte
cloud (branche d'intégration, 17 tests). Il montre l'autre forme d'action : un ordre court, une
tâche de quarante minutes.

<!-- FIGURE:aspirateur -->

| Sens | Ce que fait le module `roborock` |
|---|---|
| Lecture | `get_status` toutes les **30 s** ; toutes les **5 s** quand le robot travaille (états actifs) et pendant la minute qui suit un ordre. Points au canon : état (23 codes **natifs** du robot), erreur (28 codes natifs), batterie (%), en ménage (dérivé de l'état), puissance d'aspiration, durée et surface du nettoyage (en dixièmes, entiers d'échelle 1). Un code inconnu vaut « inconnu », jamais une supposition. Aujourd'hui, l'état est publié en texte anglais (`"cleaning"`) et deux codes d'erreur partagent un libellé : le code natif devient le code canon |
| Santé | rythme 30 s ; trois relevés manqués de suite mettent le robot hors ligne (muet à 90 s). Une réponse sans statut exploitable (état inconnu, batterie hors de 0 à 100) le met hors ligne **et interdit les ordres**. Aujourd'hui, « injoignable » passe aussi par `wait_for` : c'est un statut de santé, il devient `disponibilite(false)` seul |
| Action | `control` en écriture seule, `effet` appui (« pause » puis « stop » ne se fusionnent pas) : démarrer, pause, stop, retour à la base ; sous la garde. Une file de 4 derrière l'échange en cours ; appelant et échéance revérifiés avant l'émission ; un datagramme, **aucun réessai** |
| Retour déclaré | `accuse`, puis l'état **par code** (tableau ci-dessous) ; `suivi_s` 60 s, la fenêtre de relevé rapide qui existe déjà |
| Hors MQTT | rien aujourd'hui : les cartes, les pièces et les zones **n'existent pas** dans le pilote. S'il les ajoute, une carte est un document servi à la demande par REST, et un nettoyage de zones un « objet court » validé par schéma, jamais une valeur sur le courtier |

| Ordre | Réponse « ok » | Confirmé quand l'état devient… | Erreur |
|---|---|---|---|
| démarrer | `accepte` | `starting`, `cleaning`, `spot_cleaning`, `zoned_cleaning` ou `segment_cleaning`, avant l'échéance | une erreur de l'appareil : `echec` ; `state = error` dans les 60 s : suite tardive `echec` avec le code d'erreur |
| pause | `accepte` | `paused` | idem |
| stop | `accepte` | non vérifié (aucun robot réel) : à qualifier | idem |
| retour à la base | `accepte` | `returning_home`, `docking`, `charging` ou `charging_complete` | idem |
| silence | `incertain` (`sans_reponse`), jamais renvoyé | | |

Aujourd'hui, aucun lien n'existe entre l'ordre et l'état qui suit : personne n'est prévenu, au titre
de l'ordre, d'un robot qui accepte « démarrer » puis tombe en erreur. Une panne vingt minutes plus
tard n'est pas l'échec de l'ordre : c'est une valeur (`error`), que le workflow lit sur `val/`.

La suite n'est plus une affaire d'ordre, mais de **workflow** : « quand tout le monde est parti,
lancer l'aspirateur ; attendre qu'il soit revenu à la base (2 h au plus) ; prévenir s'il est en
erreur ». L'attente est un `wait_for` sur `val/aspirateur/etat`, gardée par un **bail durable** :
elle survit à un redémarrage et à une bascule (§ 13). Le moteur n'interroge jamais le robot.

Bloc technique — lancer le nettoyage, puis attendre le retour à la base :

```text
→ req/automation                       {"id":"x52:n2","p":"aspirateur/control","v":1,"a":"auto:depart","e":1792675629123}
→ cmd/roborock/aspirateur/control      {"id":"x52:n2","v":1,"e":1792675629123}
   module → robot (UDP 54321)          app_start          robot → module : ["ok"]
   relevés toutes les 5 s              get_status → {"state":5,"battery":96,"error_code":0,…}
→ in/roborock/aspirateur/etat          {"v":5,"o":1792675625310,"n":118}                (5 : cleaning, code natif)
→ val/aspirateur/etat                  {"v":5,"s":88412,…}
→ res/automation                       {"id":"x52:n2","i":1,"ms":2740}                  (1 = confirme)
   … 41 min plus tard
→ val/aspirateur/etat                  {"v":8,"s":91877,…}                              (8 : charging)
   le moteur : wait_for « etat ∈ {charging, charging_complete} » → port « c'est fait » → nœud suivant
```

La tondeuse Landroid a la même forme, à une différence près : son accusé est celui du courtier du
cloud Worx, pas celui de la tondeuse. C'est `envoye`, puis `confirme` quand son statut change ; et
son planning est une lecture-modification-écriture du planning entier, un ordre composé (§ 5.1).

### 5.9 Toutes les intégrations, dans les deux sens

Le contrat a été éprouvé sur **toutes** les intégrations du dépôt : les 37 genres de pilotes de
`KINDS` (`moli-os/src/main.rs:994-1032`), dont les aides, la démo, la présence et les profils, plus
les luminaires, l'énergie et les personnes. Huit sous-agents ont lu chacun un lot de crates, une
quarantaine de leurs citations `fichier:ligne` ont été revérifiées sans erreur ; le rapport complet
est l'annexe D. Le critère qui les range est celui du § 5.6 : **ce que l'appareil sait dire après
un ordre.**

Verdict, sur 58 lignes : **12 couvertes** telles quelles ; **31 avec réserves** (la lecture et
l'action tiennent, un détail du kit ou de la méta manquait) ; **6 mal couvertes** (Philips, les
ordinateurs, les minuteries, les luminaires, les réservations, l'alarme) ; **9 hors du modèle
« module »** (briques propriétaires du hub, canaux privés, cache REST, bibliothèques). La lecture
de tout ce qui est une valeur courte était couverte d'emblée ; les écarts sont dans l'action et dans
les services. Ils sont tous réglés au § 5.1 et au § 5.3 ; leur liste est au § 5.10.

**L'état revient : `confirme`** (retour `etat`)

| Intégration | Sens lecture : comment, à quel rythme | Sens action | Ce qui confirme | À noter |
|---|---|---|---|---|
| Zigbee (`z2m`) | poussée MQTT ; les routeurs sont relus au démarrage (`/get`, un toutes les 500 ms) | selon les `exposes` : lumières, prises, vannes, thermostats… | l'état republié par Zigbee2MQTT | un appareil endormi répond tard : `envoye` ; les codes des claviers passent par le crochet du courtier (§ 5.1) |
| Philips Hue (`hue`) | flux SSE du pont ; muet 10 min, il est rouvert | lumières, pièces | l'événement de la lumière | appairage : un humain appuie sur le bouton du pont |
| Tuya local (`tuya`) | connexion TCP chiffrée permanente : battement 10 s, morte à 35 s ; puissance redemandée toutes les 30 s | booléens, entiers, énumérations | le rapport de l'appareil après l'écriture | capteurs sur pile : par le cloud, en lecture seule ; le catalogue est lu une fois (un ajout demande un import) |
| Tapo (`tapo`) | relevé toutes les 5 s | tout | une relecture juste après l'écriture | aujourd'hui, la valeur demandée est publiée comme état dès le succès : un écho optimiste, à retirer |
| Sonos (`sonos`) | relevé toutes les 2 s | lecture, volume, sourdine ; annonce | le relevé ; une annonce est `accepte`, puis la suite (voix et clip : une minute au plus) | même écho optimiste aujourd'hui ; le clip de l'annonce est hors MQTT |
| Télés : Philips, Samsung, Android TV ; Cast | relevé 5 s (Philips), 30 s (Samsung) ; poussée (Android TV Remote, canal Cast) | marche, volume, sourdine, appli ; touches | l'état relu ; une touche Philips (accusée par un 2xx) est `execute`, une touche Samsung (aucun accusé) `envoye` ; Samsung ne confirme qu'à son relevé de 30 s | allumer par Wake-on-LAN : `reveillable`, `envoye` puis la suite ; Philips ne doit plus mettre `power=false` quand la télé ne répond pas ; un média Cast est une adresse, hors MQTT |
| Enceintes Echo (`alexa`) | cloud de l'appli Alexa : 30 s, volumes 2 min | musique, lecture, pause, volume, phrase, annonce | le relevé (volume, lecture) ; une phrase ou une annonce est `execute` | interface non officielle ; la connexion au compte (`sign_in`) n'est jamais un ordre |
| ESPHome (`esphome`) | API native, connexion gardée, ping 20 s | les entités déclarées par l'appareil | l'état poussé par l'appareil | l'audio du satellite vocal reste hors MQTT |
| Frigate (`frigate`) | poussée MQTT (sujets de Frigate) | détection, enregistrement, instantanés | l'état republié par Frigate | images JPEG à la demande ; plus aucune image retenue sur le courtier |
| Imprimantes 3D : Bambu, Klipper (`moonraker`) | MQTT TLS poussé, rapport complet toutes les 5 min (Bambu) ; relevé 5 s en activité, 15 s au repos (Klipper) | lumière ; `control` (pause, reprise, arrêt ; un agent passe par un humain) | l'état **du code** : `control` se confirme sur `state` | caméra hors MQTT |
| Twinkly (`twinkly`) | relevé toutes les 15 s | marche, luminosité, couleur | la relecture, seule source des valeurs | une consigne demande jusqu'à 14 échanges : un ordre composé |
| Profils déclaratifs (`profile` : Daikin…) | relevé à la période de chaque requête du profil (30 s à 1 h) | les `[[write]]` du profil | une relecture juste après l'écriture, à ajouter au format | l'écriture Daikin fait trois appels : un ordre composé |
| Aides (`helpers`) | internes au hub | tout | tout de suite : le hub est propriétaire du point, sans `cmd/` | `proprietaire = hub` |
| Luminaires (`lumieres`) | internes, sur d'autres appareils | marche, luminosité, couleur d'un groupe | les états des lampes commandées | aujourd'hui `Ok` dès qu'**une** lampe a obéi : la V7 rend `echec` cause `partiel`, `n/m` |

**Un accusé tout de suite, l'état plus tard : `accepte`, puis `confirme`** (retour `accuse`)

| Intégration | Sens lecture | Sens action | Ce qui accuse, puis ce qui confirme | À noter |
|---|---|---|---|---|
| Roborock (`roborock`) | UDP local, 30 s ; 5 s au travail | `control` | « ok » du robot, puis l'état, par code | § 5.8 |
| Worx Landroid (`landroid`) | cloud Worx, MQTT sur WebSocket, maintien 25 s | `control`, délai de pluie, planning | l'accusé du **courtier cloud** : `envoye` ; puis le statut de la tondeuse (suivi 60 s) | trop de connexions : 1 h de pause (`pause_jusqu_a`) ; la tondeuse se tait au repos : `veille` |
| Ariston, Voltalis | cloud : 3 min (Ariston), 2 min (Voltalis) ; 20 s ou 10 s pendant une confirmation | marche, modes, consignes, programmes | le cloud prend l'ordre, puis la valeur se relit | la confirmation arrive presque toujours **après** l'échéance : suite tardive ; pauses imposées de 900 s et 600 s |
| Nuki (`nuki`) | HTTP local, `/list` toutes les 30 s | verrouiller ; déverrouiller et tirer le pêne, retenus pour un humain | le pont répond, puis l'état (« déverrouillage » est transitoire) | `retenue` pour ouvrir ; une serrure bloquée n'est jamais « verrouillée » |
| TaHoma io (`overkiz`, classes à ajouter) | événements de la box, 10 s | position, commandes | `COMPLETED`, puis la position à l'arrêt | § 5.7.3 |
| Ordinateurs (`host`) | `/proc` toutes les 15 s ; l'agent d'un PC rapporte toutes les 5 s | allumer (Wake-on-LAN), éteindre, redémarrer, veille ; session à distance | l'agent prend l'ordre dans sa boîte : `accepte` ; l'ordinateur reparaît ou disparaît | aujourd'hui la boîte garde un ordre 15 min et le rejoue jusqu'à `done`, qui veut dire « reçu » : la V7 donne une durée de vie par type d'ordre ; la session à distance devient un appui à décision humaine |

**Le pont ou l'appareil dit « fait », ou rien du tout : `execute` ou `envoye`**

| Intégration | Sens lecture | Sens action | Ce qui revient | Issue |
|---|---|---|---|---|
| TaHoma RTS, portails (`overkiz`) | événements de la box, 10 s ; aucun état d'appareil | ouvrir, stop, favori, fermer | `COMPLETED` de la box | `execute` ; jamais de position supposée (§ 5.7) |
| Imprimante (`ipp`) | 30 s ; 4 s pendant une impression | identifier, annuler ; imprimer (tableau de bord) | `cancel` rend aujourd'hui `Ok` même quand `Cancel-Job` échoue, et n'est jamais relu | `envoye` ; le document imprimé est hors MQTT |
| Telegram (`telegram`) | — | prévenir | l'API rend le message envoyé | `execute` ; une série de dix conversations : `echec` `partiel` si l'une échoue |
| Téléphones (`phones`) | rapports poussés par l'appli | prévenir | le ticket du service de push ne dit rien de la remise au téléphone | `envoye` (`accepte` puis la suite si la requête dépasse 5 s) |

Pour ces deux dernières lignes, le texte du message ne passe jamais par `cmd/` : l'ordre porte une
référence, le texte passe par le canal de services (`sensible: texte_prive`, § 5.1).

**Lecture seule : aucun ordre**

| Intégration | Sens lecture | Santé, exemple de rythme |
|---|---|---|
| Présence (`presence`) | un paquet UDP par adresse chaque minute, puis la table ARP | rythme 60 s ; absent après 10 min sans signe ; une table ARP illisible rend la sonde indisponible, jamais tout le monde « absent » |
| Box internet (`igd`) | UPnP toutes les 10 s | `uptime` quantifié (sinon six messages par minute pour rien) |
| Routeurs (`asuswrt`, `netgear`), clé 4G (`huawei-lte`) | HTTP ou SOAP toutes les 60 s, chaque cycle borné à 30 s | le rythme du cycle ; des requêtes lentes (5 min, 1 h) déclarent leurs points à part dans `$vu` |
| Caméras Reolink (`reolink`) | événements relevés chaque seconde | la sonnette est un niveau relevé : `source: niveau`, résolution 1 s ; images à la demande, hors MQTT ; `say` et l'interphone sont des ordres (`accepte`) |
| Netatmo (`netatmo`) | cloud du fournisseur, OAuth donné par un humain | une date de mesure répétée ne rafraîchit rien : pas de `$vu` sans horodatage source plus récent |
| Wyoming (`wyoming`) | sonde `describe` des services vocaux | aucun audio |
| Réservations (`reservations`) | calendrier ICS au coffre | les fenêtres d'accès passent par le canal de services, jamais par `in/` ; aucun titre ni participant publié |
| Météo, Tempo, iopool, Meross, SMLIGHT (`profile`) | relevé à la période de chaque requête | `observed_at` de la source ; `[sante]` déclarée dans le profil |

### 5.10 Les 26 écarts du crible, et où la V7 les règle

| # | Gravité | L'écart trouvé dans le code | La règle de la V7 | Où |
|---|---|---|---|---|
| G1 | haute | au moins 17 intégrations ont une suite après les 5 s (de 30 s à 15 min) ; aujourd'hui : silence, notice ou journal | `suivi_s`, `Suivi::conclure` une fois, succès comme échec ; le hub conclut à l'expiration ; seconde ligne de journal ; confirmation jugée sur la lecture brute | 5.1, 5.3, 5.6 |
| G2 | haute | `Kind::Button` n'accepte que `null` : touches, `control`, `unlatch` sont des énumérations ; la voie pourrait fusionner trois « volume + » ou sauter un « stop » | `effet` : consigne ou appui ; fusion des seules consignes consécutives, jamais par-dessus un appui ; cause `remplace` | 5.1, 5.3, 20.2 |
| G3 | haute | `Confirmations` compare « même point, même valeur » ; Roborock, Bambu, Klipper, Landroid, Nuki confirment sur `state`, parmi plusieurs valeurs | retour **par code** : point, valeurs attendues, tolérance, « à l'arrêt » | 5.1, 5.6 |
| G4 | haute | 24 genres lisent le coffre, 15 y écrivent, 27 attendent un humain, 5 partagent une passerelle avec l'API : « le contrat ne change pas » en processus à part était faux | le canal de services, hors courtier ; les modules à passerelle partagée restent dans le processus du hub | 5.1, 5.3, 9.4.2 |
| G5 | moyenne | `null` publié par au moins 18 intégrations sur un appareil en ligne ; Philips met `power=false` quand la télé ne répond pas ; silences normaux pris pour des pannes | `null` = inconnu, qualité dite ; `veille` ; aucun rythme appris quand le silence est le signal | 5.1, 10.5 |
| G6 | moyenne | `$vu` par appareil contre plusieurs rythmes ; « une réponse réussie compte comme une nouvelle », faux pour Netatmo, iopool, Tempo, Open-Meteo ; pauses imposées plus longues que le délai | `$vu` avec liste de points et horodatage source ; fenêtre ≤ rythme ; `pause_jusqu_a` | 5.1, 5.3, 10.5 |
| G7 | haute | un code de clavier Z2M entrerait dans le courtier ; l'URL `sign_in` d'Alexa passe par le journal ; position et textes de notification sont des points ordinaires | jamais de secret sur `moli/v1` ni au journal ; `sensible` ; crochet d'entrée du courtier pour les claviers ; flux humains avec secret en REST | 5.1, 16, 19.2 |
| G8 | haute | téléphones et agents de PC **appellent** Sarah Home ; la boîte des PC rejoue un ordre 15 min jusqu'à « reçu » | passerelle HTTP dans le processus du hub ; boîte à durée de vie par type d'ordre ; `accepte` puis la suite | 5.1, 5.9 |
| G9 | moyenne | images, documents, voix sans appel de kit ; la méta n'a ni sémantique, ni pièce, ni membres ; sept pilotes retirent des appareils | `fournir_images`, `fournir_document`, `media` avec politique d'accès ; méta étendue ; `retirer` | 5.1, 5.3, 7.4 |
| G10 | moyenne | textes bornés en caractères (4 000) contre une charge bornée en octets (4 096) ; méta de 5 Kio ; Zigbee2MQTT dépend de retenus | bornes en octets ; budget de méta ; exception de retenus bornée pour l'arbre de Zigbee2MQTT ; plus d'image retenue de Frigate | 5.1, 6.3 |
| G11 | moyenne | énumérations à libellés pour valeurs (aides, profils, Sonos, IPP…) ; vocabulaires ouverts | `{code, cle, libelle}`, code natif, repli « autre », code jamais réattribué, aucune prose traduite en valeur | 5.1 |
| G12 | basse | valeurs composées (couleur, planning, filament, `play_media`) | « objet court » de 3 Kio au plus, schéma déclaré | 5.1 |
| G13 | moyenne | ordres composés (Daikin trois appels, Twinkly jusqu'à 14, volume touche à touche) ; une bascule de repli chez Philips | « une seule fois » par ordre ; la voie gardée jusqu'à la dernière étape ; échec après une écriture : `incertain` ; aucune bascule | 5.1 |
| G14 | moyenne | Wake-on-LAN contre « refusé si hors ligne » | `reveillable` ; le refus prédictif vise un bus ou un module, jamais un appareil qui dort | 5.1, 20.2 |
| G15 | basse | voies par compte (Ariston, Voltalis), par box (TaHoma), par station (Reolink) | clé de voie choisie par le module ; causes `remplace`, `voie_pleine` | 5.1, 5.3 |
| G16 | moyenne | 14 verdicts de l'alarme et les causes des pannes perdus dans neuf issues | raison `r` (vocabulaire fermé) et détail `d` (200 o) sur `res/` | 5.1, 5.6 |
| G17 | basse | envois en série (Telegram, Alexa), luminaires `Ok` dès une lampe | `echec` cause `partiel`, `n/m` | 5.1, 5.9 |
| G18 | moyenne | aides, minuteries, compteurs, calculs, scènes sont propriétaires du hub ; idempotence par `request_id` ; livraison durable de la fin d'une minuterie ; sept fichiers métier hors de la réplication | `proprietaire = hub`, sans `cmd/` ; `volatile` ; clé d'idempotence ; fichiers ajoutés à la réplication | 5.1, 5.3, 18.2.2 |
| G19 | moyenne | sonnettes dérivées d'un niveau relevé ; occurrences sans instant ni identité | `evenement` avec instant et identité ; `source: niveau` | 5.1, 5.3 |
| G20 | moyenne | « ses ordres en cours sont refusés » quand un module meurt ; or un ordre parti sans réponse n'est pas « non envoyé » | `incertain`, cause `module` | 5.1, 18.2.1 |
| G21 | basse | permis vivants (invités, scènes, exécutions) revérifiés jusqu'à l'émission | échéance min(5 s, fin de fenêtre) ; `$annule` | 5.1, 5.3 |
| G22 | moyenne | fenêtres d'accès des invités : un registre éphémère de 4 096 entrées, jamais restauré | appel en mémoire `fenetres`, hors courtier | 5.1, 5.3 |
| G23 | moyenne | ordres de l'alarme à code ; origine « clavier » absente ; sortie différée de 60 s | canal privé de l'alarme, origine `clavier`, `suivi_s` sur l'armement | 5.1, 20.11 |
| G24 | moyenne | format des profils : échelle en flottant, libellé pour valeur, santé non déclarable, et **approbation forgeable** (`moli-os profile trust` enregistre l'empreinte sans question) | `observed_at`, `fraicheur_s`, `[sante]`, table de codes, `round` obligatoire, bornes d'ordre à part, approbation par un geste authentifié | 5.4, 19.2 |
| G25 | basse | `nom()` par genre, alors que le code confine par instance | `<module>` des sujets = instance | 5.1, 5.3 |
| G26 | basse | valeurs qui varient sans cesse (`uptime`, débits, `remaining`) | durées croissantes quantifiées ; `volatile` | 5.1 |

## 6. Le moteur MQTT embarqué

### 6.1 `rumqttd`, repris et complété

`rumqttd` est le courtier Rust de Bytebeam, sous licence Apache-2.0. Rien n'empêche de le
reprendre dans l'espace de travail de Sarah Home, sous la forme d'une crate `moli-courtier`, et de
compléter ce qui manque. Ce qu'on en garde, et ce qu'on y ajoute :

| On garde | Code lu (`rumqttd` 0.20.0) |
|---|---|
| Le routeur, sur son thread, avec un journal par filtre découpé en segments | `router/`, `max_segment_size`, `max_segment_count` |
| Les codecs MQTT 3.1.1 et 5, les messages retenus, le testament, le maintien de liaison | `protocol/` |
| Les clients en mémoire, sans TCP : `Broker::link(client_id)` rend un `LinkTx` et un `LinkRx` | `server/broker.rs:150`, `link/local.rs` |
| Les écouteurs TCP et TLS, l'authentification à la connexion | `lib.rs:146-168` |

| On ajoute | Pour qui | Pourquoi |
|---|---|---|
| **Des ACL par compte** (motifs avec `+` et `#`), vérifiées à chaque publication et à chaque abonnement | les **seuls clients extérieurs** | `rumqttd` n'a pas d'autorisation par sujet ; son seul cloisonnement, le tenant, exige des certificats clients (`router/connection.rs:41-59`, `server/tls.rs:64-76`) |
| Un compte en lecture seule | la machine de secours | elle lit tout et n'écrit jamais |
| Des quotas par client extérieur : taille, débit, file | Zigbee2MQTT, appareils du réseau local | une inondation ou un appareil fou ne passe pas le courtier |
| Des compteurs exposés à `/api/health` | tous | la santé du courtier se voit comme celle d'un pilote |
| **Des caches par sujet bornés** | tous | mesuré : chaque sujet neuf est gardé à vie dans le cache « sujet → filtres » (≈ 190 o), et chaque connexion garde un compteur par sujet publié (73 o) ; des sujets à identifiant unique fuient donc sans fin |
| **Un signal de perte** à l'abonné qui saute en avant | tous les abonnés | mesuré : au-delà de ~1 400 messages de retard, l'abonné perd des blocs entiers sans être averti ; l'alerte interne `cursor_jump` a un champ `lost` toujours à 0 |
| La correction d'une panique du chemin de connexion | les clients TCP | observé sous macOS : `server/broker.rs:544` panique en tenant un verrou, qui reste empoisonné ; plus aucune connexion n'est acceptée jusqu'au redémarrage |

La reprise reste un petit correctif sur le code d'origine. Il touche le routeur aux endroits où une
publication et un abonnement sont acceptés et où les retenus sont livrés, plus la configuration. Il faut pouvoir suivre
les versions de `rumqttd` sans réécrire. Le jour où le correctif grossirait, l'option d'un moteur
propre à Sarah Home, construit sur les mêmes codecs, reste ouverte.

### 6.2 Deux niveaux de confiance, et un chemin rapide

| Client | Confiance | Connexion | Vérification par message |
|---|---|---|---|
| Modules de Sarah Home dans le binaire (api-to-mqtt, hub, workflow, base, WebSocket, API) | **de confiance** : notre code, compilé, relu | lien en mémoire | **aucune** : le kit forme les sujets (§ 5.3), le chemin reste le plus court |
| Modules de Sarah Home en processus à part | **de confiance** : le même code, isolé pour sa robustesse, pas pour sa loyauté | écouteur local, compte authentifié | **aucune** |
| Zigbee2MQTT, contrôleur Matter | **extérieurs** | écouteur local, compte authentifié | **ACL** sur leur arbre (`zigbee2mqtt/#`), bornes, quota |
| Appareils MQTT du réseau local (Tasmota, Shelly…) | **extérieurs**, et sur le réseau | écouteur du réseau local, TLS si l'appareil le sait | **ACL** sur leur arbre, bornes, débit ; jamais `moli/v1/…` |
| Machine de secours | de confiance, en lecture seule | écouteur du réseau local, TLS, certificat exigé | **ACL en lecture seule** |

Le coût de la vérification ne tombe que sur les clients extérieurs. Vérifier un sujet d'une
quarantaine d'octets contre quelques motifs coûte des dizaines de nanosecondes (mesuré au § 11.4),
à comparer aux microsecondes d'un passage par le courtier. Au rythme réel de Zigbee2MQTT (quelques
messages par seconde, 239 par seconde à la pointe des deux maisons), c'est invisible. Le chemin des
modules internes, lui, n'en paie rien.

### 6.3 Ce que le courtier garde, et ce qu'il ne garde pas

| État | Où | Survit à un redémarrage ? |
|---|---|---|
| Valeurs courantes, disponibilités, méta | **le hub seul** (vérité, journal) ; **aucun message retenu** : un abonné qui veut l'état le demande au hub (§ 10.4) | le hub, oui, par son journal ; le courtier n'a rien à reconstruire |
| Sessions des clients | aucune : **sessions propres partout** | sans objet : aucun ordre ne doit survivre dans une file (§ 8.4.3) |
| Ordres, demandes, issues | jamais retenus | non, et c'est voulu |
| Messages QoS 1 en vol | files bornées du courtier | non : l'échéance de l'ordre tranche |
| Retenus d'un pont extérieur qui en dépend (Zigbee2MQTT : catalogue, disponibilités) | une exception bornée, sur son seul arbre, plafonnée en nombre et en taille ; jamais un message de clavier (§ 5.1) | non : le pont les republie à sa reconnexion |

Le courtier n'a donc **rien à persister ni à répliquer**, et rien à reconstruire au démarrage : la
machine de secours démarre son courtier vide, et il le reste.

Ne garder aucun message retenu a été mesuré par le sous-agent mémoire (§ 11.4). Un retenu coûte près
de 1 Kio dans `rumqttd` 0.20, soit 2,7 Mio à ×1, 9 Mio à ×3 et 32,6 Mio à ×10, pour une copie que le
hub tient déjà. Et un nouvel abonné n'en recevrait de toute façon que 200 (§ 11.4).

### 6.4 Configuration

Bloc technique — le courtier de Sarah Home : la forme de `rumqttd` 0.20, avec les ajouts de Sarah Home marqués (proposé) :

```toml
id = 0

[router]
max_connections = 128                 # modules + ponts + secours : quelques dizaines
max_segment_size = 65536              # 64 Kio par segment du journal d'un filtre
max_segment_count = 4                 # mesuré : plafond de 2,6 à 2,8 Mio ; un abonné qui ne lit plus ne perd
                                      # rien jusqu'à ~1 400 messages de retard (~7 s à 200/s, ~5 min au
                                      # rythme moyen), puis saute en avant et comble par le hub (§ 10.3).
                                      # L'exemple de rumqttd (100 Mio × 10) tue un conteneur de 128 Mio.

# Le filtre in/# (les modules vers le hub) a un journal plus long : le hub ne doit jamais sauter.
[router.custom_segment."moli/v1/in/#"]
max_segment_size = 1048576
max_segment_count = 4                 # plafond ~10 Mio, ~25 000 messages de marge

# Écouteur 1 : la machine seulement. Ponts extérieurs et modules de Sarah Home en processus à part.
[v4.local]
name = "local"
listen = "127.0.0.1:1883"
next_connection_delay_ms = 1
    [v4.local.connections]
    connection_timeout_ms = 5000
    max_payload_size = 4194304        # bridge/devices de Zigbee2MQTT : le catalogue entier (§ 14.2.3)
    max_inflight_count = 100
    dynamic_filters = true
    # mots de passe : au coffre de Sarah Home, jamais ici

# Écouteur 2 : le réseau local. Appareils MQTT et machine de secours, chiffré.
[v4.reseau]
name = "reseau"
listen = "0.0.0.0:8883"
next_connection_delay_ms = 100
    [v4.reseau.tls]
    capath = "/data/courtier/ca.pem"
    certpath = "/data/courtier/courtier.pem"
    keypath = "/data/courtier/courtier.key"
    [v4.reseau.connections]
    connection_timeout_ms = 5000
    max_payload_size = 4096           # une valeur courte, jamais plus (§ 16)
    max_inflight_count = 20
    dynamic_filters = true

# AJOUT DE SARAH HOME : confiance et ACL par compte. Un compte absent d'ici n'est pas admis.
[confiance]
"module-cloud-x" = "interne"          # de confiance : aucune vérification par message
"zigbee2mqtt"    = { lire = ["zigbee2mqtt/#"], ecrire = ["zigbee2mqtt/#"], max_par_seconde = 500 }
"tasmota-prises" = { lire = ["tasmota/+/cmnd/#"], ecrire = ["tasmota/+/stat/#", "tasmota/+/tele/#"], max_par_seconde = 20 }
"secours"        = { lire = ["moli/v1/#"], ecrire = [] }      # lecture seule
```

`max_payload_size` est fixé par écouteur. Les 4 Mio de l'écouteur local servent au catalogue de
Zigbee2MQTT ; le réseau local garde 4 Kio par message. Les liens en mémoire ne passent par aucun
écouteur : la règle des 4 Kio y est tenue par le kit des modules (§ 5.3).

Bloc technique — un client en mémoire, tel que le kit des modules l'enveloppe (API de `rumqttd` 0.20) :

```rust
// Au démarrage de Sarah Home : le courtier sur son thread, puis un lien par composant.
let mut courtier = rumqttd::Broker::new(config);
let (mut tx, mut rx) = courtier.link("hub")?;          // pas de TCP, pas de chiffrement, pas d'ACL
std::thread::Builder::new().name("courtier".into()).spawn(move || courtier.start())?;

tx.subscribe("moli/v1/in/#")?;                         // le hub lit ce que les modules publient
// Les modules du binaire partagent un seul lien abonné à cmd/# ; le kit répartit par module (§ 11.4).
tx.subscribe("moli/v1/req/#")?;                        // et les demandes d'ordre
tx.publish("moli/v1/val/salon_thermo/temperature", charge)?;  // il publie la vérité, jamais retenue
while let Some(notification) = rx.next().await? {      // et lit à son rythme
    // Notification::Forward(publication) → ingestion, garde…
}
```

## 7. Les sujets et les charges

### 7.1 L'arbre des sujets

Bloc technique — l'arbre `moli/v1/`, qui l'écrit, qui le lit :

```text
moli/v1/in/<module>/<appareil>/<point>     module → hub     QoS 0   non retenu   une valeur au canon, avec n (rang du module)
moli/v1/in/<module>/<appareil>/$vu         module → hub     QoS 0   non retenu   l'appareil a répondu (instant « reçu »)
moli/v1/evt/<appareil>/<point>             hub → tous       QoS 1   non retenu   une occurrence : appui, sonnette, passage
moli/v1/val/<appareil>/<point>             hub → tous       QoS 0   non retenu   la vérité : valeur, instants, qualité
moli/v1/avail/<appareil>                   hub → tous       QoS 1   non retenu   en ligne, hors ligne, et pourquoi
moli/v1/meta/<appareil>                    hub → tous       QoS 1   non retenu   spécification des points, dictionnaires
moli/v1/req/<origine>                      surfaces → hub   QoS 1   non retenu   une demande d'ordre ; l'identifiant est dans la charge
moli/v1/cmd/<module>/<appareil>/<point>    hub → module     QoS 1   non retenu   l'ordre après la garde, avec échéance
moli/v1/res/<origine>                      hub → surface    QoS 1   non retenu   l'issue : confirmé, non confirmé, retenu… (identifiant dans la charge)
moli/v1/sys/…                              hub → tous       QoS 1   non retenu   époque, santé des modules et des bus

Aucun message retenu : l'état courant se demande au hub (§ 10.4). Aucun sujet ne porte un identifiant
unique : les caches par sujet du courtier restent bornés (§ 11.4).
```

`v1` est la version du contrat : un changement incompatible ouvre `v2` à côté, sans couper les
clients de `v1`.

### 7.2 Qui a le droit d'écrire quoi

| Rôle | Écrit | Lit | Comment c'est tenu |
|---|---|---|---|
| Module (dans le binaire) | `in/<lui>/#`, l'issue de ses ordres | ses ordres, répartis par le kit depuis un lien `cmd/#` partagé | par le kit, à la compilation (§ 5.3) |
| Module en processus à part | idem | idem | de confiance : compte authentifié, aucune vérification par message (§ 6.2) |
| Zigbee2MQTT, appareils MQTT du réseau local | leur arbre (`zigbee2mqtt/#`, `tasmota/…`) | idem | **ACL du courtier** (ajout de Sarah Home), bornes, débit (§ 6.2) |
| Hub | `val/`, `evt/`, `avail/`, `meta/`, `cmd/`, `res/`, `sys/` | `in/#`, `req/#` | le seul composant qui écrit la vérité et les ordres |
| Moteur d'automatismes, module WebSocket, API de Sarah Home | `req/<origine>` | `val/`, `evt/`, `avail/`, `meta/`, leur `res/<origine>` | par le kit |
| Archiveur | rien | `val/#`, `evt/#`, `avail/#` | par le kit |

**Personne d'autre que le hub n'écrit `cmd/` ni `val/`.** C'est la traduction MQTT de la règle 9
(« le binaire fait respecter les règles ») : la garde ne se contourne pas par le courtier. Les modules de Sarah Home sont de confiance et ne paient aucune vérification ; les clients extérieurs sont vérifiés à chaque message.

### 7.3 La charge d'une valeur

Bloc technique — la valeur telle que le hub la publie sur `val/` :

```json
{"v":2150,"o":1792675624123,"r":1792675624125,"c":1792675624123,"s":918273}
```

| Champ | Sens | Note |
|---|---|---|
| `v` | la valeur au canon : entier mis à l'échelle, booléen, code d'énumération, texte court | l'échelle, l'unité et le dictionnaire sont dans `meta/` : ils ne voyagent pas avec chaque valeur |
| `o` | observé : l'instant de la mesure selon la source (ms) | — |
| `r` | reçu : le dernier rapport, même identique | tenu par `$vu` (§ 5.2) |
| `c` | changé : le dernier changement de valeur | — |
| `q` | qualité, présente seulement si elle n'est pas bonne : `u` incertaine, `m` mauvaise | un automatisme ne déclenche jamais sur `m` |
| `s` | le rang de la publication dans le flux `val/`, sans trou | tout abonné à `val/#` (base de données, workflow, module WebSocket, secours) voit un saut et sait qu'il a perdu des valeurs (§ 10.3) |

Les tailles, calculées octet par octet (MQTT 3.1.1, hors TCP, IP et TLS) :

| Message | Sujet | Charge | Total QoS 0 | Total QoS 1 (+ accusé de 4 o) |
|---|---|---|---|---|
| `in/` : une température lue par un module (avec son rang `n`) | 39 o | 37 o | **80 o** | 82 + 4 o |
| `val/` : la même, publiée par le hub | 36 o | 75 o | **115 o** | 117 + 4 o |
| `val/` : un contact (booléen) | 32 o | 75 o | 111 o | 113 + 4 o |
| `val/` : un mode (code d'énumération) | 23 o | 72 o | 99 o | 101 + 4 o |
| `val/` : une valeur incertaine | 36 o | 83 o | 123 o | 125 + 4 o |
| `req/` : allumer une lampe (identifiant dans la charge) | 14 o | 105 o | 123 o | 125 + 4 o |
| `cmd/` : l'ordre au module | 30 o | 62 o | 96 o | 98 + 4 o |
| `res/` : l'issue (identifiant dans la charge) | 14 o | 59 o | 77 o | 79 + 4 o |
| `avail/` : en ligne | 25 o | 35 o | 64 o | 66 + 4 o |

Un relevé de température coûte donc **195 octets sur le courtier** (80 à l'entrée, 115 à la
sortie), plus 115 octets par abonné qui le reçoit. La V5 compare ces tailles à celles de
l'architecture actuelle (§ 14.2) : un événement SSE porte, en plus, l'identité complète du point
en clair.

Le JSON reste lisible avec n'importe quel client MQTT, et c'est un argument de poids pour
déboguer une maison. Un codage binaire (CBOR) réduirait la charge d'environ moitié. Il ne vaut
la peine que si la mesure du § 11.4 montre que les octets comptent, ce qui est peu probable à
l'échelle d'une maison.

### 7.4 La méta : ce qui ne voyage qu'une fois

Bloc technique — la spécification des points d'un appareil, publiée une fois et retenue sur `meta/` :

```json
{
  "nom": "Thermostat du salon",
  "sante": {"rythme_s": 600, "pertes_tolerees": 2},
  "points": {
    "temperature": {"kind": "numeric", "unit": "°C", "echelle": 2, "nature": "mesure", "fraicheur_s": 600},
    "consigne":    {"kind": "numeric", "unit": "°C", "echelle": 1, "min": 50, "max": 300, "ecrit": true},
    "mode":        {"kind": "enum", "codes": {"1": "Auto", "2": "Manuel", "3": "Absent"}, "ecrit": true}
  }
}
```

`sante` donne le rythme attendu de l'appareil et les pertes tolérées : sans nouvelle au bout de `rythme × (pertes + 1)`, ici 30 min, il est déclaré muet (§ 10.5).

La méta complète d'un point écrivable (effet, retour par code, suivi, sensibilité, propriétaire,
réveil, accès) est au § 5.1 ; celle d'un volet RTS et d'un volet io, au § 5.7.1.

Le libellé d'un code se traduit à l'affichage, dans la langue de l'écran, et ne s'écrit jamais
dans l'historique (stockage V3, « codes de bout en bout »). Ce texte est bien sûr fictif.

## 8. Le chemin d'une valeur et d'un ordre, et ce que « fait » veut dire

<!-- FIGURE:flux -->

### 8.1 Une valeur

1. Le module lit l'appareil, met la valeur au canon (`21,57 °C` → `2157`), la compare à la
   précédente et, si elle a changé, la publie sur `in/<module>/<appareil>/<point>`.
2. Le hub la reçoit. Il la passe par le tampon d'entrée (pendant une rafale, la dernière valeur
   gagne), la valide contre la spécification du point, fixe les trois instants et la qualité, et
   l'écrit dans son journal.
3. Il la publie sur `val/<appareil>/<point>`, avec son rang `s` dans le flux ; jamais retenue.
4. Le courtier la remet à ses abonnés : le moteur de workflow, s'il regarde ce point ; la base de
   données ; le module WebSocket, qui la pousse aux écrans qui ont le droit de la voir.

Deux sauts de courtier. Le premier abonné ne reçoit la valeur qu'après le journal : **rien n'est
montré, décidé ni archivé qui ne soit déjà durable.**

### 8.2 Un ordre

1. L'écran envoie « allumer » au module WebSocket. L'api-to-mqtt de Sarah Home le met au canon et publie
   une demande sur `req/ui`, avec son identifiant dans la charge.
2. Le hub applique la garde : origine, pièce, heure. Il exécute la demande, ou la retient pour une
   approbation (l'issue le dit tout de suite). Il écrit la ligne de journal et publie l'ordre sur
   `cmd/<module>/<appareil>/<point>`, avec son échéance absolue.
3. Le module le passe par la voie de l'appareil (un seul ordre en vol), le traduit et l'envoie.
4. L'état revient par `in/`, puis par `val/`. Le module en juge sur sa lecture brute, avant la
   déduplication : si c'est l'état attendu, avant l'échéance, l'issue est « confirmé » (§ 5.6). Le hub publie l'issue sur `res/ui`, et le module WebSocket la rend à l'écran
   qui l'a demandée, d'après l'identifiant.

Cinq sauts de courtier : `req/`, `cmd/`, `in/`, `val/`, `res/`. À quelques microsecondes chacun
(§ 11.4), ils ne pèsent rien devant l'aller-retour Zigbee (des centaines de millisecondes) ni devant
l'échéance (5 s). Le coût qui compte est ailleurs : **aucun saut ne doit perdre un ordre en
silence, et aucun ne doit contourner la garde.** D'où trois règles :

- `req/`, `cmd/` et `res/` passent en QoS 1 ;
- les sessions restent propres : un ordre perdu à une reconnexion est perdu, et l'échéance en
  décide, puisqu'il n'est jamais rejoué ;
- le hub est le seul à écrire `cmd/`.

### 8.3 Ce que devient une rafale

Un pont qui se reconnecte republie tout d'un coup : 5 000 valeurs sur 700 capteurs (V3 P2). Dans
la V6, la rafale traverse trois étages :

| Étage | Ce qu'il fait de la rafale |
|---|---|
| Le module | déduplique : une valeur identique à la précédente n'est pas publiée ; un pont qui republie son état après une reconnexion ne fait donc presque rien passer |
| Le hub | le tampon d'entrée fusionne par point (la dernière valeur gagne) : 5 000 valeurs deviennent environ 700 publications sur `val/` |
| Le courtier | range dans le journal de chaque filtre ; un abonné trop lent saute en avant (§ 11.4) ; le moteur de workflow et la base de données repèrent le trou par le numéro `s` et le comblent (§ 10.3) |

Les occurrences (appui, sonnette) ne sont jamais dédupliquées ni fusionnées : chacune passe une
fois, en QoS 1, sur `evt/`.

### 8.4 Reçu n'est pas fait : la livraison des ordres

<!-- FIGURE:ordre -->

La discussion externe a raison sur l'essentiel : MQTT garantit au mieux qu'un message a été
**reçu** par le maillon suivant, jamais qu'une action a été **faite**. Le `PUBACK` d'un QoS 1 dit
que le courtier a pris le message ; il ne dit rien de la lampe.

#### 8.4.1 Ce que Sarah Home fait aujourd'hui, ligne par ligne

| Étape | Code | Ce qui est garanti |
|---|---|---|
| La garde accepte, l'ordre part avec une échéance absolue (5 s) | `hub.rs:816`, ARCHITECTURE « le chemin d'un ordre » | un ordre hors délai n'est jamais exécuté, même après une reconnexion |
| Le pilote refuse si le courtier n'est pas connecté | `moli-z2m/src/lib.rs:100-103, 130` | pas d'ordre qui attendrait dans une file locale pour partir des minutes plus tard |
| Le pilote publie `…/set` en QoS 1, non retenu | `lib.rs:238-250` | — |
| **Le pilote répond « fait » dès que `try_publish` est accepté** | `lib.rs:246-249` | seulement que la file locale de `rumqttc` (64 places) a pris l'ordre ; **rien n'a encore quitté la machine** |
| Une erreur de connexion termine le pilote, qui est relancé avec un client neuf | `lib.rs:107` | l'ordre resté dans l'ancienne file est perdu, **jamais rejoué** (règle tenue) |
| L'observateur écrit une ligne de journal par ordre | `hub.rs:357-430` | la réponse du pilote, « abandonné » ou « issue inconnue » |
| La confirmation revient comme un événement `state` | ARCHITECTURE, étape 5 | rien ne la rattache à l'ordre qui l'a demandée |

L'écart est donc précis. Sarah Home répond « fait » plus tôt qu'un QoS 1 ne le permettrait : avant même
le `PUBACK`. Trois cas réels donnent un « fait » faux :

1. **Le courtier tombe entre `try_publish` et l'envoi** : l'ordre meurt avec le client. Le journal
   dit « fait ».
2. **Zigbee2MQTT est arrêté, le courtier vivant** : le courtier accepte le `PUBLISH`. Si la
   session de Zigbee2MQTT est propre, il ne le remet à personne. Si elle est persistante, il le
   remettra au retour du pont, peut-être bien après l'échéance, ce qui est pire (§ 8.4.3). Dans les
   deux cas, le journal dit « fait ».
3. **La lampe est hors du maillage**, ou coupée au mur : Zigbee2MQTT essaie, échoue, et l'écrit
   dans son propre journal (`bridge/logging`) ; aucun état ne revient. Le journal dit « fait ».

#### 8.4.2 Trois issues au lieu d'une

La proposition sépare ce que l'on sait :

| Issue | Preuve | Ce que la famille voit | Ce qu'un workflow peut faire |
|---|---|---|---|
| **Confirmé** | l'état demandé est revenu de l'appareil avant l'échéance | l'état, normalement | continuer |
| **Envoyé, non confirmé** | le `PUBACK` du courtier est arrivé, aucun état à l'échéance | « pas de confirmation » sur la tuile, ligne de journal | brancher : réessayer une consigne, prévenir, prendre une autre voie |
| **Non remis** | pas de `PUBACK`, ou courtier déconnecté | « injoignable », comme un bus ouvert | idem, et le disjoncteur compte un échec |

« Confirmé » est la seule preuve qui compte ; « remis » est un diagnostic. Le travail se fait dans
la voie de l'appareil (§ 17.4) : l'ordre reste en vol jusqu'à l'état attendu ou jusqu'à l'échéance.

Le prix est un temps de réponse plus long, le temps d'un aller-retour Zigbee (quelques centaines de
millisecondes, estimé). Le tableau de bord n'en souffre pas : il affiche déjà l'état attendu et le
corrige par le flux SSE. Une variante plus prudente répond « envoyé » tout de suite, comme
aujourd'hui, et met à jour la ligne de journal à la confirmation. Elle prive alors les workflows
de la branche « non confirmé ».

Bloc technique — l'issue d'un ordre, et ce que la voie Zigbee attend (esquisse, pas du code livré) :

```rust
/// Ce qu'un pilote sait d'un ordre à son échéance, du plus sûr au moins sûr.
pub enum Issue {
    Confirme { apres: Duration },              // l'état demandé est revenu
    EnvoyeNonConfirme,                         // remis au courtier (PUBACK), rien n'est revenu
    NonRemis,                                  // pas de PUBACK : le disjoncteur compte un échec
    Refuse(String),                            // garde, disjoncteur ouvert, valeur invalide
}

/// Un ordre en vol vers un appareil Zigbee : un seul par appareil (la voie).
struct EnVol {
    cle: Arc<str>,          // "state", "brightness"…
    attendu: Value,         // la valeur canonique demandée, comparée à celle qui revient
    pkid: Option<u16>,      // l'identifiant MQTT, pour rattacher le PUBACK
    ordre: CommandRequest,  // porte l'échéance ; abandoned() revérifié avant l'envoi
}

// Dans la boucle du pilote (les variantes d'événement de rumqttc) :
//   Event::Outgoing(Outgoing::Publish(pkid))     → l'ordre en tête de file reçoit son pkid
//   Event::Incoming(Packet::PubAck(ack))         → « remis » pour l'ordre qui porte ack.pkid
//   Event::Incoming(Packet::Publish(p))          → set_state(...) ; si clé et valeur = attendu → Confirme
//   échéance atteinte                            → EnvoyeNonConfirme ou NonRemis
```

Une valeur qui revient « presque » égale doit compter : une luminosité demandée à 128 peut revenir
à 127. La comparaison se fait sur la valeur canonique, avec la tolérance déclarée du point (la
« bande » du stockage V3). Un point qui ne renvoie jamais d'état est déclaré **muet** dans son
profil : pour lui, la remise au courtier suffit.

#### 8.4.3 Doublons, idempotence et ordres différés

| Risque soulevé | Dans Sarah Home | Reste à faire |
|---|---|---|
| QoS 1 peut livrer deux fois | les doublons naissent d'une retransmission après reconnexion d'une session persistante. Sarah Home ouvre une **session propre** (`clean_session` vrai, la valeur par défaut de `rumqttc`) et recrée son client à chaque erreur : pas de retransmission | garder ce réglage ; l'écrire dans le contrat (§ 20.8) |
| Un ordre en double fait deux fois l'action | une consigne (`ON`, 128) posée deux fois donne le même état. Sarah Home envoie des états explicites, pas des bascules (`TOGGLE`). Les **opérations** (appuis, compteurs) ont déjà leur clé d'idempotence `request_id` dans le hub | ne jamais traduire un ordre en `TOGGLE` ; réessais réservés aux consignes (V4) |
| Le courtier garde un ordre et le livre des heures plus tard (le portail) | Sarah Home ne dépose rien dans une file du courtier : session propre, ordre refusé si déconnecté, échéance de 5 s | vérifier que Zigbee2MQTT se connecte en session propre sur l'installation (le courtier le montre à la connexion) ; en MQTT 5, poser une expiration de message égale au temps restant de l'échéance |
| Un `…/set` **retenu** rejoué à chaque redémarrage de Zigbee2MQTT | ignoré en silence : `is_request` écarte les `…/set` (`lib.rs:156, 205-210`) | **le signaler** : un `…/set` reçu avec le drapeau `retain` est un danger (une lampe rallumée à chaque redémarrage du pont). Sarah Home le journalise, prévient, et l'efface (publication vide retenue), § 19.1.3 |

Bloc technique — en MQTT 5, l'échéance de Sarah Home devient l'expiration du message (esquisse, `rumqttc::v5`) :

```rust
// Le temps restant de l'échéance, arrondi par défaut : un courtier ne peut pas livrer plus tard.
let reste = ordre.deadline.saturating_duration_since(Instant::now());
let mut props = PublishProperties::default();
props.message_expiry_interval = Some(reste.as_secs() as u32);   // 0 s : livré tout de suite ou jamais
client.try_publish_with_properties(sujet, QoS::AtLeastOnce, false, charge, props)?;
```

#### 8.4.4 QoS, message par message

| Message | Aujourd'hui | Recommandé | Pourquoi |
|---|---|---|---|
| Relevé (température, puissance) | publié par Zigbee2MQTT (QoS 0 par défaut) ; Sarah Home s'abonne à `…/#` en QoS 1 | inchangé | la QoS effective est la plus petite des deux : QoS 0. Un relevé perdu est remplacé par le suivant |
| Ordre `…/set` | QoS 1, non retenu | inchangé, plus la confirmation (§ 8.4.2) | le `PUBACK` sert au diagnostic, l'état à la preuve |
| Lecture `…/get` au démarrage | QoS 0, 2 par seconde (`lib.rs:122-128, 198`) | inchangé | une lecture perdue se refait ; le rythme protège le maillage |
| Disponibilité `…/availability` | retenue par Zigbee2MQTT | inchangé | c'est l'état qu'on veut retrouver à la connexion |
| `bridge/devices` | retenu | inchangé ; surveiller sa taille (§ 14.2.3) | le catalogue entier à chaque changement |
| Appui (`action`) | QoS du pont ; retenu ignoré (`keypad.rs`) | QoS 1 côté Zigbee2MQTT pour les appuis qui comptent (sonnette, alarme) | un appui ne se rattrape pas ; la clé d'idempotence évite le double |
| Frigate (événements), Bambu, Landroid | abonnements QoS 0, ordres QoS 1 (Frigate, Landroid) ou QoS 0 (Bambu, imposé par l'imprimante) | inchangé | la confirmation par l'état vaut pour eux aussi |

## 9. Le pattern et l'isolation : une panne reste là où elle naît

<!-- FIGURE:pattern -->

**Toute décision se prend sur l'état en mémoire**, sans attendre ni un disque ni un appareil.
**Chaque frontière a sa cloison** et sa règle quand ça déborde : le thread d'un pilote, l'appareil,
le bus, l'entrée du hub. **Le disque ne sert qu'à ne rien perdre** : un journal en ajout, des
instantanés, et les étages du stockage.

### 9.1 Les sept couches, et ce que chacune a prouvé

| Couche | Ce qu'elle garantit | Coût mesuré | Preuve |
|---|---|---|---|
| 1. Un thread et un runtime par pilote | un pilote qui bloque ne bloque que lui | **19 Kio par pilote** (Linux musl) ; ordre aller-retour 0,03 ms | V2 C1, V3, V4 L5 |
| 2. Voies paresseuses par appareil | un appareil muet ne retient que ses ordres | **6 Kio au repos** pour toute la maison ; < 0,6 Mio en charge | V4 V1-V3 |
| 3. Disjoncteurs par appareil et par bus, à hystérésis, avec budget de latence | un bus mort répond en 0,1 ms, un bus lent ne fait plus attendre, un bus qui bat ne fait pas osciller | quelques octets | V3 P1, V4 Q1-Q3 |
| 4. Tampon d'entrée du hub | une inondation à la reconnexion ne fait plus perdre de déclencheurs | quelques Kio ; ≤ 15 ms pendant une rafale seulement | V3 P2, V4 T1 |
| 5. Noyau vif en mémoire | aucune décision n'attend le disque | **3,7 Mio réels** pour les deux maisons, 8,1 Mio à ×3 | V5 (banc `etat_vif`) |
| 6. Moteur compilé et durable, corrigé | 2 à 18 ns par événement ; plus aucune transition perdue sur un retard ; baux repris | ~520 o par automatisme (9,4 Kio aujourd'hui) | V2 I1, V4 F1 |
| 7. Durabilité par journal, instantanés et époque | reprise sans perte, ici ou sur la secours, jamais deux maîtres | ~2,6 Mo écrits par jour | V3 B2, V4 S1-S3 |

### 9.2 Le détail des couches

1. **Un thread et un runtime par pilote.** Le superviseur actuel garde son rôle, mais chaque
   pilote tourne sur un thread dédié portant son propre runtime Tokio. Ce qui reste coincé sur ce
   thread n'y retient que le pilote : un appel bloquant, un décodage coûteux (le XML de la V1,
   E8) ou une boucle sans attente. Le hub reçoit à sa construction le `Handle` du runtime du cœur,
   et y lance ses propres tâches (horloges, durées, observateurs d'ordres). Sous Linux à deux
   cœurs, ce n'est pas un luxe : deux pilotes bloquants partagés figent toute la maison (V4 :
   médiane 202 ms, une minuterie de 10 ms part après 405 ms).
2. **Voies paresseuses par appareil.**
   - Un seul répartiteur par pilote ; une file n'existe que pour un appareil qui a un ordre en
     attente, et disparaît vide.
   - Un seul ordre en vol par appareil ; l'échéance est revérifiée juste avant l'envoi.
   - Une nouvelle consigne remplace celle qui attend encore. Un appui n'est jamais remplacé.
3. **Disjoncteurs** (§ 17.2.3).
   - Par appareil : après deux échecs, refus immédiat et appareil hors ligne.
   - Par bus : ouverture sur **trois échecs consécutifs d'ordres** ou sur le signal du transport
     (déconnexion MQTT, pont injoignable) ; ouvert 5 s au moins ; trois succès consécutifs, ou la
     reconnexion du transport (ConnAck), pour refermer.
   - Un budget de latence : au-delà de 1,5 s de médiane, un ordre qui ferait la queue au-delà de
     son échéance est refusé tout de suite.
   - Des réessais à délai adaptatif, pour les seules consignes idempotentes.
   - Derrière une API partagée (pont, cloud), une **passerelle** plafonne la concurrence et
     répartit les places entre appareils (V2 C3).
4. **Tampon d'entrée du hub.** Quand un pilote publie plus vite que son budget, les états et les
   mesures sont fusionnés par point, la dernière valeur gagne, vidés en un lot. Les
   **occurrences** (appuis, sonnette, expiration de minuteur) ne sont jamais fusionnées. La
   minuterie n'est armée que si le tampon n'est pas vide : rien au repos.
5. **Noyau vif en mémoire** (§ 11). Toute l'information utile à une décision vit en mémoire :
   - les valeurs, la précédente, trois instants (observé, reçu, changé) et la qualité ;
   - la santé de chaque appareil et de chaque bus ;
   - les automatismes compilés, leurs index, les baux, les exécutions en cours ;
   - les agrégats tenus au fil de l'eau, et l'historique récent selon le profil.
6. **Moteur compilé et durable.**
   - Les plans sont compilés à la validation et les déclencheurs indexés par point ; l'horloge
     fonctionne par échéance.
   - Les agrégats sont incrémentaux ; les exécutions et les baux sont persistés aux points
     d'attente.
   - Le moteur ne reçoit que les événements des points qu'il regarde.
   - Sur un retard du bus d'événements, il **rejoue les différences** au lieu d'écraser ce qu'il
     savait : c'est le correctif de la § 17.3.
7. **Durabilité découplée.**
   - Un journal compact en ajout, validé par petits lots, et un instantané binaire périodique.
   - L'époque est écrite sur disque avant toute activation.
   - Les étages du stockage V3 (§ 12.1) gardent l'état courant, le récent et le long terme.
   - Le journal et les instantanés partent aussi vers la machine de secours (§ 18.1).

S'ajoute une **surveillance** transversale :

- un détecteur de gel du runtime ;
- la vivacité des pilotes ;
- la sentinelle des briques (V1) ;
- la santé par bus, affichée à la famille.

### 9.3 Pourquoi ce pattern plutôt qu'un autre

| Pattern | Ce qu'il protège | Coût mesuré ou estimé | Verdict |
|---|---|---|---|
| Tout sur un runtime partagé (aujourd'hui) | les paniques des pilotes seulement | rien | insuffisant : deux blocages gèlent tout (V2 C1, V4 L5) |
| **Un thread par pilote** (recommandé) | les blocages et les calculs longs | **19 Kio par pilote** sous Linux musl, 52 Kio sous macOS | protège l'essentiel pour presque rien |
| Un processus par pilote | aussi le débordement de pile, la mémoire, `abort` | **577 Kio par processus minimal** sous Linux musl, 30 fois un thread, plus la sérialisation de chaque état et de chaque ordre (estimé) | à réserver à un ou deux pilotes à risque, jamais à tous sur un Pi |
| Bac à sable WASM par extension | aussi le code non relu | plusieurs Mio par instance et un calcul plus lent (estimé) ; écarté par D2 | seulement si des extensions non relues doivent tourner |
| Cadriciel d'acteurs (actix, ractor…) | comme les voies | une dépendance de plus, pas d'avantage mesurable sur des tâches Tokio et des files | inutile |
| Un courtier MQTT au centre de Sarah Home (tout passe par MQTT) | pour la réactivité, rien de plus : le hub diffuse déjà en mémoire | une sérialisation et un routage par événement | pas nécessaire à la réactivité ; retenu par la [V6](isolation-reactivite-runtime-v6.md) pour d'autres raisons : un contrat unique pour tous les modules, l'isolation par processus quand il le faut, l'archivage branché sur le bus |

### 9.4 L'isolation : un module qui plante ne plante pas les autres

<!-- FIGURE:isolation -->

#### 9.4.1 Ce que chaque cloison arrête

« Planter » prend plusieurs formes (V1). Chaque forme a sa cloison :

| Défaillance d'un module | Arrêtée par un thread et son runtime | Arrêtée par un processus à part | Arrêtée par le courtier | Arrêtée par le hub |
|---|---|---|---|---|
| Panique | **oui** : `catch_unwind` du superviseur (`supervisor.rs:38`), `panic = "unwind"` voulu (`Cargo.toml`) | oui | — | — |
| Appel bloquant, boucle sans attente | **oui** : il ne bloque que son thread (V2 C1, V4) | oui | — | — |
| Calcul long (décodage hostile) | **oui** pour les autres ; le module, lui, reste lent (V1 E8) | oui | — | — |
| `abort`, débordement de pile, bibliothèque C qui écrit hors de sa mémoire | **non** : tout le processus tombe | **oui** | — | — |
| Fuite de mémoire | **non** : elle mange le budget de tous | **oui**, avec une limite de mémoire par processus | — | — |
| Inondation de publications | en partie (son thread) | en partie | **oui** : 4 Kio par message, file bornée par client | **oui** : tampon d'entrée, budget par module (§ 20.5) |
| Lenteur ou réponses partielles de l'appareil | voies, disjoncteur, budget de latence (V4) | idem | — | — |
| Écrire là où il ne doit pas | sans objet : module de confiance ; le kit forme ses sujets (§ 5.3) | sans objet : module de confiance (§ 6.2) | **ACL** pour les clients extérieurs | seul le hub écrit `cmd/` et `val/` |

Le thread suffit pour presque tout, pour 19 Kio par module (V3, Linux musl). Le processus ne se
justifie que pour ce que le thread ne peut pas arrêter : un `abort`, une pile qui déborde, une
mémoire qui fuit ou se corrompt.

#### 9.4.2 Quels modules en processus à part

| Critère | Exemples | Décision |
|---|---|---|
| Code tiers non relu, ou lié à une bibliothèque C (FFI) | un SDK de fabricant, un décodeur vidéo | **processus à part** |
| Protocole rétro-conçu, entrées très libres | certains clouds, une API qui change sans prévenir | processus à part si le module a déjà planté en vrai, thread sinon |
| Gros consommateur de mémoire | une analyse d'images, un modèle local | processus à part, avec sa limite de mémoire |
| Tout le reste | les pilotes actuels, en Rust, relus, bornés | **thread** |

Un processus minimal coûte 577 Kio sous Linux musl (V3), trente fois un thread. Avec MQTT au
centre, ce coût est le seul, **à une condition** : le contrat MQTT ne change pas, mais le module parle au
canal de services (coffre, attente d'un humain, images, saisies privées) par une socket locale
authentifiée, jamais par le courtier ; et un module qui monte une passerelle HTTP partagée avec
l'API (Cast, Android TV, téléphones, ordinateurs, Netatmo) reste dans le processus du hub (§ 5.1,
crible G4). Le même module, compilé dans le
binaire, tourne aussi dans un petit exécutable qui se connecte à l'écouteur local avec son compte. Sarah Home le lance, le surveille et le relance comme un thread : même attente croissante,
mêmes appareils hors ligne pendant la panne.

Bloc technique — un module lancé en processus à part par le superviseur (esquisse) :

```toml
# moli.toml : le même module, déplacé hors du binaire sans changer son code.
[[module]]
id = "cloud-x"
kind = "cloud-x"
isolation = "processus"            # "thread" par défaut
memoire_max = "48MiB"              # limite du processus (cgroup ou RLIMIT_AS)
redemarrage = "croissant"          # 1 s, 2 s, 4 s… 5 min, comme un pilote aujourd'hui
# Le superviseur lance : moli-os module run cloud-x --courtier 127.0.0.1:8884 --certificat …
```

#### 9.4.3 Et si c'est le courtier qui plante ?

Le courtier embarqué n'est pas un module : s'il tombe, plus rien ne circule. `rumqttd` fait tourner
son routeur sur un thread à lui. Trois protections :

1. **Rien ne lui est confié d'irremplaçable** (§ 6.3). Le relancer, c'est repartir à vide ;
   le hub republie ses retenus, et chaque client se reconnecte.
2. **Une panique du routeur fait redémarrer Sarah Home** : une sentinelle surveille le thread du
   routeur, et à sa mort le processus sort pour être relancé (systemd, Docker). La reprise à chaud
   prend quelques millisecondes pour le hub (V4 : 4,3 ms pour 7 jours d'historique en mémoire),
   plus les reconnexions des modules.
3. **Le moteur de haute disponibilité** prend le relais si le redémarrage échoue (§ 18.2).

Le risque est réel : `rumqttd` devient le composant le plus critique de Sarah Home, et il n'est pas
écrit par l'équipe. C'est un argument de plus pour l'option C du § 6.1 à terme : un moteur à sujets
propre à Sarah Home, dont on connaît chaque branche.

## 10. Le hub : la vérité, la garde, et rien d'autre

### 10.1 Le seul écrivain

| Le hub… | Comment | Pourquoi lui seul |
|---|---|---|
| valide chaque valeur | contre la spécification du point (type, plage, longueur, unité) | une valeur hors spécification ne doit atteindre ni un automatisme ni l'historique |
| fixe les trois instants et la qualité | observé (du module), reçu (`$vu`), changé ; bonne, incertaine, mauvaise | une seule horloge de référence pour toute la maison |
| écrit le journal | une ligne par changement, par lots de 100 ms au plus (V4) | la reprise et la secours partent de là |
| publie `val/`, `avail/`, `meta/` | jamais retenus ; `s` est le rang sans trou dans le flux `val/` | « Sarah Home tient la vérité, le courtier diffuse » |
| sert l'état courant | en mémoire aux modules du binaire ; par blocs de 4 Kio à qui le demande | un abonné qui arrive commence par l'état, puis suit le flux |
| applique la garde | origine, pièce, heure, approbation ; PIN pour ce qui l'exige | ARCHITECTURE, règles 2 et 9 |
| publie `cmd/` et `res/` | l'ordre avec son échéance absolue, puis son issue | personne d'autre ne commande un appareil |

### 10.2 L'ingestion

Bloc technique — la boucle du hub sur son lien local (esquisse) :

```rust
loop {
    match rx.next().await? {
        // Une valeur d'un module : tampon d'entrée (dernière valeur gagne pendant une rafale).
        Some(Publication { sujet, charge }) if sujet.starts_with("moli/v1/in/") => {
            let (module, appareil, point) = decouper(&sujet)?;    // le kit (ou le compte, hors du binaire) dit quel module publie
            let lue: Lue = decoder_borne(&charge, 4096)?;         // entrée hostile, bornée
            tampon.deposer(appareil, point, lue);                 // vidé toutes les 10 ms en rafale
        }
        // Une demande d'ordre : la garde, puis l'ordre.
        Some(Publication { sujet, charge }) if sujet.starts_with("moli/v1/req/") => {
            let demande: Demande = decoder_borne(&charge, 4096)?;
            match garde.juger(&demande) {
                Verdict::Execute(ordre) => { journal.ecrire(&ordre); tx.publish(ordre.sujet(), ordre.charge())?; }
                Verdict::Retenu(id) => tx.publish(demande.sujet_issue(), Issue::Retenue(id).charge())?,
                Verdict::Refuse(raison) => tx.publish(demande.sujet_issue(), Issue::Refusee(raison).charge())?,
            }
        }
        _ => {}
    }
    // Ce que le tampon rend : validé, horodaté, journalisé, puis publié sur val/ (retenu).
    for v in tampon.pret() { let v = etat.appliquer(v)?; journal.ecrire(&v); tx.publish(v.sujet(), v.charge())?; }
}
```

### 10.3 Les trous, et comment les combler

Un abonné qui lit trop lentement peut perdre des messages : le courtier ne garde qu'un nombre
borné de segments par filtre (§ 6.4 et § 11.4). Le numéro `s` rend la perte **visible** : dans le
flux `val/`, il n'a jamais de trou. Un saut de `s` dit exactement combien de valeurs ont été
perdues.

| Abonné | Ce qu'il fait d'un saut de `s` |
|---|---|
| Moteur de workflow (dans le binaire) | demande au hub la différence depuis le dernier `s` vu, et **rejoue chaque changement comme une transition** : c'est le correctif de la V4 (§ 9.4 de la V5), généralisé |
| Base de données (dans le binaire) | relit le tronçon manquant dans le journal du hub, par `s` ; aucune valeur ne manque dans l'historique |
| Module WebSocket | renvoie un instantané à l'écran concerné, comme le flux SSE aujourd'hui (`sse.rs:128`) |
| Secours (autre machine) | demande le tronçon manquant par le canal de réplication (§ 18.2) |

Le hub garde pour cela, en mémoire, la queue récente de son flux (quelques milliers de
publications). Au-delà, l'abonné repart d'un instantané que le hub lui donne directement, puis du
flux. Il ne repart pas des messages retenus : `rumqttd` 0.20 n'en livre que 100 à 200 à un nouvel
abonné, sans le dire (§ 6.1, § 11.4).

Le banc l'a montré : **le courtier n'avertit jamais un abonné qu'il a sauté des messages**. Le rang
`s` est donc le seul moyen de le savoir, et il doit exister dans les deux sens :

- sur `val/`, le rang `s` du hub, sans trou ;
- sur `in/`, un rang `n` par module, sans trou lui aussi. Si le hub voit un saut, il demande au
  module de republier tout son état (`cmd/<module>/$etat`). Le module garde toujours ses dernières
  valeurs, et la demande ne coûte qu'une rafale de quelques centaines de messages.

Le filtre `in/#` a en plus un journal plus long dans le courtier (§ 6.4) : le hub ne devrait jamais
sauter, mais s'il saute, il le sait et il répare.

### 10.4 L'état courant : le hub le sert, le courtier ne le garde pas

Aucun message n'est retenu sur le courtier (§ 6.3). Un abonné qui arrive (le module WebSocket, le
moteur de workflow au démarrage, un outil, la secours) commence donc par **demander l'état au hub** :

- dans le binaire, par un appel en mémoire : `hub.etat()` rend un instantané en colonnes compactes,
  sans copie de l'état entier (§ 11.4) ;
- hors du binaire, par une demande sur `moli/v1/etat/demande`, avec l'identifiant de l'abonné dans
  la charge. Le hub répond par blocs de 4 Kio au plus sur `moli/v1/etat/<abonné>` : environ 50 blocs
  pour les deux maisons. Chaque bloc porte le rang `s` auquel l'état correspond ;
- ensuite, l'abonné suit le flux `val/` à partir de ce rang `s`, sans trou ni doublon.

Au démarrage de Sarah Home, au redémarrage du courtier ou à l'activation de la secours, il n'y a donc
**rien à republier** : le courtier repart vide, et les abonnés redemandent l'état.

### 10.5 La santé de chaque appareil : vivant, à jour, ou muet

Pour chaque appareil physique, le hub garde en mémoire trois informations, qui ne se confondent
pas :

| Information | Ce qu'elle dit | D'où elle vient |
|---|---|---|
| **Dernier contact** | la dernière fois que l'appareil a donné signe de vie : une valeur, même identique, un `$vu`, une réponse à un ordre | chaque message de son module |
| **Dernière mise à jour d'une valeur** | la dernière fois qu'une de ses valeurs a changé ; par point, l'instant « changé » | les publications sur `in/` qui changent quelque chose |
| **Rythme attendu** | le délai sous lequel l'appareil doit normalement redonner des nouvelles | déclaré par le module ou le profil (le compteur Linky toutes les 3 min, un contact sur pile toutes les 24 h, un thermomètre Zigbee toutes les 10 min), sinon appris (§ ci-dessous) |

Avec ces trois informations, le hub sait distinguer **un capteur calme d'un capteur mort**, ce que
la discussion externe demandait sans le résoudre (« ne pas basculer parce qu'un capteur sur pile
n'a pas communiqué »). Un thermomètre dont la température ne change pas depuis deux heures, mais
qui a donné signe de vie il y a 4 minutes, est vivant. Un contact de porte silencieux depuis
30 heures, alors qu'il doit se manifester toutes les 24 heures, est muet.

Chaque appareil déclare son **rythme** et le nombre de **pertes tolérées**. Le hub en tire un
**délai maximal sans nouvelle** :

```text
délai maximal = rythme × (pertes tolérées + 1)

API interrogée toutes les 3 min, une perte tolérée   → 3 min × 2 = 6 min
thermomètre Zigbee, rapport toutes les 10 min, 2 pertes → 30 min
contact de porte sur pile, rapport toutes les 24 h, 0 perte → 24 h (+ une marge fixe d'horloge)
```

Pour une API interrogée, le module connaît sa période de sondage : il la déclare lui-même. Une
réponse réussie ne compte comme une nouvelle que si la source dit du neuf : un horodatage source
plus récent. Une API qui rend la dernière mesure (Netatmo, iopool, Tempo, Open-Meteo) ne rafraîchit
rien en répétant la même date. Le `$vu` porte la liste des points qu'il rafraîchit quand un appareil
a plusieurs sources ou rythmes. Une interrogation en échec ou sans réponse ne compte pas ; une pause
imposée par la source (`pause_jusqu_a`) suspend l'échéance ; un appareil en `veille` (télé, ampoule
coupée au mur, capteur sur pile) n'est jamais déclaré muet, et pour la présence le silence est le
signal lui-même (§ 5.1, crible G5 et G6).

L'échéancier unique du hub (V3, stockage V3) tient une échéance par appareil :
`dernier contact + délai maximal`. Tant qu'elle n'est pas passée, rien ne bouge : une perte
tolérée est normale, elle ne doit ni alerter ni dégrader une valeur. Quand elle passe :

1. l'appareil passe **hors ligne** sur `avail/`, avec sa raison (`"aucune nouvelle depuis 6 min"`) ;
2. ses valeurs passent en qualité **mauvaise** sur `val/` : un automatisme ne déclenche jamais sur
   elles, et une condition qui les lit vaut « inconnu » ;
3. la famille le voit sur l'écran (« muet depuis 6 min »), et une notice part si l'appareil est
   marqué important (un détecteur de fumée, une serrure, un compteur).

La première nouvelle qui arrive ensuite remet tout d'aplomb : en ligne, qualité bonne, échéance
relancée. Un point peut en plus exiger sa propre fraîcheur pour une règle (« une température de
moins de 5 min ») : c'est la fraîcheur par point du stockage V3, qui vient s'ajouter au délai de
l'appareil sans le remplacer.

Bloc technique — ce que le hub garde par appareil physique (esquisse, ~80 octets par appareil) :

```rust
pub struct SanteAppareil {
    pub dernier_contact: Instant,          // toute nouvelle : valeur, même identique, $vu, réponse à un ordre
    pub dernier_changement: Instant,       // la dernière valeur qui a changé (les points gardent le leur)
    pub rythme: Option<Duration>,          // déclaré par le module ou le profil ; None : n'émet qu'à l'action
    pub pertes_tolerees: u8,               // 1 : une interrogation ou un rapport peut manquer
    pub rythme_appris: Option<Duration>,   // médiane des intervalles observés, proposée à l'humain
    pub etat: Vie,                         // Vivant, Muet (délai dépassé), HorsLigne (module ou bus en panne)
    pub echeance: Option<Instant>,         // dernier_contact + rythme × (pertes_tolerees + 1)
}
```

Bloc technique — la déclaration, dans un profil ou par un module :

```toml
[profile]
id = "chaudiere-api"
transport = "http"

[sante]
rythme = "3min"            # la période d'interrogation
pertes_tolerees = 1        # une réponse peut manquer : hors ligne au bout de 6 min sans nouvelle
important = true           # une notice à la famille quand il devient muet
```

Quand le rythme n'est déclaré nulle part, le hub l'**apprend** : la médiane des intervalles entre
contacts sur les derniers jours. Il ne l'impose pas : il le propose (« ce capteur parle toutes les
55 min environ ; le surveiller à 2 h ? »), comme les automatismes que Sarah propose. Un appareil qui
ne parle que lorsqu'on agit sur lui (un interrupteur, un bouton) n'a pas de rythme : il n'est
jamais déclaré muet, seulement hors ligne quand son module ou son bus tombe.

Ces informations coûtent environ 80 octets par appareil, soit 120 Kio pour trois fois les deux
maisons (1 488 appareils). La V3 comptait 25 octets par appareil pour la santé seule : la différence
est le rythme et l'échéance.

## 11. La mémoire : ce que la maison compte, ce que porte une valeur, ce que tout coûte

Toute la maison vit en mémoire : chaque valeur avec ses instants, la santé de chaque appareil, les
automatismes compilés, leurs baux et leurs exécutions. Cette section le montre en quatre schémas,
du nombre d'objets jusqu'au total en Mio. Tous les chiffres sont **mesurés** par le banc mémoire
(Linux musl, conteneur de 2 CPU et 128 Mio, un processus neuf par mesure ; annexe A), sauf mention
contraire.

### 11.1 Ce que la maison compte

<!-- FIGURE:compte -->

| Ce qui est compté | ×1 : les deux maisons | ×3 | ×10 | D'où vient le nombre |
|---|---|---|---|---|
| Appareils | **496** | 1 488 | 4 960 | comptes agrégés des deux Home Assistant |
| Valeurs, ou attributs (les « points ») | **2 264**, ≈ 4,6 par appareil | 6 792 | 22 640 | idem |
| … dont mesures, binaires, énumérations, textes courts, sans valeur | 50 %, 25 %, 15 %, 5 %, 5 % | | | forme du banc mémoire |
| Relevés par jour | **≈ 163 000** : 1,9 par seconde, 239 à la pointe ; 109 par point en moyenne | 489 000 | 1,63 million | séries du stockage V3, vingtiles de la V4 |
| Automatismes | **400** | 1 200 | 4 000 | hypothèse de tous les bancs depuis la V3 |
| Nœuds de workflow | **≈ 2 400** : 6 par automatisme (`when_state`, `if`, `set`, `wait`, `set`, `notify`), 5 liens, 3 règles | 7 200 | 24 000 | automatisme type de la maquette des règles |
| Points lus par une règle | **453**, une valeur sur cinq | 1 359 | 4 530 | banc mémoire |
| Baux et attentes durables (« pendant 30 min », « attendre que… ») | **1 000** | 3 000 | 10 000 | banc mémoire |
| Exécutions en cours, au pire | 400, une par automatisme | 1 200 | 4 000 | banc mémoire |
| Entrées de la queue récente de `val/` | 4 096 | 12 288 | 40 960 | banc mémoire |

Un automatisme type pèse 1 972 octets de JSON, soit ≈ 330 o par nœud : c'est sa définition, telle
qu'on l'édite. Elle reste **sur disque**. En mémoire, il n'y a que son plan compilé : **≈ 315 o par
automatisme, ≈ 52 o par nœud** (§ 13.2). Les 2 400 nœuds des deux maisons tiennent donc en
0,12 Mio, index compris.

### 11.2 Ce que porte une valeur, octet par octet

Une valeur ne porte pas qu'un nombre. Pour qu'une condition se lise sans disque, qu'un « depuis
15 min » se calcule, qu'un profil ne restaure pas une valeur modifiée depuis, et qu'un appareil
muet se voie, chaque point garde **six informations** à côté de sa valeur, et chaque appareil garde
sa santé.

<!-- FIGURE:valeur -->

| Champ | Octets | Ce qu'il dit | À quoi il sert |
|---|---|---|---|
| **valeur** | 8 | la valeur au canon : entier mis à l'échelle (21,57 °C → `2157`), booléen, code d'énumération, ou rang d'un texte court | conditions, écrans, historique |
| **précédente** | 8 | la valeur d'avant | « passe de fermé à ouvert » : une transition, pas un état |
| **observé** | 8 (ms) | l'instant de la mesure, selon la source | l'ordre réel des événements, l'historique |
| **changé** | 8 (ms) | le **dernier changement** de valeur | « depuis 15 min » ; la restauration d'un profil (« le lampadaire gardé, modifié à 16 h 05 ») |
| **reçu** | 4 (s) | le **dernier rapport**, même identique, tenu par `$vu` (§ 5.2) | la fraîcheur : une valeur à jour, ou une vieille valeur |
| **genres** | 1 | le genre de la valeur et de la précédente : entier, booléen, code, texte | lire `valeur` sans ambiguïté |
| **qualité** | 1 | bonne, incertaine, mauvaise | **un automatisme ne déclenche jamais sur une valeur mauvaise** |
| *alignement* | 2 | — | — |
| **Total : la valeur vive d'un point** | **40** | | |

Et pour chaque appareil, 20 octets de santé (§ 10.5) :

| Champ | Octets | Ce qu'il dit |
|---|---|---|
| **contact** | 4 (s) | le **dernier signe de vie** de l'appareil, même sans rien de neuf |
| **changé** | 4 (s) | le dernier changement d'un de ses points : « rien n'a bougé depuis trois jours » |
| **rythme** | 4 (s) | le rythme attendu : 180 s pour un appareil interrogé toutes les 3 min ; 0 pour un appareil qui n'émet qu'à l'action (un volet RTS, une télécommande) |
| **appris** | 4 (s) | le rythme observé, en moyenne glissante, proposé à l'humain quand il s'écarte du rythme déclaré |
| **pertes** | 1 | les pertes tolérées : une, pour l'appareil interrogé toutes les 3 min |
| **vie** | 1 | vivant, en retard, muet |

L'échéance n'est pas stockée : elle se calcule au passage de l'échéancier, une fois par seconde,
`contact + rythme × (pertes + 1)`. Pour l'appareil interrogé toutes les 3 min avec une perte
tolérée : **muet à 6 min**, comme demandé.

Ce qui ne se répète plus :

- **les noms** (`z2m:0x00158d00…/temperature`, ≈ 33 caractères) existent aujourd'hui en une dizaine
  de copies (bus, journal, miroirs, queue, retenus). Ils sont **internés** une fois, et partout
  ailleurs un point est un rang de 4 octets ;
- **l'échelle, l'unité et le dictionnaire** d'une énumération sont dans la définition (16 o) et
  sur `meta/`, jamais dans la valeur ;
- **un texte court** est rangé une fois dans une table, et la valeur porte son rang.

Une variante à 32 octets existe (l'instant « observé » devient un écart à « changé », saturé à
± 32 s) ; elle gagne 8 octets par point au prix d'un instant approché. La V7 garde 40 octets :
2 264 points × 8 o, c'est 18 Kio.

**Au total, un point coûte 96 octets au lieu de 719, et un appareil avec ses 4 à 5 valeurs, sa
santé et son identité environ 0,5 Kio.** Une condition se lit en 0,7 à 0,9 ns, au lieu de 40 à
170 ns dans les structures d'aujourd'hui.

### 11.3 La mémoire par type de donnée, et le total

<!-- FIGURE:types -->

Le schéma ci-dessus est par objet ; celui-ci est le total, poste par poste. Rouge : la conception
écrite simplement, avec les types d'aujourd'hui. Vert : la même conception, optimisée.

<!-- FIGURE:postes -->

| Famille de données | Aujourd'hui ×1 / ×3 / ×10 (Mio) | **Optimisé ×1 / ×3 / ×10 (Mio)** | Ce qui fait la différence |
|---|---|---|---|
| Socle : runtime, traductions, hub vide | 1,26 / 1,26 / 1,26 | **1,26 / 1,26 / 1,26** | rien : fixe (la seule langue de la maison en gagnerait ~0,5) |
| Le hub : définitions, valeurs vives, santé | 1,64 / 4,71 / 15,80 | **0,27 / 0,65 / 2,14** | colonnes, noms internés, valeur au canon `i64` |
| Les copies de l'état : miroirs, queue, bus | 1,82 / 4,74 / 16,35 | **0,14 / 0,40 / 1,30** | 16 o par point lu pour le workflow, aucun miroir pour le WebSocket, une queue en anneau de 24 o |
| Les automatismes : définitions, plans, baux, exécutions, agrégats | 4,87 / 14,57 / 48,62 | **0,49 / 1,47 / 4,78** | les définitions sur disque, une seule copie compilée, des seaux fixes |
| Le courtier : valeurs, routeur, journaux, demandes | 13,91 / 21,38 / 54,53 | **4,30 / 8,05 / 17,51** | aucun retenu, sujets `req/` et `res/` stables, un seul lien `cmd/#` |
| **Total** | **23,5 / 46,7 / 136,6** | **6,5 / 11,8 / 27,0** | |

Trois lectures :

- **Le budget de 32 Mio tient jusqu'à ×3**, avec environ 20 Mio pour les modules, la pile HTTP,
  SQLite et les écrans (§ 11.4.2). À ×10, la V7 optimisée tient dans le conteneur de 128 Mio.
- **Le courtier est le premier poste** dès que le reste est compact : ses caches par sujet, à
  supprimer dans notre version de `rumqttd` (§ 6.1), gagneraient encore 1,4 à 12 Mio (estimé).
- **L'historique n'est pas compté ici** : il vit sur disque (§ 12). En mémoire, s'il le faut, il
  coûte 8,1 à 8,9 o par relevé et par jour (24 h : ≈ 1,4 Mio à ×1 ; 7 jours : 8,8 Mio), et c'est un
  choix de profil (§ 11.5).

### 11.4 La mémoire mesurée, poste par poste, et le prix du courtier

Deux bancs ont été joués pour la conception MQTT au centre, en Linux musl dans le conteneur limité à 2 CPU et 128 Mio :
celui du courtier (`rumqttd` 0.20.0 embarqué), et celui d'un sous-agent chargé de trouver la
représentation la plus compacte de tout ce que la V7 garde en mémoire. Celui-ci a mesuré un
processus neuf par mesure, et prototypé les structures en Rust (annexe A).

#### 11.4.1 Toute la mémoire de la V7, poste par poste

Mémoire anonyme réelle, en Mio. Il n'y a ni historique en mémoire, ni pilotes, ni SQLite, ni
connexions d'écrans (voir 17.2). « Aujourd'hui » veut dire : les types actuels de `moli-core` et de
`moli-runtime`, les esquisses de la V7 écrites simplement, et `rumqttd` réglé comme au § 6.4 avec des
retenus. « Optimisé » veut dire : les recommandations du § 11.4.3, mesurées sur le prototype.

| Poste | Aujourd'hui ×1 | ×3 | ×10 | **Optimisé ×1** | **×3** | **×10** |
|---|---|---|---|---|---|---|
| Socle : runtime, catalogues de traduction, hub vide | 1,26 | 1,26 | 1,26 | 1,26 | 1,26 | 1,26 |
| Hub : définitions, valeurs, étage vif, santé des appareils | 1,64 | 4,71 | 15,80 | 0,27 | 0,65 | 2,14 |
| Bus d'événements interne (1 024, plein) | 0,06 | 0,05 | 0,06 | 0 | 0 | 0 |
| Miroir du moteur de workflow | 0,33 | 0,78 | 2,84 | 0,04 | 0,11 | 0,35 |
| Miroir du module WebSocket | 0,48 | 1,09 | 4,09 | 0 | 0 | 0 |
| Queue récente du flux `val/` | 0,95 | 2,82 | 9,36 | 0,10 | 0,29 | 0,95 |
| Automatismes : définitions en mémoire | 3,97 | 11,90 | 39,69 | 0 (sur disque) | 0 | 0 |
| Automatismes compilés et leurs index | 0,28 | 0,84 | 2,82 | 0,12 | 0,35 | 1,11 |
| Baux et échéancier | 0,08 | 0,23 | 0,77 | 0,04 | 0,14 | 0,39 |
| Exécutions en cours | 0,09 | 0,26 | 0,87 | 0,02 | 0,05 | 0,16 |
| Agrégats 1 h / 24 h | 0,45 | 1,34 | 4,47 | 0,31 | 0,93 | 3,12 |
| Courtier : valeurs (retenus aujourd'hui, caches de sujets optimisé) | 2,73 | 9,00 | 32,61 | 0,50 | 1,41 | 4,84 |
| Courtier : routeur, liens, filtres, journaux, caches, rafale | 7,02 | 8,68 | 15,88 | 3,37 | 5,50 | 12,33 |
| Courtier : les 10 000 premières demandes d'ordre | 4,16 | 3,70 | 6,04 | 0,43 | 1,14 | 0,34 |
| **Total** | **23,5** | **46,7** | **136,6** | **6,5** | **11,8** | **27,0** |
| *puis, par tranche de 10 000 demandes de plus* | *+1,86* | *+3,39* | *+1,86* | *+0,03* | *+0,04* | *+0,01* |

Ce que le tableau dit :

- **Écrite comme aujourd'hui, la V7 ne tient pas dans le budget de 32 Mio à ×3** : 46,7 Mio pour
  ce seul périmètre. Avec des sujets de demande uniques, le courtier grossit en plus de 1,9 à
  3,4 Mio toutes les 10 000 demandes, sans jamais rendre cette mémoire.
- **Optimisée, elle tient : 11,8 Mio à ×3**, et il reste environ 20 Mio pour les modules, HTTP,
  SQLite et les écrans. Aujourd'hui, tout Sarah Home tient en 4 à 7 Mio pour une maison (PERFORMANCE.md).
- **À ×10, la version optimisée (27 Mio) tient dans le conteneur de 128 Mio**, pas dans 32 Mio avec
  le reste. Les caches par sujet du courtier deviennent alors le premier poste.
- Trois postes faisaient l'essentiel du coût :
  - le courtier : retenus de près de 1 Kio, caches par sujet, un filtre et un journal par module ;
  - les définitions d'automatismes, gardées deux fois avec leur graphe : 9,3 Kio par automatisme ;
  - les copies de l'état : miroirs et queue.

#### 11.4.2 Ce qui s'ajoute, hors de ce périmètre

| Poste | Coût | D'où il vient |
|---|---|---|
| Threads des modules | 19 Kio × modules (0,76 Mio pour 40) | V3 |
| Écrans | 11 Kio par WebSocket réglée (0,7 Mio pour 64) ; 39 Kio en SSE | § 14.2.5 |
| Clients TCP du courtier (Zigbee2MQTT, modules à part, secours) | ~62 Kio chacun, sans retenus | banc courtier |
| Historique récent en mémoire, si le profil le demande | 8,1 à 8,9 o par relevé et par jour (24 h : ~1,4 Mio à ×1) | V3, § 21 |
| Caches de SQLite et Turso, pile HTTP, pilotes | non mesurés ici | — |

#### 11.4.3 Les optimisations retenues, par gain sur effort

| # | Optimisation | Gain réel ×1 / ×3 / ×10 | Effort | Où dans la V7 |
|---|---|---|---|---|
| 1 | **Aucun message retenu** : le hub sert l'état | 2,2 / 7,6 / 27,8 Mio (mesuré) | faible | § 6.3, § 10.4 |
| 2 | **Sujets `req/` et `res/` stables**, l'identifiant dans la charge | supprime une fuite d'environ 190 o par demande (mesuré) | trivial | § 5.5, § 7.1 |
| 3 | **Automatismes : une seule copie**, puis le plan compilé et les définitions sur disque | 4,1 / 12,4 / 41,4 Mio (mesuré) | faible, puis moyen | § 13.2 |
| 4 | **Miroirs** : 16 o par point pour le moteur ; **aucun** pour le module WebSocket | 0,8 / 1,8 / 6,6 Mio (mesuré) | faible | § 13.2, § 14.1.2 |
| 5 | **Queue `val/` en anneau** d'entrées de 24 o, rang `s` implicite | 0,9 / 2,5 / 8,4 Mio (mesuré) | faible | § 10.3 |
| 6 | Courtier : **un seul lien `cmd/#`** pour les modules du binaire, journaux courts | 3,7 / 3,2 / 3,6 Mio (mesuré) | faible | § 6.4 |
| 7 | **État vif du hub en colonnes**, identités internées (`u32`), valeur au canon `i64` | 1,4 / 4,1 / 13,7 Mio (mesuré) | moyen à fort | § 11.4.4 |
| 8 | **Garder l'allocateur de musl** | évite +2,4 à +4,3 Mio (mesuré) | nul | § 11.4.6 |
| 9 | Baux, exécutions et agrégats compacts (seaux fixes) | 0,3 / 0,7 / 2,4 Mio (mesuré) | faible à moyen | — |
| 10 | Catalogues de traduction : la seule langue de la maison | ~0,5 Mio fixe (estimé) | faible | — |
| 11 | Courtier sans caches par sujet (dans notre version de `rumqttd`) | ~1,4 / 4,0 / 12 Mio (estimé) | fort | § 6.1 |

#### 11.4.4 Les types compacts

Le nom d'un point (`z2m:0x00124b00aaaa0001/temperature`) apparaît aujourd'hui dans une dizaine de
copies, près de 1 Kio par point : dans le hub, le moteur, les index, les événements, et les sujets
du courtier. Interné une fois, il coûte environ 15 octets. Un point coûte aujourd'hui **719 octets**
(353 de définition, 243 de valeur et d'étage vif, 104 d'index, 19 de santé). Dans le prototype, il en
coûte **96**.

Bloc technique — l'état vif du hub en colonnes (prototype mesuré, extraits, sans `unsafe`) :

```rust
/// Chaque nom une fois, bout à bout ; l'index d'un nom est son rang.
pub struct Noms { texte: String, fins: Vec<u32>, index: hashbrown::HashTable<u32>, hacheur: RandomState }

pub struct Def {          // 16 o : la définition d'un point
    pub appareil: u32, pub libelle: u32, pub cle: u16,
    pub dico: u16,        // dictionnaire d'énumération partagé (0 : aucun)
    pub nature: u8, pub unite: u8,
    pub echelle: i8,      // valeur réelle = canon × 10^echelle (−2 : 2157 vaut 21,57)
    pub sens: u8,         // sémantique, lecture, écriture
}

pub struct Vif {          // 40 o : la valeur vive d'un point, aucune allocation
    pub valeur: i64,      // canon : mesure mise à l'échelle, booléen 0/1, code d'énumération
    pub precedente: i64,
    pub observe_ms: u64,  // instant de la source
    pub change_ms: u64,   // dernier changement
    pub recu_s: u32,      // dernier rapport, en secondes : la fraîcheur se juge à la seconde
    pub genres: u8,       // genre de la valeur et de la précédente (4 bits chacun)
    pub qualite: u8,      // bonne, incertaine, mauvaise
}

pub struct Sante {        // 20 o : délai maximal = rythme × (pertes + 1), calculé au passage (§ 10.5)
    pub contact_s: u32, pub change_s: u32, pub rythme_s: u32, pub appris_s: u32,
    pub pertes: u8, pub vie: u8,
}

pub struct Etat {         // des colonnes, indexées par point (u32) et par appareil (u32)
    pub defs: Vec<Def>, pub vifs: Vec<Vif>, pub appareils: Vec<App>, pub sante: Vec<Sante>,
    pub ids: Noms, pub cles: Noms, pub mots: Noms,     // identités, clés, libellés, dictionnaires
    pub textes: HashMap<u32, Box<str>>,                // les seuls points texte
    index: hashbrown::HashTable<u32>,                  // (appareil, clé) → point
}

pub struct Entree { pub valeur: i64, pub change_ms: u64, pub point: u32, pub genre_qualite: u8 } // 24 o
pub struct File { entrees: Box<[Entree]>, prochain: u64 }      // la queue val/ en anneau ; s implicite
pub struct Copie { pub valeur: i64, pub change_s: u32, pub genre_qualite: u8 }                   // 16 o
```

`DeviceId`, `PointId`, `Value`, `Device` et `PointSpec` restent la forme des surfaces, du journal et
des fichiers : le hub convertit à l'entrée. Le seul ajout à `moli-core` est l'échelle du canon par
point, un champ facultatif de `PointSpec`, que la méta de la V7 porte déjà (§ 7.4).

#### 11.4.5 Le processeur

| Opération | Prototype ou V7 | Aujourd'hui | Source |
|---|---|---|---|
| Lire une condition | **0,7 à 0,9 ns** (par index) ; 3,6 ns derrière un verrou | 40 à 52 ns (`HashMap<String, Value>` du moteur) ; 92 à 170 ns (`hub.state()`, qui alloue un `DeviceId` à chaque lecture) | banc mémoire, Linux ×1 et ×10 |
| Entrer une mesure (valeur, santé, queue) | **3,3 à 4,2 ns** | 62 à 74 ns (`insert` avec `format!`) | banc mémoire |
| Résoudre un sujet `val/` en point | 68 à 78 ns (trois hachages) | 47 à 56 ns | banc mémoire ; un module du binaire n'en a pas besoin, le kit lui donne l'index |
| Passer par le courtier (lien en mémoire) | **26 µs** de latence médiane, **3,3 µs** de CPU du routeur à 50 000 messages/s (10 µs à 1 000/s) ; 50 000 messages/s sans perte | le bus actuel : ~10 µs de latence médiane | banc courtier |
| Livrer à un client TCP | 15 à 27 µs par livraison | — | banc courtier |
| Vérifier une ACL (client extérieur) | 30 à 44 ns (1 à 10 motifs), 91 à 114 ns (50 motifs) : au plus 3,5 % du CPU du routeur | — | banc courtier |

Au rythme réel des deux maisons, une valeur fait deux sauts (`in/`, `val/`) et quelques livraisons
en mémoire. À la pointe de 239 valeurs par seconde, cela fait de l'ordre de 1 500 opérations du
routeur par seconde, soit environ 1 % d'un cœur du conteneur, et 8 à 10 % sur un Pi Zero 2 W
(estimé). Au rythme moyen de 1,9 valeur par seconde, c'est invisible.

#### 11.4.6 L'allocateur

**Garder `mallocng`, l'allocateur de musl**, celui de l'image de production. Sur l'état compact et
sur le courtier, c'est le meilleur des cinq réglages mesurés : son surcoût réel est de 0 à 8 %.
`mimalloc` fait pire dès que l'état est compact : +0,5 à +2,6 Mio sans pages énormes, +11 à
+30 Mio avec. Le facteur 2,7 de la V4 ne venait pas de l'allocateur : son banc comptait des voies en
tâches permanentes (2,7 Mio) et le socle du processus (1,3 Mio). Sous musl, la mémoire réelle vaut
**0,98 à 1,08 fois les octets demandés**. Sur un Pi, la politique de pages énormes (THP) du noyau
décide de tout : la mesure est à refaire sur la carte avant tout changement d'allocateur.

### 11.5 Tout en mémoire : ce que cela recouvre, les profils, le coût caché

#### 11.5.1 Ce que « toutes les données importantes » recouvre

| Famille | Contenu | Pourquoi en mémoire |
|---|---|---|
| Valeurs | valeur courante, précédente, trois instants (observé, reçu, changé), qualité | une condition se lit en nanosecondes ; « depuis 10 min » se calcule sans disque |
| Appareils et bus | définitions, santé, disjoncteurs, files d'attente | un ordre vers un appareil mort échoue tout de suite |
| Workflows | plans compilés, index par point, échéancier, baux, exécutions en cours | un événement ne réveille que ce qui le concerne ; un bail ne s'oublie pas |
| Agrégats | moyennes, extrêmes, durées sur 1 h et 24 h des points lus par les règles | « moyenne sur 1 h » en 1 ns au lieu de 150 µs |
| Historique récent | 24 h, 7 jours ou 30 jours selon le profil, en colonnes compactes | courbes du tableau de bord instantanées, redémarrage à chaud |

La valeur elle-même suit les règles du stockage V3 (§ 4 de ce document-là) : canonique au bord
(le pilote traduit une fois l'échelle, l'unité, le sens et le code d'énumération), stockée en
code et jamais en libellé, avec ses trois instants, sa qualité (bonne, incertaine, mauvaise) et
sa fraîcheur attendue par appareil. **Un automatisme ne déclenche jamais sur une valeur de
mauvaise qualité.**

#### 11.5.2 Le budget, demandé et réel

L'allocateur qui compte chaque octet donne les **octets demandés** (forme réelle des deux maisons,
banc B1 de la V3). Le conteneur Linux donne la **mémoire anonyme réelle** du cgroup, la mesure
que PERFORMANCE.md applique à Sarah Home et à Home Assistant (banc L1 de la V4).

| Élément | Deux maisons (496 appareils, 2 264 valeurs, 400 automatismes) | ×10 |
|---|---|---|
| Modèle du hub (ce qui existe aujourd'hui) | 1,00 Mio | 9,86 Mio |
| Étage vif : précédente, trois instants, qualité, index des points | 434 Kio | 4,04 Mio |
| Santé par appareil | 12 Kio | 116 Kio |
| Automatismes compilés et index par point | 203 Kio | 1,98 Mio |
| Baux (1 000) et une exécution en cours par automatisme | 150 Kio | 1,56 Mio |
| Agrégats 1 h et 24 h des points lus par les règles | 443 Kio | 4,69 Mio |
| Voies paresseuses, au repos | 6 Kio | 10 Kio |
| Tampon du bus d'événements (1 024 événements) | 128 Kio | 128 Kio |
| **Sous-total demandé : tout l'état vif** | **2,35 Mio** | **22,4 Mio** |
| **Mémoire anonyme réelle, état vif** (Linux, cgroup, voies paresseuses) | **3,7 Mio** (×1,5) ; **8,1 Mio à ×3** (×1,2) | **24,1 Mio** (×1,05) |
| Historique compact 24 h | 1,37 Mio | 14,3 Mio |
| Historique compact 7 jours | 8,78 Mio | 88,4 Mio |
| **Mémoire anonyme réelle, état vif + 7 jours** | ≈ 13,7 Mio (estimé ; 16,3 mesurés en V4 avec des voies en tâches permanentes) | — |
| Historique compact 30 jours | 37,1 Mio | 379 Mio (extrapolé) |
| Threads des pilotes (40 pilotes, Linux musl) | 0,76 Mio | 0,76 Mio |

Le facteur entre demandé et réel vaut **1,5 à ×1, 1,2 à ×3 et 1,05 à ×10** (banc `etat_vif`, un
processus neuf par échelle). Il se décompose en environ 1,2 Mio fixes (runtime, threads,
première allocation de chaque classe de taille de l'allocateur) et un peu plus d'un octet réel par
octet demandé. La V4 avait mesuré 6,3 Mio pour ×1 : son banc comptait des voies en **tâches
permanentes**, qui ajoutent 3,5 Mio demandés (§ 11.5.4). Avec des voies paresseuses, l'état vif des
deux maisons tient en **3,7 Mio réels**, et trois fois les deux maisons en **8,1 Mio**.

#### 11.5.3 Trois profils

| Profil | Contenu | Réel, deux maisons | Matériel |
|---|---|---|---|
| **Sobre** | état vif + 24 h d'historique + threads | **~6 Mio** | Pi Zero 2 W (512 Mo) : tient dans le budget de 32 Mio de mémoire anonyme de PERFORMANCE.md, avec le reste du cœur (4 à 7 Mio mesurés aujourd'hui) |
| **Généreux** | état vif + 7 jours + threads | **~14 Mio** | Pi 4 ou 5, mini-PC ; tient aussi dans les 32 Mio |
| **Maximal** | état vif + 30 jours + threads | ~43 Mio (estimé) | 2 Go et plus ; dépasse le budget du Pi |

Même le profil maximal reste environ 50 fois sous Home Assistant (de l'ordre de 2 Gio pour
l'étalon de l'architecture).

#### 11.5.4 Ce que la mémoire rapporte, et son coût caché

| Opération | Depuis le disque | Depuis la mémoire | Preuve |
|---|---|---|---|
| Lire « moyenne sur 1 h » | 150 à 190 µs (SQLite), plus un passage de thread | 1 ns (agrégat tenu au fil de l'eau) | V2 I2, V4 |
| Courbe 24 h d'un capteur à 1 Hz, réduite à 500 points | 21,8 ms (SQLite, Linux) | 0,10 ms | V3 B2.3, V4 |
| Redémarrage à chaud avec 7 jours en mémoire | 134 ms pour relire 1,13 million de relevés (Linux) | 4,3 ms depuis un instantané binaire de 8,6 Mio | V3 B2.3, V4 |
| Trouver et évaluer les workflows d'un événement | 0,9 à 54 µs (balayage actuel) | 2 à 18 ns (compilé, indexé) | V2 I1, V4 |

Le coût caché est celui des **voies en tâches permanentes**. Avec une tâche et une file de 8 par
appareil, on paie **7,3 Kio par appareil**, parce qu'une file Tokio réserve d'avance un bloc de 32
places de 200 octets (`CommandRequest`), plus la tâche. Cela fait 3,5 Mio pour les deux maisons
et 35 Mio à ×10, le plus gros poste du budget. Les voies paresseuses rendent la même isolation
pour 6 Kio au repos, et la V4 l'a vérifié sous charge (§ 17.4).

## 12. Le stockage et l'historique

### 12.1 Les quatre étages du stockage

<!-- FIGURE:etages -->

La mémoire sert la réactivité ; elle ne remplace pas la durabilité. Le stockage V3 range tout le
reste en quatre étages, chacun avec sa technique :

| Étage | Où | Contient | Horizon | Écrit |
|---|---|---|---|---|
| 1. Vif | mémoire du hub | valeur, trois instants, qualité, bandes ouvertes | maintenant | jamais : c'est le journal qui le protège |
| 2. État courant | `maison.db` (Turso) | une ligne complète par point ; appareils, état métier, échéances | la dernière valeur | par lots de 60 s : **≈ 65 Mo par jour**, contre ≈ 117 Mo pour `state.json` aujourd'hui (estimé) |
| 3. Récent | `moli.db` (Turso) | journal du jour, relevés à bande rangés par point, énergie, catalogue, dictionnaires | 7 à 31 jours (D13) ; heures d'énergie 2 mois | par lots de 2 s : **≈ 0,26 Go par jour**, contre ≈ 3,1 Go pour le chemin de D13 tel que décidé (mesuré) |
| 4. Long terme | archive Parquet (`moli-archive`) | paliers par nature : bande, puis heures, puis jours | des mois à toujours | lot par lot à l'export ; **17 Mo par an** d'heures, 4 Mo de jours, 175 Mo de détail à bande (mesuré, valeurs synthétiques) |

Et, à côté, les deux pièces de reprise des V3 et V4 :

| Pièce | Forme | Écrit par jour |
|---|---|---|
| Journal de reprise | une ligne binaire par changement : `seq u64, point u32, charge, instant u64`, ~9 o compressés ; lots ≤ 100 ms ou 500 lignes | **~1,3 Mo** |
| Instantané binaire de l'état vif | `(point u32, étiquette u8, charge, 3 instants)`, avec le `seq` du journal ; 0,45 ms (Linux) | **~1,3 Mo** au quart d'heure ; à chaque seconde vers le réseau si une secours écoute |

Quatre règles tiennent ce rangement :

1. **Un étage ne garde jamais ce qui revient au suivant.** `moli.db` n'a pas d'agrégat ;
   l'archive n'a pas d'état courant ; le cœur ne calcule plus aucun résumé.
2. **La mémoire est bon marché, les écritures ne le sont pas.** Garder 7 jours en mémoire coûte
   environ 9 Mio réels ; un instantané JSON par seconde écrirait 3,8 Go par jour et userait la carte SD.
3. **Le type décide** : une mesure passe en relevés à bande puis en heures ; un état garde ses
   transitions sans jamais être lissé ; un index cumulé s'élague en deltas, jamais en moyenne ;
   une occurrence reste un événement (stockage V3, § 5).
4. **Le journal de reprise est la seule source de la machine de secours.** Elle rejoue le même
   journal et peut en tirer ses propres `maison.db` et `moli.db`. Seul le porteur de l'époque
   exporte vers l'archive ; le `seq` du journal rend l'export rejouable sans doublon (proposé,
   § 18.1.4).

### 12.2 Ce qui est écrit, au total

| Poste | Par jour | Source |
|---|---|---|
| Journal de reprise et instantanés au quart d'heure | ~2,6 Mo | V3 B2 |
| `maison.db`, état courant par lots de 60 s | ≈ 65 Mo | stockage V3, série écritures |
| `moli.db`, historique en opérations compactes par lots de 2 s | ≈ 0,22 à 0,26 Go | stockage V3, série écritures |
| À éviter : instantané JSON complet chaque seconde | 3,8 Go | V3 B2 |
| À éviter : historique écrit selon D13 tel que décidé | ≈ 3,1 Go | stockage V3 |

Sur un Pi, la carte SD est le vrai coût : le `fsync` par lot est le poste à surveiller (1,9 ms
par `fsync` dans le conteneur, soit 1,9 % d'un cœur en attente d'entrées-sorties pour un lot
toutes les 100 ms), et PERFORMANCE.md mesure déjà les écritures.

### 12.3 La base de données et l'archive, branchées sur MQTT

#### 12.3.1 Le module base de données

La base de l'historique se branche directement sur le courtier. C'est un abonné comme les autres,
qui écrit les deux étages « disque » du stockage V3 :

| Ce qu'il lit | Ce qu'il écrit | Rythme | Règles du stockage V3 tenues |
|---|---|---|---|
| `val/#` | `maison.db` : une ligne complète par point, l'état courant | lots de 60 s, ≈ 65 Mo écrits par jour | suffit au démarrage |
| `val/#`, `evt/#`, `avail/#` | `moli.db` : le récent (7 à 31 jours, D13), relevés à bande rangés par point, transitions des états, occurrences, disponibilités | lots de 2 s, ≈ 0,26 Go écrits par jour au lieu de 3,1 | le type décide ; un état n'est jamais lissé ; un index s'élague ; une énumération reste un code |

Le module tient trois engagements :

- **Il ne perd rien.** Le flux `val/` n'a pas de trou (`s`). Un saut, après une surcharge ou un
  redémarrage du module, est comblé depuis le journal du hub (§ 10.3). L'historique est complet,
  même si le courtier a sauté des messages pour cet abonné.
- **Il ne ralentit personne.** Ses lots et ses écritures tournent sur son thread et ses propres
  connexions Turso. Un disque lent (une carte SD) le retarde, lui ; le courtier saute en avant
  pour lui, et il comble ensuite. Les autres abonnés ne le voient pas.
- **Il n'écrit rien sur MQTT.** Il répond aux questions courtes du moteur de workflow (§ 12.3.4), et
  sert l'historique au module WebSocket (§ 14.1).

#### 12.3.2 L'archive

L'archive ne change pas : `moli-archive`, service à part, Parquet typé, codes, zstd (stockage V3,
décisions 1 à 3). Elle est alimentée **depuis `moli.db`**, par l'export en lots de 5 000 opérations,
et non par MQTT : le long terme se nourrit d'un récent déjà rangé, vérifié et complet.

| Pièce | Rôle |
|---|---|
| Export (C1) | lots de 5 000 opérations, rejouables par `s` : aucun doublon si un lot repart |
| Paliers (E1) | la nuit, côté archive : bande, puis heures (`moli-rollup`), puis jours |
| Lecture longue (D1) | heures et jours par l'archive ; brut et bande en local ; la valeur vive comme dernier point ; archive injoignable signalée |
| Qui exporte | **seul le porteur de l'époque**. La secours tient sa base à jour, mais n'exporte pas |

#### 12.3.3 Ce que les écrans demandent

Une courbe passe par le module WebSocket, qui la demande à la base : au plus 2 000 points,
300 par défaut, en seaux (moyenne, min, max, nombre) dès que les changements sont plus nombreux
(c'est déjà le contrat de `GET /api/history`, `rest.rs:676-793`). Au-delà de la fenêtre locale,
l'algorithme D1 va chercher les heures et les jours dans l'archive. La réponse part par blocs sur
la WebSocket (§ 14.1.3), jamais par MQTT. Les tailles des réponses sont mesurées au § 12.4.

#### 12.3.4 Ce que le moteur de workflow demande

`read_history` (« la moyenne sur 24 h ») n'attend qu'**une valeur**. La question et la réponse
sont courtes : elles peuvent passer par MQTT, sur le modèle demande et issue.

```text
moli/v1/req/automation      {"id":"x41:n3","hist":"salon_thermo/temperature","stat":"moyenne","h":24}
moli/v1/res/automation      {"id":"x41:n3","v":2084,"n":1440,"q":"g"}
```

Le module base de données y répond depuis ses agrégats tenus au fil de l'eau quand il les a (une
lecture en nanosecondes, V3), et sinon depuis `moli.db` (environ 150 µs, V2 I2). Une série complète,
elle, ne passe jamais par MQTT.

### 12.4 L'historique

#### 12.4.1 Pourquoi jamais par MQTT

La discussion externe a raison : envoyer l'historique d'un capteur dans un seul message MQTT pose
de vrais problèmes.

- **La taille** : depuis Mosquitto 2.1, un paquet de plus de 2 000 000 o est refusé par défaut.
  Un mois brut d'un capteur à la minute en fait déjà 946 Ko (§ 12.4.3).
- **La mémoire** : le courtier et le client gardent le message entier en tampon, et un JSON
  décodé occupe bien plus que ses octets.
- **La diffusion** : dix abonnés qui reçoivent les mêmes 5 Mo, c'est 50 Mo à livrer.
- **Le direct qui attend** : un gros message retient la connexion, et les relevés suivants font
  la queue derrière lui.

#### 12.4.2 Ce que Sarah Home fait déjà

`GET /api/history` répond déjà comme la discussion le recommande (`rest.rs:676-793`) :

| Règle | Valeur |
|---|---|
| Fenêtre | 1 min à 1 an |
| Nombre de points | 300 par défaut, de 10 à **2 000 au plus** |
| Forme | les changements bruts s'ils tiennent dans le nombre de points, sinon **des seaux** : moyenne, minimum, maximum, nombre, premier instant |
| Compression | gzip ou brotli selon le navigateur (`tower-http`, brotli niveau 4) |
| Le direct | par le flux, jamais par l'historique |

Garder le minimum et le maximum de chaque seau, c'est ce que demandait la discussion « si les
pics doivent rester visibles » : un pic d'une minute ne disparaît pas dans la moyenne d'une heure.

#### 12.4.3 Ce que pèse une courbe (banc T3)

<!-- FIGURE:historique -->

| Série | Brut | Par élément | Servi en brotli | Sérialisation |
|---|---|---|---|---|
| 300 seaux (défaut, 24 h) | **23 307 o** | 78 o par seau | **4 673 o** | 0,06 ms |
| 2 000 seaux (maximum) | 151 109 o | 76 o | 28 212 o | 0,31 ms |
| 24 h brutes à la minute (1 440 points) | 31 762 o | 22 o par point | 8 618 o | 0,05 ms |
| 7 jours bruts (hypothèse) | 220 943 o | 22 o | 61 Ko | 0,32 ms |
| 30 jours bruts (hypothèse) | 946 293 o | 22 o | 266 Ko | 1,17 ms |
| 1 an brut (hypothèse) | **11,5 Mo** | 22 o | 3,1 Mo, en **4,2 s** de compression | 14 ms |

Trois enseignements :

1. **La borne de 2 000 points fait tout le travail.** Quelle que soit la durée, une courbe tient
   en moins de 30 Ko compressés. Sans elle, un an de données coûterait 11,5 Mo et plus de 4 s de
   processeur pour la seule compression.
2. **La moyenne est trop précise.** `AVG()` de SQLite rend 16 à 17 chiffres significatifs ;
   l'arrondir à la précision du point (0,01 °C) gagne 11 à 12 % sur chaque réponse.
3. **Le nombre de points doit suivre l'écran.** Une courbe de 800 pixels n'a pas besoin de plus
   de 800 points : demander `points=800` au lieu de 2 000 divise la réponse par 2,5.

#### 12.4.4 Ce qui manque encore

| Besoin | Proposé |
|---|---|
| Une courbe qui dépasse la fenêtre locale (plus de 7 à 31 jours) | l'algorithme D1 du stockage V3 : heures et jours par l'archive, brut et bande en local, la valeur vive comme dernier point, archive injoignable signalée |
| Un export (CSV, une année entière) | un autre chemin que la courbe : par pages ou en flux (NDJSON, CSV), borné, repris par curseur, jamais une réponse de plusieurs Mo d'un bloc |
| Une courbe qui se prolonge en direct | le client continue la courbe avec les événements du flux ; Home Assistant fait de même avec `history/stream` |
| Des courbes instantanées | l'historique récent en mémoire selon le profil (§ 11.5.3) : 0,10 ms au lieu de 21,8 ms pour 24 h |

## 13. Le moteur de workflow : il réagit à MQTT et répond en MQTT

La maquette des règles (`maquette-regles-sarah-v3.html`) fixe le modèle. Le moteur de workflow
de la V7 en est l'exécution, en Rust, sur le courtier : **il s'abonne aux valeurs, il demande des
ordres, il lit les issues.** Il ne parle jamais directement à un appareil ni au hub.

<!-- FIGURE:workflow -->

### 13.1 Le modèle de la maquette, nœud par nœud

| Famille | Nœuds de la maquette | Ce que le moteur en fait sur MQTT |
|---|---|---|
| **Quand** | `when_state` (avec `for_s` : « depuis 15 min »), `when_threshold` (au-dessus, en dessous, avec durée), `when_occurrence` | abonnement à `val/` ou `evt/` des points concernés ; une transition (et non un état) déclenche ; la durée est une échéance de l'échéancier, désarmée si la condition retombe |
| | `at_time`, `at_sun` (décalage), `every`, `on_start` | l'échéancier unique du moteur ; `on_start` = l'activation de l'époque (pas chaque redémarrage du courtier) |
| | `manual` (bouton, voix, montre) | une demande sur `req/ui` ou `req/assistant`, adressée à l'automatisme |
| | `when_mail` | un module courriel publie l'**événement** (un code, l'expéditeur classé) sur `evt/` ; le texte reste hors MQTT |
| **Lire** | `read_value` | le miroir compact du moteur (16 o par point), tenu par `val/` : une lecture en moins d'une nanoseconde, jamais un aller-retour |
| | `read_history` (moyenne, tendance sur 24 h) | une question courte au module base de données ; la réponse est **une valeur** (§ 12.3.4) |
| | `read_weather` | les valeurs du module météo, sur `val/` |
| **Si** | `if` (règles, toutes ou une), ports `oui` et `non` | évalué sur le miroir ; une valeur de qualité mauvaise vaut « inconnu » |
| **Faire** | `set` (avec `restore_s` : « pendant 30 min »), `toggle`, `scene` | une demande sur `req/automation` (l'exécution et le nœud dans la charge), puis l'issue sur `res/automation` ; « pendant » ouvre un **bail durable** qui rétablit la valeur à son terme ; une scène attend les confirmations de ses membres |
| | `mail_move`, `cal_add` | des appels à des modules hors MQTT (courriel, agenda) : le texte n'y passe pas |
| **Attendre** | `wait` (secondes) | l'échéancier |
| | `wait_for` (règle, délai), ports `c'est fait` et `trop long` | abonnement et échéance ; un **bail durable** garde l'attente à travers un redémarrage ou une bascule |
| **Prévenir** | `notify` (canaux, titre, message à gabarit `{{maison.conso}}`) | le texte part hors MQTT, au service des messages ; `evt/` en garde la trace par un code |
| **Sarah** | `write` (« elle écrit, elle ne décide jamais ») | appel LLM hors MQTT ; le texte rédigé va au nœud suivant (une notification) |
| | `decide` (branches, seuil de confiance, moteur : règles fixes, Sarah à la maison, service en ligne avec accord ; confirmation possible) | appel hors MQTT ; le résultat est **court** (la branche choisie, la confiance) et passe sur `evt/` pour le journal ; sous le seuil, la branche « à revoir », ou une demande de confirmation |

Les automatismes eux-mêmes gardent ce que la maquette prévoit :

- un **mode** (`restart` : une nouvelle cause relance l'exécution en cours ; `single` : elle est
  ignorée tant que la première tourne) ;
- une **empreinte de validation** (`fp`, calculée sur les nœuds, les liens et le mode) : un
  automatisme ne tourne que si son empreinte est celle qu'un humain a validée, et toute
  modification demande une nouvelle validation (règle 2) ;
- des **exécutions** (`runs`), avec leur cause, leur durée et l'état de chaque nœud. Le moteur les
  publie au fil de l'eau sur `evt/`, par des codes : l'écran montre « en cours : attend que la
  fenêtre se ferme ».

### 13.2 Le moteur, en Rust

Bloc technique — un automatisme compilé et une exécution (esquisse) :

```rust
/// Compilé une fois, à la validation : graphe vérifié, ports résolus, déclencheurs indexés.
pub struct Plan {
    pub id: AutoId,
    pub empreinte: u64,                     // fp : doit égaler celle qu'un humain a validée
    pub mode: Mode,                         // Restart | Single
    pub declencheurs: Vec<Declencheur>,     // indexés par point : un événement ne réveille que ses plans
    pub noeuds: Vec<Noeud>,                 // indices, pas de chaînes : 2 à 18 ns par événement (V4)
}

pub enum Noeud {
    Lire { point: PointIdx },
    Si { regles: Vec<Regle>, toutes: bool, oui: NoeudIdx, non: Option<NoeudIdx> },
    Regler { point: PointIdx, valeur: Valeur, pendant: Option<Duration>, suite: NoeudIdx },
    Scene { scene: SceneId, suite: NoeudIdx },
    Attendre { duree: Duration, suite: NoeudIdx },
    AttendreQue { regle: Regle, delai: Duration, fait: NoeudIdx, trop_long: Option<NoeudIdx> },
    Prevenir { canaux: Canaux, gabarit: GabaritId, suite: Option<NoeudIdx> },     // hors MQTT
    Ecrire { consigne: ConsigneId, suite: NoeudIdx },                             // Sarah, hors MQTT
    Decider { branches: Vec<NoeudIdx>, seuil: u8, moteur: MoteurDecision, confirmer: bool },
}

/// Une exécution : ce qui doit survivre à un redémarrage et à une bascule est dans le journal.
pub struct Execution {
    pub id: ExecId,
    pub plan: AutoId,
    pub cause: Cause,                       // la transition, l'heure, la demande qui l'a lancée
    pub curseur: NoeudIdx,
    pub attente: Option<Bail>,              // wait_for, pendant… : écrit au journal à l'ouverture
    pub en_vol: Option<DemandeId>,          // la demande publiée sur req/, dont on attend l'issue
}
```

Ce qui le rend sûr :

| Règle | Comment |
|---|---|
| Aucune décision n'attend le réseau | les conditions lisent le **miroir** en mémoire : l'état demandé au hub au démarrage, puis le flux `val/` ; 0,7 à 0,9 ns par lecture (§ 11.4) |
| Aucune transition perdue | un saut de `s` déclenche la resynchronisation : le moteur demande au hub la différence et rejoue chaque changement comme une transition, une seule fois (V4) |
| Aucun ordre direct | le moteur publie des **demandes** (origine `automation`) ; la garde décide ; l'issue revient sur `res/` |
| Aucun ordre rejoué | une demande dont l'échéance est passée n'est jamais renvoyée ; un automatisme qui veut insister le dit (« réessayer 3 fois en 2 min ») |
| Durées et attentes fiables | « depuis 15 min », « pendant 30 min », « attendre que… 4 h » sont des **baux** au journal, repris au démarrage et par la secours |
| Rien ne s'emballe | au-delà de 20 lancements par minute, le moteur fusionne et ralentit au lieu d'éteindre l'automatisme (V2) |
| Une seule copie en mémoire | les définitions (le graphe tel qu'on l'édite, la version validée) restent sur disque ; en mémoire, seul le plan compilé, partagé : 0,12 Mio pour 400 automatismes au lieu de 4,3 (§ 11.4) |

### 13.3 Les groupes de modes et les transitions

La maquette donne aussi des **groupes de modes** : la maison (Normal, Nuit, Absent, Vacances),
l'activité du salon (Normal, Lecture, Cinéma), la vigilance (Vert, Orange, Rouge, qui change seule
d'après des règles validées), l'armement de l'alarme (sous la garde), la chambre d'amis, les
enfants, l'entretien. Dans la V7 :

- **le mode courant d'un groupe est une valeur** comme une autre : `val/groupe_maison/mode`, un
  code d'énumération. Tout le monde le voit ; un automatisme peut s'y abonner ;
- **changer de mode est une demande** (`req/`) : la famille, le planning, Sarah ou une règle.
  L'armement de l'alarme passe par la garde (PIN), comme aujourd'hui ;
- **une transition s'exécute en trois phases**, dans cet ordre : en quittant l'ancien mode, au
  passage d'un mode à l'autre, en entrant dans le nouveau. Une réaction propre au passage ne
  remplace jamais en silence une réaction générale ;
- **un profil** s'applique, et se termine en restaurant l'état d'avant. La restauration ne
  touche pas une valeur que quelqu'un a modifiée depuis (« 4 valeurs rétablies, le lampadaire
  gardé, modifié à 16 h 05 »). C'est l'instant « changé », et son origine, qui le permettent ;
- **l'essai à blanc** joue la transition sans publier aucune demande : le moteur rend la liste
  des étapes et dit lesquelles deviendraient une demande à valider ;
- **le journal des transitions** (quand, quoi, par qui : planning, toi, Sarah) est fait de lignes
  du journal du hub, publiées sur `evt/` pour l'écran.

### 13.4 Sarah dans le workflow

Sarah reste **hors de la boucle de contrôle** (règle 1 de l'architecture : aucun LLM dans la
boucle) :

- elle **rédige** un texte, qui part dans une notification ;
- elle **choisit entre les branches qu'un humain a écrites**, avec un seuil de confiance. Sous le
  seuil, c'est la branche « à revoir », ou une confirmation demandée ;
- elle tourne sur le moteur choisi : règles fixes sans modèle de langage, Sarah à la maison, ou
  un service en ligne avec l'accord du foyer.

Ni le texte, ni la consigne donnée au modèle, ni sa réponse longue ne passent par MQTT. Seul le
résultat court (la branche, la confiance) est publié sur `evt/`, pour le journal et pour l'écran.

## 14. Les écrans et les transports, mesurés

### 14.1 Le module WebSocket : une connexion par écran

#### 14.1.1 Ce qu'il remplace, et pourquoi WebSocket

Aujourd'hui, le tableau de bord ouvre un flux SSE (`/api/events`) pour le direct, et appelle REST
pour les ordres et les courbes. Le module WebSocket fait les trois sur une seule connexion :

| Besoin | Aujourd'hui | Avec le module WebSocket |
|---|---|---|
| L'état au démarrage | un instantané SSE, en JSON, non compressé (469 Kio pour les deux maisons) | l'état demandé au hub, filtré par les droits, envoyé par blocs de 64 Kio au plus |
| Le direct | un événement SSE par changement | les `val/` filtrés, poussés en trames |
| Un ordre | `POST /api/command`, une requête par ordre | un message `ordre`, puis son **issue** (confirmé, exécuté, accepté…, § 5.6) sur la même connexion |
| Une courbe | `GET /api/history` | un message `historique`, la réponse **par blocs**, annulable |
| Un écran en retard | un nouvel instantané complet (`sse.rs:128`) | idem, mais seulement les points que cet écran voit |

Le § 14.2 mesure ce que coûte une connexion WebSocket et une connexion SSE dans Sarah Home. Le
choix de la V7 ne tient pas à la mémoire, qui reste faible à l'échelle d'une maison (64 écrans au
plus). Il tient à ceci : **une seule connexion porte les quatre besoins, et l'issue d'un ordre
revient par le même chemin que l'ordre.**

#### 14.1.2 Les droits

Le module applique les droits que l'API applique aujourd'hui :

- une session (le PIN, une fois par navigateur et par mois, D0) ;
- la vue d'un **invité**, qui ne suit que ses pièces : il ne reçoit que les `val/` et les
  `avail/` de ses appareils, jamais le journal ni les notices (`sse.rs:1-8`) ;
- l'origine des ordres (`ui` avec une session, `household` sans) ;
- les plafonds de connexions (8 par appelant, 64 en tout, comme les flux SSE).

Il est abonné à `val/#` une seule fois pour tous les écrans, et filtre par écran : il ne crée pas
un abonnement au courtier par écran. Il ne garde **aucun miroir** de l'état : pour un nouvel écran,
il lit l'instantané du hub (§ 10.4), ce qui économise 0,5 à 4 Mio selon la maison (§ 11.4).

#### 14.1.3 Le protocole

Bloc technique — les messages d'une connexion (JSON, une trame par message) :

```text
écran → module   {"t":"bonjour","session":"…","vue":"maison"}
module → écran   {"t":"etat","s":918273,"val":{"salon_thermo/temperature":{"v":2150,"o":…,"c":…}, …},
                  "avail":{…},"meta":{…}}                     ← le miroir filtré par les droits
module → écran   {"t":"val","p":"salon_thermo/temperature","v":2151,"o":…,"c":…,"s":918274}
écran → module   {"t":"ordre","id":"01J9…","p":"salon_lampe/on","v":true}
module → écran   {"t":"issue","id":"01J9…","i":"confirme","ms":412}
écran → module   {"t":"historique","id":"h7","p":"salon_thermo/temperature","h":168,"points":800}
module → écran   {"t":"bloc","id":"h7","n":1,"fin":false,"seaux":[[ts,moy,min,max,nb], …]}
module → écran   {"t":"bloc","id":"h7","n":2,"fin":true,"seaux":[…]}
écran → module   {"t":"annuler","id":"h7"}
module → écran   {"t":"retard"}  puis  {"t":"etat",…}      ← l'écran a pris du retard : nouvel état
```

Les règles du protocole :

| Règle | Pourquoi |
|---|---|
| Une file bornée par écran ; si elle déborde, l'écran reçoit `retard` puis un nouvel état | un téléphone sur un mauvais réseau ne retient ni le module ni les autres écrans |
| Blocs d'historique de 64 Kio au plus, un seul bloc en vol par écran, annulables | une courbe d'un an ne bloque pas le direct ; l'écran garde la main |
| Au plus 2 000 points par courbe ; `points` égal à la largeur du graphique | une courbe de 800 pixels n'a pas besoin de 500 000 points |
| Le direct passe avant l'historique | une lampe qui s'allume ne doit pas attendre la fin d'une courbe |
| Pas de compression par message (`permessage-deflate`) | elle coûte de la mémoire par connexion pour des messages d'une centaine d'octets ; les blocs d'historique peuvent être compressés à part |
| Tampons réglés : lecture 4 Kio, écriture 0 | mesuré : 11 Kio par écran au lieu de 138 Kio avec les réglages par défaut d'axum (§ 14.2.5) |
| Aucun message de plus de 64 Kio, l'état initial compris (envoyé par blocs) | mesuré : après un message de 480 Ko, le tampon d'écriture garde cette taille, soit 480 à 607 Kio par écran |

#### 14.1.4 REST et MCP restent

Les scripts gardent REST, et les agents gardent MCP (22 outils). Leurs ordres passent par
l'api-to-mqtt de Sarah Home : une demande sur `req/`, puis l'issue sur `res/`. Rien ne change pour eux,
sinon qu'un ordre rend désormais son issue (§ 14.1.3), comme pour un écran.

### 14.2 Les transports, mesurés

Cette sous-section mesure les transports tels qu'ils sont aujourd'hui : SSE et REST vers les
écrans, MQTT au bord, là où un appareil le parle. La V7 en tire le module WebSocket du § 14.1 et
les tailles des sujets du § 7.3.

#### 14.2.1 Ce que Sarah Home utilise aujourd'hui

| Liaison | Protocole | Réglages relevés dans le code | Bornes |
|---|---|---|---|
| Zigbee2MQTT → Sarah Home | MQTT 3.1.1 (`rumqttc` 0.25) | abonnement `zigbee2mqtt/#` en QoS 1, ordres `…/set` en QoS 1 non retenus, lectures `…/get` en QoS 0 à 2 par seconde, maintien 30 s, session propre | 4 Mio entrant, 256 Kio sortant, file de 64 |
| Frigate, Bambu, Landroid → Sarah Home | MQTT (Bambu en TLS, Landroid sur WebSocket du cloud) | abonnements en QoS 0 ; ordres en QoS 1 (Frigate, Landroid) ou 0 (Bambu) | 256 Kio / 16 Kio |
| Samsung, Tuya cloud, Landroid | WebSocket (`moli-net`) | TLS épinglé pour Samsung | par pilote |
| ESPHome, Roborock, Android TV | TCP ou UDP natifs | Noise pour ESPHome, miIO, Polo | par pilote |
| Ponts et appareils HTTP, profils | HTTP(S) | interrogation périodique, délais bornés | par profil (5 s, 1 Mio par échange pour SMLIGHT) |
| Sarah Home → tableau de bord | **SSE** `GET /api/events` + **REST** | un instantané, puis un événement par changement ; maintien 15 s ; non compressé | 8 flux par appelant, 64 en tout |
| Sarah Home → agents | MCP (HTTP « streamable », `rmcp`) | 22 outils | la garde |

La discussion externe opposait HTTP par interrogation périodique, WebSocket et MQTT. Sarah Home utilise
déjà la quatrième voie, **SSE**, pour pousser vers les écrans : une connexion, aucun sondage.

#### 14.2.2 Ce qu'un message coûte sur le fil (banc T1)

<!-- FIGURE:fil -->

| Message (charge JSON de Zigbee2MQTT) | MQTT 3.1.1 QoS 0 | QoS 1 (+ accusé de 4 o) | MQTT 5 QoS 0 / 1 | **SSE de Sarah Home** | WebSocket, mêmes événements |
|---|---|---|---|---|---|
| Thermomètre (64 o) | **97 o** | 99 + 4 o | 98 / 100 o | 4 événements → **467 o** | 367 o |
| Détecteur (50 o) | **83 o** | 85 + 4 o | 84 / 86 o | 3 → **352 o** | 277 o |
| Prise avec mesure (74 o) | **97 o** | 99 + 4 o | 98 / 100 o | 5 → **578 o** | 453 o |
| Ordre lampe (14 o, sur `…/set`) | **41 o** | 43 + 4 o | 42 / 44 o | 1 → 114 o, plus l'événement du journal : 244 o | 89 o |

Ce que le banc montre :

1. **Un relevé coûte quelques dizaines d'octets en MQTT, cinq fois plus en SSE.** Sarah Home émet un
   événement par attribut, et chacun porte l'identité complète du point en clair
   (`"point":"z2m:0x00124b00aaaa0001/temperature"`). Un événement par appareil (hypothèse, non
   implémentée) ferait 176 o au lieu de 467.
2. **Un ordre coûte 84 o côté MQTT** (PUBLISH QoS 1, accusé, état revenu) et **1 006 o pour un
   navigateur** : la requête `POST /api/command` avec la session (349 o), la réponse (299 o, dont
   288 d'en-têtes), et deux événements SSE (114 et 244 o). Un script sans session paie 477 o
   requête et réponse comprises ; un Chromium complet, 1 055 o.
3. **Les maintiens de liaison sont négligeables.** SSE : un commentaire vide toutes les 15 s, 8 o
   sur le fil, 45 Kio par jour et par navigateur. MQTT : PINGREQ et PINGRESP toutes les 30 s, 4 o,
   11 Kio par jour et par liaison. Avec TCP, IP et TLS 1.3 : 754 et 574 Kio par jour.
4. **Sous l'application**, TCP et IP ajoutent 52 o par segment (horodatage compris), TLS 1.3
   22 o par enregistrement : un relevé MQTT seul dans un segment fait 149 o, 171 o chiffré.

#### 14.2.3 Combien de messages

Les deux maisons réelles produisent **environ 163 000 relevés par jour**, soit 1,9 par seconde en
moyenne et 239 par seconde à la pointe (V3). Au tarif du § 14.2.2 :

| Lien | Par relevé | Par jour, deux maisons |
|---|---|---|
| Zigbee2MQTT → courtier → Sarah Home (MQTT QoS 0) | ~97 o, deux fois (vers le courtier, puis vers Sarah Home) | ~32 Mo |
| Sarah Home → un écran (SSE, un événement par attribut) | ~117 o par événement | ~19 Mo par écran ouvert toute la journée, non compressés |

La discussion externe estimait 69 920 messages par jour pour 100 appareils simulés : le même
ordre de grandeur. Elle avait aussi raison sur un point que les comptes réels confirment : **un
seul appareil bavard pèse plus que tout le reste**. Une prise qui publie sa puissance chaque seconde
fait 86 400 messages par jour ; cinquante détecteurs d'ouverture, 1 500. La déduplication au bord
(§ 5.2) et le tampon d'entrée (§ 9) s'en chargent.

#### 14.2.4 Les gros messages (banc T2)

Ce ne sont pas les relevés qui posent problème, ce sont trois gros messages :

| Gros message | Taille mesurée | Quand | Risque | Ce qu'il faut |
|---|---|---|---|---|
| **L'instantané SSE** des deux maisons | **480 707 o** bruts (469 Kio) ; 17 469 o en gzip, 16 404 en brotli 4 ; 0,85 ms à sérialiser | à chaque connexion d'un écran, et à chaque retard (`Lagged`) | **jamais compressé** : `DefaultPredicate` de `tower-http` exclut `text/event-stream`. 64 flux × 469 Kio = 30 Mio d'un coup après un retard général ; un téléphone sur un mauvais réseau met des secondes à le recevoir | l'envoyer compressé à part (un `GET` compressé, puis le flux), ou le découper ; pour un invité, il est plus petit (ses pièces seulement) |
| **`bridge/devices`** de Zigbee2MQTT | ~3,3 Ko par appareil (fixture du dépôt : 26,5 Ko pour 8 appareils) | à chaque démarrage du pont et à chaque changement du réseau | **Mosquitto 2.1 refuse les paquets de plus de 2 000 000 o** par défaut et coupe le client : le pont tombe vers 600 appareils | régler `max_packet_size` sur le courtier ; Sarah Home prévient dès 1 Mo |
| **Une série d'historique brute** | 11,5 Mo pour un an à la minute | si on la demandait d'un bloc | voir § 12.4 | jamais : au plus 2 000 points |

Un `Event::State` moyen fait 95 o de JSON, 116 o cadré en SSE. Une petite maison (50 appareils,
250 valeurs) a un instantané de 52 Ko.

#### 14.2.5 Ce qu'une connexion coûte (banc T4)

Mémoire anonyme du processus serveur par connexion vivante (pente entre 64 et 1 000 connexions,
Linux, conteneur 2 CPU / 128 Mio), avec un vrai serveur axum 0.8.9 et les couches de Sarah Home :

| Connexion | Par connexion | 64 connexions (le plafond de Sarah Home) | Après l'envoi d'un instantané de 480 Ko | CPU par livraison |
|---|---|---|---|---|
| **SSE de Sarah Home** (avec la couche de compression) | **39,1 Kio** | 2,5 Mio | 37 Kio : rien n'est retenu | 17 µs à 1 000 abonnés |
| SSE sans compression | 29,6 Kio | 1,9 Mio | — | 16 µs |
| **WebSocket, réglages par défaut d'axum** | **138,4 Kio** | 8,9 Mio | **480 à 607 Kio** : le tampon d'écriture garde la taille du plus gros message | 31 µs |
| WebSocket « léger » (lecture 4 Kio, écriture 0) | **11 Kio** | 0,7 Mio | 480 Kio (même effet) | 14 µs |

La couche de compression de Sarah Home coûte à elle seule ~9,5 Kio par flux SSE : `CompressionBody`
pèse 5 816 o en ligne. Mille WebSocket aux réglages par défaut dépassent 128 Mio : le conteneur
est tué à 919 connexions. Sarah Home plafonne à 64 flux, donc rien de tout cela n'est un risque
aujourd'hui.

Les chiffres de la discussion externe (14 à 64 Kio par connexion WebSocket) venaient d'une
bibliothèque Python. En Rust, l'ordre de grandeur est le même, mais il dépend surtout des réglages
des tampons : de 11 à 138 Kio pour le même protocole.

#### 14.2.6 SSE ou WebSocket ?

| Critère | SSE + REST (aujourd'hui) | WebSocket |
|---|---|---|
| Mémoire par écran | 39 Kio | 11 Kio réglé, 138 Kio par défaut |
| Après un gros message | rien n'est retenu | le tampon garde sa taille, sauf si on le réduit |
| Un ordre et son issue | une requête REST par ordre (477 à 1 055 o), l'issue par le flux | sur la même connexion |
| L'historique | une requête REST compressée | par blocs, sur la même connexion |
| Simplicité, mandataires, Cloudflare Access | HTTP ordinaire, reconnexion automatique du navigateur | à régler, mais standard |

**À l'échelle d'une maison, aucun des deux ne pose de problème de mémoire ni de débit.** L'écart
qui compte est ailleurs : avec SSE, l'ordre et son issue suivent deux chemins différents.

La V7 choisit un module WebSocket (§ 14.1), réglé à 11 Kio par écran,
qui ne renvoie jamais un message de plus de 64 Kio. Il porte le direct, l'historique, les ordres
et leur issue sur une seule connexion.

## 15. Zigbee, Matter et les appareils qui parlent déjà MQTT

### 15.1 Zigbee : Zigbee2MQTT sur le courtier de Sarah Home

| Aujourd'hui | Avec la V7 |
|---|---|
| Zigbee2MQTT publie sur un Mosquitto à part ; le pilote `moli-z2m` s'y abonne | Zigbee2MQTT se connecte au courtier de Sarah Home, sur l'écouteur local, avec son compte : **l'ACL le borne à `zigbee2mqtt/#`**, il ne peut ni lire ni écrire `moli/v1/…` |
| le pilote ignore `bridge/state` : une panne du pont est invisible (§ 18.1.2) | le module `z2m` lit `bridge/state`, le testament MQTT du pont : `offline` ouvre le disjoncteur du bus, tous les appareils Zigbee passent hors ligne, les ordres sont refusés en 0,1 ms |
| un message Zigbee2MQTT de quatre attributs devient quatre `set_state` | le module traduit `zigbee2mqtt/<nom>` en `in/z2m/<appareil>/<point>`, au canon, dédupliqué : seuls les attributs qui changent passent |
| ordre « fait » dès la file locale | l'ordre arrive sur `cmd/z2m/…`, le module publie `…/set`, et l'issue attend l'état (§ 8.4) |
| `bridge/devices` limité à 2 000 000 o par Mosquitto 2.1 | l'écouteur local accepte 4 Mio ; le module prévient dès 1 Mio |
| aucune santé par appareil | rythme déclaré par appareil (la disponibilité de Zigbee2MQTT, ou les rapports attendus), pertes tolérées, appareil muet signalé (§ 10.5) |

Le réseau radio ne change pas : un seul coordinateur, la procédure de promotion et les quinze
tests du § 18.1.10 restent ceux de la V7. Zigbee2MQTT suit l'époque : il ne tourne que sur la
machine qui la porte.

### 15.2 Matter : un module api-to-mqtt de plus

Le module `matter` parle au contrôleur Matter. Aujourd'hui, ce contrôleur est Matter Server 9.0, sur
matter.js, par son API WebSocket ; plus tard, ce pourrait être `rs-matter` dans le binaire (D9). Le
module publie les attributs des nœuds en valeurs au canon, et exécute les ordres. Pour le reste de
Sarah Home, un appareil Matter est un appareil comme un autre :

- même arbre de sujets, même santé, même garde ;
- aucun pont `matter2mqtt` en Go : le module, en Rust, fait la traduction, l'énumération en code et
  la déduplication ;
- la fabric reste à un seul écrivain : le contrôleur suit l'époque, comme Zigbee2MQTT (§ 18.1.6) ;
- Thread garde ses deux routeurs de bordure actifs : rien à basculer.

### 15.3 Les appareils MQTT du réseau local

Tasmota, Shelly ou ESPHome en mode MQTT publient eux-mêmes sur un courtier. Avec la V7, un profil
déclaratif suffit pour les intégrer (§ 5.4). Il reste une condition : ces appareils doivent pouvoir
se connecter au courtier de Sarah Home **sans pouvoir écrire ailleurs que chez eux**.

Avec le courtier repris, c'est l'**écouteur du réseau local** qui les reçoit, chacun avec son compte et
son ACL, ses bornes et son débit (§ 6.2, § 6.4) :

| Garde-fou | Ce qu'il empêche |
|---|---|
| ACL sur l'arbre de l'appareil (`tasmota/+/stat/#`, `tasmota/+/tele/#` en écriture) | qu'un appareil écrive `moli/v1/…`, ou l'arbre d'un autre |
| 4 Kio par message, une file courte, un débit plafonné | qu'un appareil fou inonde le courtier |
| TLS quand l'appareil le sait ; sinon, un réseau des objets séparé (règle 10) | qu'on écoute ou qu'on usurpe un appareil sur le réseau |
| Le module de l'intégration met tout au canon et valide | qu'une valeur absurde atteigne le hub |

Dans tous les cas, l'appareil n'écrit jamais `moli/v1/…` : il écrit ses propres sujets, et son
module les met au canon.

### 15.4 Ce qui ne change pas pour les autres pilotes

Les 37 genres de pilotes actuels gardent leurs transports : HTTP, WebSocket, TCP natif, UPnP, cloud. Ils
deviennent des modules api-to-mqtt par la façade du kit (§ 5.3), puis gagnent un par un la
déduplication, la santé et l'issue confirmée.

## 16. Ce qui reste hors MQTT

La règle est simple : **MQTT ne porte que des valeurs courtes.** Une charge de plus de 4 Kio, ou
qui n'est pas une valeur, passe ailleurs. Le courtier le fait respecter : `max_payload_size = 4096`
sur tous les écouteurs, sauf celui de Zigbee2MQTT (§ 6.4).

| Contenu | Pourquoi pas MQTT | Par où il passe | Ce qui passe quand même sur MQTT |
|---|---|---|---|
| Appels LLM (Sarah : rédiger, choisir une branche, converser) | textes longs, latences de secondes, contenus privés | HTTP vers le modèle local ou en ligne, depuis le module qui en a besoin | le résultat court d'une décision (la branche, la confiance) sur `evt/` |
| Notifications, annonces, messages | du texte, des destinataires, parfois une image | le service des messages (téléphone, voix, écran) | un code de suivi sur `evt/` (« notification 42 envoyée, lue ») |
| Devoirs, courriels, agenda (services famille) | des textes longs et personnels (notes de cours, devoirs, courriels) | les modules de ces services, en HTTP, avec leurs propres bornes | un événement court (« nouveau devoir pour demain ») ; une **note chiffrée** peut être une valeur |
| Images des caméras, audio, voix synthétique | des Kio à des Mio, des flux | le canal des médias du hub (images, `moli-audio`, Wyoming) | la disponibilité de la caméra, un événement « personne » |
| Historique long | des milliers de points | le module WebSocket, par blocs ; REST pour les scripts | une statistique courte demandée par le workflow (§ 12.3.4) |
| Fichiers : exports, sauvegardes, plan de la maison | des Mio, rares | HTTP | rien |
| Archive en masse | des lots de 5 000 opérations | l'export HTTP vers `moli-archive` | rien |
| Réplication vers la secours | le journal, les instantanés, les fichiers des ponts | le canal de réplication, chiffré | rien de plus que ce qu'elle lit déjà en abonnée (§ 18.2.2) |
| Codes, PIN, jetons, URL d'autorisation ; position des personnes ; texte d'une notification | des secrets, ou des données privées | le canal de services et les routes REST des modules ; la position sur `val/` sous `sensible: position` (§ 5.1) | jamais un secret ; pour une notification, une référence et un code de suivi |
| Configuration, validation des automatismes | des documents, et un PIN | REST, sous la garde | la méta qui en découle, sur `meta/` |

Cette frontière protège trois choses :

- la **réactivité** : un message d'une centaine d'octets ne fait jamais la queue derrière un
  devoir ou une image ;
- la **vie privée** : le texte d'un courriel ou d'un devoir ne circule pas sur un bus que tant de
  modules lisent ;
- la **mémoire du courtier** : ses files et ses retenus restent petits (§ 11.4).

## 17. Quand ça va mal : production, bus capricieux, correctif, voies

### 17.1 En production : Linux musl, 2 CPU, 128 Mio

Les bancs des V2 et V3 ont été recompilés par la V4 en Linux musl statique (le profil de l'image
`FROM scratch`, sans LTO) et exécutés dans un conteneur `alpine` limité à deux cœurs et 128 Mio,
la limite de `compose.yaml`. Les écarts avec le Mac viennent de l'allocateur de musl (plus lent,
surtout sur les nombreuses allocations de `serde_json`) et de la virtualisation. Le Pi Zero 2 W
est estimé 8 à 10 fois plus lent que ce conteneur (non mesuré).

#### 17.1.1 Les mêmes bancs, Linux contre Mac

| Mesure | Mac | Linux | Écart |
|---|---|---|---|
| Moteur actuel, balayage, 400 automatismes | 1,8 à 2,1 µs par événement | 3,4 µs | ×1,7 |
| Moteur compilé, 400 / 10 000 automatismes | 3 / 10 à 11 ns | 2 / 18 ns | — |
| « Moyenne 1 h » : SQLite / agrégat au fil de l'eau | 150 à 190 µs / 1 ns | 153 µs / 1 ns | — |
| Expression interprétée / compilée | 743 à 754 ns / 28 à 32 ns | 444 ns / 25 ns | — |
| Instantané JSON complet, compressé | 6,8 ms | **20,1 ms** | ×3 |
| Instantané binaire de l'état vif, compressé | 0,30 ms | 0,45 ms | ×1,5 |
| Ordre vers un pilote isolé (thread), p50 / p99 | 0,01 / 0,02 ms | 0,03 / 0,06 ms | identique au pilote partagé |
| Moteur réel, croisière, 400 automatismes, p50 / p99 | 0,5 / 2,9 ms | 1,1 / 1,4 ms | — |
| Moteur réel, rafale de 2 000, action au milieu / après | 36,6 / 59,7 ms | 44,1 / 78,5 ms | ×1,3 |
| Moteur réel, rafale de 5 000, 400 automatismes | 80 ms / **perdue** | 102 ms / **perdue** | — |
| Deux pilotes bloquants partagés : médiane d'un ordre, pire retard d'une minuterie | 1,6 à 6,7 ms / ~195 ms | **202 ms / 405 ms** | toute la maison au pas des bloqueurs |
| Six pilotes bloquants isolés | ≤ 0,1 ms / 3 à 44 ms | 0,1 ms / 6,8 ms | — |
| Pont, lampe muette : boucle unique / voie par appareil | 37 à 42 / 237 sur 237 ordres sains | 41 / 237 sur 237 | — |
| API à 4 places : tâche par ordre / voie par appareil | 1,43 s / 32 ms | 1,35 s / 34,5 ms | — |
| Bus mort 15 s : ordres pendant la panne, voies / disjoncteur | 4 002 / 0,1 ms | 4 003 / 0,1 ms | — |
| Inondation de 5 000 au retour d'un bus, sans / avec tampon | 5 sur 5 perdus / 0 perdu | 5 sur 5 perdus / 0 perdu, 6 à 15 ms | — |

Ce qui change de lecture sous Linux :

- **Le moteur actuel décroche plus tôt.** À 5 000 événements, il prend du retard dès
  50 automatismes et perd des déclencheurs dès 100 ; à 200, il perd même des déclencheurs injectés
  au milieu de la rafale. Sur un Pi, le même décrochage arriverait sur des rafales de quelques
  centaines d'événements.
- **Deux pilotes bloquants gèlent tout.** Avec deux cœurs, il ne reste rien pour le cœur.
- **L'instantané JSON coûte trois fois plus** : 2 % d'un cœur s'il était fait chaque seconde, 15
  à 20 % sur un Pi (estimé). C'est aussi, à peu de chose près, le prix de l'instantané SSE envoyé
  à chaque nouvel écran (§ 14.2.3).
- Les cloisons, les voies et le tampon d'entrée donnent exactement les mêmes résultats : leur
  effet ne dépend pas de la machine.

#### 17.1.2 Le plafond du moteur actuel

Le banc T1 de la V4 pousse 400 automatismes réels sous un **débit soutenu** pendant 3 s (bruit sur
700 capteurs, par lots d'une milliseconde), avec une sonde toutes les 500 ms.

| Débit soutenu | Mac, sans tampon | Mac, avec tampon (10 ms) | Linux, sans / avec tampon |
|---|---|---|---|
| 1 000 par seconde | 0 perdu, 0,5 ms | 0 perdu, 9,0 ms | 0 perdu, 1,0 ms / 0 perdu, 11,5 ms |
| 16 000 | 0 perdu, 0,7 ms | 0 perdu, 11,5 ms | 0 perdu, 0,9 ms / 0 perdu, 5,4 ms |
| 64 000 | 0 perdu, 0,8 ms | 0 perdu, 6,6 ms | 0 perdu, 3,9 ms / 0 perdu, 8,9 ms |
| 128 000 | 0 perdu, 0,8 ms | 1 perdu sur 6, 17 retards | **1 perdu sur 6, 24 retards** / 0 perdu, 13,7 ms |

**Ce n'est pas le débit qui fait décrocher le moteur actuel, c'est la profondeur d'une rafale
instantanée.** À 64 000 événements par seconde soutenus sous Linux, 270 fois la pointe réelle des
deux maisons (239 par seconde), le moteur suit, parce que chaque lot d'une milliseconde reste
très en dessous des 1 024 places du bus d'événements. Une rafale de 5 000 événements envoyée d'un
trait (un pont qui republie tout à sa reconnexion) remplit ces places en quelques millisecondes.
Le tampon d'entrée transforme une rafale en lots, au plus un événement par point et par 10 ms,
pour 3 à 15 ms de latence pendant la rafale et rien au repos.

Pour la suite, une conséquence directe : **la question n'est jamais « combien de messages par
seconde peut-on encaisser », mais « combien en arrive-t-il d'un coup ».** Un courtier MQTT qui
redistribue ses messages retenus à la reconnexion, un pont Zigbee qui republie tout, un écran qui
redemande un instantané : ce sont des rafales, et elles se traitent en amont (§ 14.2.3).

### 17.2 Quand ça va mal : bus morts, inondations, bus capricieux

#### 17.2.1 Un bus entier tombe (V3 P1)

Un bus de 50 appareils meurt pendant 15 s. On envoie 10 ordres par seconde vers ses appareils et
10 par seconde vers un autre pilote.

| | Autre pilote, p50 / p99 | Ordres au bus pendant la panne | Reprise après le retour du bus |
|---|---|---|---|
| Voies seules | 22,1 / 25,3 ms | médiane **4 002 ms** (147 ordres, tous en délai dépassé) | premier succès 108 ms après |
| Voies + disjoncteur de bus, essai à 500 ms | 22,1 / 23,0 ms | médiane **0,1 ms** (refus immédiat, appareils hors ligne) | premier succès 503 ms après (634 ms sous Linux) |

Les voies protègent déjà les **autres** pilotes. Le disjoncteur protège les **workflows** : un
ordre vers un bus mort échoue tout de suite, ce qui permet de prendre une autre branche au lieu
d'attendre 4 s par étape. Le signal de reconnexion du transport (le ConnAck MQTT, pour
Zigbee2MQTT) ramènerait la reprise à presque rien.

#### 17.2.2 Le bus revient et inonde le hub (V3 P2)

Le bus revient et republie 5 000 valeurs d'un coup sur 700 capteurs, puis 5 vrais déclencheurs
arrivent ; 400 automatismes réels tournent.

| | Déclencheurs perdus | Latence des déclencheurs | Retards du moteur |
|---|---|---|---|
| Sans tampon d'entrée | **5 sur 5, aux trois essais** | — | 2 retards, ~2 600 événements manqués |
| Avec tampon d'entrée (dernière valeur gagne, vidé toutes les 10 ms) | **0** | 4,9 à 10,8 ms (6 à 15 ms sous Linux) | aucun |

5 000 valeurs sur 700 points deviennent ~700 événements. La correction du moteur (§ 17.3) reste
nécessaire pour les rafales que rien ne fusionne.

#### 17.2.3 Un bus qui répond mal (V4 Q1 à Q3)

<!-- FIGURE:disjoncteur -->

Un bus réel répond souvent **mal** avant de ne plus répondre du tout. Trois caprices ont été
simulés : un pont saturé qui met 2,5 s à répondre, un coordinateur qui redémarre en boucle, un
service cloud qui perd une requête sur deux. Chacun a été joué contre quatre protections, à charge
égale : 10 ordres par seconde vers 10 appareils du bus pendant 20 s, et 10 par seconde vers un
autre pilote. L'« attente totale » est le temps cumulé de tous les appelants (un workflow, la
famille) : c'est le coût vécu.

| Caprice | Protection | Réussis sur 200 | p50 / p95 des réussis | Échoués (p50) | Attente totale | Bascules | Autre pilote p99 |
|---|---|---|---|---|---|---|---|
| **Q1 lent** (2,5 s, sous le délai) | voies seules | 20 | 2,5 s / 4,0 s | 180 (5,0 s, échéance) | **965 s** | 0 | 23 ms |
| | disjoncteur simple | 20 | 2,5 s / 4,0 s | 180 (5,0 s) | 965 s | 0 | 24 ms |
| | hystérésis + refus prédictif | **85** | 4,3 s / 4,9 s | 115 (**0 ms**, refusés d'avance) | **380 s** | 1 | 24 ms |
| | adaptatif + réessais | 85 | 4,3 s / 4,9 s | 115 (0 ms) | 382 s | 3 | 24 ms |
| **Q2 qui bat** (mort 1,5 s, vivant 1,5 s) | voies seules | 145 | 1,9 s / 4,9 s | 55 (4,0 s) | 500 s | 0 | 23 ms |
| | disjoncteur simple | 38 | 23 ms / 0,5 s | 162 (2,0 s) | 352 s | **9** | 24 ms |
| | hystérésis + refus prédictif | 15 | 23 ms / 23 ms | 185 (0 ms) | 136 s | 1 | 24 ms |
| | adaptatif + réessais | **200** | **26 ms / 1,5 s** | **0** | **105 s** | 0 | 24 ms |
| **Q3 partiel** (une requête sur deux perdue) | voies seules | 62 | 4,7 s / 5,0 s | 138 (5,0 s) | 912 s | 0 | 24 ms |
| | disjoncteur simple | 33 | 23 ms / 3,0 s | 167 (1,9 s) | 316 s | **22** | 24 ms |
| | hystérésis + refus prédictif | 57 | 3,0 s / 5,0 s | 143 (5,0 s) | 847 s | 0 | 24 ms |
| | adaptatif + réessais | **144** | 2,7 s / 4,5 s | 56 (5,0 s) | 573 s | 4 | 24 ms |

Sous Linux, les douze cas donnent la même lecture :

- bus lent : 20 réussites avec les voies seules, contre 96 et 92 avec les deux protections
  complètes ;
- bus qui bat : 200 sur 200 avec les réessais adaptatifs ;
- bus partiel : 148 sur 200 avec les réessais, contre 34 à 62 sans ;
- l'autre pilote reste à 31 ms de p99 dans les douze cas.

Ce qu'on en tire :

1. **Le bus lent est le vrai piège.** Rien n'échoue, donc un disjoncteur fondé sur les échecs ne
   s'ouvre jamais, et chaque ordre attend son échéance derrière les précédents. Il faut un
   **budget de latence** : le refus prédictif garde les ordres qui peuvent aboutir et refuse en
   0 ms ceux qui feraient la queue au-delà de leur échéance.
2. **Les réessais sauvent le bus qui bat et le bus partiel, pas le bus lent.** Ils demandent deux
   conditions. Tant que la latence du bus est inconnue, une seule tentative avec tout le temps
   restant. Et seulement pour les **ordres idempotents** : une consigne, jamais un appui.
3. **Le disjoncteur compte des ordres, pas des tentatives**, et des échecs consécutifs, pas des
   échecs par fenêtre. Sinon un bus partiel s'ouvre et se referme sans fin : 22 bascules en 20 s.
4. **L'hystérésis supprime l'oscillation** : 1 bascule au lieu de 9 à 22. Son prix : un bus qui
   bat est traité comme mort tant qu'il n'est pas redevenu stable, et la famille le voit.
5. **L'autre pilote ne voit jamais rien.** Les voies et le thread par pilote protègent le reste
   de la maison ; disjoncteur, budget de latence et réessais protègent **l'appelant**.

Ce sont exactement les pannes d'un courtier ou d'un pont MQTT. Un Mosquitto saturé est un bus
lent ; un Zigbee2MQTT qui redémarre en boucle est un bus qui bat ; un coordinateur qui perd des
trames est un bus partiel. **La haute disponibilité de la § 18.1 ne remplace pas ces protections :
elle s'y ajoute.** Une bascule prend une seconde au mieux ; pendant cette seconde et pendant
toutes les pannes qui ne justifient pas une bascule, ce sont les voies et le disjoncteur qui
gardent la maison réactive.

#### 17.2.4 Les autres pannes mesurées

| Panne | Sans le pattern | Avec le pattern | Preuve |
|---|---|---|---|
| Une lampe muette derrière un pont | 16 à 18 % d'ordres sains réussis, médiane ~4 s | 100 %, médiane 32 ms | V2 C2, V4 |
| API partagée limitée à 4 requêtes, une lampe muette | lampes saines à 1,4 s de médiane | 32 ms | V2 C3, V4 |
| Deux pilotes bloquent leur thread | ~200 ms par pas pour toute la maison | ≤ 0,1 ms de médiane pour le cœur | V2 C1, V4 |
| Un décodeur XML reçoit une réponse hostile | 96,6 s de CPU sur un worker partagé | le même coût, sur le seul thread du pilote, et refusé par le pré-contrôle | V1 E8 |
| La machine principale tombe | état vif et baux perdus | secours active en ~1 s, rien de perdu | V4 S1-S3 |

#### 17.2.5 Les workflows quand quelque chose ne va pas

Le principe : **ce qui ne marche plus ne bloque jamais ce qui marche.**

1. **Chaque action rend un résultat typé** : faite, retenue par la garde, refusée, appareil
   injoignable (disjoncteur), délai dépassé, et, proposé en § 8.4, « envoyée sans confirmation ».
   Un graphe peut brancher sur l'échec, comme `wait_for` branche déjà sur le délai. Par défaut,
   l'échec est journalisé et l'exécution continue ses autres branches.
2. **Un appareil injoignable répond en 0,1 ms**, pas en 4 s. Un workflow qui allume dix lampes
   dont trois sont sur un bus mort allume les sept autres tout de suite.
3. **Une condition sur un appareil hors ligne** lit une valeur de qualité « inconnue ». Elle est
   fausse par défaut ; un nœud peut aussi tester « inconnu » explicitement.
4. **Un ordre non exécuté à temps ne part jamais plus tard**, même après une reconnexion. Un
   workflow qui veut insister le dit (« réessayer 3 fois en 2 minutes »), avec une échéance.
5. **Les déclencheurs horaires, les baux et les minuteurs continuent** quand un bus tombe. Un bail
   échu pendant la panne s'applique au retour de l'appareil, s'il en est encore temps.
6. **Au retour d'un bus**, le tampon d'entrée fusionne la rafale : un « quand la température
   change » part une fois, sur la valeur finale. Une occurrence (appui, sonnette) n'est jamais
   fusionnée.
7. **La famille voit la panne** : bus et appareils hors ligne sur le tableau de bord, étape « non
   faite : bus injoignable » dans l'exécution, notification si le workflow le demande.
8. **Le garde-fou d'emballement** ralentit (anti-rebond, fusion) au lieu d'éteindre
   l'automatisme au-delà de 20 lancements par minute.

### 17.3 Le correctif de la perte sur retard, prouvé

Le défaut (V2, 3.3) : sur un retard du bus d'événements (`RecvError::Lagged`), le moteur recopie
le présent du hub dans sa mémoire des valeurs précédentes. Les événements encore en file, relus
ensuite, ne montrent plus aucune transition : les déclencheurs sont perdus.

Le correctif remplace `remember_all` par une fonction `resync` de 20 lignes dans la branche
`Lagged`. Il a été appliqué sur une **copie du moteur hors du dépôt** : comparer le présent du hub
à ce que le moteur connaissait, et **rejouer chaque différence comme une transition**. Un
événement encore en file, relu ensuite, compare égal et ne déclenche pas deux fois.

| Automatismes | Rafale de 5 000 événements | Avant, Mac | Après, Mac | Avant, Linux | Après, Linux |
|---|---|---|---|---|---|
| 100 | action injectée après la rafale | **perdue** (3 retards, 1 441 manqués) | **servie en 36,5 ms** | **perdue** (4 retards, 931 manqués) | **servie en 78,8 ms** |
| 200 | actions injectées au milieu | 36 ms | — | **9 perdues** | — |
| 400 | action injectée après la rafale | **perdue** (2 retards, 1 483 manqués) | **servie en 108,5 ms** | **perdue** (2 retards, 1 453 manqués) | **servie en 242 ms** |
| 400 | actions injectées au milieu | 80 ms, pire 161 ms | 61,5 ms | 102 ms | 75,4 ms |

Sous Linux, le moteur prend plus souvent du retard (4 à 5 fois par rafale), et le correctif
rattrape à chaque fois : **0 déclencheur perdu dans toutes les passes**. Son coût : à chaque
retard, un instantané du hub et une comparaison de toutes les valeurs, de l'ordre de la
milliseconde. Les actions manquées partent **une fois**, sur la valeur finale. Le correctif
n'empêche pas le retard (le moteur compilé et le tampon d'entrée s'en chargent) ; il empêche qu'un
retard coûte une transition.

Bloc technique — le correctif (copie de `moli-automation/src/engine.rs`, hors dépôt) :

```rust
/// Après un retard : le présent du hub comparé à ce que le moteur savait. Chaque point
/// qui diffère est une transition manquée, rejouée une fois. Un événement encore en file,
/// relu ensuite, compare égal et est ignoré : rien ne part deux fois.
fn resync(autos: &Automations) {
    let snapshot = autos.0.hub.snapshot();
    let changed: Vec<(PointId, Value)> = {
        let last = lock(&autos.0.last);
        snapshot.devices.iter().flat_map(|d| d.state.iter().filter_map(|(key, sample)| {
            let k = format!("{}/{key}", d.device.id);
            (last.get(&k) != Some(&sample.value)).then(|| (PointId::from(k), sample.value.clone()))
        })).collect()
    };
    for (point, value) in changed {
        on_state(autos, &point, &value);
    }
}
```

Le même principe sert le flux SSE : un écran en retard reçoit un **instantané** au lieu des
événements manqués (`sse.rs:128-131`). C'est juste pour un écran, qui veut l'état et pas
l'histoire ; c'est faux pour le moteur, qui veut les transitions. D'où deux traitements
différents du même `Lagged`.

### 17.4 Les voies paresseuses, écrites pour de bon

La V4 les a écrites :

- un répartiteur unique par pilote ;
- une file par appareil, créée au premier ordre en attente et détruite une fois vide ;
- un seul ordre en vol par appareil, et l'échéance revérifiée avant chaque envoi ;
- la consigne en attente remplacée par la suivante ;
- un disjoncteur par appareil, qui vide la file et passe l'appareil hors ligne.

| Scénario | Ordres aux lampes saines réussis | Médiane | Pire | Lampe muette | Pic mémoire pendant la charge (processus entier) |
|---|---|---|---|---|---|
| Pont sans limite, une lampe muette | 237 sur 237 | 32,6 ms | 34,9 ms | 5,0 s | 536 Kio |
| idem, avec disjoncteur | 237 sur 237 | 32,6 ms | 35,5 ms | 2,9 s puis refus immédiat | 533 Kio |
| API à 4 places | 237 sur 237 | 32,8 ms | 54,3 ms | 5,0 s | 536 Kio |
| idem, avec disjoncteur | 237 sur 237 | 33,0 ms | 33,7 ms | 2,9 s puis refus immédiat | 533 Kio |
| Le pont entier tombe de 2 s à 6 s, puis revient | 218 sur 237 (les 19 autres, pendant la panne, refusés) | 33,1 ms | 3,2 s | — | 534 Kio |

C'est la même isolation que les voies en tâches permanentes, pour un pic de moins de 0,6 Mio
pendant une charge de 25 ordres par seconde, harnais compris, au lieu de 3,5 Mio réservés en
permanence. Le dernier scénario montre le disjoncteur par appareil face à un pont entier qui
tombe : chaque lampe s'ouvre après deux échecs, et tout repart au retour du pont. Le disjoncteur
**de bus** fait la même chose en une fois au lieu de vingt.

Pour le pilote Zigbee2MQTT, la « voie » d'un appareil est son sujet `…/set`, et la règle d'un
seul ordre en vol prend une forme précise (proposée) :

- pour un point dont l'état revient (la plupart des lampes, prises et volets Zigbee), l'ordre
  reste en vol jusqu'à l'état confirmé ou jusqu'à l'échéance ;
- pour un point muet, la remise au courtier suffit ;
- dans les deux cas, une nouvelle consigne qui attend remplace la précédente.

C'est le même mécanisme qui donne la confirmation de la § 8.4.

## 18. La haute disponibilité, de bout en bout

### 18.1 Les radios, le courtier et les ponts

Cette sous-section reprend la haute disponibilité de la V5, pensée pour l'architecture d'aujourd'hui,
où Zigbee2MQTT publie sur un Mosquitto à part. Les domaines de panne, la topologie, les radios, le
témoin et les quinze tests valent tels quels pour la V7. Une chose change : le courtier n'est plus
un Mosquitto à côté de Sarah Home, c'est le sien, embarqué, qui suit l'époque avec elle (§ 18.1.5,
§ 18.2). Là où un test dit « Mosquitto », lire « le courtier de Sarah Home ».

La discussion externe a conçu une architecture à haute disponibilité pour deux boîtiers
identiques à deux radios (Zigbee et Thread), avec MQTT comme passerelle commune. Le boîtier
annoncé correspond à la famille SMLIGHT SLZB-MR : deux puces séparées, Zigbee et Thread en même
temps, alimentation PoE (annexe C). Le modèle exact reste à confirmer sur la page
d'administration du boîtier. Sarah Home garde le fond de cette architecture, et la ramène à la taille
d'une maison et aux règles de Sarah Home.

#### 18.1.1 Ce que Sarah Home garde, et ce qu'elle change

| Proposition externe | Décision | Pourquoi |
|---|---|---|
| Deux boîtiers double radio identiques, sur des alimentations et des switchs distincts | **gardé** | deux domaines de panne ; même famille de puce, condition d'une restauration Zigbee |
| Thread actif/actif sur les deux boîtiers | **gardé** | natif : plusieurs routeurs de bordure sur un même réseau, avec le même jeu de données |
| Zigbee2MQTT actif/passif sur deux VM | **gardé, mais attaché à l'époque de Sarah Home** | un seul mécanisme de bascule pour toute la maison, pas deux qui pourraient se contredire |
| Coordinateur Zigbee de secours, promotion automatique « à valider » | **promotion humaine d'abord**, automatique seulement après le test 2 réussi sur le matériel réel | identité IEEE, clés, compteur de trames : une restauration ratée coûte un réappairage de toute la maison |
| Clore la seule radio Zigbee de A, en gardant Thread A | **clore tout le boîtier A par son port PoE** | Thread B route déjà seul ; couper l'alimentation est la seule clôture sûre ; le profil `smlight` de Sarah Home ne fait volontairement aucune écriture vers le boîtier |
| Matter Server actif/passif, plus un pont `matter2mqtt` (Go) | **contrôleur Matter attaché à l'époque ; Sarah Home lui parle directement**, ou embarque `rs-matter` plus tard (D9) ; pas de `matter2mqtt` | un maillon et un langage de moins ; le serveur de référence est maintenant Matter Server 9.0, sur matter.js |
| Cluster RMQTT à trois nœuds Raft | **non** : un courtier actif sur l'hôte qui porte l'époque, sans état (un Mosquitto dans la V5, le courtier embarqué dans la V7) | journal Raft en mémoire seulement, intervention manuelle si la moitié des nœuds tombe, messages retenus en cluster via Redis (annexe C) ; un courtier sans état n'a rien à répliquer (§ 18.1.5) |
| Historique dans PostgreSQL ou TimescaleDB | **non** : Turso et archive Parquet | décidé (D13), mesuré (stockage V3) |
| Prometheus, Grafana, Alertmanager | **non** : la santé est déjà dans Sarah Home (`/api/health`, page Système, notices) | le budget d'un Pi ; une seule vérité sur l'état de la maison |
| Liaison Zigbee directe pour l'essentiel | **gardé** | la redondance la moins chère (§ 18.1.6) |
| Une batterie de tests de reprise avant toute automatisation | **gardé**, porté à quinze tests (§ 18.1.10) | — |

#### 18.1.2 Les domaines de panne

| Composant | Panne | Ce qui se passe aujourd'hui | Avec la V7 | Test |
|---|---|---|---|---|
| Zigbee2MQTT (processus) | plante, redémarre en boucle | **Sarah Home ne le voit pas** : il ignore `zigbee2mqtt/bridge/state` (`lib.rs:156`). Les appareils restent « en ligne », les ordres partent vers le courtier et répondent « fait » | `bridge/state` = `offline` (le testament MQTT du pont) ouvre le disjoncteur du bus : appareils hors ligne, ordres refusés en 0,1 ms. Relance locale ; aucune bascule | 1 |
| Courtier | tombe | le pilote sort en erreur, ses appareils passent hors ligne, il est relancé avec une attente croissante ; ordres refusés (« broker injoignable ») | inchangé (c'est déjà le bon comportement) ; relance locale | 10 |
| Hôte H1 | gel, coupure, disque | tout s'arrête | la secours H2 prend l'époque N+1 : Sarah Home en ~1 s (V4), puis le courtier et les ponts | 6, 7 |
| Boîtier A | alimentation, carte | Zigbee et Thread de A perdus | Thread : B route seul, sans rien faire. Zigbee : bus ouvert, notice, puis promotion de B | 2, 4 |
| Lien H1 – H2 | coupure entre les deux | — | le témoin départage : seul le porteur du bail agit | 8, 9 |
| Switch PoE 1 | tombe | A tombe (et H1 s'il en dépend) | comme « boîtier A » ou « domaine 1 » | 6 |
| Contrôleur Matter | plante | appareils Matter hors ligne | relance locale ; il suit l'époque si l'hôte change | 5 |
| `bridge/devices` trop gros | Mosquitto 2.1 refuse le paquet | le courtier coupe Zigbee2MQTT ; le catalogue ne passe plus | `max_packet_size` écrit (§ 19.1.3) ; Sarah Home prévient bien avant | 11 |

La première ligne est un écart à corriger, indépendant de toute haute disponibilité : c'est le
« signal du transport » que la V3 demandait déjà pour le disjoncteur de bus.

#### 18.1.3 Ce qui se redonde, et ce qui ne se redonde pas

| Élément | Écrivains possibles en même temps | Mécanisme |
|---|---|---|
| Routeur de bordure Thread | **plusieurs** | actif/actif : même jeu de données opérationnel (*Active Operational Dataset*) |
| Coordinateur Zigbee | **un seul au monde** pour un réseau donné | actif/secours : sauvegarde, clôture de l'ancien, restauration sur le nouveau |
| Fabric Matter (le stockage du contrôleur) | **un seul** | actif/passif : stockage copié, jamais ouvert par deux contrôleurs |
| Zigbee2MQTT | un seul, attaché au coordinateur | actif/passif, suit l'époque |
| Courtier MQTT | un par hôte, utilisé par l'actif seul | sans état : rien à répliquer |
| Sarah Home | un seul porteur de l'époque | actif/passif : journal, instantanés, époque (V4) |
| Liaison Zigbee directe | aucune : l'appareil agit seul | rien à basculer |

Une seule règle couvre les cinq premières lignes : **ce qui porte une identité n'a qu'un
écrivain à la fois, désigné par l'époque, l'ancien clos avant que le nouveau démarre.**

#### 18.1.4 La topologie retenue

<!-- FIGURE:topologie -->

- **Deux domaines de panne.** Chacun a son boîtier, son hôte, son switch PoE et sa prise (un
  onduleur si possible).
- **Sarah Home porte l'époque**, et les ponts la suivent. Zigbee2MQTT et le contrôleur Matter ne
  tournent que sur l'hôte qui porte l'époque. Comme Sarah Home, Zigbee2MQTT et le courtier sont sur le
  même hôte, le courtier reste local (§ 19.1.3).
- **Ce qui part vers la secours :**

  | Quoi | Taille | Quand |
  |---|---|---|
  | Journal de reprise et instantanés de Sarah Home | ~2,6 Mo par jour | en continu (V4) |
  | Données de Zigbee2MQTT : `configuration.yaml`, `database.db`, `coordinator_backup.json`, `state.json` | quelques centaines de Kio pour une maison (estimé) | à chaque changement et à l'arrêt propre |
  | Stockage du contrôleur Matter : la fabric, les nœuds | quelques Kio à quelques Mio (estimé) | à chaque changement |
  | Courtier | rien | — |

  Le canal de réplication de Sarah Home peut porter ces fichiers, ce qui évite un second outil de
  copie (proposé).
- **Le témoin** est un troisième appareil déjà présent (NAS, routeur, Pi). Il ne fait que voter
  (§ 18.1.7).

#### 18.1.5 Le courtier : actif là où sont les ponts, sans état

| Option | Mémoire | À répliquer | Bascule | Verdict |
|---|---|---|---|---|
| Mosquitto local à l'hôte actif (la V5) | 1 à 2 Mio (estimé) | rien : `persistence false`, sessions propres | il est déjà là : les ponts et Sarah Home s'y connectent sur l'hôte qui porte l'époque | simple, suffisant : retenu par la V5, remplacé dans la V7 |
| **Courtier embarqué dans Sarah Home** (`rumqttd` repris, la V7) | routeur, liens et journaux : 3,4 à 12,3 Mio selon l'échelle, optimisé (§ 11.4.1) | rien : aucun retenu, sessions propres | avec Sarah Home : il suit l'époque | **retenu** : pas pour la réactivité, mais pour un contrat unique de tous les modules (§ 3.1, § 18.2) |
| Cluster RMQTT, trois nœuds Raft | plusieurs dizaines de Mio par nœud (estimé) | journal Raft en mémoire seulement ; messages retenus via Redis en cluster | quorum de 2 sur 3 ; intervention manuelle si la moitié des nœuds tombe | non pour une maison : trois machines, une base de plus, aucun état utile à garder |
| EMQX, HiveMQ | centaines de Mio (estimé) | — | — | non |

Le point qui fait pencher : **pour Sarah Home, le courtier n'a aucun état à préserver.** Zigbee2MQTT
republie ses messages retenus (disponibilités, `bridge/devices`) à son démarrage, Sarah Home relit les
états des appareils à l'écoute (`…/get`, `lib.rs:179-196`), et aucun ordre ne doit survivre dans
une file. Répliquer un courtier sans état, c'est ne rien répliquer.

#### 18.1.6 Les radios

**Zigbee : un seul coordinateur, une promotion préparée.**

1. Les deux boîtiers ont la **même puce et le même firmware** sur leur radio Zigbee. La
   sauvegarde et la restauration de Zigbee2MQTT sont officielles pour les familles Z-Stack (Texas
   Instruments) et EmberZNet (Silicon Labs), pas d'une famille à l'autre.
2. Zigbee2MQTT écrit `coordinator_backup.json` : clé réseau, PAN, PAN étendu, canal, compteur de
   trames. Ce fichier part vers H2 avec le reste.
3. Pour **clore A**, on coupe son port PoE. Arrêter Zigbee2MQTT ne prouve pas que la radio A s'est
   tue.
4. **Restaurer sur B** : Zigbee2MQTT démarre, voit un adaptateur différent et une sauvegarde, et
   pose les paramètres du réseau sur B.
5. **Les deux pièges à tester** (test 2) :
   - **l'adresse IEEE du coordinateur** n'est pas toujours restaurée, et des retours de terrain
     montrent des appareils « disparus » de la base de Zigbee2MQTT après une migration ;
   - **le compteur de trames** : les appareils rejettent un compteur qui revient en arrière. Une
     sauvegarde trop ancienne peut donc rendre le réseau sourd.
6. **A ne revient jamais seul.** Son port reste coupé jusqu'à ce qu'un humain le réinitialise
   comme nouveau secours.

**Thread : rien à basculer.** Les deux routeurs de bordure sont actifs avec le même jeu de
données. La panne de l'un ne demande aucune action ; le maillage passe par l'autre. Le seul
travail est de partager ce jeu de données au départ, et de le vérifier (test 4).

**Matter : la fabric a un seul écrivain.**

- Le stockage du contrôleur contient l'identité de la fabric (certificats, numéros des nœuds) ;
  deux contrôleurs sur le même stockage la corrompent.
- Le serveur de référence est maintenant Matter Server 9.0, réécrit sur matter.js ; l'ancien
  serveur Python est archivé (annexe C).
- Sarah Home lui parle par son API WebSocket, comme le pilote Samsung le fait déjà avec un téléviseur
  (WebSocket TLS épinglé, `moli-samsung`).
- Si D9 fait de Sarah Home le contrôleur (`rs-matter`), la fabric vit dans le coffre et suit
  l'époque avec le journal : plus de processus séparé.
- Un appareil Matter accepte plusieurs fabrics (*multi-admin*). Une seconde fabric, celle d'un
  téléphone, garde une commande manuelle possible pendant une panne du contrôleur. Ce n'est pas
  une bascule, mais c'est une issue de secours qui ne coûte rien.

**La liaison Zigbee directe : la redondance la moins chère.** Un interrupteur lié à une lampe ou à
un groupe la commande sans coordinateur actif, sans courtier et sans Sarah Home. Pour les fonctions qui
ne doivent jamais manquer (l'entrée, l'escalier, la salle de bains), c'est plus sûr que toute
bascule. Cela complète la règle D12 sur les luminaires : la lampe reste pilotable même quand tout
le reste est tombé.

#### 18.1.7 Le témoin, le bail et la décision

Deux machines seules ne distinguent pas une panne d'une coupure entre elles. Si chacune croit
l'autre morte, il y a deux maîtres : deux Zigbee2MQTT sur la même radio, deux contrôleurs sur la
même fabric. Trois réponses existent, de la plus sûre à la plus rapide :

1. **La décision humaine** (par défaut) : la secours prévient, la famille ou l'administrateur
   promeut d'un geste.
2. **Le témoin et le bail** (proposé, une fois les tests 7 à 9 passés) : l'actif renouvelle un
   bail auprès du témoin ; la secours ne prend l'époque suivante qu'après l'expiration de ce bail.
3. **La clôture électrique** de l'ancien hôte (prise pilotée, port PoE), en plus du témoin, quand
   le matériel le permet.

Bloc technique — le bail d'époque avec un témoin (esquisse ; valeurs de départ à régler) :

```text
actif (porte N)   : toutes les 1 s → témoin : renouveler(N, H1)
                    sans réponse depuis 2 s → se rétrograder : arrêter les ponts, refuser les ordres
témoin            : bail(N, H1) valable 3 s ; refuse tout autre porteur tant qu'il court
secours           : silence de H1 ≥ 1 s → témoin : demander(N+1, H2)
                    accordé seulement après l'expiration du bail N (≤ 3 s)
                    → écrire N+1 sur disque → clore ce qui doit l'être (PoE de A si A est en cause)
                    → démarrer : Sarah Home, courtier, Zigbee2MQTT, contrôleur Matter
ancien actif      : revient, annonce N, apprend N+1, reste passif ; ses ponts restent arrêtés
```

L'actif se rétrograde à 2 s, avant la fin de son bail à 3 s. Quand le témoin accorde N+1, l'ancien
a donc cessé d'agir, même s'il est vivant mais isolé. Le prix : une reprise en 3 à 5 s au lieu
de la seconde mesurée en V4 sans témoin (estimé). Pour une maison, c'est un bon échange.

#### 18.1.8 Trois pannes, trois reprises

<!-- FIGURE:bascule -->

- **Zigbee2MQTT plante, l'hôte vit** : c'est la panne la plus fréquente, et elle ne justifie
  aucune bascule. Le disjoncteur s'ouvre sur `bridge/state`, le service redémarre sur place avec
  la même radio, le disjoncteur se referme au ConnAck et aux premiers états.
- **L'hôte H1 tombe** : la secours prend l'époque N+1. Sarah Home est prêt en quelques millisecondes ;
  le courtier, Zigbee2MQTT (avec la radio A, par TCP) et le contrôleur Matter (avec la même
  fabric) démarrent ensuite. Les ordres d'avant la panne ne partent jamais : leur échéance est
  passée.
- **Le boîtier A tombe** : Thread continue par B sans rien faire. Pour Zigbee, le bus s'ouvre,
  la famille est prévenue, un humain décide ; on coupe le port PoE de A, on restaure sur B, on
  teste un appareil sur secteur et un sur pile.

#### 18.1.9 Ce que coûte la haute disponibilité

| Poste | Une machine | La topologie retenue |
|---|---|---|
| Matériel | 1 hôte, 1 boîtier | 2 hôtes, 2 boîtiers, 2 switchs PoE (ou injecteurs pilotables), un témoin déjà présent |
| Mémoire de Sarah Home | profil sobre ~8 Mio | la même sur chaque hôte : la secours tient l'état vif à jour |
| Zigbee2MQTT et contrôleur Matter (Node.js) | une centaine de Mio chacun et plus (estimé) | sur l'hôte actif seulement ; la secours les garde arrêtés |
| Réseau | — | ~2,6 Mo par jour de journal, plus les fichiers des ponts |
| Hôte | Pi Zero 2 W possible pour Sarah Home seul (D10) | Pi 4 ou 5, ou mini-PC, dès que Zigbee2MQTT et Matter tournent à côté |
| Exploitation | — | une époque, un témoin, quinze tests, une procédure de retour de A |
| Ce qu'on y gagne | panne d'hôte : des heures, jusqu'à une intervention | panne d'hôte : quelques secondes. Boîtier : Thread tout de suite, Zigbee en minutes |

Sans aucun matériel de plus, trois mesures donnent déjà l'essentiel. Ce sont les plus rentables :

- les liaisons Zigbee directes pour les fonctions essentielles ;
- `bridge/state` qui ouvre le disjoncteur ;
- une sauvegarde de `coordinator_backup.json` hors de la machine.

#### 18.1.10 Quinze tests de reprise

Chaque test a un critère mesurable. Aucune promotion ne devient automatique avant que les
tests 2, 7, 8 et 9 passent sur le matériel réel.

| # | Scénario | Comment | Réussi si |
|---|---|---|---|
| 1 | Zigbee2MQTT tué sur l'hôte actif | `kill -9` du processus | disjoncteur ouvert en moins d'1 s par `bridge/state` ; refermé au retour ; aucun ordre parti après son échéance |
| 2 | Restauration du coordinateur A → B | couper le port PoE de A ; démarrer Zigbee2MQTT sur B avec la sauvegarde | tous les appareils répondent, dont un sur pile, sans réappairage ; clé, PAN, canal et compteur vérifiés ; durée notée |
| 3 | Retour inopiné de A après la promotion de B | rallumer le port de A | A ne forme pas de second réseau (balayage radio) ; aucun Zigbee2MQTT ne s'y connecte |
| 4 | Thread : un routeur, puis l'autre | couper A, rétablir, couper B | les appareils Matter répondent dans les deux cas, sans action |
| 5 | Contrôleur Matter déplacé | arrêt sur H1, démarrage sur H2 avec la fabric copiée | aucun nouvel appairage ; ordres et états passent |
| 6 | Perte du domaine 1 entier | couper H1, le switch PoE 1 et A ensemble | Sarah Home reprend sur H2 sans perte (RPO 0) ; Thread continue ; la procédure Zigbee se déroule comme prévu |
| 7 | Gel de H1 | `SIGSTOP`, ou gel de la VM | H2 actif après l'expiration du bail ; H1 dégelé reste passif, ponts arrêtés |
| 8 | Coupure entre H1 et H2, témoin visible des deux | filtrer le lien | un seul porteur d'époque ; jamais deux Zigbee2MQTT connectés au courtier ni à la radio |
| 9 | H1 isolé de tout | débrancher son réseau | H1 se rétrograde avant la fin du bail ; H2 promu ensuite ; aucun intervalle avec deux actifs |
| 10 | Courtier tué sur l'hôte actif | `kill -9` de Mosquitto | ordres refusés en 0,1 ms ; appareils hors ligne ; reprise quelques secondes après le retour |
| 11 | `bridge/devices` au-delà de 2 000 000 o | catalogue gonflé, Mosquitto 2.1 par défaut, puis `max_packet_size` réglé | la coupure est constatée, puis disparaît ; Sarah Home prévient dès 1 Mo |
| 12 | Ordre pendant une bascule | ordre envoyé 100 ms avant le gel | issue « envoyé » (`envoye`), « peut-être parti » (`incertain`) ou « non envoyé » (§ 5.6) ; jamais exécuté après l'échéance ; une ligne de journal |
| 13 | `…/set` retenu déposé par un tiers | publication retenue | Sarah Home le signale et l'efface ; au redémarrage du pont, rien ne se rallume |
| 14 | Client MQTT inconnu, ou compte Sarah Home hors de son arbre | connexion anonyme, publication interdite | refusés par le courtier ; tentative journalisée |
| 15 | Liaison directe, tout éteint | couper H1, H2, les courtiers et le boîtier A (le coordinateur) | l'interrupteur lié allume toujours sa lampe, par les routeurs du maillage |

### 18.2 Le moteur de haute disponibilité de Sarah Home

<!-- FIGURE:ha -->

#### 18.2.1 Trois niveaux de reprise

| Panne | Qui reprend | Durée | Ce qui est perdu |
|---|---|---|---|
| Un module (panique, blocage, processus à part qui meurt) | son superviseur, avec attente croissante | ms à s | rien : ses ordres en attente sont refusés, et un ordre déjà pris vaut « peut-être parti » (cause `module`), ses appareils passent hors ligne le temps de la panne |
| Le processus Sarah Home, ou le routeur du courtier | la sentinelle, puis le système (systemd, Docker) | quelques ms pour le hub (V4 : 4,3 ms avec 7 jours en mémoire), plus les reconnexions des modules | rien : journal et instantané |
| La machine | la secours, qui prend l'époque N+1 | ~1 s de détection (V4), le bail du témoin (≤ 3 s), puis la republication des retenus et le démarrage des modules (§ 11.4) | rien de ce qui était écrit (RPO 0, V4) ; les ordres en vol ne partent jamais après leur échéance |

#### 18.2.2 La secours, abonnée MQTT : toute la maison en mémoire

Avec MQTT au centre, la secours devient plus simple à tenir chaude. **Elle s'abonne au courtier
de la machine active**, comme un écran ou la base de données :

- à l'abonnement, elle demande **tout l'état d'un coup** au hub actif (§ 10.4), par blocs de 4 Kio,
  avec le rang `s` auquel il correspond. Aucun message retenu n'est en jeu ;
- ensuite, le flux `val/` la tient à jour, valeur par valeur, avec `s` pour repérer un trou ;
- son hub tient le même état vif que l'actif, avec les trois instants, la qualité et la santé des
  appareils (§ 10.5). Son moteur de workflow tient son miroir. Sa base de données écrit sa propre
  `maison.db` et sa propre `moli.db` : au moment de la bascule, la base est déjà à jour ;
- **elle n'écrit rien** : ni sur le courtier actif, ni vers les appareils. Ses modules et ses ponts
  restent arrêtés.

Ce que MQTT ne porte pas arrive par le **journal de réplication** de la V4 (~1,3 Mo par jour) :

- les baux et les exécutions en cours (« attendre que la fenêtre se ferme, encore 2 h 40 ») ;
- les approbations en attente ;
- l'époque ;
- les fichiers métier des aides : occurrences des boutons et des automatismes, compteurs, dates,
  minuteries, dépendances et durées (`backup.rs:32-38`), absents de la première version (crible G18) ;
- les tronçons qui manquent quand `s` saute.

La donnée la plus volumineuse, l'état des valeurs, passe donc par le chemin le plus simple. Le
journal ne porte plus que l'essentiel.

Bloc technique — l'écouteur de réplication du courtier actif, réservé à la secours (forme de `rumqttd` 0.20) :

```toml
# Écouteur 3 : la machine de secours seulement, sur le réseau local, chiffré, certificat exigé.
[v4.secours]
name = "secours"
listen = "0.0.0.0:8885"
next_connection_delay_ms = 100
    [v4.secours.tls]
    capath = "/data/courtier/ca.pem"          # seule l'autorité de Sarah Home signe le certificat de la secours
    certpath = "/data/courtier/courtier.pem"
    keypath = "/data/courtier/courtier.key"
    [v4.secours.connections]
    connection_timeout_ms = 5000
    max_payload_size = 4096
    max_inflight_count = 100
    dynamic_filters = true
# La secours s'abonne à moli/v1/val/#, avail/#, meta/#, sys/#, evt/#, avec le compte « secours » :
# ACL en lecture seule (ajout de Sarah Home, § 6.4), sauf sa demande d'état sur moli/v1/etat/demande.
```

#### 18.2.3 L'époque, le bail, le témoin

Le mécanisme est celui du § 18.1.7, qui reste valable tel quel :

- l'actif renouvelle son bail auprès du **témoin** chaque seconde ; sans réponse depuis 2 s, il se
  **rétrograde** : il arrête ses ponts et refuse les ordres ;
- le témoin ne donne le bail de l'époque N+1 qu'après l'expiration du bail N (3 s) ;
- la secours, devenue **active**, écrit N+1 sur disque, clôt ce qui doit l'être (le port PoE du
  boîtier radio en cause), ouvre son courtier aux clients, sert son état à qui le demande, puis démarre ses
  modules, Zigbee2MQTT et le contrôleur Matter ;
- l'ancien actif qui revient annonce N, apprend N+1 et redevient **passif** : il s'abonne à son
  tour au courtier du nouvel actif ;
- sans témoin, la promotion reste humaine.

Pour les clients, trois cas :

| Client | Comment il trouve le nouvel actif |
|---|---|
| Modules, ponts (Zigbee2MQTT, contrôleur Matter) | ils tournent sur la machine active : ils démarrent avec elle, en local |
| Écrans | le module WebSocket connaît les deux adresses, ou une adresse virtuelle portée par l'actif ; à la reconnexion, un nouvel état complet |
| Appareils MQTT du réseau local | une adresse virtuelle (VRRP, `keepalived`), portée par l'actif seul |

#### 18.2.4 Les tests qui s'ajoutent aux quinze premiers

Les quinze tests du § 18.1.10 restent. La V7 en ajoute quatre :

| # | Scénario | Réussi si |
|---|---|---|
| 16 | La secours s'abonne pendant que la maison vit | après l'abonnement, son état vif est identique à celui de l'actif (comparaison par `s` et par empreinte de l'état), en moins d'une seconde pour les deux maisons |
| 17 | La secours prend du retard (lien lent, gel de 30 s) | elle voit le saut de `s`, demande le tronçon au journal, et revient identique ; aucun trou dans sa `moli.db` |
| 18 | Le routeur du courtier panique sur l'actif | la sentinelle relance Sarah Home ; les modules se reconnectent ; aucune valeur perdue dans la base (comblée par `s`) |
| 19 | Bascule pendant une attente « 4 h au plus » | après la bascule, l'exécution est reprise sur la secours, au même nœud, avec la même échéance |

## 19. La sécurité

### 19.1 Ce que fait le code aujourd'hui, et la porte de service

Cette sous-section décrit le code d'aujourd'hui, où le courtier est un Mosquitto à côté de Sarah
Home, et ses correctifs. Ils valent jusqu'à l'étape où le courtier embarqué le remplace (§ 22.2) ;
ensuite, les mêmes protections vivent dans notre courtier (§ 19.2).

#### 19.1.1 Ce que Sarah Home fait aujourd'hui

| Sujet | Aujourd'hui | Code |
|---|---|---|
| Adresse du courtier | `127.0.0.1:1883` par défaut, TCP sans TLS | `moli-z2m/src/lib.rs:41-46` |
| Authentification | facultative : un utilisateur, et un mot de passe lu dans une **variable d'environnement** | `lib.rs:31-37, 82-88` |
| TLS | non compilé pour `moli-z2m` (`rumqttc` sans fonction TLS) ; `moli-bambu` l'a, car l'imprimante l'impose | `Cargo.toml` des deux crates |
| Taille des paquets | 4 Mio en entrée (pour `bridge/devices`), 256 Kio en sortie | `lib.rs:80-81` |
| Ce qui entre | traité comme hostile : JSON décodé dans un catalogue borné, appareil inconnu ignoré, `bridge/devices` validé, tests de confidentialité du transport | `catalog.rs`, `privacy_transport_tests.rs` |

Le mot de passe en variable d'environnement est une petite entorse à la règle 6 de
l'architecture (« aucun secret en clair, coffre chiffré »). Les autres pilotes lisent leurs
secrets dans le coffre (`DriverCtx::secret`). Le correctif tient en une ligne de configuration et
une ligne de code.

#### 19.1.2 Ce que le courtier permet à qui y est connecté

La règle 9 de l'architecture dit que **le binaire fait respecter les règles** : toute écriture
passe par le hub, donc par la garde et le journal. Mais un courtier ouvert crée une porte de
service :

| Qui peut… | …fait ceci | Ce que Sarah Home voit |
|---|---|---|
| publier sur `zigbee2mqtt/<appareil>/set` | commande l'appareil **sans garde, sans approbation, sans journal** (une serrure, une chambre aux heures calmes) | l'état qui change, sans ordre derrière |
| publier sur `zigbee2mqtt/<appareil>` | fabrique de **faux états** : un faux mouvement lance un automatisme, une fausse porte fermée trompe l'alarme | un état ordinaire |
| lire `zigbee2mqtt/#` | suit la présence pièce par pièce (mouvement, portes, consommation) | rien |
| publier un `…/set` **retenu** | rejoue l'ordre à chaque redémarrage du pont | rien : le message est ignoré en silence (`lib.rs:156`) |
| ouvrir l'interface web de Zigbee2MQTT | tout ce qui précède, plus l'appairage et la suppression d'appareils | rien |

Sur un réseau domestique de confiance, le risque est faible. Mais la règle 10 traite le réseau
local comme hostile, et un seul objet compromis sur le LAN suffit.

#### 19.1.3 Les correctifs, par rapport gain sur coût

1. **Le courtier n'écoute que la machine.** Quand Zigbee2MQTT et Sarah Home tournent sur le même hôte,
   ce qui est le cas de la topologie retenue (§ 18.1.4 : les ponts suivent l'époque de Sarah Home), le
   courtier n'a aucune raison d'écouter le LAN. Coût nul, et la porte se ferme.
2. **Un compte par client, aucun anonyme, des ACL par sujet.** Deux comptes seulement,
   Zigbee2MQTT et Sarah Home ; tout autre client est refusé. Les ACL ne peuvent pas restreindre Sarah Home
   aux seuls `…/set` quand les noms d'appareils ont plusieurs niveaux (bloc ci-dessous) : la
   protection vient surtout de l'absence de tout autre compte.
3. **Le mot de passe de Sarah Home dans le coffre**, plus en variable d'environnement.
4. **TLS dès que le courtier sort de la machine** (un pont sur un autre hôte, un courtier
   partagé) : la fonction `use-rustls` de `rumqttc`, déjà dans l'arbre des dépendances par
   `moli-bambu`.
5. **Une taille de paquet écrite des deux côtés.** Mosquitto 2.1 a abaissé sa limite par défaut à
   **2 000 000 octets** (annexe C). Au-delà, le courtier refuse le paquet et coupe le client qui
   l'envoie : un `bridge/devices` trop gros coupe Zigbee2MQTT. Les 4 Mio réglés par Sarah Home ne
   servent donc à rien sans le réglage correspondant du courtier (§ 14.2.3 pour les tailles).
6. **Sarah Home signale ce qu'il ne devrait pas voir** :
   - un `…/set` qui ne vient pas de lui : il connaît ses propres ordres, et l'écho de `…/#` lui
     montre les autres ;
   - un `…/set` retenu, qu'il efface ;
   - un état sans appareil connu.

   Une ligne de journal et une notice à la famille suffisent : la porte de service devient au
   moins visible.
7. **L'interface web de Zigbee2MQTT** derrière un mot de passe, et sur la seule machine.

Bloc technique — `mosquitto.conf`, courtier local à l'hôte, comptes et ACL :

```conf
# Le courtier n'écoute que la machine : Zigbee2MQTT et Sarah Home tournent sur le même hôte (§ 18.1.4).
listener 1883 127.0.0.1
allow_anonymous false
password_file /etc/mosquitto/passwd
acl_file /etc/mosquitto/acl

# Mosquitto 2.1 : 2 000 000 o par défaut. Écrit ici, aligné sur ce que Sarah Home accepte (4 Mio).
max_packet_size 4194304

# Aucune session persistante utile : Sarah Home et Zigbee2MQTT se reconnectent en session propre.
persistence false
max_queued_messages 100

# Si un pont doit vivre sur un autre hôte : un second écouteur, chiffré, jamais en clair.
# listener 8883 0.0.0.0
# certfile /etc/mosquitto/certs/courtier.crt
# keyfile  /etc/mosquitto/certs/courtier.key
```

Bloc technique — `/etc/mosquitto/acl`, chaque client à sa place :

```conf
# Deux comptes, et aucun autre : c'est là qu'est la protection.
# MQTT ne sait pas écrire « tout sujet qui finit par /set » : avec des noms d'appareils à
# plusieurs niveaux (salon/lampe), on ne peut pas restreindre Sarah Home à …/set seulement.

# Zigbee2MQTT : son arbre entier (états, pont, ordres reçus).
user zigbee2mqtt
topic readwrite zigbee2mqtt/#

# Sarah Home : lit tout le pont ; écrit dans l'arbre (ordres …/set, lectures …/get).
user moli-os
topic read  zigbee2mqtt/#
topic write zigbee2mqtt/#
# Si les noms d'appareils n'ont qu'un niveau, resserrer :
#   topic write zigbee2mqtt/+/set
#   topic write zigbee2mqtt/+/get
```

`persistence false` et `max_queued_messages` vont avec les sessions propres du § 8.4.3 : le
courtier n'a rien à sauver. Ce point compte pour la haute disponibilité (§ 18.1.5) : un courtier
sans état n'a pas besoin d'être répliqué.

Bloc technique — `moli.toml`, le pilote avec un compte (la clé du mot de passe passe au coffre, proposé) :

```toml
[[driver]]
id = "z2m"
kind = "z2m"

[driver.options]
host = "127.0.0.1"
port = 1883
base_topic = "zigbee2mqtt"
username = "moli-os"
# Aujourd'hui : password_env = "MOLI_Z2M_PASSWORD" (variable d'environnement).
# Proposé : le mot de passe au coffre, comme les autres pilotes :
#   moli-os secrets set z2m password
# tls = true          # proposé : seulement si le courtier n'est pas sur la machine
```

### 19.2 Quand tout passe par le courtier

Mettre un courtier au centre rend la règle 9 (« le binaire fait respecter les règles ») plus
exigeante : **qui peut écrire sur le courtier peut tout.** Chaque menace a sa protection :

| Menace | Protection dans la V7 | § |
|---|---|---|
| Commander un appareil sans la garde | seul le hub écrit `cmd/` ; les modules de Sarah Home sont de confiance et le kit forme leurs sujets ; les clients extérieurs sont bornés par leur ACL ; aucun compte inconnu n'est admis | 6.2, 7.2 |
| Fabriquer de faux états | un module n'écrit que `in/<lui>/` ; le hub valide chaque valeur contre sa spécification ; seul le hub écrit `val/` | 5.3, 10.1 |
| Écouter la maison | écouteurs sur `127.0.0.1` ; celui de la secours, chiffré, avec certificat exigé ; le module WebSocket filtre par les droits | 6.4, 14.1.2, 18.2.2 |
| Un message géant, une inondation | 4 Kio par message, files bornées par client, tampon d'entrée du hub, budget par module | 6.4, 9.4.1 |
| Un ordre retenu rejoué à chaque redémarrage | aucun ordre n'est jamais retenu ; un module qui en voit un le signale et l'efface | 6.3, 19.1 |
| Un module compromis | nos modules sont de confiance : relus, bornés, et ils traitent leurs entrées comme hostiles ; un module à risque va en processus pour sa robustesse | 9.4.2 |
| Lire un code tapé sur un clavier Zigbee | un crochet d'entrée du courtier détourne le code avant le journal, vers le canal privé ; seul le module `z2m` lit l'arbre `zigbee2mqtt/` (ACL) ; sans crochet, Zigbee2MQTT garde son propre courtier | 5.1, 20.11 |
| Faire approuver son propre profil déclaratif | aujourd'hui, `moli-os profile trust` enregistre l'empreinte sans question : un agent qui a un shell approuve son fichier. La V7 exige un geste authentifié (code PIN), qui montre les correspondances du profil | 5.4 |
| Laisser un secret dans le journal (l'URL de connexion d'Alexa) | aucun code, jeton, PIN ni URL d'autorisation dans un ordre ni au journal ; ces flux restent des routes REST du module | 5.1, 22.3 |
| Un appareil MQTT du réseau local | écouteur du réseau local, compte, ACL sur son arbre, 4 Kio, débit plafonné | 6.4, 15.3 |
| Les secrets (mots de passe, certificats, clés) | au coffre de Sarah Home (règle 6), jamais en variable d'environnement ni dans la configuration | 19.1 |
| Le témoin | il ne vote que pour un bail ; il ne voit aucune donnée de la maison | 18.2.3 |

Trois points demandent de la vigilance :

1. **Le correctif d'ACL est à nous.** Il touche le routeur de `rumqttd` aux deux endroits où une
   publication et un abonnement sont acceptés. Il faut le garder petit, le tester à chaque
   version reprise, et mesurer qu'il ne coûte rien aux modules internes, qui ne passent pas par
   lui (§ 11.4).
2. **`rumqttd` a des défauts mesurés** : retenus tronqués sans signal, pertes silencieuses au-delà
   de ~1 400 messages de retard, une panique de connexion qui empoisonne un verrou (observée sous
   macOS). La V7 les contourne (le rang `s` et le rang `n`, l'instantané par le hub, la sentinelle)
   et notre version doit les corriger.
3. **La secours lit tout.** Elle est un autre Sarah Home, même binaire, même opérateur ; son compte est
   en lecture seule et son certificat signé par l'autorité de Sarah Home seule. Une secours compromise
   lirait la maison entière : c'est le prix d'une secours chaude, et il vaut pour tout système à
   haute disponibilité.

## 20. Les contrats

Ce que chaque brique du pattern devrait exposer, avec ses règles. Ce sont des esquisses pour cadrer
l'implémentation, pas du code livré ; les noms sont en français comme le reste du domaine. Les
contrats 15.1 à 15.6 viennent de la V4 ; 15.7 à 15.10 sont nouveaux.

### 20.1 `spawn_driver_isole` : un thread et un runtime par pilote

Bloc technique — `moli-runtime`, à côté de `spawn_driver` :

```rust
/// Comme `spawn_driver`, mais sur un thread dédié portant son propre runtime Tokio
/// (`current_thread`). Le superviseur, la relance avec attente croissante et le statut
/// public sont inchangés. Le `Handle` du cœur est donné au pilote pour ce qui doit y vivre.
pub fn spawn_driver_isole(
    hub: &Hub,
    instance: InstanceId,
    driver: Arc<dyn Driver>,
    cancel: CancellationToken,
) -> std::thread::JoinHandle<()>;
```

Règles :

- le thread porte le nom du pilote (`pilote-<instance>`), lisible dans un rapport de gel ;
- la pile est celle par défaut : 2 Mio réservés, ~19 Kio touchés au repos sous Linux musl ;
- une panique dans une sous-tâche du pilote fait sortir `run`, donc relancer le pilote ;
- le hub reçoit le `Handle` du cœur à sa construction (`HubOptions::coeur`) et y lance ses
  propres tâches. Un pilote isolé ne porte jamais une tâche du hub.

### 20.2 `Voies` : une file par appareil, à la demande

Bloc technique — le répartiteur d'un pilote :

```rust
pub struct Voies<E: Envoi> { /* répartiteur unique */ }

pub trait Envoi: Send + Sync + 'static {
    /// Envoie un ordre à un appareil ; rend le résultat à la voie, qui répond au cœur.
    fn envoyer(&self, appareil: &DeviceId, ordre: CommandRequest) -> BoxFuture<'static, Issue>;
}

impl<E: Envoi> Voies<E> {
    pub fn new(envoi: E, reglage: Reglage) -> Self;
    /// Envoyé tout de suite s'il n'y a rien en vol pour l'appareil, sinon mis en attente ;
    /// refusé sans attendre si le disjoncteur de l'appareil est ouvert.
    pub fn ordre(&mut self, ordre: CommandRequest);
    /// À appeler dans le `select!` du pilote : fait avancer les files.
    pub async fn tour(&mut self) -> Option<Evenement>;
}

pub struct Reglage {
    pub attente_max: usize,          // 8 : au-delà, la plus ancienne consigne cède
    pub en_vol_max: usize,           // 1 : un seul ordre en vol par appareil
    pub echecs_avant_ouverture: u8,  // 2
    pub ouverture: Duration,         // 30 s, doublée à chaque réouverture, 10 min au plus
}

pub enum Evenement {
    /// Le disjoncteur d'un appareil s'ouvre ou se referme : le pilote publie la disponibilité.
    Disponibilite { appareil: DeviceId, en_ligne: bool },
}
```

Règles :

- une file n'existe que pour un appareil qui a un ordre en attente, et disparaît vide ;
- avant chaque envoi, `ordre.abandoned()` : un ordre que personne n'attend plus ne part pas ;
- une consigne qui attend est remplacée par la suivante pour le même point, **si aucun appui ne
  les sépare** ; un appui ou une impulsion n'est jamais remplacé ; le module déclare l'effet de chaque
  point (`consigne` ou `appui`, § 5.1), et une consigne écartée rend `non_envoye`, cause `remplace` ;
- la clé de la voie est choisie par le module : l'appareil, la box, le compte, la station ;
- un appareil `reveillable` qui dort n'ouvre pas son disjoncteur : l'ordre part (Wake-on-LAN) ;
- à l'ouverture du disjoncteur, la file de l'appareil est vidée avec un refus immédiat ;
- l'essai de réouverture est un ordre réel (le prochain), jamais une requête fabriquée.

### 20.3 `Passerelle` : une API partagée

| Pièce | Rôle |
|---|---|
| `places: Semaphore` | la concurrence acceptée par le pont ou le service |
| tourniquet | équité entre appareils : un appareil n'occupe qu'une place |
| priorité | un ordre passe avant une lecture périodique |
| `SeauAJetons` | le quota du fournisseur (cloud) |
| `DisjoncteurDeBus` | ferme la passerelle entière |

Règles :

- un ordre attend une place au plus `echeance − marge`, sinon il est refusé « occupé » tout de
  suite ;
- le sondage cède sa place à un ordre et reprend après.

### 20.4 `DisjoncteurDeBus` : à hystérésis, avec budget de latence

| Paramètre | Valeur proposée | Pourquoi |
|---|---|---|
| Ouverture | 3 échecs **consécutifs** d'ordres, ou le signal du transport : déconnexion MQTT, `bridge/state` = `offline`, pont injoignable | un seul appareil muet ne doit pas ouvrir le bus ; un bus partiel reste utilisable |
| Durée minimale ouvert | 5 s | un bus qui bat ne fait pas osciller la maison (Q2) |
| Essai | un ordre réel, délai court (500 ms), intervalle 1 s doublé jusqu'à 30 s | reprise en 0,5 s après le retour (P1), sans marteler un bus mort |
| Fermeture | 3 succès consécutifs, ou le signal de reconnexion du transport (ConnAck, `bridge/state` = `online`) | un essai réussi par hasard ne rouvre pas la vanne |
| Dégradé | médiane des 20 dernières latences > 1,5 s | un bus lent sous le délai (Q1) est traité avant que les files ne débordent |
| En dégradé | refus prédictif ; délai par ordre ramené à 1 s ; un seul ordre en attente par appareil ; sondage suspendu | la latence des ordres reste bornée, le workflow sait vite |
| Effet visible | tous les appareils du bus hors ligne ; ordres refusés en 0,1 ms ; notice à la famille ; raison dans `/api/health` | la panne se voit, elle ne se devine pas |

### 20.5 Le tampon d'entrée du hub

Bloc technique — le budget d'entrée par pilote :

```rust
pub struct BudgetEntree {
    pub par_pilote: u32,     // 200 événements par seconde par pilote, au-delà on fusionne
    pub fusion: Duration,    // 10 ms : états et mesures fusionnés par point, la dernière gagne
}
```

Règles :

- sous le budget, rien ne change : chaque `set_state` est publié comme aujourd'hui ;
- au-delà, les `State` d'un même point sont fusionnés sur une fenêtre de 10 ms ; la minuterie
  n'est armée que si le tampon n'est pas vide ;
- jamais fusionnés : `pulse_state` (appuis), occurrences, expirations, `DeviceUpserted`,
  `Availability` ;
- un pilote au-delà du budget plus de 10 s de suite est signalé (journal, santé).

### 20.6 Le journal, l'instantané, l'époque

| Pièce | Forme | Règle |
|---|---|---|
| Journal | une ligne binaire par changement : `seq u64, point u32, charge, instant u64` (~9 octets compressés) ; validé par lots (≤ 100 ms ou 500 lignes) | la source de la reprise et de la réplication ; jamais réécrit |
| Instantané | binaire compact de l'état vif `(point u32, étiquette u8, charge, 3 instants)`, avec le `seq` du journal | au quart d'heure sur disque ; chaque seconde vers le réseau si une secours écoute |
| Baux et exécutions | dans le journal (ouverture, prolongation, fin) et dans l'instantané | repris au démarrage, appliqués une fois |
| Époque | entier monotone, écrit sur disque **avant** toute activation, annoncé à la connexion | la plus grande gagne ; l'autre reste passive |
| Secours | reçoit instantané et journal ; applique en mémoire ; **n'ouvre aucune connexion** | bascule sur décision (humain, ou témoin et bail : § 18.1.7), après la clôture de l'ancienne |

### 20.7 L'issue d'un ordre (nouveau)

| Règle | Détail |
|---|---|
| Neuf issues | `Confirme`, `Execute`, `Accepte`, `Envoye`, `Incertain`, `NonEnvoye`, `Echec`, `Refuse`, `Retenue` (§ 5.6) ; la V5 en proposait quatre (§ 8.4.2) |
| Confirmation | la valeur canonique revenue égale la valeur demandée, à la tolérance du point près, avant l'échéance |
| Retour | déclaré par le module pour chaque point écrivable, **par code** d'énumération : `etat` (point, valeurs attendues, tolérance, à l'arrêt), `pont`, `accuse`, `aucun` ; le kit borne l'issue par le retour (un volet RTS ne rend jamais `Confirme`) ; le module juge sur sa lecture brute, avant déduplication |
| Journal | l'observateur écrit l'issue finale, une ligne par ordre, comme aujourd'hui |
| Raison | pour `refuse`, `echec` et `non_envoye` : `r`, un code d'un vocabulaire fermé par domaine (`sans_reponse`, `liaison`, `remplace`, `voie_pleine`, `module`, `partiel`…), et `d`, un détail de 200 o au plus, jamais un secret ni un texte de l'appareil recopié |
| Suivi | après `Accepte`, `Envoye` ou `Incertain`, la suite est publiée une fois sur `evt/`, avec l'identifiant de la demande ; un suivi expiré sans nouvelle prévient la maison, une fois (§ 5.3) |
| Workflow | une étape peut brancher sur `Envoye`, `Incertain`, `NonEnvoye` et `Echec`, comme sur un délai ; la validation refuse d'attendre un état qu'un point ne renvoie jamais |
| API | un champ `issue` ajouté à la réponse de `POST /api/command` et de `send_command` ; l'existant ne change pas |

### 20.8 Les options MQTT d'un pilote (nouveau)

| Option | Valeur | Pourquoi |
|---|---|---|
| Session | **propre** (`clean_session = true`), client recréé à chaque erreur | aucun ordre retransmis ni différé (§ 8.4.3) |
| Maintien de liaison | 30 s | coût négligeable, détection en 45 s au pire côté courtier |
| Taille de paquet | entrée et sortie **écrites des deux côtés** : pilote et courtier | Mosquitto 2.1 coupe au-delà de 2 000 000 o par défaut |
| Ordres | QoS 1, jamais retenus ; en MQTT 5, expiration = reste de l'échéance | — |
| Retenus reçus | un `…/set` retenu est signalé et effacé | § 19.1.3 |
| Signal du pont | `bridge/state` (le testament MQTT) lu et relié au disjoncteur | § 18.1.2 |
| Secret | mot de passe au coffre, plus en variable d'environnement | règle 6 |
| TLS | obligatoire si le courtier n'est pas sur la machine | règle 10 |

### 20.9 Le rôle des ponts, attaché à l'époque (nouveau)

Bloc technique — ce que Sarah Home fait des ponts quand l'époque change (esquisse) :

```rust
/// Un service externe qui porte une identité (Zigbee2MQTT, contrôleur Matter) :
/// il ne tourne que sur l'hôte qui porte l'époque.
pub trait Pont: Send + Sync + 'static {
    fn nom(&self) -> &'static str;
    /// Les fichiers qui partent vers la secours (données du pont, sauvegarde du coordinateur).
    fn fichiers(&self) -> Vec<PathBuf>;
    /// Démarrer après l'écriture de l'époque ; prêt quand le pont a publié son état « online ».
    fn demarrer(&self, epoque: u64) -> BoxFuture<'_, anyhow::Result<()>>;
    /// S'arrêter avant la fin du bail ; ne rend la main qu'une fois le processus terminé.
    fn arreter(&self) -> BoxFuture<'_, anyhow::Result<()>>;
}

pub enum Cloture {
    /// Couper l'alimentation d'un boîtier radio par son port PoE (switch pilotable).
    PortPoe { switch: String, port: u16 },
    /// L'hôte est injoignable pour le témoin et pour la secours, et son bail a expiré.
    BailExpire,
}
```

Règles :

- ordre d'activation : clôture, époque N+1 sur disque, Sarah Home, courtier, ponts ;
- ordre d'arrêt (rétrogradation) : ponts, puis refus des ordres ; Sarah Home garde son état vif ;
- les fichiers d'un pont partent vers la secours à chaque changement et à l'arrêt propre ;
- la secours ne démarre jamais un pont tant qu'elle ne porte pas l'époque.

### 20.10 L'API d'historique (nouveau)

| Règle | Détail |
|---|---|
| Une courbe | `GET /api/history` : 1 min à 1 an, 10 à 2 000 points, 300 par défaut ; brute si le nombre de changements tient, sinon des seaux avec moyenne, min, max et nombre ; compressée |
| Les points demandés | ceux de la largeur du graphique en pixels, pas plus (§ 12.4.3) |
| Au-delà de la fenêtre locale | l'algorithme D1 du stockage V3 : heures et jours par l'archive, brut local, valeur vive en dernier point, archive injoignable signalée |
| Un export | un autre chemin, en pages ou en flux (NDJSON ou CSV), borné, repris par curseur ; jamais une réponse unique de plusieurs Mo |
| Le temps réel | jamais par l'historique : la courbe se prolonge avec les valeurs du direct (le flux SSE aujourd'hui, la WebSocket du § 14.1 dans la V7) |

### 20.11 Les saisies privées (nouveau)

| Règle | Détail |
|---|---|
| Ce qui est privé | les champs que `moli_core::privacy` réserve : codes de clavier, `pin_code.*`, codes temporaires, commandes de serrure qui reçoivent un mot de passe ; une nouvelle propriété de ce genre étend la liste et ses tests |
| Le chemin | du module au hub par `saisie_privee`, en mémoire, vers le seul consommateur inscrit (l'alarme) ; aucune route REST, MCP ni MQTT |
| Sur le courtier | le message brut de Zigbee2MQTT n'est lu que par le module `z2m` (ACL) ; aucun retenu n'est gardé ; le journal du filtre est court, jamais écrit sur disque |
| Ce qui est publié | le fait seul, sur `evt/` : « code saisi au clavier », avec l'identité du clavier ; jamais le code, l'action brute ni la transaction |
| Les bornes | file de 16 ; code et action de 64 octets au plus ; un refus est compté sans garder le contenu ; aucune troncature |
| Si l'alarme passe un jour en processus à part | ce canal ne passe toujours pas par le courtier : une socket Unix dédiée, avec les mêmes règles |

## 21. Le modèle de coût

Des coefficients unitaires, tirés des mesures, pour chiffrer n'importe quelle maison. Notations :
`V` valeurs (points), `A` appareils, `N` automatismes, `R` relevés par jour, `P` pilotes, `E`
écrans ouverts.

### 21.1 La mémoire

| Poste | Coefficient | D'où il vient |
|---|---|---|
| Modèle du hub (ce qui existe) | **463 o × V** | V4 : 1,00 Mio pour 2 264 valeurs |
| Étage vif (précédente, 3 instants, qualité, index) | **196 o × V** | V3 B1 |
| Santé des appareils | 25 o × A | V3 B1 |
| Automatismes compilés | **520 o × N** (le moteur actuel : 9 640 o × N) | V4 |
| Exécutions en cours, baux | ~410 o et ~64 o chacun | V4 |
| Agrégats 1 h + 24 h d'un point lu par une règle | ~1 000 o par point | V3 B1 |
| Historique compact en mémoire | **8,1 à 8,9 o × R × jours** | V3 B1 |
| Voies paresseuses | 6 Kio + 200 o par ordre en attente | V4 |
| Tampon du bus d'événements | 128 Kio | 1 024 × 128 o |
| Thread et runtime par pilote | **19 Kio × P** (Linux musl) | V3 |
| Processus par pilote | 577 Kio × P (processus minimal) | V3 |
| Flux SSE d'un écran | **39 Kio × E** (couche de compression comprise) | V5 T4 |
| WebSocket d'un écran | 11 Kio × E réglé, 138 Kio × E par défaut | V5 T4 |
| **Mémoire anonyme réelle** | **≈ 1,2 Mio fixes + 1,05 × les octets demandés** | V5 `etat_vif` : 3,7 Mio réels pour 2,34 demandés à ×1, 8,1 pour 6,7 à ×3, 24,1 pour 22,4 à ×10 |

Trois maisons, profil sobre (état vif, 24 h d'historique, 40 pilotes) :

| Maison | V / A / N | État vif demandé | Réel, état vif | + 24 h | + threads | **Réel, profil sobre** |
|---|---|---|---|---|---|---|
| Petite (un appartement) | 250 / 50 / 30 | 0,36 Mio | ~1,6 Mio (estimé) | +0,16 Mio | +0,74 Mio | **~2,5 Mio** |
| Les deux maisons réunies | 2 264 / 496 / 400 | 2,34 Mio (mesuré) | **3,7 Mio (mesuré)** | +1,46 Mio | +0,74 Mio | **~5,9 Mio** |
| ×3 | 6 792 / 1 488 / 1 200 | 6,70 Mio (mesuré) | **8,1 Mio (mesuré)** | +4,4 Mio | +0,74 Mio | **~13 Mio** |
| ×10 | 22 640 / 4 960 / 4 000 | 22,4 Mio (mesuré) | **24,1 Mio (mesuré)** | +15,0 Mio | +0,74 Mio | **~40 Mio** |

Tout le profil sobre tient dans le budget de 32 Mio de mémoire anonyme de PERFORMANCE.md jusqu'à
trois fois les deux maisons, avec le reste du cœur (4 à 7 Mio mesurés aujourd'hui). À ×10, il faut
une machine plus grande qu'un Pi Zero 2 W, ou un historique récent sur disque seulement.

### 21.2 Les octets sur le fil

| Poste | Coefficient | D'où il vient |
|---|---|---|
| Un relevé, MQTT QoS 0 | ~97 o (charge JSON de Zigbee2MQTT + 33 o) | V5 T1 |
| Un relevé vers un écran (SSE) | ~117 o par attribut, soit ×4 à ×6 le message MQTT | V5 T1 |
| Un ordre | 84 o côté MQTT ; 477 à 1 055 o pour un client HTTP, selon ses en-têtes | V5 T1 |
| L'instantané d'un écran | ~212 o × V, non compressé (480 Ko pour les deux maisons) | V5 T2 |
| Une courbe | 78 o par seau, 22 o par point brut ; ÷5 en brotli | V5 T3 |
| Maintien de liaison | SSE 45 Kio par jour et par écran ; MQTT 11 Kio par jour et par liaison | V5 T1 |

### 21.3 Le processeur

Au rythme réel des deux maisons (1,9 changement par seconde en moyenne, pointe à 239 par seconde) :

| Poste | Coût mesuré (Linux) | Au rythme moyen | À la pointe |
|---|---|---|---|
| Moteur actuel, 400 automatismes | 3,4 µs par événement | 0,0006 % d'un cœur | 0,08 % |
| Moteur compilé, 400 automatismes | 2 ns par événement | négligeable | négligeable |
| Agrégats au fil de l'eau | 5 ns par relevé | négligeable | négligeable |
| Livraison SSE à un écran | 17 à 27 µs par événement | 0,005 % par écran | 0,6 % par écran |
| Instantané SSE | 0,85 ms par connexion ou par retard | — | — |
| Instantané binaire toutes les 15 min | 0,45 ms | 0,00005 % | — |
| Journal, un `fsync` par lot de 100 ms | 1,9 ms par `fsync` | 1,9 % d'un cœur **en attente d'entrées-sorties**, pas en calcul | — |

Sur un Pi Zero 2 W, multiplier les coûts de calcul par 8 à 10 (estimé) : le moteur actuel reste
sous 1 % à la pointe, le moteur compilé est invisible, et la livraison SSE reste sous 6 % par écran
à la pointe. Le vrai coût du Pi reste la carte SD.

## 22. Le plan, de l'existant à la V7

### 22.1 Les étapes qui ne dépendent pas de MQTT

Chaque étape apporte un gain seule. L'ordre suit le rapport gain sur coût. Les étapes marquées
**nouveau** viennent de la V5 ; les autres sont celles des V3 et V4, avec leur état.

| Étape | Gain | Coût à l'exécution | Effort | Preuve d'acceptation | État |
|---|---|---|---|---|---|
| 1. Correctif de la perte sur retard | plus aucun déclencheur perdu sur un retard | ~1 ms par retard | faible | V2 W2 rejoué : 0 perdu | **prouvé** sur copie (§ 17.3) : à reporter tel quel |
| 2. `bridge/state` relié au disjoncteur du bus (**nouveau**) | une panne de Zigbee2MQTT se voit ; ordres refusés en 0,1 ms au lieu de « fait » | nul | faible | test 1 | à faire |
| 3. Sécurité du courtier (**nouveau**) : écoute locale, comptes, ACL, mot de passe au coffre, `max_packet_size` écrit, signalement des `…/set` étrangers ou retenus | la porte de service se ferme ou se voit | nul | faible | tests 11, 13, 14 | à faire |
| 4. Liaisons Zigbee directes pour les fonctions essentielles (**nouveau**) | l'essentiel marche même quand tout est tombé | nul | faible (configuration de Zigbee2MQTT) | test 15 | à faire |
| 5. Tâches du hub sur le `Handle` du cœur | horloges du hub hors des pilotes | nul | faible | horloges vivantes quand un pilote est gelé | à faire |
| 6. Tampon d'entrée du hub | inondations désamorcées | ≤ 15 ms en rafale | faible | P2 rejoué : 0 perdu | confirmé sous Linux |
| 7. Voies paresseuses et disjoncteurs (hystérésis, refus prédictif, réessais idempotents) | appareils et bus cloisonnés, bus capricieux tenus | < 0,6 Mio en charge, 6 Kio au repos | moyen | C2, C3, P1, Q1-Q3 rejoués | **écrites et mesurées** hors dépôt |
| 8. Un thread par pilote | blocages et calculs isolés | 19 Kio par pilote | moyen | C1 rejoué | indispensable à deux cœurs |
| 9. Issue d'un ordre confirmée par l'état (**nouveau**) | « fait » veut dire fait ; les workflows branchent sur « non confirmé » | quelques centaines de ms de réponse (estimé) | moyen | ordre vers une lampe débranchée : « envoyé, non confirmé » ; test 12 | à faire |
| 10. Bail durable pour « pendant N minutes » | workflows temporaires fiables | ~64 o par bail ; 1,9 à 3,7 ms à l'ouverture | faible | redémarrage au milieu d'un bail | à faire |
| 11. Étage vif étendu et agrégats | conditions « depuis », moyennes sans disque | ~0,9 Mio demandés, ~1 Mio réel | moyen | lecture d'agrégat en mémoire | coût confirmé |
| 12. Moteur compilé, filtré à la source | 280 à 5 000 fois moins de CPU par événement | moins de mémoire qu'aujourd'hui | élevé | T1 de la V4 rejoué à 4 000 automatismes | plafond actuel chiffré |
| 13. Historique récent en mémoire selon le profil | courbes et reprise instantanées | 8 à 9 o par relevé | moyen | B2.3 | redémarrage à chaud 4,3 ms sous Linux |
| 14. Les quatre étages du stockage (C1 archive, C2 récent sur Turso, D1 lectures, E1 vieillissement) | écritures ÷ 12, long terme 6,7 à 38 fois plus léger | ≈ 0,26 Go écrits par jour au lieu de 3,1 | élevé | séries datées du stockage V3 | proposés (stockage V3, § 16) |
| 15. Journal, instantanés binaires, époque, secours passive | reprise sans perte, ici ou ailleurs | ~2,6 Mo écrits par jour | élevé | S1-S3 | **bascule jouée** ; reste : reconnexion des pilotes |
| 16. Ponts attachés à l'époque, fichiers des ponts répliqués, témoin et bail (**nouveau**) | la maison entière, pas seulement Sarah Home, change d'hôte | réseau : quelques Mo par jour | élevé | tests 5 à 9, 12 | à faire |
| 17. Second boîtier radio, PoE pilotable, procédure de promotion Zigbee (**nouveau**) | Thread sans coupure ; Zigbee repris en minutes | matériel | moyen | tests 2, 3, 4, 6 | à faire, après l'étape 16 |
| 18. Export d'historique en flux (**nouveau**, au besoin) | des exports de plusieurs Mo sans réponse géante | nul au repos | faible | export d'un an d'un capteur à la minute | au besoin |

Sans aucun matériel de plus, les étapes 1 à 9 rendent la maison plus réactive, plus honnête sur
ce qu'elle a fait, et moins exposée. La haute disponibilité (étapes 15 à 17) vient ensuite, et
elle s'appuie sur elles : une bascule ne remplace ni les voies, ni le disjoncteur, ni la
confirmation des ordres.

### 22.2 La migration vers MQTT au centre, par étapes livrables

Chaque étape se livre seule, garde la maison en marche, et a sa preuve. Les étapes 1 à 9 du § 22.1
(correctif du moteur, `bridge/state`, sécurité du courtier, tampon d'entrée, voies, thread par
pilote, issue confirmée…) restent valables : elles **précèdent** ou **accompagnent** celles-ci.

| Étape | Ce qui change | Ce qui ne change pas encore | Preuve d'acceptation |
|---|---|---|---|
| 1. Le courtier embarqué, en miroir | `rumqttd`, repris dans `moli-courtier`, démarre dans Sarah Home ; le hub publie `val/`, `avail/`, `meta/` (sans retenus : l'état est servi par le hub) en plus de son bus interne | tout le reste : SSE, REST, pilotes | un client MQTT voit tout l'état ; mémoire et CPU mesurés en production (§ 11.4) |
| 1 bis. Les ACL des clients extérieurs | le correctif de `moli-courtier` : comptes, motifs, quotas, lecture seule | les modules internes, qui n'y passent pas | un compte extérieur ne peut écrire hors de son arbre ; coût par message mesuré |
| 2. Le module base de données abonné | l'enregistrement de l'historique lit `val/` et comble par `s` | `moli.db`, l'archive, le format | aucune valeur manquante sur une semaine, comparée à l'enregistrement actuel |
| 3. Le module WebSocket | le tableau de bord passe en WebSocket : état, direct, historique par blocs, ordres et issues | SSE reste servi pour les anciens clients le temps de la transition | mêmes droits que SSE (tests invités), écrans au même rythme |
| 4. Le kit des modules, en façade | `DriverCtx` publie par le kit (`in/`) ; le hub lit `in/` au lieu des appels directs | le code des 37 genres de pilotes | les bancs C1-C3, P1-P2, Q1-Q3 rejoués à travers le courtier |
| 5. Zigbee2MQTT sur le courtier, avec son compte et son ACL | le module `z2m` traduit, déduplique, lit `bridge/state` | le réseau radio | tests 1, 11, 13 et 14 (§ 18.1.10) |
| 6. Les ordres par `req/`, `cmd/`, `res/` | surfaces et moteur demandent ; le hub publie l'ordre ; l'issue revient | la garde elle-même | aucun ordre sans ligne de journal ; aucun ordre rejoué après son échéance |
| 7. Le moteur de workflow abonné | il lit `val/`, demande sur `req/`, attend `res/`, resynchronise par `s` | le modèle des automatismes (celui de la maquette) | V4 F1 rejoué : 0 transition perdue ; les automatismes de la maquette joués de bout en bout |
| 8. La santé des appareils | rythme, pertes tolérées, appareils muets (§ 10.5) | — | un appareil débranché passe muet au délai prévu, et revient seul |
| 9. Les modules à risque en processus | `isolation = "processus"` pour les modules qui le méritent | les autres restent en thread | un `abort` dans le module ne fait tomber que lui |
| 10. Le moteur de haute disponibilité | secours abonnée, journal, époque, bail, témoin, ponts attachés à l'époque | — | tests 1 à 19 (§ 18.1.10, § 18.2.4) |
| 11. Le module Matter | Matter par un module api-to-mqtt | Thread | appairage, ordres et états d'un appareil Matter réel |
| 12. Les appareils MQTT du réseau local | écouteur du réseau local, comptes, ACL, débit ; profils déclaratifs MQTT | le contrat des modules et des sujets | un appareil ne peut écrire que son arbre ; une inondation est coupée au courtier |

L'ordre est choisi pour que chaque étape **ajoute** sans retirer. Le courtier vit d'abord en
miroir, puis prend un à un les rôles du bus interne. À chaque étape, on peut s'arrêter : la maison
marche.

### 22.3 Les corrections de code relevées par le crible

Le contrat les interdit déjà ; le portage de chaque pilote doit les traiter. Elles viennent du
crible (annexe D), relues dans le code ; les chemins sont relatifs à `crates/<crate>/src/`.

| Écart | Où | Correction |
|---|---|---|
| Écho optimiste : la valeur demandée publiée comme état | Tapo (`lib.rs:675-678`), Sonos (`lib.rs:381-385`) | publier la relecture, jamais la demande |
| Attente réseau dans la boucle qui répond aux ordres | Moonraker, ESPHome, IPP (jusqu'à 8 s), Tapo, Sonos (4 appels SOAP de 4 s), Philips, moteur de profils, Telegram, téléphones (15 s) | toute attente réseau à côté de la boucle, comme Overkiz et Roborock |
| Ni l'appelant ni l'échéance revérifiés avant l'envoi | 17 des 27 pilotes qui envoient des ordres | la voie et le permis du kit (§ 5.3) |
| `app` répond `Ok` tout de suite et part jusqu'à 60 s plus tard ; réveil relancé pendant 45 s ; touche Standby de repli qui **bascule** | Philips (`android.rs:28,232-240`, `lib.rs:459-467,596-611`) | `accepte` puis la suite ; retirer la bascule |
| Une boîte d'ordres de 15 min, rejouée jusqu'à « reçu » | ordinateurs (`agents.rs:22,140-149`) | une durée de vie par type d'ordre ; `accepte`, puis la suite |
| `cancel` rend `Ok` même quand `Cancel-Job` échoue | IPP (`lib.rs:571-574`) | `envoye`, ou relire l'état du travail |
| Table ARP illisible : tout le monde « absent » au bout de 10 min | présence (`lib.rs:261,284`) | sonde indisponible, jamais « absent » |
| « Non envoyé » confondu avec « peut-être parti » | Samsung, Sonos, moteur de profils ; Cast et Android TV (un seul « incertain ») | le permis du kit sépare les deux |
| L'URL de connexion d'Alexa (`sign_in`) en clair dans le journal ; `command` (voix libre) sans approbation pour un agent | Alexa (`hub.rs:973-976`, `echo.rs:82`) | `sign_in` en route REST du module, jamais un ordre ; `command` en sémantique `Control` |
| Signalés par un sous-agent, à revérifier | serrures Zigbee sans sémantique `Lock` ; Frigate `detection` et `recording` sans accord humain ; `phones.json` écrit sans droits `0600` ; `Debug` des options MQTT de Bambu qui montre le mot de passe | confirmer, puis corriger avant le portage |
| Documentation en retard | Reolink « lecture seule » (il a `say` et l'interphone), IPP « 5 min » (15 min), code PIN « à chaque fois » (cookie de 30 jours), nombre de profils, README de Bambu, réveil de Philips | mettre `docs/components/pilotes.md` et les README à jour |

## 23. Limites et questions ouvertes

### 23.1 Les limites des mesures

- **Machines de mesure.** Le conteneur Linux tourne sur le Mac : même architecture que le Pi
  (arm64), mais un cœur 8 à 10 fois plus rapide. Aucune mesure sur un Pi Zero 2 W, ni sur une
  carte SD.
- **Bancs des transports (T1 à T4).** Ils mesurent les octets et la mémoire du processus serveur. Ils ne mesurent
  ni le TLS, ni le chemin réel jusqu'à un téléphone, ni la mémoire des tampons du noyau, sauf
  mention contraire dans l'annexe A.
- **Haute disponibilité.** Rien n'a été joué sur du matériel réel : ni la restauration du
  coordinateur (adresse IEEE, compteur de trames), ni le partage du jeu de données Thread, ni le
  déplacement d'une fabric Matter. Le modèle exact du boîtier n'est pas confirmé. Les durées
  marquées « estimé » le restent jusqu'aux tests de la § 18.1.10.
- **Le témoin et le bail** sont conçus, pas mesurés. Leur reprise en 3 à 5 s est une estimation.
- **La confirmation des ordres** a un coût de latence estimé, pas mesuré.
- **Sources externes.** Seules les affirmations qui pèsent sur une décision ont été vérifiées
  (annexe C). Les autres chiffres de la discussion externe ne sont pas repris comme des faits.
- **Limites héritées.** Les bus capricieux de la V4 sont simulés par des délais. Le correctif du
  moteur est prouvé sur une copie. Le modèle de coût est linéaire. Les formes Parquet du stockage
  V3 ont été mesurées sur des valeurs synthétiques.

### 23.2 Les limites de la conception et les questions à trancher

#### 23.2.1 Limites

- **Mesures sur un conteneur**, sur le Mac (arm64, 2 CPU, 128 Mio), pas sur un Pi. Les rapports
  tiennent, les durées absolues non (V4 : un Pi Zero 2 W est estimé 8 à 10 fois plus lent).
- **`rumqttd` lu et mesuré, pas éprouvé** en longue durée ni sous des clients hostiles. Son
  comportement exact quand un abonné décroche est celui que le banc a observé (§ 11.4).
- **Le correctif d'ACL de `rumqttd`** est à écrire, à tester et à suivre à chaque version reprise.
- **Le kit des modules, la santé des appareils, le moteur de workflow abonné et le module
  WebSocket sont des esquisses**, sans code livré.
- **Le crible est une lecture du code** : aucun appareil, aucune exécution. Ne sont pas vérifiés : ce
  que `COMPLETED` veut dire pour un volet io, les états qui suivent « stop » d'un Roborock, le filtrage
  de la position pour MCP et l'historique, la portée du journal pour la connexion d'Alexa, les
  durées réelles (réveil d'un PC, trajet d'un volet, démarrage d'un robot) et les quotas des clouds.
- **La haute disponibilité** n'a été jouée qu'en partie (V4 : la bascule du hub). La secours
  abonnée, le bail, le témoin et les ponts attachés à l'époque restent à jouer (tests 6 à 9,
  16 à 19).

#### 23.2.2 Questions à trancher

| # | Question | Proposé | Ce qui en dépend |
|---|---|---|---|
| 1 | Jusqu'où reprendre `rumqttd` ? | décidé : le reprendre et le compléter (ACL, quotas, lecture seule) ; un moteur propre seulement si le correctif grossit | la sécurité des écouteurs, la maintenance (§ 6.1, 16) |
| 2 | Le module WebSocket remplace-t-il SSE tout de suite, ou après une période où les deux vivent ? | les deux pendant la transition | l'application mobile et les tablettes murales |
| 3 | Les notifications : leur texte hors MQTT, leur suivi en `evt/` ? | oui | le moteur de workflow (§ 13.4, 14) |
| 4 | Une note scolaire chiffrée : une valeur sur MQTT ? Le texte d'un devoir : jamais ? | oui, et jamais | le module des services famille (§ 16) |
| 5 | Le rythme des appareils : déclaré par le profil d'abord, appris ensuite et proposé ? | oui | la santé (§ 10.5) |
| 6 | La secours abonnée par MQTT, plus le journal pour le reste ? | oui | la haute disponibilité (§ 18.2.2) |
| 7 | Le témoin : quel appareil de la maison (NAS, routeur, Pi) ? | à choisir sur l'installation | le bail (§ 18.2.3) |
| 8 | Les profils déclaratifs MQTT (§ 5.4) : quel format exact, et qui les écrit ? | le format cible du crible (G24) ; un agent les écrit, un humain les valide par un geste authentifié, ce que `moli-os profile trust` ne fait pas encore (D5, D6) | l'intégration sans code des appareils MQTT |
| 9 | Zigbee2MQTT sur le courtier de Sarah Home, avec un crochet d'entrée qui détourne les codes des claviers avant le journal, ou sur son propre courtier ? | le crochet ; tant qu'il n'existe pas, son propre courtier, comme aujourd'hui | les codes des claviers (§ 5.1), le module `z2m` (§ 15.1) |

## Annexe A — Bancs

Tous les bancs sont **hors du dépôt**, dans des crates jetables qui dépendent par chemin des vraies
crates `moli-core`, `moli-runtime` et `moli-history`. Ils utilisent les versions du dépôt d'axum
(0.8.9), de `tower-http` (0.7.1) et de `rumqttc` (0.25.1), ainsi que `tungstenite` 0.29.0 et
`rumqttd` 0.20.0. Compilation musl dans un conteneur `rust:1-alpine`, exécution dans `alpine:3`
limité à **2 CPU et 128 Mio**, comme la production ; un processus neuf par mesure. Les chiffres
retenus sont ceux du conteneur Linux ; le Mac a servi à la mise au point.

| Banc | Crate | Ce qu'il mesure | Où il sert |
|---|---|---|---|
| `tailles` (T1) | `preuves-v5` | les octets sur le fil : MQTT encodé par `rumqttc`, SSE et WebSocket lus sur la socket d'un vrai serveur axum avec les couches de Sarah Home, `POST /api/command` avec trois jeux d'en-têtes, les maintiens de liaison | § 7.3, § 14.2 |
| `instantane` (T2) | `preuves-v5` | le JSON de `hub.snapshot()` pour les deux maisons et pour une petite maison : brut, gzip, brotli, temps de sérialisation | § 14.2.4 |
| `historique` (T3) | `preuves-v5` | de vraies `moli_history::Series` : seaux, brut, compression, temps ; contrôle sur le fil avec la compression de Sarah Home | § 12.4 |
| `connexions` (T4) | `preuves-v5` | la mémoire anonyme du processus serveur par connexion SSE ou WebSocket, de 1 à 1 000 connexions, et le CPU de diffusion | § 14.2.5 |
| `etat_vif` | `preuves-v5` | l'état vif seul, sans historique, à ×1, ×3 et ×10 : octets demandés et mémoire réelle du cgroup | § 11.5.2 |
| `courtier` | `preuves-v5/courtier` | `rumqttd` 0.20.0 embarqué : liens en mémoire contre `tokio::sync::broadcast`, mémoire des retenus et des abonnés TCP, rafale de 5 000 avec un abonné lent ou bloqué, journal sous trafic soutenu, seuil de retard et perte, coût d'une ACL par sujet | § 6, § 11.4.5 |
| `etat`, `inventaire`, `fuite`, `lecture`, `courtier` | `preuves-memoire` | ce que coûte chaque objet en mémoire aujourd'hui, le prototype compact, la fuite des sujets uniques, la lecture d'une condition, le courtier réglé ; allocateurs musl et `mimalloc` | § 11 |

Les mesures des V1 à V4 et des séries du stockage sont citées sans être rejouées
([annexe C](#annexe-c--bancs-des-v2-à-v4-séries-du-stockage-et-sources)).

Les limites des bancs :

- chaque point n'a été mesuré qu'une fois ; les queues de latence sont donc bruitées ;
- les maisons sont synthétiques, avec la forme réelle des identités de Sarah Home ; elles
  compressent mieux que du vrai JSON ;
- le profil de compilation est sans LTO ;
- les clients tournent sur la boucle locale, donc rapides : un client lent sur un vrai réseau
  garderait plus de données en tampon ;
- aucun banc n'a touché une vraie box TaHoma ni un vrai aspirateur : les exemples du § 5.7 et du
  § 5.8 suivent les pilotes et leurs tests, pas une mesure.

Une observation du vrai courtier de Home Assistant a été préparée (lecture seule, statistiques
anonymes), mais n'a pas pu avoir lieu : depuis la machine des bancs, le réseau local répond
« No route to host » à tous les outils (terminal, Docker, Codex), signe d'un accès au réseau local
refusé par macOS. Elle reste à faire, une fois l'accès ouvert.

Le code complet et les sorties brutes suivent, repliés.

Sortie brute — `tailles` (Linux) :

`````text
# Banc `tailles` : octets sur le fil (application ; TCP/IP/TLS en note)
- plateforme : linux / aarch64 ; axum 0.8.9, tungstenite 0.29 (via axum `ws`), rumqttc 0.25.1, serde_json compact
- « encodé » = octets produits par `rumqttc` (PUBLISH/PUBACK) ; « mesuré » = lu sur une socket TCP brute d'un vrai serveur axum avec la pile de Moli (`CompressionLayer` + en-têtes de sécurité de `guard.rs`) ; « calculé » = formule, vérifiée par `assert` contre l'encodé/mesuré quand les deux existent
- ids fictifs : instance `z2m`, appareils `z2m:0x00124b00aaaa000N` ; friendly names `thermometre_salon`, `detecteur_cuisine`, `prise_1`, `lampe_1` ; horodatage 1792675795123 (13 chiffres)

## 1. MQTT : PUBLISH encodé par `rumqttc` (octets du paquet, sans TCP/IP/TLS)

Topic `zigbee2mqtt/<friendly_name>` (état publié par Z2M) ; pour l'ordre lampe, topic `zigbee2mqtt/lampe_1/set` (ce que le pilote Moli publie, QoS 1). MQTT 5 sans propriétés (1 octet de longueur de propriétés).

| message | charge JSON | topic | v3.1.1 QoS 0 | v3.1.1 QoS 1 | + PUBACK | v5 QoS 0 | v5 QoS 1 | + PUBACK v5 | QoS 1 aller-retour (v3.1.1) |
|---|---|---|---|---|---|---|---|---|---|
| thermomètre | 64 o | 29 o (`zigbee2mqtt/thermometre_salon`) | **97 o** | 99 o | 4 o | 98 o | 100 o | 4 o | 103 o |
| détecteur | 50 o | 29 o (`zigbee2mqtt/detecteur_cuisine`) | **83 o** | 85 o | 4 o | 84 o | 86 o | 4 o | 89 o |
| prise + mesure | 74 o | 19 o (`zigbee2mqtt/prise_1`) | **97 o** | 99 o | 4 o | 98 o | 100 o | 4 o | 103 o |
| ordre lampe | 14 o | 23 o (`zigbee2mqtt/lampe_1/set`) | **41 o** | 43 o | 4 o | 42 o | 44 o | 4 o | 47 o |

Surcoût fixe d'un PUBLISH v3.1.1 QoS 0 = 1 (en-tête fixe) + 1 (longueur restante, ≤ 127) + 2 (longueur du topic) + topic ; QoS 1 : +2 (identifiant) ; MQTT 5 : +1 (propriétés).
Avec l'identifiant Z2M par défaut en friendly name (`zigbee2mqtt/0x00124b00aaaa0001`, 30 o) au lieu d'un nom court, le topic ne varie que de quelques octets (ex. thermomètre : 29 → 30 o).

## 2. Moli → navigateur : SSE (vrais `Event::State`, un par attribut décodé)

En-têtes de la réponse `GET /api/events` (une fois par connexion) : **297 o**
```
HTTP/1.1 200 OK
content-type: text/event-stream
cache-control: no-cache
x-frame-options: DENY
x-content-type-options: nosniff
content-security-policy: frame-ancestors 'none'
referrer-policy: strict-origin-when-cross-origin
transfer-encoding: chunked
date: Thu, 22 Oct 2026 15:45:04 GMT

```

Cadrage axum 0.8.9 (`response/sse.rs`) : `event: event\n` (13 o) + `data: ` (6 o) + JSON + `\n` + `\n` final ; chaque événement est un `Frame::data` donc un chunk HTTP/1.1 `<taille hex>\r\n<événement>\r\n`.

| message | événements | JSON (somme) | JSON moyen | SSE cadré (somme) | + chunked = **sur le fil** | par événement | rapport au PUBLISH QoS 0 |
|---|---|---|---|---|---|---|---|
| thermomètre | 4 | 359 o | 89.8 o | 443 o | **467 o** | 116.8 o | 4.8× |
| détecteur | 3 | 271 o | 90.3 o | 334 o | **352 o** | 117.3 o | 4.2× |
| prise + mesure | 5 | 443 o | 88.6 o | 548 o | **578 o** | 115.6 o | 6.0× |
| ordre lampe | 1 | 87 o | 87.0 o | 108 o | **114 o** | 114.0 o | 2.8× |

Exemple d'événement (thermomètre, attribut `temperature`) : `{"type":"state","point":"z2m:0x00124b00aaaa0001/temperature","value":21.5,"ts":1792675795123}`

L'ordre lampe produit aussi un `Event::Journal` (une ligne par ordre, diffusée au foyer) : `{"type":"journal","entry":{"id":4217,"ts":1792675795123,"origin":"ui","actor":"192.168.1.23","action":{"kind":"command","point":"z2m:0x00124b00aaaa0004/state","value":true},"outcome":{"result":"ok"},"duration_ms":62}}` → JSON 217 o, sur le fil (SSE + chunked) **244 o**.

Comparaison hypothétique (non implémentée dans Moli) : un seul événement par appareil, `{"type":"device_state","device":…,"state":{…attributs…},"ts":…}` :

| message | événement unique hypothétique (JSON) | sur le fil (SSE + chunked) | contre mesuré par attribut |
|---|---|---|---|
| thermomètre | 149 o | 176 o | 467 o |
| détecteur | 135 o | 162 o | 352 o |
| prise + mesure | 159 o | 186 o | 578 o |
| ordre lampe | 99 o | 126 o | 114 o |

## 3. WebSocket : le même JSON (trames serveur→client lues sur le fil, axum `ws` / tungstenite)

Poignée de main minimale (une fois par connexion) : requête 156 o, réponse `101` **321 o** :
```
HTTP/1.1 101 Switching Protocols
connection: upgrade
upgrade: websocket
sec-websocket-accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=
x-frame-options: DENY
x-content-type-options: nosniff
content-security-policy: frame-ancestors 'none'
referrer-policy: strict-origin-when-cross-origin
date: Thu, 22 Oct 2026 15:45:04 GMT

```

Un navigateur ajoute `Origin`, `User-Agent`, `Accept-*`, `Sec-WebSocket-Extensions`… (≈ +300 o) ; hors du sujet « message ».

| message | événements | JSON (somme) | en-têtes de trame (2 o ≤ 125 o de charge) | **trame serveur→client (somme)** | par événement | client→serveur (+4 o de masque) | rapport au PUBLISH QoS 0 |
|---|---|---|---|---|---|---|---|
| thermomètre | 4 | 359 o | 8 o | **367 o** | 91.8 o | 383 o | 3.8× |
| détecteur | 3 | 271 o | 6 o | **277 o** | 92.3 o | 289 o | 3.3× |
| prise + mesure | 5 | 443 o | 10 o | **453 o** | 90.6 o | 473 o | 4.7× |
| ordre lampe | 1 | 87 o | 2 o | **89 o** | 89.0 o | 93 o | 2.2× |

En-tête de trame calculé : 2 o si charge ≤ 125 o, 4 o si ≤ 65 535 o, 10 o au-delà (vérifié sur les trames lues ci-dessus ; le snapshot de 480 Kio prendrait 10 o). Aucune compression par message (`permessage-deflate` n'est pas négocié par tungstenite).

## 4. `POST /api/command` (HTTP/1.1 keep-alive, requête et réponse mesurées)

Corps réel (`CommandBody` : `point`, `value`, `actor?`, `request_id?`) tel que le tableau de bord l'envoie (`JSON.stringify({ point, value })`) : `{"point":"z2m:0x00124b00aaaa0004/state","value":true}` (53 o).

| jeu | en-têtes de la requête | requête (o) | dont en-têtes | réponse : en-têtes | réponse : corps | réponse (o) | requête + réponse |
|---|---|---|---|---|---|---|---|
| A | agent/script : Host, Content-Type, Content-Length, X-Moli-Origin | 178 | 125 | 288 | 11 | 299 | **477** |
| B | tableau de bord, même origine, session ouverte (en-têtes strictement nécessaires + ceux que le navigateur impose : Origin, Sec-Fetch-Site, Cookie, Connection) | 349 | 296 | 288 | 11 | 299 | **648** |
| C | navigateur Chromium complet (+ User-Agent, sec-ch-ua*, Accept*, Referer, Sec-Fetch-Mode/Dest) | 756 | 703 | 288 | 11 | 299 | **1055** |

Jeu B détaillé (requête envoyée) :
```
POST /api/command HTTP/1.1
Host: moli.local:8790
Connection: keep-alive
Content-Length: 53
Content-Type: application/json
X-Moli-Origin: ui
Origin: http://moli.local:8790
Sec-Fetch-Site: same-origin
Cookie: moli_session=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef

{"point":"z2m:0x00124b00aaaa0004/state","value":true}```
Réponse reçue (octets réels de la pile Moli : en-têtes axum par défaut + `date` hyper + en-têtes de sécurité de `guard::headers` ; pas de `vary` car `CompressionLayer` ne l'ajoute que si le corps dépasse 32 o, seuil de `DefaultPredicate`) :
```
HTTP/1.1 200 OK
content-type: application/json
x-frame-options: DENY
x-content-type-options: nosniff
content-security-policy: frame-ancestors 'none'
referrer-policy: strict-origin-when-cross-origin
cache-control: no-store
content-length: 11
date: Thu, 22 Oct 2026 15:45:05 GMT

{"ok":true}
```
Pour comparaison, la même réponse d'un axum nu (sans les couches de Moli) : en-têtes 108 o + corps 11 o = **119 o**.

## 5. Maintiens de liaison

SSE : `KeepAlive::default()` d'axum 0.8.9 = intervalle **15 s** (`KeepAlive::new`, `Duration::from_secs(15)`), texte **vide** : commentaire `:\n\n` (`Event::DEFAULT_KEEP_ALIVE`, 3 o). Mesuré sur un flux sans événement : premier maintien après 15.00 s, charge ":\n\n" (3 o), sur le fil avec le chunk **8 o** (en-têtes de réponse de ce flux : 297 o). Le délai est remis à zéro à chaque événement : un foyer bavard n'émet aucun maintien.
MQTT : `keep_alive` 30 s du pilote (`options.set_keep_alive(30 s)`) → PINGREQ (2 o, `C0 00`) + PINGRESP (2 o, `D0 00`) toutes les 30 s lorsque la liaison est silencieuse (même taille en MQTT 5).

| liaison | cycle | octets applicatifs / cycle | cycles / jour | **applicatif / jour** | sur le fil, IPv4+TCP (52 o/segment), 1 message = 1 segment, ACK compris | sur le fil + TLS 1.3 (22 o/enregistrement) |
|---|---|---|---|---|---|---|
| MQTT (1 liaison Moli↔courtier) | PINGREQ + PINGRESP / 30 s | 4 | 2880 | **11520 o (11.2 Kio)** | 160 o/cycle → 460800 o/jour (450 Kio) | 204 o/cycle → 587520 o/jour (574 Kio) |
| SSE (par navigateur connecté) | commentaire `:` / 15 s | 8 | 5760 | **46080 o (45.0 Kio)** | 112 o/cycle → 645120 o/jour (630 Kio) | 134 o/cycle → 771840 o/jour (754 Kio) |

Nota : pas de maintien SSE quand un événement circule moins de 15 s avant (timer remis à zéro) ; MQTT de même (le PINGREQ n'est émis qu'après `keep_alive` sans autre paquet sortant). Les colonnes « sur le fil » sont CALCULÉES, pas mesurées.

## 6. Bilan d'un ordre de lampe, de bout en bout (octets applicatifs ; jeu B pour la requête ; état de retour = `{"state":"ON"}` en QoS 0)

| étape | octets |
|---|---|
| navigateur → Moli : `POST /api/command` (jeu B) | 349 |
| Moli → navigateur : réponse 200 | 299 |
| Moli → Z2M : PUBLISH `…/lampe_1/set` QoS 1 (v3.1.1) | 43 |
| courtier → Moli : PUBACK | 4 |
| Z2M → Moli : état `{"state":"ON"}` sur `zigbee2mqtt/lampe_1`, QoS 0 | 37 |
| Moli → chaque navigateur : SSE `Event::State` (chunk compris) | 114 |
| Moli → chaque navigateur : SSE `Event::Journal` (chunk compris) | 244 |
| **total côté MQTT (trois paquets)** | **84** |
| **total côté HTTP/SSE pour 1 navigateur** | **1006** |

## Notes : ce qui s'ajoute sous l'application (non compté ci-dessus)

- IPv4 : 20 o ; TCP : 20 o + options (horodatage : +12 o → 32 o, courant sous Linux) ; Ethernet : 14 o (+ 4 o de FCS).
- TLS 1.3 : un enregistrement par message ou groupe d'écritures = 5 o d'en-tête + 1 o de type interne + 16 o d'étiquette AEAD = 22 o (TLS 1.2 AES-GCM : 5 + 8 de nonce explicite + 16 = 29 o).
- Exemple : un PUBLISH QoS 0 de thermomètre (97 o) seul dans un segment = 149 o sur un LAN en clair (+ 22 o sous TLS 1.3) ; l'ACK de l'autre côté ajoute un segment vide de 52 o (+ 14 o d'Ethernet chacun).
- Rafale : plusieurs événements SSE ou trames WebSocket écrits ensemble partagent un segment (cadrage inchangé) ; MQTT de même (plusieurs paquets par `write`).
`````

Sortie brute — `instantane` (Linux) :

`````text
# Banc `instantané` : snapshot SSE (`hub.snapshot()` en JSON)
- plateforme : linux / aarch64, 20 passes (médiane) pour les temps sérialisation et gzip, 5 passes pour brotli
- JSON compact (`serde_json::to_vec`), tel que `SseEvent::json_data` l'écrit ; gzip = flate2 niveau 6 ; brotli = crate `brotli` 9, fenêtre 22, qualité 4 (celle que `tower-http` 0.7 utilise par défaut, comme NGINX) et 11 (maximum)
- SSE n'est PAS compressé par tower-http (`DefaultPredicate` exclut `text/event-stream`) : les colonnes gzip/brotli sont ce que donnerait une route REST du même JSON

## Tailles du JSON du snapshot

| forme | appareils | valeurs | JSON brut | gzip (6) | brotli 4 (tower-http) | brotli 11 | cadré SSE (`event: snapshot…`) | brut / valeur |
|---|---|---|---|---|---|---|---|---|
| a. deux maisons, constructeur V4 (496 appareils, 2 264 valeurs) | 496 | 2264 | 480 707 o (469.4 Kio) | 17 469 o (17.1 Kio, 4 %) | 16 404 o (3 %) | 6 068 o (1 %) | 480 731 o | 212 o |
| b. petite maison, constructeur V4 (50 appareils, 250 valeurs) | 50 | 250 | 52 289 o (51.1 Kio) | 2 718 o (2.7 Kio, 5 %) | 2 814 o (5 %) | 1 725 o (3 %) | 52 313 o | 209 o |
| c. deux maisons, forme riche Zigbee2MQTT (≈ 496 appareils, 2 264 valeurs) | 496 | 2264 | 531 573 o (519.1 Kio) | 19 277 o (18.8 Kio, 4 %) | 19 434 o (4 %) | 10 676 o (2 %) | 531 597 o | 235 o |
| d. petite maison, forme riche Zigbee2MQTT (50 appareils, 250 valeurs) | 50 | 250 | 57 812 o (56.5 Kio) | 3 003 o (2.9 Kio, 5 %) | 3 021 o (5 %) | 2 181 o (4 %) | 57 836 o | 231 o |

## Temps (médiane de 20 passes)

| forme | `hub.snapshot()` (copie du modèle) | sérialisation JSON | gzip 6 | brotli 4 (tower-http) | brotli 11 |
|---|---|---|---|---|---|
| a. deux maisons, constructeur V4 (496 appareils, 2 264 valeurs) | 0.15 ms (min 0.15 / max 0.18) | 0.85 ms (min 0.79 / max 0.95) | 1.27 ms (min 1.26 / max 1.41) | 2.03 ms (min 2.01 / max 2.16) | 177.10 ms (min 175.41 / max 177.93) |
| b. petite maison, constructeur V4 (50 appareils, 250 valeurs) | 0.01 ms (min 0.01 / max 0.02) | 0.10 ms (min 0.10 / max 0.11) | 0.20 ms (min 0.19 / max 0.26) | 0.44 ms (min 0.42 / max 0.55) | 36.76 ms (min 35.81 / max 37.42) |
| c. deux maisons, forme riche Zigbee2MQTT (≈ 496 appareils, 2 264 valeurs) | 0.22 ms (min 0.21 / max 0.30) | 0.98 ms (min 0.97 / max 1.12) | 1.32 ms (min 1.20 / max 2.02) | 2.50 ms (min 2.30 / max 2.88) | 187.93 ms (min 185.69 / max 195.88) |
| d. petite maison, forme riche Zigbee2MQTT (50 appareils, 250 valeurs) | 0.02 ms (min 0.02 / max 0.03) | 0.12 ms (min 0.12 / max 0.14) | 0.18 ms (min 0.17 / max 0.26) | 0.45 ms (min 0.42 / max 0.57) | 22.87 ms (min 22.33 / max 23.58) |

## Un `Event::State` moyen de ces formes (JSON sérialisé comme `json_data`)

| forme | JSON moyen | médian | min | max | cadré SSE moyen (`event: event\ndata: …\n\n`) |
|---|---|---|---|---|---|
| a. deux maisons, constructeur V4 (496 appareils, 2 264 valeurs) | 95.4 o | 95 o | 91 o | 105 o | 116.4 o |
| b. petite maison, constructeur V4 (50 appareils, 250 valeurs) | 95.3 o | 95 o | 91 o | 103 o | 116.3 o |
| c. deux maisons, forme riche Zigbee2MQTT (≈ 496 appareils, 2 264 valeurs) | 90.5 o | 90 o | 87 o | 104 o | 111.5 o |
| d. petite maison, forme riche Zigbee2MQTT (50 appareils, 250 valeurs) | 90.9 o | 90 o | 87 o | 104 o | 111.9 o |

Nota : formes a et b = ids `maison:0x00158d00…` et clés `<nom>_<k>` (constructeur V4) ; formes c et d = ids `z2m:0x00124b00…` et clés Zigbee2MQTT réelles.
`````

Sortie brute — `historique` (Linux) :

`````text
# Banc `historique` : réponses `GET /api/history` (vraies `moli_history::Series`)
- plateforme : linux / aarch64
- JSON compact (`serde_json`, ce que `axum::Json` écrit) ; gzip = flate2 niveau 6 ; brotli = crate `brotli` 9 (qualité 4 = défaut de `tower-http` 0.7 d'après `compression/body.rs`, et 11 = maximum ; fenêtre 22) ; temps = médiane (20 passes jusqu'à 3 000 éléments, 5 passes jusqu'à 50 000, 2 au-delà)
- valeurs : température ≈ 21 °C (cycle jour/nuit ±1,2 °C, bruit capteur), 2 décimales ; ts en ms (13 chiffres) ; `avg` des buckets = somme/compte en f64 (16-17 chiffres significatifs, comme `AVG()` de SQLite)
- le serveur actuel (`store.rs`) ne renvoie `raw` que si `total ≤ points ≤ 2 000` ; les `raw` de 10 080 / 43 200 / 525 600 sont HYPOTHÉTIQUES (comparaison)

## Tailles de la réponse (corps JSON)

| série | éléments | JSON brut | octets / élément | gzip 6 | brotli 4 (tower-http) | brotli 11 |
|---|---|---|---|---|---|---|
| 300 buckets (défaut, 24 h) | 300 | 23 307 o (22.8 Kio) | 77.7 | 4 740 o (20 %) | 4 653 o (20 %) | 3 587 o (15 %) |
| 2 000 buckets (maximum, 24 h) | 2 000 | 151 109 o (147.6 Kio) | 75.6 | 27 855 o (18 %) | 28 152 o (19 %) | 20 758 o (14 %) |
| 300 buckets, `avg` arrondi à 0,01 (hypothétique) | 300 | 20 536 o (20.1 Kio) | 68.5 | 4 062 o (20 %) | 3 864 o (19 %) | 2 759 o (13 %) |
| 2 000 buckets, `avg` arrondi à 0,01 (hypothétique) | 2 000 | 134 312 o (131.2 Kio) | 67.2 | 23 685 o (18 %) | 23 164 o (17 %) | 16 287 o (12 %) |
| raw 1 440 (24 h à la minute) | 1 440 | 31 762 o (31.0 Kio) | 22.1 | 9 028 o (28 %) | 8 590 o (27 %) | 5 721 o (18 %) |
| raw 1 440, précision 0,1 °C | 1 440 | 30 465 o (29.8 Kio) | 21.2 | 7 275 o (24 %) | 6 777 o (22 %) | 4 723 o (16 %) |
| raw 10 080 (7 j) hypothétique | 10 080 | 220 943 o (215.8 Kio) | 21.9 | 61 746 o (28 %) | 61 241 o (28 %) | 38 630 o (17 %) |
| raw 43 200 (30 j) hypothétique | 43 200 | 946 293 o (924.1 Kio) | 21.9 | 262 574 o (28 %) | 265 612 o (28 %) | 165 135 o (17 %) |
| raw 525 600 (1 an) hypothétique | 525 600 | 11 510 578 o (10.98 Mio) | 21.9 | 3 189 614 o (28 %) | 3 087 952 o (27 %) | — |

## Temps (ms, médiane)

| série | sérialisation JSON | gzip 6 | brotli 4 (tower-http) | brotli 11 |
|---|---|---|---|---|
| 300 buckets (défaut, 24 h) | 0.05 | 0.35 | 0.39 | 19.3 |
| 2 000 buckets (maximum, 24 h) | 0.30 | 2.92 | 1.88 | 131.5 |
| 300 buckets, `avg` arrondi à 0,01 (hypothétique) | 0.05 | 0.32 | 0.38 | 16.6 |
| 2 000 buckets, `avg` arrondi à 0,01 (hypothétique) | 0.28 | 2.76 | 1.66 | 131.0 |
| raw 1 440 (24 h à la minute) | 0.05 | 0.68 | 0.62 | 24.7 |
| raw 1 440, précision 0,1 °C | 0.05 | 0.56 | 0.58 | 23.1 |
| raw 10 080 (7 j) hypothétique | 0.29 | 5.88 | 3.38 | 200.7 |
| raw 43 200 (30 j) hypothétique | 1.15 | 25.36 | 14.79 | 908.9 |
| raw 525 600 (1 an) hypothétique | 14.23 | 311.12 | 244.77 | — |

## Contrôle sur le fil : vrai serveur axum 0.8 + `CompressionLayer` de Moli (octets du corps, hors en-têtes)

| route | Accept-Encoding | corps reçu | content-encoding | transfer |
|---|---|---|---|---|
| 300 buckets | `identity` | 23 307 o | — | content-length 23307 |
| 300 buckets | `gzip` | 4 760 o | gzip | chunked |
| 300 buckets | `br` | 4 673 o | br | chunked |
| 300 buckets | `gzip, deflate, br, zstd` | 4 673 o | br | chunked |
| 2 000 buckets | `identity` | 151 109 o | — | content-length 151109 |
| 2 000 buckets | `gzip` | 27 915 o | gzip | chunked |
| 2 000 buckets | `br` | 28 212 o | br | chunked |
| 2 000 buckets | `gzip, deflate, br, zstd` | 28 212 o | br | chunked |
| raw 1 440 | `identity` | 31 762 o | — | content-length 31762 |
| raw 1 440 | `gzip` | 9 056 o | gzip | chunked |
| raw 1 440 | `br` | 8 618 o | br | chunked |
| raw 1 440 | `gzip, deflate, br, zstd` | 8 618 o | br | chunked |

En-têtes de réponse (identity, 300 buckets) :
```
HTTP/1.1 200 OK
content-type: application/json
vary: accept-encoding
x-frame-options: DENY
x-content-type-options: nosniff
content-security-policy: frame-ancestors 'none'
referrer-policy: strict-origin-when-cross-origin
cache-control: no-store
content-length: 23307
connection: close
date: Thu, 22 Oct 2026 14:23:59 GMT

```

Lecture : la taille « corps reçu » avec `gzip` doit égaler la colonne gzip 6 (même codec) ; avec `br`, la colonne brotli 4 (tower-http fixe `Level::Precise(4)` pour `CompressionLevel::Default`), à quelques dizaines d'octets près (flux, pas un bloc unique).
`````

Sortie brute — `connexions` (Linux) :

`````text
# Banc `connexions` (Linux, serveur : 2 CPU / 128 Mio ; clients : autre conteneur)

### sse · N = 1 · snapshot 4096 o · 10 événements/s pendant 10 s
- connexions établies et snapshot reçu : 1/1 en 0.06 s ; erreurs client : 0 ; `ulimit -n` du serveur (souple/dur) : 65536/65536 ; réponse de diffusion : `{"envoyes":100,"recepteurs":1}`
- connexions comptées par le serveur : 1 ; descripteurs ouverts : 12 ; fils : 3

| phase (mesurée par le serveur sur lui-même, Kio) | RssAnon | VmRSS | VmHWM (pic) | cgroup `anon` | cgroup `kernel` | cgroup `slab` | cgroup `sock` | cgroup `memory.current` |
|---|---|---|---|---|---|---|---|---|
| avant (0 connexion) | 372 | 1932 | 1932 | 376 | 372 | 242 | 0 | 1168 |
| avec 1 connexions inactives | 424 | 2048 | 2048 | 392 | 360 | 238 | 0 | 1256 |
| après la diffusion | 460 | 2084 | 2084 | 460 | 364 | 242 | 0 | 1052 |
| après fermeture | 412 | 2036 | 2036 | 380 | 360 | 239 | 0 | 1208 |

Par connexion (avec 1 − avant) / 1 : RssAnon **52.00 Kio**, VmRSS 116.00 Kio, cgroup `anon` 16.00 Kio, `kernel` -12.00 Kio, `slab` -4.00 Kio, `sock` 0.00 Kio, `memory.current` 88.00 Kio.

CPU du serveur (utime+stime) :
- 1 connexions inactives pendant 16 s : 10 ms ; octets reçus par les clients dans l'intervalle : 8 (maintiens SSE)
- diffusion de 100 événements (10/s × 10 s) à 1 connexions : **40 ms** ; 100/100 livraisons reçues ; 12090 o reçus par les clients (120.9 o par livraison, cadrage compris)
- soit 400.0 µs de CPU serveur par livraison et 0.4 % d'un cœur pendant les 10 s
RESULTAT {"apres":{"RssAnon":424,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":4608,"VmHWM":2048,"VmRSS":2048,"VmSize":7520,"VmStk":132,"cg_anon":392,"cg_current":1256,"cg_file":0,"cg_kernel":360,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":3668,"cg_percpu":2,"cg_shmem":0,"cg_slab":238,"cg_sock":0,"conns":1,"cpu_ms":0,"fds":12,"nofile":"65536/65536","pid":1,"stime_ms":0,"utime_ms":0},"attendus":100,"avant":{"RssAnon":372,"RssFile":1560,"RssShmem":0,"Threads":3,"VmData":4560,"VmHWM":1932,"VmRSS":1932,"VmSize":7472,"VmStk":132,"cg_anon":376,"cg_current":1168,"cg_file":0,"cg_kernel":372,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":3668,"cg_percpu":2,"cg_shmem":0,"cg_slab":242,"cg_sock":0,"conns":0,"cpu_ms":0,"fds":11,"nofile":"65536/65536","pid":1,"stime_ms":0,"utime_ms":0},"cpu_diffusion_ms":40,"cpu_inactif_ms":10,"diffusion":{"RssAnon":460,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":4608,"VmHWM":2084,"VmRSS":2084,"VmSize":7520,"VmStk":132,"cg_anon":460,"cg_current":1052,"cg_file":0,"cg_kernel":364,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":3668,"cg_percpu":2,"cg_shmem":0,"cg_slab":242,"cg_sock":0,"conns":1,"cpu_ms":50,"fds":12,"nofile":"65536/65536","pid":1,"stime_ms":30,"utime_ms":20},"ferme":{"RssAnon":412,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":4560,"VmHWM":2036,"VmRSS":2036,"VmSize":7472,"VmStk":132,"cg_anon":380,"cg_current":1208,"cg_file":0,"cg_kernel":360,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":3668,"cg_percpu":2,"cg_shmem":0,"cg_slab":239,"cg_sock":0,"conns":0,"cpu_ms":50,"fds":11,"nofile":"65536/65536","pid":1,"stime_ms":30,"utime_ms":20},"hz":10,"livres":100,"mode":"sse","n":1,"octets_diffusion":12090,"secs":10,"secs_inactif":16,"snapshot":4096,"temps_connexion_s":0.056526661}
- serveur : OOMKilled=false, état=running, journal : serveur sse sur :8800, snapshot 4096 o, nofile Some(String("65536/65536"))

### sse · N = 64 · snapshot 4096 o · 10 événements/s pendant 10 s
- connexions établies et snapshot reçu : 64/64 en 0.05 s ; erreurs client : 0 ; `ulimit -n` du serveur (souple/dur) : 65536/65536 ; réponse de diffusion : `{"envoyes":100,"recepteurs":64}`
- connexions comptées par le serveur : 64 ; descripteurs ouverts : 75 ; fils : 3

| phase (mesurée par le serveur sur lui-même, Kio) | RssAnon | VmRSS | VmHWM (pic) | cgroup `anon` | cgroup `kernel` | cgroup `slab` | cgroup `sock` | cgroup `memory.current` |
|---|---|---|---|---|---|---|---|---|
| avant (0 connexion) | 372 | 1932 | 1932 | 364 | 372 | 245 | 0 | 1172 |
| avec 64 connexions inactives | 2944 | 4568 | 4568 | 2916 | 464 | 338 | 0 | 3884 |
| après la diffusion | 2960 | 4584 | 4584 | 2964 | 532 | 400 | 0 | 3984 |
| après fermeture | 428 | 2052 | 4480 | 396 | 420 | 298 | 0 | 1288 |

Par connexion (avec 64 − avant) / 64 : RssAnon **40.19 Kio**, VmRSS 41.19 Kio, cgroup `anon` 39.88 Kio, `kernel` 1.44 Kio, `slab` 1.45 Kio, `sock` 0.00 Kio, `memory.current` 42.38 Kio.

CPU du serveur (utime+stime) :
- 64 connexions inactives pendant 16 s : 0 ms ; octets reçus par les clients dans l'intervalle : 512 (maintiens SSE)
- diffusion de 100 événements (10/s × 10 s) à 64 connexions : **170 ms** ; 6400/6400 livraisons reçues ; 773760 o reçus par les clients (120.9 o par livraison, cadrage compris)
- soit 26.6 µs de CPU serveur par livraison et 1.7 % d'un cœur pendant les 10 s
RESULTAT {"apres":{"RssAnon":2944,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":7644,"VmHWM":4568,"VmRSS":4568,"VmSize":10556,"VmStk":132,"cg_anon":2916,"cg_current":3884,"cg_file":0,"cg_kernel":464,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":3932,"cg_percpu":2,"cg_shmem":0,"cg_slab":338,"cg_sock":0,"conns":64,"cpu_ms":20,"fds":75,"nofile":"65536/65536","pid":1,"stime_ms":10,"utime_ms":10},"attendus":6400,"avant":{"RssAnon":372,"RssFile":1560,"RssShmem":0,"Threads":3,"VmData":4560,"VmHWM":1932,"VmRSS":1932,"VmSize":7472,"VmStk":132,"cg_anon":364,"cg_current":1172,"cg_file":0,"cg_kernel":372,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":3932,"cg_percpu":2,"cg_shmem":0,"cg_slab":245,"cg_sock":0,"conns":0,"cpu_ms":0,"fds":11,"nofile":"65536/65536","pid":1,"stime_ms":0,"utime_ms":0},"cpu_diffusion_ms":170,"cpu_inactif_ms":0,"diffusion":{"RssAnon":2960,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":7644,"VmHWM":4584,"VmRSS":4584,"VmSize":10556,"VmStk":132,"cg_anon":2964,"cg_current":3984,"cg_file":0,"cg_kernel":532,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":4228,"cg_percpu":2,"cg_shmem":0,"cg_slab":400,"cg_sock":0,"conns":64,"cpu_ms":190,"fds":75,"nofile":"65536/65536","pid":1,"stime_ms":100,"utime_ms":90},"ferme":{"RssAnon":428,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":4568,"VmHWM":4480,"VmRSS":2052,"VmSize":7480,"VmStk":132,"cg_anon":396,"cg_current":1288,"cg_file":0,"cg_kernel":420,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":4228,"cg_percpu":2,"cg_shmem":0,"cg_slab":298,"cg_sock":0,"conns":0,"cpu_ms":200,"fds":11,"nofile":"65536/65536","pid":1,"stime_ms":110,"utime_ms":90},"hz":10,"livres":6400,"mode":"sse","n":64,"octets_diffusion":773760,"secs":10,"secs_inactif":16,"snapshot":4096,"temps_connexion_s":0.052540308}
- serveur : OOMKilled=false, état=running, journal : serveur sse sur :8800, snapshot 4096 o, nofile Some(String("65536/65536"))

### sse · N = 1000 · snapshot 4096 o · 10 événements/s pendant 10 s
- connexions établies et snapshot reçu : 1000/1000 en 1.12 s ; erreurs client : 0 ; `ulimit -n` du serveur (souple/dur) : 65536/65536 ; réponse de diffusion : `{"envoyes":100,"recepteurs":1000}`
- connexions comptées par le serveur : 1000 ; descripteurs ouverts : 1011 ; fils : 3

| phase (mesurée par le serveur sur lui-même, Kio) | RssAnon | VmRSS | VmHWM (pic) | cgroup `anon` | cgroup `kernel` | cgroup `slab` | cgroup `sock` | cgroup `memory.current` |
|---|---|---|---|---|---|---|---|---|
| avant (0 connexion) | 400 | 1960 | 1960 | 344 | 360 | 240 | 0 | 1172 |
| avec 1000 connexions inactives | 39512 | 41136 | 41136 | 39516 | 2864 | 2650 | 0 | 42716 |
| après la diffusion | 39524 | 41148 | 41148 | 39528 | 2868 | 2651 | 0 | 42892 |
| après fermeture | 492 | 2116 | 40960 | 460 | 1272 | 1146 | 0 | 2220 |

Par connexion (avec 1000 − avant) / 1000 : RssAnon **39.11 Kio**, VmRSS 39.18 Kio, cgroup `anon` 39.17 Kio, `kernel` 2.50 Kio, `slab` 2.41 Kio, `sock` 0.00 Kio, `memory.current` 41.54 Kio.

CPU du serveur (utime+stime) :
- 1000 connexions inactives pendant 16 s : 40 ms ; octets reçus par les clients dans l'intervalle : 8000 (maintiens SSE)
- diffusion de 100 événements (10/s × 10 s) à 1000 connexions : **1720 ms** ; 100000/100000 livraisons reçues ; 12090000 o reçus par les clients (120.9 o par livraison, cadrage compris)
- soit 17.2 µs de CPU serveur par livraison et 17.2 % d'un cœur pendant les 10 s
RESULTAT {"apres":{"RssAnon":39512,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":48812,"VmHWM":41136,"VmRSS":41136,"VmSize":51724,"VmStk":132,"cg_anon":39516,"cg_current":42716,"cg_file":0,"cg_kernel":2864,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":42724,"cg_percpu":2,"cg_shmem":0,"cg_slab":2650,"cg_sock":0,"conns":1000,"cpu_ms":160,"fds":1011,"nofile":"65536/65536","pid":1,"stime_ms":110,"utime_ms":50},"attendus":100000,"avant":{"RssAnon":400,"RssFile":1560,"RssShmem":0,"Threads":3,"VmData":4560,"VmHWM":1960,"VmRSS":1960,"VmSize":7472,"VmStk":132,"cg_anon":344,"cg_current":1172,"cg_file":0,"cg_kernel":360,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":3684,"cg_percpu":2,"cg_shmem":0,"cg_slab":240,"cg_sock":0,"conns":0,"cpu_ms":30,"fds":11,"nofile":"65536/65536","pid":1,"stime_ms":20,"utime_ms":10},"cpu_diffusion_ms":1720,"cpu_inactif_ms":40,"diffusion":{"RssAnon":39524,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":48812,"VmHWM":41148,"VmRSS":41148,"VmSize":51724,"VmStk":132,"cg_anon":39528,"cg_current":42892,"cg_file":0,"cg_kernel":2868,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":44572,"cg_percpu":2,"cg_shmem":0,"cg_slab":2651,"cg_sock":0,"conns":1000,"cpu_ms":1930,"fds":1011,"nofile":"65536/65536","pid":1,"stime_ms":1080,"utime_ms":850},"ferme":{"RssAnon":492,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":4632,"VmHWM":40960,"VmRSS":2116,"VmSize":7544,"VmStk":132,"cg_anon":460,"cg_current":2220,"cg_file":0,"cg_kernel":1272,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":44572,"cg_percpu":2,"cg_shmem":0,"cg_slab":1146,"cg_sock":0,"conns":0,"cpu_ms":1990,"fds":11,"nofile":"65536/65536","pid":1,"stime_ms":1100,"utime_ms":890},"hz":10,"livres":100000,"mode":"sse","n":1000,"octets_diffusion":12090000,"secs":10,"secs_inactif":16,"snapshot":4096,"temps_connexion_s":1.119275694}
- serveur : OOMKilled=false, état=running, journal : serveur sse sur :8800, snapshot 4096 o, nofile Some(String("65536/65536"))

### ws · N = 1 · snapshot 4096 o · 10 événements/s pendant 10 s
- connexions établies et snapshot reçu : 1/1 en 0.06 s ; erreurs client : 0 ; `ulimit -n` du serveur (souple/dur) : 65536/65536 ; réponse de diffusion : `{"envoyes":100,"recepteurs":1}`
- connexions comptées par le serveur : 1 ; descripteurs ouverts : 12 ; fils : 3

| phase (mesurée par le serveur sur lui-même, Kio) | RssAnon | VmRSS | VmHWM (pic) | cgroup `anon` | cgroup `kernel` | cgroup `slab` | cgroup `sock` | cgroup `memory.current` |
|---|---|---|---|---|---|---|---|---|
| avant (0 connexion) | 372 | 1932 | 1932 | 340 | 360 | 238 | 0 | 1168 |
| avec 1 connexions inactives | 524 | 2148 | 2148 | 492 | 364 | 240 | 0 | 1292 |
| après la diffusion | 560 | 2184 | 2184 | 564 | 384 | 252 | 0 | 1332 |
| après fermeture | 424 | 2048 | 2048 | 392 | 360 | 239 | 0 | 1220 |

Par connexion (avec 1 − avant) / 1 : RssAnon **152.00 Kio**, VmRSS 216.00 Kio, cgroup `anon` 152.00 Kio, `kernel` 4.00 Kio, `slab` 2.00 Kio, `sock` 0.00 Kio, `memory.current` 124.00 Kio.

CPU du serveur (utime+stime) :
- 1 connexions inactives pendant 16 s : 0 ms ; octets reçus par les clients dans l'intervalle : 0 (maintiens SSE)
- diffusion de 100 événements (10/s × 10 s) à 1 connexions : **40 ms** ; 100/100 livraisons reçues ; 9590 o reçus par les clients (95.9 o par livraison, cadrage compris)
- soit 400.0 µs de CPU serveur par livraison et 0.4 % d'un cœur pendant les 10 s
RESULTAT {"apres":{"RssAnon":524,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":4696,"VmHWM":2148,"VmRSS":2148,"VmSize":7608,"VmStk":132,"cg_anon":492,"cg_current":1292,"cg_file":0,"cg_kernel":364,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":3852,"cg_percpu":2,"cg_shmem":0,"cg_slab":240,"cg_sock":0,"conns":1,"cpu_ms":10,"fds":12,"nofile":"65536/65536","pid":1,"stime_ms":10,"utime_ms":0},"attendus":100,"avant":{"RssAnon":372,"RssFile":1560,"RssShmem":0,"Threads":3,"VmData":4560,"VmHWM":1932,"VmRSS":1932,"VmSize":7472,"VmStk":132,"cg_anon":340,"cg_current":1168,"cg_file":0,"cg_kernel":360,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":3852,"cg_percpu":2,"cg_shmem":0,"cg_slab":238,"cg_sock":0,"conns":0,"cpu_ms":0,"fds":11,"nofile":"65536/65536","pid":1,"stime_ms":0,"utime_ms":0},"cpu_diffusion_ms":40,"cpu_inactif_ms":0,"diffusion":{"RssAnon":560,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":4696,"VmHWM":2184,"VmRSS":2184,"VmSize":7608,"VmStk":132,"cg_anon":564,"cg_current":1332,"cg_file":0,"cg_kernel":384,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":3852,"cg_percpu":2,"cg_shmem":0,"cg_slab":252,"cg_sock":0,"conns":1,"cpu_ms":50,"fds":12,"nofile":"65536/65536","pid":1,"stime_ms":30,"utime_ms":20},"ferme":{"RssAnon":424,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":4560,"VmHWM":2048,"VmRSS":2048,"VmSize":7472,"VmStk":132,"cg_anon":392,"cg_current":1220,"cg_file":0,"cg_kernel":360,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":3852,"cg_percpu":2,"cg_shmem":0,"cg_slab":239,"cg_sock":0,"conns":0,"cpu_ms":50,"fds":11,"nofile":"65536/65536","pid":1,"stime_ms":30,"utime_ms":20},"hz":10,"livres":100,"mode":"ws","n":1,"octets_diffusion":9590,"secs":10,"secs_inactif":16,"snapshot":4096,"temps_connexion_s":0.056134896}
- serveur : OOMKilled=false, état=running, journal : serveur ws sur :8800, snapshot 4096 o, nofile Some(String("65536/65536"))

### ws · N = 64 · snapshot 4096 o · 10 événements/s pendant 10 s
- connexions établies et snapshot reçu : 64/64 en 0.06 s ; erreurs client : 0 ; `ulimit -n` du serveur (souple/dur) : 65536/65536 ; réponse de diffusion : `{"envoyes":100,"recepteurs":64}`
- connexions comptées par le serveur : 64 ; descripteurs ouverts : 75 ; fils : 3

| phase (mesurée par le serveur sur lui-même, Kio) | RssAnon | VmRSS | VmHWM (pic) | cgroup `anon` | cgroup `kernel` | cgroup `slab` | cgroup `sock` | cgroup `memory.current` |
|---|---|---|---|---|---|---|---|---|
| avant (0 connexion) | 400 | 1960 | 1960 | 404 | 372 | 243 | 0 | 1168 |
| avec 64 connexions inactives | 9328 | 10952 | 10952 | 9296 | 476 | 339 | 0 | 10268 |
| après la diffusion | 9328 | 10952 | 10952 | 9332 | 548 | 403 | 0 | 10332 |
| après fermeture | 444 | 2068 | 10752 | 412 | 420 | 297 | 0 | 1300 |

Par connexion (avec 64 − avant) / 64 : RssAnon **139.50 Kio**, VmRSS 140.50 Kio, cgroup `anon` 138.94 Kio, `kernel` 1.62 Kio, `slab` 1.50 Kio, `sock` 0.00 Kio, `memory.current` 142.19 Kio.

CPU du serveur (utime+stime) :
- 64 connexions inactives pendant 16 s : 0 ms ; octets reçus par les clients dans l'intervalle : 0 (maintiens SSE)
- diffusion de 100 événements (10/s × 10 s) à 64 connexions : **200 ms** ; 6400/6400 livraisons reçues ; 613760 o reçus par les clients (95.9 o par livraison, cadrage compris)
- soit 31.2 µs de CPU serveur par livraison et 2.0 % d'un cœur pendant les 10 s
RESULTAT {"apres":{"RssAnon":9328,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":13480,"VmHWM":10952,"VmRSS":10952,"VmSize":16392,"VmStk":132,"cg_anon":9296,"cg_current":10268,"cg_file":0,"cg_kernel":476,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":10284,"cg_percpu":2,"cg_shmem":0,"cg_slab":339,"cg_sock":0,"conns":64,"cpu_ms":20,"fds":75,"nofile":"65536/65536","pid":1,"stime_ms":20,"utime_ms":0},"attendus":6400,"avant":{"RssAnon":400,"RssFile":1560,"RssShmem":0,"Threads":3,"VmData":4560,"VmHWM":1960,"VmRSS":1960,"VmSize":7472,"VmStk":132,"cg_anon":404,"cg_current":1168,"cg_file":0,"cg_kernel":372,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":4092,"cg_percpu":2,"cg_shmem":0,"cg_slab":243,"cg_sock":0,"conns":0,"cpu_ms":0,"fds":11,"nofile":"65536/65536","pid":1,"stime_ms":0,"utime_ms":0},"cpu_diffusion_ms":200,"cpu_inactif_ms":0,"diffusion":{"RssAnon":9328,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":13480,"VmHWM":10952,"VmRSS":10952,"VmSize":16392,"VmStk":132,"cg_anon":9332,"cg_current":10332,"cg_file":0,"cg_kernel":548,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":10600,"cg_percpu":2,"cg_shmem":0,"cg_slab":403,"cg_sock":0,"conns":64,"cpu_ms":230,"fds":75,"nofile":"65536/65536","pid":1,"stime_ms":110,"utime_ms":120},"ferme":{"RssAnon":444,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":4564,"VmHWM":10752,"VmRSS":2068,"VmSize":7476,"VmStk":132,"cg_anon":412,"cg_current":1300,"cg_file":0,"cg_kernel":420,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":10608,"cg_percpu":2,"cg_shmem":0,"cg_slab":297,"cg_sock":0,"conns":0,"cpu_ms":240,"fds":11,"nofile":"65536/65536","pid":1,"stime_ms":110,"utime_ms":130},"hz":10,"livres":6400,"mode":"ws","n":64,"octets_diffusion":613760,"secs":10,"secs_inactif":16,"snapshot":4096,"temps_connexion_s":0.055907766}
- serveur : OOMKilled=false, état=running, journal : serveur ws sur :8800, snapshot 4096 o, nofile Some(String("65536/65536"))

### ws · N = 1000 · snapshot 4096 o · 10 événements/s pendant 10 s
- connexions établies et snapshot reçu : 919/1000 en 60.00 s ; erreurs client : 0 ; `ulimit -n` du serveur (souple/dur) : 65536/65536 ; réponse de diffusion : ``
- connexions comptées par le serveur : — ; descripteurs ouverts : — ; fils : —

| phase (mesurée par le serveur sur lui-même, Kio) | RssAnon | VmRSS | VmHWM (pic) | cgroup `anon` | cgroup `kernel` | cgroup `slab` | cgroup `sock` | cgroup `memory.current` |
|---|---|---|---|---|---|---|---|---|
| avant (0 connexion) | 372 | 1932 | 1932 | 340 | 360 | 239 | 0 | 1172 |
| avec 1000 connexions inactives | — | — | — | — | — | — | — | — |
| après la diffusion | — | — | — | — | — | — | — | — |
| après fermeture | — | — | — | — | — | — | — | — |

Par connexion (avec 1000 − avant) / 1000 : RssAnon **— Kio**, VmRSS — Kio, cgroup `anon` — Kio, `kernel` — Kio, `slab` — Kio, `sock` — Kio, `memory.current` — Kio.

CPU du serveur (utime+stime) :
- 1000 connexions inactives pendant 16 s : — ms ; octets reçus par les clients dans l'intervalle : 0 (maintiens SSE)
- diffusion de 100 événements (10/s × 10 s) à 1000 connexions : **— ms** ; 0/100000 livraisons reçues ; 0 o reçus par les clients (0.0 o par livraison, cadrage compris)
RESULTAT {"apres":{},"attendus":100000,"avant":{"RssAnon":372,"RssFile":1560,"RssShmem":0,"Threads":3,"VmData":4560,"VmHWM":1932,"VmRSS":1932,"VmSize":7472,"VmStk":132,"cg_anon":340,"cg_current":1172,"cg_file":0,"cg_kernel":360,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":3824,"cg_percpu":2,"cg_shmem":0,"cg_slab":239,"cg_sock":0,"conns":0,"cpu_ms":0,"fds":11,"nofile":"65536/65536","pid":1,"stime_ms":0,"utime_ms":0},"cpu_diffusion_ms":null,"cpu_inactif_ms":null,"diffusion":{},"ferme":{},"hz":10,"livres":0,"mode":"ws","n":1000,"octets_diffusion":0,"secs":10,"secs_inactif":16,"snapshot":4096,"temps_connexion_s":60.004806955}
- serveur : OOMKilled=true, état=exited, journal : serveur ws sur :8800, snapshot 4096 o, nofile Some(String("65536/65536"))

### ws-leger · N = 1 · snapshot 4096 o · 10 événements/s pendant 10 s
- connexions établies et snapshot reçu : 1/1 en 0.05 s ; erreurs client : 0 ; `ulimit -n` du serveur (souple/dur) : 65536/65536 ; réponse de diffusion : `{"envoyes":100,"recepteurs":1}`
- connexions comptées par le serveur : 1 ; descripteurs ouverts : 12 ; fils : 3

| phase (mesurée par le serveur sur lui-même, Kio) | RssAnon | VmRSS | VmHWM (pic) | cgroup `anon` | cgroup `kernel` | cgroup `slab` | cgroup `sock` | cgroup `memory.current` |
|---|---|---|---|---|---|---|---|---|
| avant (0 connexion) | 372 | 1932 | 1932 | 340 | 360 | 240 | 0 | 1172 |
| avec 1 connexions inactives | 436 | 2060 | 2060 | 404 | 364 | 241 | 0 | 1200 |
| après la diffusion | 436 | 2060 | 2060 | 440 | 384 | 254 | 0 | 1204 |
| après fermeture | 424 | 2048 | 2048 | 392 | 360 | 240 | 0 | 1220 |

Par connexion (avec 1 − avant) / 1 : RssAnon **64.00 Kio**, VmRSS 128.00 Kio, cgroup `anon` 64.00 Kio, `kernel` 4.00 Kio, `slab` 1.00 Kio, `sock` 0.00 Kio, `memory.current` 28.00 Kio.

CPU du serveur (utime+stime) :
- 1 connexions inactives pendant 16 s : 0 ms ; octets reçus par les clients dans l'intervalle : 0 (maintiens SSE)
- diffusion de 100 événements (10/s × 10 s) à 1 connexions : **30 ms** ; 100/100 livraisons reçues ; 9590 o reçus par les clients (95.9 o par livraison, cadrage compris)
- soit 300.0 µs de CPU serveur par livraison et 0.3 % d'un cœur pendant les 10 s
RESULTAT {"apres":{"RssAnon":436,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":4580,"VmHWM":2060,"VmRSS":2060,"VmSize":7492,"VmStk":132,"cg_anon":404,"cg_current":1200,"cg_file":0,"cg_kernel":364,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":3776,"cg_percpu":2,"cg_shmem":0,"cg_slab":241,"cg_sock":0,"conns":1,"cpu_ms":30,"fds":12,"nofile":"65536/65536","pid":1,"stime_ms":20,"utime_ms":10},"attendus":100,"avant":{"RssAnon":372,"RssFile":1560,"RssShmem":0,"Threads":3,"VmData":4560,"VmHWM":1932,"VmRSS":1932,"VmSize":7472,"VmStk":132,"cg_anon":340,"cg_current":1172,"cg_file":0,"cg_kernel":360,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":3776,"cg_percpu":2,"cg_shmem":0,"cg_slab":240,"cg_sock":0,"conns":0,"cpu_ms":20,"fds":11,"nofile":"65536/65536","pid":1,"stime_ms":20,"utime_ms":0},"cpu_diffusion_ms":30,"cpu_inactif_ms":0,"diffusion":{"RssAnon":436,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":4580,"VmHWM":2060,"VmRSS":2060,"VmSize":7492,"VmStk":132,"cg_anon":440,"cg_current":1204,"cg_file":0,"cg_kernel":384,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":3776,"cg_percpu":2,"cg_shmem":0,"cg_slab":254,"cg_sock":0,"conns":1,"cpu_ms":60,"fds":12,"nofile":"65536/65536","pid":1,"stime_ms":40,"utime_ms":20},"ferme":{"RssAnon":424,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":4560,"VmHWM":2048,"VmRSS":2048,"VmSize":7472,"VmStk":132,"cg_anon":392,"cg_current":1220,"cg_file":0,"cg_kernel":360,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":3776,"cg_percpu":2,"cg_shmem":0,"cg_slab":240,"cg_sock":0,"conns":0,"cpu_ms":60,"fds":11,"nofile":"65536/65536","pid":1,"stime_ms":40,"utime_ms":20},"hz":10,"livres":100,"mode":"ws-leger","n":1,"octets_diffusion":9590,"secs":10,"secs_inactif":16,"snapshot":4096,"temps_connexion_s":0.051391947}
- serveur : OOMKilled=false, état=running, journal : serveur ws-leger sur :8800, snapshot 4096 o, nofile Some(String("65536/65536"))

### ws-leger · N = 64 · snapshot 4096 o · 10 événements/s pendant 10 s
- connexions établies et snapshot reçu : 64/64 en 0.05 s ; erreurs client : 0 ; `ulimit -n` du serveur (souple/dur) : 65536/65536 ; réponse de diffusion : `{"envoyes":100,"recepteurs":64}`
- connexions comptées par le serveur : 64 ; descripteurs ouverts : 75 ; fils : 3

| phase (mesurée par le serveur sur lui-même, Kio) | RssAnon | VmRSS | VmHWM (pic) | cgroup `anon` | cgroup `kernel` | cgroup `slab` | cgroup `sock` | cgroup `memory.current` |
|---|---|---|---|---|---|---|---|---|
| avant (0 connexion) | 372 | 1932 | 1932 | 340 | 360 | 239 | 0 | 1168 |
| avec 64 connexions inactives | 1172 | 2796 | 2816 | 1140 | 464 | 341 | 0 | 2108 |
| après la diffusion | 1172 | 2796 | 2816 | 1176 | 532 | 405 | 0 | 2168 |
| après fermeture | 440 | 2064 | 2816 | 408 | 420 | 297 | 0 | 1300 |

Par connexion (avec 64 − avant) / 64 : RssAnon **12.50 Kio**, VmRSS 13.50 Kio, cgroup `anon` 12.50 Kio, `kernel` 1.62 Kio, `slab` 1.59 Kio, `sock` 0.00 Kio, `memory.current` 14.69 Kio.

CPU du serveur (utime+stime) :
- 64 connexions inactives pendant 16 s : 0 ms ; octets reçus par les clients dans l'intervalle : 0 (maintiens SSE)
- diffusion de 100 événements (10/s × 10 s) à 64 connexions : **130 ms** ; 6400/6400 livraisons reçues ; 613760 o reçus par les clients (95.9 o par livraison, cadrage compris)
- soit 20.3 µs de CPU serveur par livraison et 1.3 % d'un cœur pendant les 10 s
RESULTAT {"apres":{"RssAnon":1172,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":5428,"VmHWM":2816,"VmRSS":2796,"VmSize":8340,"VmStk":132,"cg_anon":1140,"cg_current":2108,"cg_file":0,"cg_kernel":464,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":4328,"cg_percpu":2,"cg_shmem":0,"cg_slab":341,"cg_sock":0,"conns":64,"cpu_ms":10,"fds":75,"nofile":"65536/65536","pid":1,"stime_ms":10,"utime_ms":0},"attendus":6400,"avant":{"RssAnon":372,"RssFile":1560,"RssShmem":0,"Threads":3,"VmData":4560,"VmHWM":1932,"VmRSS":1932,"VmSize":7472,"VmStk":132,"cg_anon":340,"cg_current":1168,"cg_file":0,"cg_kernel":360,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":4328,"cg_percpu":2,"cg_shmem":0,"cg_slab":239,"cg_sock":0,"conns":0,"cpu_ms":0,"fds":11,"nofile":"65536/65536","pid":1,"stime_ms":0,"utime_ms":0},"cpu_diffusion_ms":130,"cpu_inactif_ms":0,"diffusion":{"RssAnon":1172,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":5428,"VmHWM":2816,"VmRSS":2796,"VmSize":8340,"VmStk":132,"cg_anon":1176,"cg_current":2168,"cg_file":0,"cg_kernel":532,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":4328,"cg_percpu":2,"cg_shmem":0,"cg_slab":405,"cg_sock":0,"conns":64,"cpu_ms":150,"fds":75,"nofile":"65536/65536","pid":1,"stime_ms":100,"utime_ms":50},"ferme":{"RssAnon":440,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":4564,"VmHWM":2816,"VmRSS":2064,"VmSize":7476,"VmStk":132,"cg_anon":408,"cg_current":1300,"cg_file":0,"cg_kernel":420,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":4328,"cg_percpu":2,"cg_shmem":0,"cg_slab":297,"cg_sock":0,"conns":0,"cpu_ms":170,"fds":11,"nofile":"65536/65536","pid":1,"stime_ms":110,"utime_ms":60},"hz":10,"livres":6400,"mode":"ws-leger","n":64,"octets_diffusion":613760,"secs":10,"secs_inactif":16,"snapshot":4096,"temps_connexion_s":0.052480406}
- serveur : OOMKilled=false, état=running, journal : serveur ws-leger sur :8800, snapshot 4096 o, nofile Some(String("65536/65536"))

### ws-leger · N = 1000 · snapshot 4096 o · 10 événements/s pendant 10 s
- connexions établies et snapshot reçu : 1000/1000 en 1.11 s ; erreurs client : 0 ; `ulimit -n` du serveur (souple/dur) : 65536/65536 ; réponse de diffusion : `{"envoyes":100,"recepteurs":1000}`
- connexions comptées par le serveur : 1000 ; descripteurs ouverts : 1011 ; fils : 3

| phase (mesurée par le serveur sur lui-même, Kio) | RssAnon | VmRSS | VmHWM (pic) | cgroup `anon` | cgroup `kernel` | cgroup `slab` | cgroup `sock` | cgroup `memory.current` |
|---|---|---|---|---|---|---|---|---|
| avant (0 connexion) | 372 | 1932 | 1932 | 376 | 376 | 242 | 0 | 1172 |
| avec 1000 connexions inactives | 11448 | 13072 | 13072 | 11448 | 2820 | 2664 | 0 | 14616 |
| après la diffusion | 11448 | 13072 | 13072 | 11448 | 2820 | 2665 | 0 | 14752 |
| après fermeture | 484 | 2108 | 13044 | 468 | 1276 | 1146 | 0 | 2212 |

Par connexion (avec 1000 − avant) / 1000 : RssAnon **11.08 Kio**, VmRSS 11.14 Kio, cgroup `anon` 11.07 Kio, `kernel` 2.44 Kio, `slab` 2.42 Kio, `sock` 0.00 Kio, `memory.current` 13.44 Kio.

CPU du serveur (utime+stime) :
- 1000 connexions inactives pendant 16 s : 10 ms ; octets reçus par les clients dans l'intervalle : 0 (maintiens SSE)
- diffusion de 100 événements (10/s × 10 s) à 1000 connexions : **1370 ms** ; 100000/100000 livraisons reçues ; 9590000 o reçus par les clients (95.9 o par livraison, cadrage compris)
- soit 13.7 µs de CPU serveur par livraison et 13.7 % d'un cœur pendant les 10 s
RESULTAT {"apres":{"RssAnon":11448,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":15736,"VmHWM":13072,"VmRSS":13072,"VmSize":18648,"VmStk":132,"cg_anon":11448,"cg_current":14616,"cg_file":0,"cg_kernel":2820,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":14624,"cg_percpu":2,"cg_shmem":0,"cg_slab":2664,"cg_sock":0,"conns":1000,"cpu_ms":60,"fds":1011,"nofile":"65536/65536","pid":1,"stime_ms":40,"utime_ms":20},"attendus":100000,"avant":{"RssAnon":372,"RssFile":1560,"RssShmem":0,"Threads":3,"VmData":4560,"VmHWM":1932,"VmRSS":1932,"VmSize":7472,"VmStk":132,"cg_anon":376,"cg_current":1172,"cg_file":0,"cg_kernel":376,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":3856,"cg_percpu":2,"cg_shmem":0,"cg_slab":242,"cg_sock":0,"conns":0,"cpu_ms":0,"fds":11,"nofile":"65536/65536","pid":1,"stime_ms":0,"utime_ms":0},"cpu_diffusion_ms":1370,"cpu_inactif_ms":10,"diffusion":{"RssAnon":11448,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":15736,"VmHWM":13072,"VmRSS":13072,"VmSize":18648,"VmStk":132,"cg_anon":11448,"cg_current":14752,"cg_file":0,"cg_kernel":2820,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":16524,"cg_percpu":2,"cg_shmem":0,"cg_slab":2665,"cg_sock":0,"conns":1000,"cpu_ms":1460,"fds":1011,"nofile":"65536/65536","pid":1,"stime_ms":920,"utime_ms":540},"ferme":{"RssAnon":484,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":4700,"VmHWM":13044,"VmRSS":2108,"VmSize":7612,"VmStk":132,"cg_anon":468,"cg_current":2212,"cg_file":0,"cg_kernel":1276,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":16524,"cg_percpu":2,"cg_shmem":0,"cg_slab":1146,"cg_sock":0,"conns":0,"cpu_ms":1500,"fds":11,"nofile":"65536/65536","pid":1,"stime_ms":940,"utime_ms":560},"hz":10,"livres":100000,"mode":"ws-leger","n":1000,"octets_diffusion":9590000,"secs":10,"secs_inactif":16,"snapshot":4096,"temps_connexion_s":1.112377083}
- serveur : OOMKilled=false, état=running, journal : serveur ws-leger sur :8800, snapshot 4096 o, nofile Some(String("65536/65536"))

### sse · N = 64 · snapshot 480731 o · 10 événements/s pendant 10 s
- connexions établies et snapshot reçu : 64/64 en 0.05 s ; erreurs client : 0 ; `ulimit -n` du serveur (souple/dur) : 65536/65536 ; réponse de diffusion : `{"envoyes":100,"recepteurs":64}`
- connexions comptées par le serveur : 64 ; descripteurs ouverts : 75 ; fils : 3

| phase (mesurée par le serveur sur lui-même, Kio) | RssAnon | VmRSS | VmHWM (pic) | cgroup `anon` | cgroup `kernel` | cgroup `slab` | cgroup `sock` | cgroup `memory.current` |
|---|---|---|---|---|---|---|---|---|
| avant (0 connexion) | 856 | 2416 | 2432 | 800 | 360 | 239 | 0 | 1632 |
| avec 64 connexions inactives | 3232 | 4856 | 5520 | 3196 | 468 | 336 | 0 | 4164 |
| après la diffusion | 3256 | 4880 | 5520 | 3260 | 536 | 399 | 0 | 4280 |
| après fermeture | 884 | 2508 | 5520 | 852 | 420 | 297 | 0 | 1744 |

Par connexion (avec 64 − avant) / 64 : RssAnon **37.12 Kio**, VmRSS 38.12 Kio, cgroup `anon` 37.44 Kio, `kernel` 1.69 Kio, `slab` 1.52 Kio, `sock` 0.00 Kio, `memory.current` 39.56 Kio.

CPU du serveur (utime+stime) :
- 64 connexions inactives pendant 16 s : 0 ms ; octets reçus par les clients dans l'intervalle : 512 (maintiens SSE)
- diffusion de 100 événements (10/s × 10 s) à 64 connexions : **160 ms** ; 6400/6400 livraisons reçues ; 773760 o reçus par les clients (120.9 o par livraison, cadrage compris)
- soit 25.0 µs de CPU serveur par livraison et 1.6 % d'un cœur pendant les 10 s
RESULTAT {"apres":{"RssAnon":3232,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":8100,"VmHWM":5520,"VmRSS":4856,"VmSize":11012,"VmStk":132,"cg_anon":3196,"cg_current":4164,"cg_file":0,"cg_kernel":468,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":13440,"cg_percpu":2,"cg_shmem":0,"cg_slab":336,"cg_sock":0,"conns":64,"cpu_ms":50,"fds":75,"nofile":"65536/65536","pid":1,"stime_ms":30,"utime_ms":20},"attendus":6400,"avant":{"RssAnon":856,"RssFile":1560,"RssShmem":0,"Threads":3,"VmData":5016,"VmHWM":2432,"VmRSS":2416,"VmSize":7928,"VmStk":132,"cg_anon":800,"cg_current":1632,"cg_file":0,"cg_kernel":360,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":4016,"cg_percpu":2,"cg_shmem":0,"cg_slab":239,"cg_sock":0,"conns":0,"cpu_ms":0,"fds":11,"nofile":"65536/65536","pid":1,"stime_ms":0,"utime_ms":0},"cpu_diffusion_ms":160,"cpu_inactif_ms":0,"diffusion":{"RssAnon":3256,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":8100,"VmHWM":5520,"VmRSS":4880,"VmSize":11012,"VmStk":132,"cg_anon":3260,"cg_current":4280,"cg_file":0,"cg_kernel":536,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":13440,"cg_percpu":2,"cg_shmem":0,"cg_slab":399,"cg_sock":0,"conns":64,"cpu_ms":210,"fds":75,"nofile":"65536/65536","pid":1,"stime_ms":120,"utime_ms":90},"ferme":{"RssAnon":884,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":5024,"VmHWM":5520,"VmRSS":2508,"VmSize":7936,"VmStk":132,"cg_anon":852,"cg_current":1744,"cg_file":0,"cg_kernel":420,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":13440,"cg_percpu":2,"cg_shmem":0,"cg_slab":297,"cg_sock":0,"conns":0,"cpu_ms":210,"fds":11,"nofile":"65536/65536","pid":1,"stime_ms":120,"utime_ms":90},"hz":10,"livres":6400,"mode":"sse","n":64,"octets_diffusion":773760,"secs":10,"secs_inactif":16,"snapshot":480731,"temps_connexion_s":0.052984361}
- serveur : OOMKilled=false, état=running, journal : serveur sse sur :8800, snapshot 480731 o, nofile Some(String("65536/65536"))

### ws · N = 64 · snapshot 480731 o · 10 événements/s pendant 10 s
- connexions établies et snapshot reçu : 64/64 en 0.10 s ; erreurs client : 0 ; `ulimit -n` du serveur (souple/dur) : 65536/65536 ; réponse de diffusion : `{"envoyes":100,"recepteurs":64}`
- connexions comptées par le serveur : 64 ; descripteurs ouverts : 75 ; fils : 3

| phase (mesurée par le serveur sur lui-même, Kio) | RssAnon | VmRSS | VmHWM (pic) | cgroup `anon` | cgroup `kernel` | cgroup `slab` | cgroup `sock` | cgroup `memory.current` |
|---|---|---|---|---|---|---|---|---|
| avant (0 connexion) | 832 | 2392 | 2432 | 836 | 376 | 243 | 0 | 1632 |
| avec 64 connexions inactives | 39700 | 41324 | 41488 | 39668 | 552 | 345 | 0 | 40724 |
| après la diffusion | 39700 | 41324 | 41488 | 39704 | 620 | 409 | 0 | 40784 |
| après fermeture | 900 | 2524 | 41488 | 868 | 424 | 297 | 0 | 1764 |

Par connexion (avec 64 − avant) / 64 : RssAnon **607.31 Kio**, VmRSS 608.31 Kio, cgroup `anon` 606.75 Kio, `kernel` 2.75 Kio, `slab` 1.59 Kio, `sock` 0.00 Kio, `memory.current` 610.81 Kio.

CPU du serveur (utime+stime) :
- 64 connexions inactives pendant 16 s : 0 ms ; octets reçus par les clients dans l'intervalle : 0 (maintiens SSE)
- diffusion de 100 événements (10/s × 10 s) à 64 connexions : **220 ms** ; 6400/6400 livraisons reçues ; 613760 o reçus par les clients (95.9 o par livraison, cadrage compris)
- soit 34.4 µs de CPU serveur par livraison et 2.2 % d'un cœur pendant les 10 s
RESULTAT {"apres":{"RssAnon":39700,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":43832,"VmHWM":41488,"VmRSS":41324,"VmSize":46744,"VmStk":132,"cg_anon":39668,"cg_current":40724,"cg_file":0,"cg_kernel":552,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":40984,"cg_percpu":2,"cg_shmem":0,"cg_slab":345,"cg_sock":0,"conns":64,"cpu_ms":120,"fds":75,"nofile":"65536/65536","pid":1,"stime_ms":110,"utime_ms":10},"attendus":6400,"avant":{"RssAnon":832,"RssFile":1560,"RssShmem":0,"Threads":3,"VmData":5016,"VmHWM":2432,"VmRSS":2392,"VmSize":7928,"VmStk":132,"cg_anon":836,"cg_current":1632,"cg_file":0,"cg_kernel":376,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":3972,"cg_percpu":2,"cg_shmem":0,"cg_slab":243,"cg_sock":0,"conns":0,"cpu_ms":0,"fds":11,"nofile":"65536/65536","pid":1,"stime_ms":0,"utime_ms":0},"cpu_diffusion_ms":220,"cpu_inactif_ms":0,"diffusion":{"RssAnon":39700,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":43832,"VmHWM":41488,"VmRSS":41324,"VmSize":46744,"VmStk":132,"cg_anon":39704,"cg_current":40784,"cg_file":0,"cg_kernel":620,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":40984,"cg_percpu":2,"cg_shmem":0,"cg_slab":409,"cg_sock":0,"conns":64,"cpu_ms":340,"fds":75,"nofile":"65536/65536","pid":1,"stime_ms":210,"utime_ms":130},"ferme":{"RssAnon":900,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":5024,"VmHWM":41488,"VmRSS":2524,"VmSize":7936,"VmStk":132,"cg_anon":868,"cg_current":1764,"cg_file":0,"cg_kernel":424,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":41048,"cg_percpu":2,"cg_shmem":0,"cg_slab":297,"cg_sock":0,"conns":0,"cpu_ms":340,"fds":11,"nofile":"65536/65536","pid":1,"stime_ms":210,"utime_ms":130},"hz":10,"livres":6400,"mode":"ws","n":64,"octets_diffusion":613760,"secs":10,"secs_inactif":16,"snapshot":480731,"temps_connexion_s":0.103982044}
- serveur : OOMKilled=false, état=running, journal : serveur ws sur :8800, snapshot 480731 o, nofile Some(String("65536/65536"))

## Compléments : WebSocket (configuration par défaut d'axum) à N = 250 et 500, le palier 1 000 ayant été tué par l'OOM du cgroup (128 Mio)

### ws · N = 250 · snapshot 4096 o · 10 événements/s pendant 10 s
- connexions établies et snapshot reçu : 250/250 en 0.05 s ; erreurs client : 0 ; `ulimit -n` du serveur (souple/dur) : 65536/65536 ; réponse de diffusion : `{"envoyes":100,"recepteurs":250}`
- connexions comptées par le serveur : 250 ; descripteurs ouverts : 261 ; fils : 3

| phase (mesurée par le serveur sur lui-même, Kio) | RssAnon | VmRSS | VmHWM (pic) | cgroup `anon` | cgroup `kernel` | cgroup `slab` | cgroup `sock` | cgroup `memory.current` |
|---|---|---|---|---|---|---|---|---|
| avant (0 connexion) | 372 | 1932 | 1932 | 376 | 372 | 243 | 0 | 1168 |
| avec 250 connexions inactives | 35100 | 36724 | 36724 | 35080 | 816 | 626 | 0 | 36160 |
| après la diffusion | 35100 | 36724 | 36724 | 35100 | 1056 | 862 | 0 | 36396 |
| après fermeture | 480 | 2104 | 36608 | 464 | 596 | 468 | 0 | 1528 |

Par connexion (avec 250 − avant) / 250 : RssAnon **138.91 Kio**, VmRSS 139.17 Kio, cgroup `anon` 138.82 Kio, `kernel` 1.78 Kio, `slab` 1.53 Kio, `sock` 0.00 Kio, `memory.current` 139.97 Kio.

CPU du serveur (utime+stime) :
- 250 connexions inactives pendant 16 s : 0 ms ; octets reçus par les clients dans l'intervalle : 0 (maintiens SSE)
- diffusion de 100 événements (10/s × 10 s) à 250 connexions : **350 ms** ; 25000/25000 livraisons reçues ; 2397500 o reçus par les clients (95.9 o par livraison, cadrage compris)
- soit 14.0 µs de CPU serveur par livraison et 3.5 % d'un cœur pendant les 10 s
RESULTAT {"apres":{"RssAnon":35100,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":39392,"VmHWM":36724,"VmRSS":36724,"VmSize":42304,"VmStk":132,"cg_anon":35080,"cg_current":36160,"cg_file":0,"cg_kernel":816,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":36412,"cg_percpu":2,"cg_shmem":0,"cg_slab":626,"cg_sock":0,"conns":250,"cpu_ms":60,"fds":261,"nofile":"65536/65536","pid":1,"stime_ms":50,"utime_ms":10},"attendus":25000,"avant":{"RssAnon":372,"RssFile":1560,"RssShmem":0,"Threads":3,"VmData":4560,"VmHWM":1932,"VmRSS":1932,"VmSize":7472,"VmStk":132,"cg_anon":376,"cg_current":1168,"cg_file":0,"cg_kernel":372,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":3852,"cg_percpu":2,"cg_shmem":0,"cg_slab":243,"cg_sock":0,"conns":0,"cpu_ms":0,"fds":11,"nofile":"65536/65536","pid":1,"stime_ms":0,"utime_ms":0},"cpu_diffusion_ms":350,"cpu_inactif_ms":0,"diffusion":{"RssAnon":35100,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":39392,"VmHWM":36724,"VmRSS":36724,"VmSize":42304,"VmStk":132,"cg_anon":35100,"cg_current":36396,"cg_file":0,"cg_kernel":1056,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":37648,"cg_percpu":2,"cg_shmem":0,"cg_slab":862,"cg_sock":0,"conns":250,"cpu_ms":410,"fds":261,"nofile":"65536/65536","pid":1,"stime_ms":180,"utime_ms":230},"ferme":{"RssAnon":480,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":4696,"VmHWM":36608,"VmRSS":2104,"VmSize":7608,"VmStk":132,"cg_anon":464,"cg_current":1528,"cg_file":0,"cg_kernel":596,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":37648,"cg_percpu":2,"cg_shmem":0,"cg_slab":468,"cg_sock":0,"conns":0,"cpu_ms":410,"fds":11,"nofile":"65536/65536","pid":1,"stime_ms":180,"utime_ms":230},"hz":10,"livres":25000,"mode":"ws","n":250,"octets_diffusion":2397500,"secs":10,"secs_inactif":16,"snapshot":4096,"temps_connexion_s":0.052100161}
- serveur : OOMKilled=false, état=running, journal : serveur ws sur :8800, snapshot 4096 o, nofile Some(String("65536/65536"))

### ws · N = 500 · snapshot 4096 o · 10 événements/s pendant 10 s
- connexions établies et snapshot reçu : 500/500 en 0.17 s ; erreurs client : 0 ; `ulimit -n` du serveur (souple/dur) : 65536/65536 ; réponse de diffusion : `{"envoyes":100,"recepteurs":500}`
- connexions comptées par le serveur : 500 ; descripteurs ouverts : 511 ; fils : 3

| phase (mesurée par le serveur sur lui-même, Kio) | RssAnon | VmRSS | VmHWM (pic) | cgroup `anon` | cgroup `kernel` | cgroup `slab` | cgroup `sock` | cgroup `memory.current` |
|---|---|---|---|---|---|---|---|---|
| avant (0 connexion) | 372 | 1932 | 1932 | 376 | 372 | 243 | 0 | 1168 |
| avec 500 connexions inactives | 69680 | 71304 | 71304 | 69668 | 1264 | 1003 | 0 | 71628 |
| après la diffusion | 69688 | 71312 | 71312 | 69688 | 1724 | 1458 | 0 | 71892 |
| après fermeture | 476 | 2100 | 71168 | 460 | 820 | 693 | 0 | 1752 |

Par connexion (avec 500 − avant) / 500 : RssAnon **138.62 Kio**, VmRSS 138.74 Kio, cgroup `anon` 138.58 Kio, `kernel` 1.78 Kio, `slab` 1.52 Kio, `sock` 0.00 Kio, `memory.current` 140.92 Kio.

CPU du serveur (utime+stime) :
- 500 connexions inactives pendant 16 s : 0 ms ; octets reçus par les clients dans l'intervalle : 0 (maintiens SSE)
- diffusion de 100 événements (10/s × 10 s) à 500 connexions : **850 ms** ; 50000/50000 livraisons reçues ; 4795000 o reçus par les clients (95.9 o par livraison, cadrage compris)
- soit 17.0 µs de CPU serveur par livraison et 8.5 % d'un cœur pendant les 10 s
RESULTAT {"apres":{"RssAnon":69680,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":73904,"VmHWM":71304,"VmRSS":71304,"VmSize":76816,"VmStk":132,"cg_anon":69668,"cg_current":71628,"cg_file":0,"cg_kernel":1264,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":71636,"cg_percpu":2,"cg_shmem":0,"cg_slab":1003,"cg_sock":0,"conns":500,"cpu_ms":110,"fds":511,"nofile":"65536/65536","pid":1,"stime_ms":90,"utime_ms":20},"attendus":50000,"avant":{"RssAnon":372,"RssFile":1560,"RssShmem":0,"Threads":3,"VmData":4560,"VmHWM":1932,"VmRSS":1932,"VmSize":7472,"VmStk":132,"cg_anon":376,"cg_current":1168,"cg_file":0,"cg_kernel":372,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":4152,"cg_percpu":2,"cg_shmem":0,"cg_slab":243,"cg_sock":0,"conns":0,"cpu_ms":10,"fds":11,"nofile":"65536/65536","pid":1,"stime_ms":10,"utime_ms":0},"cpu_diffusion_ms":850,"cpu_inactif_ms":0,"diffusion":{"RssAnon":69688,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":73904,"VmHWM":71312,"VmRSS":71312,"VmSize":76816,"VmStk":132,"cg_anon":69688,"cg_current":71892,"cg_file":0,"cg_kernel":1724,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":73160,"cg_percpu":2,"cg_shmem":0,"cg_slab":1458,"cg_sock":0,"conns":500,"cpu_ms":970,"fds":511,"nofile":"65536/65536","pid":1,"stime_ms":500,"utime_ms":470},"ferme":{"RssAnon":476,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":4600,"VmHWM":71168,"VmRSS":2100,"VmSize":7512,"VmStk":132,"cg_anon":460,"cg_current":1752,"cg_file":0,"cg_kernel":820,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":73160,"cg_percpu":2,"cg_shmem":0,"cg_slab":693,"cg_sock":0,"conns":0,"cpu_ms":1000,"fds":11,"nofile":"65536/65536","pid":1,"stime_ms":530,"utime_ms":470},"hz":10,"livres":50000,"mode":"ws","n":500,"octets_diffusion":4795000,"secs":10,"secs_inactif":16,"snapshot":4096,"temps_connexion_s":0.170200324}
- serveur : OOMKilled=false, état=running, journal : serveur ws sur :8800, snapshot 4096 o, nofile Some(String("65536/65536"))

## Complément : SSE sans les couches de Moli (axum nu : ni CompressionLayer ni en-têtes de sécurité), pour isoler le coût de hyper/axum

### sse-nu · N = 64 · snapshot 4096 o · 10 événements/s pendant 10 s
- connexions établies et snapshot reçu : 64/64 en 0.05 s ; erreurs client : 0 ; `ulimit -n` du serveur (souple/dur) : 65536/65536 ; réponse de diffusion : `{"envoyes":100,"recepteurs":64}`
- connexions comptées par le serveur : 64 ; descripteurs ouverts : 75 ; fils : 3

| phase (mesurée par le serveur sur lui-même, Kio) | RssAnon | VmRSS | VmHWM (pic) | cgroup `anon` | cgroup `kernel` | cgroup `slab` | cgroup `sock` | cgroup `memory.current` |
|---|---|---|---|---|---|---|---|---|
| avant (0 connexion) | 352 | 1976 | 1976 | 360 | 376 | 244 | 0 | 1176 |
| avec 64 connexions inactives | 2244 | 3868 | 3868 | 2208 | 464 | 335 | 0 | 3164 |
| après la diffusion | 2248 | 3936 | 3936 | 2252 | 536 | 399 | 0 | 3012 |
| après fermeture | 364 | 2052 | 3840 | 332 | 424 | 297 | 0 | 1252 |

Par connexion (avec 64 − avant) / 64 : RssAnon **29.56 Kio**, VmRSS 29.56 Kio, cgroup `anon` 28.88 Kio, `kernel` 1.38 Kio, `slab` 1.42 Kio, `sock` 0.00 Kio, `memory.current` 31.06 Kio.

CPU du serveur (utime+stime) :
- 64 connexions inactives pendant 16 s : 0 ms ; octets reçus par les clients dans l'intervalle : 512 (maintiens SSE)
- diffusion de 100 événements (10/s × 10 s) à 64 connexions : **160 ms** ; 6400/6400 livraisons reçues ; 773760 o reçus par les clients (120.9 o par livraison, cadrage compris)
- soit 25.0 µs de CPU serveur par livraison et 1.6 % d'un cœur pendant les 10 s
RESULTAT {"apres":{"RssAnon":2244,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":6968,"VmHWM":3868,"VmRSS":3868,"VmSize":9884,"VmStk":132,"cg_anon":2208,"cg_current":3164,"cg_file":0,"cg_kernel":464,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":3848,"cg_percpu":2,"cg_shmem":0,"cg_slab":335,"cg_sock":0,"conns":64,"cpu_ms":10,"fds":75,"nofile":"65536/65536","pid":1,"stime_ms":10,"utime_ms":0},"attendus":6400,"avant":{"RssAnon":352,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":4552,"VmHWM":1976,"VmRSS":1976,"VmSize":7468,"VmStk":132,"cg_anon":360,"cg_current":1176,"cg_file":0,"cg_kernel":376,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":3848,"cg_percpu":2,"cg_shmem":0,"cg_slab":244,"cg_sock":0,"conns":0,"cpu_ms":0,"fds":11,"nofile":"65536/65536","pid":1,"stime_ms":0,"utime_ms":0},"cpu_diffusion_ms":160,"cpu_inactif_ms":0,"diffusion":{"RssAnon":2248,"RssFile":1688,"RssShmem":0,"Threads":3,"VmData":6968,"VmHWM":3936,"VmRSS":3936,"VmSize":9884,"VmStk":132,"cg_anon":2252,"cg_current":3012,"cg_file":0,"cg_kernel":536,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":3848,"cg_percpu":2,"cg_shmem":0,"cg_slab":399,"cg_sock":0,"conns":64,"cpu_ms":170,"fds":75,"nofile":"65536/65536","pid":1,"stime_ms":100,"utime_ms":70},"ferme":{"RssAnon":364,"RssFile":1688,"RssShmem":0,"Threads":3,"VmData":4560,"VmHWM":3840,"VmRSS":2052,"VmSize":7476,"VmStk":132,"cg_anon":332,"cg_current":1252,"cg_file":0,"cg_kernel":424,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":3848,"cg_percpu":2,"cg_shmem":0,"cg_slab":297,"cg_sock":0,"conns":0,"cpu_ms":180,"fds":11,"nofile":"65536/65536","pid":1,"stime_ms":110,"utime_ms":70},"hz":10,"livres":6400,"mode":"sse-nu","n":64,"octets_diffusion":773760,"secs":10,"secs_inactif":16,"snapshot":4096,"temps_connexion_s":0.05316566}
- serveur : OOMKilled=false, état=running, journal : serveur sse-nu sur :8800, snapshot 4096 o, nofile Some(String("65536/65536"))

### sse-nu · N = 1000 · snapshot 4096 o · 10 événements/s pendant 10 s
- connexions établies et snapshot reçu : 1000/1000 en 1.12 s ; erreurs client : 0 ; `ulimit -n` du serveur (souple/dur) : 65536/65536 ; réponse de diffusion : `{"envoyes":100,"recepteurs":1000}`
- connexions comptées par le serveur : 1000 ; descripteurs ouverts : 1011 ; fils : 3

| phase (mesurée par le serveur sur lui-même, Kio) | RssAnon | VmRSS | VmHWM (pic) | cgroup `anon` | cgroup `kernel` | cgroup `slab` | cgroup `sock` | cgroup `memory.current` |
|---|---|---|---|---|---|---|---|---|
| avant (0 connexion) | 340 | 1964 | 1964 | 348 | 372 | 243 | 0 | 1164 |
| avec 1000 connexions inactives | 29936 | 31560 | 31560 | 29948 | 2848 | 2652 | 0 | 32860 |
| après la diffusion | 29944 | 31632 | 31632 | 29948 | 2848 | 2652 | 0 | 33272 |
| après fermeture | 400 | 2088 | 31488 | 368 | 1272 | 1145 | 0 | 2072 |

Par connexion (avec 1000 − avant) / 1000 : RssAnon **29.60 Kio**, VmRSS 29.60 Kio, cgroup `anon` 29.60 Kio, `kernel` 2.48 Kio, `slab` 2.41 Kio, `sock` 0.00 Kio, `memory.current` 31.70 Kio.

CPU du serveur (utime+stime) :
- 1000 connexions inactives pendant 16 s : 10 ms ; octets reçus par les clients dans l'intervalle : 8000 (maintiens SSE)
- diffusion de 100 événements (10/s × 10 s) à 1000 connexions : **1580 ms** ; 100000/100000 livraisons reçues ; 12090000 o reçus par les clients (120.9 o par livraison, cadrage compris)
- soit 15.8 µs de CPU serveur par livraison et 15.8 % d'un cœur pendant les 10 s
RESULTAT {"apres":{"RssAnon":29936,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":39060,"VmHWM":31560,"VmRSS":31560,"VmSize":41976,"VmStk":132,"cg_anon":29948,"cg_current":32860,"cg_file":0,"cg_kernel":2848,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":32876,"cg_percpu":2,"cg_shmem":0,"cg_slab":2652,"cg_sock":0,"conns":1000,"cpu_ms":80,"fds":1011,"nofile":"65536/65536","pid":1,"stime_ms":50,"utime_ms":30},"attendus":100000,"avant":{"RssAnon":340,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":4552,"VmHWM":1964,"VmRSS":1964,"VmSize":7468,"VmStk":132,"cg_anon":348,"cg_current":1164,"cg_file":0,"cg_kernel":372,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":3664,"cg_percpu":2,"cg_shmem":0,"cg_slab":243,"cg_sock":0,"conns":0,"cpu_ms":0,"fds":11,"nofile":"65536/65536","pid":1,"stime_ms":0,"utime_ms":0},"cpu_diffusion_ms":1580,"cpu_inactif_ms":10,"diffusion":{"RssAnon":29944,"RssFile":1688,"RssShmem":0,"Threads":3,"VmData":39060,"VmHWM":31632,"VmRSS":31632,"VmSize":41976,"VmStk":132,"cg_anon":29948,"cg_current":33272,"cg_file":0,"cg_kernel":2848,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":35388,"cg_percpu":2,"cg_shmem":0,"cg_slab":2652,"cg_sock":0,"conns":1000,"cpu_ms":1690,"fds":1011,"nofile":"65536/65536","pid":1,"stime_ms":960,"utime_ms":730},"ferme":{"RssAnon":400,"RssFile":1688,"RssShmem":0,"Threads":3,"VmData":4596,"VmHWM":31488,"VmRSS":2088,"VmSize":7512,"VmStk":132,"cg_anon":368,"cg_current":2072,"cg_file":0,"cg_kernel":1272,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":35388,"cg_percpu":2,"cg_shmem":0,"cg_slab":1145,"cg_sock":0,"conns":0,"cpu_ms":1730,"fds":11,"nofile":"65536/65536","pid":1,"stime_ms":980,"utime_ms":750},"hz":10,"livres":100000,"mode":"sse-nu","n":1000,"octets_diffusion":12090000,"secs":10,"secs_inactif":16,"snapshot":4096,"temps_connexion_s":1.118081491}
- serveur : OOMKilled=false, état=running, journal : serveur sse-nu sur :8800, snapshot 4096 o, nofile Some(String("65536/65536"))

## Complément : WebSocket « léger » (read 4 Kio, write 0) avec le snapshot de la taille réelle de « deux maisons »

### ws-leger · N = 64 · snapshot 480731 o · 10 événements/s pendant 10 s
- connexions établies et snapshot reçu : 64/64 en 0.11 s ; erreurs client : 0 ; `ulimit -n` du serveur (souple/dur) : 65536/65536 ; réponse de diffusion : `{"envoyes":100,"recepteurs":64}`
- connexions comptées par le serveur : 64 ; descripteurs ouverts : 75 ; fils : 3

| phase (mesurée par le serveur sur lui-même, Kio) | RssAnon | VmRSS | VmHWM (pic) | cgroup `anon` | cgroup `kernel` | cgroup `slab` | cgroup `sock` | cgroup `memory.current` |
|---|---|---|---|---|---|---|---|---|
| avant (0 connexion) | 856 | 2480 | 2480 | 860 | 376 | 243 | 0 | 1632 |
| avec 64 connexions inactives | 31544 | 33168 | 33536 | 31508 | 528 | 339 | 0 | 32536 |
| après la diffusion | 31544 | 33232 | 33536 | 31548 | 596 | 403 | 0 | 32604 |
| après fermeture | 896 | 2584 | 33536 | 864 | 424 | 297 | 0 | 1504 |

Par connexion (avec 64 − avant) / 64 : RssAnon **479.50 Kio**, VmRSS 479.50 Kio, cgroup `anon` 478.88 Kio, `kernel` 2.38 Kio, `slab` 1.50 Kio, `sock` 0.00 Kio, `memory.current` 482.88 Kio.

CPU du serveur (utime+stime) :
- 64 connexions inactives pendant 16 s : 0 ms ; octets reçus par les clients dans l'intervalle : 0 (maintiens SSE)
- diffusion de 100 événements (10/s × 10 s) à 64 connexions : **150 ms** ; 6400/6400 livraisons reçues ; 613760 o reçus par les clients (95.9 o par livraison, cadrage compris)
- soit 23.4 µs de CPU serveur par livraison et 1.5 % d'un cœur pendant les 10 s
RESULTAT {"apres":{"RssAnon":31544,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":35688,"VmHWM":33536,"VmRSS":33168,"VmSize":38604,"VmStk":132,"cg_anon":31508,"cg_current":32536,"cg_file":0,"cg_kernel":528,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":32664,"cg_percpu":2,"cg_shmem":0,"cg_slab":339,"cg_sock":0,"conns":64,"cpu_ms":100,"fds":75,"nofile":"65536/65536","pid":1,"stime_ms":90,"utime_ms":10},"attendus":6400,"avant":{"RssAnon":856,"RssFile":1624,"RssShmem":0,"Threads":3,"VmData":5016,"VmHWM":2480,"VmRSS":2480,"VmSize":7932,"VmStk":132,"cg_anon":860,"cg_current":1632,"cg_file":0,"cg_kernel":376,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":3732,"cg_percpu":2,"cg_shmem":0,"cg_slab":243,"cg_sock":0,"conns":0,"cpu_ms":0,"fds":11,"nofile":"65536/65536","pid":1,"stime_ms":0,"utime_ms":0},"cpu_diffusion_ms":150,"cpu_inactif_ms":0,"diffusion":{"RssAnon":31544,"RssFile":1688,"RssShmem":0,"Threads":3,"VmData":35688,"VmHWM":33536,"VmRSS":33232,"VmSize":38604,"VmStk":132,"cg_anon":31548,"cg_current":32604,"cg_file":0,"cg_kernel":596,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":32892,"cg_percpu":2,"cg_shmem":0,"cg_slab":403,"cg_sock":0,"conns":64,"cpu_ms":250,"fds":75,"nofile":"65536/65536","pid":1,"stime_ms":190,"utime_ms":60},"ferme":{"RssAnon":896,"RssFile":1688,"RssShmem":0,"Threads":3,"VmData":5020,"VmHWM":33536,"VmRSS":2584,"VmSize":7936,"VmStk":132,"cg_anon":864,"cg_current":1504,"cg_file":0,"cg_kernel":424,"cg_kernel_stack":48,"cg_max":131072,"cg_peak":32892,"cg_percpu":2,"cg_shmem":0,"cg_slab":297,"cg_sock":0,"conns":0,"cpu_ms":260,"fds":11,"nofile":"65536/65536","pid":1,"stime_ms":200,"utime_ms":60},"hz":10,"livres":6400,"mode":"ws-leger","n":64,"octets_diffusion":613760,"secs":10,"secs_inactif":16,"snapshot":480731,"temps_connexion_s":0.10515403}
- serveur : OOMKilled=false, état=running, journal : serveur ws-leger sur :8800, snapshot 480731 o, nofile Some(String("65536/65536"))

## Synthèse : coût par connexion (processus serveur)

| mode | snapshot | N | RssAnon avant → avec (Kio) | **Δ RssAnon / connexion (Kio)** (macOS : VmRSS) | Δ VmRSS / connexion (Kio) | VmHWM après connexion (Kio) | Δ `memory.current` du cgroup / connexion (Kio) | CPU inactif (ms/s) | CPU diffusion (µs/livraison) | livré |
|---|---|---|---|---|---|---|---|---|---|---|
| sse | 4096 o | 1 | 372 → 424 | **52.00** | 116.00 | 2048 | 88.00 | 0.62 | 400.00 | 100/100 |
| sse | 4096 o | 64 | 372 → 2944 | **40.19** | 41.19 | 4568 | 42.38 | 0.00 | 26.56 | 6400/6400 |
| sse | 4096 o | 1000 | 400 → 39512 | **39.11** | 39.18 | 41136 | 41.54 | 2.50 | 17.20 | 100000/100000 |
| sse | 480731 o | 64 | 856 → 3232 | **37.12** | 38.12 | 5520 | 39.56 | 0.00 | 25.00 | 6400/6400 |
| sse-nu | 4096 o | 64 | 352 → 2244 | **29.56** | 29.56 | 3868 | 31.06 | 0.00 | 25.00 | 6400/6400 |
| sse-nu | 4096 o | 1000 | 340 → 29936 | **29.60** | 29.60 | 31560 | 31.70 | 0.62 | 15.80 | 100000/100000 |
| ws | 4096 o | 1 | 372 → 524 | **152.00** | 216.00 | 2148 | 124.00 | 0.00 | 400.00 | 100/100 |
| ws | 4096 o | 64 | 400 → 9328 | **139.50** | 140.50 | 10952 | 142.19 | 0.00 | 31.25 | 6400/6400 |
| ws | 4096 o | 250 | 372 → 35100 | **138.91** | 139.17 | 36724 | 139.97 | 0.00 | 14.00 | 25000/25000 |
| ws | 4096 o | 500 | 372 → 69680 | **138.62** | 138.74 | 71304 | 140.92 | 0.00 | 17.00 | 50000/50000 |
| ws | 4096 o | 1000 | 372 → — | **—** | — | — | — | — | — | 0/100000 |
| ws | 480731 o | 64 | 832 → 39700 | **607.31** | 608.31 | 41488 | 610.81 | 0.00 | 34.38 | 6400/6400 |
| ws-leger | 4096 o | 1 | 372 → 436 | **64.00** | 128.00 | 2060 | 28.00 | 0.00 | 300.00 | 100/100 |
| ws-leger | 4096 o | 64 | 372 → 1172 | **12.50** | 13.50 | 2816 | 14.69 | 0.00 | 20.31 | 6400/6400 |
| ws-leger | 4096 o | 1000 | 372 → 11448 | **11.08** | 11.14 | 13072 | 13.44 | 0.62 | 13.70 | 100000/100000 |
| ws-leger | 480731 o | 64 | 856 → 31544 | **479.50** | 479.50 | 33536 | 482.88 | 0.00 | 23.44 | 6400/6400 |

### Pente entre deux paliers (RssAnon, VmRSS, memory.current), Kio par connexion = (M(N₂) − M(N₁)) / (N₂ − N₁)

| mode | snapshot | N₁ → N₂ | pente RssAnon | pente VmRSS | pente `memory.current` |
|---|---|---|---|---|---|
| sse | 4096 o | 64 → 1000 | **39.07 Kio** | 39.07 Kio | 41.49 Kio |
| sse-nu | 4096 o | 64 → 1000 | **29.59 Kio** | 29.59 Kio | 31.73 Kio |
| ws | 4096 o | 64 → 500 | **138.42 Kio** | 138.42 Kio | 140.73 Kio |
| ws-leger | 4096 o | 64 → 1000 | **10.98 Kio** | 10.98 Kio | 13.36 Kio |
`````

Sortie brute — `etat_vif` (Linux) :

`````text
# État vif seul (sans historique) : mémoire demandée et mémoire réelle, ×1, ×3 et ×10 « deux maisons »
- plateforme : linux / aarch64 ; tailles : Value 24 o, PointVif 80 o, Event 128 o
- « deux maisons » ×1 = 496 appareils, 2 264 valeurs, 400 automatismes ; ×3 = 1 488 / 6 792 / 1 200 ; ×10 = 4 960 / 22 640 / 4 000 (constructeur de la V4)
- mémoire réelle au démarrage du banc, avant toute charge : RSS 1128 Kio, RssAnon 32 Kio, anonyme du cgroup 32 Kio, memory.current du cgroup 784 Kio

## ×1 : 496 appareils, 2264 valeurs, 400 automatismes
Octets demandés à l'allocateur (octets + 16 o d'en-tête par bloc) :
- Modèle du hub (appareils, définitions, valeurs : ce qui existe) : 1.00 Mio
- Étage vif : précédente, 3 instants, qualité + index des points : 434 Kio
- Santé par appareil (vu le, échecs, disjoncteur) : 12 Kio
- Automatismes compilés (9 règles, 2 actions) + index par point : 203 Kio
- 1 000 baux, échéancier et une exécution en cours par automatisme : 150 Kio
- Agrégats 1 h et 24 h des points lus par les règles : 443 Kio
- Voies paresseuses (répartiteur unique, files à la demande), au repos : 6 Kio
- Tampon du bus : 1 024 événements (charges partagées en Arc) : 128 Kio
- **Sous-total « tout l'état vif »** : 2.34 Mio (2458576 octets)
Mémoire réelle du processus (une échelle par processus neuf) :
- au démarrage du processus (avant le runtime et le hub) : RSS 1128 Kio, RssAnon 32 Kio, anonyme du cgroup 32 Kio, memory.current du cgroup 1008 Kio
- avec tout l'état vif : RSS 5816 Kio, RssAnon 3692 Kio, anonyme du cgroup 3732 Kio, memory.current du cgroup 4240 Kio
- **écart « état vif » − démarrage** : RSS +4688 Kio, RssAnon +3660 Kio, anonyme du cgroup +3700 Kio, memory.current du cgroup +3232 Kio
- rapport RssAnon ajoutée / octets demandés : 1.52
- aparté, mesure de la V4 : voies en tâches permanentes (file de 8) : 3.47 Mio pour 496 appareils, soit 7.2 Kio chacune ; CommandRequest 200 o ; mémoire réelle avec elles en plus : RSS 8596 Kio, RssAnon 6472 Kio, anonyme du cgroup 6516 Kio, memory.current du cgroup 7028 Kio

## ×3 : 1488 appareils, 6792 valeurs, 1200 automatismes
Octets demandés à l'allocateur (octets + 16 o d'en-tête par bloc) :
- Modèle du hub (appareils, définitions, valeurs : ce qui existe) : 2.95 Mio
- Étage vif : précédente, 3 instants, qualité + index des points : 1.17 Mio
- Santé par appareil (vu le, échecs, disjoncteur) : 35 Kio
- Automatismes compilés (9 règles, 2 actions) + index par point : 609 Kio
- 1 000 baux, échéancier et une exécution en cours par automatisme : 467 Kio
- Agrégats 1 h et 24 h des points lus par les règles : 1.36 Mio
- Voies paresseuses (répartiteur unique, files à la demande), au repos : 7 Kio
- Tampon du bus : 1 024 événements (charges partagées en Arc) : 128 Kio
- **Sous-total « tout l'état vif »** : 6.70 Mio (7026600 octets)
Mémoire réelle du processus (une échelle par processus neuf) :
- au démarrage du processus (avant le runtime et le hub) : RSS 1128 Kio, RssAnon 32 Kio, anonyme du cgroup 76 Kio, memory.current du cgroup 1040 Kio
- avec tout l'état vif : RSS 10284 Kio, RssAnon 8160 Kio, anonyme du cgroup 8204 Kio, memory.current du cgroup 8888 Kio
- **écart « état vif » − démarrage** : RSS +9156 Kio, RssAnon +8128 Kio, anonyme du cgroup +8128 Kio, memory.current du cgroup +7848 Kio
- rapport RssAnon ajoutée / octets demandés : 1.18
- aparté, mesure de la V4 : voies en tâches permanentes (file de 8) : 10.40 Mio pour 1488 appareils, soit 7.2 Kio chacune ; CommandRequest 200 o ; mémoire réelle avec elles en plus : RSS 18484 Kio, RssAnon 16360 Kio, anonyme du cgroup 16404 Kio, memory.current du cgroup 17040 Kio

## ×10 : 4960 appareils, 22640 valeurs, 4000 automatismes
Octets demandés à l'allocateur (octets + 16 o d'en-tête par bloc) :
- Modèle du hub (appareils, définitions, valeurs : ce qui existe) : 9.86 Mio
- Étage vif : précédente, 3 instants, qualité + index des points : 4.04 Mio
- Santé par appareil (vu le, échecs, disjoncteur) : 116 Kio
- Automatismes compilés (9 règles, 2 actions) + index par point : 1.98 Mio
- 1 000 baux, échéancier et une exécution en cours par automatisme : 1.56 Mio
- Agrégats 1 h et 24 h des points lus par les règles : 4.69 Mio
- Voies paresseuses (répartiteur unique, files à la demande), au repos : 10 Kio
- Tampon du bus : 1 024 événements (charges partagées en Arc) : 128 Kio
- **Sous-total « tout l'état vif »** : 22.39 Mio (23476712 octets)
Mémoire réelle du processus (une échelle par processus neuf) :
- au démarrage du processus (avant le runtime et le hub) : RSS 1128 Kio, RssAnon 32 Kio, anonyme du cgroup 76 Kio, memory.current du cgroup 1000 Kio
- avec tout l'état vif : RSS 26304 Kio, RssAnon 24180 Kio, anonyme du cgroup 24220 Kio, memory.current du cgroup 25180 Kio
- **écart « état vif » − démarrage** : RSS +25176 Kio, RssAnon +24148 Kio, anonyme du cgroup +24144 Kio, memory.current du cgroup +24180 Kio
- rapport RssAnon ajoutée / octets demandés : 1.05
- aparté, mesure de la V4 : voies en tâches permanentes (file de 8) : 34.69 Mio pour 4960 appareils, soit 7.2 Kio chacune ; CommandRequest 200 o ; mémoire réelle avec elles en plus : RSS 53516 Kio, RssAnon 51392 Kio, anonyme du cgroup 51436 Kio, memory.current du cgroup 52220 Kio
`````

Sortie brute — `courtier` (Linux) :

`````text
# Banc `courtier` (rumqttd 0.20.0 sans TLS ni WebSocket ; Linux, courtier : 2 CPU / 128 Mio, clients : autre conteneur sauf mesures en processus)

## Tailles de structures de rumqttd
- size_of : Notification 248 o, Packet 392 o, Publish 72 o, Forward 248 o ; tampons alloués d'avance par connexion : 200 × (Packet + Notification) = 125 Kio
- conteneur : OOMKilled=false, code de sortie=0

## Liens locaux de rumqttd (segments 1 Mio × 4)
- liens locaux `rumqttd::local::{LinkTx, LinkRx}` (QoS 0, sans TCP), éditeur et abonné dans deux fils du même processus, routeur = fil `router-0` de rumqttd
- charge JSON de 120 o, 64 topics `moli/z2m:0x…/temperature` (≈ 49 o), un abonné `moli/#` ; segments du courtier 1048576 o × 4
- latence = de l'appel `publish()` (instant relevé juste avant) à la réception de la notification par l'abonné ; l'éditeur dort 50 µs au plus entre deux salves et envoie ce qui est dû (retard de l'éditeur sur son horaire en colonne `retard`)

| débit offert | envoyés | reçus | perdus | débit reçu | p50 | p90 | p99 | p99,9 | max | retard p99 de l'éditeur | CPU routeur / éditeur / abonné (ms) | RssAnon avant → après (Kio) |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1000/s × 5 s | 5000 | 5000 | 0 | **1000/s** | 27 µs | 35 µs | **279 µs** | 3.80 ms | 58.14 ms | 7.48 ms | 50 / 170 / 70 | 872 → 2932 |
| 5000/s × 5 s | 25000 | 25000 | 0 | **5000/s** | 26 µs | 44 µs | **328 µs** | 16.78 ms | 17.61 ms | 722 µs | 260 / 210 / 80 | 2872 → 10516 |
| 10000/s × 5 s | 50000 | 50000 | 0 | **10000/s** | 25 µs | 28 µs | **62 µs** | 803 µs | 1.93 ms | 81 µs | 440 / 280 / 160 | 10300 → 10156 |
| 20000/s × 5 s | 100000 | 100000 | 0 | **19999/s** | 25 µs | 38 µs | **623 µs** | 2.42 ms | 5.56 ms | 781 µs | 570 / 340 / 170 | 9744 → 9644 |
| 50000/s × 5 s | 250000 | 250000 | 0 | **49992/s** | 26 µs | 41 µs | **1.87 ms** | 16.78 ms | 32.76 ms | 3.18 ms | 820 / 390 / 220 | 8844 → 13872 |
| à fond × 5 s | 5512967 | 1104807 | 4408160 | **200874/s** | 16.78 ms | 27.26 ms | **48.23 ms** | 109.05 ms | 115.47 ms | 0 µs | 4180 / 2370 / 130 | 11900 → 12328 |
- conteneur : OOMKilled=false, code de sortie=0

## tokio::sync::broadcast (1 024 places) avec les vrais moli_core::Event
- `tokio::sync::broadcast::channel::<moli_core::Event>(1024)` (capacité du bus du hub, `HubOptions::event_capacity`), un abonné ; `Event::State { point, value, ts, source_ts }` cloné à la réception (Arc), sans sérialisation ; `size_of::<Event>()` = 128 o
- même éditeur (salves, sommeil de 50 µs au plus) et même mesure de latence que pour les liens locaux (`ts` = ns de l'envoi) ; `perdus` = messages écrasés pour un abonné en retard (`RecvError::Lagged`)

| débit offert | envoyés | reçus | perdus | débit reçu | p50 | p90 | p99 | p99,9 | max | retard p99 de l'éditeur | CPU routeur / éditeur / abonné (ms) | RssAnon avant → après (Kio) |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1000/s × 5 s | 5000 | 5000 | 0 | **1000/s** | 13 µs | 32 µs | **115 µs** | 2.95 ms | 7.97 ms | 359 µs | — / 140 / 10 | 212 → 268 |
| 5000/s × 5 s | 25000 | 25000 | 0 | **5000/s** | 12 µs | 24 µs | **688 µs** | 6.03 ms | 18.26 ms | 5.14 ms | — / 240 / 80 | 212 → 424 |
| 10000/s × 5 s | 50000 | 50000 | 0 | **10000/s** | 12 µs | 16 µs | **176 µs** | 5.24 ms | 8.98 ms | 2.15 ms | — / 310 / 150 | 212 → 620 |
| 20000/s × 5 s | 100000 | 100000 | 0 | **20000/s** | 10 µs | 25 µs | **2.10 ms** | 13.37 ms | 13.57 ms | 5.81 ms | — / 370 / 190 | 212 → 1012 |
| 50000/s × 5 s | 250000 | 250000 | 0 | **50000/s** | 9 µs | 14 µs | **115 µs** | 10.75 ms | 10.89 ms | 321 µs | — / 400 / 230 | 212 → 2184 |
| à fond × 5 s | 34014255 | 33886585 | 127670 | **6777317/s** | 0 µs | 9 µs | **74 µs** | 152 µs | 4.75 ms | 0 µs | — / 4980 / 4940 | 212 → 224 |
- conteneur : OOMKilled=false, code de sortie=0

## Croissance mémoire selon le volume publié, configuration d'exemple (segments 100 Mio × 10)

### segments 104857600 o × 10 (comptabilité du journal : 4 + topic + charge par message), message = topic 49 o + charge 120 o
| messages publiés | volume comptable | RssAnon (Kio) | RssAnon par message en mémoire (o) |
|---|---|---|---|
| 25000 | 4.1 Mio | 11668 | 467 |
| 50000 | 8.2 Mio | 21572 | 436 |
| 100000 | 16.5 Mio | 41116 | 418 |
| 200000 | 33.0 Mio | 80160 | 409 |
- conteneur : OOMKilled=true, code de sortie=137

## Croissance mémoire selon le volume publié, segments 256 Kio × 4

### segments 262144 o × 4 (comptabilité du journal : 4 + topic + charge par message), message = topic 49 o + charge 120 o
| messages publiés | volume comptable | RssAnon (Kio) | RssAnon par message en mémoire (o) |
|---|---|---|---|
| 25000 | 4.1 Mio | 4368 | 168 |
| 50000 | 8.2 Mio | 4120 | 79 |
| 100000 | 16.5 Mio | 4196 | 40 |
| 200000 | 33.0 Mio | 4336 | 21 |
| 400000 | 66.0 Mio | 4464 | 11 |
| 600000 | 99.0 Mio | 4588 | 7 |
| 800000 | 132.0 Mio | 4260 | 5 |
| 1000000 | 165.0 Mio | 4500 | 4 |
| 1200000 | 198.0 Mio | 4560 | 4 |
| 1400000 | 231.0 Mio | 4264 | 3 |
(RssAnon au départ, courtier et liens créés : 264 Kio ; lignes `kio` : 7540)
- conteneur : OOMKilled=false, code de sortie=0

## Plateau du journal, segments 1 Mio × 4 : publication à fond jusqu'au plafond

### segments 1048576 o × 4 (comptabilité du journal : 4 + topic + charge par message), message = topic 49 o + charge 120 o
| messages publiés | volume comptable | RssAnon (Kio) | RssAnon par message en mémoire (o) |
|---|---|---|---|
| 25000 | 4.1 Mio | 11704 | 469 |
| 50000 | 8.2 Mio | 11536 | 231 |
| 100000 | 16.5 Mio | 11012 | 110 |
| 200000 | 33.0 Mio | 10100 | 50 |
| 400000 | 66.0 Mio | 10284 | 26 |
| 600000 | 99.0 Mio | 10508 | 17 |
| 800000 | 132.0 Mio | 10724 | 13 |
| 1000000 | 165.0 Mio | 10936 | 11 |
(RssAnon au départ, courtier et liens créés : 264 Kio ; lignes `kio` : 15028)
- conteneur : OOMKilled=false, code de sortie=0

## Plateau du journal, segments 64 Kio × 4 : publication à fond jusqu'au plafond

### segments 65536 o × 4 (comptabilité du journal : 4 + topic + charge par message), message = topic 49 o + charge 120 o
| messages publiés | volume comptable | RssAnon (Kio) | RssAnon par message en mémoire (o) |
|---|---|---|---|
| 25000 | 4.1 Mio | 2644 | 98 |
| 50000 | 8.2 Mio | 2680 | 50 |
| 100000 | 16.5 Mio | 2708 | 25 |
| 200000 | 33.0 Mio | 2744 | 13 |
| 400000 | 66.0 Mio | 2768 | 6 |
| 600000 | 99.0 Mio | 2828 | 4 |
| 800000 | 132.0 Mio | 2780 | 3 |
| 1000000 | 165.0 Mio | 2848 | 3 |
(RssAnon au départ, courtier et liens créés : 260 Kio ; lignes `kio` : 5852)
- conteneur : OOMKilled=false, code de sortie=0

## Journal sous trafic soutenu : 200 messages/s pendant 60 s, 4 abonnés, segments 1 Mio × 4

### segments 1048576 o × 4 (4096 Kio de journal comptable par filtre au plus) · 200 messages/s pendant 60 s · charge 115 o · topics `moli/v1/val/z2m:0x…/<clé>` (≈ 47 o), 2 264 points distincts · 4 abonnés en mémoire à `moli/v1/val/#`
| t (s) | messages publiés | RssAnon (Kio) | écart avec le départ (Kio) |
|---|---|---|---|
| 0 | 0 | 380 | 0 |
| 10 | 2001 | 2484 | +2104 |
| 20 | 4001 | 4100 | +3720 |
| 30 | 6001 | 5348 | +4968 |
| 40 | 8001 | 7320 | +6940 |
| 50 | 10001 | 8568 | +8188 |
| 60 (fin) | 12000 | 9780 | +9400 |

RssAnon avant → après : **380 → 9780 Kio (+9400 Kio)** ; messages reçus par chacun des 4 abonnés : [12000, 12000, 12000, 12000] (publiés : 12000) ; mémoire par message publié encore en journal : 802 o ; pic VmHWM : 12492 Kio
- conteneur : OOMKilled=false, code de sortie=0

## Journal sous trafic soutenu : 200 messages/s pendant 60 s, 4 abonnés, segments 64 Kio × 4

### segments 65536 o × 4 (256 Kio de journal comptable par filtre au plus) · 200 messages/s pendant 60 s · charge 115 o · topics `moli/v1/val/z2m:0x…/<clé>` (≈ 47 o), 2 264 points distincts · 4 abonnés en mémoire à `moli/v1/val/#`
| t (s) | messages publiés | RssAnon (Kio) | écart avec le départ (Kio) |
|---|---|---|---|
| 0 | 0 | 380 | 0 |
| 10 | 2001 | 2328 | +1948 |
| 20 | 4001 | 2948 | +2568 |
| 30 | 6001 | 3212 | +2832 |
| 40 | 8001 | 4192 | +3812 |
| 50 | 10001 | 4436 | +4056 |
| 60 (fin) | 12000 | 4656 | +4276 |

RssAnon avant → après : **380 → 4656 Kio (+4276 Kio)** ; messages reçus par chacun des 4 abonnés : [12000, 12000, 12000, 12000] (publiés : 12000) ; mémoire par message publié encore en journal : 365 o ; pic VmHWM : 7396 Kio
- conteneur : OOMKilled=false, code de sortie=0

## Seuil de retard, segments 64 Kio × 4

### seuil de retard · segments 65536 o × 4 · message comptable = 4 + topic (46 o) + charge 115 o = 165 o → 398 messages par segment, journal retenu entre 1194 et 1592 messages ; l'abonné garde de surcroît 199 notifications déjà poussées dans sa file de sortie
| messages publiés pendant que l'abonné ne lit pas | reçus après reprise | **perdus** | séquences manquantes | ordre | notifications reçues (Forward / Unschedule / autres) | alertes du routeur |
|---|---|---|---|---|---|---|
| 200 | 200 | **0** | — | conservé | 200 / 1 / 0 | aucune |
| 600 | 600 | **0** | — | conservé | 600 / 3 / 0 | aucune |
| 1000 | 1000 | **0** | — | conservé | 1000 / 5 / 0 | aucune |
| 1200 | 1200 | **0** | — | conservé | 1200 / 6 / 0 | aucune |
| 1400 | 1400 | **0** | — | conservé | 1400 / 7 / 0 | aucune |
| 1500 | 1500 | **0** | — | conservé | 1500 / 7 / 0 | aucune |
| 1550 | 1550 | **0** | — | conservé | 1550 / 7 / 0 | aucune |
| 1575 | 1575 | **0** | — | conservé | 1575 / 7 / 0 | aucune |
| 1580 | 1580 | **0** | — | conservé | 1580 / 7 / 0 | aucune |
| 1585 | 1585 | **0** | — | conservé | 1585 / 7 / 0 | aucune |
| 1590 | 1590 | **0** | — | conservé | 1590 / 7 / 0 | aucune |
| 1595 | 1595 | **0** | — | conservé | 1595 / 7 / 0 | aucune |
| 1600 | 1600 | **0** | — | conservé | 1600 / 8 / 0 | aucune |
| 1605 | 1605 | **0** | — | conservé | 1605 / 8 / 0 | aucune |
| 1610 | 1610 | **0** | — | conservé | 1610 / 8 / 0 | aucune |
| 1650 | 1446 | **204** | 200–403 | conservé | 1446 / 7 / 0 | 1 : cursor_jump (client `en-retard`, Filter: moli/v1/val/#, Lost: 0) |
| 1700 | 1496 | **204** | 200–403 | conservé | 1496 / 7 / 0 | 1 : cursor_jump (client `en-retard`, Filter: moli/v1/val/#, Lost: 0) |
| 2000 | 1796 | **204** | 200–403 | conservé | 1796 / 8 / 0 | 1 : cursor_jump (client `en-retard`, Filter: moli/v1/val/#, Lost: 0) |
| 3000 | 1584 | **1416** | 200–1615 | conservé | 1584 / 7 / 0 | 1 : cursor_jump (client `en-retard`, Filter: moli/v1/val/#, Lost: 0) |
| 5000 | 1564 | **3436** | 200–3635 | conservé | 1564 / 7 / 0 | 1 : cursor_jump (client `en-retard`, Filter: moli/v1/val/#, Lost: 0) |
| 10000 | 1716 | **8284** | 200–8483 | conservé | 1716 / 8 / 0 | 1 : cursor_jump (client `en-retard`, Filter: moli/v1/val/#, Lost: 0) |
- conteneur : OOMKilled=false, code de sortie=0

## Seuil de retard, segments 1 Mio × 4 (pour comparaison), L = 20000

### seuil de retard · segments 1048576 o × 4 · message comptable = 4 + topic (46 o) + charge 115 o = 165 o → 6356 messages par segment, journal retenu entre 19068 et 25424 messages ; l'abonné garde de surcroît 199 notifications déjà poussées dans sa file de sortie
| messages publiés pendant que l'abonné ne lit pas | reçus après reprise | **perdus** | séquences manquantes | ordre | notifications reçues (Forward / Unschedule / autres) | alertes du routeur |
|---|---|---|---|---|---|---|
| 20000 | 20000 | **0** | — | conservé | 20000 / 100 / 0 | aucune |
- conteneur : OOMKilled=false, code de sortie=0

## Seuil de retard, segments 1 Mio × 4 (pour comparaison), L = 25000

### seuil de retard · segments 1048576 o × 4 · message comptable = 4 + topic (46 o) + charge 115 o = 165 o → 6356 messages par segment, journal retenu entre 19068 et 25424 messages ; l'abonné garde de surcroît 199 notifications déjà poussées dans sa file de sortie
| messages publiés pendant que l'abonné ne lit pas | reçus après reprise | **perdus** | séquences manquantes | ordre | notifications reçues (Forward / Unschedule / autres) | alertes du routeur |
|---|---|---|---|---|---|---|
| 25000 | 25000 | **0** | — | conservé | 25000 / 125 / 0 | aucune |
- conteneur : OOMKilled=false, code de sortie=0

## Seuil de retard, segments 1 Mio × 4 (pour comparaison), L = 25500

### seuil de retard · segments 1048576 o × 4 · message comptable = 4 + topic (46 o) + charge 115 o = 165 o → 6356 messages par segment, journal retenu entre 19068 et 25424 messages ; l'abonné garde de surcroît 199 notifications déjà poussées dans sa file de sortie
| messages publiés pendant que l'abonné ne lit pas | reçus après reprise | **perdus** | séquences manquantes | ordre | notifications reçues (Forward / Unschedule / autres) | alertes du routeur |
|---|---|---|---|---|---|---|
| 25500 | 25500 | **0** | — | conservé | 25500 / 127 / 0 | aucune |
- conteneur : OOMKilled=false, code de sortie=0

## Seuil de retard, segments 1 Mio × 4 (pour comparaison), L = 26000

### seuil de retard · segments 1048576 o × 4 · message comptable = 4 + topic (46 o) + charge 115 o = 165 o → 6356 messages par segment, journal retenu entre 19068 et 25424 messages ; l'abonné garde de surcroît 199 notifications déjà poussées dans sa file de sortie
| messages publiés pendant que l'abonné ne lit pas | reçus après reprise | **perdus** | séquences manquantes | ordre | notifications reçues (Forward / Unschedule / autres) | alertes du routeur |
|---|---|---|---|---|---|---|
| 26000 | 19743 | **6257** | 200–6456 | conservé | 19743 / 98 / 0 | 1 : cursor_jump (client `en-retard`, Filter: moli/v1/val/#, Lost: 0) |
- conteneur : OOMKilled=false, code de sortie=0

## Seuil de retard, segments 1 Mio × 4 (pour comparaison), L = 26500

### seuil de retard · segments 1048576 o × 4 · message comptable = 4 + topic (46 o) + charge 115 o = 165 o → 6356 messages par segment, journal retenu entre 19068 et 25424 messages ; l'abonné garde de surcroît 199 notifications déjà poussées dans sa file de sortie
| messages publiés pendant que l'abonné ne lit pas | reçus après reprise | **perdus** | séquences manquantes | ordre | notifications reçues (Forward / Unschedule / autres) | alertes du routeur |
|---|---|---|---|---|---|---|
| 26500 | 20243 | **6257** | 200–6456 | conservé | 20243 / 101 / 0 | 1 : cursor_jump (client `en-retard`, Filter: moli/v1/val/#, Lost: 0) |
- conteneur : OOMKilled=false, code de sortie=0

## Seuil de retard, segments 1 Mio × 4 (pour comparaison), L = 30000

### seuil de retard · segments 1048576 o × 4 · message comptable = 4 + topic (46 o) + charge 115 o = 165 o → 6356 messages par segment, journal retenu entre 19068 et 25424 messages ; l'abonné garde de surcroît 199 notifications déjà poussées dans sa file de sortie
| messages publiés pendant que l'abonné ne lit pas | reçus après reprise | **perdus** | séquences manquantes | ordre | notifications reçues (Forward / Unschedule / autres) | alertes du routeur |
|---|---|---|---|---|---|---|
| 30000 | 23743 | **6257** | 200–6456 | conservé | 23743 / 118 / 0 | 1 : cursor_jump (client `en-retard`, Filter: moli/v1/val/#, Lost: 0) |
- conteneur : OOMKilled=false, code de sortie=0

## Seuil de retard selon la phase du journal, segments 64 Kio × 4 (dichotomie), phase 0

### seuil de retard selon la phase du journal · segments 65536 o × 4
| messages déjà publiés avant l'abonnement (phase) | **plus petit L avec perte** | perdus à ce L | dernier L sans perte | pertes à L + 200 |
|---|---|---|---|---|
| 0 | **1617** | 204 | 1616 | 204 |
- conteneur : OOMKilled=false, code de sortie=0

## Seuil de retard selon la phase du journal, segments 64 Kio × 4 (dichotomie), phase 100

### seuil de retard selon la phase du journal · segments 65536 o × 4
| messages déjà publiés avant l'abonnement (phase) | **plus petit L avec perte** | perdus à ce L | dernier L sans perte | pertes à L + 200 |
|---|---|---|---|---|
| 100 | **1517** | 104 | 1516 | 104 |
- conteneur : OOMKilled=false, code de sortie=0

## Seuil de retard selon la phase du journal, segments 64 Kio × 4 (dichotomie), phase 203

### seuil de retard selon la phase du journal · segments 65536 o × 4
| messages déjà publiés avant l'abonnement (phase) | **plus petit L avec perte** | perdus à ce L | dernier L sans perte | pertes à L + 200 |
|---|---|---|---|---|
| 203 | **1414** | 1 | 1413 | 1 |
- conteneur : OOMKilled=false, code de sortie=0

## Seuil de retard selon la phase du journal, segments 64 Kio × 4 (dichotomie), phase 204

### seuil de retard selon la phase du journal · segments 65536 o × 4
| messages déjà publiés avant l'abonnement (phase) | **plus petit L avec perte** | perdus à ce L | dernier L sans perte | pertes à L + 200 |
|---|---|---|---|---|
| 204 | **1817** | 404 | 1816 | 404 |
- conteneur : OOMKilled=false, code de sortie=0

## Seuil de retard selon la phase du journal, segments 64 Kio × 4 (dichotomie), phase 205

### seuil de retard selon la phase du journal · segments 65536 o × 4
| messages déjà publiés avant l'abonnement (phase) | **plus petit L avec perte** | perdus à ce L | dernier L sans perte | pertes à L + 200 |
|---|---|---|---|---|
| 205 | **1816** | 403 | 1815 | 403 |
- conteneur : OOMKilled=false, code de sortie=0

## Seuil de retard selon la phase du journal, segments 64 Kio × 4 (dichotomie), phase 300

### seuil de retard selon la phase du journal · segments 65536 o × 4
| messages déjà publiés avant l'abonnement (phase) | **plus petit L avec perte** | perdus à ce L | dernier L sans perte | pertes à L + 200 |
|---|---|---|---|---|
| 300 | **1721** | 308 | 1720 | 308 |
- conteneur : OOMKilled=false, code de sortie=0

## Seuil de retard selon la phase du journal, segments 64 Kio × 4 (dichotomie), phase 403

### seuil de retard selon la phase du journal · segments 65536 o × 4
| messages déjà publiés avant l'abonnement (phase) | **plus petit L avec perte** | perdus à ce L | dernier L sans perte | pertes à L + 200 |
|---|---|---|---|---|
| 403 | **1618** | 205 | 1617 | 205 |
- conteneur : OOMKilled=false, code de sortie=0

## Retenus, sans abonné attrape-tout

### mémoire du courtier selon les messages retenus · abonné attrape-tout `moli/#` présent : non
- un message retenu = un point : topic `moli/z2m:0x00124b00<appareil>/<clé>` (≈ 45 o), charge JSON de 150 o, QoS 1, retain ; posés par un `rumqttc` d'un autre processus ; `ulimit -n` du courtier : 65536/65536
- niveaux : 2 264 (une maison), 6 792 (×3), 22 640 (×10) ; acquittés : 22640/22 640

| phase (mesurée par le courtier sur lui-même, Kio) | RssAnon | VmRSS | VmHWM (pic) | fils | descripteurs | cgroup `slab` | cgroup `memory.current` |
|---|---|---|---|---|---|---|---|
| au repos | 184 | 2624 | 2624 | 4 | 9 | 244 | 836 |
| avec 2264 retenus | 2340 | 5100 | 5216 | 4 | 10 | 254 | 2984 |
| avec 6792 retenus | 6820 | 9580 | 9580 | 4 | 10 | 257 | 7292 |
| avec 22640 retenus | 25004 | 27764 | 27764 | 4 | 10 | 255 | 25688 |
| après fermeture des clients (les retenus restent) | 23632 | 26392 | 32472 | 4 | 10 | 256 | 24320 |

| niveau | pose (durée) | RssAnon ajouté depuis le départ (Kio) | **octets par retenu (cumul)** | ajouté depuis le niveau précédent (Kio) | octets par retenu (incrément) |
|---|---|---|---|---|---|
| 2264 | 0.04 s | 2156 | **975** | 2156 | 975 |
| 6792 | 0.05 s | 6636 | **1000** | 4480 | 1013 |
| 22640 | 0.10 s | 24820 | **1123** | 18184 | 1175 |

Messages retenus reçus par un NOUVEL abonné `moli/#` (attendus : tous les retenus en place) :

| retenus en place | abonnement QoS 0 | abonnement QoS 1 |
|---|---|---|
| 2264 | **200** | **100** |
| 6792 | **200** | **100** |
| 22640 | **200** | **100** |
RESULTAT {"au_repos":{"RssAnon":184,"RssFile":2440,"Threads":4,"VmData":15772,"VmHWM":2624,"VmRSS":2624,"VmSize":19668,"cg_anon":184,"cg_current":836,"cg_file":0,"cg_kernel":400,"cg_max":131072,"cg_peak":3784,"cg_slab":244,"cg_sock":0,"cpu_ms":250,"fds":9,"nofile":"65536/65536","pid":1},"avec_abonne":{"RssAnon":184,"RssFile":2440,"Threads":4,"VmData":15772,"VmHWM":2624,"VmRSS":2624,"VmSize":19668,"cg_anon":184,"cg_current":836,"cg_file":0,"cg_kernel":400,"cg_max":131072,"cg_peak":3784,"cg_slab":244,"cg_sock":0,"cpu_ms":260,"fds":9,"nofile":"65536/65536","pid":1},"catchall":false,"ferme":{"RssAnon":23632,"RssFile":2760,"Threads":4,"VmData":42352,"VmHWM":32472,"VmRSS":26392,"VmSize":46248,"cg_anon":23632,"cg_current":24320,"cg_file":0,"cg_kernel":468,"cg_max":131072,"cg_peak":30636,"cg_slab":256,"cg_sock":0,"cpu_ms":430,"fds":10,"nofile":"65536/65536","pid":1},"niveaux":[{"mesure":{"RssAnon":2340,"RssFile":2760,"Threads":4,"VmData":18048,"VmHWM":5216,"VmRSS":5100,"VmSize":21944,"cg_anon":2340,"cg_current":2984,"cg_file":0,"cg_kernel":416,"cg_max":131072,"cg_peak":3784,"cg_slab":254,"cg_sock":0,"cpu_ms":270,"fds":10,"nofile":"65536/65536","pid":1},"n":2264,"pose_s":0.036420514,"recus_q0":200,"recus_q1":100},{"mesure":{"RssAnon":6820,"RssFile":2760,"Threads":4,"VmData":23352,"VmHWM":9580,"VmRSS":9580,"VmSize":27248,"cg_anon":6820,"cg_current":7292,"cg_file":0,"cg_kernel":424,"cg_max":131072,"cg_peak":7584,"cg_slab":257,"cg_sock":0,"cpu_ms":320,"fds":10,"nofile":"65536/65536","pid":1},"n":6792,"pose_s":0.045265609,"recus_q0":200,"recus_q1":100},{"mesure":{"RssAnon":25004,"RssFile":2760,"Threads":4,"VmData":43652,"VmHWM":27764,"VmRSS":27764,"VmSize":47548,"cg_anon":25004,"cg_current":25688,"cg_file":0,"cg_kernel":464,"cg_max":131072,"cg_peak":25892,"cg_slab":255,"cg_sock":0,"cpu_ms":410,"fds":10,"nofile":"65536/65536","pid":1},"n":22640,"pose_s":0.099076628,"recus_q0":200,"recus_q1":100}],"scenario":"memoire"}
- courtier : OOMKilled=false, état=running, journal : courtier rumqttd 0.20.0 sur :1883 (segments 104857600 o × 10, dynamic_filters=true, max_outgoing_packet_count=200), contrôle :8801, nofile Some(String("65536/65536"))

## Retenus, avec un abonné attrape-tout

### mémoire du courtier selon les messages retenus · abonné attrape-tout `moli/#` présent : oui (QoS 0)
- un message retenu = un point : topic `moli/z2m:0x00124b00<appareil>/<clé>` (≈ 45 o), charge JSON de 150 o, QoS 1, retain ; posés par un `rumqttc` d'un autre processus ; `ulimit -n` du courtier : 65536/65536
- niveaux : 2 264 (une maison), 6 792 (×3), 22 640 (×10) ; acquittés : 22640/22 640

| phase (mesurée par le courtier sur lui-même, Kio) | RssAnon | VmRSS | VmHWM (pic) | fils | descripteurs | cgroup `slab` | cgroup `memory.current` |
|---|---|---|---|---|---|---|---|
| au repos | 180 | 2620 | 2620 | 4 | 9 | 246 | 780 |
| avec l'abonné attrape-tout connecté | 308 | 3068 | 3068 | 4 | 10 | 252 | 936 |
| avec 2264 retenus | 3344 | 6168 | 6168 | 4 | 11 | 279 | 3984 |
| avec 6792 retenus | 7644 | 10468 | 10468 | 4 | 11 | 255 | 8296 |
| avec 22640 retenus | 25880 | 28704 | 28704 | 4 | 11 | 255 | 26572 |
| après fermeture des clients (les retenus restent) | 24316 | 27140 | 33500 | 4 | 10 | 261 | 25004 |

| niveau | pose (durée) | RssAnon ajouté depuis le départ (Kio) | **octets par retenu (cumul)** | ajouté depuis le niveau précédent (Kio) | octets par retenu (incrément) |
|---|---|---|---|---|---|
| 2264 | 0.04 s | 3036 | **1373** | 3036 | 1373 |
| 6792 | 0.05 s | 7336 | **1106** | 4300 | 972 |
| 22640 | 0.07 s | 25572 | **1157** | 18236 | 1178 |

Messages retenus reçus par un NOUVEL abonné `moli/#` (attendus : tous les retenus en place) :

| retenus en place | abonnement QoS 0 | abonnement QoS 1 |
|---|---|---|
| 2264 | **200** | **100** |
| 6792 | **200** | **100** |
| 22640 | **200** | **100** |
RESULTAT {"au_repos":{"RssAnon":180,"RssFile":2440,"Threads":4,"VmData":15768,"VmHWM":2620,"VmRSS":2620,"VmSize":19664,"cg_anon":180,"cg_current":780,"cg_file":0,"cg_kernel":400,"cg_max":131072,"cg_peak":3920,"cg_slab":246,"cg_sock":0,"cpu_ms":10,"fds":9,"nofile":"65536/65536","pid":1},"avec_abonne":{"RssAnon":308,"RssFile":2760,"Threads":4,"VmData":16292,"VmHWM":3068,"VmRSS":3068,"VmSize":20188,"cg_anon":308,"cg_current":936,"cg_file":0,"cg_kernel":416,"cg_max":131072,"cg_peak":3920,"cg_slab":252,"cg_sock":0,"cpu_ms":10,"fds":10,"nofile":"65536/65536","pid":1},"catchall":true,"ferme":{"RssAnon":24316,"RssFile":2824,"Threads":4,"VmData":42556,"VmHWM":33500,"VmRSS":27140,"VmSize":46452,"cg_anon":24316,"cg_current":25004,"cg_file":0,"cg_kernel":480,"cg_max":131072,"cg_peak":31284,"cg_slab":261,"cg_sock":0,"cpu_ms":190,"fds":10,"nofile":"65536/65536","pid":1},"niveaux":[{"mesure":{"RssAnon":3344,"RssFile":2824,"Threads":4,"VmData":19672,"VmHWM":6168,"VmRSS":6168,"VmSize":23568,"cg_anon":3344,"cg_current":3984,"cg_file":0,"cg_kernel":444,"cg_max":131072,"cg_peak":4296,"cg_slab":279,"cg_sock":4,"cpu_ms":40,"fds":11,"nofile":"65536/65536","pid":1},"n":2264,"pose_s":0.04061949,"recus_q0":200,"recus_q1":100},{"mesure":{"RssAnon":7644,"RssFile":2824,"Threads":4,"VmData":23884,"VmHWM":10468,"VmRSS":10468,"VmSize":27780,"cg_anon":7644,"cg_current":8296,"cg_file":0,"cg_kernel":432,"cg_max":131072,"cg_peak":8552,"cg_slab":255,"cg_sock":0,"cpu_ms":80,"fds":11,"nofile":"65536/65536","pid":1},"n":6792,"pose_s":0.045595208,"recus_q0":200,"recus_q1":100},{"mesure":{"RssAnon":25880,"RssFile":2824,"Threads":4,"VmData":44212,"VmHWM":28704,"VmRSS":28704,"VmSize":48108,"cg_anon":25880,"cg_current":26572,"cg_file":0,"cg_kernel":472,"cg_max":131072,"cg_peak":26668,"cg_slab":255,"cg_sock":0,"cpu_ms":150,"fds":11,"nofile":"65536/65536","pid":1},"n":22640,"pose_s":0.073292503,"recus_q0":200,"recus_q1":100}],"scenario":"memoire"}
- courtier : OOMKilled=false, état=running, journal : courtier rumqttd 0.20.0 sur :1883 (segments 104857600 o × 10, dynamic_filters=true, max_outgoing_packet_count=200), contrôle :8801, nofile Some(String("65536/65536"))

## Retenus, max_outgoing_packet_count = 30000

### mémoire du courtier selon les messages retenus · abonné attrape-tout `moli/#` présent : non
- un message retenu = un point : topic `moli/z2m:0x00124b00<appareil>/<clé>` (≈ 45 o), charge JSON de 150 o, QoS 1, retain ; posés par un `rumqttc` d'un autre processus ; `ulimit -n` du courtier : 65536/65536
- niveaux : 2 264 (une maison), 6 792 (×3), 22 640 (×10) ; acquittés : 22640/22 640

| phase (mesurée par le courtier sur lui-même, Kio) | RssAnon | VmRSS | VmHWM (pic) | fils | descripteurs | cgroup `slab` | cgroup `memory.current` |
|---|---|---|---|---|---|---|---|
| au repos | 180 | 2620 | 2620 | 4 | 9 | 245 | 772 |
| avec 2264 retenus | 2380 | 5204 | 5388 | 4 | 10 | 248 | 3004 |
| avec 6792 retenus | 6832 | 9656 | 9656 | 4 | 10 | 255 | 7492 |
| avec 22640 retenus | 25008 | 27832 | 27832 | 4 | 10 | 254 | 25712 |
| après fermeture des clients (les retenus restent) | 23632 | 26456 | 46208 | 4 | 10 | 256 | 24336 |

| niveau | pose (durée) | RssAnon ajouté depuis le départ (Kio) | **octets par retenu (cumul)** | ajouté depuis le niveau précédent (Kio) | octets par retenu (incrément) |
|---|---|---|---|---|---|
| 2264 | 0.03 s | 2200 | **995** | 2200 | 995 |
| 6792 | 0.04 s | 6652 | **1003** | 4452 | 1007 |
| 22640 | 0.06 s | 24828 | **1123** | 18176 | 1174 |

Messages retenus reçus par un NOUVEL abonné `moli/#` (attendus : tous les retenus en place) :

| retenus en place | abonnement QoS 0 | abonnement QoS 1 |
|---|---|---|
| 2264 | **2264** | **100** |
| 6792 | **6792** | **100** |
| 22640 | **22640** | **100** |
RESULTAT {"au_repos":{"RssAnon":180,"RssFile":2440,"Threads":4,"VmData":15768,"VmHWM":2620,"VmRSS":2620,"VmSize":19664,"cg_anon":180,"cg_current":772,"cg_file":0,"cg_kernel":396,"cg_max":131072,"cg_peak":3880,"cg_slab":245,"cg_sock":0,"cpu_ms":10,"fds":9,"nofile":"65536/65536","pid":1},"avec_abonne":{"RssAnon":180,"RssFile":2440,"Threads":4,"VmData":15768,"VmHWM":2620,"VmRSS":2620,"VmSize":19664,"cg_anon":180,"cg_current":780,"cg_file":0,"cg_kernel":396,"cg_max":131072,"cg_peak":3880,"cg_slab":245,"cg_sock":0,"cpu_ms":10,"fds":9,"nofile":"65536/65536","pid":1},"catchall":false,"ferme":{"RssAnon":23632,"RssFile":2824,"Threads":4,"VmData":42356,"VmHWM":46208,"VmRSS":26456,"VmSize":46252,"cg_anon":23632,"cg_current":24336,"cg_file":0,"cg_kernel":468,"cg_max":131072,"cg_peak":44184,"cg_slab":256,"cg_sock":0,"cpu_ms":190,"fds":10,"nofile":"65536/65536","pid":1},"niveaux":[{"mesure":{"RssAnon":2380,"RssFile":2824,"Threads":4,"VmData":18048,"VmHWM":5388,"VmRSS":5204,"VmSize":21944,"cg_anon":2380,"cg_current":3004,"cg_file":0,"cg_kernel":404,"cg_max":131072,"cg_peak":3880,"cg_slab":248,"cg_sock":0,"cpu_ms":30,"fds":10,"nofile":"65536/65536","pid":1},"n":2264,"pose_s":0.033094317,"recus_q0":2264,"recus_q1":100},{"mesure":{"RssAnon":6832,"RssFile":2824,"Threads":4,"VmData":23356,"VmHWM":9656,"VmRSS":9656,"VmSize":27252,"cg_anon":6832,"cg_current":7492,"cg_file":0,"cg_kernel":424,"cg_max":131072,"cg_peak":7612,"cg_slab":255,"cg_sock":0,"cpu_ms":50,"fds":10,"nofile":"65536/65536","pid":1},"n":6792,"pose_s":0.040261116,"recus_q0":6792,"recus_q1":100},{"mesure":{"RssAnon":25008,"RssFile":2824,"Threads":4,"VmData":43656,"VmHWM":27832,"VmRSS":27832,"VmSize":47552,"cg_anon":25008,"cg_current":25712,"cg_file":0,"cg_kernel":468,"cg_max":131072,"cg_peak":25908,"cg_slab":254,"cg_sock":0,"cpu_ms":120,"fds":10,"nofile":"65536/65536","pid":1},"n":22640,"pose_s":0.059861864,"recus_q0":22640,"recus_q1":100}],"scenario":"memoire"}
- courtier : OOMKilled=false, état=running, journal : courtier rumqttd 0.20.0 sur :1883 (segments 104857600 o × 10, dynamic_filters=true, max_outgoing_packet_count=30000), contrôle :8801, nofile Some(String("65536/65536"))

## 64 abonnés QoS 1, 0 retenus

### courtier rumqttd 0.20.0 · 64 abonnés TCP à `moli/#` (QoS 1) · 0 messages retenus en place · diffusion 10/s × 10 s (QoS 0, 120 o)
- abonnés établis : 64/64 en 0.15 s ; reconnexions (connexions − abonnés) : 0 ; `ulimit -n` du courtier : 65536/65536
- messages retenus reçus au total à l'abonnement : 0 sur 0 attendus si tout était livré (64 × 0), soit 0.0 par abonné ; stabilisés après 3.20 s

| phase (mesurée par le courtier sur lui-même, Kio) | RssAnon | VmRSS | VmHWM (pic) | fils | descripteurs | cgroup `slab` | cgroup `memory.current` |
|---|---|---|---|---|---|---|---|
| au repos | 176 | 2616 | 2616 | 4 | 9 | 245 | 772 |
| avec 64 abonnés | 4296 | 7056 | 7056 | 4 | 74 | 345 | 5052 |
| après la diffusion | 4640 | 7400 | 7400 | 4 | 74 | 408 | 5660 |
| après fermeture des clients | 988 | 3748 | 7296 | 4 | 10 | 314 | 1724 |

Par abonné (64 abonnés − avant) / 64 : RssAnon **64.19 Kio**.

CPU du courtier (utime+stime) :
- à l'abonnement de 64 clients (retenus compris) : 30 ms
- diffusion de 100 messages à 64 abonnés : **250 ms** ; 6400/6400 livraisons reçues en 9.96 s ; 39.1 µs de CPU courtier par livraison
RESULTAT {"apres_abonnes":{"RssAnon":4296,"RssFile":2760,"Threads":4,"VmData":34476,"VmHWM":7056,"VmRSS":7056,"VmSize":38372,"cg_anon":4296,"cg_current":5052,"cg_file":0,"cg_kernel":536,"cg_max":131072,"cg_peak":5600,"cg_slab":345,"cg_sock":0,"cpu_ms":30,"fds":74,"nofile":"65536/65536","pid":1},"attendus":6400,"au_repos":{"RssAnon":176,"RssFile":2440,"Threads":4,"VmData":15768,"VmHWM":2616,"VmRSS":2616,"VmSize":19664,"cg_anon":176,"cg_current":772,"cg_file":0,"cg_kernel":396,"cg_max":131072,"cg_peak":3852,"cg_slab":245,"cg_sock":0,"cpu_ms":0,"fds":9,"nofile":"65536/65536","pid":1},"avec_retenus":{"RssAnon":188,"RssFile":2500,"Threads":4,"VmData":15772,"VmHWM":2688,"VmRSS":2688,"VmSize":19668,"cg_anon":176,"cg_current":1168,"cg_file":0,"cg_kernel":396,"cg_max":131072,"cg_peak":3852,"cg_slab":245,"cg_sock":0,"cpu_ms":0,"fds":10,"nofile":"65536/65536","pid":1},"cpu_abonnement_ms":30,"cpu_diffusion_ms":250,"diffusion":{"RssAnon":4640,"RssFile":2760,"Threads":4,"VmData":34508,"VmHWM":7400,"VmRSS":7400,"VmSize":38404,"cg_anon":4644,"cg_current":5660,"cg_file":0,"cg_kernel":604,"cg_max":131072,"cg_peak":6100,"cg_slab":408,"cg_sock":0,"cpu_ms":290,"fds":74,"nofile":"65536/65536","pid":1},"ferme":{"RssAnon":988,"RssFile":2760,"Threads":4,"VmData":18240,"VmHWM":7296,"VmRSS":3748,"VmSize":22136,"cg_anon":988,"cg_current":1724,"cg_file":0,"cg_kernel":500,"cg_max":131072,"cg_peak":6100,"cg_slab":314,"cg_sock":0,"cpu_ms":300,"fds":10,"nofile":"65536/65536","pid":1},"livres":6400,"n":64,"qos":1,"retenus":0,"retenus_recus":0,"scenario":"abonnes"}
- courtier : OOMKilled=false, état=running, journal : courtier rumqttd 0.20.0 sur :1883 (segments 104857600 o × 10, dynamic_filters=true, max_outgoing_packet_count=200), contrôle :8801, nofile Some(String("65536/65536"))

## 64 abonnés QoS 1, 2264 retenus

### courtier rumqttd 0.20.0 · 64 abonnés TCP à `moli/#` (QoS 1) · 2264 messages retenus en place · diffusion 10/s × 10 s (QoS 0, 120 o)
- abonnés établis : 64/64 en 0.21 s ; reconnexions (connexions − abonnés) : 0 ; `ulimit -n` du courtier : 65536/65536
- messages retenus reçus au total à l'abonnement : 6400 sur 144896 attendus si tout était livré (64 × 2264), soit 100.0 par abonné ; stabilisés après 3.47 s

| phase (mesurée par le courtier sur lui-même, Kio) | RssAnon | VmRSS | VmHWM (pic) | fils | descripteurs | cgroup `slab` | cgroup `memory.current` |
|---|---|---|---|---|---|---|---|
| au repos | 184 | 2624 | 2624 | 4 | 9 | 245 | 720 |
| avec 2264 retenus | 2332 | 5156 | 5212 | 4 | 10 | 248 | 2964 |
| avec 64 abonnés | 12884 | 15708 | 15708 | 4 | 74 | 349 | 13664 |
| après la diffusion | 13108 | 15932 | 15932 | 4 | 74 | 403 | 14172 |
| après fermeture des clients | 3072 | 5896 | 15740 | 4 | 10 | 314 | 3832 |

Par abonné (64 abonnés − avant) / 64 : RssAnon **164.88 Kio**.

CPU du courtier (utime+stime) :
- à l'abonnement de 64 clients (retenus compris) : 50 ms
- diffusion de 100 messages à 64 abonnés : **260 ms** ; 6400/6400 livraisons reçues en 9.96 s ; 40.6 µs de CPU courtier par livraison
RESULTAT {"apres_abonnes":{"RssAnon":12884,"RssFile":2824,"Threads":4,"VmData":37108,"VmHWM":15708,"VmRSS":15708,"VmSize":41004,"cg_anon":12884,"cg_current":13664,"cg_file":0,"cg_kernel":544,"cg_max":131072,"cg_peak":14204,"cg_slab":349,"cg_sock":0,"cpu_ms":60,"fds":74,"nofile":"65536/65536","pid":1},"attendus":6400,"au_repos":{"RssAnon":184,"RssFile":2440,"Threads":4,"VmData":15772,"VmHWM":2624,"VmRSS":2624,"VmSize":19668,"cg_anon":184,"cg_current":720,"cg_file":0,"cg_kernel":396,"cg_max":131072,"cg_peak":3776,"cg_slab":245,"cg_sock":0,"cpu_ms":0,"fds":9,"nofile":"65536/65536","pid":1},"avec_retenus":{"RssAnon":2332,"RssFile":2824,"Threads":4,"VmData":18048,"VmHWM":5212,"VmRSS":5156,"VmSize":21944,"cg_anon":2332,"cg_current":2964,"cg_file":0,"cg_kernel":412,"cg_max":131072,"cg_peak":3776,"cg_slab":248,"cg_sock":0,"cpu_ms":10,"fds":10,"nofile":"65536/65536","pid":1},"cpu_abonnement_ms":50,"cpu_diffusion_ms":260,"diffusion":{"RssAnon":13108,"RssFile":2824,"Threads":4,"VmData":37132,"VmHWM":15932,"VmRSS":15932,"VmSize":41028,"cg_anon":13108,"cg_current":14172,"cg_file":0,"cg_kernel":604,"cg_max":131072,"cg_peak":14504,"cg_slab":403,"cg_sock":0,"cpu_ms":330,"fds":74,"nofile":"65536/65536","pid":1},"ferme":{"RssAnon":3072,"RssFile":2824,"Threads":4,"VmData":19172,"VmHWM":15740,"VmRSS":5896,"VmSize":23068,"cg_anon":3072,"cg_current":3832,"cg_file":0,"cg_kernel":508,"cg_max":131072,"cg_peak":14716,"cg_slab":314,"cg_sock":0,"cpu_ms":350,"fds":10,"nofile":"65536/65536","pid":1},"livres":6400,"n":64,"qos":1,"retenus":2264,"retenus_recus":6400,"scenario":"abonnes"}
- courtier : OOMKilled=false, état=running, journal : courtier rumqttd 0.20.0 sur :1883 (segments 104857600 o × 10, dynamic_filters=true, max_outgoing_packet_count=200), contrôle :8801, nofile Some(String("65536/65536"))

## 1000 abonnés QoS 1, 0 retenu (relevé des paniques)

### courtier rumqttd 0.20.0 · 1000 abonnés TCP à `moli/#` (QoS 1) · 0 messages retenus en place · diffusion 10/s × 10 s (QoS 0, 120 o)
- abonnés établis : 1000/1000 en 3.12 s ; reconnexions (connexions − abonnés) : 0 ; `ulimit -n` du courtier : 65536/65536
- messages retenus reçus au total à l'abonnement : 0 sur 0 attendus si tout était livré (1000 × 0), soit 0.0 par abonné ; stabilisés après 6.17 s

| phase (mesurée par le courtier sur lui-même, Kio) | RssAnon | VmRSS | VmHWM (pic) | fils | descripteurs | cgroup `slab` | cgroup `memory.current` |
|---|---|---|---|---|---|---|---|
| au repos | 180 | 2556 | 2556 | 4 | 9 | 245 | 776 |
| avec 1000 abonnés | 62496 | 65336 | 65336 | 4 | 1010 | 2659 | 65948 |
| après la diffusion | 63788 | 66628 | 66628 | 4 | 1010 | 2661 | 67376 |
| après fermeture des clients | 3644 | 6484 | 66560 | 4 | 10 | 1172 | 5388 |

Par abonné (1000 abonnés − avant) / 1000 : RssAnon **62.32 Kio**.

CPU du courtier (utime+stime) :
- à l'abonnement de 1000 clients (retenus compris) : 520 ms
- diffusion de 100 messages à 1000 abonnés : **2430 ms** ; 100000/100000 livraisons reçues en 9.96 s ; 24.3 µs de CPU courtier par livraison
RESULTAT {"apres_abonnes":{"RssAnon":62496,"RssFile":2840,"Threads":4,"VmData":283988,"VmHWM":65336,"VmRSS":65336,"VmSize":287944,"cg_anon":62496,"cg_current":65948,"cg_file":0,"cg_kernel":3340,"cg_max":131072,"cg_peak":65956,"cg_slab":2659,"cg_sock":0,"cpu_ms":540,"fds":1010,"nofile":"65536/65536","pid":1},"attendus":100000,"au_repos":{"RssAnon":180,"RssFile":2376,"Threads":4,"VmData":15772,"VmHWM":2556,"VmRSS":2556,"VmSize":19728,"cg_anon":180,"cg_current":776,"cg_file":0,"cg_kernel":396,"cg_max":131072,"cg_peak":3944,"cg_slab":245,"cg_sock":0,"cpu_ms":20,"fds":9,"nofile":"65536/65536","pid":1},"avec_retenus":{"RssAnon":180,"RssFile":2376,"Threads":4,"VmData":15776,"VmHWM":2556,"VmRSS":2556,"VmSize":19732,"cg_anon":180,"cg_current":1192,"cg_file":0,"cg_kernel":396,"cg_max":131072,"cg_peak":3944,"cg_slab":245,"cg_sock":0,"cpu_ms":20,"fds":10,"nofile":"65536/65536","pid":1},"cpu_abonnement_ms":520,"cpu_diffusion_ms":2430,"diffusion":{"RssAnon":63788,"RssFile":2840,"Threads":4,"VmData":284408,"VmHWM":66628,"VmRSS":66628,"VmSize":288364,"cg_anon":63788,"cg_current":67376,"cg_file":0,"cg_kernel":3340,"cg_max":131072,"cg_peak":68268,"cg_slab":2661,"cg_sock":0,"cpu_ms":3000,"fds":1010,"nofile":"65536/65536","pid":1},"ferme":{"RssAnon":3644,"RssFile":2840,"Threads":4,"VmData":19388,"VmHWM":66560,"VmRSS":6484,"VmSize":23344,"cg_anon":3644,"cg_current":5388,"cg_file":0,"cg_kernel":1492,"cg_max":131072,"cg_peak":68268,"cg_slab":1172,"cg_sock":0,"cpu_ms":3110,"fds":10,"nofile":"65536/65536","pid":1},"livres":100000,"n":1000,"qos":1,"retenus":0,"retenus_recus":0,"scenario":"abonnes"}
- courtier : OOMKilled=false, état=running, lignes de panique dans son journal : 0, journal : courtier rumqttd 0.20.0 sur :1883 (segments 104857600 o × 10, dynamic_filters=true, max_outgoing_packet_count=200), contrôle :8801, nofile Some(String("65536/65536"))

## 1000 abonnés QoS 1, 2264 retenus (relevé des paniques)

### courtier rumqttd 0.20.0 · 1000 abonnés TCP à `moli/#` (QoS 1) · 2264 messages retenus en place · diffusion 10/s × 10 s (QoS 0, 120 o)
- abonnés établis : 741/1000 en 120.03 s ; reconnexions (connexions − abonnés) : -258 ; `ulimit -n` du courtier : 65536/65536
- messages retenus reçus au total à l'abonnement : 74100 sur 2264000 attendus si tout était livré (1000 × 2264), soit 74.1 par abonné ; stabilisés après 120.03 s

| phase (mesurée par le courtier sur lui-même, Kio) | RssAnon | VmRSS | VmHWM (pic) | fils | descripteurs | cgroup `slab` | cgroup `memory.current` |
|---|---|---|---|---|---|---|---|
| au repos | 180 | 2556 | 2556 | 4 | 9 | 250 | 776 |
| avec 2264 retenus | 2344 | 5120 | 5324 | 4 | 10 | 253 | 2968 |
| avec 1000 abonnés | — | — | — | — | — | — | — |
| après la diffusion | — | — | — | — | — | — | — |
| après fermeture des clients | — | — | — | — | — | — | — |

Par abonné (1000 abonnés − avant) / 1000 : RssAnon **— Kio**.

CPU du courtier (utime+stime) :
- à l'abonnement de 1000 clients (retenus compris) : — ms
- diffusion de 100 messages à 1000 abonnés : **— ms** ; 0/100000 livraisons reçues en 39.92 s ; 0.0 µs de CPU courtier par livraison
RESULTAT {"apres_abonnes":{},"attendus":100000,"au_repos":{"RssAnon":180,"RssFile":2376,"Threads":4,"VmData":15772,"VmHWM":2556,"VmRSS":2556,"VmSize":19728,"cg_anon":180,"cg_current":776,"cg_file":0,"cg_kernel":396,"cg_max":131072,"cg_peak":3652,"cg_slab":250,"cg_sock":0,"cpu_ms":10,"fds":9,"nofile":"65536/65536","pid":1},"avec_retenus":{"RssAnon":2344,"RssFile":2776,"Threads":4,"VmData":18052,"VmHWM":5324,"VmRSS":5120,"VmSize":22008,"cg_anon":2344,"cg_current":2968,"cg_file":0,"cg_kernel":404,"cg_max":131072,"cg_peak":3652,"cg_slab":253,"cg_sock":0,"cpu_ms":10,"fds":10,"nofile":"65536/65536","pid":1},"cpu_abonnement_ms":null,"cpu_diffusion_ms":null,"diffusion":{},"ferme":{},"livres":0,"n":1000,"qos":1,"retenus":2264,"retenus_recus":74100,"scenario":"abonnes"}
- courtier : OOMKilled=true, état=exited, lignes de panique dans son journal : 0, journal : courtier rumqttd 0.20.0 sur :1883 (segments 104857600 o × 10, dynamic_filters=true, max_outgoing_packet_count=200), contrôle :8801, nofile Some(String("65536/65536"))

## 1000 abonnés QoS 0, 0 retenu (relevé des paniques)

### courtier rumqttd 0.20.0 · 1000 abonnés TCP à `moli/#` (QoS 0) · 0 messages retenus en place · diffusion 10/s × 10 s (QoS 0, 120 o)
- abonnés établis : 1000/1000 en 6.35 s ; reconnexions (connexions − abonnés) : 0 ; `ulimit -n` du courtier : 65536/65536
- messages retenus reçus au total à l'abonnement : 0 sur 0 attendus si tout était livré (1000 × 0), soit 0.0 par abonné ; stabilisés après 9.42 s

| phase (mesurée par le courtier sur lui-même, Kio) | RssAnon | VmRSS | VmHWM (pic) | fils | descripteurs | cgroup `slab` | cgroup `memory.current` |
|---|---|---|---|---|---|---|---|
| au repos | 180 | 2556 | 2556 | 4 | 9 | 246 | 776 |
| avec 1000 abonnés | 62448 | 65288 | 65288 | 4 | 1010 | 2661 | 65884 |
| après la diffusion | 63144 | 65984 | 65984 | 4 | 1010 | 2662 | 66668 |
| après fermeture des clients | 3320 | 6160 | 65664 | 4 | 10 | 1175 | 5040 |

Par abonné (1000 abonnés − avant) / 1000 : RssAnon **62.25 Kio**.

CPU du courtier (utime+stime) :
- à l'abonnement de 1000 clients (retenus compris) : 400 ms
- diffusion de 100 messages à 1000 abonnés : **1520 ms** ; 100000/100000 livraisons reçues en 9.96 s ; 15.2 µs de CPU courtier par livraison
RESULTAT {"apres_abonnes":{"RssAnon":62448,"RssFile":2840,"Threads":4,"VmData":283976,"VmHWM":65288,"VmRSS":65288,"VmSize":287932,"cg_anon":62448,"cg_current":65884,"cg_file":0,"cg_kernel":3340,"cg_max":131072,"cg_peak":65900,"cg_slab":2661,"cg_sock":0,"cpu_ms":410,"fds":1010,"nofile":"65536/65536","pid":1},"attendus":100000,"au_repos":{"RssAnon":180,"RssFile":2376,"Threads":4,"VmData":15772,"VmHWM":2556,"VmRSS":2556,"VmSize":19728,"cg_anon":180,"cg_current":776,"cg_file":0,"cg_kernel":400,"cg_max":131072,"cg_peak":3480,"cg_slab":246,"cg_sock":0,"cpu_ms":10,"fds":9,"nofile":"65536/65536","pid":1},"avec_retenus":{"RssAnon":196,"RssFile":2440,"Threads":4,"VmData":15776,"VmHWM":2636,"VmRSS":2636,"VmSize":19732,"cg_anon":180,"cg_current":1080,"cg_file":0,"cg_kernel":400,"cg_max":131072,"cg_peak":3480,"cg_slab":246,"cg_sock":0,"cpu_ms":10,"fds":10,"nofile":"65536/65536","pid":1},"cpu_abonnement_ms":400,"cpu_diffusion_ms":1520,"diffusion":{"RssAnon":63144,"RssFile":2840,"Threads":4,"VmData":284004,"VmHWM":65984,"VmRSS":65984,"VmSize":287960,"cg_anon":63144,"cg_current":66668,"cg_file":0,"cg_kernel":3348,"cg_max":131072,"cg_peak":68644,"cg_slab":2662,"cg_sock":0,"cpu_ms":1960,"fds":1010,"nofile":"65536/65536","pid":1},"ferme":{"RssAnon":3320,"RssFile":2840,"Threads":4,"VmData":19072,"VmHWM":65664,"VmRSS":6160,"VmSize":23028,"cg_anon":3320,"cg_current":5040,"cg_file":0,"cg_kernel":1484,"cg_max":131072,"cg_peak":68644,"cg_slab":1175,"cg_sock":0,"cpu_ms":2060,"fds":10,"nofile":"65536/65536","pid":1},"livres":100000,"n":1000,"qos":0,"retenus":0,"retenus_recus":0,"scenario":"abonnes"}
- courtier : OOMKilled=false, état=running, lignes de panique dans son journal : 0, journal : courtier rumqttd 0.20.0 sur :1883 (segments 104857600 o × 10, dynamic_filters=true, max_outgoing_packet_count=200), contrôle :8801, nofile Some(String("65536/65536"))

## Rafale de 5000, QoS 0, abonné aucun, configuration d'exemple

### rafale de 5000 publications QoS 0 · abonné « aucun » · observation 10 s
- deux abonnés rapides + aucun abonné lent (témoin) ; tous abonnés à `moli/#` en QoS 0 ; 100 topics, charge de 120 o ; `ulimit -n` du courtier : 65536/65536
- éditeur : 5000 appels `publish()` rendus en 0.02 s ; tout écrit sur la socket en 0.03 s ; débit éditeur 197205 messages/s
- abonné rapide 1 : tout reçu en 0.07 s — 5000/5000 reçus, 0 manquants, 0 doublons, ordre conservé ; abonné rapide 2 : tout reçu en 0.07 s — 5000/5000 reçus, 0 manquants, 0 doublons, ordre conservé

| phase (mesurée par le courtier sur lui-même, Kio) | RssAnon | VmRSS | VmHWM (pic) | fils | descripteurs | cgroup `slab` | cgroup `memory.current` |
|---|---|---|---|---|---|---|---|
| au repos (3 abonnés connectés) | 380 | 3140 | 3140 | 4 | 11 | 253 | 1024 |
| juste après la rafale | 3264 | 6088 | 6144 | 4 | 12 | 260 | 3720 |
| 10 s plus tard (abonné aucun toujours dans son état) | 3264 | 6088 | 6144 | 4 | 12 | 256 | 3912 |
| après la reprise | 3264 | 6088 | 6144 | 4 | 12 | 256 | 3908 |
| après fermeture des clients | 2716 | 5540 | 6144 | 4 | 10 | 254 | 3360 |

CPU du courtier pendant la rafale : 30 ms ; mémoire (RssAnon) due à la rafale : 2884 Kio ; 10 s plus tard : 2884 Kio ; après reprise : 2884 Kio.
RESULTAT {"apres_observation":{"RssAnon":3264,"RssFile":2824,"Threads":4,"VmData":20116,"VmHWM":6144,"VmRSS":6088,"VmSize":24012,"cg_anon":3264,"cg_current":3912,"cg_file":0,"cg_kernel":428,"cg_max":131072,"cg_peak":4260,"cg_slab":256,"cg_sock":0,"cpu_ms":30,"fds":12,"nofile":"65536/65536","pid":1},"apres_rafale":{"RssAnon":3264,"RssFile":2824,"Threads":4,"VmData":20116,"VmHWM":6144,"VmRSS":6088,"VmSize":24012,"cg_anon":3264,"cg_current":3720,"cg_file":0,"cg_kernel":440,"cg_max":131072,"cg_peak":4260,"cg_slab":260,"cg_sock":0,"cpu_ms":30,"fds":12,"nofile":"65536/65536","pid":1},"apres_reprise":{"RssAnon":3264,"RssFile":2824,"Threads":4,"VmData":20116,"VmHWM":6144,"VmRSS":6088,"VmSize":24012,"cg_anon":3264,"cg_current":3908,"cg_file":0,"cg_kernel":424,"cg_max":131072,"cg_peak":4260,"cg_slab":256,"cg_sock":0,"cpu_ms":30,"fds":12,"nofile":"65536/65536","pid":1},"avant":{"RssAnon":380,"RssFile":2760,"Threads":4,"VmData":16572,"VmHWM":3140,"VmRSS":3140,"VmSize":20468,"cg_anon":380,"cg_current":1024,"cg_file":0,"cg_kernel":420,"cg_max":131072,"cg_peak":3840,"cg_slab":253,"cg_sock":0,"cpu_ms":0,"fds":11,"nofile":"65536/65536","pid":1},"lent":"0/5000 reçus, 5000 manquants (séquences 0 à 4999), 0 doublons, ordre conservé","mode":"aucun","n":5000,"qos":0,"rapide1":"5000/5000 reçus, 0 manquants, 0 doublons, ordre conservé","rapide2":"5000/5000 reçus, 0 manquants, 0 doublons, ordre conservé","scenario":"rafale"}
- courtier : OOMKilled=false, état=running, journal : courtier rumqttd 0.20.0 sur :1883 (segments 104857600 o × 10, dynamic_filters=true, max_outgoing_packet_count=200), contrôle :8801, nofile Some(String("65536/65536"))

## Rafale de 5000, QoS 0, abonné bloque, configuration d'exemple

### rafale de 5000 publications QoS 0 · abonné « bloque » · observation 10 s
- deux abonnés rapides + un abonné qui cesse de lire sa socket (tampon de réception TCP de 4 Kio, aucun accusé) ; tous abonnés à `moli/#` en QoS 0 ; 100 topics, charge de 120 o ; `ulimit -n` du courtier : 65536/65536
- éditeur : 5000 appels `publish()` rendus en 0.02 s ; tout écrit sur la socket en 0.02 s ; débit éditeur 216993 messages/s
- abonné rapide 1 : tout reçu en 0.07 s — 5000/5000 reçus, 0 manquants, 0 doublons, ordre conservé ; abonné rapide 2 : tout reçu en 0.07 s — 5000/5000 reçus, 0 manquants, 0 doublons, ordre conservé
- abonné bloque : 0 messages reçus pendant la fenêtre de 10 s ; après reprise à pleine vitesse : 5000/5000 reçus, 0 manquants, 0 doublons, ordre conservé ; reconnexions : 0

| phase (mesurée par le courtier sur lui-même, Kio) | RssAnon | VmRSS | VmHWM (pic) | fils | descripteurs | cgroup `slab` | cgroup `memory.current` |
|---|---|---|---|---|---|---|---|
| au repos (3 abonnés connectés) | 440 | 3200 | 3200 | 4 | 12 | 254 | 1084 |
| juste après la rafale | 3648 | 6408 | 6408 | 4 | 13 | 269 | 5432 |
| 10 s plus tard (abonné bloque toujours dans son état) | 3648 | 6408 | 6408 | 4 | 13 | 260 | 5464 |
| après la reprise | 3648 | 6408 | 6408 | 4 | 13 | 260 | 4308 |
| après fermeture des clients | 2764 | 5524 | 6400 | 4 | 10 | 258 | 3424 |

CPU du courtier pendant la rafale : 30 ms ; mémoire (RssAnon) due à la rafale : 3208 Kio ; 10 s plus tard : 3208 Kio ; après reprise : 3208 Kio.
RESULTAT {"apres_observation":{"RssAnon":3648,"RssFile":2760,"Threads":4,"VmData":20896,"VmHWM":6408,"VmRSS":6408,"VmSize":24792,"cg_anon":3648,"cg_current":5464,"cg_file":0,"cg_kernel":428,"cg_max":131072,"cg_peak":5716,"cg_slab":260,"cg_sock":1152,"cpu_ms":30,"fds":13,"nofile":"65536/65536","pid":1},"apres_rafale":{"RssAnon":3648,"RssFile":2760,"Threads":4,"VmData":20896,"VmHWM":6408,"VmRSS":6408,"VmSize":24792,"cg_anon":3648,"cg_current":5432,"cg_file":0,"cg_kernel":436,"cg_max":131072,"cg_peak":5716,"cg_slab":269,"cg_sock":1152,"cpu_ms":30,"fds":13,"nofile":"65536/65536","pid":1},"apres_reprise":{"RssAnon":3648,"RssFile":2760,"Threads":4,"VmData":20896,"VmHWM":6408,"VmRSS":6408,"VmSize":24792,"cg_anon":3648,"cg_current":4308,"cg_file":0,"cg_kernel":424,"cg_max":131072,"cg_peak":5716,"cg_slab":260,"cg_sock":0,"cpu_ms":30,"fds":13,"nofile":"65536/65536","pid":1},"avant":{"RssAnon":440,"RssFile":2760,"Threads":4,"VmData":16840,"VmHWM":3200,"VmRSS":3200,"VmSize":20736,"cg_anon":440,"cg_current":1084,"cg_file":0,"cg_kernel":420,"cg_max":131072,"cg_peak":3668,"cg_slab":254,"cg_sock":0,"cpu_ms":0,"fds":12,"nofile":"65536/65536","pid":1},"lent":"5000/5000 reçus, 0 manquants, 0 doublons, ordre conservé","mode":"bloque","n":5000,"qos":0,"rapide1":"5000/5000 reçus, 0 manquants, 0 doublons, ordre conservé","rapide2":"5000/5000 reçus, 0 manquants, 0 doublons, ordre conservé","scenario":"rafale"}
- courtier : OOMKilled=false, état=running, journal : courtier rumqttd 0.20.0 sur :1883 (segments 104857600 o × 10, dynamic_filters=true, max_outgoing_packet_count=200), contrôle :8801, nofile Some(String("65536/65536"))

## Rafale de 5000, QoS 0, abonné lent, configuration d'exemple

### rafale de 5000 publications QoS 0 · abonné « lent » · observation 10 s
- deux abonnés rapides + un abonné lent (il dort 20 ms après chaque événement de son client MQTT, soit 25 à 50 messages/s ; tampon de réception TCP de 4 Kio) ; tous abonnés à `moli/#` en QoS 0 ; 100 topics, charge de 120 o ; `ulimit -n` du courtier : 65536/65536
- éditeur : 5000 appels `publish()` rendus en 0.02 s ; tout écrit sur la socket en 0.03 s ; débit éditeur 167696 messages/s
- abonné rapide 1 : tout reçu en 0.07 s — 5000/5000 reçus, 0 manquants, 0 doublons, ordre conservé ; abonné rapide 2 : tout reçu en 0.07 s — 5000/5000 reçus, 0 manquants, 0 doublons, ordre conservé
- abonné lent : 426 messages reçus pendant la fenêtre de 10 s ; après reprise à pleine vitesse : 5000/5000 reçus, 0 manquants, 0 doublons, ordre conservé ; reconnexions : 0

| phase (mesurée par le courtier sur lui-même, Kio) | RssAnon | VmRSS | VmHWM (pic) | fils | descripteurs | cgroup `slab` | cgroup `memory.current` |
|---|---|---|---|---|---|---|---|
| au repos (3 abonnés connectés) | 440 | 3200 | 3200 | 4 | 12 | 255 | 1072 |
| juste après la rafale | 3824 | 6584 | 6656 | 4 | 13 | 265 | 5448 |
| 10 s plus tard (abonné lent toujours dans son état) | 3824 | 6584 | 6656 | 4 | 13 | 262 | 5548 |
| après la reprise | 3824 | 6584 | 6656 | 4 | 13 | 262 | 4488 |
| après fermeture des clients | 2964 | 5724 | 6656 | 4 | 10 | 260 | 3624 |

CPU du courtier pendant la rafale : 30 ms ; mémoire (RssAnon) due à la rafale : 3384 Kio ; 10 s plus tard : 3384 Kio ; après reprise : 3384 Kio.
RESULTAT {"apres_observation":{"RssAnon":3824,"RssFile":2760,"Threads":4,"VmData":21092,"VmHWM":6656,"VmRSS":6584,"VmSize":24988,"cg_anon":3824,"cg_current":5548,"cg_file":0,"cg_kernel":432,"cg_max":131072,"cg_peak":5896,"cg_slab":262,"cg_sock":1068,"cpu_ms":40,"fds":13,"nofile":"65536/65536","pid":1},"apres_rafale":{"RssAnon":3824,"RssFile":2760,"Threads":4,"VmData":21092,"VmHWM":6656,"VmRSS":6584,"VmSize":24988,"cg_anon":3824,"cg_current":5448,"cg_file":0,"cg_kernel":440,"cg_max":131072,"cg_peak":5896,"cg_slab":265,"cg_sock":1144,"cpu_ms":40,"fds":13,"nofile":"65536/65536","pid":1},"apres_reprise":{"RssAnon":3824,"RssFile":2760,"Threads":4,"VmData":21092,"VmHWM":6656,"VmRSS":6584,"VmSize":24988,"cg_anon":3824,"cg_current":4488,"cg_file":0,"cg_kernel":428,"cg_max":131072,"cg_peak":5896,"cg_slab":262,"cg_sock":0,"cpu_ms":40,"fds":13,"nofile":"65536/65536","pid":1},"avant":{"RssAnon":440,"RssFile":2760,"Threads":4,"VmData":16840,"VmHWM":3200,"VmRSS":3200,"VmSize":20736,"cg_anon":440,"cg_current":1072,"cg_file":0,"cg_kernel":416,"cg_max":131072,"cg_peak":3876,"cg_slab":255,"cg_sock":0,"cpu_ms":10,"fds":12,"nofile":"65536/65536","pid":1},"lent":"5000/5000 reçus, 0 manquants, 0 doublons, ordre conservé","mode":"lent","n":5000,"qos":0,"rapide1":"5000/5000 reçus, 0 manquants, 0 doublons, ordre conservé","rapide2":"5000/5000 reçus, 0 manquants, 0 doublons, ordre conservé","scenario":"rafale"}
- courtier : OOMKilled=false, état=running, journal : courtier rumqttd 0.20.0 sur :1883 (segments 104857600 o × 10, dynamic_filters=true, max_outgoing_packet_count=200), contrôle :8801, nofile Some(String("65536/65536"))

## Rafale de 5000, QoS 1, abonné aucun, configuration d'exemple

### rafale de 5000 publications QoS 1 · abonné « aucun » · observation 10 s
- deux abonnés rapides + aucun abonné lent (témoin) ; tous abonnés à `moli/#` en QoS 1 ; 100 topics, charge de 120 o ; `ulimit -n` du courtier : 65536/65536
- éditeur : 5000 appels `publish()` rendus en 0.03 s ; tous les PUBACK reçus en 0.04 s ; débit éditeur 130130 messages/s
- abonné rapide 1 : tout reçu en 0.04 s — 5000/5000 reçus, 0 manquants, 0 doublons, ordre conservé ; abonné rapide 2 : tout reçu en 0.04 s — 5000/5000 reçus, 0 manquants, 0 doublons, ordre conservé

| phase (mesurée par le courtier sur lui-même, Kio) | RssAnon | VmRSS | VmHWM (pic) | fils | descripteurs | cgroup `slab` | cgroup `memory.current` |
|---|---|---|---|---|---|---|---|
| au repos (3 abonnés connectés) | 384 | 3208 | 3208 | 4 | 11 | 254 | 1036 |
| juste après la rafale | 2976 | 5800 | 5800 | 4 | 12 | 264 | 3464 |
| 10 s plus tard (abonné aucun toujours dans son état) | 2976 | 5800 | 5800 | 4 | 12 | 255 | 3612 |
| après la reprise | 2976 | 5800 | 5800 | 4 | 12 | 255 | 3612 |
| après fermeture des clients | 2484 | 5308 | 5632 | 4 | 10 | 261 | 3120 |

CPU du courtier pendant la rafale : 50 ms ; mémoire (RssAnon) due à la rafale : 2592 Kio ; 10 s plus tard : 2592 Kio ; après reprise : 2592 Kio.
RESULTAT {"apres_observation":{"RssAnon":2976,"RssFile":2824,"Threads":4,"VmData":19592,"VmHWM":5800,"VmRSS":5800,"VmSize":23488,"cg_anon":2976,"cg_current":3612,"cg_file":0,"cg_kernel":416,"cg_max":131072,"cg_peak":4036,"cg_slab":255,"cg_sock":0,"cpu_ms":50,"fds":12,"nofile":"65536/65536","pid":1},"apres_rafale":{"RssAnon":2976,"RssFile":2824,"Threads":4,"VmData":19592,"VmHWM":5800,"VmRSS":5800,"VmSize":23488,"cg_anon":2976,"cg_current":3464,"cg_file":0,"cg_kernel":436,"cg_max":131072,"cg_peak":4036,"cg_slab":264,"cg_sock":4,"cpu_ms":50,"fds":12,"nofile":"65536/65536","pid":1},"apres_reprise":{"RssAnon":2976,"RssFile":2824,"Threads":4,"VmData":19592,"VmHWM":5800,"VmRSS":5800,"VmSize":23488,"cg_anon":2976,"cg_current":3612,"cg_file":0,"cg_kernel":416,"cg_max":131072,"cg_peak":4036,"cg_slab":255,"cg_sock":0,"cpu_ms":50,"fds":12,"nofile":"65536/65536","pid":1},"avant":{"RssAnon":384,"RssFile":2824,"Threads":4,"VmData":16576,"VmHWM":3208,"VmRSS":3208,"VmSize":20472,"cg_anon":384,"cg_current":1036,"cg_file":0,"cg_kernel":412,"cg_max":131072,"cg_peak":4036,"cg_slab":254,"cg_sock":0,"cpu_ms":0,"fds":11,"nofile":"65536/65536","pid":1},"lent":"0/5000 reçus, 5000 manquants (séquences 0 à 4999), 0 doublons, ordre conservé","mode":"aucun","n":5000,"qos":1,"rapide1":"5000/5000 reçus, 0 manquants, 0 doublons, ordre conservé","rapide2":"5000/5000 reçus, 0 manquants, 0 doublons, ordre conservé","scenario":"rafale"}
- courtier : OOMKilled=false, état=running, journal : courtier rumqttd 0.20.0 sur :1883 (segments 104857600 o × 10, dynamic_filters=true, max_outgoing_packet_count=200), contrôle :8801, nofile Some(String("65536/65536"))

## Rafale de 5000, QoS 1, abonné bloque, configuration d'exemple

### rafale de 5000 publications QoS 1 · abonné « bloque » · observation 10 s
- deux abonnés rapides + un abonné qui cesse de lire sa socket (tampon de réception TCP de 4 Kio, aucun accusé) ; tous abonnés à `moli/#` en QoS 1 ; 100 topics, charge de 120 o ; `ulimit -n` du courtier : 65536/65536
- éditeur : 5000 appels `publish()` rendus en 0.03 s ; tous les PUBACK reçus en 0.04 s ; débit éditeur 116895 messages/s
- abonné rapide 1 : tout reçu en 0.04 s — 5000/5000 reçus, 0 manquants, 0 doublons, ordre conservé ; abonné rapide 2 : tout reçu en 0.04 s — 5000/5000 reçus, 0 manquants, 0 doublons, ordre conservé
- abonné bloque : 0 messages reçus pendant la fenêtre de 10 s ; après reprise à pleine vitesse : 5000/5000 reçus, 0 manquants, 0 doublons, ordre conservé ; reconnexions : 0

| phase (mesurée par le courtier sur lui-même, Kio) | RssAnon | VmRSS | VmHWM (pic) | fils | descripteurs | cgroup `slab` | cgroup `memory.current` |
|---|---|---|---|---|---|---|---|
| au repos (3 abonnés connectés) | 440 | 3200 | 3200 | 4 | 12 | 255 | 1080 |
| juste après la rafale | 3048 | 5808 | 5808 | 4 | 13 | 263 | 3640 |
| 10 s plus tard (abonné bloque toujours dans son état) | 3048 | 5808 | 5808 | 4 | 13 | 257 | 3728 |
| après la reprise | 3112 | 5872 | 5888 | 4 | 13 | 257 | 3768 |
| après fermeture des clients | 2476 | 5236 | 5888 | 4 | 10 | 254 | 3132 |

CPU du courtier pendant la rafale : 50 ms ; mémoire (RssAnon) due à la rafale : 2608 Kio ; 10 s plus tard : 2608 Kio ; après reprise : 2672 Kio.
RESULTAT {"apres_observation":{"RssAnon":3048,"RssFile":2760,"Threads":4,"VmData":19840,"VmHWM":5808,"VmRSS":5808,"VmSize":23736,"cg_anon":3048,"cg_current":3728,"cg_file":0,"cg_kernel":424,"cg_max":131072,"cg_peak":3988,"cg_slab":257,"cg_sock":20,"cpu_ms":50,"fds":13,"nofile":"65536/65536","pid":1},"apres_rafale":{"RssAnon":3048,"RssFile":2760,"Threads":4,"VmData":19840,"VmHWM":5808,"VmRSS":5808,"VmSize":23736,"cg_anon":3048,"cg_current":3640,"cg_file":0,"cg_kernel":432,"cg_max":131072,"cg_peak":3988,"cg_slab":263,"cg_sock":24,"cpu_ms":50,"fds":13,"nofile":"65536/65536","pid":1},"apres_reprise":{"RssAnon":3112,"RssFile":2760,"Threads":4,"VmData":19836,"VmHWM":5888,"VmRSS":5872,"VmSize":23732,"cg_anon":3112,"cg_current":3768,"cg_file":0,"cg_kernel":420,"cg_max":131072,"cg_peak":4244,"cg_slab":257,"cg_sock":0,"cpu_ms":50,"fds":13,"nofile":"65536/65536","pid":1},"avant":{"RssAnon":440,"RssFile":2760,"Threads":4,"VmData":16840,"VmHWM":3200,"VmRSS":3200,"VmSize":20736,"cg_anon":440,"cg_current":1080,"cg_file":0,"cg_kernel":420,"cg_max":131072,"cg_peak":3696,"cg_slab":255,"cg_sock":0,"cpu_ms":0,"fds":12,"nofile":"65536/65536","pid":1},"lent":"5000/5000 reçus, 0 manquants, 0 doublons, ordre conservé","mode":"bloque","n":5000,"qos":1,"rapide1":"5000/5000 reçus, 0 manquants, 0 doublons, ordre conservé","rapide2":"5000/5000 reçus, 0 manquants, 0 doublons, ordre conservé","scenario":"rafale"}
- courtier : OOMKilled=false, état=running, journal : courtier rumqttd 0.20.0 sur :1883 (segments 104857600 o × 10, dynamic_filters=true, max_outgoing_packet_count=200), contrôle :8801, nofile Some(String("65536/65536"))

## Rafale de 5000, QoS 1, abonné lent, configuration d'exemple

### rafale de 5000 publications QoS 1 · abonné « lent » · observation 10 s
- deux abonnés rapides + un abonné lent (il dort 20 ms après chaque événement de son client MQTT, soit 25 à 50 messages/s ; tampon de réception TCP de 4 Kio) ; tous abonnés à `moli/#` en QoS 1 ; 100 topics, charge de 120 o ; `ulimit -n` du courtier : 65536/65536
- éditeur : 5000 appels `publish()` rendus en 0.03 s ; tous les PUBACK reçus en 0.04 s ; débit éditeur 128774 messages/s
- abonné rapide 1 : tout reçu en 0.04 s — 5000/5000 reçus, 0 manquants, 0 doublons, ordre conservé ; abonné rapide 2 : tout reçu en 0.04 s — 5000/5000 reçus, 0 manquants, 0 doublons, ordre conservé
- abonné lent : 212 messages reçus pendant la fenêtre de 10 s ; après reprise à pleine vitesse : 5000/5000 reçus, 0 manquants, 0 doublons, ordre conservé ; reconnexions : 0

| phase (mesurée par le courtier sur lui-même, Kio) | RssAnon | VmRSS | VmHWM (pic) | fils | descripteurs | cgroup `slab` | cgroup `memory.current` |
|---|---|---|---|---|---|---|---|
| au repos (3 abonnés connectés) | 440 | 3200 | 3200 | 4 | 12 | 250 | 1072 |
| juste après la rafale | 3092 | 5852 | 5852 | 4 | 13 | 290 | 4032 |
| 10 s plus tard (abonné lent toujours dans son état) | 3092 | 5852 | 5852 | 4 | 13 | 256 | 3756 |
| après la reprise | 3124 | 5948 | 5948 | 4 | 13 | 256 | 3768 |
| après fermeture des clients | 2500 | 5324 | 5888 | 4 | 10 | 253 | 3140 |

CPU du courtier pendant la rafale : 50 ms ; mémoire (RssAnon) due à la rafale : 2652 Kio ; 10 s plus tard : 2652 Kio ; après reprise : 2684 Kio.
RESULTAT {"apres_observation":{"RssAnon":3092,"RssFile":2760,"Threads":4,"VmData":19884,"VmHWM":5852,"VmRSS":5852,"VmSize":23780,"cg_anon":3092,"cg_current":3756,"cg_file":0,"cg_kernel":420,"cg_max":131072,"cg_peak":4276,"cg_slab":256,"cg_sock":12,"cpu_ms":50,"fds":13,"nofile":"65536/65536","pid":1},"apres_rafale":{"RssAnon":3092,"RssFile":2760,"Threads":4,"VmData":19884,"VmHWM":5852,"VmRSS":5852,"VmSize":23780,"cg_anon":3092,"cg_current":4032,"cg_file":0,"cg_kernel":460,"cg_max":131072,"cg_peak":4232,"cg_slab":290,"cg_sock":24,"cpu_ms":50,"fds":13,"nofile":"65536/65536","pid":1},"apres_reprise":{"RssAnon":3124,"RssFile":2824,"Threads":4,"VmData":19884,"VmHWM":5948,"VmRSS":5948,"VmSize":23780,"cg_anon":3124,"cg_current":3768,"cg_file":0,"cg_kernel":424,"cg_max":131072,"cg_peak":4276,"cg_slab":256,"cg_sock":0,"cpu_ms":70,"fds":13,"nofile":"65536/65536","pid":1},"avant":{"RssAnon":440,"RssFile":2760,"Threads":4,"VmData":16840,"VmHWM":3200,"VmRSS":3200,"VmSize":20736,"cg_anon":440,"cg_current":1072,"cg_file":0,"cg_kernel":408,"cg_max":131072,"cg_peak":3936,"cg_slab":250,"cg_sock":0,"cpu_ms":0,"fds":12,"nofile":"65536/65536","pid":1},"lent":"5000/5000 reçus, 0 manquants, 0 doublons, ordre conservé","mode":"lent","n":5000,"qos":1,"rapide1":"5000/5000 reçus, 0 manquants, 0 doublons, ordre conservé","rapide2":"5000/5000 reçus, 0 manquants, 0 doublons, ordre conservé","scenario":"rafale"}
- courtier : OOMKilled=false, état=running, journal : courtier rumqttd 0.20.0 sur :1883 (segments 104857600 o × 10, dynamic_filters=true, max_outgoing_packet_count=200), contrôle :8801, nofile Some(String("65536/65536"))

## Rafale de 5000, QoS 0, abonné bloque, segments 64 Kio × 2

### rafale de 5000 publications QoS 0 · abonné « bloque » · observation 10 s
- deux abonnés rapides + un abonné qui cesse de lire sa socket (tampon de réception TCP de 4 Kio, aucun accusé) ; tous abonnés à `moli/#` en QoS 0 ; 100 topics, charge de 120 o ; `ulimit -n` du courtier : 65536/65536
- éditeur : 5000 appels `publish()` rendus en 0.01 s ; tout écrit sur la socket en 0.02 s ; débit éditeur 252782 messages/s
- abonné rapide 1 : tout reçu en jamais (30 s) — 2907/5000 reçus, 2093 manquants (séquences 205 à 4498), 0 doublons, ordre conservé ; abonné rapide 2 : tout reçu en jamais (30 s) — 2707/5000 reçus, 2293 manquants (séquences 205 à 4498), 0 doublons, ordre conservé
- abonné bloque : 0 messages reçus pendant la fenêtre de 10 s ; après reprise à pleine vitesse : 2707/5000 reçus, 2293 manquants (séquences 205 à 4498), 0 doublons, ordre conservé ; reconnexions : 0

| phase (mesurée par le courtier sur lui-même, Kio) | RssAnon | VmRSS | VmHWM (pic) | fils | descripteurs | cgroup `slab` | cgroup `memory.current` |
|---|---|---|---|---|---|---|---|
| au repos (3 abonnés connectés) | 440 | 3200 | 3200 | 4 | 12 | 250 | 1084 |
| juste après la rafale | 2040 | 4864 | 4928 | 4 | 13 | 260 | 3308 |
| 10 s plus tard (abonné bloque toujours dans son état) | 2040 | 4864 | 4928 | 4 | 13 | 260 | 3308 |
| après la reprise | 2040 | 4864 | 4928 | 4 | 13 | 265 | 2680 |
| après fermeture des clients | 996 | 3820 | 4928 | 4 | 10 | 259 | 1632 |

CPU du courtier pendant la rafale : 30 ms ; mémoire (RssAnon) due à la rafale : 1600 Kio ; 10 s plus tard : 1600 Kio ; après reprise : 1600 Kio.
RESULTAT {"apres_observation":{"RssAnon":2040,"RssFile":2824,"Threads":4,"VmData":18900,"VmHWM":4928,"VmRSS":4864,"VmSize":22796,"cg_anon":2040,"cg_current":3308,"cg_file":0,"cg_kernel":420,"cg_max":131072,"cg_peak":3680,"cg_slab":260,"cg_sock":624,"cpu_ms":30,"fds":13,"nofile":"65536/65536","pid":1},"apres_rafale":{"RssAnon":2040,"RssFile":2824,"Threads":4,"VmData":18900,"VmHWM":4928,"VmRSS":4864,"VmSize":22796,"cg_anon":2040,"cg_current":3308,"cg_file":0,"cg_kernel":420,"cg_max":131072,"cg_peak":3680,"cg_slab":260,"cg_sock":624,"cpu_ms":30,"fds":13,"nofile":"65536/65536","pid":1},"apres_reprise":{"RssAnon":2040,"RssFile":2824,"Threads":4,"VmData":18900,"VmHWM":4928,"VmRSS":4864,"VmSize":22796,"cg_anon":2048,"cg_current":2680,"cg_file":0,"cg_kernel":424,"cg_max":131072,"cg_peak":3680,"cg_slab":265,"cg_sock":0,"cpu_ms":30,"fds":13,"nofile":"65536/65536","pid":1},"avant":{"RssAnon":440,"RssFile":2760,"Threads":4,"VmData":16840,"VmHWM":3200,"VmRSS":3200,"VmSize":20736,"cg_anon":440,"cg_current":1084,"cg_file":0,"cg_kernel":408,"cg_max":131072,"cg_peak":3508,"cg_slab":250,"cg_sock":0,"cpu_ms":0,"fds":12,"nofile":"65536/65536","pid":1},"lent":"2707/5000 reçus, 2293 manquants (séquences 205 à 4498), 0 doublons, ordre conservé","mode":"bloque","n":5000,"qos":0,"rapide1":"2907/5000 reçus, 2093 manquants (séquences 205 à 4498), 0 doublons, ordre conservé","rapide2":"2707/5000 reçus, 2293 manquants (séquences 205 à 4498), 0 doublons, ordre conservé","scenario":"rafale"}
- courtier : OOMKilled=false, état=running, journal : courtier rumqttd 0.20.0 sur :1883 (segments 65536 o × 2, dynamic_filters=true, max_outgoing_packet_count=200), contrôle :8801, nofile Some(String("65536/65536"))

## Rafale de 5000, QoS 1, abonné bloque, segments 64 Kio × 2

### rafale de 5000 publications QoS 1 · abonné « bloque » · observation 10 s
- deux abonnés rapides + un abonné qui cesse de lire sa socket (tampon de réception TCP de 4 Kio, aucun accusé) ; tous abonnés à `moli/#` en QoS 1 ; 100 topics, charge de 120 o ; `ulimit -n` du courtier : 65536/65536
- éditeur : 5000 appels `publish()` rendus en 0.01 s ; tous les PUBACK reçus en 0.02 s ; débit éditeur 284777 messages/s
- abonné rapide 1 : tout reçu en 0.02 s — 5000/5000 reçus, 0 manquants, 0 doublons, ordre conservé ; abonné rapide 2 : tout reçu en 0.02 s — 5000/5000 reçus, 0 manquants, 0 doublons, ordre conservé
- abonné bloque : 0 messages reçus pendant la fenêtre de 10 s ; après reprise à pleine vitesse : 601/5000 reçus, 4399 manquants (séquences 100 à 4498), 0 doublons, ordre conservé ; reconnexions : 0

| phase (mesurée par le courtier sur lui-même, Kio) | RssAnon | VmRSS | VmHWM (pic) | fils | descripteurs | cgroup `slab` | cgroup `memory.current` |
|---|---|---|---|---|---|---|---|
| au repos (3 abonnés connectés) | 440 | 3200 | 3200 | 4 | 12 | 249 | 1016 |
| juste après la rafale | 1280 | 4040 | 4040 | 4 | 13 | 269 | 2020 |
| 10 s plus tard (abonné bloque toujours dans son état) | 1280 | 4040 | 4040 | 4 | 13 | 256 | 1948 |
| après la reprise | 1356 | 4116 | 4116 | 4 | 13 | 256 | 2008 |
| après fermeture des clients | 700 | 3460 | 4020 | 4 | 10 | 252 | 1576 |

CPU du courtier pendant la rafale : 10 ms ; mémoire (RssAnon) due à la rafale : 840 Kio ; 10 s plus tard : 840 Kio ; après reprise : 916 Kio.
RESULTAT {"apres_observation":{"RssAnon":1280,"RssFile":2760,"Threads":4,"VmData":17696,"VmHWM":4040,"VmRSS":4040,"VmSize":21592,"cg_anon":1280,"cg_current":1948,"cg_file":0,"cg_kernel":412,"cg_max":131072,"cg_peak":3768,"cg_slab":256,"cg_sock":20,"cpu_ms":20,"fds":13,"nofile":"65536/65536","pid":1},"apres_rafale":{"RssAnon":1280,"RssFile":2760,"Threads":4,"VmData":17696,"VmHWM":4040,"VmRSS":4040,"VmSize":21592,"cg_anon":1280,"cg_current":2020,"cg_file":0,"cg_kernel":432,"cg_max":131072,"cg_peak":3768,"cg_slab":269,"cg_sock":28,"cpu_ms":20,"fds":13,"nofile":"65536/65536","pid":1},"apres_reprise":{"RssAnon":1356,"RssFile":2760,"Threads":4,"VmData":17692,"VmHWM":4116,"VmRSS":4116,"VmSize":21588,"cg_anon":1356,"cg_current":2008,"cg_file":0,"cg_kernel":416,"cg_max":131072,"cg_peak":3768,"cg_slab":256,"cg_sock":0,"cpu_ms":20,"fds":13,"nofile":"65536/65536","pid":1},"avant":{"RssAnon":440,"RssFile":2760,"Threads":4,"VmData":16840,"VmHWM":3200,"VmRSS":3200,"VmSize":20736,"cg_anon":440,"cg_current":1016,"cg_file":0,"cg_kernel":404,"cg_max":131072,"cg_peak":3768,"cg_slab":249,"cg_sock":0,"cpu_ms":10,"fds":12,"nofile":"65536/65536","pid":1},"lent":"601/5000 reçus, 4399 manquants (séquences 100 à 4498), 0 doublons, ordre conservé","mode":"bloque","n":5000,"qos":1,"rapide1":"5000/5000 reçus, 0 manquants, 0 doublons, ordre conservé","rapide2":"5000/5000 reçus, 0 manquants, 0 doublons, ordre conservé","scenario":"rafale"}
- courtier : OOMKilled=false, état=running, journal : courtier rumqttd 0.20.0 sur :1883 (segments 65536 o × 2, dynamic_filters=true, max_outgoing_packet_count=200), contrôle :8801, nofile Some(String("65536/65536"))
`````

Sortie brute — `acl` (Linux) :

`````text
# Banc `acl` : une vérification de sujet MQTT contre des motifs autorisés (jokers `+` et `#`)
- plateforme : linux / aarch64 ; Rust release, `opt-level = "s"` (comme la production, sans LTO)
- justesse vérifiée avant la mesure : 19 cas de la norme et de l'usage, et 20000 couples (sujet, motif) tirés au hasard identiques à `rumqttd::protocol::matches`
- **proposé** = motifs découpés une fois à la construction (`Vec<Box<[Niveau]>>`), sujet découpé une fois par vérification dans un tableau de 16 niveaux sur la pile, comparaison niveau par niveau, règle `$` de la norme ; **re-découpe** = même chose mais le sujet est re-découpé pour chaque motif ; **rumqttd** = `rumqttd::protocol::matches(sujet, motif)` pour chaque motif (re-découpe tout à chaque appel, refuse tout sujet en `$`)
- sujets de 40 octets exactement, 64 sujets différents en rotation (même longueur, nom d'appareil différent) ; médiane de 5 mesures (minimum entre parenthèses) ; exemples de motifs : `zigbee2mqtt/#`, `tasmota/+/stat/#`, `site04/area/+/dev/+/state`, `fam05/#`, `hub06/+/stat/#`, `edge07/dev/+/state`

| motifs | cas | **proposé** (ns / vérification) | re-découpe (ns) | `rumqttd::matches` (ns) | exemple de sujet |
|---|---|---|---|---|---|
| 1 | accordé, par le 1er motif | **30.5** (30.2) | 13.9 (13.5) | 17.2 (16.8) | `zigbee2mqtt/d000___________/availability` |
| 1 | refusé (aucun motif ne correspond) | **32.2** (31.8) | 8.3 (8.2) | 11.1 (11.0) | `inconnu/s000__/area/salon/dev/lampe_1/st` |
| 10 | accordé, par le 1er motif | **30.6** (30.3) | 13.8 (13.7) | 16.9 (16.8) | `zigbee2mqtt/d000___________/availability` |
| 10 | accordé, par le dernier motif (10ᵉ) | **37.1** (36.7) | 82.7 (80.2) | 114.0 (112.7) | `fam09/d000________________/telemetry/a/b` |
| 10 | refusé (aucun motif ne correspond) | **44.2** (44.0) | 82.9 (80.9) | 107.5 (105.1) | `inconnu/s000__/area/salon/dev/lampe_1/st` |
| 50 | accordé, par le 1er motif | **31.9** (30.6) | 13.8 (13.7) | 18.1 (17.4) | `zigbee2mqtt/d000___________/availability` |
| 50 | accordé, par le dernier motif (50ᵉ) | **113.7** (104.9) | 457.2 (445.0) | 587.5 (560.5) | `fam49/d000________________/telemetry/a/b` |
| 50 | refusé (aucun motif ne correspond) | **91.1** (86.8) | 382.7 (376.2) | 544.8 (504.2) | `inconnu/s000__/area/salon/dev/lampe_1/st` |

Lecture : un passage par le courtier coûte de l'ordre de 3 à 10 µs de CPU du routeur par message et ~25 µs de latence (liens locaux, voir `courtier.txt`) ; une vérification à 50 motifs coûte ce qui est indiqué ci-dessus.
- exécuté dans le conteneur limité à 2 CPU / 128 Mio (OOMKilled=false)
`````

Sortie brute — mémoire : tableau final (Linux, musl) :

`````text
Allocateur : musl (mallocng). Mio ; « dem. » = octets demandés + 16 o/bloc ; « réel » = mémoire anonyme (smaps_rollup).

| poste | aujourd'hui ×1 dem. / réel | ×3 | ×10 | optimisé ×1 dem. / réel | ×3 | ×10 |
|---|---:|---:|---:|---:|---:|---:|
| Socle : runtime, catalogues de traduction, hub vide | 1.21 / 1.26 | 1.21 / 1.26 | 1.21 / 1.26 | 1.21 / 1.26 | 1.21 / 1.26 | 1.21 / 1.26 |
| Hub : définitions, valeurs, étage vif, santé | 1.60 / 1.64 | 4.65 / 4.71 | 15.68 / 15.80 | 0.22 / 0.27 | 0.61 / 0.65 | 2.11 / 2.14 |
| Bus d'événements (1 024, plein) | 0.06 / 0.06 | 0.06 / 0.05 | 0.06 / 0.06 | 0.00 / 0.00 | 0.00 / 0.00 | 0.00 / 0.00 |
| Miroir du moteur de workflow | 0.31 / 0.33 | 0.75 / 0.78 | 2.76 / 2.84 | 0.04 / 0.04 | 0.11 / 0.11 | 0.35 / 0.35 |
| Miroir du module WebSocket | 0.47 / 0.48 | 1.06 / 1.09 | 4.01 / 4.09 | 0.00 / 0.00 | 0.00 / 0.00 | 0.00 / 0.00 |
| Queue récente du flux `val/` | 0.94 / 0.95 | 2.82 / 2.82 | 9.40 / 9.36 | 0.09 / 0.10 | 0.28 / 0.29 | 0.94 / 0.95 |
| Automatismes : définitions en mémoire | 3.54 / 3.97 | 10.63 / 11.90 | 35.43 / 39.69 | 0.00 / 0.00 | 0.00 / 0.00 | 0.00 / 0.00 |
| Automatismes compilés et index par point | 0.26 / 0.28 | 0.79 / 0.84 | 2.66 / 2.82 | 0.11 / 0.12 | 0.33 / 0.35 | 1.08 / 1.11 |
| Baux et échéancier | 0.08 / 0.08 | 0.25 / 0.23 | 0.86 / 0.77 | 0.04 / 0.04 | 0.16 / 0.14 | 0.63 / 0.39 |
| Exécutions en cours | 0.09 / 0.09 | 0.26 / 0.26 | 0.87 / 0.87 | 0.02 / 0.02 | 0.05 / 0.05 | 0.15 / 0.16 |
| Agrégats 1 h / 24 h | 0.43 / 0.45 | 1.36 / 1.34 | 4.69 / 4.47 | 0.31 / 0.31 | 0.94 / 0.93 | 3.13 / 3.12 |
| Courtier : messages retenus (`val/`, `avail/`, `meta/`) | 2.63 / 2.73 | 8.88 / 9.00 | 32.45 / 32.61 | 0.40 / 0.50 | 1.27 / 1.41 | 4.55 / 4.84 |
| Courtier : routeur, liens, filtres, journaux, caches par sujet, rafale | 20.87 / 7.02 | 22.34 / 8.68 | 30.20 / 15.88 | 5.87 / 3.37 | 7.90 / 5.50 | 15.13 / 12.33 |
| Courtier : 10 000 premières demandes `req/` (journal `req/#` compris) | 4.74 / 4.16 | 4.35 / 3.70 | 6.65 / 6.04 | 0.76 / 0.43 | 1.75 / 1.14 | 0.76 / 0.34 |
| Courtier : chaque 10 000 demandes de plus (croissance sans fin si sujets uniques) | 1.95 / 1.86 | 3.71 / 3.39 | 1.95 / 1.86 | 0.03 / 0.03 | 0.03 / 0.04 | 0.03 / 0.01 |
| **Total (hors croissance des demandes)** | **37.23 / 23.48** | **59.42 / 46.68** | **146.93 / 136.58** | **9.07 / 6.45** | **14.60 / 11.84** | **30.04 / 26.98** |
`````

Sortie brute — mémoire : inventaire (Linux) :

`````text
# Inventaire mesuré (allocateur : musl (mallocng))

Effectifs de ×10 construits pour la mesure : 4960 appareils, 22640 points, 4000 automatismes. Les totaux ×1/×3/×10 multiplient le coût par objet (demandé + 16 o/bloc, la mesure de la V4) par l'effectif.

(`Kind` 48 o, `PointSpec` 112 o, `Device` 160 o)

(`CommandRequest` 200 o, en vol seulement ; `Action` 160 o)

(JSON d'un automatisme : 1972 o)

| chose | forme Rust | `size_of` | tas par objet : octets (blocs) | par objet, demandé (+16 o/bloc) | ×1 | ×3 | ×10 |
|---|---|---:|---:|---:|---:|---:|---:|
| **Identités** | | | | | | | |
| `DeviceId` (22 car.) | `Arc<str>` neuf | 16 o | 56.0 (1.00) | 72 o | 35 Kio | 105 Kio | 349 Kio |
| `PointId` (≈ 33 car.) | `Arc<str>` neuf (`PointId::new`, à chaque événement) | 16 o | 67.8 (1.00) | 84 o | 185 Kio | 556 Kio | 1.81 Mio |
| sujet MQTT `val/` (≈ 45 car.) | `String` | 24 o | 72.6 (1.00) | 89 o | 196 Kio | 588 Kio | 1.91 Mio |
| clé de miroir (le `PointId` en `String`) | `String` (moteur d'automatismes, `last`) | 24 o | 56.5 (1.00) | 73 o | 160 Kio | 481 Kio | 1.57 Mio |
| **Valeurs** | | | | | | | |
| mesure, booléen | `Value::Float`, `Value::Bool` | 24 o | 24.0 (0.00) | 24 o | — | — | — |
| énumération (« auto ») | `Value::Text(Arc<str>)` neuf | 24 o | 48.0 (1.00) | 64 o | — | — | — |
| texte court (11 car.) | `Value::Text(Arc<str>)` neuf | 24 o | 56.0 (1.00) | 72 o | — | — | — |
| valeur d'un point, mélange de la maison | `Value` | 24 o | 29.2 (0.20) | 32 o | 72 Kio | 215 Kio | 716 Kio |
| **Définitions** | | | | | | | |
| spécification d'une mesure | `PointSpec` (`key`, `label` en `Arc<str>`) | 112 o | 176.0 (2.00) | 208 o | — | — | — |
| spécification d'une énumération (5 libellés) | `PointSpec` + `Vec<Arc<str>>` | 112 o | 376.0 (8.00) | 504 o | — | — | — |
| appareil et ses 4-5 spécifications | `Arc<Device>` | 160 o | 1274.8 (20.98) | 1611 o | 780 Kio | 2.29 Mio | 7.62 Mio |
| … ramené au point |  | — | 279.3 (4.60) | 353 o | — | — | — |
| **Socle du hub (indépendant de la maison)** | | | | | | | |
| catalogues de traductions (fr + en, chargés au premier mot) | `HashMap<&str, HashMap<String, String>>` | — | 824475.0 (14340.00) | 1053915 o | 1.01 Mio | 1.01 Mio | 1.01 Mio |
| bus vide (`broadcast` de 1 024 places) | `broadcast::Sender<Event>` | — | 155784.0 (2.00) | 155816 o | 152 Kio | 152 Kio | 152 Kio |
| hub vide (`Hub::new`, catalogues déjà chargés) | `Hub` | — | 159832.0 (8.00) | 159960 o | 156 Kio | 156 Kio | 156 Kio |
| **Magasin du hub (moli-runtime, `Model`)** | | | | | | | |
| entrée `devices` | `BTreeMap<DeviceId, Arc<Device>>` | — | 26.3 (0.09) | 28 o | 13 Kio | 40 Kio | 135 Kio |
| valeur d'un point (`states`) | `HashMap<DeviceId, BTreeMap<Arc<str>, Sample>>` | 32 o | 139.2 (0.42) | 146 o | 323 Kio | 968 Kio | 3.15 Mio |
| disponibilité | `HashMap<DeviceId, bool>` | — | 41.3 (0.00) | 41 o | 20 Kio | 60 Kio | 200 Kio |
| étiquette (nom, pièce) | `BTreeMap<DeviceId, Label>` | 48 o | 90.2 (2.09) | 124 o | 60 Kio | 180 Kio | 599 Kio |
| **Événements, journal, ordres** | | | | | | | |
| événement d'une valeur | `Event::State` (+ `PointId` neuf) | 128 o | 185.0 (1.20) | 204 o | 204 Kio | 204 Kio | 204 Kio |
| tampon du bus plein (`broadcast` de 1 024) | `broadcast::Sender<Event>` | — | 209.1 (1.20) | 228 o | 228 Kio | 228 Kio | 228 Kio |
| ligne du journal en mémoire (500 gardées) | `Arc<JournalEntry>` | 272 o | 359.0 (3.00) | 407 o | 199 Kio | 199 Kio | 199 Kio |
| **V6, formes naïves (esquisses de la V6, banc V4)** | | | | | | | |
| étage vif d'un point | `PointVif` (2 `Value`, 3 × `u64`, `u8`) | 80 o | 90.4 (0.40) | 97 o | 214 Kio | 642 Kio | 2.09 Mio |
| index des points | `HashMap<PointId, u32>` | — | 87.9 (1.00) | 104 o | 230 Kio | 689 Kio | 2.24 Mio |
| santé d'un appareil (§ 9.5) | `SanteAppareil` (`Instant`, `Option<Duration>`) | 88 o | 88.0 (0.00) | 88 o | 43 Kio | 128 Kio | 426 Kio |
| entrée du miroir du moteur | `HashMap<String, Value>` (moli-automation, `last`) | — | 108.6 (1.20) | 128 o | 283 Kio | 848 Kio | 2.76 Mio |
| entrée du miroir WebSocket | `HashMap<String, ValeurMiroir>` | 64 o | 166.5 (1.20) | 186 o | 411 Kio | 1.20 Mio | 4.01 Mio |
| entrée de la queue `val/` | `VecDeque<Publication>` (sujet + charge JSON) | 56 o | 208.6 (2.00) | 241 o | 963 Kio | 2.82 Mio | 9.40 Mio |
| automatisme, une copie (6 nœuds, 3 règles) | `moli_automation::Automation` (graphe + version approuvée) | 248 o | 3539.4 (69.00) | 4643 o | 1.77 Mio | 5.31 Mio | 17.71 Mio |
| … deux copies (`list` + `live_set`) |  | — | 7078.8 (138.00) | 9287 o | 3.54 Mio | 10.63 Mio | 35.43 Mio |
| … une copie sans la version approuvée (identique au graphe) | `Automation`, `approved_version: None` | 248 o | 1948.4 (36.00) | 2524 o | 986 Kio | 2.89 Mio | 9.63 Mio |
| automatisme compilé + index | `Plan` du § 10.2 (`Vec` par plan, `Value`) | 88 o | 617.6 (5.00) | 698 o | 272 Kio | 817 Kio | 2.66 Mio |
| bail + échéancier | `Bail` (2 `Value`) + `BinaryHeap<(u64, u32)>` | 64 o | 90.2 (0.00) | 90 o | 88 Kio | 264 Kio | 881 Kio |
| exécution en cours | `Execution` du § 10.2 (cause avec `PointId`) | 160 o | 211.8 (1.00) | 228 o | 89 Kio | 267 Kio | 890 Kio |
| agrégats 1 h + 24 h d'un point lu | 2 × `Agregat` (`VecDeque<(u32, f32)>`), taux réels | 48 o | 1075.5 (0.70) | 1087 o | 480 Kio | 1.41 Mio | 4.69 Mio |
| **Prototype compact** | | | | | | | |
| état vif complet, ramené au point | `compact::Etat` (définitions, noms, index, valeurs, santé) | — | 96.9 (0.05) | 98 o | 216 Kio | 648 Kio | 2.11 Mio |
| valeur vive | `Vif` | 40 o | 40.0 (0.00) | 40 o | 88 Kio | 265 Kio | 884 Kio |
| variante à 32 o (observé en écart, saturé) | `Vif32` | 32 o | 32.0 (0.00) | 32 o | 71 Kio | 212 Kio | 708 Kio |
| texte court en ligne (11 car.) | `ValeurCompacte::Texte(CompactString)` | 24 o | 24.0 (0.00) | 24 o | — | — | — |
| texte de plus de 24 octets | `ValeurCompacte::Texte(CompactString)` | 24 o | 92.0 (1.00) | 108 o | — | — | — |
| définition d'un point | `Def` | 16 o | 16.0 (0.00) | 16 o | 35 Kio | 106 Kio | 354 Kio |
| appareil | `App` | 28 o | 28.0 (0.00) | 28 o | 14 Kio | 41 Kio | 136 Kio |
| santé d'un appareil | `Sante` | 20 o | 20.0 (0.00) | 20 o | 9.7 Kio | 29 Kio | 97 Kio |
| entrée du miroir du moteur | `Copie` dans un `Vec` indexé par point | 16 o | 16.0 (0.00) | 16 o | 35 Kio | 106 Kio | 354 Kio |
| entrée de la queue `val/` | `Entree` dans un anneau (`s` implicite) | 24 o | 24.0 (0.00) | 24 o | 96 Kio | 288 Kio | 960 Kio |
| automatisme compilé + index CSR | `PlanC` + 6 `Noeud` + 3 `Regle` (tableaux communs) | 24 o | 283.8 (0.00) | 284 o | 111 Kio | 333 Kio | 1.08 Mio |
| bail + échéancier | `BailC` + `BinaryHeap<(u32, u32)>` | 32 o | 65.5 (0.00) | 66 o | 64 Kio | 192 Kio | 640 Kio |
| exécution en cours | `ExecC` | 40 o | 40.0 (0.00) | 40 o | 16 Kio | 47 Kio | 156 Kio |
| agrégats 1 h + 24 h d'un point lu | 30 `Seau` (6 × 10 min, 24 × 1 h) | 24 o | 724.0 (0.00) | 724 o | 320 Kio | 960 Kio | 3.13 Mio |
| **Courtier rumqttd 0.20 (tailles ; le coût d'un retenu est mesuré par le banc `courtier`)** | | | | | | | |
| `Publish` (sujet et charge en `Bytes`) | `rumqttd::protocol::Publish` | 72 o | — | — | — | — | — |
| `Option<PublishProperties>` | propriétés MQTT 5, `None` en 3.1.1 | 144 o | — | — | — | — | — |
| `PublishData` (retenu, entrée du journal d'un filtre) | `Publish` + propriétés + `Instant` | 232 o | — | — | — | — | — |
| `Notification` (file de sortie d'une connexion) | `rumqttd::Notification` | 248 o | — | — | — | — | — |
| segment neuf d'un filtre | `Vec::with_capacity(1024)` de `PublishData` | 237568 o | — | — | — | — | — |
| paquet en file d'entrée d'un lien | `rumqttd::protocol::Packet` | 392 o | — | — | — | — | — |

## Bilan du prototype à ×10 (capacités des tableaux)

| partie | octets | par point |
|---|---:|---:|
| noms des appareils (DeviceId) | 166 Kio | 7.5 o |
| clés des points | 760 o | 0.0 o |
| mots (libellés, fabricants, modèles, pièces, unités, énumérations) | 151 Kio | 6.8 o |
| index (appareil, clé) → point | 160 Kio | 7.2 o |
| définitions des points (Def, 16 o) | 354 Kio | 16.0 o |
| dictionnaires, unités, bornes | 116 Kio | 5.2 o |
| appareils (App, 28 o) | 136 Kio | 6.1 o |
| valeurs vives (Vif, 40 o) | 884 Kio | 40.0 o |
| textes courts (courant et précédent) | 55 Kio | 2.5 o |
| santé des appareils (Sante, 20 o) | 97 Kio | 4.4 o |
| **total** | **2.07 Mio** | **95.8 o** |
(conteneur : OOMKilled=false, code de sortie 0)
`````

Sortie brute — mémoire : fuite des sujets uniques (Linux) :

`````text
sans abonné, sujet unique répété                                 52959 o       5 blocs  (5.3 o, 0.00 bloc par publication)
sans abonné, sujet unique répété (2e série)                          0 o       0 blocs  (0.0 o, 0.00 bloc par publication)
sans abonné, 10 000 sujets neufs                                730470 o   10000 blocs  (73.0 o, 1.00 bloc par publication)
sans abonné, les mêmes 10 000 sujets                                 0 o       0 blocs  (0.0 o, 0.00 bloc par publication)
abonné abo/#, sujet répété                                      262766 o     805 blocs  (26.3 o, 0.08 bloc par publication)
abonné abo/#, sujet répété (2e série)                              512 o      16 blocs  (0.1 o, 0.00 bloc par publication)
abonné abo/#, 10 000 sujets neufs                              1732930 o   30616 blocs  (173.3 o, 3.06 bloc par publication)
abonné abo/#, les mêmes 10 000 sujets                            -3668 o    -112 blocs  (-0.4 o, -0.01 bloc par publication)
retenus sans abonné, 10 000 sujets neufs                       7136700 o   60001 blocs  (713.7 o, 6.00 bloc par publication)
retenus sans abonné, les mêmes 10 000 sujets                         0 o       0 blocs  (0.0 o, 0.00 bloc par publication)
(conteneur : OOMKilled=false, code de sortie 0)
`````

Sortie brute — mémoire : lecture d'une condition (Linux) :

`````text
# Lecture d'une condition, résolution d'un sujet, entrée d'une valeur (ns par opération)

### ×1 : 2264 points

| opération | ns |
|---|---:|
| lire une condition — prototype, `Vec<Vif>` par index | 0.6 |
| lire une condition — prototype derrière un `RwLock` (verrou à chaque lecture) | 3.7 |
| lire une condition — `HashMap<String, Value>` par le nom du point (moteur actuel, `last`) | 39.1 |
| lire une condition — index `HashMap<PointId, u32>` puis `Vec<PointVif>` (banc V4) | 32.3 |
| lire une condition — `hub.state(&PointId)` (moli-runtime aujourd'hui) | 91.1 |
| résoudre un sujet `val/` → point — prototype (`Noms` + index) | 61.8 |
| résoudre un sujet `val/` → point — `HashMap<String, u32>` | 45.5 |
| entrer une mesure — prototype : `appliquer` + santé + queue `val/` | 4.3 |
| entrer une mesure — `HashMap<String, Value>` : `insert` d'une clé neuve (`format!`) | 65.1 |

### ×10 : 22640 points

| opération | ns |
|---|---:|
| lire une condition — prototype, `Vec<Vif>` par index | 1.6 |
| lire une condition — prototype derrière un `RwLock` (verrou à chaque lecture) | 5.3 |
| lire une condition — `HashMap<String, Value>` par le nom du point (moteur actuel, `last`) | 114.2 |
| lire une condition — index `HashMap<PointId, u32>` puis `Vec<PointVif>` (banc V4) | 48.0 |
| lire une condition — `hub.state(&PointId)` (moli-runtime aujourd'hui) | 117.5 |
| résoudre un sujet `val/` → point — prototype (`Noms` + index) | 82.2 |
| résoudre un sujet `val/` → point — `HashMap<String, u32>` | 58.1 |
| entrer une mesure — prototype : `appliquer` + santé + queue `val/` | 4.5 |
| entrer une mesure — `HashMap<String, Value>` : `insert` d'une clé neuve (`format!`) | 97.6 |
(conteneur : OOMKilled=false, code de sortie 0)
`````

Code complet — `preuves-v5/src/bin/connexions.rs` :

`````rust
//! Banc 4 : mémoire et CPU du PROCESSUS SERVEUR par connexion vivante, SSE contre WebSocket.
//!
//! Trois rôles dans un seul binaire :
//! - `serveur <sse|ws|ws-leger> <port> <octets_snapshot>` : serveur axum 0.8 reproduisant le motif de
//!   `moli-api` (broadcast de 1 024, `Sse` + `KeepAlive::default()`, snapshot initial, vraies couches
//!   `CompressionLayer` + en-têtes de sécurité) ; `ws` = même chose en WebSocket avec la configuration par
//!   défaut d'axum, `ws-leger` = tampons de tungstenite réduits. `GET /mesure` : il lit lui-même
//!   `/proc/self/status`, `/proc/self/stat` et le cgroup ; `POST /diffuser?hz=&secs=` : diffuse des
//!   `Event::State` à toutes les connexions.
//! - `clients <mode> <port> <N> <octets_snapshot> <hz> <secs> <secs_inactif>` : AUTRE processus (voire autre
//!   conteneur) qui ouvre N connexions inactives, pilote les mesures du serveur et imprime les résultats.
//! - `tout <mode> <N> <octets_snapshot> …` : lance le serveur en processus fils puis les clients (Mac, mise au point).
//! - `resume` : lit sur l'entrée standard des lignes `RESULTAT {json}` et imprime la synthèse (pentes).

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::stream::{self};
use moli_core::{DeviceId, Event, InstanceId, PointId, Value};
use preuves_v5::{avec_pile_moli, decouper, echange};
use serde_json::{Map, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{Semaphore, broadcast};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Sse,
    /// SSE sans les couches de Moli (ni compression ni en-têtes de sécurité) : isole le coût de hyper/axum seuls.
    SseNu,
    Ws,
    WsLeger,
}

impl Mode {
    fn parse(s: &str) -> Mode {
        match s {
            "sse" => Mode::Sse,
            "sse-nu" => Mode::SseNu,
            "ws" => Mode::Ws,
            "ws-leger" => Mode::WsLeger,
            autre => panic!("mode inconnu : {autre}"),
        }
    }
    fn nom(self) -> &'static str {
        match self {
            Mode::Sse => "sse",
            Mode::SseNu => "sse-nu",
            Mode::Ws => "ws",
            Mode::WsLeger => "ws-leger",
        }
    }
    fn est_ws(self) -> bool {
        matches!(self, Mode::Ws | Mode::WsLeger)
    }
}

// =============================================================================================
// Serveur
// =============================================================================================

/// Compte les connexions vivantes (décrémente quand le flux ou la tâche est libéré).
struct Garde(Arc<AtomicUsize>);
impl Garde {
    fn new(c: &Arc<AtomicUsize>) -> Self {
        c.fetch_add(1, Relaxed);
        Garde(c.clone())
    }
}
impl Drop for Garde {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Relaxed);
    }
}

#[derive(Clone)]
struct App {
    tx: broadcast::Sender<Event>,
    snapshot: Arc<str>,
    conns: Arc<AtomicUsize>,
    mode: Mode,
}

/// Même forme que `moli-api/src/sse.rs` (sans l'autorisation d'invité) : un événement `snapshot` à la
/// connexion, puis un événement `event` par changement, nouveau snapshot si l'abonné est en retard.
async fn sse(State(app): State<App>) -> Response {
    let garde = Garde::new(&app.conns);
    let rx = app.tx.subscribe();
    let etat = (rx, app.snapshot.clone(), true, garde);
    let flux = stream::unfold(etat, |(mut rx, snap, premier, garde)| async move {
        let ev = if premier {
            SseEvent::default().event("snapshot").data(&*snap)
        } else {
            loop {
                match rx.recv().await {
                    Ok(e) => break SseEvent::default().event("event").json_data(&e).ok()?,
                    Err(broadcast::error::RecvError::Lagged(_)) => break SseEvent::default().event("snapshot").data(&*snap),
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        };
        Some((Ok::<_, Infallible>(ev), (rx, snap, false, garde)))
    });
    Sse::new(flux).keep_alive(KeepAlive::default()).into_response()
}

/// Même chose en WebSocket : snapshot en premier message, puis un message texte JSON par événement.
async fn ws(upgrade: WebSocketUpgrade, State(app): State<App>) -> Response {
    let garde = Garde::new(&app.conns);
    let mut rx = app.tx.subscribe();
    let upgrade = if app.mode == Mode::WsLeger { upgrade.read_buffer_size(4096).write_buffer_size(0) } else { upgrade };
    let snap = app.snapshot.clone();
    upgrade.on_upgrade(move |mut socket| async move {
        let _garde = garde;
        if socket.send(Message::Text(snap.to_string().into())).await.is_err() {
            return;
        }
        loop {
            tokio::select! {
                e = rx.recv() => match e {
                    Ok(e) => {
                        let texte = serde_json::to_string(&e).unwrap();
                        if socket.send(Message::Text(texte.into())).await.is_err() { break; }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        if socket.send(Message::Text(snap.to_string().into())).await.is_err() { break; }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                },
                m = socket.recv() => match m {
                    None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                    _ => {}
                },
            }
        }
    })
}

#[derive(serde::Deserialize)]
struct Diffusion {
    hz: u64,
    secs: u64,
}

async fn diffuser(State(app): State<App>, Query(d): Query<Diffusion>) -> Json<serde_json::Value> {
    let device = DeviceId::new(&InstanceId::from("z2m"), "0x00124b00aaaa0001");
    let point = PointId::new(&device, "temperature");
    let total = d.hz * d.secs;
    let mut tic = tokio::time::interval(Duration::from_micros(1_000_000 / d.hz));
    tic.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);
    let (mut envoyes, mut recepteurs) = (0u64, 0usize);
    for k in 0..total {
        tic.tick().await;
        let ts = 1_792_675_795_000 + k * (1000 / d.hz);
        let e = Event::State { point: point.clone(), value: Value::Float(21.0 + (k % 100) as f64 * 0.01), ts, source_ts: None };
        recepteurs = app.tx.send(e).unwrap_or(0);
        envoyes += 1;
    }
    Json(json!({ "envoyes": envoyes, "recepteurs": recepteurs }))
}

async fn mesure(State(app): State<App>) -> Json<serde_json::Value> {
    let mut m = systeme();
    m.insert("conns".into(), app.conns.load(Relaxed).into());
    Json(serde_json::Value::Object(m))
}

/// Lit, côté serveur et sur lui-même, ce que Linux sait de la mémoire et du CPU ; sur macOS, `ps`.
fn systeme() -> Map<String, serde_json::Value> {
    let mut m = Map::new();
    let pid = std::process::id();
    m.insert("pid".into(), pid.into());
    if let Ok(statut) = std::fs::read_to_string("/proc/self/status") {
        for cle in ["VmRSS", "RssAnon", "RssFile", "RssShmem", "VmHWM", "VmData", "VmSize", "VmStk", "Threads"] {
            let prefixe = format!("{cle}:");
            if let Some(v) = statut.lines().find(|l| l.starts_with(&prefixe)).and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse::<u64>().ok()) {
                m.insert(cle.into(), v.into());
            }
        }
        if let Ok(stat) = std::fs::read_to_string("/proc/self/stat") {
            // Après la parenthèse fermante du nom : état (0), ppid (1)… utime (11), stime (12), en ticks (USER_HZ = 100).
            if let Some(fin) = stat.rfind(')') {
                let champs: Vec<&str> = stat[fin + 2..].split_whitespace().collect();
                let (u, s): (u64, u64) = (champs[11].parse().unwrap_or(0), champs[12].parse().unwrap_or(0));
                m.insert("utime_ms".into(), (u * 10).into());
                m.insert("stime_ms".into(), (s * 10).into());
                m.insert("cpu_ms".into(), ((u + s) * 10).into());
            }
        }
        if let Ok(d) = std::fs::read_dir("/proc/self/fd") {
            m.insert("fds".into(), (d.count().saturating_sub(1)).into());
        }
        if let Ok(l) = std::fs::read_to_string("/proc/self/limits") {
            if let Some(ligne) = l.lines().find(|l| l.starts_with("Max open files")) {
                m.insert("nofile".into(), ligne.split_whitespace().skip(3).take(2).collect::<Vec<_>>().join("/").into());
            }
        }
    } else if let Ok(sortie) = std::process::Command::new("ps").args(["-o", "rss=,time=", "-p", &pid.to_string()]).output() {
        // macOS : `ps` donne le RSS en Kio et le temps CPU (user+sys) sous la forme m:ss.cc.
        let texte = String::from_utf8_lossy(&sortie.stdout).into_owned();
        let mut it = texte.split_whitespace();
        if let Some(rss) = it.next().and_then(|v| v.parse::<u64>().ok()) {
            m.insert("VmRSS".into(), rss.into());
        }
        if let Some(t) = it.next() {
            if let Some((min, sec)) = t.split_once(':') {
                let ms = min.parse::<f64>().unwrap_or(0.0) * 60_000.0 + sec.parse::<f64>().unwrap_or(0.0) * 1000.0;
                m.insert("cpu_ms".into(), (ms.round() as u64).into());
            }
        }
    }
    // cgroup v2 : `memory.stat` (octets) et `memory.current` ; ce que le noyau charge au conteneur (sockets, dalles).
    if let Ok(stat) = std::fs::read_to_string("/sys/fs/cgroup/memory.stat") {
        for cle in ["anon", "file", "kernel", "kernel_stack", "slab", "sock", "percpu", "shmem"] {
            if let Some(v) = stat.lines().find_map(|l| l.strip_prefix(&format!("{cle} ")).and_then(|v| v.trim().parse::<u64>().ok())) {
                m.insert(format!("cg_{cle}"), (v / 1024).into()); // en Kio
            }
        }
    }
    for (fichier, cle) in [("memory.current", "cg_current"), ("memory.peak", "cg_peak"), ("memory.max", "cg_max")] {
        if let Some(v) = std::fs::read_to_string(format!("/sys/fs/cgroup/{fichier}")).ok().and_then(|s| s.trim().parse::<u64>().ok()) {
            m.insert(cle.into(), (v / 1024).into());
        }
    }
    m
}

/// Un faux snapshot JSON de la taille demandée (forme d'un `hub.snapshot()` : appareils et états).
fn faux_snapshot(octets: usize) -> String {
    let mut s = String::with_capacity(octets + 256);
    s.push_str("{\"devices\":[");
    let mut i = 0;
    while s.len() < octets.saturating_sub(40) {
        use std::fmt::Write;
        if i > 0 {
            s.push(',');
        }
        let _ = write!(s, "{{\"id\":\"z2m:0x00124b00{i:08x}\",\"state\":{{\"temperature\":{{\"value\":21.5,\"ts\":1792675795123}}}}}}");
        i += 1;
    }
    s.push_str("],\"drivers\":[],\"approvals\":[]}");
    s
}

async fn serveur(mode: Mode, port: u16, octets: usize) {
    let (tx, _) = broadcast::channel::<Event>(1024);
    let app = App { tx, snapshot: faux_snapshot(octets).into(), conns: Arc::new(AtomicUsize::new(0)), mode };
    let base = Router::new().route("/mesure", get(mesure)).route("/diffuser", post(diffuser));
    let base = if mode.est_ws() { base.route("/ws", get(ws)) } else { base.route("/api/events", get(sse)) };
    let routeur = if mode == Mode::SseNu { base.with_state(app) } else { avec_pile_moli(base.with_state(app)) };
    let ecoute = tokio::net::TcpListener::bind(("0.0.0.0", port)).await.unwrap();
    eprintln!("serveur {} sur :{port}, snapshot {octets} o, nofile {:?}", mode.nom(), systeme().get("nofile"));
    axum::serve(ecoute, routeur).await.unwrap();
}

// =============================================================================================
// Clients (processus distinct)
// =============================================================================================

#[derive(Default)]
struct Stats {
    connectes: AtomicUsize,
    /// Clients ayant reçu leur snapshot complet.
    instantane: AtomicUsize,
    /// Messages reçus après le snapshot, tous clients confondus.
    messages: AtomicU64,
    octets: AtomicU64,
    erreurs: AtomicUsize,
}

const REQUETE_SSE: &str = "GET /api/events HTTP/1.1\r\nHost: moli.local:8790\r\nAccept: text/event-stream\r\nCache-Control: no-cache\r\n\r\n";
const REQUETE_WS: &str = "GET /ws HTTP/1.1\r\nHost: moli.local:8790\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n";

/// Décodeur minimal de trames WebSocket serveur→client : compte les messages sans stocker les charges.
#[derive(Default)]
struct Trames {
    en_tete: Vec<u8>,
    restant: u64,
}
impl Trames {
    fn nourrir(&mut self, mut d: &[u8], messages: &mut u64) {
        while !d.is_empty() {
            if self.restant > 0 {
                let k = (self.restant.min(d.len() as u64)) as usize;
                self.restant -= k as u64;
                d = &d[k..];
                if self.restant == 0 {
                    *messages += 1;
                }
                continue;
            }
            self.en_tete.push(d[0]);
            d = &d[1..];
            if self.en_tete.len() >= 2 {
                let l7 = self.en_tete[1] & 0x7f;
                let besoin = 2 + match l7 {
                    126 => 2,
                    127 => 8,
                    _ => 0,
                };
                if self.en_tete.len() == besoin {
                    let longueur = match l7 {
                        126 => u64::from(u16::from_be_bytes([self.en_tete[2], self.en_tete[3]])),
                        127 => u64::from_be_bytes(self.en_tete[2..10].try_into().unwrap()),
                        n => u64::from(n),
                    };
                    self.en_tete.clear();
                    if longueur == 0 {
                        *messages += 1;
                    } else {
                        self.restant = longueur;
                    }
                }
            }
        }
    }
}

async fn client(addr: SocketAddr, mode: Mode, stats: Arc<Stats>, portes: Arc<Semaphore>) {
    let permis = portes.acquire_owned().await.unwrap();
    let Ok(mut s) = TcpStream::connect(addr).await else {
        stats.erreurs.fetch_add(1, Relaxed);
        return;
    };
    let _ = s.set_nodelay(true);
    if s.write_all(if mode.est_ws() { REQUETE_WS } else { REQUETE_SSE }.as_bytes()).await.is_err() {
        stats.erreurs.fetch_add(1, Relaxed);
        return;
    }
    let mut tampon = vec![0u8; 16 * 1024];
    let mut tete: Vec<u8> = Vec::new();
    let mut en_tetes_finis = false;
    let mut permis = Some(permis);
    let mut trames = Trames::default();
    let (mut messages, mut precedent) = (0u64, 0u8);
    loop {
        let Ok(n) = s.read(&mut tampon).await else { break };
        if n == 0 {
            break;
        }
        stats.octets.fetch_add(n as u64, Relaxed);
        let mut corps: &[u8] = &tampon[..n];
        if !en_tetes_finis {
            tete.extend_from_slice(corps);
            let Some(i) = tete.windows(4).position(|w| w == b"\r\n\r\n") else { continue };
            en_tetes_finis = true;
            stats.connectes.fetch_add(1, Relaxed);
            drop(permis.take());
            let reste = tete.split_off(i + 4);
            tete = reste;
            corps = &tete[..];
            // `corps` emprunte `tete` : on traite ici puis on vide.
            let mut nouveaux = 0u64;
            decompter(mode, corps, &mut trames, &mut precedent, &mut nouveaux);
            enregistrer(&stats, &mut messages, nouveaux);
            tete.clear();
            continue;
        }
        let mut nouveaux = 0u64;
        decompter(mode, corps, &mut trames, &mut precedent, &mut nouveaux);
        enregistrer(&stats, &mut messages, nouveaux);
    }
}

/// Compte les messages complets dans `corps` : trames en WebSocket, `\n\n` en SSE (un événement, ou un maintien).
fn decompter(mode: Mode, corps: &[u8], trames: &mut Trames, precedent: &mut u8, nouveaux: &mut u64) {
    if mode.est_ws() {
        trames.nourrir(corps, nouveaux);
    } else {
        for &b in corps {
            if b == b'\n' && *precedent == b'\n' {
                *nouveaux += 1;
                *precedent = 0;
            } else {
                *precedent = b;
            }
        }
    }
}

fn enregistrer(stats: &Stats, deja: &mut u64, nouveaux: u64) {
    for _ in 0..nouveaux {
        *deja += 1;
        if *deja == 1 {
            stats.instantane.fetch_add(1, Relaxed); // le premier message est le snapshot
        } else {
            stats.messages.fetch_add(1, Relaxed);
        }
    }
}

async fn http(addr: SocketAddr, methode: &str, chemin: &str) -> Vec<u8> {
    let requete = format!("{methode} {chemin} HTTP/1.1\r\nHost: moli.local:8790\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let rep = echange(addr, requete.as_bytes()).await;
    if !rep.windows(4).any(|w| w == b"\r\n\r\n") {
        return Vec::new();
    }
    let (entetes, corps) = decouper(&rep);
    if entetes.to_ascii_lowercase().contains("transfer-encoding: chunked") { preuves_v5::dechunk(corps).0 } else { corps.to_vec() }
}

async fn lire_mesure(addr: SocketAddr) -> Map<String, serde_json::Value> {
    match serde_json::from_slice::<serde_json::Value>(&http(addr, "GET", "/mesure").await) {
        Ok(serde_json::Value::Object(m)) => m,
        _ => Map::new(),
    }
}

fn val(m: &Map<String, serde_json::Value>, cle: &str) -> Option<u64> {
    m.get(cle).and_then(|v| v.as_u64())
}

fn kio(m: &Map<String, serde_json::Value>, cle: &str) -> String {
    val(m, cle).map_or("—".into(), |v| v.to_string())
}

async fn clients(mode: Mode, port: u16, n: usize, octets: usize, hz: u64, secs: u64, secs_inactif: u64) {
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    // Attente du serveur.
    let debut = Instant::now();
    while lire_mesure(addr).await.is_empty() {
        assert!(debut.elapsed() < Duration::from_secs(20), "serveur injoignable");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
    let avant = lire_mesure(addr).await;
    let stats = Arc::new(Stats::default());
    let portes = Arc::new(Semaphore::new(150));
    let t0 = Instant::now();
    let taches: Vec<_> = (0..n).map(|_| tokio::spawn(client(addr, mode, stats.clone(), portes.clone()))).collect();
    // Toutes connectées et snapshot reçu partout.
    let delai = Instant::now();
    while stats.instantane.load(Relaxed) < n && delai.elapsed() < Duration::from_secs(60) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let temps_connexion = t0.elapsed();
    let recus = stats.instantane.load(Relaxed);
    tokio::time::sleep(Duration::from_secs(2)).await;
    let apres = lire_mesure(addr).await;
    let octets_snapshot_recus = stats.octets.load(Relaxed);

    // Inactif : N connexions ouvertes, aucun événement ; couvre au moins un maintien SSE de 15 s.
    let inactif_debut = lire_mesure(addr).await;
    tokio::time::sleep(Duration::from_secs(secs_inactif)).await;
    let inactif_fin = lire_mesure(addr).await;
    let octets_inactif = stats.octets.load(Relaxed) - octets_snapshot_recus;

    // Diffusion : `hz` événements par seconde pendant `secs` secondes à toutes les connexions.
    let diff_debut = lire_mesure(addr).await;
    let msgs_avant = stats.messages.load(Relaxed);
    let octets_avant = stats.octets.load(Relaxed);
    let reponse = http(addr, "POST", &format!("/diffuser?hz={hz}&secs={secs}")).await;
    let attendus = n as u64 * hz * secs;
    let delai = Instant::now();
    while stats.messages.load(Relaxed) - msgs_avant < attendus && delai.elapsed() < Duration::from_secs(10) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let diff_fin = lire_mesure(addr).await;
    let livres = stats.messages.load(Relaxed) - msgs_avant;
    let octets_diffusion = stats.octets.load(Relaxed) - octets_avant;

    // Fermeture : les ressources reviennent-elles ?
    for t in &taches {
        t.abort();
    }
    tokio::time::sleep(Duration::from_secs(3)).await;
    let ferme = lire_mesure(addr).await;

    let cpu = |a: &Map<String, serde_json::Value>, b: &Map<String, serde_json::Value>| val(b, "cpu_ms").zip(val(a, "cpu_ms")).map(|(b, a)| b.saturating_sub(a));
    println!("\n### {} · N = {n} · snapshot {octets} o · {hz} événements/s pendant {secs} s", mode.nom());
    println!("- connexions établies et snapshot reçu : {recus}/{n} en {:.2} s ; erreurs client : {} ; `ulimit -n` du serveur (souple/dur) : {} ; réponse de diffusion : `{}`", temps_connexion.as_secs_f64(), stats.erreurs.load(Relaxed), avant.get("nofile").and_then(|v| v.as_str()).unwrap_or("?"), String::from_utf8_lossy(&reponse));
    println!("- connexions comptées par le serveur : {} ; descripteurs ouverts : {} ; fils : {}", kio(&apres, "conns"), kio(&apres, "fds"), kio(&apres, "Threads"));
    println!("\n| phase (mesurée par le serveur sur lui-même, Kio) | RssAnon | VmRSS | VmHWM (pic) | cgroup `anon` | cgroup `kernel` | cgroup `slab` | cgroup `sock` | cgroup `memory.current` |");
    println!("|---|---|---|---|---|---|---|---|---|");
    let phases: Vec<(String, &Map<String, serde_json::Value>)> = vec![
        ("avant (0 connexion)".into(), &avant),
        (format!("avec {n} connexions inactives"), &apres),
        ("après la diffusion".into(), &diff_fin),
        ("après fermeture".into(), &ferme),
    ];
    for (nom, m) in phases {
        println!("| {nom} | {} | {} | {} | {} | {} | {} | {} | {} |", kio(m, "RssAnon"), kio(m, "VmRSS"), kio(m, "VmHWM"), kio(m, "cg_anon"), kio(m, "cg_kernel"), kio(m, "cg_slab"), kio(m, "cg_sock"), kio(m, "cg_current"));
    }
    let d = |cle: &str| val(&apres, cle).zip(val(&avant, cle)).map(|(a, b)| a as i64 - b as i64);
    let par = |x: Option<i64>| x.map_or("—".into(), |x| format!("{:.2}", x as f64 / n as f64));
    println!("\nPar connexion (avec {n} − avant) / {n} : RssAnon **{} Kio**, VmRSS {} Kio, cgroup `anon` {} Kio, `kernel` {} Kio, `slab` {} Kio, `sock` {} Kio, `memory.current` {} Kio.", par(d("RssAnon")), par(d("VmRSS")), par(d("cg_anon")), par(d("cg_kernel")), par(d("cg_slab")), par(d("cg_sock")), par(d("cg_current")));
    println!("\nCPU du serveur (utime+stime) :");
    println!("- {n} connexions inactives pendant {secs_inactif} s : {} ms ; octets reçus par les clients dans l'intervalle : {octets_inactif} (maintiens SSE)", cpu(&inactif_debut, &inactif_fin).map_or("—".into(), |v| v.to_string()));
    println!("- diffusion de {} événements ({hz}/s × {secs} s) à {n} connexions : **{} ms** ; {livres}/{attendus} livraisons reçues ; {octets_diffusion} o reçus par les clients ({:.1} o par livraison, cadrage compris)", hz * secs, cpu(&diff_debut, &diff_fin).map_or("—".into(), |v| v.to_string()), octets_diffusion as f64 / livres.max(1) as f64);
    if let Some(c) = cpu(&diff_debut, &diff_fin) {
        println!("- soit {:.1} µs de CPU serveur par livraison et {:.1} % d'un cœur pendant les {secs} s", c as f64 * 1000.0 / livres.max(1) as f64, c as f64 / (secs as f64 * 10.0));
    }
    let resultat = json!({
        "mode": mode.nom(), "n": n, "snapshot": octets, "hz": hz, "secs": secs, "livres": livres, "attendus": attendus,
        "avant": avant, "apres": apres, "diffusion": diff_fin, "ferme": ferme,
        "cpu_inactif_ms": cpu(&inactif_debut, &inactif_fin), "cpu_diffusion_ms": cpu(&diff_debut, &diff_fin), "secs_inactif": secs_inactif,
        "temps_connexion_s": temps_connexion.as_secs_f64(), "octets_diffusion": octets_diffusion,
    });
    println!("RESULTAT {resultat}");
}

// =============================================================================================
// Synthèse
// =============================================================================================

fn resume() {
    use std::io::BufRead;
    let mut lignes: Vec<serde_json::Value> = Vec::new();
    for l in std::io::stdin().lock().lines().map_while(Result::ok) {
        if let Some(j) = l.strip_prefix("RESULTAT ") {
            if let Ok(v) = serde_json::from_str(j) {
                lignes.push(v);
            }
        }
    }
    // (mode, snapshot) -> n -> résultat
    let mut groupes: BTreeMap<(String, u64), BTreeMap<u64, serde_json::Value>> = BTreeMap::new();
    for v in lignes {
        groupes.entry((v["mode"].as_str().unwrap().to_owned(), v["snapshot"].as_u64().unwrap())).or_default().insert(v["n"].as_u64().unwrap(), v);
    }
    println!("\n## Synthèse : coût par connexion (processus serveur)\n");
    println!("| mode | snapshot | N | RssAnon avant → avec (Kio) | **Δ RssAnon / connexion (Kio)** (macOS : VmRSS) | Δ VmRSS / connexion (Kio) | VmHWM après connexion (Kio) | Δ `memory.current` du cgroup / connexion (Kio) | CPU inactif (ms/s) | CPU diffusion (µs/livraison) | livré |");
    println!("|---|---|---|---|---|---|---|---|---|---|---|");
    let g = |v: &serde_json::Value, p: &str, c: &str| v[p][c].as_u64();
    for ((mode, snap), par_n) in &groupes {
        for (n, v) in par_n {
            let delta = |c: &str| {
                let lire = |p: &str| g(v, p, c).or_else(|| if c == "RssAnon" { g(v, p, "VmRSS") } else { None });
                lire("apres").zip(lire("avant")).map(|(a, b)| (a as f64 - b as f64) / *n as f64)
            };
            let f = |x: Option<f64>| x.map_or("—".into(), |x| format!("{x:.2}"));
            let inactif = v["cpu_inactif_ms"].as_u64().map(|c| c as f64 / v["secs_inactif"].as_u64().unwrap_or(1) as f64);
            let diffusion = v["cpu_diffusion_ms"].as_u64().map(|c| c as f64 * 1000.0 / v["livres"].as_u64().unwrap_or(1).max(1) as f64);
            println!(
                "| {mode} | {snap} o | {n} | {} → {} | **{}** | {} | {} | {} | {} | {} | {}/{} |",
                g(v, "avant", "RssAnon").map_or("—".into(), |x| x.to_string()),
                g(v, "apres", "RssAnon").map_or("—".into(), |x| x.to_string()),
                f(delta("RssAnon")),
                f(delta("VmRSS")),
                g(v, "apres", "VmHWM").map_or("—".into(), |x| x.to_string()),
                f(delta("cg_current")),
                f(inactif),
                f(diffusion),
                v["livres"], v["attendus"]
            );
        }
    }
    println!("\n### Pente entre deux paliers (RssAnon, VmRSS, memory.current), Kio par connexion = (M(N₂) − M(N₁)) / (N₂ − N₁)\n");
    println!("| mode | snapshot | N₁ → N₂ | pente RssAnon | pente VmRSS | pente `memory.current` |");
    println!("|---|---|---|---|---|---|");
    for ((mode, snap), par_n) in &groupes {
        // Du palier 64 au plus grand palier mesuré (1 000, ou moins si le conteneur a été tué par l'OOM).
        let Some(a) = par_n.get(&64) else { continue };
        let Some((n2, b)) = par_n.iter().rev().find(|(n, v)| **n > 64 && (g(v, "apres", "RssAnon").or_else(|| g(v, "apres", "VmRSS"))).is_some()) else { continue };
        let pente = |p: &str, c: &str| {
            let lire = |v: &serde_json::Value| g(v, p, c).or_else(|| if c == "RssAnon" { g(v, p, "VmRSS") } else { None });
            lire(b).zip(lire(a)).map(|(y, x)| (y as f64 - x as f64) / (*n2 as f64 - 64.0))
        };
        let f = |x: Option<f64>| x.map_or("—".into(), |x| format!("{x:.2} Kio"));
        println!("| {mode} | {snap} o | 64 → {n2} | **{}** | {} | {} |", f(pente("apres", "RssAnon")), f(pente("apres", "VmRSS")), f(pente("apres", "cg_current")));
    }
}

// =============================================================================================

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let nombre = |i: usize, defaut: u64| args.get(i).and_then(|v| v.parse::<u64>().ok()).unwrap_or(defaut);
    match args.first().map(String::as_str) {
        Some("serveur") => {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(serveur(Mode::parse(&args[1]), nombre(2, 8800) as u16, nombre(3, 4096) as usize));
        }
        Some("clients") => {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(clients(Mode::parse(&args[1]), nombre(2, 8800) as u16, nombre(3, 1) as usize, nombre(4, 4096) as usize, nombre(5, 10), nombre(6, 10), nombre(7, 16)));
        }
        Some("tout") => {
            // Serveur en processus fils (même binaire), clients dans ce processus-ci.
            let mode = Mode::parse(&args[1]);
            let (n, octets) = (nombre(2, 1) as usize, nombre(3, 4096) as usize);
            let port = 20_000 + (std::process::id() % 20_000) as u16;
            struct Fils(std::process::Child);
            impl Drop for Fils {
                fn drop(&mut self) {
                    let _ = self.0.kill();
                    let _ = self.0.wait();
                }
            }
            let _fils = Fils(
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["serveur", mode.nom(), &port.to_string(), &octets.to_string()])
                    .spawn()
                    .unwrap(),
            );
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(clients(mode, port, n, octets, nombre(4, 10), nombre(5, 10), nombre(6, 16)));
        }
        Some("resume") => resume(),
        _ => eprintln!("usage : connexions serveur|clients|tout|resume …"),
    }
}
`````

Code complet — `preuves-v5/src/bin/etat_vif.rs` :

`````rust
//! Copie de `preuves-v2/src/bin/etat_complet.rs` (V4) réduite à l'ÉTAT VIF : modèle du hub, étage vif, santé,
//! automatismes compilés, baux, agrégats, voies, tampon du bus. Les lignes d'historique (24 h, 7 j, 30 j) sont
//! retirées. Octets demandés (allocateur qui compte) puis mémoire réelle du processus (RSS, mémoire anonyme du
//! cgroup), pour ×1, ×3 et ×10 « deux maisons ». Chaque échelle tourne dans un processus neuf : la RSS ne garde
//! aucune trace de l'échelle précédente.

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::{BinaryHeap, HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::Duration;

use moli_core::{Access, Device, Kind, PointId, PointSpec, Semantic, Value};
use moli_runtime::{BoxFuture, CommandRequest, Driver, DriverCtx, Hub, HubOptions, spawn_driver};
use tokio_util::sync::CancellationToken;

struct Compte;
static OCTETS: AtomicIsize = AtomicIsize::new(0);
static BLOCS: AtomicIsize = AtomicIsize::new(0);
unsafe impl GlobalAlloc for Compte {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            OCTETS.fetch_add(l.size() as isize, Ordering::Relaxed);
            BLOCS.fetch_add(1, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) };
        OCTETS.fetch_sub(l.size() as isize, Ordering::Relaxed);
        BLOCS.fetch_sub(1, Ordering::Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, l, n) };
        if !q.is_null() {
            OCTETS.fetch_add(n as isize - l.size() as isize, Ordering::Relaxed);
        }
        q
    }
}
#[global_allocator]
static A: Compte = Compte;

fn tas() -> (isize, isize) {
    (OCTETS.load(Ordering::Relaxed), BLOCS.load(Ordering::Relaxed))
}
/// Octets demandés + 16 o d'en-tête par bloc.
fn depuis(avant: (isize, isize)) -> f64 {
    let a = tas();
    ((a.0 - avant.0) + 16 * (a.1 - avant.1)) as f64
}


/// Mémoire réelle du processus, quand le système la donne (Linux) : RSS, mémoire anonyme du processus (`RssAnon`) et du
/// cgroup (la mesure de PERFORMANCE.md), plus `memory.current` du cgroup (tout ce que le noyau charge au conteneur).
#[derive(Clone, Copy, Default)]
struct Reel {
    rss: Option<u64>,
    anon: Option<u64>,
    cg_anon: Option<u64>,
    cg_courant: Option<u64>,
}

fn memoire_reelle() -> Reel {
    let statut = |cle: &str| {
        std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|s| s.lines().find(|l| l.starts_with(cle)).and_then(|l| l.split_whitespace().nth(1).and_then(|v| v.parse::<u64>().ok())))
    };
    let cg_anon = std::fs::read_to_string("/sys/fs/cgroup/memory.stat")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("anon ")).and_then(|l| l.split_whitespace().nth(1).and_then(|v| v.parse::<u64>().ok())))
        .map(|v| v / 1024);
    let cg_courant = std::fs::read_to_string("/sys/fs/cgroup/memory.current").ok().and_then(|s| s.trim().parse::<u64>().ok()).map(|v| v / 1024);
    // macOS : pas de /proc, on prend le RSS de `ps` (seul chiffre disponible).
    let rss = statut("VmRSS:").or_else(|| {
        let sortie = std::process::Command::new("ps").args(["-o", "rss=", "-p", &std::process::id().to_string()]).output().ok()?;
        String::from_utf8_lossy(&sortie.stdout).trim().parse::<u64>().ok()
    });
    Reel { rss, anon: statut("RssAnon:"), cg_anon, cg_courant }
}

impl Reel {
    fn texte(&self) -> String {
        let f = |v: Option<u64>| v.map_or("—".to_owned(), |v| format!("{v} Kio"));
        if self.rss.is_none() {
            return "mémoire réelle non disponible sur ce système".into();
        }
        format!("RSS {}, RssAnon {}, anonyme du cgroup {}, memory.current du cgroup {}", f(self.rss), f(self.anon), f(self.cg_anon), f(self.cg_courant))
    }
    /// Écart (Kio) par rapport à un état antérieur.
    fn ecart(&self, avant: &Reel) -> String {
        let d = |a: Option<u64>, b: Option<u64>| a.zip(b).map_or("—".to_owned(), |(a, b)| format!("{:+} Kio", a as i64 - b as i64));
        if self.rss.is_none() {
            return "mémoire réelle non disponible sur ce système".into();
        }
        format!("RSS {}, RssAnon {}, anonyme du cgroup {}, memory.current du cgroup {}", d(self.rss, avant.rss), d(self.anon, avant.anon), d(self.cg_anon, avant.cg_anon), d(self.cg_courant, avant.cg_courant))
    }
}

fn kio(o: f64) -> String {
    if o >= 1_048_576.0 { format!("{:.2} Mio", o / 1_048_576.0) } else { format!("{:.0} Kio", o / 1024.0) }
}

// ---- le modèle du hub (comme aujourd'hui) ---------------------------------------

struct Maison {
    appareils: usize,
    valeurs: usize,
}
fn valeur(i: usize) -> (Kind, Value) {
    match i % 200 {
        0..37 => (Kind::Numeric { min: None, max: None, step: None }, Value::Float(20.0 + (i % 17) as f64 * 0.5)),
        37..77 => (Kind::Binary, Value::Bool(i % 3 == 0)),
        77..105 => (Kind::Text, Value::Text(["idle", "heat_cool", "playing", "auto"][i % 4].into())),
        _ => (Kind::Text, Value::Null),
    }
}
impl Driver for Maison {
    fn kind(&self) -> &'static str {
        "maison"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let instance = ctx.instance().clone();
            let cles = ["temperature", "humidity", "battery", "linkquality", "power", "state", "illuminance", "mode"];
            let mut n = 0;
            for d in 0..self.appareils {
                let par = self.valeurs / self.appareils + usize::from(d < self.valeurs % self.appareils);
                let id = ctx.device_id(&format!("0x00158d00{d:08x}"));
                let points: Vec<PointSpec> = (0..par)
                    .map(|k| PointSpec {
                        key: format!("{}_{k}", cles[(d + k) % cles.len()]).into(),
                        label: cles[(d + k) % cles.len()].into(),
                        kind: valeur(n + k).0,
                        access: Access { read: true, write: k == 0 },
                        unit: None,
                        semantic: Semantic::Other,
                    })
                    .collect();
                ctx.upsert_device(Device {
                    id: id.clone(),
                    instance: instance.clone(),
                    native_name: format!("Appareil fictif {d}").into(),
                    manufacturer: Some("Fabricant".into()),
                    model: Some("Modèle-X1".into()),
                    description: None,
                    native_room: Some("Pièce".into()),
                    members: vec![],
                    points: points.clone(),
                });
                for p in &points {
                    ctx.set_state(&id, &p.key, valeur(n).1);
                    n += 1;
                }
            }
            ctx.ready();
            std::future::pending::<()>().await;
            Ok(())
        })
    }
}

// ---- ce qu'on ajoute pour être très réactif -------------------------------------

/// L'étage « vif » de la proposition stockage V2 : valeur, valeur précédente, trois
/// instants (observé, reçu, changé) et qualité.
#[allow(dead_code)]
struct PointVif {
    valeur: Value,
    precedente: Value,
    observe: u64,
    recu: u64,
    change: u64,
    qualite: u8,
}

#[allow(dead_code)]
struct SanteAppareil {
    vu: u64,
    echecs: u16,
    disjoncte_jusqua: u64,
    en_file: u8,
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
enum Op {
    Ge,
    Gt,
}

#[allow(dead_code)]
struct Plan {
    nom: String,
    regles: Vec<(u32, Op, f64)>,
    actions: Vec<(u32, Value)>,
}

#[allow(dead_code)]
struct Bail {
    point: u32,
    pose: Value,
    retour: Value,
    echeance: u64,
}

#[allow(dead_code)]
struct Execution {
    automatisme: u32,
    noeud: u16,
    depuis: u64,
    variables: Vec<(u16, Value)>,
}

/// Agrégat glissant : relevés de la fenêtre (décalage u32, valeur f32) et sommes tenues.
#[allow(dead_code)]
struct Agregat {
    fenetre: VecDeque<(u32, f32)>,
    somme: f64,
    min: f32,
    max: f32,
}

const VINGTILES: [f64; 20] = [
    0.28, 0.3, 0.3, 0.3, 0.3, 0.3, 0.3, 0.33, 0.48, 0.63, 0.9, 1.14, 1.41, 2.16, 3.57, 5.73,
    9.22, 21.92, 140.88, 1990.22,
];
const SERIES: usize = 1_487;
fn taux(s: usize) -> f64 {
    VINGTILES[(s % SERIES) * 20 / SERIES]
}

fn budget(echelle: usize) {
    let au_depart = memoire_reelle();
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    rt.block_on(async {
        let (appareils, valeurs) = (496 * echelle, 2_264 * echelle);
        let automatismes = 400 * echelle;
        let mut lignes: Vec<(&str, f64)> = Vec::new();

        let hub = Hub::new(HubOptions::default()).unwrap();
        let cancel = CancellationToken::new();
        let avant = tas();
        spawn_driver(&hub, "maison".into(), Arc::new(Maison { appareils, valeurs }), cancel.clone());
        while hub.stats().points < valeurs {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        lignes.push(("Modèle du hub (appareils, définitions, valeurs : ce qui existe)", depuis(avant)));

        let avant = tas();
        let snap = hub.snapshot();
        let mut index: HashMap<PointId, u32> = HashMap::with_capacity(valeurs);
        let mut vifs: Vec<PointVif> = Vec::with_capacity(valeurs);
        for d in &snap.devices {
            for (cle, s) in &d.state {
                index.insert(PointId::new(&d.device.id, cle), vifs.len() as u32);
                vifs.push(PointVif { valeur: s.value.clone(), precedente: Value::Null, observe: s.ts, recu: s.ts, change: s.ts, qualite: 0 });
            }
        }
        drop(snap);
        lignes.push(("Étage vif : précédente, 3 instants, qualité + index des points", depuis(avant)));

        let avant = tas();
        let sante: Vec<SanteAppareil> = (0..appareils).map(|_| SanteAppareil { vu: 0, echecs: 0, disjoncte_jusqua: 0, en_file: 0 }).collect();
        lignes.push(("Santé par appareil (vu le, échecs, disjoncteur)", depuis(avant)));

        let avant = tas();
        let plans: Vec<Plan> = (0..automatismes)
            .map(|k| Plan {
                nom: format!("Automatisme {k}"),
                regles: (0..9).map(|j| (((k * 7 + j * 13) % valeurs) as u32, if j % 2 == 0 { Op::Ge } else { Op::Gt }, 0.0)).collect(),
                actions: vec![((k % valeurs) as u32, Value::Bool(true)), ((k % valeurs) as u32, Value::Bool(false))],
            })
            .collect();
        let mut par_point: Vec<Vec<u32>> = vec![Vec::new(); valeurs];
        for k in 0..automatismes {
            par_point[(k * 31) % valeurs].push(k as u32);
        }
        lignes.push(("Automatismes compilés (9 règles, 2 actions) + index par point", depuis(avant)));

        let avant = tas();
        let mut echeances = BinaryHeap::new();
        let baux: Vec<Bail> = (0..1_000 * echelle)
            .map(|i| {
                echeances.push(std::cmp::Reverse((1_792_000_000_000u64 + i as u64 * 1_000, i as u32)));
                Bail { point: (i % valeurs) as u32, pose: Value::Bool(true), retour: Value::Bool(false), echeance: 0 }
            })
            .collect();
        let executions: Vec<Execution> = (0..automatismes)
            .map(|k| Execution { automatisme: k as u32, noeud: 3, depuis: 0, variables: (0..4).map(|v| (v, Value::Float(1.0))).collect() })
            .collect();
        lignes.push(("1 000 baux, échéancier et une exécution en cours par automatisme", depuis(avant)));

        let avant = tas();
        // Agrégats 1 h et 24 h sur les points numériques lus par les règles (un sur cinq).
        let mut agregats: Vec<Agregat> = Vec::new();
        let lus = valeurs / 5;
        for s in 0..lus {
            for heures in [1.0, 24.0] {
                // Points répartis sur toute la distribution réelle, calmes comme bavards.
                let n = (taux(s * SERIES / lus) * heures / 24.0).round() as usize;
                let mut f = VecDeque::with_capacity(n);
                for i in 0..n {
                    f.push_back((i as u32, i as f32));
                }
                agregats.push(Agregat { fenetre: f, somme: 0.0, min: 0.0, max: 0.0 });
            }
        }
        lignes.push(("Agrégats 1 h et 24 h des points lus par les règles", depuis(avant)));

        // Voies paresseuses : un seul répartiteur, une file créée seulement quand un ordre attend, au plus un ordre
        // en vol par appareil. Au repos : une table vide. (La variante en tâches permanentes est mesurée plus bas,
        // APRÈS la mémoire réelle, pour qu'elle ne la fausse pas.)
        let avant = tas();
        let attente: HashMap<u32, VecDeque<CommandRequest>> = HashMap::with_capacity(64);
        let en_vol: Vec<bool> = vec![false; appareils];
        lignes.push(("Voies paresseuses (répartiteur unique, files à la demande), au repos", depuis(avant)));
        std::hint::black_box((&attente, &en_vol));

        let bus = 1_024.0 * std::mem::size_of::<moli_core::Event>() as f64;
        lignes.push(("Tampon du bus : 1 024 événements (charges partagées en Arc)", bus));

        let indispensable: f64 = lignes.iter().map(|(_, o)| o).sum();
        let reel_vif = memoire_reelle();

        // Aparté (mesure de la V4) : les voies en tâches permanentes, file de 8 par appareil.
        let avant = tas();
        let mut voies = Vec::new();
        let mut taches = tokio::task::JoinSet::new();
        for _ in 0..appareils {
            let (tx, mut rx) = tokio::sync::mpsc::channel::<CommandRequest>(8);
            voies.push(tx);
            taches.spawn(async move { while let Some(c) = rx.recv().await { c.reply(Ok(())); } });
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        let voies_taches = depuis(avant);
        let reel_voies = memoire_reelle();

        println!("\n## ×{echelle} : {appareils} appareils, {valeurs} valeurs, {automatismes} automatismes");
        println!("Octets demandés à l'allocateur (octets + 16 o d'en-tête par bloc) :");
        for (nom, o) in &lignes {
            println!("- {nom} : {}", kio(*o));
        }
        println!("- **Sous-total « tout l'état vif »** : {} ({:.0} octets)", kio(indispensable), indispensable);
        println!("Mémoire réelle du processus (une échelle par processus neuf) :");
        println!("- au démarrage du processus (avant le runtime et le hub) : {}", au_depart.texte());
        println!("- avec tout l'état vif : {}", reel_vif.texte());
        println!("- **écart « état vif » − démarrage** : {}", reel_vif.ecart(&au_depart));
        if let (Some(ra), Some(rb)) = (reel_vif.anon, au_depart.anon) {
            println!("- rapport RssAnon ajoutée / octets demandés : {:.2}", (ra as f64 - rb as f64) * 1024.0 / indispensable);
        }
        println!("- aparté, mesure de la V4 : voies en tâches permanentes (file de 8) : {} pour {appareils} appareils, soit {:.1} Kio chacune ; CommandRequest {} o ; mémoire réelle avec elles en plus : {}", kio(voies_taches), voies_taches / appareils as f64 / 1024.0, std::mem::size_of::<CommandRequest>(), reel_voies.texte());
        std::hint::black_box((index, vifs, sante, plans, par_point, baux, executions, echeances, agregats, voies));
        cancel.cancel();
        taches.abort_all();
    });
}

fn main() {
    // Une échelle en argument : on la mesure dans CE processus. Sans argument : un processus neuf par échelle,
    // pour que la RSS d'une échelle ne garde pas la trace (allocateur) de la précédente.
    if let Some(e) = std::env::args().nth(1).and_then(|v| v.parse::<usize>().ok()) {
        budget(e);
        return;
    }
    println!("# État vif seul (sans historique) : mémoire demandée et mémoire réelle, ×1, ×3 et ×10 « deux maisons »");
    println!("- plateforme : {} / {} ; tailles : Value {} o, PointVif {} o, Event {} o", std::env::consts::OS, std::env::consts::ARCH, std::mem::size_of::<Value>(), std::mem::size_of::<PointVif>(), std::mem::size_of::<moli_core::Event>());
    println!("- « deux maisons » ×1 = 496 appareils, 2 264 valeurs, 400 automatismes ; ×3 = 1 488 / 6 792 / 1 200 ; ×10 = 4 960 / 22 640 / 4 000 (constructeur de la V4)");
    println!("- mémoire réelle au démarrage du banc, avant toute charge : {}", memoire_reelle().texte());
    for e in [1, 3, 10] {
        let _ = std::process::Command::new(std::env::current_exe().unwrap()).arg(e.to_string()).status();
    }
}
`````

Code complet — `preuves-v5/src/bin/historique.rs` :

`````rust
//! Banc 3 : tailles des réponses `GET /api/history` (vraies `moli_history::Series`).
//! Séries : 300 buckets (défaut), 2 000 buckets (maximum), `raw` de 1 440 points (24 h à la minute),
//! puis `raw` hypothétiques de 10 080 (7 j), 43 200 (30 j) et 525 600 points (1 an à la minute) :
//! le serveur actuel ne renvoie `raw` que s'il y a au plus `points` ≤ 2 000 changements.

use std::net::SocketAddr;

use axum::Json;
use axum::Router;
use axum::routing::get;
use moli_core::Value;
use moli_history::{Bucket, Series};
use preuves_v5::{avec_pile_moli, brotli, chronometrer, dechunk, decouper, echange, gzip, kio, o};

const FIN: u64 = 1_792_675_800_000; // « maintenant » fictif (ms)
const POINT: &str = "z2m:0x00124b00aaaa0001/temperature";

/// Générateur pseudo-aléatoire déterministe (LCG), pour des bruits reproductibles.
struct Lcg(u64);
impl Lcg {
    fn suivant(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 33) as f64) / ((1u64 << 31) as f64) // [0, 1)
    }
}

/// Température réaliste : cycle jour/nuit (±1,2 °C), oscillation du chauffage, bruit capteur ;
/// arrondie à `decimales` (Zigbee2MQTT publie 2 décimales par défaut).
fn temperature(i: usize, bruit: &mut Lcg, decimales: i32) -> f64 {
    let jour = (i as f64 / 1440.0) * std::f64::consts::TAU;
    let chauffage = (i as f64 / 97.0) * std::f64::consts::TAU;
    let t = 21.0 + 1.2 * jour.sin() + 0.2 * chauffage.sin() + (bruit.suivant() - 0.5) * 0.08;
    let f = 10f64.powi(decimales);
    (t * f).round() / f
}

/// `n` changements espacés d'`intervalle_ms` (avec une petite gigue), jusqu'à `FIN`.
fn serie_raw(n: usize, intervalle_ms: u64, decimales: i32) -> Series {
    let mut bruit = Lcg(42);
    let debut = FIN - n as u64 * intervalle_ms;
    let raw: Vec<(u64, Value)> = (0..n)
        .map(|i| (debut + i as u64 * intervalle_ms + (bruit.suivant() * 900.0) as u64, Value::Float(temperature(i, &mut bruit, decimales))))
        .collect();
    Series {
        point: POINT.into(),
        from: debut,
        to: FIN,
        total: n as u64,
        before: Some((debut - intervalle_ms, Value::Float(21.0))),
        first: raw.first().cloned(),
        last: raw.last().cloned(),
        raw,
        buckets: vec![],
        truncated: false,
    }
}

/// `n` buckets sur 24 h, `par_bucket` relevés bruts chacun (moyenne calculée comme `AVG` de SQLite :
/// somme de doubles / nombre, donc 16-17 chiffres significatifs).
fn serie_buckets(n: usize, par_bucket: usize, arrondi: bool) -> Series {
    let mut bruit = Lcg(7);
    let fenetre = 24 * 3_600_000u64;
    let largeur = fenetre / n as u64;
    let debut = FIN - fenetre;
    let mut buckets = Vec::with_capacity(n);
    let mut i = 0usize;
    for b in 0..n {
        let k = par_bucket + (bruit.suivant() * 3.0) as usize; // un peu de variation du compte
        let vals: Vec<f64> = (0..k).map(|j| temperature(i + j, &mut bruit, 2)).collect();
        i += k;
        let somme: f64 = vals.iter().sum();
        buckets.push(Bucket {
            ts: debut + b as u64 * largeur + (bruit.suivant() * 5000.0) as u64,
            avg: if arrondi { (somme / k as f64 * 100.0).round() / 100.0 } else { somme / k as f64 },
            min: vals.iter().cloned().fold(f64::MAX, f64::min),
            max: vals.iter().cloned().fold(f64::MIN, f64::max),
            count: k as u64,
        });
    }
    let total = buckets.iter().map(|b| b.count).sum();
    Series {
        point: POINT.into(),
        from: debut,
        to: FIN,
        total,
        before: Some((debut - 60_000, Value::Float(21.0))),
        first: Some((buckets[0].ts, Value::Float(buckets[0].min))),
        last: Some((buckets[n - 1].ts + largeur - 1, Value::Float(buckets[n - 1].max))),
        raw: vec![],
        buckets,
        truncated: false,
    }
}

struct Ligne {
    nom: String,
    elements: usize,
    brut: usize,
    gz: usize,
    br4: usize,
    br11: Option<usize>,
    ser_ms: f64,
    gz_ms: f64,
    br4_ms: f64,
    br11_ms: Option<f64>,
}

fn mesurer(nom: &str, elements: usize, s: &Series) -> Ligne {
    let json = serde_json::to_vec(s).unwrap();
    let passes = match elements {
        0..=3_000 => 20,
        3_001..=50_000 => 5,
        _ => 2,
    };
    let (ser_ms, _, _) = chronometrer(passes, || serde_json::to_vec(s).unwrap());
    let gz = gzip(&json);
    let (gz_ms, _, _) = chronometrer(passes, || gzip(&json));
    let br4 = brotli(&json, 4);
    let (br4_ms, _, _) = chronometrer(passes.min(5), || brotli(&json, 4));
    // brotli 11 : ~0,5-1 Mo/s ; au-delà de 1,5 Mo de JSON on le laisse de côté (secondes par passe).
    let (br11, br11_ms) = if json.len() <= 1_500_000 {
        let b = brotli(&json, 11);
        let (t, _, _) = chronometrer(passes.min(3), || brotli(&json, 11));
        (Some(b.len()), Some(t))
    } else {
        (None, None)
    };
    Ligne { nom: nom.into(), elements, brut: json.len(), gz: gz.len(), br4: br4.len(), br11, ser_ms, gz_ms, br4_ms, br11_ms }
}

async fn controle_reel(addr: SocketAddr, chemin: &str, accept: &str) -> (usize, String) {
    let requete = format!("GET {chemin} HTTP/1.1\r\nHost: moli.local:8790\r\nAccept-Encoding: {accept}\r\nConnection: close\r\n\r\n");
    let rep = echange(addr, requete.as_bytes()).await;
    let (entetes, corps) = decouper(&rep);
    let taille_corps = if entetes.to_ascii_lowercase().contains("transfer-encoding: chunked") { dechunk(corps).1.iter().sum() } else { corps.len() };
    (taille_corps, entetes)
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    println!("# Banc `historique` : réponses `GET /api/history` (vraies `moli_history::Series`)");
    println!("- plateforme : {} / {}", std::env::consts::OS, std::env::consts::ARCH);
    println!("- JSON compact (`serde_json`, ce que `axum::Json` écrit) ; gzip = flate2 niveau 6 ; brotli = crate `brotli` 9 (qualité 4 = défaut de `tower-http` 0.7 d'après `compression/body.rs`, et 11 = maximum ; fenêtre 22) ; temps = médiane (20 passes jusqu'à 3 000 éléments, 5 passes jusqu'à 50 000, 2 au-delà)");
    println!("- valeurs : température ≈ 21 °C (cycle jour/nuit ±1,2 °C, bruit capteur), 2 décimales ; ts en ms (13 chiffres) ; `avg` des buckets = somme/compte en f64 (16-17 chiffres significatifs, comme `AVG()` de SQLite)");
    println!("- le serveur actuel (`store.rs`) ne renvoie `raw` que si `total ≤ points ≤ 2 000` ; les `raw` de 10 080 / 43 200 / 525 600 sont HYPOTHÉTIQUES (comparaison)\n");

    let cas: Vec<(String, usize, Series)> = vec![
        ("300 buckets (défaut, 24 h)".into(), 300, serie_buckets(300, 10, false)),
        ("2 000 buckets (maximum, 24 h)".into(), 2_000, serie_buckets(2_000, 8, false)),
        ("300 buckets, `avg` arrondi à 0,01 (hypothétique)".into(), 300, serie_buckets(300, 10, true)),
        ("2 000 buckets, `avg` arrondi à 0,01 (hypothétique)".into(), 2_000, serie_buckets(2_000, 8, true)),
        ("raw 1 440 (24 h à la minute)".into(), 1_440, serie_raw(1_440, 60_000, 2)),
        ("raw 1 440, précision 0,1 °C".into(), 1_440, serie_raw(1_440, 60_000, 1)),
        ("raw 10 080 (7 j) hypothétique".into(), 10_080, serie_raw(10_080, 60_000, 2)),
        ("raw 43 200 (30 j) hypothétique".into(), 43_200, serie_raw(43_200, 60_000, 2)),
        ("raw 525 600 (1 an) hypothétique".into(), 525_600, serie_raw(525_600, 60_000, 2)),
    ];
    let lignes: Vec<Ligne> = cas.iter().map(|(nom, n, s)| mesurer(nom, *n, s)).collect();

    println!("## Tailles de la réponse (corps JSON)\n");
    println!("| série | éléments | JSON brut | octets / élément | gzip 6 | brotli 4 (tower-http) | brotli 11 |");
    println!("|---|---|---|---|---|---|---|");
    for l in &lignes {
        println!(
            "| {} | {} | {} o ({}) | {:.1} | {} o ({:.0} %) | {} o ({:.0} %) | {} |",
            l.nom,
            o(l.elements),
            o(l.brut),
            kio(l.brut),
            l.brut as f64 / l.elements as f64,
            o(l.gz),
            100.0 * l.gz as f64 / l.brut as f64,
            o(l.br4),
            100.0 * l.br4 as f64 / l.brut as f64,
            l.br11.map_or("—".to_owned(), |b| format!("{} o ({:.0} %)", o(b), 100.0 * b as f64 / l.brut as f64)),
        );
    }
    println!("\n## Temps (ms, médiane)\n");
    println!("| série | sérialisation JSON | gzip 6 | brotli 4 (tower-http) | brotli 11 |");
    println!("|---|---|---|---|---|");
    for l in &lignes {
        println!("| {} | {:.2} | {:.2} | {:.2} | {} |", l.nom, l.ser_ms, l.gz_ms, l.br4_ms, l.br11_ms.map_or("—".to_owned(), |t| format!("{t:.1}")));
    }

    // ---- contrôle avec la vraie pile : tower-http CompressionLayer + en-têtes de Moli ----------------
    println!("\n## Contrôle sur le fil : vrai serveur axum 0.8 + `CompressionLayer` de Moli (octets du corps, hors en-têtes)\n");
    let petit = std::sync::Arc::new(cas[0].2.clone());
    let raw = std::sync::Arc::new(cas[4].2.clone());
    let b2000 = std::sync::Arc::new(cas[1].2.clone());
    let (p1, p2, p3) = (petit.clone(), raw.clone(), b2000.clone());
    let app: Router = avec_pile_moli(
        Router::new()
            .route("/api/history/b300", get(move || { let s = p1.clone(); async move { Json((*s).clone()) } }))
            .route("/api/history/raw", get(move || { let s = p2.clone(); async move { Json((*s).clone()) } }))
            .route("/api/history/b2000", get(move || { let s = p3.clone(); async move { Json((*s).clone()) } })),
    );
    let ecoute = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = ecoute.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(ecoute, app).await.unwrap() });
    println!("| route | Accept-Encoding | corps reçu | content-encoding | transfer |");
    println!("|---|---|---|---|---|");
    let mut exemple_entetes = String::new();
    for (nom, chemin) in [("300 buckets", "/api/history/b300"), ("2 000 buckets", "/api/history/b2000"), ("raw 1 440", "/api/history/raw")] {
        for accept in ["identity", "gzip", "br", "gzip, deflate, br, zstd"] {
            let (taille, entetes) = controle_reel(addr, chemin, accept).await;
            let ligne = |nom: &str| entetes.lines().find(|l| l.to_ascii_lowercase().starts_with(nom)).map(|l| l.split_once(':').unwrap().1.trim().to_owned()).unwrap_or_else(|| "—".into());
            println!("| {nom} | `{accept}` | {} o | {} | {} |", o(taille), ligne("content-encoding"), if entetes.to_ascii_lowercase().contains("transfer-encoding: chunked") { "chunked".to_owned() } else { format!("content-length {}", ligne("content-length")) });
            if exemple_entetes.is_empty() {
                exemple_entetes = entetes;
            }
        }
    }
    println!("\nEn-têtes de réponse (identity, 300 buckets) :\n```\n{}```", exemple_entetes);
    println!("\nLecture : la taille « corps reçu » avec `gzip` doit égaler la colonne gzip 6 (même codec) ; avec `br`, la colonne brotli 4 (tower-http fixe `Level::Precise(4)` pour `CompressionLevel::Default`), à quelques dizaines d'octets près (flux, pas un bloc unique).");
}
`````

Code complet — `preuves-v5/src/bin/instantane.rs` :

`````rust
//! Banc 2 : taille et temps du snapshot SSE (`hub.snapshot()` sérialisé en JSON).
//! Deux formes : « deux maisons » (496 appareils, 2 264 valeurs, constructeur de la V4) et une petite
//! maison (50 appareils, 250 valeurs). Variante « riche » (clés Zigbee2MQTT, unités, bornes,
//! libellés) pour montrer l'écart avec le constructeur minimal de la V4.

use moli_core::{Event, PointId};
use preuves_v5::{brotli, chronometrer, gzip, kio, mediane, o};

const PASSES: usize = 20;

struct Mesure {
    brut: usize,
    gz: usize,
    br4: usize,
    br11: usize,
    snapshot_ms: (f64, f64, f64),
    json_ms: (f64, f64, f64),
    gz_ms: (f64, f64, f64),
    br4_ms: (f64, f64, f64),
    br11_ms: (f64, f64, f64),
    appareils: usize,
    valeurs: usize,
    evenement: (f64, usize, usize), // moyenne JSON, min, max
    evenement_sse: f64,             // moyenne cadrée SSE
    mediane_evt: f64,
    snapshot_sse: usize,
}

async fn mesurer(appareils: usize, valeurs: usize, riche: bool) -> Mesure {
    let (hub, cancel) = preuves_v5::construire(appareils, valeurs, riche).await;
    let snap = hub.snapshot();
    let json = serde_json::to_vec(&snap).unwrap();
    let nb_app = snap.devices.len();
    let nb_val: usize = snap.devices.iter().map(|d| d.state.len()).sum();
    let enveloppe = format!("event: snapshot\ndata: {}\n\n", String::from_utf8_lossy(&json)).len();

    // Événements State moyens : un par valeur de l'état, comme le ferait un pilote.
    let mut tailles = Vec::new();
    for d in &snap.devices {
        for (cle, s) in &d.state {
            let ev = Event::State { point: PointId::new(&d.device.id, cle), value: s.value.clone(), ts: s.ts, source_ts: None };
            tailles.push(serde_json::to_vec(&ev).unwrap().len());
        }
    }
    let somme: usize = tailles.iter().sum();
    let moyenne = somme as f64 / tailles.len() as f64;
    // Cadrage SSE : `event: event\ndata: ` (19 o) + JSON + `\n\n` (2 o).
    let cadre = "event: event\ndata: ".len() + "\n\n".len();
    let med = mediane(tailles.iter().map(|t| *t as f64).collect());

    let snapshot_ms = chronometrer(PASSES, || hub.snapshot());
    let json_ms = chronometrer(PASSES, || serde_json::to_vec(&snap).unwrap());
    let gz = gzip(&json);
    let br4 = brotli(&json, 4);
    let br11 = brotli(&json, 11);
    let gz_ms = chronometrer(PASSES, || gzip(&json));
    let br4_ms = chronometrer(5, || brotli(&json, 4));
    let br11_ms = chronometrer(5, || brotli(&json, 11));
    cancel.cancel();
    Mesure {
        brut: json.len(),
        gz: gz.len(),
        br4: br4.len(),
        br11: br11.len(),
        snapshot_ms,
        json_ms,
        gz_ms,
        br4_ms,
        br11_ms,
        appareils: nb_app,
        valeurs: nb_val,
        evenement: (moyenne, *tailles.iter().min().unwrap(), *tailles.iter().max().unwrap()),
        evenement_sse: moyenne + cadre as f64,
        mediane_evt: med,
        snapshot_sse: enveloppe,
    }
}

fn ms(t: (f64, f64, f64)) -> String {
    format!("{:.2} ms (min {:.2} / max {:.2})", t.0, t.1, t.2)
}

fn main() {
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    println!("# Banc `instantané` : snapshot SSE (`hub.snapshot()` en JSON)");
    println!("- plateforme : {} / {}, {PASSES} passes (médiane) pour les temps sérialisation et gzip, 5 passes pour brotli", std::env::consts::OS, std::env::consts::ARCH);
    println!("- JSON compact (`serde_json::to_vec`), tel que `SseEvent::json_data` l'écrit ; gzip = flate2 niveau 6 ; brotli = crate `brotli` 9, fenêtre 22, qualité 4 (celle que `tower-http` 0.7 utilise par défaut, comme NGINX) et 11 (maximum)");
    println!("- SSE n'est PAS compressé par tower-http (`DefaultPredicate` exclut `text/event-stream`) : les colonnes gzip/brotli sont ce que donnerait une route REST du même JSON\n");
    let formes = [
        ("a. deux maisons, constructeur V4 (496 appareils, 2 264 valeurs)", 496, 2_264, false),
        ("b. petite maison, constructeur V4 (50 appareils, 250 valeurs)", 50, 250, false),
        ("c. deux maisons, forme riche Zigbee2MQTT (≈ 496 appareils, 2 264 valeurs)", 496, 2_264, true),
        ("d. petite maison, forme riche Zigbee2MQTT (50 appareils, 250 valeurs)", 50, 250, true),
    ];
    let mut resultats = Vec::new();
    for (nom, a, v, riche) in formes {
        let m = rt.block_on(mesurer(a, v, riche));
        resultats.push((nom, m));
    }
    println!("## Tailles du JSON du snapshot\n");
    println!("| forme | appareils | valeurs | JSON brut | gzip (6) | brotli 4 (tower-http) | brotli 11 | cadré SSE (`event: snapshot…`) | brut / valeur |");
    println!("|---|---|---|---|---|---|---|---|---|");
    for (nom, m) in &resultats {
        println!(
            "| {nom} | {} | {} | {} o ({}) | {} o ({}, {:.0} %) | {} o ({:.0} %) | {} o ({:.0} %) | {} o | {:.0} o |",
            m.appareils,
            m.valeurs,
            o(m.brut),
            kio(m.brut),
            o(m.gz),
            kio(m.gz),
            100.0 * m.gz as f64 / m.brut as f64,
            o(m.br4),
            100.0 * m.br4 as f64 / m.brut as f64,
            o(m.br11),
            100.0 * m.br11 as f64 / m.brut as f64,
            o(m.snapshot_sse),
            m.brut as f64 / m.valeurs as f64
        );
    }
    println!("\n## Temps (médiane de {PASSES} passes)\n");
    println!("| forme | `hub.snapshot()` (copie du modèle) | sérialisation JSON | gzip 6 | brotli 4 (tower-http) | brotli 11 |");
    println!("|---|---|---|---|---|---|");
    for (nom, m) in &resultats {
        println!("| {nom} | {} | {} | {} | {} | {} |", ms(m.snapshot_ms), ms(m.json_ms), ms(m.gz_ms), ms(m.br4_ms), ms(m.br11_ms));
    }
    println!("\n## Un `Event::State` moyen de ces formes (JSON sérialisé comme `json_data`)\n");
    println!("| forme | JSON moyen | médian | min | max | cadré SSE moyen (`event: event\\ndata: …\\n\\n`) |");
    println!("|---|---|---|---|---|---|");
    for (nom, m) in &resultats {
        println!("| {nom} | {:.1} o | {:.0} o | {} o | {} o | {:.1} o |", m.evenement.0, m.mediane_evt, m.evenement.1, m.evenement.2, m.evenement_sse);
    }
    println!("\nNota : formes a et b = ids `maison:0x00158d00…` et clés `<nom>_<k>` (constructeur V4) ; formes c et d = ids `z2m:0x00124b00…` et clés Zigbee2MQTT réelles.");
}
`````

Code complet — `preuves-v5/src/bin/tailles.rs` :

`````rust
//! Banc 1 : octets exacts sur le fil (application, sans TCP/IP/TLS).
//! MQTT 3.1.1 et 5 (encodés par `rumqttc`), SSE de Moli (vrais `moli_core::Event::State`, vrai serveur
//! axum 0.8 avec la pile de Moli, octets lus sur une socket brute), WebSocket (idem), `POST /api/command`,
//! maintiens de liaison. Les valeurs « mesuré » sont lues sur le fil ; « calculé » = formule vérifiée par assertion.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::extract::ws::{Message, WebSocketUpgrade};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::{Bytes, BytesMut};
use futures::stream::{self};
use moli_core::{Action, DeviceId, Event, InstanceId, JournalEntry, Origin, Outcome, PointId, Value};
use preuves_v5::{avec_pile_moli, o};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::broadcast;

// ---- les quatre messages types -----------------------------------------------------------------

struct Msg {
    nom: &'static str,
    /// Topic d'état publié par Zigbee2MQTT (ou topic d'ordre pour la lampe).
    topic: &'static str,
    json: &'static str,
    ieee: &'static str,
}

const MESSAGES: [Msg; 4] = [
    Msg { nom: "thermomètre", topic: "zigbee2mqtt/thermometre_salon", json: r#"{"temperature":21.5,"humidity":48,"battery":91,"linkquality":96}"#, ieee: "0x00124b00aaaa0001" },
    Msg { nom: "détecteur", topic: "zigbee2mqtt/detecteur_cuisine", json: r#"{"occupancy":true,"battery":100,"linkquality":120}"#, ieee: "0x00124b00aaaa0002" },
    Msg { nom: "prise + mesure", topic: "zigbee2mqtt/prise_1", json: r#"{"state":"ON","power":152.3,"voltage":231.2,"current":0.68,"energy":12.41}"#, ieee: "0x00124b00aaaa0003" },
    Msg { nom: "ordre lampe", topic: "zigbee2mqtt/lampe_1/set", json: r#"{"state":"ON"}"#, ieee: "0x00124b00aaaa0004" },
];

const TS: u64 = 1_792_675_795_123;

/// Ce que fait le pilote Z2M (`catalog.rs::decode`) : un couple (clé, valeur) par attribut ;
/// `state` ON/OFF devient un booléen, les autres scalaires passent par `Value::from_json`.
fn decoder(json: &str) -> Vec<(String, Value)> {
    let serde_json::Value::Object(map) = serde_json::from_str(json).unwrap() else { panic!() };
    map.iter()
        .map(|(k, v)| {
            let valeur = match (k.as_str(), v.as_str()) {
                ("state", Some("ON")) => Value::Bool(true),
                ("state", Some("OFF")) => Value::Bool(false),
                _ => Value::from_json(v),
            };
            (k.clone(), valeur)
        })
        .collect()
}

fn evenements(m: &Msg) -> Vec<Event> {
    let device = DeviceId::new(&InstanceId::from("z2m"), m.ieee);
    decoder(m.json).into_iter().map(|(cle, valeur)| Event::State { point: PointId::new(&device, &cle), value: valeur, ts: TS, source_ts: None }).collect()
}

fn evenement_journal() -> Event {
    let device = DeviceId::new(&InstanceId::from("z2m"), MESSAGES[3].ieee);
    Event::Journal {
        entry: Arc::new(JournalEntry {
            id: 4217,
            ts: TS,
            origin: Origin::Ui,
            actor: Some("192.168.1.23".into()),
            action: Action::Command { point: PointId::new(&device, "state"), value: Value::Bool(true) },
            outcome: Outcome::Ok,
            duration_ms: 62,
            correlation: None,
        }),
    }
}

// ---- MQTT : octets encodés par rumqttc ------------------------------------------------------------

fn len_len(n: usize) -> usize {
    match n {
        0..=127 => 1,
        128..=16_383 => 2,
        _ => 3,
    }
}

struct Mqtt {
    v4_q0: usize,
    v4_q1: usize,
    puback4: usize,
    v5_q0: usize,
    v5_q1: usize,
    puback5: usize,
}

fn mqtt(topic: &str, charge: &str) -> Mqtt {
    use rumqttc::mqttbytes::QoS as Q4;
    use rumqttc::mqttbytes::v4::{PubAck as A4, Publish as P4};
    use rumqttc::v5::mqttbytes::QoS as Q5;
    use rumqttc::v5::mqttbytes::v5::{PubAck as A5, Publish as P5};
    let v4 = |qos: Q4| {
        let mut p = P4::new(topic, qos, charge.as_bytes().to_vec());
        if qos != Q4::AtMostOnce {
            p.pkid = 1;
        }
        let mut b = BytesMut::new();
        p.write(&mut b).unwrap();
        b.len()
    };
    let v5 = |qos: Q5| {
        let mut p = P5::new(topic, qos, Bytes::copy_from_slice(charge.as_bytes()), None);
        if qos != Q5::AtMostOnce {
            p.pkid = 1;
        }
        let mut b = BytesMut::new();
        p.write(&mut b).unwrap();
        b.len()
    };
    let mut b = BytesMut::new();
    A4::new(1).write(&mut b).unwrap();
    let puback4 = b.len();
    let mut b = BytesMut::new();
    A5::new(1, None).write(&mut b).unwrap();
    let puback5 = b.len();
    let r = Mqtt { v4_q0: v4(Q4::AtMostOnce), v4_q1: v4(Q4::AtLeastOnce), puback4, v5_q0: v5(Q5::AtMostOnce), v5_q1: v5(Q5::AtLeastOnce), puback5 };
    // Contrôle contre la norme : 1 octet d'en-tête fixe + longueur restante + 2 (longueur du topic) + topic
    // + 2 (identifiant, QoS 1) + 1 (longueur des propriétés, MQTT 5) + charge.
    let (t, c) = (topic.len(), charge.len());
    let calc = |ident: usize, props: usize| {
        let reste = 2 + t + ident + props + c;
        1 + len_len(reste) + reste
    };
    assert_eq!(r.v4_q0, calc(0, 0));
    assert_eq!(r.v4_q1, calc(2, 0));
    assert_eq!(r.v5_q0, calc(0, 1));
    assert_eq!(r.v5_q1, calc(2, 1));
    r
}

// ---- le vrai serveur : la pile de Moli ------------------------------------------------------------------

#[derive(Clone)]
struct Etat {
    tx: broadcast::Sender<Arc<Event>>,
}

/// Même forme que `moli-api/src/sse.rs` : `event: event` + `json_data`, `KeepAlive::default()`.
async fn sse(State(e): State<Etat>) -> Response {
    let rx = e.tx.subscribe();
    let flux = stream::unfold(rx, |mut rx| async move {
        match rx.recv().await {
            Ok(ev) => Some((Ok::<_, Infallible>(SseEvent::default().event("event").json_data(&*ev).ok()?), rx)),
            Err(_) => None,
        }
    });
    Sse::new(flux).keep_alive(KeepAlive::default()).into_response()
}

/// Un flux sans aucun événement : seul le maintien de liaison passe.
async fn silence() -> Response {
    Sse::new(stream::pending::<Result<SseEvent, Infallible>>()).keep_alive(KeepAlive::default()).into_response()
}

async fn ws(upgrade: WebSocketUpgrade, State(e): State<Etat>) -> Response {
    let mut rx = e.tx.subscribe();
    upgrade.on_upgrade(move |mut socket| async move {
        while let Ok(ev) = rx.recv().await {
            let texte = serde_json::to_string(&*ev).unwrap();
            if socket.send(Message::Text(texte.into())).await.is_err() {
                break;
            }
        }
    })
}

async fn commande() -> Response {
    Json(json!({ "ok": true })).into_response()
}

// ---- client brut ---------------------------------------------------------------------------------------

struct Brut {
    s: TcpStream,
    tampon: Vec<u8>,
}

impl Brut {
    async fn ouvrir(addr: std::net::SocketAddr, requete: &str) -> Self {
        let mut s = TcpStream::connect(addr).await.unwrap();
        s.set_nodelay(true).unwrap();
        s.write_all(requete.as_bytes()).await.unwrap();
        Brut { s, tampon: Vec::new() }
    }
    async fn plus(&mut self) {
        let mut tmp = [0u8; 4096];
        let n = self.s.read(&mut tmp).await.unwrap();
        assert!(n > 0, "fermeture inattendue");
        self.tampon.extend_from_slice(&tmp[..n]);
    }
    async fn jusque(&mut self, motif: &[u8]) -> Vec<u8> {
        loop {
            if let Some(i) = self.tampon.windows(motif.len()).position(|w| w == motif) {
                return self.tampon.drain(..i + motif.len()).collect();
            }
            self.plus().await;
        }
    }
    async fn n(&mut self, n: usize) -> Vec<u8> {
        while self.tampon.len() < n {
            self.plus().await;
        }
        self.tampon.drain(..n).collect()
    }
    /// Un chunk HTTP : (charge, octets sur le fil dont taille hexadécimale et CRLF).
    async fn chunk(&mut self) -> (Vec<u8>, usize) {
        let ligne = self.jusque(b"\r\n").await;
        let taille = usize::from_str_radix(std::str::from_utf8(&ligne[..ligne.len() - 2]).unwrap(), 16).unwrap();
        let d = self.n(taille + 2).await;
        (d[..taille].to_vec(), ligne.len() + taille + 2)
    }
    /// Une trame WebSocket serveur→client : (taille d'en-tête, charge).
    async fn trame(&mut self) -> (usize, Vec<u8>) {
        let h = self.n(2).await;
        assert_eq!(h[1] & 0x80, 0, "une trame serveur n'est jamais masquée");
        let (en_tete, longueur) = match h[1] & 0x7f {
            126 => {
                let e = self.n(2).await;
                (4, u16::from_be_bytes([e[0], e[1]]) as usize)
            }
            127 => {
                let e = self.n(8).await;
                (10, u64::from_be_bytes(e.try_into().unwrap()) as usize)
            }
            l => (2, l as usize),
        };
        (en_tete, self.n(longueur).await)
    }
}

fn requete_post(jeu: &str, corps: &str) -> String {
    let cookie = format!("moli_session={}", "0123456789abcdef".repeat(4));
    let h = "Host: moli.local:8790\r\n";
    let base = format!("POST /api/command HTTP/1.1\r\n{h}");
    let fin = |entetes: String| format!("{base}{entetes}\r\n{corps}");
    match jeu {
        // A : un agent ou un script, sans cookie de navigateur.
        "A" => fin(format!("Content-Type: application/json\r\nContent-Length: {}\r\nX-Moli-Origin: cli\r\n", corps.len())),
        // B : le tableau de bord, même origine, session ouverte, en-têtes que le navigateur ajoute seul réduits au strict.
        "B" => fin(format!(
            "Connection: keep-alive\r\nContent-Length: {}\r\nContent-Type: application/json\r\nX-Moli-Origin: ui\r\nOrigin: http://moli.local:8790\r\nSec-Fetch-Site: same-origin\r\nCookie: {cookie}\r\n",
            corps.len()
        )),
        // C : un navigateur complet (Chromium typique).
        _ => fin(format!(
            "Connection: keep-alive\r\nContent-Length: {}\r\nsec-ch-ua: \"Chromium\";v=\"130\", \"Not_A Brand\";v=\"99\"\r\nContent-Type: application/json\r\nsec-ch-ua-mobile: ?0\r\nUser-Agent: Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36\r\nx-moli-origin: ui\r\nsec-ch-ua-platform: \"Linux\"\r\nAccept: */*\r\nOrigin: http://moli.local:8790\r\nSec-Fetch-Site: same-origin\r\nSec-Fetch-Mode: cors\r\nSec-Fetch-Dest: empty\r\nReferer: http://moli.local:8790/\r\nAccept-Encoding: gzip, deflate, br, zstd\r\nAccept-Language: fr-FR,fr;q=0.9,en-US;q=0.8,en;q=0.7\r\nCookie: {cookie}\r\n",
            corps.len()
        )),
    }
}

fn pct(a: usize, b: usize) -> String {
    format!("{:.1}×", a as f64 / b as f64)
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    println!("# Banc `tailles` : octets sur le fil (application ; TCP/IP/TLS en note)");
    println!("- plateforme : {} / {} ; axum 0.8.9, tungstenite 0.29 (via axum `ws`), rumqttc 0.25.1, serde_json compact", std::env::consts::OS, std::env::consts::ARCH);
    println!("- « encodé » = octets produits par `rumqttc` (PUBLISH/PUBACK) ; « mesuré » = lu sur une socket TCP brute d'un vrai serveur axum avec la pile de Moli (`CompressionLayer` + en-têtes de sécurité de `guard.rs`) ; « calculé » = formule, vérifiée par `assert` contre l'encodé/mesuré quand les deux existent");
    println!("- ids fictifs : instance `z2m`, appareils `z2m:0x00124b00aaaa000N` ; friendly names `thermometre_salon`, `detecteur_cuisine`, `prise_1`, `lampe_1` ; horodatage {TS} (13 chiffres)\n");

    // ---------------------------------------------------------------- 1. MQTT
    println!("## 1. MQTT : PUBLISH encodé par `rumqttc` (octets du paquet, sans TCP/IP/TLS)\n");
    println!("Topic `zigbee2mqtt/<friendly_name>` (état publié par Z2M) ; pour l'ordre lampe, topic `zigbee2mqtt/lampe_1/set` (ce que le pilote Moli publie, QoS 1). MQTT 5 sans propriétés (1 octet de longueur de propriétés).\n");
    println!("| message | charge JSON | topic | v3.1.1 QoS 0 | v3.1.1 QoS 1 | + PUBACK | v5 QoS 0 | v5 QoS 1 | + PUBACK v5 | QoS 1 aller-retour (v3.1.1) |");
    println!("|---|---|---|---|---|---|---|---|---|---|");
    let mut mq = Vec::new();
    for m in &MESSAGES {
        let r = mqtt(m.topic, m.json);
        println!(
            "| {} | {} o | {} o (`{}`) | **{} o** | {} o | {} o | {} o | {} o | {} o | {} o |",
            m.nom,
            m.json.len(),
            m.topic.len(),
            m.topic,
            r.v4_q0,
            r.v4_q1,
            r.puback4,
            r.v5_q0,
            r.v5_q1,
            r.puback5,
            r.v4_q1 + r.puback4
        );
        mq.push(r);
    }
    println!("\nSurcoût fixe d'un PUBLISH v3.1.1 QoS 0 = 1 (en-tête fixe) + 1 (longueur restante, ≤ 127) + 2 (longueur du topic) + topic ; QoS 1 : +2 (identifiant) ; MQTT 5 : +1 (propriétés).");
    println!("Avec l'identifiant Z2M par défaut en friendly name (`zigbee2mqtt/0x00124b00aaaa0001`, 30 o) au lieu d'un nom court, le topic ne varie que de quelques octets (ex. thermomètre : 29 → 30 o).");

    // ---------------------------------------------------------------- serveur de mesure
    let (tx, _) = broadcast::channel::<Arc<Event>>(1024);
    let app: Router = avec_pile_moli(
        Router::new()
            .route("/api/events", get(sse))
            .route("/api/silence", get(silence))
            .route("/ws", get(ws))
            .route("/api/command", post(commande))
            .with_state(Etat { tx: tx.clone() }),
    );
    let ecoute = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = ecoute.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(ecoute, app).await.unwrap() });

    // Le maintien de liaison se mesure sur son propre flux : 15 s d'attente, en parallèle du reste.
    let silencieux = tokio::spawn(async move {
        let mut c = Brut::ouvrir(addr, "GET /api/silence HTTP/1.1\r\nHost: moli.local:8790\r\nAccept: text/event-stream\r\n\r\n").await;
        let entetes = c.jusque(b"\r\n\r\n").await;
        let t0 = Instant::now();
        let (charge, fil) = c.chunk().await;
        (entetes.len(), t0.elapsed(), charge, fil)
    });

    // ---------------------------------------------------------------- 2. SSE
    println!("\n## 2. Moli → navigateur : SSE (vrais `Event::State`, un par attribut décodé)\n");
    let mut sse = Brut::ouvrir(addr, "GET /api/events HTTP/1.1\r\nHost: moli.local:8790\r\nAccept: text/event-stream\r\nCache-Control: no-cache\r\n\r\n").await;
    let entetes_sse = sse.jusque(b"\r\n\r\n").await;
    let texte_entetes_sse = String::from_utf8_lossy(&entetes_sse).into_owned();
    println!("En-têtes de la réponse `GET /api/events` (une fois par connexion) : **{} o**\n```\n{}```\n", entetes_sse.len(), texte_entetes_sse);
    println!("Cadrage axum 0.8.9 (`response/sse.rs`) : `event: event\\n` (13 o) + `data: ` (6 o) + JSON + `\\n` + `\\n` final ; chaque événement est un `Frame::data` donc un chunk HTTP/1.1 `<taille hex>\\r\\n<événement>\\r\\n`.\n");
    println!("| message | événements | JSON (somme) | JSON moyen | SSE cadré (somme) | + chunked = **sur le fil** | par événement | rapport au PUBLISH QoS 0 |");
    println!("|---|---|---|---|---|---|---|---|");
    let mut sse_total = Vec::new();
    let mut exemple = String::new();
    for (m, r) in MESSAGES.iter().zip(&mq) {
        let evts = evenements(m);
        let (mut json_somme, mut cadre_somme, mut fil_somme) = (0, 0, 0);
        tokio::time::sleep(Duration::from_millis(30)).await;
        for e in &evts {
            tx.send(Arc::new(e.clone())).unwrap();
        }
        for e in &evts {
            let (charge, fil) = sse.chunk().await;
            let json = serde_json::to_string(e).unwrap();
            let attendu = format!("event: event\ndata: {json}\n\n");
            assert_eq!(String::from_utf8(charge).unwrap(), attendu, "le cadrage mesuré diffère du cadrage calculé");
            // Calcul : taille hexadécimale + CRLF + événement + CRLF.
            assert_eq!(fil, format!("{:x}", attendu.len()).len() + 2 + attendu.len() + 2);
            json_somme += json.len();
            cadre_somme += attendu.len();
            fil_somme += fil;
            if exemple.is_empty() || (m.nom == "thermomètre" && json.contains("temperature")) {
                exemple = json;
            }
        }
        println!(
            "| {} | {} | {} o | {:.1} o | {} o | **{} o** | {:.1} o | {} |",
            m.nom,
            evts.len(),
            json_somme,
            json_somme as f64 / evts.len() as f64,
            cadre_somme,
            fil_somme,
            fil_somme as f64 / evts.len() as f64,
            pct(fil_somme, r.v4_q0)
        );
        sse_total.push((evts.len(), json_somme, cadre_somme, fil_somme));
    }
    println!("\nExemple d'événement (thermomètre, attribut `temperature`) : `{exemple}`");
    // Événement de journal : une ligne par ordre, diffusée au foyer.
    let journal = evenement_journal();
    tx.send(Arc::new(journal.clone())).unwrap();
    let (charge, fil_journal) = sse.chunk().await;
    let json_journal = serde_json::to_string(&journal).unwrap();
    assert_eq!(String::from_utf8(charge).unwrap(), format!("event: event\ndata: {json_journal}\n\n"));
    println!("\nL'ordre lampe produit aussi un `Event::Journal` (une ligne par ordre, diffusée au foyer) : `{json_journal}` → JSON {} o, sur le fil (SSE + chunked) **{fil_journal} o**.", json_journal.len());
    // Hypothèse : un événement par appareil au lieu d'un par attribut (comparaison seulement).
    println!("\nComparaison hypothétique (non implémentée dans Moli) : un seul événement par appareil, `{{\"type\":\"device_state\",\"device\":…,\"state\":{{…attributs…}},\"ts\":…}}` :\n");
    println!("| message | événement unique hypothétique (JSON) | sur le fil (SSE + chunked) | contre mesuré par attribut |");
    println!("|---|---|---|---|");
    for (m, (_, _, _, fil)) in MESSAGES.iter().zip(&sse_total) {
        let attrs: serde_json::Map<String, serde_json::Value> = decoder(m.json).into_iter().map(|(k, v)| (k, v.to_json())).collect();
        let device = DeviceId::new(&InstanceId::from("z2m"), m.ieee);
        let j = serde_json::to_string(&json!({ "type": "device_state", "device": device, "state": attrs, "ts": TS })).unwrap();
        let cadre = format!("event: event\ndata: {j}\n\n").len();
        let sur_fil = format!("{:x}", cadre).len() + 2 + cadre + 2;
        println!("| {} | {} o | {} o | {} o |", m.nom, j.len(), sur_fil, fil);
    }

    // ---------------------------------------------------------------- 3. WebSocket
    println!("\n## 3. WebSocket : le même JSON (trames serveur→client lues sur le fil, axum `ws` / tungstenite)\n");
    let mut w = Brut::ouvrir(
        addr,
        "GET /ws HTTP/1.1\r\nHost: moli.local:8790\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n",
    )
    .await;
    let poignee = w.jusque(b"\r\n\r\n").await;
    println!("Poignée de main minimale (une fois par connexion) : requête {} o, réponse `101` **{} o** :\n```\n{}```\n", "GET /ws HTTP/1.1\r\nHost: moli.local:8790\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n".len(), poignee.len(), String::from_utf8_lossy(&poignee));
    println!("Un navigateur ajoute `Origin`, `User-Agent`, `Accept-*`, `Sec-WebSocket-Extensions`… (≈ +300 o) ; hors du sujet « message ».\n");
    println!("| message | événements | JSON (somme) | en-têtes de trame (2 o ≤ 125 o de charge) | **trame serveur→client (somme)** | par événement | client→serveur (+4 o de masque) | rapport au PUBLISH QoS 0 |");
    println!("|---|---|---|---|---|---|---|---|");
    for (m, r) in MESSAGES.iter().zip(&mq) {
        let evts = evenements(m);
        tokio::time::sleep(Duration::from_millis(30)).await;
        for e in &evts {
            tx.send(Arc::new(e.clone())).unwrap();
        }
        let (mut json, mut total, mut entete) = (0, 0, 0);
        for e in &evts {
            let (h, charge) = w.trame().await;
            let attendu = serde_json::to_string(e).unwrap();
            assert_eq!(String::from_utf8(charge).unwrap(), attendu);
            assert_eq!(h, if attendu.len() <= 125 { 2 } else if attendu.len() <= 65_535 { 4 } else { 10 });
            json += attendu.len();
            entete += h;
            total += h + attendu.len();
        }
        println!("| {} | {} | {} o | {} o | **{} o** | {:.1} o | {} o | {} |", m.nom, evts.len(), json, entete, total, total as f64 / evts.len() as f64, total + 4 * evts.len(), pct(total, r.v4_q0));
    }
    println!("\nEn-tête de trame calculé : 2 o si charge ≤ 125 o, 4 o si ≤ 65 535 o, 10 o au-delà (vérifié sur les trames lues ci-dessus ; le snapshot de 480 Kio prendrait 10 o). Aucune compression par message (`permessage-deflate` n'est pas négocié par tungstenite).");

    // ---------------------------------------------------------------- 4. POST /api/command
    println!("\n## 4. `POST /api/command` (HTTP/1.1 keep-alive, requête et réponse mesurées)\n");
    let corps = r#"{"point":"z2m:0x00124b00aaaa0004/state","value":true}"#;
    println!("Corps réel (`CommandBody` : `point`, `value`, `actor?`, `request_id?`) tel que le tableau de bord l'envoie (`JSON.stringify({{ point, value }})`) : `{corps}` ({} o).\n", corps.len());
    let mut lignes = Vec::new();
    let mut reponse_b = String::new();
    for (jeu, desc) in [
        ("A", "agent/script : Host, Content-Type, Content-Length, X-Moli-Origin"),
        ("B", "tableau de bord, même origine, session ouverte (en-têtes strictement nécessaires + ceux que le navigateur impose : Origin, Sec-Fetch-Site, Cookie, Connection)"),
        ("C", "navigateur Chromium complet (+ User-Agent, sec-ch-ua*, Accept*, Referer, Sec-Fetch-Mode/Dest)"),
    ] {
        let requete = requete_post(jeu, corps);
        let mut c = Brut::ouvrir(addr, &requete).await;
        let entetes = c.jusque(b"\r\n\r\n").await;
        let texte = String::from_utf8_lossy(&entetes).into_owned();
        let longueur: usize = texte.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap())).unwrap();
        let corps_reponse = c.n(longueur).await;
        let total = entetes.len() + corps_reponse.len();
        lignes.push((jeu, desc, requete.len(), requete.len() - corps.len(), entetes.len(), longueur, total));
        if jeu == "B" {
            reponse_b = format!("{texte}{}", String::from_utf8_lossy(&corps_reponse));
        }
    }
    println!("| jeu | en-têtes de la requête | requête (o) | dont en-têtes | réponse : en-têtes | réponse : corps | réponse (o) | requête + réponse |");
    println!("|---|---|---|---|---|---|---|---|");
    for (jeu, desc, req, ent, rep_ent, rep_corps, rep_total) in &lignes {
        println!("| {jeu} | {desc} | {req} | {ent} | {rep_ent} | {rep_corps} | {rep_total} | **{}** |", req + rep_total);
    }
    println!("\nJeu B détaillé (requête envoyée) :\n```\n{}```", requete_post("B", corps));
    println!("Réponse reçue (octets réels de la pile Moli : en-têtes axum par défaut + `date` hyper + en-têtes de sécurité de `guard::headers` ; pas de `vary` car `CompressionLayer` ne l'ajoute que si le corps dépasse 32 o, seuil de `DefaultPredicate`) :\n```\n{reponse_b}\n```");
    // Réponse « axum nu » (sans la pile de Moli) pour comparaison.
    let nu: Router = Router::new().route("/api/command", post(commande));
    let ecoute_nu = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr_nu = ecoute_nu.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(ecoute_nu, nu).await.unwrap() });
    let mut c = Brut::ouvrir(addr_nu, &requete_post("B", corps)).await;
    let ent_nu = c.jusque(b"\r\n\r\n").await;
    let corps_nu = c.n(11).await;
    println!("Pour comparaison, la même réponse d'un axum nu (sans les couches de Moli) : en-têtes {} o + corps {} o = **{} o**.", ent_nu.len(), corps_nu.len(), ent_nu.len() + corps_nu.len());

    // ---------------------------------------------------------------- 5. maintiens de liaison
    println!("\n## 5. Maintiens de liaison\n");
    let (taille_entetes_silence, delai, charge, fil) = silencieux.await.unwrap();
    println!("SSE : `KeepAlive::default()` d'axum 0.8.9 = intervalle **15 s** (`KeepAlive::new`, `Duration::from_secs(15)`), texte **vide** : commentaire `:\\n\\n` (`Event::DEFAULT_KEEP_ALIVE`, 3 o). Mesuré sur un flux sans événement : premier maintien après {:.2} s, charge {:?} ({} o), sur le fil avec le chunk **{fil} o** (en-têtes de réponse de ce flux : {taille_entetes_silence} o). Le délai est remis à zéro à chaque événement : un foyer bavard n'émet aucun maintien.", delai.as_secs_f64(), String::from_utf8_lossy(&charge), charge.len());
    assert_eq!(charge, b":\n\n");
    assert!(delai > Duration::from_millis(14_500) && delai < Duration::from_millis(15_800));
    let (ping, pong) = {
        let mut b = BytesMut::new();
        rumqttc::mqttbytes::v4::PingReq.write(&mut b).unwrap();
        let ping = b.len();
        let mut b = BytesMut::new();
        rumqttc::mqttbytes::v4::PingResp.write(&mut b).unwrap();
        (ping, b.len())
    };
    println!("MQTT : `keep_alive` 30 s du pilote (`options.set_keep_alive(30 s)`) → PINGREQ ({ping} o, `C0 00`) + PINGRESP ({pong} o, `D0 00`) toutes les 30 s lorsque la liaison est silencieuse (même taille en MQTT 5).\n");
    // Coûts par jour : application, et sur le fil (calculé) avec IPv4 20 + TCP 20 + options horodatage 12 = 52 o par segment.
    let seg = 52usize; // IPv4 20 + TCP 20 + options (horodatage) 12
    let tls = 22usize; // enregistrement TLS 1.3 : 5 en-tête + 1 type interne + 16 étiquette
    let cyc_mqtt_app = ping + pong;
    let cyc_mqtt_fil = (ping + seg) + (pong + seg) + seg; // PINGREQ, PINGRESP (l'ACK du PINGREQ voyage avec lui), ACK final du client
    let cyc_mqtt_tls = (ping + tls + seg) + (pong + tls + seg) + seg;
    let cyc_sse_app = fil;
    let cyc_sse_fil = (fil + seg) + seg; // l'événement, puis l'ACK du client
    let cyc_sse_tls = (fil + tls + seg) + seg;
    let (n_mqtt, n_sse) = (86_400 / 30, 86_400 / 15);
    println!("| liaison | cycle | octets applicatifs / cycle | cycles / jour | **applicatif / jour** | sur le fil, IPv4+TCP (52 o/segment), 1 message = 1 segment, ACK compris | sur le fil + TLS 1.3 (22 o/enregistrement) |");
    println!("|---|---|---|---|---|---|---|");
    println!("| MQTT (1 liaison Moli↔courtier) | PINGREQ + PINGRESP / 30 s | {} | {} | **{} o ({:.1} Kio)** | {} o/cycle → {} o/jour ({:.0} Kio) | {} o/cycle → {} o/jour ({:.0} Kio) |", cyc_mqtt_app, n_mqtt, cyc_mqtt_app * n_mqtt, (cyc_mqtt_app * n_mqtt) as f64 / 1024.0, cyc_mqtt_fil, cyc_mqtt_fil * n_mqtt, (cyc_mqtt_fil * n_mqtt) as f64 / 1024.0, cyc_mqtt_tls, cyc_mqtt_tls * n_mqtt, (cyc_mqtt_tls * n_mqtt) as f64 / 1024.0);
    println!("| SSE (par navigateur connecté) | commentaire `:` / 15 s | {} | {} | **{} o ({:.1} Kio)** | {} o/cycle → {} o/jour ({:.0} Kio) | {} o/cycle → {} o/jour ({:.0} Kio) |", cyc_sse_app, n_sse, cyc_sse_app * n_sse, (cyc_sse_app * n_sse) as f64 / 1024.0, cyc_sse_fil, cyc_sse_fil * n_sse, (cyc_sse_fil * n_sse) as f64 / 1024.0, cyc_sse_tls, cyc_sse_tls * n_sse, (cyc_sse_tls * n_sse) as f64 / 1024.0);
    println!("\nNota : pas de maintien SSE quand un événement circule moins de 15 s avant (timer remis à zéro) ; MQTT de même (le PINGREQ n'est émis qu'après `keep_alive` sans autre paquet sortant). Les colonnes « sur le fil » sont CALCULÉES, pas mesurées.");

    // ---------------------------------------------------------------- 6. bilan d'un ordre
    println!("\n## 6. Bilan d'un ordre de lampe, de bout en bout (octets applicatifs ; jeu B pour la requête ; état de retour = `{{\"state\":\"ON\"}}` en QoS 0)\n");
    let b = &lignes[1];
    let q = &mq[3];
    let echo = mqtt("zigbee2mqtt/lampe_1", r#"{"state":"ON"}"#);
    let (sse_etat, sse_journal) = (sse_total[3].3, fil_journal);
    println!("| étape | octets |");
    println!("|---|---|");
    println!("| navigateur → Moli : `POST /api/command` (jeu B) | {} |", b.2);
    println!("| Moli → navigateur : réponse 200 | {} |", b.6);
    println!("| Moli → Z2M : PUBLISH `…/lampe_1/set` QoS 1 (v3.1.1) | {} |", q.v4_q1);
    println!("| courtier → Moli : PUBACK | {} |", q.puback4);
    println!("| Z2M → Moli : état `{{\"state\":\"ON\"}}` sur `zigbee2mqtt/lampe_1`, QoS 0 | {} |", echo.v4_q0);
    println!("| Moli → chaque navigateur : SSE `Event::State` (chunk compris) | {sse_etat} |");
    println!("| Moli → chaque navigateur : SSE `Event::Journal` (chunk compris) | {sse_journal} |");
    println!("| **total côté MQTT (trois paquets)** | **{}** |", q.v4_q1 + q.puback4 + echo.v4_q0);
    println!("| **total côté HTTP/SSE pour 1 navigateur** | **{}** |", b.2 + b.6 + sse_etat + sse_journal);

    // ---------------------------------------------------------------- notes
    println!("\n## Notes : ce qui s'ajoute sous l'application (non compté ci-dessus)\n");
    println!("- IPv4 : 20 o ; TCP : 20 o + options (horodatage : +12 o → 32 o, courant sous Linux) ; Ethernet : 14 o (+ 4 o de FCS).");
    println!("- TLS 1.3 : un enregistrement par message ou groupe d'écritures = 5 o d'en-tête + 1 o de type interne + 16 o d'étiquette AEAD = 22 o (TLS 1.2 AES-GCM : 5 + 8 de nonce explicite + 16 = 29 o).");
    println!("- Exemple : un PUBLISH QoS 0 de thermomètre ({} o) seul dans un segment = {} o sur un LAN en clair (+ 22 o sous TLS 1.3) ; l'ACK de l'autre côté ajoute un segment vide de 52 o (+ 14 o d'Ethernet chacun).", mq[0].v4_q0, mq[0].v4_q0 + 52);
    println!("- Rafale : plusieurs événements SSE ou trames WebSocket écrits ensemble partagent un segment (cadrage inchangé) ; MQTT de même (plusieurs paquets par `write`).");
    let _ = o(0);
}
`````

Code complet — `preuves-v5/courtier/src/main.rs` :

`````rust
//! Bancs `courtier` : rumqttd 0.20.0 embarqué. Voir `lien.rs` (en processus) et `tcp.rs` (clients TCP).
//!
//! En processus :  `lien [octets_segment] [segments] [secs]` · `bus [secs]` · `volume [octets_segment] [segments] [plafond_kio] [max_messages]`
//!                 `soutenu [octets_segment] [segments] [secs] [taux]` · `seuil <octets_segment> <segments> <L…>` · `tailles`
//! Courtier TCP :  `broker <port> <controle> [octets_segment] [segments] [dynamic_filters 0|1] [max_sortie]`
//! Clients TCP  :  `clients <scénario> <port> <controle> …` avec
//!   `memoire <attrape-tout 0|1>` · `abonnes <N> <retenus> <qos> <hz> <secs>` · `rafale <qos> <aucun|lent|bloque> <n> <observation_s>`
//! Mac          :  `tout <scénario> …` lance le courtier en processus fils (variables SEG, NSEG, DYN, MAXOUT) puis les clients.

mod acl;
mod lien;
mod systeme;
mod tcp;

fn lancer(rt: &tokio::runtime::Runtime, scenario: &str, port: u16, ctl: u16, a: &[String]) {
    let nb = |i: usize, d: u64| a.get(i).and_then(|v| v.parse::<u64>().ok()).unwrap_or(d);
    match scenario {
        "memoire" => rt.block_on(tcp::memoire(port, ctl, nb(0, 0) == 1)),
        "abonnes" => rt.block_on(tcp::abonnes(port, ctl, nb(0, 64) as usize, nb(1, 0) as usize, nb(2, 1) as u8, nb(3, 10), nb(4, 10))),
        "rafale" => rt.block_on(tcp::rafale(port, ctl, nb(0, 0) as u8, a.get(1).map_or("bloque", String::as_str), nb(2, 5_000), nb(3, 10))),
        autre => eprintln!("scénario inconnu : {autre}"),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let nb = |i: usize, d: u64| args.get(i).and_then(|v| v.parse::<u64>().ok()).unwrap_or(d);
    match args.first().map(String::as_str) {
        Some("lien") => lien::lien(nb(1, 1_048_576), nb(2, 4), nb(3, 5)),
        Some("bus") => lien::bus(nb(1, 5)),
        Some("acl") => acl::acl(),
        Some("tailles") => {
            // Pourquoi ≈ 62 Kio par abonné avant tout trafic : deux tampons de 200 éléments alloués d'avance par connexion
            // (`Incoming` : `Packet`, `Outgoing` : `Notification`, `MAX_CHANNEL_CAPACITY` = 200 dans rumqttd).
            let (n, p, pu, f) = (std::mem::size_of::<rumqttd::Notification>(), std::mem::size_of::<rumqttd::protocol::Packet>(), std::mem::size_of::<rumqttd::protocol::Publish>(), std::mem::size_of::<rumqttd::Forward>());
            println!("- size_of : Notification {n} o, Packet {p} o, Publish {pu} o, Forward {f} o ; tampons alloués d'avance par connexion : 200 × (Packet + Notification) = {} Kio", 200 * (n + p) / 1024);
        }
        Some("soutenu") => lien::soutenu(nb(1, 1_048_576), nb(2, 4), nb(3, 60), nb(4, 200)),
        Some("seuil") => {
            let liste: Vec<u64> = args.iter().skip(3).filter_map(|v| v.parse().ok()).collect();
            lien::seuil(nb(1, 65_536), nb(2, 4), &liste);
        }
        Some("seuil-phase") => {
            let phases: Vec<u64> = args.iter().skip(3).filter_map(|v| v.parse().ok()).collect();
            lien::seuil_phase(nb(1, 65_536), nb(2, 4), &phases);
        }
        Some("volume") => lien::volume(nb(1, 104_857_600), nb(2, 10), nb(3, 100_000), nb(4, 3_000_000)),
        Some("broker") => tcp::courtier(tcp::ParamsCourtier {
            port: nb(1, 1883) as u16,
            controle: nb(2, 8801) as u16,
            octets_segment: nb(3, 104_857_600),
            segments: nb(4, 10),
            dynamic_filters: nb(5, 1) == 1,
            max_sortie: nb(6, 200),
        }),
        Some("clients") => {
            let rt = tokio::runtime::Runtime::new().unwrap();
            lancer(&rt, args.get(1).map_or("", String::as_str), nb(2, 1883) as u16, nb(3, 8801) as u16, &args[4.min(args.len())..]);
        }
        Some("tout") => {
            struct Fils(std::process::Child);
            impl Drop for Fils {
                fn drop(&mut self) {
                    let _ = self.0.kill();
                    let _ = self.0.wait();
                }
            }
            let env = |c: &str, d: &str| std::env::var(c).unwrap_or_else(|_| d.to_owned());
            let port = 31_000 + (std::process::id() % 10_000) as u16;
            let _fils = Fils(
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["broker", &port.to_string(), &(port + 1).to_string(), &env("SEG", "104857600"), &env("NSEG", "10"), &env("DYN", "1"), &env("MAXOUT", "200")])
                    .spawn()
                    .unwrap(),
            );
            let rt = tokio::runtime::Runtime::new().unwrap();
            lancer(&rt, args.get(1).map_or("", String::as_str), port, port + 1, &args[2.min(args.len())..]);
        }
        _ => eprintln!("usage : courtier lien|bus|volume|broker|clients|tout …"),
    }
}
`````

Code complet — `preuves-v5/courtier/src/lien.rs` :

`````rust
//! Bancs en processus : liens locaux de rumqttd (sans TCP), `tokio::sync::broadcast` portant les vrais
//! `moli_core::Event`, et croissance de la mémoire du courtier selon le volume publié.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::*};
use std::time::{Duration, Instant};

use bytes::Bytes;
use moli_core::{Event, PointId, Value};
use rumqttd::local::{LinkRx, LinkTx};
use rumqttd::{Broker, Config, Notification};
use serde_json::json;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;

use crate::systeme::{Histo, cpu_fil_ms, cpu_fils, duree, kio, percentile, rss, systeme};

/// Configuration du courtier sans écouteur réseau : seuls les liens locaux servent.
pub fn config(octets_segment: u64, segments: u64, max_sortie: u64) -> Config {
    serde_json::from_value(json!({
        "id": 0,
        "router": { "id": 0, "max_connections": 10010, "max_outgoing_packet_count": max_sortie, "max_segment_size": octets_segment, "max_segment_count": segments }
    }))
    .expect("configuration rumqttd")
}

// ---- charge utile de 120 o --------------------------------------------------------------------------------

const CHARGE: usize = 120;

/// JSON de `CHARGE` o (120) dont deux champs de largeur fixe sont réécrits à chaque message : `seq` (9 chiffres) et `t`
/// (15 chiffres, ns depuis le début du palier), ce qui évite d'analyser du JSON côté abonné.
pub struct Modele {
    base: Vec<u8>,
    off_seq: usize,
    off_t: usize,
}

impl Modele {
    pub fn new() -> Self {
        Self::de_taille(CHARGE)
    }
    pub fn de_taille(taille: usize) -> Self {
        let mut s = String::from(r#"{"point":"z2m:0x00124b00aaaa0001/temperature","value":21.5,"seq":000000000,"t":000000000000000"#);
        let manque = taille - s.len() - r#","q":""}"#.len();
        s.push_str(r#","q":""#);
        s.push_str(&"x".repeat(manque));
        s.push_str(r#""}"#);
        assert_eq!(s.len(), taille);
        let off_seq = s.find("\"seq\":").unwrap() + 6;
        let off_t = s.find("\"t\":").unwrap() + 4;
        Modele { base: s.into_bytes(), off_seq, off_t }
    }
    pub fn fabriquer(&self, seq: u64, t_ns: u64) -> Bytes {
        let mut v = self.base.clone();
        ecrire(&mut v[self.off_seq..self.off_seq + 9], seq);
        ecrire(&mut v[self.off_t..self.off_t + 15], t_ns);
        Bytes::from(v)
    }
    pub fn lire_t(&self, p: &[u8]) -> u64 {
        p[self.off_t..self.off_t + 15].iter().fold(0, |a, b| a * 10 + u64::from(b - b'0'))
    }
    pub fn lire_seq(&self, p: &[u8]) -> u64 {
        p[self.off_seq..self.off_seq + 9].iter().fold(0, |a, b| a * 10 + u64::from(b - b'0'))
    }
}

fn ecrire(zone: &mut [u8], mut n: u64) {
    for i in (0..zone.len()).rev() {
        zone[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
}

/// 64 topics `moli/z2m:0x00124b00aaaaNNNN/temperature`.
fn topics() -> Vec<Bytes> {
    (0..64).map(|k| Bytes::from(format!("moli/z2m:0x00124b00aaaa{k:04x}/temperature"))).collect()
}

// ---- résultat d'un palier ------------------------------------------------------------------------------------

pub struct Palier {
    taux: u64,
    secs: u64,
    envoyes: u64,
    recus: u64,
    perdus: u64,
    reel_s: f64,
    lat: Histo,
    retard_editeur_p99: u64,
    cpu_routeur: Option<u64>,
    cpu_editeur: Option<u64>,
    cpu_abonne: Option<u64>,
    rss_avant: Option<u64>,
    rss_apres: Option<u64>,
}

fn en_tete_table() {
    println!("| débit offert | envoyés | reçus | perdus | débit reçu | p50 | p90 | p99 | p99,9 | max | retard p99 de l'éditeur | CPU routeur / éditeur / abonné (ms) | RssAnon avant → après (Kio) |");
    println!("|---|---|---|---|---|---|---|---|---|---|---|---|---|");
}

fn ligne(p: &mut Palier) {
    let debit = p.recus as f64 / p.reel_s;
    let offert = if p.taux == 0 { "à fond".to_owned() } else { format!("{}/s", p.taux) };
    let cpu = |c: Option<u64>| c.map_or("—".to_owned(), |v| v.to_string());
    println!(
        "| {offert} × {} s | {} | {} | {} | **{:.0}/s** | {} | {} | **{}** | {} | {} | {} | {} / {} / {} | {} → {} |",
        p.secs,
        p.envoyes,
        p.recus,
        p.perdus,
        debit,
        duree(p.lat.percentile(0.50)),
        duree(p.lat.percentile(0.90)),
        duree(p.lat.percentile(0.99)),
        duree(p.lat.percentile(0.999)),
        duree(p.lat.max),
        duree(p.retard_editeur_p99),
        cpu(p.cpu_routeur),
        cpu(p.cpu_editeur),
        cpu(p.cpu_abonne),
        p.rss_avant.map_or("—".into(), |v| v.to_string()),
        p.rss_apres.map_or("—".into(), |v| v.to_string()),
    );
}

// ---- liens locaux de rumqttd ----------------------------------------------------------------------------------

fn palier_lien(ptx: &mut LinkTx, srx: &mut LinkRx, modele: &Modele, topics: &[Bytes], taux: u64, secs: u64) -> Palier {
    let rss_avant = rss(&systeme());
    let cpu_routeur0 = cpu_fils().get("router-0").copied();
    let base = Instant::now();
    let envoyes = AtomicU64::new(0);
    let fini = AtomicBool::new(false);
    let (recus, lat, cpu_ab, retards, cpu_ed) = std::thread::scope(|s| {
        let abonne = std::thread::Builder::new()
            .name("abonne".into())
            .spawn_scoped(s, || {
                let cpu0 = cpu_fil_ms();
                let mut lat = Histo::new();
                let mut recus = 0u64;
                let mut dernier = Instant::now();
                loop {
                    match srx.recv_deadline(Instant::now() + Duration::from_millis(50)) {
                        Ok(Some(Notification::Forward(f))) => {
                            let maintenant = base.elapsed().as_nanos() as u64;
                            lat.ajouter(maintenant.saturating_sub(modele.lire_t(&f.publish.payload)));
                            recus += 1;
                            dernier = Instant::now();
                        }
                        // Le routeur a « déprogrammé » le lien (file de sortie pleine, 200 notifications) : sans `ready()`
                        // il ne reprendrait jamais l'envoi. Un abonné réseau le fait après chaque écriture (`RemoteLink::start`).
                        Ok(Some(Notification::Unschedule)) => {
                            let _ = srx.ready();
                        }
                        _ => {}
                    }
                    if fini.load(Acquire) && (recus >= envoyes.load(Acquire) || dernier.elapsed() > Duration::from_secs(3)) {
                        break;
                    }
                }
                (recus, lat, cpu_fil_ms().zip(cpu0).map(|(a, b)| a - b))
            })
            .unwrap();
        let editeur = std::thread::Builder::new()
            .name("editeur".into())
            .spawn_scoped(s, || {
                let cpu0 = cpu_fil_ms();
                let mut retards: Vec<u64> = Vec::new();
                let mut i = 0u64;
                if taux > 0 {
                    let periode = 1_000_000_000 / taux;
                    let total = taux * secs;
                    while i < total {
                        let maintenant = base.elapsed().as_nanos() as u64;
                        let du = (maintenant / periode + 1).min(total);
                        if i >= du {
                            let attente = (i * periode).saturating_sub(maintenant);
                            std::thread::sleep(Duration::from_nanos(attente.min(50_000)));
                            continue;
                        }
                        while i < du {
                            let t = base.elapsed().as_nanos() as u64;
                            retards.push(t.saturating_sub(i * periode));
                            ptx.publish(topics[(i % 64) as usize].clone(), modele.fabriquer(i, t)).expect("publication");
                            i += 1;
                        }
                    }
                } else {
                    // À fond : autant que possible pendant `secs` secondes.
                    while base.elapsed() < Duration::from_secs(secs) {
                        let t = base.elapsed().as_nanos() as u64;
                        ptx.publish(topics[(i % 64) as usize].clone(), modele.fabriquer(i, t)).expect("publication");
                        i += 1;
                    }
                }
                envoyes.store(i, Release);
                fini.store(true, Release);
                (retards, cpu_fil_ms().zip(cpu0).map(|(a, b)| a - b))
            })
            .unwrap();
        let (retards, cpu_ed) = editeur.join().unwrap();
        let (recus, lat, cpu_ab) = abonne.join().unwrap();
        (recus, lat, cpu_ab, retards, cpu_ed)
    });
    let reel_s = base.elapsed().as_secs_f64();
    let envoyes = envoyes.load(Acquire);
    let mut retards = retards;
    retards.sort_unstable();
    Palier {
        taux,
        secs,
        envoyes,
        recus,
        perdus: envoyes.saturating_sub(recus),
        reel_s: reel_s.min(secs as f64 + 0.5).max(secs as f64),
        lat,
        retard_editeur_p99: percentile(&retards, 0.99),
        cpu_routeur: cpu_fils().get("router-0").copied().zip(cpu_routeur0).map(|(a, b)| a - b),
        cpu_editeur: cpu_ed,
        cpu_abonne: cpu_ab,
        rss_avant,
        rss_apres: rss(&systeme()),
    }
}

/// Débits offerts de 1 000 à 50 000 messages/s (puis à fond), un éditeur, un abonné `moli/#`, tous deux
/// des liens locaux du même processus. Courtier : segments bornés (voir l'en-tête de la sortie).
pub fn lien(octets_segment: u64, segments: u64, secs: u64) {
    let broker = Broker::new(config(octets_segment, segments, 200));
    let (mut stx, mut srx) = broker.link("abonne").expect("lien abonné");
    let (mut ptx, _prx) = broker.link("editeur").expect("lien éditeur");
    stx.subscribe("moli/#").unwrap();
    // On attend l'accusé d'abonnement avant de publier : sans filtre correspondant, une publication
    // d'un lien local (dynamic_filters = false) est refusée et le lien déconnecté.
    loop {
        if let Ok(Some(Notification::DeviceAck(_))) = srx.recv_deadline(Instant::now() + Duration::from_secs(5)) {
            break;
        }
    }
    let modele = Modele::new();
    let topics = topics();
    println!("- liens locaux `rumqttd::local::{{LinkTx, LinkRx}}` (QoS 0, sans TCP), éditeur et abonné dans deux fils du même processus, routeur = fil `router-0` de rumqttd");
    println!("- charge JSON de {CHARGE} o, 64 topics `moli/z2m:0x…/temperature` (≈ 49 o), un abonné `moli/#` ; segments du courtier {octets_segment} o × {segments}");
    println!("- latence = de l'appel `publish()` (instant relevé juste avant) à la réception de la notification par l'abonné ; l'éditeur dort 50 µs au plus entre deux salves et envoie ce qui est dû (retard de l'éditeur sur son horaire en colonne `retard`)\n");
    en_tete_table();
    // Échauffement.
    let mut w = palier_lien(&mut ptx, &mut srx, &modele, &topics, 1_000, 1);
    std::hint::black_box(&mut w);
    for taux in [1_000, 5_000, 10_000, 20_000, 50_000, 0] {
        let mut p = palier_lien(&mut ptx, &mut srx, &modele, &topics, taux, secs);
        ligne(&mut p);
    }
}

// ---- tokio::sync::broadcast (le bus actuel du hub) -----------------------------------------------------------------

fn palier_bus(taux: u64, secs: u64, ids: &[PointId]) -> Palier {
    let (tx, mut rx) = broadcast::channel::<Event>(1024);
    let rss_avant = rss(&systeme());
    let base = Instant::now();
    let envoyes = AtomicU64::new(0);
    let (recus, perdus, lat, cpu_ab, retards, cpu_ed) = std::thread::scope(|s| {
        let abonne = std::thread::Builder::new()
            .name("abonne".into())
            .spawn_scoped(s, || {
                let cpu0 = cpu_fil_ms();
                let mut lat = Histo::new();
                let (mut recus, mut perdus) = (0u64, 0u64);
                loop {
                    match rx.blocking_recv() {
                        Ok(Event::State { ts, .. }) => {
                            let maintenant = base.elapsed().as_nanos() as u64;
                            lat.ajouter(maintenant.saturating_sub(ts));
                            recus += 1;
                        }
                        Ok(_) => {}
                        Err(RecvError::Lagged(n)) => perdus += n,
                        Err(RecvError::Closed) => break,
                    }
                }
                (recus, perdus, lat, cpu_fil_ms().zip(cpu0).map(|(a, b)| a - b))
            })
            .unwrap();
        let editeur = std::thread::Builder::new()
            .name("editeur".into())
            .spawn_scoped(s, || {
                let tx = tx;
                let cpu0 = cpu_fil_ms();
                let mut retards: Vec<u64> = Vec::new();
                let mut i = 0u64;
                let emettre = |i: u64, t: u64| {
                    let _ = tx.send(Event::State { point: ids[(i % 64) as usize].clone(), value: Value::Float(21.5), ts: t, source_ts: None });
                };
                if taux > 0 {
                    let periode = 1_000_000_000 / taux;
                    let total = taux * secs;
                    while i < total {
                        let maintenant = base.elapsed().as_nanos() as u64;
                        let du = (maintenant / periode + 1).min(total);
                        if i >= du {
                            let attente = (i * periode).saturating_sub(maintenant);
                            std::thread::sleep(Duration::from_nanos(attente.min(50_000)));
                            continue;
                        }
                        while i < du {
                            let t = base.elapsed().as_nanos() as u64;
                            retards.push(t.saturating_sub(i * periode));
                            emettre(i, t);
                            i += 1;
                        }
                    }
                } else {
                    while base.elapsed() < Duration::from_secs(secs) {
                        let t = base.elapsed().as_nanos() as u64;
                        emettre(i, t);
                        i += 1;
                    }
                }
                envoyes.store(i, Release);
                (retards, cpu_fil_ms().zip(cpu0).map(|(a, b)| a - b))
                // `tx` est libéré ici : l'abonné reçoit `Closed` après avoir tout lu.
            })
            .unwrap();
        let (retards, cpu_ed) = editeur.join().unwrap();
        let (recus, perdus, lat, cpu_ab) = abonne.join().unwrap();
        (recus, perdus, lat, cpu_ab, retards, cpu_ed)
    });
    let envoyes = envoyes.load(Acquire);
    let mut retards = retards;
    retards.sort_unstable();
    Palier {
        taux,
        secs,
        envoyes,
        recus,
        perdus,
        reel_s: secs as f64,
        lat,
        retard_editeur_p99: percentile(&retards, 0.99),
        cpu_routeur: None,
        cpu_editeur: cpu_ed,
        cpu_abonne: cpu_ab,
        rss_avant,
        rss_apres: rss(&systeme()),
    }
}

pub fn bus(secs: u64) {
    let ids: Vec<PointId> = (0..64).map(|k| PointId::from(format!("z2m:0x00124b00aaaa{k:04x}/temperature"))).collect();
    println!("- `tokio::sync::broadcast::channel::<moli_core::Event>(1024)` (capacité du bus du hub, `HubOptions::event_capacity`), un abonné ; `Event::State {{ point, value, ts, source_ts }}` cloné à la réception (Arc), sans sérialisation ; `size_of::<Event>()` = {} o", std::mem::size_of::<Event>());
    println!("- même éditeur (salves, sommeil de 50 µs au plus) et même mesure de latence que pour les liens locaux (`ts` = ns de l'envoi) ; `perdus` = messages écrasés pour un abonné en retard (`RecvError::Lagged`)\n");
    en_tete_table();
    let mut w = palier_bus(1_000, 1, &ids);
    std::hint::black_box(&mut w);
    for taux in [1_000, 5_000, 10_000, 20_000, 50_000, 0] {
        let mut p = palier_bus(taux, secs, &ids);
        ligne(&mut p);
    }
}

// ---- croissance mémoire selon le volume publié ---------------------------------------------------------------------

/// Publie sans arrêt vers un abonné `moli/#` qui consomme tout, et relève la mémoire du processus : le journal de
/// validation (« commit log ») de rumqttd garde les messages jusqu'à `max_segment_size × max_segment_count` par filtre.
pub fn volume(octets_segment: u64, segments: u64, plafond_kio: u64, max_messages: u64) {
    let broker = Broker::new(config(octets_segment, segments, 200));
    let (mut stx, mut srx) = broker.link("abonne").expect("lien abonné");
    let (mut ptx, _prx) = broker.link("editeur").expect("lien éditeur");
    stx.subscribe("moli/#").unwrap();
    loop {
        if let Ok(Some(Notification::DeviceAck(_))) = srx.recv_deadline(Instant::now() + Duration::from_secs(5)) {
            break;
        }
    }
    let modele = Modele::new();
    let topics = topics();
    let recus = AtomicU64::new(0);
    let arret = AtomicBool::new(false);
    println!("\n### segments {octets_segment} o × {segments} (comptabilité du journal : 4 + topic + charge par message), message = topic 49 o + charge {CHARGE} o");
    println!("| messages publiés | volume comptable | RssAnon (Kio) | RssAnon par message en mémoire (o) |");
    println!("|---|---|---|---|");
    let base_rss = rss(&systeme()).unwrap_or(0);
    std::thread::scope(|s| {
        s.spawn(|| {
            while !arret.load(Acquire) {
                match srx.recv_deadline(Instant::now() + Duration::from_millis(20)) {
                    Ok(Some(Notification::Forward(_))) => {
                        recus.fetch_add(1, Relaxed);
                    }
                    Ok(Some(Notification::Unschedule)) => {
                        let _ = srx.ready();
                    }
                    _ => {}
                }
            }
        });
        let mut i = 0u64;
        let mut prochain = 25_000u64;
        let mut arrete_pour_memoire = false;
        while i < max_messages && !arrete_pour_memoire {
            ptx.publish(topics[(i % 64) as usize].clone(), modele.fabriquer(i, 0)).expect("publication");
            i += 1;
            if i % 1_000 == 0 {
                // Laisse l'abonné suivre : on ne mesure pas un retard de lecture.
                while i.saturating_sub(recus.load(Relaxed)) > 5_000 {
                    std::thread::sleep(Duration::from_micros(200));
                }
            }
            if i == prochain {
                std::thread::sleep(Duration::from_millis(300));
                let r = rss(&systeme()).unwrap_or(0);
                println!("| {i} | {:.1} Mio | {r} | {:.0} |", (i * (4 + 49 + CHARGE as u64)) as f64 / 1_048_576.0, (r.saturating_sub(base_rss)) as f64 * 1024.0 / i as f64);
                if r > plafond_kio {
                    arrete_pour_memoire = true;
                }
                prochain = if prochain < 200_000 { prochain * 2 } else { prochain + 200_000 };
            }
        }
        arret.store(true, Release);
        if arrete_pour_memoire {
            println!("\nArrêt volontaire à {i} messages : RssAnon a dépassé {plafond_kio} Kio (le conteneur serait tué par l'OOM à 131 072 Kio).");
        }
    });
    println!("(RssAnon au départ, courtier et liens créés : {base_rss} Kio ; lignes `kio` : {})", kio(&systeme(), "VmHWM"));
}


// ---- mémoire du journal sous trafic soutenu, puis seuil de retard (réglage 64 Kio) -----------------------------------

fn topic_val(i: u64) -> Bytes {
    Bytes::from(format!("moli/v1/val/z2m:0x00124b00{:08x}/{}", i / 5 % 1_000_000, ["temperature", "humidity", "battery", "linkquality", "state"][(i % 5) as usize]))
}

/// ~200 messages/s pendant `secs` s sur `moli/v1/val/…` (2 264 points distincts), 4 abonnés en mémoire à `moli/v1/val/#`
/// qui lisent normalement. Relève RssAnon avant, toutes les 10 s, puis après.
pub fn soutenu(octets_segment: u64, segments: u64, secs: u64, taux: u64) {
    let broker = Broker::new(config(octets_segment, segments, 200));
    let mut abonnes: Vec<(LinkTx, LinkRx)> = (0..4).map(|k| broker.link(&format!("abonne-{k}")).expect("lien abonné")).collect();
    let (mut ptx, _prx) = broker.link("editeur").expect("lien éditeur");
    for (tx, rx) in &mut abonnes {
        tx.subscribe("moli/v1/val/#").unwrap();
        loop {
            if let Ok(Some(Notification::DeviceAck(_))) = rx.recv_deadline(Instant::now() + Duration::from_secs(5)) {
                break;
            }
        }
    }
    let modele = Modele::de_taille(115);
    std::thread::sleep(Duration::from_millis(500));
    let avant = rss(&systeme()).unwrap_or(0);
    let recus: Vec<AtomicU64> = (0..4).map(|_| AtomicU64::new(0)).collect();
    let fini = AtomicBool::new(false);
    let envoyes = AtomicU64::new(0);
    println!("\n### segments {octets_segment} o × {segments} ({} Kio de journal comptable par filtre au plus) · {taux} messages/s pendant {secs} s · charge 115 o · topics `moli/v1/val/z2m:0x…/<clé>` (≈ 47 o), 2 264 points distincts · 4 abonnés en mémoire à `moli/v1/val/#`", octets_segment * segments / 1024);
    println!("| t (s) | messages publiés | RssAnon (Kio) | écart avec le départ (Kio) |");
    println!("|---|---|---|---|");
    println!("| 0 | 0 | {avant} | 0 |");
    let mut apres = avant;
    std::thread::scope(|s| {
        for (k, (tx, rx)) in abonnes.iter_mut().enumerate() {
            let _ = tx;
            let (recus, fini, envoyes) = (&recus, &fini, &envoyes);
            s.spawn(move || {
                let mut dernier = Instant::now();
                loop {
                    match rx.recv_deadline(Instant::now() + Duration::from_millis(50)) {
                        Ok(Some(Notification::Forward(_))) => {
                            recus[k].fetch_add(1, Relaxed);
                            dernier = Instant::now();
                        }
                        Ok(Some(Notification::Unschedule)) => {
                            let _ = rx.ready();
                        }
                        _ => {}
                    }
                    if fini.load(Acquire) && (recus[k].load(Relaxed) >= envoyes.load(Acquire) || dernier.elapsed() > Duration::from_secs(3)) {
                        break;
                    }
                }
            });
        }
        let debut = Instant::now();
        let periode = Duration::from_micros(1_000_000 / taux);
        let total = taux * secs;
        let mut prochain_releve = 10u64;
        for i in 0..total {
            let cible = debut + periode * (i as u32);
            let maintenant = Instant::now();
            if cible > maintenant {
                std::thread::sleep(cible - maintenant);
            }
            ptx.publish(topic_val(i), modele.fabriquer(i, 0)).expect("publication");
            envoyes.store(i + 1, Release);
            if debut.elapsed().as_secs() >= prochain_releve && prochain_releve < secs {
                let r = rss(&systeme()).unwrap_or(0);
                println!("| {prochain_releve} | {} | {r} | {:+} |", i + 1, r as i64 - avant as i64);
                prochain_releve += 10;
            }
        }
        envoyes.store(total, Release);
        fini.store(true, Release);
    });
    std::thread::sleep(Duration::from_millis(500));
    let r = rss(&systeme()).unwrap_or(0);
    apres = apres.max(r);
    println!("| {secs} (fin) | {} | {r} | {:+} |", envoyes.load(Relaxed), r as i64 - avant as i64);
    let lus: Vec<u64> = recus.iter().map(|c| c.load(Relaxed)).collect();
    println!("\nRssAnon avant → après : **{avant} → {r} Kio ({:+} Kio)** ; messages reçus par chacun des 4 abonnés : {:?} (publiés : {}) ; mémoire par message publié encore en journal : {:.0} o ; pic VmHWM : {} Kio", r as i64 - avant as i64, lus, envoyes.load(Relaxed), (r as i64 - avant as i64) as f64 * 1024.0 / envoyes.load(Relaxed).max(1) as f64, kio(&systeme(), "VmHWM"));
    let _ = apres;
}

/// Configuration avec un écouteur inutilisé (nécessaire à `Broker::start`) et l'envoi des alertes du routeur chaque seconde.
/// Les deux métriques (`alerts` et `meters`) doivent être déclarées : avec `alerts` seul, le fil `timer` de rumqttd 0.20.0
/// panique (`link/timer.rs:40`, `unwrap()` sur la métrique absente évalué avant la précondition du `select!`) et aucune alerte ne part.
fn config_alertes(octets_segment: u64, segments: u64) -> Config {
    serde_json::from_value(json!({
        "id": 0,
        "router": { "id": 0, "max_connections": 10010, "max_outgoing_packet_count": 200, "max_segment_size": octets_segment, "max_segment_count": segments },
        "v4": { "1": { "name": "v4-1", "listen": "127.0.0.1:0", "next_connection_delay_ms": 1,
                       "connections": { "connection_timeout_ms": 60000, "max_payload_size": 20480, "max_inflight_count": 100, "dynamic_filters": false } } },
        "metrics": { "alerts": { "push_interval": 1 }, "meters": { "push_interval": 3600 } }
    }))
    .expect("configuration rumqttd avec alertes")
}

struct Essai {
    l: u64,
    recus: u64,
    manquants: Vec<(u64, u64)>, // plages [a, b] de séquences manquantes
    forward: u64,
    unschedule: u64,
    autres: u64,
    ordre: bool,
    alertes: Vec<String>,
}

/// Un abonné `moli/v1/val/#` qui ne lit pas pendant que `l` messages sont publiés (le plus vite possible), puis lit tout.
fn essai_seuil(octets_segment: u64, segments: u64, l: u64, phase: u64, avec_alertes: bool, modele: &Modele) -> Essai {
    let broker = Broker::new(config_alertes(octets_segment, segments));
    let (mut stx, mut srx) = broker.link("en-retard").expect("lien");
    let (mut ttx, mut trx) = broker.link("temoin").expect("lien");
    let (mut ptx, _prx) = broker.link("editeur").expect("lien");
    let alertes = broker.alerts().expect("lien d'alertes");
    let mut b = broker;
    std::thread::Builder::new().name("courtier".into()).spawn(move || {
        let _ = b.start();
    }).unwrap();
    if phase > 0 {
        // Un témoin crée le filtre, puis `phase` messages décalent le remplissage du segment actif avant que l'abonné s'abonne.
        ttx.subscribe("moli/v1/val/#").unwrap();
        loop {
            if let Ok(Some(Notification::DeviceAck(_))) = trx.recv_deadline(Instant::now() + Duration::from_secs(5)) {
                break;
            }
        }
        for i in 0..phase {
            ptx.publish(topic_val(i), modele.fabriquer(i, 0)).expect("publication");
            if i % 200 == 199 {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    stx.subscribe("moli/v1/val/#").unwrap();
    loop {
        if let Ok(Some(Notification::DeviceAck(_))) = srx.recv_deadline(Instant::now() + Duration::from_secs(5)) {
            break;
        }
    }
    // L'abonné ne lit plus : les messages s'accumulent dans le journal (et 199 dans sa file de sortie).
    for i in 0..l {
        ptx.publish(topic_val(i), modele.fabriquer(i, 0)).expect("publication");
        if i % 200 == 199 {
            std::thread::sleep(Duration::from_millis(2)); // laisse le routeur suivre : on mesure le retard, pas la saturation de l'entrée
        }
    }
    std::thread::sleep(Duration::from_millis(400));
    // Reprise : il lit tout ce qu'on lui livre, et rend la main au routeur (`ready`) quand celui-ci le « déprogramme ».
    let (mut seqs, mut forward, mut unschedule, mut autres) = (Vec::new(), 0u64, 0u64, 0u64);
    let mut dernier = Instant::now();
    while dernier.elapsed() < Duration::from_millis(600) {
        match srx.recv_deadline(Instant::now() + Duration::from_millis(50)) {
            Ok(Some(Notification::Forward(f))) => {
                seqs.push(modele.lire_seq(&f.publish.payload));
                forward += 1;
                dernier = Instant::now();
            }
            Ok(Some(Notification::Unschedule)) => {
                unschedule += 1;
                let _ = srx.ready();
                dernier = Instant::now();
            }
            Ok(Some(_)) => {
                autres += 1;
                dernier = Instant::now();
            }
            _ => {}
        }
    }
    let mut vus = vec![false; l as usize];
    for &q in &seqs {
        if (q as usize) < vus.len() {
            vus[q as usize] = true;
        }
    }
    let mut manquants = Vec::new();
    let mut debut: Option<u64> = None;
    for (i, v) in vus.iter().enumerate() {
        match (v, debut) {
            (false, None) => debut = Some(i as u64),
            (true, Some(a)) => {
                manquants.push((a, i as u64 - 1));
                debut = None;
            }
            _ => {}
        }
    }
    if let Some(a) = debut {
        manquants.push((a, l - 1));
    }
    // Les alertes du routeur arrivent par paquets, une fois par seconde.
    let mut textes = Vec::new();
    let limite = Instant::now() + if avec_alertes { Duration::from_millis(2500) } else { Duration::ZERO };
    while Instant::now() < limite {
        if let Ok(a) = alertes.router_rx.recv_timeout(Duration::from_millis(200)) {
            for al in a {
                textes.push(format!("{} (client `{}`, {})", al.kind.name(), al.client_id, al.kind.description()));
            }
        }
    }
    Essai { l, recus: seqs.len() as u64, manquants, forward, unschedule, autres, ordre: seqs.windows(2).all(|w| w[0] < w[1]), alertes: textes }
}

pub fn seuil(octets_segment: u64, segments: u64, liste: &[u64]) {
    let modele = Modele::de_taille(115);
    let comptable = 4 + topic_val(0).len() as u64 + 115;
    println!("\n### seuil de retard · segments {octets_segment} o × {segments} · message comptable = 4 + topic ({} o) + charge 115 o = {comptable} o → {} messages par segment, journal retenu entre {} et {} messages ; l'abonné garde de surcroît 199 notifications déjà poussées dans sa file de sortie", topic_val(0).len(), octets_segment.div_ceil(comptable), octets_segment.div_ceil(comptable) * (segments - 1), octets_segment.div_ceil(comptable) * segments);
    println!("| messages publiés pendant que l'abonné ne lit pas | reçus après reprise | **perdus** | séquences manquantes | ordre | notifications reçues (Forward / Unschedule / autres) | alertes du routeur |");
    println!("|---|---|---|---|---|---|---|");
    for &l in liste {
        let e = essai_seuil(octets_segment, segments, l, 0, true, &modele);
        let plages = if e.manquants.is_empty() { "—".to_owned() } else { e.manquants.iter().map(|(a, b)| if a == b { format!("{a}") } else { format!("{a}–{b}") }).collect::<Vec<_>>().join(", ") };
        let alertes = if e.alertes.is_empty() { "aucune".to_owned() } else { format!("{} : {}", e.alertes.len(), e.alertes[0]) };
        println!("| {} | {} | **{}** | {plages} | {} | {} / {} / {} | {alertes} |", e.l, e.recus, e.l - e.recus, if e.ordre { "conservé" } else { "rompu" }, e.forward, e.unschedule, e.autres);
    }
}

/// Seuil de perte en fonction de la phase du journal : pour chaque `phase` (messages déjà publiés avant l'abonnement de
/// l'abonné en retard), plus petit L qui fait perdre au moins un message (recherche par dichotomie).
pub fn seuil_phase(octets_segment: u64, segments: u64, phases: &[u64]) {
    let modele = Modele::de_taille(115);
    println!("\n### seuil de retard selon la phase du journal · segments {octets_segment} o × {segments}");
    println!("| messages déjà publiés avant l'abonnement (phase) | **plus petit L avec perte** | perdus à ce L | dernier L sans perte | pertes à L + 200 |");
    println!("|---|---|---|---|---|");
    for &phase in phases {
        let (mut bas, mut haut) = (200u64, 2_600u64); // bas : sans perte ; haut : avec perte
        let perdus = |l: u64| {
            let e = essai_seuil(octets_segment, segments, l, phase, false, &modele);
            e.l - e.recus
        };
        while haut - bas > 1 {
            let milieu = (bas + haut) / 2;
            if perdus(milieu) == 0 { bas = milieu } else { haut = milieu }
        }
        let (p_haut, p_plus) = (perdus(haut), perdus(haut + 200));
        println!("| {phase} | **{haut}** | {p_haut} | {bas} | {p_plus} |");
    }
}
`````

Code complet — `preuves-v5/courtier/src/tcp.rs` :

`````rust
//! Bancs TCP : le courtier rumqttd dans son propre processus (écouteur v4), des clients `rumqttc` dans un autre
//! processus (ou conteneur). Scénarios : mémoire selon les messages retenus, abonnés à `moli/#`, rafale avec un
//! abonné lent ou bloqué.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::*};
use std::time::{Duration, Instant};

use rumqttc::{AsyncClient, Event, MqttOptions, NetworkOptions, Outgoing, Packet, QoS};
use serde_json::{Map, Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Notify;

use crate::systeme::{kio, rss, systeme, val};

// =============================================================================================
// Le courtier (processus serveur)
// =============================================================================================

pub struct ParamsCourtier {
    pub port: u16,
    pub controle: u16,
    pub octets_segment: u64,
    pub segments: u64,
    pub dynamic_filters: bool,
    pub max_sortie: u64,
}

fn configuration(p: &ParamsCourtier) -> rumqttd::Config {
    serde_json::from_value(json!({
        "id": 0,
        "router": { "id": 0, "max_connections": 10010, "max_outgoing_packet_count": p.max_sortie, "max_segment_size": p.octets_segment, "max_segment_count": p.segments },
        "v4": { "1": {
            "name": "v4-1",
            "listen": format!("0.0.0.0:{}", p.port),
            "next_connection_delay_ms": 1,
            "connections": { "connection_timeout_ms": 60000, "max_payload_size": 20480, "max_inflight_count": 100, "dynamic_filters": p.dynamic_filters }
        } }
    }))
    .expect("configuration rumqttd")
}

pub fn courtier(p: ParamsCourtier) {
    let mut b = rumqttd::Broker::new(configuration(&p));
    std::thread::Builder::new()
        .name("courtier".into())
        .spawn(move || {
            if let Err(e) = b.start() {
                eprintln!("courtier arrêté : {e}");
            }
        })
        .unwrap();
    eprintln!(
        "courtier rumqttd 0.20.0 sur :{} (segments {} o × {}, dynamic_filters={}, max_outgoing_packet_count={}), contrôle :{}, nofile {:?}",
        p.port, p.octets_segment, p.segments, p.dynamic_filters, p.max_sortie, p.controle, systeme().get("nofile")
    );
    let l = TcpListener::bind(("0.0.0.0", p.controle)).unwrap();
    for s in l.incoming().flatten() {
        let mut s = s;
        let mut tampon = [0u8; 1024];
        let _ = s.read(&mut tampon);
        let corps = serde_json::to_string(&systeme()).unwrap();
        let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{corps}", corps.len());
    }
}

// =============================================================================================
// Clients
// =============================================================================================

async fn mesure(addr: SocketAddr) -> Map<String, Value> {
    let Ok(mut s) = tokio::net::TcpStream::connect(addr).await else { return Map::new() };
    if s.write_all(b"GET /mesure HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").await.is_err() {
        return Map::new();
    }
    let mut tout = Vec::new();
    let _ = s.read_to_end(&mut tout).await;
    let texte = String::from_utf8_lossy(&tout);
    match texte.split_once("\r\n\r\n").and_then(|(_, c)| serde_json::from_str::<Value>(c).ok()) {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

async fn attendre_courtier(ctl: SocketAddr) -> Map<String, Value> {
    let debut = Instant::now();
    loop {
        let m = mesure(ctl).await;
        if !m.is_empty() {
            return m;
        }
        assert!(debut.elapsed() < Duration::from_secs(30), "courtier injoignable");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn options(id: &str, port: u16) -> MqttOptions {
    let mut o = MqttOptions::new(id, "127.0.0.1", port);
    o.set_keep_alive(Duration::from_secs(30)); // comme le pilote Z2M de Moli
    o.set_max_packet_size(256 * 1024, 256 * 1024);
    o
}

fn qos(n: u8) -> QoS {
    if n == 0 { QoS::AtMostOnce } else { QoS::AtLeastOnce }
}

#[derive(Default)]
struct Compteurs {
    abonnes: AtomicUsize,
    recus: AtomicU64,
    connexions: AtomicU64,
    acquittes: AtomicU64,
    sortis: AtomicU64,
}

/// Un abonné `rumqttc` qui compte ce qu'il reçoit ; `seq` relève les numéros de séquence des charges de la rafale.
async fn abonne(id: String, port: u16, filtre: &'static str, q: u8, c: Arc<Compteurs>, seqs: Option<Arc<std::sync::Mutex<Vec<u64>>>>) {
    let (client, mut boucle) = AsyncClient::new(options(&id, port), 64);
    loop {
        match boucle.poll().await {
            Ok(Event::Incoming(Packet::ConnAck(_))) => {
                c.connexions.fetch_add(1, Relaxed);
                let _ = client.try_subscribe(filtre, qos(q));
            }
            Ok(Event::Incoming(Packet::SubAck(_))) => {
                c.abonnes.fetch_add(1, Relaxed);
            }
            Ok(Event::Incoming(Packet::Publish(p))) => {
                c.recus.fetch_add(1, Relaxed);
                if let Some(seqs) = &seqs {
                    seqs.lock().unwrap().push(lire_seq(&p.payload));
                }
            }
            Ok(_) => {}
            Err(_) => tokio::time::sleep(Duration::from_millis(200)).await,
        }
    }
}

/// Un éditeur : compte les accusés (QoS 1) et les envois (`Outgoing::Publish`).
async fn editeur(id: &str, port: u16, c: Arc<Compteurs>) -> AsyncClient {
    let mut o = options(id, port);
    o.set_inflight(100);
    let (client, mut boucle) = AsyncClient::new(o, 1024);
    tokio::spawn(async move {
        loop {
            match boucle.poll().await {
                Ok(Event::Incoming(Packet::PubAck(_))) => {
                    c.acquittes.fetch_add(1, Relaxed);
                }
                Ok(Event::Outgoing(Outgoing::Publish(_))) => {
                    c.sortis.fetch_add(1, Relaxed);
                }
                Ok(_) => {}
                Err(_) => tokio::time::sleep(Duration::from_millis(200)).await,
            }
        }
    });
    client
}

const CLES: [&str; 5] = ["temperature", "humidity", "battery", "linkquality", "state"];

/// Topic d'un point : `moli/` + identifiant de point (`z2m:0x…/clé`), 5 points par appareil.
fn topic_point(i: usize) -> String {
    format!("moli/z2m:0x00124b00{:08x}/{}", i / 5, CLES[i % 5])
}

/// Valeur d'un point (~150 o) : identifiant, valeur, horodatage, unité, état de liaison, complété à 150 o.
fn charge_point(i: usize) -> String {
    let mut s = format!(
        "{{\"point\":\"z2m:0x00124b00{:08x}/{}\",\"value\":{}.5,\"ts\":1792675795123,\"unit\":\"°C\",\"online\":true",
        i / 5,
        CLES[i % 5],
        20 + i % 10
    );
    let fin = r#","q":""}"#;
    let manque = 150usize.saturating_sub(s.len() + fin.len());
    s.push_str(r#","q":""#);
    s.push_str(&"x".repeat(manque));
    s.push_str(r#""}"#);
    s
}

/// Charge de la rafale : 120 o, `seq` à 9 chiffres.
fn charge_rafale(seq: u64) -> String {
    let mut s = format!("{{\"point\":\"z2m:0x00124b00aaaa0001/temperature\",\"value\":21.5,\"ts\":1792675795123,\"seq\":{seq:09}");
    let fin = r#","q":""}"#;
    let manque = 120usize.saturating_sub(s.len() + fin.len());
    s.push_str(r#","q":""#);
    s.push_str(&"x".repeat(manque));
    s.push_str(r#""}"#);
    s
}

fn lire_seq(charge: &[u8]) -> u64 {
    let t = String::from_utf8_lossy(charge);
    t.split("\"seq\":").nth(1).and_then(|r| r.get(..9)).and_then(|d| d.parse().ok()).unwrap_or(u64::MAX)
}

/// Publie des messages retenus pour les points `de..a` (QoS 1) et attend les accusés.
async fn publier_retenus(ed: &AsyncClient, c: &Compteurs, de: usize, a: usize) -> Duration {
    let t = Instant::now();
    let avant = c.acquittes.load(Relaxed);
    for i in de..a {
        ed.publish(topic_point(i), QoS::AtLeastOnce, true, charge_point(i)).await.unwrap();
    }
    let limite = Instant::now() + Duration::from_secs(60);
    while c.acquittes.load(Relaxed) - avant < (a - de) as u64 && Instant::now() < limite {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    t.elapsed()
}

fn delta_rss(a: &Map<String, Value>, b: &Map<String, Value>) -> Option<i64> {
    rss(b).zip(rss(a)).map(|(y, x)| y as i64 - x as i64)
}

fn cpu(a: &Map<String, Value>, b: &Map<String, Value>) -> Option<u64> {
    val(b, "cpu_ms").zip(val(a, "cpu_ms")).map(|(y, x)| y.saturating_sub(x))
}

fn ligne_phase(nom: &str, m: &Map<String, Value>) {
    println!("| {nom} | {} | {} | {} | {} | {} | {} | {} |", rss(m).map_or("—".into(), |v| v.to_string()), kio(m, "VmRSS"), kio(m, "VmHWM"), kio(m, "Threads"), kio(m, "fds"), kio(m, "cg_slab"), kio(m, "cg_current"));
}

fn tete_phases() {
    println!("| phase (mesurée par le courtier sur lui-même, Kio) | RssAnon | VmRSS | VmHWM (pic) | fils | descripteurs | cgroup `slab` | cgroup `memory.current` |");
    println!("|---|---|---|---|---|---|---|---|");
}

// ---------------------------------------------------------------------------------------------
// Scénario 1 : mémoire selon les messages retenus
// ---------------------------------------------------------------------------------------------

/// Combien de messages retenus un nouvel abonné `moli/#` reçoit-il ? (QoS 0 puis QoS 1)
async fn compter_retenus_recus(port: u16, ctl: SocketAddr, q: u8, nom: &str) -> (u64, Map<String, Value>) {
    let c = Arc::new(Compteurs::default());
    let t = tokio::spawn(abonne(format!("test-retenus-{nom}"), port, "moli/#", q, c.clone(), None));
    let limite = Instant::now() + Duration::from_secs(20);
    let (mut dernier, mut vu) = (Instant::now(), 0u64);
    while Instant::now() < limite {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let n = c.recus.load(Relaxed);
        if n != vu {
            vu = n;
            dernier = Instant::now();
        }
        if c.abonnes.load(Relaxed) > 0 && dernier.elapsed() > Duration::from_millis(1500) {
            break;
        }
    }
    let m = mesure(ctl).await;
    t.abort();
    tokio::time::sleep(Duration::from_millis(500)).await;
    (c.recus.load(Relaxed), m)
}

pub async fn memoire(port: u16, controle: u16, catchall: bool) {
    let ctl: SocketAddr = ([127, 0, 0, 1], controle).into();
    let au_repos = attendre_courtier(ctl).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let au_repos = {
        let m = mesure(ctl).await;
        if m.is_empty() { au_repos } else { m }
    };
    // Option : un abonné « attrape-tout » `moli/#` (l'enregistreur d'historique de Moli, par exemple) déjà présent
    // pendant la pose des retenus : les topics correspondent alors à un filtre existant.
    let c0 = Arc::new(Compteurs::default());
    let t0 = if catchall { Some(tokio::spawn(abonne("attrape-tout".into(), port, "moli/#", 0, c0.clone(), None))) } else { None };
    if catchall {
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    let avec_abonne = mesure(ctl).await;
    let ce = Arc::new(Compteurs::default());
    let ed = editeur("editeur-retenus", port, ce.clone()).await;
    // Trois niveaux : une maison (2 264 valeurs), trois maisons (×3) et dix maisons (×10).
    let niveaux = [2_264usize, 6_792, 22_640];
    let mut mesures: Vec<(usize, Duration, Map<String, Value>, u64, u64)> = Vec::new();
    let mut pose = 0usize;
    for &niveau in &niveaux {
        let d = publier_retenus(&ed, &ce, pose, niveau).await;
        pose = niveau;
        tokio::time::sleep(Duration::from_secs(2)).await;
        let m = mesure(ctl).await;
        let (q0, _) = compter_retenus_recus(port, ctl, 0, &format!("q0-{niveau}")).await;
        let (q1, _) = compter_retenus_recus(port, ctl, 1, &format!("q1-{niveau}")).await;
        mesures.push((niveau, d, m, q0, q1));
    }
    let _ = ed.disconnect().await;
    if let Some(t) = &t0 {
        t.abort();
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    let ferme = mesure(ctl).await;

    println!("\n### mémoire du courtier selon les messages retenus · abonné attrape-tout `moli/#` présent : {}", if catchall { "oui (QoS 0)" } else { "non" });
    println!("- un message retenu = un point : topic `moli/z2m:0x00124b00<appareil>/<clé>` (≈ 45 o), charge JSON de 150 o, QoS 1, retain ; posés par un `rumqttc` d'un autre processus ; `ulimit -n` du courtier : {}", au_repos.get("nofile").and_then(Value::as_str).unwrap_or("?"));
    println!("- niveaux : 2 264 (une maison), 6 792 (×3), 22 640 (×10) ; acquittés : {}/22 640\n", ce.acquittes.load(Relaxed));
    tete_phases();
    ligne_phase("au repos", &au_repos);
    if catchall {
        ligne_phase("avec l'abonné attrape-tout connecté", &avec_abonne);
    }
    for (niveau, _, m, _, _) in &mesures {
        ligne_phase(&format!("avec {niveau} retenus"), m);
    }
    ligne_phase("après fermeture des clients (les retenus restent)", &ferme);
    let base = if catchall { &avec_abonne } else { &au_repos };
    println!("\n| niveau | pose (durée) | RssAnon ajouté depuis le départ (Kio) | **octets par retenu (cumul)** | ajouté depuis le niveau précédent (Kio) | octets par retenu (incrément) |");
    println!("|---|---|---|---|---|---|");
    let mut precedent: (&Map<String, Value>, usize) = (base, 0);
    for (niveau, d, m, _, _) in &mesures {
        let cumul = delta_rss(base, m);
        let incr = delta_rss(precedent.0, m);
        println!(
            "| {niveau} | {} | {} | **{}** | {} | {} |",
            duree_s(*d),
            cumul.map_or("—".into(), |v| v.to_string()),
            cumul.map_or("—".into(), |v| format!("{:.0}", v as f64 * 1024.0 / *niveau as f64)),
            incr.map_or("—".into(), |v| v.to_string()),
            incr.map_or("—".into(), |v| format!("{:.0}", v as f64 * 1024.0 / (*niveau - precedent.1) as f64)),
        );
        precedent = (m, *niveau);
    }
    println!("\nMessages retenus reçus par un NOUVEL abonné `moli/#` (attendus : tous les retenus en place) :\n");
    println!("| retenus en place | abonnement QoS 0 | abonnement QoS 1 |");
    println!("|---|---|---|");
    for (niveau, _, _, q0, q1) in &mesures {
        println!("| {niveau} | **{q0}** | **{q1}** |");
    }
    println!("RESULTAT {}", json!({ "scenario": "memoire", "catchall": catchall, "au_repos": au_repos, "avec_abonne": avec_abonne, "niveaux": mesures.iter().map(|(n, d, m, q0, q1)| json!({"n": n, "pose_s": d.as_secs_f64(), "mesure": m, "recus_q0": q0, "recus_q1": q1})).collect::<Vec<_>>(), "ferme": ferme }));
}

fn duree_s(d: Duration) -> String {
    format!("{:.2} s", d.as_secs_f64())
}

// ---------------------------------------------------------------------------------------------
// Scénario 2 : N abonnés à `moli/#`, 10 messages/s pendant 10 s
// ---------------------------------------------------------------------------------------------

pub async fn abonnes(port: u16, controle: u16, n: usize, retenus: usize, q: u8, hz: u64, secs: u64) {
    let ctl: SocketAddr = ([127, 0, 0, 1], controle).into();
    attendre_courtier(ctl).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let au_repos = mesure(ctl).await;
    let ce = Arc::new(Compteurs::default());
    let ed = editeur("editeur", port, ce.clone()).await;
    if retenus > 0 {
        publier_retenus(&ed, &ce, 0, retenus).await;
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    let avec_retenus = mesure(ctl).await;
    // N abonnés : des tâches rumqttc ; chacun compte ses retenus puis ses diffusions.
    let c = Arc::new(Compteurs::default());
    let t0 = Instant::now();
    let taches: Vec<_> = (0..n).map(|i| tokio::spawn(abonne(format!("abonne-{i}"), port, "moli/#", q, c.clone(), None))).collect();
    let limite = Instant::now() + Duration::from_secs(120);
    while c.abonnes.load(Relaxed) < n && Instant::now() < limite {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let temps_abonnement = t0.elapsed();
    // Laisse les retenus arriver (on attend la stabilisation du compteur).
    let (mut vu, mut dernier) = (0u64, Instant::now());
    while Instant::now() < limite {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let r = c.recus.load(Relaxed);
        if r != vu {
            vu = r;
            dernier = Instant::now();
        }
        if dernier.elapsed() > Duration::from_secs(3) {
            break;
        }
    }
    let temps_retenus = t0.elapsed();
    let retenus_recus = c.recus.load(Relaxed);
    let apres_abonnes = mesure(ctl).await;
    // Inactif 5 s puis diffusion.
    tokio::time::sleep(Duration::from_secs(5)).await;
    let diff_debut = mesure(ctl).await;
    let (recus_avant, t_diff) = (c.recus.load(Relaxed), Instant::now());
    let total = hz * secs;
    let mut tic = tokio::time::interval(Duration::from_micros(1_000_000 / hz));
    for k in 0..total {
        tic.tick().await;
        ed.publish(topic_point((k % 64) as usize), QoS::AtMostOnce, false, charge_rafale(k)).await.unwrap();
    }
    let attendus = total * n as u64;
    let limite2 = Instant::now() + Duration::from_secs(30);
    while c.recus.load(Relaxed) - recus_avant < attendus && Instant::now() < limite2 {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let temps_total_diff = t_diff.elapsed();
    let diff_fin = mesure(ctl).await;
    let livres = c.recus.load(Relaxed) - recus_avant;
    let _ = ed.disconnect().await;
    for t in &taches {
        t.abort();
    }
    tokio::time::sleep(Duration::from_secs(3)).await;
    let ferme = mesure(ctl).await;

    println!("\n### courtier rumqttd 0.20.0 · {n} abonnés TCP à `moli/#` (QoS {q}) · {retenus} messages retenus en place · diffusion {hz}/s × {secs} s (QoS 0, 120 o)");
    println!("- abonnés établis : {}/{n} en {} ; reconnexions (connexions − abonnés) : {} ; `ulimit -n` du courtier : {}", c.abonnes.load(Relaxed), duree_s(temps_abonnement), c.connexions.load(Relaxed) as i64 - n as i64, au_repos.get("nofile").and_then(Value::as_str).unwrap_or("?"));
    println!("- messages retenus reçus au total à l'abonnement : {retenus_recus} sur {} attendus si tout était livré ({n} × {retenus}), soit {:.1} par abonné ; stabilisés après {}", n * retenus, retenus_recus as f64 / n.max(1) as f64, duree_s(temps_retenus));
    println!();
    tete_phases();
    ligne_phase("au repos", &au_repos);
    if retenus > 0 {
        ligne_phase(&format!("avec {retenus} retenus"), &avec_retenus);
    }
    ligne_phase(&format!("avec {n} abonnés"), &apres_abonnes);
    ligne_phase("après la diffusion", &diff_fin);
    ligne_phase("après fermeture des clients", &ferme);
    let d = delta_rss(&avec_retenus, &apres_abonnes);
    println!("\nPar abonné ({n} abonnés − avant) / {n} : RssAnon **{} Kio**.", d.map_or("—".into(), |d| format!("{:.2}", d as f64 / n as f64)));
    println!("\nCPU du courtier (utime+stime) :");
    println!("- à l'abonnement de {n} clients (retenus compris) : {} ms", cpu(&avec_retenus, &apres_abonnes).map_or("—".into(), |v| v.to_string()));
    println!("- diffusion de {total} messages à {n} abonnés : **{} ms** ; {livres}/{attendus} livraisons reçues en {} ; {:.1} µs de CPU courtier par livraison", cpu(&diff_debut, &diff_fin).map_or("—".into(), |v| v.to_string()), duree_s(temps_total_diff), cpu(&diff_debut, &diff_fin).map_or(0.0, |v| v as f64 * 1000.0 / livres.max(1) as f64));
    println!("RESULTAT {}", json!({ "scenario": "abonnes", "n": n, "retenus": retenus, "qos": q, "retenus_recus": retenus_recus, "livres": livres, "attendus": attendus, "au_repos": au_repos, "avec_retenus": avec_retenus, "apres_abonnes": apres_abonnes, "diffusion": diff_fin, "ferme": ferme, "cpu_diffusion_ms": cpu(&diff_debut, &diff_fin), "cpu_abonnement_ms": cpu(&avec_retenus, &apres_abonnes) }));
}

// ---------------------------------------------------------------------------------------------
// Scénario 3 : rafale de N publications avec un abonné lent ou bloqué
// ---------------------------------------------------------------------------------------------

/// Résumé d'une suite de numéros de séquence reçus.
fn resume_seq(v: &[u64], attendus: u64) -> String {
    let mut vus = vec![false; attendus as usize];
    let mut doublons = 0;
    for &s in v {
        if (s as usize) < vus.len() {
            if vus[s as usize] {
                doublons += 1;
            }
            vus[s as usize] = true;
        }
    }
    let recus = vus.iter().filter(|b| **b).count();
    let manquants = attendus as usize - recus;
    let premier_manquant = vus.iter().position(|b| !*b);
    let dernier_manquant = vus.iter().rposition(|b| !*b);
    let ordre = v.windows(2).all(|w| w[0] < w[1]);
    format!(
        "{recus}/{attendus} reçus, {manquants} manquants{}, {doublons} doublons, ordre {}",
        match (premier_manquant, dernier_manquant) {
            (Some(a), Some(b)) => format!(" (séquences {a} à {b})"),
            _ => String::new(),
        },
        if ordre { "conservé" } else { "rompu" }
    )
}

pub async fn rafale(port: u16, controle: u16, q: u8, mode: &str, n: u64, observation_s: u64) {
    let ctl: SocketAddr = ([127, 0, 0, 1], controle).into();
    attendre_courtier(ctl).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let au_repos = mesure(ctl).await;
    // Deux abonnés rapides.
    let (cf1, cf2) = (Arc::new(Compteurs::default()), Arc::new(Compteurs::default()));
    let (sf1, sf2) = (Arc::new(std::sync::Mutex::new(Vec::new())), Arc::new(std::sync::Mutex::new(Vec::new())));
    let tf1 = tokio::spawn(abonne("rapide-1".into(), port, "moli/#", q, cf1.clone(), Some(sf1.clone())));
    let tf2 = tokio::spawn(abonne("rapide-2".into(), port, "moli/#", q, cf2.clone(), Some(sf2.clone())));
    // L'abonné lent ou bloqué : tampon de réception TCP minimal (4 Kio), pas d'accusés tant qu'il ne lit pas.
    let cs = Arc::new(Compteurs::default());
    let ss = Arc::new(std::sync::Mutex::new(Vec::new()));
    let reprise = Arc::new(Notify::new());
    let bloque_actif = Arc::new(AtomicBool::new(true));
    let ts = if mode == "aucun" {
        None
    } else {
        let (cs2, ss2, reprise2, mode2, actif2) = (cs.clone(), ss.clone(), reprise.clone(), mode.to_owned(), bloque_actif.clone());
        Some(tokio::spawn(async move {
            let (client, mut boucle) = AsyncClient::new(options("lent", port), 64);
            let mut reseau = NetworkOptions::new();
            reseau.set_tcp_recv_buffer_size(4096);
            boucle.set_network_options(reseau);
            let mut abonne_ok = false;
            loop {
                match boucle.poll().await {
                    Ok(Event::Incoming(Packet::ConnAck(_))) => {
                        cs2.connexions.fetch_add(1, Relaxed);
                        let _ = client.try_subscribe("moli/#", qos(q));
                    }
                    Ok(Event::Incoming(Packet::SubAck(_))) => {
                        cs2.abonnes.fetch_add(1, Relaxed);
                        abonne_ok = true;
                    }
                    Ok(Event::Incoming(Packet::Publish(p))) => {
                        cs2.recus.fetch_add(1, Relaxed);
                        ss2.lock().unwrap().push(lire_seq(&p.payload));
                    }
                    Ok(_) => {}
                    Err(_) => tokio::time::sleep(Duration::from_millis(200)).await,
                }
                if abonne_ok && actif2.load(Acquire) {
                    if mode2 == "bloque" {
                        // On cesse de lire la socket jusqu'à la reprise.
                        reprise2.notified().await;
                    } else {
                        // Lent : un événement toutes les 20 ms (50 messages/s).
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                }
            }
        }))
    };
    let t0 = Instant::now();
    while (cf1.abonnes.load(Relaxed) < 1 || cf2.abonnes.load(Relaxed) < 1 || (ts.is_some() && cs.abonnes.load(Relaxed) < 1)) && t0.elapsed() < Duration::from_secs(20) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
    let avant = mesure(ctl).await;
    // La rafale : n publications d'un coup.
    let ce = Arc::new(Compteurs::default());
    let ed = editeur("editeur-rafale", port, ce.clone()).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let t_rafale = Instant::now();
    for k in 0..n {
        ed.publish(topic_point((k % 100) as usize), qos(q), false, charge_rafale(k)).await.unwrap();
    }
    let t_appels = t_rafale.elapsed();
    // Attend l'envoi complet (QoS 0 : `Outgoing::Publish` ; QoS 1 : accusés du courtier).
    let limite = Instant::now() + Duration::from_secs(60);
    loop {
        let fait = if q == 0 { ce.sortis.load(Relaxed) } else { ce.acquittes.load(Relaxed) };
        if fait >= n || Instant::now() > limite {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let t_editeur = t_rafale.elapsed();
    // Les abonnés rapides ont-ils tout reçu, et quand ?
    let (mut t_f1, mut t_f2) = (None, None);
    let limite = Instant::now() + Duration::from_secs(30);
    while (t_f1.is_none() || t_f2.is_none()) && Instant::now() < limite {
        if t_f1.is_none() && cf1.recus.load(Relaxed) >= n {
            t_f1 = Some(t_rafale.elapsed());
        }
        if t_f2.is_none() && cf2.recus.load(Relaxed) >= n {
            t_f2 = Some(t_rafale.elapsed());
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let apres_rafale = mesure(ctl).await;
    // Fenêtre d'observation : le lent ou bloqué est toujours dans son état.
    tokio::time::sleep(Duration::from_secs(observation_s)).await;
    let recus_lent_pendant = cs.recus.load(Relaxed);
    let apres_observation = mesure(ctl).await;
    // Reprise : on remet l'abonné à lire à pleine vitesse, puis on compte.
    bloque_actif.store(false, Release);
    reprise.notify_one();
    let (mut vu, mut dernier) = (cs.recus.load(Relaxed), Instant::now());
    let limite = Instant::now() + Duration::from_secs(40);
    while Instant::now() < limite && dernier.elapsed() < Duration::from_secs(4) {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let r = cs.recus.load(Relaxed);
        if r != vu {
            vu = r;
            dernier = Instant::now();
        }
    }
    let apres_reprise = mesure(ctl).await;
    let (v1, v2, vs) = (sf1.lock().unwrap().clone(), sf2.lock().unwrap().clone(), ss.lock().unwrap().clone());
    let reconnexions_lent = cs.connexions.load(Relaxed).saturating_sub(1);
    tf1.abort();
    tf2.abort();
    if let Some(t) = &ts {
        t.abort();
    }
    let _ = ed.disconnect().await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let ferme = mesure(ctl).await;

    println!("\n### rafale de {n} publications QoS {q} · abonné « {mode} » · observation {observation_s} s");
    println!("- deux abonnés rapides + {} ; tous abonnés à `moli/#` en QoS {q} ; 100 topics, charge de 120 o ; `ulimit -n` du courtier : {}", if mode == "aucun" { "aucun abonné lent (témoin)".to_owned() } else if mode == "bloque" { "un abonné qui cesse de lire sa socket (tampon de réception TCP de 4 Kio, aucun accusé)".to_owned() } else { "un abonné lent (il dort 20 ms après chaque événement de son client MQTT, soit 25 à 50 messages/s ; tampon de réception TCP de 4 Kio)".to_owned() }, au_repos.get("nofile").and_then(Value::as_str).unwrap_or("?"));
    println!("- éditeur : {n} appels `publish()` rendus en {} ; {} en {} ; débit éditeur {:.0} messages/s", duree_s(t_appels), if q == 0 { "tout écrit sur la socket" } else { "tous les PUBACK reçus" }, duree_s(t_editeur), n as f64 / t_editeur.as_secs_f64());
    let t_txt = |t: Option<Duration>| t.map_or("jamais (30 s)".to_owned(), duree_s);
    println!("- abonné rapide 1 : tout reçu en {} — {} ; abonné rapide 2 : tout reçu en {} — {}", t_txt(t_f1), resume_seq(&v1, n), t_txt(t_f2), resume_seq(&v2, n));
    if mode != "aucun" {
        println!("- abonné {mode} : {recus_lent_pendant} messages reçus pendant la fenêtre de {observation_s} s ; après reprise à pleine vitesse : {} ; reconnexions : {reconnexions_lent}", resume_seq(&vs, n));
    }
    println!();
    tete_phases();
    ligne_phase("au repos (3 abonnés connectés)", &avant);
    ligne_phase("juste après la rafale", &apres_rafale);
    ligne_phase(&format!("{observation_s} s plus tard (abonné {mode} toujours dans son état)"), &apres_observation);
    ligne_phase("après la reprise", &apres_reprise);
    ligne_phase("après fermeture des clients", &ferme);
    println!("\nCPU du courtier pendant la rafale : {} ms ; mémoire (RssAnon) due à la rafale : {} Kio ; {observation_s} s plus tard : {} Kio ; après reprise : {} Kio.", cpu(&avant, &apres_rafale).map_or("—".into(), |v| v.to_string()), delta_rss(&avant, &apres_rafale).map_or("—".into(), |v| v.to_string()), delta_rss(&avant, &apres_observation).map_or("—".into(), |v| v.to_string()), delta_rss(&avant, &apres_reprise).map_or("—".into(), |v| v.to_string()));
    println!("RESULTAT {}", json!({ "scenario": "rafale", "qos": q, "mode": mode, "n": n, "rapide1": resume_seq(&v1, n), "rapide2": resume_seq(&v2, n), "lent": resume_seq(&vs, n), "avant": avant, "apres_rafale": apres_rafale, "apres_observation": apres_observation, "apres_reprise": apres_reprise }));
}
`````

Code complet — `preuves-v5/courtier/src/systeme.rs` :

`````rust
//! Ce que Linux sait du processus (mémoire, CPU, descripteurs, cgroup) ; sur macOS, `ps`.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

fn ticks(stat: &str) -> Option<(u64, u64)> {
    let fin = stat.rfind(')')?;
    let champs: Vec<&str> = stat[fin + 2..].split_whitespace().collect();
    Some((champs.get(11)?.parse().ok()?, champs.get(12)?.parse().ok()?))
}

/// Mesures du processus courant. Clés : VmRSS, RssAnon, VmHWM, Threads (Kio), cpu_ms, fds, nofile, cg_* (Kio).
pub fn systeme() -> Map<String, Value> {
    let mut m = Map::new();
    let pid = std::process::id();
    m.insert("pid".into(), pid.into());
    if let Ok(statut) = std::fs::read_to_string("/proc/self/status") {
        for cle in ["VmRSS", "RssAnon", "RssFile", "VmHWM", "VmData", "VmSize", "Threads"] {
            let prefixe = format!("{cle}:");
            if let Some(v) = statut.lines().find(|l| l.starts_with(&prefixe)).and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse::<u64>().ok()) {
                m.insert(cle.into(), v.into());
            }
        }
        if let Some((u, s)) = std::fs::read_to_string("/proc/self/stat").ok().as_deref().and_then(ticks) {
            m.insert("cpu_ms".into(), ((u + s) * 10).into());
        }
        if let Ok(d) = std::fs::read_dir("/proc/self/fd") {
            m.insert("fds".into(), (d.count().saturating_sub(1)).into());
        }
        if let Some(l) = std::fs::read_to_string("/proc/self/limits").ok().and_then(|l| l.lines().find(|l| l.starts_with("Max open files")).map(str::to_owned)) {
            m.insert("nofile".into(), l.split_whitespace().skip(3).take(2).collect::<Vec<_>>().join("/").into());
        }
    } else if let Ok(sortie) = std::process::Command::new("ps").args(["-o", "rss=,time=", "-p", &pid.to_string()]).output() {
        let texte = String::from_utf8_lossy(&sortie.stdout).into_owned();
        let mut it = texte.split_whitespace();
        if let Some(rss) = it.next().and_then(|v| v.parse::<u64>().ok()) {
            m.insert("VmRSS".into(), rss.into());
        }
        if let Some((min, sec)) = it.next().and_then(|t| t.split_once(':')) {
            let ms = min.parse::<f64>().unwrap_or(0.0) * 60_000.0 + sec.parse::<f64>().unwrap_or(0.0) * 1000.0;
            m.insert("cpu_ms".into(), (ms.round() as u64).into());
        }
    }
    if let Ok(stat) = std::fs::read_to_string("/sys/fs/cgroup/memory.stat") {
        for cle in ["anon", "file", "kernel", "slab", "sock"] {
            if let Some(v) = stat.lines().find_map(|l| l.strip_prefix(&format!("{cle} ")).and_then(|v| v.trim().parse::<u64>().ok())) {
                m.insert(format!("cg_{cle}"), (v / 1024).into());
            }
        }
    }
    for (fichier, cle) in [("memory.current", "cg_current"), ("memory.peak", "cg_peak"), ("memory.max", "cg_max")] {
        if let Some(v) = std::fs::read_to_string(format!("/sys/fs/cgroup/{fichier}")).ok().and_then(|s| s.trim().parse::<u64>().ok()) {
            m.insert(cle.into(), (v / 1024).into());
        }
    }
    m
}

pub fn val(m: &Map<String, Value>, c: &str) -> Option<u64> {
    m.get(c).and_then(Value::as_u64)
}

/// RssAnon (Linux) ou, à défaut, VmRSS (macOS), en Kio.
pub fn rss(m: &Map<String, Value>) -> Option<u64> {
    val(m, "RssAnon").or_else(|| val(m, "VmRSS"))
}

pub fn kio(m: &Map<String, Value>, c: &str) -> String {
    val(m, c).map_or("—".into(), |v| v.to_string())
}

/// CPU (ms) du fil appelant, depuis `/proc/thread-self/stat` (Linux seulement).
pub fn cpu_fil_ms() -> Option<u64> {
    let (u, s) = ticks(&std::fs::read_to_string("/proc/thread-self/stat").ok()?)?;
    Some((u + s) * 10)
}

/// CPU (ms) cumulé par nom de fil du processus (Linux seulement).
pub fn cpu_fils() -> BTreeMap<String, u64> {
    let mut m = BTreeMap::new();
    let Ok(d) = std::fs::read_dir("/proc/self/task") else { return m };
    for e in d.flatten() {
        let p = e.path();
        let nom = std::fs::read_to_string(p.join("comm")).unwrap_or_default().trim().to_owned();
        if let Some((u, s)) = std::fs::read_to_string(p.join("stat")).ok().as_deref().and_then(ticks) {
            *m.entry(nom).or_insert(0) += (u + s) * 10;
        }
    }
    m
}

pub fn percentile(tries: &[u64], p: f64) -> u64 {
    if tries.is_empty() {
        return 0;
    }
    let i = ((tries.len() as f64 - 1.0) * p).round() as usize;
    tries[i.min(tries.len() - 1)]
}

/// Format lisible d'une durée en ns.
pub fn duree(ns: u64) -> String {
    if ns >= 1_000_000_000 {
        format!("{:.2} s", ns as f64 / 1e9)
    } else if ns >= 1_000_000 {
        format!("{:.2} ms", ns as f64 / 1e6)
    } else {
        format!("{:.0} µs", ns as f64 / 1e3)
    }
}

/// Histogramme logarithmique à mémoire fixe (≈ 3 % de résolution) : pas de tableau de millions de latences.
pub struct Histo {
    compte: Vec<u64>,
    pub n: u64,
    pub max: u64,
}

impl Histo {
    pub fn new() -> Self {
        Histo { compte: vec![0; 2048], n: 0, max: 0 }
    }
    fn indice(v: u64) -> usize {
        if v < 32 {
            return v as usize;
        }
        let e = 63 - v.leading_zeros() as usize; // >= 5
        let m = ((v >> (e - 5)) & 31) as usize;
        ((e - 4) * 32 + m).min(2047)
    }
    fn borne_basse(i: usize) -> u64 {
        if i < 32 {
            return i as u64;
        }
        let e = i / 32 + 4;
        let m = (i % 32) as u64;
        (32 + m) << (e - 5)
    }
    pub fn ajouter(&mut self, v: u64) {
        self.compte[Self::indice(v)] += 1;
        self.n += 1;
        self.max = self.max.max(v);
    }
    pub fn percentile(&self, p: f64) -> u64 {
        if self.n == 0 {
            return 0;
        }
        let cible = ((self.n as f64) * p).ceil().max(1.0) as u64;
        let mut cumul = 0;
        for (i, c) in self.compte.iter().enumerate() {
            cumul += c;
            if cumul >= cible {
                return Self::borne_basse(i).min(self.max);
            }
        }
        self.max
    }
}
`````

Code complet — `preuves-v5/courtier/src/acl.rs` :

`````rust
//! Coût d'une vérification d'ACL par sujet MQTT, telle qu'on l'ajouterait au chemin de publication d'une version
//! modifiée de rumqttd pour les seuls clients extérieurs : un sujet contre une liste de motifs autorisés avec jokers
//! `+` (un niveau) et `#` (le reste, y compris aucun niveau).
//!
//! Forme retenue (la plus simple et rapide) : motifs découpés UNE fois à la construction, sujet découpé une fois par
//! vérification (tableau de 16 niveaux sur la pile), comparaison niveau par niveau. Références mesurées : le même
//! avec re-découpe du sujet pour chaque motif, et `rumqttd::protocol::matches` (qui re-découpe motif et sujet à
//! chaque appel).

use std::hint::black_box;
use std::time::Instant;

use rumqttd::protocol::matches as matches_rumqttd;

/// Un niveau d'un motif.
#[derive(Clone, Debug, PartialEq)]
enum Niveau {
    Litteral(Box<str>),
    Plus,
    Diese,
}

/// Niveaux maximum d'un sujet vérifié ; au-delà, refus (un sujet de plus de 16 niveaux n'est pas un sujet de maison).
const MAX_NIVEAUX: usize = 16;

pub struct Acl {
    motifs: Vec<Box<[Niveau]>>,
}

impl Acl {
    /// Découpe les motifs une fois. Panique sur un motif invalide (`#` ailleurs qu'en dernier niveau seul, `+` mêlé à du
    /// texte) : c'est une erreur de configuration, à refuser au chargement.
    pub fn new(motifs: &[&str]) -> Self {
        let motifs = motifs
            .iter()
            .map(|m| {
                let niveaux: Vec<&str> = m.split('/').collect();
                niveaux
                    .iter()
                    .enumerate()
                    .map(|(i, n)| match *n {
                        "#" if i + 1 == niveaux.len() => Niveau::Diese,
                        "#" => panic!("motif invalide `{m}` : `#` doit être le dernier niveau"),
                        "+" => Niveau::Plus,
                        n if n.contains(['+', '#']) => panic!("motif invalide `{m}` : joker mêlé à du texte"),
                        n => Niveau::Litteral(n.into()),
                    })
                    .collect()
            })
            .collect();
        Acl { motifs }
    }

    /// `true` si au moins un motif autorise ce sujet.
    pub fn autorise(&self, sujet: &str) -> bool {
        let mut niveaux = [""; MAX_NIVEAUX];
        let mut n = 0;
        for s in sujet.split('/') {
            if n == MAX_NIVEAUX {
                return false;
            }
            niveaux[n] = s;
            n += 1;
        }
        let niveaux = &niveaux[..n];
        // MQTT 4.7.2 : un sujet de la famille `$` ne correspond jamais à un motif dont le premier niveau est un joker.
        let dollar = sujet.starts_with('$');
        self.motifs.iter().any(|m| correspond(m, niveaux, dollar))
    }

    /// Variante de référence : le sujet est re-découpé pour chaque motif.
    pub fn autorise_redecoupe(&self, sujet: &str) -> bool {
        self.motifs.iter().any(|m| correspond_flux(m, sujet))
    }
}

#[inline]
fn correspond(motif: &[Niveau], niveaux: &[&str], dollar: bool) -> bool {
    if dollar && !matches!(motif[0], Niveau::Litteral(_)) {
        return false;
    }
    for (i, m) in motif.iter().enumerate() {
        match m {
            Niveau::Diese => return true,
            Niveau::Plus => {
                if i >= niveaux.len() {
                    return false;
                }
            }
            Niveau::Litteral(l) => {
                if i >= niveaux.len() || **l != *niveaux[i] {
                    return false;
                }
            }
        }
    }
    motif.len() == niveaux.len()
}

fn correspond_flux(motif: &[Niveau], sujet: &str) -> bool {
    if sujet.starts_with('$') && !matches!(motif[0], Niveau::Litteral(_)) {
        return false;
    }
    let mut niveaux = sujet.split('/');
    for m in motif {
        match m {
            Niveau::Diese => return true,
            Niveau::Plus => {
                if niveaux.next().is_none() {
                    return false;
                }
            }
            Niveau::Litteral(l) => match niveaux.next() {
                Some(s) if s == &**l => {}
                _ => return false,
            },
        }
    }
    niveaux.next().is_none()
}

// ---- vérifications de justesse (exécutées avant toute mesure) --------------------------------------------------------

struct Lcg(u64);
impl Lcg {
    fn suivant(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

fn verifier() -> usize {
    // Cas de la norme (MQTT 3.1.1, 4.7) et de l'usage.
    let cas: [(&str, &str, bool); 19] = [
        ("sport/tennis/player1", "sport/tennis/player1/#", true),
        ("sport", "sport/#", true),
        ("sport/", "sport/#", true),
        ("sport/tennis/player1/ranking", "sport/tennis/player1/#", true),
        ("/finance", "+/+", true),
        ("/finance", "/+", true),
        ("/finance", "+", false),
        ("a/b", "a/b/c", false),
        ("a/b/c", "a/b", false),
        ("a/b", "#", true),
        ("a", "+", true),
        ("a/b", "+", false),
        ("$SYS/broker", "#", false),
        ("$SYS/broker", "+/broker", false),
        ("$SYS/broker", "$SYS/#", true),
        ("zigbee2mqtt/thermometre_salon", "zigbee2mqtt/#", true),
        ("tasmota/prise_1/stat/POWER", "tasmota/+/stat/#", true),
        ("tasmota/prise_1/tele/STATE", "tasmota/+/stat/#", false),
        ("a/b/c/d/e/f/g/h/i/j/k/l/m/n/o/p/q", "a/#", false), // plus de 16 niveaux : refusé
    ];
    for (sujet, motif, attendu) in cas {
        let acl = Acl::new(&[motif]);
        assert_eq!(acl.autorise(sujet), attendu, "{sujet} contre {motif}");
        if sujet.split('/').count() <= MAX_NIVEAUX {
            assert_eq!(acl.autorise_redecoupe(sujet), attendu, "{sujet} contre {motif} (re-découpé)");
        }
    }
    // Équivalence avec `rumqttd::protocol::matches` sur des milliers de couples aléatoires (hors famille `$`, où rumqttd
    // refuse tout sujet commençant par `$`, même contre un motif littéral).
    let mut hasard = Lcg(1234);
    let mots = ["a", "b", "c", "", "dd"];
    let mut compares = 0;
    for _ in 0..20_000 {
        let nb_sujet = 1 + hasard.suivant() as usize % 5;
        let sujet: Vec<&str> = (0..nb_sujet).map(|_| mots[hasard.suivant() as usize % mots.len()]).collect();
        let sujet = sujet.join("/");
        let nb_motif = 1 + hasard.suivant() as usize % 5;
        let mut motif: Vec<&str> = (0..nb_motif).map(|_| ["a", "b", "c", "+", "dd", ""][hasard.suivant() as usize % 6]).collect();
        if hasard.suivant() % 3 == 0 {
            *motif.last_mut().unwrap() = "#";
        }
        let motif = motif.join("/");
        let acl = Acl::new(&[&motif]);
        assert_eq!(acl.autorise(&sujet), matches_rumqttd(&sujet, &motif), "sujet `{sujet}` contre motif `{motif}`");
        compares += 1;
    }
    compares
}

// ---- mesure -----------------------------------------------------------------------------------------------------------

/// Complète `base` à exactement 40 octets avec des `_` insérés à l'emplacement `{}`.
fn a_40_octets(modele: &str, variable: &str) -> String {
    let sans = modele.replace("{}", "").len();
    let manque = 40usize.saturating_sub(sans + variable.len());
    modele.replace("{}", &format!("{variable}{}", "_".repeat(manque)))
}

/// 50 motifs : deux vrais (`zigbee2mqtt/#`, `tasmota/+/stat/#`) puis 48 synthétiques de formes variées, premiers niveaux
/// tous distincts.
fn motifs() -> Vec<String> {
    let mut v = vec!["zigbee2mqtt/#".to_owned(), "tasmota/+/stat/#".to_owned()];
    for k in 2..50 {
        v.push(match k % 4 {
            0 => format!("site{k:02}/area/+/dev/+/state"),
            1 => format!("fam{k:02}/#"),
            2 => format!("hub{k:02}/+/stat/#"),
            _ => format!("edge{k:02}/dev/+/state"),
        });
    }
    v
}

/// Un sujet de ~40 o accordé par le motif d'indice `k`.
fn sujet_pour(k: usize, i: usize) -> String {
    let dev = format!("d{i:03}");
    match k {
        0 => a_40_octets("zigbee2mqtt/{}/availability", &dev),
        1 => a_40_octets("tasmota/{}/stat/POWER/etat", &dev),
        _ => match k % 4 {
            0 => a_40_octets(&format!("site{k:02}/area/salon/dev/{{}}/state"), &dev),
            1 => a_40_octets(&format!("fam{k:02}/{{}}/telemetry/a/b"), &dev),
            2 => a_40_octets(&format!("hub{k:02}/{{}}/stat/POWER/x"), &dev),
            _ => a_40_octets(&format!("edge{k:02}/dev/{{}}/state"), &dev),
        },
    }
}

/// Un sujet de ~40 o que aucun motif n'autorise.
fn sujet_refuse(i: usize) -> String {
    a_40_octets("inconnu/{}/area/salon/dev/lampe_1/st", &format!("s{i:03}"))
}

/// Nanosecondes par appel : médiane (et minimum) de 5 mesures, chacune d'au moins `min_ms` ms.
fn ns_par_appel(sujets: &[String], mut f: impl FnMut(&str) -> bool, min_ms: u128) -> (f64, f64) {
    // Calibrage : nombre d'itérations pour durer min_ms.
    let mut n = 2_000u64;
    loop {
        let t = Instant::now();
        let mut acc = 0u64;
        for i in 0..n {
            acc += u64::from(f(black_box(&sujets[(i as usize) & 63])));
        }
        black_box(acc);
        if t.elapsed().as_millis() >= min_ms {
            break;
        }
        n *= 2;
    }
    let mut mesures: Vec<f64> = (0..5)
        .map(|_| {
            let t = Instant::now();
            let mut acc = 0u64;
            for i in 0..n {
                acc += u64::from(f(black_box(&sujets[(i as usize) & 63])));
            }
            black_box(acc);
            t.elapsed().as_nanos() as f64 / n as f64
        })
        .collect();
    mesures.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (mesures[2], mesures[0])
}

pub fn acl() {
    let couples = verifier();
    let tous = motifs();
    println!("# Banc `acl` : une vérification de sujet MQTT contre des motifs autorisés (jokers `+` et `#`)");
    println!("- plateforme : {} / {} ; Rust release, `opt-level = \"s\"` (comme la production, sans LTO)", std::env::consts::OS, std::env::consts::ARCH);
    println!("- justesse vérifiée avant la mesure : 19 cas de la norme et de l'usage, et {couples} couples (sujet, motif) tirés au hasard identiques à `rumqttd::protocol::matches`");
    println!("- **proposé** = motifs découpés une fois à la construction (`Vec<Box<[Niveau]>>`), sujet découpé une fois par vérification dans un tableau de {MAX_NIVEAUX} niveaux sur la pile, comparaison niveau par niveau, règle `$` de la norme ; **re-découpe** = même chose mais le sujet est re-découpé pour chaque motif ; **rumqttd** = `rumqttd::protocol::matches(sujet, motif)` pour chaque motif (re-découpe tout à chaque appel, refuse tout sujet en `$`)");
    println!("- sujets de 40 octets exactement, 64 sujets différents en rotation (même longueur, nom d'appareil différent) ; médiane de 5 mesures (minimum entre parenthèses) ; exemples de motifs : `zigbee2mqtt/#`, `tasmota/+/stat/#`, `site04/area/+/dev/+/state`, `fam05/#`, `hub06/+/stat/#`, `edge07/dev/+/state`\n");
    println!("| motifs | cas | **proposé** (ns / vérification) | re-découpe (ns) | `rumqttd::matches` (ns) | exemple de sujet |");
    println!("|---|---|---|---|---|---|");
    for n in [1usize, 10, 50] {
        let liste: Vec<&str> = tous[..n].iter().map(String::as_str).collect();
        let acl = Acl::new(&liste);
        let cas: [(String, Vec<String>, bool); 3] = [
            ("accordé, par le 1er motif".to_owned(), (0..64).map(|i| sujet_pour(0, i)).collect(), true),
            (format!("accordé, par le dernier motif ({n}ᵉ)"), (0..64).map(|i| sujet_pour(n - 1, i)).collect(), true),
            ("refusé (aucun motif ne correspond)".to_owned(), (0..64).map(sujet_refuse).collect(), false),
        ];
        for (nom, sujets, attendu) in &cas {
            assert!(sujets.iter().all(|s| s.len() == 40), "sujets de 40 o : {:?}", sujets[0]);
            assert!(sujets.iter().all(|s| acl.autorise(s) == *attendu && acl.autorise_redecoupe(s) == *attendu && liste.iter().any(|m| matches_rumqttd(s, m)) == *attendu), "résultat inattendu pour {:?} ({nom})", sujets[0]);
            if n == 1 && nom.contains("dernier") {
                continue; // identique au 1er motif quand il n'y en a qu'un
            }
            let (p, pmin) = ns_par_appel(sujets, |s| acl.autorise(s), 150);
            let (r, rmin) = ns_par_appel(sujets, |s| acl.autorise_redecoupe(s), 150);
            let (m, mmin) = ns_par_appel(sujets, |s| liste.iter().any(|f| matches_rumqttd(s, f)), 150);
            println!("| {n} | {nom} | **{p:.1}** ({pmin:.1}) | {r:.1} ({rmin:.1}) | {m:.1} ({mmin:.1}) | `{}` |", sujets[0]);
        }
    }
    println!("\nLecture : un passage par le courtier coûte de l'ordre de 3 à 10 µs de CPU du routeur par message et ~25 µs de latence (liens locaux, voir `courtier.txt`) ; une vérification à 50 motifs coûte ce qui est indiqué ci-dessus.");
}
`````

Code complet — `preuves-memoire/src/compact.rs (le prototype)` :

`````rust
//! Le prototype retenu : l'état vif de la V6 en colonnes denses.
//!
//! - Identités internées : un appareil, une clé, un libellé est un `u32` ; chaque chaîne n'existe
//!   qu'une fois, bout à bout dans un seul tampon (`Noms`). Un point est le couple (appareil, clé),
//!   résolu par une table de hachage de `u32` : aucune chaîne `PointId` n'est gardée.
//! - Colonnes denses indexées par point (`Vec<Def>`, `Vec<Vif>`) et par appareil (`Vec<App>`,
//!   `Vec<Sante>`) : aucune allocation par point ni par appareil.
//! - La valeur au canon dans un `i64` : mesure mise à l'échelle, booléen 0/1, code d'énumération ; un
//!   texte vit dans une table à part, pour les seuls points texte.
//! - Instants : observé et changé en ms (`u64`), reçu en secondes depuis 2020 (`u32`) ; la qualité sur
//!   2 bits ; le genre de la valeur et celui de la précédente sur 4 bits chacun.
//!
//! Aucun `unsafe` (le dépôt l'interdit) ; `hashbrown` est déjà dans le Cargo.lock du dépôt.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::hash::{BuildHasher, RandomState};

use hashbrown::HashTable;

// ---- les noms -----------------------------------------------------------------------------------

/// Des noms, chacun une seule fois, bout à bout ; l'index d'un nom est son rang d'arrivée.
#[derive(Default)]
pub struct Noms {
    texte: String,
    fins: Vec<u32>,
    index: HashTable<u32>,
    hacheur: RandomState,
}

fn tranche<'a>(texte: &'a str, fins: &[u32], i: u32) -> &'a str {
    let i = i as usize;
    let debut = if i == 0 { 0 } else { fins[i - 1] as usize };
    &texte[debut..fins[i] as usize]
}

impl Noms {
    #[must_use]
    pub fn len(&self) -> usize {
        self.fins.len()
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.fins.is_empty()
    }
    #[must_use]
    pub fn octets(&self) -> usize {
        self.texte.len()
    }
    #[must_use]
    pub fn nom(&self, i: u32) -> &str {
        tranche(&self.texte, &self.fins, i)
    }
    #[must_use]
    pub fn chercher(&self, s: &str) -> Option<u32> {
        let h = self.hacheur.hash_one(s);
        self.index
            .find(h, |&i| tranche(&self.texte, &self.fins, i) == s)
            .copied()
    }
    pub fn interner(&mut self, s: &str) -> u32 {
        if let Some(i) = self.chercher(s) {
            return i;
        }
        let i = u32::try_from(self.fins.len()).expect("moins de 4 milliards de noms");
        self.texte.push_str(s);
        self.fins
            .push(u32::try_from(self.texte.len()).expect("moins de 4 Gio de noms"));
        let (texte, fins, hacheur) = (&self.texte, &self.fins, &self.hacheur);
        self.index.insert_unique(hacheur.hash_one(s), i, |&j| {
            hacheur.hash_one(tranche(texte, fins, j))
        });
        i
    }
    pub fn ajuster(&mut self) {
        self.texte.shrink_to_fit();
        self.fins.shrink_to_fit();
        let (texte, fins, hacheur) = (&self.texte, &self.fins, &self.hacheur);
        self.index
            .shrink_to_fit(|&j| hacheur.hash_one(tranche(texte, fins, j)));
    }
}

// ---- les codes ----------------------------------------------------------------------------------

/// Le genre de la valeur courante (4 bits).
pub mod genre {
    pub const ABSENT: u8 = 0;
    pub const NUL: u8 = 1;
    pub const BOOL: u8 = 2;
    pub const ENTIER: u8 = 3;
    pub const CODE: u8 = 4;
    pub const TEXTE: u8 = 5;
}

/// La nature d'un point (le `Kind` de moli-core, réduit à son code ; les paramètres rares vont dans
/// des tables à part).
pub mod nature {
    pub const MESURE: u8 = 0;
    pub const BINAIRE: u8 = 1;
    pub const ENUM: u8 = 2;
    pub const TEXTE: u8 = 3;
    pub const BOUTON: u8 = 4;
}

pub mod qualite {
    pub const BONNE: u8 = 0;
    pub const INCERTAINE: u8 = 1;
    pub const MAUVAISE: u8 = 2;
}

pub mod vie {
    pub const VIVANT: u8 = 0;
    pub const MUET: u8 = 1;
    pub const HORS_LIGNE: u8 = 2;
}

/// L'époque des instants en secondes : 2020-01-01T00:00:00Z (un `u32` tient jusqu'en 2156).
pub const EPOQUE_S: u64 = 1_577_836_800;

#[must_use]
pub fn secondes(ms: u64) -> u32 {
    u32::try_from((ms / 1000).saturating_sub(EPOQUE_S)).unwrap_or(u32::MAX)
}

pub const AUCUN: u32 = u32::MAX;

// ---- les définitions et l'état vif --------------------------------------------------------------

/// La définition d'un point : 16 octets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Def {
    pub appareil: u32,
    pub libelle: u32,
    pub cle: u16,
    /// 0 : aucun ; sinon rang + 1 dans `Etat::dicos`.
    pub dico: u16,
    pub nature: u8,
    /// 0 : aucune ; sinon rang + 1 dans `Etat::unites`.
    pub unite: u8,
    /// Valeur réelle = canon × 10^echelle (−2 : 2157 vaut 21,57).
    pub echelle: i8,
    /// `Semantic` (5 bits), lecture (bit 5), écriture (bit 6).
    pub sens: u8,
}

/// Un appareil : 28 octets ; les chaînes sont des rangs dans `Etat::mots` (`AUCUN` : absente).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct App {
    pub nom_natif: u32,
    pub fabricant: u32,
    pub modele: u32,
    pub piece_native: u32,
    /// L'étiquette de l'humain (nom, pièce).
    pub nom: u32,
    pub piece: u32,
    pub instance: u16,
    pub drapeaux: u16,
}

/// La valeur vive d'un point : 40 octets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Vif {
    pub valeur: i64,
    pub precedente: i64,
    pub observe_ms: u64,
    pub change_ms: u64,
    pub recu_s: u32,
    /// Genre de la valeur (bits 0-3) et de la précédente (bits 4-7).
    pub genres: u8,
    pub qualite: u8,
}

/// Variante à 32 octets de `Vif`, mesurée par sa taille seulement : « observé » devient un écart
/// (ms, saturé à ±32 s) à « changé ». Elle gagne 8 octets par point, au prix d'un instant approché.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Vif32 {
    pub valeur: i64,
    pub precedente: i64,
    pub change_ms: u64,
    pub recu_s: u32,
    pub observe_ecart_ms: i16,
    pub genres: u8,
    pub qualite: u8,
}

/// Une valeur compacte qui garde la forme de `moli_core::Value` : un texte court (≤ 24 octets) en
/// ligne, sans allocation (`compact_str`).
#[derive(Clone, Debug, PartialEq)]
pub enum ValeurCompacte {
    Nul,
    Bool(bool),
    Entier(i64),
    Reel(f64),
    Texte(compact_str::CompactString),
}

/// La santé d'un appareil : 20 octets. L'échéance (contact + rythme × (pertes + 1)) se calcule au
/// passage de l'échéancier, une fois par seconde : rien à tenir à jour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sante {
    pub contact_s: u32,
    pub change_s: u32,
    /// Rythme attendu (0 : n'émet qu'à l'action, jamais muet).
    pub rythme_s: u32,
    /// Rythme appris : moyenne glissante des intervalles, proposée à l'humain.
    pub appris_s: u32,
    pub pertes: u8,
    pub vie: u8,
}

/// Bornes d'une consigne, au canon (les seules mesures qui en ont).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bornes {
    pub min: i64,
    pub max: i64,
    pub pas: i64,
}

pub struct PointDecl<'a> {
    pub cle: &'a str,
    pub libelle: &'a str,
    pub nature: u8,
    pub unite: Option<&'a str>,
    pub echelle: i8,
    pub sens: u8,
    pub dico: &'a [&'a str],
    pub bornes: Option<Bornes>,
}

pub struct AppDecl<'a> {
    pub id: &'a str,
    pub nom_natif: &'a str,
    pub fabricant: Option<&'a str>,
    pub modele: Option<&'a str>,
    pub piece_native: Option<&'a str>,
    pub instance: u16,
    pub points: Vec<PointDecl<'a>>,
}

#[derive(Default)]
pub struct Etat {
    pub appareils: Vec<App>,
    pub sante: Vec<Sante>,
    pub defs: Vec<Def>,
    pub vifs: Vec<Vif>,
    /// `DeviceId` des appareils.
    pub ids: Noms,
    /// Clés des points (`temperature`, `state_l1`…).
    pub cles: Noms,
    /// Libellés, noms natifs, fabricants, modèles, pièces, symboles d'unités, libellés d'énumération.
    pub mots: Noms,
    pub unites: Vec<u32>,
    pub dicos: Vec<Box<[u32]>>,
    pub bornes: HashMap<u32, Bornes>,
    pub textes: HashMap<u32, Box<str>>,
    pub textes_prec: HashMap<u32, Box<str>>,
    index: HashTable<u32>,
    hacheur: RandomState,
}

impl Etat {
    fn mot(&mut self, s: Option<&str>) -> u32 {
        s.map_or(AUCUN, |s| self.mots.interner(s))
    }

    fn unite(&mut self, symbole: &str) -> u8 {
        let m = self.mots.interner(symbole);
        let rang = self.unites.iter().position(|&u| u == m).unwrap_or_else(|| {
            self.unites.push(m);
            self.unites.len() - 1
        });
        u8::try_from(rang + 1).expect("moins de 255 unités distinctes")
    }

    fn dico(&mut self, libelles: &[&str]) -> u16 {
        let codes: Box<[u32]> = libelles.iter().map(|l| self.mots.interner(l)).collect();
        let rang = self.dicos.iter().position(|d| *d == codes).unwrap_or_else(|| {
            self.dicos.push(codes);
            self.dicos.len() - 1
        });
        u16::try_from(rang + 1).expect("moins de 65 535 dictionnaires")
    }

    /// Déclare (ou redéclare) un appareil et ses points ; rend son rang.
    pub fn declarer(&mut self, a: &AppDecl) -> u32 {
        let d = self.ids.interner(a.id);
        let app = App {
            nom_natif: self.mots.interner(a.nom_natif),
            fabricant: self.mot(a.fabricant),
            modele: self.mot(a.modele),
            piece_native: self.mot(a.piece_native),
            nom: AUCUN,
            piece: AUCUN,
            instance: a.instance,
            drapeaux: 0,
        };
        if d as usize == self.appareils.len() {
            self.appareils.push(app);
            self.sante.push(Sante::default());
        } else {
            self.appareils[d as usize] = app;
        }
        for p in &a.points {
            let cle = u16::try_from(self.cles.interner(p.cle)).expect("moins de 65 536 clés");
            let def = Def {
                appareil: d,
                libelle: self.mots.interner(p.libelle),
                cle,
                dico: if p.dico.is_empty() { 0 } else { self.dico(p.dico) },
                nature: p.nature,
                unite: p.unite.map_or(0, |u| self.unite(u)),
                echelle: p.echelle,
                sens: p.sens,
            };
            let i = if let Some(i) = self.point_idx(d, cle) {
                self.defs[i as usize] = def;
                i
            } else {
                let i = u32::try_from(self.defs.len()).expect("moins de 4 milliards de points");
                self.defs.push(def);
                self.vifs.push(Vif::default());
                let (defs, hacheur) = (&self.defs, &self.hacheur);
                self.index.insert_unique(hacheur.hash_one((d, cle)), i, |&j| {
                    let x = defs[j as usize];
                    hacheur.hash_one((x.appareil, x.cle))
                });
                i
            };
            if let Some(b) = p.bornes {
                self.bornes.insert(i, b);
            }
        }
        d
    }

    #[must_use]
    pub fn point_idx(&self, appareil: u32, cle: u16) -> Option<u32> {
        let h = self.hacheur.hash_one((appareil, cle));
        self.index
            .find(h, |&p| {
                let x = &self.defs[p as usize];
                x.appareil == appareil && x.cle == cle
            })
            .copied()
    }

    /// `"<appareil>"`, `"<clé>"` → point.
    #[must_use]
    pub fn point(&self, appareil: &str, cle: &str) -> Option<u32> {
        let d = self.ids.chercher(appareil)?;
        let c = u16::try_from(self.cles.chercher(cle)?).ok()?;
        self.point_idx(d, c)
    }

    /// `moli/v1/<flux>/<appareil>/<clé>` → point (un `DeviceId` ne contient jamais `/`).
    #[must_use]
    pub fn sujet(&self, sujet: &str) -> Option<u32> {
        let (_, reste) = sujet.strip_prefix("moli/v1/")?.split_once('/')?;
        let (appareil, cle) = reste.split_once('/')?;
        self.point(appareil, cle)
    }

    /// Le `PointId` d'un point, reconstruit à la demande (rien n'est gardé).
    #[must_use]
    pub fn nom_point(&self, p: u32) -> String {
        let d = self.defs[p as usize];
        format!("{}/{}", self.ids.nom(d.appareil), self.cles.nom(u32::from(d.cle)))
    }

    /// Une valeur au canon reçue pour `p` ; rend `true` si elle change. L'appareil a donné signe de vie.
    pub fn appliquer(
        &mut self,
        p: u32,
        genre: u8,
        valeur: i64,
        observe_ms: u64,
        maintenant_ms: u64,
    ) -> bool {
        let maintenant_s = secondes(maintenant_ms);
        let v = &mut self.vifs[p as usize];
        v.recu_s = maintenant_s;
        v.observe_ms = observe_ms;
        v.qualite = qualite::BONNE;
        let change = v.genres & 0x0f != genre || v.valeur != valeur;
        if change {
            v.precedente = v.valeur;
            v.genres = (v.genres << 4) | genre;
            v.valeur = valeur;
            v.change_ms = maintenant_ms;
        }
        let s = &mut self.sante[self.defs[p as usize].appareil as usize];
        s.contact_s = maintenant_s;
        if change {
            s.change_s = maintenant_s;
        }
        change
    }

    /// Un texte court : gardé à part ; la valeur du point devient la génération du texte.
    pub fn appliquer_texte(
        &mut self,
        p: u32,
        texte: &str,
        observe_ms: u64,
        maintenant_ms: u64,
    ) -> bool {
        let v = self.vifs[p as usize];
        let change = v.genres & 0x0f != genre::TEXTE
            || self.textes.get(&p).is_none_or(|t| **t != *texte);
        let generation = if change {
            if let Some(ancien) = self.textes.insert(p, texte.into()) {
                self.textes_prec.insert(p, ancien);
            }
            v.valeur.wrapping_add(1)
        } else {
            v.valeur
        };
        self.appliquer(p, genre::TEXTE, generation, observe_ms, maintenant_ms)
    }

    /// Ce qu'une condition lit : (genre, valeur au canon, qualité).
    #[inline]
    #[must_use]
    pub fn lire(&self, p: u32) -> (u8, i64, u8) {
        let v = &self.vifs[p as usize];
        (v.genres & 0x0f, v.valeur, v.qualite)
    }

    /// Le passage de l'échéancier : les appareils sans nouvelle depuis rythme × (pertes + 1) deviennent
    /// muets, et leurs valeurs mauvaises. Rend le nombre de nouveaux muets.
    pub fn echeances(&mut self, maintenant_s: u32) -> usize {
        let mut muets = Vec::new();
        for (d, s) in self.sante.iter_mut().enumerate() {
            let delai = s.rythme_s.saturating_mul(u32::from(s.pertes) + 1);
            if s.rythme_s != 0
                && s.vie == vie::VIVANT
                && maintenant_s > s.contact_s.saturating_add(delai)
            {
                s.vie = vie::MUET;
                muets.push(u32::try_from(d).expect("rang d'appareil"));
            }
        }
        // ponytail: balayage des points pour chaque muet, O(points) ; une liste de points par appareil
        // s'il y en a des milliers par seconde (jamais dans une maison).
        if !muets.is_empty() {
            for (def, vif) in self.defs.iter().zip(self.vifs.iter_mut()) {
                if muets.contains(&def.appareil) {
                    vif.qualite = qualite::MAUVAISE;
                }
            }
        }
        muets.len()
    }

    /// Ce que pèse chaque partie (capacités des tableaux, sans les en-têtes de l'allocateur).
    #[must_use]
    pub fn bilan(&self) -> Vec<(&'static str, usize)> {
        use std::mem::size_of;
        let noms = |n: &Noms| n.texte.capacity() + n.fins.capacity() * 4 + n.index.allocation_size();
        let textes = |t: &HashMap<u32, Box<str>>| {
            t.capacity() * (size_of::<(u32, Box<str>)>() + 1) + t.values().map(|s| s.len()).sum::<usize>()
        };
        vec![
            ("noms des appareils (DeviceId)", noms(&self.ids)),
            ("clés des points", noms(&self.cles)),
            ("mots (libellés, fabricants, modèles, pièces, unités, énumérations)", noms(&self.mots)),
            ("index (appareil, clé) → point", self.index.allocation_size()),
            ("définitions des points (Def, 16 o)", self.defs.capacity() * size_of::<Def>()),
            ("dictionnaires, unités, bornes", self.dicos.iter().map(|d| d.len() * 4 + 16).sum::<usize>()
                + self.unites.capacity() * 4
                + self.bornes.capacity() * (size_of::<(u32, Bornes)>() + 1)),
            ("appareils (App, 28 o)", self.appareils.capacity() * size_of::<App>()),
            ("valeurs vives (Vif, 40 o)", self.vifs.capacity() * size_of::<Vif>()),
            ("textes courts (courant et précédent)", textes(&self.textes) + textes(&self.textes_prec)),
            ("santé des appareils (Sante, 20 o)", self.sante.capacity() * size_of::<Sante>()),
        ]
    }

    /// Rend la capacité en trop des tableaux, une fois la maison déclarée.
    pub fn ajuster(&mut self) {
        self.appareils.shrink_to_fit();
        self.sante.shrink_to_fit();
        self.defs.shrink_to_fit();
        self.vifs.shrink_to_fit();
        self.ids.ajuster();
        self.cles.ajuster();
        self.mots.ajuster();
        self.unites.shrink_to_fit();
        self.dicos.shrink_to_fit();
        let (defs, hacheur) = (&self.defs, &self.hacheur);
        self.index.shrink_to_fit(|&j| {
            let x = defs[j as usize];
            hacheur.hash_one((x.appareil, x.cle))
        });
    }
}

// ---- la queue récente du flux `val/` ------------------------------------------------------------

/// Une publication `val/` gardée pour combler un trou : 24 octets. Son rang `s` est implicite.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Entree {
    pub valeur: i64,
    pub change_ms: u64,
    pub point: u32,
    /// Genre (bits 0-3) et qualité (bits 4-5).
    pub genre_qualite: u8,
}

/// Un anneau de capacité fixe : la dernière publication écrase la plus ancienne.
pub struct File {
    entrees: Box<[Entree]>,
    prochain: u64,
}

impl File {
    #[must_use]
    pub fn new(capacite: usize) -> Self {
        Self {
            entrees: vec![Entree::default(); capacite].into_boxed_slice(),
            prochain: 0,
        }
    }
    /// Range une publication ; rend son rang `s`.
    pub fn pousser(&mut self, e: Entree) -> u64 {
        let s = self.prochain;
        let n = self.entrees.len() as u64;
        #[allow(clippy::cast_possible_truncation)]
        let i = (s % n) as usize;
        self.entrees[i] = e;
        self.prochain += 1;
        s
    }
    /// Les publications après `s`, si l'anneau les a encore ; sinon `None` : repartir d'un instantané.
    #[must_use]
    pub fn depuis(&self, s: u64) -> Option<impl Iterator<Item = (u64, Entree)> + '_> {
        let n = self.entrees.len() as u64;
        let plus_vieux = self.prochain.saturating_sub(n);
        #[allow(clippy::cast_possible_truncation)]
        (s + 1 >= plus_vieux)
            .then(|| (s + 1..self.prochain).map(move |t| (t, self.entrees[(t % n) as usize])))
    }
}

// ---- le miroir du moteur de workflow ------------------------------------------------------------

/// La copie qu'un abonné garde d'un point : 16 octets, indexée par point.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Copie {
    pub valeur: i64,
    pub change_s: u32,
    pub genre_qualite: u8,
}

pub struct Miroir {
    pub copies: Vec<Copie>,
    /// Le dernier rang `s` vu : un saut dit combien de publications manquent.
    pub vu: u64,
}

// ---- les automatismes compilés ------------------------------------------------------------------

pub mod sorte {
    pub const DECLENCHEUR: u8 = 0;
    pub const SI: u8 = 1;
    pub const REGLER: u8 = 2;
    pub const ATTENDRE: u8 = 3;
    pub const PREVENIR: u8 = 4;
}

/// Une règle : 16 octets. Pour une règle d'heure ou de soleil, `point` vaut `AUCUN` et `op` dit laquelle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Regle {
    pub valeur: i64,
    pub point: u32,
    pub op: u8,
    pub genre: u8,
}

/// Un nœud : 24 octets. `arg` : valeur à régler, durée (ms), gabarit… ; `premiere`, `n` : ses règles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Noeud {
    pub arg: i64,
    pub point: u32,
    pub suite: u16,
    pub autre: u16,
    pub premiere: u16,
    pub n: u8,
    pub sorte: u8,
}

/// Un plan : 24 octets ; ses nœuds et ses règles sont des tranches des tableaux communs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlanC {
    pub empreinte: u64,
    pub noeuds: u32,
    pub regles: u32,
    pub nom: u32,
    pub n_noeuds: u16,
    pub mode: u8,
    pub drapeaux: u8,
}

/// Tous les plans dans quatre tableaux, et l'index des déclencheurs par point (format CSR).
#[derive(Default)]
pub struct Plans {
    pub plans: Vec<PlanC>,
    pub noeuds: Vec<Noeud>,
    pub regles: Vec<Regle>,
    pub noms: Noms,
    pub debut: Vec<u32>,
    pub reveils: Vec<u32>,
}

impl Plans {
    pub fn ajouter(
        &mut self,
        nom: &str,
        empreinte: u64,
        mode: u8,
        noeuds: &[Noeud],
        regles: &[Regle],
    ) -> u32 {
        let plan = PlanC {
            empreinte,
            noeuds: u32::try_from(self.noeuds.len()).expect("nœuds"),
            regles: u32::try_from(self.regles.len()).expect("règles"),
            nom: self.noms.interner(nom),
            n_noeuds: u16::try_from(noeuds.len()).expect("moins de 65 536 nœuds"),
            mode,
            drapeaux: 0,
        };
        self.noeuds.extend_from_slice(noeuds);
        self.regles.extend_from_slice(regles);
        self.plans.push(plan);
        u32::try_from(self.plans.len() - 1).expect("plans")
    }

    /// Construit l'index : pour chaque point, les plans qu'un changement réveille.
    pub fn indexer(&mut self, points: usize) {
        let mut paires: Vec<(u32, u32)> = Vec::new();
        for (k, p) in self.plans.iter().enumerate() {
            let debut = p.noeuds as usize;
            for n in &self.noeuds[debut..debut + p.n_noeuds as usize] {
                if n.sorte == sorte::DECLENCHEUR {
                    paires.push((n.point, u32::try_from(k).expect("plans")));
                }
            }
        }
        paires.sort_unstable();
        paires.dedup();
        self.debut = vec![0; points + 1];
        for &(p, _) in &paires {
            self.debut[p as usize + 1] += 1;
        }
        for i in 1..=points {
            self.debut[i] += self.debut[i - 1];
        }
        self.reveils = paires.into_iter().map(|(_, k)| k).collect();
    }

    #[must_use]
    pub fn reveilles(&self, p: u32) -> &[u32] {
        let p = p as usize;
        &self.reveils[self.debut[p] as usize..self.debut[p + 1] as usize]
    }

    pub fn ajuster(&mut self) {
        self.plans.shrink_to_fit();
        self.noeuds.shrink_to_fit();
        self.regles.shrink_to_fit();
        self.noms.ajuster();
    }
}

// ---- baux et exécutions -------------------------------------------------------------------------

/// Un bail : 32 octets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BailC {
    pub pose: i64,
    pub retour: i64,
    pub echeance_s: u32,
    pub point: u32,
    pub genres: u8,
}

#[derive(Default)]
pub struct Baux {
    pub baux: Vec<BailC>,
    /// (échéance, bail) : 8 octets par entrée.
    pub echeancier: BinaryHeap<Reverse<(u32, u32)>>,
}

/// Une exécution en cours : 40 octets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExecC {
    pub de: i64,
    pub vers: i64,
    pub plan: u32,
    pub depuis_s: u32,
    pub cause_point: u32,
    pub en_vol: u32,
    pub bail: u32,
    pub noeud: u16,
    pub cause: u8,
    pub etat: u8,
}

// ---- agrégats -----------------------------------------------------------------------------------

/// Un seau d'agrégat : 24 octets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seau {
    pub somme: i64,
    pub n: u32,
    pub min: i32,
    pub max: i32,
}

impl Default for Seau {
    fn default() -> Self {
        Self {
            somme: 0,
            n: 0,
            min: i32::MAX,
            max: i32::MIN,
        }
    }
}

/// 6 seaux de 10 min (la dernière heure) et 24 seaux d'une heure (le dernier jour) par point lu.
pub const SEAUX: usize = 30;

pub struct Agregats {
    pub seaux: Vec<Seau>,
    pub dernier_s: Vec<u32>,
}

impl Agregats {
    #[must_use]
    pub fn new(points: usize) -> Self {
        Self {
            seaux: vec![Seau::default(); points * SEAUX],
            dernier_s: vec![0; points],
        }
    }

    /// Un relevé du point lu `i` à l'instant `t_s` (s).
    pub fn ajouter(&mut self, i: usize, t_s: u32, valeur: i64) {
        let seaux = &mut self.seaux[i * SEAUX..(i + 1) * SEAUX];
        let avant = self.dernier_s[i];
        // Les seaux traversés depuis le dernier relevé repartent de zéro.
        let vider = |seaux: &mut [Seau], debut: usize, n: usize, de: u32, a: u32, pas: u32| {
            let (de, a) = (de / pas, a / pas);
            for t in (de + 1..=a).take(n) {
                seaux[debut + (t as usize % n)] = Seau::default();
            }
        };
        vider(seaux, 0, 6, avant, t_s, 600);
        vider(seaux, 6, 24, avant, t_s, 3_600);
        #[allow(clippy::cast_possible_truncation)]
        let v32 = valeur.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
        for k in [(t_s / 600) as usize % 6, 6 + (t_s / 3_600) as usize % 24] {
            let s = &mut seaux[k];
            s.somme += valeur;
            s.n += 1;
            s.min = s.min.min(v32);
            s.max = s.max.max(v32);
        }
        self.dernier_s[i] = t_s;
    }

    /// Moyenne du dernier jour (au canon), `None` sans relevé.
    #[must_use]
    pub fn moyenne_jour(&self, i: usize) -> Option<i64> {
        let jour = &self.seaux[i * SEAUX + 6..(i + 1) * SEAUX];
        let (s, n) = jour
            .iter()
            .fold((0i64, 0i64), |(s, n), x| (s + x.somme, n + i64::from(x.n)));
        (n > 0).then(|| s / n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn maison() -> Etat {
        let mut e = Etat::default();
        for d in 0..3 {
            let id = format!("z2m:0x{d:016x}");
            let points = ["temperature", "state", "mode", "color.x"]
                .iter()
                .map(|&cle| PointDecl {
                    cle,
                    libelle: cle,
                    nature: if cle == "mode" { nature::ENUM } else { nature::MESURE },
                    unite: (cle == "temperature").then_some("°C"),
                    echelle: -2,
                    sens: 0,
                    dico: if cle == "mode" { &["auto", "heat"] } else { &[] },
                    bornes: None,
                })
                .collect();
            e.declarer(&AppDecl {
                id: &id,
                nom_natif: "x",
                fabricant: Some("IKEA"),
                modele: None,
                piece_native: None,
                instance: 0,
                points,
            });
        }
        e.ajuster();
        e
    }

    #[test]
    fn noms_points_et_sujets() {
        let mut e = maison();
        assert_eq!(e.defs.len(), 12);
        assert_eq!(e.ids.len(), 3);
        assert_eq!(e.cles.len(), 4);
        assert_eq!(e.dicos.len(), 1, "un dictionnaire partagé par trois points");
        let p = e.point("z2m:0x0000000000000001", "color.x").unwrap();
        assert_eq!(e.nom_point(p), "z2m:0x0000000000000001/color.x");
        assert_eq!(e.sujet("moli/v1/val/z2m:0x0000000000000001/color.x"), Some(p));
        assert_eq!(e.sujet("moli/v1/val/z2m:0x0000000000000009/color.x"), None);
        assert!(e.appliquer(p, genre::ENTIER, 2157, 1, 1_800_000_000_000));
        assert!(!e.appliquer(p, genre::ENTIER, 2157, 2, 1_800_000_001_000));
        assert!(e.appliquer(p, genre::ENTIER, 2158, 3, 1_800_000_002_000));
        let v = e.vifs[p as usize];
        assert_eq!((v.valeur, v.precedente, v.genres), (2158, 2157, 0x33));
        assert_eq!(e.lire(p), (genre::ENTIER, 2158, qualite::BONNE));
        let t = e.point("z2m:0x0000000000000002", "state").unwrap();
        assert!(e.appliquer_texte(t, "on", 0, 1_800_000_000_000));
        assert!(!e.appliquer_texte(t, "on", 0, 1_800_000_000_000));
        assert!(e.appliquer_texte(t, "off", 0, 1_800_000_000_000));
        assert_eq!(&*e.textes[&t], "off");
        assert_eq!(&*e.textes_prec[&t], "on");
    }

    #[test]
    fn echeances_rendent_muet_et_mauvais() {
        let mut e = maison();
        let p = e.point("z2m:0x0000000000000000", "temperature").unwrap();
        e.appliquer(p, genre::ENTIER, 1, 0, 1_800_000_000_000);
        e.sante[0].rythme_s = 600;
        e.sante[0].pertes = 2;
        let t = secondes(1_800_000_000_000);
        assert_eq!(e.echeances(t + 1_800), 0, "30 min tout juste : pas encore");
        assert_eq!(e.echeances(t + 1_801), 1);
        assert_eq!(e.lire(p).2, qualite::MAUVAISE);
    }

    #[test]
    fn file_rend_le_trou_ou_rien() {
        let mut f = File::new(4);
        for k in 0..6 {
            f.pousser(Entree { point: k, ..Entree::default() });
        }
        let suite: Vec<u64> = f.depuis(2).unwrap().map(|(s, _)| s).collect();
        assert_eq!(suite, vec![3, 4, 5]);
        assert_eq!(f.depuis(4).unwrap().next().unwrap().1.point, 5);
        assert!(f.depuis(0).is_none(), "le rang 1 est écrasé : instantané");
    }

    #[test]
    fn index_des_declencheurs() {
        let mut plans = Plans::default();
        let declencheur = |point| Noeud { point, sorte: sorte::DECLENCHEUR, ..Noeud::default() };
        plans.ajouter("a", 1, 0, &[declencheur(2), declencheur(0)], &[]);
        plans.ajouter("b", 2, 0, &[declencheur(2)], &[]);
        plans.indexer(4);
        assert_eq!(plans.reveilles(2), &[0, 1]);
        assert_eq!(plans.reveilles(0), &[0]);
        assert!(plans.reveilles(1).is_empty() && plans.reveilles(3).is_empty());
    }

    #[test]
    fn agregats_glissants() {
        let mut a = Agregats::new(1);
        a.ajouter(0, 3_600, 10);
        a.ajouter(0, 3_700, 20);
        assert_eq!(a.moyenne_jour(0), Some(15));
        a.ajouter(0, 3_600 + 25 * 3_600, 40);
        assert_eq!(a.moyenne_jour(0), Some(40), "plus d'un jour plus tard, tout est vidé");
    }
}
`````

Code complet — `preuves-memoire/src/bin/etat.rs` :

`````rust
//! Tout l'état vif de la V6, construit dans un processus neuf, phase par phase : octets demandés
//! (allocateur qui compte) et mémoire réelle (Linux : mémoire anonyme exacte, smaps_rollup), chaque
//! structure gardée vivante jusqu'à la fin.
//!
//! Usage : `etat <actuel|compact> <échelle> [voies]`
//! - `actuel` : le vrai hub de moli-runtime (définitions, valeurs, bus) et les postes nouveaux de la V6
//!   sous leur forme naïve (esquisses de la V6, banc V4) ;
//! - `compact` : le prototype ;
//! - `voies` (avec `actuel`) : ajoute les voies en tâches permanentes du banc V4, pour mesurer ce
//!   qu'elles pesaient dans sa mémoire réelle.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, VecDeque};
use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::time::{Duration, Instant};

use moli_core::{InstanceId, PointId, Value};
use moli_runtime::{BoxFuture, CommandRequest, Driver, DriverCtx, Hub, HubOptions, spawn_driver};
use preuves_memoire::construire::{self, rang};
use preuves_memoire::maison::{self, Forme};
use preuves_memoire::rapport::{Mesures, taille};
use preuves_memoire::{actuel, compact, compte};
use tokio_util::sync::CancellationToken;

const T: u64 = maison::MAINTENANT_MS;

/// Le pilote fictif : déclare la maison, puis publie ses valeurs quand on le lui dit.
struct Pilote {
    forme: Forme,
    etape: Arc<AtomicUsize>,
}

impl Pilote {
    async fn attendre(&self, etape: usize) {
        while self.etape.load(SeqCst) < etape {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }
}

impl Driver for Pilote {
    fn kind(&self) -> &'static str {
        "maison"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let f = self.forme;
            for d in 0..f.appareils {
                ctx.upsert_device(construire::appareil(&f, d));
            }
            self.etape.store(1, SeqCst);
            self.attendre(2).await;
            for d in 0..f.appareils {
                let id = ctx.device_id(&maison::natif(d));
                for k in 0..f.points_de(d) {
                    ctx.set_state(&id, maison::cle(d, k), construire::valeur(rang(&f, d, k), 0));
                }
            }
            self.etape.store(3, SeqCst);
            self.attendre(4).await;
            // Le bus : 1 024 changements avec un abonné qui ne lit pas (le tampon se remplit).
            for i in 0..1_024.min(f.points) {
                let d = (0..f.appareils).find(|&d| rang(&f, d, f.points_de(d)) > i).unwrap_or(0);
                let k = i - rang(&f, d, 0);
                let id = ctx.device_id(&maison::natif(d));
                ctx.set_state(&id, maison::cle(d, k), construire::valeur(i, 1));
            }
            self.etape.store(5, SeqCst);
            ctx.ready();
            std::future::pending::<()>().await;
            Ok(())
        })
    }
}

async fn attendre(etape: &AtomicUsize, valeur: usize) {
    while etape.load(SeqCst) < valeur {
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    tokio::time::sleep(Duration::from_millis(20)).await;
}

#[allow(clippy::too_many_lines, clippy::cast_possible_truncation)]
async fn actuel(f: Forme, m: &mut Mesures, voies: bool) -> Vec<Box<dyn std::any::Any + Send>> {
    let mut garde: Vec<Box<dyn std::any::Any + Send>> = Vec::new();
    let hub = Hub::new(HubOptions::default()).unwrap();
    let cancel = CancellationToken::new();
    let etape = Arc::new(AtomicUsize::new(0));
    m.phase("_socle : runtime, hub vide (bus de 1 024 places)", 0);
    spawn_driver(&hub, InstanceId::from(maison::INSTANCE), Arc::new(Pilote { forme: f, etape: etape.clone() }), cancel.clone());
    attendre(&etape, 1).await;
    m.phase("hub : définitions (appareils, spécifications) — moli-runtime", f.appareils);
    etape.store(2, SeqCst);
    attendre(&etape, 3).await;
    m.phase("hub : valeurs (`states`) — moli-runtime", f.points);
    let bus = hub.subscribe();
    etape.store(4, SeqCst);
    attendre(&etape, 5).await;
    m.phase("hub : bus plein (1 024 événements, un abonné)", 1_024);
    garde.push(Box::new(bus));

    let pts = construire::points(&f);
    m.phase("_liste des PointId du banc", 0);
    let n = f.points;

    let vifs: Vec<actuel::PointVif> = (0..n)
        .map(|i| actuel::PointVif { valeur: construire::valeur(i, 0), precedente: construire::valeur(i, 1), observe: T, recu: T, change: T, qualite: 0 })
        .collect();
    let mut index = HashMap::with_capacity(n);
    for (i, p) in pts.iter().enumerate() {
        index.insert(PointId::from(p.as_str()), i as u32);
    }
    m.phase("étage vif (V4 : `PointVif` + `HashMap<PointId, u32>`)", n);
    garde.push(Box::new((vifs, index)));

    let sante: Vec<actuel::SanteAppareil> = (0..f.appareils)
        .map(|_| actuel::SanteAppareil {
            dernier_contact: Instant::now(),
            dernier_changement: Instant::now(),
            rythme: Some(Duration::from_secs(600)),
            pertes_tolerees: 2,
            rythme_appris: None,
            etat: actuel::Vie::Vivant,
            echeance: Some(Instant::now()),
        })
        .collect();
    m.phase("santé des appareils (§ 9.5)", f.appareils);
    garde.push(Box::new(sante));

    let mut last: HashMap<String, Value> = HashMap::new();
    for (i, p) in pts.iter().enumerate() {
        last.insert(p.to_string(), construire::valeur(i, 0));
    }
    m.phase("miroir du moteur (`HashMap<String, Value>`)", n);
    garde.push(Box::new(last));

    let mut miroir: HashMap<String, actuel::ValeurMiroir> = HashMap::new();
    for (i, p) in pts.iter().enumerate() {
        miroir.insert(p.to_string(), actuel::ValeurMiroir { valeur: construire::valeur(i, 0), o: T, r: T, c: T, q: 0, s: i as u64 });
    }
    m.phase("miroir WebSocket (`HashMap<String, ValeurMiroir>`)", n);
    garde.push(Box::new(miroir));

    let mut file = VecDeque::with_capacity(f.file);
    for s in 0..f.file {
        let i = s % n;
        file.push_back(actuel::Publication {
            s: s as u64,
            sujet: format!("moli/v1/val/{}", pts[i]),
            charge: maison::charge_val(&maison::valeur_json(i, 0), s as u64).into_bytes(),
        });
    }
    m.phase("queue `val/` (`VecDeque<Publication>`)", f.file);
    garde.push(Box::new(file));

    // Chaque automatisme est lu de son JSON, un à un (comme le fichier `automations.json`).
    let liste: Vec<moli_automation::Automation> = (0..f.automatismes)
        .map(|k| serde_json::from_value(construire::automatisme_json(k, &pts)).unwrap())
        .collect();
    let vivants = Arc::new(liste.clone());
    m.phase("automatismes : définitions, 2 copies (`list`, `live_set`)", f.automatismes);
    garde.push(Box::new((liste, vivants)));

    let plans: Vec<actuel::Plan> = (0..f.automatismes).map(|k| construire::plan_v6(k, n)).collect();
    let mut reveils: HashMap<u32, Vec<u32>> = HashMap::new();
    for (k, p) in plans.iter().enumerate() {
        for d in &p.declencheurs {
            reveils.entry(d.point).or_default().push(k as u32);
        }
    }
    m.phase("automatismes compilés (§ 10.2) + index", f.automatismes);
    garde.push(Box::new((plans, reveils)));

    let mut echeancier = BinaryHeap::new();
    let baux: Vec<actuel::Bail> = (0..f.baux)
        .map(|i| {
            echeancier.push(Reverse((T + i as u64 * 1_000, i as u32)));
            actuel::Bail { point: (i % n) as u32, pose: Value::Bool(true), retour: Value::Bool(false), echeance: T }
        })
        .collect();
    m.phase("baux + échéancier", f.baux);
    garde.push(Box::new((baux, echeancier)));

    let executions: Vec<actuel::Execution> = (0..f.automatismes)
        .map(|k| actuel::Execution {
            id: k as u64,
            plan: k as u32,
            cause: actuel::Cause::Transition { point: PointId::from(pts[k % n].as_str()), de: Value::Bool(false), vers: Value::Bool(true) },
            curseur: 3,
            attente: (k % 2 == 0).then(|| actuel::Bail { point: (k % n) as u32, pose: Value::Bool(true), retour: Value::Bool(false), echeance: T }),
            en_vol: Some(k as u64),
        })
        .collect();
    m.phase("exécutions en cours (une par automatisme)", f.automatismes);
    garde.push(Box::new(executions));

    let lus = n / 5;
    let mut agregats = Vec::new();
    for s in 0..lus {
        for heures in [1.0, 24.0] {
            #[allow(clippy::cast_sign_loss, clippy::cast_precision_loss)]
            let k = (maison::taux_par_jour(s * 1_487 / lus, 1_487) * heures / 24.0).round() as usize;
            let mut fenetre = VecDeque::with_capacity(k);
            for i in 0..k {
                #[allow(clippy::cast_precision_loss)]
                fenetre.push_back((i as u32, i as f32));
            }
            agregats.push(actuel::Agregat { fenetre, somme: 0.0, min: 0.0, max: 0.0 });
        }
    }
    m.phase("agrégats 1 h + 24 h (points lus : 1 sur 5)", lus);
    garde.push(Box::new(agregats));

    if voies {
        // Les voies du banc V4 : une tâche permanente et une file de 8 par appareil.
        let mut taches = tokio::task::JoinSet::new();
        let mut files = Vec::new();
        for _ in 0..f.appareils {
            let (tx, mut rx) = tokio::sync::mpsc::channel::<CommandRequest>(8);
            files.push(tx);
            taches.spawn(async move {
                while let Some(c) = rx.recv().await {
                    c.reply(Ok(()));
                }
            });
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        m.phase("voies en tâches permanentes (banc V4, hors état vif)", f.appareils);
        garde.push(Box::new((taches, files)));
    }
    garde.push(Box::new((hub, cancel)));
    garde
}

#[allow(clippy::cast_possible_truncation)]
fn compact(f: Forme, m: &mut Mesures) -> Vec<Box<dyn std::any::Any + Send>> {
    let mut garde: Vec<Box<dyn std::any::Any + Send>> = Vec::new();
    m.phase("_socle : runtime", 0);
    let n = f.points;
    let mut etat = compact::Etat::default();
    for d in 0..f.appareils {
        construire::declarer_compact(&mut etat, &f, d);
    }
    etat.ajuster();
    m.phase("état : définitions, noms, index, valeurs vives, santé (colonnes)", n);
    for i in 0..n {
        construire::appliquer_compact(&mut etat, i, 0, T);
    }
    m.phase("état : textes courts des points texte", n / 20);
    for (nom, o) in etat.bilan() {
        println!("- {nom} : {}", taille(o as f64));
    }
    garde.push(Box::new(etat));

    let miroir = compact::Miroir { copies: vec![compact::Copie::default(); n], vu: 0 };
    m.phase("miroir du moteur (`Vec<Copie>`, 16 o par point)", n);
    garde.push(Box::new(miroir));
    m.phase("miroir WebSocket : aucun (instantané lu dans le hub)", 0);

    let file = compact::File::new(f.file);
    m.phase("queue `val/` (anneau de `Entree`, 24 o)", f.file);
    garde.push(Box::new(file));

    let mut plans = compact::Plans::default();
    for k in 0..f.automatismes {
        construire::plan_compact(&mut plans, k, n);
    }
    plans.indexer(n);
    plans.ajuster();
    m.phase("automatismes compilés (tableaux communs, index CSR) ; définitions sur disque", f.automatismes);
    garde.push(Box::new(plans));

    let mut baux = compact::Baux::default();
    for i in 0..f.baux {
        baux.baux.push(compact::BailC { pose: 1, retour: 0, echeance_s: i as u32, point: (i % n) as u32, genres: 0x22 });
        baux.echeancier.push(Reverse((i as u32, i as u32)));
    }
    m.phase("baux + échéancier", f.baux);
    garde.push(Box::new(baux));

    let executions = vec![compact::ExecC::default(); f.automatismes];
    m.phase("exécutions en cours (une par automatisme)", f.automatismes);
    garde.push(Box::new(executions));

    let agregats = compact::Agregats::new(n / 5);
    m.phase("agrégats 1 h + 24 h (30 seaux par point lu)", n / 5);
    garde.push(Box::new(agregats));
    m.phase("bus : aucun (le courtier route)", 0);
    garde
}

fn main() {
    let mut m = Mesures::debut();
    let mode = std::env::args().nth(1).unwrap_or_else(|| "compact".into());
    let f = Forme::de(std::env::args().nth(2).and_then(|a| a.parse().ok()).unwrap_or(1));
    let voies = std::env::args().nth(3).as_deref() == Some("voies");
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    m.phase("_runtime tokio (2 fils)", 0);
    let garde = rt.block_on(async {
        match mode.as_str() {
            "actuel" => actuel(f, &mut m, voies).await,
            _ => compact(f, &mut m),
        }
    });
    let titre = format!(
        "{mode} ×{} : {} appareils, {} points, {} automatismes (allocateur : {})",
        f.echelle, f.appareils, f.points, f.automatismes, compte::allocateur()
    );
    m.imprimer(&titre);
    let depart = m.depart.principal().unwrap_or(0);
    let fin = preuves_memoire::reel::reel();
    println!(
        "\nMémoire réelle du processus : au départ {} ; à la fin {} (anonyme), RSS {}, cgroup `anon` {}.",
        taille(depart as f64 * 1024.0),
        fin.anon.map_or("—".into(), |k| taille(k as f64 * 1024.0)),
        fin.rss.map_or("—".into(), |k| taille(k as f64 * 1024.0)),
        fin.cgroup_anon.map_or("—".into(), |k| taille(k as f64 * 1024.0)),
    );
    m.json(&format!("etat-{mode}{}", if voies { "-voies" } else { "" }), f.echelle, &serde_json::json!({}));
    black_box(&garde);
    drop(garde);
    drop(rt);
}
`````

Code complet — `preuves-memoire/src/bin/inventaire.rs` :

`````rust
//! Inventaire mesuré : pour chaque chose que la V6 garde en mémoire, sa forme Rust aujourd'hui (types
//! de moli-core, esquisses de la V6) et dans le prototype, `size_of`, le tas alloué par objet
//! (allocateur qui compte : octets demandés et blocs), et le total à ×1, ×3, ×10. Chaque mesure
//! construit l'effectif de ×10, le garde vivant, puis le rend.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap, HashMap, VecDeque};
use std::hint::black_box;
use std::mem::size_of;
use std::sync::Arc;
use std::time::{Duration, Instant};

use moli_core::{
    Action, Device, DeviceId, Event, JournalEntry, Kind, Label, Origin, Outcome, PointId, PointSpec,
    Sample, Value,
};
use preuves_memoire::maison::{self, Forme};
use preuves_memoire::rapport::taille;
use preuves_memoire::{actuel, compact, compte, construire};

const T: u64 = maison::MAINTENANT_MS;

#[derive(Clone, Copy)]
struct Mesure {
    octets: f64,
    blocs: f64,
}

impl Mesure {
    fn v4(self) -> f64 {
        self.octets + 16.0 * self.blocs
    }
}

#[allow(clippy::cast_precision_loss)]
fn par_objet<R>(n: usize, construire: impl FnOnce() -> R) -> Mesure {
    let avant = compte::tas();
    let garde = black_box(construire());
    let d = compte::tas().moins(avant);
    drop(garde);
    Mesure {
        octets: d.octets as f64 / n as f64,
        blocs: d.blocs as f64 / n as f64,
    }
}

#[derive(Clone, Copy)]
enum Effectif {
    Points,
    Appareils,
    PointsLus,
    Automatismes,
    Baux,
    File,
    Fixe(usize),
}

fn effectif(e: Effectif, k: usize) -> usize {
    let f = Forme::de(k);
    match e {
        Effectif::Points => f.points,
        Effectif::Appareils => f.appareils,
        Effectif::PointsLus => f.points / 5,
        Effectif::Automatismes => f.automatismes,
        Effectif::Baux => f.baux,
        Effectif::File => f.file,
        Effectif::Fixe(n) => n,
    }
}

struct Inventaire {
    lignes: Vec<(String, String, String, Mesure, Effectif)>,
}

impl Inventaire {
    fn ligne(&mut self, chose: &str, forme: &str, taille_de: usize, m: Mesure, e: Effectif) {
        let t = if taille_de == 0 { "—".into() } else { format!("{taille_de} o") };
        self.lignes.push((chose.into(), forme.into(), t, m, e));
    }
    fn titre(&mut self, t: &str) {
        self.lignes.push((format!("**{t}**"), String::new(), String::new(), Mesure { octets: f64::NAN, blocs: 0.0 }, Effectif::Fixe(0)));
    }
    #[allow(clippy::cast_precision_loss)]
    fn imprimer(&self) {
        println!("| chose | forme Rust | `size_of` | tas par objet : octets (blocs) | par objet, demandé (+16 o/bloc) | ×1 | ×3 | ×10 |");
        println!("|---|---|---:|---:|---:|---:|---:|---:|");
        for (chose, forme, t, m, e) in &self.lignes {
            if m.octets.is_nan() {
                println!("| {chose} | | | | | | | |");
                continue;
            }
            let tot = |k| match e {
                Effectif::Fixe(0) => "—".to_owned(),
                _ => taille(m.v4() * effectif(*e, k) as f64),
            };
            if m.octets == 0.0 && m.blocs == 0.0 {
                println!("| {chose} | {forme} | {t} | — | — | — | — | — |");
                continue;
            }
            println!(
                "| {chose} | {forme} | {t} | {:.1} ({:.2}) | {:.0} o | {} | {} | {} |",
                m.octets,
                m.blocs,
                m.v4(),
                tot(1),
                tot(3),
                tot(10)
            );
        }
    }
}

#[allow(clippy::too_many_lines, clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn main() {
    let f = Forme::de(10);
    let n = f.points;
    println!("# Inventaire mesuré (allocateur : {})\n", compte::ALLOCATEUR);
    println!(
        "Effectifs de ×10 construits pour la mesure : {} appareils, {} points, {} automatismes. Les totaux ×1/×3/×10 multiplient le coût par objet (demandé + 16 o/bloc, la mesure de la V4) par l'effectif.\n",
        f.appareils, f.points, f.automatismes
    );

    let pts = construire::points(&f);
    let devids: Vec<DeviceId> = (0..f.appareils).map(|d| DeviceId::from(maison::appareil(d))).collect();
    let mut inv = Inventaire { lignes: Vec::new() };

    // ---- identités ----
    inv.titre("Identités");
    let m = par_objet(f.appareils, || (0..f.appareils).map(|d| DeviceId::from(maison::appareil(d))).collect::<Vec<_>>());
    inv.ligne("`DeviceId` (22 car.)", "`Arc<str>` neuf", size_of::<DeviceId>(), m, Effectif::Appareils);
    let m = par_objet(n, || {
        let mut v = Vec::with_capacity(n);
        for d in 0..f.appareils {
            for k in 0..f.points_de(d) {
                v.push(PointId::new(&devids[d], maison::cle(d, k)));
            }
        }
        v
    });
    inv.ligne("`PointId` (≈ 33 car.)", "`Arc<str>` neuf (`PointId::new`, à chaque événement)", size_of::<PointId>(), m, Effectif::Points);
    let m = par_objet(n, || pts.iter().map(|p| format!("moli/v1/val/{p}")).collect::<Vec<String>>());
    inv.ligne("sujet MQTT `val/` (≈ 45 car.)", "`String`", size_of::<String>(), m, Effectif::Points);
    let m = par_objet(n, || pts.iter().map(ToString::to_string).collect::<Vec<String>>());
    inv.ligne("clé de miroir (le `PointId` en `String`)", "`String` (moteur d'automatismes, `last`)", size_of::<String>(), m, Effectif::Points);

    // ---- valeurs ----
    inv.titre("Valeurs");
    let m = par_objet(n, || (0..n).map(|i| Value::Float(i as f64)).collect::<Vec<_>>());
    inv.ligne("mesure, booléen", "`Value::Float`, `Value::Bool`", size_of::<Value>(), m, Effectif::Fixe(0));
    let m = par_objet(n, || (0..n).map(|i| Value::Text(maison::MODES[i % 5].into())).collect::<Vec<_>>());
    inv.ligne("énumération (« auto »)", "`Value::Text(Arc<str>)` neuf", size_of::<Value>(), m, Effectif::Fixe(0));
    let m = par_objet(n, || (0..n).map(|i| Value::Text(maison::texte(i, 0).into())).collect::<Vec<_>>());
    inv.ligne("texte court (11 car.)", "`Value::Text(Arc<str>)` neuf", size_of::<Value>(), m, Effectif::Fixe(0));
    let m = par_objet(n, || (0..n).map(|i| construire::valeur(i, 0)).collect::<Vec<_>>());
    inv.ligne("valeur d'un point, mélange de la maison", "`Value`", size_of::<Value>(), m, Effectif::Points);

    // ---- définitions ----
    inv.titre("Définitions");
    let m = par_objet(n, || (0..n).map(|_| construire::spec(0, "temperature", false)).collect::<Vec<_>>());
    inv.ligne("spécification d'une mesure", "`PointSpec` (`key`, `label` en `Arc<str>`)", size_of::<PointSpec>(), m, Effectif::Fixe(0));
    let m = par_objet(n, || (0..n).map(|_| construire::spec(15, "system_mode", true)).collect::<Vec<_>>());
    inv.ligne("spécification d'une énumération (5 libellés)", "`PointSpec` + `Vec<Arc<str>>`", size_of::<PointSpec>(), m, Effectif::Fixe(0));
    let m = par_objet(f.appareils, || (0..f.appareils).map(|d| Arc::new(construire::appareil(&f, d))).collect::<Vec<_>>());
    let m_point = Mesure { octets: m.octets * f.appareils as f64 / n as f64, blocs: m.blocs * f.appareils as f64 / n as f64 };
    inv.ligne("appareil et ses 4-5 spécifications", "`Arc<Device>`", size_of::<Device>(), m, Effectif::Appareils);
    inv.ligne("… ramené au point", "", 0, m_point, Effectif::Fixe(0));
    println!("(`Kind` {} o, `PointSpec` {} o, `Device` {} o)\n", size_of::<Kind>(), size_of::<PointSpec>(), size_of::<Device>());

    // ---- le socle du hub ----
    inv.titre("Socle du hub (indépendant de la maison)");
    let m = par_objet(1, moli_i18n::languages);
    inv.ligne("catalogues de traductions (fr + en, chargés au premier mot)", "`HashMap<&str, HashMap<String, String>>`", 0, m, Effectif::Fixe(1));
    let m = par_objet(1, || tokio::sync::broadcast::channel::<Event>(1_024));
    inv.ligne("bus vide (`broadcast` de 1 024 places)", "`broadcast::Sender<Event>`", 0, m, Effectif::Fixe(1));
    let m = par_objet(1, || moli_runtime::Hub::new(moli_runtime::HubOptions::default()).unwrap());
    inv.ligne("hub vide (`Hub::new`, catalogues déjà chargés)", "`Hub`", 0, m, Effectif::Fixe(1));

    // ---- le magasin du hub ----
    inv.titre("Magasin du hub (moli-runtime, `Model`)");
    let devs: Vec<Arc<Device>> = (0..f.appareils).map(|d| Arc::new(construire::appareil(&f, d))).collect();
    let m = par_objet(f.appareils, || devs.iter().map(|d| (d.id.clone(), d.clone())).collect::<BTreeMap<DeviceId, Arc<Device>>>());
    inv.ligne("entrée `devices`", "`BTreeMap<DeviceId, Arc<Device>>`", 0, m, Effectif::Appareils);
    let m = par_objet(n, || {
        let mut s: HashMap<DeviceId, BTreeMap<Arc<str>, Sample>> = HashMap::new();
        let mut i = 0;
        for d in &devs {
            let e = s.entry(d.id.clone()).or_default();
            for p in &d.points {
                e.insert(p.key.clone(), Sample { value: construire::valeur(i, 0), ts: T });
                i += 1;
            }
        }
        s
    });
    inv.ligne("valeur d'un point (`states`)", "`HashMap<DeviceId, BTreeMap<Arc<str>, Sample>>`", size_of::<Sample>(), m, Effectif::Points);
    let m = par_objet(f.appareils, || devs.iter().map(|d| (d.id.clone(), true)).collect::<HashMap<DeviceId, bool>>());
    inv.ligne("disponibilité", "`HashMap<DeviceId, bool>`", 0, m, Effectif::Appareils);
    let m = par_objet(f.appareils, || {
        devs.iter()
            .enumerate()
            .map(|(k, d)| (d.id.clone(), Label { name: Some(format!("Capteur {k}")), room: Some(maison::PIECES[k % 15].into()) }))
            .collect::<BTreeMap<DeviceId, Label>>()
    });
    inv.ligne("étiquette (nom, pièce)", "`BTreeMap<DeviceId, Label>`", size_of::<Label>(), m, Effectif::Appareils);

    // ---- événements, journal, ordres ----
    inv.titre("Événements, journal, ordres");
    let m = par_objet(n, || {
        let mut v = Vec::with_capacity(n);
        let mut i = 0;
        for d in 0..f.appareils {
            for k in 0..f.points_de(d) {
                v.push(Event::State { point: PointId::new(&devids[d], maison::cle(d, k)), value: construire::valeur(i, 1), ts: T, source_ts: None });
                i += 1;
            }
        }
        v
    });
    inv.ligne("événement d'une valeur", "`Event::State` (+ `PointId` neuf)", size_of::<Event>(), m, Effectif::Fixe(1_024));
    let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
    let m = rt.block_on(async {
        par_objet(1_024, || {
            let (tx, rx) = tokio::sync::broadcast::channel::<Event>(1_024);
            for i in 0..2_048 {
                let d = i % f.appareils;
                let _ = tx.send(Event::State { point: PointId::new(&devids[d], maison::cle(d, 0)), value: construire::valeur(i, 2), ts: T, source_ts: None });
            }
            (tx, rx)
        })
    });
    inv.ligne("tampon du bus plein (`broadcast` de 1 024)", "`broadcast::Sender<Event>`", 0, m, Effectif::Fixe(1_024));
    let m = par_objet(500, || {
        (0..500u64)
            .map(|i| {
                Arc::new(JournalEntry {
                    id: i,
                    ts: T,
                    origin: Origin::Ui,
                    actor: Some("ui:session-3f2a".into()),
                    action: Action::Command { point: PointId::new(&devids[i as usize], "state"), value: Value::Bool(true) },
                    outcome: Outcome::Ok,
                    duration_ms: 412,
                    correlation: None,
                })
            })
            .collect::<VecDeque<_>>()
    });
    inv.ligne("ligne du journal en mémoire (500 gardées)", "`Arc<JournalEntry>`", size_of::<JournalEntry>(), m, Effectif::Fixe(500));
    println!("(`CommandRequest` {} o, en vol seulement ; `Action` {} o)\n", size_of::<moli_runtime::CommandRequest>(), size_of::<Action>());

    // ---- V6, formes naïves ----
    inv.titre("V6, formes naïves (esquisses de la V6, banc V4)");
    let m = par_objet(n, || {
        (0..n)
            .map(|i| actuel::PointVif { valeur: construire::valeur(i, 0), precedente: construire::valeur(i, 1), observe: T, recu: T, change: T, qualite: 0 })
            .collect::<Vec<_>>()
    });
    inv.ligne("étage vif d'un point", "`PointVif` (2 `Value`, 3 × `u64`, `u8`)", size_of::<actuel::PointVif>(), m, Effectif::Points);
    let m = par_objet(n, || {
        let mut idx = HashMap::with_capacity(n);
        for (i, p) in pts.iter().enumerate() {
            idx.insert(PointId::from(p.as_str()), i as u32);
        }
        idx
    });
    inv.ligne("index des points", "`HashMap<PointId, u32>`", 0, m, Effectif::Points);
    let m = par_objet(f.appareils, || {
        (0..f.appareils)
            .map(|_| actuel::SanteAppareil {
                dernier_contact: Instant::now(),
                dernier_changement: Instant::now(),
                rythme: Some(Duration::from_secs(600)),
                pertes_tolerees: 2,
                rythme_appris: None,
                etat: actuel::Vie::Vivant,
                echeance: Some(Instant::now()),
            })
            .collect::<Vec<_>>()
    });
    inv.ligne("santé d'un appareil (§ 9.5)", "`SanteAppareil` (`Instant`, `Option<Duration>`)", size_of::<actuel::SanteAppareil>(), m, Effectif::Appareils);
    let m = par_objet(n, || {
        let mut last: HashMap<String, Value> = HashMap::new();
        for (i, p) in pts.iter().enumerate() {
            last.insert(p.to_string(), construire::valeur(i, 0));
        }
        last
    });
    inv.ligne("entrée du miroir du moteur", "`HashMap<String, Value>` (moli-automation, `last`)", 0, m, Effectif::Points);
    let m = par_objet(n, || {
        let mut miroir: HashMap<String, actuel::ValeurMiroir> = HashMap::new();
        for (i, p) in pts.iter().enumerate() {
            miroir.insert(p.to_string(), actuel::ValeurMiroir { valeur: construire::valeur(i, 0), o: T, r: T, c: T, q: 0, s: i as u64 });
        }
        miroir
    });
    inv.ligne("entrée du miroir WebSocket", "`HashMap<String, ValeurMiroir>`", size_of::<actuel::ValeurMiroir>(), m, Effectif::Points);
    let m = par_objet(f.file, || {
        let mut q = VecDeque::with_capacity(f.file);
        for s in 0..f.file {
            let i = s % n;
            q.push_back(actuel::Publication {
                s: s as u64,
                sujet: format!("moli/v1/val/{}", pts[i]),
                charge: maison::charge_val(&maison::valeur_json(i, 0), s as u64).into_bytes(),
            });
        }
        q
    });
    inv.ligne("entrée de la queue `val/`", "`VecDeque<Publication>` (sujet + charge JSON)", size_of::<actuel::Publication>(), m, Effectif::File);

    let json_o = construire::automatisme_json(1, &pts).to_string().len();
    let m = par_objet(f.automatismes, || {
        (0..f.automatismes)
            .map(|k| serde_json::from_value::<moli_automation::Automation>(construire::automatisme_json(k, &pts)).unwrap())
            .collect::<Vec<_>>()
    });
    inv.ligne("automatisme, une copie (6 nœuds, 3 règles)", "`moli_automation::Automation` (graphe + version approuvée)", size_of::<moli_automation::Automation>(), m, Effectif::Automatismes);
    inv.ligne("… deux copies (`list` + `live_set`)", "", 0, Mesure { octets: 2.0 * m.octets, blocs: 2.0 * m.blocs }, Effectif::Automatismes);
    let m = par_objet(f.automatismes, || {
        (0..f.automatismes)
            .map(|k| {
                let mut j = construire::automatisme_json(k, &pts);
                j.as_object_mut().unwrap().remove("approved_version");
                serde_json::from_value::<moli_automation::Automation>(j).unwrap()
            })
            .collect::<Vec<_>>()
    });
    inv.ligne("… une copie sans la version approuvée (identique au graphe)", "`Automation`, `approved_version: None`", size_of::<moli_automation::Automation>(), m, Effectif::Automatismes);
    println!("(JSON d'un automatisme : {json_o} o)\n");
    let m = par_objet(f.automatismes, || {
        let plans: Vec<actuel::Plan> = (0..f.automatismes).map(|k| construire::plan_v6(k, n)).collect();
        let mut index: HashMap<u32, Vec<u32>> = HashMap::new();
        for (k, p) in plans.iter().enumerate() {
            for d in &p.declencheurs {
                index.entry(d.point).or_default().push(k as u32);
            }
        }
        (plans, index)
    });
    inv.ligne("automatisme compilé + index", "`Plan` du § 10.2 (`Vec` par plan, `Value`)", size_of::<actuel::Plan>(), m, Effectif::Automatismes);
    let m = par_objet(f.baux, || {
        let mut h = BinaryHeap::new();
        let b: Vec<actuel::Bail> = (0..f.baux)
            .map(|i| {
                h.push(Reverse((T + i as u64 * 1_000, i as u32)));
                actuel::Bail { point: (i % n) as u32, pose: Value::Bool(true), retour: Value::Bool(false), echeance: T }
            })
            .collect();
        (b, h)
    });
    inv.ligne("bail + échéancier", "`Bail` (2 `Value`) + `BinaryHeap<(u64, u32)>`", size_of::<actuel::Bail>(), m, Effectif::Baux);
    let m = par_objet(f.automatismes, || {
        (0..f.automatismes)
            .map(|k| actuel::Execution {
                id: k as u64,
                plan: k as u32,
                cause: actuel::Cause::Transition { point: PointId::from(pts[k % n].as_str()), de: Value::Bool(false), vers: Value::Bool(true) },
                curseur: 3,
                attente: (k % 2 == 0).then(|| actuel::Bail { point: (k % n) as u32, pose: Value::Bool(true), retour: Value::Bool(false), echeance: T }),
                en_vol: Some(k as u64),
            })
            .collect::<Vec<_>>()
    });
    inv.ligne("exécution en cours", "`Execution` du § 10.2 (cause avec `PointId`)", size_of::<actuel::Execution>(), m, Effectif::Automatismes);
    let lus = n / 5;
    let m = par_objet(lus, || {
        let mut v = Vec::new();
        for s in 0..lus {
            for heures in [1.0, 24.0] {
                let k = (maison::taux_par_jour(s * 1_487 / lus, 1_487) * heures / 24.0).round() as usize;
                let mut fenetre = VecDeque::with_capacity(k);
                for i in 0..k {
                    fenetre.push_back((i as u32, i as f32));
                }
                v.push(actuel::Agregat { fenetre, somme: 0.0, min: 0.0, max: 0.0 });
            }
        }
        v
    });
    inv.ligne("agrégats 1 h + 24 h d'un point lu", "2 × `Agregat` (`VecDeque<(u32, f32)>`), taux réels", size_of::<actuel::Agregat>(), m, Effectif::PointsLus);

    // ---- le prototype ----
    inv.titre("Prototype compact");
    let mut bilan = Vec::new();
    let m = par_objet(n, || {
        let mut e = compact::Etat::default();
        for d in 0..f.appareils {
            construire::declarer_compact(&mut e, &f, d);
        }
        for i in 0..n {
            construire::appliquer_compact(&mut e, i, 0, T);
        }
        e.ajuster();
        bilan = e.bilan();
        e
    });
    inv.ligne("état vif complet, ramené au point", "`compact::Etat` (définitions, noms, index, valeurs, santé)", 0, m, Effectif::Points);
    inv.ligne("valeur vive", "`Vif`", size_of::<compact::Vif>(), Mesure { octets: size_of::<compact::Vif>() as f64, blocs: 0.0 }, Effectif::Points);
    inv.ligne("variante à 32 o (observé en écart, saturé)", "`Vif32`", size_of::<compact::Vif32>(), Mesure { octets: size_of::<compact::Vif32>() as f64, blocs: 0.0 }, Effectif::Points);
    let m = par_objet(n, || (0..n).map(|i| compact::ValeurCompacte::Texte(maison::texte(i, 0).into())).collect::<Vec<_>>());
    inv.ligne("texte court en ligne (11 car.)", "`ValeurCompacte::Texte(CompactString)`", size_of::<compact::ValeurCompacte>(), m, Effectif::Fixe(0));
    let m = par_objet(n, || (0..n).map(|i| compact::ValeurCompacte::Texte(format!("{} — un texte de plus de 24 octets", i).into())).collect::<Vec<_>>());
    inv.ligne("texte de plus de 24 octets", "`ValeurCompacte::Texte(CompactString)`", size_of::<compact::ValeurCompacte>(), m, Effectif::Fixe(0));
    inv.ligne("définition d'un point", "`Def`", size_of::<compact::Def>(), Mesure { octets: size_of::<compact::Def>() as f64, blocs: 0.0 }, Effectif::Points);
    inv.ligne("appareil", "`App`", size_of::<compact::App>(), Mesure { octets: size_of::<compact::App>() as f64, blocs: 0.0 }, Effectif::Appareils);
    inv.ligne("santé d'un appareil", "`Sante`", size_of::<compact::Sante>(), Mesure { octets: size_of::<compact::Sante>() as f64, blocs: 0.0 }, Effectif::Appareils);
    let m = par_objet(n, || compact::Miroir { copies: vec![compact::Copie::default(); n], vu: 0 });
    inv.ligne("entrée du miroir du moteur", "`Copie` dans un `Vec` indexé par point", size_of::<compact::Copie>(), m, Effectif::Points);
    let m = par_objet(f.file, || compact::File::new(f.file));
    inv.ligne("entrée de la queue `val/`", "`Entree` dans un anneau (`s` implicite)", size_of::<compact::Entree>(), m, Effectif::File);
    let m = par_objet(f.automatismes, || {
        let mut p = compact::Plans::default();
        for k in 0..f.automatismes {
            construire::plan_compact(&mut p, k, n);
        }
        p.indexer(n);
        p.ajuster();
        p
    });
    inv.ligne("automatisme compilé + index CSR", "`PlanC` + 6 `Noeud` + 3 `Regle` (tableaux communs)", size_of::<compact::PlanC>(), m, Effectif::Automatismes);
    let m = par_objet(f.baux, || {
        let mut b = compact::Baux::default();
        for i in 0..f.baux {
            b.baux.push(compact::BailC { pose: 1, retour: 0, echeance_s: i as u32, point: (i % n) as u32, genres: 0x22 });
            b.echeancier.push(Reverse((i as u32, i as u32)));
        }
        b
    });
    inv.ligne("bail + échéancier", "`BailC` + `BinaryHeap<(u32, u32)>`", size_of::<compact::BailC>(), m, Effectif::Baux);
    let m = par_objet(f.automatismes, || vec![compact::ExecC::default(); f.automatismes]);
    inv.ligne("exécution en cours", "`ExecC`", size_of::<compact::ExecC>(), m, Effectif::Automatismes);
    let m = par_objet(lus, || compact::Agregats::new(lus));
    inv.ligne("agrégats 1 h + 24 h d'un point lu", "30 `Seau` (6 × 10 min, 24 × 1 h)", size_of::<compact::Seau>(), m, Effectif::PointsLus);

    // ---- courtier ----
    inv.titre("Courtier rumqttd 0.20 (tailles ; le coût d'un retenu est mesuré par le banc `courtier`)");
    let publish = size_of::<rumqttd::protocol::Publish>();
    let props = size_of::<Option<rumqttd::protocol::PublishProperties>>();
    let donnee = size_of::<(rumqttd::protocol::Publish, Option<rumqttd::protocol::PublishProperties>, Instant)>();
    let zero = Mesure { octets: 0.0, blocs: 0.0 };
    inv.ligne("`Publish` (sujet et charge en `Bytes`)", "`rumqttd::protocol::Publish`", publish, zero, Effectif::Fixe(0));
    inv.ligne("`Option<PublishProperties>`", "propriétés MQTT 5, `None` en 3.1.1", props, zero, Effectif::Fixe(0));
    inv.ligne("`PublishData` (retenu, entrée du journal d'un filtre)", "`Publish` + propriétés + `Instant`", donnee, zero, Effectif::Fixe(0));
    inv.ligne("`Notification` (file de sortie d'une connexion)", "`rumqttd::Notification`", size_of::<rumqttd::Notification>(), zero, Effectif::Fixe(0));
    inv.ligne("segment neuf d'un filtre", "`Vec::with_capacity(1024)` de `PublishData`", 1_024 * donnee, zero, Effectif::Fixe(0));
    inv.ligne("paquet en file d'entrée d'un lien", "`rumqttd::protocol::Packet`", size_of::<rumqttd::protocol::Packet>(), zero, Effectif::Fixe(0));

    inv.imprimer();

    println!("\n## Bilan du prototype à ×10 (capacités des tableaux)\n");
    println!("| partie | octets | par point |");
    println!("|---|---:|---:|");
    for (nom, o) in &bilan {
        println!("| {nom} | {} | {:.1} o |", taille(*o as f64), *o as f64 / n as f64);
    }
    let total: usize = bilan.iter().map(|(_, o)| o).sum();
    println!("| **total** | **{}** | **{:.1} o** |", taille(total as f64), total as f64 / n as f64);
}
`````

Code complet — `preuves-memoire/src/bin/fuite.rs` :

`````rust
//! Mise au point : ce que le routeur de rumqttd 0.20 garde par publication, selon que le sujet est
//! neuf ou déjà vu, retenu ou non, avec ou sans abonné. Blocs et octets vivants après chaque série.

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use bytes::Bytes;
use preuves_memoire::compte;
use rumqttd::local::LinkTx;
use rumqttd::protocol::{Packet, Publish};
use rumqttd::{Broker, Config, Notification};

fn bloquer<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
    }
}

fn publier(tx: &mut LinkTx, sujet: String, retenu: bool) {
    let p = Publish::new(Bytes::from(sujet), Bytes::from(vec![b'x'; 75]), retenu);
    bloquer(tx.send(Packet::Publish(p, None))).unwrap();
}

fn main() {
    let mut config = Config::default();
    config.router.max_connections = 16;
    config.router.max_outgoing_packet_count = 200;
    config.router.max_segment_size = 16_384;
    config.router.max_segment_count = 2;
    let broker = Broker::new(config);
    let (mut tx, mut rx) = broker.link("p").unwrap();
    let (mut abo, mut abo_rx) = broker.link("a").unwrap();
    abo.subscribe("abo/#").unwrap();
    std::thread::spawn(move || while let Ok(n) = abo_rx.recv() { if let Some(Notification::Unschedule) = n { let _ = abo_rx.ready(); } });
    std::thread::spawn(move || while rx.recv().is_ok() {});
    let mut serie = |nom: &str, f: &mut dyn FnMut(usize) -> (String, bool)| {
        let avant = compte::tas();
        for k in 0..10_000 {
            let (s, r) = f(k);
            publier(&mut tx, s, r);
            if k % 64 == 63 {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        std::thread::sleep(Duration::from_millis(300));
        let d = compte::tas().moins(avant);
        println!("{nom:60} {:>9} o {:>7} blocs  ({:.1} o, {:.2} bloc par publication)", d.octets, d.blocs, d.octets as f64 / 1e4, d.blocs as f64 / 1e4);
    };
    serie("sans abonné, sujet unique répété", &mut |_| ("x/a".into(), false));
    serie("sans abonné, sujet unique répété (2e série)", &mut |_| ("x/a".into(), false));
    serie("sans abonné, 10 000 sujets neufs", &mut |k| (format!("x/{k}"), false));
    serie("sans abonné, les mêmes 10 000 sujets", &mut |k| (format!("x/{k}"), false));
    serie("abonné abo/#, sujet répété", &mut |_| ("abo/a".into(), false));
    serie("abonné abo/#, sujet répété (2e série)", &mut |_| ("abo/a".into(), false));
    serie("abonné abo/#, 10 000 sujets neufs", &mut |k| (format!("abo/{k}"), false));
    serie("abonné abo/#, les mêmes 10 000 sujets", &mut |k| (format!("abo/{k}"), false));
    serie("retenus sans abonné, 10 000 sujets neufs", &mut |k| (format!("r/{k}"), true));
    serie("retenus sans abonné, les mêmes 10 000 sujets", &mut |k| (format!("r/{k}"), true));
}
`````

Code complet — `preuves-memoire/src/bin/lecture.rs` :

`````rust
//! Le processeur : ce que coûte une lecture de condition, la résolution d'un sujet MQTT et l'entrée
//! d'une valeur, selon la représentation. Points tirés au hasard (indices précalculés), ns par
//! opération, à ×1 et ×10.

use std::collections::HashMap;
use std::hint::black_box;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use moli_core::{InstanceId, PointId, Value};
use moli_runtime::{BoxFuture, Driver, DriverCtx, Hub, HubOptions, spawn_driver};
use preuves_memoire::construire::{self, rang};
use preuves_memoire::maison::{self, Forme};
use preuves_memoire::{actuel, compact};
use tokio_util::sync::CancellationToken;

const T: u64 = maison::MAINTENANT_MS;
const OPS: usize = 2_000_000;

struct Pilote(Forme);

impl Driver for Pilote {
    fn kind(&self) -> &'static str {
        "maison"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let f = self.0;
            for d in 0..f.appareils {
                ctx.upsert_device(construire::appareil(&f, d));
                let id = ctx.device_id(&maison::natif(d));
                for k in 0..f.points_de(d) {
                    ctx.set_state(&id, maison::cle(d, k), construire::valeur(rang(&f, d, k), 0));
                }
            }
            ctx.ready();
            std::future::pending::<()>().await;
            Ok(())
        })
    }
}

/// Des rangs au hasard (xorshift), précalculés pour que le tirage ne compte pas.
fn tirages(n: usize) -> Vec<u32> {
    let mut x: u64 = 0x2545_F491_4F6C_DD1D;
    (0..OPS)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            #[allow(clippy::cast_possible_truncation)]
            let r = (x % n as u64) as u32;
            r
        })
        .collect()
}

#[allow(clippy::cast_precision_loss)]
fn mesurer(nom: &str, ops: usize, mut f: impl FnMut(usize) -> u64) {
    let mut acc = 0u64;
    for i in 0..ops / 10 {
        acc = acc.wrapping_add(f(i)); // chauffe
    }
    let debut = Instant::now();
    for i in 0..ops {
        acc = acc.wrapping_add(f(i));
    }
    let ns = debut.elapsed().as_nanos() as f64 / ops as f64;
    black_box(acc);
    println!("| {nom} | {ns:.1} |");
}

#[allow(clippy::too_many_lines, clippy::cast_sign_loss, clippy::cast_possible_truncation)]
fn banc(echelle: usize) {
    let f = Forme::de(echelle);
    let n = f.points;
    let tir = tirages(n);
    let pts = construire::points(&f);
    let sujets: Vec<String> = pts.iter().map(|p| format!("moli/v1/val/{p}")).collect();

    let mut etat = compact::Etat::default();
    for d in 0..f.appareils {
        construire::declarer_compact(&mut etat, &f, d);
    }
    etat.ajuster();
    for i in 0..n {
        construire::appliquer_compact(&mut etat, i, 0, T);
    }
    let partage = RwLock::new(etat);
    let etat = partage.read().unwrap();

    let mut last: HashMap<String, Value> = HashMap::new();
    for (i, p) in pts.iter().enumerate() {
        last.insert(p.to_string(), construire::valeur(i, 0));
    }
    let mut index: HashMap<PointId, u32> = HashMap::with_capacity(n);
    for (i, p) in pts.iter().enumerate() {
        index.insert(p.clone(), i as u32);
    }
    let vifs: Vec<actuel::PointVif> = (0..n)
        .map(|i| actuel::PointVif { valeur: construire::valeur(i, 0), precedente: Value::Null, observe: T, recu: T, change: T, qualite: 0 })
        .collect();
    let par_sujet: HashMap<String, u32> = sujets.iter().enumerate().map(|(i, s)| (s.clone(), i as u32)).collect();

    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let hub = rt.block_on(async {
        let hub = Hub::new(HubOptions::default()).unwrap();
        spawn_driver(&hub, InstanceId::from(maison::INSTANCE), Arc::new(Pilote(f)), CancellationToken::new());
        while hub.state(&pts[n - 1]).is_none() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        hub
    });

    println!("\n### ×{echelle} : {n} points\n");
    println!("| opération | ns |");
    println!("|---|---:|");
    let lire = |p: u32| {
        let (g, v, q) = etat.lire(p);
        u64::from(g) ^ v as u64 ^ u64::from(q)
    };
    mesurer("lire une condition — prototype, `Vec<Vif>` par index", OPS, |i| lire(tir[i]));
    drop(etat);
    mesurer("lire une condition — prototype derrière un `RwLock` (verrou à chaque lecture)", OPS, |i| {
        let e = partage.read().unwrap();
        let (g, v, q) = e.lire(tir[i]);
        u64::from(g) ^ v as u64 ^ u64::from(q)
    });
    mesurer("lire une condition — `HashMap<String, Value>` par le nom du point (moteur actuel, `last`)", OPS, |i| {
        match last.get(pts[tir[i] as usize].as_str()) {
            Some(Value::Float(x)) => x.to_bits(),
            Some(Value::Bool(b)) => u64::from(*b),
            Some(_) => 1,
            None => 0,
        }
    });
    mesurer("lire une condition — index `HashMap<PointId, u32>` puis `Vec<PointVif>` (banc V4)", OPS, |i| {
        let k = index[&pts[tir[i] as usize]];
        match &vifs[k as usize].valeur {
            Value::Float(x) => x.to_bits(),
            _ => 1,
        }
    });
    mesurer("lire une condition — `hub.state(&PointId)` (moli-runtime aujourd'hui)", OPS / 4, |i| {
        hub.state(&pts[tir[i] as usize]).map_or(0, |s| s.ts)
    });
    let etat = partage.read().unwrap();
    mesurer("résoudre un sujet `val/` → point — prototype (`Noms` + index)", OPS, |i| {
        u64::from(etat.sujet(&sujets[tir[i] as usize]).unwrap_or(0))
    });
    mesurer("résoudre un sujet `val/` → point — `HashMap<String, u32>`", OPS, |i| {
        u64::from(par_sujet[sujets[tir[i] as usize].as_str()])
    });
    drop(etat);
    let mut e = partage.write().unwrap();
    let mut file = compact::File::new(f.file);
    let mut tour = 1usize;
    mesurer("entrer une mesure — prototype : `appliquer` + santé + queue `val/`", OPS, |i| {
        let p = tir[i];
        tour += 1;
        let v = maison::mesure(p as usize, tour);
        let change = e.appliquer(p, compact::genre::ENTIER, v, T, T + i as u64);
        if change {
            file.pousser(compact::Entree { valeur: v, change_ms: T, point: p, genre_qualite: compact::genre::ENTIER });
        }
        u64::from(change)
    });
    drop(e);
    let mut last2 = last.clone();
    mesurer("entrer une mesure — `HashMap<String, Value>` : `insert` d'une clé neuve (`format!`)", OPS / 4, |i| {
        let p = tir[i] as usize;
        u64::from(last2.insert(pts[p].to_string(), Value::Float(i as f64)).is_some())
    });
    drop(hub);
    drop(rt);
}

fn main() {
    println!("# Lecture d'une condition, résolution d'un sujet, entrée d'une valeur (ns par opération)");
    for e in [1, 10] {
        banc(e);
    }
}
`````

Code complet — `preuves-memoire/src/bin/courtier.rs` :

`````rust
//! Le courtier rumqttd 0.20 embarqué, en liens locaux (même processus), réglé comme le § 4.3 de la V6
//! (segments 4 × 64 Kio, 200 paquets en sortie par connexion). Octets demandés et mémoire réelle :
//! 1. les messages retenus de la V6 (`val/` par point, `avail/` et `meta/` par appareil) ;
//! 2. ce qu'un nouvel abonné à `val/#` reçoit vraiment de ces retenus ;
//! 3. le journal du filtre `val/#` une fois plein ;
//! 4. le flux `in/` des modules vers le hub ;
//! 5. les demandes `req/` à sujet unique (le cache des filtres par sujet).
//!
//! Usage : `courtier <échelle> [optimise]`. La variante `optimise` applique les recommandations :
//! aucun retenu (le hub sert `val/`, `avail/` et `meta/`), segments 2 × 16 Kio, un seul filtre `cmd/#` pour les modules
//! du binaire (le kit répartit), sujets `req/` stables (l'identifiant dans la charge).

use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::sync::mpsc;
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use bytes::Bytes;
use rumqttd::local::{LinkRx, LinkTx};
use rumqttd::protocol::{Packet, Publish};
use rumqttd::{Broker, Config, Notification, RouterConfig};

use preuves_memoire::construire;
use preuves_memoire::maison::{self, Forme};
use preuves_memoire::rapport::Mesures;

/// `LinkTx::send` est `async` sans jamais attendre : un seul `poll` suffit.
fn bloquer<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::yield_now();
    }
}

fn publier(tx: &mut LinkTx, sujet: String, charge: Vec<u8>, retenu: bool) {
    let p = Publish::new(Bytes::from(sujet), Bytes::from(charge), retenu);
    bloquer(tx.send(Packet::Publish(p, None))).expect("publication");
}

/// Un abonné qui lit tout ce qu'on lui remet, compte les publications et, pour le témoin, rend leurs
/// sujets. Une connexion locale dont la file de sortie a débordé reçoit `Unschedule` : elle doit
/// dire `ready` pour être servie de nouveau.
fn lecteur(mut rx: LinkRx, compte: Arc<AtomicUsize>, sujets: Option<mpsc::Sender<String>>) {
    std::thread::spawn(move || {
        while let Ok(n) = rx.recv() {
            match n {
                Some(Notification::Forward(f)) => {
                    compte.fetch_add(1, SeqCst);
                    if let Some(s) = &sujets {
                        let _ = s.send(String::from_utf8_lossy(&f.publish.topic).into_owned());
                    }
                }
                Some(Notification::Unschedule) => {
                    let _ = rx.ready();
                }
                _ => {}
            }
        }
    });
}

struct Temoin {
    sujets: mpsc::Receiver<String>,
    n: usize,
}

impl Temoin {
    /// Publie un repère par la connexion `tx` et attend que le témoin le reçoive : le routeur a
    /// traité tout ce que `tx` a publié avant.
    fn synchroniser(&mut self, tx: &mut LinkTx) {
        self.n += 1;
        let repere = format!("moli/v1/sync/{}", self.n);
        publier(tx, repere.clone(), b"1".to_vec(), false);
        while self.sujets.recv_timeout(Duration::from_secs(30)).expect("repère reçu") != repere {}
    }

    /// Publie par lots de 64, synchronisés : aucun tampon ne dépasse 64 paquets.
    fn par_lots(&mut self, tx: &mut LinkTx, messages: impl IntoIterator<Item = (String, Vec<u8>)>, retenu: bool) {
        for (k, (sujet, charge)) in messages.into_iter().enumerate() {
            publier(tx, sujet, charge, retenu);
            if k % 64 == 63 {
                self.synchroniser(tx);
            }
        }
        self.synchroniser(tx);
    }
}

/// Attend qu'un compteur ne bouge plus pendant 300 ms (5 s au plus).
fn stable(c: &AtomicUsize) -> usize {
    let debut = Instant::now();
    let mut v = c.load(SeqCst);
    loop {
        std::thread::sleep(Duration::from_millis(300));
        let w = c.load(SeqCst);
        if w == v || debut.elapsed() > Duration::from_secs(5) {
            return w;
        }
        v = w;
    }
}

fn config(opt: bool) -> Config {
    Config {
        router: RouterConfig {
            max_connections: 128,
            max_outgoing_packet_count: 200,
            max_segment_size: if opt { 16_384 } else { 65_536 },
            max_segment_count: if opt { 2 } else { 4 },
            custom_segment: None,
            initialized_filters: None,
            shared_subscriptions_strategy: rumqttd::Strategy::default(),
        },
        ..Config::default()
    }
}

#[allow(clippy::too_many_lines)]
fn main() {
    let mut m = Mesures::debut();
    let f = Forme::depuis_args();
    let opt = std::env::args().nth(2).as_deref() == Some("optimise");
    let retenir = !opt;
    let n = f.points;
    let pts = construire::points(&f);
    let charges: Vec<Vec<u8>> = (0..n).map(|i| maison::charge_val(&maison::valeur_json(i, 0), i as u64).into_bytes()).collect();
    m.phase("_entrées du banc (PointId, charges)", 0);

    let broker = Broker::new(config(opt));
    let (mut hub, hub_rx) = broker.link("hub").unwrap();
    let recus_hub = Arc::new(AtomicUsize::new(0));
    lecteur(hub_rx, recus_hub.clone(), None);
    let (mut temoin_tx, temoin_rx) = broker.link("temoin").unwrap();
    temoin_tx.subscribe("moli/v1/sync/#").unwrap();
    let (s_tx, s_rx) = mpsc::channel();
    lecteur(temoin_rx, Arc::new(AtomicUsize::new(0)), Some(s_tx));
    let mut temoin = Temoin { sujets: s_rx, n: 0 };
    temoin.synchroniser(&mut hub);
    m.phase("_courtier vide : routeur, liens hub et témoin", 0);

    // 1. Les retenus, publiés par lots de 64 (le coût d'un retenu, sans les tampons d'une rafale).
    let debut = Instant::now();
    temoin.par_lots(&mut hub, pts.iter().enumerate().map(|(i, p)| (format!("moli/v1/val/{p}"), charges[i].clone())), retenir);
    let duree_val = debut.elapsed();
    m.phase(if opt { "`val/` non retenus (le hub sert l'état)" } else { "retenus `val/` (un par point : sujet ≈ 45 o, charge 75 o)" }, n);
    let meta = |d: usize| {
        let points: Vec<String> = (0..f.points_de(d))
            .map(|k| format!(r#""{}":{{"kind":"numeric","unit":"°C","echelle":2}}"#, maison::cle(d, k)))
            .collect();
        format!(r#"{{"nom":"{}","sante":{{"rythme_s":600,"pertes_tolerees":2}},"points":{{{}}}}}"#, maison::nom_natif(d), points.join(","))
    };
    let meta_o = (0..f.appareils).map(|d| meta(d).len()).sum::<usize>() / f.appareils;
    let app = (0..f.appareils).flat_map(|d| {
        let id = maison::appareil(d);
        [
            (format!("moli/v1/avail/{id}"), br#"{"en_ligne":true,"depuis":1792675624123}"#.to_vec()),
            (format!("moli/v1/meta/{id}"), meta(d).into_bytes()),
        ]
    });
    temoin.par_lots(&mut hub, app, retenir);
    m.phase(if opt { "`avail/` et `meta/` non retenus (le hub les sert aussi)" } else { "retenus `avail/` et `meta/` (deux par appareil)" }, 2 * f.appareils);

    // 2. Un nouvel abonné à `val/#` (l'archiveur, le moteur, la secours…).
    let (mut base, base_rx) = broker.link("base").unwrap();
    let recus_base = Arc::new(AtomicUsize::new(0));
    lecteur(base_rx, recus_base.clone(), None);
    base.subscribe("moli/v1/val/#").unwrap();
    temoin.synchroniser(&mut base);
    let retenus_recus = stable(&recus_base);
    m.phase("un abonné à `val/#` : lien, filtre, retenus remis", 1);

    // La topologie de la V6 (§ 3) : 32 modules lisent `cmd/<module>/#` ; le moteur et le module
    // WebSocket lisent `val/#` (même filtre que l'archiveur : un seul journal) et `evt/#` ; l'API lit
    // `res/#`. Les modules n'ont pas de lecteur (aucun ordre ne leur est envoyé ici).
    let mut liens = Vec::new();
    if opt {
        let (mut tx, rx) = broker.link("modules").unwrap();
        tx.subscribe("moli/v1/cmd/#").unwrap();
        liens.push((tx, Some(rx)));
    } else {
        for k in 0..32 {
            let (mut tx, rx) = broker.link(&format!("module-{k}")).unwrap();
            tx.subscribe(format!("moli/v1/cmd/module-{k}/#")).unwrap();
            liens.push((tx, Some(rx)));
        }
    }
    for (nom, filtres) in [("moteur", &["moli/v1/val/#", "moli/v1/evt/#"][..]), ("websocket", &["moli/v1/val/#", "moli/v1/evt/#"][..]), ("api", &["moli/v1/res/#"][..])] {
        let (mut tx, rx) = broker.link(nom).unwrap();
        for f in filtres {
            tx.subscribe(*f).unwrap();
        }
        lecteur(rx, Arc::new(AtomicUsize::new(0)), None);
        liens.push((tx, None));
    }
    for (tx, _) in &mut liens {
        temoin.synchroniser(tx);
    }
    stable(&recus_base);
    m.phase(
        if opt {
            "topologie V6 : un lien `cmd/#` pour les modules du binaire, moteur et WebSocket (`val/#`, `evt/#`), API (`res/#`)"
        } else {
            "topologie V6 : 32 modules (`cmd/<m>/#`), moteur et WebSocket (`val/#`, `evt/#`), API (`res/#`)"
        },
        liens.len(),
    );

    // 3. Le trafic : chaque point republié trois fois, retenu, par lots (le journal du filtre se remplit).
    let tours = 3;
    let trafic = (0..tours * n).map(|t| {
        let i = t % n;
        (format!("moli/v1/val/{}", pts[i]), maison::charge_val(&maison::valeur_json(i, 1 + t / n), (n + t) as u64).into_bytes())
    });
    temoin.par_lots(&mut hub, trafic, retenir);
    let livrees = stable(&recus_base);
    m.phase(
        if opt { "trafic `val/` : journal du filtre `val/#` plein (2 + 1 segments de 16 Kio)" } else { "trafic `val/` : journal du filtre `val/#` plein (4 + 1 segments de 64 Kio), retenus remplacés" },
        tours * n,
    );

    // 4. La rafale du démarrage (§ 9.4) : tous les retenus d'un coup ; les tampons gardent leur crête.
    let debut = Instant::now();
    for (i, p) in pts.iter().enumerate() {
        publier(&mut hub, format!("moli/v1/val/{p}"), charges[i].clone(), retenir);
    }
    temoin.synchroniser(&mut hub);
    let duree_rafale = debut.elapsed();
    stable(&recus_base);
    m.phase("rafale : tous les `val/` d'un coup (crête des tampons d'entrée)", n);

    // 5. Le flux `in/` : un module publie, le hub lit `in/#`.
    hub.subscribe("moli/v1/in/#").unwrap();
    temoin.synchroniser(&mut hub);
    let (mut module, module_rx) = broker.link("module").unwrap();
    lecteur(module_rx, Arc::new(AtomicUsize::new(0)), None);
    let entree = (0..2 * n).map(|t| {
        let i = t % n;
        (format!("moli/v1/in/z2m/{}", pts[i]), format!("{{\"v\":{},\"o\":{}}}", maison::valeur_json(i, t / n), maison::MAINTENANT_MS).into_bytes())
    });
    temoin.par_lots(&mut module, entree, false);
    stable(&recus_hub);
    m.phase("flux `in/` : filtre `in/#` du hub, sujets mis en cache", n);

    // 6. Des demandes à sujet unique (`req/<origine>/<id>`, § 5.5).
    hub.subscribe("moli/v1/req/#").unwrap();
    temoin.synchroniser(&mut hub);
    let (mut ui, ui_rx) = broker.link("ui").unwrap();
    lecteur(ui_rx, Arc::new(AtomicUsize::new(0)), None);
    for lot in 0..2 {
        let demandes = (0..10_000).map(|r| {
            let id = lot * 10_000 + r;
            if opt {
                (
                    "moli/v1/req/ui".to_owned(),
                    format!(r#"{{"id":"01J9Z8K3M4N5P6Q7R8{id:08}","p":"salon_lampe/on","v":true,"a":"ui:session-3f2a","e":1792675629123}}"#).into_bytes(),
                )
            } else {
                (
                    format!("moli/v1/req/ui/01J9Z8K3M4N5P6Q7R8{id:08}"),
                    br#"{"p":"salon_lampe/on","v":true,"a":"ui:session-3f2a","e":1792675629123}"#.to_vec(),
                )
            }
        });
        temoin.par_lots(&mut ui, demandes, false);
        stable(&recus_hub);
        m.phase(
            match (opt, lot) {
                (false, 0) => "10 000 demandes `req/` à sujet unique",
                (false, _) => "10 000 demandes de plus",
                (true, 0) => "10 000 demandes `req/ui` (sujet stable, identifiant dans la charge)",
                (true, _) => "10 000 demandes de plus (sujet stable)",
            },
            10_000,
        );
    }

    m.imprimer(&format!(
        "Courtier rumqttd 0.20{} ×{} : {} points, {} appareils (allocateur : {})",
        if opt { ", configuration optimisée" } else { "" },
        f.echelle,
        n,
        f.appareils,
        preuves_memoire::compte::allocateur()
    ));
    println!(
        "\n- republication de tous les `val/` : {} en {:.1} ms d'un coup, {:.1} ms par lots de 64 synchronisés (liens locaux, un seul fil qui publie) ; charge `meta/` moyenne {meta_o} o ;",
        n,
        duree_rafale.as_secs_f64() * 1000.0,
        duree_val.as_secs_f64() * 1000.0,
    );
    println!("- **retenus remis au nouvel abonné à `val/#` : {retenus_recus} sur {n}** (`max_outgoing_packet_count` = 200) ;");
    println!("- publications `val/` livrées à cet abonné en tout : {livrees} (retenus remis + {} publiées après) ;", tours * n);
    m.json(if opt { "courtier-optimise" } else { "courtier" }, f.echelle, &serde_json::json!({
        "retenus_recus": retenus_recus, "retenus": n, "livrees": livrees,
        "duree_val_lots_ms": duree_val.as_secs_f64() * 1000.0, "duree_rafale_ms": duree_rafale.as_secs_f64() * 1000.0, "meta_o": meta_o,
    }));
}
`````

Code complet — `docker-build-v5.sh` :

`````sh
#!/bin/sh
# Compile les bancs V5 en Linux musl (comme l'image de production) dans un conteneur jetable, HORS LIGNE
# (registre cargo du Mac monté en lecture seule : lancer d'abord `cargo build --release` sur le Mac, qui
# télécharge et extrait les dépendances), avec un volume nommé pour le cache de compilation.
# Usage : docker-build-v5.sh [--bin NOM …]   (sans argument : tous les binaires)
# Sortie : $P/linux-bins/ (un exécutable par banc).
set -eu
P=$SCRATCH/preuves-v5
REPO=$DEPOT
CTX=$P/ctx
rm -rf "$CTX"; mkdir -p "$CTX/nikyOS" "$CTX/preuves-v5"
cp "$REPO/Cargo.toml" "$REPO/Cargo.lock" "$CTX/nikyOS/"
cp -R "$REPO/crates" "$REPO/locales" "$REPO/integrations" "$CTX/nikyOS/"
find "$CTX/nikyOS/crates" -name target -prune -exec rm -rf {} + 2>/dev/null || true
cp -R "$P/src" "$P/Cargo.lock" "$CTX/preuves-v5/"
sed 's#$DEPOT/crates/#/src/nikyOS/crates/#g' "$P/Cargo.toml" > "$CTX/preuves-v5/Cargo.toml"
BINS="${*:---bins}"
ID=$(docker create -v "$HOME/.cargo/registry:/usr/local/cargo/registry:ro" -v moli-preuves-v5-target:/target -e CARGO_TARGET_DIR=/target -w /src/preuves-v5 rust:1-alpine \
  sh -c "apk add --no-cache musl-dev >/dev/null 2>&1; cargo build --release --offline $BINS 2>&1 | grep -E '^(error|warning: unused)|-->|Finished'; mkdir -p /out && for b in /target/release/*; do [ -f \"\$b\" ] && [ -x \"\$b\" ] && cp \"\$b\" /out/; done; ls /out")
docker cp "$CTX/." "$ID:/src" >/dev/null
docker start -a "$ID"
rm -rf "$P/linux-bins"; docker cp "$ID:/out" "$P/linux-bins" >/dev/null
docker rm "$ID" >/dev/null
ls -la "$P/linux-bins"
`````

Code complet — `docker-run-v5.sh` :

`````sh
#!/bin/sh
# Exécute un banc Linux dans un conteneur limité comme la production : 2 CPU, 128 Mio (sans swap),
# `nofile` relevé à 65 536 (1 000 connexions + marge ; la valeur par défaut de Docker est affichée par `connexions`).
# Usage : docker-run-v5.sh <binaire> [args…]
set -eu
P=$SCRATCH/preuves-v5
BIN=$1; shift
ID=$(docker create --cpuset-cpus=0,1 --memory=128m --memory-swap=128m --ulimit nofile=65536:65536 alpine:3 "/bin/$BIN" "$@")
docker cp "$P/linux-bins/$BIN" "$ID:/bin/$BIN" >/dev/null
docker start -a "$ID" || true
docker rm "$ID" >/dev/null
`````

## Annexe B — Sources externes vérifiées

Les affirmations de la discussion externe qui pèsent sur une décision ont été vérifiées le
22 octobre 2026 :

| Affirmation | Verdict | Source |
|---|---|---|
| Mosquitto 2.1 limite par défaut la taille d'un paquet à 2 000 000 octets (au lieu des 256 Mo de la norme) | **vérifié** : changement de comportement listé dans la note de version 2.1.0 | [Mosquitto 2.1.0](https://mosquitto.org/blog/2026/01/version-2-1-0-released/) |
| Le journal Raft de RMQTT n'est qu'en mémoire | **vérifié** : « in-memory logs only », pas de stockage persistant configurable ; un nœud relancé se resynchronise sur les autres ; si la moitié des nœuds ou moins tourne, intervention manuelle | [RMQTT, discussion 347](https://github.com/rmqtt/rmqtt/discussions/347) |
| RMQTT en cluster : messages retenus | **vérifié** : en mode cluster, seul Redis est pris en charge pour les messages retenus | [RMQTT, retainer](https://github.com/rmqtt/rmqtt/blob/master/docs/en_US/retainer.md) |
| Le serveur Matter de référence est maintenant matter.js | **vérifié** : Matter Server 9.0, sur matter.js, remplace le serveur Python, archivé | [Home Assistant, 23 juin 2026](https://www.home-assistant.io/blog/2026/06/23/the-matter-upgrade-youve-been-waiting-for/), [python-matter-server](https://github.com/matter-js/python-matter-server) |
| Zigbee2MQTT sait migrer un coordinateur sans réappairage | **vérifié, avec réserves** : officiel pour Z-Stack et EmberZNet, pas d'une famille à l'autre ; retours de terrain sur l'adresse IEEE non restaurée et des appareils à réappairer | [guide de migration](https://smarthomescene.com/guides/how-to-migrate-your-zigbee-network-to-a-new-coordinator/), [discussion 26716](https://github.com/Koenkk/zigbee2mqtt/discussions/26716), [FAQ Zigbee2MQTT](https://www.zigbee2mqtt.io/guide/faq/) |
| Un boîtier SMLIGHT à deux radios fait Zigbee et Thread en même temps | **vérifié** pour le SLZB-MR1 : deux puces séparées (TI et Silicon Labs), PoE ; le modèle exact du boîtier reste à confirmer | [SLZB-MR1](https://smlight.tech/product/slzb-mr1) |
| Plusieurs routeurs de bordure Thread sur un même réseau, avec le même jeu de données | cité par la discussion ; conforme à la norme Thread ; non revérifié ici | [Home Assistant, Thread](https://www.home-assistant.io/integrations/thread/) |
| QoS 0, 1, 2 ; `PUBACK` ; expiration de message en MQTT 5 | norme | [MQTT 5.0, OASIS](https://docs.oasis-open.org/mqtt/mqtt/v5.0/mqtt-v5.0.html) |
| 14 Kio (64 Kio compressé) par connexion WebSocket | **ne s'applique pas** : chiffre de la bibliothèque Python `websockets` ; mesuré en Rust (T4) | [websockets, mémoire](https://websockets.readthedocs.io/en/stable/topics/memory.html) |

Les autres affirmations sont des généralités sur MQTT et HTTP. Elles ne pèsent sur aucune
décision de ce document et n'ont pas été vérifiées.

## Annexe C — Bancs des V2 à V4, séries du stockage et sources

La V7 cite ces mesures sans les rejouer. Leurs codes et leurs sorties brutes restent dans leurs
documents :

| Repère | Ce qu'il mesure | Où |
|---|---|---|
| E1 à E8 | paniques, blocages, XML hostile | [V1, annexe A](isolation-reactivite-runtime.md#annexe-a--preuves-exécutables) |
| C1 à C3, W1 à W2, I1 à I2, R1 | cloisons, workflows, index, relance | [V2, annexe A](isolation-reactivite-runtime-v2.md#annexe-a--bancs-dessai) |
| B1, B2, P1, P2 | budget mémoire, coût des choix, bus mort, inondation | [V3, annexe A](isolation-reactivite-runtime-v3.md#annexe-a--bancs-de-la-v3) |
| L1 à L6, Q1 à Q3, F1, V1 à V3, S1 à S3, T1 | Linux, bus capricieux, correctif, voies, bascule, plafond | [V4, annexe A](isolation-reactivite-runtime-v4.md#annexe-a--bancs-de-la-v4) |
| Série écritures | octets écrits par jour selon la façon d'écrire | [mesure-ecritures-2026-10-21T114647Z](../reports/mesure-ecritures-2026-10-21T114647Z.md) |
| Série profil | qui écrit l'historique, et combien | [profil-historique-2026-10-21T151507Z](../reports/profil-historique-2026-10-21T151507Z.md) |
| Série formes | ce que pèse chaque forme en Parquet | [formes-parquet-2026-10-21T200551Z](../reports/formes-parquet-2026-10-21T200551Z.md) |

| Source | Ce qui en est tiré | Où |
|---|---|---|
| Code de Sarah Home, commit `bd1714d` | le hub, la garde, `sse.rs`, `moli-z2m`, l'API d'historique, le superviseur, `panic = "unwind"` | références `fichier:ligne` dans le texte |
| `rumqttd` 0.20.0 (code lu) | liens en mémoire (`Broker::link`), authentification à la connexion seulement, tenants par certificat (organisation), journal par filtre en segments | `server/broker.rs:150`, `lib.rs:146-168`, `router/connection.rs:41-59`, `server/tls.rs:64-76` |
| Maquette des règles, V3 | le modèle des automatismes, des groupes de modes, des transitions, des profils, de Sarah | `maquette-regles-sarah-v3.html` (dossier des maquettes) |
| V3 à V6 de cette série ; stockage V2 et V3 | le pattern, les mesures de production, les étages, la livraison des ordres, la haute disponibilité des radios | `docs/proposition/` |
| Sources externes vérifiées le 22 octobre 2026 | Mosquitto 2.1, RMQTT, Matter Server 9.0, migration d'un coordinateur Zigbee, SLZB-MR1 | [annexe B](#annexe-b--sources-externes-vérifiées) |

## Annexe D — Crible des intégrations

Le crible des § 5.9 et § 5.10 a été fait le 22 octobre 2026 par un agent et huit sous-agents, un lot
de crates chacun, en lecture seule du dépôt (`main`, `bd1714d`) : aucun `cargo`, aucun appareil,
aucun réseau. Chaque affirmation porte sa preuve `fichier:ligne` ; une quarantaine de citations,
prises au hasard dans tous les lots, ont été relues sans erreur de ligne ni de sens.

Le rapport complet est publié à côté de ce document :
[isolation-reactivite-runtime-v7-crible.md](isolation-reactivite-runtime-v7-crible.md). Il contient :

- le grand tableau des 58 intégrations et briques : lecture, actions, retour, issues, couverture ;
- les 26 écarts, avec leur constat, leurs preuves et leur extension minimale ;
- les services du hub que les pilotes utilisent hors des valeurs, crate par crate (la preuve de G4) ;
- les cas détaillés de TaHoma (RTS et io) et de Roborock ;
- les écarts entre une version antérieure de la V7 et le code, tous corrigés ici ;
- les écarts de code à corriger au portage (§ 22.3) ;
- ce qui n'est pas vérifié.

Les numéros de ligne `l.NNN` que cite le rapport renvoient à l'état de la V7 lu le 22 octobre à
18 h 40, avant l'intégration du crible.
