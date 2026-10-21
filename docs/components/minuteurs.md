# Aides minuteur (AUTO-H1) — contrat proposé

Statut : contrat de l'implémentation proposée à la revue AUTO-H1. Sa présence sur une
branche ne vaut pas publication ou activation. Socle AUTO-H2 approuvé (`18cff34`), avec le
transport des identités de requête, la garde et le fichier métier atomique d'AUTO-H4.
La publication du présent document ne change aucune règle d'exécution.

## But

Démarrer ou remplacer un minuteur, l'annuler, lire son état et le temps restant.
Restaurer une échéance après arrêt, puis livrer sa terminaison au moteur sans
déclencher deux fois la même génération. Les actions restent celles des seuls
graphes validés et actifs ; aucun LLM dans l'exécution, aucun appareil physique
nécessaire aux tests. La capture privée et les autres primitives restent hors périmètre.

## Modèle et limites proposés

- Aide `timer`, durée initiale configurable en secondes entières exactes, 300 par
  défaut. Une commande `start` porte une durée entière de 1 à 604 800 secondes ;
  `cancel` porte `null`. Aucun flottant, expression ou dépassement implicite.
- Points en lecture seule : `state` (`idle`, `running`, `cancelled`, `finished`),
  `remaining` (secondes entières, jamais négatives), `generation` et échéance UTC.
  Point d'occurrence en lecture seule `finished`, distinct des commandes.
- `start` alloue une nouvelle génération, remplace la précédente et son échéance.
  `cancel` invalide la génération courante et ses livraisons non admises. Les
  effets déjà confirmés d'un graphe ne sont pas annulés rétroactivement.
- Générations monotones de 1 à `MAX_OCCURRENCE_SEQUENCE`, sans retour à zéro.
  À épuisement, un nouveau démarrage est refusé ; l'annulation reste possible.
- Un propriétaire commun au hub pour toutes les instances ; au plus 128 minuteurs,
  un événement de terminaison en attente par minuteur, 64 clients de requêtes et
  32 reçus par client, fichier métier de 1 Mio. Aucun client ou minuteur persistant
  n'est évincé automatiquement. Refus visible à une limite atteinte.

Chaque minuteur dispose d'une source d'occurrence aléatoire stable, persistée avec
son état. L'identité d'expiration est `(source, génération)`, sans dépendre du nom,
de l'heure murale, d'un redémarrage ou d'une valeur de point dédupliquée.

La frontière d'éligibilité est le démarrage : les destinataires validés et actifs et
leurs lignées sont capturés à l'admission, après la garde, et persistés avec la
génération. Une validation ou réactivation ultérieure n'hérite pas de cette génération,
même si sa toute première livraison est tardive. Pour inclure un nouvel automatisme,
redémarrer explicitement le minuteur après sa validation.

## Temps et transitions

L'exécution utilise une échéance monotone ; les sauts de l'horloge murale ne changent
pas la durée déjà engagée. L'état durable conserve aussi la durée et une échéance
UTC en millisecondes, calculée sans débordement lors du démarrage.

Le restant visible arrondit la durée positive à la seconde supérieure et vaut zéro
à échéance. Il est une projection, pas une écriture du fichier métier chaque seconde.
Les mutations, annulations et expirations sont sérialisées par le propriétaire.

Au redémarrage, une échéance future reconstruit l'attente monotone depuis UTC ; une
échéance passée termine une seule fois sa génération. Il n'y a ni nouvelle génération
ni rattrapage de chaque seconde manquée. Si l'horloge restaurée ferait dépasser la
durée initiale, signaler une horloge incohérente, sans rallonger silencieusement le
minuteur. L'état n'est pas présenté comme confirmé tant que la reprise est incertaine ;
un nouveau démarrage ou une annulation explicite permet de résoudre cette situation.

Une expiration n'est confirmée qu'après commit `running → finished` et de son événement
en attente dans le même état métier. Une annulation ou un remplacement qui gagne la
course avant admission invalide l'ancienne livraison ; son identité ne peut pas être
réallouée. Une tâche d'une génération ancienne ne termine jamais la nouvelle.

## Persistance, reçus et arrêt

Un seul rédacteur possède `helper-timers.json` au niveau du hub. Format versionné,
validation intégrale et bornée à la lecture. Écriture entière en temporaire, `fsync`,
renommage atomique et synchronisation du répertoire avant acquittement/publication.
Aucun SQLite, `moli.db` ou stockage historique dans ce contrat.

Valeur, génération, échéance, événement en attente et reçu d'opération appartiennent
au même commit. Les états dérivés du hub ne sont jamais l'autorité à la reprise.
Sauvegarde et restauration emportent le fichier complet ; une sauvegarde ancienne
restaure aussi son horizon de déduplication, sans promettre de connaître le futur.

Une identité de requête est celle d'AUTO-H2. Le reçu associe compteur de séquence,
point et charge normalisée : réutiliser une identité avec une autre durée, opération
ou cible est un conflit. Un retry retrouve l'issue initiale sans redémarrer, annuler,
allonger ni republier quoi que ce soit. Les refus métier identifiés restent des refus
après modification de l'état. Les reçus trop anciens sont refusés, jamais réacceptés.
L'espace des reçus minuteur est distinct de celui du compteur et des boutons.

Avant engagement, revérifier abandon, échéance de commande et arrêt après attente du
propriétaire. Une fois l'écriture engagée, sa tâche suit le commit malgré la perte du
demandeur ou l'arrêt du pilote. Après renommage d'issue incertaine, clôturer mutations
et livraisons, rendre l'erreur visible et reprendre depuis le fichier validé après
redémarrage. Un fichier abîmé n'est ni réinitialisé ni remplacé silencieusement.

## Livraison durable au moteur

AUTO-H4 reste inchangé : appuis en direct, admission au plus une fois, pas de rejeu
automatique. Le minuteur ajoute explicitement une livraison avec rattrapage ; utiliser
seulement `press_button` ou `set_state` ne satisfait pas ce contrat.

1. Le propriétaire persiste la terminaison et son identité stable en attente.
2. Le moteur s'abonne et annonce sa disponibilité avant le premier drainage. Les
   expirations restaurées ne sont pas envoyées avant cette disponibilité. Sans moteur,
   l'état `finished` est lisible et l'événement reste en attente, sans boucle de diffusion.
3. La première diffusion dans un démarrage du hub attribue une seule enveloppe de bus
   (`stream`, `position`). Un retry dans le même démarrage réutilise cette enveloppe :
   il ne devient pas un événement postérieur à une nouvelle validation ou réactivation.
   Après redémarrage, l'enveloppe est nouvelle, l'identité métier reste la même.
4. Le moteur sélectionne les cibles depuis son index d'activations validées, en ne
   conservant que les lignées capturées au démarrage et encore actives. Une nouvelle
   enveloppe après crash n'élargit pas ce groupe. L'admission
   persistée conserve les destinataires, l'identité et la fenêtre de déduplication avant
   tout lancement. Une admission retrouvée ne relance aucune action. Les séquences sont
   ordonnées par source ; une ancienne génération invalidée ne bloque pas la suivante.
5. Après admission durable, le moteur acquitte l'événement au propriétaire. Sans cible
   admissible, il persiste d'abord un reçu de consommation sans effet, puis acquitte :
   valider ultérieurement un brouillon ne doit
   pas provoquer un historique d'expirations. Un échec de persistance ou une capacité
   atteinte laisse l'événement en attente et rend l'erreur visible.
6. L'acquittement est persisté avant de retirer l'événement. S'il est perdu, le retry
   retrouve l'admission côté moteur et ne crée pas une seconde exécution. Une perte du
   bus est réparée par lecture des événements en attente, pas par lecture de `remaining`.

L'annulation ou le remplacement invalide aussi le jeton de génération en mémoire.
Le commit est publié par le propriétaire sous son verrou, avec révocation de l'ancien
jeton avant projection ou réponse. Un échec avant renommage garde l'ancien état et
son jeton ; une issue incertaine après renommage clôture la source. Après admission
consommateur persistée, le lancement reprend le verrou producteur et vérifie la
génération. Aucun verrou consommateur n'est gardé pendant cette reprise : pas d'ordre
de verrous inversé, ni de fenêtre de lancement entre commit et révocation.
Le moteur vérifie ce jeton après son attente de persistance et avant lancement ; les
exécutions déjà admises portent à la fois cette annulation et celle de leur activation.
Une vieille diffusion ne commande donc pas la nouvelle génération. Un envoi de transport
déjà engagé ne devient pas réversible par ce mécanisme.

Le jeton de génération et celui de l'activation restent aussi attachés aux ordres
adressés aux pilotes par `CommandPermit`. `CommandRequest::abandoned()` réévalue
ce permis au contrôle précédant l'émission : prendre un ordre dans une file ne
vaut pas encore émission. Les actions Set/Toggle et notifications interrompent
leur attente sur révocation ; le journal suit néanmoins l'issue d'un ordre pris.
Les luminaires composites transmettent le permis à leurs seuls membres en
réévaluant le parent, y compris pour leur continuation bornée après remise sous
tension. Origine, acteur et corrélation des actions autorisées sont conservés.
Une commande sans permis conserve son chemin existant. Aucun observateur asynchrone
n'est requis pour rendre la révocation visible au pilote.

Une interruption après admission mais avant ou pendant un effet reste ambiguë. Les
exécutions interrompues sont signalées et ne sont pas rejouées aveuglément. La promesse
porte sur la livraison récupérable jusqu'à admission et sur l'absence de double lancement,
pas sur une transaction entre le disque et les appareils de la maison.

Les plafonds de sources, fenêtres et exécutions du moteur restent partagés avec AUTO-H4.
Leur saturation est explicite ; elle n'autorise ni perte silencieuse ni allocation
illimitée. Le rattrapage ne concerne que la génération courante de chaque minuteur.

## Lignées d'activation et restauration

`automations.json` conserve une identité serveur par activation, hors de l'empreinte
de logique. Une extinction suivie d'un allumage, une nouvelle validation après changement
de logique ou une restauration invalide l'ancienne lignée. Réapprouver la même version
active est idempotent. Une sauvegarde envoyée par un agent ne choisit jamais cette identité.

Les graphes validés antérieurs sans identité reçoivent une lignée persistée avant
admission du moteur. Un repli sur le fichier d'automatismes de secours crée de nouvelles
lignées : ce secours ne prouve pas quelles activations ultérieures ont été révoquées.
Les anciennes expirations ne sont donc pas confiées à ses activations restaurées.

La sauvegarde emporte `helper-timers.json`, `automation-occurrences.json` et
`automations.json` : horizons du producteur et du consommateur. Une ancienne sauvegarde
ne prouve pas les effets accomplis après sa date. Une restauration partielle de ces
fichiers n'offre pas de nouvelle garantie d'exécution exactement une fois.

## Surfaces et journal

REST `/api/command`, MCP `command` et CLI transportent les mêmes opérations, valeurs
et identités. Le tableau de bord expose durée de démarrage, état, restant, annulation,
demande retenue et retry de même identité. Le schéma OpenAPI décrit les valeurs et erreurs ;
tous les messages visibles sont en français et en anglais dès l'écriture.

Les commandes passent par validation et garde, conservent origine, acteur et corrélation
et ne se font pas passer pour `Origin::Automation` ou `System`. L'expiration est un
événement interne ; elle n'accorde aucun droit d'exécuter un graphe non validé. Les
transitions métier et erreurs sont journalisées avec génération et identité ; les
reprises et interruptions sont distinguées des nouvelles commandes.

La configuration initiale reste déclarative. Ce lot n'ajoute pas d'éditeur de définitions
exécutables ni de registre de versions ; les règles actuelles de validation des graphes,
de session, de PIN et de garde restent inchangées. Aucun déploiement ou test physique.

## Coût et points de réalisation

État et index initialisés uniquement si un minuteur est configuré. Un seul ordonnanceur
pour les échéances actives ; aucun poll ni minuterie périodique sans échéance ou livraison
en attente. Index borné avec remplacement de l'entrée d'une génération, sans accumulation
de vieilles tâches. Le restant peut être rafraîchi à la seconde quand un minuteur tourne,
sans réécrire l'état métier. Une livraison attend un signal de disponibilité ou un retry
borné ; une erreur ne produit pas une boucle active.
Seules les variations sont publiées : au plus une valeur `remaining` par seconde et
par minuteur actif, donc 128 par seconde au plafond, hors transitions. Elles sont
visibles en SSE et à l'historique selon B1 ; aucune rétention ne change implicitement.
Ce plafond représente jusqu'à 11 059 200 variations par jour si tous restent actifs.
Les projections métier n'entrent pas dans le cache périodique ; leur affichage ne
force donc pas un `fsync` de ce cache ou du fichier métier chaque seconde. Ce budget
borné n'est pas une mesure réalisée sur une installation réelle.

Le code purement métier et l'horloge simulée sont séparés de l'ordonnanceur et du hub.
`moli-runtime` possède stockage, projections et livraison ; `moli-helpers` déclare l'aide
et route ses opérations ; `moli-automation` admet et acquitte ; `moli-api`, la CLI et le
dashboard utilisent ces opérations communes. Sauvegarde et catalogues évoluent avec ce lot.

## Preuves attendues

- Durées minimale/maximale, entier exact, durée nulle/négative/flottante/hors plage,
  débordement d'échéance et épuisement de génération.
- Deux démarrages concurrents, remplacement et annulation à l'échéance ; callback
  ancien reçu après un nouveau démarrage ou une annulation, sans effet.
- Restant positif puis nul avec horloge simulée ; saut mural avant/arrière en cours
  d'exécution ; reprise future/passée et horloge de reprise incohérente.
- Coupures avant écriture, avant renommage, après renommage ; perte du demandeur
  pendant commit ; aucune publication ou réussite avant durabilité.
- Reprises répétées après échéance, conservation de l'identité, aucun doublon.
- Coupure après admission moteur et avant acquittement producteur ; effet interrompu
  signalé, jamais rejoué. Retard du bus, moteur démarré après les aides, moteur absent.
- Aucun graphe en brouillon ; extinction/réactivation et changement de version pendant
  admission ; aucune ancienne enveloppe admise dans une nouvelle activation.
- Deux instances sur le même propriétaire, deux intentions retenues plus retry ;
  conflit d'identité avec durée/opération/cible différentes ; fenêtre et capacité.
- Configuration/fichier incompatibles sans reset, sauvegarde/restauration emportant
  échéance, génération, reçus et événement en attente.
- Régressions AUTO-H2, AUTO-H4 et aides historiques ; REST/MCP/CLI/UI, traductions.

Validation native Rust 1.99.1, deux tâches de compilation, suites ciblées puis format et
Clippy workspace ; `npm ci`, tests/build web, contrôle i18n et fuites. La soumission se fait
sur ces preuves ; le garde-fou complet reste réservé au tour de fusion après approbation.


## Exemple et commandes

```toml
[[driver.options.helper]]
id = "pause"
name = "Pause"
kind = "timer"
duration_s = 300
```

- REST : `POST /api/command` avec `{"point":"aides:pause/start","value":30,"request_id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:1"}` ; annuler sur `aides:pause/cancel` avec `value: null` et une nouvelle identité.
- MCP : outil `command`, mêmes champs, garde et reçus.
- CLI : `moli-os timer aides:pause start 30 --request-id aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:1` ; annulation : `moli-os timer aides:pause cancel --request-id aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:2`.
- Graphe : `set` sur `start` avec une durée entière ; `when_occurrence` sur `finished`.
  Créer ou modifier un graphe par agent reste un brouillon jusqu'à validation humaine.

Le [schéma OpenAPI](../api/timer-command.openapi.yaml) décrit les valeurs et erreurs.
