# Huawei LTE — HiLink en lecture seule

Crate native `moli-huawei-lte`, `kind = "huawei-lte"`. Les sessions, jetons et
variantes de login nécessitent un état natif (D2). [Installation](onboarding.md),
[cadrage](../../docs/components/reseau.md) et [qualification](../../docs/components/reseau-qualification.md).

## Repris

Connaissance du protocole public [huawei-lte-api f416c63](https://github.com/Salamek/huawei-lte-api/tree/f416c63d2a01d7e1d743a2d039ae2d1ff6b71df5)
(LGPL-3.0), notamment Session.py, api/User.py et enums/client.py/user.py.
Aucun code fournisseur copié ou dépendance Python intégrée. Les unités de trafic et
l'identité SerialNumber sont recoupées dans [HA core cbbd58b](https://github.com/home-assistant/core/tree/cbbd58b4ca6b657168339661a638136ce4387a02/homeassistant/components/huawei_lte)
(Apache-2.0). Aucun export privé ni capture réelle utilisé ; les sept fixtures sont synthétiques.

- Routes fixes : GET `/api/webserver/SesTokInfo` **ou** `/api/webserver/token`,
  choisi par `session` ; aucun fallback HTML/variante. Puis GET `device/information`,
  `monitoring/status`, `device/signal`, `monitoring/traffic-statistics`, sous `/api/`.
- `auth = "sha256-v4"` : GET `/api/user/state-login`, password_type4 requis ; State-1
  autorise l'unique POST `/api/user/login`, State0 réutilise la session sans forcer
  une connexion, autre état/type => humain. Preuve de login : base64 du SHA256
  hexadécimal de username + base64(SHA256 hexadécimal(password)) + jeton courant.
  Identifiants ASCII bornés, username échappé XML ; aucun mot de passe dans l'URL.
- `auth = "none"` : aucun login, aucune variante essayée et aucun POST. Un refus
  d'accès ne fait pas passer automatiquement à une authentification.
- Cookies locaux à l'hôte, <=8 /4 Kio cumulés ; jetons ASCII <=1 Kio, file <=2,
  remplacée à la rotation single/one/two, doublons ou mélange ambigu refusés.
  Un GET transmet le jeton seulement si la file en contient un ; une paire reste
  disponible pour POST. Aucun cookie jar partagé ni persistance de session.
- Une requête en vol, réponse <=1 Mio avant parsing, 5 s/échange, 30 s et16 envois
  au plus par cycle, authentification comprise. XML : profondeur16, 10000 éléments,
  textes/attributs <=4 Kio, doublons directs refusés, aucun DTD/XXE/entité déclarée.
- Les codes XML sont interprétés même sous HTTP200/500 ; un refus108xxx/115002
  reste terminal sous401. HTTP403/423/429 terminaux. Expiration100003/125001–3 :
  un rétablissement de session, puis rejeu de la **seule lecture interrompue**.
  Un refus/type inconnu/changement d'identité ou certificat attend un humain.
- TLS épinglé et empreinte persistée **avant** session/identifiants ; identité native
  SerialNumber exigée, au coffre avant publication. Pas de repli IP/IMEI/nom.
  Un seul modem est admis par instance, identités déjà connues/cache comprises :
  aucune accumulation sur une relance avec un autre équipement, aucune suppression.
- Périodes60 s et3600 s pour l'identité, bornées30–86400 s, sans rattrapage. Panne
  réseau => superviseur/backoff ; arrêt annule les échanges et efface les valeurs.

## Points et conservation B1

Au plus **11 points**, aucun client découvert. Tous sont RO, `Diagnostic` pour
la santé propre du modem, sauf les quatre signaux `SignalStrength` ; **tous classés
Family::Diagnostic**, prouvé avec le classificateur réel de moli-history. Les
politiques/durées/filtres configurés par l'humain restent inchangés.

| Point | Source et unité |
|---|---|
| `connected` | ConnectionStatus901 : vrai ; 902/904 : faux ; transition/inconnu : Null |
| `technology` | CurrentNetworkType, code HiLink brut ; aucune équivalence radio devinée |
| `rssi`, `rsrp` | suffixe dBm requis |
| `rsrq`, `sinr` | suffixe dB requis |
| `rx_bytes`, `tx_bytes` | CurrentDownload/CurrentUpload, en B |
| `rx_rate`, `tx_rate` | CurrentDownloadRate/CurrentUploadRate, en B/s, jamais multipliés par8 |
| `duration` | CurrentConnectTime, en s |

Les compteurs u64 restent exacts en interne. Une valeur >2^53−1 n'est pas publiée
comme un entier prétendument exact dans le navigateur : elle reste inconnue. Si le
routeur ne fournit pas de débit publiable, différence entière vérifiée entre
compteurs compatibles, divisée par le temps monotone des relevés. Baisse de
compteur/durée, durée absente, trou, arrêt ou session renouvelée => nouvelle
référence, aucun débit négatif ni pic à travers la rupture.

SIM absente : connexion fausse, signal/trafic non interrogés et anciennes valeurs
inconnues. Sans connexion de données prouvée, pas de trafic interrogé/publié comme
frais ; le signal peut rester lisible. Une valeur absente, sentinelle, censurée
(`<`/`>`) ou sans unité radio attestée reste inconnue. Pas de zéro inventé ni point
pour une capacité jamais observée. Cycle entièrement validé avant publication ;
valeurs inchangées sans nouvel événement. Aucune présence humaine n'est déduite.

## Deviné et reste

Les tests qualifient les contrats synthétiques, pas le matériel : firmware, support
des routes, modes d'authentification, unités et charge réelle restent à qualifier
après H1. Aucun essai d'autres password_type, authentification RSA/challenge,
modèle 5G spécial ou page HTML pour contourner une incompatibilité. Aucun SMS,
identifiant SIM/cellule, numéro, opérateur, administration, logout, reboot, scan,
ping ou test de débit. HA reste propriétaire par usage jusqu'à validation humaine.
