# Identités et droits sur les ordres : ce que le code fait aujourd'hui

Inventaire fait le 24 octobre 2026 par un agent Claude (Opus), en lecture seule, pour la
[v3 de l'analyse des ordres des appareils](ordres-appareils-v3.md) (§ 5.4). J'en ai revérifié sept
faits dans le code (les origines, la personne arrêtée à HTTP, la session PIN liée à l'appelant,
`resolve_approval`, la voix sans locuteur, `household_may_run`, l'alarme non raccordée) : tous
exacts. Texte de l'agent, non retouché.

Dépôt `nikyOS`, branche `main`, commit `a835893`. Lecture seule : `grep`, lecture des sources et
des tests, sans build ni exécution. Les chemins sont relatifs à la racine du dépôt. Aucune valeur
de secret n'est citée, seulement des noms de champs.

Conventions :
- **lu** : constaté dans le code à la ligne citée ;
- **testé** : un test existant l'affirme (nom du test donné) ;
- **déduit** : conséquence de plusieurs lignes lues, sans test qui la couvre ;
- « foyer » = `Caller::is_household()` (`crates/moli-api/src/caller.rs:133-138`).

---

## 0. L'essentiel en dix lignes

1. Les droits sur un ordre se décident selon l'**origine** (`Origin`, 8 valeurs), jamais selon la **personne**.
   La personne n'existe que dans la couche HTTP (`Caller.person`). Elle ne descend ni dans le hub,
   ni dans la demande d'approbation, ni dans le pilote, ni dans le journal, sauf sous forme de
   texte libre dans `actor` (« tableau de bord (Prénom) »).
2. Il y a **une seule preuve « humaine »** : la session PIN, ouverte avec **le code commun de la
   maison**. Elle n'est liée à aucune personne ; elle est liée à l'appelant (adresse IP, e-mail Access ou
   personne). Tout porteur d'une session PIN est « humain » : il approuve, valide les automatismes et commande les
   chambres.
3. Les **rôles** `owner` / `member` / `guest` n'ont d'effet que sur la gestion des personnes
   (propriétaire) et sur le cloisonnement d'un invité **connecté**. `member` n'a aucun droit
   propre ; il n'existe aucun drapeau « enfant ».
4. Un appareil **anonyme du réseau local** est « le foyer » : il lit tout et commande partout, sauf
   dans les pièces protégées, sur les machines et sur les serrures. Un invité n'est cloisonné que s'il
   s'est connecté avec son cookie personnel.
5. La voix, l'assistant, MCP, la CLI et les scripts n'ont **aucune personne**. Leur origine est
   `assistant` (acteur fixe « Moli »), `mcp`, `cli` ou `api` (acteur déclaré par le client).
6. Un automatisme validé agit en `automation`, une origine que la garde ne retient jamais : pièces
   protégées, serrures et machines comprises. Il ignore **qui l'a déclenché**, et ne garde la trace de la
   personne que par la corrélation d'un appui de bouton virtuel.
7. Le **permis vivant** (`CommandPermit`) n'existe que pour les invités (et pour l'annulation des
   exécutions d'automatisme et de scène). Il est revérifié avant la préparation, à l'envoi, au retrait de
   la file, juste avant l'émission dans 13 pilotes, à l'approbation, et chaque seconde dans le flux SSE.

---

## 1. Les identités

### 1.1 Les preuves d'identité qui existent

| Mécanisme | Ce qu'il prouve | Lié à | Durée, bornes | Où il est gardé | Preuve |
|---|---|---|---|---|---|
| Session PIN, cookie `moli_session` | « quelqu'un connaît **le code de la maison** » | `Caller::key` de l'appelant qui l'a ouverte (IP sur le LAN, `access:<e-mail>`, `person:<id>`) ; **aucune personne** (`Session { created, ip }`) | 30 jours (`COOKIE_MAX_AGE_S`), 20 sessions au plus ; toutes fermées si le code change | coffre `core/ui_sessions` : SHA-256 du jeton et empreinte du code | `crates/moli-api/src/session.rs:46-63`, `:153-158`, `:421-431`, `:365-376`, `:599-617` |
| Le code lui-même | un code unique pour toute la maison (≥ 4 chiffres, ≥ 6 sans propriétaire Access) | — | verrou de 15 min par appelant après 5 échecs, doublé jusqu'à 24 h ; 20 échecs par heure et par « côté » (foyer, Internet) | coffre `core/ui_pin`, ou `MOLI_UI_PIN` / `MOLI_UI_PIN_FILE` | `session.rs:61-81`, `:281-320`, `:468-513` ; `crates/moli-os/src/main.rs:462-469` |
| Session personnelle, cookie `moli_person` | **une personne** : mot de passe ou invitation rachetée | `person.id`, quelle que soit l'adresse | 30 jours, 10 sessions par personne ; revérifiée à **chaque requête** (`get` → `active`) | coffre `core/person_sessions` (SHA-256) ; mot de passe `core/person/<id>/password` (PBKDF2) | `crates/moli-api/src/people.rs:41-44`, `:653-660`, `:682-735`, `:282-327` |
| Cloudflare Access | un **e-mail** signé (JWT RS256) arrivé par le tunnel | e-mail → personne si `people.json` le connaît | à chaque requête | — | `caller.rs:265-314` ; `crates/moli-api/src/access.rs:1-75` |
| Propriétaires Access de la configuration | `[server.access] owners` : e-mails propriétaires | e-mail | — | `moli.toml` ; créés propriétaires au premier démarrage | `access.rs:17-29` ; `people.rs:449-488` ; `session.rs:322-352` |
| Code d'installation | « a lu les journaux de la machine » | — | une heure, une fois, maison sans code | mémoire | `session.rs:221-258`, `:380-412` |
| Jeton de téléphone (`Authorization: Bearer`) | « ce téléphone appairé » | téléphone → personne facultative | — | `data/phones.json` (SHA-256) | `caller.rs:257-297` ; `crates/moli-api/src/phones.rs:111-145` |
| Jeton de machine | « l'agent de ce PC » | machine | — | — | `crates/moli-api/src/machines.rs:48-69` |
| Code d'alarme (profil) | **la personne** du profil (« le code identifie la personne ») | profil → personne facultative | relu à chaque autorisation | coffre `alarm/code_key`, `alarm/profile_codes` | `crates/moli-alarm-access/src/authorization.rs:8-58` ; **non raccordé** (§ 3.8) |

Il n'y a **aucune** autre preuve : pas de jeton par agent MCP, pas de compte par écran, pas
d'identification du locuteur à la voix, pas de lien entre une adresse MAC et une personne pour les
requêtes (les `macs` d'une personne servent seulement au pilote de présence,
`crates/moli-presence/src/lib.rs:1-40`).

### 1.2 Les personnes (`data/people.json`)

`Person { id, name, role, emails, color, rooms, starts, expires, reservation, macs, created }`
(`people.rs:71-100`). `PersonRef`, ce qui circule dans une requête :
`{ id, name, role, starts, expires, rooms }` (`people.rs:141-154`).

| Rôle | Ce qui le distingue réellement dans le code | Preuve |
|---|---|---|
| `owner` | gère les personnes, les zones et les invitations ; attribue un téléphone ; change le code de la maison ; gérera les profils d'alarme (avec une session PIN en plus) | `crates/moli-api/src/people_api.rs:55-77`, `:150-263` ; `crates/moli-api/src/rest.rs:425-476` ; `crates/moli-api/src/phones.rs:170-190` ; `crates/moli-api/src/alarm_access.rs:36-66` |
| `member` | **rien** : aucune vérification ne lit `Role::Member` hors des tests. Un membre se comporte comme un appareil anonyme du foyer, à ceci près qu'il est nommé dans `actor` | `grep is_owner\|Role::Member\|is_guest` : seuls `is_owner()` et `is_guest()` sont lus (`people.rs:157-163`) |
| `guest` | ses pièces seulement, des routes en liste blanche, une fenêtre `[starts, expires)`, une réservation ICS facultative | `people.rs:165-209`, `:434-444` ; `caller.rs:57-86` |
| (enfant) | **n'existe pas** : aucun champ, aucun rôle. « Enfant » n'apparaît que dans le nom des pièces protégées | `grep -i enfant\|child` dans `crates/` |

Une personne non invitée perd ses `rooms`, `starts`, `expires` et `reservation` à l'enregistrement
(`people.rs:1196-1203`). Il existe donc **aucune restriction par pièce ou par appareil pour un
membre ou un propriétaire**.

`active()` : la fenêtre du compte **et**, pour un invité lié, la réservation vivante
(`people.rs:434-444` ; `crates/moli-runtime/src/hub.rs:1965-1972`).

### 1.3 Qui appelle : `Caller`

`Caller { via, key, person }` (`caller.rs:122-129`), établi **une fois par requête** par le
middleware `identify` (`caller.rs:265-363`, posé en `crates/moli-api/src/lib.rs:391`, derrière la
garde `Host`, `lib.rs:392`).

| `Via` | Quand | `key` | Foyer ? | Preuve |
|---|---|---|---|---|
| `Loopback` | la machine elle-même, sans en-tête de relais | l'IP | oui | `caller.rs:241-249` |
| `Lan` | adresse privée, lien local ou ULA | l'IP | oui | `caller.rs:198-208`, `:245-246` |
| `Access { email }` | tunnel (en-têtes Cloudflare **et** boucle locale) avec JWT valide | `access:<e-mail>` | oui | `caller.rs:214-239`, `:276-283` |
| `Person` | depuis Internet, cookie `moli_person` valide | `person:<id>` | oui | `caller.rs:315-323` |
| `Phone` | `/api/phones/<id>/report` avec `Authorization` | `phone:<id>` | non | `caller.rs:284-297` |
| `Visitor` | maison de démonstration | — | oui | `caller.rs:338-350` |
| `Outside` | tout le reste ; **403** hors des routes publiques | IP ou `cf:<ip>` | non | `caller.rs:351-360` |

Pour la personne, l'e-mail Access passe en premier, puis le cookie `moli_person`, d'où qu'il vienne
(`caller.rs:298-323`). Un e-mail Access connu mais inactif reçoit 403 (`caller.rs:300-314`). Un
e-mail Access **inconnu** de `people.json` donne un appelant du foyer **sans personne**.

Pour le journal, `Caller::describe()` produit `tableau de bord (<nom de la personne>)`, sinon
`(<e-mail Access>)`, sinon `(<clé>)`, c'est-à-dire l'IP (`caller.rs:145-155`).

### 1.4 L'origine : `Origin` (`crates/moli-core/src/event.rs:126-147`)

Huit valeurs : `ui`, `household`, `api`, `mcp`, `cli`, `system`, `assistant`, `automation`.
La documentation en énumère sept et oublie `household` (`docs/components/coeur.md:19` ;
`docs/components/api.md:248`).

| Origine | Attribuée par | Condition | Preuve |
|---|---|---|---|
| `ui` | `rest::origin` | en-tête `x-moli-origin: ui` **et** session PIN de ce `Caller::key` | `rest.rs:75-77` |
| `household` | `rest::origin` | `x-moli-origin: ui`, sans session, `[guard] dashboard = "trusted"` (défaut), appelant du foyer | `rest.rs:78-80` ; `crates/moli-runtime/src/guard.rs:33-43` |
| `cli` | `rest::origin` | `x-moli-origin: cli` **et** `Via::Loopback` | `rest.rs:81` ; envoyé par la CLI : `crates/moli-os/src/main.rs:1508` |
| `api` | `rest::origin` | tout le reste : sans en-tête, valeur inconnue, `ui` sans preuve en mode `pin` | `rest.rs:82` |
| `mcp` | codé en dur dans chaque outil MCP | toujours | `crates/moli-api/src/mcp.rs:256`, `:289`, `:316`, `:426`, `:460` |
| `assistant` | codé en dur dans Moli | tout ordre de la conversation, **quel que soit l'interlocuteur** | `crates/moli-assistant/src/lib.rs:1236-1245`, `:1313-1318`, `:817-826` ; `crates/moli-assistant/src/door.rs:119-131` |
| `automation` | codé en dur dans le moteur | actions d'un automatisme validé | `crates/moli-automation/src/run.rs:343-348`, `:523-526`, `:785-791` |
| `system` | cœur, luminaires | expirations, ordres dérivés des luminaires sans permis, entrée clavier, imports HA (journal) | `hub.rs:679`, `:1287`, `:1345`, `:1432` ; `crates/moli-lights/src/lib.rs:549` ; `crates/moli-automation/src/lib.rs:103` |

L'en-tête `x-moli-origin` n'admet que `ui` et `cli` (`rest.rs:76-83`). Trois clients l'envoient :
le tableau de bord (`web/src/lib/hub.svelte.js:215`), le script d'appairage de l'appli
(`mobile/src/moli.ts:72-86`) et la CLI (`main.rs:1508`). Le même en-tête ouvre aussi le budget
élargi de 100 commandes par 10 s, réservé au foyer (`crates/moli-api/src/limit.rs:120-129`, `:35-46`).

### 1.5 Ce que porte un ordre, de l'entrée au journal

```
REST POST /api/command  {point, value, actor?, request_id?}            rest.rs:60-68
  │  origin = rest::origin(headers, session PIN, caller)                rest.rs:75-84
  │  actor  = describe() si ui|household, sinon actor DÉCLARÉ          rest.rs:86-94
  │  invité : guest_may(device) puis CommandPermit (persons.command_permit)
  ▼                                                                    rest.rs:96-136, people.rs:623-641
Hub::command_with_permit / command_with_request_id                     hub.rs:828-893
  │  clean_actor (64 caractères, sans contrôle)                        hub.rs:180-192, :933
  │  privacy + bornes                                                  hub.rs:201-211, :934-951
  │  permit.allows(point) ?                                            hub.rs:953-955
  │  prepare_context → CommandMetadata {origin, actor, request_id, correlation}
  │                                                                    hub.rs:1026-1075, driver.rs:19-24
  │  verdict(point, value, origin)                                     hub.rs:970, :1095-1112
  ├─ Approval → hold() → ApprovalRequest {origin, actor, …} + permit   hub.rs:996-1006, :1184-1295
  └─ Allow → send(prepared, journal, metadata, permit)                 hub.rs:972-994
        permit.allows ? (2ᵉ fois)                                      hub.rs:1798-1805
        CommandRequest {point, device, key, value, deadline, metadata, permit, reply, claim}
                                                                       hub.rs:1816-1826, driver.rs:66-82
        pilote : next_command() écarte un ordre abandonné ou révoqué   driver.rs:585-595
        JournalEntry {id, ts, origin, actor, action, outcome, duration_ms, correlation}
                                                                       event.rs:261-274, hub.rs:1858-1896
```

| Porté ? | Entrée HTTP | Hub (`CommandMetadata`) | Approbation (`ApprovalRequest`) | Pilote (`CommandRequest`) | Journal (`JournalEntry`) |
|---|---|---|---|---|---|
| **Personne** (`PersonRef`) | oui (`Caller.person`) | **non** | **non** | **non** | **non** (seulement le texte `actor` pour `ui`/`household`) |
| Origine | calculée | oui | oui | oui (`metadata.origin`) | oui |
| Acteur | attesté (`ui`/`household`) ou déclaré (autres) | oui | oui | oui | oui, champ texte unique |
| Canal (écran, appli, tablette, satellite, voix) | non (`Via` seulement) | non | non | non | non |
| Permis vivant | invité seulement | `CommandContext.permit` (`hub.rs:195-199`) | `Approvals.permits` (`hub.rs:243-247`, `:1276-1278`) | `CommandRequest.permit` | non |

Les pilotes ne lisent pas `metadata.origin` ni `metadata.actor`. Seuls les luminaires
(`moli-lights/src/lib.rs:530-552`) et le journal des minuteries et des dates
(`crates/moli-runtime/src/timer_hub.rs:124-125`, `temporal_hub.rs:80-81`) s'en servent.

---

## 2. Chaque canal

### 2.1 Canaux humains

| Canal | Authentification | Personne portée | Origine | Droits sur les ordres | Preuve |
|---|---|---|---|---|---|
| Tableau de bord sur le LAN, **sans code** | aucune (adresse privée) | aucune, sauf cookie `moli_person` | `household` (ou `api` en mode `pin`) | obéi partout **sauf** pièce protégée, appareil sans pièce si `protect_unassigned`, machine (`Control`), ouverture de serrure (`Lock`), sortie d'une pièce protégée : 202. Heures calmes ignorées. Peut renommer un appareil déjà nommé. Lance une routine validée si `household_may_run` | `rest.rs:78-80` ; `guard.rs:131-164` ; `hub.rs:1095-1127`, `:1669-1698` ; `crates/moli-api/src/automations.rs:249-284` ; testé : `crates/moli-api/tests/household.rs:149` |
| Tableau de bord **avec session PIN** | code de la maison (une fois, 30 jours) | aucune, sauf identification par ailleurs | `ui` | **tout**, sans jamais être retenu ; approuve ; valide, allume, restaure, supprime et importe des automatismes ; publie les dépendances et les scènes ; plan, clés, enregistrements ; caméras des pièces protégées | `rest.rs:77`, `:252-280` ; `guard.rs:133-135` ; `automations.rs:32-38` ; `crates/moli-api/src/assistant.rs:171-216`, `:270-291` ; `crates/moli-api/src/plan.rs:408-420` ; `rest.rs:862-890` |
| **Tablette murale partagée** | **aucune notion propre** : c'est un navigateur comme un autre | aucune | `household`, ou `ui` pendant 30 jours si quelqu'un y a tapé le code | ceux de la ligne du dessus : toute personne devant une tablette déverrouillée par le code est « humaine » 30 jours. Ni mode kiosque, ni pièce par écran, ni verrouillage à l'inactivité. Le seul code dédié recharge l'interface à une nouvelle version | `session.rs:63`, `:421-431` ; `web/src/lib/hub.svelte.js:173-176` ; recherche `kiosk\|kiosque\|tablette` dans `web/src` |
| Accès distant (Cloudflare Access) | JWT vérifié | la personne si l'e-mail est dans `people.json`, sinon aucune | `household` / `ui` | comme sur le LAN | `caller.rs:276-314` ; `access.rs` |
| Personne connectée par mot de passe, depuis Internet | cookie `moli_person` | **oui** | `household` / `ui` (`Via::Person` compte comme foyer) | comme sur le LAN ; elle peut ouvrir une session PIN (`login` exige seulement le foyer) | `caller.rs:315-323`, `:133-138` ; `rest.rs:559-605` |
| **Invité** connecté (`role = guest`) | mot de passe d'invitation ou e-mail Access | **oui** (`PersonRef` avec `rooms`) | `household` (ou `ui` s'il connaît le code) | ses pièces seulement : toutes les pièces de l'appareil, membres compris, doivent être dans `rooms`, et l'appareil ne doit pas appartenir aux pilotes `phones`, `people` ou `presence`. Routes en liste blanche. Permis vivant (fenêtre, réservation, pièces) jusqu'à l'émission. Il ne peut pas approuver | `people.rs:165-209`, `:623-641` ; `caller.rs:57-86`, `:324-337` ; `rest.rs:96-136`, `:188-235` ; testé : `crates/moli-api/tests/http.rs:925`, `:1105`, `:1624` |
| Invité **sur le Wi-Fi, non connecté** | aucune | **aucune** | `household` | **ceux du foyer entier** : le cloisonnement ne s'applique que si `Caller.person` est un invité | déduit : `caller.rs:241-254` (pas de personne sans cookie), `:324-337` (cloisonnement conditionné à `is_guest`) |
| Appli mobile « Moli » | vue web du tableau de bord (LAN ou Access, cookie éventuel) + jeton de téléphone pour les rapports | celle de la vue web ; le téléphone est lié à la personne s'il a été appairé depuis sa session | `household` / `ui` (l'appli envoie `x-moli-origin: ui`) | ceux du tableau de bord. Le jeton du téléphone ne sert **qu'**aux rapports, jamais à commander | `mobile/src/moli.ts:72-86` ; `phones.rs:39-79`, `:111-145` |
| Micro du tableau de bord ou de l'appli | aucune de plus | aucune | transcription puis `assistant` | ceux d'un agent (§ 2.2, Moli) | `assistant.rs:43-61`, `:30-41` |

### 2.2 Canaux non humains, agents et appareils

| Canal | Authentification | Personne | Origine, acteur | Droits | Preuve |
|---|---|---|---|---|---|
| REST, script | aucune (foyer) | aucune (une personne connectée sans en-tête `ui` n'est **pas** nommée : l'acteur reste celui que déclare le client) | `api`, acteur **déclaré** | garde « agent » : retenu en pièce protégée, en heures calmes, sur un appareil sans pièce si `protect_unassigned`, sur une machine ou une serrure | `rest.rs:82`, `:86-94` ; `hub.rs:1099-1101` ; `guard.rs:131-164` |
| CLI locale | boucle locale + en-tête | aucune | `cli`, acteur déclaré | comme `api` | `rest.rs:81` ; `main.rs:1508`, `:1601-1624` |
| MCP `/mcp` | **aucune** (garde `Host` seulement ; rmcp sans contrôle d'hôte) | aucune (`MoliMcp` ne reçoit pas de `Caller`) | `mcp`, acteur **déclaré** par l'outil | garde « agent ». Aucun outil d'approbation, de validation, d'allumage ni de lancement réel. Brouillons d'automatisme (`Author::Agent`). Peut **éteindre** un automatisme (brouillon enregistré avec `enabled: false` par défaut) et **annuler une scène** | `mcp.rs:19-46`, `:415-441`, `:578-613` (`:595-596`), `:306-320` ; `docs/components/api.md:708`, `:743-745` |
| Moli, l'assistant (texte) | aucune (foyer ; invité refusé) | **aucune** : `turn()` ne reçoit ni `Caller` ni personne | `assistant`, acteur fixe `"Moli"` | garde « agent » ; un ordre retenu revient au tableau de bord. Positions et Wi-Fi masqués au modèle | `assistant.rs:30-41` ; `crates/moli-assistant/src/lib.rs:584`, `:1227-1245` ; `crates/moli-assistant/src/house.rs:236-242` |
| Satellites vocaux ESPHome | aucune (appareil du foyer) | **aucune** ; le cerveau ne reçoit **ni le satellite, ni sa pièce** : `answer(conversation: Vec<Said>)`, avec `Speaker::Person \| Moli` | `assistant` via `turn(surface: "satellite")`, acteur `"Moli"` | comme Moli : rien ne restreint un satellite à sa pièce. L'enregistrement de ce qu'entendent les boîtiers exige une session PIN | `crates/moli-runtime/src/voice.rs:7-12`, `:49-57` ; `crates/moli-assistant/src/satellite.rs:101-131` ; `crates/moli-esphome/src/satellite.rs:233-256` ; `assistant.rs:171-216` |
| Wyoming (Piper, Whisper, openWakeWord) | — | — | — | supervision en lecture seule : aucun ordre | `crates/moli-wyoming/src/lib.rs:1` ; `docs/components/wyoming.md:3-8` |
| Interphone (Reolink) | aucune (un visiteur) | aucune | `assistant` pour relayer le message (Telegram, enceintes) | **n'agit pas sur la maison** : seulement des messages | `crates/moli-assistant/src/door.rs:1-8`, `:119-131` ; `crates/moli-reolink/src/door.rs:225`, `:262` |
| Telegram | jeton du bot (fichier) | conversations fixées dans `moli.toml`, sans lien avec les personnes | ordres `notify` (journalisés sous l'origine de l'appelant) | **sortant seulement** : aucune commande entrante | `crates/moli-telegram/src/lib.rs:1-11`, `:31-37`, `:182-194` |
| Notifications sur téléphone | — | — | `automation` (canal « telephone ») | envoyées à **tous** les points `push`. Un téléphone sans personne les reçoit toujours ; celui d'une personne retirée ou d'un invité au séjour fini, plus du tout | `crates/moli-automation/src/lib.rs:349-352` ; `crates/moli-automation/src/run.rs:812-818` ; `crates/moli-phones/src/lib.rs:405-423`, `:710-722` |
| Boutons et télécommandes physiques (Zigbee, `moli-z2m`) | — | aucune | ils publient des états et des occurrences, pas des ordres | ils **déclenchent** des automatismes, qui agissent en `automation` (§ 3.6) | `crates/moli-core/src/occurrence.rs:28-41` (ni origine ni personne) |
| Clavier d'alarme (Zigbee) | code saisi | (le code nommerait la personne) | entrée journalisée `Action::KeypadInput { device }` en `system` | **aucun consommateur dans le binaire** : le canal privé n'est jamais lu, l'alarme n'est pas raccordée | `crates/moli-z2m/src/keypad.rs:7-55` ; `hub.rs:655-687` ; `grep take_keypad_inputs` |
| Moteur d'automatismes | — | acteur = **nom de l'automatisme** | `automation` | jamais retenu, y compris pour une serrure ou une pièce protégée | `run.rs:503-540`, `:330-366` ; `guard.rs:133` ; `hub.rs:1099` |
| Luminaires (ordres aux membres) | — | — | **`system`** sans permis ; origine et acteur conservés avec un permis (invité, exécution) | l'ordre au luminaire a déjà été jugé sur les pièces de ses membres | `moli-lights/src/lib.rs:530-552`, `:781-784` ; `docs/ARCHITECTURE.md:183` |
| Ambiances | — | celle de l'appelant | origine et acteur de l'appelant ; permis de l'invité | chaque lampe passe par la garde | `crates/moli-ambiance/src/lib.rs:436-460` |
| Agents des PC (`moli-host`) | jeton de machine | — | `ui` + `describe()` pour réveiller le PC, après une session PIN | la commande à distance exige une session PIN | `machines.rs:94-129` |
| Réservations ICS | URL au coffre | — | aucun ordre | n'ouvrent ni ne ferment rien, conditionnent seulement l'accès d'un invité lié | `docs/components/reservations.md:9-20`, `:77-83` |

---

## 3. Les décisions de droit sur un ordre

### 3.1 Les contrôles, dans l'ordre

| Étape | Contrôle | Critère | Personne prise en compte ? | Preuve |
|---|---|---|---|---|
| Middleware | garde `Host`, refus intersite (`Origin`, `Sec-Fetch-Site`) | en-têtes | non | `crates/moli-api/src/guard.rs:61-110` |
| Middleware | `identify` : étranger 403, invité limité aux routes de la liste blanche, démo verrouillée | `Via`, rôle invité | rôle invité seulement | `caller.rs:265-363` |
| Middleware | débit par `Caller::key` | classe de la route | non | `limit.rs:35-140` |
| Route | `guest_may` (invité) | toutes les pièces de l'appareil ⊂ `rooms`, hors pilotes de localisation | **oui** (invité) | `rest.rs:96-123` ; `people.rs:194-209` |
| Route | `origin()` | en-tête + session PIN + foyer + mode | non | `rest.rs:75-84` |
| Hub | validation de l'entrée (code privé, bornes) | texte | non | `hub.rs:201-211`, `:934-951` |
| Hub | `permit.allows(point)` | vivant (invité) | **oui** (invité) | `hub.rs:953-955` |
| Hub | `prepare` : point inscriptible, valeur valide, pilote présent | spécification du point | non | `hub.rs:1731-1787` |
| Hub | `verdict` : `sensitive`, puis `GuardPolicy::check` | origine, sémantique, valeur, pièces, heure | non | `hub.rs:1095-1127` ; `guard.rs:131-164` |
| Hub | `send` : permis revérifié | vivant | **oui** (invité) | `hub.rs:1798-1805` |
| Pilote | `next_command` écarte un ordre abandonné ou révoqué ; `abandoned()` ou `send_permit` juste avant l'émission dans 13 pilotes | échéance, appelant présent, permis | **oui** (invité) | `driver.rs:585-595`, `:157-193` ; § 3.10 |

### 3.2 La garde : `verdict` et `GuardPolicy::check`

`agent = origin ∉ {ui, system, automation}` : `household` compte donc comme agent pour `sensitive`
(`hub.rs:1099`).

| Origine | Machine (`Semantic::Control`) | Ouvrir une serrure (`Lock` + valeur qui ouvre) | Pièce protégée (une pièce de l'appareil ou d'un membre) | Sans pièce, si `protect_unassigned` | Heures calmes | Preuve |
|---|---|---|---|---|---|---|
| `ui`, `system`, `automation` | passe | passe | passe | passe | passe | `guard.rs:133-135` ; `hub.rs:1099` |
| `household` | **retenu** | **retenu** | **retenu** | **retenu** | passe | `hub.rs:1100-1101` ; `guard.rs:136-148` |
| `api`, `mcp`, `cli`, `assistant` | **retenu** | **retenu** | **retenu** | **retenu** | **retenu** (pièces visées, ou toute la maison) | `guard.rs:136-163` |

- `rooms_of` réunit l'étiquette et la pièce native, de l'appareil et de chaque membre d'un groupe
  (`hub.rs:1129-1152`). La comparaison ignore la casse, les accents et les espaces multiples
  (`guard.rs:85-101`).
- `sensitive` : `Control` (« machine ») ; `spec.opens(value)` (« serrure »). Écrire `true` sur un pêne
  lisible verrouille et passe ; toute autre valeur, ou un point en écriture seule, ouvre
  (`hub.rs:1114-1127` ; `crates/moli-core/src/spec.rs:261-266`).
- Il n'y a **aucune sémantique « alarme »** dans la garde : la règle documentée « la garde retient
  l'écriture du point `arm` pour tout agent, dans toutes les pièces » (`docs/components/alarme.md:269-271`)
  n'est pas dans `sensitive` (énumération des sémantiques : `spec.rs:223-257`).
- `would_hold` : la même décision, sans envoi. Une valeur inconnue (bascule) est jugée au pire
  (`hub.rs:1077-1092`). On l'appelle seulement pour décider si le foyer peut lancer une routine
  (`automations.rs:274-284`).
- Étiquettes (`label_verdict`) : `ui` et `system` sont libres ; `household` nomme et renomme mais ne change pas la
  pièce d'un appareil gardé ; un agent peut nommer un appareil sans nom, jamais en renommer un ni le
  déplacer hors d'une pièce gardée (`hub.rs:1666-1698` ; `guard.rs:166-185`).
- La politique vit dans `moli.toml` (`[guard]`). Aucune surface ne la modifie
  (`crates/moli-os/src/config.rs:69-108`).

### 3.3 Les invités : `CommandPermit`, `guest_may` et leurs fenêtres

- **Ce qu'un invité voit et commande** : `guest_devices` et `in_rooms` exigent que **toutes** les
  pièces de l'appareil soient dans `rooms` (non vide) et excluent les pilotes de type `phones`,
  `people` et `presence` (`people.rs:59-61`, `:165-220`). La comparaison est une **égalité exacte de
  chaînes** (`self.rooms.contains(r)`, `people.rs:208`), alors que la garde normalise. En cas
  d'écart, elle refuse plutôt qu'elle n'autorise.
- **Le permis** : une fermeture sur une référence faible à `People`. À chaque appel, il relit la
  personne courante (`get` → `active` : fenêtre `[starts, expires)` et réservation vivante),
  vérifie qu'elle est encore invitée et qu'elle voit encore l'appareil (`people.rs:623-641`, `:434-444`).
- **La réservation** : n'est admise que si le pilote `reservations` tourne, si son instantané est frais
  (`poll + 10 s`, horloges murale et monotone) et si `début <= maintenant < fin`. Rien n'est restauré
  du cache (`crates/moli-runtime/src/reservations.rs:86-104` ; `hub.rs:1963-1992` ;
  `docs/components/reservations.md:56-75`).
- **Les luminaires composés** transmettent le permis à leurs seuls membres déclarés, toujours
  évalué sur le point parent (`driver.rs:210-223` ; `moli-lights/src/lib.rs:781-784`).
- **Les routes de l'invité** : en lecture, `/api/session`, `/api/health`, `/api/home`, `/api/ambiances`,
  `/api/devices*` (sans `/snapshot`), `/api/history`, `/api/approvals`, `/api/events`, `/api/people`,
  `/api/plan`, `/api/weather/*` ; en écriture, `/api/command`, `/api/ambiance`, `/api/session`,
  `/api/people/<id>/password` (`caller.rs:57-86`). Tout le reste reçoit 403, y compris MCP, l'assistant,
  le journal, les approbations, les automatismes et les caméras.
- **Un invité qui connaît le code de la maison** peut ouvrir une session PIN : `POST /api/session` est sur
  sa liste et `login` n'exige que le foyer (`rest.rs:559-605`). Ses ordres deviennent alors `ui`.
  Ils restent bornés à ses pièces par le permis, mais **la garde ne les retient plus** : serrure ou
  pièce protégée situées dans ses pièces (déduit : `rest.rs:77`, `:124-136` ; `guard.rs:133`). Il ne
  peut toujours pas approuver (route bloquée).

### 3.4 Les approbations

**Création** (`hold`, `hub.rs:1184-1295`) :
- `ApprovalRequest { id, ts, expires (+10 min), point, value, origin, actor, reason, device_name,
  native_name, rooms, request_id, correlation }`. Le nom et les pièces sont figés au moment de la demande
  (`event.rs:76-100`). Le permis de l'invité est gardé à côté (`hub.rs:1276-1278`).
- Une nouvelle demande pour le **même point et la même origine** remplace l'ancienne, **quel que soit
  l'acteur** : deux personnes `household`, ou deux agents MCP, s'écrasent l'une l'autre
  (`hub.rs:1239-1250`).
- 50 au plus. Au-delà, la plus ancienne est évincée, d'où qu'elle vienne (`hub.rs:174-176`, `:1251-1259`).
- Les demandes vivent en mémoire seulement : un redémarrage les efface (`docs/components/api.md:235-236`).

**Qui approuve** (`rest::decide`, `rest.rs:252-280` ; `Hub::resolve_approval`, `hub.rs:1313-1403`) :
- La seule condition est `origin == Ui`, et `decide` la fixe à partir de `humans.is_human(headers,
  caller.key)` seul (`rest.rs:262-266`). L'en-tête `x-moli-origin: ui` n'est **pas** vérifié, alors que la
  documentation l'exige (`docs/components/api.md:308`).
- Il suffit donc de **n'importe quelle session PIN valide** liée à cet appelant. Personne, rôle, droit
  sur la pièce visée : rien n'est vérifié. L'approbateur peut être celui qui a demandé. Un appareil anonyme du LAN muni d'un cookie
  PIN approuve.
- **Invité** : la route lui est fermée, même avec une session PIN (`caller.rs:84-85`). **Enfant** :
  la notion n'existe pas ; quiconque connaît le code approuve.
- L'agent et MCP n'ont aucun outil d'approbation (`docs/components/api.md:743-745`).

**Ce qui est revérifié à l'approbation** :
- l'existence de la demande, le point affiché, l'échéance (`hub.rs:1331-1351`) ;
- la validité de l'ordre (`prepare`, `hub.rs:1363`) ;
- le **permis de l'invité**, à l'envoi (`hub.rs:1339`, `:1386-1400`). Testé :
  `an_approval_retains_live_admission_instead_of_reviving_an_expired_guest` (`hub.rs:2984-3011`).

**Ce qui n'est pas revérifié** :
- la garde : un ordre approuvé part avec l'origine du demandeur, sans nouveau verdict
  (`hub.rs:1380-1385`) ;
- les droits de l'approbateur.

**Journal** : la ligne de la demande porte l'origine et l'acteur du demandeur, avec l'issue `pending`
(`hub.rs:1011-1022`). La ligne `Action::Approval` porte l'origine `ui` et l'acteur
`caller.describe()` de l'approbateur, c'est-à-dire son nom s'il est identifié, sinon son e-mail ou son
IP. Les deux lignes partagent la même `correlation` (`hub.rs:1440-1467`). L'expiration est journalisée en
`system` (`hub.rs:1406-1438`).

**Les autres validations humaines**, toutes « une session PIN quelconque » :

| Acte | Contrôle | Journal de l'auteur | Preuve |
|---|---|---|---|
| valider, allumer, restaurer, supprimer, importer un automatisme | `by.human = is_human` | origine `ui`, acteur **« une personne (code) »** : jamais un nom | `automations.rs:32-38` ; `crates/moli-automation/src/lib.rs:89-117`, `:534-544`, `:558-687` ; `locales/fr/moteur.json:183` |
| publier une dépendance, un planning, un calcul ou une scène | `human` + aperçu exact | `describe()` | `crates/moli-api/src/dependencies.rs:98-122` |
| réduire l'historique | `origin == Ui` | `describe()` | `rest.rs:742-787` ; `crates/moli-history/src/retention.rs:643-656` |
| clé de l'assistant, enregistrement des satellites, plan, Tuya, Netatmo, Cast, Android TV, téléphone oublié, mode distant d'un PC | `is_human` | **pas de journal** (au mieux un `tracing`) | `assistant.rs:171-291` ; `plan.rs:408-460` ; `phones.rs:82-109` ; `machines.rs:94-114` |

### 3.5 Les automatismes

**Qui crée et modifie** (`automations.rs:122-155`) :
- tout appelant du foyer (anonyme compris) et MCP (`mcp.rs:578-613`) ;
- sans session PIN, l'auteur est `Author::Agent` et le journal porte l'origine `api` et l'acteur « un agent ».
  Un membre de la famille au tableau de bord sans code est donc journalisé comme un agent
  (`lib.rs:97-105`) ;
- l'enregistrement n'approuve jamais. Un non-humain ne peut qu'**éteindre**, et ne peut changer ni le nom ni la note
  d'un automatisme déjà validé (`lib.rs:438-532`, `:454-469`) ;
- un import HA est journalisé en `system`, alors qu'une personne l'a lancé (`lib.rs:103`).

**Qui valide, allume, restaure, supprime, lance pour de vrai** : une session PIN quelconque
(`lib.rs:558-740`). **Exception** : le foyer sans code lance une routine validée si `household_may_run`
est vrai (`automations.rs:249-269`). Or `household_may_run` n'examine que `Set` et `Toggle`. Une étape
**`Scene`** est réputée libre (`automations.rs:274-284`, ligne `_ => true` en `:282`), et la scène
s'exécute ensuite en `automation` (`run.rs:330-366`). Le test existant ne couvre que `Set`
(`crates/moli-api/tests/household.rs:229-290`).

**Sous quelle identité ils agissent** : `Origin::Automation`, acteur = nom de l'automatisme, permis
= annulation de l'exécution (`run.rs:145-148`, `:503-540`, `:785-791`). La garde ne les retient jamais
(`guard.rs:133`). C'est voulu : « un automatisme validé par un humain peut ouvrir : c'est l'intention
de cet humain » (`docs/components/coeur.md:88-93`).

**La trace de la personne qui l'a déclenché** :
- déclencheur `when_occurrence` (appui d'un bouton virtuel par `/api/command` avec `request_id`) :
  la ligne d'appui porte l'origine et l'acteur de l'appelant ainsi qu'une `correlation`
  (`hub.rs:2652-2737`). L'occurrence, l'exécution et les lignes d'action reprennent cette corrélation
  (`run.rs:62-66` ; `occurrence.rs:30-41`). La personne se retrouve **par jointure** sur `correlation`,
  si l'appui venait de `ui` ou `household` ;
- déclencheur `when_state`, `when_threshold`, horaire ou soleil : **aucune** trace. Ni `Event::State`
  ni `Occurrence` ne portent d'origine ou de personne (`event.rs:22-30` ; `occurrence.rs:30-41`) ;
- lancement manuel : **aucune ligne de journal** pour le lancement lui-même (`lib.rs:689-740` ;
  `crates/moli-automation/src/engine.rs:602-748`). L'exécution note seulement `why =
  « Lancée à la main »` (`locales/fr/moteur.json:178`). La documentation dit l'inverse : « acteur au
  journal… au lancement d'un automatisme » (`docs/components/api.md:107-111`).

**Ce que voit l'approbateur** : la liste des pièces protégées atteintes, scènes comprises
(`crates/moli-automation/src/check.rs:243-245`, `:383-395`). Le vérificateur ne signale **ni** les
serrures, **ni** les machines, **ni** qui peut déclencher l'automatisme.

### 3.6 Les scènes et leurs baux

- Exécution par REST : origine `rest::origin(...)`, acteur `caller.describe()`, même pour un script
  `api`, qui est alors étiqueté « tableau de bord (…) » (`crates/moli-api/src/scenes.rs:36-59`). Par
  MCP : `mcp`, sans acteur (`mcp.rs:276-293`). Par un automatisme : `automation` (`run.rs:343-348`).
- Chaque cible passe par la garde avec cette origine, sous un permis fait de l'échéance, de la
  supervision de la scène et du permis parent (`crates/moli-runtime/src/scene_execution.rs:358-370`).
- **Annuler** une exécution, et révoquer ses approbations en attente, n'exige **aucune** vérification
  d'origine ni d'auteur. Tout appelant du foyer, MCP compris, annule la scène de n'importe qui
  (`crates/moli-runtime/src/scene_hub.rs:325-344` ; `scenes.rs:66-81` ; `mcp.rs:306-320`).
- Les **baux** (`Lease`) ne portent qu'une corrélation et une échéance. Un ordre ultérieur sur le même
  appareil, quel que soit son auteur, les invalide (`crates/moli-runtime/src/scenes.rs:139-147`,
  `:263-285`). Ce mécanisme sert la cohérence, pas les droits.
- Les invités n'ont accès à aucune route de scène (`caller.rs:64-86` ; `docs/components/api.md:1070-1071`).

### 3.7 La serrure (Nuki)

- Points `locked` et `unlatch`, sémantique `Lock` (`crates/moli-nuki/src/model.rs:168`, `:188`).
  Ouvrir est retenu pour toute origine autre que `ui`, `system` et `automation` ; verrouiller passe
  (`hub.rs:1114-1127`).
- Le pilote vérifie `command.abandoned()`, donc le permis, juste avant d'agir
  (`crates/moli-nuki/src/lib.rs:285`). Le pont ne rapporte pas qui a ouvert depuis l'appli Nuki ou
  depuis son clavier : aucun champ « auteur » n'est publié (`model.rs:56`, `:92`).
- Pour un invité : `202`, il ne peut pas approuver, l'approbation revérifie son permis
  (`docs/components/api.md:1019-1022` ; `hub.rs:2984-3011`).

### 3.8 L'alarme : un moteur et des profils, **non raccordés**

- **Moteur pur** (`crates/moli-alarm`). `Source = Ui | Household | Keypad(id) | Automation | Agent`,
  sans personne (`model.rs:105-112`) :
  - agent qui arme depuis `disarmed` : `HumanApprovalRequired` + `RetainArm` ; sinon `SourceDenied`
    (`engine/commands.rs:49-65`) ;
  - automatisme : ne change pas de mode (`commands.rs:66-69`) ;
  - sortir sans délai : `Ui` seulement (`commands.rs:93-96`) ;
  - **désarmer** : `Agent` et `Automation` reçoivent `SourceDenied` ; un code est exigé **à chaque fois**
    (`missing_allowed = false`) (`commands.rs:200-216`).
- **Profils** (`crates/moli-alarm-access`) : `can_arm`, `can_disarm`, modes, `from`, `until`, personne
  facultative. Pour un invité, la fenêtre est l'intersection de celle du profil et du compte, plus une
  réservation vivante. Une personne disparue rend son profil révoqué (`authorization.rs:8-58` ; `model.rs:52-93`).
  `RequestSource::dashboard(id, human)` vaut `Ui` si `human`, sinon `Household` : la session PIN ne
  dispense pas du code (`service.rs:79-107`). Gérer les profils exigera un **propriétaire** et une session PIN
  vivante (`AlarmOwner`, `crates/moli-api/src/alarm_access.rs:36-66`). Testé :
  `alarm_management_requires_both_owner_identity_and_a_live_pin_session` (`alarm_access.rs:182-228`).
- **Raccordement** : « HTTP routes remain in lot 7/9 » (`alarm_access.rs:1`). Aucun appel à
  `alarm_profiles(` hors des tests, aucun consommateur de `take_keypad_inputs`. `moli-os` ne fait que
  réserver l'espace de noms `alarm` du coffre (`crates/moli-os/src/config.rs:313`, `:366-367`). En pratique,
  **aucun ordre d'alarme n'existe encore dans Moli**.

### 3.9 Les autres ordres sensibles

- **Impression** : `ui` ou `household` seulement, aucun agent (`rest.rs:935-955`). **Non journalisée**
  (`tracing` en `rest.rs:989` ; `hub.rs:2243-2267`).
- **Caméras** : une pièce protégée exige une session PIN en REST et est toujours refusée en MCP. Les
  autres sont libres pour le foyer et MCP, refusées à l'invité (`rest.rs:862-905` ; `mcp.rs:393-413` ;
  `hub.rs:2313-2326`). Les consultations ne sont **pas journalisées**.

### 3.10 Le permis est-il revérifié à l'envoi, au transport, dans le flux SSE ?

| Frontière | Revérifié ? | Preuve |
|---|---|---|
| Avant préparation | oui | `hub.rs:953-955` |
| À l'envoi au pilote | oui | `hub.rs:1798-1805` |
| Au retrait de la file (tous les pilotes) | oui : `next_command` refuse un ordre `abandoned()` (échéance, appelant parti, **permis**) | `driver.rs:585-595`, `:157-193` |
| Juste avant l'émission (transport) | oui, **dans les pilotes qui l'appellent** : `abandoned()`, `send_permit()`, `wanted()` ou `begin_send()` dans 13 pilotes (androidtv, ariston, cast, landroid, lights, netatmo, nuki, roborock, samsung, twinkly, voltalis, et la file `orders.rs` qu'utilisent ariston et voltalis). Les 39 fichiers qui appellent `next_command` ne le font pas tous | `driver.rs:112-148`, `:194-206` ; `crates/moli-runtime/src/orders.rs:123`, `:191-197` |
| Ordres dérivés d'un luminaire | oui, `member_permit` | `driver.rs:210-223` |
| Approbation d'une demande retenue | oui, permis gardé avec la demande | `hub.rs:1276-1278`, `:1339`, `:1399` |
| Flux SSE | **oui pour un invité seulement** : avant chaque donnée, première photo comprise, et chaque seconde au repos ; le flux se ferme si l'invité n'est plus actif | `crates/moli-api/src/sse.rs:101-121` |
| Flux SSE d'un non-invité | **non** : `view.guest = None`, aucune relecture | `sse.rs:97-100`, `:104-112` |
| Requête HTTP | oui, la personne est recalculée à chaque requête | `caller.rs:298-323` ; `people.rs:653-660` |

Testé : `auto_h1_r1_queued_or_taken_orders_observe_revocation_without_a_task_wakeup`,
`revocation_after_dequeue_still_prevents_emission` (`hub.rs:2923-2982`) ;
`reservations_ics_gate_an_existing_guest_session_and_live_stream` (`tests/http.rs:1624`).

---

## 4. La lecture : qui voit quoi

| Ressource | Foyer anonyme (LAN, Access inconnu) | Membre ou propriétaire | Invité connecté | MCP | Assistant (modèle) | Preuve |
|---|---|---|---|---|---|---|
| Appareils et états (`/api/devices`, SSE) | tout | tout | appareils dont **toutes** les pièces sont les siennes, hors localisation. Mais la photo garde **tous les pilotes** et le statut de la garde (noms des pièces protégées) | tout (`list_devices`, `get_device`) | tout, sauf latitude, longitude, précision, Wi-Fi | `rest.rs:36-42` ; `people.rs:179-191` ; `mcp.rs:344-391` ; `house.rs:236` |
| Localisation : `telephones:*` (latitude, longitude, Wi-Fi), `personnes:*` (latitude, longitude, zone, maison) | **tout** | tout | rien (`WHEREABOUTS`) | **tout**, historique compris | masqué | `crates/moli-phones/src/lib.rs:606-626`, `:747-784` ; `crates/moli-api/src/whereabouts.rs:207-273`, `:317-330` ; `people.rs:59-61` |
| Personnes (`/api/people`) | noms, rôles, couleurs ; zones (coordonnées du domicile) | le propriétaire voit aussi e-mails, pièces, dates, réservation, MAC, mot de passe défini | lui-même, sans zone | — | — | `people_api.rs:86-148` |
| Journal (`/api/journal`, SSE `journal`, `get_journal`) | **tout** : origine, acteur (noms, e-mails Access, IP), action, issue | tout | rien | tout | — | `rest.rs:1065-1067` ; `sse.rs:190-193` ; `mcp.rs:468-476` |
| Demandes retenues | toutes, avec l'acteur | toutes | celles de ses pièces | toutes (`system_status`) | — | `rest.rs:237-243` ; `mcp.rs:322-342` |
| Historique (`/api/history`) | tout | tout | les points de ses pièces | tout, plus `history_sql` (SQL brut en lecture seule) | sans les points privés | `rest.rs:680-718` ; `mcp.rs:478-494`, `:635-649` |
| Plan, disposition (`/api/plan`, `/api/home`) | tout | tout | vides | — | — | `plan.rs:385-406` ; `rest.rs:801-826` |
| Caméras | sauf pièce protégée (code) | idem | rien | sauf pièce protégée | jamais d'image | § 3.9 |
| Échanges avec Moli (`/api/assistant/exchanges`) | tout, 30 jours (questions, réponses, ordres, `surface`) | tout | rien | — | — | `assistant.rs:149-162` ; `crates/moli-assistant/src/exchanges.rs:1-60` |
| Enregistrements des satellites | code | code | rien | — | — | `assistant.rs:190-216` |
| Notices (`maison`) | oui | oui | non (filtrées) | — | — | `sse.rs:190-193` |
| Automatismes et exécutions | tout | tout | rien | tout | — | `automations.rs:90-120`, `:325-361` ; `mcp.rs:533-576` |

**Le journal et l'historique disent-ils qui a commandé ?**
- Le **journal** : l'origine (`origin`) et un **texte** `actor` de 64 caractères au plus
  (`event.rs:261-274` ; `hub.rs:180-192`). Pour `ui` et `household`, c'est le serveur qui l'écrit :
  `tableau de bord (Nom | e-mail | IP)`. Pour `api`, `mcp` et `cli`, c'est la valeur déclarée par le client,
  dans le **même champ**. Rien ne distingue un acteur attesté d'un acteur déclaré, sinon l'origine
  (`rest.rs:86-94`). Aucun champ structuré pour une personne.
- L'**historique** : des valeurs seulement, `samples(point_id, ts, kind, num, txt)`, sans origine ni
  acteur (`crates/moli-history/src/store.rs:22-30`). Les positions y sont conservées, sauf exclusion
  `[history] exclude` (`crates/moli-history/src/lib.rs:302-333`).
- **Jamais journalisés** : création, modification et suppression d'une personne ; invitations ;
  changement du code ; appairage, attribution et oubli d'un téléphone ; zones ; plan ; clé de l'assistant ;
  enregistrement des satellites ; consultation d'une caméra ; impression ; lancement manuel d'un
  automatisme. L'énumération `Action` n'a pas de variante pour ces actes (`event.rs:149-228`) et
  aucun appel `record` n'existe dans `crates/moli-api` (`grep`).

---

## 5. Les trous, classés par gravité

Élevé : un ordre peut partir, ou un droit être exercé, par quelqu'un qu'on ne voulait pas.
Moyen : traçabilité, révocation ou confidentialité en défaut. Faible : écart, fuite mineure.

### Élevé

| # | Trou | Preuve | Nature |
|---|---|---|---|
| E1 | **La personne n'est pas propagée** au-delà de la couche HTTP. `CommandMetadata`, `ApprovalRequest`, `CommandRequest` et `JournalEntry` n'ont pas de champ personne. Aucune décision du hub ne peut dépendre de qui agit | `driver.rs:19-24`, `:66-82` ; `event.rs:76-100`, `:261-274` | lu |
| E2 | **Le code de la maison est commun et anonyme**. Une session PIN = « un humain », lié à une adresse et non à une personne. Approuver, valider, commander les chambres, ouvrir une porte, regarder une caméra de chambre : tout porteur du code, pendant 30 jours, sur l'appareil qui l'a tapé, **tablette partagée comprise**. Aucun verrouillage à l'inactivité, aucune distinction propriétaire, membre ou enfant | `session.rs:153-158`, `:421-431` ; `rest.rs:252-280` ; `automations.rs:32-38` | lu |
| E3 | **Approbation sans droit sur l'ordre** : `resolve_approval` ne vérifie que `origin == Ui`. Ni la personne, ni le rôle, ni le droit sur la pièce, ni la séparation entre demandeur et approbateur ne sont contrôlés. L'approbateur d'un automatisme n'est **jamais nommé** (« une personne (code) ») | `hub.rs:1313-1323` ; `crates/moli-automation/src/lib.rs:108-116`, `:534-544` ; `locales/fr/moteur.json:183` | lu |
| E4 | **Escalade par automatisme** (« confused deputy ») : un automatisme validé agit en `automation`, que la garde ne retient jamais (serrure, machine, pièce protégée). Il peut être déclenché par un état ou une occurrence produits par **n'importe qui** : agent MCP, script, foyer anonyme, invité dans ses pièces. Les déclencheurs ne connaissent pas l'origine, et le vérificateur n'avertit pas qu'un déclencheur est accessible à un agent. Exemple : un agent appuie sur un bouton virtuel sans pièce (libre hors heures calmes si `protect_unassigned = false`, le défaut), et l'automatisme validé ouvre la porte | `guard.rs:131-135` ; `hub.rs:1099` ; `occurrence.rs:30-41` ; `event.rs:22-30` ; `run.rs:503-540` ; `guard.rs:19-30` (défaut) | déduit |
| E5 | **Le foyer sans code lance une scène de chambre** : `household_may_run` ignore les étapes `Scene` (`_ => true`). Une routine validée qui contient une scène atteignant une pièce protégée ou une serrure se lance depuis un appareil anonyme du LAN, puis s'exécute en `automation`. Cela contourne la règle D0 « les chambres demandent le code » | `automations.rs:274-284` ; `run.rs:330-366` ; seul `Set` est testé : `tests/household.rs:229-290` | déduit |
| E6 | **Réseau local = foyer, et l'invité n'est cloisonné que s'il s'est connecté**. Un invité sur le Wi-Fi principal, sans cookie, ou après une déconnexion, retrouve tous les droits du foyer : il lit le journal et les positions, commande hors des chambres, parle à Moli et à MCP. Aucune documentation ne recommande un réseau invité séparé (seulement un « réseau IoT isolé, ou mode `pin` ») | `caller.rs:241-254`, `:324-337` ; `SECURITY.md:13-20` ; `docs/ARCHITECTURE.md:114-117` | déduit |

### Moyen

| # | Trou | Preuve |
|---|---|---|
| M1 | **MCP sans authentification** : tout programme du LAN, ou toute personne admise par Access, lit le journal (noms, e-mails, IP), les positions (`telephones:*`, `personnes:*`), tout l'historique en SQL, et commande avec un **acteur qu'il choisit**. Aucun agent n'est identifié ni borné par pièce | `mcp.rs:19-46`, `:160-172`, `:415-441`, `:468-476`, `:635-649` ; `docs/components/api.md:708` |
| M2 | **La révocation n'est pas vivante pour les non-invités** : (a) le flux SSE d'un membre ou d'un propriétaire supprimé, rétrogradé, ou retiré d'Access continue, journal et notices compris, jusqu'à la reconnexion ; (b) retirer une personne ne ferme pas les **sessions PIN** ouvertes depuis ses appareils du réseau local, dont la clé est l'IP (elles ne sont pas liées aux personnes) : seul un changement de code le fait. Depuis Internet, la clé `person:<id>` cesse d'être reconnue et l'appelant redevient un étranger ; (c) un automatisme validé par quelqu'un qui part continue de tourner | `sse.rs:97-112` ; `people.rs:977-1017` (seul `end_sessions` personnel, `:1014`) ; `session.rs:365-376` |
| M3 | **La voix n'a ni locuteur ni pièce** : le satellite connaît son identifiant, mais `VoiceBrain::answer` ne le reçoit pas. L'ordre vocal est `assistant`, acteur « Moli », comme une question tapée. Un satellite de chambre n'est pas restreint à sa pièce, et une voix d'enfant ne se distingue pas d'une autre. La `surface` d'un tour (`satellite`, `bubble`, `page`) est déclarée par le client et journalisée telle quelle dans les échanges | `voice.rs:7-12`, `:49-57` ; `crates/moli-esphome/src/satellite.rs:233-256` ; `crates/moli-assistant/src/lib.rs:257-269`, `:584-650` |
| M4 | **Les rôles sont presque vides** : `member` n'a aucun effet ; il n'y a ni ACL par personne, ni restriction par pièce ou par appareil pour un membre, ni drapeau enfant. Le propriétaire ne gère que les personnes, le code et les téléphones | `people.rs:63-69`, `:1196-1203` ; `grep` (§ 1.2) |
| M5 | **Traçabilité lacunaire** : `actor` mêle des valeurs attestées et déclarées dans un seul texte. Une personne connectée qui appelle sans l'en-tête `ui` n'est pas nommée, et un script peut écrire `tableau de bord (Untel)` en `api`. Des actes ne sont pas journalisés (§ 4). Le lancement manuel d'un automatisme et l'auteur d'un déclenchement par état ne laissent aucune trace. L'import HA est journalisé en `system` | `rest.rs:86-94` ; `hub.rs:180-192` ; `lib.rs:103`, `:689-740` |
| M6 | **Un invité qui connaît le code** devient `ui` pour ses ordres : il ouvre la serrure et commande la pièce protégée situées dans ses pièces sans approbation | `rest.rs:559-605`, `:77`, `:124-136` |
| M7 | **L'alarme est spécifiée, pas raccordée** : pas de pilote, pas de routes, pas de consommateur du clavier. La règle « la garde retient `arm` pour tout agent » n'est pas dans `sensitive`, faute de sémantique « alarme » | `alarm_access.rs:1` ; `hub.rs:655`, `:1116-1127` ; `spec.rs:223-257` ; `docs/components/alarme.md:269-271` |
| M8 | **Des approbations sans auteur et volatiles** : une demande en remplace une autre à point et origine égaux, quel que soit l'acteur ; un afflux (MCP : 120 requêtes par 10 s) évince la demande d'un tiers au-delà de 50 ; tout disparaît au redémarrage | `hub.rs:1239-1259` ; `limit.rs:49-51` |
| M9 | **Tout appelant du foyer, ou MCP, annule la scène d'un autre** et révoque ses approbations en attente | `scene_hub.rs:325-344` ; `mcp.rs:306-320` |

### Faible

| # | Trou | Preuve |
|---|---|---|
| F1 | La vue de l'invité laisse passer la **liste des pilotes** et le statut de la garde (noms des pièces protégées, heures calmes) : `/api/health`, et la photo `guest_view` ne filtre que les appareils et les demandes | `rest.rs:22-34` ; `people.rs:179-191` |
| F2 | Les notifications vont à **tous** les téléphones et à toutes les conversations Telegram de la configuration, sans destinataire par personne. Un téléphone appairé sans personne les reçoit indéfiniment. `POST /api/assistant/recap` (sans code) envoie sur Telegram le résumé des échanges des dernières 24 h | `crates/moli-automation/src/lib.rs:349-357` ; `crates/moli-phones/src/lib.rs:411-423` ; `assistant.rs:218-224` |
| F3 | Pièces de l'invité comparées par **égalité exacte** (`rooms.contains`) alors que la garde normalise. Il faut y lister l'étiquette **et** la pièce native : le défaut refuse, il ne laisse pas passer | `people.rs:203-209` ; `guard.rs:85-101` |
| F4 | Les exécutions de scène et les brouillons de dépendance par REST prennent l'acteur « tableau de bord (…) » même pour un script `api` | `scenes.rs:47-53` ; `dependencies.rs:78-85` |
| F5 | Écarts entre documentation et code : `decide` n'exige pas l'en-tête `ui` (`api.md:308`) ; le lancement d'un automatisme n'est pas journalisé (`api.md:107-111`) ; `household` manque à la liste des origines (`coeur.md:19`, `api.md:248`) ; MCP compte plus de 14 outils (`api.md:724`) | § 3.4, § 3.5, § 1.4 |
| F6 | Le foyer anonyme renomme un appareil déjà nommé hors des pièces gardées, alors que les cartes d'approbation affichent ce nom. Le nom natif est montré à côté | `hub.rs:1680-1689` ; `event.rs:89-95` |

---

## 6. Documents existants et décisions déjà prises

### Versionnés dans `a835893`

| Sujet | Ce qui est décidé ou écrit | Où |
|---|---|---|
| D0, le tableau de bord et le code | la famille commande sans code sur le LAN, hors pièces protégées et machines ; le code sert à valider, à autoriser et pour les chambres ; mode `pin` disponible. Resserré le 12 oct. après audit : « l'en-tête du dashboard est public » | `docs/ARCHITECTURE.md:20-23`, `:155` ; `docs/README.md:81-86` ; `docs/components/coeur.md:115-123` |
| Origines et confiance | tableau `ui`, `household`, `automation`, `system`, `api`/`mcp`/`cli`/`assistant` ; « le mode `trusted` suppose un réseau domestique de confiance » ; « avant une exposition plus large : sessions d'appareil » | `docs/ARCHITECTURE.md:100-117`, `:419-428` ; `docs/components/api.md:91-200` |
| Garde | pièces protégées (« là où dort un enfant, aucun agent ne touche à rien sans un humain »), heures calmes, machines, serrures (« un automatisme validé par un humain peut ouvrir : c'est l'intention de cet humain ») | `docs/components/coeur.md:73-111` |
| Approbations | 202, 10 min, point nommé, « seul un humain la libère », en mémoire | `docs/components/api.md:202-236` |
| Agents | « Agents never act freely » ; « un automatisme écrit par un agent reste un brouillon » ; aucun outil MCP d'approbation | `SECURITY.md:21-25` ; `AGENTS.md:31` ; `docs/components/api.md:743-745` |
| Invités | « Guests see and command their rooms only: no other room, no camera, no one's location, no journal, no assistant, no setting, and no phone of their own » ; fenêtres datées, invitations, réservation liée, « l'invité ne peut pas l'approuver lui-même » | `SECURITY.md:29-32` ; `docs/components/api.md:951-1022` ; `docs/components/reservations.md:28-75` ; `docs/components/dashboard.md:120-129` |
| Automatismes | « un humain valide, la maison exécute » ; enregistrer n'approuve jamais ; un agent ne peut qu'éteindre ; agit en `automation` ; journal « qui l'a faite (une personne, Moli, un agent, l'import) » | `docs/components/automatismes.md:7-31`, `:68-75` |
| Alarme | Q1 désarmer : code à chaque fois, session PIN ou non ; Q3 un automatisme arme seulement ; agents : armer retenu, désarmer jamais (« même la demande de désarmer est refusée, pas retenue ») ; profils gérés par un propriétaire avec session humaine ; destinataires des notifications : propriétaires et membres, jamais un invité ; le code identifie la personne | `docs/components/alarme.md:228-271`, `:345`, `:478-488`, `:536-543` ; `docs/components/alarme-profils.md:20-26`, `:78-90` |
| Assistant | jamais de position précise ni de nom de Wi-Fi au modèle ; agit en `Origin::Assistant` sous la garde | `SECURITY.md:33-34` ; `docs/components/moli.md:45` |
| Points ouverts déjà notés | lectures ouvertes sans session « par conception » (état, journal, historique, énergie, caméras hors chambres, routes du modèle), « à revoir avant toute exposition hors du réseau local » ; `cli` fragile derrière un proxy local | `docs/components/api.md:891-905` |

### Non versionnés : présents dans l'arbre de travail, hors `a835893`

| Document | Ce qu'il propose sur ce sujet |
|---|---|
| `docs/proposition/ordres-appareils-v2.md` § 11 « Les principaux et leurs droits » (l. 666-696), § 5.2 (l. 329-362) | le principal vient de la connexion, un compte par écran, appli, agent ou script ; ce qu'un message dit de sa personne est ignoré ; un permis borné dans le temps et par pièces (« un invité, un écran de pièce ») ; chaque cible autorisée ; revérification à l'arrivée, à l'approbation et à l'envoi ; à la révocation, les demandes retenues refusées et les voies vidées ; « seul un humain, à l'écran, décide d'une retenue » |
| `docs/proposition/ordres-appareils.md` (v1), `ordres-appareils-revue-codex.md` | version précédente et sa revue |
| `docs/proposition/critique-workflow.md` | nouvelles origines « Dehors », « Service », « Sarah » (l. 398, 887, 1040, 1070) ; « des droits et une visibilité par personne : la santé, ce que Sarah a retenu, les questions d'un enfant » (l. 950) ; « Demander aux parents » et « un enfant juge et partie » (l. 343, 681, 1518) |
| `docs/revue-complete/` | compilation des propositions (non relue en détail ici) |

---

## Annexe : tests qui fixent les règles actuelles

- Sessions PIN liées à l'appelant : `sessions_are_bound_to_their_client` ; un agent ne devine pas le
  code et ne bloque pas l'humain : `an_agent_can_neither_guess_nor_lock_the_human_out`
  (`crates/moli-api/src/session.rs:700-731`).
- Tableau de bord sans code arrêté aux chambres : `crates/moli-api/tests/household.rs:149`. Routine :
  `:229` (étapes `Set` seulement).
- Invité : `crates/moli-api/tests/http.rs:925` (lecture et commande de ses pièces), `:1105` (flux en
  direct), `:1302` (depuis Internet), `:1624` (réservation, session et flux déjà ouverts).
- Permis vivant : `crates/moli-runtime/src/hub.rs:2923-3011` ; luminaires : `driver.rs:803-848`.
- Téléphones : `crates/moli-api/tests/http.rs:1160` (`a_phone_follows_its_person`).
- Alarme, gestion des profils : `crates/moli-api/src/alarm_access.rs:182-364`.

Non vérifié ici : le comportement réel des interfaces (aucune exécution), le détail de chacun des
39 pilotes au transport, et le contenu de `docs/revue-complete/`.
