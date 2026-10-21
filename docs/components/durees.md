# Temps dans un état et couverture connue (AUTO-K3)

Une aide `computed` peut porter une définition `duration`. Elle réutilise les deux
versions, le CAS, les consommateurs transitifs et la validation PIN des
[calculs](calculs.md). Une proposition d’agent reste un brouillon. Le point `value`
est une durée mesurée en secondes, en lecture seule, jamais une commande physique.

```toml
[[driver.options.helper]]
id = "duration"
name = "Temps observé"
kind = "computed"
duration = { source = "sensors:relay/on", value = true, window_s = 3600, max_age_s = 60 }
```

`aggregate` et `duration` sont exclusifs. `source_unit`, facultative, signifie
explicitement sans unité lorsqu’elle est absente ou `null` ; pour un état numérique
avec unité, fournir celle de sa source. Le point doit être lisible, non événementiel,
avec une valeur scalaire non nulle compatible. Entier et flottant équivalents sur un
point numérique ne sont pas des changements d’état. Textes et booléens restent typés.
Les sources calculées et les plannings doivent avoir un courant validé. Les cycles
mixtes agrégats/durées et les chaînes dépassant 16 niveaux sont refusés.

## Mesure et couverture

La fenêtre glissante se termine au checkpoint durable affiché. Elle commence au plus
tôt à la publication de cette version ; aucune histoire n’est inventée avant elle.
Une nouvelle version publiée recommence la mesure. Une réapprobation de la même
version et un brouillon ne remettent rien à zéro.

Le bilan expose `acquired_ms` (temps observé dans l’état voulu), `known_ms` (temps
dans un état valide, voulu ou différent), `unknown_ms`, `entered_ms` (entrée dans
l’intervalle correspondant encore ouvert), `observing`, `window_start_ms`,
`as_of_ms` et `limited`. Ainsi une durée nulle avec une couverture nulle signifie
absence d’observation. Ce n’est pas la preuve que l’appareil n’a jamais été dans cet
état. L’interface montre les trois durées et la date du checkpoint.

Les intervalles sont demi-ouverts : la péremption termine la couverture à
`observation + max_age_s`, même sans événement suivant. Une observation égale renouvelle
la fraîcheur sans doubler l’acquis. L’entrée continue reste identique quel que soit
l’ordre entre ce rafraîchissement et le checkpoint à l’expiration. La provenance
d’un intervalle périmé est conservée à cette seule fin, sans prolonger sa couverture ;
une invalidation explicite ou une reprise la retire. Un événement antérieur à la dernière observation
est ignoré ; un événement retardé entre elle et un checkpoint corrige le suffixe
supposé de cet intervalle. Les changements à la frontière ne créent pas de chevauchement.
Un changement de description, une suppression, une valeur invalide ou un passage hors
ligne ferme la couverture. La perte de lisibilité la ferme aussi ; chaque capture
revérifie le descripteur courant. Une remise en ligne ou le rétablissement de la
lisibilité seul ne la rouvre pas : une nouvelle observation réelle est nécessaire.
Retrait et invalidation sont sérialisés sous le même verrou du modèle que les
producteurs, dans l’ordre modèle puis registre.

Le temps interne suit une ancre monotone. Un recul de l’horloge civile dans le même
processus ne double ni ne soustrait du temps. À la reprise avec une horloge antérieure
au checkpoint, la couverture reste fermée jusqu’à ce que l’ancre atteigne cet instant.
Les dates du bilan sont celles de cette chronologie ; elles peuvent temporairement
différer de l’horloge civile après un saut.

## Durabilité et interruption

`durations.json` est indépendant de `moli.db`, du cache et de la migration historique.
Il contient le schéma, les versions des définitions, leurs paramètres, les segments
et l’intervalle ouvert jusqu’au dernier checkpoint. Un seul rédacteur écrit l’état
entier par `StateFile` : temporaire, fsync, rename, fsync du répertoire. La sauvegarde
inclut ce fichier. Les bilans et projections ne sont publiés qu’après cette écriture.

Au redémarrage, les acquis durables sont conservés ; l’intervalle ouvert est fermé au
dernier checkpoint. Ni l’interruption ni le cache restauré ne sont comptés comme du
temps observé. Une nouvelle observation réelle est nécessaire ; un agrégat produit
uniquement depuis le cache ne suffit pas. Répéter la reprise ne recompte pas les
segments. Un arrêt brutal peut perdre la partie non encore checkpointée : elle devient
inconnue. Une erreur d’écriture suspend la brique et rend ses projections indisponibles ;
une issue après rename reste incertaine. Ne pas effacer ou réinitialiser silencieusement
un fichier corrompu. Restaurer le fichier puis redémarrer après diagnostic.

## Bornes et surfaces

- Fenêtre et fraîcheur : entiers de 1 à 604800 secondes ; un seul point source.
- 128 définitions communes aux dépendances, 256 segments par durée, fichier de 4 Mio.
- À saturation, les segments les plus anciens sont écartés ; leur temps redevient
  inconnu et `limited` le signale tant que cette partie appartient à la fenêtre.
- Index inverse par point et appareil ; aucune file qui coalesce des transitions.
  Aucun travail de durée lorsqu’aucun courant `duration` n’est configuré.
- Un ordonnanceur global actif, checkpoint au plus une fois par seconde en régime
  normal, sans tâche par calcul. Au pire 128 bilans et 32768 segments à sérialiser.
  Le snapshot est pris en mémoire avant fsync ; les observations continuent pendant
  l’écriture et seront incluses dans le checkpoint suivant. Le bilan peut donc être
  en retard et expose toujours sa date. Publication et reprise écrivent aussi un checkpoint.
- Le journal commun trace les publications, checkpoints et erreurs ; `Action::Computed`
  porte version, durée, unité et le bilan `coverage` complet (connu, inconnu,
  entrée, fenêtre et limite), sans modifier les anciens événements K1. Aucun LLM ni appareil réel.

REST : routes communes `/api/dependencies/computed/{id}` pour la définition et sa
validation ; `GET /api/durations/{id}` pour le dernier bilan durable (404 sans courant,
503 si suspendu). Accès réservé au foyer, aucun accès invité. MCP : outils communs de
brouillon et `get_duration`. CLI : `moli-os dependency duration helpers:duration`.
Les mêmes règles PIN/CAS s’appliquent, aucun argument déclaré humain ne les remplace.
Voir l’[OpenAPI](../api/dependencies.openapi.yaml).
