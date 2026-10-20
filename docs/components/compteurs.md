# Aides compteur (AUTO-H2)

Une aide `counter` expose une valeur entière `value` en lecture seule et deux
commandes à `null`, `increment` et `reset`. La garde et le journal du hub s'appliquent
aux deux commandes. Aucun LLM ni appareil physique ne participe au calcul.

```toml
[[driver.options.helper]]
id = "cycles"
name = "Cycles"
kind = "counter"
initial = 7
step = 2
min = 0
max = 1000
```

`initial` vaut 0 et `step` vaut 1 par défaut. Les bornes omises sont
−9007199254740991 et 9007199254740991. Les quatre paramètres sont des entiers exacts,
sans conversion par flottant ; `1.0` n'est pas un entier de configuration. Le pas
peut être négatif mais jamais nul. `min ≤ initial ≤ max` et tous les paramètres
restent dans cette plage exactement représentable par les clients JSON.
Un incrément hors bornes ou en dépassement arithmétique est refusé sans changer la
valeur ; `reset` revient à l'initial configuré, même non nul.

## Ordre, identité et rejeu

L'ordre effectif est celui de l'admission par le rédacteur commun **après** la
garde ; les numéros de requête et l'ordre des clics ne constituent pas une horloge
d'exécution. Deux commandes distinctes retenues restent deux demandes à approuver.
Un retry identifié retrouve sa demande en attente.

Les mêmes surfaces REST, MCP et CLI transportent `request_id` sous la forme
`<client hexadécimal de 32 caractères>:<séquence positive>`, comme AUTO-H4.
Une nouvelle identité signifie une nouvelle intention. Un retry réutilise exactement
l'identité et le point d'origine. Sans identité, chaque appel admis est une nouvelle
opération et n'offre aucune déduplication après une perte de réponse.

L'espace d'identités des compteurs est global à leurs instances : utiliser la même
identité sur un autre compteur ou une autre opération est un conflit. Il est distinct
de celui des boutons AUTO-H4 ; les clients partagent néanmoins leur générateur de
séquences pour éviter une réutilisation accidentelle entre types.

Valeur et reçu sont un seul commit. Le reçu d'un refus de bornes ou de dépassement arithmétique est aussi durable :
une incrémentation refusée à la borne ne devient pas une réussite si on la rejoue
après un reset. Rejouer une réussite n'applique ni ne republie sa valeur historique :
`increment A → reset B → retry A` laisse l'initial de B ;
`reset A → increment B → retry A` laisse la valeur de B.
Les réponses n'exposent aucune valeur historique ; lire `value` pour l'état courant.

## Persistance et reprise

Un seul propriétaire au niveau du hub écrit `data/helper-counters.json` pour toutes
les instances. Le format est versionné, borné à 1 Mio, 128 compteurs au total,
64 clients et une fenêtre des 32 dernières séquences par client. Aucun client n'est
évincé ; une séquence sortie de la fenêtre est refusée, jamais exécutée à nouveau.
Les compteurs retirés de la configuration conservent leur valeur dans ce plafond.
La file de commandes du pilote et l'échéance du hub restent les limites d'admission.

Chaque commit écrit l'état entier en temporaire, le synchronise, le renomme puis
synchronise son répertoire. L'acquittement et la publication de la valeur attendent
ce commit. Une erreur avant renommage laisse l'ancienne valeur ; après renommage,
l'issue est incertaine : erreur visible, aucune valeur présentée comme confirmée,
mutations clôturées jusqu'au redémarrage puis reprise du fichier validé. Une perte
du demandeur après engagement ne coupe pas le suivi du commit : s'il réussit, la
nouvelle valeur est publiée. Avant engagement, abandon et échéance sont revérifiés
après l'attente du propriétaire et avant la première écriture. L'arrêt du pilote
est aussi revérifié à cette frontière ; il ne coupe pas un commit déjà engagé.

Le résultat métier final possède aussi une entrée `counter` au journal avec la
corrélation de la demande, même lorsque la réponse de transport est perdue.

Le cache périodique du hub n'est jamais l'autorité du compteur. Un fichier abîmé,
une version inconnue ou une valeur restaurée incompatible avec la configuration
produisent une erreur explicite, sans clamp ni remise à zéro silencieuse.
Sauvegarde et restauration emportent valeurs et reçus dans le même fichier ;
restaurer une sauvegarde ancienne restaure aussi son horizon de déduplication.

Les quatre aides historiques (`select`, `switch`, `number`, `text`) et le bouton
AUTO-H4 gardent leurs comportements. Aucune tâche ni lecture de ce fichier n'est
démarrée lorsque les compteurs ne sont pas configurés.

## Surfaces

- REST : `POST /api/command` avec `{"point":"helpers:cycles/increment","value":null,"request_id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:1"}` ; même forme pour `/reset`.
- MCP : outil `command`, mêmes champs et mêmes erreurs métier.
- CLI : `moli-os counter helpers:cycles increment --request-id aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:1` ; remplacer `increment` par `reset` pour revenir à l'initial. La CLI contacte le serveur déjà lancé, sans second pilote ni rédacteur.
- Tableau de bord : valeur courante, incrément et retour à l'initial ; une réponse perdue se réessaie avec la même identité. Un 202 reste en attente d'approbation. Une issue incertaine (409, `retryable: true`) conserve la demande à réessayer après réparation/redémarrage ; les conflits et requêtes trop anciennes restent refusés.

La description de `value` porte `kind: {"type":"counter","spec":{"initial":7,"step":2,"min":0,"max":1000}}`
et `access: {"read":true,"write":false}`. Les opérations portent
`kind: {"type":"counter_command","operation":"increment"}` (ou `reset`),
avec `access: {"read":false,"write":true}`. Leur dernier état n'est jamais mis en cache.
