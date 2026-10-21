# Moli OS : la doc

Moli OS est une plateforme domotique écrite en Rust, née dans une maison habitée, qui
remplace Home Assistant. Elle est pensée pour une famille qui veut que ça marche, et pour
des agents (Moli, Claude, Codex) qui configurent, créent les automatismes et écrivent les
intégrations. Ces agents ne sont jamais aux commandes en temps réel.

## Où on en est (4 octobre 2026)

Mesures sur une installation réelle, à côté de son Home Assistant (détail dans
[PERFORMANCE.md](PERFORMANCE.md)) :

| | Moli OS | Home Assistant (même machine) |
|---|---|---|
| Mémoire (pages anonymes du conteneur) | **4 à 7 Mio** selon les clients connectés | 2,15 Gio |
| CPU au repos | **0,24 à 0,3 %** | 1,5 à 2,3 % |
| Image | **17 Mo** | 2,3 Go |
| Échelle | 77 appareils (22 pilotes) | 171 appareils (1 591 entités) |

Ce qui marche aujourd'hui :
- 28 intégrations : aides de la maison, chauffe-eau Ariston (cloud), Bambu (et son AMS), box UPnP, Frigate, hôte et
  ordinateurs, Hue, imprimante IPP, Klipper, tondeuses Landroid, serrures Nuki, télés Philips et Samsung Tizen (Samsung sur contrats simulés),
  présence par Wi-Fi, profils déclaratifs (Daikin, iopool, Meross, Open-Meteo, Tempo), Reolink,
  aspirateurs Roborock, Somfy TaHoma (volets et portails, local et cloud), Sonos (et ses
  annonces vocales), Tapo, **Telegram** (votre bot, sans relais), **téléphones** (appli Moli),
  Tuya (local et cloud), chauffage Voltalis (cloud) et Zigbee (Zigbee2MQTT) ;
- la garde (pièces protégées, heures calmes), l'historique et l'énergie ;
- le dashboard « Maison », avec des **filtres par type** et le **plan vivant** : la maison
  redessinée en données, en 2D zoomable à 4 habillages et en **3D** ;
- Moli, l'assistant ; les automatismes (graphe, validation humaine), avec les canaux
  maison, voix, **téléphone** et Telegram ;
- l'**appli mobile « Moli »** (iOS, TestFlight) : le dashboard dans l'appli, et le téléphone
  devient un capteur de la maison (à la maison ou non, batterie, Wi-Fi, position) et reçoit
  les notifications.

Sur le réseau local, Moli OS répond sur `http://<hôte>:8790`. Depuis l'extérieur, il peut
passer par un tunnel Cloudflare protégé par Cloudflare Access (par exemple
`https://maison.example.org`, réservé aux e-mails du foyer) ; seuls les rapports des
téléphones passent sans Access, avec leur propre jeton.

## Par où commencer

| Je veux… | Lire |
|---|---|
| installer Moli chez soi (Docker, code d'installation, premiers appareils) | [components/installation.md](components/installation.md) |
| comprendre comment c'est construit, et les décisions prises | [ARCHITECTURE.md](ARCHITECTURE.md), puis la carte vivante `#/systeme` du dashboard |
| l'appli mobile (`mobile/`) | [components/api.md](components/api.md) |
| connaître la règle de performance et les mesures | [PERFORMANCE.md](PERFORMANCE.md) |
| compiler, déployer, opérer, gérer les secrets | [components/exploitation.md](components/exploitation.md) |

## Les composants

| Doc | Contenu |
|---|---|
| [api.md](components/api.md) | REST, SSE, MCP, CLI, sécurité des routes |
| [automatismes.md](components/automatismes.md) | modèle de graphe, validation humaine, moteur, éditeur, import HA |
| [capteurs-lumieres.md](components/capteurs-lumieres.md) | cadrage : Xiaomi BLE, pont EnOcean MQTT, Netatmo météo ; Twinkly natif sur contrats simulés, qualification H1 encore requise |
| [coeur.md](components/coeur.md) | vocabulaire, hub, bus, garde et approbations, sessions PIN, journal, étiquettes, coffre des secrets, superviseur |
| [dashboard.md](components/dashboard.md) | interface Maison et Atelier (Svelte 5), carte vivante |
| [exploitation.md](components/exploitation.md) | image Docker, clé maître, sauvegarde, mise à jour, développement |
| [historique-energie.md](components/historique-energie.md) | historique SQLite, SQL pour agents, énergie (kWh, €, Tempo), import HA |
| [installation.md](components/installation.md) | premier démarrage, code de la maison, clé maître, premiers appareils, accès hors de chez soi |
| [medias.md](components/medias.md) | contrats purs de découverte média R1 : identités, validation et sélection, sans catalogue réel ni lancement raccordé |
| [moli.md](components/moli.md) | l'assistant : conversation, outils, garde, création d'automatismes, filtres |
| [netgear.md](components/netgear.md) | client Netgear SOAP en lecture, contrats synthétiques et limites de qualification |
| [pilotes.md](components/pilotes.md) | trait `Driver`, pilotes natifs dont Huawei LTE en lecture et Netatmo météo, `moli-net` |
| [profils.md](components/profils.md) | moteur de profils déclaratifs, format TOML, ajouter un appareil sans code |
| [reseau.md](components/reseau.md) | cadrage réseau : diagnostic, ASUSWRT, Huawei LTE, Netgear et SMLIGHT ; Huawei LTE et Netgear natifs sur fixtures, autres lots à implémenter |
| [reseau-qualification.md](components/reseau-qualification.md) | preuves de contrat réseau, fixtures, bornes et porte de qualification humaine |
| [traduction.md](components/traduction.md) | catalogues `locales/`, clés, pluriels, ajouter une langue, contrôle |
| [wyoming.md](components/wyoming.md) | supervision locale Piper, Whisper et openWakeWord ; garde-fou complet réussi, revue architecte et qualification réelle après H1 distinctes |

## Les règles à ne jamais oublier

- **La famille commande sans friction** ; le PIN sert à valider un automatisme et à autoriser
  un agent, jamais à commander (décision D0).
- **Pièces protégées** (là où dort un enfant) : aucun agent n'y agit sans un humain. Les
  tests physiques se font sur une cible neutre (jamais une chambre), un humain présent, en
  remettant l'état comme avant.
- **Performance** : chaque déploiement est mesuré contre HA. Le budget est dans
  [PERFORMANCE.md](PERFORMANCE.md).
- **Aucun secret en clair** : ils vont dans le coffre chiffré de Moli, dont la clé maître
  arrive par l'environnement (de préférence un fichier tmpfs fourni par un gestionnaire de
  secrets). Ne jamais régénérer `MOLI_MASTER_KEY`.
