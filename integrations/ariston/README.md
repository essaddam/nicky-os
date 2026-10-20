# Ariston (chauffe-eau Velis)

Pilote natif `moli-ariston` : les chauffe-eau Velis d'un compte **Ariston NET**, par le cloud
Ariston (`www.ariston-net.remotethermo.com`, adresse fixée dans le pilote). Secrets :
`username`, `password` du compte (humain).

Un chauffe-eau n'est pas un relais : il a une température d'eau, une consigne et celle qu'il
applique vraiment, un mode (manuel ou sa propre programmation), un mode éco, un cycle
anti-légionelle et un plafond de consigne. Le pilote les lit tous et confirme chaque ordre
par une relecture.

## Repris

- La bibliothèque Python `ariston` 0.19.9 (MIT), qu'utilise l'intégration HA personnalisée
  « ariston », lue, pas recopiée : connexion (`accounts/login`, jeton dans l'en-tête
  `ar.authToken`, 405 = jeton expiré, 429 = quota), liste (`velis/plants` : `gw`, `sys` = 4,
  `wheType` 6 = Velis Evo 2 et 5 = Andris 2), état (`velis/medPlantData/{gw}` : `on`, `mode`
  1 = manuel et 5 = programmation, `temp`, `reqTemp`, `procReqTemp`, `eco`, `heatReq`,
  `antiLeg`, `avShw`, `rmTm`), réglages (`…/plantSettings` : `MedMaxSetpointTemperature` et
  ses bornes, `MedAntilegionellaOnOff`), erreurs (`busErrors`), ordres (`…/switch`, `…/mode`,
  `…/temperature`, `…/switchEco`, `…/plantSettings` avec l'ancienne valeur).
- Les 12 entités qu'avait la maison de référence : chauffe-eau (température, consigne,
  mode), marche, éco, anti-légionelle, cycle en cours, en chauffe, consigne maximale,
  douches, temps restant, consigne appliquée, erreurs, énergie (voir « Reste »).

## Ce que fait le pilote

- Points : `on` (marche), `mode` (`manuel`, `programme`), `setpoint` (40 à 80 °C, pas de 1,
  et jamais au-dessus du plafond réglé sur l'appareil : refusé avant l'envoi, avec la
  raison), `eco`, `max_setpoint` (bornes données par l'appareil), `antilegionella` ; en
  lecture `temperature`, `effective_setpoint`, `heating`, `showers`, `remaining` (min),
  `antilegionella_cycle`, `errors` (nombre d'erreurs signalées).
- Sémantique : couper l'eau chaude (`on`) et couper la protection anti-légionelle sont des
  commandes (`control`) : un agent le demande à un humain. Consigne, mode, éco et plafond
  sont des réglages (`config`) ; la garde des pièces protégées s'applique à tous.
- Relevés : état toutes les 3 min (toutes les 20 s, réglages compris, tant qu'un ordre attend
  sa confirmation), réglages et erreurs toutes les 30 min, liste des chauffe-eau toutes les
  6 h ; trois relevés ratés de suite relancent la liaison (appareil hors ligne entre-temps).
- Ordres : **une seule fois** (une connexion par requête, jamais rejouée, ni par Moli ni après
  une nouvelle connexion), un à la fois dans l'ordre reçu, au plus 4 appelants en attente,
  jamais après leur instant limite (il faut au moins une seconde devant soi pour partir).
  Les intentions sont recomposées et le plafond est revérifié au départ. Après une
  modification de `plantSettings`, les ordres qui dépendent des réglages attendent
  une lecture commencée après l’acquittement et confirmant la valeur demandée ;
  les couples `old`/`new` utilisent cette lecture. Cette attente conserve la file,
  l’abandon et l’échéance : un ordre expiré n’est jamais libéré plus tard.
  L'appelant entend « fait » quand le cloud a pris l'ordre ; ce n'est pas celui du
  chauffe-eau : la valeur demandée doit se relire dans les 3 min, sinon la maison est
  prévenue (notice). Pas de réponse à temps : « peut-être parti, jamais renvoyé », et Moli
  vérifie quand même à la relecture.
- Session : gardée à travers les redémarrages du pilote ; un jeton refusé (401, 405) la fait
  renouveler, trois sessions refusées de suite ou des identifiants refusés mettent le pilote
  en attente d'un humain (`waiting`), appareil hors ligne, sans réessayer en boucle. Quota
  (429) ou service indisponible : relevés et ordres suspendus 15 min. Les refus ne sont jamais
  cités, seulement leur code HTTP.
- Entrées bornées : réponses 1 Mio, 4 chauffe-eau au plus, noms 64 caractères sans caractère
  de contrôle, identifiant de passerelle alphanumérique (32 au plus) vérifié avant d'entrer
  dans un chemin, jeton de session borné ; une réponse qui n'est pas une liste ne vide jamais
  la maison ; une valeur inconnue (un autre mode) est montrée telle quelle, jamais devinée.

## Volontairement différent de la bibliothèque Python

- **Aucun rejeu.** Elle renvoie toute requête ratée après 5 s, ordres compris, et se
  reconnecte puis renvoie après un 405 : Moli ne renvoie jamais un ordre ; un jeton expiré
  sur un ordre est rendu à l'appelant (« l'ordre n'est pas parti, réessayer »).
- **Pas de valeur supposée.** Elle écrit la valeur demandée dans son état dès l'envoi : Moli
  n'affiche que ce que le chauffe-eau relit.

## Deviné

- Les fixtures sont **reconstruites** d'après les champs que lit la bibliothèque, avec un
  identifiant de passerelle et un nom fictifs, pas capturées : aucun accès au compte avant le
  jalon humain H1. À remplacer par des captures réelles anonymisées à la qualification.
- La cadence (3 min) suit celle de l'intégration HA ; la tolérance du cloud au rythme de suivi
  (20 s pendant 3 min au plus) reste à vérifier sur le compte réel.

## Reste

- Qualification sur le compte réel (H1, humain présent, état remis à l'identique).
- L'énergie (`consSequencesApi8`) : la bibliothèque n'en garde que la dernière valeur d'une
  série journalière, présentée comme un total ; à reprendre proprement avec le module énergie,
  pas recopiée.
- Les autres modèles Velis (Lydos, Nuos, Lux, Evo première génération) et les chaudières
  (`remote/plants`) : d'autres routes, non vues dans les maisons de référence.
- Vacances (`holiday`), nombre de douches demandé (`showers`), vitesse de chauffe : non
  écrits. Une carte « chauffe-eau » dans le tableau de bord (le point s'appelle `setpoint`
  et non `target_temperature`, pour ne pas tomber dans la carte des climatiseurs).
