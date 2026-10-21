# SMLIGHT — supervision HTTP publique

Profil déclaratif `smlight`, sept liaisons fixes RO. Radio et socket de coordination
restent à Zigbee2MQTT ; aucune reprise de fonction HA avant validation humaine.

## Repris

Connaissance des [endpoints constructeur](https://smlight.tech/support/manuals/books/slzb-os/page/slzb-os-api-endpoints)
(consultés le 21 octobre 2026), de [pysmlight 85142c6](https://github.com/smlight-workspace/pysmlight/tree/85142c6b80123819d31a388b210532fff09458eb)
(`web.py`, `models.py`, Apache-2.0) et de [HA core cbbd58b](https://github.com/home-assistant/core/blob/cbbd58b4ca6b657168339661a638136ce4387a02/homeassistant/components/smlight/sensor.py)
(Apache-2.0, Celsius et uptime en secondes). Aucun code tiers intégré. Toutes les
fixtures sont synthétiques, licence du dépôt ; aucune capture privée ni donnée réelle.

GET `/ha_info` toutes les 3600 s et `/ha_sensors` toutes les 60 s. La MAC `Info.MAC`
est une identité obligatoire, validée et normalisée ; jamais un point ni un repli IP/nom.
L'humain fournit hôte/port. `fixed_target` interdit ARP, découverte, adresse apprise ou
relocalisation ; une identité différente connue/en cache bloque la publication. Chaque
lecture fraîche d'identité est vérifiée avant publication. Aucun secret ni login.

Les lectures sont espacées d'au moins une seconde, y compris la première et les reprises.
Le délai est aussi tenu après achèvement, pour protéger le coordinateur d'une connexion lente.
Une pause initiale précède le premier départ ; la barrière temporelle est partagée
entre les reprises. Cadences ensuite
calculées après achèvement, sans rattrapage. Un seul échange en vol, 5 s/1 Mio par échange.
401/403, redirection, HTML de login ou réponse illisible rendent la variante inutilisable :
aucun fallback, auth forcée ou désactivation de sécurité. Pour la cible fixe, toutes les
lectures du premier tour doivent réussir et une erreur ultérieure invalide le relevé ;
le superviseur peut seulement retenter ces lectures avec backoff, jamais s'authentifier.

| Liaison | Source / unité |
|---|---|
| firmware | Info.sw_version, texte |
| esp32_temp, zb_temp | Sensors, températures techniques °C |
| uptime | Sensors.uptime, s |
| ram_usage | Sensors.ram_usage, valeur brute sans unité ni conversion |
| ethernet, wifi_connected | booléens de liaison du coordinateur |

Toutes sont explicitement Diagnostic (y compris températures et booléens), avec
Family::Diagnostic vérifiée par le classificateur réel. Sept définitions fixes, aucune
création par champ inconnu, tableau radio ou index. Une valeur facultative absente,
null ou invalide reste inconnue ; une liaison non observée n'est pas une présence humaine.
`missing = "null"` efface immédiatement une mesure disparue d'une réponse valide et
invalide les anciennes valeurs du cache au premier tour. Arrêt/échec de la cible fixe :
valeurs Null/hors ligne. Reprise/retour des mesures : mêmes DeviceId/PointId, sept au plus.

## Deviné et reste

La mémoire est étiquetée kilooctets par HA, mais la base et la signification firmware
n'ont pas été établies : conserver la valeur brute, sans supposer % ni facteur vers B.
Aucun autre canal, API `/api`/`api2`, MQTT/SSE, radio multichip indexée, mode, toggle,
reboot, flash, scan ou modification. Champs `zb_temp2`/radios ignorés, jamais assimilés
à une autre puce. Version/modèle/firmware et budgets réels restent à qualifier après H1.
Ne pas désactiver une authentification pour rendre ces routes publiques. Le profil n'est
pas un engagement de compatibilité avec un appareil installé.

## English

Public monitoring profile only: two fixed GETs, no credentials or radio access.
Seven static read-only diagnostic bindings; absent readings stay Null across restart,
including old cached values. Native MAC identity is mandatory and checked before publishing.
The owner supplies the sole target; no address discovery or fallback. Reads start at least
one second apart, including startup/restarts; each exchange is bounded to 5 s / 1 MiB.
Authentication-required or invalid endpoints remain unsupported. Memory stays raw without
an inferred unit. Zigbee2MQTT and HA retain their usages until human H1 qualification.
