# Analyse V3 — Le pattern recommandé, tout en mémoire, et le coût de chaque choix

Date : 21 octobre 2026. Statut : **analyse, aucune modification du code**.
Base analysée : `main` au commit `bd1714d`. Cette V3 répond à trois questions : quel pattern
rend la maison réactive même quand des bus ou des appareils tombent, combien coûte de tout
garder en mémoire, et ce que coûte chaque choix. La [V1](isolation-reactivite-runtime.md) et la
[V2](isolation-reactivite-runtime-v2.md) restent valables et inchangées ; la V3 s'appuie sur
leurs mesures et sur la proposition [Valeurs, appareils et stockage V2](valeurs-appareils-stockage-v2.md),
avec laquelle elle reste cohérente (même étage « vif » en mémoire, mêmes mesures d'écritures).

Méthode : quatre nouveaux bancs, exécutés hors du dépôt contre les vraies crates `moli-runtime`
et `moli-automation` ([annexe A](#annexe-a--bancs-de-la-v3)) ; mesure des coûts de thread et de
processus sous Linux musl, la cible de production ; comptes agrégés des deux mêmes
Home Assistant (« instance A » et « instance B » : 496 appareils, 2 264 valeurs, ~163 000
relevés par jour à elles deux). Machine de test : Apple M4, runtime à 2 workers comme en
production ; les durées absolues ne se transposent pas au Pi Zero 2 W, les rapports si.

Un mot de vocabulaire : un **bus** désigne ici un transport partagé par plusieurs appareils
(broker MQTT, coordinateur Zigbee, pont Hue ou TaHoma, service cloud) ; le **bus d'événements**
est le canal interne du hub.

## 1. Réponse courte

**Le pattern : un noyau vif en mémoire, des cloisons à chaque frontière, la durabilité par un
journal.** Toute décision se prend sur l'état en mémoire, sans attendre ni un disque ni un
appareil. Chaque frontière (le thread d'un pilote, l'appareil, le bus, l'entrée du hub) a sa
cloison et sa règle quand ça déborde. Le disque ne sert qu'à ne rien perdre, par un journal en
ajout et des instantanés.

| Couche | Ce qu'elle garantit | Coût mesuré |
|---|---|---|
| 1. Un thread et un runtime par pilote | un pilote qui bloque ne bloque que lui | **19 Kio par pilote sous Linux musl** ; aucun surcoût de latence (0,01 ms) |
| 2. Voies paresseuses par appareil | un appareil muet ne retient que ses propres ordres | **6 Kio au repos pour toute la maison** (3,5 Mio en tâches permanentes) |
| 3. Disjoncteur par appareil et par bus | un bus mort fait échouer ses ordres en 0,1 ms au lieu de 4 s | quelques octets ; reprise 0,5 s après le retour du bus |
| 4. Tampon d'entrée du hub (dernière valeur gagne) | une inondation à la reconnexion ne fait plus perdre de déclencheurs | quelques Kio ; ≤ 10 ms de délai, pendant les rafales seulement |
| 5. Noyau vif en mémoire | aucune décision n'attend le disque | **2,35 Mio pour les deux maisons** ; avec 24 h d'historique 3,7 Mio, avec 7 jours 11 Mio |
| 6. Moteur compilé et durable | 3 à 11 ns par événement ; workflows temporaires repris après un redémarrage | ~0,5 Kio par automatisme, au lieu de 9,4 Kio aujourd'hui |
| 7. Durabilité par journal et instantanés | une reprise sans perte, ici ou sur la machine de secours | ~1,3 Mo par jour de journal compressé ; un instantané binaire en 0,3 ms |

Avec ce pattern, une panne reste à l'endroit où elle se produit :

| Panne | Aujourd'hui | Avec le pattern | Preuve |
|---|---|---|---|
| Un bus entier tombe 15 s | chaque ordre vers lui attend 4 s avant d'échouer | refus en 0,1 ms, appareils hors ligne, reprise 0,5 s après le retour ; les autres pilotes ne voient rien | P1 |
| Le bus revient et republie 5 000 valeurs | 5 déclencheurs sur 5 perdus, ~2 600 événements manqués par le moteur | aucun déclencheur perdu, latence 5 à 11 ms | P2 |
| Une lampe muette derrière un pont | 16 à 18 % des ordres aux lampes **saines** réussissent | 100 %, en 32 ms | V2 C2 |
| Deux pilotes qui bloquent leur thread | toute la maison à ~200 ms par pas | cœur à ≤ 0,1 ms | V2 C1 |
| La machine principale tombe | « allume 30 minutes » perdu, valeurs à refaire | reprise ailleurs, hub reconstruit en 11 à 14 ms, baux repris | V2 R1 |

## 2. Le pattern recommandé

### 2.1 Vue d'ensemble

```text
   appareils et services            frontières cloisonnées                   noyau vif (cœur)
 ┌──────────────────────┐   ┌──────────────────────────────────┐   ┌──────────────────────────────────┐
 │ bus Zigbee (MQTT)    │──►│ thread du pilote Zigbee2MQTT      │   │  état vif : valeurs, 3 instants, │
 │ pont Hue, TaHoma     │──►│   disjoncteur de bus              │   │  qualité, santé des appareils    │
 │ appareils Wi-Fi      │──►│   voies paresseuses par appareil  │══►│  moteur compilé : index, baux,   │
 │ services cloud       │──►│   budget, recul, cache            │   │  agrégats, historique récent     │
 └──────────────────────┘   └──────────────────────────────────┘   │  garde, journal, API, SSE        │
                                       ▲                ║           └───────────────┬──────────────────┘
                                       │  ordres        ║ états : tampon d'entrée    │ journal en ajout,
                                       └────────────────╫── (dernière valeur gagne)  │ instantanés
                                                        ▼                            ▼
                                                 entrée du hub              disque, puis secours
```

### 2.2 Les sept couches

1. **Un thread et un runtime par pilote.** Le superviseur actuel tourne sur un thread dédié au
   pilote. Un appel bloquant, un décodage coûteux (le XML de la V1, E8) ou une boucle sans
   attente ne retiennent plus que ce thread. Le hub reçoit à sa construction le `Handle` du
   runtime du cœur, et y lance ses propres tâches (V2, 4.2).
2. **Voies paresseuses par appareil.** Un seul répartiteur par pilote ; une file n'existe que
   pour un appareil qui a un ordre en attente ; un seul ordre en vol par appareil ; l'échéance
   est revérifiée juste avant l'envoi ; une nouvelle consigne remplace celle qui attend encore.
3. **Disjoncteurs.**
   - Par appareil : après deux échecs, refus immédiat et appareil hors ligne.
   - Par bus : trois échecs d'appareils différents en 10 s, c'est le bus. Tous ses appareils
     passent hors ligne et leurs ordres échouent tout de suite.
   - Un essai à délai court (500 ms) chaque seconde, ou mieux, le signal de reconnexion du
     transport (ConnAck MQTT, état du pont), referme le disjoncteur.
   - Derrière une API partagée (pont, cloud) : une passerelle plafonne la concurrence et
     répartit équitablement les places entre appareils (V2, C3).
4. **Tampon d'entrée du hub.** Quand un pilote publie plus vite que son budget, les états et
   les mesures sont fusionnés par point : la dernière valeur gagne, vidée en un lot. Les
   **occurrences** (appuis, sonnette, expiration de minuteur) ne sont jamais fusionnées. La
   minuterie de vidage n'est armée que si le tampon n'est pas vide : rien au repos.
5. **Noyau vif en mémoire.** Toute l'information utile à une décision vit en mémoire :
   - les valeurs, la précédente, trois instants (observé, reçu, changé) et la qualité ;
   - la santé de chaque appareil et de chaque bus ;
   - les automatismes compilés, leurs index, les baux, les exécutions en cours ;
   - les agrégats tenus au fil de l'eau, et l'historique récent selon le profil.

   Le détail chiffré est à la section 3.
6. **Moteur compilé et durable.** Plans compilés à la validation, déclencheurs indexés, horloge
   par échéance, agrégats incrémentaux, exécutions et baux persistés aux points d'attente
   (V2, 3.6). Il ne reçoit que les événements des points qu'il regarde. Sur un retard du bus
   d'événements, il recalcule les transitions au lieu d'écraser ce qu'il savait (V2, 3.3).
7. **Durabilité découplée.**
   - Un journal compact en ajout, validé par petits lots.
   - Un instantané binaire périodique de l'état vif.
   - L'état courant et l'historique dans les étages de la proposition stockage V2 :
     `maison.db` par lots de 60 s, `moli.db` pour le récent, l'archive pour le long terme.
   - Le journal et les instantanés partent aussi vers la machine de secours (V2, 6).

S'ajoute une **surveillance** transversale : détecteur de gel du runtime, vivacité des
pilotes, sentinelle des briques (V1), et santé par bus affichée à la famille.

### 2.3 Pourquoi ce pattern plutôt qu'un autre

| Pattern | Ce qu'il protège | Coût mesuré ou estimé | Verdict |
|---|---|---|---|
| Tout sur un runtime partagé (aujourd'hui) | les paniques des pilotes seulement | rien | insuffisant : deux blocages gèlent tout (V2 C1) |
| **Un thread par pilote** (recommandé) | les blocages et les calculs longs | **19 Kio par pilote** sous Linux musl, 52 Kio sous macOS | protège l'essentiel pour presque rien |
| Un processus par pilote | aussi le débordement de pile, la mémoire, `abort` | **577 Kio par processus minimal** sous Linux musl, 30 fois un thread, plus la sérialisation de chaque état et de chaque ordre entre processus (estimé) | à réserver à un ou deux pilotes à risque, jamais à tous sur un Pi |
| Bac à sable WASM par extension | aussi le code non relu | plusieurs Mio par instance et un calcul plus lent (estimé) ; écarté par D2 | seulement si des extensions non relues doivent tourner |
| Cadriciel d'acteurs (actix, ractor…) | comme les voies | une dépendance de plus, pas d'avantage mesurable sur des tâches Tokio et des files | inutile |

## 3. Tout en mémoire : le noyau vif

### 3.1 Ce que « toutes les données importantes » recouvre

| Famille | Contenu | Pourquoi en mémoire |
|---|---|---|
| Valeurs | valeur courante, précédente, observé / reçu / changé, qualité | une condition se lit en nanosecondes ; « depuis 10 min » se calcule sans disque |
| Appareils et bus | définitions, santé, disjoncteurs, files d'attente | un ordre vers un appareil mort échoue tout de suite |
| Workflows | plans compilés, index par point, échéancier, baux, exécutions en cours | un événement ne réveille que ce qui le concerne ; un bail ne s'oublie pas |
| Agrégats | moyennes, extrêmes, durées sur 1 h et 24 h des points lus par les règles | « moyenne sur 1 h » en 1 ns au lieu de 150 µs |
| Historique récent | 24 h, 7 jours ou 30 jours selon le profil, en colonnes compactes | courbes du tableau de bord instantanées, redémarrage à chaud |

### 3.2 Le budget mesuré

Allocateur qui compte chaque octet, forme réelle des deux maisons (B1).

| Élément | Les deux maisons (496 appareils, 2 264 valeurs, 400 automatismes) | ×10 (4 960 appareils, 22 640 valeurs, 4 000 automatismes) |
|---|---|---|
| Modèle du hub (ce qui existe aujourd'hui) | 1,00 Mio | 9,86 Mio |
| Étage vif : valeur précédente, trois instants, qualité, index des points | 434 Kio | 4,04 Mio |
| Santé par appareil | 12 Kio | 116 Kio |
| Automatismes compilés et index par point | 203 Kio | 1,98 Mio |
| Baux (1 000) et une exécution en cours par automatisme | 150 Kio | 1,56 Mio |
| Agrégats 1 h et 24 h des points lus par les règles | 443 Kio | 4,69 Mio |
| Voies paresseuses, au repos | 6 Kio | 10 Kio |
| Tampon du bus d'événements (1 024 événements) | 128 Kio | 128 Kio |
| **Sous-total : tout l'état vif** | **2,35 Mio** | **22,4 Mio** |
| Historique compact 24 h | 1,37 Mio | 14,3 Mio |
| Historique compact 7 jours | 8,78 Mio | 88,4 Mio |
| Historique compact 30 jours | 37,1 Mio | 379 Mio (extrapolé) |
| Threads des pilotes (40 pilotes, Linux musl) | 0,76 Mio | 0,76 Mio |

### 3.3 Trois profils

| Profil | Contenu | Total mesuré, deux maisons | Total ×10 | Matériel |
|---|---|---|---|---|
| **Sobre** | état vif + 24 h d'historique + threads | **~4,5 Mio** | ~38 Mio | Pi Zero 2 W (512 Mo) : tient dans le budget de 32 Mio de mémoire anonyme (PERFORMANCE.md) avec le reste du cœur |
| **Généreux** | état vif + 7 jours + threads | **~12 Mio** | ~112 Mio | Pi 4 ou 5, mini-PC ; pour ×10, au-delà du plafond actuel du conteneur (128 Mio) |
| **Maximal** | état vif + 30 jours + threads | **~40 Mio** | ~402 Mio | ≥ 2 Go ; dépasse le budget de 32 Mio du Pi |

Aujourd'hui, Moli occupe 4 à 7 Mio de mémoire anonyme (PERFORMANCE.md) et Home Assistant de
l'ordre de 2 Gio. Même le profil maximal reste 50 fois sous Home Assistant.

### 3.4 Ce que la mémoire rapporte

| Opération | Depuis le disque | Depuis la mémoire | Preuve |
|---|---|---|---|
| Lire « moyenne sur 1 h » | 150 à 190 µs (SQLite), plus un passage de thread | 1 ns (agrégat tenu au fil de l'eau) | V2 I2 |
| Courbe 24 h d'un capteur à 1 Hz, réduite à 500 points | 17 ms (SQLite) | 0,04 ms | B2.3 |
| Redémarrage à chaud avec 7 jours en mémoire | 76 ms pour relire 1,13 million de relevés depuis SQLite | 1,3 ms depuis un instantané binaire de 8,6 Mio | B2.3 |
| Trouver et évaluer les workflows d'un événement | 0,9 à 54 µs (balayage actuel) | 3 à 11 ns (compilé, indexé) | V2 I1 |

### 3.5 Ce qui reste sur disque

La mémoire sert la réactivité ; elle ne remplace pas la durabilité. Un étage ne garde jamais ce
qui revient au suivant, comme le pose la proposition stockage V2 :

| Étage | Rôle |
|---|---|
| Mémoire (noyau vif) | la vérité vive ; toutes les décisions |
| Journal en ajout | ne rien perdre entre deux instantanés ; la source de la reprise et de la réplication |
| `maison.db` | l'état courant complet, par lots de 60 s, suffit au démarrage |
| `moli.db` | le récent (fenêtres de D13) |
| Archive | le long terme, hors de la machine |

### 3.6 Le coût caché : les voies en tâches permanentes

Une voie par appareil en tâche permanente, avec sa file de 8, coûte **7,3 Kio par appareil** :
une file Tokio réserve d'avance un bloc de 32 places de 200 octets (`CommandRequest`), plus la
tâche. C'est 3,5 Mio pour les deux maisons et 35 Mio à ×10, le plus gros poste du budget. Les
voies paresseuses (un répartiteur, une file créée seulement quand un ordre attend) rendent la
même isolation pour 6 Kio au repos. C2 et C3 de la V2 ont mesuré cette isolation avec des
tâches ; avec des voies paresseuses, le comportement attendu est le même, à confirmer une fois
écrites dans `moli-runtime` (section 8).

## 4. Le coût de chaque choix

Mesuré sur la machine de test, sauf mention « estimé ». « Pi » : ordre de grandeur pour un
Pi Zero 2 W, 10 à 20 fois plus lent par cœur (estimé).

| Choix | Mémoire | CPU au repos | Coût par opération | Écritures par jour | Ce qu'on y gagne |
|---|---|---|---|---|---|
| Thread et runtime par pilote | 19 Kio par pilote (Linux musl) | nul (thread parqué) | aller-retour d'un ordre inchangé : 0,01 ms | — | le cœur ne gèle plus (V2 C1) |
| Processus par pilote | 577 Kio par processus minimal (Linux musl) | nul | sérialisation de chaque état et de chaque ordre (estimé) | — | aussi la pile et la mémoire ; trop cher pour tous les pilotes |
| Voies en tâches permanentes | 7,3 Kio par appareil | nul | ~0 | — | isolation par appareil |
| Voies paresseuses | 6 Kio pour toute la maison, +200 o par ordre en attente | nul | ~0 | — | la même isolation, sans le coût |
| Disjoncteur par appareil et par bus | quelques octets | un essai par seconde pendant une panne | refus en 0,1 ms au lieu de 4 s ; reprise 0,5 s après le retour (P1) | — | un bus mort ne fait plus attendre |
| Tampon d'entrée (dernière valeur gagne) | quelques Kio pendant une rafale | nul si la minuterie n'est armée qu'en rafale | ≤ 10 ms de délai pendant une rafale (P2) | — | plus aucun déclencheur perdu à la reconnexion |
| Étage vif étendu (précédente, trois instants, qualité) | 434 Kio | nul | quelques ns par mise à jour (estimé) | par `maison.db` : ~65 Mo par jour (stockage V2) | « depuis », « avant », fraîcheur en mémoire |
| Moteur compilé | ~0,5 Kio par automatisme (9,4 Kio aujourd'hui) | nul (horloge par échéance) | 3 à 11 ns par événement ; compilation 7 µs par automatisme | — | 280 à 5 000 fois moins de CPU par événement |
| Agrégats au fil de l'eau | 443 Kio | nul | 5 à 6 ns par relevé, 1 ns par lecture | — | des règles « moyenne, durée » sans disque |
| Historique 24 h en mémoire | 1,37 Mio | nul | quelques ns par relevé | — | courbes 24 h en 0,04 ms |
| Historique 7 jours en mémoire | 8,78 Mio | nul | idem | — | redémarrage à chaud en 1,3 ms |
| Historique 30 jours en mémoire | 37,1 Mio | nul | idem | — | courbes du mois sans disque |
| Instantané JSON complet chaque seconde | ~1 Mio transitoire | **0,68 % d'un cœur** ; Pi : 7 à 14 % (estimé) | 6,8 ms | **3,8 Go** s'il est écrit | à éviter |
| Instantané binaire de l'état vif chaque seconde | ~0,1 Mio transitoire | 0,03 % d'un cœur ; Pi : 0,3 à 0,6 % (estimé) | 0,30 ms | 1,2 Go s'il est écrit | acceptable vers le réseau, pas sur une carte SD |
| Instantané binaire toutes les 15 min + journal compact | négligeable | négligeable | 0,30 ms par instantané | **~1,3 Mo d'instantanés + ~1,3 Mo de journal** | la reprise sans perte, sans user la carte SD |
| Historique écrit selon D13 tel que décidé | — | — | — | ~3,1 Go (stockage V2, série `2026-10-21T114647Z`) | — |
| Historique en opérations compactes, lots de 2 s | — | — | — | ~0,22 à 0,26 Go (même série) | 12 fois moins d'écritures |

Trois règles en sortent :

1. **La mémoire est bon marché, les écritures ne le sont pas.** Garder 7 jours en mémoire coûte
   9 Mio ; écrire un instantané par seconde userait la carte SD de plusieurs Go par jour. On
   garde beaucoup en mémoire, on écrit peu, en ajout et par lots.
2. **Une cloison coûte quelques Kio ; une tâche permanente par appareil coûte des Mio.** On
   cloisonne par thread pour les pilotes, et de façon paresseuse pour les appareils.
3. **Le moteur compilé fait gagner sur les deux tableaux** : moins de mémoire par automatisme et
   beaucoup moins de CPU par événement.

## 5. Pannes : ce qui se passe, avec et sans le pattern

### 5.1 Un bus entier tombe (P1)

Un bus de 50 appareils meurt pendant 15 s. On envoie 10 ordres par seconde vers ses appareils
et 10 par seconde vers un autre pilote.

| | Autre pilote, p50 / p99 | Ordres au bus pendant la panne | Reprise après le retour du bus |
|---|---|---|---|
| Voies seules | 22,1 / 25,3 ms | médiane **4 002 ms** (147 ordres, tous en délai dépassé) | premier succès 108 ms après |
| Voies + disjoncteur de bus, essai à 500 ms | 22,1 / 23,0 ms | médiane **0,1 ms** (refus immédiat, appareils hors ligne) | premier succès 503 ms après |

Les voies suffisent déjà à protéger les **autres** pilotes : leur latence ne bouge pas (la
simulation de l'appareil sain coûte 20 ms). Le disjoncteur de bus protège les **workflows**.
Un ordre vers un bus mort échoue tout de suite, ce qui permet de prendre une autre branche
au lieu d'attendre 4 s par étape. Un premier essai attendait le délai complet de 4 s pour
sonder, et la reprise traînait à 4,6 s ; avec un essai à 500 ms, elle tombe à 0,5 s. Le signal
de reconnexion du transport la ramènerait à presque rien.

### 5.2 Le bus revient et inonde le hub (P2)

Le bus revient et republie 5 000 valeurs d'un coup sur 700 capteurs, puis 5 vrais déclencheurs
arrivent ; 400 automatismes réels tournent. Trois essais de chaque :

| | Déclencheurs perdus | Latence des déclencheurs | Retards du moteur |
|---|---|---|---|
| Sans tampon d'entrée | **5 sur 5, aux trois essais** | — | 2 retards, ~2 600 événements manqués |
| Avec tampon d'entrée (dernière valeur gagne, vidé toutes les 10 ms) | **0** | 4,9 à 10,8 ms | aucun |

C'est la combinaison du défaut de la V2 (3.3 : sur un retard, le moteur perd aussi ce qui reste
en file) et d'une inondation ordinaire, celle d'un pont qui se reconnecte. Le tampon d'entrée la
désamorce en amont : 5 000 valeurs sur 700 points deviennent ~700 événements. La correction du
moteur reste nécessaire, pour les rafales que rien ne fusionne.

### 5.3 Rappel des autres pannes mesurées

| Panne | Sans le pattern | Avec le pattern | Preuve |
|---|---|---|---|
| Une lampe muette derrière un pont | 16 à 18 % d'ordres sains réussis, médiane ~4 s | 100 %, médiane 32 ms | V2 C2 |
| API partagée limitée à 4 requêtes, une lampe muette | lampes saines à 1,4 s de médiane | 32 ms | V2 C3 |
| Deux pilotes bloquent leur thread | ~200 ms par pas pour toute la maison | ≤ 0,1 ms de médiane pour le cœur | V2 C1 |
| Un décodeur XML reçoit une réponse hostile | 96,6 s de CPU sur un worker | le même coût, sur le seul thread du pilote, et refusé par le pré-contrôle | V1 E8 |
| La machine principale tombe | état vif et baux perdus | hub reconstruit en 11 à 14 ms ailleurs | V2 R1 |

## 6. Les workflows quand quelque chose ne va pas

Le principe : **ce qui ne marche plus ne bloque jamais ce qui marche.**

1. **Chaque action rend un résultat typé** : faite, retenue par la garde, refusée, appareil
   injoignable (disjoncteur), délai dépassé. Un graphe peut brancher sur l'échec, comme
   `wait_for` branche déjà sur le délai. Par défaut, l'échec est journalisé et l'exécution
   continue ses autres branches.
2. **Un appareil injoignable répond en 0,1 ms**, pas en 4 s. Un workflow qui allume dix lampes
   dont trois sont sur un bus mort allume les sept autres tout de suite.
3. **Une condition sur un appareil hors ligne** lit une valeur de qualité « inconnue ». Elle est
   fausse par défaut, comme aujourd'hui (`holds` refuse une valeur nulle) ; un nœud peut aussi
   tester « inconnu » explicitement.
4. **Un ordre non exécuté à temps ne part jamais plus tard** : c'est une règle de l'architecture.
   Un workflow qui veut insister le dit (« réessayer 3 fois en 2 minutes »), avec une échéance.
5. **Les déclencheurs horaires, les baux et les minuteurs continuent** quand un bus tombe ; un
   bail échu pendant la panne s'applique au retour de l'appareil, s'il en est encore temps.
6. **Au retour d'un bus**, le tampon d'entrée fusionne la rafale : un « quand la température
   change » part une fois, sur la valeur finale, pas sept fois. Une occurrence (appui, sonnette)
   n'est jamais fusionnée : chacune part une fois.
7. **La famille voit la panne** : bus et appareils hors ligne sur le tableau de bord, étape « non
   faite : bus injoignable » dans l'exécution du workflow, notification si le workflow le
   demande.
8. **Le garde-fou d'emballement** ralentit (anti-rebond, fusion) au lieu d'éteindre
   l'automatisme au-delà de 20 lancements par minute (V2, 3.1).

## 7. Plan par étapes, avec ce que chaque étape coûte

Chaque étape apporte un gain seule ; l'ordre suit le rapport gain sur coût.

| Étape | Gain | Coût à l'exécution (mesuré) | Effort | Preuve d'acceptation |
|---|---|---|---|---|
| 1. Corriger la perte sur retard du moteur ; tâches du hub sur le `Handle` du cœur | plus de déclencheurs perdus ; horloges du hub hors des pilotes | nul | faible | V2 W2 rejoué : 0 perdu |
| 2. Tampon d'entrée du hub | inondations désamorcées | quelques Kio, ≤ 10 ms en rafale | faible | P2 rejoué : 0 perdu |
| 3. Voies paresseuses et disjoncteurs dans `moli-runtime` | appareils et bus cloisonnés | 6 Kio au repos | moyen | P1, V2 C2 et C3 rejoués |
| 4. Un thread par pilote | blocages et calculs isolés | 19 Kio par pilote | moyen | V2 C1 rejoué dans `moli-os` |
| 5. Bail durable pour « pendant N minutes » | workflows temporaires fiables | ~80 o par bail en mémoire ; 3,4 à 3,7 ms d'écriture au lancement | faible | redémarrage au milieu d'un bail |
| 6. Étage vif étendu et agrégats | conditions « depuis », moyennes sans disque | ~0,9 Mio | moyen | lecture d'agrégat en mémoire |
| 7. Moteur compilé, filtré à la source | 280 à 5 000 fois moins de CPU par événement | moins de mémoire qu'aujourd'hui | élevé | V2 W2 rejoué à 4 000 automatismes |
| 8. Historique récent en mémoire selon le profil | courbes et reprise instantanées | 1,4 Mio (24 h) à 37 Mio (30 j) | moyen | B2.3 rejoué |
| 9. Journal, instantanés binaires, secours passif | reprise sans perte, ici ou ailleurs | ~2,6 Mo écrits par jour | élevé | arrêt brutal, reprise sur la secours |

## 8. Limites

- Machine de test Apple M4 : les rapports entre options sont fiables, les durées absolues ne le
  sont pas pour le Pi. Les coûts de thread et de processus ont été mesurés sous Linux musl dans
  un conteneur arm64, pas sur un Pi.
- Les transports de P1 et P2 sont simulés : un bus qui ne répond plus du tout. Un bus qui
  répond mal (lenteur intermittente, réponses partielles) reste à jouer.
- Les voies paresseuses sont mesurées en mémoire ; leur comportement sous charge est celui des
  voies en tâches de la V2, à confirmer une fois écrites dans `moli-runtime`.
- Le coût d'un processus par pilote ne compte que le processus minimal ; un vrai pilote avec TLS
  et sa sérialisation coûte davantage.

## Annexe A — Bancs de la V3

Même crate jetable que la V2, **hors du dépôt**, avec quatre binaires de plus :
`etat_complet` (B1), `couts` (B2), `pannes` (P1, P2) et `veille` (processus minimal sous macOS).
La mesure Linux musl est un petit programme compilé et lancé dans un conteneur
`rust:1-alpine` jetable. Reproduire : `cargo run --release --bin <nom>` (`pannes p1` ne joue que
P1).

Sorties brutes :

```text
# Budget mémoire : tout ce qui compte, en mémoire
- tailles : Value 24 o, PointVif 80 o, Event 128 o
  (voies en tâches permanentes avec file de 8 : 3.53 Mio pour 496 appareils, soit 7.3 Kio chacune ; CommandRequest 200 o)

## ×1 : 496 appareils, 2264 valeurs, 400 automatismes
- Modèle du hub (appareils, définitions, valeurs : ce qui existe) : 1.00 Mio
- Étage vif : précédente, 3 instants, qualité + index des points : 434 Kio
- Santé par appareil (vu le, échecs, disjoncteur) : 12 Kio
- Automatismes compilés (9 règles, 2 actions) + index par point : 203 Kio
- 1 000 baux, échéancier et une exécution en cours par automatisme : 150 Kio
- Agrégats 1 h et 24 h des points lus par les règles : 443 Kio
- Voies paresseuses (répartiteur unique, files à la demande), au repos : 6 Kio
- Tampon du bus : 1 024 événements (charges partagées en Arc) : 128 Kio
- **Sous-total « tout l'état vif »** : 2.35 Mio
- Historique compact en mémoire : 24 h 1.37 Mio ; 7 j 8.78 Mio ; 30 j 37.10 Mio
- Totaux : vif + 24 h 3.71 Mio ; vif + 7 j 11.12 Mio ; vif + 30 j 39.44 Mio
  (voies en tâches permanentes avec file de 8 : 35.29 Mio pour 4960 appareils, soit 7.3 Kio chacune ; CommandRequest 200 o)

## ×10 : 4960 appareils, 22640 valeurs, 4000 automatismes
- Modèle du hub (appareils, définitions, valeurs : ce qui existe) : 9.86 Mio
- Étage vif : précédente, 3 instants, qualité + index des points : 4.04 Mio
- Santé par appareil (vu le, échecs, disjoncteur) : 116 Kio
- Automatismes compilés (9 règles, 2 actions) + index par point : 1.98 Mio
- 1 000 baux, échéancier et une exécution en cours par automatisme : 1.56 Mio
- Agrégats 1 h et 24 h des points lus par les règles : 4.69 Mio
- Voies paresseuses (répartiteur unique, files à la demande), au repos : 10 Kio
- Tampon du bus : 1 024 événements (charges partagées en Arc) : 128 Kio
- **Sous-total « tout l'état vif »** : 22.39 Mio
- Historique compact en mémoire : 24 h 14.33 Mio ; 7 j 88.42 Mio ; 30 j 378.95 Mio (extrapolé)
- Totaux : vif + 24 h 36.72 Mio ; vif + 7 j 110.81 Mio ; vif + 30 j 401.34 Mio

# Le coût de chaque choix (machine de test, 2 workers)

## B2.1 — instantané : coût selon le format et la fréquence (machine de test)
- JSON complet (valeurs + 400 automatismes), compressé : 6.80 ms par instantané, 43 Kio
- binaire compact des valeurs (étage vif), compressé : 0.30 ms par instantané, 13 Kio
| intervalle | CPU d'un cœur, JSON | CPU d'un cœur, compact | écrit par jour, JSON | écrit par jour, compact |
| 1 s | 0.680 % | 0.0299 % | 3837 Mo | 1207.1 Mo |
| 10 s | 0.068 % | 0.0030 % | 384 Mo | 120.7 Mo |
| 60 s | 0.011 % | 0.0005 % | 64 Mo | 20.1 Mo |
| 15 min | 0.001 % | 0.0000 % | 4 Mo | 1.3 Mo |

## B2.2 — ordre vers un pilote : même runtime ou thread isolé (2 000 ordres)
- pilote sur le runtime du cœur : p50 0.01 ms, p99 0.02 ms
- pilote sur son thread : p50 0.01 ms, p99 0.02 ms

## B2.3 — courbe 24 h et redémarrage à chaud : mémoire contre SQLite
- courbe 24 h d'un capteur à 1 Hz réduite à 500 points : mémoire 0.04 ms ; SQLite 17.07 ms (500 seaux)
- redémarrage à chaud, 7 jours en mémoire (1129783 relevés) : relus depuis SQLite 76.30 ms ; depuis un instantané binaire de 8.6 Mio 1.28 ms

## B2.4 — compiler les automatismes à la validation
- 2 000 automatismes lus depuis leur JSON et compilés : 14.04 ms au total, 7.0 µs chacun

# Pannes : un bus qui tombe, puis l'inondation à son retour

## P1 — un bus de 50 appareils mort 15 s (de 5 s à 20 s), 10 ordres/s vers lui, 10 ordres/s vers un autre pilote
- voies seules               : autre pilote p50 22.1 ms p99 25.3 ms ; ordres au bus pendant la panne : médiane 4001.7 ms (147 ordres) ; reprise : premier succès 107.9 ms après le retour du bus ; appareil du bus à la fin : Some(Some(true))
- voies + disjoncteur de bus : autre pilote p50 22.1 ms p99 23.0 ms ; ordres au bus pendant la panne : médiane 0.1 ms (147 ordres) ; reprise : premier succès 502.6 ms après le retour du bus ; appareil du bus à la fin : Some(Some(true))

## P2 — le bus revient et republie 5 000 valeurs d'un coup ; 400 automatismes réels
- sans tampon d'entrée                         : déclencheurs après l'inondation : 5 perdus sur 5, latence médiane — ; retards du moteur 2 (2626 événements manqués)
- avec tampon d'entrée (dernière valeur, 10 ms) : déclencheurs après l'inondation : 0 perdus sur 5, latence médiane 4.9 ms ; retards du moteur 0 (0 événements manqués)
- sans tampon d'entrée                         : déclencheurs après l'inondation : 5 perdus sur 5, latence médiane — ; retards du moteur 2 (2638 événements manqués)
- avec tampon d'entrée (dernière valeur, 10 ms) : déclencheurs après l'inondation : 0 perdus sur 5, latence médiane 10.1 ms ; retards du moteur 0 (0 événements manqués)
- sans tampon d'entrée                         : déclencheurs après l'inondation : 5 perdus sur 5, latence médiane — ; retards du moteur 2 (2614 événements manqués)
- avec tampon d'entrée (dernière valeur, 10 ms) : déclencheurs après l'inondation : 0 perdus sur 5, latence médiane 10.8 ms ; retards du moteur 0 (0 événements manqués)

- processus Rust minimal avec un runtime Tokio : 2102 Kio de RSS en moyenne (10 processus, machine de test)

- Linux musl : processus minimal avec un runtime Tokio : 577 Kio de RSS en moyenne
- Linux musl : 40 threads avec chacun un runtime Tokio : 788 Kio de RSS, soit 19 Kio par pilote
```

Codes sources :

Code complet de `src/bin/etat_complet.rs` :

```rust
//! B1 : le budget mémoire de « tout ce qui compte en mémoire », mesuré octet par octet
//! (allocateur qui compte), à l'échelle des deux maisons réunies puis ×10.

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

fn historique(jours: f64, echelle: usize) -> f64 {
    let avant = tas();
    let mut h: HashMap<u32, (Vec<u32>, Vec<f32>)> = HashMap::new();
    for s in 0..SERIES * echelle {
        let n = (taux(s) * jours).round() as usize;
        let (t, v) = h.entry(s as u32).or_default();
        t.reserve_exact(n);
        v.reserve_exact(n);
        for i in 0..n {
            t.push(i as u32);
            v.push(i as f32);
        }
    }
    let c = depuis(avant);
    drop(h);
    c
}

fn budget(echelle: usize) {
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
        // Variante paresseuse : un seul répartiteur, une file créée seulement quand un ordre
        // attend, au plus un ordre en vol par appareil. Au repos : une table vide.
        let avant = tas();
        let attente: HashMap<u32, VecDeque<CommandRequest>> = HashMap::with_capacity(64);
        let en_vol: Vec<bool> = vec![false; appareils];
        lignes.push(("Voies paresseuses (répartiteur unique, files à la demande), au repos", depuis(avant)));
        std::hint::black_box((&attente, &en_vol));
        println!("  (voies en tâches permanentes avec file de 8 : {} pour {appareils} appareils, soit {:.1} Kio chacune ; CommandRequest {} o)",
            kio(voies_taches), voies_taches / appareils as f64 / 1024.0, std::mem::size_of::<CommandRequest>());

        let bus = 1_024.0 * std::mem::size_of::<moli_core::Event>() as f64;
        lignes.push(("Tampon du bus : 1 024 événements (charges partagées en Arc)", bus));

        let indispensable: f64 = lignes.iter().map(|(_, o)| o).sum();
        let h24 = historique(1.0, echelle);
        let h7 = historique(7.0, echelle);
        let h30 = if echelle == 1 { historique(30.0, 1) } else { h7 / 7.0 * 30.0 };

        println!("\n## ×{echelle} : {appareils} appareils, {valeurs} valeurs, {automatismes} automatismes");
        for (nom, o) in &lignes {
            println!("- {nom} : {}", kio(*o));
        }
        println!("- **Sous-total « tout l'état vif »** : {}", kio(indispensable));
        println!("- Historique compact en mémoire : 24 h {} ; 7 j {} ; 30 j {}{}", kio(h24), kio(h7), kio(h30), if echelle == 1 { "" } else { " (extrapolé)" });
        println!(
            "- Totaux : vif + 24 h {} ; vif + 7 j {} ; vif + 30 j {}",
            kio(indispensable + h24),
            kio(indispensable + h7),
            kio(indispensable + h30)
        );
        std::hint::black_box((index, vifs, sante, plans, par_point, baux, executions, echeances, agregats, voies));
        cancel.cancel();
        taches.abort_all();
    });
}

fn main() {
    println!("# Budget mémoire : tout ce qui compte, en mémoire");
    println!("- tailles : Value {} o, PointVif {} o, Event {} o", std::mem::size_of::<Value>(), std::mem::size_of::<PointVif>(), std::mem::size_of::<moli_core::Event>());
    budget(1);
    budget(10);
}
```

Code complet de `src/bin/couts.rs` :

```rust
//! B2 : le coût de chaque choix, mesuré : instantané selon sa fréquence et son format,
//! saut de thread d'un pilote isolé, courbe 24 h en mémoire ou par SQLite, redémarrage
//! à chaud, compilation des automatismes.

use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};

use moli_core::{Access, Device, InstanceId, Kind, Origin, PointId, PointSpec, Semantic, Value};
use moli_runtime::{BoxFuture, Driver, DriverCtx, Hub, HubOptions, spawn_driver};
use tokio_util::sync::CancellationToken;

fn ms(d: Duration) -> String {
    format!("{:.2} ms", d.as_secs_f64() * 1e3)
}

struct Maison;
impl Driver for Maison {
    fn kind(&self) -> &'static str {
        "maison"
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
                        access: Access { read: true, write: k == 0 },
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
                    ctx.set_state(&id, &p.key, if n % 5 == 0 { Value::Text("idle".into()) } else { Value::Float(n as f64 / 7.0) });
                }
            }
            ctx.ready();
            while let Some(cmd) = ctx.next_command().await {
                cmd.reply(Ok(()));
            }
            Ok(())
        })
    }
}

fn graphe(k: usize) -> serde_json::Value {
    let p = |i: usize| format!("maison:0x00158d00{:08x}/valeur_0", i % 496);
    serde_json::json!({
        "name": format!("Automatisme {k}"), "mode": "restart",
        "nodes": [
            {"id":"t","type":"when_state","point":p(k)},
            {"id":"c1","type":"if","rules":[{"kind":"state","point":p(k+1),"op":"ge","value":0},{"kind":"state","point":p(k+2),"op":"ge","value":0},{"kind":"state","point":p(k+3),"op":"ge","value":0}]},
            {"id":"c2","type":"if","rules":[{"kind":"state","point":p(k+4),"op":"ge","value":0},{"kind":"state","point":p(k+5),"op":"ge","value":0},{"kind":"state","point":p(k+6),"op":"ge","value":0}]},
            {"id":"c3","type":"if","rules":[{"kind":"state","point":p(k+7),"op":"ge","value":0},{"kind":"state","point":p(k+8),"op":"ge","value":0},{"kind":"state","point":p(k+9),"op":"gt","value":1e9}]},
            {"id":"a","type":"toggle","point":p(k+10)}
        ],
        "edges": [{"from":"t","to":"c1"},{"from":"c1","port":"yes","to":"c2"},{"from":"c2","port":"yes","to":"c3"},{"from":"c3","port":"yes","to":"a"}]
    })
}

/// Encodage compact de l'étage vif : identifiant, étiquette, charge, trois instants.
fn compact(hub: &Hub) -> Vec<u8> {
    let mut o = Vec::with_capacity(2_264 * 40);
    let mut id = 0u32;
    for d in hub.snapshot().devices {
        for s in d.state.values() {
            o.extend_from_slice(&id.to_le_bytes());
            match &s.value {
                Value::Float(f) => {
                    o.push(1);
                    o.extend_from_slice(&f.to_le_bytes());
                }
                Value::Text(t) => {
                    o.push(2);
                    o.extend_from_slice(&(t.len() as u16).to_le_bytes());
                    o.extend_from_slice(t.as_bytes());
                }
                _ => o.push(0),
            }
            for _ in 0..3 {
                o.extend_from_slice(&s.ts.to_le_bytes());
            }
            id += 1;
        }
    }
    o
}

fn gz(b: &[u8]) -> usize {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    e.write_all(b).unwrap();
    e.finish().unwrap().len()
}

fn b1_instantanes(hub: &Hub) {
    println!("\n## B2.1 — instantané : coût selon le format et la fréquence (machine de test)");
    let automatismes: Vec<serde_json::Value> = (0..400).map(graphe).collect();
    let t = Instant::now();
    let mut taille_json = 0;
    for _ in 0..20 {
        let v = serde_json::to_vec(&serde_json::json!({"maison": hub.snapshot(), "automatismes": automatismes})).unwrap();
        taille_json = gz(&v);
    }
    let json = t.elapsed() / 20;
    let t = Instant::now();
    let mut taille_bin = 0;
    for _ in 0..200 {
        taille_bin = gz(&compact(hub));
    }
    let bin = t.elapsed() / 200;
    println!("- JSON complet (valeurs + 400 automatismes), compressé : {} par instantané, {} Kio", ms(json), taille_json / 1024);
    println!("- binaire compact des valeurs (étage vif), compressé : {} par instantané, {} Kio", ms(bin), taille_bin / 1024);
    println!("| intervalle | CPU d'un cœur, JSON | CPU d'un cœur, compact | écrit par jour, JSON | écrit par jour, compact |");
    for (nom, s) in [("1 s", 1.0f64), ("10 s", 10.0), ("60 s", 60.0), ("15 min", 900.0)] {
        let par_jour = 86_400.0 / s;
        println!(
            "| {nom} | {:.3} % | {:.4} % | {:.0} Mo | {:.1} Mo |",
            json.as_secs_f64() / s * 100.0,
            bin.as_secs_f64() / s * 100.0,
            taille_json as f64 * par_jour / 1e6,
            taille_bin as f64 * par_jour / 1e6
        );
    }
}

fn isole(hub: &Hub, nom: &str, pilote: Arc<dyn Driver>, cancel: CancellationToken) {
    let (hub, nom) = (hub.clone(), InstanceId::from(nom));
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async move {
            let _ = spawn_driver(&hub, nom, pilote, cancel).await;
        });
    });
}

struct Sain;
impl Driver for Sain {
    fn kind(&self) -> &'static str {
        "sain"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let id = ctx.device_id("l1");
            ctx.upsert_device(Device {
                id: id.clone(),
                instance: ctx.instance().clone(),
                native_name: "lampe".into(),
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
            ctx.ready();
            while let Some(cmd) = ctx.next_command().await {
                ctx.set_state(&cmd.device.id, &cmd.key, cmd.value.clone());
                cmd.reply(Ok(()));
            }
            Ok(())
        })
    }
}

async fn aller_retour(hub: &Hub, instance: &str) -> (Duration, Duration) {
    let p = PointId::from(format!("{instance}:l1/on"));
    while hub.state(&p).is_none() && hub.device(&p.split().unwrap().0).is_none() {
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    let mut v = Vec::with_capacity(2_000);
    for i in 0..2_000 {
        let t = Instant::now();
        hub.command(&p, Value::Bool(i % 2 == 0), Origin::Ui, None).await.unwrap();
        v.push(t.elapsed());
    }
    v.sort();
    (v[1_000], v[1_979])
}

fn b2_saut_de_thread() {
    println!("\n## B2.2 — ordre vers un pilote : même runtime ou thread isolé (2 000 ordres)");
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    rt.block_on(async {
        let hub = Hub::new(HubOptions::default()).unwrap();
        let cancel = CancellationToken::new();
        spawn_driver(&hub, "partage".into(), Arc::new(Sain), cancel.clone());
        isole(&hub, "isole", Arc::new(Sain), cancel.clone());
        tokio::time::sleep(Duration::from_millis(200)).await;
        let (p50, p99) = aller_retour(&hub, "partage").await;
        println!("- pilote sur le runtime du cœur : p50 {}, p99 {}", ms(p50), ms(p99));
        let (p50, p99) = aller_retour(&hub, "isole").await;
        println!("- pilote sur son thread : p50 {}, p99 {}", ms(p50), ms(p99));
        cancel.cancel();
    });
}

fn b3_courbe_et_reprise() {
    println!("\n## B2.3 — courbe 24 h et redémarrage à chaud : mémoire contre SQLite");
    // Un capteur qui change chaque seconde : 86 400 relevés en 24 h.
    let ts: Vec<u32> = (0..86_400).collect();
    let val: Vec<f32> = (0..86_400).map(|i| (i % 300) as f32 / 10.0).collect();
    let t = Instant::now();
    let mut sortie = Vec::new();
    for _ in 0..100 {
        sortie.clear();
        for seau in ts.chunks(173).zip(val.chunks(173)) {
            let (min, max, somme) = seau.1.iter().fold((f32::MAX, f32::MIN, 0.0f64), |(a, b, s), v| (a.min(*v), b.max(*v), s + f64::from(*v)));
            sortie.push((seau.0[0], min, max, somme / seau.1.len() as f64));
        }
    }
    let memoire = t.elapsed() / 100;
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE samples (point INTEGER, ts INTEGER, num REAL, PRIMARY KEY (point, ts)) WITHOUT ROWID;").unwrap();
    {
        let tx = conn.unchecked_transaction().unwrap();
        let mut ins = tx.prepare("INSERT INTO samples VALUES (?1, ?2, ?3)").unwrap();
        for p in 0..14i64 {
            for s in 0..86_400i64 {
                ins.execute((p, s * 1000, (s % 300) as f64 / 10.0)).unwrap();
            }
        }
        drop(ins);
        tx.commit().unwrap();
    }
    let mut req = conn
        .prepare("SELECT ts / 173000, min(num), max(num), avg(num) FROM samples WHERE point = ?1 AND ts >= 0 AND ts < 86400000 GROUP BY ts / 173000")
        .unwrap();
    let t = Instant::now();
    let mut lignes = 0;
    for i in 0..20i64 {
        lignes = req.query_map([i % 14], |r| r.get::<_, i64>(0)).unwrap().count();
    }
    let sql = t.elapsed() / 20;
    println!(
        "- courbe 24 h d'un capteur à 1 Hz réduite à {} points : mémoire {} ; SQLite {} ({} seaux)",
        sortie.len(),
        ms(memoire),
        ms(sql),
        lignes
    );
    // Redémarrage à chaud : recharger 7 jours de toute la maison (1,13 million de relevés).
    let total: usize = 1_129_783;
    let par_point = total / 1_487 + 1;
    {
        let tx = conn.unchecked_transaction().unwrap();
        tx.execute_batch("DELETE FROM samples").unwrap();
        let mut ins = tx.prepare("INSERT INTO samples VALUES (?1, ?2, ?3)").unwrap();
        let mut n = 0;
        'fin: for p in 0..1_487i64 {
            for s in 0..par_point as i64 {
                ins.execute((p, s * 1000, s as f64)).unwrap();
                n += 1;
                if n == total {
                    break 'fin;
                }
            }
        }
        drop(ins);
        tx.commit().unwrap();
    }
    let t = Instant::now();
    let mut colonnes: HashMap<i64, (Vec<u32>, Vec<f32>)> = HashMap::new();
    let mut tout = conn.prepare("SELECT point, ts, num FROM samples").unwrap();
    let mut lignes = tout.query([]).unwrap();
    while let Some(r) = lignes.next().unwrap() {
        let (p, ts, v): (i64, i64, f64) = (r.get(0).unwrap(), r.get(1).unwrap(), r.get(2).unwrap());
        let c = colonnes.entry(p).or_default();
        c.0.push((ts / 1000) as u32);
        c.1.push(v as f32);
    }
    let depuis_sql = t.elapsed();
    // Le même contenu écrit en binaire compact, puis relu.
    let mut binaire = Vec::with_capacity(total * 8 + 1_487 * 12);
    for (p, (t, v)) in &colonnes {
        binaire.extend_from_slice(&(*p as u32).to_le_bytes());
        binaire.extend_from_slice(&(t.len() as u32).to_le_bytes());
        for x in t {
            binaire.extend_from_slice(&x.to_le_bytes());
        }
        for x in v {
            binaire.extend_from_slice(&x.to_le_bytes());
        }
    }
    let t = Instant::now();
    let mut relu: HashMap<u32, (Vec<u32>, Vec<f32>)> = HashMap::new();
    let mut i = 0;
    while i < binaire.len() {
        let p = u32::from_le_bytes(binaire[i..i + 4].try_into().unwrap());
        let n = u32::from_le_bytes(binaire[i + 4..i + 8].try_into().unwrap()) as usize;
        i += 8;
        let t: Vec<u32> = binaire[i..i + 4 * n].chunks_exact(4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).collect();
        i += 4 * n;
        let v: Vec<f32> = binaire[i..i + 4 * n].chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect();
        i += 4 * n;
        relu.insert(p, (t, v));
    }
    let depuis_bin = t.elapsed();
    println!(
        "- redémarrage à chaud, 7 jours en mémoire ({} relevés) : relus depuis SQLite {} ; depuis un instantané binaire de {:.1} Mio {}",
        total,
        ms(depuis_sql),
        binaire.len() as f64 / 1_048_576.0,
        ms(depuis_bin)
    );
    std::hint::black_box(relu);
}

fn b4_compilation() {
    println!("\n## B2.4 — compiler les automatismes à la validation");
    let graphes: Vec<String> = (0..2_000).map(|k| graphe(k).to_string()).collect();
    let t = Instant::now();
    let mut points: HashMap<String, u32> = HashMap::new();
    let mut plans: Vec<Vec<(u32, f64)>> = Vec::with_capacity(graphes.len());
    for g in &graphes {
        let v: serde_json::Value = serde_json::from_str(g).unwrap();
        let mut regles = Vec::new();
        for n in v["nodes"].as_array().unwrap() {
            if let Some(rs) = n["rules"].as_array() {
                for r in rs {
                    let nom = r["point"].as_str().unwrap().to_owned();
                    let suivant = points.len() as u32;
                    let id = *points.entry(nom).or_insert(suivant);
                    regles.push((id, r["value"].as_f64().unwrap()));
                }
            }
        }
        plans.push(regles);
    }
    let duree = t.elapsed();
    println!(
        "- 2 000 automatismes lus depuis leur JSON et compilés : {} au total, {:.1} µs chacun",
        ms(duree),
        duree.as_secs_f64() * 1e6 / 2_000.0
    );
    std::hint::black_box(plans);
}

fn main() {
    println!("# Le coût de chaque choix (machine de test, 2 workers)");
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    rt.block_on(async {
        let hub = Hub::new(HubOptions::default()).unwrap();
        let cancel = CancellationToken::new();
        spawn_driver(&hub, "maison".into(), Arc::new(Maison), cancel.clone());
        while hub.stats().points < 2_264 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        b1_instantanes(&hub);
        cancel.cancel();
    });
    b2_saut_de_thread();
    b3_courbe_et_reprise();
    b4_compilation();
}
```

Code complet de `src/bin/pannes.rs` :

```rust
//! P1-P2 : un bus entier qui tombe (broker, coordinateur, pont), puis l'inondation quand il
//! revient. Mesuré contre le vrai `moli-runtime` et le vrai moteur `moli-automation`.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use moli_automation::{Actor, Author, Automation, Automations, Config, Graph, Mode};
use moli_core::{Access, Device, DeviceId, InstanceId, Kind, Origin, PointId, PointSpec, Semantic, Value};
use moli_runtime::{BoxFuture, CommandRequest, Driver, DriverCtx, Hub, HubOptions, spawn_driver};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::layer::SubscriberExt;

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap()
}
fn ms(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1e3)
}
fn q(v: &mut [Duration], p: f64) -> String {
    if v.is_empty() {
        return "—".into();
    }
    v.sort();
    ms(v[((v.len() - 1) as f64 * p) as usize])
}
fn lampe(id: DeviceId, instance: &InstanceId, cle: &str, kind: Kind, ecriture: bool) -> Device {
    Device {
        id,
        instance: instance.clone(),
        native_name: "fictif".into(),
        manufacturer: None,
        model: None,
        description: None,
        native_room: None,
        members: vec![],
        points: vec![PointSpec {
            key: cle.into(),
            label: cle.into(),
            kind,
            access: Access { read: true, write: ecriture },
            unit: None,
            semantic: Semantic::Other,
        }],
    }
}

// ---- P1 : un bus entier tombe ----------------------------------------------------

const APPAREILS: usize = 50;

/// État partagé du transport (le broker, le coordinateur Zigbee, le pont).
struct Transport {
    vivant: AtomicBool,
}

#[derive(Default)]
struct Disjoncteur {
    echecs: VecDeque<Instant>,
    ouvert_jusqua: Option<Instant>,
    sonde: bool,
}

struct Bus {
    transport: Arc<Transport>,
    disjoncteur_de_bus: bool,
}

async fn requete(t: &Transport) -> Result<(), String> {
    // Une requête envoyée sur un transport mort n'aboutit jamais : elle tombe au délai de
    // 4 s de moli-net, même si le transport revient entre-temps (la connexion est perdue).
    if t.vivant.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(20)).await;
        Ok(())
    } else {
        tokio::time::sleep(Duration::from_secs(4)).await;
        Err("délai dépassé".into())
    }
}

impl Driver for Bus {
    fn kind(&self) -> &'static str {
        "bus"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            for i in 0..APPAREILS {
                let id = ctx.device_id(&format!("a{i}"));
                ctx.upsert_device(lampe(id.clone(), &ctx.instance().clone(), "on", Kind::Binary, true));
                ctx.set_availability(&id, true);
            }
            ctx.ready();
            let etat = Arc::new(Mutex::new(Disjoncteur::default()));
            let (dispo_tx, mut dispo_rx) = mpsc::unbounded_channel::<bool>();
            let mut voies = Vec::new();
            let mut taches = tokio::task::JoinSet::new();
            for _ in 0..APPAREILS {
                let (tx, mut rx) = mpsc::channel::<CommandRequest>(8);
                voies.push(tx);
                let (transport, etat, dispo_tx, actif) = (self.transport.clone(), etat.clone(), dispo_tx.clone(), self.disjoncteur_de_bus);
                taches.spawn(async move {
                    while let Some(cmd) = rx.recv().await {
                        if cmd.abandoned() {
                            cmd.reply(Err("expiré avant l'envoi".into()));
                            continue;
                        }
                        let mut sonde = false;
                        if actif {
                            let mut e = etat.lock().unwrap();
                            if let Some(t) = e.ouvert_jusqua {
                                if Instant::now() < t || e.sonde {
                                    drop(e);
                                    cmd.reply(Err("bus injoignable".into()));
                                    continue;
                                }
                                e.sonde = true; // une seule requête d'essai
                                sonde = true;
                            }
                        }
                        // L'essai du disjoncteur a un délai court : la reprise ne doit pas
                        // attendre le délai complet d'une requête.
                        let r = if sonde {
                            tokio::time::timeout(Duration::from_millis(500), requete(&transport))
                                .await
                                .unwrap_or_else(|_| Err("essai sans réponse".into()))
                        } else {
                            requete(&transport).await
                        };
                        if actif {
                            let mut e = etat.lock().unwrap();
                            let maintenant = Instant::now();
                            if r.is_ok() {
                                if e.ouvert_jusqua.take().is_some() {
                                    let _ = dispo_tx.send(true);
                                }
                                e.echecs.clear();
                            } else {
                                e.echecs.push_back(maintenant);
                                while e.echecs.front().is_some_and(|t| maintenant - *t > Duration::from_secs(10)) {
                                    e.echecs.pop_front();
                                }
                                // Trois échecs sur des appareils du bus en 10 s : c'est le bus.
                                if e.echecs.len() >= 3 || sonde {
                                    if e.ouvert_jusqua.is_none() {
                                        let _ = dispo_tx.send(false);
                                    }
                                    e.ouvert_jusqua = Some(maintenant + Duration::from_secs(1));
                                }
                            }
                            if sonde {
                                e.sonde = false;
                            }
                        }
                        cmd.reply(r);
                    }
                });
            }
            loop {
                tokio::select! {
                    cmd = ctx.next_command() => {
                        let Some(cmd) = cmd else { break };
                        let i: usize = cmd.device.id.as_str()["bus:a".len()..].parse().unwrap();
                        if let Err(e) = voies[i].try_send(cmd) {
                            let c = match e { mpsc::error::TrySendError::Full(c) | mpsc::error::TrySendError::Closed(c) => c };
                            c.reply(Err("file de l'appareil pleine".into()));
                        }
                    }
                    Some(en_ligne) = dispo_rx.recv() => {
                        for i in 0..APPAREILS {
                            ctx.set_availability(&ctx.device_id(&format!("a{i}")), en_ligne);
                        }
                    }
                    fin = taches.join_next() => anyhow::bail!("voie arrêtée : {fin:?}"),
                }
            }
            Ok(())
        })
    }
}

struct Sain;
impl Driver for Sain {
    fn kind(&self) -> &'static str {
        "sain"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let id = ctx.device_id("l1");
            ctx.upsert_device(lampe(id, &ctx.instance().clone(), "on", Kind::Binary, true));
            ctx.ready();
            while let Some(cmd) = ctx.next_command().await {
                tokio::time::sleep(Duration::from_millis(20)).await;
                cmd.reply(Ok(()));
            }
            Ok(())
        })
    }
}

fn p1(disjoncteur: bool) {
    rt().block_on(async {
        let hub = Hub::new(HubOptions::default()).unwrap();
        let cancel = CancellationToken::new();
        let transport = Arc::new(Transport { vivant: AtomicBool::new(true) });
        spawn_driver(&hub, "bus".into(), Arc::new(Bus { transport: transport.clone(), disjoncteur_de_bus: disjoncteur }), cancel.clone());
        spawn_driver(&hub, "sain".into(), Arc::new(Sain), cancel.clone());
        while hub.device(&DeviceId::from("bus:a49")).is_none() || hub.device(&DeviceId::from("sain:l1")).is_none() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let debut = Instant::now();
        let sain = Arc::new(Mutex::new(Vec::new()));
        let panne = Arc::new(Mutex::new(Vec::new()));
        let premier_succes = Arc::new(Mutex::new(None::<Duration>));
        let mut taches = Vec::new();
        // 30 s : bus vivant 5 s, mort 15 s, vivant 10 s ; 10 ordres/s vers le bus, 10/s ailleurs.
        for n in 0..300usize {
            let t = debut.elapsed();
            transport.vivant.store(!(Duration::from_secs(5)..Duration::from_secs(20)).contains(&t), Ordering::Release);
            let (h, panne, premier) = (hub.clone(), panne.clone(), premier_succes.clone());
            taches.push(tokio::spawn(async move {
                let p = PointId::from(format!("bus:a{}/on", (n * 7) % APPAREILS));
                let t0 = Instant::now();
                let r = h.command(&p, Value::Bool(n % 2 == 0), Origin::Automation, None).await;
                let d = t0.elapsed();
                if (Duration::from_secs(5)..Duration::from_secs(20)).contains(&t) {
                    panne.lock().unwrap().push(d);
                } else if t >= Duration::from_secs(20) && r.is_ok() {
                    let mut f = premier.lock().unwrap();
                    if f.is_none() {
                        *f = Some(t + d - Duration::from_secs(20));
                    }
                }
            }));
            let (h, sain) = (hub.clone(), sain.clone());
            taches.push(tokio::spawn(async move {
                let t0 = Instant::now();
                let _ = h.command(&PointId::from("sain:l1/on"), Value::Bool(n % 2 == 0), Origin::Automation, None).await;
                sain.lock().unwrap().push(t0.elapsed());
            }));
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        for t in taches {
            let _ = t.await;
        }
        let hors_ligne = hub.device(&DeviceId::from("bus:a3")).map(|d| d.online);
        let mut s = sain.lock().unwrap().clone();
        let mut p = panne.lock().unwrap().clone();
        println!(
            "- {} : autre pilote p50 {} p99 {} ; ordres au bus pendant la panne : médiane {} ({} ordres) ; reprise : premier succès {} après le retour du bus ; appareil du bus à la fin : {:?}",
            if disjoncteur { "voies + disjoncteur de bus" } else { "voies seules              " },
            q(&mut s, 0.5),
            q(&mut s, 0.99),
            q(&mut p, 0.5),
            p.len(),
            premier_succes.lock().unwrap().map_or("—".into(), ms),
            hors_ligne
        );
        cancel.cancel();
    });
}

// ---- P2 : l'inondation au retour du bus, contre le vrai moteur -------------------

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

const CAPTEURS: usize = 1_800;
const LAMPES: usize = 400;

struct Maison {
    arrivees: Arc<Mutex<Vec<(usize, Instant)>>>,
    injections: Mutex<Option<mpsc::UnboundedReceiver<(usize, f64)>>>,
    /// Tampon d'entrée : la dernière valeur de chaque point gagne, vidé toutes les 10 ms.
    coalescence: bool,
}
fn capteur(s: usize) -> (DeviceId, String) {
    (DeviceId::from(format!("maison:c{}", s / 5)), format!("v{}", s % 5))
}
impl Driver for Maison {
    fn kind(&self) -> &'static str {
        "maison"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let instance = ctx.instance().clone();
            for d in 0..CAPTEURS / 5 {
                let id = DeviceId::from(format!("maison:c{d}"));
                let mut dev = lampe(id.clone(), &instance, "v0", Kind::Numeric { min: None, max: None, step: None }, false);
                dev.points = (0..5)
                    .map(|k| PointSpec {
                        key: format!("v{k}").into(),
                        label: "v".into(),
                        kind: Kind::Numeric { min: None, max: None, step: None },
                        access: Access { read: true, write: false },
                        unit: None,
                        semantic: Semantic::Other,
                    })
                    .collect();
                ctx.upsert_device(dev);
                for k in 0..5 {
                    ctx.set_state(&id, &format!("v{k}"), Value::Float(0.0));
                }
            }
            for k in 0..LAMPES {
                let id = DeviceId::from(format!("maison:l{k}"));
                ctx.upsert_device(lampe(id.clone(), &instance, "on", Kind::Binary, true));
                ctx.set_state(&id, "on", Value::Bool(false));
            }
            let mut rx = self.injections.lock().unwrap().take().unwrap();
            let mut tampon: HashMap<usize, f64> = HashMap::new();
            let mut vidage = tokio::time::interval(Duration::from_millis(10));
            ctx.ready();
            loop {
                tokio::select! {
                    cmd = ctx.next_command() => {
                        let Some(cmd) = cmd else { return Ok(()) };
                        let k: usize = cmd.device.id.as_str()["maison:l".len()..].parse().unwrap();
                        self.arrivees.lock().unwrap().push((k, Instant::now()));
                        ctx.set_state(&cmd.device.id, &cmd.key, cmd.value.clone());
                        cmd.reply(Ok(()));
                    }
                    inj = rx.recv() => {
                        let Some((s, v)) = inj else { return Ok(()) };
                        if self.coalescence {
                            tampon.insert(s, v);
                        } else {
                            let (id, key) = capteur(s);
                            ctx.set_state(&id, &key, Value::Float(v));
                        }
                    }
                    _ = vidage.tick(), if self.coalescence => {
                        for (s, v) in tampon.drain() {
                            let (id, key) = capteur(s);
                            ctx.set_state(&id, &key, Value::Float(v));
                        }
                    }
                }
            }
        })
    }
}

fn declencheur(k: usize) -> usize {
    100 + k
}

fn p2(coalescence: bool, fois: &Arc<AtomicUsize>, manques: &Arc<AtomicU64>) {
    rt().block_on(async {
        let hub = Hub::new(HubOptions::default()).unwrap();
        let cancel = CancellationToken::new();
        let arrivees = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = mpsc::unbounded_channel();
        spawn_driver(&hub, "maison".into(), Arc::new(Maison { arrivees: arrivees.clone(), injections: Mutex::new(Some(rx)), coalescence }), cancel.clone());
        while hub.state(&PointId::from(format!("maison:l{}/on", LAMPES - 1))).is_none() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let (autos, _t) = Automations::start(&hub, Config::default(), None, cancel.clone()).await.unwrap();
        let humain = Actor { human: true, author: Author::Human };
        for k in 0..LAMPES {
            let (d, key) = capteur(declencheur(k));
            let regles: Vec<serde_json::Value> = (0..3)
                .map(|j| {
                    let (cd, ck) = capteur(1_000 + (k * 7 + j * 13) % 690);
                    serde_json::json!({"kind":"state","point":format!("{cd}/{ck}"),"op":"ge","value":0})
                })
                .collect();
            let g = serde_json::json!({
                "nodes":[{"id":"t","type":"when_state","point":format!("{d}/{key}")},
                         {"id":"c","type":"if","rules":regles},
                         {"id":"a","type":"toggle","point":format!("maison:l{k}/on")}],
                "edges":[{"from":"t","to":"c"},{"from":"c","port":"yes","to":"a"}]
            });
            let a = autos
                .save(
                    Automation {
                        id: String::new(),
                        name: format!("A{k}"),
                        enabled: true,
                        mode: Mode::Restart,
                        graph: serde_json::from_value::<Graph>(g).unwrap(),
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
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        fois.store(0, Ordering::Relaxed);
        manques.store(0, Ordering::Relaxed);
        let depuis = arrivees.lock().unwrap().len();
        // Le bus revient : 5 000 valeurs republiées d'un coup sur 700 capteurs (7 chacun),
        // puis cinq vrais déclencheurs.
        let mut v = 1.0;
        for i in 0..5_000usize {
            v += 1.0;
            tx.send((1_000 + (i * 7_919) % 700, v)).unwrap();
        }
        let mut sondes = Vec::new();
        for j in 0..5 {
            v += 1.0;
            let k = 20 + 17 * j;
            sondes.push((k, Instant::now()));
            tx.send((declencheur(k), v)).unwrap();
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
        let arr = arrivees.lock().unwrap();
        let mut lat = Vec::new();
        let mut perdues = 0;
        for (k, t0) in &sondes {
            match arr[depuis..].iter().find(|(a, t)| a == k && t >= t0) {
                Some((_, t)) => lat.push(*t - *t0),
                None => perdues += 1,
            }
        }
        println!(
            "- {} : déclencheurs après l'inondation : {} perdus sur 5, latence médiane {} ; retards du moteur {} ({} événements manqués)",
            if coalescence { "avec tampon d'entrée (dernière valeur, 10 ms)" } else { "sans tampon d'entrée                        " },
            perdues,
            q(&mut lat, 0.5),
            fois.load(Ordering::Relaxed),
            manques.load(Ordering::Relaxed)
        );
        cancel.cancel();
    });
}

fn main() {
    let fois = Arc::new(AtomicUsize::new(0));
    let manques = Arc::new(AtomicU64::new(0));
    tracing::subscriber::set_global_default(tracing_subscriber::registry().with(Retards { fois: fois.clone(), manques: manques.clone() })).unwrap();
    println!("# Pannes : un bus qui tombe, puis l'inondation à son retour");
    println!("\n## P1 — un bus de 50 appareils mort 15 s (de 5 s à 20 s), 10 ordres/s vers lui, 10 ordres/s vers un autre pilote");
    p1(false);
    p1(true);
    if std::env::args().nth(1).as_deref() == Some("p1") {
        return;
    }
    println!("\n## P2 — le bus revient et republie 5 000 valeurs d'un coup ; 400 automatismes réels");
    for _ in 0..3 {
        p2(false, &fois, &manques);
        p2(true, &fois, &manques);
    }
}
```

Code complet de `src/bin/veille.rs` :

```rust
//! B3 : la mémoire résidente d'un processus Rust minimal avec un runtime Tokio,
//! l'ordre de grandeur d'« un processus par pilote ».
fn main() {
    if std::env::args().nth(1).as_deref() == Some("enfant") {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async { tokio::time::sleep(std::time::Duration::from_secs(3)).await });
        return;
    }
    let mut enfants: Vec<std::process::Child> = (0..10)
        .map(|_| std::process::Command::new(std::env::current_exe().unwrap()).arg("enfant").spawn().unwrap())
        .collect();
    std::thread::sleep(std::time::Duration::from_secs(1));
    let mut total = 0u64;
    for e in &enfants {
        let out = std::process::Command::new("ps").args(["-o", "rss=", "-p", &e.id().to_string()]).output().unwrap();
        total += String::from_utf8_lossy(&out.stdout).trim().parse::<u64>().unwrap_or(0);
    }
    println!("- processus Rust minimal avec un runtime Tokio : {} Kio de RSS en moyenne (10 processus, machine de test)", total / 10);
    for e in &mut enfants {
        let _ = e.wait();
    }
}
```

Code complet de `linux-couts/src/main.rs (conteneur rust:1-alpine)` :

```rust
//! Linux musl : RSS d'un processus minimal par pilote, et d'un thread + runtime par pilote.
fn rss_kio(pid: u32) -> u64 {
    std::fs::read_to_string(format!("/proc/{pid}/status")).ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("VmRSS:")).map(|l| l.split_whitespace().nth(1).unwrap().parse().unwrap()))
        .unwrap_or(0)
}
fn main() {
    if std::env::args().nth(1).as_deref() == Some("enfant") {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async { tokio::time::sleep(std::time::Duration::from_secs(3)).await });
        return;
    }
    let mut enfants: Vec<std::process::Child> = (0..10)
        .map(|_| std::process::Command::new(std::env::current_exe().unwrap()).arg("enfant").spawn().unwrap())
        .collect();
    std::thread::sleep(std::time::Duration::from_secs(1));
    let total: u64 = enfants.iter().map(|e| rss_kio(e.id())).sum();
    println!("- Linux musl : processus minimal avec un runtime Tokio : {} Kio de RSS en moyenne", total / 10);
    for e in &mut enfants { let _ = e.wait(); }
    let moi = std::process::id();
    let avant = rss_kio(moi);
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let rx = std::sync::Arc::new(std::sync::Mutex::new(rx));
    let mut fils = Vec::new();
    for _ in 0..40 {
        let rx = rx.clone();
        fils.push(std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            rt.block_on(async { tokio::time::sleep(std::time::Duration::from_secs(2)).await });
            drop(rx);
        }));
    }
    std::thread::sleep(std::time::Duration::from_millis(500));
    let apres = rss_kio(moi);
    println!("- Linux musl : 40 threads avec chacun un runtime Tokio : {} Kio de RSS, soit {} Kio par pilote", apres - avant, (apres - avant) / 40);
    drop(tx);
    for f in fils { let _ = f.join(); }
}
```
