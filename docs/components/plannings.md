# Plannings et versions des dépendances exécutables (AUTO-H5)

Le planning est un point `schedule` booléen en lecture seule (`instance:id/active`).
Il n’envoie aucune commande : un graphe validé peut employer ce point comme état,
condition ou déclencheur. Ni agent ni LLM ne participent à son exécution.

## Définition et calendrier

Une définition comprend un fuseau IANA et au plus 256 créneaux normalisés. Lundi vaut
1, dimanche 7 ; `start` et `end` sont des minutes entières. Le début est inclus, la fin
exclue ; 1440 est autorisé uniquement comme fin de journée. Une fin inférieure au début
traverse minuit : le serveur la découpe en deux créneaux, y compris dimanche/lundi.
Une fin égale au début est refusée ; une journée entière s’écrit 0..1440. Les créneaux
sont triés, les chevauchements rejetés et les adjacences autorisées. La limite de 256
s’applique aussi après découpage. Une liste vide est toujours inactive.

```toml
[[driver.options.helper]]
id = "weekly"
name = "Planning"
kind = "schedule"
weekly = { timezone = "UTC", slots = [{ day = 1, start = 540, end = 1020 }] }
```

Cette configuration initialise un **brouillon** lors de la première création. Elle
ne valide jamais la définition et ne remplace pas un état métier déjà enregistré.
Les champs `initial`, `options`, `min`, `max` et `step` sont refusés pour un planning ;
`weekly` est refusé pour les autres aides.

Le prédicat utilise le jour et la minute civils dans le fuseau choisi. Une heure sautée
ne crée aucun instant artificiel ; un intervalle entièrement sauté ne devient pas
actif. Si un changement de fuseau saute à l’intérieur d’un créneau, celui-ci prend
effet à ce changement. Les deux copies d’une heure répétée suivent le même prédicat :
02:15..02:45 peut devenir actif deux fois à des instants absolus distincts ; 02:00..03:00
reste actif pendant les deux copies. Ce ne sont pas des doublons de livraison.

Un ordonnanceur global conserve les prochaines échéances des définitions courantes.
Il considère aussi les changements de décalage du fuseau ; il n’alloue aucune tâche
par créneau. Son battement maximal de 60 secondes détecte les sauts d’horloge ; seuls
les plannings arrivés à échéance sont recalculés en temps normal. Sans définition
courante ayant une échéance, il attend une notification sans scruter l’horloge.

## Contrat commun : brouillon, courant et consommateurs

`VersionedDefinition<T>` conserve deux têtes : brouillon et courant validé. Le premier
brouillon est v1, toute édition CAS incrémente son numéro. Les versions sont des entiers
1..2^53−1 ; l’épuisement est refusé. Deux écritures attendant la même version ne peuvent
pas toutes deux réussir. Aucun historique illimité n’est conservé.

Toute édition, humaine comprise, prépare un brouillon sans effet sur l’exécution.
La publication est une opération distincte :

1. Lire l’aperçu : identité stable, versions brouillon/courant, graphes concernés et
   empreinte SHA-256 de cet ensemble avec les définitions.
2. Montrer le brouillon et ces graphes à la personne. L’éditeur distingue les deux têtes
   et avertit qu’une publication peut provoquer un changement d’état.
3. Confirmer l’aperçu exact via la session PIN existante. Une origine `ui` ou un champ
   déclaré humain ne suffit pas ; aucun nouveau chemin de confiance n’est introduit.
4. Vérifier de nouveau versions, définitions et consommateurs sous le verrou partagé
   avec les enregistrements, validations, restaurations et suppressions de graphes.
   Un changement donne **409**, sans publication ; il faut relire l’aperçu.

L’index inclut les graphes en brouillon et les références dans déclencheurs, actions,
conditions, attentes et références de points dans les messages. L’aperçu présente identités, noms et empreintes des graphes ;
une modification de ces éléments l’invalide. Les drapeaux d’activation ne changent pas
son empreinte : allumer conserve les vérifications humaines existantes, éteindre ne
peut qu’enlever une autorité. Une mutation de graphe non encore enregistrée ou dont l’écriture a échoué ferme la
publication des dépendances jusqu’à un enregistrement réussi ou une reprise. Cela
empêche un ancien graphe encore sur disque de disparaître d’un aperçu en mémoire.
La sauvegarde des graphes synchronise aussi le répertoire après rename.
Le verrou n’entrave pas les coupures d’urgence. L’aperçu relit également les consommateurs
auprès du propriétaire des graphes : un événement de bus en retard ne peut pas en cacher.

L’empreinte du graphe couvre les identités des points, sans y figer les versions des
dépendances. Le graphe utilise la version **courante validée**, jamais le brouillon.
Sans courant, le vérificateur refuse sa validation et le moteur le retire de l’ensemble
exécutable. Une publication courante explicite ne retire pas la validation du graphe :
la personne vient d’autoriser ce remplacement en voyant ses consommateurs.

Une nouvelle version ne rejoue pas une action physique déjà admise et n’annule pas
rétroactivement une commande émise. Les étapes suivantes lisent le nouvel état courant.
La garde, les échéances et les règles habituelles des commandes restent applicables.
Les aides de valeur existantes ne reçoivent pas de version à chaque changement de valeur.
AUTO-K1 étend l’énumération typée avec les agrégats et réutilise cette enveloppe,
l’aperçu et le verrou de publication. Ses sources versionnées et consommateurs
transitifs sont liés dans l’aperçu commun, voir [calculs.md](calculs.md). Les scènes AUTO-S1 étendent le même propriétaire et l’aperçu avec leur portée
validée, sans projection de point ni JSON exécutable libre ; voir [scenes.md](scenes.md).

## Persistance, reprise et journal

Un propriétaire unique par hub sérialise `dependencies.json` : schéma 1, 128 définitions,
4 Mio au plus. Une définition retirée de la configuration est conservée, sans projection
ni exposition, et occupe toujours sa place ; aucune éviction silencieuse. Le fichier
est inclus dans la sauvegarde. Il n’a aucune dépendance à la base historique.

Les versions et le dernier booléen projeté sont écrits ensemble en temporaire, fsync,
rename et fsync du répertoire. L’acquittement arrive après cette séquence ; la disparition
du demandeur n’interrompt pas une écriture engagée. Avant rename, un échec conserve les
anciennes têtes/projections pour une édition ou publication refusée. Pour une transition
horaire échouée, le dernier booléen ne peut plus représenter l’heure présente : les
projections sont fermées et exigent une reprise après rétablissement du stockage.
Après rename, une issue incertaine ferme les mutations et
invalide les projections jusqu’au redémarrage et à la relecture. Aucun état corrompu
n’est réinitialisé automatiquement. Une réponse perdue se résout par relecture des
versions ; aucune nouvelle édition aveugle n’est conseillée.

La reprise choisit le courant enregistré, pas le brouillon ni la configuration initiale.
Elle calcule l’état civil présent, sans rejouer les frontières manquées. Le premier
état d’un point après démarrage n’est pas un déclenchement. Un tick identique ne réémet
rien ; un retour d’horloge fait recalculer le prédicat actuel et peut donc produire une
nouvelle transition réelle. Une perte du bus se répare par lecture d’état, sans inventer
de transition manquée. Chargements, éditions, publications, transitions et leurs échecs
métier ont une trace `Action::Dependency` avec version, origine et résultat.

## Surfaces

Les routes sont documentées dans [le schéma OpenAPI](../api/dependencies.openapi.yaml).
REST fournit lecture, édition CAS, aperçu et publication PIN. MCP fournit
`list_dependencies`, `get_dependency` et `save_dependency_draft` ; il ne valide rien.
Les invités n’accèdent ni à ces routes ni aux métadonnées de dépendances dans SSE.
Les mutations sont interdites en démonstration comme celles des graphes.

La CLI parle uniquement au serveur déjà lancé :

```
moli-os dependency list
moli-os dependency get helpers:weekly
moli-os dependency preview helpers:weekly
moli-os dependency draft helpers:weekly 1 definition.json
```

Le fichier JSON borné à 256 Kio contient la définition typée
`{"type":"schedule","value":{"timezone":"UTC","slots":[]}}`. CLI et MCP préparent
un brouillon ; la publication se fait dans l’éditeur ou par REST avec une session humaine
valide, liée au client selon les règles existantes. Aucun PIN sur la ligne de commande.

AUTO-S1 réutilise ce même propriétaire pour `DependencyKind::Scene`, sans point
booléen ni tâche d’horloge. Son aperçu inclut sa portée physique et son courant
reste séparé du brouillon. Voir [scenes.md](scenes.md).
