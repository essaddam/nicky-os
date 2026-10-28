# Revue Codex — les droits sur les ordres

Seconde revue de [l'analyse des ordres des appareils, v2](ordres-appareils-v2.md), sur les droits :
qui donne un ordre, pour qui, avec quelle preuve, et sous quel mandat. Faite le 24 octobre 2026 par
Codex (`gpt-6.1-sol`, effort `max`, session `01a16d7c-2e5b-7a22-8924-cdf2f5f8b84b`, reprise de la
première revue), en lecture seule : aucun fichier du dépôt modifié. Texte de Codex, non retouché ;
seul le titre est changé. La [v3](ordres-appareils-v3.md) reprend chacun des 23 constats (§ 3.5).
Les numéros de ligne du banc sont ceux de la v2.

## Verdict

La v2 établit un principal technique et un périmètre de pièces ; elle ne définit pas encore une autorisation par personne.
Ajouter un identifiant utilisateur ne suffit pas : il faut établir sa provenance et conserver son autorité dans les délégations et les exécutions.
Des contre-exemples subsistent dans le banc : approbation hors périmètre, révocation avant remise au module, réutilisation d’une ancienne approbation et rejeu après reprise non sûre.
L’architecture reste viable sans abandonner MQTT direct, avec un contexte d’autorité commun, des permissions par classe d’ordre et une vérification finale effective.

Référence applicative : `main`, commit `a835893`. La proposition et le banc examinés sont ceux de la v2. Les contre-exemples ci-dessous résultent de la lecture du code ; aucun build ni test n’a été lancé.

« Bloquant » signifie ici : à résoudre avant de considérer le contrat proposé comme sûr. Cela ne signifie pas que le défaut est exploitable dans l’installation actuelle : plusieurs surfaces de la cible ne sont pas encore implémentées.

## Un modèle cohérent des droits

**Identité.** Le principal désigne la connexion authentifiée : tablette, application, satellite, bot, agent, moteur ou hub. La personne désigne éventuellement l’humain dont les droits sont engagés. Une tablette partagée, un bouton et un satellite vocal peuvent avoir un principal connu sans personne connue. Un PIN partagé prouve la connaissance du PIN ; il ne distingue pas les membres du foyer. Une identité déclarée ou une reconnaissance vocale probabiliste reste un indice.

**Décision.** Pour chaque cible physique et chaque opération effective :

```text
droits effectifs =
    droits du principal et du canal
  ∩ droits actuels de la personne, ou profil explicite « personne inconnue »
  ∩ portée du mandat utilisé
  ∩ contraintes de contexte
  ∩ exigences propres à l’ordre et à ses arguments
```

Les contraintes de contexte comprennent pièces, heure, validité, mode d’alarme et, lorsque la politique le demande, présence. La présence ne prouve pas l’identité. Une interdiction explicite ou une exigence métier ne peut pas être annulée par un autre rôle. L’origine `Ui`, `Automation` ou `System` ne constitue pas, seule, une permission.

**Trois résultats.** Refuser ; retenir pour une approbation prévue par la politique ; autoriser. Une approbation exige une personne habilitée à approuver **cette opération sur ces cibles**, avec les preuves requises. Elle porte sur une carte identifiée par une révision unique : arguments, forme, cibles, échéance et contexte d’autorité. Elle conserve demandeur et approbateur. Un enfant ne reçoit pas un droit d’ouverture parce qu’il sait cliquer sur une carte.

**Délégation.** Une exécution interactive conserve l’autorité de son demandeur ; passer par un assistant, une scène ou un automatisme ne l’élargit pas. Un automatisme autonome de la maison reçoit un mandat explicite, borné à une version du graphe, des opérations, des cibles et des déclencheurs. Son auteur, son approbateur, son mandat et sa cause restent distincts. Un bouton accessible à tous ne vaut pas identification du propriétaire.

**Durée.** Les droits sont revérifiés après une attente, avant chaque ordre et au dernier point d’autorisation précédant l’envoi. Révoquer annule les effets encore annulables ; cela ne reprend pas un effet physique déjà parti. Une ancienne génération des droits invalide un cache et impose une nouvelle décision, sans réécrire l’identité historique.

**Groupes et secours.** Toutes les feuilles sont autorisées avant le premier départ. Une cible initialement interdite fait refuser l’ensemble ; une révocation après un départ partiel arrête le reste et produit un bilan honnête. Le secours reprend les identités stables, politiques, mandats, révocations et contextes. Il refuse les opérations sensibles lorsque la fraîcheur de ces droits n’est pas établie.

**Domaines particuliers.** Le désarmement conserve son passage privé avec code ; ni un agent ni un automatisme ne l’obtient par approbation générique. Une urgence utilise un mandat interne précis associé à une source qualifiée, sans privilège « urgence » déclarable par un client. Lire l’état d’un appareil et lire l’identité de ses utilisateurs sont deux permissions distinctes.

## Qui est qui, aujourd’hui et dans la cible

La confiance indiquée porte sur ce que le logiciel établit, pas sur la certitude que le titulaire du compte tient effectivement l’appareil.

| Canal | Ce qui existe au commit de référence | Personne et confiance | Conséquence pour la v2 |
|---|---|---|---|
| **Tablette murale partagée** | Le dashboard peut obtenir `Household`, ou `Ui` avec session PIN : `crates/moli-api/src/rest.rs:75`. La session est liée à une clé d’appelant : `crates/moli-api/src/session.rs:154`. | Principal reconnaissable ; personne généralement inconnue. Le PIN commun ne distingue pas enfant, invité et propriétaire. | Le compte de tablette ne doit pas être associé implicitement à son installateur. Profil partagé limité, puis authentification personnelle pour les opérations qui l’exigent. |
| **Application ou navigateur personnel** | `Caller.person` existe. Il est établi par identité Access vérifiée ou connexion personnelle : `crates/moli-api/src/caller.rs:123`, `crates/moli-api/src/caller.rs:275`. | Personne liée à un compte authentifié, si cette connexion est présente. Le simple fait d’utiliser un téléphone ne suffit pas. | Lier la session personnelle au principal de l’application, sans confondre compte de l’appareil et session de la personne. |
| **Téléphone appairé** | Le jeton de remontée du téléphone ouvre une route spécifique ; le `Caller` ainsi créé a `person: None` : `crates/moli-api/src/caller.rs:287`. | Preuve du téléphone pour ses remontées, pas session générale de commande d’une personne. | Ne pas transformer ce jeton en droit général de commander. |
| **Voix : satellite ESPHome, Wyoming** | Le satellite appelle le cerveau avec la conversation : `crates/moli-esphome/src/satellite.rs:245`. `VoiceBrain::answer` ne reçoit pas d’identité du locuteur : `crates/moli-runtime/src/voice.rs:49`. Wyoming produit une transcription : `crates/moli-net/src/wyoming.rs:60`. | Appareil éventuellement authentifié ; locuteur inconnu. Je n’ai pas trouvé de preuve d’identité vocale dans ces chemins. | Principal du satellite, personne inconnue. Une reconnaissance probabiliste future peut personnaliser le dialogue, sans accorder seule des droits sensibles. |
| **Assistant depuis l’écran ou l’application** | La route transmet `Turn`, sans `Caller` : `crates/moli-api/src/assistant.rs:30`. `Turn` porte messages, surface et indicateur vocal : `crates/moli-assistant/src/lib.rs:260`. | Une personne peut être connue à l’entrée HTTP, mais cette information n’est pas transmise à l’exécution de l’assistant. | Transporter un contexte établi par le serveur jusque dans les outils et les commandes. |
| **Agent MCP** | `MoliMcp` conserve le hub, sans personne par invocation : `crates/moli-api/src/mcp.rs:19`. Les commandes prennent l’origine MCP : `crates/moli-api/src/mcp.rs:426`. | Principal agent ; aucune délégation personnelle démontrée par ce chemin. | Un agent agissant « pour Alice » doit présenter un mandat serveur borné. Le texte « pour Alice » n’accorde rien. |
| **Telegram, bot** | Le module examiné envoie des messages ; sa boucle est sortante : `crates/moli-telegram/src/lib.rs:182`, `crates/moli-telegram/src/lib.rs:201`. | Aucun canal entrant de commande avec identification de l’expéditeur n’est établi par ce code. | Si des commandes sont ajoutées : associer l’identifiant vérifié de l’expéditeur à une personne. L’identité du bot ou du groupe de discussion ne suffit pas. |
| **Boutons et télécommandes physiques** | Les événements viennent d’un appareil connu du module, notamment Zigbee : `crates/moli-z2m/src/lib.rs:165`. Une occurrence ne porte pas d’identité humaine : `crates/moli-core/src/occurrence.rs:30`. | Source physique connue ; personne inconnue. Posséder ou avoir installé le bouton ne prouve pas qui appuie. | Conserver la source et appliquer un mandat explicitement ouvert à ce déclencheur physique. |
| **Clavier d’alarme** | Chemin privé dédié : `crates/moli-runtime/src/keypad.rs:1`, `crates/moli-z2m/src/keypad.rs:19`. Vérification du code vers un profil : `crates/moli-alarm-access/src/verification.rs:133`. | Profil authentifié par code ; lien avec une personne Moli optionnel : `crates/moli-alarm-access/src/model.rs:52`. | Conserver source, profil et résultat d’autorisation ; jamais le code dans une demande générique persistée. |
| **Automatisme** | Exécution sous `Origin::Automation`, avec le nom de l’automatisme : `crates/moli-automation/src/run.rs:523`. `Author` distingue humain, assistant, agent et import, sans identifier une personne : `crates/moli-automation/src/model.rs:99`. | Programme validé, mais pas identité personnelle complète de son auteur ni du déclencheur. | Séparer exécution interactive déléguée et automatisme autonome mandaté par la maison. |
| **Scène** | Le moteur de scène compose déjà son permis avec celui du parent : `crates/moli-runtime/src/scene_execution.rs:357`. | L’autorité du demandeur peut être conservée par le permis parent. | Réutiliser ce principe pour toutes les cibles, avec le contexte personnel et la génération des droits. |
| **Tâche planifiée de l’assistant** | Des récapitulatifs internes programmés existent : `crates/moli-assistant/src/lib.rs:776`. Cela ne démontre pas un ordonnanceur général de tâches personnelles. | Principal assistant ; personne bénéficiaire et autorité ne sont pas interchangeables. | Une future tâche personnelle doit conserver son mandat et son échéance, sans s’exécuter sous les droits généraux de l’assistant. |
| **Invité** | Personne et périmètre existent ; permis vivant : `crates/moli-api/src/people.rs:623`. Les invités sont exclus de l’assistant et de MCP : `crates/moli-api/src/caller.rs:324`. | Personne connue lorsqu’authentifiée, avec droits temporaires. | Conserver ces restrictions. La v2 ne peut pas remplacer cela par « mêmes pièces, origine UI ». |
| **Hub, moteur interne** | Principaux internes dans le banc : `tools/ordres-banc/src/hub.rs:207`. | Service connu ; aucune personne implicite. | Chaque privilège doit être associé à une fonction ou un mandat, pas au seul caractère interne. |

## Constats sur les droits

### D1 — Bloquant — La connexion est assimilée à une personne (§ 11)

Le § 11 déduit de la connexion « la personne qu’il représente », sans traiter les appareils partagés ni les changements de session personnelle. L’identifiant de personne doit provenir d’une preuve distincte, ou rester absent.

**Preuve.** `Principal` n’a qu’un `qui` fixe : `tools/ordres-banc/src/hub.rs:185`. Aujourd’hui, `Caller.person` est explicitement optionnel : `crates/moli-api/src/caller.rs:123`. La session PIN n’est pas une session nominative : `crates/moli-api/src/session.rs:154`.

La mention d’un compte MQTT authentifié par le jeton du PIN « comme aujourd’hui » est également trompeuse : le chemin actuel crée une session HTTP, pas ce compte MQTT, dans `crates/moli-api/src/rest.rs:565`.

**Correction.** Distinguer principal, session personnelle et preuve d’identité. La tablette garde son principal lorsque l’utilisateur change ; les nouveaux ordres prennent le contexte personnel courant. À la déconnexion, les données personnelles affichées et abonnements correspondants sont invalidés.

### D2 — Bloquant — Le permis par pièces n’est pas un droit sur les ordres ; l’invité contourne la garde (§§ 11–12)

La permission actuelle du banc autorise toutes les opérations d’une pièce, sans classe d’ordre ni distinction enfant/adulte. En outre, l’invité reçoit `UI`, ce qui lui évite la retenue appliquée aux agents.

**Preuve.** `permis` vérifie seulement existence, expiration et pièces : `tools/ordres-banc/src/hub.rs:791`. `Principal::invite` donne l’origine `UI` : `tools/ordres-banc/src/hub.rs:210`. La garde n’est appliquée que si `origine::agent(p.origine)` : `tools/ordres-banc/src/hub.rs:901`.

Scénario : inviter quelqu’un dans l’entrée pour commander son éclairage lui permet aussi de demander directement l’ouverture d’une serrure déclarée dans cette pièce.

Le code actuel distingue pourtant le contrôle d’une machine et l’ouverture : `crates/moli-runtime/src/hub.rs:1116`. La sensibilité dépend aussi des arguments : verrouiller et déverrouiller ne sont pas équivalents, `crates/moli-core/src/spec.rs:248`.

**Correction.** Ajouter des permissions par classe d’opération, puis appliquer garde et preuves requises. Prévoir un profil enfant ou des restrictions personnelles équivalentes. Classer l’opération avec ses arguments ; une origine UI ne supprime aucune restriction personnelle.

### D3 — Bloquant — L’approbateur n’a pas à être autorisé sur l’opération (§§ 11–12)

Pouvoir décider d’une retenue est un booléen global. Un humain limité à une pièce peut approuver une demande concernant une autre pièce ; aucun contrôle ne vérifie son droit sur l’opération.

**Preuve.** `decider` vérifie seulement `p.decide` et l’expiration du décideur : `tools/ordres-banc/src/hub.rs:1125`. Le permis ensuite revérifié est celui du demandeur, `d.de`, à la ligne 1152.

Le chemin actuel des approbations exige lui aussi une session PIN, sans contrôle de propriétaire à cet endroit : `crates/moli-api/src/rest.rs:254`. Il existe cependant un meilleur modèle pour l’administration de l’alarme : propriétaire nominatif **et** session humaine, avec contrôle vivant, `crates/moli-api/src/alarm_access.rs:44`.

**Correction.** Définir `approuver(classe, cibles)` séparément de `commander`. L’approbateur doit satisfaire les exigences de l’acte autorisé. Par défaut, un enfant et une tablette sans personne identifiée n’approuvent pas une ouverture de porte.

### D4 — Bloquant — Une ancienne approbation peut accepter une carte renouvelée (§ 12.3)

L’approbation nomme l’ordre et la forme, mais aucune révision unique de la carte. Si seules les cibles changent, une répétition de l’ancien clic peut approuver les nouvelles cibles.

**Preuve.** `Carte` contient texte, forme et cibles, sans révision : `tools/ordres-banc/src/hub.rs:311`. `decider` compare la forme vue à la forme de la carte : ligne 1136.

Scénario :

1. L’écran montre les cibles A, sous la forme F.
2. La composition devient B, toujours sous F.
3. Le premier ancien clic provoque une nouvelle carte B : lignes 1161–1180.
4. Une répétition du même clic contient toujours le bon ordre et F. La carte courante correspond désormais à B : elle est acceptée.

**Correction.** Donner à chaque carte une révision ou un jeton unique, retourné obligatoirement lors de la décision. Lier ce jeton aux arguments, cibles, forme et autorité ; consommer la décision une seule fois.

### D5 — Bloquant — Assistant et MCP ne transmettent pas l’autorité personnelle (§ 11 et intégration des surfaces)

Une personne connue à l’entrée peut devenir un simple appel à un service général. Ajouter son identifiant au journal ne corrige pas cette perte d’autorité.

**Preuve.** La route de l’assistant ne transmet pas `Caller` : `crates/moli-api/src/assistant.rs:30`. Son inventaire vient du snapshot complet : `crates/moli-assistant/src/lib.rs:1139`. Ses commandes passent sous `Origin::Assistant`, acteur « Moli », sans permis personnel : `crates/moli-assistant/src/lib.rs:1236`. Le serveur MCP ne conserve pas de personne par invocation : `crates/moli-api/src/mcp.rs:19`.

Cela ne démontre pas un accès actuel des invités : ils sont bloqués en amont sur ces deux surfaces.

**Correction.** Transmettre un contexte serveur aux lectures et aux effets des outils. Filtrer inventaire, recherches et journal selon ce contexte. Un agent agissant pour une personne conserve les plafonds de cette personne et de son propre canal ; le LLM ne choisit ni identité ni mandat.

### D6 — Bloquant — Une révocation peut laisser partir une commande encore dans le hub (§ 11)

La révocation annule les commandes présentes dans les voies, mais pas celles qui attendent encore leur remise au kit. La vérification finale annoncée n’est pas réalisée.

**Preuve.** `fin_de_permis` ajoute des références à `annulations`, sans retirer `hub.cmd` : `tools/ordres-banc/src/hub.rs:824`. La propagation traite d’abord les annulations, puis remet les commandes au kit : `tools/ordres-banc/src/soiree.rs:555`. Le kit vérifie l’existence de la demande et son délai, pas le permis courant : `tools/ordres-banc/src/module.rs:189`.

Scénario : admettre une commande ; révoquer avant propagation ; annuler une voie encore vide ; remettre ensuite la commande ; l’envoyer avant son ancien délai.

**Correction.** Invalider toutes les étapes précédant l’envoi, y compris `hub.cmd` et les libérations après acquittement. Au dernier point d’envoi, vérifier un permis vivant ou une autorisation équivalente sérialisée avec les révocations. Le code actuel fournit déjà ce principe : `crates/moli-runtime/src/hub.rs:1798` et `crates/moli-api/src/people.rs:623`.

### D7 — Important — L’expiration d’un permis n’est pas une échéance de réveil (§§ 11–12)

La fin d’un permis est traitée par `tic`, mais ne participe pas au calcul du prochain réveil. L’expiration à la seconde montrée dans la soirée dépend d’un réveil périodique ajouté par le scénario.

**Preuve.** `Hub::prochaine` ne considère que demandes et attente du secours : `tools/ordres-banc/src/hub.rs:1422`. Les permis expirés sont balayés à la ligne 1451. Une retenue reçoit dix minutes, indépendamment d’une expiration personnelle plus proche. La soirée impose `.min(self.mur + 1_000)` : `tools/ordres-banc/src/soiree.rs:663`.

Scénario sans autre événement : retenue à 18 h 39, permis expirant à 18 h 40, retenue échéant à 18 h 49. Le réveil propre au hub peut attendre 18 h 49.

**Correction.** Inclure les fins de permis dans les échéances ; borner également la durée de conservation des retenues. Éprouver ce contrat sans battement périodique de secours.

### D8 — Important — Voir une pièce donne trop d’informations ; l’audit nominatif n’a pas de politique (§§ 13 et 16)

La v2 ne sépare pas visibilité de l’état, des retenues et des auteurs. Un filtrage exclusivement par pièce ne conserve pas les protections actuelles des invités et ne répond pas à la confidentialité entre membres du foyer.

**Preuve.** Le contenu client est sélectionné par `voit(piece)` : `tools/ordres-banc/src/hub.rs:2047`. Les retenues le sont aussi selon la pièce de leur appareil : ligne 2081. Aujourd’hui, téléphones, personnes et présence sont exclus des invités indépendamment de leur pièce : `crates/moli-api/src/people.rs:59`. Le SSE retire leur journal et les notices du foyer : `crates/moli-api/src/sse.rs:190`.

**Correction.** Distinguer lecture d’état, lecture d’audit personnel, lecture d’audit du foyer et lecture des cartes à approuver. Par défaut : état commun sans identité nominative inutile ; chacun voit ses actes ; audit de sécurité réservé aux personnes habilitées ; aucun journal général pour les invités.

Ces projections doivent aussi avoir une continuité de flux cohérente : un rang de pièce ne suffit plus si certains événements de cette même pièce sont volontairement masqués.

### D9 — Bloquant — Le contrat générique ne suffit pas pour l’alarme (§§ 11–12 et formes sensibles)

Les drapeaux « sensible » et « texte privé » n’expriment pas les interdictions métier de l’alarme. Une demande retenue puis approuvée ne doit jamais devenir un moyen indirect de désarmer.

**Preuve.** Le contrat impose toujours le code pour désarmer ; les automatismes ne peuvent qu’armer ; les agents ne peuvent même pas demander un désarmement retenu : `docs/components/alarme.md:243`, `docs/components/alarme.md:253`. Le moteur refuse explicitement `Source::Agent | Source::Automation` : `crates/moli-alarm/src/engine/commands.rs:206`.

La v2 conserve et réplique les textes privés des demandes vivantes : `tools/ordres-banc/src/hub.rs:1772`. Un code d’alarme ne doit pas suivre ce chemin ; le code actuel possède une entrée privée dédiée, `crates/moli-runtime/src/keypad.rs:1`.

**Correction.** Ajouter des exigences métier non contournables : source autorisée, preuve spécifique et interdiction de délégation. Le code est consommé par le chemin privé de l’alarme ; le hub générique ne conserve qu’un résultat d’autorisation borné, jamais le secret. La politique d’urgence doit rester distincte de ce mécanisme.

## Constats sur la mémoire

### M1 — Bloquant — Les principaux sont des positions de configuration, pas des identités stables (§§ 11 et 17)

Une demande, un reçu et une révocation désignent une position dans un `Vec`. Réordonner les comptes au redémarrage peut attribuer une ancienne révocation ou une ancienne demande à quelqu’un d’autre.

**Preuve.** L’ajout retourne `len - 1` : `tools/ordres-banc/src/hub.rs:478`. L’instantané ne garde que les expirations dans cet ordre : ligne 1785. La relecture les applique aux positions de la nouvelle configuration : ligne 1856.

Scénario : permuter un invité révoqué et un propriétaire dans la configuration. L’expiration frappe le propriétaire ; l’invité peut perdre sa révocation.

Le cycle de vie pose aussi une borne cumulative : les principaux expirés ne sont pas récupérés par ces chemins, et le 257ᵉ ajout déclenche un `assert!`.

**Correction.** Utiliser des identifiants stables avec génération ou non-réutilisation garantie ; relire par identifiant, jamais par position. Définir le retrait des comptes temporaires, de leurs reçus et de leurs références historiques. Dépasser le plafond doit produire un refus contrôlé.

### M2 — Important — `qui` confond plusieurs identités et le journal perd l’approbateur (§§ 12–13)

`qui` représente alternativement personne, agent ou exécution. Il ne permet pas de conserver simultanément demandeur, personne représentée, déclencheur et décideur.

**Preuve.** Le mélange est explicite dans `tools/ordres-banc/src/demandes.rs:113`. `decider` reçoit un décideur mais ne le transmet pas au traçage : `tools/ordres-banc/src/hub.rs:1193`. La ligne de décision reprend `d.qui`, `d.de` et `d.origine` : ligne 1284.

Une décision humaine est donc journalisée avec l’identité du demandeur initial. Le code actuel conserve déjà un acteur pour la résolution d’approbation : `crates/moli-runtime/src/hub.rs:1440`.

**Correction.** Introduire une référence vers un contexte d’autorité immuable et distinguer l’acteur de l’événement du contexte de la demande. Conserver séparément la référence d’exécution. Le chiffrage proposé plus bas évite de grossir systématiquement les fiches.

### M3 — Important — La réplication des droits reste à raccorder au contrat HA (§ 17)

La HA v2 prévoit déjà `conf/` et la réplication des sessions ; il ne manque donc pas un second protocole. En revanche, le banc des ordres ne démontre que la réplication d’une expiration, sans contrat complet pour personnes, rôles, périmètres et mandats.

**Preuve.** Sa seule sortie de droits est `Revocation { de, fin_mur_ms }` : `tools/ordres-banc/src/hub.rs:339`. L’instantané suppose les autres permissions dans la configuration : ligne 1800. La HA prévoit les fichiers métier critiques et les sessions : `docs/proposition/haute-disponibilite-v2.md:511`, `docs/proposition/haute-disponibilite-v2.md:689`.

Scénario à couvrir : retirer une permission d’ouverture, perdre l’actif avant application sur le pair, puis approuver une retenue sur ce pair.

**Correction.** Faire passer annuaire stable, politiques, générations, mandats et révocations dans le rang critique existant. Définir quand une révocation est réputée acquise et ce qu’un secours insuffisamment frais refuse. Les caches, le courtier et le moteur doivent observer la même révision effective.

### M4 — Important — Le budget n’inclut pas encore le modèle demandé ; la fiche d’exécution de 64 o doit changer (§ 19 et mémoire v2 § 8)

Les chiffres actuels du banc ne mesurent pas une autorisation personnelle complète. En outre, le format d’exécution de la mémoire v2 conserve une référence de demande sur 4 octets, incompatible avec les références de 8 octets de la v2 des ordres.

**Preuve.** La fiche de 64 o réserve 4 o à `demande` : `docs/proposition/memoire-v2.md:704`. Le moteur du banc utilise désormais `u64` : `tools/ordres-banc/src/moteur.rs:156`.

La borne annoncée de 65 Kio de reçus correspond à 64 principaux, tandis que le banc en accepte 256 : `docs/proposition/ordres-appareils-v2.md:1180`, `tools/ordres-banc/src/hub.rs:45`. Avec les mêmes hypothèses de disposition, cela représente **260 Kio**, hors coût de l’allocateur.

**Correction.** Publier un budget incluant autorité, cycle de vie des contextes et exécutions corrigées. Conserver la réserve déjà formulée au § 19 : le budget intégré de 32 Mio n’est pas encore vérifié.

### M5 — Important — Le compteur d’ordres actifs sature avant le plafond global (§ 13)

Le compteur sur 8 bits peut annoncer zéro alors qu’une demande de l’ordre reste vivante. Cela fausse `ENCOURS`.

**Preuve.** `actives: u8` : `tools/ordres-banc/src/catalogue.rs:217`. Incrément et décrément saturants : `tools/ordres-banc/src/hub.rs:975`, `tools/ordres-banc/src/hub.rs:1264`. Le plafond de 4 096 demandes n’impose pas de borne de 255 par ordre.

Calcul : 256 admissions donnent 255 ; 255 conclusions donnent zéro, avec une demande restante.

**Correction.** Passer le compteur à `u16`, en réorganisant l’octet réservé pour conserver une fiche de 40 o, ou imposer explicitement un plafond par ordre avant toute admission. Ne pas utiliser une saturation silencieuse pour un compteur de vérité.

### M6 — Important — L’empreinte des arguments ne garantit pas le conflit annoncé (§ 12.5)

Des arguments différents ont exactement la même entrée de hachage, avant même de considérer les collisions du résultat sur 32 bits.

**Preuve.** Les textes sont concaténés sans type ni longueur : `tools/ordres-banc/src/hub.rs:465`.

```text
[T("ab"), T("c")] → octets "abc"
[T("a"), T("bc")] → octets "abc"
```

Le reçu ne compare ensuite que l’ordre et cette empreinte : `tools/ordres-banc/src/demandes.rs:375`.

**Correction.** Encoder canoniquement nombre, types, longueurs et valeurs, avec l’identité logique représentée lorsqu’elle fait partie de la demande. Employer une empreinte suffisamment forte ou une comparaison exacte. Une empreinte de 128 bits impose de recalculer le reçu de 32 o ; ce changement n’est pas gratuit.

### M7 — Bloquant — Une répétition de la première tentative contourne la reprise non sûre (§§ 12.6 et 17.2)

La protection dépend d’un compteur de tentative déclaré par le client. Une duplication identique du premier message conserve `tentative = 1` et échappe à la protection.

**Preuve.** Le refus exige `r.tentative > 1` : `tools/ordres-banc/src/hub.rs:891`.

Scénario :

1. Un appui part sans secours ; le dernier instantané le précède.
2. Le hub redémarre depuis cet instantané : son reçu est absent.
3. Le transport ou le client répète le message original, inchangé.
4. Il est considéré neuf et peut repartir, malgré une première date antérieure à la reprise.

**Correction.** Identifier les demandes par une session ou une époque d’émission établie avec le serveur. Après reprise non sûre, une demande d’appui inconnue issue d’un ancien contexte d’émission reste ambiguë, même déclarée « première tentative ». L’horloge et le compteur du client ne doivent pas constituer la preuve de non-exécution.

### M8 — Important — Plusieurs textes privés s’écrasent (§ 12.7)

Le contrat permet plusieurs paramètres, mais le tiroir privé ne garde qu’un texte par demande. Une forme avec deux paramètres textuels privés perd leur distinction.

**Preuve.** La boucle remplace chaque texte par sa longueur et écrase `prive` avec le dernier : `tools/ordres-banc/src/hub.rs:915`. `args_complets` remplace ensuite tous les paramètres privés par ce même texte : ligne 1088.

**Correction.** Pour la première version, refuser explicitement les formes comportant plusieurs textes privés. Si le besoin existe, stocker les contenus par position de paramètre et chiffrer ce tiroir. Ne pas ajouter cette généralité sans usage concret.

## Ce qu’il faut ranger, et combien

Les tailles suivantes sont des **propositions de disposition et des calculs**, pas des mesures.

### Un contexte partagé plutôt que tous les champs dans chaque demande

Un contexte d’autorité de **32 o** peut contenir :

| Champ | Octets | Sens |
|---|---:|---|
| Cause ou déclencheur stable | 8 | Référence d’occurrence, demande source ou cause d’exécution |
| Principal stable | 4 | Connexion responsable |
| Personne | 4 | Zéro si inconnue |
| Mandat ou délégation | 4 | Porte notamment le « au nom de » autorisé et son périmètre |
| Génération des droits | 4 | Révision observée, à comparer aux droits courants |
| Référence de preuve | 4 | Preuve établie par le serveur, sans secret brut |
| Canal | 1 | Écran, voix, MCP, physique, moteur… |
| Nature de la preuve | 1 | Inconnue, compte personnel, preuve métier… |
| Drapeaux | 2 | Propriétés bornées du contexte |
| **Total** | **32** | |

La « confiance » doit d’abord être une catégorie de preuve. Un score vocal probabiliste peut rester une donnée diagnostique séparée ; il ne doit pas devenir un coefficient qui accorde automatiquement un droit.

L’auteur et l’approbateur d’un programme, son empreinte et son mandat vivent dans la définition de programme. La personne à l’origine d’une exécution et son déclencheur vivent dans le contexte et la cause de cette exécution. Un bénéficiaire annoncé par un client n’est pas une personne authentifiée.

### Effet sur les fiches

| Structure | Proposition | Taille calculée |
|---|---|---:|
| **Forme** | Utiliser les trois octets réservés pour classe, sensibilité et exigence de preuve. Les inclure dans l’égalité des formes. | **24 o**, inchangée |
| **Demande** | Remplacer `_s[4]` par une référence de contexte. Conserver une référence d’exécution distincte de la personne. | **72 o**, inchangée |
| **Dernière commande** | Remplacer le `qui` ambigu par une référence de contexte. | **40 o**, inchangée |
| **Ligne de journal** | Deux références de contexte : demande initiale et acteur de l’événement. Retirer les copies de principal et d’origine résolubles par ces contextes. | **48 o**, après réorganisation |
| **Exécution mémoire v2** | Référence de demande sur 8 o et contexte sur 4 o. En réordonnant les champs, le contexte occupe le remplissage introduit avant les champs sur 8 o. | **72 o**, contre 64 o annoncés |

Les réserves existent dans `tools/ordres-banc/src/catalogue.rs:116` et `tools/ordres-banc/src/demandes.rs:135`. Pour la ligne de journal, une disposition possible est : trois champs de 8 o, cinq champs de 4 o, puis quatre codes de 1 o, soit 48 o.

**La référence compacte ne supprime pas le coût.** Un contexte doit survivre aux demandes, dernières commandes, lignes de journal, exécutions et baux qui le citent. Il faut un retrait contrôlé ; libérer à la conclusion crée des références pendantes, ne jamais libérer crée une fuite. Sa réutilisation doit aussi respecter les références du secours.

À titre de borne illustrative, 4 096 contextes de demandes, 802 dernières commandes et deux références distinctes pour chacune des 500 lignes représentent jusqu’à :

```text
(4 096 + 802 + 2 × 500) × 32 = 188 736 o ≈ 184,3 Kio
```

Les partages réduisent ce nombre ; exécutions, baux et données annexes peuvent l’augmenter. Ce calcul n’est ni une consommation mesurée ni une borne complète.

### Tables de politique

Exemple de dimensionnement : 32 personnes, 64 principaux, 8 rôles, 32 classes d’ordres, 128 pièces et 512 mandats.

| Élément | Hypothèse de disposition | Budget calculé |
|---|---|---:|
| Personnes | 32 × 32 o, hors noms et données d’authentification | 1 Kio |
| Principaux | 64 × 32 o, hors noms et secrets | 2 Kio |
| Périmètres de pièces | 96 ensembles de 128 bits | 1,5 Kio |
| Rôles × classes | 8 × 32 × 6 permissions en bits | 192 o |
| Mandats bornés | 512 × 64 o | 32 Kio |
| Révision des droits | Compteur persistant sur 4 o | 4 o |
| **Sous-total indicatif** | Hors contextes, index, textes, preuves annexes et allocateur | **≈ 36,7 Kio** |

Les six permissions peuvent couvrir lire l’état, proposer, commander, approuver, lire l’audit et administrer. Les contraintes temporelles, les exceptions personnelles et les règles métier restent des contrôles supplémentaires.

**Ce qui passe au secours :** identifiants et correspondances stables, personnes et rôles utiles à l’autorisation, politiques et générations, mandats et révocations, contextes référencés, approbations et révisions des cartes, autorité des exécutions et des baux. Les sessions suivent le mécanisme protégé prévu par la HA. Aucun code d’alarme brut, enregistrement vocal ou secret de connexion ne doit être ajouté au journal générique des ordres.

## Constats sur les automatismes

### W1 — Bloquant — Un déclencheur humain peut acquérir les droits généraux du moteur (§ 15 et mémoire v2 § 8)

Le moteur utilise son principal interne pour toutes les exécutions. Il ne distingue pas une exécution déléguée par une personne d’un automatisme autonome mandaté par le foyer.

**Preuve.** Le principal est un champ commun du moteur : `tools/ordres-banc/src/moteur.rs:175`. Une commande utilise `self.de` et remplace `qui` par la référence d’exécution : ligne 407. La trace reçue ne transmet pas d’autorité personnelle : ligne 306.

Le code actuel exécute aussi les automatismes sous une origine dispensée de garde : `crates/moli-automation/src/run.rs:523`, `crates/moli-runtime/src/guard.rs:133`.

Scénario cible : un invité déclenche un bouton ou un script qui lance « ouvrir le garage ». Si l’exécution prend les droits du moteur, le refus de l’ordre direct n’a plus d’effet.

**Correction.** Deux contrats explicites :

- exécution interactive : droits du demandeur, du canal et du mandat conservés jusqu’au dernier effet ;
- automatisme autonome : mandat du foyer, validé avec les déclencheurs autorisés.

Autoriser un bouton anonyme à ouvrir est possible, mais doit être une décision explicite sur ce bouton accessible à tous. La condition `VIENT_DE` ne prouve pas l’identité de celui qui a causé une variation.

### W2 — Important — La validation du graphe n’est pas une validation des droits de son responsable (§ 15)

Vérifier types et cibles ne garantit pas que le responsable puisse lire les entrées et autoriser les sorties. Le modèle d’auteur actuel ne permet pas non plus de réagir à une modification de ses droits personnels.

**Preuve.** `Def` ne porte ni personne responsable ni mandat : `tools/ordres-banc/src/moteur.rs:113`. La compilation valide le catalogue et les paramètres, sans contexte d’autorisation : ligne 182. Dans le code actuel, `Author` est une catégorie, pas une identité : `crates/moli-automation/src/model.rs:99`.

L’approbation exacte d’une version existe déjà : `crates/moli-automation/src/lib.rs:558`. Il faut la prolonger, pas la remplacer.

**Correction.** À la validation, calculer un manifeste des lectures, commandes, scènes, baux et déclencheurs. Le validateur nominatif doit pouvoir accorder ce manifeste. Un agent peut rédiger un brouillon ; cela ne crée aucun mandat.

Par défaut, une délégation personnelle cesse lorsque ses droits disparaissent. Un automatisme peut survivre au départ de son auteur après adoption explicite par le foyer, avec un mandat et un responsable distincts.

### W3 — Important — Les baux créent un effet futur sans conserver son autorité (§§ 15 et 17)

Le bail est ouvert avant le résultat de la commande initiale, puis son rétablissement utilise les droits généraux du moteur. Il peut donc produire un effet après un refus initial ou après la perte des droits du demandeur.

**Preuve.** Le bail est enregistré avant `hub.demander` : `tools/ordres-banc/src/moteur.rs:390`. Au terme, il est fermé puis la restauration est demandée avec `self.de`, en ignorant le résultat : ligne 439.

Scénario : une commande temporaire est refusée ; le bail reste vivant. Entre-temps, un humain change l’état. Au terme, le moteur rétablit tout de même la valeur ancienne.

**Correction.** N’activer le bail qu’au niveau de réussite explicitement requis. Conserver contexte, commande initiale et condition de propriété de l’état temporaire. La restauration est une nouvelle commande, autorisée à son terme ; une révocation ne crée pas un droit automatique de compensation. Les scènes possèdent déjà une composition de permis parent exploitable : `crates/moli-runtime/src/scene_execution.rs:357`.

### W4 — Important — Une exécution persistée peut reprendre dans un autre programme (§ 17)

La persistance du moteur utilise un rang de programme et un curseur, sans identité de version suffisante. Une modification ou un réordonnancement des définitions peut redonner un autre sens à ces nombres.

**Preuve.** `Execution.auto` est un index : `tools/ordres-banc/src/moteur.rs:153`. `compiler` reconstruit `autos` depuis l’ordre des définitions : ligne 182. L’instantané conserve cet index et le curseur : ligne 519. La reprise les utilise directement : ligne 365.

Scénario : une exécution attend dans le programme d’index 0 ; les définitions A et B sont permutées ; elle reprend au même curseur dans B.

**Correction.** Référencer une activation stable liée à l’empreinte du programme. Conserver la version nécessaire aux exécutions vivantes, ou arrêter celles dont la logique a changé. C’est déjà le contrat de `docs/proposition/memoire-v2.md:659` et le principe des lignées d’annulation actuelles, `crates/moli-automation/src/run.rs:128`.

### W5 — Important — La bascule du banc n’éprouve pas la réplication des exécutions (§ 17)

Le redémarrage sérialisé du moteur ne prouve pas sa reprise sur le secours. Les transitions d’exécution ne sont pas émises dans le banc.

**Preuve.** `Moteur::sortir` est vide : `tools/ordres-banc/src/moteur.rs:428`. La copie n’applique que les baux : ligne 456. La propagation transmet ces sorties au moteur du secours : `tools/ordres-banc/src/soiree.rs:591`. La bascule active ensuite ce moteur, sans lui charger l’instantané des exécutions de l’ancien actif : ligne 786.

Scénario : A commande une première cible, attend son issue, puis doit commander B. Le secours reçoit les traces de la demande, mais pas l’exécution attendant cette issue.

**Correction.** Raccorder les transitions au flux `wf/` existant et ajouter une contre-épreuve de bascule entre deux étapes. Le commentaire du banc reconnaît cette réduction ; les conclusions de la soirée doivent donc la conserver comme limite. La V7 prévoit bien la reprise des exécutions : `docs/proposition/isolation-reactivite-runtime-v7.md:82`.

### W6 — Important — Les nouveaux déclencheurs sur ordres peuvent produire des rafales sans borne d’exécution (§ 15)

Le moteur réduit peut relancer un automatisme à chaque trace sélectionnée, sans quota d’exécution ni protection démontrée contre les boucles entre automatismes. Le plafond de demandes ne constitue pas un budget de travail du hub.

**Preuve.** Chaque trace correspondante appelle `lancer` : `tools/ordres-banc/src/moteur.rs:325`. `lancer` agrandit les exécutions au besoin : ligne 347. La propagation draine les sorties dans une boucle : `tools/ordres-banc/src/soiree.rs:551`.

Scénario : A réagit à un ordre X en demandant Y ; B réagit à Y en demandant X. Les traces d’admission peuvent entretenir la chaîne avant même les résultats physiques.

Le système actuel prévoit déjà 20 exécutions par minute et une file de 10 : `docs/components/automatismes.md:28`.

**Correction.** Conserver ces limites et définir précisément l’événement déclencheur : transition unique d’une demande identifiée, pas chaque republication de son état. Ajouter un budget de progression par tour du moteur pour préserver la réactivité de la V7.

## Ce qui tient et ce qu’il faut réutiliser

Le principal déduit de la connexion, les contrôles sur toutes les pièces d’un groupe et les reçus par principal sont de bonnes bases. Ils évitent de faire confiance à une origine déclarée dans le contenu.

Les retenues persistantes peuvent rester des demandes à réautoriser. La séparation des textes privés est utile, sous réserve de son domaine d’emploi et de la correction M8. Les traces peuvent rester dans `ord/`, avec des projections de confidentialité distinctes.

Le code actuel fournit déjà plusieurs mécanismes à prolonger :

- `Caller.person`, pour une identité personnelle optionnelle ;
- `CommandPermit`, pour une permission vivante jusqu’à l’envoi ;
- les permis parents des scènes ;
- l’approbation liée à l’empreinte du graphe et ses lignées d’annulation ;
- le chemin privé et les règles propres à l’alarme.

## Les alternatives plus simples

1. **Quelques classes fixes, des ensembles de pièces en bits et des mandats bornés.** Cela suffit pour les premiers rôles foyer, enfant et invité. Un langage général de politiques ajouterait beaucoup de cas difficiles à vérifier.

2. **Un seul contexte transmis de bout en bout.** Éviter un modèle de droits distinct dans REST, MCP, assistant, MQTT et moteur. Chaque adaptateur établit le contexte ; le hub décide ; chaque effet conserve le même plafond d’autorité.

3. **Séparer état commun et audit nominatif dans `ord/`.** Cela préserve MQTT direct et évite de publier toutes les identités à tous les lecteurs d’une pièce.

4. **Réduire explicitement les fonctions non nécessaires.** Pour commencer : une seule chaîne privée par forme ; aucune reconnaissance vocale accordant des droits sensibles ; aucune compensation automatique privilégiée ; aucun mandat implicite créé par le nom d’un script.

## Questions à poser au propriétaire

Les recommandations ci-dessous servent d’hypothèses pour cette revue ; aucune réponse n’est nécessaire pour en appliquer les corrections techniques.

| Sujet | Options | Recommandation |
|---|---|---|
| **Tablette partagée** | A. Profil commun limité, puis connexion personnelle pour les actes sensibles. B. Connexion personnelle pour toute commande. C. Tablette assimilée à une personne fixe. | **A** : conserve l’usage familial sans attribuer tous les gestes au propriétaire de la tablette. |
| **Voix et identité du locuteur** | A. Locuteur inconnu ; confirmation personnelle sur un autre canal pour les actes sensibles. B. Identification probabiliste pour personnaliser et commander le confort. C. Identification vocale suffisante pour les actes sensibles. | **A**, avec B éventuellement pour le confort. La reconnaissance vocale seule ne donne pas le droit d’ouverture ou d’approbation. |
| **Enfants** | A. Profil nominatif de confort avec exceptions explicites. B. Droits de membre ordinaires. C. Aucun compte personnel, seulement le profil partagé. | **A** : droits vérifiables et révocables, sans faire du PIN commun un droit d’adulte. |
| **Automatismes et départ de l’auteur** | A. Délégation personnelle par défaut ; adoption explicite possible par le foyer. B. Toujours les droits actuels de l’auteur. C. Tout automatisme validé appartient immédiatement au foyer. | **A** : évite les privilèges résiduels tout en permettant aux automatismes domestiques adoptés de continuer. |
| **Approbation d’un ordre sensible** | A. Personne nominative habilitée sur l’opération et les cibles. B. Propriétaires uniquement. C. Tout détenteur du PIN. | **A**, avec propriétaires comme configuration initiale pour les ouvertures. C est insuffisant. |
| **Qui voit qui a fait quoi ?** | A. État commun, actes personnels visibles à leur auteur, audit de sécurité réservé. B. Journal nominatif complet pour tous les adultes. C. Journal complet pour tous les membres. | **A**. Les droits d’audit doivent être affichés et configurables explicitement. |
| **Groupe contenant une cible interdite** | A. Refus initial de tout le groupe. B. Exécution automatique du sous-ensemble autorisé. C. Choix explicite entre les deux modes. | **A** par défaut. Un mode partiel peut être ajouté s’il est demandé et annoncé avant l’action. |
| **Urgence incendie** | A. Actions internes précisément mandatées sur source qualifiée. B. Approbation humaine pour toute action. C. Contournement général des droits en mode urgence. | **A** : définir les effets attendus et leur source ; ne pas créer un privilège général ni modifier implicitement le contrat du désarmement. |

## Hypothèses et limites

- Une personne non établie reste inconnue ; je ne l’assimile ni à un propriétaire ni à un invité authentifié.
- Une personne reconnue par compte n’est pas nécessairement le locuteur ou l’utilisateur physique du moment.
- Les adaptateurs externes doivent construire eux-mêmes les champs internes : principal, origine, contexte, parent et référence d’exécution. Le banc ne démontre pas encore cette frontière réseau.
- Les règles d’alarme citées sont le contrat existant ; toutes ses surfaces et intégrations physiques ne sont pas livrées, comme l’indique `docs/components/alarme.md:14`.
- Je n’ai pas certifié de nouveau les 25 corrections de la première revue. Les constats portent sur les droits, les exécutions et les contre-exemples relevés dans cette lecture.

Non vérifié : aucun build, test, broker réel, matériel ou basculement exécuté ; aucun fichier modifié. Les tailles proposées sont calculées, et la sûreté ainsi que le budget du système intégré restent à démontrer.

**Bilan : 10 constats bloquants, 13 importants, 0 mineur.**