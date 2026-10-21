# Points calculés : min, max, moyenne (AUTO-K1)

Une aide `computed` expose `instance:id/value`, numérique en lecture seule. Sa
valeur vient de la version **courante validée** d’une définition `aggregate` ;
enregistrer prépare seulement un brouillon. Le lot réutilise l’enveloppe de versions,
le propriétaire `dependencies.json`, les écritures CAS, l’aperçu et la session PIN
[d’AUTO-H5](plannings.md). Il n’ajoute ni base de données ni LLM d’exécution.

```toml
[[driver.options.helper]]
id = "average"
name = "Moyenne"
kind = "computed"
aggregate = { operation = "mean", sources = ["sensors:a/temperature", "sensors:b/temperature"], unit = "°C", max_age_s = 300 }
```

`operation` vaut `min`, `max` ou `mean`. Les sources sont des identités de points,
triées sans doublons (32 au plus). Une liste vide est admise mais produit une valeur
indisponible. `max_age_s` est un entier de 1 à 604800 secondes. L’unité de sortie est
facultative : son absence signifie sans unité, sans inférence automatique.
Les champs `initial`, `options`, `min`, `max`, `step` et `weekly` sont refusés pour
cette aide ; `aggregate` est refusé sur les autres aides. La configuration initialise
un brouillon à la première création et ne remplace pas le courant restauré.

## Données utilisables

La publication vérifie que les descriptions des sources existent, sont numériques
et lisibles, avec des unités compatibles. Elles peuvent être hors ligne ou sans
mesure. Une source calculée doit posséder un courant validé. Après publication,
chaque lecture vérifie encore description, type et unité : une source supprimée ou
devenue incompatible est exclue. Un changement d’unité ne réinterprète pas une ancienne
observation, même avec des unités convertibles : une nouvelle observation dans la
nouvelle unité est nécessaire.

Sont ignorés les points absents, les échantillons absents/nuls/non numériques/non
finis, les sources explicitement hors ligne et les observations arrivées à leur
échéance de fraîcheur. Un état dont la disponibilité n’a pas été déclarée peut
contribuer s’il possède une observation numérique fraîche. La fenêtre est
`[observation, observation + fraîcheur)` : à l’égalité de fin, la donnée est périmée.
La moyenne porte uniquement sur les données utilisables ; un ensemble vide donne
`null`, jamais zéro. Les opérations utilisent des `f64`, avec une moyenne mise à
l’échelle pour éviter le débordement de valeurs finies extrêmes.

Les unités égales sont compatibles. Des rapports explicites sont également admis
entre Wh/kWh, mV/V et s/min/h ; le calcul convertit vers l’unité déclarée. Les autres
symboles, y compris les unités personnalisées, exigent une égalité exacte. Une
conversion qui déborde est exclue comme une valeur non finie. Il n’existe pas de
conversion implicite de dimension ni de transformation inventée par un agent.

## Temps, rafraîchissement et reprise

Un rafraîchissement réel de source renouvelle son âge même si sa valeur n’a pas changé.
Le hub en informe l’index interne sans fabriquer de changement d’état sur le bus : un
capteur revenu avec exactement la même valeur redevient utilisable. Une simple
métadonnée, une remise en ligne ou un recalcul ne renouvelle pas l’observation.

En processus, l’âge suit une horloge monotone. Un saut de l’horloge civile ne rajeunit
pas une observation et ne ressuscite pas une donnée déjà expirée. Au chargement d’un
cache, l’âge initial vient de son horodatage absolu conservé ; un horodatage futur
est ignoré jusqu’à son instant. À la reprise, aucune valeur calculée n’est restaurée
comme une observation neuve : les projections sont recomputées depuis les sources.
Leurs valeurs sont exclues du cache périodique ; leurs définitions et validations
restent dans le fichier métier commun, inclus dans la sauvegarde.

L’échantillon projeté conserve le plus ancien horodatage des sources utilisées. La
provenance monotone traverse aussi les chaînes : un calcul intermédiaire ne renouvelle
pas l’âge de ses entrées. Le bus distingue l’heure de publication (`ts`) de celle des
sources (`source_ts`, facultative), de sorte qu’un changement de définition ne
rétrodate pas une nouvelle valeur dans l’historique ou l’énergie. Le modèle courant
et le dashboard conservent l’horodatage source pour afficher l’âge réel.

Un ordonnanceur global de calcul, indépendant du rédacteur durable, traite les échéances même sans
nouvel événement. Un brouillon suspendu en écriture ne bloque ni les projections
courantes ni leur péremption. Il n’existe aucune tâche par source ni calcul. Sans courant calculé,
aucune échéance de calcul n’est entretenue. L’attente est plafonnée pour détecter les
sauts d’horloge des données futures ; elle ne renouvelle pas les observations vivantes.

## Propagation et bornes

Le registre partagé reste limité à 128 définitions, deux têtes par définition et
4 Mio. Un calcul possède au plus 32 sources ; une chaîne est limitée à 16 niveaux.
Cycles directs/indirects, unités incompatibles entre calculs et dépassements sont
refusés avant activation durable. Un brouillon peut proposer une topologie à corriger,
mais ne change aucun index d’exécution courant.

L’index inverse connaît les seuls calculs affectés par un point ou un appareil. Une
file de racines sales coalesce les **états**, pas des occurrences. Les calculs touchés
sont ordonnés par un rang topologique stable, avec l’identité comme départage. Une
propagation consulte au plus 128 calculs et 8192 arêtes/lectures ; elle applique les
projections depuis une vue cohérente du modèle avant de publier leurs événements.
Elle ne parcourt pas toutes les définitions à chaque observation ordinaire. Des files
ordonnées d’échéances évitent aussi de balayer tous les calculs non échus.

La file sale contient au plus 128 identités ; les index et observations contiennent
au plus 4096 références de sources. Une même valeur ne réémet pas de changement
numérique ; une mise à jour d’horodatage alimente néanmoins les descendants.
Les changements de valeur entrent au journal `Action::Computed` avec version,
unité, horodatage source et nombre de sources valides. Les changements de définition
et leurs erreurs suivent le journal commun des dépendances.

## Validation et consommateurs transitifs

L’aperçu commun inclut les graphes concernés directement **et via des calculs en aval**.
Il vérifie la validité du brouillon avant la confirmation humaine. Il lie également
les versions courantes des sources exécutables du brouillon, et les
versions brouillon/courant des calculs concernés. Une publication ou un changement de
portée entre aperçu et confirmation donne 409 : il faut examiner à nouveau.
Les consommateurs sont relus en un seul instantané auprès du propriétaire des graphes,
sous le verrou commun ; la validation ne dépend pas du retard du bus.

La session PIN existante reste obligatoire. Ni un en-tête UI, ni un argument MCP/CLI,
ni un champ déclaré humain ne valide une définition. Une mise à jour courante ne
modifie pas l’empreinte d’identité des graphes : elle est autorisée explicitement en
montrant leurs consommateurs. Les graphes et calculs restés en brouillon restent inactifs.
Les valeurs des autres aides restent sans version de définition.

## Surfaces

REST utilise les routes communes avec `kind=computed` :
`GET /api/dependencies/computed/{id}`, `PUT` avec `expected` et `definition`, puis
`GET /preview` et `POST /approve`. La définition est
`{"type":"aggregate","value":{"operation":"mean","sources":[],"unit":null,"max_age_s":60}}`.
Voir [OpenAPI](../api/dependencies.openapi.yaml). Le corps de confirmation est borné
à 512 Kio pour les consommateurs transitifs ; les éditions restent bornées à 256 Kio.

MCP réutilise `list_dependencies`, `get_dependency` et `save_dependency_draft`, avec
`kind: "computed"` (défaut compatible : `schedule`). CLI :

```
moli-os dependency get helpers:average --kind computed
moli-os dependency preview helpers:average --kind computed
moli-os dependency draft helpers:average 1 definition.json --kind computed
```

MCP et CLI préparent seulement un brouillon. L’éditeur du point affiche sources,
opération, unité, fraîcheur, courant/brouillon et portée avant confirmation PIN.
Textes et erreurs existent en français et en anglais. Les invités n’accèdent pas aux
métadonnées du registre ; les mutations sont interdites en démonstration.

Ce lot fournit les identités de projection, le suivi des dépendances et la validation
que pourront réutiliser les compteurs périodiques et le temps dans un état. Ces
algorithmes, les modèles d’estimation et les reprises de configurations restent dans
leurs tickets respectifs.
