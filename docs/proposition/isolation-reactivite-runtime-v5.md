# Analyse V5 — La conception complète : du capteur à l'écran, réactive, cloisonnée, à haute disponibilité

Date : 22 octobre 2026. Statut : **analyse, aucune modification du code**.
Base analysée : `main` au commit `bd1714d`. La V5 est le document de référence de la série. Elle
reprend en un seul endroit le pattern de la [V3](isolation-reactivite-runtime-v3.md), les preuves
de la [V4](isolation-reactivite-runtime-v4.md) et les quatre étages du
[stockage V3](valeurs-appareils-stockage-v3.md), qui consolide la [V2](valeurs-appareils-stockage-v2.md).
Elle les prolonge ensuite sur les sujets ouverts par une discussion externe qui comparait MQTT,
WebSocket et HTTP. La [V1](isolation-reactivite-runtime.md) et la [V2](isolation-reactivite-runtime-v2.md)
restent valables ; leurs mesures sont citées par leur repère (E8, C2, W2…).

| Sujet | D'où il vient | Ce que fait la V5 |
|---|---|---|
| Le pattern, la mémoire, le coût des choix | V3 | repris, avec les chiffres de production de la V4 |
| Production, bus capricieux, correctif, voies, bascule | V4 | repris, avec les contrats et le modèle de coût |
| Valeurs, étages, écritures, archive | stockage V2 et V3 | les quatre étages raccordés au journal de reprise et à la machine de secours |
| Modules et API | demande | la carte des 57 crates et, pour chaque API, le code Rust qui la sert |
| MQTT, WebSocket, HTTP : taille, nombre, mémoire | discussion externe | confrontée au code et **mesurée** (bancs T1 à T4) |
| L'historique d'un capteur | discussion externe | ce que Moli fait déjà, mesuré (T3), et ce qui manque |
| Sécurité et livraison des messages MQTT | discussion externe | le chemin réel d'un ordre, l'écart, les correctifs |
| Haute disponibilité Zigbee, Thread, Matter, MQTT | discussion externe | une topologie à la taille d'une maison, trois reprises, quinze tests |

Méthode :

- lecture du code, avec des références `fichier:ligne` au commit analysé ;
- quatre nouveaux bancs, hors du dépôt, compilés en Linux musl et exécutés dans un conteneur
  limité à 2 CPU et 128 Mio comme la production ([annexe A](#annexe-a--bancs-de-la-v5)) ;
- vérification en ligne des affirmations externes qui pèsent sur une décision
  ([annexe C](#annexe-c--sources-externes-vérifiées)) ;
- les mesures des V2 à V4 et des séries du stockage, reprises telles quelles, avec leur repère.

Les chiffres de charge viennent des comptes agrégés des deux mêmes Home Assistant (« instance A » et
« instance B ») : 496 appareils, 2 264 valeurs et environ 163 000 relevés par jour à elles deux.
Aucun nom de lieu, d'appareil ni d'entité n'est repris.

Vocabulaire :

- un **bus** est un transport partagé par plusieurs appareils : courtier MQTT, coordinateur
  Zigbee, pont Hue ou TaHoma, service cloud ;
- le **bus d'événements** est le canal interne du hub ;
- le **courtier** est le serveur MQTT (Mosquitto, par exemple) ;
- l'**époque** est le numéro qui désigne la seule machine autorisée à agir ;
- **clore** une machine ou une radio, c'est s'assurer qu'elle ne peut plus agir avant d'en activer
  une autre.

## 1. Réponse courte

**Le pattern des V3 et V4 tient, et la maison tient en mémoire.** Ce qui manquait était autour :
la taille réelle des messages, les gros messages, ce que « fait » veut dire pour un ordre, la porte
de service du courtier MQTT, et la haute disponibilité au-delà de Moli lui-même.

| Question | Réponse | Preuve |
|---|---|---|
| Le pattern (noyau vif, cloisons, journal) tient-il en production ? | Oui : Linux musl, 2 CPU, 128 Mio ; correctif du moteur prouvé (0 déclencheur perdu) ; voies paresseuses à 237 ordres sains sur 237 ; bascule sans perte en ~1 s | V4, § 4 à 9 |
| Combien de mémoire pour toutes les valeurs temps réel ? | **3,7 Mio réels pour les deux maisons, 8,1 Mio pour trois fois les deux maisons, 24,1 Mio pour dix fois**, sans historique. La V4 annonçait 6,3 Mio : elle comptait des voies en tâches permanentes | `etat_vif`, § 5 |
| MQTT, WebSocket ou HTTP ? | MQTT au bord, là où l'appareil le parle ; SSE et REST vers les écrans aujourd'hui. Un relevé fait **97 o en MQTT, 467 o en SSE** (un événement par attribut), 367 o en WebSocket. Une connexion coûte 39 Kio en SSE, 11 à 138 Kio en WebSocket selon ses tampons | T1, T4, § 10 |
| Quels messages posent problème ? | Trois gros : **l'instantané SSE (469 Kio, jamais compressé)**, `bridge/devices` (Mosquitto 2.1 refuse au-delà de 2 000 000 o), une série d'historique brute (11,5 Mo pour un an) | T2, T3, § 10.4 |
| Et l'historique ? | Jamais par MQTT. Moli renvoie déjà au plus 2 000 points en seaux (moyenne, min, max) : **4,7 Ko compressés** pour une courbe de 24 h | T3, § 11 |
| Quand un ordre est-il « fait » ? | Aujourd'hui, dès que la file locale du client MQTT l'accepte : avant même l'accusé du courtier. Proposé : trois issues, **confirmé** (l'état est revenu), envoyé non confirmé, non remis | code, § 12 |
| Le courtier est-il sûr ? | Non tel quel : qui publie sur `zigbee2mqtt/…/set` commande **sans garde ni journal**. Et une panne de Zigbee2MQTT est invisible : Moli ignore `bridge/state` | code, § 13, § 14.2 |
| Haute disponibilité ? | Thread actif/actif ; Zigbee actif/secours, promotion humaine d'abord, clôture par le port PoE ; ponts attachés à l'époque de Moli ; un témoin ; un Mosquitto local, sans état. Pas de cluster RMQTT pour une maison : son journal Raft ne vit qu'en mémoire | § 14, annexe C |
| Et ensuite ? | La [V6](isolation-reactivite-runtime-v6.md) met un courtier MQTT en Rust au centre de Moli, à la demande du propriétaire, et s'appuie sur ces mesures | V6 |

## 2. La conception, de bout en bout

Le schéma de couverture se lit de gauche à droite. Chaque maillon a sa règle et son coût, et une
preuve quand il en existe une :

| Maillon | Ce qui s'y passe | Règle | Coût | Preuve |
|---|---|---|---|---|
| Appareil → radio | Zigbee, Thread, Wi-Fi | les fonctions essentielles en liaison Zigbee directe (*binding*), sans serveur | nul | § 14.6 |
| Radio → pont | Zigbee2MQTT lit le boîtier par TCP ; le contrôleur Matter parle aux routeurs Thread | **un seul écrivain** par identité réseau (coordinateur, fabric) | — | § 14 |
| Pont → courtier | MQTT : relevés en QoS 0, ordres en QoS 1, jamais retenus | comptes, ACL, 2 000 000 o par paquet au plus | quelques dizaines d'octets par relevé (T1) | § 10, § 13 |
| Courtier → Moli | pilote `moli-z2m` (`rumqttc`) : un thread, des voies, un disjoncteur | refus en 0,1 ms quand le bus est mort ; hystérésis quand il bat | 19 Kio par pilote | V3 P1, V4 Q1-Q3 |
| Entrée du hub | tampon « dernière valeur gagne » pendant les rafales | une occurrence n'est jamais fusionnée | ≤ 15 ms pendant une rafale, rien au repos | V3 P2, V4 T1 |
| Décision | noyau vif en mémoire, moteur compilé | aucune décision n'attend un disque, un courtier ni un appareil | 3,7 Mio réels pour les deux maisons, 8,1 Mio à ×3 ; 2 ns par événement | V5 (banc `etat_vif`), V2 I1 |
| Durabilité | journal de reprise, instantané, quatre étages | un étage ne garde jamais ce qui revient au suivant | ~2,6 Mo par jour pour la reprise, ~65 Mo pour l'état courant, ~0,26 Go pour le récent | V3 B2, stockage V3 |
| Moli → écrans | SSE : un instantané, puis les changements ; REST : ordres et historique | 64 flux au plus ; un abonné en retard reçoit un nouvel instantané | T2, T4 | § 10 |
| Moli → agents | MCP (22 outils), REST, CLI | toujours sous la garde | — | ARCHITECTURE |
| Secours | journal et époque vers la machine passive, témoin | jamais deux maîtres | RPO 0, reprise en ~1 s | V4 S1-S3 |

### 2.1 Ce que la discussion externe apporte, et ce qu'elle ne voit pas

La discussion externe pose de bonnes questions. Ses réponses sont justes en général, mais sur
plusieurs points elles ne s'appliquent pas à Moli telles quelles.

**Ce qu'elle apporte, et que la V5 retient :**

- séparer le temps réel de l'historique ;
- « reçu » n'est pas « exécuté » : QoS 1, puis une confirmation par l'état réel ;
- un identifiant pour les ordres qui ne se répètent pas sans dommage ;
- une échéance courte pour un ordre physique ;
- se méfier des messages retenus (*retained*) ;
- un seul coordinateur Zigbee à la fois, plusieurs routeurs Thread à la fois ;
- clore l'ancien avant d'activer le nouveau ;
- tester chaque reprise avant de l'automatiser.

**Ce qu'elle ne voit pas, et que la V5 corrige :**

| Affirmation externe | Ce qui vaut pour Moli | § |
|---|---|---|
| HTTP classique veut dire interrogation périodique (*polling*) | Moli pousse déjà en **SSE** : une connexion, un instantané, puis les changements. La discussion ne cite SSE qu'en passant ; c'est pourtant la quatrième option, et celle que Moli utilise | 10 |
| 14 à 64 Kio par connexion WebSocket | chiffres d'une bibliothèque Python ; mesuré ici en Rust (T4) | 10.4 |
| Un relevé coûte quelques dizaines d'octets | vrai sur le courtier ; mais Moli découpe un message d'appareil en un événement par valeur, et chacun porte son identité (T1) | 10.2 |
| Un cluster MQTT Raft à trois nœuds en Rust (RMQTT) | son journal Raft ne vit qu'en mémoire (vérifié) ; trois nœuds plus Node.js pour les ponts dépassent le budget d'un Pi Zero 2 W (D10) ; pour une maison, un courtier actif/passif attaché à l'époque suffit | 14.5 |
| Historique dans PostgreSQL ou TimescaleDB | Moli a décidé Turso pour le récent et une archive Parquet pour le long terme (D13, stockage V3). Rien dans la discussion ne justifie d'en changer | 5, 11 |
| La sécurité MQTT, c'est TLS et des ACL | juste, mais incomplet pour Moli : un client du courtier qui publie sur `zigbee2mqtt/…/set` commande un appareil **sans passer par la garde ni le journal** de Moli. Le courtier est une porte de service autour de la règle 9 | 13 |
| Le gros message à craindre, c'est l'historique | chez Moli, les gros messages sont trois autres : l'instantané SSE, `bridge/devices` et une réponse d'historique brute. Ils sont mesurés (T2, T3) | 10.3, 11 |

## 3. Les modules et les API

### 3.1 Cinq couches

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
hub. C'est elle qui rend possible un thread par pilote (§ 4) sans rien changer aux briques.

### 3.2 Rust par API

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
trancher (§ 10.5).

### 3.3 Les contrats Rust au centre

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

Deux détails du hub comptent pour la livraison des ordres (§ 12) :

- **chaque ordre accepté a un observateur** (`hub.rs:357-430`) qui écrit une seule ligne de
  journal : la réponse du pilote, « abandonné » si l'appelant est parti avant l'envoi, ou « issue
  inconnue » si tout s'arrête avant la réponse ;
- **`request_id` est déjà une clé d'idempotence**, réservée aux points de type opération (un
  appui, un compteur) : une demande d'approbation en double rend la première, et un compteur ne
  compte pas deux fois le même identifiant (`hub.rs:1037-1056`, `hub.rs:1219-1233`). Une
  consigne (allumer, régler) n'en a pas besoin : la poser deux fois revient au même.

## 4. Le pattern : un noyau vif, des cloisons, un journal

<!-- FIGURE:pattern -->

**Toute décision se prend sur l'état en mémoire**, sans attendre ni un disque ni un appareil.
**Chaque frontière a sa cloison** et sa règle quand ça déborde : le thread d'un pilote, l'appareil,
le bus, l'entrée du hub. **Le disque ne sert qu'à ne rien perdre** : un journal en ajout, des
instantanés, et les étages du stockage.

### 4.1 Les sept couches, et ce que chacune a prouvé

| Couche | Ce qu'elle garantit | Coût mesuré | Preuve |
|---|---|---|---|
| 1. Un thread et un runtime par pilote | un pilote qui bloque ne bloque que lui | **19 Kio par pilote** (Linux musl) ; ordre aller-retour 0,03 ms | V2 C1, V3, V4 L5 |
| 2. Voies paresseuses par appareil | un appareil muet ne retient que ses ordres | **6 Kio au repos** pour toute la maison ; < 0,6 Mio en charge | V4 V1-V3 |
| 3. Disjoncteurs par appareil et par bus, à hystérésis, avec budget de latence | un bus mort répond en 0,1 ms, un bus lent ne fait plus attendre, un bus qui bat ne fait pas osciller | quelques octets | V3 P1, V4 Q1-Q3 |
| 4. Tampon d'entrée du hub | une inondation à la reconnexion ne fait plus perdre de déclencheurs | quelques Kio ; ≤ 15 ms pendant une rafale seulement | V3 P2, V4 T1 |
| 5. Noyau vif en mémoire | aucune décision n'attend le disque | **3,7 Mio réels** pour les deux maisons, 8,1 Mio à ×3 | V5 (banc `etat_vif`) |
| 6. Moteur compilé et durable, corrigé | 2 à 18 ns par événement ; plus aucune transition perdue sur un retard ; baux repris | ~520 o par automatisme (9,4 Kio aujourd'hui) | V2 I1, V4 F1 |
| 7. Durabilité par journal, instantanés et époque | reprise sans perte, ici ou sur la secours, jamais deux maîtres | ~2,6 Mo écrits par jour | V3 B2, V4 S1-S3 |

### 4.2 Le détail des couches

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
3. **Disjoncteurs** (§ 7.3).
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
5. **Noyau vif en mémoire** (§ 5). Toute l'information utile à une décision vit en mémoire :
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
     savait : c'est le correctif de la § 8.
7. **Durabilité découplée.**
   - Un journal compact en ajout, validé par petits lots, et un instantané binaire périodique.
   - L'époque est écrite sur disque avant toute activation.
   - Les étages du stockage V3 (§ 5.5) gardent l'état courant, le récent et le long terme.
   - Le journal et les instantanés partent aussi vers la machine de secours (§ 14).

S'ajoute une **surveillance** transversale :

- un détecteur de gel du runtime ;
- la vivacité des pilotes ;
- la sentinelle des briques (V1) ;
- la santé par bus, affichée à la famille.

### 4.3 Pourquoi ce pattern plutôt qu'un autre

| Pattern | Ce qu'il protège | Coût mesuré ou estimé | Verdict |
|---|---|---|---|
| Tout sur un runtime partagé (aujourd'hui) | les paniques des pilotes seulement | rien | insuffisant : deux blocages gèlent tout (V2 C1, V4 L5) |
| **Un thread par pilote** (recommandé) | les blocages et les calculs longs | **19 Kio par pilote** sous Linux musl, 52 Kio sous macOS | protège l'essentiel pour presque rien |
| Un processus par pilote | aussi le débordement de pile, la mémoire, `abort` | **577 Kio par processus minimal** sous Linux musl, 30 fois un thread, plus la sérialisation de chaque état et de chaque ordre (estimé) | à réserver à un ou deux pilotes à risque, jamais à tous sur un Pi |
| Bac à sable WASM par extension | aussi le code non relu | plusieurs Mio par instance et un calcul plus lent (estimé) ; écarté par D2 | seulement si des extensions non relues doivent tourner |
| Cadriciel d'acteurs (actix, ractor…) | comme les voies | une dépendance de plus, pas d'avantage mesurable sur des tâches Tokio et des files | inutile |
| Un courtier MQTT au centre de Moli (tout passe par MQTT) | pour la réactivité, rien de plus : le hub diffuse déjà en mémoire | une sérialisation et un routage par événement | pas nécessaire à la réactivité ; retenu par la [V6](isolation-reactivite-runtime-v6.md) pour d'autres raisons : un contrat unique pour tous les modules, l'isolation par processus quand il le faut, l'archivage branché sur le bus |

## 5. Tout en mémoire, et les quatre étages

### 5.1 Ce que « toutes les données importantes » recouvre

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

### 5.2 Le budget, demandé et réel

L'allocateur qui compte chaque octet donne les **octets demandés** (forme réelle des deux maisons,
banc B1 de la V3). Le conteneur Linux donne la **mémoire anonyme réelle** du cgroup, la mesure
que PERFORMANCE.md applique à Moli et à Home Assistant (banc L1 de la V4).

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

Le facteur entre demandé et réel vaut **1,5 à ×1, 1,2 à ×3 et 1,05 à ×10** (banc `etat_vif` de la
V5, un processus neuf par échelle). Il se décompose en environ 1,2 Mio fixes (runtime, threads,
première allocation de chaque classe de taille de l'allocateur) et un peu plus d'un octet réel par
octet demandé. La V4 avait mesuré 6,3 Mio pour ×1 : son banc comptait des voies en **tâches
permanentes**, qui ajoutent 3,5 Mio demandés (§ 5.4). Avec des voies paresseuses, l'état vif des
deux maisons tient en **3,7 Mio réels**, et trois fois les deux maisons en **8,1 Mio**.

### 5.3 Trois profils

| Profil | Contenu | Réel, deux maisons | Matériel |
|---|---|---|---|
| **Sobre** | état vif + 24 h d'historique + threads | **~6 Mio** | Pi Zero 2 W (512 Mo) : tient dans le budget de 32 Mio de mémoire anonyme de PERFORMANCE.md, avec le reste du cœur (4 à 7 Mio mesurés aujourd'hui) |
| **Généreux** | état vif + 7 jours + threads | **~14 Mio** | Pi 4 ou 5, mini-PC ; tient aussi dans les 32 Mio |
| **Maximal** | état vif + 30 jours + threads | ~43 Mio (estimé) | 2 Go et plus ; dépasse le budget du Pi |

Même le profil maximal reste environ 50 fois sous Home Assistant (de l'ordre de 2 Gio pour
l'étalon de l'architecture).

### 5.4 Ce que la mémoire rapporte, et son coût caché

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
pour 6 Kio au repos, et la V4 l'a vérifié sous charge (§ 9).

### 5.5 Les quatre étages du stockage

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
   § 14.4).

### 5.6 Ce qui est écrit, au total

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

## 6. En production : Linux musl, 2 CPU, 128 Mio

Les bancs des V2 et V3 ont été recompilés par la V4 en Linux musl statique (le profil de l'image
`FROM scratch`, sans LTO) et exécutés dans un conteneur `alpine` limité à deux cœurs et 128 Mio,
la limite de `compose.yaml`. Les écarts avec le Mac viennent de l'allocateur de musl (plus lent,
surtout sur les nombreuses allocations de `serde_json`) et de la virtualisation. Le Pi Zero 2 W
est estimé 8 à 10 fois plus lent que ce conteneur (non mesuré).

### 6.1 Les mêmes bancs, Linux contre Mac

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
  à chaque nouvel écran (§ 10.3).
- Les cloisons, les voies et le tampon d'entrée donnent exactement les mêmes résultats : leur
  effet ne dépend pas de la machine.

### 6.2 Le plafond du moteur actuel

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
redemande un instantané : ce sont des rafales, et elles se traitent en amont (§ 10.3).

## 7. Quand ça va mal : bus morts, inondations, bus capricieux

### 7.1 Un bus entier tombe (V3 P1)

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

### 7.2 Le bus revient et inonde le hub (V3 P2)

Le bus revient et republie 5 000 valeurs d'un coup sur 700 capteurs, puis 5 vrais déclencheurs
arrivent ; 400 automatismes réels tournent.

| | Déclencheurs perdus | Latence des déclencheurs | Retards du moteur |
|---|---|---|---|
| Sans tampon d'entrée | **5 sur 5, aux trois essais** | — | 2 retards, ~2 600 événements manqués |
| Avec tampon d'entrée (dernière valeur gagne, vidé toutes les 10 ms) | **0** | 4,9 à 10,8 ms (6 à 15 ms sous Linux) | aucun |

5 000 valeurs sur 700 points deviennent ~700 événements. La correction du moteur (§ 8) reste
nécessaire pour les rafales que rien ne fusionne.

### 7.3 Un bus qui répond mal (V4 Q1 à Q3)

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
trames est un bus partiel. **La haute disponibilité de la § 14 ne remplace pas ces protections :
elle s'y ajoute.** Une bascule prend une seconde au mieux ; pendant cette seconde et pendant
toutes les pannes qui ne justifient pas une bascule, ce sont les voies et le disjoncteur qui
gardent la maison réactive.

### 7.4 Les autres pannes mesurées

| Panne | Sans le pattern | Avec le pattern | Preuve |
|---|---|---|---|
| Une lampe muette derrière un pont | 16 à 18 % d'ordres sains réussis, médiane ~4 s | 100 %, médiane 32 ms | V2 C2, V4 |
| API partagée limitée à 4 requêtes, une lampe muette | lampes saines à 1,4 s de médiane | 32 ms | V2 C3, V4 |
| Deux pilotes bloquent leur thread | ~200 ms par pas pour toute la maison | ≤ 0,1 ms de médiane pour le cœur | V2 C1, V4 |
| Un décodeur XML reçoit une réponse hostile | 96,6 s de CPU sur un worker partagé | le même coût, sur le seul thread du pilote, et refusé par le pré-contrôle | V1 E8 |
| La machine principale tombe | état vif et baux perdus | secours active en ~1 s, rien de perdu | V4 S1-S3 |

### 7.5 Les workflows quand quelque chose ne va pas

Le principe : **ce qui ne marche plus ne bloque jamais ce qui marche.**

1. **Chaque action rend un résultat typé** : faite, retenue par la garde, refusée, appareil
   injoignable (disjoncteur), délai dépassé, et, proposé en § 12, « envoyée sans confirmation ».
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

## 8. Le correctif de la perte sur retard, prouvé

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

## 9. Les voies paresseuses, écrites pour de bon

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

C'est le même mécanisme qui donne la confirmation de la § 12.

## 10. Les transports, mesurés

### 10.1 Ce que Moli utilise aujourd'hui

| Liaison | Protocole | Réglages relevés dans le code | Bornes |
|---|---|---|---|
| Zigbee2MQTT → Moli | MQTT 3.1.1 (`rumqttc` 0.25) | abonnement `zigbee2mqtt/#` en QoS 1, ordres `…/set` en QoS 1 non retenus, lectures `…/get` en QoS 0 à 2 par seconde, maintien 30 s, session propre | 4 Mio entrant, 256 Kio sortant, file de 64 |
| Frigate, Bambu, Landroid → Moli | MQTT (Bambu en TLS, Landroid sur WebSocket du cloud) | abonnements en QoS 0 ; ordres en QoS 1 (Frigate, Landroid) ou 0 (Bambu) | 256 Kio / 16 Kio |
| Samsung, Tuya cloud, Landroid | WebSocket (`moli-net`) | TLS épinglé pour Samsung | par pilote |
| ESPHome, Roborock, Android TV | TCP ou UDP natifs | Noise pour ESPHome, miIO, Polo | par pilote |
| Ponts et appareils HTTP, profils | HTTP(S) | interrogation périodique, délais bornés | par profil (5 s, 1 Mio par échange pour SMLIGHT) |
| Moli → tableau de bord | **SSE** `GET /api/events` + **REST** | un instantané, puis un événement par changement ; maintien 15 s ; non compressé | 8 flux par appelant, 64 en tout |
| Moli → agents | MCP (HTTP « streamable », `rmcp`) | 22 outils | la garde |

La discussion externe opposait HTTP par interrogation périodique, WebSocket et MQTT. Moli utilise
déjà la quatrième voie, **SSE**, pour pousser vers les écrans : une connexion, aucun sondage.

### 10.2 Ce qu'un message coûte sur le fil (banc T1)

<!-- FIGURE:fil -->

| Message (charge JSON de Zigbee2MQTT) | MQTT 3.1.1 QoS 0 | QoS 1 (+ accusé de 4 o) | MQTT 5 QoS 0 / 1 | **SSE de Moli** | WebSocket, mêmes événements |
|---|---|---|---|---|---|
| Thermomètre (64 o) | **97 o** | 99 + 4 o | 98 / 100 o | 4 événements → **467 o** | 367 o |
| Détecteur (50 o) | **83 o** | 85 + 4 o | 84 / 86 o | 3 → **352 o** | 277 o |
| Prise avec mesure (74 o) | **97 o** | 99 + 4 o | 98 / 100 o | 5 → **578 o** | 453 o |
| Ordre lampe (14 o, sur `…/set`) | **41 o** | 43 + 4 o | 42 / 44 o | 1 → 114 o, plus l'événement du journal : 244 o | 89 o |

Ce que le banc montre :

1. **Un relevé coûte quelques dizaines d'octets en MQTT, cinq fois plus en SSE.** Moli émet un
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

### 10.3 Combien de messages

Les deux maisons réelles produisent **environ 163 000 relevés par jour**, soit 1,9 par seconde en
moyenne et 239 par seconde à la pointe (V3). Au tarif du § 10.2 :

| Lien | Par relevé | Par jour, deux maisons |
|---|---|---|
| Zigbee2MQTT → courtier → Moli (MQTT QoS 0) | ~97 o, deux fois (vers le courtier, puis vers Moli) | ~32 Mo |
| Moli → un écran (SSE, un événement par attribut) | ~117 o par événement | ~19 Mo par écran ouvert toute la journée, non compressés |

La discussion externe estimait 69 920 messages par jour pour 100 appareils simulés : le même
ordre de grandeur. Elle avait aussi raison sur un point que les comptes réels confirment : **un
seul appareil bavard pèse plus que tout le reste**. Une prise qui publie sa puissance chaque seconde
fait 86 400 messages par jour ; cinquante détecteurs d'ouverture, 1 500. La déduplication au bord
(V6) et le tampon d'entrée (§ 4) s'en chargent.

### 10.4 Les gros messages (banc T2)

Ce ne sont pas les relevés qui posent problème, ce sont trois gros messages :

| Gros message | Taille mesurée | Quand | Risque | Ce qu'il faut |
|---|---|---|---|---|
| **L'instantané SSE** des deux maisons | **480 707 o** bruts (469 Kio) ; 17 469 o en gzip, 16 404 en brotli 4 ; 0,85 ms à sérialiser | à chaque connexion d'un écran, et à chaque retard (`Lagged`) | **jamais compressé** : `DefaultPredicate` de `tower-http` exclut `text/event-stream`. 64 flux × 469 Kio = 30 Mio d'un coup après un retard général ; un téléphone sur un mauvais réseau met des secondes à le recevoir | l'envoyer compressé à part (un `GET` compressé, puis le flux), ou le découper ; pour un invité, il est plus petit (ses pièces seulement) |
| **`bridge/devices`** de Zigbee2MQTT | ~3,3 Ko par appareil (fixture du dépôt : 26,5 Ko pour 8 appareils) | à chaque démarrage du pont et à chaque changement du réseau | **Mosquitto 2.1 refuse les paquets de plus de 2 000 000 o** par défaut et coupe le client : le pont tombe vers 600 appareils | régler `max_packet_size` sur le courtier ; Moli prévient dès 1 Mo |
| **Une série d'historique brute** | 11,5 Mo pour un an à la minute | si on la demandait d'un bloc | voir § 11 | jamais : au plus 2 000 points |

Un `Event::State` moyen fait 95 o de JSON, 116 o cadré en SSE. Une petite maison (50 appareils,
250 valeurs) a un instantané de 52 Ko.

### 10.5 Ce qu'une connexion coûte (banc T4)

Mémoire anonyme du processus serveur par connexion vivante (pente entre 64 et 1 000 connexions,
Linux, conteneur 2 CPU / 128 Mio), avec un vrai serveur axum 0.8.9 et les couches de Moli :

| Connexion | Par connexion | 64 connexions (le plafond de Moli) | Après l'envoi d'un instantané de 480 Ko | CPU par livraison |
|---|---|---|---|---|
| **SSE de Moli** (avec la couche de compression) | **39,1 Kio** | 2,5 Mio | 37 Kio : rien n'est retenu | 17 µs à 1 000 abonnés |
| SSE sans compression | 29,6 Kio | 1,9 Mio | — | 16 µs |
| **WebSocket, réglages par défaut d'axum** | **138,4 Kio** | 8,9 Mio | **480 à 607 Kio** : le tampon d'écriture garde la taille du plus gros message | 31 µs |
| WebSocket « léger » (lecture 4 Kio, écriture 0) | **11 Kio** | 0,7 Mio | 480 Kio (même effet) | 14 µs |

La couche de compression de Moli coûte à elle seule ~9,5 Kio par flux SSE : `CompressionBody`
pèse 5 816 o en ligne. Mille WebSocket aux réglages par défaut dépassent 128 Mio : le conteneur
est tué à 919 connexions. Moli plafonne à 64 flux, donc rien de tout cela n'est un risque
aujourd'hui.

Les chiffres de la discussion externe (14 à 64 Kio par connexion WebSocket) venaient d'une
bibliothèque Python. En Rust, l'ordre de grandeur est le même, mais il dépend surtout des réglages
des tampons : de 11 à 138 Kio pour le même protocole.

### 10.6 SSE ou WebSocket ?

| Critère | SSE + REST (aujourd'hui) | WebSocket |
|---|---|---|
| Mémoire par écran | 39 Kio | 11 Kio réglé, 138 Kio par défaut |
| Après un gros message | rien n'est retenu | le tampon garde sa taille, sauf si on le réduit |
| Un ordre et son issue | une requête REST par ordre (477 à 1 055 o), l'issue par le flux | sur la même connexion |
| L'historique | une requête REST compressée | par blocs, sur la même connexion |
| Simplicité, mandataires, Cloudflare Access | HTTP ordinaire, reconnexion automatique du navigateur | à régler, mais standard |

**À l'échelle d'une maison, aucun des deux ne pose de problème de mémoire ni de débit.** L'écart
qui compte est ailleurs : avec SSE, l'ordre et son issue suivent deux chemins différents.

La [V6](isolation-reactivite-runtime-v6.md) choisit un module WebSocket, réglé à 11 Kio par écran,
qui ne renvoie jamais un message de plus de 64 Kio. Il porte le direct, l'historique, les ordres
et leur issue sur une seule connexion.

## 11. L'historique

### 11.1 Pourquoi jamais par MQTT

La discussion externe a raison : envoyer l'historique d'un capteur dans un seul message MQTT pose
de vrais problèmes.

- **La taille** : depuis Mosquitto 2.1, un paquet de plus de 2 000 000 o est refusé par défaut.
  Un mois brut d'un capteur à la minute en fait déjà 946 Ko (§ 11.3).
- **La mémoire** : le courtier et le client gardent le message entier en tampon, et un JSON
  décodé occupe bien plus que ses octets.
- **La diffusion** : dix abonnés qui reçoivent les mêmes 5 Mo, c'est 50 Mo à livrer.
- **Le direct qui attend** : un gros message retient la connexion, et les relevés suivants font
  la queue derrière lui.

### 11.2 Ce que Moli fait déjà

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

### 11.3 Ce que pèse une courbe (banc T3)

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

### 11.4 Ce qui manque encore

| Besoin | Proposé |
|---|---|
| Une courbe qui dépasse la fenêtre locale (plus de 7 à 31 jours) | l'algorithme D1 du stockage V3 : heures et jours par l'archive, brut et bande en local, la valeur vive comme dernier point, archive injoignable signalée |
| Un export (CSV, une année entière) | un autre chemin que la courbe : par pages ou en flux (NDJSON, CSV), borné, repris par curseur, jamais une réponse de plusieurs Mo d'un bloc |
| Une courbe qui se prolonge en direct | le client continue la courbe avec les événements du flux ; Home Assistant fait de même avec `history/stream` |
| Des courbes instantanées | l'historique récent en mémoire selon le profil (§ 5.3) : 0,10 ms au lieu de 21,8 ms pour 24 h |

## 12. La livraison des ordres : reçu n'est pas fait

<!-- FIGURE:ordre -->

La discussion externe a raison sur l'essentiel : MQTT garantit au mieux qu'un message a été
**reçu** par le maillon suivant, jamais qu'une action a été **faite**. Le `PUBACK` d'un QoS 1 dit
que le courtier a pris le message ; il ne dit rien de la lampe.

### 12.1 Ce que Moli fait aujourd'hui, ligne par ligne

| Étape | Code | Ce qui est garanti |
|---|---|---|
| La garde accepte, l'ordre part avec une échéance absolue (5 s) | `hub.rs:816`, ARCHITECTURE « le chemin d'un ordre » | un ordre hors délai n'est jamais exécuté, même après une reconnexion |
| Le pilote refuse si le courtier n'est pas connecté | `moli-z2m/src/lib.rs:100-103, 130` | pas d'ordre qui attendrait dans une file locale pour partir des minutes plus tard |
| Le pilote publie `…/set` en QoS 1, non retenu | `lib.rs:238-250` | — |
| **Le pilote répond « fait » dès que `try_publish` est accepté** | `lib.rs:246-249` | seulement que la file locale de `rumqttc` (64 places) a pris l'ordre ; **rien n'a encore quitté la machine** |
| Une erreur de connexion termine le pilote, qui est relancé avec un client neuf | `lib.rs:107` | l'ordre resté dans l'ancienne file est perdu, **jamais rejoué** (règle tenue) |
| L'observateur écrit une ligne de journal par ordre | `hub.rs:357-430` | la réponse du pilote, « abandonné » ou « issue inconnue » |
| La confirmation revient comme un événement `state` | ARCHITECTURE, étape 5 | rien ne la rattache à l'ordre qui l'a demandée |

L'écart est donc précis. Moli répond « fait » plus tôt qu'un QoS 1 ne le permettrait : avant même
le `PUBACK`. Trois cas réels donnent un « fait » faux :

1. **Le courtier tombe entre `try_publish` et l'envoi** : l'ordre meurt avec le client. Le journal
   dit « fait ».
2. **Zigbee2MQTT est arrêté, le courtier vivant** : le courtier accepte le `PUBLISH`. Si la
   session de Zigbee2MQTT est propre, il ne le remet à personne. Si elle est persistante, il le
   remettra au retour du pont, peut-être bien après l'échéance, ce qui est pire (§ 12.3). Dans les
   deux cas, le journal dit « fait ».
3. **La lampe est hors du maillage**, ou coupée au mur : Zigbee2MQTT essaie, échoue, et l'écrit
   dans son propre journal (`bridge/logging`) ; aucun état ne revient. Le journal dit « fait ».

### 12.2 Trois issues au lieu d'une

La proposition sépare ce que l'on sait :

| Issue | Preuve | Ce que la famille voit | Ce qu'un workflow peut faire |
|---|---|---|---|
| **Confirmé** | l'état demandé est revenu de l'appareil avant l'échéance | l'état, normalement | continuer |
| **Envoyé, non confirmé** | le `PUBACK` du courtier est arrivé, aucun état à l'échéance | « pas de confirmation » sur la tuile, ligne de journal | brancher : réessayer une consigne, prévenir, prendre une autre voie |
| **Non remis** | pas de `PUBACK`, ou courtier déconnecté | « injoignable », comme un bus ouvert | idem, et le disjoncteur compte un échec |

« Confirmé » est la seule preuve qui compte ; « remis » est un diagnostic. Le travail se fait dans
la voie de l'appareil (§ 9) : l'ordre reste en vol jusqu'à l'état attendu ou jusqu'à l'échéance.

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

### 12.3 Doublons, idempotence et ordres différés

| Risque soulevé | Dans Moli | Reste à faire |
|---|---|---|
| QoS 1 peut livrer deux fois | les doublons naissent d'une retransmission après reconnexion d'une session persistante. Moli ouvre une **session propre** (`clean_session` vrai, la valeur par défaut de `rumqttc`) et recrée son client à chaque erreur : pas de retransmission | garder ce réglage ; l'écrire dans le contrat (§ 15.8) |
| Un ordre en double fait deux fois l'action | une consigne (`ON`, 128) posée deux fois donne le même état. Moli envoie des états explicites, pas des bascules (`TOGGLE`). Les **opérations** (appuis, compteurs) ont déjà leur clé d'idempotence `request_id` dans le hub | ne jamais traduire un ordre en `TOGGLE` ; réessais réservés aux consignes (V4) |
| Le courtier garde un ordre et le livre des heures plus tard (le portail) | Moli ne dépose rien dans une file du courtier : session propre, ordre refusé si déconnecté, échéance de 5 s | vérifier que Zigbee2MQTT se connecte en session propre sur l'installation (le courtier le montre à la connexion) ; en MQTT 5, poser une expiration de message égale au temps restant de l'échéance |
| Un `…/set` **retenu** rejoué à chaque redémarrage de Zigbee2MQTT | ignoré en silence : `is_request` écarte les `…/set` (`lib.rs:156, 205-210`) | **le signaler** : un `…/set` reçu avec le drapeau `retain` est un danger (une lampe rallumée à chaque redémarrage du pont). Moli le journalise, prévient, et l'efface (publication vide retenue), § 13.3 |

Bloc technique — en MQTT 5, l'échéance de Moli devient l'expiration du message (esquisse, `rumqttc::v5`) :

```rust
// Le temps restant de l'échéance, arrondi par défaut : un courtier ne peut pas livrer plus tard.
let reste = ordre.deadline.saturating_duration_since(Instant::now());
let mut props = PublishProperties::default();
props.message_expiry_interval = Some(reste.as_secs() as u32);   // 0 s : livré tout de suite ou jamais
client.try_publish_with_properties(sujet, QoS::AtLeastOnce, false, charge, props)?;
```

### 12.4 QoS, message par message

| Message | Aujourd'hui | Recommandé | Pourquoi |
|---|---|---|---|
| Relevé (température, puissance) | publié par Zigbee2MQTT (QoS 0 par défaut) ; Moli s'abonne à `…/#` en QoS 1 | inchangé | la QoS effective est la plus petite des deux : QoS 0. Un relevé perdu est remplacé par le suivant |
| Ordre `…/set` | QoS 1, non retenu | inchangé, plus la confirmation (§ 12.2) | le `PUBACK` sert au diagnostic, l'état à la preuve |
| Lecture `…/get` au démarrage | QoS 0, 2 par seconde (`lib.rs:122-128, 198`) | inchangé | une lecture perdue se refait ; le rythme protège le maillage |
| Disponibilité `…/availability` | retenue par Zigbee2MQTT | inchangé | c'est l'état qu'on veut retrouver à la connexion |
| `bridge/devices` | retenu | inchangé ; surveiller sa taille (§ 10.3) | le catalogue entier à chaque changement |
| Appui (`action`) | QoS du pont ; retenu ignoré (`keypad.rs`) | QoS 1 côté Zigbee2MQTT pour les appuis qui comptent (sonnette, alarme) | un appui ne se rattrape pas ; la clé d'idempotence évite le double |
| Frigate (événements), Bambu, Landroid | abonnements QoS 0, ordres QoS 1 (Frigate, Landroid) ou QoS 0 (Bambu, imposé par l'imprimante) | inchangé | la confirmation par l'état vaut pour eux aussi |

## 13. La sécurité du courtier : une porte de service autour de la garde

### 13.1 Ce que Moli fait aujourd'hui

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

### 13.2 Ce que le courtier permet à qui y est connecté

La règle 9 de l'architecture dit que **le binaire fait respecter les règles** : toute écriture
passe par le hub, donc par la garde et le journal. Mais un courtier ouvert crée une porte de
service :

| Qui peut… | …fait ceci | Ce que Moli voit |
|---|---|---|
| publier sur `zigbee2mqtt/<appareil>/set` | commande l'appareil **sans garde, sans approbation, sans journal** (une serrure, une chambre aux heures calmes) | l'état qui change, sans ordre derrière |
| publier sur `zigbee2mqtt/<appareil>` | fabrique de **faux états** : un faux mouvement lance un automatisme, une fausse porte fermée trompe l'alarme | un état ordinaire |
| lire `zigbee2mqtt/#` | suit la présence pièce par pièce (mouvement, portes, consommation) | rien |
| publier un `…/set` **retenu** | rejoue l'ordre à chaque redémarrage du pont | rien : le message est ignoré en silence (`lib.rs:156`) |
| ouvrir l'interface web de Zigbee2MQTT | tout ce qui précède, plus l'appairage et la suppression d'appareils | rien |

Sur un réseau domestique de confiance, le risque est faible. Mais la règle 10 traite le réseau
local comme hostile, et un seul objet compromis sur le LAN suffit.

### 13.3 Les correctifs, par rapport gain sur coût

1. **Le courtier n'écoute que la machine.** Quand Zigbee2MQTT et Moli tournent sur le même hôte,
   ce qui est le cas de la topologie retenue (§ 14.4 : les ponts suivent l'époque de Moli), le
   courtier n'a aucune raison d'écouter le LAN. Coût nul, et la porte se ferme.
2. **Un compte par client, aucun anonyme, des ACL par sujet.** Deux comptes seulement,
   Zigbee2MQTT et Moli ; tout autre client est refusé. Les ACL ne peuvent pas restreindre Moli
   aux seuls `…/set` quand les noms d'appareils ont plusieurs niveaux (bloc ci-dessous) : la
   protection vient surtout de l'absence de tout autre compte.
3. **Le mot de passe de Moli dans le coffre**, plus en variable d'environnement.
4. **TLS dès que le courtier sort de la machine** (un pont sur un autre hôte, un courtier
   partagé) : la fonction `use-rustls` de `rumqttc`, déjà dans l'arbre des dépendances par
   `moli-bambu`.
5. **Une taille de paquet écrite des deux côtés.** Mosquitto 2.1 a abaissé sa limite par défaut à
   **2 000 000 octets** (annexe C). Au-delà, le courtier refuse le paquet et coupe le client qui
   l'envoie : un `bridge/devices` trop gros coupe Zigbee2MQTT. Les 4 Mio réglés par Moli ne
   servent donc à rien sans le réglage correspondant du courtier (§ 10.3 pour les tailles).
6. **Moli signale ce qu'il ne devrait pas voir** :
   - un `…/set` qui ne vient pas de lui : il connaît ses propres ordres, et l'écho de `…/#` lui
     montre les autres ;
   - un `…/set` retenu, qu'il efface ;
   - un état sans appareil connu.

   Une ligne de journal et une notice à la famille suffisent : la porte de service devient au
   moins visible.
7. **L'interface web de Zigbee2MQTT** derrière un mot de passe, et sur la seule machine.

Bloc technique — `mosquitto.conf`, courtier local à l'hôte, comptes et ACL :

```conf
# Le courtier n'écoute que la machine : Zigbee2MQTT et Moli tournent sur le même hôte (§ 14.4).
listener 1883 127.0.0.1
allow_anonymous false
password_file /etc/mosquitto/passwd
acl_file /etc/mosquitto/acl

# Mosquitto 2.1 : 2 000 000 o par défaut. Écrit ici, aligné sur ce que Moli accepte (4 Mio).
max_packet_size 4194304

# Aucune session persistante utile : Moli et Zigbee2MQTT se reconnectent en session propre.
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
# plusieurs niveaux (salon/lampe), on ne peut pas restreindre Moli à …/set seulement.

# Zigbee2MQTT : son arbre entier (états, pont, ordres reçus).
user zigbee2mqtt
topic readwrite zigbee2mqtt/#

# Moli : lit tout le pont ; écrit dans l'arbre (ordres …/set, lectures …/get).
user moli-os
topic read  zigbee2mqtt/#
topic write zigbee2mqtt/#
# Si les noms d'appareils n'ont qu'un niveau, resserrer :
#   topic write zigbee2mqtt/+/set
#   topic write zigbee2mqtt/+/get
```

`persistence false` et `max_queued_messages` vont avec les sessions propres du § 12.3 : le
courtier n'a rien à sauver. Ce point compte pour la haute disponibilité (§ 14.5) : un courtier
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

## 14. La haute disponibilité, de bout en bout

La discussion externe a conçu une architecture à haute disponibilité pour deux boîtiers
identiques à deux radios (Zigbee et Thread), avec MQTT comme passerelle commune. Le boîtier
annoncé correspond à la famille SMLIGHT SLZB-MR : deux puces séparées, Zigbee et Thread en même
temps, alimentation PoE (annexe C). Le modèle exact reste à confirmer sur la page
d'administration du boîtier. La V5 garde le fond de cette architecture, et la ramène à la taille
d'une maison et aux règles de Moli.

### 14.1 Ce que la V5 garde, et ce qu'elle change

| Proposition externe | Décision V5 | Pourquoi |
|---|---|---|
| Deux boîtiers double radio identiques, sur des alimentations et des switchs distincts | **gardé** | deux domaines de panne ; même famille de puce, condition d'une restauration Zigbee |
| Thread actif/actif sur les deux boîtiers | **gardé** | natif : plusieurs routeurs de bordure sur un même réseau, avec le même jeu de données |
| Zigbee2MQTT actif/passif sur deux VM | **gardé, mais attaché à l'époque de Moli** | un seul mécanisme de bascule pour toute la maison, pas deux qui pourraient se contredire |
| Coordinateur Zigbee de secours, promotion automatique « à valider » | **promotion humaine d'abord**, automatique seulement après le test 2 réussi sur le matériel réel | identité IEEE, clés, compteur de trames : une restauration ratée coûte un réappairage de toute la maison |
| Clore la seule radio Zigbee de A, en gardant Thread A | **clore tout le boîtier A par son port PoE** | Thread B route déjà seul ; couper l'alimentation est la seule clôture sûre ; le profil `smlight` de Moli ne fait volontairement aucune écriture vers le boîtier |
| Matter Server actif/passif, plus un pont `matter2mqtt` (Go) | **contrôleur Matter attaché à l'époque ; Moli lui parle directement**, ou embarque `rs-matter` plus tard (D9) ; pas de `matter2mqtt` | un maillon et un langage de moins ; le serveur de référence est maintenant Matter Server 9.0, sur matter.js |
| Cluster RMQTT à trois nœuds Raft | **non** : un Mosquitto local à l'hôte actif, sans état | journal Raft en mémoire seulement, intervention manuelle si la moitié des nœuds tombe, messages retenus en cluster via Redis (annexe C) ; un courtier sans état n'a rien à répliquer (§ 14.5) |
| Historique dans PostgreSQL ou TimescaleDB | **non** : Turso et archive Parquet | décidé (D13), mesuré (stockage V3) |
| Prometheus, Grafana, Alertmanager | **non** : la santé est déjà dans Moli (`/api/health`, page Système, notices) | le budget d'un Pi ; une seule vérité sur l'état de la maison |
| Liaison Zigbee directe pour l'essentiel | **gardé** | la redondance la moins chère (§ 14.6) |
| Une batterie de tests de reprise avant toute automatisation | **gardé**, porté à quinze tests (§ 14.10) | — |

### 14.2 Les domaines de panne

| Composant | Panne | Ce qui se passe aujourd'hui | Avec la V5 | Test |
|---|---|---|---|---|
| Zigbee2MQTT (processus) | plante, redémarre en boucle | **Moli ne le voit pas** : il ignore `zigbee2mqtt/bridge/state` (`lib.rs:156`). Les appareils restent « en ligne », les ordres partent vers le courtier et répondent « fait » | `bridge/state` = `offline` (le testament MQTT du pont) ouvre le disjoncteur du bus : appareils hors ligne, ordres refusés en 0,1 ms. Relance locale ; aucune bascule | 1 |
| Courtier | tombe | le pilote sort en erreur, ses appareils passent hors ligne, il est relancé avec une attente croissante ; ordres refusés (« broker injoignable ») | inchangé (c'est déjà le bon comportement) ; relance locale | 10 |
| Hôte H1 | gel, coupure, disque | tout s'arrête | la secours H2 prend l'époque N+1 : Moli en ~1 s (V4), puis le courtier et les ponts | 6, 7 |
| Boîtier A | alimentation, carte | Zigbee et Thread de A perdus | Thread : B route seul, sans rien faire. Zigbee : bus ouvert, notice, puis promotion de B | 2, 4 |
| Lien H1 – H2 | coupure entre les deux | — | le témoin départage : seul le porteur du bail agit | 8, 9 |
| Switch PoE 1 | tombe | A tombe (et H1 s'il en dépend) | comme « boîtier A » ou « domaine 1 » | 6 |
| Contrôleur Matter | plante | appareils Matter hors ligne | relance locale ; il suit l'époque si l'hôte change | 5 |
| `bridge/devices` trop gros | Mosquitto 2.1 refuse le paquet | le courtier coupe Zigbee2MQTT ; le catalogue ne passe plus | `max_packet_size` écrit (§ 13.3) ; Moli prévient bien avant | 11 |

La première ligne est un écart à corriger, indépendant de toute haute disponibilité : c'est le
« signal du transport » que la V3 demandait déjà pour le disjoncteur de bus.

### 14.3 Ce qui se redonde, et ce qui ne se redonde pas

| Élément | Écrivains possibles en même temps | Mécanisme |
|---|---|---|
| Routeur de bordure Thread | **plusieurs** | actif/actif : même jeu de données opérationnel (*Active Operational Dataset*) |
| Coordinateur Zigbee | **un seul au monde** pour un réseau donné | actif/secours : sauvegarde, clôture de l'ancien, restauration sur le nouveau |
| Fabric Matter (le stockage du contrôleur) | **un seul** | actif/passif : stockage copié, jamais ouvert par deux contrôleurs |
| Zigbee2MQTT | un seul, attaché au coordinateur | actif/passif, suit l'époque |
| Courtier MQTT | un par hôte, utilisé par l'actif seul | sans état : rien à répliquer |
| Moli | un seul porteur de l'époque | actif/passif : journal, instantanés, époque (V4) |
| Liaison Zigbee directe | aucune : l'appareil agit seul | rien à basculer |

Une seule règle couvre les cinq premières lignes : **ce qui porte une identité n'a qu'un
écrivain à la fois, désigné par l'époque, l'ancien clos avant que le nouveau démarre.**

### 14.4 La topologie retenue

<!-- FIGURE:topologie -->

- **Deux domaines de panne.** Chacun a son boîtier, son hôte, son switch PoE et sa prise (un
  onduleur si possible).
- **Moli porte l'époque**, et les ponts la suivent. Zigbee2MQTT et le contrôleur Matter ne
  tournent que sur l'hôte qui porte l'époque. Comme Moli, Zigbee2MQTT et le courtier sont sur le
  même hôte, le courtier reste local (§ 13.3).
- **Ce qui part vers la secours :**

  | Quoi | Taille | Quand |
  |---|---|---|
  | Journal de reprise et instantanés de Moli | ~2,6 Mo par jour | en continu (V4) |
  | Données de Zigbee2MQTT : `configuration.yaml`, `database.db`, `coordinator_backup.json`, `state.json` | quelques centaines de Kio pour une maison (estimé) | à chaque changement et à l'arrêt propre |
  | Stockage du contrôleur Matter : la fabric, les nœuds | quelques Kio à quelques Mio (estimé) | à chaque changement |
  | Courtier | rien | — |

  Le canal de réplication de Moli peut porter ces fichiers, ce qui évite un second outil de
  copie (proposé).
- **Le témoin** est un troisième appareil déjà présent (NAS, routeur, Pi). Il ne fait que voter
  (§ 14.7).

### 14.5 Le courtier : actif là où sont les ponts, sans état

| Option | Mémoire | À répliquer | Bascule | Verdict |
|---|---|---|---|---|
| **Mosquitto local à l'hôte actif** (recommandé) | 1 à 2 Mio (estimé) | rien : `persistence false`, sessions propres | il est déjà là : les ponts et Moli s'y connectent sur l'hôte qui porte l'époque | simple, suffisant |
| Courtier embarqué dans Moli (`rumqttd`) | ~1 Kio par point retenu (2,3 Mio pour les deux maisons), 62 Kio par client TCP, journal de 2,6 à 11 Mio selon ses segments (mesuré, V6 § 17) | rien | avec Moli | possible ; mais un serveur réseau de plus dans le cœur, pour aucun gain mesurable |
| Cluster RMQTT, trois nœuds Raft | plusieurs dizaines de Mio par nœud (estimé) | journal Raft en mémoire seulement ; messages retenus via Redis en cluster | quorum de 2 sur 3 ; intervention manuelle si la moitié des nœuds tombe | non pour une maison : trois machines, une base de plus, aucun état utile à garder |
| EMQX, HiveMQ | centaines de Mio (estimé) | — | — | non |

Le point qui fait pencher : **pour Moli, le courtier n'a aucun état à préserver.** Zigbee2MQTT
republie ses messages retenus (disponibilités, `bridge/devices`) à son démarrage, Moli relit les
états des appareils à l'écoute (`…/get`, `lib.rs:179-196`), et aucun ordre ne doit survivre dans
une file. Répliquer un courtier sans état, c'est ne rien répliquer.

### 14.6 Les radios

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
- Moli lui parle par son API WebSocket, comme le pilote Samsung le fait déjà avec un téléviseur
  (WebSocket TLS épinglé, `moli-samsung`).
- Si D9 fait de Moli le contrôleur (`rs-matter`), la fabric vit dans le coffre et suit
  l'époque avec le journal : plus de processus séparé.
- Un appareil Matter accepte plusieurs fabrics (*multi-admin*). Une seconde fabric, celle d'un
  téléphone, garde une commande manuelle possible pendant une panne du contrôleur. Ce n'est pas
  une bascule, mais c'est une issue de secours qui ne coûte rien.

**La liaison Zigbee directe : la redondance la moins chère.** Un interrupteur lié à une lampe ou à
un groupe la commande sans coordinateur actif, sans courtier et sans Moli. Pour les fonctions qui
ne doivent jamais manquer (l'entrée, l'escalier, la salle de bains), c'est plus sûr que toute
bascule. Cela complète la règle D12 sur les luminaires : la lampe reste pilotable même quand tout
le reste est tombé.

### 14.7 Le témoin, le bail et la décision

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
                    → démarrer : Moli, courtier, Zigbee2MQTT, contrôleur Matter
ancien actif      : revient, annonce N, apprend N+1, reste passif ; ses ponts restent arrêtés
```

L'actif se rétrograde à 2 s, avant la fin de son bail à 3 s. Quand le témoin accorde N+1, l'ancien
a donc cessé d'agir, même s'il est vivant mais isolé. Le prix : une reprise en 3 à 5 s au lieu
de la seconde mesurée en V4 sans témoin (estimé). Pour une maison, c'est un bon échange.

### 14.8 Trois pannes, trois reprises

<!-- FIGURE:bascule -->

- **Zigbee2MQTT plante, l'hôte vit** : c'est la panne la plus fréquente, et elle ne justifie
  aucune bascule. Le disjoncteur s'ouvre sur `bridge/state`, le service redémarre sur place avec
  la même radio, le disjoncteur se referme au ConnAck et aux premiers états.
- **L'hôte H1 tombe** : la secours prend l'époque N+1. Moli est prêt en quelques millisecondes ;
  le courtier, Zigbee2MQTT (avec la radio A, par TCP) et le contrôleur Matter (avec la même
  fabric) démarrent ensuite. Les ordres d'avant la panne ne partent jamais : leur échéance est
  passée.
- **Le boîtier A tombe** : Thread continue par B sans rien faire. Pour Zigbee, le bus s'ouvre,
  la famille est prévenue, un humain décide ; on coupe le port PoE de A, on restaure sur B, on
  teste un appareil sur secteur et un sur pile.

### 14.9 Ce que coûte la haute disponibilité

| Poste | Une machine | La topologie V5 |
|---|---|---|
| Matériel | 1 hôte, 1 boîtier | 2 hôtes, 2 boîtiers, 2 switchs PoE (ou injecteurs pilotables), un témoin déjà présent |
| Mémoire de Moli | profil sobre ~8 Mio | la même sur chaque hôte : la secours tient l'état vif à jour |
| Zigbee2MQTT et contrôleur Matter (Node.js) | une centaine de Mio chacun et plus (estimé) | sur l'hôte actif seulement ; la secours les garde arrêtés |
| Réseau | — | ~2,6 Mo par jour de journal, plus les fichiers des ponts |
| Hôte | Pi Zero 2 W possible pour Moli seul (D10) | Pi 4 ou 5, ou mini-PC, dès que Zigbee2MQTT et Matter tournent à côté |
| Exploitation | — | une époque, un témoin, quinze tests, une procédure de retour de A |
| Ce qu'on y gagne | panne d'hôte : des heures, jusqu'à une intervention | panne d'hôte : quelques secondes. Boîtier : Thread tout de suite, Zigbee en minutes |

Sans aucun matériel de plus, trois mesures donnent déjà l'essentiel. Ce sont les plus rentables :

- les liaisons Zigbee directes pour les fonctions essentielles ;
- `bridge/state` qui ouvre le disjoncteur ;
- une sauvegarde de `coordinator_backup.json` hors de la machine.

### 14.10 Quinze tests de reprise

Chaque test a un critère mesurable. Aucune promotion ne devient automatique avant que les
tests 2, 7, 8 et 9 passent sur le matériel réel.

| # | Scénario | Comment | Réussi si |
|---|---|---|---|
| 1 | Zigbee2MQTT tué sur l'hôte actif | `kill -9` du processus | disjoncteur ouvert en moins d'1 s par `bridge/state` ; refermé au retour ; aucun ordre parti après son échéance |
| 2 | Restauration du coordinateur A → B | couper le port PoE de A ; démarrer Zigbee2MQTT sur B avec la sauvegarde | tous les appareils répondent, dont un sur pile, sans réappairage ; clé, PAN, canal et compteur vérifiés ; durée notée |
| 3 | Retour inopiné de A après la promotion de B | rallumer le port de A | A ne forme pas de second réseau (balayage radio) ; aucun Zigbee2MQTT ne s'y connecte |
| 4 | Thread : un routeur, puis l'autre | couper A, rétablir, couper B | les appareils Matter répondent dans les deux cas, sans action |
| 5 | Contrôleur Matter déplacé | arrêt sur H1, démarrage sur H2 avec la fabric copiée | aucun nouvel appairage ; ordres et états passent |
| 6 | Perte du domaine 1 entier | couper H1, le switch PoE 1 et A ensemble | Moli reprend sur H2 sans perte (RPO 0) ; Thread continue ; la procédure Zigbee se déroule comme prévu |
| 7 | Gel de H1 | `SIGSTOP`, ou gel de la VM | H2 actif après l'expiration du bail ; H1 dégelé reste passif, ponts arrêtés |
| 8 | Coupure entre H1 et H2, témoin visible des deux | filtrer le lien | un seul porteur d'époque ; jamais deux Zigbee2MQTT connectés au courtier ni à la radio |
| 9 | H1 isolé de tout | débrancher son réseau | H1 se rétrograde avant la fin du bail ; H2 promu ensuite ; aucun intervalle avec deux actifs |
| 10 | Courtier tué sur l'hôte actif | `kill -9` de Mosquitto | ordres refusés en 0,1 ms ; appareils hors ligne ; reprise quelques secondes après le retour |
| 11 | `bridge/devices` au-delà de 2 000 000 o | catalogue gonflé, Mosquitto 2.1 par défaut, puis `max_packet_size` réglé | la coupure est constatée, puis disparaît ; Moli prévient dès 1 Mo |
| 12 | Ordre pendant une bascule | ordre envoyé 100 ms avant le gel | issue « envoyé, non confirmé » ou « non remis » ; jamais exécuté après l'échéance ; une ligne de journal |
| 13 | `…/set` retenu déposé par un tiers | publication retenue | Moli le signale et l'efface ; au redémarrage du pont, rien ne se rallume |
| 14 | Client MQTT inconnu, ou compte Moli hors de son arbre | connexion anonyme, publication interdite | refusés par le courtier ; tentative journalisée |
| 15 | Liaison directe, tout éteint | couper H1, H2, les courtiers et le boîtier A (le coordinateur) | l'interrupteur lié allume toujours sa lampe, par les routeurs du maillage |

## 15. Les contrats

Ce que chaque brique du pattern devrait exposer, avec ses règles. Ce sont des esquisses pour cadrer
l'implémentation, pas du code livré ; les noms sont en français comme le reste du domaine. Les
contrats 15.1 à 15.6 viennent de la V4 ; 15.7 à 15.10 sont nouveaux.

### 15.1 `spawn_driver_isole` : un thread et un runtime par pilote

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

### 15.2 `Voies` : une file par appareil, à la demande

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
- une consigne qui attend est remplacée par la suivante pour le même point ; un appui ou une
  impulsion n'est jamais remplacé ;
- à l'ouverture du disjoncteur, la file de l'appareil est vidée avec un refus immédiat ;
- l'essai de réouverture est un ordre réel (le prochain), jamais une requête fabriquée.

### 15.3 `Passerelle` : une API partagée

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

### 15.4 `DisjoncteurDeBus` : à hystérésis, avec budget de latence

| Paramètre | Valeur proposée | Pourquoi |
|---|---|---|
| Ouverture | 3 échecs **consécutifs** d'ordres, ou le signal du transport : déconnexion MQTT, `bridge/state` = `offline`, pont injoignable | un seul appareil muet ne doit pas ouvrir le bus ; un bus partiel reste utilisable |
| Durée minimale ouvert | 5 s | un bus qui bat ne fait pas osciller la maison (Q2) |
| Essai | un ordre réel, délai court (500 ms), intervalle 1 s doublé jusqu'à 30 s | reprise en 0,5 s après le retour (P1), sans marteler un bus mort |
| Fermeture | 3 succès consécutifs, ou le signal de reconnexion du transport (ConnAck, `bridge/state` = `online`) | un essai réussi par hasard ne rouvre pas la vanne |
| Dégradé | médiane des 20 dernières latences > 1,5 s | un bus lent sous le délai (Q1) est traité avant que les files ne débordent |
| En dégradé | refus prédictif ; délai par ordre ramené à 1 s ; un seul ordre en attente par appareil ; sondage suspendu | la latence des ordres reste bornée, le workflow sait vite |
| Effet visible | tous les appareils du bus hors ligne ; ordres refusés en 0,1 ms ; notice à la famille ; raison dans `/api/health` | la panne se voit, elle ne se devine pas |

### 15.5 Le tampon d'entrée du hub

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

### 15.6 Le journal, l'instantané, l'époque

| Pièce | Forme | Règle |
|---|---|---|
| Journal | une ligne binaire par changement : `seq u64, point u32, charge, instant u64` (~9 octets compressés) ; validé par lots (≤ 100 ms ou 500 lignes) | la source de la reprise et de la réplication ; jamais réécrit |
| Instantané | binaire compact de l'état vif `(point u32, étiquette u8, charge, 3 instants)`, avec le `seq` du journal | au quart d'heure sur disque ; chaque seconde vers le réseau si une secours écoute |
| Baux et exécutions | dans le journal (ouverture, prolongation, fin) et dans l'instantané | repris au démarrage, appliqués une fois |
| Époque | entier monotone, écrit sur disque **avant** toute activation, annoncé à la connexion | la plus grande gagne ; l'autre reste passive |
| Secours | reçoit instantané et journal ; applique en mémoire ; **n'ouvre aucune connexion** | bascule sur décision (humain, ou témoin et bail : § 14.7), après la clôture de l'ancienne |

### 15.7 L'issue d'un ordre (nouveau)

| Règle | Détail |
|---|---|
| Quatre issues | `Confirme`, `EnvoyeNonConfirme`, `NonRemis`, `Refuse` (§ 12.2) |
| Confirmation | la valeur canonique revenue égale la valeur demandée, à la tolérance du point près, avant l'échéance |
| Point muet | déclaré par le profil ou le pilote ; la remise au courtier suffit |
| Journal | l'observateur écrit l'issue finale, une ligne par ordre, comme aujourd'hui |
| Workflow | une étape peut brancher sur `EnvoyeNonConfirme` et `NonRemis`, comme sur un délai |
| API | un champ `issue` ajouté à la réponse de `POST /api/command` et de `send_command` ; l'existant ne change pas |

### 15.8 Les options MQTT d'un pilote (nouveau)

| Option | Valeur | Pourquoi |
|---|---|---|
| Session | **propre** (`clean_session = true`), client recréé à chaque erreur | aucun ordre retransmis ni différé (§ 12.3) |
| Maintien de liaison | 30 s | coût négligeable, détection en 45 s au pire côté courtier |
| Taille de paquet | entrée et sortie **écrites des deux côtés** : pilote et courtier | Mosquitto 2.1 coupe au-delà de 2 000 000 o par défaut |
| Ordres | QoS 1, jamais retenus ; en MQTT 5, expiration = reste de l'échéance | — |
| Retenus reçus | un `…/set` retenu est signalé et effacé | § 13.3 |
| Signal du pont | `bridge/state` (le testament MQTT) lu et relié au disjoncteur | § 14.2 |
| Secret | mot de passe au coffre, plus en variable d'environnement | règle 6 |
| TLS | obligatoire si le courtier n'est pas sur la machine | règle 10 |

### 15.9 Le rôle des ponts, attaché à l'époque (nouveau)

Bloc technique — ce que Moli fait des ponts quand l'époque change (esquisse) :

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

- ordre d'activation : clôture, époque N+1 sur disque, Moli, courtier, ponts ;
- ordre d'arrêt (rétrogradation) : ponts, puis refus des ordres ; Moli garde son état vif ;
- les fichiers d'un pont partent vers la secours à chaque changement et à l'arrêt propre ;
- la secours ne démarre jamais un pont tant qu'elle ne porte pas l'époque.

### 15.10 L'API d'historique (nouveau)

| Règle | Détail |
|---|---|
| Une courbe | `GET /api/history` : 1 min à 1 an, 10 à 2 000 points, 300 par défaut ; brute si le nombre de changements tient, sinon des seaux avec moyenne, min, max et nombre ; compressée |
| Les points demandés | ceux de la largeur du graphique en pixels, pas plus (§ 11.3) |
| Au-delà de la fenêtre locale | l'algorithme D1 du stockage V3 : heures et jours par l'archive, brut local, valeur vive en dernier point, archive injoignable signalée |
| Un export | un autre chemin, en pages ou en flux (NDJSON ou CSV), borné, repris par curseur ; jamais une réponse unique de plusieurs Mo |
| Le temps réel | jamais par l'historique : la courbe se prolonge avec les événements du flux SSE |

## 16. Le modèle de coût

Des coefficients unitaires, tirés des mesures, pour chiffrer n'importe quelle maison. Notations :
`V` valeurs (points), `A` appareils, `N` automatismes, `R` relevés par jour, `P` pilotes, `E`
écrans ouverts.

### 16.1 La mémoire

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

### 16.2 Les octets sur le fil

| Poste | Coefficient | D'où il vient |
|---|---|---|
| Un relevé, MQTT QoS 0 | ~97 o (charge JSON de Zigbee2MQTT + 33 o) | V5 T1 |
| Un relevé vers un écran (SSE) | ~117 o par attribut, soit ×4 à ×6 le message MQTT | V5 T1 |
| Un ordre | 84 o côté MQTT ; 477 à 1 055 o pour un client HTTP, selon ses en-têtes | V5 T1 |
| L'instantané d'un écran | ~212 o × V, non compressé (480 Ko pour les deux maisons) | V5 T2 |
| Une courbe | 78 o par seau, 22 o par point brut ; ÷5 en brotli | V5 T3 |
| Maintien de liaison | SSE 45 Kio par jour et par écran ; MQTT 11 Kio par jour et par liaison | V5 T1 |

### 16.3 Le processeur

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

## 17. Le plan complet

Chaque étape apporte un gain seule. L'ordre suit le rapport gain sur coût. Les étapes marquées
**nouveau** viennent de la V5 ; les autres sont celles des V3 et V4, avec leur état.

| Étape | Gain | Coût à l'exécution | Effort | Preuve d'acceptation | État |
|---|---|---|---|---|---|
| 1. Correctif de la perte sur retard | plus aucun déclencheur perdu sur un retard | ~1 ms par retard | faible | V2 W2 rejoué : 0 perdu | **prouvé** sur copie (§ 8) : à reporter tel quel |
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
| 16. Ponts attachés à l'époque, fichiers des ponts répliqués, témoin et bail (**nouveau**) | la maison entière, pas seulement Moli, change d'hôte | réseau : quelques Mo par jour | élevé | tests 5 à 9, 12 | à faire |
| 17. Second boîtier radio, PoE pilotable, procédure de promotion Zigbee (**nouveau**) | Thread sans coupure ; Zigbee repris en minutes | matériel | moyen | tests 2, 3, 4, 6 | à faire, après l'étape 16 |
| 18. Export d'historique en flux (**nouveau**, au besoin) | des exports de plusieurs Mo sans réponse géante | nul au repos | faible | export d'un an d'un capteur à la minute | au besoin |

Sans aucun matériel de plus, les étapes 1 à 9 rendent la maison plus réactive, plus honnête sur
ce qu'elle a fait, et moins exposée. La haute disponibilité (étapes 15 à 17) vient ensuite, et
elle s'appuie sur elles : une bascule ne remplace ni les voies, ni le disjoncteur, ni la
confirmation des ordres.

## 18. Limites

- **Machines de mesure.** Le conteneur Linux tourne sur le Mac : même architecture que le Pi
  (arm64), mais un cœur 8 à 10 fois plus rapide. Aucune mesure sur un Pi Zero 2 W, ni sur une
  carte SD.
- **Bancs de la V5.** Ils mesurent les octets et la mémoire du processus serveur. Ils ne mesurent
  ni le TLS, ni le chemin réel jusqu'à un téléphone, ni la mémoire des tampons du noyau, sauf
  mention contraire dans l'annexe A.
- **Haute disponibilité.** Rien n'a été joué sur du matériel réel : ni la restauration du
  coordinateur (adresse IEEE, compteur de trames), ni le partage du jeu de données Thread, ni le
  déplacement d'une fabric Matter. Le modèle exact du boîtier n'est pas confirmé. Les durées
  marquées « estimé » le restent jusqu'aux tests de la § 14.10.
- **Le témoin et le bail** sont conçus, pas mesurés. Leur reprise en 3 à 5 s est une estimation.
- **La confirmation des ordres** a un coût de latence estimé, pas mesuré.
- **Sources externes.** Seules les affirmations qui pèsent sur une décision ont été vérifiées
  (annexe C). Les autres chiffres de la discussion externe ne sont pas repris comme des faits.
- **Limites héritées.** Les bus capricieux de la V4 sont simulés par des délais. Le correctif du
  moteur est prouvé sur une copie. Le modèle de coût est linéaire. Les formes Parquet du stockage
  V3 ont été mesurées sur des valeurs synthétiques.

## Annexe A — Bancs de la V5

Une crate jetable `preuves-v5`, **hors du dépôt**, qui dépend par chemin des vraies crates
`moli-core`, `moli-runtime` et `moli-history`. Elle utilise aussi les versions du dépôt d'axum
(0.8.9), de `tower-http` (0.7.1) et de `rumqttc` (0.25.1), ainsi que `tungstenite` 0.29.0 et
`rumqttd` 0.20.0. La chaîne Linux est celle de la V4 : compilation musl dans un conteneur
`rust:1-alpine`, exécution dans `alpine:3` limité à 2 CPU et 128 Mio. Les chiffres retenus sont
ceux du conteneur Linux ; le Mac a servi à la mise au point.

| Banc | Ce qu'il mesure |
|---|---|
| `tailles` (T1) | les octets sur le fil : MQTT encodé par `rumqttc`, SSE et WebSocket lus sur la socket d'un vrai serveur axum avec les couches de Moli, `POST /api/command` avec trois jeux d'en-têtes, les maintiens de liaison |
| `instantane` (T2) | le JSON de `hub.snapshot()` pour les deux maisons et pour une petite maison : brut, gzip, brotli, temps de sérialisation |
| `historique` (T3) | de vraies `moli_history::Series` : seaux, brut, compression, temps ; contrôle sur le fil avec la compression de Moli |
| `connexions` (T4) | la mémoire anonyme du processus serveur par connexion SSE ou WebSocket, de 1 à 1 000 connexions, et le CPU de diffusion |
| `etat_vif` | l'état vif seul (sans historique) à ×1, ×3 et ×10 : octets demandés et mémoire réelle du cgroup |
| `courtier` (V6) | `rumqttd` 0.20.0 embarqué : liens locaux, retenus, abonnés TCP, rafales, journal, seuil de retard, coût d'une ACL |

Les limites des bancs :

- chaque point n'a été mesuré qu'une fois ; les queues de latence sont donc bruitées ;
- les maisons sont synthétiques, et compressent mieux que du vrai JSON ;
- le profil de compilation est sans LTO ;
- les clients tournent sur la boucle locale, donc rapides : un client lent sur un vrai réseau
  garderait plus de données en tampon.

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

## Annexe B — Les bancs des V2 à V4 et les séries du stockage

La V5 cite ces mesures sans les rejouer. Leurs codes et leurs sorties brutes restent dans leurs
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

## Annexe C — Sources externes vérifiées

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
