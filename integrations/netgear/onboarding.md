# Ajouter un équipement Netgear en lecture

Avant H1, utiliser les fixtures de contrat seulement. Après accord humain, faire fournir
par le propriétaire l'hôte, le port, le transport HTTPS ou HTTP et les variantes de login
et d'inventaire correspondant au firmware. Ne jamais rechercher ces valeurs sur le réseau.
HA garde ses usages jusqu'à validation explicite, usage par usage.

Les options `host`, `port`, `transport`, `login` et `inventory` sont obligatoires ; variantes
`v1` ou `v2`. Les autres options sont `every` (60 s), `inventory_every` (300 s), `info_every`
(3600 s), `selected` (liste vide, 32 identités natives maximum) et `allow_plain_auth` (faux).
Les périodes vont de 30 à 86400 s ; inventaire et informations ne peuvent être plus fréquents que les mesures. L’hôte est une IPv4 privée/loopback ou un nom DNS ASCII, sans URL ni littéral IPv6. Aucun chemin, URL, paramètre SOAP libre ou secret
n'entre dans les options. Les identités sélectionnées restent dans la configuration privée.

Un humain saisit `username` et `password` dans le coffre de l'instance par un tube fourni
par son gestionnaire de secrets. Ni l'agent ni la conversation ne reçoivent leur valeur.
Ne jamais régénérer la clé maître. Secrets absents : pilote en attente sans requête.

HTTPS : la première poignée de main épingle le certificat ; `cert_sha256` est stocké au
coffre avant l'envoi d'identifiants. Une empreinte qui change bloque la connexion ; faire
vérifier le remplacement par l'humain avant qu'il oublie l'ancienne empreinte. Aucun repli
vers HTTP. Si le firmware impose HTTP, expliquer que les identifiants et les données
transitent en clair sur le LAN ; seul le choix humain explicite `allow_plain_auth=true`
autorise cette variante. Une session reste en mémoire jusqu'à l'arrêt.

Le numéro de série observé est enregistré dans `device_identity` ; s'il change ou disparaît,
le pilote bloque. L'humain doit vérifier le remplacement avant d'oublier l'identité stockée.
Ne jamais publier ce numéro ni les noms/IP/MAC/SSID dans un rapport ou au dépôt public.
Sélectionner les clients utiles par identité native, sans fusion avec le pilote présence.
Après redémarrage, retirer un client de `selected` retire sa définition publiée, sans effacer
ses étiquettes ni son historique. Un client encore sélectionné mais absent reste déclaré
avec des mesures inconnues. Les capacités déjà observées sont conservées à la reprise,
sans créer de mesure jamais observée. Le registre reste à 101 points au plus (budget 144).
Une identité routeur ou un cache incompatible bloque la reprise avant sa publication.

La qualification se fait sur une cible neutre avec un humain, hors pièces protégées.
Comparer identité, disponibilité, CPU/mémoire et lien avec la source existante ; contrôler
les unités avant toute utilisation. Un inventaire en échec signifie inconnu. Le compteur
inactif n'est pas activé pour obtenir une valeur. Ne provoquer ni coupure, changement de
configuration, scan ni test de débit. Conserver l'état initial et laisser HA propriétaire
jusqu'à validation de chaque usage. Aucune mise en production implicite à la revue du code.

# English

Before human gate H1, use synthetic contract fixtures only. The owner supplies the host,
port, explicit transport and the firmware's login/inventory variant (`v1` or `v2`); never
scan or auto-detect them. Required options: `host`, `port`, `transport`, `login`, `inventory`.
Polling defaults are 60/300/3600 seconds (30–86400); inventory and information cannot run more frequently than measurements. The host accepts a private/loopback IPv4 address or an ASCII DNS name, without a URL or an IPv6 literal. `selected` is empty by
default, with at most 32 native client identities kept in private configuration.

The owner pipes `username` and `password` into the instance vault without sharing values
with the agent. HTTPS stores `cert_sha256` before sending credentials, and rejects changed
certificates. HTTP requires informed human consent through `allow_plain_auth=true`: both
credentials and data cross the LAN in plaintext. No silent fallback. `device_identity`
pins the native serial number in the vault. Investigate changes before resetting a pin.
Sessions stay in memory. Missing secrets or rejected credentials park the driver.
After restart, removing a client from `selected` retires its published definition while
keeping labels and history. Inventory absence alone never retires a selected client.
Known declared/cached capabilities survive restart with missing values set to Null;
never-observed capabilities are not created. Repeated disjoint selections stay within
101 published points (144-point budget). A different router identity or incompatible cache
blocks publication before a new identity is pinned.

This client only reads fixed SOAP actions. It never enables Traffic Meter, changes settings,
blocks clients or performs speed tests. Bare traffic/rate values have no qualified unit;
missing/disabled data is unknown, never a fabricated zero. Attached-client tables do not
prove current association or a person's presence. Client metadata is never published.
Human qualification uses a neutral target outside protected rooms, preserves the initial
state, and compares each reading with its existing source. HA retains ownership until each
usage is explicitly validated. Contract tests and code review do not qualify real hardware.
