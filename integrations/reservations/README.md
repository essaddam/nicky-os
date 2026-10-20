# Réservations ICS

Pilote natif en lecture seule, `crates/moli-reservations`, sur l’URL HTTPS placée au coffre
par un humain. Aucun client de serrure, aucune commande directe, aucun calcul par LLM.
Une réservation n’est jamais un constat de présence.

- Six appareils au maximum : calendrier et cinq prochaines fenêtres, identités de slots
  stables. Les références de réservation sont des hachages opaques ; titre, UID fournisseur,
  participants et liens ne sont pas publiés.
- `calendar/state` : `no_reservation` (aucun séjour), `awaiting_checkin` (un séjour futur),
  `checked_in` (au moins une fenêtre courante), `checked_out` (séjours connus tous terminés).
  Ce sont des états **planifiés**, pas une confirmation d’arrivée physique.
- `calendar/arrival` et `calendar/departure` : occurrences portant la référence opaque.
  Seules les bornes déjà connues franchies pendant l’observation sont émises. Démarrage,
  reconnexion, horloge reculée et interruption de plus de deux secondes réconcilient l’état
  sans rejouer les ouvertures manquées. Une fenêtre peut donc être active sans arrivée émise.
- Une liaison humaine d’un invité impose `[début, fin)` à ses sessions et à chaque ordre
  jusqu’à l’émission. Les flux vérifient les droits avant toute donnée et périodiquement
  au repos. L’instantané du hub est éphémère, ne revient jamais du cache et expire au plus
  `poll_seconds + 10` secondes après lecture (horloge murale et monotone).
- Coordination serrure facultative : un humain crée et valide un automatisme déclenché
  par l’occurrence, exécuté avec l’origine `automation`. La garde `lock` reste intacte.

## Contrat d’entrée

Corps ≤ 1 Mio, ligne dépliée ≤ 4096 octets, 512 événements, UID ≤ 512 octets,
4096 occurrences et 3660 pas d’expansion. UTC, TZID IANA et DATE ; les heures flottantes
emploient le fuseau configuré. Les jours DATE suivent les jours civils, minuit local,
y compris au changement d’heure. Heures ambiguës/inexistantes refusées, jamais élargies.

Récurrences DAILY/WEEKLY avec INTERVAL, COUNT ou UNTIL ; BYDAY hebdomadaire, EXDATE,
RDATE, versions SEQUENCE, annulations et exceptions RECURRENCE-ID. Horizon de 366 jours
avant/après le calcul. Une règle inconnue (dont MONTHLY, BYSETPOS, WKST), un DURATION,
un doublon contradictoire ou un dépassement invalide le calendrier entier. Échanges
HTTPS bornés à dix secondes, sans redirection ni compression ; aucun chargement des liens
embarqués. Les erreurs ne contiennent ni URL ni contenu du calendrier.

## Preuves et limites

Fixture synthétique construite pour le contrat, jamais extraite d’un vrai calendrier.
Tests du parseur, du client HTTP local, du pilote/hub et des sessions invitées ;
aucune qualification de fournisseur réel, commande physique ou migration de propriété HA.
Voir [la mise en route](onboarding.md) et [le contrat](../../docs/components/reservations.md).
