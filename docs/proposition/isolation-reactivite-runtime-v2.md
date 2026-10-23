# Analyse V2 — Workflows, cloisons, mémoire et relance rapide

Date : 21 octobre 2026. Statut : **analyse, aucune modification du code**.
Base analysée : `main` au commit `bd1714d`. La [première analyse](isolation-reactivite-runtime.md)
reste valable et inchangée ; cette V2 l'approfondit sur quatre demandes.

Méthode : lecture du code ; cinq bancs d'essai exécutés hors du dépôt contre les vraies crates
`moli-runtime` et `moli-automation` ([annexe A](#annexe-a--bancs-dessai)) ; comptes agrégés de
deux Home Assistant réels très chargés, appelés ici « instance A » et « instance B » (aucun nom
de lieu, d'appareil ni d'entité n'est repris). Machine de test : Apple M4, runtime Tokio à
2 workers comme en production. Les durées absolues ne se transposent pas au Pi Zero 2 W (D10),
les effets relatifs si ; les extrapolations sont marquées « estimé ».

## 1. Réponse courte

| Question | Réponse | Preuve |
|---|---|---|
| Le moteur tient-il des workflows complexes ? | Oui à l'échelle d'une maison : moins de 3 ms entre un événement et l'action, à 400 automatismes. Mais chaque automatisme coûte à **chaque** événement, une rafale retarde les actions jusqu'à 161 ms, et un retard du bus **perd des déclencheurs**, y compris ceux arrivés après la rafale. | W1, W2 |
| « Allume 30 minutes » est-il fiable ? | Non : l'attente vit en mémoire ; un redémarrage ou une bascule l'efface, la lumière reste allumée. | lecture du code |
| Un moteur compilé changerait-il la donne ? | Oui : de ×280 (100 automatismes) à ×5 000 (10 000) sur le chemin chaud, soit 3 à 11 ns par événement au lieu de 0,9 à 54 µs ; agrégats lus en 1 ns au lieu de 150 à 190 µs par SQLite ; calculs ×23 à ×27. | I1 à I4 |
| Des tampons peuvent-ils empêcher le blocage ? | Une file ne libère pas un thread bloqué ; il faut une **cloison** : un thread par pilote (~52 Kio chacun) garde le cœur à ≤ 0,1 ms de médiane avec six pilotes bloquants. Les files servent ailleurs : une **voie par appareil** fait passer de 16 % à 100 % d'ordres réussis derrière un pont dont une lampe est muette. | C1 à C3 |
| « Tout passe par le moteur » ? | Vérifié : le hub est le point de convergence et le moteur voit tout, même avec des pilotes sur leurs propres threads (réaction en 0,2 ms). Deux précautions : les tâches propres au hub doivent tourner sur le runtime du cœur, et le hub doit se protéger des inondations. | K1, lecture du code |
| Peut-on se lâcher en mémoire ? | Oui, avec un format compact : les valeurs courantes des deux maisons réunies tiennent dans 1 Mio, 24 h d'historique dans 1,4 Mio, 7 jours dans 8,8 Mio (40,7 Mio en format naïf). | M1 à M3 |
| Relancer vite ailleurs ? | Réaliste : instantané de 20 à 54 Kio compressé, hub reconstruit en 11 à 14 ms, journal de 1,3 Mio par jour. Conditions : rendre durables les workflows en cours, une seule machine maîtresse à la fois, secours passif. | M4, R1, R2 |

## 2. Données de référence : deux Home Assistant réels

| | Instance A | Instance B | Réunies |
|---|---|---|---|
| Appareils | 206 | 290 | 496 |
| Entités enregistrées / actives | 1 524 / 977 | 1 830 / 1 274 | 3 354 / 2 251 |
| États vivants | 986 | 1 278 | 2 264 |
| Automatismes actifs | 12 | 10 | 22 |
| Changements par seconde, moyenne / pointe | 0,51 / 39 | 0,98 / 239 | ~1,5 |
| Pointe sur une minute | 214 | 1 471 | |
| Taille moyenne d'un jeu d'attributs (Recorder) | 229 caractères | 1 365 caractères | |

Les deux instances réunies produisent ~163 000 relevés par jour sur 1 487 séries. La
distribution est très concentrée : une série médiane change 0,7 fois par jour, les 1 % les
plus actives font 51 % du trafic, les 10 % les plus actives 98 %. Les bancs suivants sont
calibrés sur ces chiffres, puis poussés à 10 fois au-delà.

## 3. Workflows : tous les types

### 3.1 Ce que le moteur sait faire aujourd'hui

Un automatisme est un graphe à la manière de n8n (`moli-automation/src/model.rs`) :

- **Déclencheurs :** changement d'état, seuil (avec durée « depuis »), occurrence (appui),
  heure, soleil, période, démarrage, manuel.
- **Logique :** `If` avec des règles ET/OU sur un état, l'heure ou le soleil ; l'imbrication
  passe par des `If` en chaîne.
- **Actions :** `Set`, `Toggle`, `Scene`, `Wait`, `WaitFor`, `Notify`, `Write` (texte rédigé
  par l'assistant).
- **Modes d'exécution :** `restart`, `single`, `queued`.

Plafonds actuels : 400 automatismes (`lib.rs:994`), 60 nœuds et 120 liens par graphe
(`check.rs:16-17`), 100 étapes par exécution (`run.rs:17`), ni boucle, ni variable, ni calcul,
ni lecture d'historique. Garde-fou anti-emballement : au-delà de 20 lancements par minute,
l'automatisme est **éteint** (`engine.rs:15-16`). Un automatisme branché sur un capteur
bavard (puissance, mouvement) est donc coupé, pas ralenti.

À côté du moteur, le hub porte déjà des primitives durables : minuteurs, compteurs,
plannings, calculs (agrégats min/max/moyenne), scènes et, depuis `5bf57b1`, **durées avec
couverture connue** (`moli-runtime/src/durations.rs`, échantillonnées sous le verrou du
modèle avec un index par point et un écrivain durable à part). C'est le bon germe pour les
agrégats en mémoire.

### 3.2 Le vrai moteur sous charge

Chaque automatisme du banc a 1 déclencheur, 3 conditions imbriquées de 3 règles et
1 action ; 10 % sont temporaires (« allume 30 min »), 10 % attendent une condition
(`wait_for`) ; 1 800 capteurs et 400 lampes, la forme d'une grosse maison.

| Automatismes | Mémoire du moteur | Croisière, p50 / p99 | Rafale de 2 000 événements : action au milieu / après | Rafale de 5 000 : action au milieu / après |
|---|---|---|---|---|
| 50 | 0,7 Mio | 0,4 / 2,2 ms | 6,4 / 11,5 ms | 12,8 / 23,3 ms |
| 100 | 1,1 Mio | 0,4 / 1,3 ms | 10,5 / 18,6 ms | 40,3 ms / **perdues** |
| 200 | 2,0 Mio | 0,5 / 0,9 ms | 21,9 / 33,7 ms | 36,1 / 70,0 ms |
| 400 | 3,7 Mio | 0,5 / 2,9 ms | 36,6 / 59,7 ms | 80 ms (pire 161 ms) / **perdues** |

Lecture :

- En croisière, le moteur est très réactif.
- Dans une rafale (un pont qui se reconnecte et republie tout), chaque événement coûte au
  moteur environ **60 ns par automatisme**, qu'il le concerne ou non : il les parcourt tous
  (`engine.rs:351-357`). À 400 automatismes, ~30 µs par événement ; estimé 0,3 à 0,6 ms
  sur un Pi Zero 2 W, soit un retard du bus dès qu'une rafale dépasse ~1 000 événements.
- Créer et valider les automatismes un par un coûte de plus en plus cher : 0,3 s pour 50,
  17,5 s pour 400 (environ 60 ms par enregistrement quand 300 existent).

### 3.3 Un retard du bus perd des déclencheurs, même après la rafale — Élevé

À 5 000 événements, le moteur prend du retard sur le bus (`Lagged`) et les cinq déclencheurs
injectés **juste après** la rafale ne partent jamais. Le mécanisme est dans le code :

1. Sur `Lagged`, le moteur appelle `remember_all()` (`engine.rs:77-85`), qui recopie les
   valeurs **actuelles** du hub dans sa table `last` (`engine.rs:340-349`).
2. Ces valeurs contiennent déjà les événements encore en file.
3. Quand le moteur lit ensuite ces événements, `on_state` voit
   `valeur précédente == nouvelle valeur` et s'arrête (`engine.rs:351-356`).

Un retard ne perd donc pas seulement les événements sautés ; il neutralise aussi ceux qui
restent. Correctif simple : sur `Lagged`, comparer l'instantané à `last` et produire une
transition pour chaque valeur qui a changé, au lieu d'écraser `last`.

### 3.4 Workflows temporaires — Élevé

« Allume pendant 5 minutes » devient `set on → wait 300 → set off`
(`moli-assistant/src/before.rs:86`). L'attente est une tâche en mémoire ; à l'arrêt, les
exécutions en cours sont interrompues (`engine.rs:99-113`) et rien ne les reprend au
démarrage. Après un redémarrage, une mise à jour ou une bascule, la lumière reste allumée.

Proposition : un **bail** durable, primitive à part entière.

- `allumer le salon pour 30 minutes` = un ordre et une échéance absolue, persistés ensemble
  (une écriture synchronisée, 3,4 à 3,7 ms mesurées, une seule fois au lancement).
- Le retour est conditionnel : il n'éteint que si la valeur est toujours celle que le bail a
  posée ; si quelqu'un a changé la lumière entre-temps, le bail s'éteint sans rien faire.
- Un nouveau « 30 minutes » prolonge le bail au lieu d'en empiler un second.
- Au redémarrage ou sur la machine de secours, un bail échu s'applique une fois, un bail en
  cours reprend son attente, comme les minuteurs AUTO-H1 le font déjà pour leur état.
- C'est un ordre de la famille, jugé par la garde et journalisé comme les autres ; ce n'est
  pas un automatisme permanent, il n'a pas besoin d'une validation par PIN.

10 000 baux en attente coûtent 0,3 à 0,4 ms d'ordonnancement au total (tas binaire, I4).

### 3.5 Les lectures dont les workflows ont besoin

| Lecture | Aujourd'hui | Mesure | Proposition |
|---|---|---|---|
| États et conditions imbriquées | Chaque règle relit la valeur par son nom texte, prend deux fois le verrou du modèle et reconvertit sa constante JSON (`run.rs:544-625`) | Balayage : 1,8 à 2,1 µs par événement à 400 automatismes, 48 à 54 µs à 10 000 ; compilé : 3 à 11 ns (I1) | Compiler à la validation : points numérotés, constantes typées, règles en arbre |
| Historique et agrégats (« moyenne 1 h », « allumé depuis ») | Pas dans les graphes ; SQLite en `spawn_blocking` pour l'historique | SQLite : 150 à 190 µs par lecture, plus un passage de thread ; fenêtre en mémoire : 3,7 à 4,2 µs ; agrégat tenu au fil de l'eau : 5 à 6 ns par relevé, 1 ns par lecture (I2) | Agrégats incrémentaux déclarés par les plans compilés, comme les durées ; SQLite pour le long terme seulement |
| Services extérieurs (météo, agenda, HTTP) | Pilotes et profils qui publient des points ; météo lue à la demande | — | Toujours en amont : une source publie des valeurs avec leur fraîcheur (`max_age`), le workflow lit un état ; jamais d'attente réseau dans la boucle du moteur. Une étape `fetch` explicite, si elle existe un jour, a une échéance et une sortie « délai dépassé » |
| Variables et calculs | Absents | Interprété depuis le JSON : 743 à 754 ns ; compilé : 28 à 32 ns (I3) | Langage d'expressions typé, non Turing-complet, compilé à la validation ; variables d'exécution bornées |

### 3.6 Le moteur cible

```text
validation humaine ──► compilation ──► plan exécutable
                         │   points numérotés, constantes typées, expressions compilées,
                         │   bornes vérifiées (nœuds, profondeur, boucles, sous-workflows)
                         ▼
hub ──(événements des points indexés seulement)──► file du moteur ──► index par point
                                                                   ├─► agrégats au fil de l'eau
horloge : prochaine échéance (tas), pas de balayage à 1 Hz ────────┤
                                                                   ▼
                                     exécutions : machines à états ; curseur et baux
                                     persistés aux points d'attente (wait, wait_for, bail)
```

1. **Compiler à la validation.** La validation humaine porte déjà sur une empreinte exacte ;
   la compilation produit au même moment un plan exécutable, sans relire le JSON ensuite.
2. **Filtrer à la source.** Le hub ne transmet au moteur que les événements des points
   indexés (déclencheurs, conditions en attente). Une rafale de capteurs qu'aucun workflow
   ne regarde n'entre plus dans sa file.
3. **Index et horloge par échéance.** Un événement ne réveille que ses déclencheurs ;
   l'heure ne réveille le moteur qu'à la prochaine échéance due (aujourd'hui : chaque
   seconde, avec un recalcul du soleil par déclencheur, `engine.rs:499-536`).
4. **Exécution durable.** Une exécution est une machine à états ; son curseur et ses baux
   sont persistés aux points d'attente. Un redémarrage ou une bascule la reprend.
5. **Workflows façon n8n.** Sous-workflows appelés par identifiant (profondeur bornée),
   boucles sur des listes bornées, variables typées, branches d'erreur et de délai.
   Toujours déterministe, toujours sous la garde, jamais de LLM dans l'exécution.
6. **Garde-fou d'emballement.** Remplacer l'extinction au-delà de 20 lancements par minute
   par un anti-rebond et une coalescence par déclencheur ; l'extinction reste le dernier
   recours, annoncé.

Options étudiées :

| Option | Effet | Coût | Choix |
|---|---|---|---|
| A. Garder le moteur, corriger la perte sur retard, indexer les déclencheurs d'état, rendre `wait` durable | Supprime les deux défauts graves | Faible à moyen | **d'abord** |
| B. Moteur compilé, filtré à la source, exécution durable | Tient 10 000 automatismes, workflows complexes | Moyen à élevé | **ensuite, par étapes** |
| C. Moteur de workflows généraliste ou script embarqué (Lua, Rhai, WASM) | Flexible | Déterminisme, garde et relecture humaine plus difficiles ; mémoire | non |

## 4. Tampons et cloisons

### 4.1 Ce qu'un tampon résout, et ce qu'il ne résout pas

- **Un tampon (une file) absorbe une différence de vitesse :** un appareil lent, une rafale.
  Il doit être borné, avoir une règle quand il est plein (refuser, coalescer, remplacer) et
  une échéance (un ordre trop vieux ne part pas).
- **Un tampon ne libère pas un thread occupé :** si un pilote bloque son worker, la file
  derrière lui n'avance plus. Contre cela, il faut une **cloison** : un thread à part.
- La V1 a montré que deux pilotes bloquants gèlent la maison (E2). Les mesures ci-dessous
  disent quelle cloison ou quelle file résout quel cas.

### 4.2 « Tout passe par le moteur » : vérification

C'est exact. Chaque valeur passe par `ctx.set_state` (verrou du modèle puis bus), chaque
ordre par `hub.command` (garde, échéance, journal), et le moteur d'automatismes est un
abonné du bus. Mettre un pilote sur son propre thread ne coupe rien : le hub est une mémoire
partagée et ses canaux Tokio fonctionnent d'un runtime à l'autre.

Preuve (K1) : capteur → vrai moteur → lampe. Le capteur et la lampe tournent sur leurs propres
threads, deux pilotes bloquants aussi, et le moteur reste sur le runtime du cœur.

| Configuration | Médiane | Pire |
|---|---|---|
| Pilotes partagés sur les 2 workers | 5,5 à 12,1 ms | 202 à 204 ms |
| Pilotes isolés, un thread chacun | 0,2 ms | 0,4 à 1,9 ms |

Deux précautions découlent de cette convergence :

1. **Les tâches propres au hub doivent tourner sur le runtime du cœur.** Aujourd'hui elles
   partent par `tokio::spawn`, donc sur le runtime de **l'appelant**. Le pilote des aides
   (`moli-helpers/src/lib.rs:605-632`) démarre ainsi les horloges globales des calculs
   (`computed_clock.rs:16`), des dépendances (`dependency_hub.rs:456`) et des durées
   (`duration_hub.rs:279`). Avec un thread par pilote, ces horloges vivraient sur le thread
   du pilote des aides : s'il bloque, tous les calculs de la maison s'arrêtent. Le hub doit
   recevoir à sa construction le `Handle` du runtime du cœur et y lancer ses tâches.
2. **Le hub doit se protéger des inondations.** Un pilote isolé qui publie 100 000 valeurs
   par seconde ne bloque plus personne par son thread, mais il prend le verrou du modèle et
   remplit le bus. Il faut un budget d'événements par pilote, et une coalescence par point
   au-delà de ce budget (la dernière valeur gagne).

### 4.3 Un thread par pilote (C1)

| Situation | Ordres servis en 2 s | Médiane | Pire | Pire retard d'une minuterie de 10 ms |
|---|---|---|---|---|
| 2 pilotes bloquants partagés | 9 | 1,6 à 6,7 ms | ~200 ms | ~195 ms |
| 6 pilotes bloquants isolés | 87 à 93 | ≤ 0,1 ms | 0,2 à 48 ms | 3 à 44 ms |

Coût mesuré : **51 à 52 Kio de RSS par pilote isolé** au repos (40 pilotes : ~2 Mio). La pile de
2 Mio est réservée mais seules les pages touchées comptent ; chaque runtime a sa propre roue
de minuteries et son descripteur epoll. Pour un Pi Zero 2 W, c'est acceptable pour tous les
pilotes, et indispensable pour ceux qui décodent des données hostiles ou font du calcul
(XML, audio).

### 4.4 Une voie par appareil derrière un pont (C2)

Un pont, 20 lampes, une lampe muette (chaque requête vers elle attend 4 s, le délai de
`moli-net`), 25 ordres par seconde pendant 10 s :

| Stratégie | Ordres aux lampes saines réussis | Médiane | Pire | Lampe muette |
|---|---|---|---|---|
| Boucle unique | **37 à 42 sur 237 (16 à 18 %)** | ~4,0 s | 4,6 s | 4,0 s, ou refus immédiat quand la file est pleine |
| Voie par appareil | 237 sur 237 | 32 ms | 33 à 37 ms | 5,0 s (délai de l'ordre), file bornée |
| Voies + disjoncteur | 237 sur 237 | 32 ms | 33 à 52 ms | refus en moins d'1 ms une fois le disjoncteur ouvert (après deux délais de 4 s), appareil hors ligne |

La boucle unique est le modèle le plus courant dans le dépôt (V1, section 5.2) : une lampe
muette fait échouer plus de huit ordres sur dix vers les lampes **saines**. Une voie par
appareil (file bornée à 8, un ordre en vol, échéance revérifiée avant l'envoi) les isole
complètement.

### 4.5 Une API partagée qui ne traite que 4 requêtes à la fois (C3)

C'est le cas « une seule API » : Hue, Zigbee2MQTT, TaHoma, ou un service cloud.

| Stratégie | Ordres aux lampes saines réussis | Médiane | Pire | Lampe muette |
|---|---|---|---|---|
| Tâche par ordre, sans plafond par appareil | 237 sur 237 | **1,43 à 1,45 s** | 3,7 à 4,0 s | 5,0 s |
| Voie par appareil (une requête en vol) | 237 sur 237 | 32 ms | 34 à 51 ms | 5,0 s |
| Voies + disjoncteur | 237 sur 237 | 32 ms | 35 à 52 ms | refus en moins d'1 ms une fois ouvert |

Lancer une tâche par ordre sans plafond par appareil semble plus « parallèle », mais les
requêtes vers la lampe muette occupent les 4 places de l'API et les lampes saines attendent
1,4 s en médiane. Une voie par appareil limite chaque appareil à **une** requête en vol :
la lampe muette ne prend jamais plus d'une place.

### 4.6 API cloud : quotas, pannes, jetons

| Règle | Pourquoi | Déjà dans le dépôt |
|---|---|---|
| Budget de requêtes (seau à jetons) par fournisseur | Ne jamais dépasser un quota, même en rafale | — |
| Recul exponentiel avec gigue sur 429 et 5xx, `Retry-After` respecté | Une panne du service ne devient pas une tempête | Ariston 15 min, Voltalis 10 min, Netatmo borné |
| Compteur de refus de session | Éviter un bannissement du compte | Ariston (`MAX_SESSION_REFUSALS`) ; manque à Landroid |
| Un seul rafraîchissement de jeton en vol | Les jetons OAuth tournants s'invalident entre eux | Netatmo n'active que des jetons persistés |
| Cache des lectures avec durée de vie, valeur périmée servie pendant la relecture, cache négatif | Une panne n'empile pas les appels (défaut constaté sur la météo, V1 5.5) | — |
| Disjoncteur par fournisseur | Échouer vite, signaler, sonder plus tard | — |

### 4.7 Une boîte à outils commune dans `moli-runtime`

Plutôt que chaque pilote réinvente ces mécanismes, `moli-runtime` peut fournir :

| Outil | Rôle |
|---|---|
| `Voies` | Une file bornée par appareil, un ordre en vol, échéance revérifiée, coalescence des consignes (« la dernière gagne », l'ancienne répond « remplacée »), disjoncteur par appareil qui vide la file et passe l'appareil hors ligne |
| `Passerelle` | Concurrence plafonnée d'une API partagée, équité entre appareils (tourniquet), priorité des ordres sur le sondage, débit plafonné, disjoncteur global si le pont lui-même tombe |
| `spawn_driver_isole` | Le superviseur actuel, sur un thread et un runtime dédiés au pilote |
| Budget d'entrée du hub | Plafond d'événements par pilote et coalescence par point au-delà |
| `Handle` du cœur | Les tâches internes du hub ne dépendent plus du thread de l'appelant |

Les pilotes Samsung, Twinkly, Ariston et Android TV contiennent déjà des versions locales de
ces idées (V1, section 2.7). Les généraliser corrige d'un coup les pilotes à boucle unique
relevés en V1.

## 5. Mémoire : peut-on se lâcher ?

### 5.1 Mesures

Allocateur qui compte chaque octet, forme réelle des deux instances réunies.

| Mesure | Résultat |
|---|---|
| Types du cœur | `Value` 24 o, `Sample` 32 o, `Arc<str>` 16 o |
| Modèle du hub : 496 appareils, 2 264 valeurs | **1,00 Mio** (~464 o par valeur, définitions comprises) |
| Même modèle ×10 (22 640 valeurs) | 9,86 Mio : coût linéaire |
| Ajouter « dernier changement » et « valeur précédente » à chaque valeur | +32 o par valeur, soit ~72 Kio (calcul sur les tailles mesurées) |
| 24 h d'historique (161 000 relevés) | naïf 5,48 Mio ; compact **1,37 Mio** ; mixte 2,28 Mio |
| 7 jours d'historique (1,13 million de relevés) | naïf 40,7 Mio ; compact **8,78 Mio** ; mixte 15,1 Mio |
| Mémoire du moteur d'automatismes actuel | ~9,4 Kio par automatisme (3,7 Mio pour 400) |

« Compact » : décalage en secondes sur 4 octets et valeur sur 4 octets, en colonnes par série.
« Naïf » : un `(u64, Value)` de 32 octets par relevé dans une `VecDeque`. Le format compte
plus que la quantité : ×4,6. Les seaux par minute n'aident pas ici, parce que les séries les
plus actives changent moins d'une fois par seconde.

### 5.2 Deux profils

| | Profil sobre (Pi Zero 2 W, 512 Mo) | Profil généreux (2 Go et plus) |
|---|---|---|
| Valeurs courantes, dernier changement, valeur précédente | toutes | toutes |
| Agrégats au fil de l'eau | ceux déclarés par les workflows | ceux des workflows et du tableau de bord |
| Historique en mémoire, format compact | 24 h des points lus par un workflow ou un écran | 7 jours de tout |
| Instantané pour la relance | toutes les 10 s | chaque seconde |
| Surcoût mesuré à l'échelle des deux maisons | ~3 Mio | ~12 à 15 Mio |
| Budget de référence | 32 Mio de mémoire anonyme (PERFORMANCE.md) | 128 Mio, plafond actuel du conteneur |

Dans les deux profils, SQLite (puis Turso, D13) reste la mémoire durable et le long terme, et
l'archive reste le long terme hors de la machine. La mémoire sert la réactivité ; elle ne
remplace pas la durabilité. À titre de comparaison, Home Assistant occupe de l'ordre de
2 Gio sur la même maison ([PERFORMANCE.md](../PERFORMANCE.md)).

## 6. Relancer vite sur une autre machine

### 6.1 Ce qui est déjà durable, et ce qui ne l'est pas

| État | Aujourd'hui | Pour la relance |
|---|---|---|
| Configuration, profils, automatismes, étiquettes, plan, personnes | fichiers | répliquer à chaque écriture |
| Coffre des secrets | `secrets.enc` chiffré ; clé maître hors disque | répliquer le fichier ; la clé vient du même gestionnaire de secrets |
| États métier (minuteurs, compteurs, plannings, durées, occurrences consommées) | fichiers bornés, fsync, renommage atomique | répliquer à chaque écriture |
| Valeurs courantes | cache `state.json` toutes les 60 s | instantané fréquent, valeurs marquées « reprises, non confirmées » |
| Journal d'audit | `journal.jsonl` en ajout | flux répliqué |
| Historique, énergie | SQLite ; long terme vers l'archive (D13) | l'archive couvre le long terme ; la fenêtre récente se reconstruit ou accepte un trou |
| Exécutions en cours, baux | **en mémoire, perdues** | à rendre durables (3.4) |
| Demandes d'approbation (10 min) | en mémoire | perte acceptable, ou répliquées |

### 6.2 Le mécanisme proposé

1. **Instantané et journal.** La machine principale pousse à la machine de secours un
   instantané de l'état d'exécution (20 à 54 Kio compressés selon ce qu'il contient, écrit et
   synchronisé en 7,5 à 10 ms) toutes les quelques secondes, plus les fichiers métier à chaque écriture et le
   journal des changements (1,3 Mio par jour compressé, 9 octets par changement, R2). Même
   contrat que l'archive : HTTP borné, aucun disque partagé.
2. **Secours passif.** La machine de secours tourne, à jour, mais **ne se connecte à aucun
   appareil ni service** : pas de MQTT avec le même identifiant client, pas de session Tuya
   locale (une seule par appareil), pas de rafraîchissement de jeton OAuth (les jetons
   tournants s'invalideraient entre eux), pas de connexion cloud qui ferait bannir le compte.
3. **Bascule.** Un humain ou le chien de garde de l'hôte décide. La machine de secours
   incrémente une **époque**, l'écrit, active ses pilotes, reprend les baux et les
   exécutions, et se fait trouver sous le même nom (alias mDNS ou DNS, tunnel).
4. **Jamais deux maîtres.** L'ancienne machine, si elle revient, voit une époque plus grande
   et reste passive. Une bascule par chien de garde doit d'abord arrêter l'ancienne
   (conteneur ou alimentation) avant d'activer la nouvelle.
5. **Valeurs reprises.** Elles gardent leur horodatage d'origine et ne sont pas des
   changements : le moteur ne déclenche rien dessus (déjà le cas au démarrage). Les pilotes
   les confirment en quelques secondes.

### 6.3 Délai de reprise et perte

| Étape | Mesuré ou estimé |
|---|---|
| Lire l'instantané et reconstruire le hub (496 appareils, 2 264 valeurs) | **11 à 14 ms** mesurées (R1) |
| Démarrage jusqu'à « sain » | ≤ 2 s (budget actuel) |
| Reconnexion des pilotes | quelques secondes en local ; plus pour les services cloud (connexion, quotas) — estimé |
| Perte de valeurs | l'intervalle de l'instantané (1 à 10 s selon le profil) ; les pilotes reconfirment tout |
| Perte d'état métier | nulle si la réplication précède l'acquittement ; sinon l'intervalle de poussée |

Une reprise en moins de 30 s après la décision de bascule est réaliste (estimé). Le délai
dépend surtout de la décision (humaine ou chien de garde) et des reconnexions aux appareils.

### 6.4 Pièges par intégration

| Intégration | Piège | Règle |
|---|---|---|
| MQTT (Zigbee2MQTT, Frigate, Bambu) | même identifiant client : les deux se déconnectent mutuellement | secours passif, aucune connexion |
| Tuya local, pont Nuki | une session ou une requête à la fois | idem |
| OAuth (Netatmo et autres) | jetons tournants | ne rafraîchir que sur la machine maîtresse ; jetons repris du coffre répliqué |
| Appairages (Hue, Android TV, Cast) | clés et certificats | dans le coffre : portables avec lui |
| Réseau (mDNS, SSDP, Wake-on-LAN) | dépend du réseau local | machine de secours sur le même réseau |
| Téléphones, tableaux de bord, tunnel | pointent une adresse | un nom stable plutôt qu'une adresse |

## 7. Plan proposé

Chaque piste est notée en impact (risque de panne ou de latence évité) et en effort (taille du
changement, risque de régression).

| | Effort faible | Effort moyen |
|---|---|---|
| **Impact fort** | correction de la perte sur retard du bus (3.3) · tâches du hub sur le `Handle` du cœur (4.2) · bail durable pour « pendant N minutes » (3.4) · index des déclencheurs d'état (3.6, option A) · budget d'entrée du hub (4.2) | voies par appareil et passerelle dans `moli-runtime` (4.7) · un thread par pilote (4.3) · exécution durable des workflows (3.6) · réplication et secours passif (6.2) |
| **Impact modéré** | horloge par échéance au lieu de 1 Hz (3.6) · anti-rebond à la place de l'extinction à 20/min (3.6) · règles cloud (4.6) · profil mémoire sobre (5.2) | moteur compilé et filtré à la source (3.6, option B) · langage d'expressions et sous-workflows (3.6) · profil généreux (5.2) · bascule par chien de garde avec époque (6.2) |

Les recommandations de la V1 restent valables (sentinelle des briques, détecteur de gel,
XML hors des workers, démarrage découplé…). Plusieurs se recoupent : un thread par pilote
traite à la fois le gel par blocage (V1 3.1) et les décodeurs coûteux (V1 E8).

Chaque lot se vérifie par un banc rejouable de l'annexe A :

| Lot | Preuve d'acceptation |
|---|---|
| Perte sur retard | W2 rejoué : les déclencheurs injectés après une rafale de 5 000 événements partent tous |
| Bail durable | « allume 30 min », redémarrage au milieu : la lumière s'éteint à l'échéance, une seule fois ; changée à la main entre-temps, elle n'est pas éteinte |
| Index et filtrage à la source | W2 rejoué à 400 automatismes : retard d'une action derrière 5 000 événements de bruit inférieur à 5 ms |
| Thread par pilote | C1 rejoué dans `moli-os` : six pilotes bloquants, `/api/health` répond en moins de 5 ms |
| Voies et passerelle | C2 et C3 rejoués sur Hue et Tuya simulés : 100 % des ordres sains réussis, appareil muet refusé vite et hors ligne |
| Relance ailleurs | arrêt brutal de la machine principale : la secours reprend après décision, sans double maître, baux compris |

## 8. Limites

- Bancs synthétiques sur Apple M4 : les rapports entre options sont fiables, les durées
  absolues ne le sont pas pour le Pi ; aucune mesure sur de vrais appareils.
- Les comptes des deux instances viennent d'une capture du 17 octobre 2026 ; les attributs
  de `states.json` sont expurgés (au plus 7 clés par entité), d'où l'usage des tailles du
  Recorder.
- Le banc I1 compare des algorithmes, pas deux moteurs complets ; le banc W, lui, mesure le
  vrai moteur.
- La bascule n'a pas été jouée de bout en bout ; seules ses briques (instantané, relecture,
  journal) ont été mesurées.

## Annexe A — Bancs d'essai

Une crate jetable **hors du dépôt**, qui dépend par chemin de `moli-runtime`, `moli-core` et
`moli-automation`, avec le `Cargo.lock` du dépôt. Cinq binaires : `moteur` (W), `index` (I),
`cloisons` (C), `convergence` (K1), `memoire` (M), `relance` (R).

```toml
[package]
name = "preuves-v2"
version = "0.1.0"
edition = "2024"
publish = false

[workspace]

[dependencies]
moli-runtime = { path = "<dépôt>/crates/moli-runtime" }
moli-core = { path = "<dépôt>/crates/moli-core" }
moli-automation = { path = "<dépôt>/crates/moli-automation" }
anyhow = "1"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync", "time"] }
tokio-util = "0.7"
serde_json = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", default-features = false, features = ["fmt", "registry", "std"] }
flate2 = "1"
rusqlite = { version = "0.40", default-features = false, features = ["bundled"] }

[profile.release]
opt-level = "s"
```

Reproduire : copier le `Cargo.lock` du dépôt, puis `cargo run --release --bin <nom>`
(`moteur` accepte une liste de tailles, par exemple `-- 50 400`).

Sorties brutes :

```text
# Le vrai moteur d'automatismes sous charge (2 workers, machine de test)
Chaque automatisme : 1 déclencheur, 3 conditions imbriquées de 3 règles, 1 action ;
10 % sont temporaires (« allume 30 min »), 10 % attendent une condition (wait_for).
N =  50 : chargement 0.29 s, mémoire du moteur 698 Kio ; croisière p50 0.4 ms p99 2.2 ms (20 actions)
        rafale  2000 : sondes au milieu p50 6.4 ms max 6.9 ms perdues 0 ; sonde après p50 11.5 ms ; retards du moteur 0 (0 événements manqués)
        rafale  5000 : sondes au milieu p50 12.8 ms max 13.5 ms perdues 0 ; sonde après p50 23.3 ms ; retards du moteur 0 (0 événements manqués)
N = 100 : chargement 1.01 s, mémoire du moteur 1154 Kio ; croisière p50 0.4 ms p99 1.3 ms (19 actions)
        rafale  2000 : sondes au milieu p50 10.5 ms max 10.6 ms perdues 0 ; sonde après p50 18.6 ms ; retards du moteur 0 (0 événements manqués)
        rafale  5000 : sondes au milieu p50 40.3 ms max 40.7 ms perdues 0 ; sonde après p50 — ; retards du moteur 3 (1441 événements manqués)
N = 200 : chargement 5.38 s, mémoire du moteur 2066 Kio ; croisière p50 0.5 ms p99 0.9 ms (19 actions)
        rafale  2000 : sondes au milieu p50 21.9 ms max 22.5 ms perdues 0 ; sonde après p50 33.7 ms ; retards du moteur 0 (0 événements manqués)
        rafale  5000 : sondes au milieu p50 36.1 ms max 36.2 ms perdues 0 ; sonde après p50 70.0 ms ; retards du moteur 0 (0 événements manqués)
N = 400 : chargement 17.55 s, mémoire du moteur 3766 Kio ; croisière p50 0.5 ms p99 2.9 ms (19 actions)
        rafale  2000 : sondes au milieu p50 36.6 ms max 38.3 ms perdues 0 ; sonde après p50 59.7 ms ; retards du moteur 0 (0 événements manqués)
        rafale  5000 : sondes au milieu p50 80.1 ms max 161.0 ms perdues 0 ; sonde après p50 — ; retards du moteur 2 (1483 événements manqués)

# Workflows complexes : balayage d'aujourd'hui contre moteur compilé (machine de test)

## I1 — trouver les workflows à lancer et évaluer leurs 9 règles
200 000 événements (20 000 pour N = 10 000), dont 10 % sur un déclencheur ; temps moyen par événement.
- N =    100 : balayage       908 ns/événement ; compilé     3 ns/événement ; ×282 ; 20000 lancements
- N =    400 : balayage      1788 ns/événement ; compilé     3 ns/événement ; ×664 ; 20000 lancements
- N =   1000 : balayage      4222 ns/événement ; compilé     3 ns/événement ; ×1263 ; 20000 lancements
- N =  10000 : balayage     47574 ns/événement ; compilé    10 ns/événement ; ×4825 ; 2000 lancements

## I2 — lire « moyenne sur 1 h » d'un capteur qui change chaque seconde
- SQLite (index (point, ts), 3 600 lignes lues) : 192 µs par lecture, plus le passage par un thread bloquant
- balayage d'une fenêtre en mémoire : 3.7 µs par lecture
- agrégat tenu au fil de l'eau : 5 ns par relevé reçu, 1.0 ns par lecture

## I3 — variables et calculs : « (moyenne 1 h du salon − 2) > 19 et (présence ou 7 h ≤ heure < 22 h) »
- interprété depuis le JSON : 743 ns ; compilé une fois à la validation : 28 ns (×27)

## I4 — « allume 30 minutes » en bail durable : 10 000 baux
- ordonnancement de 10 000 échéances (tas binaire) et extraction des 5000 dues : 0.31 ms au total
- écrire un bail de façon durable (ajout + fsync) : 3.4 ms, une seule fois au lancement ; rien pendant l'attente

# Cloisons : un matériel muet ou un pilote bloquant ne doit pas arrêter le reste

## C1 — un thread par pilote contre les deux workers partagés
- 2 pilotes bloquants partagés :   9 ordres en 2 s, médiane   2.7 ms, pire 200.2 ms ; pire retard d'une minuterie de 10 ms  195.0 ms
- 6 pilotes bloquants isolés   :  87 ordres en 2 s, médiane   0.1 ms, pire  47.7 ms ; pire retard d'une minuterie de 10 ms   43.6 ms
- coût : 40 pilotes isolés au repos ajoutent 2048 Kio de RSS, soit 51 Kio par pilote (pile de 2 Mio réservée, seules les pages touchées comptent)

## C2 — un pont, 20 lampes, une lampe muette (API du pont sans limite de concurrence)
- boucle unique       : lampes saines 37/237 réussis, médiane 3992.8 ms, pire 4622.8 ms ; lampe muette : médiane 4002.2 ms, réponse la plus rapide   0.0 ms
- voie par appareil   : lampes saines 237/237 réussis, médiane  32.0 ms, pire   33.6 ms ; lampe muette : médiane 5001.7 ms, réponse la plus rapide 4002.1 ms
- voies + disjoncteur : lampes saines 237/237 réussis, médiane  32.0 ms, pire   42.1 ms ; lampe muette : médiane 2972.5 ms, réponse la plus rapide   0.0 ms

## C3 — le même pont, mais son API ne traite que 4 requêtes à la fois
- tâche par ordre     : lampes saines 237/237 réussis, médiane 1453.1 ms, pire 3991.4 ms ; lampe muette : médiane 5001.2 ms, réponse la plus rapide 4001.6 ms
- voie par appareil   : lampes saines 237/237 réussis, médiane  32.0 ms, pire   51.2 ms ; lampe muette : médiane 5001.8 ms, réponse la plus rapide 4002.2 ms
- voies + disjoncteur : lampes saines 237/237 réussis, médiane  32.0 ms, pire   52.1 ms ; lampe muette : médiane 2854.3 ms, réponse la plus rapide   0.0 ms

# Le moteur voit tout, même quand les pilotes vivent sur leurs propres threads
Capteur → moteur d'automatismes réel → lampe, avec deux pilotes qui bloquent 200 ms en boucle.
- pilotes partagés (2 workers)     : 20 actions sur 40 événements, médiane 12.1 ms, pire 204.2 ms ; capteur sur « coeur », lampe sur « coeur », moteur sur « coeur »
- pilotes isolés (un thread chacun) : 20 actions sur 40 événements, médiane 0.2 ms, pire 1.9 ms ; capteur sur « pilote-capteurs », lampe sur « pilote-lampes », moteur sur « coeur »

# Mémoire : l'état utile d'une maison en Rust

## M1 — tailles des types du cœur
- Value 24 o, Sample 32 o, (u64, Value) 32 o, Arc<str> 16 o

## M2 — modèle du hub à l'échelle des deux Home Assistant réunis
- ×1 : 496 appareils, 2264 valeurs → 1.00 Mio pour le modèle du hub (≈ 464 o par valeur, définitions comprises)
- ×10 : 4960 appareils, 22640 valeurs → 9.86 Mio pour le modèle du hub (≈ 457 o par valeur, définitions comprises)

## M3 — garder l'historique récent en mémoire (taux réels, 1 487 séries)
- fenêtre 1 j : 161398 relevés → naïf 5.48 Mio ; compact 1.37 Mio ; mixte (seaux/minute pour les séries > 1/min) 2.28 Mio
- fenêtre 7 j : 1129783 relevés → naïf 40.71 Mio ; compact 8.78 Mio ; mixte (seaux/minute pour les séries > 1/min) 15.14 Mio

## M4 — instantané pour une relance ailleurs
- instantané d'exécution (2 264 valeurs + 400 automatismes) : JSON 877 Kio en 9.7 ms, gzip 54 Kio en 1.0 ms, écriture + fsync 7.5 ms, relecture 10.5 ms (400 automatismes relus)

# Relance ailleurs depuis l'état
- R1 : instantané de 20 Kio compressé ; décompression, lecture et reconstruction du hub (496 appareils, 2 264 valeurs) : 14 ms
- R2 : journal d'une journée (163387 changements) : 3.0 Mio brut, 1.3 Mio compressé, produit en 46 ms ; soit 9 octets par changement
```

Codes sources :

Code complet de `src/bin/moteur.rs` :

```rust
//! W1-W3 : le vrai moteur `moli-automation` sous des workflows réalistes, jusqu'à son
//! plafond (400 automatismes). Combien coûte un événement, combien attend une action
//! derrière une rafale, et un déclencheur peut-il être perdu ?

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::HashMap;
use std::sync::atomic::{AtomicIsize, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use moli_automation::{Actor, Author, Automation, Automations, Config, Graph, Mode};
use moli_core::{Access, Device, DeviceId, Kind, PointId, PointSpec, Semantic, Value};
use moli_runtime::{BoxFuture, Driver, DriverCtx, Hub, HubOptions, spawn_driver};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::layer::SubscriberExt;

struct Compte;
static OCTETS: AtomicIsize = AtomicIsize::new(0);
unsafe impl GlobalAlloc for Compte {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            OCTETS.fetch_add(l.size() as isize, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) };
        OCTETS.fetch_sub(l.size() as isize, Ordering::Relaxed);
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

/// Compte les avertissements « automations lagged » du moteur et les événements manqués.
struct Retards {
    fois: Arc<AtomicUsize>,
    manques: Arc<AtomicU64>,
}
impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Retards {
    fn on_event(&self, e: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        if *e.metadata().level() != tracing::Level::WARN {
            return;
        }
        struct V(bool, u64);
        impl tracing::field::Visit for V {
            fn record_debug(&mut self, f: &tracing::field::Field, v: &dyn std::fmt::Debug) {
                if f.name() == "message" && format!("{v:?}").contains("automations lagged") {
                    self.0 = true;
                }
            }
            fn record_u64(&mut self, f: &tracing::field::Field, v: u64) {
                if f.name() == "missed" {
                    self.1 = v;
                }
            }
        }
        let mut v = V(false, 0);
        e.record(&mut v);
        if v.0 {
            self.fois.fetch_add(1, Ordering::Relaxed);
            self.manques.fetch_add(v.1, Ordering::Relaxed);
        }
    }
}

const CAPTEURS: usize = 1_800; // 360 appareils × 5 valeurs
const LAMPES: usize = 400; // une lampe par automatisme
const CHAUDS: usize = 10; // capteurs partagés par 20 % des automatismes
const BRUIT: std::ops::Range<usize> = 1_000..1_700; // capteurs qui ne déclenchent rien

fn capteur(s: usize) -> (DeviceId, String) {
    (DeviceId::from(format!("maison:c{}", s / 5)), format!("v{}", s % 5))
}
fn lampe(k: usize) -> PointId {
    PointId::from(format!("maison:l{k}/on"))
}

struct Banc {
    arrivees: Mutex<Vec<(usize, Instant)>>,
}

struct Maison {
    banc: Arc<Banc>,
    injections: Mutex<Option<mpsc::UnboundedReceiver<(usize, f64)>>>,
}

impl Driver for Maison {
    fn kind(&self) -> &'static str {
        "maison"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let instance = ctx.instance().clone();
            let numerique = |key: String| PointSpec {
                key: key.clone().into(),
                label: key.into(),
                kind: Kind::Numeric { min: None, max: None, step: None },
                access: Access { read: true, write: false },
                unit: None,
                semantic: Semantic::Other,
            };
            for d in 0..CAPTEURS / 5 {
                let id = DeviceId::from(format!("maison:c{d}"));
                ctx.upsert_device(Device {
                    id: id.clone(),
                    instance: instance.clone(),
                    native_name: format!("capteur {d}").into(),
                    manufacturer: None,
                    model: None,
                    description: None,
                    native_room: None,
                    members: vec![],
                    points: (0..5).map(|k| numerique(format!("v{k}"))).collect(),
                });
                for k in 0..5 {
                    ctx.set_state(&id, &format!("v{k}"), Value::Float(0.0));
                }
            }
            for k in 0..LAMPES {
                let id = DeviceId::from(format!("maison:l{k}"));
                ctx.upsert_device(Device {
                    id: id.clone(),
                    instance: instance.clone(),
                    native_name: format!("lampe {k}").into(),
                    manufacturer: None,
                    model: None,
                    description: None,
                    native_room: None,
                    members: vec![],
                    points: vec![PointSpec {
                        key: "on".into(),
                        label: "on".into(),
                        kind: Kind::Binary,
                        access: Access { read: true, write: true },
                        unit: None,
                        semantic: Semantic::OnOff,
                    }],
                });
                ctx.set_state(&id, "on", Value::Bool(false));
            }
            let mut injections = self.injections.lock().unwrap().take().expect("une seule exécution");
            ctx.ready();
            loop {
                tokio::select! {
                    cmd = ctx.next_command() => {
                        let Some(cmd) = cmd else { return Ok(()) };
                        let k: usize = cmd.device.id.as_str()["maison:l".len()..].parse().unwrap();
                        self.banc.arrivees.lock().unwrap().push((k, Instant::now()));
                        ctx.set_state(&cmd.device.id, &cmd.key, cmd.value.clone());
                        cmd.reply(Ok(()));
                    }
                    inj = injections.recv() => {
                        let Some((s, v)) = inj else { return Ok(()) };
                        let (id, key) = capteur(s);
                        ctx.set_state(&id, &key, Value::Float(v));
                    }
                }
            }
        })
    }
}

/// Le déclencheur de l'automatisme k, et s'il passe ses conditions.
fn declencheur(k: usize, n: usize) -> usize {
    if k < n / 5 { k % CHAUDS } else { 100 + k }
}
fn passe(k: usize) -> bool {
    k % 2 == 0
}

fn graphe(k: usize, n: usize) -> serde_json::Value {
    let t = declencheur(k, n);
    let (td, tk) = capteur(t);
    let regle = |j: usize| {
        let (d, key) = capteur(BRUIT.start + (k * 7 + j * 13) % 690);
        if passe(k) {
            serde_json::json!({"kind":"state","point":format!("{d}/{key}"),"op":"ge","value":0})
        } else {
            serde_json::json!({"kind":"state","point":format!("{d}/{key}"),"op":"gt","value":1e9})
        }
    };
    let mut nodes = vec![
        serde_json::json!({"id":"t","type":"when_state","point":format!("{td}/{tk}")}),
        serde_json::json!({"id":"c1","type":"if","rules":[regle(0),regle(1),regle(2)]}),
        serde_json::json!({"id":"c2","type":"if","rules":[regle(3),regle(4),regle(5)]}),
        serde_json::json!({"id":"c3","type":"if","rules":[regle(6),regle(7),regle(8)]}),
        serde_json::json!({"id":"a","type":"toggle","point":lampe(k).as_str()}),
    ];
    let mut edges = vec![
        serde_json::json!({"from":"t","to":"c1"}),
        serde_json::json!({"from":"c1","port":"yes","to":"c2"}),
        serde_json::json!({"from":"c2","port":"yes","to":"c3"}),
        serde_json::json!({"from":"c3","port":"yes","to":"a"}),
    ];
    match k % 10 {
        // Workflow temporaire : « allume 30 minutes ».
        4 => {
            nodes.push(serde_json::json!({"id":"w","type":"wait","seconds":1800}));
            nodes.push(serde_json::json!({"id":"b","type":"toggle","point":lampe(k).as_str()}));
            edges.push(serde_json::json!({"from":"a","to":"w"}));
            edges.push(serde_json::json!({"from":"w","to":"b"}));
        }
        // Attente d'une condition qui ne vient pas (réévaluée à chaque événement).
        6 => {
            let (d, key) = capteur(1_799);
            nodes.push(serde_json::json!({"id":"wf","type":"wait_for","timeout_s":600,
                "rule":{"kind":"state","point":format!("{d}/{key}"),"op":"eq","value":12345}}));
            edges.push(serde_json::json!({"from":"a","to":"wf"}));
        }
        _ => {}
    }
    serde_json::json!({"nodes": nodes, "edges": edges})
}

fn automatisme(k: usize, n: usize) -> Automation {
    Automation {
        id: String::new(),
        name: format!("Automatisme {k}"),
        enabled: true,
        mode: Mode::Restart,
        graph: serde_json::from_value::<Graph>(graphe(k, n)).unwrap(),
        author: Author::Human,
        approved: None,
        approved_version: None,
        activation: None,
        note: None,
        created: 0,
        updated: 0,
    }
}

const HUMAIN: Actor = Actor { human: true, author: Author::Human };

struct Mesure {
    latences: Vec<Duration>,
    perdus: usize,
}

/// Latences des automatismes « passants » déclenchés par chaque sonde.
fn latences(banc: &Banc, sondes: &[(usize, Instant)], n: usize, depuis: usize) -> Mesure {
    let arrivees = banc.arrivees.lock().unwrap();
    let mut m = Mesure { latences: vec![], perdus: 0 };
    for &(s, t0) in sondes {
        for k in (0..n).filter(|&k| passe(k) && declencheur(k, n) == s) {
            match arrivees[depuis..].iter().find(|(a, t)| *a == k && *t >= t0) {
                Some((_, t)) => m.latences.push(*t - t0),
                None => m.perdus += 1,
            }
        }
    }
    m.latences.sort();
    m
}

fn ms(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1e3)
}
fn q(v: &[Duration], p: f64) -> String {
    if v.is_empty() { "—".into() } else { ms(v[((v.len() - 1) as f64 * p) as usize]) }
}

fn essai(n: usize, retards: &Arc<AtomicUsize>, manques: &Arc<AtomicU64>) {
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    rt.block_on(async {
        let hub = Hub::new(HubOptions::default()).unwrap();
        let cancel = CancellationToken::new();
        let banc = Arc::new(Banc { arrivees: Mutex::new(vec![]) });
        let (tx, rx) = mpsc::unbounded_channel();
        spawn_driver(&hub, "maison".into(), Arc::new(Maison { banc: banc.clone(), injections: Mutex::new(Some(rx)) }), cancel.clone());
        while hub.state(&lampe(LAMPES - 1)).is_none() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let (autos, _tache) = Automations::start(&hub, Config::default(), None, cancel.clone()).await.unwrap();
        let avant = OCTETS.load(Ordering::Relaxed);
        let t = Instant::now();
        for k in 0..n {
            let a = autos.save(automatisme(k, n), HUMAIN).await.unwrap();
            autos.approve(&a.id, &a.fingerprint(), HUMAIN).await.unwrap();
        }
        let charge = t.elapsed();
        let memoire = (OCTETS.load(Ordering::Relaxed) - avant) as f64;
        let mut valeur = 1.0;
        let mut injecte = |s: usize, sondes: &mut Vec<(usize, Instant)>, sonde: bool| {
            valeur += 1.0;
            if sonde {
                sondes.push((s, Instant::now()));
            }
            tx.send((s, valeur)).unwrap();
        };
        // Les workflows en attente (« 30 minutes », « attendre que ») démarrent.
        let mut amorces = vec![];
        for k in (0..n).filter(|k| passe(*k) && matches!(k % 10, 4 | 6)) {
            injecte(declencheur(k, n), &mut amorces, false);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        // W1 — régime de croisière : 10 déclencheurs par seconde pendant 5 s.
        let depuis = banc.arrivees.lock().unwrap().len();
        let mut sondes = vec![];
        for i in 0..50 {
            let k = (i * 37) % n;
            let s = if declencheur(k, n) < CHAUDS { 100 + (k | 1) } else { declencheur(k, n) };
            injecte(s, &mut sondes, true);
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        let croisiere = latences(&banc, &sondes, n, depuis);
        let mut lignes = vec![format!(
            "N = {n:>3} : chargement {:.2} s, mémoire du moteur {:.0} Kio ; croisière p50 {} p99 {} ({} actions)",
            charge.as_secs_f64(),
            memoire / 1024.0,
            q(&croisiere.latences, 0.5),
            q(&croisiere.latences, 0.99),
            croisiere.latences.len()
        )];
        // W2 — rafale de bruit (reconnexion d'un pont qui republie tout) avec des sondes
        // au milieu et une après.
        for rafale in [2_000usize, 5_000] {
            retards.store(0, Ordering::Relaxed);
            manques.store(0, Ordering::Relaxed);
            let depuis = banc.arrivees.lock().unwrap().len();
            let (mut milieu, mut apres, mut _rien) = (vec![], vec![], vec![]);
            let sonde_k = |i: usize| -> usize {
                // Un automatisme passant à déclencheur propre, jamais un capteur chaud.
                let k = (n / 5 + 2 * i) & !1;
                declencheur(k.min(n - 2) & !1, n)
            };
            for i in 0..rafale {
                if i == rafale / 2 {
                    for j in 0..5 {
                        injecte(sonde_k(j), &mut milieu, true);
                    }
                    injecte(0, &mut milieu, true); // un capteur chaud : plusieurs automatismes
                }
                injecte(BRUIT.start + (i * 7919) % (BRUIT.end - BRUIT.start), &mut _rien, false);
            }
            for j in 5..10 {
                injecte(sonde_k(j), &mut apres, true);
            }
            tokio::time::sleep(Duration::from_secs(4)).await;
            let m = latences(&banc, &milieu, n, depuis);
            let a = latences(&banc, &apres, n, depuis);
            lignes.push(format!(
                "        rafale {rafale:>5} : sondes au milieu p50 {} max {} perdues {} ; sonde après p50 {} ; retards du moteur {} ({} événements manqués)",
                q(&m.latences, 0.5),
                q(&m.latences, 1.0),
                m.perdus,
                q(&a.latences, 0.5),
                retards.load(Ordering::Relaxed),
                manques.load(Ordering::Relaxed)
            ));
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        for l in lignes {
            println!("{l}");
        }
        cancel.cancel();
        tokio::time::sleep(Duration::from_millis(200)).await;
    });
}

fn main() {
    let fois = Arc::new(AtomicUsize::new(0));
    let manques = Arc::new(AtomicU64::new(0));
    let abonne = tracing_subscriber::registry().with(Retards { fois: fois.clone(), manques: manques.clone() });
    tracing::subscriber::set_global_default(abonne).unwrap();
    println!("# Le vrai moteur d'automatismes sous charge (2 workers, machine de test)");
    println!("Chaque automatisme : 1 déclencheur, 3 conditions imbriquées de 3 règles, 1 action ;");
    println!("10 % sont temporaires (« allume 30 min »), 10 % attendent une condition (wait_for).");
    let n_list: Vec<usize> = std::env::args().skip(1).filter_map(|a| a.parse().ok()).collect();
    for n in if n_list.is_empty() { vec![50, 100, 200, 400] } else { n_list } {
        essai(n, &fois, &manques);
    }
}
```

Code complet de `src/bin/index.rs` :

```rust
//! I1-I4 : ce que coûte un workflow complexe selon la façon de l'exécuter.
//! Même charge, deux moteurs : un balayage à la manière d'aujourd'hui (chaînes, JSON
//! relu à chaque évaluation) et un moteur compilé (points numérotés, règles compilées,
//! index par point, agrégats tenus au fil de l'eau).

use std::collections::{BTreeMap, BinaryHeap, HashMap, VecDeque};
use std::hint::black_box;
use std::time::Instant;

use moli_core::Value;

#[derive(Clone, Copy)]
enum Op {
    Ge,
    Gt,
    Eq,
}

struct Regle {
    point: String,
    op: Op,
    valeur: serde_json::Value,
}

struct Auto {
    declencheur: String,
    regles: Vec<Regle>, // 3 conditions de 3 règles, mises bout à bout
}

fn nom(p: usize) -> String {
    format!("z2m:0x00158d00{:08x}/temperature_{}", p / 5, p % 5)
}

/// N automatismes : déclencheur propre, 9 règles sur des points de contexte.
fn automatismes(n: usize) -> Vec<Auto> {
    (0..n)
        .map(|k| Auto {
            declencheur: nom(k),
            regles: (0..9)
                .map(|j| Regle {
                    point: nom(20_000 + (k * 7 + j * 13) % 2_000),
                    op: if j == 8 { Op::Eq } else if k % 2 == 0 { Op::Ge } else { Op::Gt },
                    valeur: if j == 8 { serde_json::json!(0.0) } else { serde_json::json!(0) },
                })
                .collect(),
        })
        .collect()
}

// ---- moteur à balayage, comme aujourd'hui -----------------------------------------

struct Balayage {
    autos: Vec<Auto>,
    derniers: HashMap<String, Value>,
    etat: BTreeMap<String, Value>,
}

fn comparer(cur: &Value, op: Op, attendu: &Value) -> bool {
    match (cur.as_f64(), attendu.as_f64(), op) {
        (Some(c), Some(v), Op::Ge) => c >= v,
        (Some(c), Some(v), Op::Gt) => c > v,
        (Some(c), Some(v), Op::Eq) => (c - v).abs() < 1e-9,
        _ => false,
    }
}

impl Balayage {
    fn tient(&self, r: &Regle) -> bool {
        let point = r.point.clone(); // PointId::from(point)
        let Some(cur) = self.etat.get(&point).cloned() else { return false }; // hub.state
        let attendu = r.valeur.clone(); // canonical_expected
        comparer(&cur, r.op, &Value::from_json(&attendu)) // same / as_f64
    }
    fn evenement(&mut self, point: &str, v: Value) -> usize {
        self.etat.insert(point.to_owned(), v.clone());
        let cle = point.to_string();
        let avant = self.derniers.insert(cle.clone(), v.clone());
        if avant.as_ref() == Some(&v) {
            return 0;
        }
        let mut lances = 0;
        for a in &self.autos {
            if a.declencheur == cle && a.regles.iter().all(|r| self.tient(r)) {
                lances += 1;
            }
        }
        lances
    }
}

// ---- moteur compilé et indexé ----------------------------------------------------

struct Compile {
    etat: Vec<f64>,
    par_point: Vec<Vec<u32>>,
    regles: Vec<Vec<(u32, Op, f64)>>,
}

impl Compile {
    fn depuis(autos: &[Auto], points: &HashMap<String, u32>) -> Self {
        let mut par_point = vec![Vec::new(); points.len()];
        let mut regles = Vec::with_capacity(autos.len());
        for (k, a) in autos.iter().enumerate() {
            par_point[points[&a.declencheur] as usize].push(k as u32);
            regles.push(
                a.regles
                    .iter()
                    .map(|r| (points[&r.point], r.op, Value::from_json(&r.valeur).as_f64().unwrap()))
                    .collect(),
            );
        }
        Self { etat: vec![0.0; points.len()], par_point, regles }
    }
    fn evenement(&mut self, p: u32, v: f64) -> usize {
        let avant = std::mem::replace(&mut self.etat[p as usize], v);
        if avant == v {
            return 0;
        }
        let mut lances = 0;
        for &a in &self.par_point[p as usize] {
            if self.regles[a as usize].iter().all(|&(q, op, x)| {
                let c = self.etat[q as usize];
                match op {
                    Op::Ge => c >= x,
                    Op::Gt => c > x,
                    Op::Eq => (c - x).abs() < 1e-9,
                }
            }) {
                lances += 1;
            }
        }
        lances
    }
}

fn i1() {
    println!("\n## I1 — trouver les workflows à lancer et évaluer leurs 9 règles");
    println!("200 000 événements (20 000 pour N = 10 000), dont 10 % sur un déclencheur ; temps moyen par événement.");
    for n in [100usize, 400, 1_000, 10_000] {
        let autos = automatismes(n);
        let mut points: HashMap<String, u32> = HashMap::new();
        for p in (0..n).chain(20_000..22_000).chain(30_000..32_000) {
            let id = points.len() as u32;
            points.entry(nom(p)).or_insert(id);
        }
        let evenements = if n >= 10_000 { 20_000usize } else { 200_000 };
        let flux: Vec<usize> = (0..evenements)
            .map(|i| if i % 10 == 0 { (i * 7919) % n } else { 30_000 + (i * 104_729) % 2_000 })
            .collect();
        // Balayage.
        let mut b = Balayage { autos: automatismes(n), derniers: HashMap::new(), etat: BTreeMap::new() };
        for p in 20_000..22_000 {
            b.etat.insert(nom(p), Value::Float(0.0));
        }
        let noms: Vec<String> = flux.iter().map(|&p| nom(p)).collect();
        let t = Instant::now();
        let mut total_b = 0;
        for (i, p) in noms.iter().enumerate() {
            total_b += b.evenement(p, Value::Float((i + 1) as f64));
        }
        let ns_b = t.elapsed().as_nanos() as f64 / flux.len() as f64;
        // Compilé.
        let mut c = Compile::depuis(&autos, &points);
        let ids: Vec<u32> = flux.iter().map(|&p| points[&nom(p)]).collect();
        let t = Instant::now();
        let mut total_c = 0;
        for (i, &p) in ids.iter().enumerate() {
            total_c += c.evenement(p, (i + 1) as f64);
        }
        let ns_c = t.elapsed().as_nanos() as f64 / flux.len() as f64;
        assert_eq!(total_b, total_c, "les deux moteurs lancent les mêmes workflows");
        println!(
            "- N = {n:>6} : balayage {:>9.0} ns/événement ; compilé {:>5.0} ns/événement ; ×{:.0} ; {} lancements",
            ns_b,
            ns_c,
            ns_b / ns_c,
            black_box(total_c)
        );
    }
}

fn i2() {
    println!("\n## I2 — lire « moyenne sur 1 h » d'un capteur qui change chaque seconde");
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE samples (point INTEGER, ts INTEGER, num REAL, PRIMARY KEY (point, ts)) WITHOUT ROWID;",
    )
    .unwrap();
    let maintenant: i64 = 1_792_000_000_000;
    {
        let tx = conn.unchecked_transaction().unwrap();
        let mut ins = tx.prepare("INSERT INTO samples VALUES (?1, ?2, ?3)").unwrap();
        for p in 0..20i64 {
            for s in 0..86_400i64 {
                ins.execute((p, maintenant - (86_400 - s) * 1000, (s % 100) as f64)).unwrap();
            }
        }
        drop(ins);
        tx.commit().unwrap();
    }
    let mut req = conn.prepare("SELECT avg(num) FROM samples WHERE point = ?1 AND ts >= ?2").unwrap();
    let t = Instant::now();
    let mut s = 0.0;
    for i in 0..2_000i64 {
        let v: f64 = req.query_row((i % 20, maintenant - 3_600_000), |r| r.get(0)).unwrap();
        s += v;
    }
    let sql = t.elapsed().as_nanos() as f64 / 2_000.0;
    let fenetre: VecDeque<(i64, f64)> = (0..3_600).map(|s| (maintenant - (3_600 - s) * 1000, (s % 100) as f64)).collect();
    let t = Instant::now();
    for _ in 0..2_000 {
        s += black_box(&fenetre).iter().map(|(_, v)| v).sum::<f64>() / fenetre.len() as f64;
    }
    let balayage = t.elapsed().as_nanos() as f64 / 2_000.0;
    // Agrégat tenu au fil de l'eau : somme et compte glissants, mis à jour à chaque relevé.
    let (mut somme, mut compte, mut fen) = (0.0f64, 0usize, VecDeque::new());
    let t = Instant::now();
    for i in 0..1_000_000i64 {
        let ts = maintenant + i * 1000;
        let v = (i % 100) as f64;
        fen.push_back((ts, v));
        somme += v;
        compte += 1;
        while fen.front().is_some_and(|(t0, _)| *t0 < ts - 3_600_000) {
            let (_, ancien) = fen.pop_front().unwrap();
            somme -= ancien;
            compte -= 1;
        }
    }
    let maj = t.elapsed().as_nanos() as f64 / 1_000_000.0;
    let t = Instant::now();
    for _ in 0..1_000_000 {
        s += black_box(somme) / black_box(compte) as f64;
    }
    let lecture = t.elapsed().as_nanos() as f64 / 1_000_000.0;
    black_box(s);
    println!(
        "- SQLite (index (point, ts), 3 600 lignes lues) : {:.0} µs par lecture, plus le passage par un thread bloquant",
        sql / 1000.0
    );
    println!("- balayage d'une fenêtre en mémoire : {:.1} µs par lecture", balayage / 1000.0);
    println!("- agrégat tenu au fil de l'eau : {:.0} ns par relevé reçu, {:.1} ns par lecture", maj, lecture);
}

// ---- I3 : variables et calculs ----------------------------------------------------

fn interpreter(e: &serde_json::Value, vars: &HashMap<String, f64>) -> f64 {
    if let Some(c) = e.get("const") {
        return c.as_f64().unwrap();
    }
    if let Some(v) = e.get("var") {
        return vars[v.as_str().unwrap()];
    }
    let args: Vec<f64> = e["args"].as_array().unwrap().iter().map(|a| interpreter(a, vars)).collect();
    match e["op"].as_str().unwrap() {
        "sub" => args[0] - args[1],
        "gt" => f64::from(u8::from(args[0] > args[1])),
        "and" => f64::from(u8::from(args.iter().all(|a| *a != 0.0))),
        "or" => f64::from(u8::from(args.iter().any(|a| *a != 0.0))),
        "between" => f64::from(u8::from(args[1] <= args[0] && args[0] < args[2])),
        _ => unreachable!(),
    }
}

enum Expr {
    Const(f64),
    Var(usize),
    Sub(Box<Expr>, Box<Expr>),
    Gt(Box<Expr>, Box<Expr>),
    And(Vec<Expr>),
    Or(Vec<Expr>),
    Between(Box<Expr>, Box<Expr>, Box<Expr>),
}

fn compiler(e: &serde_json::Value, slots: &HashMap<&str, usize>) -> Expr {
    if let Some(c) = e.get("const") {
        return Expr::Const(c.as_f64().unwrap());
    }
    if let Some(v) = e.get("var") {
        return Expr::Var(slots[v.as_str().unwrap()]);
    }
    let a: Vec<Expr> = e["args"].as_array().unwrap().iter().map(|x| compiler(x, slots)).collect();
    let mut it = a.into_iter();
    match e["op"].as_str().unwrap() {
        "sub" => Expr::Sub(Box::new(it.next().unwrap()), Box::new(it.next().unwrap())),
        "gt" => Expr::Gt(Box::new(it.next().unwrap()), Box::new(it.next().unwrap())),
        "and" => Expr::And(it.collect()),
        "or" => Expr::Or(it.collect()),
        "between" => Expr::Between(Box::new(it.next().unwrap()), Box::new(it.next().unwrap()), Box::new(it.next().unwrap())),
        _ => unreachable!(),
    }
}

fn evaluer(e: &Expr, v: &[f64]) -> f64 {
    match e {
        Expr::Const(c) => *c,
        Expr::Var(i) => v[*i],
        Expr::Sub(a, b) => evaluer(a, v) - evaluer(b, v),
        Expr::Gt(a, b) => f64::from(u8::from(evaluer(a, v) > evaluer(b, v))),
        Expr::And(xs) => f64::from(u8::from(xs.iter().all(|x| evaluer(x, v) != 0.0))),
        Expr::Or(xs) => f64::from(u8::from(xs.iter().any(|x| evaluer(x, v) != 0.0))),
        Expr::Between(x, lo, hi) => {
            let x = evaluer(x, v);
            f64::from(u8::from(evaluer(lo, v) <= x && x < evaluer(hi, v)))
        }
    }
}

fn i3() {
    println!("\n## I3 — variables et calculs : « (moyenne 1 h du salon − 2) > 19 et (présence ou 7 h ≤ heure < 22 h) »");
    let e = serde_json::json!({"op":"and","args":[
        {"op":"gt","args":[{"op":"sub","args":[{"var":"moyenne_salon"},{"const":2.0}]},{"const":19.0}]},
        {"op":"or","args":[{"var":"presence"},{"op":"between","args":[{"var":"heure"},{"const":7.0},{"const":22.0}]}]}
    ]});
    let mut vars = HashMap::new();
    vars.insert("moyenne_salon".to_owned(), 21.5);
    vars.insert("presence".to_owned(), 0.0);
    vars.insert("heure".to_owned(), 18.0);
    let t = Instant::now();
    let mut s = 0.0;
    for _ in 0..200_000 {
        s += interpreter(black_box(&e), &vars);
    }
    let interp = t.elapsed().as_nanos() as f64 / 200_000.0;
    let slots: HashMap<&str, usize> = [("moyenne_salon", 0), ("presence", 1), ("heure", 2)].into();
    let c = compiler(&e, &slots);
    let valeurs = [21.5, 0.0, 18.0];
    let t = Instant::now();
    for _ in 0..200_000 {
        s += evaluer(black_box(&c), black_box(&valeurs));
    }
    let comp = t.elapsed().as_nanos() as f64 / 200_000.0;
    black_box(s);
    println!("- interprété depuis le JSON : {interp:.0} ns ; compilé une fois à la validation : {comp:.0} ns (×{:.0})", interp / comp);
}

// ---- I4 : workflows temporaires en baux durables ----------------------------------

fn i4() {
    println!("\n## I4 — « allume 30 minutes » en bail durable : 10 000 baux");
    let mut tas = BinaryHeap::new();
    let t = Instant::now();
    for i in 0..10_000u64 {
        tas.push(std::cmp::Reverse((1_792_000_000_000 + (i * 7_919) % 1_800_000, i)));
    }
    let mut dus = 0;
    while let Some(std::cmp::Reverse((echeance, _))) = tas.peek() {
        if *echeance > 1_792_000_900_000 {
            break;
        }
        tas.pop();
        dus += 1;
    }
    let ordo = t.elapsed();
    let chemin = std::env::temp_dir().join("moli-baux-essai.jsonl");
    let t = Instant::now();
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&chemin).unwrap();
        for i in 0..20 {
            writeln!(f, "{{\"bail\":{i},\"point\":\"lumieres:salon/on\",\"echeance\":1792001800000,\"retour\":false}}").unwrap();
            f.sync_data().unwrap();
        }
    }
    let unitaire = t.elapsed() / 20;
    let _ = std::fs::remove_file(&chemin);
    println!(
        "- ordonnancement de 10 000 échéances (tas binaire) et extraction des {dus} dues : {:.2} ms au total",
        ordo.as_secs_f64() * 1e3
    );
    println!(
        "- écrire un bail de façon durable (ajout + fsync) : {:.1} ms, une seule fois au lancement ; rien pendant l'attente",
        unitaire.as_secs_f64() * 1e3
    );
}

fn main() {
    println!("# Workflows complexes : balayage d'aujourd'hui contre moteur compilé (machine de test)");
    i1();
    i2();
    i3();
    i4();
}
```

Code complet de `src/bin/cloisons.rs` :

```rust
//! C1-C3 : des cloisons pour qu'un matériel muet ou un pilote qui bloque n'arrête pas
//! le reste. Mesuré contre le vrai `moli-runtime`, runtime principal à 2 workers.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use moli_core::{Access, Device, DeviceId, InstanceId, Kind, Origin, PointId, PointSpec, Semantic, Value};
use moli_runtime::{BoxFuture, CommandRequest, Driver, DriverCtx, Hub, HubOptions, spawn_driver};
use tokio::sync::{Semaphore, mpsc};
use tokio_util::sync::CancellationToken;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap()
}

fn lampe(id: DeviceId, instance: &InstanceId) -> Device {
    Device {
        id,
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
            access: Access { read: true, write: true },
            unit: None,
            semantic: Semantic::OnOff,
        }],
    }
}

fn ms(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1e3)
}

fn quantile(v: &mut [Duration], p: f64) -> String {
    if v.is_empty() {
        return "—".into();
    }
    v.sort();
    ms(v[((v.len() - 1) as f64 * p) as usize])
}

async fn attendre_running(hub: &Hub, instance: &str) {
    loop {
        let ok = hub
            .snapshot()
            .drivers
            .iter()
            .any(|d| d.instance.as_str() == instance && format!("{:?}", d.status) == "Running");
        if ok {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

// ---- C1 : un thread par pilote ------------------------------------------------------

struct Sain;
impl Driver for Sain {
    fn kind(&self) -> &'static str {
        "sain"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let id = ctx.device_id("l1");
            ctx.upsert_device(lampe(id, &ctx.instance().clone()));
            ctx.ready();
            while let Some(cmd) = ctx.next_command().await {
                ctx.set_state(&cmd.device.id, &cmd.key, cmd.value.clone());
                cmd.reply(Ok(()));
            }
            Ok(())
        })
    }
}

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

struct Inactif;
impl Driver for Inactif {
    fn kind(&self) -> &'static str {
        "inactif"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            ctx.ready();
            while let Some(cmd) = ctx.next_command().await {
                cmd.reply(Ok(()));
            }
            Ok(())
        })
    }
}

/// Le pilote tourne sur son propre thread, avec son propre runtime Tokio : s'il bloque,
/// il ne bloque que lui. Ses sous-tâches (`tokio::spawn`) restent aussi chez lui.
fn isole(hub: &Hub, nom: &str, pilote: Arc<dyn Driver>, cancel: CancellationToken) {
    let (hub, nom) = (hub.clone(), InstanceId::from(nom));
    std::thread::Builder::new()
        .name(format!("pilote-{nom}"))
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            rt.block_on(async move {
                let _ = spawn_driver(&hub, nom, pilote, cancel).await;
            });
        })
        .unwrap();
}

fn rss_kio() -> u64 {
    let pid = std::process::id().to_string();
    let out = std::process::Command::new("ps").args(["-o", "rss=", "-p", &pid]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap_or(0)
}

async fn mesurer_coeur(hub: &Hub) -> (usize, String, String, String) {
    let p = PointId::from("sain:l1/on");
    let pire_retard = Arc::new(AtomicU64::new(0));
    let stop = CancellationToken::new();
    let (r, s) = (pire_retard.clone(), stop.clone());
    let sonde = tokio::spawn(async move {
        while !s.is_cancelled() {
            let t = Instant::now();
            tokio::time::sleep(Duration::from_millis(10)).await;
            let retard = t.elapsed().saturating_sub(Duration::from_millis(10));
            r.fetch_max(retard.as_micros() as u64, Ordering::Relaxed);
        }
    });
    let mut lat = vec![];
    let fin = Instant::now() + Duration::from_secs(2);
    let mut v = false;
    while Instant::now() < fin {
        v = !v;
        let t = Instant::now();
        let _ = hub.command(&p, Value::Bool(v), Origin::Ui, None).await;
        lat.push(t.elapsed());
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    stop.cancel();
    let _ = sonde.await;
    let n = lat.len();
    let med = quantile(&mut lat, 0.5);
    let max = quantile(&mut lat, 1.0);
    (n, med, max, ms(Duration::from_micros(pire_retard.load(Ordering::Relaxed))))
}

fn c1() {
    println!("\n## C1 — un thread par pilote contre les deux workers partagés");
    for (titre, combien, isoles) in [
        ("2 pilotes bloquants partagés ", 2usize, false),
        ("6 pilotes bloquants isolés   ", 6, true),
    ] {
        runtime().block_on(async {
            let hub = Hub::new(HubOptions::default()).unwrap();
            let cancel = CancellationToken::new();
            spawn_driver(&hub, "sain".into(), Arc::new(Sain), cancel.clone());
            attendre_running(&hub, "sain").await;
            for i in 0..combien {
                let nom = format!("bloquant{i}");
                if isoles {
                    isole(&hub, &nom, Arc::new(Bloquant), cancel.clone());
                } else {
                    spawn_driver(&hub, nom.as_str().into(), Arc::new(Bloquant), cancel.clone());
                }
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
            let (n, med, max, retard) = mesurer_coeur(&hub).await;
            println!("- {titre}: {n:>3} ordres en 2 s, médiane {med:>8}, pire {max:>8} ; pire retard d'une minuterie de 10 ms {retard:>9}");
            cancel.cancel();
        });
    }
    runtime().block_on(async {
        let hub = Hub::new(HubOptions::default()).unwrap();
        let cancel = CancellationToken::new();
        tokio::time::sleep(Duration::from_millis(200)).await;
        let avant = rss_kio();
        for i in 0..40 {
            isole(&hub, &format!("inactif{i}"), Arc::new(Inactif), cancel.clone());
        }
        for i in 0..40 {
            attendre_running(&hub, &format!("inactif{i}")).await;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        let apres = rss_kio();
        println!(
            "- coût : 40 pilotes isolés au repos ajoutent {} Kio de RSS, soit {:.0} Kio par pilote (pile de 2 Mio réservée, seules les pages touchées comptent)",
            apres.saturating_sub(avant),
            apres.saturating_sub(avant) as f64 / 40.0
        );
        cancel.cancel();
    });
}

// ---- C2/C3 : un pont, vingt lampes, une muette ------------------------------------

const LAMPES: usize = 20;
const MUETTE: usize = 0;

#[derive(Clone, Copy, PartialEq)]
enum Strategie {
    /// Une seule boucle attend chaque réponse (le modèle le plus courant dans le dépôt).
    BoucleUnique,
    /// Une tâche par ordre, sans plafond par appareil.
    TacheParOrdre,
    /// Une voie par appareil : file bornée, un ordre en vol, échéance revérifiée.
    Voies,
    /// Voies + disjoncteur par appareil : après 2 échecs, refus immédiat 30 s.
    VoiesDisjoncteur,
}

struct Pont {
    strategie: Strategie,
    /// L'API du pont n'accepte que `concurrence` requêtes à la fois.
    api: Arc<Semaphore>,
}

async fn appel(api: &Semaphore, appareil: usize) -> Result<(), String> {
    let _jeton = api.acquire().await.unwrap();
    if appareil == MUETTE {
        // Aucune réponse : le délai de 4 s par étape de moli-net finit par tomber.
        tokio::time::sleep(Duration::from_secs(4)).await;
        Err("délai dépassé".into())
    } else {
        tokio::time::sleep(Duration::from_millis(30)).await;
        Ok(())
    }
}

fn indice(cmd: &CommandRequest) -> usize {
    cmd.device.id.as_str()["pont:lampe".len()..].parse().unwrap()
}

impl Driver for Pont {
    fn kind(&self) -> &'static str {
        "pont"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            for i in 0..LAMPES {
                let id = ctx.device_id(&format!("lampe{i}"));
                ctx.upsert_device(lampe(id.clone(), &ctx.instance().clone()));
                ctx.set_availability(&id, true);
            }
            ctx.ready();
            match self.strategie {
                Strategie::BoucleUnique => {
                    while let Some(cmd) = ctx.next_command().await {
                        let i = indice(&cmd);
                        let r = appel(&self.api, i).await;
                        cmd.reply(r);
                    }
                }
                Strategie::TacheParOrdre => {
                    let mut taches = tokio::task::JoinSet::new();
                    while let Some(cmd) = ctx.next_command().await {
                        let api = self.api.clone();
                        taches.spawn(async move {
                            let i = indice(&cmd);
                            let r = appel(&api, i).await;
                            cmd.reply(r);
                        });
                        while taches.try_join_next().is_some() {}
                    }
                }
                Strategie::Voies | Strategie::VoiesDisjoncteur => {
                    let disjoncteur = self.strategie == Strategie::VoiesDisjoncteur;
                    let mut voies = Vec::new();
                    let mut taches = tokio::task::JoinSet::new();
                    let hors_ligne = Arc::new(Mutex::new(Vec::<usize>::new()));
                    for i in 0..LAMPES {
                        let (tx, mut rx) = mpsc::channel::<CommandRequest>(8);
                        voies.push(tx);
                        let api = self.api.clone();
                        let hors_ligne = hors_ligne.clone();
                        taches.spawn(async move {
                            let (mut echecs, mut ouvert_jusqua) = (0u32, None::<Instant>);
                            while let Some(cmd) = rx.recv().await {
                                if cmd.abandoned() {
                                    cmd.reply(Err("expiré avant l'envoi".into()));
                                    continue;
                                }
                                if let Some(t) = ouvert_jusqua {
                                    if Instant::now() < t {
                                        cmd.reply(Err("injoignable : disjoncteur ouvert".into()));
                                        continue;
                                    }
                                    ouvert_jusqua = None; // essai à demi-ouvert
                                }
                                let r = appel(&api, i).await;
                                if r.is_err() {
                                    echecs += 1;
                                    if disjoncteur && echecs >= 2 {
                                        ouvert_jusqua = Some(Instant::now() + Duration::from_secs(30));
                                        hors_ligne.lock().unwrap().push(i);
                                        // Les ordres déjà en file n'attendent pas : refus immédiat.
                                        while let Ok(c) = rx.try_recv() {
                                            c.reply(Err("injoignable : disjoncteur ouvert".into()));
                                        }
                                    }
                                } else {
                                    echecs = 0;
                                }
                                cmd.reply(r);
                            }
                        });
                    }
                    loop {
                        tokio::select! {
                            cmd = ctx.next_command() => {
                                let Some(cmd) = cmd else { break };
                                let i = indice(&cmd);
                                if let Err(e) = voies[i].try_send(cmd) {
                                    let cmd = match e {
                                        mpsc::error::TrySendError::Full(c) | mpsc::error::TrySendError::Closed(c) => c,
                                    };
                                    cmd.reply(Err("file de l'appareil pleine".into()));
                                }
                                let tombes: Vec<usize> = std::mem::take(&mut *hors_ligne.lock().unwrap());
                                for t in tombes {
                                    ctx.set_availability(&ctx.device_id(&format!("lampe{t}")), false);
                                }
                            }
                            fin = taches.join_next() => {
                                // Une voie qui meurt fait relancer tout le pilote : jamais en silence.
                                anyhow::bail!("voie arrêtée : {fin:?}");
                            }
                        }
                    }
                }
            }
            Ok(())
        })
    }
}

fn pont(strategie: Strategie, concurrence: usize) -> String {
    runtime().block_on(async {
        let hub = Hub::new(HubOptions::default()).unwrap();
        let cancel = CancellationToken::new();
        spawn_driver(
            &hub,
            "pont".into(),
            Arc::new(Pont { strategie, api: Arc::new(Semaphore::new(concurrence)) }),
            cancel.clone(),
        );
        attendre_running(&hub, "pont").await;
        let saines = Arc::new(Mutex::new(Vec::new()));
        let echecs_sains = Arc::new(AtomicUsize::new(0));
        let muette = Arc::new(Mutex::new(Vec::new()));
        let mut taches = Vec::new();
        // 10 s, 25 ordres par seconde, répartis sur les 20 lampes : 1,25 par seconde vers la muette.
        for n in 0..250usize {
            let i = n % LAMPES;
            let (hub, saines, echecs_sains, muette) = (hub.clone(), saines.clone(), echecs_sains.clone(), muette.clone());
            taches.push(tokio::spawn(async move {
                let p = PointId::from(format!("pont:lampe{i}/on"));
                let t = Instant::now();
                let r = hub.command(&p, Value::Bool(n % 2 == 0), Origin::Ui, None).await;
                let d = t.elapsed();
                if i == MUETTE {
                    muette.lock().unwrap().push(d);
                } else if r.is_ok() {
                    saines.lock().unwrap().push(d);
                } else {
                    echecs_sains.fetch_add(1, Ordering::Relaxed);
                }
            }));
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
        for t in taches {
            let _ = t.await;
        }
        cancel.cancel();
        let mut s = saines.lock().unwrap().clone();
        let mut m = muette.lock().unwrap().clone();
        format!(
            "lampes saines {}/{} réussis, médiane {:>8}, pire {:>9} ; lampe muette : médiane {:>9}, réponse la plus rapide {:>8}",
            s.len(),
            s.len() + echecs_sains.load(Ordering::Relaxed),
            quantile(&mut s, 0.5),
            quantile(&mut s, 1.0),
            quantile(&mut m, 0.5),
            quantile(&mut m, 0.0)
        )
    })
}

fn main() {
    println!("# Cloisons : un matériel muet ou un pilote bloquant ne doit pas arrêter le reste");
    c1();
    println!("\n## C2 — un pont, 20 lampes, une lampe muette (API du pont sans limite de concurrence)");
    for (titre, s) in [
        ("boucle unique      ", Strategie::BoucleUnique),
        ("voie par appareil  ", Strategie::Voies),
        ("voies + disjoncteur", Strategie::VoiesDisjoncteur),
    ] {
        println!("- {titre} : {}", pont(s, 1_000));
    }
    println!("\n## C3 — le même pont, mais son API ne traite que 4 requêtes à la fois");
    for (titre, s) in [
        ("tâche par ordre    ", Strategie::TacheParOrdre),
        ("voie par appareil  ", Strategie::Voies),
        ("voies + disjoncteur", Strategie::VoiesDisjoncteur),
    ] {
        println!("- {titre} : {}", pont(s, 4));
    }
}
```

Code complet de `src/bin/convergence.rs` :

```rust
//! V1 : « tout passe par le moteur ». Le vrai moteur d'automatismes reste sur le runtime
//! principal ; les pilotes tournent chacun sur leur thread. Le moteur voit-il tout, et
//! reste-t-il réactif quand deux pilotes bloquent ?

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use moli_automation::{Actor, Author, Automation, Automations, Config, Graph, Mode};
use moli_core::{Access, Device, DeviceId, InstanceId, Kind, PointId, PointSpec, Semantic, Value};
use moli_runtime::{BoxFuture, Driver, DriverCtx, Hub, HubOptions, spawn_driver};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

fn point(cle: &str, kind: Kind, ecriture: bool) -> PointSpec {
    PointSpec {
        key: cle.into(),
        label: cle.into(),
        kind,
        access: Access { read: true, write: ecriture },
        unit: None,
        semantic: Semantic::Other,
    }
}

fn appareil(id: DeviceId, instance: &InstanceId, p: PointSpec) -> Device {
    Device {
        id,
        instance: instance.clone(),
        native_name: "fictif".into(),
        manufacturer: None,
        model: None,
        description: None,
        native_room: None,
        members: vec![],
        points: vec![p],
    }
}

/// Un capteur : sa valeur change quand le banc le demande, et il note sur quel thread.
struct Capteur {
    injections: Mutex<Option<mpsc::UnboundedReceiver<f64>>>,
    thread: Arc<Mutex<String>>,
}
impl Driver for Capteur {
    fn kind(&self) -> &'static str {
        "capteur"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let id = ctx.device_id("c0");
            ctx.upsert_device(appareil(id.clone(), &ctx.instance().clone(), point("v", Kind::Numeric { min: None, max: None, step: None }, false)));
            ctx.set_state(&id, "v", Value::Float(0.0));
            *self.thread.lock().unwrap() = std::thread::current().name().unwrap_or("?").to_owned();
            let mut rx = self.injections.lock().unwrap().take().unwrap();
            ctx.ready();
            while let Some(v) = rx.recv().await {
                ctx.set_state(&id, "v", Value::Float(v));
            }
            Ok(())
        })
    }
}

/// Une lampe : note l'instant où l'ordre du moteur arrive, et sur quel thread.
struct Lampe {
    arrivees: Arc<Mutex<Vec<Instant>>>,
    thread: Arc<Mutex<String>>,
}
impl Driver for Lampe {
    fn kind(&self) -> &'static str {
        "lampe"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let id = ctx.device_id("l0");
            ctx.upsert_device(appareil(id.clone(), &ctx.instance().clone(), point("on", Kind::Binary, true)));
            ctx.set_state(&id, "on", Value::Bool(false));
            ctx.ready();
            while let Some(cmd) = ctx.next_command().await {
                self.arrivees.lock().unwrap().push(Instant::now());
                *self.thread.lock().unwrap() = std::thread::current().name().unwrap_or("?").to_owned();
                ctx.set_state(&cmd.device.id, &cmd.key, cmd.value.clone());
                cmd.reply(Ok(()));
            }
            Ok(())
        })
    }
}

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

fn isole(hub: &Hub, nom: &str, pilote: Arc<dyn Driver>, cancel: CancellationToken) {
    let (hub, nom) = (hub.clone(), InstanceId::from(nom));
    std::thread::Builder::new()
        .name(format!("pilote-{nom}"))
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            rt.block_on(async move {
                let _ = spawn_driver(&hub, nom, pilote, cancel).await;
            });
        })
        .unwrap();
}

fn essai(isoles: bool) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("coeur")
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let hub = Hub::new(HubOptions::default()).unwrap();
        let cancel = CancellationToken::new();
        let (tx, rx) = mpsc::unbounded_channel();
        let arrivees = Arc::new(Mutex::new(Vec::new()));
        let (t_capteur, t_lampe) = (Arc::new(Mutex::new(String::new())), Arc::new(Mutex::new(String::new())));
        let pilotes: Vec<(&str, Arc<dyn Driver>)> = vec![
            ("capteurs", Arc::new(Capteur { injections: Mutex::new(Some(rx)), thread: t_capteur.clone() })),
            ("lampes", Arc::new(Lampe { arrivees: arrivees.clone(), thread: t_lampe.clone() })),
            ("bloquant1", Arc::new(Bloquant)),
            ("bloquant2", Arc::new(Bloquant)),
        ];
        for (nom, p) in pilotes {
            if isoles {
                isole(&hub, nom, p, cancel.clone());
            } else {
                spawn_driver(&hub, nom.into(), p, cancel.clone());
            }
        }
        while hub.state(&PointId::from("lampes:l0/on")).is_none() || hub.state(&PointId::from("capteurs:c0/v")).is_none() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let (autos, _tache) = Automations::start(&hub, Config::default(), None, cancel.clone()).await.unwrap();
        let graphe = serde_json::json!({
            "nodes": [
                {"id":"t","type":"when_state","point":"capteurs:c0/v"},
                {"id":"c","type":"if","rules":[{"kind":"state","point":"capteurs:c0/v","op":"ge","value":0}]},
                {"id":"a","type":"toggle","point":"lampes:l0/on"}
            ],
            "edges": [{"from":"t","to":"c"},{"from":"c","port":"yes","to":"a"}]
        });
        let humain = Actor { human: true, author: Author::Human };
        let a = autos
            .save(
                Automation {
                    id: String::new(),
                    name: "capteur → lampe".into(),
                    enabled: true,
                    mode: Mode::Restart,
                    graph: serde_json::from_value::<Graph>(graphe).unwrap(),
                    author: Author::Human,
                    approved: None,
                    approved_version: None,
                    activation: None,
                    note: None,
                    created: 0,
                    updated: 0,
                },
                humain,
            )
            .await
            .unwrap();
        autos.approve(&a.id, &a.fingerprint(), humain).await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        let mut injections = Vec::new();
        for i in 1..=40 {
            injections.push(Instant::now());
            tx.send(f64::from(i)).unwrap();
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
        let arr = arrivees.lock().unwrap().clone();
        let mut lat: Vec<Duration> = injections
            .iter()
            .filter_map(|t0| arr.iter().find(|t| *t >= t0).map(|t| *t - *t0))
            .collect();
        lat.sort();
        let q = |p: f64| lat.get(((lat.len().max(1) - 1) as f64 * p) as usize).map_or("—".into(), |d| format!("{:.1} ms", d.as_secs_f64() * 1e3));
        println!(
            "- pilotes {} : {} actions sur 40 événements, médiane {}, pire {} ; capteur sur « {} », lampe sur « {} », moteur sur « coeur »",
            if isoles { "isolés (un thread chacun)" } else { "partagés (2 workers)    " },
            lat.len(),
            q(0.5),
            q(1.0),
            t_capteur.lock().unwrap(),
            t_lampe.lock().unwrap()
        );
        cancel.cancel();
        tokio::time::sleep(Duration::from_millis(300)).await;
    });
}

fn main() {
    println!("# Le moteur voit tout, même quand les pilotes vivent sur leurs propres threads");
    println!("Capteur → moteur d'automatismes réel → lampe, avec deux pilotes qui bloquent 200 ms en boucle.");
    essai(false);
    essai(true);
}
```

Code complet de `src/bin/memoire.rs` :

```rust
//! M1-M4 : ce que coûte en mémoire l'état « utile » d'une maison, mesuré octet par
//! octet avec un allocateur qui compte, sur la forme réelle de deux Home Assistant
//! chargés (comptes agrégés, aucune donnée nominative).

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::{Duration, Instant};

use moli_core::{Access, Device, Kind, PointSpec, Semantic, Value};
use moli_runtime::{BoxFuture, Driver, DriverCtx, Hub, HubOptions, spawn_driver};
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

/// Octets demandés, plus 16 o d'en-tête par bloc (ordre de grandeur des allocateurs).
fn cout(avant: (isize, isize), apres: (isize, isize)) -> f64 {
    ((apres.0 - avant.0) + 16 * (apres.1 - avant.1)) as f64
}

fn mio(o: f64) -> String {
    if o >= 1024.0 * 1024.0 {
        format!("{:.2} Mio", o / 1024.0 / 1024.0)
    } else {
        format!("{:.0} Kio", o / 1024.0)
    }
}

/// Forme réunie des deux instances : 496 appareils, 2 264 valeurs vivantes, dont
/// 419 nombres, 454 binaires, 318 textes et 1 073 absentes (injoignables, inconnues).
struct Maison {
    appareils: usize,
    valeurs: usize,
}

const CLES: [&str; 8] = [
    "temperature", "humidity", "battery", "linkquality", "power", "state", "illuminance", "mode",
];
const TEXTES: [&str; 6] = ["heat_cool", "idle", "playing", "paused", "auto", "standby"];

fn valeur(i: usize) -> (Kind, Value) {
    // Proportions observées : 18,5 % nombres, 20 % binaires, 14 % textes, 47,5 % absentes.
    match i % 200 {
        0..37 => (
            Kind::Numeric {
                min: None,
                max: None,
                step: None,
            },
            Value::Float(20.0 + (i % 17) as f64 * 0.5),
        ),
        37..77 => (Kind::Binary, Value::Bool(i % 3 == 0)),
        77..105 => (Kind::Text, Value::Text(TEXTES[i % TEXTES.len()].into())),
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
            let mut n = 0;
            for d in 0..self.appareils {
                let par = self.valeurs / self.appareils + usize::from(d < self.valeurs % self.appareils);
                let id = ctx.device_id(&format!("0x00158d00{d:08x}"));
                let points: Vec<PointSpec> = (0..par)
                    .map(|k| PointSpec {
                        key: format!("{}_{k}", CLES[(d + k) % CLES.len()]).into(),
                        label: CLES[(d + k) % CLES.len()].into(),
                        kind: valeur(n + k).0,
                        access: Access {
                            read: true,
                            write: k == 0,
                        },
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

fn modele(echelle: usize) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let hub = Hub::new(HubOptions::default()).unwrap();
        let cancel = CancellationToken::new();
        let avant = tas();
        let (appareils, valeurs) = (496 * echelle, 2_264 * echelle);
        spawn_driver(&hub, "maison".into(), Arc::new(Maison { appareils, valeurs }), cancel.clone());
        while hub.stats().points < valeurs || hub.snapshot().drivers.iter().all(|d| format!("{:?}", d.status) != "Running") {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        let apres = tas();
        let c = cout(avant, apres);
        println!(
            "- ×{echelle} : {appareils} appareils, {valeurs} valeurs → {} pour le modèle du hub (≈ {:.0} o par valeur, définitions comprises)",
            mio(c),
            c / valeurs as f64
        );
        cancel.cancel();
    });
}

/// Taux réels (changements par jour) par vingtile de séries, deux instances réunies.
const VINGTILES: [f64; 20] = [
    0.28, 0.3, 0.3, 0.3, 0.3, 0.3, 0.3, 0.33, 0.48, 0.63, 0.9, 1.14, 1.41, 2.16, 3.57, 5.73,
    9.22, 21.92, 140.88, 1990.22,
];
const SERIES: usize = 1_487;

fn taux(serie: usize) -> f64 {
    VINGTILES[serie * 20 / SERIES]
}

fn fenetres(jours: f64) {
    let total: f64 = (0..SERIES).map(|s| taux(s) * jours).sum();
    // a) naïf : (horodatage, Value) dans une VecDeque par série.
    let avant = tas();
    let mut naif: HashMap<u32, VecDeque<(u64, Value)>> = HashMap::new();
    for s in 0..SERIES {
        let n = (taux(s) * jours).round() as usize;
        let q = naif.entry(s as u32).or_default();
        for i in 0..n {
            q.push_back((1_792_000_000_000 + i as u64 * 1000, Value::Float(i as f64)));
        }
    }
    let c_naif = cout(avant, tas());
    // b) compact : décalage en secondes (u32) + valeur f32, colonnes par série.
    let avant = tas();
    let mut compact: HashMap<u32, (Vec<u32>, Vec<f32>)> = HashMap::new();
    for s in 0..SERIES {
        let n = (taux(s) * jours).round() as usize;
        let (t, v) = compact.entry(s as u32).or_default();
        t.reserve_exact(n);
        v.reserve_exact(n);
        for i in 0..n {
            t.push(i as u32);
            v.push(i as f32);
        }
    }
    let c_compact = cout(avant, tas());
    // c) mixte : brut compact sous 1 changement/min, seaux par minute au-delà.
    let avant = tas();
    let mut mixte: HashMap<u32, (Vec<u32>, Vec<f32>)> = HashMap::new();
    let mut seaux: HashMap<u32, Vec<[f32; 5]>> = HashMap::new();
    for s in 0..SERIES {
        let n = (taux(s) * jours).round() as usize;
        if taux(s) <= 1_440.0 {
            let (t, v) = mixte.entry(s as u32).or_default();
            t.reserve_exact(n);
            v.reserve_exact(n);
            for i in 0..n {
                t.push(i as u32);
                v.push(i as f32);
            }
        } else {
            let minutes = (1_440.0 * jours) as usize;
            let b = seaux.entry(s as u32).or_default();
            b.reserve_exact(minutes);
            for m in 0..minutes {
                b.push([m as f32, 0.0, 1.0, 0.5, 3.0]);
            }
        }
    }
    let c_mixte = cout(avant, tas());
    println!(
        "- fenêtre {jours} j : {:.0} relevés → naïf {} ; compact {} ; mixte (seaux/minute pour les séries > 1/min) {}",
        total,
        mio(c_naif),
        mio(c_compact),
        mio(c_mixte)
    );
    drop((naif, compact, mixte, seaux));
}

fn instantane() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let hub = Hub::new(HubOptions::default()).unwrap();
        let cancel = CancellationToken::new();
        spawn_driver(&hub, "maison".into(), Arc::new(Maison { appareils: 496, valeurs: 2_264 }), cancel.clone());
        while hub.stats().points < 2_264 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        // État d'exécution : valeurs, définitions, et 400 automatismes de 8 nœuds.
        let graphe = serde_json::json!({
            "name": "Automatisme fictif", "mode": "restart", "enabled": true,
            "nodes": [
                {"id":"t","type":"when_state","point":"maison:0x00158d0000000001/temperature_0"},
                {"id":"c1","type":"if","rules":[{"kind":"state","point":"maison:0x00158d0000000002/state_1","op":"eq","value":true},{"kind":"time","after":"07:00","before":"22:00"}]},
                {"id":"c2","type":"if","rules":[{"kind":"state","point":"maison:0x00158d0000000003/power_2","op":"gt","value":100}]},
                {"id":"a1","type":"set","point":"maison:0x00158d0000000004/mode_0","value":true},
                {"id":"w","type":"wait","seconds":1800},
                {"id":"a2","type":"set","point":"maison:0x00158d0000000004/mode_0","value":false},
                {"id":"n","type":"notify","message":"Message fictif"},
                {"id":"wf","type":"wait_for","rule":{"kind":"state","point":"maison:0x00158d0000000005/state_3","op":"eq","value":false},"timeout_s":600}
            ],
            "edges": [{"from":"t","to":"c1"},{"from":"c1","port":"yes","to":"c2"},{"from":"c2","port":"yes","to":"a1"},
                      {"from":"a1","to":"w"},{"from":"w","to":"a2"},{"from":"a2","to":"n"},{"from":"c2","port":"no","to":"wf"}]
        });
        let automatismes: Vec<serde_json::Value> = (0..400).map(|_| graphe.clone()).collect();
        let t = Instant::now();
        let etat = serde_json::json!({
            "version": 1,
            "maison": hub.snapshot(),
            "automatismes": automatismes,
        });
        let octets = serde_json::to_vec(&etat).unwrap();
        let t_json = t.elapsed();
        let t = Instant::now();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut gz, &octets).unwrap();
        let compresse = gz.finish().unwrap();
        let t_gz = t.elapsed();
        let chemin = std::env::temp_dir().join("moli-instantane-essai.json.gz");
        let t = Instant::now();
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&chemin).unwrap();
            f.write_all(&compresse).unwrap();
            f.sync_all().unwrap();
        }
        let t_ecriture = t.elapsed();
        let t = Instant::now();
        let relu: serde_json::Value = serde_json::from_slice(&octets).unwrap();
        let t_lecture = t.elapsed();
        let _ = std::fs::remove_file(&chemin);
        println!(
            "- instantané d'exécution (2 264 valeurs + 400 automatismes) : JSON {} en {:.1} ms, gzip {} en {:.1} ms, écriture + fsync {:.1} ms, relecture {:.1} ms ({} automatismes relus)",
            mio(octets.len() as f64),
            t_json.as_secs_f64() * 1e3,
            mio(compresse.len() as f64),
            t_gz.as_secs_f64() * 1e3,
            t_ecriture.as_secs_f64() * 1e3,
            t_lecture.as_secs_f64() * 1e3,
            relu["automatismes"].as_array().map_or(0, Vec::len)
        );
        cancel.cancel();
    });
}

fn main() {
    println!("# Mémoire : l'état utile d'une maison en Rust");
    println!("\n## M1 — tailles des types du cœur");
    println!(
        "- Value {} o, Sample {} o, (u64, Value) {} o, Arc<str> {} o",
        std::mem::size_of::<Value>(),
        std::mem::size_of::<moli_core::Sample>(),
        std::mem::size_of::<(u64, Value)>(),
        std::mem::size_of::<Arc<str>>()
    );
    println!("\n## M2 — modèle du hub à l'échelle des deux Home Assistant réunis");
    modele(1);
    modele(10);
    println!("\n## M3 — garder l'historique récent en mémoire (taux réels, 1 487 séries)");
    fenetres(1.0);
    fenetres(7.0);
    println!("\n## M4 — instantané pour une relance ailleurs");
    instantane();
}
```

Code complet de `src/bin/relance.rs` :

```rust
//! R1-R2 : relancer ailleurs depuis l'état. Combien de temps pour reconstruire un hub
//! complet depuis un instantané, et combien pèse le journal des changements entre deux
//! instantanés au rythme réel des deux maisons (163 000 relevés par jour).

use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};

use moli_core::{Access, Device, DeviceId, Kind, PointSpec, Semantic, Value};
use moli_runtime::{BoxFuture, Driver, DriverCtx, Hub, HubOptions, spawn_driver};
use tokio_util::sync::CancellationToken;

/// Republie un instantané : appareils puis valeurs, comme le ferait une reprise.
struct Restaure(Arc<serde_json::Value>);

impl Driver for Restaure {
    fn kind(&self) -> &'static str {
        "restaure"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            for d in self.0["devices"].as_array().unwrap() {
                let mut device: Device = serde_json::from_value(d.clone()).unwrap();
                device.instance = ctx.instance().clone();
                let natif = device.id.as_str().split_once(':').unwrap().1.to_owned();
                device.id = ctx.device_id(&natif);
                let id = device.id.clone();
                ctx.upsert_device(device);
                if let Some(etat) = d["state"].as_object() {
                    for (cle, s) in etat {
                        ctx.set_state(&id, cle, Value::from_json(&s["value"]));
                    }
                }
            }
            ctx.ready();
            std::future::pending::<()>().await;
            Ok(())
        })
    }
}

/// La maison d'origine : 496 appareils, 2 264 valeurs.
struct Origine;
impl Driver for Origine {
    fn kind(&self) -> &'static str {
        "origine"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let instance = ctx.instance().clone();
            let mut n = 0usize;
            for d in 0..496usize {
                let par = 2_264 / 496 + usize::from(d < 2_264 % 496);
                let id = ctx.device_id(&format!("0x00158d00{d:08x}"));
                let points: Vec<PointSpec> = (0..par)
                    .map(|k| PointSpec {
                        key: format!("valeur_{k}").into(),
                        label: "valeur".into(),
                        kind: Kind::Numeric { min: None, max: None, step: None },
                        access: Access { read: true, write: false },
                        unit: None,
                        semantic: Semantic::Other,
                    })
                    .collect();
                ctx.upsert_device(Device {
                    id: id.clone(),
                    instance: instance.clone(),
                    native_name: format!("Appareil fictif {d}").into(),
                    manufacturer: None,
                    model: None,
                    description: None,
                    native_room: None,
                    members: vec![],
                    points: points.clone(),
                });
                for p in &points {
                    n += 1;
                    ctx.set_state(&id, &p.key, Value::Float(n as f64 / 7.0));
                }
            }
            ctx.ready();
            std::future::pending::<()>().await;
            Ok(())
        })
    }
}

fn main() {
    println!("# Relance ailleurs depuis l'état");
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    rt.block_on(async {
        let source = Hub::new(HubOptions::default()).unwrap();
        let cancel = CancellationToken::new();
        spawn_driver(&source, "maison".into(), Arc::new(Origine), cancel.clone());
        while source.stats().points < 2_264 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        let instantane = serde_json::to_vec(&source.snapshot()).unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&instantane).unwrap();
        let compresse = gz.finish().unwrap();
        // R1 : la machine de secours lit l'instantané et reconstruit son hub.
        let t = Instant::now();
        let mut lu = Vec::new();
        std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(&compresse[..]), &mut lu).unwrap();
        let etat: serde_json::Value = serde_json::from_slice(&lu).unwrap();
        let cible = Hub::new(HubOptions::default()).unwrap();
        spawn_driver(&cible, "maison".into(), Arc::new(Restaure(Arc::new(etat))), cancel.clone());
        while cible.stats().points < 2_264 || cible.state(&moli_core::PointId::new(&DeviceId::from("maison:0x00158d00000001ef"), "valeur_3")).is_none() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        let reprise = t.elapsed();
        println!(
            "- R1 : instantané de {} Kio compressé ; décompression, lecture et reconstruction du hub (496 appareils, 2 264 valeurs) : {:.0} ms",
            compresse.len() / 1024,
            reprise.as_secs_f64() * 1e3
        );
        cancel.cancel();
    });
    // R2 : journal des changements entre deux instantanés, au rythme réel.
    let par_jour = 163_387usize;
    let mut brut = Vec::new();
    let t = Instant::now();
    for i in 0..par_jour {
        // (série, décalage en ms, valeur) en texte compact, une ligne par changement.
        writeln!(brut, "{} {} {:.2}", (i * 7_919) % 1_487, i * 529, (i % 1_000) as f64 / 10.0).unwrap();
    }
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(&brut).unwrap();
    let compresse = gz.finish().unwrap();
    println!(
        "- R2 : journal d'une journée ({par_jour} changements) : {:.1} Mio brut, {:.1} Mio compressé, produit en {:.0} ms ; soit {:.0} octets par changement",
        brut.len() as f64 / 1_048_576.0,
        compresse.len() as f64 / 1_048_576.0,
        t.elapsed().as_secs_f64() * 1e3,
        compresse.len() as f64 / par_jour as f64
    );
}
```

## Annexe B — Données des deux instances

Comptes tirés des résumés et des agrégats d'une capture hors ligne : registres expurgés,
analyses du Recorder et activité par série. Aucune base n'a été déchiffrée pour cette
analyse ; aucun nom n'est repris. Les quantiles de changements par série et par jour
(1 487 séries) :

| Quantile | 10 % | 25 % | 50 % | 75 % | 90 % | 95 % | 99 % | max |
|---|---|---|---|---|---|---|---|---|
| Changements par jour | 0,3 | 0,3 | 0,7 | 5,1 | 36,5 | 285 | 2 876 | 12 458 |
