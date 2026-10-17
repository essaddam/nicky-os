# Somfy TaHoma et Connectivity Kit (Overkiz)

Pilote natif `moli-overkiz` : une box Somfy, par l'une de ses deux routes.

| Route | Pour | Secrets |
|---|---|---|
| `transport = "local"` | une TaHoma en **mode développeur** (API locale officielle, HTTPS 8443) | `token` (humain), `cert_sha256` (épinglé par le pilote au premier contact, avant tout envoi du jeton) |
| `transport = "cloud"` | un **Connectivity Kit** (aucune API locale), ou une box sans mode développeur | `username`, `password` du compte Somfy (humain) |

## Repris

- API locale : la spécification OpenAPI publiée par Somfy (`Somfy-Developer/Somfy-TaHoma-Developer-Mode`) :
  `/setup/devices`, `/events/register`, `/events/{id}/fetch`, `/exec/apply`.
- Cloud : serveur Somfy Europe (`ha101-1.overkiz.com`), session OAuth de `accounts.somfy.com`
  (mot de passe, puis jeton de renouvellement) avec le client public de l'appli Somfy, et la
  table des messages d'erreur, d'après pyoverkiz (MIT). Aucun code recopié.
- Sémantique : un moteur **RTS** est en radio sens unique, il ne dit jamais où il est (HA le
  marque « état supposé ») : Moli ne publie aucune position pour lui, seulement les ordres.

## Ce que fait le pilote

- Appareils publiés : seulement les classes vues dans les maisons de référence (liste
  `models` du manifeste). Volet et portail : `control` (`open`, `stop`, `my`, `close`, selon ce
  que l'appareil déclare ; `my` = position favorite du moteur), écriture seule ; portail OGP :
  `opening` en lecture s'il le dit. Lumière : `on`, `brightness` (%). Prise : `on`.
- Un ordre part **une seule fois** (`send_once` en local, une connexion par requête dans le
  cloud, jamais rejouée), dans l'échéance du hub. Il est suivi par les événements
  d'exécution : l'appelant entend « fait » (`COMPLETED`), l'échec (`FAILED` et sa cause), ou
  « accepté » si la box travaille encore quand le hub cesse d'attendre ; un échec après
  « accepté » est dit à la maison (notice). Liaison perdue : l'appelant entend « envoyé, non
  confirmé », rien n'est renvoyé.
- Écoute : un auditeur d'événements enregistré avant le chargement, relevé toutes les 10 s
  en local, 30 s dans le cloud, chaque seconde (2 s cloud) tant qu'une exécution est suivie ;
  auditeur expiré : réenregistré et tout rechargé. Quota, file pleine, maintenance : pause de
  60 s. Session cloud renouvelée 2 min avant sa fin, gardée à travers les redémarrages du
  pilote (Somfy bloque les comptes qui se reconnectent trop) ; identifiants ou jeton refusés :
  statut `waiting` qui dit quoi faire, sans réessayer en boucle.
- Entrées bornées : réponses 2 Mio, noms 64 caractères sans caractère de contrôle, messages
  d'erreur 120, identifiants vérifiés avant d'entrer dans un chemin.

## Deviné

- Les fixtures sont **reconstruites** d'après la spécification et la forme des réponses de
  pyoverkiz, avec des identifiants fictifs, pas capturées : aucun accès aux box avant le
  jalon humain H1. À remplacer par des captures réelles anonymisées à la qualification.
- Les états du portail OGP (`core:OpenClosedState` et variantes) : déclarés si l'appareil les
  annonce, jamais supposés.

## Reste

- Qualification sur les box réelles (H1, humain présent, cible neutre, jamais une pièce
  protégée, état remis) : API locale de la TaHoma, cloud du Connectivity Kit.
- Pas de regroupement des rafales d'ordres (la box refuse au-delà de sa file : refus
  « occupée » rendu à l'appelant).
- Pas de pièce native (les `placeOID` de la box) : la famille range les appareils.
- Pas de scénarios, chauffage, capteurs, ni des autres classes Overkiz (non observés).
