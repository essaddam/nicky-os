# Automatismes (`crates/moli-automation`)

Objectif : des automatisations **plus simples que Home Assistant, plus rapides, une
expérience agréable, l'IA au cœur**, avec un éditeur visuel « à la n8n de la maison ».
Livré le 3 oct., durci après une revue adversariale (1 critique, 5 élevés corrigés).

## Règles non négociables

1. **Un humain valide, la maison exécute.** Une automatisation ne tourne que si un humain
   (session PIN du dashboard) a validé *la version qu'il avait sous les yeux* : valider,
   allumer et lancer envoient l'**empreinte** (SHA-256 complet du graphe + mode) de ce qui est
   affiché ; si quelqu'un l'a changée entre-temps → **409**, il faut relire.
2. **Enregistrer n'approuve jamais et n'allume jamais.** Seule la validation le fait. Créé
   ou modifié par Moli, un agent MCP, l'API sans PIN ou un import : brouillon. Un agent ne
   peut **qu'éteindre**. Un changement de logique d'un automatisme validé le fait cesser de
   tourner (et annule ce que l'ancienne version avait lancé) jusqu'à nouvelle validation ;
   « Revenir à la version validée » restaure la dernière version approuvée. Le nom et la
   note sont hors de l'empreinte, mais le nom titre ses messages (Telegram, notices) : une
   fois qu'une personne l'a validé, seule une personne les change (sinon refusé, `422`).
3. Une automatisation validée agit comme `Origin::Automation` : la garde la laisse passer
   (c'est l'intention d'un humain). L'éditeur prévient avant validation si elle touche une
   pièce protégée.
4. **L'IA n'est jamais dans la boucle de contrôle.** Elle *crée*, *explique*, *corrige* et
   *écrit des messages* (nœud « Moli écrit »), mais aucune décision d'allumer/éteindre
   n'est prise par un LLM à l'exécution. Le moteur est déterministe.
5. Lancer pour de vrai : humain seulement, sur la version validée. Les agents ont l'essai à
   blanc (lit les conditions, n'agit jamais, n'attend jamais, n'appelle pas le LLM).
6. Pas de boucle folle : plus de 20 exécutions par minute → l'automatisation se coupe et le
   dit. Files d'attente plafonnées à 10. Supprimer : humain seulement.
7. Bornes : 60 nœuds, 120 liens (doublons retirés), textes 2 000 caractères, 400
   automatismes ; la phrase française a un budget de calcul (pas d'explosion).

## Performance

La liste des automatismes **actifs** est en cache, recalculée seulement quand un
automatisme change : le moteur ne hache rien à chaque seconde ni à chaque événement (c'était
la régression attrapée par la mesure : CPU 0,58 % → 0,23 %).

## Modèle

Une automatisation = un **graphe** de nœuds reliés (comme n8n), stocké dans
`data/automations.json` (écriture atomique). Ports : `out` ; `yes`/`no` (Si) ;
`ok`/`timeout` (Attendre que).

| Famille | Nœud (`type`) | Paramètres |
|---|---|---|
| Déclencheurs | `when_state` | `point`, `to?`, `from?`, `for_s` (maintenu N s) |
| | `when_threshold` | `point`, `above?`, `below?`, `for_s` (franchissement) |
| | `at_time` | `at` « HH:MM », `days` (1 = lundi … 7) |
| | `at_sun` | `event` rise/set, `offset_min`, `days` |
| | `every` | `minutes` |
| | `on_start`, `manual` | — |
| Logique | `if` | `rules[]` (état / plage horaire / jour-nuit), `all` (ET) ou OU |
| Actions | `set` | `point`, `value` |
| | `toggle` | `point` |
| | `wait` | `seconds` |
| | `wait_for` | `rule`, `timeout_s` → `ok` / `timeout` |
| | `notify` | `title?`, `message`, `channels` (`maison`, `telegram`) |
| | `write` | `prompt` → texte `{{texte}}` pour la suite (IA, jamais de contrôle) |

Messages : `{{<appareil>/<clé>}}` = valeur actuelle (avec unité), `{{texte}}` = dernier texte
de Moli, `{{heure}}`.

Modes (comme HA) : `restart` (défaut : minuteries « mouvement → lumière 5 min »), `single`,
`queued`.

## Journal et coupure

Chaque modification d'un automatisme entre au journal du hub (`Action::Automation` :
créé, modifié, validé et allumé, coupé, revenu à la version validée, supprimé), avec qui
l'a faite (une personne, Moli, un agent, l'import) et l'empreinte de la version. Couper
(par `enabled` ou par un enregistrement avec `enabled: false`) **arrête toujours** ses
exécutions en cours, files d'attente comprises. Quand un non-humain coupe un automatisme ou
lui fait perdre sa validation, la maison est prévenue (notice dans le dashboard).

## Moteur

- Abonné au bus du hub : changements d'état (le hub n'émet qu'en cas de changement),
  valeur précédente gardée pour `from` et les franchissements de seuil. `for_s` = minuterie
  annulée si la valeur repart (sans `to` : chaque nouveau changement relance la minuterie).
- Une métadonnée de dépendance actualise l’index exécutable sans relire les valeurs
  du hub dans la mémoire des transitions consommées : elle ne doit pas avaler un
  `State` encore en attente. La reprise globale des valeurs reste réservée au démarrage
  et à la récupération explicite d’une perte du bus (`Lagged`).
- Horloge : un tic par seconde en heure locale (jiff, fuseau de la config) ; chaque
  déclencheur horaire retient la dernière minute où il a tiré (jamais deux fois, y compris
  après un redémarrage dans la même minute). L'heure sautée au passage à l'heure d'été ne
  tire pas (comme HA).
- Soleil : calcul astronomique local (NOAA, ±1 min, vérifié contre un almanach) depuis
  `[automations] latitude/longitude` ; à défaut, le point `daylight` de la météo.
- Exécution : parcours du graphe depuis le déclencheur, **branches dans l'ordre des liens**
  (qui fait partie de ce qu'on valide ; la position à l'écran n'y est pour rien), 100 étapes
  maximum. Chaque run a un jeton enfant de celui de l'automatisme : l'éteindre annule tout,
  files d'attente comprises ; un run en file revérifie qu'il est toujours la version validée.
- Chaque étape est journalisée (`data/automation-runs.jsonl`, 500 gardés, détails tronqués) et
  diffusée en direct (`/api/automations/live`) : l'éditeur colore les nœuds.
- Notifications : `maison` = événement `notice` du hub (toast dans le dashboard) ;
  `voix` = dit à voix haute par chaque enceinte qui sait annoncer (point `announce` du
  Sonos, commande d'automatisme au journal) ;
  `telegram` = une commande (titre, ou nom de l'automatisme, puis le message) sur chaque
  point `notify` inscriptible, c'est-à-dire l'appareil du pilote Telegram
  (`integrations/telegram/`), journalisée au nom de l'automatisme ; sans aucun point
  `notify`, le message part dans la maison.
- Persistance : `data/automations.json` écrit à part, `fsync`, puis renommé ; version
  précédente en `.bak` ; un fichier abîmé est mis de côté et Moli repart sur le `.bak`.

## Surfaces

- REST (détail dans [api.md](api.md)) : `GET/POST /api/automations`,
  `GET/PUT/DELETE /api/automations/{id}`, `POST …/{id}/approve {fingerprint}` (humain),
  `…/{id}/enabled {on, fingerprint}` (allumer = valider ; éteindre : tout le monde),
  `…/{id}/restore` (humain), `…/{id}/run {fingerprint}` (humain), `…/{id}/test` (à blanc),
  `POST /api/automations/check` (validation + phrase en français), `…/draft` (Moli),
  `…/import` (HA, humain), `GET …/runs`, `GET …/live` (SSE).
- MCP : `list_automations`, `get_automation`, `save_automation_draft` (brouillon non
  validé, documentation du format dans la description de l'outil), `test_automation`.
- Moli (conversation) : « préviens-moi quand… » → brouillon + carte « Valider ».

## Éditeur (onglet « Automatismes »)

- **Liste** : barre « Décris-le à Moli », idées prêtes en un clic, cartes (phrase en
  français, interrupteur, état Actif / À valider / Coupé / À corriger, dernière exécution),
  section repliable « Importés de Home Assistant », fil « Ce qui s'est passé ».
- **Éditeur** (`web/src/maison/auto/`) : canevas façon n8n (glisser, relier depuis un port ;
  lâcher dans le vide = ajouter une étape là ; zoom à la molette ou au pincement ; rangement
  automatique ; Suppr), panneau de réglage (appareil choisi par nom et pièce avec sa valeur
  actuelle, valeur adaptée au type, jours, durées, conditions), phrase écrite par le serveur
  en direct, « noms donnés dans Moli » (nom d'origine et identifiant de chaque appareil
  renommé, à vérifier avant de valider), problèmes signalés sur les nœuds, « Modifier avec Moli », essai à blanc,
  exécutions avec surlignage, nœuds qui s'allument en direct, validation par code PIN.

## Moli, le créateur

Voir [moli.md](moli.md) : une phrase → un graphe (JSON à schéma strict, vérifié, un tour de
correction), modèle `builder_model` (gpt-4.1), 5 à 9 s ; inventaire complet ; outil
`automation` dans la conversation.

## Import de Home Assistant

`tools/home-assistant/ha-automations-export.py` → `POST /api/automations/import`. Essai sur
un HA réel : 86 des 90 automatisations traduites en brouillons (61 sans erreur), dans leur
section repliable, **à relire** (quelques minuteries avaient une attente en double : consigne
corrigée depuis). Les présences par zone deviennent « à lancer à la main » faute d'appli
mobile ; les annonces Sonos deviennent « prévenir la maison ».

Garde-fous de l'import (détail dans [moli.md](moli.md)) :

- lancer un import exige une **session humaine** (PIN) du client qui appelle, sinon `403` ;
- chaque traduction passe par le **même sas que la conversation** (`admit()`, en laissant
  10 tours de la fenêtre à la famille) ; doublons écartés par nom enregistré et par SHA-256
  du YAML ; le plafond de 400 (existants + à traduire) est vérifié avant toute dépense ;
- **lien avec HA** : l'export garde l'identifiant HA (`ha_id`) ; la note du brouillon dit de
  quelle automatisation HA il vient et qu'il faut **désactiver l'original dans HA en le
  validant**, sinon les deux agissent. L'éditeur affiche ce début de note en avertissement
  (style `.warn`, comme la pièce protégée) tant que l'import n'a jamais été validé, et le
  rappelle après validation ; la section « Importés de Home Assistant » de la liste le dit
  aussi, et l'interrupteur d'une carte importée le rappelle en validant.

## Plus tard

Annonces vocales Sonos (TTS + `audioClip`), présence
(appli mobile), suggestions d'automatismes tirées de l'historique, Telegram testé en réel.

## Extension des primitives et reprise HA : cadrage

Le [cadrage des primitives](automatismes-primitives.md) distingue les capacités présentes
des travaux à réaliser : aides temporelles, compteur, bouton, planning, scènes, points
calculés et traduction des usages HA. Il décrit les contrats de reprise et les critères
de test de chaque lot. **Ce document n'annonce pas leur disponibilité** : les aides
à valeur historiques restent `select`, `switch`, `number` et `text`. Le bouton
événementiel AUTO-H4, le compteur AUTO-H2 et le minuteur AUTO-H1 sont décrits ci-dessous ; les autres lots
restent des propositions.
Les contrats proposés restent à relire avant implémentation ; leur publication ne
modifie aucune des règles de validation et d'exécution décrites ci-dessus.

## Bouton et occurrences (AUTO-H4)

Une aide `button` expose `instance:id/press`. Son ordre est JSON `null`, sans
changement artificiel d'un booléen ou d'un texte horodaté. L'événement
`occurrence` porte une identité `{source, sequence}` indépendante de son heure
et de sa valeur. Un pilote appelle `DriverCtx::pulse_state` pour une occurrence
réelle ; ses lectures initiales, son cache et ses reconnexions restent des
`set_state`. Hue distingue déjà la lecture initiale des événements de sa liaison.

Un nœud `{"id":"appui","type":"when_occurrence","point":"aides:depart/press"}`
déclenche chaque appui. Pour un autre point événementiel, `value` est un filtre
facultatif, selon la même comparaison que le moteur. Le mode de l'automatisme
reste applicable : `queued` conserve deux appuis successifs, `single` ignore
ceux reçus pendant un lancement, `restart` remplace le lancement précédent.
La limite de file et le coupe-circuit des boucles sont conservés. Un brouillon
ne tourne jamais ; une validation humaine porte toujours sur l'empreinte exacte.
Un événement déjà publié avant validation ou modification ne lance pas la
nouvelle version lorsqu'il est lu avec retard.
Une admission capture aussi l'activation courante avant toute attente de
persistance. Couper puis réactiver le même contenu ne réveille pas cet ancien
appui : sa lignée d'annulation reste fermée, même si l'empreinte est identique.
La définition reste verrouillée jusqu'à la publication de son index et de son
activation. L'arrêt nettoie l'ancienne activation dans cette même transition ;
une requête d'arrêt qui termine plus tard ne révoque pas une validation plus récente.

La livraison est **en direct, au plus une fois**, pas une file de commandes à
rejouer après panne. Le moteur enregistre la consommation avant de programmer
les actions. Une interruption laisse une trace non confirmée ; elle est signalée
au redémarrage, sans relancer les effets. Le bus perdu est signalé sans inventer
les appuis manquants à partir d'un instantané. Les événements d'un ancien
processus sont refusés. Les actions, lancements, demandes retenues et lignes de
journal conservent la corrélation de la requête source.

Deux fichiers métier indépendants de `moli.db` portent la version 1 :
`button-occurrences.json` (identités et reçus des requêtes) et
`automation-occurrences.json` (fenêtres de consommation et lancements non
confirmés). Un seul écrivain par fichier ; écriture bornée à 1 Mio, temporaire,
`fsync`, renommage atomique, puis synchronisation du dossier avant acquittement.
Une erreur ou une version inconnue bloque le chemin concerné, sans effacer les
preuves ni remettre les séquences à zéro. Après un renommage dont la durabilité
est incertaine, le chemin est fermé jusqu'à reprise. Aucune transaction physique
exactement une fois n'est annoncée.
La sauvegarde officielle inclut ces deux fichiers et la restauration vérifie
leur présence/taille selon son manifeste avant toute écriture. Un réessai d'une
requête déjà acquittée avant la sauvegarde garde son reçu après restauration ;
une sauvegarde antérieure à cette requête ne peut en contenir la preuve.

Les requêtes explicites ont une identité client de 32 caractères hexadécimaux
et une séquence entière positive, sous forme de texte `client:sequence`.
Une nouvelle intention utilise une nouvelle séquence ; un réessai garde la
même. Chaque client conserve une fenêtre de 32 reçus, au plus 64 clients ; une
requête plus ancienne est refusée, jamais acceptée de nouveau après éviction.
Les clients ne sont pas évincés silencieusement. Une identité réutilisée pour
un autre point est un conflit. Sans identité de requête, chaque commande est
un nouvel appui. Le navigateur réserve ses séquences dans une transaction
IndexedDB persistante, commune aux onglets ; l'indisponibilité de ce stockage
empêche l'envoi. Le moteur borne à 128 sources et 256 occurrences en cours sa
fenêtre de 64 identités par source. Une borne atteinte est un refus visible,
jamais un débordement de compteur ou un rejeu.

## Compteurs (AUTO-H2)

Les [compteurs durables](compteurs.md) exposent `value` comme état entier ; les
changements peuvent alimenter `when_state`, `when_threshold` et les comparaisons.
Un nœud `set` sur `increment` ou `reset` envoie `null` via le hub et sa garde ; il
ne lit jamais la valeur pour calculer une affectation côté client. Ces opérations
ne sont ni des états ni des occurrences. Les graphes créés par un agent ou un import
restent soumis aux mêmes validations humaines. Les quatre aides historiques et
le bouton AUTO-H4 conservent leur comportement.

## Minuteurs (AUTO-H1)

Les [minuteurs durables](minuteurs.md) exposent état, restant, génération et échéance.
`set` sur `start` porte une durée entière de 1 à 604800 secondes ; `cancel` porte null.
`when_occurrence` sur `finished` traite l'expiration de la génération courante. Valeurs,
reçus et expiration en attente sont persistés avant confirmation ; le moteur consomme
l'identité avant lancement. Un retry ou un redémarrage n'alloue pas de nouvelle expiration.
Une interruption après admission est signalée sans rejeu aveugle de commandes physiques.

Les destinataires et leurs lignées validées sont figés au démarrage. Une activation
ultérieure ne reçoit pas cette génération ; redémarrer le minuteur après validation
pour l'inclure. Sans destinataire, le reçu sans effet est durable avant acquittement.
Le contrat live-only AUTO-H4 reste inchangé.

Les ordres des actions portent un permis vivant lié à l'activation et, pour une
expiration, à sa génération. Une annulation rend abandonné un ordre encore retenu
par le pilote avant émission ; les étapes Set/Toggle et notifications cessent leur
attente. Les luminaires composites conservent ce permis jusqu'aux membres physiques
et à leur continuation bornée. Un envoi de transport déjà engagé n'est pas défait.

## Dates et heures (AUTO-H3)

Les [aides temporelles](dates-heures.md) portent une date civile, une heure civile
ou un instant UTC avec fuseau explicite. `set` affecte une chaîne validée et durable
avant confirmation ; les comparaisons et changements explicites résolvent les mêmes
instants. La saisie n'ajoute aucun réveil à l'heure indiquée. Heures sautées refusées,
heures répétées refusées sans désambiguïsation. Une définition d'agent/import ne
contourne ni la garde ni la validation humaine des graphes. Le cache périodique ne
restaure pas ces valeurs : leur fichier métier est l'autorité à la reprise.


## Plannings hebdomadaires et dépendances validées (AUTO-H5)

L’aide `schedule` publie un booléen `active` en lecture seule depuis une définition
hebdomadaire versionnée : fuseau IANA, jours 1..7, créneaux semi-ouverts et 256 créneaux
au plus après découpage de minuit. Le brouillon est inerte ; seule une publication
humaine explicite, avec la session PIN et l’aperçu exact des graphes concernés, remplace
le courant. Un agent ne peut pas modifier silencieusement un graphe validé par une
édition de planning. Le graphe conserve son empreinte d’identités stables et résout
la version validée courante, sans y figer son numéro.

Le contrat réutilisable, le calendrier DST, les bornes, le journal, la reprise sans
rejeu et les surfaces sont décrits dans [plannings.md](plannings.md). Le socle pur
brouillon/courant est isolé dans un commit ; cette première implémentation du contrat
commun est soumise avec AUTO-H5 à la revue Rust. Elle ne change aucune valeur des aides
existantes en dépendance versionnée et n’introduit aucun nouveau chemin de confiance.


## Points calculés (AUTO-K1)

L’aide `computed` publie un `value` numérique en lecture seule : minimum, maximum ou
moyenne des seules sources utilisables, avec unité et fraîcheur configurées. Absences,
nuls, non-finis, indisponibilités et péremptions sont exclus ; zéro n’est jamais inventé
pour un ensemble vide. La propagation indexée, les échéances, les horodatages, les
bornes et les surfaces sont décrits dans [calculs.md](calculs.md).

Ce lot étend le propriétaire de versions d’AUTO-H5 et sa validation PIN, avec contrôle
des sources exécutables et consommateurs transitifs. Un agent édite un brouillon
inerte ; la version courante validée seule produit des valeurs. Le correctif H5
préservant la file de State consommés reste repris sans rechargement anticipé.
