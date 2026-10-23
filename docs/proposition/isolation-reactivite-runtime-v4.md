# Analyse V4 — Approfondissement : en conditions de production, bus capricieux, correctif prouvé, bascule jouée

Date : 22 octobre 2026. Statut : **analyse, aucune modification du code**.
Base analysée : `main` au commit `bd1714d`. La V4 ferme les limites que la
[V3](isolation-reactivite-runtime-v3.md) avait elle-même listées ; la [V1](isolation-reactivite-runtime.md),
la [V2](isolation-reactivite-runtime-v2.md) et la V3 restent valables et inchangées.

| Limite de la V3 | Ce que fait la V4 |
|---|---|
| Mesures sur un Mac, pas en production | Les mêmes bancs compilés en **Linux musl** (comme l'image) et exécutés dans un conteneur limité à **2 CPU et 128 Mio**, avec la mémoire anonyme du cgroup (la mesure de PERFORMANCE.md) |
| Un bus simulé mort ou vivant, jamais « mal » | Trois bus capricieux : **lent** (sous le délai), **qui bat** (tombe et revient), **partiel** (une requête sur deux perdue) ; quatre protections comparées |
| Voies paresseuses mesurées en mémoire seulement | Les voies paresseuses **écrites pour de bon** et rejouées sous la charge de la V2 et de la V3 |
| Correctif de la perte sur retard seulement décrit | Le correctif **appliqué sur une copie du moteur hors dépôt** (20 lignes) et rejoué sous la rafale |
| Bascule mesurée par morceaux | La bascule **jouée de bout en bout** : deux processus, la machine principale gelée, la secours reprend, l'ancienne revient et reste passive |
| Pas de plafond chiffré | Le **débit soutenu** au-delà duquel le moteur actuel décroche, avec et sans tampon d'entrée |
| Pas de modèle de coût | Des **coefficients unitaires** pour chiffrer n'importe quelle maison, et les contrats des briques |

Méthode : quatre crates jetables **hors du dépôt**, qui dépendent par chemin des vraies crates
`moli-runtime`, `moli-core` et `moli-automation` (annexe A) ; deux machines de mesure : le Mac
(Apple M4, « Mac ») et un conteneur Linux musl arm64 limité à 2 CPU et 128 Mio (« Linux »), plus
proche de la production ; le Pi Zero 2 W est estimé **8 à 10 fois plus lent** que le conteneur
(estimé, non mesuré). Comptes agrégés des deux mêmes Home Assistant (« instance A » et
« instance B »), aucun nom repris.

## 1. Réponse courte

Le pattern de la V3 tient en conditions de production, et deux de ses pièces sont maintenant
prouvées plutôt que proposées :

| Question | Réponse | Preuve |
|---|---|---|
| Les chiffres tiennent-ils sous Linux musl, 2 CPU, 128 Mio ? | Oui : mêmes effets, 1,5 à 3 fois plus lents (allocateur musl, VM). **Tout l'état vif des deux maisons : 6,4 Mio de mémoire anonyme réelle** pour 2,34 Mio demandés ; le moteur actuel y **perd des déclencheurs dès 100 automatismes** sous une rafale de 5 000 événements, et deux pilotes bloquants y figent toute la maison (médiane 202 ms) | L1-L6 |
| Le correctif de la perte sur retard marche-t-il ? | **Oui** : 20 lignes sur une copie du moteur ; à 400 automatismes et 5 000 événements, **0 déclencheur perdu** (5 sur 5 avant), et l'action arrive 109 ms après la rafale | F1 |
| Un bus qui répond mal, pas seulement mort ? | Un bus **lent** sous le délai est le vrai piège : rien n'échoue, tout attend (20 réussites sur 200, 965 s d'attente). La protection retenue : voies, **refus prédictif** (un ordre qui ferait la queue au-delà de son échéance est refusé en 0 ms : 85 réussites, attente ÷ 2,5), **réessais idempotents à délai adaptatif** (bus qui bat : 200 sur 200 au lieu de 15 à 145 ; bus partiel : 144 au lieu de 33 à 62), disjoncteur à **hystérésis sur échecs consécutifs** (1 bascule au lieu de 9 à 22) | Q1-Q3 |
| Les voies paresseuses tiennent-elles sous charge ? | **Oui** : 237 ordres sains sur 237, médiane 32,6 ms, comme les voies en tâches, pour moins de 0,6 Mio de pic pendant la charge (harnais compris) | V1-V3 |
| La bascule marche-t-elle de bout en bout ? | **Oui** : machine principale gelée, **0 changement perdu** à 200 changements par seconde, secours active **1,0 s** après le gel (dont 1 s de silence maximal décidé, 3 à 5 ms de reconstruction), l'ancienne machine revenue **reste passive** | S1-S3 |
| Jusqu'où le moteur actuel tient-il ? | À 400 automatismes, il suit **128 000 événements par seconde** soutenus (500 fois la pointe réelle), mais décroche sur une **rafale instantanée de plus de 1 024 événements** : c'est la profondeur de rafale qui compte, pas le débit. Le tampon d'entrée lisse les rafales pour 3 à 12 ms de latence | T1 |
| Combien coûte une maison quelconque ? | Modèle : **~660 o par valeur** d'état vif, **~520 o par automatisme** compilé, **~9 o par relevé** d'historique, **19 Kio par pilote** isolé, facteur **2,7** entre octets demandés et mémoire anonyme réelle | section 8 |

## 2. En conditions de production : Linux musl, 2 CPU, 128 Mio

Les bancs des V2 et V3 ont été recompilés en Linux musl statique (le même profil que l'image
`FROM scratch`, sans LTO) et exécutés dans un conteneur `alpine` limité à deux cœurs et
128 Mio, la limite de `compose.yaml`. Les écarts avec le Mac viennent de l'allocateur de
musl (plus lent, surtout sur les allocations nombreuses de `serde_json`) et de la
virtualisation.

### 2.1 Mémoire réelle

Le banc `etat_complet` lit, sous Linux, la RSS du processus et la mémoire **anonyme du cgroup**
(`memory.stat`, ligne `anon`), la mesure que PERFORMANCE.md applique à Moli et à Home Assistant.

| Moment | Octets demandés (allocateur qui compte) | RSS | Mémoire anonyme du cgroup | Facteur réel / demandé |
|---|---|---|---|---|
| Au démarrage, avant toute charge | ~0 | 744 Kio | 36 Kio | — |
| Tout l'état vif des deux maisons (496 appareils, 2 264 valeurs, 400 automatismes compilés, baux, agrégats, voies, bus) | 2,34 Mio | 8,6 Mio | **6,3 Mio** | ×2,7 |
| L'état vif et 7 jours d'historique compact | 11,1 Mio | 18,8 Mio | **16,3 Mio** | ×1,5 |

Le facteur entre octets demandés et mémoire réelle est de **2,7 sur un état fait de petites
structures** (en-têtes de blocs, arrondis de l'allocateur de musl, piles des threads du runtime)
et tombe à **1,5 dès que de grands tableaux** (l'historique en colonnes) dominent. Pour le
budget, retenir : état vif ≈ 6,5 Mio réels pour les deux maisons, profil sobre (24 h)
≈ 8 Mio, profil généreux (7 jours) ≈ 16,5 Mio ; tout cela tient dans les 32 Mio de mémoire
anonyme du budget du Pi, avec le reste du cœur (4 à 7 Mio mesurés aujourd'hui). Le profil
maximal (30 jours, ~40 Mio demandés, ~55 Mio réels estimés) le dépasse : il est réservé aux
machines à 2 Go et plus.

### 2.2 Les mêmes bancs, Linux contre Mac

| Mesure | Mac | Linux | Écart |
|---|---|---|---|
| Moteur actuel, balayage, 400 automatismes | 1,8 à 2,1 µs par événement | 3,4 µs | ×1,7 |
| Moteur compilé, 400 automatismes | 3 ns | 2 ns | — |
| Moteur compilé, 10 000 automatismes | 10 à 11 ns | 18 ns | ×1,7 |
| « Moyenne 1 h » par SQLite | 150 à 190 µs | 153 µs | — |
| Agrégat au fil de l'eau | 5 à 6 ns par relevé, 1 ns par lecture | 5 ns, 1 ns | — |
| Expression interprétée / compilée | 743 à 754 ns / 28 à 32 ns | 444 ns / 25 ns | — |
| Instantané JSON complet, compressé | 6,8 ms | **20,1 ms** | ×3 |
| Instantané binaire de l'état vif, compressé | 0,30 ms | 0,45 ms | ×1,5 |
| Ordre vers un pilote isolé (thread), p50 / p99 | 0,01 / 0,02 ms | 0,03 / 0,06 ms | ×3, identique au pilote partagé |
| Courbe 24 h, mémoire / SQLite | 0,04 / 17 ms | 0,10 / 21,8 ms | — |
| Redémarrage à chaud, 7 jours : SQLite / instantané binaire | 76 / 1,3 ms | 134 / 4,3 ms | ×2 à 3 |
| Compiler un automatisme à la validation | 7 µs | 11,8 µs | ×1,7 |
| Moteur réel, croisière, 400 automatismes, p50 / p99 | 0,5 / 2,9 ms | 1,1 / 1,4 ms | ×2 |
| Moteur réel, rafale de 2 000, action au milieu / après, 400 automatismes | 36,6 / 59,7 ms | 44,1 / 78,5 ms | ×1,3 |
| Moteur réel, rafale de 5 000, 400 automatismes | 80 ms / **perdue** | 102 ms / **perdue** | — |
| Deux pilotes bloquants partagés : médiane d'un ordre, pire retard d'une minuterie | 1,6 à 6,7 ms / ~195 ms | **202 ms / 405 ms** | la maison entière au pas des bloqueurs |
| Six pilotes bloquants isolés | ≤ 0,1 ms / 3 à 44 ms | 0,1 ms / 6,8 ms | — |
| Pont, lampe muette, boucle unique : ordres sains réussis | 37 à 42 sur 237 | 41 sur 237 | — |
| Pont, lampe muette, voie par appareil | 237 sur 237, 32 ms | 237 sur 237, 34,8 ms | — |
| API à 4 places, tâche par ordre / voie par appareil | 1,43 s / 32 ms | 1,35 s / 34,5 ms | — |
| Bus mort 15 s : ordres pendant la panne, voies / disjoncteur | 4 002 / 0,1 ms | 4 003 / 0,1 ms | — |
| Reprise après le retour du bus, disjoncteur | 503 ms | 634 ms | — |
| Inondation de 5 000 à la reconnexion, sans / avec tampon d'entrée | 5 sur 5 perdus / 0 perdu, 5 à 11 ms | 5 sur 5 perdus (3 200 manqués) / 0 perdu, 6 à 15 ms | — |

Ce qui change de lecture sous Linux :

- **Le moteur actuel décroche plus tôt.** À 5 000 événements, il prend du retard dès
  50 automatismes et perd des déclencheurs dès 100 ; à 200 automatismes, il perd même des
  déclencheurs injectés **au milieu** de la rafale (9 sur 9 dans une passe). Sur un Pi, 8 à
  10 fois plus lent, le même décrochage arriverait sur des rafales de quelques centaines.
- **Deux pilotes bloquants gèlent tout, sans exception.** Avec deux cœurs seulement, il ne
  reste rien pour le cœur : 7 ordres servis en 2 s, 202 ms de médiane, une minuterie de 10 ms
  qui part après 405 ms. Sur le Mac, le vol de tâches trouvait encore des cycles ; en
  production, non.
- **L'instantané JSON coûte trois fois plus** (20 ms) : à chaque seconde, ce serait 2 % d'un
  cœur du conteneur, et 15 à 20 % sur un Pi (estimé). Le format binaire reste à 0,45 ms.
- Les cloisons, les voies et le tampon d'entrée donnent exactement les mêmes résultats :
  leur effet ne dépend pas de la machine.

### 2.3 Le plafond du moteur actuel

Le banc T1 pousse 400 automatismes réels sous un **débit soutenu** pendant 3 s (bruit sur
700 capteurs, par lots d'une milliseconde), avec une sonde toutes les 500 ms.

| Débit soutenu | Sans tampon d'entrée (Mac) | Avec tampon d'entrée, 10 ms (Mac) | Linux, sans / avec tampon |
|---|---|---|---|
| 1 000 par seconde | 0 perdu, 0,5 ms | 0 perdu, 9,0 ms | 0 perdu, 1,0 ms / 0 perdu, 11,5 ms |
| 4 000 | 0 perdu, 0,5 ms | 0 perdu, 8,0 ms | 0 perdu, 1,2 ms / 0 perdu, 10,4 ms |
| 16 000 | 0 perdu, 0,7 ms | 0 perdu, 11,5 ms | 0 perdu, 0,9 ms / 0 perdu, 5,4 ms |
| 32 000 | 0 perdu, 0,5 ms | 0 perdu, 3,7 ms | 0 perdu, 1,9 ms (pire 10,8) / 0 perdu, 13,0 ms |
| 64 000 | 0 perdu, 0,8 ms | 0 perdu, 6,6 ms | 0 perdu, 3,9 ms / 0 perdu, 8,9 ms |
| 128 000 | 0 perdu, 0,8 ms | 1 perdu sur 6, 17 retards | **1 perdu sur 6, 24 retards** / 0 perdu, 13,7 ms |

Lecture, qui corrige une idée de la V2 : **ce n'est pas le débit qui fait décrocher le moteur
actuel, c'est la profondeur d'une rafale instantanée.** À 128 000 événements par seconde
soutenus sur le Mac, et 64 000 sous Linux, soit 270 à 500 fois la pointe réelle des deux
maisons (239 par seconde), le moteur suit, parce que chaque lot d'une milliseconde reste très
en dessous des 1 024 places du bus d'événements. Sous Linux, à 128 000 par seconde, les deux
cœurs saturent sans tampon (24 retards, un déclencheur perdu) ; avec le tampon, qui divise
par 180 le nombre d'événements publiés (700 points par 10 ms au plus), tout passe encore. Une rafale de 5 000 événements envoyée d'un trait (un pont qui republie tout à
la reconnexion, W2 et P2) remplit ces 1 024 places en quelques millisecondes, le moteur prend
du retard, et sans le correctif de la section 4 il perd des transitions. Le tampon d'entrée
transforme une rafale en lots (au plus un événement par point et par 10 ms) ; il coûte 3 à
12 ms de latence pendant la rafale et rien au repos. Sur le Mac à 128 000 par seconde avec le tampon, ce sont les deux cœurs qui saturent (le
tampon lui-même, le hub, le moteur), pas le bus.

Le coût par événement du moteur actuel reste celui de la V2 : environ 60 ns par automatisme
pour un événement qui ne concerne aucun déclencheur, 18 à 30 µs par événement à 400
automatismes dans une rafale qui en concerne (W2). Le moteur compilé le ramène à quelques
nanosecondes (I1), et le filtrage à la source lui épargne les événements que personne ne
regarde.

## 3. Un bus qui répond mal

La V3 avait joué un bus mort. Un bus réel répond souvent **mal** avant de ne plus répondre :
un pont saturé qui met 2 à 3 s, un coordinateur Zigbee qui redémarre en boucle, un service
cloud qui perd une requête sur deux. Trois caprices, quatre protections, même charge : 10
ordres par seconde vers 10 appareils du bus pendant 20 s, et 10 par seconde vers un autre
pilote. L'« attente totale » est la somme du temps passé par tous les appelants (un
workflow, la famille) à attendre une réponse : c'est le coût vécu.

Les quatre protections :

| Protection | Ce qu'elle fait |
|---|---|
| Voies seules | une file par appareil, un ordre en vol, échéance revérifiée (V2) |
| Disjoncteur simple (V3) | voies + ouverture sur 3 échecs en 10 s, essai après 1 s, un succès referme |
| Hystérésis + refus prédictif | voies + ouverture sur **3 échecs consécutifs**, ouvert **5 s au moins**, essais espacés de 1 s à 30 s, **3 succès** pour refermer ; au-delà de 1,5 s de latence médiane, un ordre qui ferait la queue au-delà de son échéance est **refusé tout de suite** |
| Adaptatif + réessais | la précédente, plus un **délai par tentative adapté au bus** (3 × la médiane récente, au moins 500 ms, une fois 5 réponses connues ; sinon tout le temps restant) et des **réessais tant que l'échéance le permet**, pour les ordres idempotents (une consigne, jamais un appui) ; l'essai de réouverture est un ordre réel avec ses réessais |

Résultats (Mac) : ordres vers le bus réussis sur 200, latence des réussis, échoués et leur
latence, attente totale des appelants, nombre de bascules du disjoncteur, et p99 de l'autre
pilote.

| Caprice | Protection | Réussis | p50 / p95 des réussis | Échoués (p50) | Attente totale | Bascules | Autre pilote p99 |
|---|---|---|---|---|---|---|---|
| **Q1 lent** (2,5 s, sous le délai) | voies seules | 20 | 2,5 s / 4,0 s | 180 (5,0 s, échéance) | **965 s** | 0 | 23 ms |
| | disjoncteur simple | 20 | 2,5 s / 4,0 s | 180 (5,0 s) | 965 s | 0 | 24 ms |
| | hystérésis + refus | **85** | 4,3 s / 4,9 s | 115 (**0 ms**, refusés d'avance) | **380 s** | 1 | 24 ms |
| | adaptatif + réessais | 85 | 4,3 s / 4,9 s | 115 (0 ms) | 382 s | 3 | 24 ms |
| **Q2 qui bat** (mort 1,5 s, vivant 1,5 s) | voies seules | 145 | 1,9 s / 4,9 s | 55 (4,0 s) | 500 s | 0 | 23 ms |
| | disjoncteur simple | 38 | 23 ms / 0,5 s | 162 (2,0 s) | 352 s | **9** | 24 ms |
| | hystérésis + refus | 15 | 23 ms / 23 ms | 185 (0 ms) | 136 s | 1 | 24 ms |
| | adaptatif + réessais | **200** | **26 ms / 1,5 s** | **0** | **105 s** | 0 | 24 ms |
| **Q3 partiel** (une requête sur deux perdue) | voies seules | 62 | 4,7 s / 5,0 s | 138 (5,0 s) | 912 s | 0 | 24 ms |
| | disjoncteur simple | 33 | 23 ms / 3,0 s | 167 (1,9 s) | 316 s | **22** | 24 ms |
| | hystérésis + refus | 57 | 3,0 s / 5,0 s | 143 (5,0 s) | 847 s | 0 | 24 ms |
| | adaptatif + réessais | **144** | 2,7 s / 4,5 s | 56 (5,0 s) | 573 s | 4 | 24 ms |

Sous Linux (conteneur 2 CPU, 128 Mio), les mêmes douze cas donnent la même lecture : bus lent,
20 réussites avec les voies seules contre **96** (hystérésis + refus) et 92 (adaptatif), attente
965 s contre 404 à 428 s ; bus qui bat, **200 sur 200** avec les réessais adaptatifs (p50
0,47 s, p95 1,5 s), 11 bascules pour le disjoncteur simple ; bus partiel, **148 sur 200** avec
les réessais contre 34 à 62 ; l'autre pilote à 31 ms de p99 dans les douze cas.

Ce qu'on en tire :

1. **Le bus lent est le vrai piège.** Rien n'échoue, donc un disjoncteur fondé sur les échecs
   ne s'ouvre jamais, et chaque ordre attend jusqu'à son échéance derrière les précédents :
   20 réussites sur 200 et 965 s d'attente cumulée. Il faut un **budget de latence**. Le
   premier essai de la V4, raccourcir le délai en mode dégradé, enterrait le bus (25 sur 200) ;
   le **refus prédictif** garde les ordres qui peuvent aboutir et refuse en 0 ms ceux qui
   feraient la queue au-delà de leur échéance : 85 réussites, attente divisée par 2,5, et
   l'appelant sait tout de suite.
2. **Les réessais sauvent le bus qui bat et le bus partiel, pas le bus lent.** Un délai par
   tentative adapté à la latence connue du bus (3 × la médiane) et des réessais dans
   l'échéance donnent 200 sur 200 sur le bus qui bat, 144 sur 200 sur le bus partiel, sans
   rien changer au bus lent. Deux conditions, apprises par les essais ratés : tant que la
   latence du bus n'est pas connue, une seule tentative avec tout le temps restant (sinon un
   bus lent est pris pour un bus mort : 0 sur 200 au premier essai) ; et seulement pour les
   ordres **idempotents**, une consigne qu'on peut poser deux fois, jamais un appui, une
   impulsion ni un basculement.
3. **Le disjoncteur compte des ordres, pas des tentatives, et des échecs consécutifs, pas
   des échecs par fenêtre.** Avec « 3 échecs en 10 s », un bus qui perd une requête sur deux
   s'ouvre et se referme sans fin : 22 bascules en 20 s. Avec « 3 échecs consécutifs », un
   succès remet le compte à zéro et un bus partiel reste utilisable ; un bus mort s'ouvre
   toujours aussi vite.
4. **L'hystérésis** (ouvert 5 s au moins, 3 succès consécutifs pour refermer, essais
   espacés de 1 s à 30 s) supprime l'oscillation : 1 bascule au lieu de 9 à 22. Son prix :
   un bus qui bat toutes les 1,5 s est traité comme mort tant qu'il n'est pas redevenu
   stable, et la famille le voit. Combinée aux réessais, qui servent aussi d'essai de
   réouverture, elle ne se déclenche plus que sur un bus vraiment mort.
5. **L'autre pilote ne voit jamais rien** (p99 23 à 26 ms dans les douze cas) : les voies et
   le thread par pilote suffisent à protéger le reste de la maison ; disjoncteur, budget de
   latence et réessais protègent **l'appelant**.

Les paramètres (5 s, 3 succès, 1,5 s de médiane, 3 × la médiane, 500 ms de plancher) sont des
points de départ à régler sur du vrai matériel ; la structure, elle, est fixée : voies, budget
de latence avec refus prédictif, réessais idempotents à délai adaptatif, disjoncteur à
hystérésis sur échecs consécutifs d'ordres.

## 4. Le correctif de la perte sur retard, prouvé

Le défaut (V2, 3.3) : sur un retard du bus d'événements, le moteur recopie le présent du hub
dans sa mémoire des valeurs précédentes, ce qui neutralise aussi les événements encore en
file. Le correctif, appliqué sur une **copie du moteur hors du dépôt** (`engine.rs`, une
fonction `resync` de 20 lignes à la place de `remember_all` dans la branche `Lagged`) : comparer
le présent du hub à ce que le moteur connaissait, et **rejouer chaque différence comme une
transition**. Un événement encore en file, relu ensuite, compare égal et ne déclenche pas deux
fois.

| Automatismes | Rafale de 5 000 événements | Avant, Mac (V2 W2) | Après le correctif, Mac | Avant, Linux | Après le correctif, Linux |
|---|---|---|---|---|---|
| 100 | action injectée après la rafale | **perdue** (3 retards, 1 441 manqués) | **servie en 36,5 ms** (1 retard, 135 manqués, 0 perdu) | **perdue** (4 retards, 931 manqués) | **servie en 78,8 ms** (4 retards, 1 194 manqués, 0 perdu) |
| 200 | actions injectées au milieu | 36 ms | — | **9 perdues** (2 retards, 2 707 manqués) | — |
| 400 | action injectée après la rafale | **perdue** (2 retards, 1 483 manqués) | **servie en 108,5 ms** (0 perdu) | **perdue** (2 retards, 1 453 manqués) | **servie en 242 ms** (5 retards, 695 manqués, 0 perdu) |
| 400 | actions injectées au milieu | 80 ms, pire 161 ms | 61,5 ms | 102 ms | 75,4 ms |

Sous Linux, le moteur prend plus souvent du retard (4 à 5 fois par rafale), et le correctif
rattrape à chaque fois : **0 déclencheur perdu** dans toutes les passes.

Le coût du correctif : à chaque retard, un instantané du hub et une comparaison de toutes les
valeurs (2 264 ici), de l'ordre de la milliseconde, et les actions manquées partent **une
fois**, sur la valeur finale. Ce correctif n'empêche pas le retard (le moteur compilé et le
tampon d'entrée s'en chargent) ; il empêche qu'un retard coûte une transition.

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

## 5. Les voies paresseuses, écrites pour de bon

La V3 avait mesuré leur mémoire au repos (6 Kio) mais pas leur comportement sous charge. La V4
les écrit : un répartiteur unique par pilote, une file par appareil créée au premier ordre en
attente et détruite une fois vide, un seul ordre en vol par appareil, l'échéance revérifiée
avant chaque envoi, la consigne en attente remplacée par la suivante, un disjoncteur par
appareil qui vide la file et passe l'appareil hors ligne (code en annexe A, `voies.rs`).

| Scénario | Ordres aux lampes saines réussis | Médiane | Pire | Lampe muette | Pic mémoire pendant la charge (processus entier) |
|---|---|---|---|---|---|
| C2 rejoué : pont sans limite, une lampe muette | 237 sur 237 | 32,6 ms | 34,9 ms | 5,0 s | 536 Kio |
| idem, avec disjoncteur | 237 sur 237 | 32,6 ms | 35,5 ms | 2,9 s puis refus immédiat | 533 Kio |
| C3 rejoué : API à 4 places | 237 sur 237 | 32,8 ms | 54,3 ms | 5,0 s | 536 Kio |
| idem, avec disjoncteur | 237 sur 237 | 33,0 ms | 33,7 ms | 2,9 s puis refus immédiat | 533 Kio |
| Le pont entier tombe de 2 s à 6 s, puis revient | 218 sur 237 (les 19 autres sont tombés pendant la panne, refusés) | 33,1 ms | 3,2 s | — | 534 Kio |

Même isolation que les voies en tâches permanentes de la V2 (237 sur 237, 32 ms), pour un pic
de moins de 0,6 Mio pendant une charge de 25 ordres par seconde, harnais de test compris, au
lieu de 3,5 Mio réservés en permanence. Le dernier scénario montre le disjoncteur par
appareil face à un pont entier qui tombe : chaque lampe s'ouvre après deux échecs, les ordres
suivants sont refusés sans attendre, et tout repart au retour du pont. Le disjoncteur **de
bus** (section 3) fait la même chose en une fois au lieu de vingt.

## 6. La bascule, jouée de bout en bout

Deux processus sur la même machine. Le principal tient un hub complet (496 appareils, 2 264
valeurs), produit des changements à un rythme donné, écrit chacun dans un journal local sans
tampon (ce que le noyau a reçu survit à sa mort) et le réplique vers la secours par TCP, avec
un instantané au début et un battement de cœur toutes les 100 ms. La secours tient son état à
jour en mémoire sans toucher à aucun appareil. À 5 s, on **gèle** le principal (`SIGSTOP`) :
comme une machine qui meurt, il ne ferme pas ses connexions. La secours constate le silence,
bascule, reconstruit son hub, puis l'ancien principal revient avec son ancienne époque.

| Rythme | Instantané reçu et appliqué | Reçus par la secours | Dernier écrit par le principal / dernier reçu | **RPO** | Détection | Hub de secours prêt | **RTO** | L'ancien principal revient |
|---|---|---|---|---|---|---|---|---|
| 2 changements par seconde, machine gelée | 410 Kio en 5,5 ms | 11 | 11 / 11 | **0** | 978 ms (silence max 1 s) | 4,5 ms | **982 ms** | passif, aucun appareil touché |
| 200 par seconde, machine gelée | 410 Kio en 2,7 ms | 1 002 | 1 002 / 1 002 | **0** | 1 035 ms | 3,2 ms | **1 038 ms** | passif |
| 200 par seconde, processus tué (connexion fermée par le noyau) | 410 Kio en 5,9 ms | 1 002 | 1 002 / 1 002 | **0** | **0,8 ms** | 3,3 ms | **4,2 ms** | passif |

Sous Linux (conteneur 2 CPU, 128 Mio), les mêmes trois scénarios : RPO 0 dans les trois cas
(11 / 11, 1 004 / 1 004, 1 003 / 1 003) ; détection 993 à 1 023 ms quand la machine est gelée,
**0,0 ms** quand le processus est tué ; hub de secours prêt en 6 à 17 ms ; RTO **1,01 à
1,03 s** gelé, **6,4 ms** tué ; l'ancien principal revenu reste passif.

Lecture :

- **Rien n'est perdu** (RPO = 0) : tout ce que le principal avait écrit avait aussi été reçu,
  à 2 comme à 200 changements par seconde. La réplication suit le journal ; la perte possible
  est bornée par ce qui est en vol sur le réseau, quelques millisecondes.
- **L'indisponibilité est le temps de décision** : 1 s de silence maximal, puis 3 à 5 ms pour
  que le hub de secours soit prêt. Quand le processus meurt mais que la machine vit, le noyau
  ferme la connexion et la détection est immédiate. En vrai, s'ajoutent les reconnexions des
  pilotes aux appareils (secondes) et, pour un chien de garde, la vérification que l'ancienne
  machine est bien arrêtée avant d'activer la nouvelle.
- **L'ancienne machine revenue reste passive** : elle annonce son époque, apprend que la
  secours est passée à l'époque suivante, et ne touche à aucun appareil. C'est la garantie
  « jamais deux maîtres », à condition que l'époque soit écrite **avant** toute activation.

## 7. Les contrats, esquissés

Ce que chaque brique du pattern devrait exposer dans `moli-runtime`, avec ses règles. Ce sont
des esquisses pour cadrer l'implémentation, pas du code livré ; les noms sont en français
comme le reste du domaine.

### 7.1 `spawn_driver_isole` : un thread et un runtime par pilote

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
- la pile est celle par défaut (2 Mio réservés, ~19 Kio touchés au repos sous Linux musl) ;
- une panique dans une sous-tâche du pilote fait sortir `run`, donc relancer le pilote ;
- le hub reçoit le `Handle` du cœur à sa construction (`HubOptions::coeur`) et y lance
  ses propres tâches : horloges des calculs, des dépendances et des durées, observateurs
  d'ordres. Un pilote isolé ne porte jamais une tâche du hub.

### 7.2 `Voies` : une file par appareil, à la demande

```rust
pub struct Voies<E: Envoi> { /* répartiteur unique */ }

pub trait Envoi: Send + Sync + 'static {
    /// Envoie un ordre à un appareil ; rend le résultat à la voie, qui répond au cœur.
    fn envoyer(&self, appareil: &DeviceId, ordre: CommandRequest) -> BoxFuture<'static, Resultat>;
}

pub enum Resultat { Fait, Injoignable, DelaiDepasse, Refuse(String) }

impl<E: Envoi> Voies<E> {
    pub fn new(envoi: E, reglage: Reglage) -> Self;
    /// Un ordre du cœur : envoyé tout de suite s'il n'y a rien en vol pour l'appareil,
    /// sinon mis en attente ; refusé sans attendre si le disjoncteur de l'appareil est ouvert.
    pub fn ordre(&mut self, ordre: CommandRequest);
    /// À appeler dans le `select!` du pilote : fait avancer les files.
    pub async fn tour(&mut self) -> Option<Evenement>;
}

pub struct Reglage {
    pub attente_max: usize,          // 8 : au-delà, la plus ancienne consigne cède (« remplacée »)
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
- une consigne qui attend est remplacée par la suivante pour le même point (la dernière gagne) ;
  un appui ou une impulsion (`Kind::Button`, `TimerCommand`) n'est jamais remplacé ;
- à l'ouverture du disjoncteur, la file de l'appareil est vidée avec un refus immédiat ;
- l'essai de réouverture est un ordre réel (le prochain), jamais une requête fabriquée.

### 7.3 `Passerelle` : une API partagée

```rust
pub struct Passerelle {
    places: Semaphore,           // concurrence acceptée par le pont ou le service
    tourniquet: ...,             // équité entre appareils : un appareil n'occupe qu'une place
    priorite: Ordre > Sondage,   // un ordre passe avant une lecture périodique
    debit: SeauAJetons,          // quota du fournisseur (cloud)
    disjoncteur: DisjoncteurDeBus,
}
```

Règles :
- un ordre attend au plus `echeance − marge` une place, sinon « occupé » tout de suite ;
- le sondage cède sa place à un ordre et reprend après ;
- le disjoncteur de bus (7.4) ferme la passerelle entière.

### 7.4 `DisjoncteurDeBus` : à hystérésis, avec budget de latence

| Paramètre | Valeur proposée | Pourquoi |
|---|---|---|
| Ouverture | 3 échecs d'appareils **différents** en 10 s, ou le signal du transport (déconnexion MQTT, pont injoignable) | un seul appareil muet ne doit pas ouvrir le bus |
| Durée minimale ouvert | 5 s | un bus qui bat ne fait pas osciller la maison (Q2) |
| Essai | un ordre réel, délai court (500 ms), intervalle 1 s doublé jusqu'à 30 s | la reprise en 0,5 s après le retour (P1), sans marteler un bus mort |
| Fermeture | 3 succès consécutifs, ou le signal de reconnexion du transport | un essai réussi par hasard ne rouvre pas la vanne |
| Dégradé | médiane des 20 dernières latences > 1,5 s | un bus lent sous le délai (Q1) est traité avant que les files ne débordent |
| En dégradé | délai par ordre ramené à 1 s, un seul ordre en attente par appareil, sondage suspendu | la latence des ordres reste bornée, le workflow sait vite |
| Effet visible | tous les appareils du bus hors ligne ; ordres refusés en 0,1 ms ; notice à la famille ; raison dans `/api/health` | la panne se voit, elle ne se devine pas |

### 7.5 Le tampon d'entrée du hub

```rust
pub struct BudgetEntree {
    pub par_pilote: u32,     // 200 événements par seconde par pilote au-delà desquels on fusionne
    pub fusion: Duration,    // 10 ms : les états et mesures fusionnés par point, la dernière gagne
}
```

Règles :
- sous le budget, rien ne change : chaque `set_state` est publié comme aujourd'hui ;
- au-delà, les `State` d'un même point sont fusionnés sur une fenêtre de 10 ms ; la minuterie
  n'est armée que si le tampon n'est pas vide (rien au repos) ;
- jamais fusionnés : `pulse_state` (appuis), occurrences, expirations, `DeviceUpserted`,
  `Availability` ;
- un pilote qui dépasse le budget plus de 10 s de suite est signalé (journal, santé).

### 7.6 Le journal, l'instantané, l'époque

| Pièce | Forme | Règle |
|---|---|---|
| Journal | une ligne binaire par changement : `seq u64, point u32, charge, instant u64` (~9 octets compressés) ; validé par lots (≤ 100 ms ou 500 lignes) | la source de la reprise et de la réplication ; jamais réécrit |
| Instantané | binaire compact de l'état vif : `(point u32, étiquette u8, charge, 3 instants)` ; contient `seq` du journal auquel il correspond | au quart d'heure sur disque ; à chaque seconde vers le réseau si une secours écoute |
| Baux et exécutions | dans le journal (ouverture, prolongation, fin) et dans l'instantané | repris au démarrage, appliqués une fois |
| Époque | entier monotone, écrit sur disque avant toute activation, annoncé à la connexion | la plus grande gagne ; l'autre reste passive |
| Secours | reçoit instantané + journal ; applique en mémoire ; **n'ouvre aucune connexion** | bascule sur décision (humain ou chien de garde), après arrêt de l'ancienne |

## 8. Le modèle de coût

Coefficients unitaires tirés des mesures (allocateur qui compte chaque octet, deux maisons et
×10, Mac ; les coefficients sont indépendants de la machine), pour chiffrer n'importe quelle
maison. `V` : valeurs (points), `A` : appareils, `N` : automatismes, `R` : relevés par jour,
`P` : pilotes.

| Poste | Coefficient | D'où il vient |
|---|---|---|
| Modèle du hub (ce qui existe) | **463 o × V** | 1,00 Mio pour 2 264 valeurs ; 457 o à ×10 |
| Étage vif (précédente, 3 instants, qualité, index) | **196 o × V** | 434 Kio pour 2 264 |
| Santé des appareils | 25 o × A | 12 Kio pour 496 |
| Automatismes compilés (9 règles, 2 actions, index) | **520 o × N** | 203 Kio pour 400 ; le moteur actuel : 9 640 o × N |
| Exécutions en cours | ~410 o chacune | 1,56 Mio − 150 Kio pour 3 600 exécutions de plus |
| Baux | ~64 o chacun | taille des structures (48 o + entrée de l'échéancier) |
| Agrégats 1 h + 24 h d'un point lu | ~1 000 o par point, au taux moyen de 110 changements par jour | 443 Kio pour 453 points |
| Historique compact | **8,1 à 8,9 o × R × jours** | 1,37 Mio pour 161 398 relevés ; 8,78 Mio pour 1 129 783 |
| Voies paresseuses | 6 Kio + 200 o par ordre en attente | mesure au repos ; `CommandRequest` 200 o |
| Tampon du bus d'événements | 128 Kio (1 024 × 128 o) | taille de `Event` |
| Thread et runtime par pilote | **19 Kio × P** (Linux musl) | 788 Kio pour 40 |
| Processus par pilote | 577 Kio × P (Linux musl, processus minimal) | 10 processus |
| Mémoire anonyme réelle | **≈ 2,7 × octets demandés** sur le petit état (runtime, piles, fragmentation) ; le facteur baisse quand la part des grands tableaux (historique) monte | 6,3 Mio réels pour 2,34 Mio demandés (état vif) ; 16,3 Mio pour 11,1 Mio (état vif + 7 jours) : Linux, cgroup |

Formule de l'état vif : `(463 + 196) × V + 25 × A + 520 × N + ~1 000 × V/5 + 134 Kio`, puis
`× 2,7` pour la mémoire réelle d'un petit état. Trois maisons :

| Maison | V / A / N | État vif demandé | + 24 h | + 7 jours | Threads (40 pilotes) | Réel estimé, profil sobre |
|---|---|---|---|---|---|---|
| Petite (un appartement) | 250 / 50 / 30 | 0,36 Mio | +0,15 Mio | +0,98 Mio | 0,74 Mio | ~2 Mio |
| Les deux maisons réunies | 2 264 / 496 / 400 | 2,25 Mio (mesuré 2,34) | +1,39 Mio | +8,84 Mio | 0,74 Mio | ~8 à 10 Mio |
| ×10 | 22 640 / 4 960 / 4 000 | 21,3 Mio (mesuré 22,4) | +13,9 Mio | +88 Mio | 0,74 Mio | ~60 Mio |

Le processeur, au rythme réel des deux maisons (1,9 changement par seconde en moyenne, pointe
à 239 par seconde) :

| Poste | Coût mesuré (Linux) | Au rythme moyen | À la pointe |
|---|---|---|---|
| Moteur actuel, 400 automatismes | 3,4 µs par événement | 0,0006 % d'un cœur | 0,08 % |
| Moteur compilé, 400 automatismes | 2 ns par événement | négligeable | négligeable |
| Agrégats au fil de l'eau | 5 ns par relevé | négligeable | négligeable |
| Instantané binaire toutes les 15 min | 0,45 ms | 0,00005 % | — |
| Journal, un `fsync` par lot de 100 ms | 1,9 ms par `fsync` (Linux, disque du conteneur) | 1,9 % d'un cœur **en attente d'E/S**, pas en calcul | — |

Sur un Pi, multiplier les coûts de calcul par 8 à 10 (estimé) : le moteur actuel reste sous
1 % à la pointe, le moteur compilé invisible. Le vrai coût du Pi est la carte SD : le `fsync`
par lot est le poste à surveiller (PERFORMANCE.md mesure déjà les écritures).

## 9. Plan revu

Les étapes de la V3 restent ; la V4 en a prouvé trois et en précise deux.

| Étape | État après la V4 | Coût à l'exécution | Preuve d'acceptation |
|---|---|---|---|
| 1. Correctif de la perte sur retard | **prouvé** sur copie (section 4) : à reporter tel quel | ~1 ms par retard | V2 W2 rejoué : 0 perdu ✓ |
| 1 bis. Tâches du hub sur le `Handle` du cœur | inchangé | nul | horloges vivantes quand le pilote des aides est gelé |
| 2. Tampon d'entrée du hub | confirmé sous Linux | ≤ 15 ms en rafale | P2 rejoué : 0 perdu ✓ |
| 3. Voies paresseuses et disjoncteurs | **écrites et mesurées** (section 5) ; disjoncteur de bus à hystérésis sur échecs consécutifs, refus prédictif, réessais idempotents à délai adaptatif (section 3) | < 0,6 Mio en charge, 6 Kio au repos | C2, C3, P1, Q1-Q3 rejoués ✓ |
| 4. Un thread par pilote | confirmé sous Linux : indispensable à deux cœurs | 19 Kio par pilote | C1 rejoué ✓ |
| 5. Bail durable | inchangé | ~64 o par bail, 1,9 à 3,7 ms à l'ouverture | redémarrage au milieu d'un bail |
| 6. Étage vif et agrégats | coût confirmé (2,7 × en réel) | ~0,9 Mio demandés | lecture d'agrégat en mémoire |
| 7. Moteur compilé, filtré à la source | plafond du moteur actuel chiffré (2.3) | moins de mémoire qu'aujourd'hui | T1 rejoué à 4 000 automatismes |
| 8. Historique récent en mémoire | redémarrage à chaud 4,3 ms sous Linux | 8 à 9 o par relevé | B2.3 ✓ |
| 9. Journal, instantanés binaires, époque, secours passive | **bascule jouée** (section 6) | ~2,6 Mo écrits par jour | S1-S3 ✓ ; reste : reconnexion des pilotes, chien de garde |

## 10. Limites

- Le conteneur Linux tourne sur le Mac : même architecture que le Pi (arm64), mais un cœur
  8 à 10 fois plus rapide ; aucune mesure sur un Pi Zero 2 W ni sur une carte SD.
- Les bus capricieux sont simulés par des délais ; un vrai pont saturé a d'autres formes
  (réponses partielles, reconnexions TLS). Les paramètres des disjoncteurs sont des points de
  départ.
- La bascule est jouée sur une machine, sans réseau réel ni reconnexion aux appareils : le
  délai mesuré est celui de la décision et de la reconstruction, pas celui de la reprise des
  pilotes.
- Le correctif est prouvé sur une copie du moteur avec le banc de la V2 ; il reste à le porter
  dans le dépôt avec ses tests.
- Le modèle de coût est linéaire ; il ne tient pas compte des caches du processeur ni de la
  fragmentation à long terme.

## Annexe A — Bancs de la V4

Quatre crates jetables hors du dépôt : `preuves-v2` (bancs `caprices`, `voies`, `bascule`,
`capacite`, et `etat_complet` enrichi de la mémoire réelle), `preuves-fix` (le banc `moteur`
de la V2 contre la copie corrigée), `moli-automation-fix` (la copie du moteur, Cargo.toml
réécrit sans héritage de workspace, un seul changement de code : `resync`), et `linux-couts`
(threads et processus sous Linux). La chaîne Linux : copie du dépôt et des crates jetables dans
un conteneur `rust:1-alpine` (musl), compilation `--release --offline`, exécution des binaires
statiques dans `alpine:3` avec `--cpuset-cpus=0,1 --memory=128m`.

Sorties brutes :

### Mac — bus capricieux (caprices)

```text
# Un bus qui répond mal : 10 ordres/s vers 10 de ses appareils pendant 20 s, un autre pilote à côté

## Q1 — bus lent : répond en 2,5 s, sous le délai de 4 s
- voies seules             : réussis  20/200 (p50 2504 ms, p95 3980 ms) ; échoués 180 (p50 5003 ms) ; attente totale des appelants  965.3 s ; bascules  0 ; autre pilote p99 23 ms
- disjoncteur simple (V3)  : réussis  20/200 (p50 2504 ms, p95 3986 ms) ; échoués 180 (p50 5003 ms) ; attente totale des appelants  965.3 s ; bascules  0 ; autre pilote p99 24 ms
- hystérésis + refus       : réussis  85/200 (p50 4309 ms, p95 4881 ms) ; échoués 115 (p50    0 ms) ; attente totale des appelants  380.5 s ; bascules  1 ; autre pilote p99 24 ms
- adaptatif + réessais     : réussis  85/200 (p50 4301 ms, p95 4867 ms) ; échoués 115 (p50    0 ms) ; attente totale des appelants  381.7 s ; bascules  3 ; autre pilote p99 24 ms

## Q2 — bus qui bat : mort 1,5 s, vivant 1,5 s
- voies seules             : réussis 145/200 (p50 1937 ms, p95 4945 ms) ; échoués  55 (p50 4003 ms) ; attente totale des appelants  499.9 s ; bascules  0 ; autre pilote p99 23 ms
- disjoncteur simple (V3)  : réussis  38/200 (p50   23 ms, p95  468 ms) ; échoués 162 (p50 1950 ms) ; attente totale des appelants  352.2 s ; bascules  9 ; autre pilote p99 24 ms
- hystérésis + refus       : réussis  15/200 (p50   23 ms, p95   23 ms) ; échoués 185 (p50    0 ms) ; attente totale des appelants  135.9 s ; bascules  1 ; autre pilote p99 24 ms
- adaptatif + réessais     : réussis 200/200 (p50   26 ms, p95 1531 ms) ; échoués   0 (p50       —) ; attente totale des appelants  105.1 s ; bascules  0 ; autre pilote p99 24 ms

## Q3 — bus partiel : une requête sur deux n'aboutit jamais
- voies seules             : réussis  62/200 (p50 4741 ms, p95 4968 ms) ; échoués 138 (p50 5002 ms) ; attente totale des appelants  911.9 s ; bascules  0 ; autre pilote p99 24 ms
- disjoncteur simple (V3)  : réussis  33/200 (p50   23 ms, p95 3000 ms) ; échoués 167 (p50 1945 ms) ; attente totale des appelants  316.0 s ; bascules 22 ; autre pilote p99 24 ms
- hystérésis + refus       : réussis  57/200 (p50 2999 ms, p95 4972 ms) ; échoués 143 (p50 5003 ms) ; attente totale des appelants  847.0 s ; bascules  0 ; autre pilote p99 24 ms
- adaptatif + réessais     : réussis 144/200 (p50 2672 ms, p95 4500 ms) ; échoués  56 (p50 5002 ms) ; attente totale des appelants  572.9 s ; bascules  4 ; autre pilote p99 24 ms
```

### Mac — voies paresseuses (voies)

```text
# Voies paresseuses écrites pour de bon, sous la charge de la V2 et de la V3

## L1 — C2 rejoué : pont sans limite, 20 lampes, une muette, 25 ordres/s pendant 10 s
- voies paresseuses              : lampes saines 237/237 réussis, médiane 32.6 ms, pire  34.9 ms ; muette : médiane 5001.7 ms ; pic mémoire pendant la charge 536 Kio
- voies paresseuses + disjoncteur : lampes saines 237/237 réussis, médiane 32.6 ms, pire  35.5 ms ; muette : médiane 2875.8 ms ; pic mémoire pendant la charge 533 Kio

## L2 — C3 rejoué : l'API du pont ne traite que 4 requêtes à la fois
- voies paresseuses              : lampes saines 237/237 réussis, médiane 32.8 ms, pire  54.3 ms ; muette : médiane 5002.9 ms ; pic mémoire pendant la charge 536 Kio
- voies paresseuses + disjoncteur : lampes saines 237/237 réussis, médiane 33.0 ms, pire  33.7 ms ; muette : médiane 2903.8 ms ; pic mémoire pendant la charge 533 Kio

## L3 — le pont entier tombe de 2 s à 6 s (les 20 lampes muettes), puis revient
- voies paresseuses + disjoncteur : lampes saines 218/237 réussis, médiane 33.1 ms, pire 3185.7 ms ; muette : médiane 2890.8 ms ; pic mémoire pendant la charge 534 Kio
```

### Mac — bascule (bascule)

```text
# Bascule de bout en bout : deux processus, le principal gelé (SIGSTOP), la secours reprend

## rythme : 2 changements par seconde, machine gelée
- S1 : instantané de 410 Kio reçu et appliqué en 9.0 ms (2264 valeurs)
- S2 : 2 changements/s ; 11 reçus par la secours ; dernier écrit par le principal 11, dernier reçu 11 : 0 changements perdus (RPO)
- S2 : gel (machine morte) du principal → détection 982.0 ms (silence max 1000.0 ms) → hub de secours prêt 6.4 ms après la décision ; indisponibilité totale 988.4 ms (RTO)
- S3 : principal : époque 1 périmée (la secours est à 2) : je reste passif, aucun appareil touché

## rythme : 200 changements par seconde, machine gelée
- S1 : instantané de 410 Kio reçu et appliqué en 4.7 ms (2264 valeurs)
- S2 : 200 changements/s ; 1005 reçus par la secours ; dernier écrit par le principal 1005, dernier reçu 1005 : 0 changements perdus (RPO)
- S2 : gel (machine morte) du principal → détection 1034.1 ms (silence max 1000.0 ms) → hub de secours prêt 4.4 ms après la décision ; indisponibilité totale 1038.5 ms (RTO)
- S3 : principal : époque 1 périmée (la secours est à 2) : je reste passif, aucun appareil touché

## rythme : 200 changements par seconde, processus tué
- S1 : instantané de 410 Kio reçu et appliqué en 5.9 ms (2264 valeurs)
- S2 : 200 changements/s ; 1002 reçus par la secours ; dernier écrit par le principal 1002, dernier reçu 1002 : 0 changements perdus (RPO)
- S2 : mort du processus (connexion fermée) du principal → détection 0.8 ms (silence max 1000.0 ms) → hub de secours prêt 3.3 ms après la décision ; indisponibilité totale 4.2 ms (RTO)
- S3 : principal : époque 1 périmée (la secours est à 2) : je reste passif, aucun appareil touché
```

### Mac — plafond du moteur (capacite)

```text
# Plafond du moteur actuel : 400 automatismes, bruit soutenu pendant 3 s sur 700 capteurs

## sans tampon d'entrée
-  1000 événements/s : débit réel 999/s ; 0 déclencheurs perdus sur 6 ; latence médiane 0.5 ms, pire 0.7 ms ; retards du moteur 0 (0 événements manqués)
-  4000 événements/s : débit réel 3998/s ; 0 déclencheurs perdus sur 6 ; latence médiane 0.5 ms, pire 1.5 ms ; retards du moteur 0 (0 événements manqués)
- 16000 événements/s : débit réel 15982/s ; 0 déclencheurs perdus sur 6 ; latence médiane 0.7 ms, pire 1.0 ms ; retards du moteur 0 (0 événements manqués)
- 32000 événements/s : débit réel 31965/s ; 0 déclencheurs perdus sur 6 ; latence médiane 0.5 ms, pire 0.6 ms ; retards du moteur 0 (0 événements manqués)
- 64000 événements/s : débit réel 63895/s ; 0 déclencheurs perdus sur 6 ; latence médiane 0.8 ms, pire 1.0 ms ; retards du moteur 0 (0 événements manqués)
- 128000 événements/s : débit réel 127901/s ; 0 déclencheurs perdus sur 6 ; latence médiane 0.8 ms, pire 1.1 ms ; retards du moteur 0 (0 événements manqués)

## avec tampon d'entrée (dernière valeur, 10 ms)
-  1000 événements/s : débit réel 999/s ; 0 déclencheurs perdus sur 6 ; latence médiane 9.0 ms, pire 11.6 ms ; retards du moteur 0 (0 événements manqués)
-  4000 événements/s : débit réel 3993/s ; 0 déclencheurs perdus sur 6 ; latence médiane 8.0 ms, pire 8.1 ms ; retards du moteur 0 (0 événements manqués)
- 16000 événements/s : débit réel 15978/s ; 0 déclencheurs perdus sur 6 ; latence médiane 11.5 ms, pire 11.6 ms ; retards du moteur 0 (0 événements manqués)
- 32000 événements/s : débit réel 31957/s ; 0 déclencheurs perdus sur 6 ; latence médiane 3.7 ms, pire 4.2 ms ; retards du moteur 0 (0 événements manqués)
- 64000 événements/s : débit réel 63913/s ; 0 déclencheurs perdus sur 6 ; latence médiane 6.6 ms, pire 7.8 ms ; retards du moteur 0 (0 événements manqués)
- 128000 événements/s : débit réel 127755/s ; 1 déclencheurs perdus sur 6 ; latence médiane 8.0 ms, pire 9.5 ms ; retards du moteur 17 (18674 événements manqués)
```

### Mac — correctif sur copie (preuves-fix, moteur 100 400)

```text
# Le vrai moteur d'automatismes sous charge (2 workers, machine de test)
Chaque automatisme : 1 déclencheur, 3 conditions imbriquées de 3 règles, 1 action ;
10 % sont temporaires (« allume 30 min »), 10 % attendent une condition (wait_for).
N = 100 : chargement 0.72 s, mémoire du moteur 1154 Kio ; croisière p50 0.3 ms p99 1.3 ms (19 actions)
        rafale  2000 : sondes au milieu p50 10.3 ms max 10.7 ms perdues 0 ; sonde après p50 18.8 ms ; retards du moteur 0 (0 événements manqués)
        rafale  5000 : sondes au milieu p50 16.8 ms max 17.4 ms perdues 0 ; sonde après p50 36.5 ms ; retards du moteur 1 (135 événements manqués)
N = 400 : chargement 9.56 s, mémoire du moteur 3766 Kio ; croisière p50 0.3 ms p99 0.5 ms (19 actions)
        rafale  2000 : sondes au milieu p50 28.2 ms max 29.1 ms perdues 0 ; sonde après p50 40.7 ms ; retards du moteur 0 (0 événements manqués)
        rafale  5000 : sondes au milieu p50 61.5 ms max 61.5 ms perdues 0 ; sonde après p50 108.5 ms ; retards du moteur 0 (0 événements manqués)
```

### Linux — budget mémoire avec mémoire réelle (etat_complet)

```text
# Budget mémoire : tout ce qui compte, en mémoire
- tailles : Value 24 o, PointVif 80 o, Event 128 o
- mémoire réelle au démarrage, avant toute charge : RSS 744 Kio, anonyme du cgroup 36 Kio
  (voies en tâches permanentes avec file de 8 : 3.47 Mio pour 496 appareils, soit 7.2 Kio chacune ; CommandRequest 200 o)

## ×1 : 496 appareils, 2264 valeurs, 400 automatismes
- Modèle du hub (appareils, définitions, valeurs : ce qui existe) : 1.00 Mio
- Étage vif : précédente, 3 instants, qualité + index des points : 434 Kio
- Santé par appareil (vu le, échecs, disjoncteur) : 12 Kio
- Automatismes compilés (9 règles, 2 actions) + index par point : 203 Kio
- 1 000 baux, échéancier et une exécution en cours par automatisme : 150 Kio
- Agrégats 1 h et 24 h des points lus par les règles : 443 Kio
- Voies paresseuses (répartiteur unique, files à la demande), au repos : 6 Kio
- Tampon du bus : 1 024 événements (charges partagées en Arc) : 128 Kio
- **Sous-total « tout l'état vif »** : 2.34 Mio
- mémoire réelle avec tout l'état vif : RSS 8600 Kio, anonyme du cgroup 6476 Kio
- mémoire réelle avec l'état vif et 7 jours d'historique : RSS 18820 Kio, anonyme du cgroup 16696 Kio
- Historique compact en mémoire : 24 h 1.37 Mio ; 7 j 8.78 Mio ; 30 j 37.10 Mio
- Totaux : vif + 24 h 3.71 Mio ; vif + 7 j 11.12 Mio ; vif + 30 j 39.44 Mio
  (voies en tâches permanentes avec file de 8 : 34.69 Mio pour 4960 appareils, soit 7.2 Kio chacune ; CommandRequest 200 o)
```

### Linux — balayage contre compilé (index)

```text
# Workflows complexes : balayage d'aujourd'hui contre moteur compilé (machine de test)

## I1 — trouver les workflows à lancer et évaluer leurs 9 règles
200 000 événements (20 000 pour N = 10 000), dont 10 % sur un déclencheur ; temps moyen par événement.
- N =    100 : balayage      1144 ns/événement ; compilé     1 ns/événement ; ×826 ; 20000 lancements
- N =    400 : balayage      3395 ns/événement ; compilé     2 ns/événement ; ×1675 ; 20000 lancements
- N =   1000 : balayage      7774 ns/événement ; compilé     2 ns/événement ; ×3695 ; 20000 lancements
- N =  10000 : balayage     85119 ns/événement ; compilé    18 ns/événement ; ×4852 ; 2000 lancements

## I2 — lire « moyenne sur 1 h » d'un capteur qui change chaque seconde
- SQLite (index (point, ts), 3 600 lignes lues) : 153 µs par lecture, plus le passage par un thread bloquant
- balayage d'une fenêtre en mémoire : 2.5 µs par lecture
- agrégat tenu au fil de l'eau : 5 ns par relevé reçu, 1.0 ns par lecture

## I3 — variables et calculs : « (moyenne 1 h du salon − 2) > 19 et (présence ou 7 h ≤ heure < 22 h) »
- interprété depuis le JSON : 444 ns ; compilé une fois à la validation : 25 ns (×18)

## I4 — « allume 30 minutes » en bail durable : 10 000 baux
- ordonnancement de 10 000 échéances (tas binaire) et extraction des 5000 dues : 0.53 ms au total
- écrire un bail de façon durable (ajout + fsync) : 1.9 ms, une seule fois au lancement ; rien pendant l'attente
```

### Linux — coût des choix (couts)

```text
# Le coût de chaque choix (machine de test, 2 workers)

## B2.1 — instantané : coût selon le format et la fréquence (machine de test)
- JSON complet (valeurs + 400 automatismes), compressé : 20.05 ms par instantané, 43 Kio
- binaire compact des valeurs (étage vif), compressé : 0.45 ms par instantané, 13 Kio
| intervalle | CPU d'un cœur, JSON | CPU d'un cœur, compact | écrit par jour, JSON | écrit par jour, compact |
| 1 s | 2.005 % | 0.0446 % | 3842 Mo | 1210.6 Mo |
| 10 s | 0.200 % | 0.0045 % | 384 Mo | 121.1 Mo |
| 60 s | 0.033 % | 0.0007 % | 64 Mo | 20.2 Mo |
| 15 min | 0.002 % | 0.0000 % | 4 Mo | 1.3 Mo |

## B2.2 — ordre vers un pilote : même runtime ou thread isolé (2 000 ordres)
- pilote sur le runtime du cœur : p50 0.03 ms, p99 0.06 ms
- pilote sur son thread : p50 0.03 ms, p99 0.06 ms

## B2.3 — courbe 24 h et redémarrage à chaud : mémoire contre SQLite
- courbe 24 h d'un capteur à 1 Hz réduite à 500 points : mémoire 0.10 ms ; SQLite 21.83 ms (500 seaux)
- redémarrage à chaud, 7 jours en mémoire (1129783 relevés) : relus depuis SQLite 133.56 ms ; depuis un instantané binaire de 8.6 Mio 4.34 ms

## B2.4 — compiler les automatismes à la validation
- 2 000 automatismes lus depuis leur JSON et compilés : 23.52 ms au total, 11.8 µs chacun
```

### Linux — moteur réel (moteur 50 100 200 400)

```text
# Le vrai moteur d'automatismes sous charge (2 workers, machine de test)
Chaque automatisme : 1 déclencheur, 3 conditions imbriquées de 3 règles, 1 action ;
10 % sont temporaires (« allume 30 min »), 10 % attendent une condition (wait_for).
N =  50 : chargement 0.39 s, mémoire du moteur 691 Kio ; croisière p50 0.9 ms p99 1.2 ms (20 actions)
        rafale  2000 : sondes au milieu p50 9.8 ms max 11.6 ms perdues 0 ; sonde après p50 20.0 ms ; retards du moteur 0 (0 événements manqués)
        rafale  5000 : sondes au milieu p50 20.4 ms max 22.2 ms perdues 0 ; sonde après p50 — ; retards du moteur 2 (1055 événements manqués)
N = 100 : chargement 1.34 s, mémoire du moteur 1141 Kio ; croisière p50 0.9 ms p99 1.2 ms (19 actions)
        rafale  2000 : sondes au milieu p50 15.4 ms max 16.1 ms perdues 0 ; sonde après p50 25.4 ms ; retards du moteur 0 (0 événements manqués)
        rafale  5000 : sondes au milieu p50 32.2 ms max 34.7 ms perdues 0 ; sonde après p50 — ; retards du moteur 4 (931 événements manqués)
N = 200 : chargement 6.44 s, mémoire du moteur 2041 Kio ; croisière p50 0.9 ms p99 1.2 ms (19 actions)
        rafale  2000 : sondes au milieu p50 24.9 ms max 26.7 ms perdues 0 ; sonde après p50 42.6 ms ; retards du moteur 0 (0 événements manqués)
        rafale  5000 : sondes au milieu p50 — max — perdues 9 ; sonde après p50 — ; retards du moteur 2 (2707 événements manqués)
N = 400 : chargement 19.14 s, mémoire du moteur 3716 Kio ; croisière p50 1.1 ms p99 1.4 ms (19 actions)
        rafale  2000 : sondes au milieu p50 44.1 ms max 49.2 ms perdues 0 ; sonde après p50 78.5 ms ; retards du moteur 0 (0 événements manqués)
        rafale  5000 : sondes au milieu p50 102.4 ms max 105.7 ms perdues 0 ; sonde après p50 — ; retards du moteur 2 (1453 événements manqués)
```

### Linux — cloisons (cloisons)

```text
# Cloisons : un matériel muet ou un pilote bloquant ne doit pas arrêter le reste

## C1 — un thread par pilote contre les deux workers partagés
- 2 pilotes bloquants partagés :   7 ordres en 2 s, médiane 202.3 ms, pire 209.3 ms ; pire retard d'une minuterie de 10 ms  405.0 ms
- 6 pilotes bloquants isolés   :  87 ordres en 2 s, médiane   0.1 ms, pire   0.2 ms ; pire retard d'une minuterie de 10 ms    6.8 ms
- coût : 40 pilotes isolés au repos ajoutent 0 Kio de RSS, soit 0 Kio par pilote (pile de 2 Mio réservée, seules les pages touchées comptent)

## C2 — un pont, 20 lampes, une lampe muette (API du pont sans limite de concurrence)
- boucle unique       : lampes saines 41/237 réussis, médiane 4419.5 ms, pire 4679.4 ms ; lampe muette : médiane 5001.0 ms, réponse la plus rapide   0.0 ms
- voie par appareil   : lampes saines 237/237 réussis, médiane  34.8 ms, pire   41.5 ms ; lampe muette : médiane 5003.6 ms, réponse la plus rapide 4003.6 ms
- voies + disjoncteur : lampes saines 237/237 réussis, médiane  34.4 ms, pire   42.0 ms ; lampe muette : médiane 2825.1 ms, réponse la plus rapide   0.0 ms

## C3 — le même pont, mais son API ne traite que 4 requêtes à la fois
- tâche par ordre     : lampes saines 237/237 réussis, médiane 1347.4 ms, pire 3679.7 ms ; lampe muette : médiane 5002.9 ms, réponse la plus rapide 4000.9 ms
- voie par appareil   : lampes saines 237/237 réussis, médiane  34.5 ms, pire   41.4 ms ; lampe muette : médiane 5002.7 ms, réponse la plus rapide 4002.4 ms
- voies + disjoncteur : lampes saines 237/237 réussis, médiane  34.9 ms, pire   41.8 ms ; lampe muette : médiane 2843.9 ms, réponse la plus rapide   0.0 ms
```

### Linux — pannes (pannes)

```text
# Pannes : un bus qui tombe, puis l'inondation à son retour

## P1 — un bus de 50 appareils mort 15 s (de 5 s à 20 s), 10 ordres/s vers lui, 10 ordres/s vers un autre pilote
- voies seules               : autre pilote p50 24.2 ms p99 31.2 ms ; ordres au bus pendant la panne : médiane 4003.0 ms (142 ordres) ; reprise : premier succès 31.8 ms après le retour du bus ; appareil du bus à la fin : Some(Some(true))
- voies + disjoncteur de bus : autre pilote p50 25.2 ms p99 31.9 ms ; ordres au bus pendant la panne : médiane 0.1 ms (143 ordres) ; reprise : premier succès 634.0 ms après le retour du bus ; appareil du bus à la fin : Some(Some(true))

## P2 — le bus revient et republie 5 000 valeurs d'un coup ; 400 automatismes réels
- sans tampon d'entrée                         : déclencheurs après l'inondation : 5 perdus sur 5, latence médiane — ; retards du moteur 2 (3247 événements manqués)
- avec tampon d'entrée (dernière valeur, 10 ms) : déclencheurs après l'inondation : 0 perdus sur 5, latence médiane 5.8 ms ; retards du moteur 0 (0 événements manqués)
- sans tampon d'entrée                         : déclencheurs après l'inondation : 5 perdus sur 5, latence médiane — ; retards du moteur 2 (3249 événements manqués)
- avec tampon d'entrée (dernière valeur, 10 ms) : déclencheurs après l'inondation : 0 perdus sur 5, latence médiane 15.2 ms ; retards du moteur 0 (0 événements manqués)
- sans tampon d'entrée                         : déclencheurs après l'inondation : 5 perdus sur 5, latence médiane — ; retards du moteur 2 (3173 événements manqués)
- avec tampon d'entrée (dernière valeur, 10 ms) : déclencheurs après l'inondation : 0 perdus sur 5, latence médiane 12.8 ms ; retards du moteur 0 (0 événements manqués)
```

### Linux — voies paresseuses (voies)

```text
# Voies paresseuses écrites pour de bon, sous la charge de la V2 et de la V3

## L1 — C2 rejoué : pont sans limite, 20 lampes, une muette, 25 ordres/s pendant 10 s
- voies paresseuses              : lampes saines 237/237 réussis, médiane 35.0 ms, pire  41.7 ms ; muette : médiane 5003.3 ms ; pic mémoire pendant la charge 472 Kio
- voies paresseuses + disjoncteur : lampes saines 237/237 réussis, médiane 35.3 ms, pire  41.7 ms ; muette : médiane 2841.5 ms ; pic mémoire pendant la charge 469 Kio

## L2 — C3 rejoué : l'API du pont ne traite que 4 requêtes à la fois
- voies paresseuses              : lampes saines 237/237 réussis, médiane 35.6 ms, pire  47.4 ms ; muette : médiane 5002.4 ms ; pic mémoire pendant la charge 472 Kio
- voies paresseuses + disjoncteur : lampes saines 237/237 réussis, médiane 35.6 ms, pire  44.3 ms ; muette : médiane 2897.8 ms ; pic mémoire pendant la charge 469 Kio

## L3 — le pont entier tombe de 2 s à 6 s (les 20 lampes muettes), puis revient
- voies paresseuses + disjoncteur : lampes saines 218/237 réussis, médiane 38.0 ms, pire 3139.3 ms ; muette : médiane 2651.2 ms ; pic mémoire pendant la charge 471 Kio
```

### Linux — bascule (bascule)

```text
# Bascule de bout en bout : deux processus, le principal gelé (SIGSTOP), la secours reprend

## rythme : 2 changements par seconde, machine gelée
- S1 : instantané de 410 Kio reçu et appliqué en 6.0 ms (2264 valeurs)
- S2 : 2 changements/s ; 11 reçus par la secours ; dernier écrit par le principal 11, dernier reçu 11 : 0 changements perdus (RPO)
- S2 : gel (machine morte) du principal → détection 993.1 ms (silence max 1000.0 ms) → hub de secours prêt 17.3 ms après la décision ; indisponibilité totale 1010.4 ms (RTO)
- S3 : principal : époque 1 périmée (la secours est à 2) : je reste passif, aucun appareil touché

## rythme : 200 changements par seconde, machine gelée
- S1 : instantané de 410 Kio reçu et appliqué en 16.4 ms (2264 valeurs)
- S2 : 200 changements/s ; 1004 reçus par la secours ; dernier écrit par le principal 1004, dernier reçu 1004 : 0 changements perdus (RPO)
- S2 : gel (machine morte) du principal → détection 1022.6 ms (silence max 1000.0 ms) → hub de secours prêt 8.6 ms après la décision ; indisponibilité totale 1031.1 ms (RTO)
- S3 : principal : époque 1 périmée (la secours est à 2) : je reste passif, aucun appareil touché

## rythme : 200 changements par seconde, processus tué
- S1 : instantané de 410 Kio reçu et appliqué en 8.3 ms (2264 valeurs)
- S2 : 200 changements/s ; 1003 reçus par la secours ; dernier écrit par le principal 1003, dernier reçu 1003 : 0 changements perdus (RPO)
- S2 : mort du processus (connexion fermée) du principal → détection 0.0 ms (silence max 1000.0 ms) → hub de secours prêt 6.4 ms après la décision ; indisponibilité totale 6.4 ms (RTO)
- S3 : principal : époque 1 périmée (la secours est à 2) : je reste passif, aucun appareil touché
```

### Linux — plafond du moteur (capacite)

```text
# Plafond du moteur actuel : 400 automatismes, bruit soutenu pendant 3 s sur 700 capteurs

## sans tampon d'entrée
-  1000 événements/s : débit réel 999/s ; 0 déclencheurs perdus sur 6 ; latence médiane 1.0 ms, pire 1.5 ms ; retards du moteur 0 (0 événements manqués)
-  4000 événements/s : débit réel 3993/s ; 0 déclencheurs perdus sur 6 ; latence médiane 1.2 ms, pire 1.7 ms ; retards du moteur 0 (0 événements manqués)
- 16000 événements/s : débit réel 15973/s ; 0 déclencheurs perdus sur 6 ; latence médiane 0.9 ms, pire 1.2 ms ; retards du moteur 0 (0 événements manqués)
- 32000 événements/s : débit réel 31963/s ; 0 déclencheurs perdus sur 6 ; latence médiane 1.9 ms, pire 10.8 ms ; retards du moteur 0 (0 événements manqués)
- 64000 événements/s : débit réel 63918/s ; 0 déclencheurs perdus sur 6 ; latence médiane 3.9 ms, pire 6.4 ms ; retards du moteur 0 (0 événements manqués)
- 128000 événements/s : débit réel 127792/s ; 1 déclencheurs perdus sur 6 ; latence médiane 2.8 ms, pire 3.5 ms ; retards du moteur 24 (43864 événements manqués)

## avec tampon d'entrée (dernière valeur, 10 ms)
-  1000 événements/s : débit réel 999/s ; 0 déclencheurs perdus sur 6 ; latence médiane 11.5 ms, pire 12.1 ms ; retards du moteur 0 (0 événements manqués)
-  4000 événements/s : débit réel 3996/s ; 0 déclencheurs perdus sur 6 ; latence médiane 10.4 ms, pire 12.9 ms ; retards du moteur 0 (0 événements manqués)
- 16000 événements/s : débit réel 15966/s ; 0 déclencheurs perdus sur 6 ; latence médiane 5.4 ms, pire 12.5 ms ; retards du moteur 0 (0 événements manqués)
- 32000 événements/s : débit réel 31962/s ; 0 déclencheurs perdus sur 6 ; latence médiane 13.0 ms, pire 14.2 ms ; retards du moteur 0 (0 événements manqués)
- 64000 événements/s : débit réel 63937/s ; 0 déclencheurs perdus sur 6 ; latence médiane 8.9 ms, pire 13.2 ms ; retards du moteur 0 (0 événements manqués)
- 128000 événements/s : débit réel 127854/s ; 0 déclencheurs perdus sur 6 ; latence médiane 13.7 ms, pire 15.6 ms ; retards du moteur 0 (0 événements manqués)
```

### Linux — bus capricieux (caprices)

```text
# Un bus qui répond mal : 10 ordres/s vers 10 de ses appareils pendant 20 s, un autre pilote à côté

## Q1 — bus lent : répond en 2,5 s, sous le délai de 4 s
- voies seules             : réussis  20/200 (p50 2510 ms, p95 3957 ms) ; échoués 180 (p50 5004 ms) ; attente totale des appelants  965.4 s ; bascules  0 ; autre pilote p99 31 ms
- disjoncteur simple (V3)  : réussis  20/200 (p50 2512 ms, p95 3981 ms) ; échoués 180 (p50 5004 ms) ; attente totale des appelants  965.7 s ; bascules  0 ; autre pilote p99 31 ms
- hystérésis + refus       : réussis  96/200 (p50 4455 ms, p95 4961 ms) ; échoués 104 (p50    0 ms) ; attente totale des appelants  428.1 s ; bascules  1 ; autre pilote p99 31 ms
- adaptatif + réessais     : réussis  92/200 (p50 4330 ms, p95 4862 ms) ; échoués 108 (p50    0 ms) ; attente totale des appelants  404.4 s ; bascules  3 ; autre pilote p99 31 ms

## Q2 — bus qui bat : mort 1,5 s, vivant 1,5 s
- voies seules             : réussis 144/200 (p50 1772 ms, p95 4871 ms) ; échoués  56 (p50 4010 ms) ; attente totale des appelants  516.4 s ; bascules  0 ; autre pilote p99 31 ms
- disjoncteur simple (V3)  : réussis  34/200 (p50   26 ms, p95  361 ms) ; échoués 166 (p50 1898 ms) ; attente totale des appelants  365.0 s ; bascules 11 ; autre pilote p99 31 ms
- hystérésis + refus       : réussis  15/200 (p50   27 ms, p95   31 ms) ; échoués 185 (p50    0 ms) ; attente totale des appelants  133.3 s ; bascules  1 ; autre pilote p99 31 ms
- adaptatif + réessais     : réussis 200/200 (p50  471 ms, p95 1543 ms) ; échoués   0 (p50       —) ; attente totale des appelants  104.7 s ; bascules  0 ; autre pilote p99 31 ms

## Q3 — bus partiel : une requête sur deux n'aboutit jamais
- voies seules             : réussis  62/200 (p50 4295 ms, p95 4873 ms) ; échoués 138 (p50 5003 ms) ; attente totale des appelants  893.5 s ; bascules  0 ; autre pilote p99 31 ms
- disjoncteur simple (V3)  : réussis  34/200 (p50   26 ms, p95   33 ms) ; échoués 166 (p50 1865 ms) ; attente totale des appelants  297.8 s ; bascules 21 ; autre pilote p99 31 ms
- hystérésis + refus       : réussis  62/200 (p50 4261 ms, p95 4882 ms) ; échoués 138 (p50 5003 ms) ; attente totale des appelants  890.8 s ; bascules  1 ; autre pilote p99 31 ms
- adaptatif + réessais     : réussis 148/200 (p50 2912 ms, p95 4472 ms) ; échoués  52 (p50 5003 ms) ; attente totale des appelants  553.5 s ; bascules  4 ; autre pilote p99 31 ms
```

### Linux — correctif sur copie (fix/moteur 100 400)

```text
# Le vrai moteur d'automatismes sous charge (2 workers, machine de test)
Chaque automatisme : 1 déclencheur, 3 conditions imbriquées de 3 règles, 1 action ;
10 % sont temporaires (« allume 30 min »), 10 % attendent une condition (wait_for).
N = 100 : chargement 0.79 s, mémoire du moteur 1141 Kio ; croisière p50 1.7 ms p99 2.2 ms (19 actions)
        rafale  2000 : sondes au milieu p50 22.8 ms max 24.2 ms perdues 0 ; sonde après p50 38.4 ms ; retards du moteur 0 (0 événements manqués)
        rafale  5000 : sondes au milieu p50 41.8 ms max 43.0 ms perdues 0 ; sonde après p50 78.8 ms ; retards du moteur 4 (1194 événements manqués)
N = 400 : chargement 10.94 s, mémoire du moteur 3716 Kio ; croisière p50 2.0 ms p99 3.7 ms (19 actions)
        rafale  2000 : sondes au milieu p50 61.5 ms max 65.9 ms perdues 0 ; sonde après p50 90.4 ms ; retards du moteur 0 (0 événements manqués)
        rafale  5000 : sondes au milieu p50 75.4 ms max 76.3 ms perdues 0 ; sonde après p50 242.1 ms ; retards du moteur 5 (695 événements manqués)
```

### Linux — threads et processus (linux-couts)

```text
- Linux musl : processus minimal avec un runtime Tokio : 577 Kio de RSS en moyenne
- Linux musl : 40 threads avec chacun un runtime Tokio : 788 Kio de RSS, soit 19 Kio par pilote
```

Codes sources :

Code complet de `preuves-v2/src/bin/caprices.rs` :

```rust
//! Q1-Q3 : un bus qui répond mal, pas seulement un bus mort. Lent (sous le délai), qui bat
//! (tombe et revient sans cesse), partiel (une requête sur deux échoue). Trois protections
//! comparées : voies seules, disjoncteur simple (celui de la V3), disjoncteur à hystérésis
//! avec budget de latence.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use moli_core::{Access, Device, DeviceId, InstanceId, Kind, Origin, PointId, PointSpec, Semantic, Value};
use moli_runtime::{BoxFuture, CommandRequest, Driver, DriverCtx, Hub, HubOptions, spawn_driver};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap()
}
fn ms(d: Duration) -> String {
    format!("{:.0} ms", d.as_secs_f64() * 1e3)
}
fn q(v: &mut [Duration], p: f64) -> String {
    if v.is_empty() {
        return "—".into();
    }
    v.sort();
    ms(v[((v.len() - 1) as f64 * p) as usize])
}
fn appareil(id: DeviceId, instance: &InstanceId) -> Device {
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
            key: "on".into(),
            label: "on".into(),
            kind: Kind::Binary,
            access: Access { read: true, write: true },
            unit: None,
            semantic: Semantic::OnOff,
        }],
    }
}

const APPAREILS: usize = 50;
const DELAI: Duration = Duration::from_secs(4);

#[derive(Clone, Copy, PartialEq)]
enum Caprice {
    /// Répond, mais en 2,5 s : sous le délai de 4 s, jamais en échec.
    Lent,
    /// Mort 1,5 s, vivant 1,5 s, sans fin.
    Battant,
    /// Une requête sur deux n'aboutit jamais (délai), l'autre répond en 20 ms.
    Partiel,
}

#[derive(Clone, Copy, PartialEq)]
enum Protection {
    Voies,
    /// V3 : trois échecs en 10 s ouvrent 1 s ; un essai referme.
    Simple,
    /// Hystérésis : s'ouvre sur trois échecs d'ordres consécutifs, reste ouvert au moins 5 s,
    /// essais espacés (1 s → 30 s), trois succès pour refermer ; budget de latence : au-delà de 1,5 s en médiane récente, le bus est
    /// « dégradé » : un ordre qui, vu la file devant lui, aboutirait après son échéance est
    /// refusé tout de suite au lieu de faire la queue pour rien.
    Hysteresis,
    /// Hystérésis + refus prédictif, plus un délai par tentative adapté au bus
    /// (3 × médiane récente, entre 500 ms et le temps restant) et des réessais tant que
    /// l'échéance le permet, pour les ordres idempotents (une consigne, jamais un appui).
    /// Le disjoncteur compte les ordres échoués, pas les tentatives.
    Adaptatif,
}

struct Transport {
    caprice: Caprice,
    debut: Instant,
    compteur: AtomicUsize,
}

impl Transport {
    async fn requete(&self, delai: Duration) -> Result<(), String> {
        let n = self.compteur.fetch_add(1, Ordering::Relaxed);
        let reponse = match self.caprice {
            Caprice::Lent => Some(Duration::from_millis(2_500)),
            Caprice::Battant => {
                let phase = (self.debut.elapsed().as_millis() / 1_500) % 2;
                (phase == 0).then_some(Duration::from_millis(20))
            }
            Caprice::Partiel => (n % 2 == 0).then_some(Duration::from_millis(20)),
        };
        match reponse {
            Some(d) if d <= delai => {
                tokio::time::sleep(d).await;
                Ok(())
            }
            _ => {
                tokio::time::sleep(delai).await;
                Err("délai dépassé".into())
            }
        }
    }
}

#[derive(Default)]
struct Etat {
    echecs: VecDeque<Instant>,
    echecs_consecutifs: u32,
    latences: VecDeque<Duration>,
    ouvert_depuis: Option<Instant>,
    prochain_essai: Option<Instant>,
    intervalle_essai: Duration,
    succes_consecutifs: u32,
    essai_en_cours: bool,
    degrade: bool,
    bascules: usize,
}

impl Etat {
    fn mediane_recente(&self) -> Duration {
        if self.latences.is_empty() {
            return Duration::ZERO;
        }
        let mut v: Vec<Duration> = self.latences.iter().copied().collect();
        v.sort();
        v[v.len() / 2]
    }
}

struct Bus {
    transport: Arc<Transport>,
    protection: Protection,
    etat: Arc<Mutex<Etat>>,
}

impl Driver for Bus {
    fn kind(&self) -> &'static str {
        "bus"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            for i in 0..APPAREILS {
                let id = ctx.device_id(&format!("a{i}"));
                ctx.upsert_device(appareil(id.clone(), &ctx.instance().clone()));
                ctx.set_availability(&id, true);
            }
            ctx.ready();
            let mut voies = Vec::new();
            let mut taches = tokio::task::JoinSet::new();
            for _ in 0..APPAREILS {
                let (tx, mut rx) = mpsc::channel::<CommandRequest>(8);
                voies.push(tx);
                let (transport, etat, protection) = (self.transport.clone(), self.etat.clone(), self.protection);
                taches.spawn(async move {
                    while let Some(cmd) = rx.recv().await {
                        if cmd.abandoned() {
                            cmd.reply(Err("expiré avant l'envoi".into()));
                            continue;
                        }
                        let mut delai = DELAI;
                        let mut essai = false;
                        if protection != Protection::Voies {
                            let mut e = etat.lock().unwrap();
                            let maintenant = Instant::now();
                            if let Some(depuis) = e.ouvert_depuis {
                                let peut_essayer = match protection {
                                    Protection::Simple => maintenant >= depuis + Duration::from_secs(1),
                                    _ => maintenant >= depuis + Duration::from_secs(5) && e.prochain_essai.is_none_or(|t| maintenant >= t),
                                };
                                if !peut_essayer || e.essai_en_cours {
                                    drop(e);
                                    cmd.reply(Err("bus injoignable".into()));
                                    continue;
                                }
                                e.essai_en_cours = true;
                                essai = true;
                                delai = Duration::from_millis(500);
                            }
                        }
                        let t0 = Instant::now();
                        let r = if protection == Protection::Adaptatif {
                            let mut r = Err("jamais tenté".to_owned());
                            let mut tentatives = 0;
                            loop {
                                let restant = cmd.deadline.saturating_duration_since(Instant::now());
                                if restant < Duration::from_millis(50) {
                                    break;
                                }
                                let (mediane, connue) = {
                                    let e = etat.lock().unwrap();
                                    (e.mediane_recente(), e.latences.len() >= 5)
                                };
                                // Tant que la latence du bus n'est pas connue (moins de 5 réponses),
                                // une seule tentative avec tout le temps restant : réessayer n'aide
                                // qu'un bus rapide qui perd des requêtes, jamais un bus lent.
                                let par_tentative = if connue {
                                    (mediane * 3).max(Duration::from_millis(500)).min(restant)
                                } else {
                                    restant
                                };
                                tentatives += 1;
                                r = transport.requete(par_tentative).await;
                                if r.is_ok() {
                                    break;
                                }
                            }
                            let _ = tentatives;
                            r
                        } else {
                            transport.requete(delai).await
                        };
                        if protection != Protection::Voies {
                            let mut e = etat.lock().unwrap();
                            let maintenant = Instant::now();
                            if essai {
                                e.essai_en_cours = false;
                            }
                            match &r {
                                Ok(()) => {
                                    e.echecs_consecutifs = 0;
                                    e.latences.push_back(t0.elapsed());
                                    if e.latences.len() > 20 {
                                        e.latences.pop_front();
                                    }
                                    e.succes_consecutifs += 1;
                                    let referme = match protection {
                                        Protection::Simple => true,
                                        _ => e.succes_consecutifs >= 3,
                                    };
                                    if e.ouvert_depuis.is_some() && referme {
                                        e.ouvert_depuis = None;
                                        e.prochain_essai = None;
                                        e.intervalle_essai = Duration::from_secs(1);
                                        e.bascules += 1;
                                    } else if e.ouvert_depuis.is_some() {
                                        // Un succès d'essai : le suivant tout de suite.
                                        e.prochain_essai = Some(maintenant);
                                    }
                                }
                                Err(_) => {
                                    e.succes_consecutifs = 0;
                                    e.echecs_consecutifs += 1;
                                    e.echecs.push_back(maintenant);
                                    while e.echecs.front().is_some_and(|t| maintenant - *t > Duration::from_secs(10)) {
                                        e.echecs.pop_front();
                                    }
                                    if e.ouvert_depuis.is_some() {
                                        if matches!(protection, Protection::Hysteresis | Protection::Adaptatif) {
                                            e.intervalle_essai = (e.intervalle_essai * 2).max(Duration::from_secs(1)).min(Duration::from_secs(30));
                                            e.prochain_essai = Some(maintenant + e.intervalle_essai);
                                        }
                                    } else if match protection {
                                        Protection::Simple => e.echecs.len() >= 3,
                                        _ => e.echecs_consecutifs >= 3,
                                    } {
                                        e.ouvert_depuis = Some(maintenant);
                                        e.intervalle_essai = Duration::from_secs(1);
                                        e.prochain_essai = Some(maintenant + Duration::from_secs(5));
                                        e.bascules += 1;
                                        while let Ok(c) = rx.try_recv() {
                                            c.reply(Err("bus injoignable".into()));
                                        }
                                    }
                                }
                            }
                            if matches!(protection, Protection::Hysteresis | Protection::Adaptatif) {
                                let lent = e.mediane_recente() > Duration::from_millis(1_500);
                                if lent != e.degrade {
                                    e.degrade = lent;
                                    e.bascules += 1;
                                }
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
                        if matches!(self.protection, Protection::Hysteresis | Protection::Adaptatif) {
                            let e = self.etat.lock().unwrap();
                            if e.degrade {
                                let devant = (8 - voies[i].capacity()) as u32 + 1;
                                let prevu = e.mediane_recente() * devant;
                                let restant = cmd.deadline.saturating_duration_since(Instant::now());
                                if prevu > restant {
                                    drop(e);
                                    cmd.reply(Err("bus dégradé : l'attente dépasserait l'échéance".into()));
                                    continue;
                                }
                            }
                        }
                        if let Err(e) = voies[i].try_send(cmd) {
                            let c = match e { mpsc::error::TrySendError::Full(c) | mpsc::error::TrySendError::Closed(c) => c };
                            c.reply(Err("file de l'appareil pleine".into()));
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
            ctx.upsert_device(appareil(id, &ctx.instance().clone()));
            ctx.ready();
            while let Some(cmd) = ctx.next_command().await {
                tokio::time::sleep(Duration::from_millis(20)).await;
                cmd.reply(Ok(()));
            }
            Ok(())
        })
    }
}

fn essai(caprice: Caprice, protection: Protection) -> String {
    rt().block_on(async {
        let hub = Hub::new(HubOptions::default()).unwrap();
        let cancel = CancellationToken::new();
        let transport = Arc::new(Transport { caprice, debut: Instant::now(), compteur: AtomicUsize::new(0) });
        let etat = Arc::new(Mutex::new(Etat { intervalle_essai: Duration::from_secs(1), ..Default::default() }));
        spawn_driver(&hub, "bus".into(), Arc::new(Bus { transport, protection, etat: etat.clone() }), cancel.clone());
        spawn_driver(&hub, "sain".into(), Arc::new(Sain), cancel.clone());
        while hub.device(&DeviceId::from("bus:a49")).is_none() || hub.device(&DeviceId::from("sain:l1")).is_none() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let bus_ok = Arc::new(Mutex::new(Vec::new()));
        let bus_ko = Arc::new(Mutex::new(Vec::new()));
        let sain = Arc::new(Mutex::new(Vec::new()));
        let mut taches = Vec::new();
        // 20 s : 10 ordres par seconde vers le bus, 10 vers l'autre pilote.
        for n in 0..200usize {
            let (h, ok, ko) = (hub.clone(), bus_ok.clone(), bus_ko.clone());
            taches.push(tokio::spawn(async move {
                let p = PointId::from(format!("bus:a{}/on", (n * 7) % 10));
                let t0 = Instant::now();
                let r = h.command(&p, Value::Bool(n % 2 == 0), Origin::Automation, None).await;
                if r.is_ok() { ok.lock().unwrap().push(t0.elapsed()) } else { ko.lock().unwrap().push(t0.elapsed()) }
            }));
            let (h, s) = (hub.clone(), sain.clone());
            taches.push(tokio::spawn(async move {
                let t0 = Instant::now();
                let _ = h.command(&PointId::from("sain:l1/on"), Value::Bool(n % 2 == 0), Origin::Automation, None).await;
                s.lock().unwrap().push(t0.elapsed());
            }));
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        for t in taches {
            let _ = t.await;
        }
        cancel.cancel();
        let mut ok = bus_ok.lock().unwrap().clone();
        let mut ko = bus_ko.lock().unwrap().clone();
        let mut s = sain.lock().unwrap().clone();
        let attente: Duration = ok.iter().chain(ko.iter()).sum();
        format!(
            "réussis {:>3}/200 (p50 {:>7}, p95 {:>7}) ; échoués {:>3} (p50 {:>7}) ; attente totale des appelants {:>6.1} s ; bascules {:>2} ; autre pilote p99 {}",
            ok.len(),
            q(&mut ok, 0.5),
            q(&mut ok, 0.95),
            ko.len(),
            q(&mut ko, 0.5),
            attente.as_secs_f64(),
            etat.lock().unwrap().bascules,
            q(&mut s, 0.99)
        )
    })
}

fn main() {
    println!("# Un bus qui répond mal : 10 ordres/s vers 10 de ses appareils pendant 20 s, un autre pilote à côté");
    for (titre, caprice) in [("Q1 — bus lent : répond en 2,5 s, sous le délai de 4 s", Caprice::Lent), ("Q2 — bus qui bat : mort 1,5 s, vivant 1,5 s", Caprice::Battant), ("Q3 — bus partiel : une requête sur deux n'aboutit jamais", Caprice::Partiel)] {
        println!("\n## {titre}");
        for (nom, p) in [("voies seules            ", Protection::Voies), ("disjoncteur simple (V3) ", Protection::Simple), ("hystérésis + refus      ", Protection::Hysteresis), ("adaptatif + réessais    ", Protection::Adaptatif)] {
            println!("- {nom} : {}", essai(caprice, p));
        }
    }
}
```

Code complet de `preuves-v2/src/bin/voies.rs` :

```rust
//! L1-L3 : les voies paresseuses, écrites pour de bon (un répartiteur, des files créées à la
//! demande, un ordre en vol par appareil, disjoncteur), rejouées sous la charge de la V2
//! (C2, C3) et de la V3 (P1), avec la mémoire mesurée pendant la charge.

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use moli_core::{Access, Device, DeviceId, InstanceId, Kind, Origin, PointId, PointSpec, Semantic, Value};
use moli_runtime::{BoxFuture, CommandRequest, Driver, DriverCtx, Hub, HubOptions, spawn_driver};
use tokio::sync::{Semaphore, mpsc};
use tokio_util::sync::CancellationToken;

struct Compte;
static OCTETS: AtomicIsize = AtomicIsize::new(0);
static PIC: AtomicIsize = AtomicIsize::new(0);
unsafe impl GlobalAlloc for Compte {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            let n = OCTETS.fetch_add(l.size() as isize, Ordering::Relaxed) + l.size() as isize;
            PIC.fetch_max(n, Ordering::Relaxed);
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
            let m = OCTETS.fetch_add(n as isize - l.size() as isize, Ordering::Relaxed) + n as isize - l.size() as isize;
            PIC.fetch_max(m, Ordering::Relaxed);
        }
        q
    }
}
#[global_allocator]
static A: Compte = Compte;

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

// ---- les voies paresseuses : la bibliothèque proposée pour moli-runtime ------------

/// Résultat d'un envoi, rendu par la tâche d'envoi au répartiteur.
struct Fin {
    appareil: usize,
    ok: bool,
}

/// Une file par appareil, créée au premier ordre en attente et détruite une fois vide ;
/// un seul ordre en vol par appareil ; la consigne qui attend est remplacée par la
/// suivante (l'ancienne répond « remplacée ») ; disjoncteur par appareil.
struct Voies {
    attente: HashMap<usize, VecDeque<CommandRequest>>,
    en_vol: Vec<bool>,
    echecs: Vec<u8>,
    disjoncte_jusqua: Vec<Option<Instant>>,
    max_attente: usize,
    disjoncteur: bool,
    fins_tx: mpsc::UnboundedSender<Fin>,
}

impl Voies {
    fn new(appareils: usize, disjoncteur: bool, fins_tx: mpsc::UnboundedSender<Fin>) -> Self {
        Self {
            attente: HashMap::new(),
            en_vol: vec![false; appareils],
            echecs: vec![0; appareils],
            disjoncte_jusqua: vec![None; appareils],
            max_attente: 8,
            disjoncteur,
            fins_tx,
        }
    }

    /// Un ordre arrive : refusé si le disjoncteur est ouvert, envoyé s'il n'y a rien en
    /// vol, sinon mis en attente (au plus `max_attente`, la plus ancienne consigne cède).
    fn ordre(&mut self, i: usize, cmd: CommandRequest, envoi: &dyn Fn(usize, CommandRequest, mpsc::UnboundedSender<Fin>)) {
        if let Some(t) = self.disjoncte_jusqua[i] {
            if Instant::now() < t {
                cmd.reply(Err("injoignable : disjoncteur ouvert".into()));
                return;
            }
            self.disjoncte_jusqua[i] = None;
        }
        if !self.en_vol[i] {
            self.en_vol[i] = true;
            envoi(i, cmd, self.fins_tx.clone());
            return;
        }
        let file = self.attente.entry(i).or_default();
        if file.len() >= self.max_attente {
            if let Some(ancienne) = file.pop_front() {
                ancienne.reply(Err("remplacée par une consigne plus récente".into()));
            }
        }
        file.push_back(cmd);
    }

    /// Un envoi se termine : l'ordre suivant de l'appareil part, ou sa file disparaît.
    fn fin(&mut self, fin: Fin, envoi: &dyn Fn(usize, CommandRequest, mpsc::UnboundedSender<Fin>)) -> Option<bool> {
        let i = fin.appareil;
        let mut tombe = None;
        if self.disjoncteur {
            if fin.ok {
                self.echecs[i] = 0;
            } else {
                self.echecs[i] += 1;
                if self.echecs[i] >= 2 {
                    self.disjoncte_jusqua[i] = Some(Instant::now() + Duration::from_secs(30));
                    if let Some(file) = self.attente.remove(&i) {
                        for c in file {
                            c.reply(Err("injoignable : disjoncteur ouvert".into()));
                        }
                    }
                    tombe = Some(false);
                }
            }
        }
        let suivant = self.attente.get_mut(&i).and_then(|f| {
            let c = f.pop_front();
            c
        });
        if self.attente.get(&i).is_some_and(VecDeque::is_empty) {
            self.attente.remove(&i);
        }
        match suivant {
            Some(cmd) if cmd.abandoned() => {
                cmd.reply(Err("expiré avant l'envoi".into()));
                self.en_vol[i] = false;
            }
            Some(cmd) => envoi(i, cmd, self.fins_tx.clone()),
            None => self.en_vol[i] = false,
        }
        tombe
    }
}

// ---- un pont, vingt lampes, une muette, comme en V2 ---------------------------------

const LAMPES: usize = 20;
const MUETTE: usize = 0;

struct Pont {
    api: Arc<Semaphore>,
    disjoncteur: bool,
    vivant: Arc<AtomicBool>,
}

async fn appel(api: &Semaphore, vivant: &AtomicBool, appareil: usize) -> bool {
    let _jeton = api.acquire().await.unwrap();
    if appareil == MUETTE || !vivant.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_secs(4)).await;
        false
    } else {
        tokio::time::sleep(Duration::from_millis(30)).await;
        true
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
            let (fins_tx, mut fins_rx) = mpsc::unbounded_channel();
            let mut voies = Voies::new(LAMPES, self.disjoncteur, fins_tx);
            let (api, vivant) = (self.api.clone(), self.vivant.clone());
            let envoi = move |i: usize, cmd: CommandRequest, fins: mpsc::UnboundedSender<Fin>| {
                let (api, vivant) = (api.clone(), vivant.clone());
                tokio::spawn(async move {
                    let ok = appel(&api, &vivant, i).await;
                    cmd.reply(if ok { Ok(()) } else { Err("délai dépassé".into()) });
                    let _ = fins.send(Fin { appareil: i, ok });
                });
            };
            loop {
                tokio::select! {
                    cmd = ctx.next_command() => {
                        let Some(cmd) = cmd else { break };
                        let i = indice(&cmd);
                        voies.ordre(i, cmd, &envoi);
                    }
                    Some(fin) = fins_rx.recv() => {
                        let i = fin.appareil;
                        if let Some(en_ligne) = voies.fin(fin, &envoi) {
                            ctx.set_availability(&ctx.device_id(&format!("lampe{i}")), en_ligne);
                        }
                    }
                }
            }
            Ok(())
        })
    }
}

fn pont(disjoncteur: bool, concurrence: usize, panne_du_pont: bool) -> String {
    rt().block_on(async {
        let hub = Hub::new(HubOptions::default()).unwrap();
        let cancel = CancellationToken::new();
        let vivant = Arc::new(AtomicBool::new(true));
        spawn_driver(&hub, "pont".into(), Arc::new(Pont { api: Arc::new(Semaphore::new(concurrence)), disjoncteur, vivant: vivant.clone() }), cancel.clone());
        while hub.device(&DeviceId::from(format!("pont:lampe{}", LAMPES - 1))).is_none() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        PIC.store(OCTETS.load(Ordering::Relaxed), Ordering::Relaxed);
        let base = OCTETS.load(Ordering::Relaxed);
        let saines = Arc::new(Mutex::new(Vec::new()));
        let echecs_sains = Arc::new(AtomicUsize::new(0));
        let muette = Arc::new(Mutex::new(Vec::new()));
        let mut taches = Vec::new();
        let debut = Instant::now();
        for n in 0..250usize {
            if panne_du_pont {
                let t = debut.elapsed();
                vivant.store(!(Duration::from_secs(2)..Duration::from_secs(6)).contains(&t), Ordering::Release);
            }
            let i = n % LAMPES;
            let (h, s, e, m) = (hub.clone(), saines.clone(), echecs_sains.clone(), muette.clone());
            taches.push(tokio::spawn(async move {
                let p = PointId::from(format!("pont:lampe{i}/on"));
                let t = Instant::now();
                let r = h.command(&p, Value::Bool(n % 2 == 0), Origin::Ui, None).await;
                let d = t.elapsed();
                if i == MUETTE {
                    m.lock().unwrap().push(d);
                } else if r.is_ok() {
                    s.lock().unwrap().push(d);
                } else {
                    e.fetch_add(1, Ordering::Relaxed);
                }
            }));
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
        for t in taches {
            let _ = t.await;
        }
        let pic = PIC.load(Ordering::Relaxed) - base;
        cancel.cancel();
        let mut s = saines.lock().unwrap().clone();
        let mut m = muette.lock().unwrap().clone();
        format!(
            "lampes saines {}/{} réussis, médiane {:>7}, pire {:>8} ; muette : médiane {:>8} ; pic mémoire pendant la charge {} Kio",
            s.len(),
            s.len() + echecs_sains.load(Ordering::Relaxed),
            q(&mut s, 0.5),
            q(&mut s, 1.0),
            q(&mut m, 0.5),
            pic / 1024
        )
    })
}

fn main() {
    println!("# Voies paresseuses écrites pour de bon, sous la charge de la V2 et de la V3");
    println!("\n## L1 — C2 rejoué : pont sans limite, 20 lampes, une muette, 25 ordres/s pendant 10 s");
    println!("- voies paresseuses              : {}", pont(false, 1_000, false));
    println!("- voies paresseuses + disjoncteur : {}", pont(true, 1_000, false));
    println!("\n## L2 — C3 rejoué : l'API du pont ne traite que 4 requêtes à la fois");
    println!("- voies paresseuses              : {}", pont(false, 4, false));
    println!("- voies paresseuses + disjoncteur : {}", pont(true, 4, false));
    println!("\n## L3 — le pont entier tombe de 2 s à 6 s (les 20 lampes muettes), puis revient");
    println!("- voies paresseuses + disjoncteur : {}", pont(true, 1_000, true));
}
```

Code complet de `preuves-v2/src/bin/bascule.rs` :

```rust
//! S1-S3 : la bascule jouée de bout en bout, en deux processus sur la même machine.
//! Le principal (enfant) tient un hub et produit des changements à un rythme donné ; il
//! les réplique vers la secours (ce processus) par TCP, avec un instantané au début et un
//! battement de cœur. On gèle le principal (SIGSTOP : la machine « meurt » sans fermer
//! ses connexions), la secours détecte le silence, bascule avec une nouvelle époque, et
//! on mesure le délai et la perte. Puis l'ancien principal revient et doit rester passif.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use moli_core::{Access, Device, DeviceId, Kind, PointSpec, Semantic, Value};
use moli_runtime::{BoxFuture, Driver, DriverCtx, Hub, HubOptions, spawn_driver};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const APPAREILS: usize = 496;
const VALEURS: usize = 2_264;
const BATTEMENT: Duration = Duration::from_millis(100);
const SILENCE_MAX: Duration = Duration::from_millis(1_000);

fn ms(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1e3)
}

fn numerique(key: &str) -> PointSpec {
    PointSpec {
        key: key.into(),
        label: key.into(),
        kind: Kind::Numeric { min: None, max: None, step: None },
        access: Access { read: true, write: false },
        unit: None,
        semantic: Semantic::Other,
    }
}

/// La maison vue par un hub : 496 appareils, 2 264 valeurs, publiées depuis des deltas.
struct Maison {
    deltas: std::sync::Mutex<Option<mpsc::UnboundedReceiver<(u32, f64)>>>,
    initial: Vec<f64>,
}

fn point(i: usize) -> (DeviceId, String) {
    (DeviceId::from(format!("maison:d{}", i % APPAREILS)), format!("v{}", i / APPAREILS))
}

impl Driver for Maison {
    fn kind(&self) -> &'static str {
        "maison"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let instance = ctx.instance().clone();
            for d in 0..APPAREILS {
                let par = VALEURS / APPAREILS + usize::from(d < VALEURS % APPAREILS);
                let id = DeviceId::from(format!("maison:d{d}"));
                ctx.upsert_device(Device {
                    id: id.clone(),
                    instance: instance.clone(),
                    native_name: format!("Appareil fictif {d}").into(),
                    manufacturer: None,
                    model: None,
                    description: None,
                    native_room: None,
                    members: vec![],
                    points: (0..par).map(|k| numerique(&format!("v{k}"))).collect(),
                });
            }
            for (i, v) in self.initial.iter().enumerate() {
                let (id, key) = point(i);
                ctx.set_state(&id, &key, Value::Float(*v));
            }
            ctx.ready();
            let mut rx = self.deltas.lock().unwrap().take().unwrap();
            while let Some((i, v)) = rx.recv().await {
                let (id, key) = point(i as usize);
                ctx.set_state(&id, &key, Value::Float(v));
            }
            Ok(())
        })
    }
}

// ---- le principal ------------------------------------------------------------------

fn principal(port: u16, epoque: u64, rythme: u64) {
    let mut flux = TcpStream::connect(("127.0.0.1", port)).expect("secours joignable");
    flux.set_nodelay(true).unwrap();
    // Arbitrage d'époque : la secours annonce la sienne ; plus grande, on reste passif.
    writeln!(flux, "EPOQUE {epoque}").unwrap();
    let mut lecteur = BufReader::new(flux.try_clone().unwrap());
    let mut ligne = String::new();
    lecteur.read_line(&mut ligne).unwrap();
    let leur: u64 = ligne.trim().strip_prefix("EPOQUE ").unwrap().parse().unwrap();
    if leur > epoque {
        println!("principal : époque {epoque} périmée (la secours est à {leur}) : je reste passif, aucun appareil touché");
        return;
    }
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    rt.block_on(async move {
        let hub = Hub::new(HubOptions::default()).unwrap();
        let cancel = CancellationToken::new();
        let (tx, rx) = mpsc::unbounded_channel();
        let initial: Vec<f64> = (0..VALEURS).map(|i| i as f64 / 7.0).collect();
        spawn_driver(&hub, "maison".into(), Arc::new(Maison { deltas: std::sync::Mutex::new(Some(rx)), initial }), cancel.clone());
        while hub.stats().points < VALEURS {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        // Instantané initial, puis chaque changement en une ligne, et le battement.
        let instantane = serde_json::to_vec(&hub.snapshot()).unwrap();
        writeln!(flux, "INSTANTANE {}", instantane.len()).unwrap();
        flux.write_all(&instantane).unwrap();
        flux.write_all(b"\n").unwrap();
        let mut bus = hub.subscribe();
        let seq = Arc::new(AtomicU64::new(0));
        let journal_local = std::env::temp_dir().join(format!("moli-bascule-journal-{port}.txt"));
        let mut journal = std::fs::File::create(&journal_local).unwrap();
        let (s, t) = (seq.clone(), tx.clone());
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_micros(1_000_000 / rythme));
            let mut v = 1_000.0;
            loop {
                tick.tick().await;
                v += 1.0;
                let i = (s.load(Ordering::Relaxed) as usize * 7_919) % VALEURS;
                if t.send((i as u32, v)).is_err() {
                    return;
                }
            }
        });
        let mut battement = tokio::time::interval(BATTEMENT);
        loop {
            tokio::select! {
                e = bus.recv() => {
                    if let Ok(moli_core::Event::State { point, value, ts, .. }) = e {
                        let n = seq.fetch_add(1, Ordering::Relaxed) + 1;
                        let v = value.as_f64().unwrap_or(0.0);
                        // Le journal local est écrit sans tampon : ce que le noyau a reçu
                        // survit à la mort du processus, c'est la vérité du principal.
                        writeln!(journal, "{n} {point} {v} {ts}").unwrap();
                        if writeln!(flux, "C {n} {point} {v} {ts}").is_err() { return; }
                    }
                }
                _ = battement.tick() => {
                    if writeln!(flux, "H {}", seq.load(Ordering::Relaxed)).is_err() { return; }
                }
            }
        }
    });
}

// ---- la secours (et l'orchestration) -----------------------------------------------

struct Reprise {
    etat: Arc<std::sync::Mutex<HashMap<String, f64>>>,
}

fn secours(rythme: u64, gel: bool) {
    let ecoute = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = ecoute.local_addr().unwrap().port();
    let exe = std::env::current_exe().unwrap();
    let mut enfant = std::process::Command::new(&exe).args(["principal", &port.to_string(), "1", &rythme.to_string()]).spawn().unwrap();
    let (flux, _) = ecoute.accept().unwrap();
    let mut lecteur = BufReader::new(flux.try_clone().unwrap());
    let mut ecrivain = flux.try_clone().unwrap();
    let mut ligne = String::new();
    lecteur.read_line(&mut ligne).unwrap();
    let mut epoque: u64 = 1;
    writeln!(ecrivain, "EPOQUE {epoque}").unwrap();
    ligne.clear();
    lecteur.read_line(&mut ligne).unwrap();
    let taille: usize = ligne.trim().strip_prefix("INSTANTANE ").unwrap().parse().unwrap();
    let t_instantane = Instant::now();
    let mut brut = vec![0u8; taille + 1];
    std::io::Read::read_exact(&mut lecteur, &mut brut).unwrap();
    let instantane: serde_json::Value = serde_json::from_slice(&brut[..taille]).unwrap();
    // La secours tient son état à jour en mémoire (chaud), sans toucher à aucun appareil.
    let etat: Arc<std::sync::Mutex<HashMap<String, f64>>> = Arc::default();
    {
        let mut e = etat.lock().unwrap();
        for d in instantane["devices"].as_array().unwrap() {
            let id = d["id"].as_str().unwrap();
            for (k, s) in d["state"].as_object().unwrap() {
                e.insert(format!("{id}/{k}"), s["value"].as_f64().unwrap_or(0.0));
            }
        }
    }
    println!("- S1 : instantané de {} Kio reçu et appliqué en {} ({} valeurs)", taille / 1024, ms(t_instantane.elapsed()), etat.lock().unwrap().len());
    flux.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
    let debut = Instant::now();
    let mut dernier_seq = 0u64;
    let mut dernier_signe = Instant::now();
    let mut gele_a: Option<Instant> = None;
    let mut recus = 0u64;
    let mut detection = Instant::now();
    loop {
        if gele_a.is_none() && debut.elapsed() >= Duration::from_secs(5) {
            // La machine principale « meurt » : gelée, ses connexions restent ouvertes
            // (SIGSTOP) ; ou le processus seul meurt et le noyau ferme ses connexions (SIGKILL).
            std::process::Command::new("kill").args([if gel { "-STOP" } else { "-KILL" }, &enfant.id().to_string()]).status().unwrap();
            gele_a = Some(Instant::now());
        }
        ligne.clear();
        match lecteur.read_line(&mut ligne) {
            Ok(0) => {
                detection = Instant::now();
                break;
            }
            Ok(_) => {
                dernier_signe = Instant::now();
                let mut champs = ligne.split_whitespace();
                match champs.next() {
                    Some("C") => {
                        let n: u64 = champs.next().unwrap().parse().unwrap();
                        let p = champs.next().unwrap().to_owned();
                        let v: f64 = champs.next().unwrap().parse().unwrap();
                        etat.lock().unwrap().insert(p, v);
                        dernier_seq = n;
                        recus += 1;
                    }
                    Some("H") => {
                        dernier_seq = champs.next().unwrap().parse().unwrap();
                    }
                    _ => {}
                }
            }
            Err(_) => {
                if dernier_signe.elapsed() > SILENCE_MAX {
                    detection = Instant::now();
                    break;
                }
            }
        }
    }
    let gele = gele_a.unwrap();
    // Bascule : nouvelle époque, puis le hub de la secours prend le relais.
    epoque += 1;
    let t_bascule = Instant::now();
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let pret = rt.block_on(async {
        let hub = Hub::new(HubOptions::default()).unwrap();
        let cancel = CancellationToken::new();
        let (_tx, rx) = mpsc::unbounded_channel();
        let e = etat.lock().unwrap();
        let initial: Vec<f64> = (0..VALEURS).map(|i| { let (id, k) = point(i); *e.get(&format!("{id}/{k}")).unwrap_or(&0.0) }).collect();
        drop(e);
        spawn_driver(&hub, "maison".into(), Arc::new(Maison { deltas: std::sync::Mutex::new(Some(rx)), initial }), cancel.clone());
        while hub.stats().points < VALEURS {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        let pret = t_bascule.elapsed();
        let _ = Reprise { etat: etat.clone() };
        cancel.cancel();
        pret
    });
    std::process::Command::new("kill").args(["-KILL", &enfant.id().to_string()]).status().unwrap();
    let _ = enfant.wait();
    let journal = std::fs::read_to_string(std::env::temp_dir().join(format!("moli-bascule-journal-{port}.txt"))).unwrap_or_default();
    let dernier_ecrit: u64 = journal.lines().last().and_then(|l| l.split_whitespace().next()).and_then(|n| n.parse().ok()).unwrap_or(0);
    let _ = std::fs::remove_file(std::env::temp_dir().join(format!("moli-bascule-journal-{port}.txt")));
    println!(
        "- S2 : {rythme} changements/s ; {recus} reçus par la secours ; dernier écrit par le principal {dernier_ecrit}, dernier reçu {dernier_seq} : {} changements perdus (RPO)",
        dernier_ecrit.saturating_sub(dernier_seq)
    );
    println!(
        "- S2 : {} du principal → détection {} (silence max {}) → hub de secours prêt {} après la décision ; indisponibilité totale {} (RTO)",
        if gel { "gel (machine morte)" } else { "mort du processus (connexion fermée)" },
        ms(detection - gele),
        ms(SILENCE_MAX),
        ms(pret),
        ms(detection - gele + pret)
    );
    // L'ancien principal revient avec son ancienne époque.
    let mut revenant = std::process::Command::new(&exe).args(["principal", &port.to_string(), "1", "1"]).stdout(std::process::Stdio::piped()).spawn().unwrap();
    let (flux, _) = ecoute.accept().unwrap();
    let mut lecteur = BufReader::new(flux.try_clone().unwrap());
    let ecrivain = flux;
    ligne.clear();
    lecteur.read_line(&mut ligne).unwrap();
    writeln!(&ecrivain, "EPOQUE {epoque}").unwrap();
    let sortie = revenant.wait_with_output().unwrap();
    println!("- S3 : {}", String::from_utf8_lossy(&sortie.stdout).trim());
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("principal") {
        principal(args[2].parse().unwrap(), args[3].parse().unwrap(), args[4].parse().unwrap());
        return;
    }
    println!("# Bascule de bout en bout : deux processus, le principal gelé (SIGSTOP), la secours reprend");
    for (rythme, gel) in [(2u64, true), (200, true), (200, false)] {
        println!("\n## rythme : {rythme} changements par seconde, {}", if gel { "machine gelée" } else { "processus tué" });
        secours(rythme, gel);
    }
}
```

Code complet de `preuves-v2/src/bin/capacite.rs` :

```rust
//! T1 : le plafond du moteur actuel. À quel débit d'événements soutenu le vrai moteur
//! (400 automatismes) prend-il du retard sur le bus, avec et sans tampon d'entrée ?

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use moli_automation::{Actor, Author, Automation, Automations, Config, Graph, Mode};
use moli_core::{Access, Device, DeviceId, Kind, PointId, PointSpec, Semantic, Value};
use moli_runtime::{BoxFuture, Driver, DriverCtx, Hub, HubOptions, spawn_driver};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::layer::SubscriberExt;

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

fn essai(debit: u64, coalescence: bool, fois: &Arc<AtomicUsize>, manques: &Arc<AtomicU64>) -> String {
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    rt.block_on(async {
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
        // 3 s de bruit à `debit` événements/s sur 700 capteurs, une sonde chaque 500 ms.
        let mut v = 1.0;
        let mut sondes = Vec::new();
        let total = (debit * 3) as usize;
        let debut = Instant::now();
        let mut prochaine_sonde = Duration::from_millis(500);
        let mut j = 0usize;
        let mut envoyes = 0usize;
        // Le débit est tenu par lots d'une milliseconde : tout ce qui est dû part d'un coup
        // (la minuterie de Tokio ne descend pas sous la milliseconde).
        while envoyes < total {
            let t = debut.elapsed();
            let dus = ((t.as_secs_f64() * debit as f64) as usize).min(total);
            while envoyes < dus {
                v += 1.0;
                tx.send((1_000 + (envoyes * 7_919) % 700, v)).unwrap();
                envoyes += 1;
            }
            if t >= prochaine_sonde {
                prochaine_sonde += Duration::from_millis(500);
                v += 1.0;
                let k = 20 + 17 * j;
                j += 1;
                sondes.push((k, Instant::now()));
                tx.send((declencheur(k), v)).unwrap();
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        let reel = envoyes as f64 / debut.elapsed().as_secs_f64();
        tokio::time::sleep(Duration::from_secs(2)).await;
        let arr = arrivees.lock().unwrap();
        let mut lat: Vec<Duration> = Vec::new();
        let mut perdues = 0;
        for (k, t0) in &sondes {
            match arr[depuis..].iter().find(|(a, t)| a == k && t >= t0) {
                Some((_, t)) => lat.push(*t - *t0),
                None => perdues += 1,
            }
        }
        lat.sort();
        let med = lat.get(lat.len() / 2).map_or("—".into(), |d| format!("{:.1} ms", d.as_secs_f64() * 1e3));
        let max = lat.last().map_or("—".into(), |d| format!("{:.1} ms", d.as_secs_f64() * 1e3));
        cancel.cancel();
        format!(
            "débit réel {reel:.0}/s ; {} déclencheurs perdus sur {} ; latence médiane {med}, pire {max} ; retards du moteur {} ({} événements manqués)",
            perdues,
            sondes.len(),
            fois.load(Ordering::Relaxed),
            manques.load(Ordering::Relaxed)
        )
    })
}

fn main() {
    let fois = Arc::new(AtomicUsize::new(0));
    let manques = Arc::new(AtomicU64::new(0));
    tracing::subscriber::set_global_default(tracing_subscriber::registry().with(Retards { fois: fois.clone(), manques: manques.clone() })).unwrap();
    println!("# Plafond du moteur actuel : 400 automatismes, bruit soutenu pendant 3 s sur 700 capteurs");
    for coalescence in [false, true] {
        println!("\n## {}", if coalescence { "avec tampon d'entrée (dernière valeur, 10 ms)" } else { "sans tampon d'entrée" });
        for debit in [1_000u64, 4_000, 16_000, 32_000, 64_000, 128_000] {
            println!("- {debit:>5} événements/s : {}", essai(debit, coalescence, &fois, &manques));
        }
    }
}
```

Code complet de `preuves-v2/src/bin/etat_complet.rs (version V4, avec la mémoire réelle)` :

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


/// Mémoire réelle du processus, quand le système la donne (Linux) : RSS et mémoire
/// anonyme du cgroup, la mesure du budget de PERFORMANCE.md.
fn memoire_reelle() -> String {
    let rss = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("VmRSS:")).map(|l| l.split_whitespace().nth(1).unwrap_or("?").to_owned()));
    let anon = std::fs::read_to_string("/sys/fs/cgroup/memory.stat")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("anon ")).and_then(|l| l.split_whitespace().nth(1).map(|v| v.parse::<u64>().unwrap_or(0) / 1024)));
    match (rss, anon) {
        (Some(r), Some(a)) => format!("RSS {r} Kio, anonyme du cgroup {a} Kio"),
        (Some(r), None) => format!("RSS {r} Kio"),
        _ => "mémoire réelle non disponible sur ce système".into(),
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
        let reel_vif = memoire_reelle();
        let h24 = historique(1.0, echelle);
        let h30 = if echelle == 1 { historique(30.0, 1) } else { 0.0 };
        // 7 jours tenus en mémoire pendant la mesure réelle.
        let avant7 = tas();
        let mut tenu: HashMap<u32, (Vec<u32>, Vec<f32>)> = HashMap::new();
        for s in 0..SERIES * echelle {
            let n = (taux(s) * 7.0).round() as usize;
            let (t, v) = tenu.entry(s as u32).or_default();
            t.reserve_exact(n);
            v.reserve_exact(n);
            for i in 0..n {
                t.push(i as u32);
                v.push(i as f32);
            }
        }
        let h7 = depuis(avant7);
        let reel_7j = memoire_reelle();
        drop(tenu);
        let h30 = if echelle == 1 { h30 } else { h7 / 7.0 * 30.0 };

        println!("\n## ×{echelle} : {appareils} appareils, {valeurs} valeurs, {automatismes} automatismes");
        for (nom, o) in &lignes {
            println!("- {nom} : {}", kio(*o));
        }
        println!("- **Sous-total « tout l'état vif »** : {}", kio(indispensable));
        println!("- mémoire réelle avec tout l'état vif : {reel_vif}");
        println!("- mémoire réelle avec l'état vif et 7 jours d'historique : {reel_7j}");
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
    println!("- mémoire réelle au démarrage, avant toute charge : {}", memoire_reelle());
    budget(1);
    budget(10);
}
```

Code complet de `moli-automation-fix : le correctif (extrait d'engine.rs)` :

```rust
/// CORRECTIF (prototype hors dépôt). After a lag, the hub's present is compared
/// with what the engine last knew: every point whose value differs is a
/// transition the engine missed, and it is replayed as one. An event still
/// waiting in the queue is seen again later, compares equal, and is ignored:
/// nothing fires twice. (The previous code copied the present over `last`,
/// which also neutralised the queued events.)
fn resync(autos: &Automations) {
    let snapshot = autos.0.hub.snapshot();
    let changed: Vec<(PointId, Value)> = {
        let last = lock(&autos.0.last);
        snapshot
            .devices
            .iter()
            .flat_map(|d| {
                d.state.iter().filter_map(|(key, sample)| {
                    let k = format!("{}/{key}", d.device.id);
                    (last.get(&k) != Some(&sample.value))
                        .then(|| (PointId::from(k), sample.value.clone()))
                })
            })
            .collect()
    };
    for (point, value) in changed {
        on_state(autos, &point, &value);
    }
}
```

Code complet de `docker-build.sh : compilation Linux musl dans un conteneur` :

```sh
#!/bin/sh
# Compile les bancs en Linux musl (comme l'image de production) dans un conteneur jetable,
# avec un volume nommé pour le cache de compilation. Sortie : $S/linux-bins/ (preuves-v2)
# et $S/linux-bins/fix/ (preuves-fix, moteur corrigé).
set -eu
S=<scratchpad>
REPO=<dépôt>
CTX=$S/ctx
rm -rf "$CTX"; mkdir -p "$CTX/nikyOS" "$CTX/preuves-v2" "$CTX/preuves-fix" "$CTX/moli-automation-fix"
cp "$REPO/Cargo.toml" "$REPO/Cargo.lock" "$CTX/nikyOS/"
cp -R "$REPO/crates" "$REPO/locales" "$REPO/integrations" "$CTX/nikyOS/"
find "$CTX/nikyOS/crates" -name target -prune -exec rm -rf {} + 2>/dev/null || true
cp -R "$S/preuves-v2/src" "$S/preuves-v2/Cargo.lock" "$CTX/preuves-v2/"
sed 's#<dépôt>/crates/#/src/nikyOS/crates/#g' "$S/preuves-v2/Cargo.toml" > "$CTX/preuves-v2/Cargo.toml"
cp -R "$S/preuves-fix/src" "$S/preuves-fix/Cargo.lock" "$CTX/preuves-fix/"
sed 's#<dépôt>/crates/#/src/nikyOS/crates/#g' "$S/preuves-fix/Cargo.toml" > "$CTX/preuves-fix/Cargo.toml"
cp -R "$S/moli-automation-fix/src" "$CTX/moli-automation-fix/"
sed 's#<dépôt>/crates/#/src/nikyOS/crates/#g' "$S/moli-automation-fix/Cargo.toml" > "$CTX/moli-automation-fix/Cargo.toml"
BINS="$*"
ID=$(docker create -v "$HOME/.cargo/registry:/usr/local/cargo/registry:ro" -v moli-preuves-target:/target -e CARGO_TARGET_DIR=/target -w /src rust:1-alpine \
  sh -c "apk add --no-cache musl-dev >/dev/null 2>&1; (cd preuves-v2 && cargo build --release --offline $BINS 2>&1 | grep -E '^(error)|-->|Finished'); mkdir -p /out/fix && for b in /target/release/*; do [ -f \"\$b\" ] && [ -x \"\$b\" ] && cp \"\$b\" /out/; done; (cd preuves-fix && CARGO_TARGET_DIR=/target/fix cargo build --release --offline --bin moteur 2>&1 | grep -E '^(error)|-->|Finished') && cp /target/fix/release/moteur /out/fix/moteur; ls /out /out/fix")
docker cp "$CTX/." "$ID:/src" >/dev/null
docker start -a "$ID"
rm -rf "$S/linux-bins"; docker cp "$ID:/out" "$S/linux-bins" >/dev/null
docker rm "$ID" >/dev/null
ls -la "$S/linux-bins" "$S/linux-bins/fix"
```

Code complet de `docker-run.sh : exécution limitée à 2 CPU et 128 Mio` :

```sh
#!/bin/sh
# Exécute un banc Linux dans un conteneur limité comme la production : 2 CPU, 128 Mio.
# Usage : docker-run.sh <binaire ou fix/moteur> [args…]
set -eu
S=<scratchpad>
BIN=$1; shift
NOM=$(basename "$BIN")
ID=$(docker create --cpuset-cpus=0,1 --memory=128m --memory-swap=128m alpine:3 "/bin/$NOM" "$@")
docker cp "$S/linux-bins/$BIN" "$ID:/bin/$NOM" >/dev/null
docker start -a "$ID" || true
docker rm "$ID" >/dev/null
```
