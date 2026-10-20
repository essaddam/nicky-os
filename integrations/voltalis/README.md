# Voltalis

Pilote natif `moli-voltalis` : les appareils d'un compte Voltalis, par le cloud Voltalis
(`api.myvoltalis.com`, adresse fixée dans le pilote). Secrets : `username`, `password` du
compte (humain).

## Repris

- Le client HA personnalisé « voltalis » (et sa bibliothèque `aiovoltalis` embarquée), lu,
  pas recopié : connexion (`/auth/login`, jeton porteur), site par défaut
  (`/api/account/me`), appareils (`managed-appliance`), réglages manuels (`manualsetting`),
  joignabilité (`autodiag`), programmes de la maison (`programming/program`) et programmes
  prêts à l'emploi (`quicksettings`), préréglages (`ECO`, `CONFORT`, `TEMPERATURE`,
  `HORS_GEL`, `NORMAL`), bornes de consigne 7 à 24 °C.
- Un ordre à un radiateur est son **réglage manuel entier** (`PUT manualsetting/{id}`),
  composé au départ depuis la dernière lecture et les réglages précédents acceptés.
  Cette projection interne ne devient jamais une mesure publiée. Une réponse incertaine
  ou un changement de programme exige un nouveau relevé complet avant un PUT dépendant.

## Ce que fait le pilote

- **Radiateur** (`HEATER`) : `mode` (`programme` = retour à son planning, `manuel`,
  `arret`), `preset` (`eco`, `confort`, `temperature`, `hors_gel`, `normal`), `setpoint`
  (7 à 24 °C, pas de 0,5), et en lecture `heating` (chauffe autorisée) et `program` (le
  programme en cours). Voltalis ne mesure pas la température des pièces : Moli n'en publie
  aucune.
- **Chauffe-eau** (`WATER_HEATER`) : `setpoint`, `heating`, `program`, **en lecture
  seulement**.
- **Programme** : un appareil par programme, point `enabled`.
- Sémantique : `mode`, `preset`, `setpoint` écrivables et `enabled` sont des commandes
  (`control`). Une consigne ou un préréglage active aussi le régime manuel : la garde
  existante juge cet effet avant le PUT selon l’origine et le contexte. Les droits de
  l’interface authentifiée et des automatismes validés restent ceux du hub ; le
  chauffe-eau reste en lecture seule.
- Relevés : appareils et programmes toutes les 2 min (toutes les 10 s tant qu'un ordre attend
  sa confirmation), joignabilité toutes les 10 min, tout (site, réglages manuels) toutes les
  30 min ; trois relevés ratés de suite relancent la liaison (appareils hors ligne entre-temps).
- Ordres : **une seule fois** (une connexion par requête, jamais rejouée, ni par Moli ni après
  une nouvelle connexion), un à la fois dans l'ordre reçu, au plus 4 appelants en attente,
  jamais après leur instant limite (il faut au moins une seconde devant soi pour partir).
  L'appelant entend « fait » quand le cloud a pris l'ordre ; ce « fait » n'est pas celui du
  radiateur : la valeur demandée doit se relire dans les 2 min, sinon la maison est prévenue
  (notice), même en pause quota ; une fermeture de liaison signale les attentes restantes.
  L’abandon et l’échéance sont revérifiés après login, TCP et TLS, avant émission.
  Une fois la requête transférée à HTTP, sa réponse bornée est conservée. Pas de réponse à temps : « peut-être parti, jamais renvoyé », et Moli vérifie quand
  même à la relecture.
- Session : gardée à travers les redémarrages du pilote ; un jeton refusé (401) la fait
  renouveler, trois sessions refusées de suite ou des identifiants refusés mettent le pilote
  en attente d'un humain (`waiting`), appareils hors ligne, sans réessayer en boucle. Quota
  (429) ou service indisponible (502 à 504) : relevés et ordres suspendus 10 min. Les refus ne
  sont jamais cités, seulement leur code HTTP.
- Entrées bornées : réponses 1 Mio, 32 appareils et 32 programmes au plus, noms 64 caractères
  sans caractère de contrôle, identifiants numériques vérifiés avant d'entrer dans un chemin,
  jeton de session borné ; une réponse qui n'est pas une liste ne vide jamais la maison.

## Volontairement différent du client HA

- **Pas d'extinction du chauffe-eau.** Le client HA déclare un chauffe-eau, mais toutes ses
  commandes lèvent `NotImplementedError` : un « éteindre » (par exemple appelé par un mode
  Absent) ne faisait rien. Aucune route documentée ne l'éteint : Moli ne publie aucune
  écriture pour lui et ne reproduit pas ce faux interrupteur.
- **Aucun rejeu.** Le client HA se reconnecte puis renvoie la requête après toute erreur, y
  compris un `PUT` : Moli ne renvoie jamais un ordre ; un 401 sur un ordre est rendu à
  l'appelant (« l'ordre n'est pas parti, réessayer »), la session est renouvelée au relevé
  suivant.
- **`manuel` rallume.** Passer de « arrêt » à « chauffe manuelle » gardait `isOn = false`
  dans le client HA (le radiateur restait éteint) : Moli envoie `isOn = true`.
- **Le « OK » du cloud n'est pas une confirmation** : la valeur doit se relire.
- **Moins de requêtes** : une liste par relevé au lieu d'une requête par appareil chaque
  minute.

## Deviné

- Les fixtures sont **reconstruites** d'après les champs que lit le client HA, avec des
  identifiants et des noms fictifs, pas capturées : aucun accès au compte avant le jalon
  humain H1. À remplacer par des captures réelles anonymisées à la qualification.
- `mode` d'un radiateur qui suit un programme prêt à l'emploi (`progType` autre que `MANUAL`
  ou `USER`) : `programme` (le client HA le disait « chauffe »).
- Revenir au planning renvoie le réglage manuel avec `enabled = false`, comme le client HA.
- La durée de vie du jeton n'est pas documentée : il est renouvelé au premier refus.

## Reste

- Qualification sur le compte réel (H1, humain présent, cible neutre, jamais une pièce
  protégée, état remis à l'identique).
- Une carte « radiateur » dans le tableau de bord (le point s'appelle `setpoint` et non
  `target_temperature`, pour ne pas tomber dans la carte des climatiseurs et de leurs modes).
- Réglages manuels à durée limitée (`endDate`), plannings eux-mêmes, consommation par
  appareil : non lus.
