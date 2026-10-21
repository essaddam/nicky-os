# Réservations ICS et accès invités

`moli-reservations` lit un export ICS HTTPS **choisi par un humain**, au coffre de
l’instance sous `url`. Configuration et mise en route :
[integrations/reservations/onboarding.md](../../integrations/reservations/onboarding.md).
Aucun calendrier n’est découvert ou choisi par un agent. Aucun titre, participant,
UID fournisseur ou URL embarquée ne devient un point de la maison.

## Calendrier et cycle

Un appareil calendrier, cinq emplacements de séjours courants/futurs triés par début.
Les bornes sont des millisecondes Unix. Le cycle distingue `no_reservation`,
`awaiting_checkin`, `checked_in` et `checked_out`, **selon les horaires prévus seulement**.
Il ne modifie pas la présence, le mode de la maison, l’alarme, le ballon ou une serrure.

`arrival` et `departure` sont des occurrences portant une référence opaque de séjour.
Le démarrage et le retour après panne recalculent l’état sans rejouer les occurrences
manquées. Une interruption d’observation de plus de deux secondes et un recul d’horloge
suppriment également le rattrapage. Il est donc possible de lire une fenêtre active sans
recevoir une occurrence d’arrivée : cela évite une ouverture tardive après redémarrage.

Le sous-ensemble RFC 5545 et ses limites sont décrits dans le
[paquet](../../integrations/reservations/README.md). Une heure ambiguë/inexistante ou une
règle inconnue invalide tout le calendrier. Les dates seules gardent les jours civils et
minuit local, y compris au changement d’heure. Aucun décalage d’arrivée/départ n’est ajouté.
Les fixtures sont synthétiques ; aucune compatibilité de fournisseur réel n’est qualifiée.

## Liaison humaine d’un invité

Dans **Personnes**, un propriétaire peut associer un invité à une réservation. La liaison
s’ajoute aux pièces autorisées et à sa date d’expiration existante ; elle ne crée aucun
compte ou droit. Sans liaison, la politique invitée existante est conservée. Un propriétaire
ou membre ne conserve jamais de liaison de réservation.

L’API des personnes accepte, en option :

```json
{
  "name": "Invité exemple",
  "role": "guest",
  "rooms": ["Pièce exemple"],
  "reservation": {
    "instance": "sejours",
    "id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  }
}
```

L’identifiant est la référence opaque publiée par le pilote, **pas le UID privé du fichier**.
La sélection dans l’interface propose les cinq fenêtres publiées. Une liaison devenue
indisponible est conservée et ferme l’accès ; la retirer exige une action humaine explicite.
Un champ absent lors d’une modification conserve la liaison existante. Le champ
`reservation: null` enlève la restriction de calendrier, avec les mêmes droits
humains que toute modification d’une personne. Les comptes existants restent compatibles.

Le hub n’admet une liaison que si son propre pilote calendrier est en marche, son instantané
est frais et `début <= maintenant < fin`. Les fenêtres ne sont jamais restaurées du cache.
Une annulation ferme l’accès à la prochaine lecture ; une erreur constatée, un pilote arrêté
ou un instantané expiré le ferme immédiatement. La fraîcheur maximale est de
`poll_seconds + 10` secondes, vérifiée par horloge murale et monotone. Une modification chez
le fournisseur reste naturellement inconnue jusqu’à la prochaine lecture.

Un invité peut préparer son mot de passe avant le séjour ; cela n’ouvre aucun accès à
la maison. Les sessions déjà ouvertes sont revérifiées à chaque requête. Une identité Access connue
mais inactive est refusée plutôt que traitée comme un visiteur Access non répertorié.
Les flux SSE vérifient les droits avant chaque donnée, première image comprise, et chaque
seconde au repos. Un ordre invité, direct ou d’ambiance, conserve une autorisation vivante :
le retrait de file, l’attente du transport et une approbation humaine en attente ne
contournent pas la fermeture de la fenêtre. Les luminaires composés transmettent cette
autorisation à leurs seuls membres déclarés, en vérifiant toujours le droit sur le
luminaire parent ; la reprise différée après remise sous tension conserve ce droit.
Un transport détaché conserve le point et la même politique dans son `SendPermit` ;
`wanted()` et `begin_send()` la revérifient avant tout premier envoi, même si la
tâche du pilote n’a pas encore repris. Après l’émission, le suivi du résultat et le
journal demeurent indépendants de la survie de l’appelant.

## Coordination facultative avec une serrure

Le pilote n’a aucun client de serrure et n’envoie aucun ordre. Si le propriétaire souhaite
coordonner une serrure, il crée puis valide explicitement un automatisme sur l’occurrence
avec l’origine `automation`. L’état `checked_in` ne vaut pas une autorisation de produire
une ouverture après une panne. La garde `lock`, les pièces protégées et le journal restent
sur le chemin des commandes. Aucun effet physique n’a servi aux tests de cette brique.

HA reste propriétaire tant que le propriétaire n’a pas qualifié cette version sur sa
source réelle ; aucun transfert de propriété ni déploiement n’est effectué par ce lot.
