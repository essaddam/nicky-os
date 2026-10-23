# Analyse — Isolation des pannes, réactivité et optimisation Rust du runtime

Date : 21 octobre 2026. Statut : **analyse, aucune modification du code**.
Base analysée : `main` au commit `d0ce806`. Les exemples sont fictifs.

Périmètre : 57 crates, environ 125 000 lignes de Rust hors tests (160 000 avec).
Le cœur (`moli-runtime`, `moli-os`, `moli-api`, `moli-automation`, `moli-history`,
`moli-net`) a été lu en détail. Les 45 crates de pilotes et de briques ont été lues selon
une même grille (trois relectures parallèles), et chaque constat de gravité élevée a été
revérifié dans le code. Huit expériences ont été exécutées contre le vrai `moli-runtime`
et la vraie version de `roxmltree`, dans des crates jetables hors du dépôt
([annexe A](#annexe-a--preuves-exécutables)). Les décisions de
l'[architecture](../ARCHITECTURE.md) (D0 à D13) sont tenues pour acquises.

## 1. Réponse courte

**Oui pour les paniques des pilotes, pas encore pour le reste.** Le cœur sépare bien les
pilotes entre eux : un pilote qui panique est capturé, relancé avec une attente croissante,
ses ordres en attente sont refusés et ses appareils passent hors ligne ; le reste de la
maison ne voit rien (E1 : 3 paniques en 3,5 s, 156 ordres sur 156 servis pendant ce temps,
médiane 0,06 ms).

Mais « planter » prend plusieurs formes, et le superviseur n'en attrape qu'une :

| Défaillance d'un module | Effet aujourd'hui | Preuve | Gravité |
|---|---|---|---|
| Panique dans `Driver::run` | Isolée, relancée (1 s → 60 s), statut public | E1 | Maîtrisé |
| Appel bloquant ou calcul long sur un worker | Un seul : absorbé par le second worker. Deux : toute la maison avance par pas de ~200 ms. Une réponse XML hostile de 1 Mio occupe un worker 96,6 s | E2, E8 | **Élevé** |
| Pilote figé, ou « prêt » avant d'être connecté | Reste `running` pour toujours ; chaque ordre échoue au bout de 5 s ; jamais relancé | E3 | **Élevé** |
| Panique dans une brique (moteur d'automatismes, historique, énergie, horloges) | La brique meurt en silence, rien ne la relance, `/api/health` reste vert | E4 | **Élevé** |
| Appareil lent derrière un pilote qui traite tout à la file | Les autres ordres du pilote expirent ; pour le pilote des luminaires, toutes les lumières de la maison attendent | lecture du code | **Élevé** (luminaires), Moyen ailleurs |
| Ordre émis après que l'appelant a renoncé | L'appelant lit « délai dépassé », l'appareil agit quand même, et après l'échéance le journal dit aussi « délai dépassé » (9 pilotes) | lecture du code | Moyen |
| Faute de processus (débordement de pile, mémoire, `abort`) | Tout le processus s'arrête (code 134) ; Docker relance ; boucle possible si l'entrée fautive persiste | E7 | Moyen (aucun cas trouvé) |
| Erreur à la construction d'un pilote (fichier de données illisible) | Toute la maison refuse de démarrer | lecture du code | Moyen |

**Réactivité.** Le cœur est très rapide et bien découplé : environ 100 000 ordres
aller-retour par seconde sur la machine de test, un abonné lent ne freine personne (E5),
le verrou global du modèle n'est pas un goulot à l'échelle d'une maison (E6). Les risques de
latence sont aux bords : deux workers seulement, des pilotes qui font leurs entrées-sorties
dans leur boucle de commandes, et un moteur d'automatismes qui traite tous les événements à
la file, écritures disque synchronisées comprises.

**Pilotes et services tiers.** La grille est rassurante sur l'essentiel : aucun
`std::thread::sleep`, aucun `std::net` synchrone, aucun canal non borné, des tampons
toujours plafonnés, des décodeurs qui vérifient avant d'indexer, pas de récursion sur des
données réseau. Les défauts sont des défauts de forme, répétés : entrées-sorties dans la
boucle de commandes, garde d'abandon pas toujours appliquée, quelques sous-tâches lâchées,
deux pilotes qui se déclarent prêts avant d'être connectés, un décodeur XML au coût
quadratique.

**Optimisation Rust.** La base est saine et sobre : identifiants `Arc<str>`, bus `broadcast`
à valeurs partagées, files bornées, SQLite hors des workers, `unsafe` interdit dans les
57 crates, LTO complète. Les gains restants sont petits et localisés : allocations
évitables sur le chemin le plus chaud du hub, balayages linéaires et horloge à 1 Hz dans le
moteur d'automatismes, images de caméra copiées trois ou quatre fois.

## 2. Ce qui protège déjà la maison

### 2.1 Le superviseur des pilotes

- Chaque pilote tourne dans sa tâche ; `catch_unwind` enveloppe `run`
  (`crates/moli-runtime/src/supervisor.rs:38`). Un retour `Ok` ou `Err` est traité comme une
  panne.
- Attente de 1 s doublée jusqu'à 60 s, remise à 1 s après 60 s de bon fonctionnement
  (`supervisor.rs:16-19`, `45-47`).
- À chaque panne : file d'ordres vidée (`drain`, l. 50), appareils hors ligne (l. 51),
  statut `Backoff { error, retry_in_ms }` publié sur le bus (l. 53-59).
- Le même `DriverCtx` survit aux relances (l. 28-29) : la file n'est jamais recréée, aucun
  ordre ne tombe dans un trou pendant le redémarrage.
- `panic = "unwind"` est gardé volontairement en release (`Cargo.toml:129`) : sans lui,
  `catch_unwind` ne servirait à rien.

### 2.2 Le contrat d'un ordre

- File par pilote bornée à 64 ordres (`hub.rs:59`), remplie par `try_send` : une file pleine
  répond `Busy` tout de suite, l'appelant n'attend jamais (`hub.rs:1807-1825`).
- Échéance de 5 s portée par l'ordre (`hub.rs:115`, `1806`). Un ordre périmé n'est jamais
  remis au pilote (`driver.rs:574-586`) ; le pilote peut revérifier juste avant l'émission
  (`CommandRequest::abandoned`, `SendPermit::begin_send`, `driver.rs:136-190`).
- Prise ou abandon tranchés par un seul `compare_exchange` sur un `AtomicU8`
  (`driver.rs:96-108`) : un ordre est pris ou abandonné, jamais les deux.
- Exactement une ligne de journal par ordre, même si l'appelant disparaît (`Observer`,
  `Delivery` et leurs `Drop`, `hub.rs:354-424`).

### 2.3 Le bus et le modèle

- Bus `tokio::sync::broadcast` de 1 024 événements (`hub.rs:114`, `583`). Un abonné lent
  reçoit `Lagged(n)` au lieu de freiner les autres ; le SSE et l'historique se
  resynchronisent par un instantané (`moli-api/src/sse.rs:128-131`,
  `moli-history/src/lib.rs:343-352`).
- Un état n'est émis que s'il change (`hub.rs:2416`).
- Identités et valeurs partagées à coût constant : `DeviceId` et `PointId` sont des
  `Arc<str>` (`moli-core/src/id.rs:11-20`), `Value::Text` porte un `Arc<str>`, les appareils
  et les lignes de journal sont des `Arc`. Diffuser un événement à N abonnés coûte
  N incréments de compteur, pas N copies.
- Verrous tolérants à l'empoisonnement (`unwrap_or_else(PoisonError::into_inner)`,
  `hub.rs:2794-2800`) : une panique sous verrou ne condamne pas le hub.
- Aucun verrou `std` n'est tenu à travers un `.await` : pour un futur `Send` (tous ceux des
  pilotes et de `tokio::spawn`), le compilateur le refuse. Cette garantie vient du système
  de types, pas d'une relecture.
- L'ordre de prise des verrous est documenté (`computed/mod.rs:1-2`, mise en garde de
  [coeur.md](../components/coeur.md)). Une recherche automatique sur les 39 méthodes qui
  prennent le verrou du modèle n'a trouvé aucune reprise de ce verrou déjà tenu.

### 2.4 Entrées et sorties

- SQLite toujours hors des workers (`spawn_blocking`, `moli-history/src/lib.rs:258-264`,
  `moli-energy/src/lib.rs:447-453`). Trois connexions séparées pour l'historique : écrivain,
  lecteur, agents (`store.rs:190-192`). WAL et `synchronous = NORMAL` (`store.rs:278-279`),
  écritures par lots de 2 s ou 1 000 lignes (`lib.rs:39-41`), compactage borné quand
  l'écrivain sature (`lib.rs:389-414`). Le SQL des agents est en lecture seule, limité en
  taille et coupé à 2 s par un `progress_handler` (`store.rs:294-306`, `915-920`). La purge
  tourne hors des workers, par lots de 5 000 lignes avec une pause de 2 ms
  (`retention.rs:36-38`).
- Journal d'audit écrit par un thread dédié (`journal_log.rs:55-59`) : enregistrer une ligne
  ne touche jamais au disque depuis un worker.
- États métier écrits par `tokio::fs`, avec fsync du fichier, renommage et fsync du dossier
  (`state_file.rs`).
- Réseau commun (`moli-net`) : délai de 4 s par étape (`lib.rs:45`), corps bornés à 4 Mio et
  16 Mio (`lib.rs:53-56`, `76-86`), keepalive TCP 60 s puis 15 s contre les appareils
  débranchés sans bruit (`lib.rs:400-404`), TLS épinglé, connexion réutilisée pour les
  images de caméra, `AbortOnDrop` sur la tâche de connexion des requêtes uniques
  (`once.rs:111-117`).

### 2.5 Surfaces

- Corps limités à 256 Kio par défaut, 4 Mio pour la voix et les imports
  (`moli-api/src/lib.rs:62-64`, `383`). Quotas d'écriture par appelant et par classe
  (`limit.rs`). Flux SSE comptés : 8 par appelant, 64 au total (`sse.rs:30-32`).
- Une panique dans un handler HTTP ne coûte que la connexion concernée (tâche hyper).
- Les appels au LLM sont bornés (45 s pour une conversation, 10 à 30 s pour le reste :
  `moli-assistant/src/llm.rs:12-19`) et ne sont jamais sur le chemin d'un ordre.

### 2.6 Sûreté et sobriété

- `unsafe_code = "forbid"` hérité par les 57 crates (`Cargo.toml:109`) : un pilote ne peut
  pas corrompre la mémoire d'un autre (les dépendances mises à part).
- Briques dormantes tant qu'elles ne sont pas configurées (D1), pilotes à l'écoute plutôt
  qu'en sondage quand l'appareil le permet, release `opt-level = "s"`, `lto = "fat"`,
  `codegen-units = 1`, binaire musl statique sur `scratch`.
- Le service d'archive (D13) tourne déjà dans un processus séparé : un précédent utile.

### 2.7 Les bons modèles de pilote, déjà dans le dépôt

| Modèle | Où | Ce qu'il fait bien |
|---|---|---|
| Acteur + `JoinSet` | `moli-ariston/src/lib.rs:122-141`, `299-303`, Voltalis, Overkiz | entrées-sorties hors de la boucle ; une panique de tâche remonte par `join_next` ; sortie propre par `abort_all` |
| File d'un ordre en vol | `moli-samsung/src/queue.rs:1-116` | file de 4, sondage sous `timeout(12 s)`, ordres bornés par `timeout_at(deadline)`, garde `OfflineOnDrop` |
| Tâche de fond surveillée | `moli-androidtv-driver/src/driver.rs:178`, `moli-androidtv/src/tracked.rs:13-32` | `background.join_next()` relance le pilote si une tâche meurt ; lecteur `abort-on-drop` |
| Client déplacé dans la tâche | `moli-twinkly/src/runtime.rs:131-302` | aucun verrou pendant l'E/S, paniques propagées, file de 8 |
| Décodeur borné | `moli-cast/src/dns.rs:19,58-71`, `moli-tuya/src/protocol.rs:299-368`, `moli-asuswrt/src/json.rs:7-26` | longueur, profondeur et nombre d'éléments plafonnés avant toute indexation |

## 3. Les façons dont un module atteint encore la maison

### 3.1 Gel par appel bloquant ou calcul long — Élevé

Le runtime n'a que deux workers par défaut (`moli-os/src/config.rs:179-181`,
`moli.example.toml:8`). Tokio ne préempte pas : une tâche qui ne rend pas la main garde son
worker. Un `catch_unwind` n'y peut rien, il n'y a pas de panique ; un `timeout` non plus, il
ne s'applique qu'aux points d'attente.

Mesure E2 (machine de test, 2 workers, un pilote sain recevant un ordre toutes les 20 ms) :

| Situation | Ordres servis en 2 s | Médiane | Pire | Retard max d'une minuterie de 10 ms |
|---|---|---|---|---|
| Référence | 89 à 91 | 0,05 ms | 0,1 à 6 ms | 4 à 8 ms |
| 1 pilote bloquant (200 ms sans rendre la main) | 90 à 91 | 0,04 ms | 0,1 à 4 ms | 4 à 8 ms |
| 2 pilotes bloquants | **10** | **27 à 46 ms** | **34 à 58 ms** | **200 à 374 ms** |

Un appel bloquant est absorbé par le vol de tâches de Tokio ; deux suffisent à geler toute la
maison : API, SSE, moteur d'automatismes, horloges et tous les autres pilotes partagent ces
deux workers.

Le cœur est presque propre : SQLite, journal et états métier sont hors des workers ; seule
exception, le coffre relit son petit fichier en `std::fs` avant d'écrire
(`moli-runtime/src/secrets.rs:163`). Les sources trouvées sont dans les pilotes et quelques
calculs faits sur place :

| Source | Où | Coût |
|---|---|---|
| Décodage XML au coût quadratique en nombre d'attributs (`roxmltree` 0.21.1 cherche les doublons par balayage), fait en ligne dans l'async, plafonné à 1 Mio mais pas en attributs | `moli-netgear/src/xml.rs:16-34`, `moli-huawei-lte/src/xml.rs:14-26` | **mesuré (E8) : 0,16 s pour 64 Kio, 2,75 s pour 256 Kio, 96,6 s pour 1 Mio**, à chaque cycle de 60 s si la réponse hostile revient ; le délai de 30 s du cycle ne peut pas l'interrompre |
| Rééchantillonnage FIR (63 coefficients) d'une phrase entière avant de la dire par la sonnette | `moli-reolink/src/talk.rs:37-40`, `moli-audio/src/pcm.rs:35-48` | estimé 30 à 100 ms pour 10 s de voix sur la machine de test, davantage sur le Pi |
| Encodage FLAC d'une réponse vocale entière | `moli-assistant/src/satellite.rs:283-292` | quelques ms à dizaines de ms selon la durée |
| Lectures `/proc` et `/sys` en `std::fs` dans la boucle | `moli-host/src/lib.rs:179-213`, `638-704` | microsecondes en général, dizaines de ms pour certains capteurs (`drivetemp`, `nvme` ; estimé) |
| `std::fs::write` sous `Mutex` std depuis des handlers async | `moli-phones/src/lib.rs:195-199` | sensible sur carte SD |
| Gros JSON (2 à 4 Mio) parsés en ligne | Z2M `lib.rs:148`, Overkiz `/setup/devices`, carte météo | 10 à 40 ms, rarement |
| Agrégation d'un rapport d'énergie sur toute la plage | `moli-energy/src/lib.rs:323-339`, `report.rs:119-161` | proportionnel à la plage demandée |

Options étudiées :

| Option | Effet | Coût |
|---|---|---|
| A. Lint `clippy::disallowed_methods` (`std::fs::*`, `std::thread::sleep`, `std::net::*`) dans les crates async, exceptions annotées | Empêche à la compilation | Faible |
| B. Détecteur de gel : une tâche mesure le retard d'une minuterie de 100 ms, journalise au-delà de 50 ms et publie le pire retard dans `/api/health` | Détecte en production, y compris un interblocage | Faible |
| C. Décodages et calculs lourds en `spawn_blocking`, avec un pré-contrôle bon marché (nombre d'attributs, taille) | Supprime les sources connues | Faible |
| D. Passer à 3 ou 4 workers sur un Pi Zero 2 W (4 cœurs) | Plus de marge ; un worker inactif est parqué et ne coûte rien au repos | Faible, à mesurer (RSS) |
| E. `tokio-metrics` par tâche supervisée (D8, décidée mais absente de `Cargo.lock`) | Attribue le temps de worker à chaque pilote | Moyen |

Choix : A, B et C tout de suite (C en premier pour le XML), D après mesure sur le matériel
cible, E avec la page Système.

### 3.2 Pilote figé, ou qui se dit prêt trop tôt — Élevé

Le superviseur ne réagit qu'au retour ou à la panique de `run` (`supervisor.rs:36-44`). Un
pilote qui attend indéfiniment reste `running` : chaque ordre échoue au bout de 5 s, la file
finit pleine (`Busy`), et rien ne le relance (E3 : ordre en `Timeout` après 5,0 s, statut
toujours `running` 10 s plus tard). Aucun indicateur de dernière activité n'est exposé.

La grille a trouvé des variantes de ce silence :

- **Prêt avant d'être connecté** : ESPHome (`moli-esphome/src/lib.rs:227`) et Frigate
  (`moli-frigate/src/lib.rs:365`) appellent `ctx.ready()` avant toute connexion et gèrent
  leurs reconnexions en interne ; la panne du broker de Frigate n'est journalisée qu'en
  `debug` (`lib.rs:438`). Le superviseur ne voit jamais l'échec.
- **Parqué pour de bon** : une seule réponse `Invalid` (une page HTML servie pendant un
  redémarrage du modem, par exemple) met ASUSWRT et Huawei LTE en attente d'un humain
  jusqu'au prochain redémarrage (`moli-asuswrt/src/runtime.rs:166-173`,
  `moli-huawei-lte/src/runtime.rs:176-192`) ; Netgear, lui, repart avec attente croissante.
- **Connexion à moitié ouverte** : le keepalive TCP n'est posé que par
  `PinnedHttps::connect` (`moli-net/src/lib.rs:400`), pas par `PinnedTls::connect`
  (l. 286-303) ni `WebSocket::open` (`ws.rs:67-83`). Le WebSocket cloud de Tuya envoie un
  ping mais n'attend pas le pong (`moli-tuya/src/cloud.rs:423-426`) : une connexion morte
  n'est vue qu'à la fin des retransmissions TCP (de l'ordre du quart d'heure), et les
  capteurs endormis (fuite, porte) restent muets jusqu'à la resynchronisation de 30 min.
- **Adresse figée** : Roborock résout l'hôte une seule fois et ne retourne jamais
  (`moli-roborock/src/lib.rs:103`) ; une adresse DHCP qui change le laisse injoignable sans
  relance.

Options étudiées :

| Option | Effet | Coût |
|---|---|---|
| A. Vivacité gérée par le superviseur : `DriverCtx` note la dernière progression (événement publié, ordre pris, `ready`) dans un `AtomicU64` ; chaque pilote déclare un silence maximal ; le `select!` du superviseur gagne une branche qui abandonne `run` et relance | Attrape aussi les cas inconnus | Moyen |
| B. Règles de revue : `ready()` seulement après la première réponse ; toute attente réseau sous délai ou keepalive (keepalive ajouté à `PinnedTls` et `WebSocket`) ; ping/pong vérifié | Prévient | Faible |
| C. N ordres consécutifs en `Timeout` déclenchent une relance | Signal gratuit, déjà disponible | Faible |

Choix : A (générique) et C (immédiat), B comme règles de revue et petites corrections.
Abandonner le futur de `run` ne marche qu'aux points d'attente : un pilote bloquant (3.1)
reste hors d'atteinte.

### 3.3 Briques hors superviseur — Élevé

Le dépôt contient **un seul** `catch_unwind` : celui des pilotes (`supervisor.rs:38`). Les
briques sont lancées par un simple `tokio::spawn` et leur handle n'est attendu qu'à l'arrêt
(`moli-os/src/main.rs:490-500`) :

| Brique | Lancement | Si elle panique |
|---|---|---|
| Moteur d'automatismes | `moli-automation/src/lib.rs:315` | plus aucun automatisme ne se déclenche |
| Historique (enregistreur, écrivain, purge) | `moli-history/src/lib.rs:146-172` | l'historique s'arrête |
| Énergie (enregistreur) | `moli-energy/src/lib.rs:166` | kWh et € ne sont plus comptés |
| Ordonnanceur des minuteurs | `main.rs:1542` | les minuteurs n'expirent plus |
| Entretien (expiration des demandes, cache d'état) | `main.rs:925` | les demandes n'expirent plus, `state.json` n'est plus écrit |
| Horloges des calculs et des dépendances | `computed_clock.rs:16`, `dependency_hub.rs:444` | valeurs calculées figées ; l'indicateur « démarrée » reste vrai, rien ne les relance |

E4 le montre : une brique abonnée au bus meurt au troisième événement, la maison continue,
personne n'est prévenu. Les causes possibles existent : invariants en `expect` ou `unwrap`
(`dependencies/store.rs`, douze `unwrap` après chargement ; `computed/engine.rs:187` ;
`scene_hub.rs:316`). Aucun n'est un bogue aujourd'hui, chacun est un arrêt silencieux
potentiel demain. Les sous-tâches lâchées par les pilotes ont le même défaut à plus petite
échelle (section 5.4).

Options étudiées :

| Option | Effet | Coût |
|---|---|---|
| A. `supervise(nom, fabrique)` générique, même boucle que `spawn_driver` (capture, attente croissante, statut publié dans l'instantané et `/api/health`) | Auto-guérison | Moyen : chaque brique doit pouvoir repartir (réabonnement, instantané ; l'historique et le moteur savent déjà le faire après un `Lagged`) |
| B. Sentinelle : un `JoinSet` des briques ; à toute sortie, journal, notice et santé « dégradée » | Plus de panne silencieuse | Faible |
| C. Arrêt volontaire du processus si une brique meurt (Docker relance) | État propre garanti | Faible, mais toute la maison redémarre |

Choix : B d'abord, puis A pour le moteur, l'historique, l'énergie et les minuteurs. C reste
une solution honnête pour une brique qui ne sait pas repartir.

### 3.4 Fautes de processus — Moyen

Tout tourne dans un seul processus. Un débordement de pile, un échec d'allocation, un
dépassement du plafond mémoire du conteneur (`mem_limit: 128m`, `compose.yaml:18`), une
double panique ou un `abort` arrêtent toute la maison : E7 provoque un débordement de pile
dans un pilote, le processus sort en code 134 (`SIGABRT`), le superviseur n'a rien vu.
Docker relance (`restart: unless-stopped`, `compose.yaml:11`) : la maison est indisponible
le temps du démarrage (budget 2 s) et des reconnexions ; les dernières valeurs reprises
datent d'au plus une minute (`main.rs:705-707`). Si la faute vient d'une entrée qui revient
à chaque connexion, la relance tourne en boucle.

Exposition constatée : faible. Aucune récursion non bornée sur des données réseau
(`serde_json` s'arrête à 128 niveaux, `roxmltree` est itératif et limite les entités à
10 niveaux, les parcours internes sont bornés, par exemple `moli-automation/src/check.rs:980`
ou `moli-asuswrt/src/json.rs:7-26`), corps de réponse bornés, `unsafe` interdit. Un seul
point d'allocation non plafonné a été relevé : `moli-audio::pcm::resample` ne borne pas la
taille de sortie (`pcm.rs:52-63`) ; les appelants actuels passent des fréquences sûres.

Options étudiées :

| Option | Effet | Coût |
|---|---|---|
| A. Garder un seul processus, maintenir les règles (pas de récursion sur une entrée réseau, tailles bornées) et fuzzer les décodeurs exposés (trames ESPHome, Tuya, SOAP, XML) | Prévient | Faible à moyen |
| B. Disjoncteur de redémarrage : après 3 démarrages anormaux en 10 min, démarrer sans les pilotes qui démarraient au moment des pannes, et le dire | Évite la boucle | Moyen |
| C. Processus séparés pour les pilotes à risque, reliés au hub par une socket locale, comme l'archive | Vrai confinement mémoire et pile | Élevé (IPC, empaquetage, mémoire) |

Choix : A. C ne se justifie que si un pilote non relu doit tourner, ou si un décodeur ne peut
pas être durci ; B seulement si une boucle est observée.

### 3.5 Démarrage couplé — Moyen

`spawn_drivers` propage toute erreur de construction (`main.rs:1111-1162`, `?` aux lignes
1123 à 1153), et `serve` s'arrête (`main.rs:436`). Or certaines constructions lisent des
données d'exécution, pas seulement la configuration :

- `moli_phones::open` lit `phones.json` ; illisible, c'est une erreur
  (`moli-phones/src/lib.rs:179-185`). Ce fichier est écrit sans fsync (`lib.rs:195-199`) :
  une coupure de courant au mauvais moment peut le laisser vide.
- `ProfileDriver::new` lit le fichier de profil (`moli-profile/src/lib.rs:314-315`).
- L'ouverture de l'historique, de l'énergie et des automatismes est aussi fatale
  (`main.rs:420-434`, `462`).

Scénario : une coupure pendant l'écriture de `phones.json`, et la maison entière refuse de
démarrer ; le tableau de bord, éteint, ne peut même pas le dire.

Refuser une **configuration** invalide au démarrage reste juste (`check-config` fait la même
chose). Le problème, ce sont les **données**. Recommandation : à `serve`, un pilote dont la
construction échoue est inscrit avec un statut d'attente qui dit pourquoi, les autres
démarrent ; `phones.json` passe par l'écriture atomique synchronisée déjà utilisée ailleurs.
Le cas des bases (historique, énergie) mérite une décision : démarrer sans elles en le
signalant, ou attendre un humain.

## 4. Réactivité

### 4.1 Les chemins chauds

```text
Pilote ── ctx.set_state ──► Hub : verrou d'écriture du modèle (µs)
                                  │ valeur changée ?
                                  ▼
                         broadcast (1 024) ──► SSE (instantané si retard)
                                  │        ──► historique (lots de 2 s, SQLite hors workers)
                                  │        ──► énergie
                                  │        ──► moteur d'automatismes (une seule file)
                                  │        ──► pilote des luminaires (une seule file)
Surface ── hub.command ──► validation ─► garde ─► try_send (64) ─► tâche du pilote ─► réseau
                ▲                                                        │
                └────────── réponse (oneshot, échéance 5 s) ◄────────────┘
```

### 4.2 Mesures

Machine de test : Apple M4, runtime à 2 workers comme en production. Les valeurs absolues ne
se transposent pas au Pi Zero 2 W (D10), les effets relatifs si.

| Expérience | Résultat |
|---|---|
| E1, pilote qui panique en boucle | 3 paniques capturées en 3,5 s, statut `backoff (4000 ms)` ; pilote sain : 156 ordres sur 156, médiane 0,06 ms, pire 1,5 ms |
| E5, débit brut du cœur | 3 000 ordres aller-retour (surface → hub → pilote → réponse) en 23 à 31 ms, soit ~100 000 par seconde, pire 0,08 ms |
| E5, abonné lent | l'abonné rapide reçoit les 6 000 événements, l'abonné lent en perd 4 975 (`Lagged`) sans ralentir personne |
| E6, verrou global | `hub.snapshot()` sur 720 valeurs : 0,02 à 0,05 ms ; avec 16 tâches qui copient tout en boucle, médiane des ordres 0,09 à 0,12 ms |

Le verrou global (`std::sync::RwLock<Model>`) n'est pas un goulot : le découper (sharding,
`arc-swap`) n'apporterait rien de mesurable à cette échelle.

### 4.3 Le moteur d'automatismes : une seule file — Moyen

- Une seule boucle traite tous les événements de la maison (`engine.rs:72-98`). Pour un appui
  (occurrence), elle attend l'admission durable, avec fsync (`occurrences.rs:164`), et les
  verrous des minuteurs ; pendant ce temps, aucun autre déclencheur n'est examiné.
- Sur `Lagged`, les valeurs sont relues mais les transitions manquées sont perdues
  (`engine.rs:77-85`) : un « quand la porte s'ouvre » survenu pendant le retard ne partira
  jamais. Le journal le signale, c'est honnête, mais c'est une perte.
- Chaque changement d'état parcourt tous les automatismes actifs et tous leurs déclencheurs,
  et alloue une `String` (`engine.rs:351-357`) ; la carte `last` (`HashMap<String, Value>`)
  recopie l'état que le hub connaît déjà.
- L'horloge bat à 1 Hz, reparcourt tout, formate une date à chaque seconde et recalcule le
  soleil pour chaque déclencheur solaire (`engine.rs:499-536`), alors que tous les
  déclencheurs horaires sont à la minute.
- Chaque exécution en attente (`WaitFor`) réévalue sa règle à chaque événement de la maison
  (`run.rs:427-445`).

Recommandations : indexer les déclencheurs d'état par point, comme `occurrence_index` le
fait déjà pour les appuis ; garder `last` sous clé `PointId` ou, mieux, porter la valeur
précédente dans `Event::State` (le hub la connaît sous son verrou d'écriture) ; une horloge
qui dort jusqu'à la prochaine minute due ; sortir l'admission durable de la boucle de
distribution ; sur `Lagged`, comparer l'instantané relu aux valeurs connues pour rejouer les
transitions franches.

### 4.4 Réveils au repos

| Source | Fréquence |
|---|---|
| Horloge du moteur d'automatismes | 1 par seconde |
| Calendrier des réservations | 1 par seconde (`moli-reservations/src/timeline.rs:41-49` reconstruit une table de 4 096 entrées au plus) |
| Lots de l'historique | 1 toutes les 2 s |
| Sonos | 4 connexions TCP neuves toutes les 2 s par enceinte (`moli-sonos/src/lib.rs:19`, `243-262`), soit ~172 000 par jour et par enceinte |
| Reolink | 1 requête par seconde (`moli-reolink/src/lib.rs:33`) |
| Tapo | 1 connexion TCP toutes les 5 s par appareil, même injoignable |
| Entretien, pilote des personnes, keepalive SSE | 1 toutes les 15 s |
| Twinkly | 4 à 7 requêtes toutes les 15 s, dont la version du micrologiciel, qui ne change pas (`moli-twinkly/src/api.rs:232`) |
| Alexa | une connexion TLS neuve par enceinte toutes les 30 s |
| Routeurs (ASUSWRT, Netgear, Huawei) | un cycle par minute, TCP et TLS rouverts à chaque requête (`moli-net/src/lib.rs:562-581`), chaque réponse décodée deux fois |

Le budget est tenu (0,24 % de CPU mesuré, [PERFORMANCE.md](../PERFORMANCE.md)). Sur le
matériel cible, l'horloge à 1 Hz et les connexions rouvertes à chaque sondage (Sonos en tête)
sont les plus grosses dépenses constantes, et les plus faciles à supprimer.

## 5. Pilotes et services tiers

Lecture des 45 crates de pilotes et de briques selon la même grille : structure de la boucle,
durée de vie des sous-tâches, attentes sans délai, appels bloquants, croissance non bornée,
surface de panique, coût au repos, verrous. Tableau complet par crate en
[annexe B](#annexe-b--grille-par-pilote).

### 5.1 Ce que la grille ne trouve pas

- Aucun `std::thread::sleep`, aucun `std::net` synchrone, aucun `block_on` dans le code de
  production des pilotes.
- Aucun canal non borné ; tous les tampons réseau sont plafonnés (MQTT 4 Mio, SSE et
  WebSocket 1 Mio, Cast 256 Kio, ESPHome 32 Kio, Tuya 4 Kio…).
- Des décodeurs qui vérifient longueur, somme de contrôle ou signature avant d'indexer
  (Tuya, miIO de Roborock, protobuf d'Android TV, DNS de Cast, RTSP de Reolink).
- Des `unwrap` et `expect` de production limités à des invariants (clé `ring`, aléa système).

### 5.2 Le défaut dominant : la boucle qui fait elle-même ses entrées-sorties

Beaucoup de pilotes attendent la réponse réseau **dans** le `select!` qui lit leurs
commandes. Pendant cette attente, `next_command()` n'est plus lu : les ordres suivants
vieillissent dans la file et expirent à 5 s, et les événements de l'appareil s'accumulent.

| Pilote | Attente en ligne au pire | Où |
|---|---|---|
| **Luminaires** (`lumieres`) | ≤ 5 s par réglage et par ampoule (réglages en série, ampoules en parallèle), soit jusqu'à ~15 s ; **une seule instance sert tous les luminaires de la maison** et cesse de lire le bus pendant ce temps (`Lagged`, puis relecture de tous les luminaires) | `moli-lights/src/lib.rs:829-838`, `479-507` |
| Telegram | 10 conversations × 15 s = 150 s | `moli-telegram/src/lib.rs:182-217` |
| Tapo | 2 tentatives × 3 requêtes × 4 s = 24 s | `moli-tapo/src/lib.rs:235-249` |
| Reolink | jusqu'à ~24 s (relecture, événements, reconnexion) | `moli-reolink/src/lib.rs:505-512` |
| Sonos | 4 requêtes SOAP de ≤ 4 s par sondage, soit ≤ 16 s | `moli-sonos/src/lib.rs:381`, `393` |
| Philips | 4 requêtes HTTPS, ~8 s quand le téléviseur dort (justement quand on l'allume) | `moli-philips/src/lib.rs:734`, `740` |
| Alexa | ≤ 20 s par appel, sondage séquentiel par enceinte | `moli-alexa/src/lib.rs:298-305`, `351-381` |
| Hue | rechargement complet ≤ 2 × 4 s | `moli-hue/src/lib.rs:182`, `193` |
| Moonraker, Nuki, IGD, Host, téléphones, profils | de quelques secondes à plusieurs dizaines (Host : sondes de 1,5 s par machine, en série) | voir annexe B |

Le cas des luminaires est le plus sensible : une ampoule annoncée en ligne mais lente (un
relais Wi-Fi qui met 5 s à acquitter) retient les ordres de tous les autres luminaires, y
compris Zigbee. Ce n'est pas une panne qui se propage, c'est une latence qui se propage.

Le modèle à suivre existe déjà dans le dépôt (section 2.7) : acteur avec `JoinSet` (Ariston,
Voltalis, Overkiz), file d'un ordre en vol (Samsung), client déplacé dans la tâche (Twinkly).
Pour les luminaires : traiter chaque luminaire dans sa propre tâche (ou une tâche par ordre,
annulable), et laisser la boucle principale ne faire que distribuer.

### 5.3 Ordres émis après abandon — Moyen

L'architecture pose qu'« un ordre que personne n'attend plus n'est jamais exécuté » et le
cœur fournit les outils (`abandoned()`, `send_permit`, `web_if`, `begin_send`). Neuf pilotes
ne s'en servent pas, ou pas jusqu'au bout :

| Pilote | Ce qui part trop tard | Où |
|---|---|---|
| Hue | le PUT part jusqu'à 4,5 s après la prise de l'ordre, sans revérification | `moli-hue/src/lib.rs:354-358` |
| Tuya | échéance propre au pilote ; un appelant parti ou un permis révoqué sont ignorés | `moli-tuya/src/device.rs:49-54` |
| Philips | GET, POST puis touche, sans revérification | `moli-philips/src/lib.rs:596-611` |
| Alexa | envoi, réouverture de session puis renvoi (jusqu'à ~7 appels) | `moli-alexa/src/lib.rs:384-399` |
| Moonraker | pause ou annulation d'impression (une commande de **machine**) envoyée sans `abandoned()` | `moli-moonraker/src/lib.rs:247-261` |
| Tapo | la seconde tentative repart après un nouveau handshake | `moli-tapo/src/lib.rs:237-248` |
| Telegram | les conversations 2 à 10 partent après abandon | `moli-telegram/src/lib.rs:202-215` |
| Profils, téléphones | `timeout_at(deadline)` seulement ; la notification Expo part même après abandon | `moli-profile/src/lib.rs:831-851`, `moli-phones/src/lib.rs:683-686` |

Effet : l'appelant (la famille, un automatisme) lit « délai dépassé », et l'appareil agit
quand même quelques secondes plus tard. Le cœur attend la réponse jusqu'à l'échéance de 5 s,
même après le départ de l'appelant (`hub.rs:339-348`) ; un envoi qui part **après** cette
échéance est donc journalisé « délai dépassé » alors que l'appareil a obéi : l'écran et le
journal disent la même chose fausse, ce qui touche la règle 7 (« toute écriture est
journalisée… résultat »). Un test de contrat commun
(« un ordre abandonné avant l'émission ne part pas »), joué contre chaque pilote avec un faux
serveur, fermerait le sujet.

### 5.4 Sous-tâches lâchées

| Pilote | Sous-tâche | Risque |
|---|---|---|
| IPP | le travailleur d'impression est détaché (`moli-ipp/src/lib.rs:497-535`) | s'il panique, l'imprimante répond « file pleine » pour toujours, statut `running` ; après un redémarrage pendant une panne de papier, l'ancien travailleur boucle et garde jusqu'à 32 Mio de documents |
| Reolink | une tâche par appui sur la sonnette et par message vocal (`moli-reolink/src/lib.rs:643`, `682`) | N appuis = N visites à la suite, sans plafond ni déduplication ; des durées de configuration non bornées font paniquer `Instant + Duration` dans la tâche (`listen.rs:73`, `door.rs:212`) |
| Tuya | `JoinSet` jamais joint, trois tâches de réception UDP hors du `JoinSet` (`moli-tuya/src/lib.rs:124-204`, `379`) | une panique rend un appareil muet sans signal |
| Sonos | annonce détachée (`moli-sonos/src/announce.rs:109`) | une panique laisse le compteur d'attente incrémenté : file « pleine » après trois |
| ESPHome, Philips, luminaires, Hue | tâches détachées bornées dans le temps (15 s à 90 s) | chevauchement avec la nouvelle instance après un redémarrage |

Règle proposée : toute sous-tâche vit dans un `JoinSet` que la boucle du pilote joint dans son
`select!` ; une sortie inattendue fait retourner `run`, donc relancer le pilote (le modèle
d'Android TV).

### 5.5 Services tiers : relances et quotas

| Pilote | Constat | Risque |
|---|---|---|
| Landroid | un 401 ou 403 efface la session et relance la connexion toutes les 10 s, sans compteur (`moli-landroid/src/cloud.rs:224-227`, `lib.rs:45`, `613`) | ~12 appels par minute sans fin vers un service dont le code note lui-même qu'il bannit ; Ariston a déjà le bon garde-fou (`MAX_SESSION_REFUSALS`) |
| Overkiz | toute erreur de lecture non classée redémarre le pilote (`moli-overkiz/src/lib.rs:460`) et l'abonnement aux événements n'est jamais désenregistré | des abonnements orphelins s'accumulent côté Somfy |
| Météo | le verrou est tenu pendant la requête (15 s) et un échec ne rafraîchit pas l'horodatage (`moli-weather/src/lib.rs:260-284`) | panne d'Open-Meteo : chaque requête refait l'appel, la k-ième attend k × 15 s |
| Telegram, météo | aucun recul sur 429 | quota dépassé plus longtemps |
| Hue | `GET /clip/v2/resource` limité à 4 Mio et 4 s pour tout l'échange ; une réponse trop grosse n'est jamais rejouée (`moli-hue/src/lib.rs:265`, `273`) | très grosse installation (supposé) : redémarrage en boucle sans rien publier |
| Netgear | 4 096 octets de texte au plus par élément (`moli-netgear/src/xml.rs:44-51`), or l'inventaire V1 arrive en un seul nœud | selon la grille, au-delà de 50 à 70 appareils le cycle échoue et le pilote redémarre en boucle |
| Frigate | tout sujet MQTT `…/<nom>/motion` crée une caméra, sans plafond ni validation, sur un broker sans authentification (`moli-frigate/src/lib.rs:415-432`) | un client du réseau peut créer des appareils à volonté |

Les reculs sur 429 et 5xx sont bons ailleurs : Ariston 15 min, Voltalis 10 min, Netatmo
`Retry-After` borné, Landroid 1 h sur 429.

### 5.6 Images de caméra : trois ou quatre copies

Le pilote ne décode ni ne réencode : le JPEG passe tel quel, ce qui est juste. Mais
`media::Image` porte un `Vec<u8>` (`moli-runtime/src/media.rs:15`), d'où une copie à la
sortie de hyper (`to_vec()`, `moli-reolink/src/lib.rs:180`, `moli-frigate/src/lib.rs:77`), une
pour le cache partagé (`hub.rs:2287`) et une par lecteur (`hub.rs:2282`). Avec `bytes::Bytes`,
déjà dans l'arbre de dépendances (hyper, `moli-net`), ces trois copies deviennent des
incréments de compteur.

## 6. Optimisation Rust

### 6.1 Ce qui est déjà le bon choix

| Choix | Où | Pourquoi c'est juste |
|---|---|---|
| Identités et textes en `Arc<str>` | `moli-core/src/id.rs`, `value.rs` | clones à coût constant, diffusion sans copie |
| `broadcast` + `mpsc` bornés + `oneshot` | `hub.rs` | contre-pression explicite, aucun producteur bloqué |
| `try_send` plutôt que `send().await` vers les pilotes | `hub.rs:1807` | l'appelant ne dépend jamais de la santé du pilote |
| CAS sur `AtomicU8` pour prendre ou abandonner un ordre | `driver.rs:96-108` | exclusion sans verrou |
| Verrous `std` (pas `tokio::sync`) pour les sections courtes | `hub.rs` | plus rapides ; le compilateur interdit de les garder à travers un `.await` |
| SQLite dans `spawn_blocking`, trois connexions, lots | `moli-history` | les workers ne touchent jamais le disque |
| Thread dédié au journal | `journal_log.rs` | une écriture d'audit ne coûte qu'un envoi sur canal |
| Corps et files bornés partout | `moli-net`, `moli-api`, pilotes | mémoire prévisible sous 128 Mio |
| `unsafe` interdit, clippy pédant en erreur | `Cargo.toml` | sûreté mémoire de toutes les intégrations |
| LTO complète, une unité de code, taille optimisée, musl statique | `Cargo.toml:123-128` | image et RSS minimales |

### 6.2 Leviers restants

| Levier | Constat | Gain attendu | Coût | Décision proposée |
|---|---|---|---|---|
| Pré-contrôle et `spawn_blocking` du XML | E8 : 96,6 s pour 1 Mio d'attributs | supprime un gel de worker sur entrée hostile | faible (compter les attributs avant de parser) | faire |
| Allocations du chemin le plus chaud | `DriverCtx::owns` appelle `DeviceId::instance()`, qui alloue, à chaque `set_state` (`driver.rs:601-603`, `id.rs:74`) ; `PointId::split` alloue un `DeviceId` (`id.rs:87`) ; `record_state_update` construit trois fois le même `PointId` (`hub.rs:2418`, `2424`, `2431`) | quelques dizaines de ns par mise à jour, sur le chemin le plus fréquent | faible : comparer un préfixe, découper en `(&str, &str)`, construire le `PointId` une fois | faire |
| Images en `Bytes` | 5.6 | trois copies de moins par image (jusqu'à 16 Mio) | faible | faire |
| Connexions réutilisées dans les sondages | Sonos, routeurs, Alexa, Tapo (4.4) | moins de handshakes TCP et TLS, moins de tâches hyper | faible à moyen | faire pour Sonos d'abord |
| Index des déclencheurs d'état | 4.3 | proportionnel au nombre d'automatismes | moyen | faire |
| Horloge alignée sur la minute | 4.3, 4.4 | réveils ÷ 60 | faible | faire |
| `/api/health` sans copie complète | `rest.rs:23` copie toute la maison pour ne rendre que les pilotes | petit, budget santé 5 ms | faible (`hub.drivers()`) | faire |
| Journal sur canal borné | `std::sync::mpsc::channel()` non borné (`journal_log.rs:55`) : disque bloqué = mémoire qui monte | sûreté mémoire | faible (`sync_channel` + compteur existant) | faire |
| Sessions MCP plafonnées | `LocalSessionManager` sans plafond (`mcp.rs:43`), sessions gardées 5 min par défaut, 120 requêtes par 10 s autorisées par appelant | borne la mémoire face à un agent qui dérape | faible | faire |
| SSE : sérialiser une fois par événement | `sse.rs:124` sérialise pour chaque abonné | faible (64 flux au plus) | faible | si une mesure le demande |
| Découper le verrou global | E6 | non mesurable | élevé | ne pas faire |
| Allocateur `mimalloc` | malloc de musl lent sous contention | possible sur CPU, au prix de RSS | faible | mesurer d'abord |
| `opt-level = 3` ciblé | profil `s` partout | inconnu | faible | seulement avec un profil CPU |
| `tokio-metrics` (D8) | décidé, pas livré | observabilité par pilote | moyen | faire |

Un point de cohérence à garder en tête : l'événement d'un état est émis après la libération du
verrou d'écriture (`hub.rs:2390-2439`). Deux écrivains concurrents d'un même point pourraient
donc publier dans l'ordre inverse de leurs écritures. Aujourd'hui chaque point n'a qu'un
écrivain (son pilote), l'invariant tient ; il mérite d'être écrit.

## 7. Plan proposé

Méthode : chaque piste est notée en impact (risque de panne ou de latence évité, nombre de
pilotes touchés) et en effort (taille du changement, risque de régression), puis rangée dans
une matrice.

| | Effort faible | Effort moyen |
|---|---|---|
| **Impact fort** | P1 · sentinelle des briques (3.3 B) · détecteur de gel (3.1 B) · XML pré-contrôlé et hors workers (3.1 C) · lint anti-bloquant (3.1 A) · relance après N `Timeout` (3.2 C) · `ready()` après connexion (ESPHome, Frigate) · démarrage découplé des pilotes et `phones.json` synchronisé (3.5) · compteur de refus de session pour Landroid | P2 · supervision générique des briques (3.3 A) · vivacité des pilotes (3.2 A) · luminaires en tâches par luminaire (5.2) · moteur d'automatismes indexé, admission hors de la boucle (4.3) · test de contrat « ordre abandonné » pour tous les pilotes (5.3) |
| **Impact modéré** | P3 · images en `Bytes` · allocations du chemin chaud · `/api/health` léger · journal borné · plafond des sessions MCP · horloge à la minute · keepalive sur `PinnedTls` et `WebSocket` · recul sur 429 (Telegram, météo) | P3 · sous-tâches en `JoinSet` joint (IPP, Reolink, Tuya, Sonos) · autres pilotes à E/S en ligne (Telegram, Tapo, Reolink, Sonos d'abord) · connexions réutilisées dans les sondages · `tokio-metrics` (D8) · fuzz des décodeurs exposés · workers selon le matériel |

Hors plan pour l'instant : processus séparés par pilote, découpage du verrou global,
changement d'allocateur, `opt-level` différencié.

Chaque lot se vérifie par une preuve rejouable, sur le modèle de l'annexe A :

| Lot | Preuve d'acceptation |
|---|---|
| Sentinelle, puis supervision des briques | E4 rejoué : la panne est visible dans `/api/health`, puis la brique repart seule |
| Détecteur de gel | E2 rejoué : le pire retard de minuterie apparaît dans `/api/health` et au journal |
| XML | E8 rejoué contre `moli-netgear` et `moli-huawei-lte` : la réponse hostile est refusée en moins d'une milliseconde, sans occuper de worker |
| Vivacité des pilotes | E3 rejoué : le pilote figé est relancé après son silence maximal, statut `backoff` explicite |
| Démarrage découplé | `phones.json` vidé : la maison démarre, le pilote des téléphones dit pourquoi il attend |
| Luminaires | un membre qui met 5 s à répondre ne retarde pas l'ordre d'un autre luminaire (médiane inchangée) |
| Ordre abandonné | pour chaque pilote, faux serveur : un ordre abandonné avant l'émission n'atteint jamais le serveur |
| Moteur d'automatismes | un appui dont l'admission durable est ralentie ne retarde pas un « quand l'état change » ; budget `/api/automations` inchangé |

## 8. Limites de cette analyse

- Lecture du code et expériences synthétiques : aucun test de charge sur de vrais appareils,
  aucune mesure sur le Pi Zero 2 W. Les durées marquées « estimé » ne sont pas mesurées.
- La bascule vers Turso (D13, ticket C1) n'est pas encore dans `main` : la partie stockage
  porte sur `rusqlite`.
- La grille des pilotes est une lecture statique ; les constats élevés ont été revérifiés
  dans le code, les autres sont rapportés tels que la grille les a relevés.
- Hors champ : interface web, application mobile, internes du service d'archive (processus
  séparé, hors du domaine de panne du cœur), décodeurs tiers (`symphonia`).

## Annexe A — Preuves exécutables

Deux crates jetables, **hors du dépôt**. La première dépend de `moli-runtime` et `moli-core`
par chemin et lance chaque expérience dans un runtime à deux workers, comme `moli-os serve`
par défaut ; la seconde chronomètre `roxmltree` à la version exacte du dépôt. Rien n'est
modifié dans le projet.

```toml
# preuves/Cargo.toml (dans un dossier hors du dépôt)
[package]
name = "preuves"
version = "0.1.0"
edition = "2024"
publish = false

[workspace]

[dependencies]
moli-runtime = { path = "<dépôt>/crates/moli-runtime" }
moli-core = { path = "<dépôt>/crates/moli-core" }
anyhow = "1"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync", "time"] }
tokio-util = "0.7"
```

Reproduire : copier le `Cargo.lock` du dépôt dans le dossier, puis `cargo run --release`
(E7 se lance seule avec `cargo run --release -- pile`).

| Pilote de test | Comportement |
|---|---|
| `Sain` | publie une lampe fictive, obéit à chaque ordre |
| `Panique` | indexe un tableau vide aussitôt prêt (comme un octet réseau mal lu) |
| `Bloquant` | `std::thread::sleep(200 ms)` puis `yield_now`, en boucle |
| `Fige` | publie sa lampe, se dit prêt, puis `std::future::pending()` |
| `Maison` | publie 120 appareils de 6 valeurs |
| `Recursif` | décode récursivement 4 Mio de `[` (réponse hostile) |

Sortie brute d'une passe (les écarts entre passes sont donnés dans le rapport) :

```text
## E1 — un pilote qui panique en boucle
- paniques capturées en 3,5 s : 3
- statut du pilote fautif : backoff (4000 ms)
- ordres au pilote sain pendant ce temps : 156 réussis, 0 échoués, médiane 0.06 ms, max 0.90 ms
## E2 — un appel bloquant dans un pilote (2 workers)
- référence            ordres :  89 ok / 0 ko, médiane    0.06 ms, max    6.16 ms ; retard max d'une minuterie 10 ms :    7.90 ms
- 1 pilote bloquant    ordres :  90 ok / 0 ko, médiane    0.05 ms, max    1.07 ms ; retard max d'une minuterie 10 ms :    7.62 ms
- 2 pilotes bloquants  ordres :  10 ok / 0 ko, médiane   26.96 ms, max   36.49 ms ; retard max d'une minuterie 10 ms :  199.51 ms
## E3 — un pilote figé dans une attente sans délai
- ordre envoyé : Some(Timeout) après 5003.63 ms
- statut après 10 s : running (aucun redémarrage : le superviseur ne voit ni panique ni retour)
## E4 — une « brique » lancée par tokio::spawn qui panique
- ordres traités par la maison : 22 ; événements vus par la brique : 3 (morte au 3e)
- tâche de la brique terminée : true ; rien ne la relance, le hub reste « sain »
## E5 — un abonné lent sur le bus ne freine ni les pilotes ni les autres abonnés
- 3000 ordres en 26.45 ms (113434 ordres/s, pire 0.07 ms), événements reçus par l'abonné rapide : 6000, perdus par l'abonné lent (Lagged) : 4975
## E6 — verrou global du modèle : 720 valeurs, lecteurs qui copient tout en continu
- hub.snapshot() : 0.02 ms en moyenne
-  0 lecteurs en boucle : ordres 45 ok / 0 ko, médiane 0.04 ms, max 0.14 ms
-  4 lecteurs en boucle : ordres 48 ok / 0 ko, médiane 0.10 ms, max 0.35 ms
- 16 lecteurs en boucle : ordres 48 ok / 0 ko, médiane 0.12 ms, max 1.67 ms
## E7 — débordement de pile dans un pilote (processus séparé)
thread 'tokio-rt-worker' has overflowed its stack
fatal runtime error: stack overflow, aborting
(code de sortie 134)
## E8 — roxmltree 0.21.1, un seul élément, attributs distincts
   63 Kio,   7404 attributs :     0.16 s (parse ok = true)
  255 Kio,  27324 attributs :     2.75 s (parse ok = true)
 1023 Kio, 105425 attributs :    96.59 s (parse ok = true)
```

Code complet de la crate de preuves (`preuves/src/main.rs`) :

```rust
//! Preuves exécutables, hors dépôt, contre le vrai `moli-runtime` (aucune
//! modification du projet). Chaque expérience a son runtime à 2 workers,
//! comme la configuration par défaut de `moli-os serve`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use moli_core::{
    Access, Device, DeviceId, DriverStatus, Event, InstanceId, Kind, Origin, PointId, PointSpec,
    Semantic, Value,
};
use moli_runtime::{BoxFuture, Driver, DriverCtx, Hub, HubOptions, spawn_driver};
use tokio_util::sync::CancellationToken;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

fn lampe(id: &DeviceId, instance: &InstanceId) -> Device {
    Device {
        id: id.clone(),
        instance: instance.clone(),
        native_name: "lampe fictive".into(),
        manufacturer: None,
        model: None,
        description: None,
        native_room: None,
        members: vec![],
        points: vec![PointSpec {
            key: "on".into(),
            label: "on".into(),
            kind: Kind::Binary,
            access: Access {
                read: true,
                write: true,
            },
            unit: None,
            semantic: Semantic::OnOff,
        }],
    }
}

/// Un pilote sain : publie une lampe et obéit.
struct Sain;
impl Driver for Sain {
    fn kind(&self) -> &'static str {
        "sain"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let id = ctx.device_id("l1");
            ctx.upsert_device(lampe(&id, &ctx.instance().clone()));
            ctx.ready();
            while let Some(cmd) = ctx.next_command().await {
                ctx.set_state(&cmd.device.id, &cmd.key, cmd.value.clone());
                cmd.reply(Ok(()));
            }
            Ok(())
        })
    }
}

/// Panique aussitôt prêt.
struct Panique;
impl Driver for Panique {
    fn kind(&self) -> &'static str {
        "panique"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            ctx.ready();
            let v: Vec<u8> = Vec::new();
            let _ = v[3]; // index hors bornes, comme un octet réseau mal parsé
            Ok(())
        })
    }
}

/// Bloque son thread (E/S synchrone, calcul long) 200 ms, puis rend la main.
struct Bloquant;
impl Driver for Bloquant {
    fn kind(&self) -> &'static str {
        "bloquant"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            ctx.ready();
            loop {
                std::thread::sleep(Duration::from_millis(200));
                tokio::task::yield_now().await;
            }
        })
    }
}

/// Publie sa lampe, se dit prêt, puis attend pour toujours (lecture réseau
/// sans délai sur un appareil qui ne répond plus).
struct Fige;
impl Driver for Fige {
    fn kind(&self) -> &'static str {
        "fige"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let id = ctx.device_id("l1");
            ctx.upsert_device(lampe(&id, &ctx.instance().clone()));
            ctx.ready();
            std::future::pending::<()>().await;
            Ok(())
        })
    }
}

fn point(instance: &str) -> PointId {
    PointId::new(&DeviceId::new(&InstanceId::from(instance), "l1"), "on")
}

fn status(hub: &Hub, instance: &str) -> String {
    hub.snapshot()
        .drivers
        .iter()
        .find(|d| d.instance.as_str() == instance)
        .map(|d| match &d.status {
            DriverStatus::Backoff { retry_in_ms, .. } => format!("backoff ({retry_in_ms} ms)"),
            other => format!("{other:?}").to_lowercase(),
        })
        .unwrap_or_default()
}

async fn until_running(hub: &Hub, instance: &str) {
    while status(hub, instance) != "running" {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Latences d'ordres successifs vers le pilote sain pendant `pendant`.
async fn ordres(hub: &Hub, pendant: Duration) -> (usize, usize, Duration, Duration) {
    let p = point("sain");
    let fin = Instant::now() + pendant;
    let (mut ok, mut ko, mut toutes) = (0, 0, Vec::new());
    let mut v = false;
    while Instant::now() < fin {
        v = !v;
        let t = Instant::now();
        match hub.command(&p, Value::Bool(v), Origin::Ui, None).await {
            Ok(()) => ok += 1,
            Err(_) => ko += 1,
        }
        toutes.push(t.elapsed());
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    toutes.sort();
    let med = toutes.get(toutes.len() / 2).copied().unwrap_or_default();
    let max = toutes.last().copied().unwrap_or_default();
    (ok, ko, med, max)
}

/// Retard maximal d'une minuterie de 10 ms (latence d'ordonnancement).
fn sonde(stop: CancellationToken) -> (Arc<AtomicU64>, tokio::task::JoinHandle<()>) {
    let pire = Arc::new(AtomicU64::new(0));
    let p = pire.clone();
    let h = tokio::spawn(async move {
        while !stop.is_cancelled() {
            let t = Instant::now();
            tokio::time::sleep(Duration::from_millis(10)).await;
            let retard = t.elapsed().saturating_sub(Duration::from_millis(10));
            p.fetch_max(u64::try_from(retard.as_micros()).unwrap(), Ordering::Relaxed);
        }
    });
    (pire, h)
}

fn ms(d: Duration) -> String {
    format!("{:.2} ms", d.as_secs_f64() * 1000.0)
}

fn experience_panique() {
    println!("\n## E1 — un pilote qui panique en boucle");
    runtime().block_on(async {
        let hub = Hub::new(HubOptions::default()).unwrap();
        let cancel = CancellationToken::new();
        let mut bus = hub.subscribe();
        let paniques = Arc::new(AtomicUsize::new(0));
        let n = paniques.clone();
        tokio::spawn(async move {
            while let Ok(e) = bus.recv().await {
                if let Event::DriverStatus {
                    instance,
                    status: DriverStatus::Backoff { .. },
                } = e
                    && instance.as_str() == "panique"
                {
                    n.fetch_add(1, Ordering::Relaxed);
                }
            }
        });
        spawn_driver(&hub, "sain".into(), Arc::new(Sain), cancel.clone());
        until_running(&hub, "sain").await;
        spawn_driver(&hub, "panique".into(), Arc::new(Panique), cancel.clone());
        let (ok, ko, med, max) = ordres(&hub, Duration::from_millis(3500)).await;
        println!("- paniques capturées en 3,5 s : {}", paniques.load(Ordering::Relaxed));
        println!("- statut du pilote fautif : {}", status(&hub, "panique"));
        println!("- ordres au pilote sain pendant ce temps : {ok} réussis, {ko} échoués, médiane {}, max {}", ms(med), ms(max));
        cancel.cancel();
    });
}

fn experience_blocage() {
    println!("\n## E2 — un appel bloquant dans un pilote (2 workers)");
    runtime().block_on(async {
        let hub = Hub::new(HubOptions::default()).unwrap();
        let cancel = CancellationToken::new();
        spawn_driver(&hub, "sain".into(), Arc::new(Sain), cancel.clone());
        until_running(&hub, "sain").await;
        for (phase, bloquants) in [("référence", 0), ("1 pilote bloquant", 1), ("2 pilotes bloquants", 2)] {
            for i in 0..bloquants {
                let name = format!("bloquant{i}");
                if status(&hub, &name).is_empty() {
                    spawn_driver(&hub, name.as_str().into(), Arc::new(Bloquant), cancel.clone());
                }
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
            let stop = CancellationToken::new();
            let (pire, h) = sonde(stop.clone());
            let (ok, ko, med, max) = ordres(&hub, Duration::from_secs(2)).await;
            stop.cancel();
            let _ = h.await;
            println!(
                "- {phase:<20} ordres : {ok:>3} ok / {ko} ko, médiane {:>10}, max {:>10} ; retard max d'une minuterie 10 ms : {:>10}",
                ms(med),
                ms(max),
                ms(Duration::from_micros(pire.load(Ordering::Relaxed)))
            );
        }
        cancel.cancel();
    });
}

fn experience_fige() {
    println!("\n## E3 — un pilote figé dans une attente sans délai");
    runtime().block_on(async {
        let hub = Hub::new(HubOptions::default()).unwrap();
        let cancel = CancellationToken::new();
        let debut = Instant::now();
        spawn_driver(&hub, "fige".into(), Arc::new(Fige), cancel.clone());
        until_running(&hub, "fige").await;
        let p = point("fige");
        let t = Instant::now();
        let r = hub.command(&p, Value::Bool(true), Origin::Ui, None).await;
        println!("- ordre envoyé : {:?} après {}", r.err(), ms(t.elapsed()));
        tokio::time::sleep(Duration::from_secs(5)).await;
        println!(
            "- statut après {:.0} s : {} (aucun redémarrage : le superviseur ne voit ni panique ni retour)",
            debut.elapsed().as_secs_f64(),
            status(&hub, "fige")
        );
        cancel.cancel();
    });
}

fn experience_brique() {
    println!("\n## E4 — une « brique » lancée par tokio::spawn qui panique (moteur d'automatismes, historique…)");
    runtime().block_on(async {
        let hub = Hub::new(HubOptions::default()).unwrap();
        let cancel = CancellationToken::new();
        spawn_driver(&hub, "sain".into(), Arc::new(Sain), cancel.clone());
        until_running(&hub, "sain").await;
        let vus = Arc::new(AtomicUsize::new(0));
        let v = vus.clone();
        let mut bus = hub.subscribe();
        let brique = tokio::spawn(async move {
            while let Ok(e) = bus.recv().await {
                if matches!(e, Event::State { .. }) {
                    let n = v.fetch_add(1, Ordering::Relaxed) + 1;
                    if n == 3 {
                        let parts: Vec<&str> = "a".split(':').collect();
                        let _ = parts[1]; // invariant cassé dans une brique
                    }
                }
            }
        });
        let (ok, _, _, _) = ordres(&hub, Duration::from_millis(500)).await;
        println!("- ordres traités par la maison : {ok} ; événements vus par la brique : {} (morte au 3e)", vus.load(Ordering::Relaxed));
        println!("- tâche de la brique terminée : {} ; rien ne la relance, le hub reste « sain »", brique.is_finished());
        cancel.cancel();
    });
}

fn experience_abonne_lent() {
    println!("\n## E5 — un abonné lent sur le bus ne freine ni les pilotes ni les autres abonnés");
    runtime().block_on(async {
        let hub = Hub::new(HubOptions::default()).unwrap();
        let cancel = CancellationToken::new();
        spawn_driver(&hub, "sain".into(), Arc::new(Sain), cancel.clone());
        until_running(&hub, "sain").await;
        let (mut rapide, mut lent) = (hub.subscribe(), hub.subscribe());
        let recus = Arc::new(AtomicUsize::new(0));
        let r = recus.clone();
        tokio::spawn(async move {
            while rapide.recv().await.is_ok() {
                r.fetch_add(1, Ordering::Relaxed);
            }
        });
        let perdus = Arc::new(AtomicU64::new(0));
        let pl = perdus.clone();
        tokio::spawn(async move {
            loop {
                match lent.recv().await {
                    Ok(_) => tokio::time::sleep(Duration::from_millis(50)).await,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        pl.fetch_add(n, Ordering::Relaxed);
                    }
                    Err(_) => break,
                }
            }
        });
        let p = point("sain");
        let t = Instant::now();
        let mut pire = Duration::ZERO;
        for i in 0..3000 {
            let u = Instant::now();
            hub.command(&p, Value::Bool(i % 2 == 0), Origin::Ui, None)
                .await
                .unwrap();
            pire = pire.max(u.elapsed());
        }
        let total = t.elapsed();
        tokio::time::sleep(Duration::from_millis(100)).await;
        println!(
            "- 3000 ordres en {} ({:.0} ordres/s, pire {}), événements reçus par l'abonné rapide : {}, perdus par l'abonné lent (Lagged) : {}",
            ms(total),
            3000.0 / total.as_secs_f64(),
            ms(pire),
            recus.load(Ordering::Relaxed),
            perdus.load(Ordering::Relaxed)
        );
        cancel.cancel();
    });
}

/// 120 appareils × 6 points publiés par un pilote « maison ».
struct Maison;
impl Driver for Maison {
    fn kind(&self) -> &'static str {
        "maison"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let instance = ctx.instance().clone();
            for d in 0..120 {
                let id = ctx.device_id(&format!("d{d}"));
                let mut dev = lampe(&id, &instance);
                dev.points = (0..6)
                    .map(|k| PointSpec {
                        key: format!("p{k}").into(),
                        label: "p".into(),
                        kind: Kind::Numeric {
                            min: None,
                            max: None,
                            step: None,
                        },
                        access: Access {
                            read: true,
                            write: false,
                        },
                        unit: None,
                        semantic: Semantic::Other,
                    })
                    .collect();
                ctx.upsert_device(dev);
                for k in 0..6 {
                    ctx.set_state(&id, &format!("p{k}"), Value::Float(f64::from(d * k)));
                }
            }
            ctx.ready();
            std::future::pending::<()>().await;
            Ok(())
        })
    }
}

fn experience_verrou() {
    println!("\n## E6 — verrou global du modèle : 720 valeurs, lecteurs qui copient tout en continu");
    runtime().block_on(async {
        let hub = Hub::new(HubOptions::default()).unwrap();
        let cancel = CancellationToken::new();
        spawn_driver(&hub, "sain".into(), Arc::new(Sain), cancel.clone());
        spawn_driver(&hub, "maison".into(), Arc::new(Maison), cancel.clone());
        until_running(&hub, "sain").await;
        until_running(&hub, "maison").await;
        let t = Instant::now();
        for _ in 0..200 {
            std::hint::black_box(hub.snapshot());
        }
        println!("- hub.snapshot() (copie complète, ce que font /api/health, /api/devices, chaque SSE) : {} en moyenne", ms(t.elapsed() / 200));
        for lecteurs in [0usize, 4, 16] {
            let stop = CancellationToken::new();
            for _ in 0..lecteurs {
                let (h, s) = (hub.clone(), stop.clone());
                tokio::spawn(async move {
                    while !s.is_cancelled() {
                        std::hint::black_box(h.snapshot());
                        tokio::task::yield_now().await;
                    }
                });
            }
            let (ok, ko, med, max) = ordres(&hub, Duration::from_secs(1)).await;
            stop.cancel();
            println!("- {lecteurs:>2} lecteurs en boucle : ordres {ok} ok / {ko} ko, médiane {}, max {}", ms(med), ms(max));
        }
        cancel.cancel();
    });
}

/// Un parseur récursif naïf, comme on en écrit pour du XML ou du JSON maison :
/// il construit l'arbre des éléments imbriqués.
#[derive(Debug)]
enum Noeud {
    Liste(Vec<Noeud>),
}

#[inline(never)]
fn analyser(octets: &[u8], i: &mut usize) -> Noeud {
    let mut enfants = Vec::new();
    while *i < octets.len() && octets[*i] == b'[' {
        *i += 1;
        enfants.push(analyser(std::hint::black_box(octets), i));
    }
    Noeud::Liste(enfants)
}

fn profondeur(octets: &[u8]) -> usize {
    let mut i = 0;
    let arbre = analyser(octets, &mut i);
    let Noeud::Liste(enfants) = std::hint::black_box(&arbre);
    let n = enfants.len() + i;
    std::mem::forget(arbre);
    n
}

struct Recursif;
impl Driver for Recursif {
    fn kind(&self) -> &'static str {
        "recursif"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            ctx.ready();
            // Réponse hostile ou corrompue : 4 Mio de crochets ouvrants.
            let hostile = vec![b'['; 4 * 1024 * 1024];
            let n = profondeur(std::hint::black_box(&hostile));
            anyhow::bail!("profondeur {n}")
        })
    }
}

fn experience_pile() {
    runtime().block_on(async {
        let hub = Hub::new(HubOptions::default()).unwrap();
        let cancel = CancellationToken::new();
        spawn_driver(&hub, "recursif".into(), Arc::new(Recursif), cancel.clone());
        tokio::time::sleep(Duration::from_secs(2)).await;
        println!("non atteint si la pile déborde : {}", status(&hub, "recursif"));
    });
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("pile") {
        experience_pile();
        return;
    }
    // Les paniques provoquées exprès ne polluent pas la sortie.
    std::panic::set_hook(Box::new(|_| {}));
    println!("# Preuves exécutables contre moli-runtime (2 workers tokio)");
    experience_panique();
    experience_blocage();
    experience_fige();
    experience_brique();
    experience_abonne_lent();
    experience_verrou();
    println!("\n## E7 — débordement de pile dans un pilote (processus séparé)");
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("pile")
        .output()
        .unwrap();
    println!(
        "- code de sortie du processus : {:?} ; stdout : {:?} ; stderr : {:?}",
        out.status,
        String::from_utf8_lossy(&out.stdout).trim(),
        String::from_utf8_lossy(&out.stderr).lines().next().unwrap_or("")
    );
}
```

Code complet du chronométrage XML (`xmlbench/src/main.rs`, dépendance
`roxmltree = "=0.21.1"`) :

```rust
use std::time::Instant;
fn main() {
    for budget in [64 * 1024, 256 * 1024, 1024 * 1024] {
        // Un seul élément, autant d'attributs distincts que le plafond le permet.
        let mut xml = String::from("<a");
        let mut i = 0u32;
        while xml.len() + 16 < budget {
            xml.push_str(&format!(" b{i}=\"\""));
            i += 1;
        }
        xml.push_str("/>");
        let opts = roxmltree::ParsingOptions { allow_dtd: false, nodes_limit: 30_001, ..Default::default() };
        let t = Instant::now();
        let ok = roxmltree::Document::parse_with_options(&xml, opts).is_ok();
        println!("{:>5} Kio, {:>6} attributs : {:>8.2} s (parse ok = {ok})", xml.len() / 1024, i, t.elapsed().as_secs_f64());
    }
}
```

## Annexe B — Grille par pilote

Légende : « en ligne » = entrées-sorties faites dans la boucle qui lit les commandes ;
« détachée » = sous-tâche dont personne n'attend la fin.

| Crate | Modèle d'E/S | Sous-tâches | Attente sans délai | Bloquant ou calcul long | Non borné | Gravité max |
|---|---|---|---|---|---|---|
| z2m | MQTT (rumqttc), une boucle | aucune | non | gros JSON des capacités (≤ 4 Mio) en ligne | non | Faible |
| hue | HTTPS épinglé + SSE | PUT détaché ≤ 4,5 s | non (inactivité 10 min + keepalive) | non | non | Moyen |
| esphome | TCP chiffré (Noise) | lecteur et `JoinSet` bornés ; tâche vocale non annulée | une recherche LAN sans délai global | non | non | Moyen |
| tuya | TCP par appareil, UDP, WebSocket et HTTPS cloud | `JoinSet` jamais joint, 3 réceptions UDP hors `JoinSet` | WebSocket cloud (pong ignoré) | non | non | Moyen |
| sonos | SOAP toutes les 2 s + annonces | annonce détachée ≤ ~90 s | non | non | non | Moyen |
| androidtv | TLS 6466/8009 | lecteur `abort-on-drop` | non | non | non | Faible |
| androidtv-driver | TLS, deux boucles | `JoinSet` surveillé | une attente d'écriture sans délai | génération RSA en `spawn_blocking` | non | Faible |
| cast | mDNS + TLS 8009 | `abort-on-drop` | non | non | non | Faible |
| philips | HTTPS digest + télécommande Android | 4 tâches détachées (≤ 60 s) | non | non | non | Moyen |
| samsung | HTTPS + WebSocket par ordre | `JoinSet`, un ordre en vol | non | non | non | aucune |
| wyoming | TCP, sonde 60 s | aucune | non | non | non | aucune |
| alexa | HTTPS cloud | aucune | non (20 s par appel) | non | non | Moyen |
| twinkly | HTTP | `JoinSet`, paniques propagées | non | non | non | Faible |
| nuki | HTTP (pont mono-requête) | aucune | non | non | non | Moyen |
| lights | bus du hub + ordres aux membres | relance après mise sous tension, détachée ≤ 15 s | non (5 s par le hub) | non | non | **Élevé** |
| ariston | HTTPS cloud, acteur + `JoinSet` | jointes | non (10 s) | non | non | Faible |
| voltalis | idem | jointes | non | non | non | Faible |
| netatmo | HTTPS, une tâche dans le `select!` | aucune | non | non | non | Faible |
| overkiz | HTTPS local ou cloud, `JoinSet` | jointes | non | gros JSON en ligne | non | Moyen |
| landroid | HTTPS + MQTT sur WebSocket | aucune | réception bornée par ping/pong 25 s | non | non | Moyen |
| roborock | UDP miIO | aucune | non | non | non | Faible |
| bambu | MQTT TLS épinglé | aucune | non (keepalive 30 s) | non | état AMS non plafonné (imprimante de confiance) | Faible |
| moonraker | HTTP, sondage en ligne | aucune | non (4 s) | non | non | Moyen |
| frigate | MQTT clair + HTTP (images) | aucune | non | non | caméras créées par sujet MQTT | Moyen |
| reolink | HTTPS épinglé 1 Hz + RTSP | 2 familles de tâches détachées | non | rééchantillonnage audio en ligne | une tâche par appui | Moyen |
| tapo | HTTP (KLAP), en ligne | aucune | non (4 s) | non | non | Moyen |
| telegram | HTTPS par commande, en série | aucune | non (15 s × 10) | non | non | Moyen |
| weather | service paresseux (hors `Driver`) | aucune | non (15 s) | non | requêtes en file sur panne | Moyen |
| ipp | HTTP + travailleur d'impression | **détaché** | envoi jusqu'à 15 min (voulu) | non | non (64 documents, 32 Mio) | Moyen |
| igd | SOAP + SSDP | aucune | non par appel ; découverte sans plafond | non | candidats SSDP | Moyen |
| asuswrt | HTTP(S), cycle de 60 s | aucune | non (5 s par échange, 30 s par cycle) | non | non | Moyen |
| netgear | SOAP/XML, cycle de 60 s | aucune | non | **XML quadratique en ligne (E8)** | non | **Élevé** |
| huawei-lte | XML, cycle de 60 s | aucune | non | **XML quadratique en ligne (E8)** | non | **Élevé** |
| host | `select!` + sondes, tout en ligne | aucune | non (1,5 s par sonde) | `std::fs` sur `/proc` et `/sys` | non | Moyen |
| presence | UDP + `tokio::fs` | aucune | non | balayage de 3 s en ligne | non | Faible |
| phones | `select!`, notifications Expo en ligne | aucune | non (15 s) | `std::fs` sous `Mutex` std | non | Moyen |
| profile | requêtes séquentielles, sockets propres | 2 tâches hyper sans handle | non (5 s) | non | non | Moyen |
| reservations | relève ICS + tic de 1 s | aucune | non (10 s) | analyse iCal ≤ 1 Mio en ligne | non (budget 3 660) | Faible |
| helpers | boucle de commandes, délègue au hub | aucune | non | non | non (128 par type) | Faible |
| energy | enregistreur + pilote `Points` | enregistreur **non supervisé** | non | agrégation de rapport en ligne | non | Moyen |
| audio | bibliothèque synchrone | sans objet | sans objet | CPU chez l'appelant | taille de sortie de `resample` | Faible |
| ambiance | fonction async, 3 lampes en parallèle | aucune | bornée par le hub | non | non | Faible |
| alarm, rollup, media-catalog, media-target | bibliothèques pures | sans objet | sans objet | non | `rollup` : une statistique par heure, à borner chez l'appelant | aucune à Faible |
