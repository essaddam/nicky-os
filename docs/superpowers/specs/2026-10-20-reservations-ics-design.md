# Réservations ICS : calendrier et fenêtres d’accès

## But et périmètre

Lire le calendrier de location choisi par un humain, présenter les cinq prochaines
réservations, publier les arrivées/départs réellement observés et limiter les accès
invités aux fenêtres de réservation. Une réservation ne représente jamais la présence.
La référence est le ticket Réservations ICS, étape distincte du pilote de serrure.
Aucun appareil réel, import de données privées, déploiement ou push.

## Découpage

- `moli-reservations` : parseur et expansion bornés, calcul pur des fenêtres, client
  réseau et pilote en lecture seule. Les modules de domaine ne dépendent pas du hub.
- `moli-core` : fenêtre temporelle demi-ouverte, début inclus et fin exclue.
- `moli-runtime` : instantané de fenêtres par instance, éphémère et limité dans le
  temps. Seul le contexte du pilote propriétaire le remplace ; hors ligne ou expiré,
  l’instantané ne donne aucun accès. Il n’est jamais restauré depuis un fichier.
- `moli-api` : liaison facultative d’un invité à une réservation stable, saisie sous
  la même autorisation humaine que ses autres droits. Contrôle à chaque requête,
  y compris avec une session déjà ouverte. Les flux cessent de livrer des données après
  révocation. Un ordre encore en attente de transport ne peut pas partir après la fin
  de sa fenêtre ou sa révocation ; la garde et le contrôle avant émission restent actifs.
  Les invités existants gardent leur politique.
- Paquet `integrations/reservations/`, registre du binaire, documentation et fr/en.

## Calendrier hostile

L’URL est le secret `url` de l’instance, fourni par un humain ; aucune URL dans la
configuration ou les outils de découverte. HTTPS public vérifié, aucun suivi de
redirection, aucun chargement des liens/attachements du calendrier. Erreurs expurgées
de l’URL, du corps et des identifiants du fournisseur.

Corps maximal 1 Mio, ligne dépliée 4096 octets, 512 événements, identifiants 512 octets,
4096 occurrences et 3660 pas d’expansion au plus. Un dépassement ou une règle temporelle
inconnue invalide l’instantané entier : on ne fabrique pas une disponibilité partielle.
Les titres, descriptions, participants et URL embarquées ne deviennent pas des points.
Les identités publiques dérivent du hachage du UID et, pour une série, de son instance.

Le sous-ensemble initial comprend DTSTART/DTEND, UTC, TZID IANA et DATE ; les heures
flottantes utilisent le fuseau explicitement configuré. DATE respecte les jours civils
locaux, sans supposer une durée de 24 h. Les heures locales ambiguës ou inexistantes
ne sont jamais corrigées silencieusement pour élargir un accès. Les bornes des dates seules restent minuit local, conformément au fichier ; aucun
décalage implicite ne modifie un droit.
RRULE minimal : DAILY/WEEKLY, INTERVAL, COUNT ou UNTIL, BYDAY hebdomadaire ; EXDATE,
RDATE, STATUS:CANCELLED et exceptions RECURRENCE-ID. Une option non prise en charge
est refusée. Référence : [RFC 5545](https://www.rfc-editor.org/rfc/rfc5545.html).

## Cycle et reprise

Les états `no_reservation`, `awaiting_checkin`, `checked_in`, `checked_out` décrivent
uniquement le calendrier. Les cinq réservations affichées ont des identités stables,
des bornes et un état d’accès ; aucune création de personne ou de présence automatique.
Une réservation annulée ou disparue perd sa fenêtre à la prochaine lecture valide.
Un échec de lecture ferme les fenêtres ; aucune réponse périmée ne prolonge un droit.

Le démarrage et chaque reprise commencent par un calcul d’état sans émission d’arrivée.
Les événements ne concernent qu’une borne future déjà connue qui est franchie pendant
le fonctionnement observé. Un intervalle sauté pendant un arrêt n’est jamais rejoué.
Un recul d’horloge ne rejoue pas une borne déjà observée. Le pilote ne possède aucun
client de serrure et ne passe aucune commande au hub. Une ouverture éventuelle relève
exclusivement d’un graphe validé par un humain, avec l’origine automation habituelle.

## Vérification

Tests purs à horloge fixe pour UTC, changements d’heure, heures impossibles, dates seules,
RRULE/EXDATE, annulations, chevauchements et données trop grandes. Tests du vrai hub pour
la reprise sans événement, l’expiration, la révocation et l’absence de commande physique.
Tests HTTP des invités avant/au début/avant/à la fin, avec une session déjà ouverte.
Contrôles natifs Rust 1.99.1 en deux jobs ; avant fusion, un contrôle complet à la fois
dans la file d’intégration, puis catalogue, fuites et web/i18n si touché. Les quatre cas profile propres à Linux sont facultatifs et D13 est exclu de ce lot
qui ne touche pas moli-archive, conformément aux décisions du propriétaire.
