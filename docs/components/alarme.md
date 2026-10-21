# La centrale d'alarme (`crates/moli-alarm`)

> **Statut** : spécification validée par le propriétaire le 18 octobre 2026 (§ 20).
> Le moteur pur est livré (lot 2/9) ; la centrale complète reste à raccorder.
> Référence de parité : Alarmo 1.10 et son clavier Zigbee
> piloté par MQTT. Tant que la centrale de Moli n'est pas qualifiée sur place, **Alarmo
> reste le moteur** de la maison (§ 15).

Une aide booléenne « alarme active » (`helpers`) n'est pas une centrale : ni modes, ni
délais, ni codes, ni couverture, ni sirène. Cette page décrit la centrale qui la remplace :
une machine d'états **déterministe** (aucun LLM), sous la garde, au journal, qui survit
aux redémarrages sans ouvrir de trou.

**État de livraison au 21 octobre 2026** : le prérequis de confidentialité des codes
clavier (lot 1/9, § 8.1) et le [moteur pur](../../crates/moli-alarm/README.md) (lot 2/9)
sont dans `main`. Le moteur calcule les transitions, la couverture, les exclusions et
les données de reprise, puis renvoie des effets typés. Il ne lit ni n'écrit de fichier,
ne commande aucun appareil et ne reçoit aucun code secret. Les codes et profils, le
pilote, la persistance réelle, les sorties, les surfaces et la qualification restent
aux lots suivants. Les points, routes et exemples de configuration ci-dessous décrivent
la **cible validée**, pas des surfaces déjà raccordées. Aucun de ces deux lots ne vaut
bascule depuis Alarmo ni qualification matérielle.

## 1. Principes

1. **Aucun LLM** dans le chemin d'une décision. Moli explique et aide à configurer ; la
   centrale décide seule, par une fonction pure testée (`step`, § 4).
2. **Quatre choses distinctes, jamais confondues à l'écran ni dans un retour** : la
   demande acceptée, l'armement en cours, l'état armé confirmé, et la couverture réelle
   (capteurs exclus ou injoignables). Une sirène commandée n'est pas une sirène qui sonne.
3. **Un code d'alarme ne devient jamais une valeur** : ni point, ni état, ni bus, ni SSE,
   ni historique, ni journal, ni `state.json`, ni log, ni MCP (§ 8.1).
4. **Un seul propriétaire par sortie** (sirène, réponse au clavier, notification) : Moli
   ou l'ancienne centrale, jamais les deux (§ 15).
5. **Une seule file** : tout ce qui change la centrale (demandes, capteurs, minuteries)
   passe par une tâche unique, dans l'ordre d'arrivée. Pas deux transitions à la fois.
6. **Échéances absolues** : chaque délai est une date, persistée ; un redémarrage ne la
   remet jamais à zéro (§ 12).
7. **Rien ne sonne sans fin** : toute sirène a une durée bornée et s'arrête à toute
   sortie de l'état déclenché (§ 9).

## 2. Vocabulaire

| Terme | Sens |
|---|---|
| zone | ce que la centrale protège ; **une** zone par installation (`alarme:<zone>`) |
| mode | `away` (absence), `home` (à la maison), `night` (nuit), `vacation` (vacances) ; chacun activé ou non |
| capteur couvert | un point d'appareil (contact, mouvement, serrure, sabotage) que la centrale surveille, avec sa politique par mode (§ 5) |
| exclusion | capteur ignoré pour **cette** période d'armement (automatique ou forcée), listé et visible ; jamais permanent |
| profil | qui peut armer et désarmer : nom, personne de Moli liée (facultatif), droits, validité, un code (§ 6) |
| source | par où arrive une demande : tableau de bord, clavier, automatisme, agent (§ 7) |
| sortie | sirène, réponse au clavier, notification |
| épisode | un passage en `triggered`, jusqu'à ce qu'on en sorte |

## 3. États et points publiés

| État | Sens | Minuterie |
|---|---|---|
| `disarmed` | rien n'est surveillé (sauf capteurs « 24 h », § 5) | — |
| `arming` | délai de sortie en cours vers un mode | `exit_deadline` |
| `armed_away` · `armed_home` · `armed_night` · `armed_vacation` | armée, état **confirmé** après validation des capteurs | — |
| `pending` | délai d'entrée : une ouverture a été vue, on attend un code | `entry_deadline` |
| `triggered` | alarme déclenchée (épisode) | `trigger_deadline` (si le mode en a une), `siren_deadline` |

La centrale publie un appareil `alarme:<zone>` (pilote interne, comme `lumieres`) :

| Point | Accès | Valeur |
|---|---|---|
| `state` | lecture | l'un des états ci-dessus |
| `target` | lecture | mode visé (pendant `arming`) ou mode armé ; `null` si désarmée |
| `deadline` | lecture | fin de la minuterie en cours (ms, `null` sinon) : compte à rebours de l'interface |
| `blocking` | lecture | capteurs qui empêcheraient d'armer, par mode (« prêt à armer ») |
| `bypassed` | lecture | capteurs exclus pour l'armement en cours |
| `degraded` | lecture | capteurs couverts injoignables ou jamais confirmés depuis le démarrage |
| `cause` | lecture | ce qui a déclenché l'épisode (capteur, sabotage, panique, délai écoulé pendant un arrêt) |
| `siren` | lecture | `off`, `commanded`, `confirmed`, `unconfirmed` (§ 9) |
| `arm` | **écriture** | `away` · `home` · `night` · `vacation` : armer **sans code** depuis `disarmed` (§ 7) |

Désarmer, changer de mode, exclure un capteur : **jamais par un point** (le journal du hub
garde la valeur d'une commande, un code n'y a pas sa place). Ces actions passent par
l'entrée de la centrale, avec un code (§ 7, § 17).

## 4. La machine d'états

### 4.1 Forme

`step(état, événement, maintenant, config, profils) → (nouvel état, effets)` : pure, sans
entrée/sortie, sans horloge propre. Les effets (commander une sirène, répondre au clavier,
notifier, journaliser, écrire l'état) sont exécutés ensuite par la tâche de la centrale.
Chaque transition incrémente une **génération** ; une minuterie porte la génération qui l'a
armée, et une minuterie d'une génération passée est ignorée.

### 4.2 Événements

| Événement | Origine |
|---|---|
| `Arm{mode, code?, skip_exit, force, source}` | tableau de bord, clavier, automatisme, point `arm` |
| `Disarm{code, source}` | tableau de bord, clavier |
| `SkipDelay{source}` | tableau de bord avec code (fin de sortie immédiate) |
| `Open(s)` · `Close(s)` | capteur couvert, selon son interprétation (§ 5.2) |
| `Unavailable(s)` · `Available(s)` | capteur couvert (disponibilité, pilote en panne) |
| `Tamper(s)` | point de sabotage d'un capteur, d'une sirène ou d'un clavier |
| `Panic{source}` | bouton panique d'un clavier |
| `ExitExpired` · `EntryExpired` · `TriggerExpired` · `SirenExpired` | minuteries |
| `ArmOnClose(s)` | capteur `arm_on_close` refermé depuis 5 s pendant `arming` |
| `Restore(sauvegarde)` | démarrage (§ 12) |

Les champs `code` de ces demandes décrivent l'entrée privée de la centrale. Dans le
moteur pur livré, ils sont remplacés par un identifiant interne de demande (`RequestId`) :
l'adaptateur `Profiles` réévalue les droits et la validité au temps fourni à `step`.
Ni code ni empreinte de code ne traversent cette interface ; l'identifiant seul ne
prouve aucune autorisation.

### 4.3 Transitions

« Bloquants(m) » = capteurs couverts en mode `m`, ouverts, sans `allow_open`, non exclus
automatiquement en `m` et sans exclusion forcée. Un code « valide » = profil actif, dans sa
validité, avec le droit demandé et le mode autorisé (§ 6). Toute réponse refusée laisse
l'état inchangé et est journalisée.

| Depuis | Événement | Condition | Vers | Effets |
|---|---|---|---|---|
| `disarmed` | `Arm(m)` | `m` non activé | = | refus `not_allowed` |
| `disarmed` | `Arm(m)` | sortie(m) = 0 ou `skip_exit`, Bloquants(m) ≠ ∅ | = | refus `open_sensors` + liste |
| `disarmed` | `Arm(m)` | sortie(m) = 0 ou `skip_exit`, Bloquants(m) = ∅ | `armed_m` | exclusions automatiques listées ; notification « armée » |
| `disarmed` | `Arm(m)` | sortie(m) > 0 | `arming(m, maintenant + sortie, retour = disarmed)` | réponse **acceptée** (pas « armée ») |
| `disarmed` | `Disarm` | code valide | = | réponse `already_disarmed` |
| `disarmed` | `Open(s)` | `s` est « 24 h » (`always_on`) | `triggered` | sirènes, notifications |
| `disarmed` | `Tamper(s)` · `Panic` | — | = | notification (pas de sirène ; réglable, § 10) |
| `arming(m)` | `Arm(m)` même mode | — | = | réponse « déjà en cours », délai inchangé |
| `arming(m)` | `Arm(m')` autre mode | — | = | refus `busy` (désarmer d'abord) |
| `arming(m)` | `Disarm` | code valide | `disarmed` | annulation |
| `arming(m)` | `SkipDelay` · `ArmOnClose(s)` | — | comme `ExitExpired` | |
| `arming(m)` | `ExitExpired` | Bloquants(m) = ∅ | `armed_m` | notification « armée » (+ couverture dégradée s'il y en a) |
| `arming(m)` | `ExitExpired` | Bloquants(m) ≠ ∅ | `retour` | échec `open_sensors` + liste, notification |
| `arming(m)` | `Open(s)` | `s` sans délai de sortie, couvert en `m` | comme depuis `armed_m` | |
| `arming(m)` · `armed_m` · `pending` | `Tamper(s)` | `tamper_armed = "trigger"` (défaut) | `triggered` | cause « sabotage » |
| `arming(m)` · `armed_m` · `pending` | `Tamper(s)` | `tamper_armed = "notify"` | = | notification |
| `armed_m` | `Arm(m)` même mode | — | = | réponse « déjà armée » |
| `armed_m` | `Arm(m')` autre mode | code valide | comme depuis `disarmed`, avec `retour = armed_m` | |
| `armed_m` | `Arm(m')` autre mode | sans code ou code refusé | = | refus `code_required` / `invalid_code` |
| `armed_m` | `Disarm` | code valide | `disarmed` | notification |
| `armed_m` | `Open(s)` | `s` couvert en `m`, entrée(m) > 0 et `s` avec délai d'entrée | `pending(m, maintenant + délai(s, m))` | réponse clavier « délai d'entrée » |
| `armed_m` | `Open(s)` | sinon (capteur immédiat, ou entrée(m) = 0) | `triggered` | sirènes, notifications |
| `armed_m` · `pending` | `Unavailable(s)` | `s.trigger_unavailable`, couvert | comme `Open(s)` | |
| `armed_m` · `pending` | `Unavailable(s)` | sinon | = | `degraded` ; une notification par capteur et par période |
| `armed_m` | `Panic` | `panic = "trigger"` | `triggered` | |
| `pending` | `Disarm` | code valide | `disarmed` | |
| `pending` | `Disarm` | code refusé | = | **le délai ne s'allonge jamais** |
| `pending` | `Arm(*)` | — | = | refus `busy` |
| `pending` | `Open(s')` | `s'` immédiat | `triggered` | |
| `pending` | `Open(s')` | délai(s') plus court | = | échéance = min(échéance, maintenant + délai(s')) : raccourcie, jamais allongée |
| `pending` | `Close(s)` | — | = | refermer n'annule rien |
| `pending` | `EntryExpired` · `SkipDelay` | — | `triggered` | |
| `triggered` | `Disarm` | code valide | `disarmed` | sirènes coupées ; notification « fin d'alarme » |
| `triggered` | `Arm(*)` | — | = | refus `busy` |
| `triggered` | `Open(s')` | `s'` pas encore vu dans l'épisode, sirènes arrêtées | = | **un** nouveau cycle de sirène pour ce capteur |
| `triggered` | `SirenExpired` | — | = | sirènes coupées, l'épisode continue |
| `triggered` | `TriggerExpired` | mode avec durée (> 0) | `armed_m` | sirènes coupées ; capteurs encore ouverts **exclus** et listés (§ 18, écart 3) ; avec `after_trigger = "rearm_or_disarm"` et un capteur bloquant : `disarmed` (comme Alarmo) |
| toute | sortie de `triggered`, quelle qu'elle soit | — | — | ordre d'arrêt à chaque sirène (§ 9) |

Toute combinaison absente du tableau est **sans transition** et journalisée en `debug` ;
le test exhaustif (§ 19) parcourt chaque couple état × événement et vérifie qu'il est soit
dans ce tableau, soit sans transition. Les points dérivés (`blocking`, `degraded`) sont
recalculés à chaque changement d'un capteur couvert, hors de la machine d'états.

### 4.4 Règles transverses

- **Double armement** : une même demande répétée (clavier pressé deux fois, deux onglets)
  ne relance jamais un délai ; une demande différente pendant `arming`, `pending` ou
  `triggered` est refusée (`busy`).
- **File** : bornée à 64 ; pleine, une **demande** est refusée tout de suite (`busy`). Un
  événement de capteur n'est jamais perdu : si le bus du hub décroche (abonné en retard,
  instantané reçu à la place des changements), la centrale recalcule depuis l'état courant
  de chaque capteur couvert et émet les `Open` manquants.
- **Réponse** : une demande reçoit son verdict quand `step` a tourné (acceptée, refusée
  avec motif), jamais quand la sirène ou le clavier ont obéi.
- **Délais** par mode (`exit_s`, `entry_s`, `trigger_s`) ; `trigger_s = 0` = l'épisode dure
  jusqu'au désarmement (comme Alarmo), **mais pas la sirène** (§ 9).

## 5. Couverture des capteurs

### 5.1 Politique par capteur

| Réglage | Défaut | Sens (comme Alarmo) |
|---|---|---|
| `point` | — | le point lu (`appareil/clé`), identité stable, jamais un nom |
| `modes` | — | modes où il est couvert |
| `exit` | `true` | toléré ouvert pendant le délai de sortie ; `false` = actif dès le début de l'armement |
| `entry` | `true` | passe par le délai d'entrée ; `false` = déclenchement immédiat |
| `entry_s` | délai du mode | ne peut que **raccourcir** le délai du mode |
| `allow_open` | `false` | peut être ouvert au moment d'armer (mouvement) ; reste surveillé ensuite |
| `auto_bypass` | `[]` | modes où, ouvert à l'armement, il est exclu pour la période (listé, notifié) |
| `arm_on_close` | `false` | sa fermeture, 5 s stable pendant `arming`, termine la sortie |
| `trigger_unavailable` | `false` | injoignable = ouverture (sinon : couverture dégradée, visible) |
| `always_on` | `false` | « 24 h » : déclenche même désarmée |

### 5.2 Interprétation des valeurs

Par sémantique du point, jamais par son nom : `Contact` faux = ouvert ; `Occupancy` vrai =
mouvement ; `Lock` lisible faux = déverrouillé = ouvert (sémantique livrée par le ticket
Accès) ; `Tamper` vrai = sabotage (événement `Tamper`, pas `Open`). Toute autre valeur
(`null`, texte inattendu) = **inconnu**, jamais ouvert ni fermé. Appareil hors ligne =
injoignable.

### 5.3 Valeurs fraîches et premier état connu

- Au démarrage, les valeurs rejouées depuis `state.json` sont **en cache**, pas observées :
  un capteur couvert reste « non confirmé » (`degraded`) jusqu'à sa première valeur
  postérieure au démarrage.
- Quand cette première valeur fraîche arrive, ou la première valeur connue après
  `inconnu`, et qu'elle est **ouverte** alors que la centrale surveille ce capteur, c'est
  un `Open` (délai d'entrée s'il en a un). Alarmo l'ignore ; ici c'est une ouverture.
- Une ouverture puis une fermeture **pendant** un arrêt de Moli ne se reconstitue pas
  (limite assumée, § 10).

### 5.4 Prêt à armer et exclusions

- `blocking` est recalculé à chaque changement d'un capteur couvert : l'interface montre,
  par mode, ce qui empêcherait d'armer, avant qu'on essaie.
- **Exclusion forcée** (`force`) : seulement par une personne avec un code valide, depuis
  le tableau de bord, en nommant les capteurs exclus (pas « tout ce qui est ouvert » en
  aveugle). Les exclusions sont listées dans `bypassed`, dans la notification d'armement
  et au journal ; elles disparaissent au désarmement.
- Un capteur désactivé dans la configuration n'est pas une exclusion : il n'est simplement
  pas couvert, et la page de la centrale le dit.

## 6. Codes, profils et rôles

- **Profil** : `id` (jamais renommé), nom, personne de Moli liée (facultatif : un profil
  peut exister sans compte, par exemple pour une aide ménagère), `can_arm`, `can_disarm`,
  modes permis (défaut : tous), validité (`from`, `until`, facultatives). Un profil lié à un
  **invité** prend la validité du séjour de l'invité ; hors de cette fenêtre, son code est
  refusé.
- **Code** : 4 à 8 chiffres (un clavier n'a que des chiffres). Stocké comme
  **HMAC-SHA256** sous une clé du coffre (`alarm/code_key`), comme les codes d'invitation :
  le fichier seul ne permet pas d'essayer les codes. Comparaison en temps constant. Deux
  profils ne partagent jamais un code (le code identifie la personne). Un code est saisi par
  sa personne ou par un propriétaire, dans le tableau de bord (session humaine) ou par un
  tube en CLI ; **jamais** vu, choisi ni lu par un agent.
- **Gestion** : créer, modifier, révoquer un profil = un **propriétaire** avec session
  humaine. Révoquer est immédiat. Retirer une personne de Moli révoque ses profils.
- **Politique** (parité Alarmo) : armer depuis `disarmed` sans code ; changer de mode armé
  **avec** code ; désarmer **toujours** avec code, quelle que soit la source (§ 7). Un code
  fourni pour armer est quand même vérifié, droit d'armer compris, et nomme la personne au
  journal (Alarmo l'ignore dans ce cas).
- **Codes faux répétés**, par source (un clavier, un appelant du tableau de bord) : 5 refus
  en 10 min verrouillent cette source 15 min, doublé à chaque récidive jusqu'à 24 h ;
  notification au premier verrouillage. Le verrou est **par source** : un clavier verrouillé
  n'empêche ni le téléphone ni le tableau de bord. Pendant `pending`, un code faux n'allonge
  jamais le délai. Le verrou survit au redémarrage. Code inconnu, hors validité ou sans le
  droit : même réponse à la source (`invalid_code`), motif exact au journal seulement.

## 7. Sources et propriétaire de chaque commande

| Source | Armer depuis `disarmed` | Changer de mode | Désarmer | Exclure | Notes |
|---|---|---|---|---|---|
| tableau de bord, session humaine (`ui`) | oui | code | code (§ 20, Q1) | code | |
| tableau de bord sans code (`household`) | oui | code | code | code | parité : armer sans code |
| clavier | oui (selon sa table) | code | code | non | § 8 |
| automatisme validé (`automation`) | oui (point `arm`) | non | **non** (§ 20, Q3) | non | l'éditeur montre « arme l'alarme » avant validation |
| agents (`api`, `mcp`, `cli`, `assistant`) | **retenu** : un humain approuve | non | **jamais** | non | même la demande de désarmer est refusée, pas retenue |
| notification sur un téléphone | — | — | **non** | — | elle ouvre la page de l'alarme, rien de plus |
| `system` | — | — | — | — | réservé aux effets de la centrale elle-même |

- Les ordres de la centrale vers ses sorties (sirène, clavier, téléphones, Telegram) sont
  des commandes du hub d'origine `system`, acteur `alarme`, donc journalisées comme les
  autres.
- La garde retient l'écriture du point `arm` pour tout agent, **dans toutes les pièces et
  à toute heure** (comme ouvrir une serrure) : armer une maison où dort quelqu'un, sans
  délai, est un geste de personne.

## 8. Clavier Zigbee2MQTT

### 8.1 Le code n'est jamais une valeur

**Prérequis de sécurité livré (lot 1/9), indépendant du reste** : les champs qui portent
un code, dont `action_code`, sont exclus des points publics, de l'état, du bus, du SSE,
de l'historique et du journal. Le pilote remet l'entrée par un canal privé borné à un seul
consommateur (`Hub::take_keypad_inputs`) ; absent ou saturé, le canal jette l'entrée.
L'audit public ne garde que l'identité du clavier. Les copies actives anciennes sont
réparées au démarrage ; un échec de cette réparation empêche le démarrage. Les sauvegardes
externes ne sont pas modifiées. Le raccordement du consommateur de la centrale reste à
faire ; le même canal pourra servir à d'autres claviers (ESPHome, Z-Wave plus tard).

### 8.2 Entrée

- Actions reconnues, selon la table du clavier (réglable) : `disarm`, `arm_all_zones` →
  `away`, `arm_day_zones` → `home`, `arm_night_zones` → `night` (absent du blueprint actuel,
  permis ici), `panic`/`emergency` → `Panic`. Le reste est journalisé et ignoré.
- **Anti-rejeu** : un message MQTT **retenu** n'est jamais une action (un redémarrage de
  Moli ou du broker ne rejoue pas un ancien « désarmer ») ; une transaction déjà vue de ce
  clavier (60 s, 16 dernières) est ignorée ; un message sans action est ignoré.

### 8.3 Retours

- Réponse à chaque demande, avec sa transaction : mode armé, `invalid_code`, `not_ready`
  (ouvertures), `already_disarmed`, `busy`.
- À chaque changement d'état, tous les claviers reçoivent l'état : délai de sortie, délai
  d'entrée, déclenchée, armée dans tel mode, désarmée. Vocabulaire exact du modèle à
  confirmer sur captures réelles (§ 19, qualification).

### 8.4 Hors ligne

- Un clavier hors ligne n'empêche ni d'armer ni de désarmer par les autres sources ; il est
  signalé (page de l'alarme, une notice, pas de rafale). Aucun retour ne lui est mis en
  file : à son retour, il reçoit l'état courant.
- Une autre centrale qui répond au même clavier (publication `…/set` vers ce clavier qui ne
  vient pas de Moli) est signalée comme conflit (§ 15).

## 9. Sirènes

- Sortie = point **binaire inscriptible** (`on`/`off`), commandé par valeur absolue,
  **jamais** par bascule. Les sirènes IAS WD à commande composite (`warning`) attendent que
  `moli-z2m` sache écrire un composite : hors périmètre.
- **Marche** à l'entrée en `triggered`, et une fois par nouveau capteur pendant l'épisode si
  elles se sont arrêtées.
- **Arrêt** à la première de ces échéances : `siren_max_s` écoulé depuis la mise en marche
  (§ 20, Q2), sortie de `triggered` pour **quelque raison que ce soit** (désarmement,
  réarmement après la durée du mode, redémarrage qui conclut l'épisode).
- **Vérification** : si le point de la sirène est lisible, son état doit suivre l'ordre en
  10 s ; sinon un seul nouvel ordre (même valeur absolue), puis `siren = unconfirmed` et
  notification. Un arrêt non confirmé est renvoyé à chaque minute pendant 10 min.
- **Garde-fou** : une sirène vue allumée alors qu'aucun cycle n'est en cours (commande
  étrangère, ancien automatisme, sirène bloquée) reçoit un arrêt, et la maison est prévenue
  (« une autre commande a allumé la sirène »). En mode ombre (§ 15) : signalement seulement.
- Essai de sirène : session humaine seulement, durée courte, journalisé.

## 10. Sabotage et pannes

| Situation | Désarmée | Armée, sortie, entrée |
|---|---|---|
| sabotage d'un capteur couvert, d'une sirène ou d'un clavier | notification | `triggered`, cause « sabotage » (réglable) |
| capteur couvert injoignable | visible (`degraded`) | notification une fois par période ; déclenche seulement si `trigger_unavailable` |
| pilote d'un capteur en panne (backoff) | ses capteurs injoignables | idem, une notification groupée |
| clavier hors ligne | signalé | signalé ; les autres sources fonctionnent |
| sirène hors ligne | signalé | notification ; au déclenchement, les autres sirènes et les notifications partent quand même |
| Moli arrêté | — | rien n'est surveillé pendant l'arrêt ; la reprise applique les échéances échues (§ 12). Détecter l'arrêt lui-même relève d'une surveillance extérieure (astreinte), hors de cette centrale |
| bouton panique | notification | `triggered` si `panic = "trigger"`, sinon notification |

## 11. Notifications

La centrale prévient d'elle-même ; aucune sécurité ne dépend d'un automatisme que quelqu'un
aurait pensé à écrire. Canaux existants : `maison` (notice), téléphones de l'appli, Telegram,
voix. Destinataires par défaut : propriétaires et membres, jamais un invité.

| Événement | Défaut |
|---|---|
| armée (état confirmé), avec exclusions et capteurs injoignables | téléphones |
| échec d'armement, avec les ouvertures | téléphones, maison |
| déclenchée (cause, mode) | téléphones (alerte critique si l'appli le permet), Telegram, maison |
| fin d'alarme (désarmée après un épisode, ou réarmée après la durée du mode) | téléphones |
| sabotage ; codes faux répétés (verrou) | téléphones |
| capteur couvert injoignable pendant l'armement | téléphones |
| sirène non confirmée ; sirène allumée hors alarme ; conflit avec une autre centrale | téléphones, maison |
| état de la centrale perdu au démarrage | téléphones (critique) |

Contenu : zone, mode, noms donnés dans Moli ; jamais un code ; **jamais une action qui
désarme** (le bouton « vérifier » d'Alarmo désarmait sans code). Une notification ouvre la
page de l'alarme. Le choix des canaux par événement se règle ; on ne peut pas y supprimer
« déclenchée » de tous les canaux.

## 12. Persistance et reprise après redémarrage

- `data/alarm.json`, écrit à chaque transition : état, mode, `retour`, génération,
  échéances **en date absolue** (ms), cause, exclusions de la période, cycle de sirène en
  cours (début, capteurs déjà vus), verrous de codes. Écriture à part, `fsync`, renommage,
  `.bak` gardé (comme `automations.json`). Un échec d'écriture est compté, visible dans
  `/api/health` et signalé à la maison. Les événements de capteur sans transition n'écrivent
  rien.
- Profils dans `data/alarm-profiles.json` (0600) ; codes (HMAC) et clé dans le coffre.
- **Reprise** (`Restore`) : la durée restante d'une minuterie = `échéance − maintenant`,
  bornée entre 0 et la durée configurée (une horloge qui saute ne rallonge ni n'invente
  rien).

| Sauvegardé | Reprise |
|---|---|
| fichier et `.bak` illisibles ou absents (installation déjà configurée) | `disarmed`, notification critique « état perdu » |
| `disarmed` | `disarmed` |
| `arming`, échéance future | `arming`, temps restant seulement |
| `arming`, échéance passée | `ExitExpired` maintenant (valeurs en cache comptées comme bloquantes) |
| `armed_m` | `armed_m` ; capteurs « non confirmés » jusqu'à leur première valeur fraîche (§ 5.3) |
| `pending`, échéance future | `pending`, temps restant seulement |
| `pending`, échéance **passée** | `triggered`, cause « délai d'entrée écoulé pendant un arrêt » : couper le courant pendant le délai d'entrée ne désarme pas |
| `triggered`, sirène pas encore à échéance | sirènes rallumées pour le temps restant |
| `triggered`, sirène échue | ordre d'arrêt envoyé une fois (une sirène a pu rester bloquée) |
| `triggered`, durée du mode échue | `TriggerExpired` maintenant |

- Les demandes en cours au moment de l'arrêt sont perdues (l'appelant n'a pas eu de
  réponse) ; aucune n'est rejouée au démarrage.

## 13. Journal

Nouvelle action `Action::Alarm { zone, from, to, event, cause, source, profile? }` :
chaque transition, chaque refus (motif), chaque exclusion, chaque verrou de code, chaque
ordre de sortie (sirène, clavier, notification : ceux-là sont déjà des commandes `system`).
Un profil est nommé, un code jamais. Les refus de code ne disent pas quel chiffre était
faux.

## 14. Articulation avec le reste de la maison

- **Mode de la maison** (aide `mode_maison`) : aucune liaison implicite. La centrale ne lit
  pas le mode ; le mode ne la commande pas. Un automatisme validé peut armer à un
  changement de mode ; rien ne désarme par ce chemin. L'interface montre séparément le mode
  choisi et l'état réel de l'alarme.
- **Invités et réservations** : une réservation (calendrier) n'est pas une présence ; elle
  n'arme ni ne désarme. Un invité peut recevoir un profil d'alarme, borné à son séjour, avec
  ses propres droits (souvent : désarmer seulement).
- **Serrure** (ticket Accès) : désarmer n'ouvre rien, ouvrir ne désarme rien. Une serrure
  peut être un capteur couvert (déverrouillée = ouverte). La coordination (verrouiller en
  armant, par exemple) passe par un automatisme validé ; déverrouiller reste un geste
  humain.
- **Aide `alarme_active`** : remplacée par l'état de la centrale ; à retirer de la
  configuration à la bascule.

## 15. Coexistence avec Alarmo et bascule

| Phase | Moteur | Sirènes, clavier, notifications | Moli |
|---|---|---|---|
| 0 (aujourd'hui) | Alarmo | Alarmo, règles internes et blueprint | rien |
| 1, ombre (`outputs = "shadow"`) | Alarmo | Alarmo | la centrale tourne sur les mêmes capteurs et les mêmes demandes du clavier (codes ressaisis par chacun dans Moli), décide et journalise, **ne commande rien** ; ses états sont marqués « ombre » |
| 2, bascule (`outputs = "live"`) | Moli | Moli | dans HA, avant : désactiver les règles internes d'Alarmo, l'automatisme du clavier et la notification externe ; désarmer et désactiver le panneau |
| retour arrière | Alarmo | Alarmo | `outputs = "shadow"`, réactiver les objets HA |

- **Un seul propriétaire par sortie** : en ombre, Moli ne commande ni sirène, ni clavier,
  ni téléphone. En direct, il considère toute autre commande de ses sorties comme un conflit
  (sirène allumée hors cycle, réponse au clavier qui ne vient pas de lui) : arrêt de la
  sirène, notification « une autre centrale agit encore ».
- La comparaison ombre/Alarmo est faite par une personne (journal de Moli contre historique
  de HA), sur une durée fixée par le plan de qualification.

## 16. Configuration

Dans `moli.toml`, éditée par des humains seulement (comme `[guard]`) : aucune surface ne la
modifie. Les codes n'y figurent jamais.

```toml
[alarm]
zone = "maison"                # identité de l'appareil alarme:<zone>, jamais renommée
outputs = "shadow"             # "shadow" ou "live" (§ 15)
siren_max_s = 180              # § 9 (§ 20, Q2)
after_trigger = "rearm_excluding"   # ou "rearm_or_disarm" (comme Alarmo)
panic = "notify"               # ou "trigger"
tamper_armed = "trigger"       # ou "notify"

[alarm.modes.away]
exit_s = 60
entry_s = 30
trigger_s = 1800               # 0 = jusqu'au désarmement (pas la sirène)
[alarm.modes.home]
exit_s = 0
entry_s = 0
trigger_s = 1800

[[alarm.sensor]]
point = "z2m:0x00124b0000000001/contact"
modes = ["away", "home", "night", "vacation"]

[[alarm.sensor]]
point = "z2m:0x00124b0000000002/occupancy"
modes = ["away", "vacation"]
allow_open = true

[[alarm.siren]]
point = "z2m:0x00124b0000000003/alarm"

[[alarm.keypad]]
device = "z2m:0x00124b0000000004"
arm_all_zones = "away"
arm_day_zones = "home"
arm_night_zones = "night"
```

`moli-os check-config` refuse : point inconnu ou d'une sémantique non interprétable, mode
couvert non activé, `entry_s` d'un capteur plus long que celui du mode, sirène non binaire
ou en lecture seule, clavier qui n'expose pas de champ de code.

## 17. Surfaces

| Surface | Quoi | Accès |
|---|---|---|
| `GET /api/alarm` | état, mode, échéance, `blocking` par mode, exclusions, `degraded`, sirène, claviers, profils (noms, droits, validité ; jamais un code) | foyer ; un invité : l'état seulement |
| `POST /api/alarm` | `{action: arm\|disarm\|skip_delay, mode?, code?, force?: [capteurs]}` → verdict (§ 4.4) | foyer ; désarmer, changer de mode, exclure : code |
| profils | créer, modifier, révoquer, poser un code | propriétaire, session humaine |
| SSE | événements `state` des points de `alarme:<zone>` | comme les autres appareils |
| MCP | lire l'état ; demander l'armement (retenu pour un humain) | jamais désarmer, exclure ni voir un profil |
| automatismes | déclencheur sur `alarme:<zone>/state` ; action `set` sur `arm` | désarmer impossible |
| interface | carte résumé → détail → action explicite ; pavé de code ; « prêt à armer » ; exclusions et injoignables visibles ; compte à rebours ; sirène commandée ≠ confirmée | fr et en |

## 18. Écarts assumés avec Alarmo

| # | Alarmo | Moli |
|---|---|---|
| 1 | sirène tant que l'alarme est déclenchée (30 min ou sans fin selon le mode) | durée bornée `siren_max_s` (Q2) |
| 2 | règles d'arrêt des sirènes au seul désarmement | arrêt à toute sortie de `triggered` |
| 3 | fin de durée : réarme, ou désarme si un capteur bloque | réarme en excluant les capteurs encore ouverts, notifié (réglable) |
| 4 | délais recréés entiers au redémarrage | échéances absolues ; entrée échue pendant l'arrêt = déclenchée |
| 5 | premier état connu après `unknown` ignoré | ouverture traitée (§ 5.3) |
| 6 | bouton de notification qui désarme sans code | aucune notification ne désarme |
| 7 | aucune limite de codes faux | verrou par source |
| 8 | sabotage selon type de capteur | sabotage pendant l'armement = déclenchement (réglable) |
| 9 | services accessibles à toute intégration | agents : armer retenu, désarmer jamais |
| 10 | alias MQTT `force` fragile | exclusion explicite, nominative, avec code |
| 11 | clavier : pas de mode nuit | nuit permis |
| 12 | succès d'armement annoncé dès le début de la sortie | « acceptée » puis « armée » confirmée, deux retours distincts |
| 13 | code optionnel au tableau de bord selon réglage | code pour désarmer (Q1) |
| 14 | armer sans code ignore le droit d'armer du profil | un code fourni à l'armement est vérifié, droit compris |

## 19. Tests exigés

**Normaux** : chaque ligne du tableau § 4.3 ; armer chaque mode avec et sans délai ; sortie
terminée par `arm_on_close` ; entrée puis désarmement ; capteur à délai plus court ;
exclusion automatique et forcée ; changement de mode avec code ; réarmement après durée ;
clavier (chaque action, transaction répétée, retours) ; notifications attendues par
événement ; points publiés.

**Dégradés** : couple état × événement exhaustif (aucun effet hors du tableau) ;
redémarrage dans chaque état, échéance future et passée, horloge en avance et en retard ;
fichier d'état corrompu, `.bak` seul, aucun ; écriture d'état en échec ; bus en retard
(ouverture rattrapée) ; file pleine ; code faux répété (verrou, doublement, persistance,
autre source non touchée, délai d'entrée non allongé) ; code hors validité, révoqué, sans
droit (même réponse) ; message de clavier retenu (jamais rejoué) ; clavier hors ligne puis
retour ; sirène sans retour d'état, sirène allumée hors cycle, sirène en panne ; capteur
injoignable avec et sans `trigger_unavailable` ; premier état frais ouvert ; sabotage dans
chaque état ; agent qui arme (retenu) et qui désarme (refusé) ; automatisme qui tente de
désarmer (refusé à la validation) ; **le code n'apparaît nulle part** (état, `state.json`,
historique, journal, SSE, MCP, logs) ; mode ombre : aucune commande de sortie.

**Qualification réelle** (après le jalon humain H1, humain présent, jamais dans une pièce
protégée, état remis à l'identique) : captures du clavier réel (fixtures anonymisées),
retours lus sur le clavier, sirène sur une cible neutre et volume réduit si possible,
arrêt confirmé, coupure de Moli pendant un délai d'entrée, phase d'ombre comparée à Alarmo,
puis bascule et retour arrière répétés une fois. L'autre installation : inventaire de sa
configuration Alarmo avant d'y appliquer ce plan (plusieurs zones : ticket à part).

## 20. Décisions du propriétaire

| # | Question | Décision |
|---|---|---|
| Q1 | Désarmer depuis le tableau de bord | code d'alarme à chaque fois, session PIN ou non (§ 7) |
| Q2 | Durée de la sirène | 3 minutes au plus, `siren_max_s = 180` (§ 9, § 16) |
| Q3 | Un automatisme peut-il désarmer | non : armer seulement, refusé dès la validation (§ 7) |
| Q4 | Le reste de cette spécification | validé, découpage compris |

## 21. Hors périmètre

Plusieurs zones ; codes de contrainte ; télésurveillance ; sirènes à commande composite ;
claviers autres que Zigbee2MQTT (le canal privé du § 8.1 les prépare sans les écrire) ;
surveillance de l'arrêt de Moli lui-même ; détecteurs d'environnement (fumée, fuite) :
ils ont leurs propres alertes, pas cette centrale.

## Découpage

Sous-tickets du ticket de spécification, dans l'ordre (chacun avec ses tests normaux et
dégradés, § 19) : code de clavier jamais exposé (prérequis, indépendant) ; moteur pur ;
codes et profils ; pilote interne, persistance et reprise ; sirènes et notifications ;
clavier ; surfaces, garde et journal ; interface ; qualification réelle et bascule.
