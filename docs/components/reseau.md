# Intégrations réseau : contrat de lecture proposé

État : cadrage relu côté architecture ; aucun de ces clients n'est implémenté par ce document.
Les lots restent ouverts jusqu'à leurs tests de contrat, leur revue et leur qualification.
Les choix ci-dessous appliquent [D2 et D5](../ARCHITECTURE.md) sans changer la propriété
des usages existants. Aucun accès à un appareil réel n'est nécessaire pour ce cadrage.

## Résultat attendu et limites

Lire la santé des équipements réseau, leurs compteurs utiles et un inventaire sélectionné.
Un inventaire découvert n'entraîne jamais automatiquement la création de points. Aucun
client ne publie de point `occupancy` et aucun automatisme de présence n'est modifié.
Une association Wi-Fi, un bail DHCP, une entrée ARP et la présence d'une personne ont des
sens différents ; l'absence d'un client dans une réponse en erreur signifie « inconnu ».

Pas de redémarrage, changement de configuration, blocage de client, activation de compteur,
test de débit, mise à jour, scan radio, SMS ni commande radio. Les authentifications
créent une session technique ; elles ne donnent accès à aucune commande d'administration.
Une éventuelle extension d'écriture demandera un autre contrat et passera par le hub.
HA conserve ses usages jusqu'à une validation humaine de la reprise, usage par usage.
La radio des coordinateurs reste exclusivement à Zigbee2MQTT.

## Choix de transport

| Lot | Forme proposée | Motif |
|---|---|---|
| N0, vocabulaire | extension additive du vocabulaire des points et de B1 | classer les diagnostics techniques sans leur attribuer une sémantique fausse |
| N1, ASUSWRT | `moli-asuswrt`, paquet `integrations/asuswrt/` | session authentifiée HTTP(S), requêtes strictement sélectionnées |
| N2, Huawei LTE | `moli-huawei-lte`, paquet `integrations/huawei-lte/` | XML HiLink, renouvellement de session et de jetons |
| N3, Netgear | `moli-netgear`, paquet `integrations/netgear/` | SOAP authentifié, variantes de réponses et capacités facultatives |
| N4, SMLIGHT | profil `integrations/smlight/profile.toml` | deux lectures JSON fixes, uniquement lorsqu'elles sont publiques |

Un moteur générique de connexion, du SOAP arbitraire dans les profils ou un parseur de
scripts ne sont pas nécessaires. Les protocoles à session restent dans leurs crates.
ASUSWRT HTTP(S) est le premier périmètre proposé ; SSH/Telnet ne sont pas un repli
automatique. Un matériel incompatible reste à qualifier et sous son propriétaire actuel.

## N0 : diagnostic et unités, avant les clients

Le socle N0 fournit `Semantic::Diagnostic`, sérialisé `diagnostic`, et son classement
dans `Family::Diagnostic`. `SignalStrength` garde son classement diagnostic existant ;
un nombre `Other` en octets ou octets par seconde reste `other`. Les unités et une simple
étiquette ne remplacent donc jamais une sémantique explicitement déclarée.
Une règle `storage.point` sait régler la durée, pas changer la famille.

Contrat approuvé par l’architecte à la revue du cadrage, plan N0 relu avant implémentation.
`Diagnostic` décrit la santé technique de l’équipement lui-même : charge, mémoire, durée
de fonctionnement, trafic ou température de puce, avec une unité indépendante. Une mesure
de l’environnement garde sa sémantique physique, même venant d’un équipement réseau.
B1 classe explicitement cette nouvelle sémantique dans `Family::Diagnostic`, avant
l'inférence par unité. Les sémantiques existantes conservent leur classement ; le signal
reste `SignalStrength`. Aucune inférence globale fondée sur des noms tels que `rx`.

Les unités `B`, `B/s` et `Mbit/s` passent par `Unit::Other`, déjà sérialisable, et sont
acceptées explicitement par le validateur des profils. `s`, `%`, `°C`, `dB`
et `dBm` gardent leur sens. Un compteur réseau n'est jamais de l'énergie (`Wh`/`kWh`).
Vérifier les consommateurs Rust et web de `Semantic`, la compatibilité des données
existantes, les traductions fr/en nécessaires et les validations de profil. Ne changer
ni les durées, ni les exclusions, ni les planchers, ni la garde de conservation.
La règle `battery_voltage` de B1 reste pour la compatibilité des points existants.
La politique configurée par l'humain continue de primer ; aucune purge n'est déclenchée
par l'installation d'un client. Ce lot est indépendant de la migration du stockage.

## Contrat commun des clients natifs

- Options fermées : hôte et port fournis par l'humain, transport explicite, période de
  lecture et sélection des clients. Refuser URL complète, chemin libre, paramètres
  arbitraires, identifiants dans l'adresse, port nul et clé inconnue. L'agent ne choisit
  ni hôte ni URL. Aucun suivi de redirection ni d'URL contenue dans une réponse.
- HTTPS local : `PinnedHttps`, poignée de main et empreinte dans le coffre avant l'envoi
  d'un identifiant. Aucun repli silencieux vers HTTP. Si le modèle impose une
  authentification en clair, exiger une option humaine explicite `allow_plain_auth`,
  fausse par défaut, et expliquer la portée LAN dans l'onboarding. Pas de secret dans
  le TOML, les journaux ou les erreurs. Ne jamais régénérer la clé maître.
- Identifiant et mot de passe : noms de secrets fixes, saisis par l'humain dans le coffre
  de l'instance ; un secret manquant produit `waiting`. Les sessions et jetons restent
  en mémoire, ne sont pas des points et disparaissent à l'arrêt.
- Bornes proposées, à coder et tester : réponse complète au plus 1 Mio avant décodage,
  5 s par échange, 30 s par cycle, une requête en vol par instance, 16 échanges au plus
  par cycle (authentification comprise), aucune file de lectures différées.
  Session/cookies : 8 valeurs au plus, 4 Kio cumulés ; file de jetons : 2 valeurs,
  1 Kio chacune. Refuser CR/LF, caractères de contrôle et valeurs trop longues avant
  tout en-tête. Cookies limités au seul hôte déclaré, jamais de jar partagé.
- XML : profondeur 16, 10 000 éléments, texte de champ au plus 4 Kio ; refuser DTD,
  entités déclarées/externes et document tronqué. Ne jamais rechercher une URL pour
  résoudre une entité. Les préfixes de namespaces SOAP ne constituent pas une identité.
- Un seul rétablissement de session par cycle, puis retour au superviseur/backoff ;
  refus d'identifiants ou verrouillage : `waiting`, sans boucle de login. Une capacité
  absente est mémorisée pour la session et ne provoque pas une rafale à chaque relevé.
- Période par défaut 60 s pour les mesures, 5 min pour l'inventaire, 1 h pour les
  informations stables. Plancher 30 s ; aucun rattrapage après une pause. Arrêt annulable,
  instance non configurée = aucune tâche. Seules les valeurs modifiées sont diffusées
  (`set_state`, jamais `pulse_state` pour ces mesures).
- Échec d'une lecture : ses points deviennent `Null`, sans fabrication de zéro ; échec
  global : retour au superviseur et appareils hors ligne. Les capacités absentes ne
  produisent pas de mesure. Refuser une commande reçue sans effectuer d'échange réseau.
- Les identités reposent sur une identité native stable, jamais un nom ou une adresse IP.
  Une identité requise absente ou remplacée bloque la publication ; aucun repli sur le
  nom, l'adresse ou le numéro d'ordre d'un client. Pas de rapprochement inter-pilotes.

### Inventaire et volume des points

L'inventaire reste un état interne borné à 512 clients, sans journal de réponses brutes,
sans résolution des noms ni connexion aux adresses découvertes. Un dépassement invalide
la lecture entière : pas de troncature interprétée comme une déconnexion.
Une sélection humaine par identités natives, dans la configuration privée, contient au
plus 32 clients ; vide par défaut. Seuls ces clients deviennent des appareils Moli.
Pas de point IP, MAC, nom d'hôte ou SSID répliquant l'inventaire. L'identité native utile
au `DeviceId` reste locale à l'installation ; aucune capture réelle n'entre au dépôt.

Budget proposé : au plus 16 points par équipement principal, 4 points par client
sélectionné, soit 144 points par instance au maximum. Ne créer que les points dont la
capacité est prouvée ; les champs réseau inconnus ne créent jamais un point dynamique.
Une association perdue peut faire passer `associated` à faux après un inventaire complet
réussi qui le prouve. Une table de baux ne prouve pas cette association : ne pas produire
ce point à partir d'un bail seul. Dans le doute, `Null`. Aucun point `presence`.

## N1 : ASUSWRT

Le client amont distingue l'authentification `login.cgi`, des lectures telles que
`appGet.cgi`/`update_clients.asp`, et les chemins de modification. Reprendre seulement
la connaissance du protocole, pas ses fonctions d'administration.
Les requêtes de lecture sont des constantes dont les paramètres sont figés et relus,
jamais une option `hook` ou une commande fournie par l'appelant.

Première tranche : identité du routeur, compte de clients, compteurs WAN et débits,
charge CPU, mémoire et température technique lorsque disponibles. Inventaire sélectionné :
association prouvée, type de lien, signal et débit de liaison, selon capacités.
Ne pas interpréter/exécuter le JavaScript renvoyé par certaines routes : JSON strict
uniquement dans ce premier lot ; autre variante = erreur de compatibilité explicite.
Un refus, un cookie expiré, une réponse HTML de login avec statut 200 ou un résultat
partiel ne valent pas une liste vide de clients. Aucun `apply*.cgi`, `netool.cgi`,
scan, ping ou shell distant. Les sessions de l'humain ne sont jamais déconnectées.
À la reprise, le client conserve les capacités déclarées ou en cache pour chaque
appareil/rôle, les mesures manquantes restent `Null`. Une ancienne valeur du cache
ne devient jamais fraîche, même si son groupe n'est pas relu au premier cycle.
Aucune capacité jamais observée n'est créée ; les mêmes identités et l'admission
conservatrice sont maintenues, au plus 139 points (11 routeur + 32 × 4 clients).

## N2 : Huawei LTE

Routes de lecture à vérifier sur les fixtures publiques :
`/api/webserver/SesTokInfo` (et variante `/api/webserver/token`),
`/api/device/information`, `/api/device/signal`, `/api/monitoring/status`,
`/api/monitoring/traffic-statistics`. L'authentification `/api/user/login` et ses
variantes doivent être liées à un contrat de firmware testé ; pas d'essai de toutes
les variantes contre un appareil. Variante inconnue = attente de qualification.

Traiter les erreurs XML même avec HTTP 200. Sérialiser les échanges qui consomment
les jetons ; remplacer la file à la rotation des en-têtes, sans accumulation. Un
relogin ne rejoue que la lecture interrompue, une fois au plus. Aucun autre POST.

Points initiaux proposés (au plus 12) : connexion de données, technologie d'accès,
RSSI, RSRP, RSRQ, SINR, octets reçus/émis, débit entrant/sortant et durée de connexion
selon disponibilité. Signal en `SignalStrength`, débit/compteurs/durée en `Diagnostic`.
Les compteurs sont des octets, les débits des `B/s` : vérifier chaque champ amont,
ne pas appliquer un facteur 8 par supposition. Valeur sentinelle/absente = `Null`.
Le client implémenté conserve à la reprise les capacités déjà déclarées ou en cache
pour la même identité (11 points au plus) : SIM absente ou méthode facultative absente
ne supprime pas leurs définitions. Aucun point jamais observé n'est créé. Une identité
ou une clé incompatible dans le cache bloque la publication.
Garder les entiers bruts sans débordement pour calculer les différences. Au-delà de
2^53−1, ne pas publier un faux compteur entier exact dans un `f64`.
Une baisse de compteur réinitialise la référence ; aucun débit négatif ni pic calculé
à travers une reconnexion. Pas de numéro, identifiant SIM/cellule, opérateur nominatif,
statistique SMS ou carnet d'adresses dans les points.

## N3 : Netgear

SOAP avec actions de lecture sélectionnées : informations équipement (`GetInfo`),
système (`GetSystemInfo`), lien Ethernet (`GetEthernetLinkStatus`), compteurs
(`GetTrafficMeterStatistics`) et inventaire (`GetAttachDevice`/`GetAttachDevice2`,
confirmés dans les constantes amont). La combinaison service/action
et l'enveloppe sont fixées dans le client. Authentification v1/v2 et cookie : contrats
distincts, sans recherche automatique de ports ni activation de service.

Ne jamais enclencher le compteur de trafic pour obtenir une mesure ; s'il est inactif,
pas de zéro présenté comme consommation. Une faute SOAP de méthode non supportée retire
la capacité facultative de la session. Vérifier les formats liste/XML et les doublons,
les champs vides, les séparateurs inattendus et l'identité des satellites. Aucun
speedtest, `Set*`, configuration de session de modification ou blocage de client.
Les points des clients suivent exactement la sélection et les preuves décrites plus haut.

## N4 : SMLIGHT

Le [paquet déclaratif smlight](../../integrations/smlight/README.md) implémente deux GET
publics fixes : `/ha_info` à1h et `/ha_sensors` à60s. Aucun secret, query, `[[write]]`,
fallback vers `/api`/`api2`, SSE, MQTT, socket ou commande radio. 401/403/redirection/
HTML de login invalident la variante sans désactiver la sécurité. Radio à Zigbee2MQTT.

Le constructeur conseille au plus une requête/s et documente HTTP sans HTTPS. Le profil
opte pour un intervalle1000ms : pause initiale, espacement de tous les départs et reprises,
un seul échange en vol (1Mio/5s). Les autres profils gardent leur comportement par défaut.

Identité `info.Info.MAC` obligatoire, chaîne MAC validée et normalisée, sans repli IP/nom.
La cible reste exclusivement celle fournie par l'humain : aucun suivi LAN, ARP ou adresse
apprise. Une autre identité connue/en cache bloque la publication ; chaque relecture
d'identité est vérifiée avant toute publication issue de cette réponse.

Sept liaisons fixes, explicitement Diagnostic : firmware, températures ESP32/puce Zigbee
(°C), uptime(s), mémoire brute sans unité, booléens Ethernet/Wi-Fi du coordinateur.
Celsius et secondes sont recoupés dans HAcore épinglé ; la base firmware de ram_usage
n'est pas prouvée, aucun pourcentage ni facteur vers B inventé. Champs multichip/tableaux
ignorés, aucune création par index. Définitions fixes conservées à la reprise, mesures
absentes Null (`missing = "null"`), cache froid invalidé avant les observations fraîches.
Arrêt/échec : Null/hors ligne. Pas de présence humaine déduite des booléens réseau.
La supervision HA et la qualification firmware/budget matériel restent séparées après H1.

## Provenance et inconnues

Sources publiques consultées le 18 octobre 2026 ; aucun code tiers copié, aucune réponse
privée recopiée. Ces sources prouvent des formes de protocole, pas la compatibilité d'un
firmware installé. Les fixtures futures doivent être synthétiques et marquées comme
telles, ou issues de captures publiques correctement anonymisées et licenciées.

| Source | Référence vérifiée |
|---|---|
| ASUSWRT | [AsusRouter, endpoints](https://github.com/Vaskivskyi/asusrouter/blob/a897f28e1e07229a16f3fb553035575fbf2b2328/asusrouter/modules/endpoint/__init__.py), [connexion](https://github.com/Vaskivskyi/asusrouter/blob/a897f28e1e07229a16f3fb553035575fbf2b2328/asusrouter/connection.py) |
| Huawei LTE | [session](https://github.com/Salamek/huawei-lte-api/blob/f416c63d2a01d7e1d743a2d039ae2d1ff6b71df5/huawei_lte_api/Session.py), [signal](https://github.com/Salamek/huawei-lte-api/blob/f416c63d2a01d7e1d743a2d039ae2d1ff6b71df5/huawei_lte_api/api/Device.py), [trafic](https://github.com/Salamek/huawei-lte-api/blob/f416c63d2a01d7e1d743a2d039ae2d1ff6b71df5/huawei_lte_api/api/Monitoring.py) |
| Netgear | [pynetgear, client](https://github.com/MatMaul/pynetgear/blob/d9b34149b110cd91fbccc5fff7a285a1221af5dd/pynetgear/router.py), [constantes](https://github.com/MatMaul/pynetgear/blob/d9b34149b110cd91fbccc5fff7a285a1221af5dd/pynetgear/const.py) |
| SMLIGHT | [API constructeur](https://smlight.tech/support/manuals/books/slzb-os/page/slzb-os-api-endpoints), [pysmlight, lectures](https://github.com/smlight-workspace/pysmlight/blob/85142c6b80123819d31a388b210532fff09458eb/pysmlight/web.py), [modèles](https://github.com/smlight-workspace/pysmlight/blob/85142c6b80123819d31a388b210532fff09458eb/pysmlight/models.py) |

Restent à prouver avant toute promesse matérielle : schémas/unités exacts, variantes
d'authentification, disponibilité publique SMLIGHT, TLS proposé par chaque firmware,
stabilité d'identité et sens exact des tables de clients. Les noms d'actions ci-dessus
sont un périmètre de recherche ; seule une fixture et son test autorisent leur ajout.

## Ordre et portes de livraison

1. Faire relire et publier le présent cadrage et sa [matrice de qualification](reseau-qualification.md).
   Aucun enfant ne démarre simplement parce qu'il est créé dans le backlog.
2. Faire relire le plan de N0 avant de changer le vocabulaire partagé ; N0
   précède les autres lots. Pour chaque client, figer les requêtes, unités et fixtures
   publiques/synthétiques, faire relire son plan, puis implémenter les tests de contrat.
3. Livraison de chaque lot : paquet complet (manifeste, profil ou crate, fixtures,
   onboarding et README distinguant repris/deviné/reste), catalogue, fr/en et validation.
   Garder alphabétiques les nouvelles entrées des listes partagées ; ne pas annoncer ces
   clients dans les listes fonctionnelles avant leur implémentation.
4. Un seul créneau lourd explicitement alloué : `sh scripts/check.sh` dans le worktree,
   avant chaque commit. Si le web change : `npm ci`, build web et contrôle i18n.
   Conserver SHA de base/diff, empreintes avant/après et journaux hors dépôt.
5. Commit français sur la branche du lot, pas de push/merge par l'exécutant, puis
   soumission au relecteur. Le parent ne peut être clôturé sur le seul cadrage.
6. Qualification réelle après H1, humain présent sur cible neutre, jamais une pièce
   protégée, état initial conservé. Valider ensuite un propriétaire unique par usage.
