# Moli OS : architecture

> État au 3 octobre 2026 au soir, avec les décisions prises après une revue adversariale du
> code et les arbitrages de l'auteur.

Plateforme domotique **agent-native**, ultra-légère, en Rust. Thèse : Home Assistant a été
pensé pour un humain qui clique ; Moli OS est pensé pour une famille qui veut que ça marche,
et pour des agents qui configurent, créent les automatismes et **écrivent les intégrations**,
sans jamais être aux commandes en temps réel.

Étalon : un Home Assistant réel, sur la même machine, avec 59 intégrations, 171 appareils,
1 591 entités, 90 automatisations, **2,1 Go de mémoire**. Moli OS à côté, ce jour-là : 71
appareils, 446 valeurs, 18 pilotes, **12 Mo**, 0,23 % CPU, image 12 Mo (mesures à jour :
[PERFORMANCE.md](PERFORMANCE.md)).

## Règles non négociables

1. **Aucun LLM dans la boucle de contrôle.** Le moteur est déterministe ; l'IA crée,
   explique, configure, rédige.
2. **La famille commande sans friction, les agents proposent, un humain valide ce qui agit
   seul.** Un clic dans le dashboard est obéi. Le PIN ne sert qu'à valider un automatisme et
   à autoriser un agent dans une pièce protégée ou aux heures calmes. (Changé le 3 oct. : la
   version précédente demandait un code à la famille aux heures calmes.)
3. **L'identité ne dérive jamais d'un nom ni d'une adresse** : `DeviceId = <instance>:<id
   natif stable>` ; nom et pièce sont des étiquettes.
4. **Valeurs typées, unités explicites.**
5. **Une panne de pilote n'emporte jamais le cœur** (tâche supervisée, backoff, statut), et
   **ses appareils passent hors ligne** le temps de la panne.
6. **Aucun secret en clair** : coffre chiffré de Moli (clé maître tenue par un gestionnaire de
   secrets, livrée en tmpfs) ; la config ne contient que des noms ; un LLM ne voit jamais un
   secret.
7. **Toute écriture est journalisée** : commandes, étiquettes, approbations, **et** toute
   modification d'un automatisme (qui, par où, quoi, résultat).
8. **Agnostique aux agents** : tout est exposé en MCP, REST et CLI.
9. **Le binaire fait respecter les règles.** Un profil ou un pilote écrit par un agent ne
   peut écrire qu'en passant par le hub (donc par la garde), ne peut atteindre que l'hôte
   déclaré, et n'est chargé qu'une fois validé par un humain.
10. **Toute entrée est bornée** (taille des corps, des textes, des files, des fichiers) et
    **tout ce qui vient du réseau est traité comme hostile**, LAN compris.
11. **Performance** : chaque livraison est mesurée contre HA sous un budget
    ([PERFORMANCE.md](PERFORMANCE.md)) ; une brique inactive ne coûte rien.

## Vue d'ensemble

```
 famille ─► Maison (Svelte, embarqué)        agents ─► MCP /mcp      scripts ─► REST /api/*
              │  SSE /api/events                │                        │
              └───────────────────────┬─────────┴────────────────────────┘
                                      │ moli-api (garde Host, origines, sessions PIN, bornes)
 ┌────────────────────────────────────▼──────────────────────────────────────────────┐
 │ moli-runtime : Hub (état, bus, commandes avec échéance, GARDE, approbations,      │
 │                journal, étiquettes, coffre chiffré, images caméras, notices)      │
 └───▲───────────────────────────────▲───────────────────────────────▲───────────────┘
     │ trait Driver                  │ abonnements au bus             │ briques
 ┌───┴──────────────────────┐  ┌─────┴──────────┐   ┌────────────────┴──────────────────┐
 │ pilotes natifs : z2m,    │  │ moli-history   │   │ moli-energy · moli-automation ·    │
 │ hue, tuya, reolink,      │  │ (SQLite)       │   │ moli-assistant (LLM compatible     │
 │ sonos, philips, frigate, │  └────────────────┘   │ OpenAI, outils, sous la garde)      │
 │ bambu, tapo, igd         │                       └────────────────────────────────────┘
 │ + profils déclaratifs    │  moli-net : HTTPS épinglé (TOFU), TLS public, HTTP, UPnP, web()
 │   (catalogue)            │  moli-core : le vocabulaire (aucune entrée/sortie)
 └──────────────────────────┘
```

La carte vivante de ce schéma est dans le dashboard : `#/systeme` (« Comment ça marche »).

## Crates

| Crate | Rôle | Doc |
|---|---|---|
| `moli-core` | domaine pur : identifiants, points, valeurs, événements, origines, journal | [coeur](components/coeur.md) |
| `moli-runtime` | hub, garde, superviseur, journal, étiquettes, coffre, médias | [coeur](components/coeur.md) |
| `moli-net` | HTTPS avec certificat épinglé, TLS public, HTTP, UPnP, requêtes web bornées | [pilotes](components/pilotes.md) |
| `moli-z2m`, `moli-hue`, `moli-tuya`, `moli-reolink`, `moli-sonos`, `moli-philips`, `moli-frigate`, `moli-bambu`, `moli-tapo`, `moli-igd` | pilotes natifs | [pilotes](components/pilotes.md) |
| `moli-profile` + catalogue | moteur de profils déclaratifs (lecture **et écriture** déclarées) | [profils](components/profils.md) |
| `moli-history` | historique SQLite, SQL en lecture seule pour agents | [historique & énergie](components/historique-energie.md) |
| `moli-energy` | kWh et € par heure, Linky + circuits, import HA | [historique & énergie](components/historique-energie.md) |
| `moli-automation` | automatismes en graphe, moteur déterministe, validation humaine | [automatismes](components/automatismes.md) |
| `moli-helpers` | aides de la maison (mode, alarme, budget), comme les `input_*` de HA | [pilotes](components/pilotes.md) |
| `moli-assistant` | Moli : conversation, création d'automatismes, import HA, textes | [moli](components/moli.md) |
| `moli-api` | REST, SSE, MCP, sessions humaines, garde Host, bornes, dashboard embarqué | [api](components/api.md) |
| `moli-os` | binaire : CLI, config TOML, démarrage, sauvegarde, santé | [exploitation](components/exploitation.md) |
| `web/` | dashboard Maison + Atelier (Svelte 5), plan vivant | [dashboard](components/dashboard.md) |

Une crate = une responsabilité. Un pilote ne dépend que de `moli-runtime`, `moli-net` et de son
transport. Les briques s'abonnent au bus ; aucune ne parle directement à un pilote.

## Commandes : le chemin d'un ordre

1. le point existe et est inscriptible ; la valeur est validée contre son `PointSpec` (type,
   plage, **longueur**) ;
2. **garde** : selon l'origine, la pièce et l'heure → exécuté, ou retenu en demande
   d'approbation (202) qu'un humain libère ; le dashboard (`ui`) est toujours obéi ;
3. routage vers le pilote propriétaire avec une **échéance** (5 s) : un ordre que personne
   n'attend plus n'est jamais exécuté, même après une reconnexion ;
4. journal ;
5. la confirmation d'état revient comme un événement `state`.

## Origines et confiance

| Origine | Qui | Garde | Comment on la reconnaît |
|---|---|---|---|
| `ui` | une personne dans le dashboard | jamais retenue | en-tête `x-moli-origin: ui` **et** une session PIN |
| `household` | le dashboard du foyer sans code | retenue en pièce protégée, sur une machine, pour sortir un appareil d'une pièce protégée ; jamais en heures calmes | en-tête `x-moli-origin: ui` depuis le foyer, sans session (mode `dashboard = "trusted"`, défaut) ; en mode `"pin"`, c'est `api` |
| `automation` | un automatisme validé par un humain | jamais retenue | lancé par le moteur |
| `system` | Moli lui-même | jamais retenue | interne |
| `api`, `mcp`, `cli`, `assistant` | agents et scripts | retenue en pièce protégée, en heures calmes, sur appareil sans pièce | tout le reste |

**Le PIN** (session liée à l'adresse, 30 jours) sert à : approuver une demande retenue,
valider / allumer / restaurer un automatisme, importer, supprimer, renommer un appareil déjà
nommé, et commander une pièce protégée ou une machine.

Le mode `trusted` suppose un réseau domestique de confiance : un appareil compromis sur le
LAN peut commander comme la famille hors des pièces protégées, mais ne peut ni y agir, ni
valider un automatisme, ni autoriser un agent (n'importe qui peut envoyer l'en-tête `ui` :
seule la session le prouve). Sur un réseau moins sûr : réseau IoT isolé, ou mode `pin`.

## Le paquet d'intégration

La forme que prend **toute** intégration, native ou déclarative, dans le catalogue interne
(`integrations/<id>/`) :

```
integrations/daikin-brp069/
├── integration.toml     # manifeste : id, nom, marques, modèles, transport (http|mqtt|…),
│                        #   découverte (mdns, ssdp, ports), identifiants/variables/options
│                        #   requis, origine (intégration HA « daikin », pydaikin, licence),
│                        #   auteur, et la liste des fixtures ([[fixture]] file, request)
├── profile.toml         # profil déclaratif (lectures [[request]], écritures [[write]])
│   — ou —  crate = "moli-daikin"   dans le manifeste, pour un pilote natif
├── fixtures/            # réponses réelles capturées sur l'appareil (anonymisées) ; pour un
│                        #   profil, chacune nomme la requête dont elle est la réponse
├── onboarding.md        # la procédure : questions à poser, appuis physiques, où trouver
│                        #   les identifiants, comment tester ; écrite par l'agent fort,
│                        #   déroulée par l'agent de Moli
└── README.md            # ce qui a été réutilisé, ce qui a été deviné, ce qui reste
```

- `moli-os catalogue check` valide chaque paquet (manifeste, profil, fixtures rejouées).
- Les paquets embarqués dans le binaire sont de confiance. Un paquet externe (fichier)
  n'est chargé qu'après `moli-os profile trust <chemin>` par un humain, qui a vu la liste
  des chemins lus et écrits.
- Un profil déclare ses écritures (`[[write]]`, rendues depuis la dernière lecture) ; elles
  sont exécutées **uniquement** via `hub.command`, donc sous la garde et au journal.
- Tout secret est lié à l'hôte déclaré ; un profil qui utilise un secret doit déclarer `host`.

C'est la forme MVP du catalogue. La forge de terrain (après MVP) produit exactement ces
paquets ; l'onboarding par brique les consomme.

## Décisions (3 octobre 2026)

| # | Décision | Choix | Pourquoi |
|---|---|---|---|
| D0 | Le dashboard et le PIN | dashboard sur le LAN obéi sans code (`household`) hors des pièces protégées et des machines ; PIN pour la validation, l'autorisation et les chambres ; mode `pin` disponible en config | Demander un code pour éteindre une lumière : inacceptable pour une famille. Mais l'en-tête du dashboard est public : sans code, un programme du LAN agissait dans les chambres comme la famille (audit du 12 oct.). Le code y est donc exigé, une fois par navigateur et par mois. |
| D1 | Qu'est-ce qu'une brique | module compilé **dormant** : aucune tâche, aucune minuterie, aucune mémoire tant qu'il n'est pas configuré ; cycle de vie `start(hub, config, cancel) → handle` ; `stop` = annulation + vidage des files + appareils hors ligne + désinscription | « Une brique inactive ne coûte rien » se vérifie par `0 poll` ; les prérequis de `stop` sont ceux de l'activation à chaud, implémentés dès le durcissement (échéances, drain, hors ligne). |
| D2 | Format des intégrations | **paquet** (ci-dessus) ; déclaratif étendu d'abord (écritures, puis MQTT et websocket), natif pour les transports lourds ; **toute écriture passe par le hub** ; WASM non retenu pour le MVP | La revue a montré qu'un profil écrit : l'isolation qui compte est celle de la garde, pas celle de la mémoire. WASM se réévalue si des paquets non relus doivent tourner chez des tiers. |
| D3 | Activation à chaud | après le MVP ; la config vivante (`data/bricks.json`) et `stop_driver` arrivent avec la page Administration | Choix de l'auteur : pas le système d'agents maintenant ; mais rien dans le code ne doit l'empêcher. |
| D4 | Découverte réseau | module `moli-discover` après le MVP, exposé en outils ; dès maintenant, SSDP n'accepte qu'une adresse privée du même sous-réseau, corps bornés, UDN mémorisé | Une découverte qui accepte n'importe quelle réponse est une porte. |
| D5 | Agent d'onboarding | après le MVP ; règles figées maintenant : noms d'appareils bornés et filtrés, identité native affichée à toute validation, secrets saisis par l'humain directement dans le coffre, l'agent ne choisit jamais une URL | Un agent peut rendre une phrase de validation mensongère ; `web()` vers `127.0.0.1` donnerait les droits `cli`. |
| D6 | La forge et le catalogue | **forge de terrain** : un agent crée le paquet dans la maison, contre le vrai matériel, en lisant HA/HACS/docs, capture les traces, et le propose au catalogue commun ; licences HACS au cas par cas (HA core est Apache-2.0) | Création sur place = validation sur l'appareil réel ; le catalogue évite que chaque maison soit un cas unique. |
| D7 | Au-delà d'une maison | après le MVP ; prérequis listés : bornes et débit (durcissement), HTTPS local, sessions d'appareil | Ce qu'il faut avant d'exposer Moli plus largement que le réseau de la maison. |
| D8 | Budget par brique | `tokio-metrics` `TaskMonitor` par tâche supervisée (stable, ~200 o), part de CPU et `0 poll` affichés sur la page Système ; `cache_size` SQLite explicite | Le moins cher qui prouve « inactif = zéro ». |
| D9 | Transports standards | évaluer **Matter** (`rs-matter`) comme transport du cœur après le MVP ; liste de marques recommandées ; la flexibilité (rétro-ingénierie) reste | Réduire la longue traîne au lieu de la courir, sans fermer la porte. |
| D10 | Matériel cible | Pi Zero 2 W (ou équivalent ARM 512 Mo), binaire seul sans Docker ; compiler pour ARM, puis mesurer | 30 € contre 150 € par installation. |
| D11 | Le plan de la maison | **la maison est une donnée** (`data/plan.json` : par étage, taille, murs, pièces, ouvertures, éléments, appareils, en cm ; altitude et décalage pour empiler les étages), **le rendu est un habillage** : 2D en SVG (zoom, 4 habillages) et 3D (three.js, chargée à la demande) depuis la même donnée. Un agent écrit la donnée à partir d'une photo du plan (`scripts/plan-from-image.py` extrait les murs) ; la personne ajuste au doigt. Un étage peut rester une simple image (ancien mode). | Le propriétaire veut zoomer, changer l'aspect, ajouter des choses, voir en 3D : impossible sur une image de fond. Une donnée géométrique sert aussi plus tard (présence par pièce, trajets, éclairage par zone). |

### D12 — Les lumières : luminaires, groupes, interrupteurs (6 octobre 2026)

Constat sur une installation réelle : une même lumière apparaît trois fois (le relais
derrière l'interrupteur, chaque ampoule, la pièce Hue), éteindre « le salon » a pu couper la
télé et l'enceinte, et une ampoule coupée au mur reste « allumée » (le pont Hue garde son
dernier état).

| Sujet | Règle |
|---|---|
| **Lampe** | ce qui éclaire : une ampoule connectée, ou un relais qui alimente une lumière ordinaire. |
| **Hors tension** | une lampe injoignable n'éclaire pas : jamais comptée allumée, jamais de halo ; l'écran dit « coupée à l'interrupteur » (rallumer l'interrupteur, pas Moli). |
| **Luminaire** (`[[fixture]]` dans `moli.toml`, pilote interne `lumieres`) | plusieurs ampoules et/ou le relais qui les alimente, vus et pilotés comme **une** lampe (`lumieres:<id>` : `on`, `powered`, `brightness`, `color_temp`, `color`). Ses membres disparaissent des vues de la famille (ils restent dans l'Atelier et dans la fiche du luminaire, pour les effets ampoule par ampoule). |
| **Ordres d'un luminaire** | les ampoules d'abord, le relais en dernier recours : éteindre = éteindre les ampoules joignables (le relais reste alimenté, les ampoules restent pilotables) ; le relais ne coupe que si aucune ampoule ne répond. Allumer = remettre le relais s'il est coupé (réponse dès que le relais a obéi), puis rallumer et régler les ampoules quand elles sont revenues (≤ 15 s, en tâche de fond). |
| **Groupe** (pièce ou zone Hue) | pas une lampe : la commande « toute la pièce ». Allumé si une de ses ampoules joignables l'est (le pont compte les ampoules coupées au mur). Pas de pastille sur le plan. |
| **Une pièce** | « éteindre la pièce » = ses lampes et luminaires, **jamais** la télé, l'enceinte ni les prises. |
| **Garde** | un ordre au luminaire est jugé sur les pièces de ses membres ; les ordres qu'il donne ensuite aux membres sont ceux du système (déjà jugés). |

### D13 — Stockage : le récent sur Turso, le long terme dans un service d'archive (17 octobre 2026)

Décidé. Les tickets de la vague 1 l'appliquent : C1 (bascule du cœur), C2 (lectures et outils),
C3 (service d'archive). Chaque affirmation ci-dessous est prouvée à la version figée par le
banc [`tools/storage-bench/`](../tools/storage-bench/README.md) (une commande, hors workspace) ;
les chiffres, leur dispersion et la façon de les rejouer sont dans
[l'annexe](annexe-d13-stockage.md). La référence est la mesure redb de l'étude de
dimensionnement, pas un nouveau choix.

**Les décisions du propriétaire et ce qu'elles entraînent**

| Décision | Conséquence |
|---|---|
| Turso remplace SQLite pour le stockage local : récent, état métier, journal d'export | un seul fichier `data/moli.db` remplace `history.db` et `energy.db` ; `rusqlite` ne reste que le temps du retour arrière |
| Le long terme : archives Parquet et résumés, tenus par un service Rust ; DuckDB facultatif et hors périmètre | le cœur ne garde que des fenêtres chaudes ; chaque écriture alimente l'outbox et le journal d'export immuable dans la même transaction |
| Le service d'archive tourne d'abord sur la même machine, déplaçable sur un VPS sans refonte | contrat réseau dès le premier jour : HTTP borné, aucun disque partagé |

**Ce que Turso 0.8.3 tient**

| Besoin | Résultat à 0.8.3 | Règle qui en découle |
|---|---|---|
| Tables rowid et index secondaires : `UPDATE`, `DELETE`, UPSERT, `RETURNING`, `CHECK`, `STRICT` | tenu ; 20 000 opérations aléatoires (dont 57 transactions annulées) donnent le même contenu que SQLite, et SQLite trouve le fichier Turso intègre | — |
| Tables `WITHOUT ROWID` (le schéma actuel) | **non** : Turso ouvre les fichiers actuels mais refuse `UPDATE` / `DELETE` et **lit faux** les plages de clé primaire (0 ligne au lieu de 200) | jamais de `WITHOUT ROWID` ; le rangement par clé passe par un rowid calculé (schéma ci-dessous) ; Turso n'ouvre jamais les fichiers actuels, ils sont copiés |
| Transactions | tenu : validation, annulation, `SAVEPOINT` | — |
| Lecteurs pendant une écriture | tenu : ils ne bloquent pas, ne voient rien de la transaction en cours et gardent leur instantané | un rédacteur, des lecteurs sur leurs connexions ; `BEGIN CONCURRENT` (MVCC) fonctionne mais reste expérimental : pas utilisé |
| Durabilité | tenu : `synchronous = FULL` par défaut, WAL, point de contrôle `TRUNCATE` ; sous `strace`, 200 validations font 214 fsync en `FULL`, 14 en `NORMAL` comme en `OFF` (ceux de la création du fichier) : seul `FULL` rend chaque validation durable | `FULL`, alors que les bases actuelles sont en `NORMAL` : une séquence déjà envoyée à l'archive ne doit jamais être perdue par le cœur, qui la réattribuerait à d'autres données ; lots d'écriture bornés (≤ 100 ms ou 500 opérations) pour borner le nombre de fsync |
| Cohérence après `kill -9` | tenu : 31 arrêts brutaux pendant des transactions multi-tables et des points de contrôle ; rien d'annoncé validé n'est perdu, l'outbox reste contiguë, heures = totaux = état des compteurs, `integrity_check` correct ; SQLite relit aussi le fichier laissé par le crash | — |
| Fichier compatible dans les deux sens | tenu : SQLite lit, modifie et vérifie un fichier écrit par Turso, que Turso relit ensuite | c'est la voie de secours : le schéma n'emploie rien que SQLite ne sache faire (ni MVCC, ni vecteur, ni plein texte) |
| Autorisateur SQL | **absent** : ni autorisateur, ni limite de longueur, ni budget de temps dans la crate `turso` ; une connexion en lecture seule accepte `PRAGMA query_only = 0`, `PRAGMA journal_mode`, `BEGIN`, plusieurs instructions et 10 Mo de `zeroblob` | `history_sql` devient un outil structuré (plus bas) |
| Un second processus | **refusé** en écriture (verrou exclusif) ; une ouverture en lecture seule voit les données validées | l'import en ligne de commande et la sauvegarde passent par le processus qui tourne |
| Sauvegarde | tenu : `VACUUM INTO` depuis le processus, avec un dossier temporaire (voir « Dossier temporaire ») ; SQLite trouve la copie intègre | `backup.rs` passe par le processus qui tourne |
| Image `FROM scratch`, musl statique, linux/arm64, profil seccomp par défaut de Docker | tenu ; io_uring y est refusé, Turso n'en a pas besoin (`pread`, `pwrite`, `fsync`) | backend d'entrées / sorties par défaut, jamais `io_uring` |
| Dossier temporaire | `BEGIN IMMEDIATE` ouvre la base temporaire de la connexion, **sur disque** par défaut (`TURSO_TMPDIR`, `SQLITE_TMPDIR`, sinon `/tmp`) : sans `/tmp`, comme dans l'image `FROM scratch`, tout rédacteur échoue (SQLite, lui, se rabat sur le dossier courant). `PRAGMA temp_store = MEMORY` sauve les rédacteurs, pas `VACUUM INTO` (la sauvegarde), qui ouvre sa propre connexion. Trouvé par le banc dans l'image, invisible sur macOS | `TURSO_TMPDIR` sur le volume de données (`data/tmp`, créé au démarrage, aussi pour un déploiement sans Docker) et `temp_store = MEMORY` sur chaque connexion |

**Schéma cible** (un fichier, tables rowid, `STRICT` ; texte exact dans
`tools/storage-bench/src/schema.rs`)

```sql
meta         (id = 1, schema_version, installation, next_seq, first_energy_hour)
catalog      (id, point UNIQUE, family, unit, hot_days)
samples      (id = point_id·2^44 + ts, point_id, ts, seq, kind, num, txt)      -- aucun index secondaire
meter_state  (id ≤ 1023, meter UNIQUE, last_kwh, last_ts, first_ts, imported_until)
energy_hour  (id = heure·1024 + compteur, meter_id, hour, kwh, cost, seq)      -- aucun index secondaire
energy_total (id, span 'day'|'month', start, meter_id, kwh, cost)  UNIQUE (span, start, meter_id)
outbox       (seq PRIMARY KEY, kind, ref)                           -- attente de publication
export_journal (seq PRIMARY KEY, payload, published_at)               -- contenu immuable
archive_ack  (id = 1, durable_upto, published_upto, generation, at, replay_floor)
```

- **`samples` est rangée par point puis par instant**, comme la clé `(point_id, ts)` actuelle :
  rowid = `point_id · 2^44 + ts` (524 288 points, millisecondes jusqu'en 2527, vérifié par
  `CHECK`). Une courbe, une ancre, une purge sont des plages de rowid. Avec un rowid
  ordinaire et un index secondaire `(point_id, ts)`, chaque ligne coûte un aller-retour
  index → table : la courbe 7 jours du point actif était six fois plus lente (mesure
  exploratoire, annexe). Deux relevés d'un point à la même milliseconde : le second est
  ignoré (`ON CONFLICT (id) DO NOTHING`), comme aujourd'hui, et ne consomme pas de séquence.
- **`energy_hour`** est rangée par heure puis par compteur (rowid = `heure · 1024 + compteur`) :
  un graphique horaire est une plage, l'UPSERT additif de `moli-energy` une recherche de rowid,
  sans `RETURNING` : la clé est connue d'avance, l'outbox la reçoit telle quelle.
- **`energy_total`** tient les totaux de jour et de mois civils locaux (plus bas).
- **`meter_state`** : dernier relevé, `first_ts` (l'actuel `meter_first`), `imported_until`
  (marqueur d'import) ; son `id` sert de numéro de compteur.
- **`outbox`** : attente de publication, une ligne par opération. La transaction de la
  donnée ajoute aussi une ligne à **`export_journal`** : son contenu NDJSON autonome
  (séquence, type, point ou compteur nommé, valeurs) est sérialisé une seule fois.
  Ce contenu reste immuable, même si la même heure d'énergie change à la séquence
  suivante. Un export copie ces octets dans l'ordre des séquences ; il ne joint
  jamais l'outbox aux tables métier mutables.
- **`seq`** vient de `meta.next_seq`, mis à jour dans la transaction : un
  `INTEGER PRIMARY KEY` seul réutilise les numéros libérés (prouvé). Le `seq` des
  tables métier sert au suivi et à la purge ; le rejeu vient du journal immuable.
- **`export_journal.published_at`** est posé au reçu de la publication. L'outbox
  peut alors être vidée, mais le journal garde chaque opération au moins **7 jours
  après publication**, même pour un import portant sur une date ancienne. La purge
  avance par préfixes contigus de 10 000 opérations au plus et persiste
  **`archive_ack.replay_floor`**, la dernière séquence retirée. Le journal ne se
  purge pas avant publication ni au simple accusé de réception durable.
- **`meta`** : version du schéma (migrations en avant seulement, vérifiées à l'ouverture),
  identité de l'installation (128 bits tirés au hasard à la création, jamais un nom ni une
  adresse), prochaine séquence, première heure d'énergie (le « depuis » de l'écran).
- **L'index d'expiration est la clé** : la purge, une transaction courte par point, supprime
  `[point · 2^44, ancre)`, l'ancre étant la dernière valeur avant la limite (une valeur stable
  garde son « avant ») ; pour une famille archivée, seulement `seq ≤ published_upto`.

**Totaux de période : accumulateurs persistants.** Option écartée : garder deux mois civils
d'heures et `Energy::summary()` inchangé. À la charge ×2, `summary()` lit 127 452 heures
(p95 49,30–53,31 ms) ; depuis les totaux, 456 lignes (p95 0,48–0,52 ms), même réponse. `energy_total` est
tenue dans la transaction de l'heure, par le même apport additif, sur tous les chemins qui
écrivent une heure (relevé vivant, import, re-tarification) ; `kill -9` le vérifie. Les jours
restent 62 jours, les mois 13 mois (le graphique 12 mois reste local, p95 2,64–3,21 ms), les heures
deux mois civils (graphiques horaires jusqu'à 31 jours, recalcul des totaux). Au démarrage,
somme des jours = somme des heures sur la fenêtre locale, sinon recalcul journalisé.

**Durées par défaut par famille** (les familles de `moli-history::policy`, réglables par
point, puis par famille, puis en général ; une baisse sous le plancher d'un point passe par
la garde existante, plan validé par PIN ; aucune vue actuelle n'y perd)

| Famille | Local | Archive | Vues servies en local |
|---|---|---|---|
| `states` : états et événements (portes, présence, alarme, sonnette, passages) | 31 jours | détail 365 jours | Sécurité 720 h, Caméras 24 h, « dernier passage » 48 h |
| `power` : puissance | 7 jours | détail 30 jours, heures 10 ans | Énergie 168 h, Système 7 j |
| `climate` : température, humidité | 14 jours | détail 90 jours, heures 5 ans | Système 7 j |
| `energy` : énergie | heures : 2 mois civils ; totaux : 62 jours, 13 mois | heures 10 ans | Énergie 24 h, 7 j, 30 j, 12 mois |
| `diagnostic` : diagnostic technique | 7 jours (pas 3 : Système propose 7 j pour tout point) | aucune | Système 7 j |
| `other` : le reste | 7 jours | détail 30 jours | Système 7 j |

Au-delà du local, `/api/history` et l'outil `history` complètent par l'archive ; archive
injoignable : la partie locale seule, marquée `"archive": "unavailable"`.

**Le contrat cœur ↔ archive (v1)**

- **Déploiement** : un binaire dédié, `moli-archive` (sa crate, son image `FROM scratch`), pas
  une sous-commande de `moli-os` : Parquet et ses dépendances restent hors du budget d'image
  et de mémoire du cœur, une panne d'archive n'emporte pas le cœur, et l'image locale est
  celle qui partira sur le VPS. Par défaut, un second service du même `compose.yaml`, avec
  son propre volume ; sans Docker (D10), un second processus. Aucun disque partagé : tout
  passe par HTTP.
- **Envoi** : `POST /v1/installations/{installation}/batches/{premier}-{dernier}`, corps
  NDJSON compressé gzip, au plus 5 000 opérations et 1 Mio compressé (8 Mio décompressé),
  séquences **contiguës** depuis `durable_upto + 1`. Chaque opération est autonome : le point
  ou le compteur y est nommé (les identifiants locaux ne sortent pas). Un relevé est un ajout ;
  une heure d'énergie est un état (la plus grande séquence gagne), donc idempotente.
- **Lot idempotent** : identité = installation + plage + SHA-256 du **NDJSON
  décompressé**, concaténation des contenus persistés et d'un saut de ligne par
  opération. L'en-tête gzip ne participe pas à l'identité. Pour une même plage,
  les octets restent identiques après mise à jour métier et redémarrage. Le même
  lot rend le statut courant sans nouvelle écriture ; une plage déjà reçue avec un autre contenu → 409 ;
  un trou → 409 avec `durable_upto`. Après une erreur de transport ou un
  redémarrage, le cœur relit `status`, puis reprend à `durable_upto + 1` : un
  accusé perdu ne conduit pas à assembler une plage recouvrant un lot déjà reçu.
- **Deux accusés distincts**, dans chaque réponse et dans `GET /v1/installations/{installation}/status` :
  `durable_upto` (reçu et écrit durablement côté archive : le cœur cesse de renvoyer) et
  `published_upto` (archives Parquet et résumés publiés : le cœur peut purger), plus
  `generation`. Le cœur les recopie dans `archive_ack`, marque les opérations du
  journal comme publiées et vide l'outbox jusqu'à `published_upto` dans la même
  transaction. Les sauts importants d'accusé sont traités par tranches bornées.
- **Génération** : elle change quand l'archive repart d'une sauvegarde. Dans une
  génération, les accusés ne reculent pas. À une nouvelle génération, le cœur
  vérifie `replay_floor ≤ durable_upto ≤ next_seq - 1` et
  `0 ≤ published_upto ≤ durable_upto`. Il renvoie le **journal immuable** depuis
  `durable_upto + 1`, y compris les versions d'heures remplacées et les imports
  anciens dont les lignes locales ont été purgées. Les opérations au-delà du
  nouveau `published_upto` perdent leur ancien `published_at` : leur republication
  ouvre une nouvelle fenêtre de 7 jours. Une sauvegarde à `durable_upto` inférieur
  à `replay_floor` est explicitement refusée comme hors fenêtre ; aucune reprise
  sans perte n'est alors promise. Une séquence absente du journal bloque aussi
  l'envoi, sans fabriquer de remplacement depuis les tables métier.
- **Jeton** : un porteur aléatoire, dans le coffre de Moli côté cœur (règle 6), dans un
  fichier secret côté archive ; jamais dans `moli.toml`.
- **Délais** : connexion 5 s, lot 30 s, un seul lot en vol, reprise exponentielle de 1 s à
  5 min avec gigue ; lectures interactives 2 s, bornées en durée et en points comme
  `/api/history`.
- **Passage sur un VPS** : même image derrière TLS (le cœur refuse `http://` hors adresse
  privée), nouveau jeton ; arrêter l'archive locale, copier son dossier, démarrer sur le VPS,
  vérifier que `status` rend les mêmes `durable_upto`, `published_upto` et `generation`,
  changer `[archive] url` et le jeton du cœur. Le VPS garde une sauvegarde
  **cohérente et vérifiée** des fichiers publiés, des lots reçus durablement et de
  leurs métadonnées. Elle doit avoir moins de 7 jours et son `durable_upto` doit
  rester dans la fenêtre de reprise du cœur (`≥ replay_floor`). Le journal, retenu
  selon la date de publication et non la date métier, permet alors de rejouer
  toutes les opérations postérieures à cette sauvegarde. Une indisponibilité du
  service, une sauvegarde trop ancienne ou une restauration hors fenêtre sont
  signalées ; la purge des données non publiées reste bloquée. La seule rétention
  des lignes métier, même de 7 jours, n'assure pas cette reprise.

**`history_sql` devient un outil structuré** (C2), faute d'autorisateur : point ou motif de
points, intervalle, opération (valeurs, seaux moyenne / min / max / nombre, comptes par
valeur, première / dernière), 200 lignes au plus, servi par le cœur et l'archive. L'agent ne
tape plus de SQL.

**Bascule et retour arrière**

1. **Bascule** (C1), au démarrage, avant le hub : si `moli.db` manque et que `history.db` ou
   `energy.db` existe, copie dans `moli.db.tmp` (une séquence et une entrée d'outbox par
   ligne : l'archive reçoit l'historique existant ; totaux recalculés depuis les heures), fsync,
   renommage. Les anciens fichiers ne sont plus jamais écrits. Mesure : 100 200 opérations
   copiées en 9,33 s sur un CPU (séquences, outbox et totaux compris), soit quelques minutes
   pour un historique de 30 jours : une seule fois, hub arrêté, progression au journal.
2. **Retour arrière** : Moli arrêté, sur une **copie** des anciens fichiers, le SQL de
   `ROLLBACK_HISTORY` / `ROLLBACK_ENERGY` (SQLite seul, `ATTACH` du fichier Turso) y ajoute ce
   qui a été écrit depuis ; puis l'ancienne version. Prouvé à l'identique ; 100 200 lignes en 0,13 s.
3. **Période de retour arrière : 30 jours.** `rusqlite` reste lié (projection de 35,7 Mo), les anciens
   fichiers sont gardés, aucune purge plus courte que la rétention actuelle (30 jours) ; les
   durées par famille s'appliquent ensuite, puis `rusqlite` et les anciens fichiers partent.

**Turso avant 1.0**

- `turso = "=0.8.3"`, `default-features = false` (sans allocateur global `mimalloc` ni plein
  texte), `Cargo.lock` figé.
- Une montée de version = ce banc à la nouvelle version (compatibilité, `kill -9`, charge) sur
  linux/arm64, plus l'ouverture d'un fichier écrit par l'ancienne ; un échec bloque.
  Sauvegarde `VACUUM INTO` avant toute montée.
- Turso ne déclare pas de version minimale de Rust, et ses dépendances du jour demandent
  Rust 1.90 (`roaring` 0.11.5) et 1.89 (`uuid` 1.28) : `Cargo.lock` épingle `roaring` 0.11.3
  et `uuid` 1.26.1 (celle de Moli), et le banc compile alors en 1.88, le minimum annoncé. À
  noter hors de ce ticket : un compilateur 1.91 refuse déjà le dépôt (gardes `if let` dans
  `moli-net`), le minimum annoncé n'est donc plus vérifié par personne.
- Ultime recours : le fichier reste un fichier SQLite que `rusqlite` sait lire et écrire.

**Budget d'image : 40 Mo, décision du propriétaire (17 octobre 2026).** Mesuré en
linux/arm64 musl avec le profil de release de Moli (`strip = "debuginfo"`) : Moli aujourd'hui
23,1 Mo mesurés ; le delta des sondes Turso/SQLite est de +11,4 Mo : projection
**34,4 Mo**, ou **35,7 Mo** avec les deux moteurs pendant le retour arrière.
L’image réelle du cœur intégré reste à mesurer en C1. Le budget de 25 Mo ne tenait dans aucune
configuration : `turso_core` réduit à ses fonctions minimales ne gagne qu'environ 1 Mo
(collation Unicode, chiffrement et vecteurs ne se désactivent pas en 0.8.3), un strip
complet donne encore 26,6 Mo. Retenu : les symboles restent (des traces lisibles en
production), le budget passe à 40 Mo ([PERFORMANCE.md](PERFORMANCE.md)), réévalué à chaque
montée de Turso. Ce sont des pages de fichier, pas de la mémoire anonyme : le budget de
32 Mio ne bouge pas.

**Réserves avant bascule (série complète avec journal immuable, 18 octobre 2026).**
Le graphique d'énergie sur 168 h reste à 30,37–31,83 ms au p95 sur un CPU,
avant HTTP ; la rafale vise 974 op/s et en atteint 949,24–965,17 avec cinq
lecteurs. C1/C2 doivent prouver le budget de lecture et la capacité requise
sur le cœur intégré avant mise en service. Pic RSS du processus comparant
toutes les options : 53,21–54,32 Mio ; le budget de 32 Mio anonymes reste
à mesurer avec les seuls chemins retenus. Le journal immuable porte le fichier
à 465,14 Mo sur la charge ×2, contre 151,74 Mo sans ce journal : cette
rétention locale est le coût de la reprise bornée. La projection de taille
reste à vérifier sur l'image réelle de C1. Réserves aussi consignées dans
[PERFORMANCE.md](PERFORMANCE.md) ; choix et plafonds du propriétaire inchangés.

**Ce qui invaliderait cette décision**

- Sur le matériel cible (D10), une lecture du tableau de bord au-delà de 25 ms au p95, ou la
  mémoire anonyme au-delà de 32 Mio, que le cache de pages ne suffit pas à ramener.
- Une version de Turso qui casse la compatibilité de fichier avec SQLite ou un essai du banc,
  sans version maintenue qui le tienne ; ou la fin de la crate Rust embarquée.
- L'image au-delà de 40 Mo.
- Un besoin d'agent que l'outil structuré ne sait pas servir et qui exigerait du SQL libre.
- Un service d'archive qui aurait besoin du disque du cœur.

## Sécurité (réseau de la maison + Cloudflare Access)

Garde `Host` sur toutes les routes (anti DNS-rebinding). Pas de compte : la famille est
reconnue par son réseau, ou par Cloudflare Access quand elle passe par le tunnel
(par exemple `maison.example.org`, assertion vérifiée par Moli lui-même). **Tout autre appelant est
refusé (403)**, y compris un appareil du réseau qui imite les en-têtes de Cloudflare : ils
ne comptent que venus du tunnel, donc de la machine elle-même. Les actes qui engagent
(validation, autorisation, import, plan) exigent une session PIN liée à l'appelant ; les
agents sont soumis à la garde. Bornes de taille et de débit sur toutes les entrées ; les
gros envois sont refusés avant d'être lus si personne n'est identifié. Avant une exposition
plus large (appli mobile, accès distant) : sessions d'appareil.

## Crochets réservés (à ne pas casser)

- `DriverConfig { id, kind, enabled, options }` et `build_driver` : la page Administration
  remplacera le `match` par le catalogue, et `moli.toml` par `data/bricks.json` pour la
  config vivante. Garder `moli.toml` pour la config machine.
- `Hub::register_driver` / `set_driver_status` : `unregister_driver(instance)` viendra avec
  l'activation à chaud ; ne pas supposer qu'une instance vit toujours.
- `/api/system` : la liste des briques y est écrite à la main ; à rendre générique quand les
  briques auront un trait.
- `Action` du journal : extensible (les automatismes y ont été ajoutés) ; l'astreinte lira ce
  journal.
- `DriverStatus::Backoff { error, retry_in_ms }` : c'est le signal de l'astreinte.

## Décisions d'origine revisitées

- « Pas de WASM (trop lourd) » (2 oct.) : confirmé pour le MVP, pour une autre raison (D2).
- Transports natifs en Rust + profils déclaratifs pour la longue traîne : confirmé, avec les
  écritures déclarées et le paquet comme unité.
