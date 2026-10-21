# Aides date et heure (AUTO-H3) — contrat proposé

Statut : préparation du lot, à relire avant les changements de comportement.
Socle proposé : AUTO-H2 approuvé (`18cff345`). Le minuteur AUTO-H1 n'est pas un
prérequis. Ce document ne livre ni aide exécutable ni activation.

## Intention et périmètre

Ajouter trois aides saisissables : date civile, heure civile et date-heure dans
un fuseau explicite. Les valeurs sont validées, durables avant confirmation et
lisibles par les surfaces existantes. Une aide n'est pas une horloge : elle ne
change pas avec le temps et ne crée aucun déclencheur à l'heure saisie.

Les quatre aides historiques, le bouton et le compteur gardent leur contrat.
Les définitions restent déclaratives ; ce lot n'ajoute pas d'éditeur de
définitions exécutables ni de mécanisme de versions de dépendances. Les valeurs
restent de l'état ordinaire non versionné, soumis à la garde et au journal.
Un graphe créé ou modifié par un agent reste un brouillon jusqu'à validation
humaine. Aucun LLM dans le chemin de commande ou de validation temporelle.

## Types et représentation

Proposition : `HelperKind` date, time et datetime, avec un point `value` lisible
et inscriptible. Un type de point temporel explicite porte le format ; le format
datetime porte aussi le nom du fuseau. Les valeurs du protocole restent des
chaînes, sans imposer de nouvel objet à tous les transports.

| Aide | Valeur canonique | Signification |
|---|---|---|
| date | `YYYY-MM-DD` | Date civile, indépendante d'un fuseau |
| time | `HH:MM:SS` | Heure civile, sans date ni instant implicite |
| datetime | `YYYY-MM-DDTHH:MM:SSZ` | Instant UTC, avec fuseau d'affichage et de saisie dans le type du point |

Années de 0001 à 9999, calendrier grégorien, heures 00 à 23, minutes et secondes
00 à 59. Refuser dates impossibles, espaces, dépassements, secondes intercalaires,
fractions et valeur numérique/null. Ne pas corriger silencieusement une date ni
transformer une heure seule en instant du jour courant.

Pour datetime, la configuration fournit obligatoirement `timezone` : nom IANA
reconnu ou `UTC`. Date et time refusent ce champ, qui n'aurait pas de sens sans
date complète pour résoudre un décalage saisonnier. La valeur `initial` est
obligatoire pour ces trois nouvelles aides ; aucune lecture de l'heure courante
ou valeur d'époque implicite à l'initialisation.

## Saisie d'une date-heure

La représentation canonique de l'instant et le fuseau sont restitués ensemble
par la valeur et sa description de point. Le fuseau de la machine cliente ou du
serveur ne choisit jamais l'interprétation.

- Saisie locale sans décalage : `YYYY-MM-DDTHH:MM:SS`, interprétée dans le fuseau
  configuré seulement si elle correspond à un instant unique.
- Heure locale inexistante au passage à l'heure d'été : refus explicite, sans
  décalage vers l'heure suivante.
- Heure locale répétée au passage à l'heure d'hiver : refus sans décalage explicite.
  La personne fournit `+HH:MM` ou `-HH:MM` pour choisir l'occurrence ; vérifier que
  ce décalage appartient effectivement à cette date locale dans le fuseau.
- Saisie terminée par `Z` : instant explicitement UTC, accepté indépendamment du
  fuseau d'affichage. Cette forme sert aussi au retour de la valeur canonique.
- Toute saisie acceptée revient en UTC avec secondes entières. Refuser une
  conversion qui sortirait des bornes du calendrier canonique.

Exemple synthétique dans `Europe/Paris` : `2026-03-29T02:30:00` est inexistant ;
`2026-10-25T02:30:00` est ambigu. Ses formes `+02:00` et `+01:00` désignent deux
instants distincts, respectivement `00:30:00Z` et `01:30:00Z`. Le nom du fuseau
reste celui de la définition, quelle que soit la représentation UTC de la valeur.

Les règles de calendrier et de forme restent pures dans le cœur. La résolution
du fuseau utilise Jiff dans la couche qui possède déjà cette dépendance ; aucun
accès à la base des fuseaux depuis le domaine pur. Hub, initialisation et contrôle
des graphes partagent la même validation complète. Une validation de graphe ne
doit pas annoncer une saisie valide que le hub refuserait pour ambiguïté.

## Durabilité et reprise

Fichier proposé `helper-datetimes.json`, versionné et borné à 1 Mio, au plus 128
aides temporelles au niveau du hub. Un propriétaire sérialise toutes les instances.
Il conserve définition du format/fuseau et valeur canonique dans le même état.
Pas de nouvelle base SQL, pas de dépendance à l'historique, pas d'ordonnanceur.

Écriture complète en temporaire, fsync, renommage atomique puis synchronisation
du répertoire, via `StateFile`. Confirmer et publier seulement après succès.
Avant engagement, revérifier abandon, échéance et arrêt après attente du rédacteur.
Une tâche engagée suit sa mutation et son journal même si le demandeur disparaît.

Échec avant renommage : ancienne valeur conservée, erreur visible. Issue incertaine
après renommage : fermer les mutations et ne plus présenter une projection comme
confirmée jusqu'à reprise depuis le fichier validé. Ne jamais réinitialiser un
fichier abîmé ou incompatible. Restaurer une valeur qui ne correspond plus au
format/fuseau déclaré doit être refusé, sans réinterprétation silencieuse.

Le cache périodique `state.json` n'est pas l'autorité de ces trois aides et ne
doit ni les rétablir avant validation métier ni recevoir leurs projections.
Changer `initial` n'écrase pas une valeur durable existante de même définition.
Une restauration conserve l'instant UTC ; un saut d'horloge ou un changement de
date courante ne déplace pas cet instant et ne crée aucune occurrence.

Ces écritures sont des affectations absolues, pas des opérations comme increment
ou start. Le lot n'ajoute pas de registre de reçus : réémettre une ancienne valeur
peut remplacer une écriture plus récente. Après issue incertaine, relire l'état
confirmé après reprise avant de décider d'une nouvelle affectation. Deux commandes
simultanées sont sérialisées ; la dernière engagée détermine la valeur finale.

Sauvegarde et restauration incluent le fichier métier entier. Une sauvegarde
ancienne rétablit les valeurs de sa date, sans garantie sur les écritures futures.
Les restaurations sont effectuées service arrêté selon le contrat existant.

## Surfaces et graphes

REST et MCP utilisent la commande existante sur `value` ; la CLI utilise le même
serveur, sans ouvrir un second rédacteur. Documenter valeurs, fuseau et erreurs
dans un schéma OpenAPI et les descriptions d'outils. Valeur temporelle invalide ou
ambiguë : 422 ; stockage impossible : 500 ; capacité : 503 ; issue incertaine :
409 avec indication de reprise ; garde : demande 202 comme pour les autres aides.

Le dashboard et l'éditeur de graphe proposent les contrôles adaptés. Date et heure
gardent leur sens civil ; une date-heure affiche le fuseau configuré et un moyen
explicite de préciser le décalage d'une heure répétée. Ne pas dépendre de la
conversion automatique de `Date` JavaScript dans le fuseau du navigateur.
Erreurs de saisie et libellés sont traduits en français et en anglais.

Les graphes peuvent affecter une valeur et réagir explicitement à son changement
avec les déclencheurs existants. Ce lot n'introduit ni réveil à l'instant saisi ni
comparaisons calendaires implicites. L'initialisation/restauration ne vaut pas une
nouvelle saisie ; les règles de brouillon et de validation humaine restent intactes.

## Preuves à livrer

- Calendrier : années bissextiles et siècles, fins de mois, minuit et changement de
  jour ; formats stricts, limites, valeurs numériques/null et secondes invalides.
- Fuseau : heure sautée, heure répétée refusée sans choix, deux décalages corrects
  produisant deux instants distincts ; décalage incompatible refusé, UTC stable.
- Reprise : sérialisation canonique, instant inchangé après redémarrage, saut
  d'horloge sans mutation, ancien cache ignoré et configuration historique lisible.
- Persistance : coupures avant/après renommage, demandeur perdu pendant commit,
  absence de succès ou publication précoce, erreur/incertitude et état incompatible.
- Concurrence de deux instances, garde et journal, affectation sans déclencheur
  horaire ajouté, aucun graphe en brouillon exécuté.
- REST/MCP/CLI/UI, tests des formats côté client, fr/en, sauvegarde/restauration.

Validation native Rust 1.99.1, deux tâches, fmt/Clippy workspace et suites ciblées.
Interface : npm ci, tests et build, contrôle i18n ; catalogue et fuites. Soumission
au relecteur affecté sur ces preuves ; contrôle complet au tour d'intégration après
approbation. Aucun déploiement, effet physique, import privé, merge ou push ici.
