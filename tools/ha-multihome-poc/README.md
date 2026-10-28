# POC : actif chez soi, passif pour l’autre

Deux nœuds portent chacun les deux maisons. `node-a` est actif pour `house-a`
et passif pour `house-b` ; `node-b` a les rôles inverses. Un rôle appartient à
une maison, pas globalement au serveur. Le petit master précédent reste séparé.

Le POC autonome utilise de vrais processus Rust et du MQTT TCP en loopback.
Ses entrées initiales passent par la projection `House` existante, à partir des
deux exports privés HA, avec identités anonymisées. Une lecture MQTT réelle des
deux installations est documentée séparément : elle ne transforme pas le rejeu
en flux physique vivant. Les HA ne sont ni arrêtés ni reconfigurés par le banc.

## Utiliser

La commande Dev « Maisons — HA croisée et tolérance aux pannes » lance :

```sh
python3 tools/ha-multihome-poc/run-poc.py
```

Elle compile avec Rust 1.99.1 et deux tâches, vérifie formatage/Clippy/tests,
puis lance les scénarios. `--fixture` permet un essai sans exports privés.
Les résultats publics agrégés sont dans
[`ha-multimaison-resultats.json`](../../docs/proposition/analyse/v6/ha-multimaison-resultats.json).
Le lanceur trouve les deux exports hors dépôt comme le banc de charge existant ;
aucun nom de fichier privé ni contenu d’état n’est imprimé.

La commande Dev « Maisons — POC HA et interface commune (1416) » lance :

```sh
python3 tools/ha-multihome-poc/run-poc.py --serve --skip-check
```

`--skip-check` réutilise le binaire HA déjà vérifié ; ce n’est pas une validation.
Le service écoute seulement sur `127.0.0.1:1416`. Le POC principal ajoute un
onglet **Continuité**, servi sous `/ha/` par la passerelle1414, et un sélecteur
pour parcourir les maisons. Le proxy reste sur une destination loopback fixe,
avec corps bornés et ticket signé pour les mutations. Aucun endpoint ne permet
de choisir un hôte, un binaire, un PID externe ou un pilote physique.

## Paramétrer

L’interface propose : sans copie, secours local, secours cloud, local+cloud et
secours croisés. Le JSON avancé décrit `enabled`, `witness`, les nœuds
`{id,site,enabled}` et les maisons `{id,primary,standbys:[{node,automatic}]}`.
Il est limité à16Kio, deux maisons, huit nœuds et quatre détenteurs par maison.
Les IDs restent stables ; les libellés ne sont pas des clés. Un cloud peut porter
plusieurs maisons, ou plusieurs clouds distincts peuvent être déclarés.

Configuration et copies restent en RAM. Il n’existe pas de journal de
réplication sur disque ; exporter la topologie télécharge seulement ces
paramètres fictifs. Aucun coffre ou jeton de fournisseur n’est répliqué.

Une coupure WAN interrompt les copies entre sites, pas le primaire local de
chaque maison. Les deux maisons continuent à avancer. Rétablir le lien applique
les deltas manquants depuis la file RAM bornée à256entrées/1Mio, sinon un
instantané complet. Avec le JSON de ce banc, le rejeu est limité à16deltas
par rattrapage : au-delà, un instantané évite de recalculer toute l’empreinte
pour chaque petite mutation. Les morceaux sont bornés, contrôlés parSHA-256 puis publiés
atomiquement ; un état incomplet ou périmé ne remplace pas une copie valide.

Les nœuds reçoivent des snapshots≤8Mio/maison et≤20000enregistrements.
Les messages MQTT font≤4096octets, dont des morceaux de2048octets avantbase64.
Le broker de test existant conserve ses propres bornes. Les accusés de morceaux,
la file réseau et le staging sont bornés ; une erreur n’autorise aucun effet.

## Reprise et autorité

Une autorité de test par maison attribue un bail de3secondes et une époque
croissante. Le renouvellement200ms et les battements500ms s’exécutent hors des
opérations de copie ; le coordinateur a deux threads, chaque nœud un seul.
Un calcul bloquant volontaire de3,5secondes vérifie cette indépendance.

Une promotion demande une copie non vide, même époque, même séquence et même
empreinte. L’ancienne époque est refusée. Une absence de réponse ne prouve pas
un arrêt : le fencing du banc nécessite l’arrêt et la récolte de son propre
processus. Sans témoin, cette preuve est indispensable. Un retour du même
primaire reçoit aussi une nouvelle époque ; il ne ranime pas son ancien bail.

Le secours local peut être automatique avec témoin. Un secours d’un autre site
requiert une action humaine et reçoit seulement la portée cloud. Il conserve
la lecture de toute la copie ; il ne récupère pas automatiquement les radios
ou le réseau local de la maison distante. Une seconde promotion n’arrête pas
l’activité de la maison que le serveur portait déjà.

Les workflows sont factices : l’admission d’effets est déterministe, avec
propriétaire/époque/bail/portée/copie vérifiés et reçus bornés conservés entre
promotions. Aucune lampe, prise, radio, musique ou notification réelle n’est
commandée. Le compteur d’ordres physiques reste0.

## Mesurer et qualifier

RSS, CPU et allocations sont mesurés par processus. Les octets de document par
maison sont un compteur logique, pas une RSS individuelle. Les sommes de RSS
peuvent compter plusieurs fois des pages partagées ; le coût des outils `ps`
de mesure n’est pas inclus. Le profil debug et les objets JSON de ce banc ne
constituent pas le budget du cœur compact de production.

L’état HA est aussi republié sur `moli-probe/ha/config` (description retained)
et `moli-probe/ha/state` (mesures et rôles), après ouverture de la vue HA.
`moli-probe/ha/availability` donne la disponibilité du module et un instant UTC ;
une connexion HTTP ouverte ne suffit pas à considérer une ancienne copie fraîche.
La passerelle n’interroge pas ce module tant qu’il n’a pas été activé par la vue.

Les répliques sont des processus locaux, pas des conteneurs déployés sur les
serveurs HA. Le témoin et le générateur de données sont centralisés dans le
coordinateur de test : ce lot ne prouve pas le consensus distribué, la survie
à sa perte, l’horloge de machines distinctes, le VPN/TLS intersites, le coffre
répliqué ou les effets de production. La garde réelle et les pilotes ne sont
pas remplacés. L’intégration dans le démarrage Moli et la qualification sur
plusieurs machines restent des étapes distinctes.

## Perte de connexion TCP et silence réseau

```sh
python3 tools/ha-multihome-poc/run-poc.py --network-loss
```

Ce mode ajoute deux connexions par nœud : contrôle local direct et données de
la maison étrangère par un relais TCP. Il ferme réellement les flux, puis les
laisse ouverts sans aucun transfert. Les opérations étrangères sur le canal
local sont refusées. Les nœuds doivent continuer leurs maisons, les copies
rester figées, puis retrouver rang et empreinte sans appliquer une sonde ayant
expiré. Les générations MQTT et un compteur worker indépendant des documents
empêchent une fausse preuve de reconnexion ou de non-rejeu.

Le test ne coupe pas le WAN des serveurs HA. Le témoin central et les canaux
locaux restent joignables. Résultats dans `v6/ha-multimaison-tcp.json` et protocole
dans `v6/ha-multimaison-tcp-protocole.md`. Les services déjà actifs sont laissés
intacts ; aucun nouveau mode n’est imposé à leurs processus.

## Mode autonome sur deux hôtes VPN — qualification bloquée

La qualification demandée a ensuite été précisée : deux instances sur le Mac
avec perte de leur pont. Cette campagne locale est exécutée et documentée
ci-dessous ; les obstacles VPN de cette section ne la bloquent pas.

Le mode explicite `--site-bench` prépare un primaire et un courtier local par
hôte, avec une copie passive de l’autre maison. Il n’est pas raccordé aux
pilotes physiques. Le Mac observe et déclenche les défauts, sans produire les
changements locaux. Le POC interactif antérieur reste séparé.

Validation locale et préparation d’une archive de sources Linux :

```sh
python3 tools/ha-multihome-poc/check-sites.py
```

Le binaire lit son JSON initial borné sur stdin. Les champs sont `node`,
`house`, `bind_ip`, `peer_ip`, `observer_ip`, `mqtt_port`, `peer_mqtt_port`, `control_port`,
`prefix`, `bundle` et `ttl_seconds`. Les adresses doivent appartenir au VPN
CGNAT déclaré ; la configuration propre à une installation reste hors dépôt.
Une option `loopback_fixture: true`, désactivée par défaut, exige au contraire
trois adresses loopback et permet un contrôle local sans aucune sortie VPN.
Le statut expose alors `network_scope: loopback_fixture`.
Dans ce mode, les instances peuvent partager `127.0.0.1` si leurs ports MQTT
sont distincts ; `peer_mqtt_port` désigne le port de l’autre instance et prend
par défaut la valeur de `mqtt_port` pour les anciennes configurations.
Les deux sites reçoivent les mêmes structures initiales anonymisées. Il faut
un exécutable Linux compatible ; la construction native macOS ne le remplace
pas. Le transfert et le lancement ne sont pas effectués par `check-sites.py`.

Chaque primaire produit une valeur synthétique toutes les2s. Son bail est
renouvelé toutes les200ms, y compris en mode gelé. La file contient au plus
256deltas/1Mio ; un écart initial supérieur à16deltas privilégie un instantané,
puis les deltas produits pendant son transfert sont rattrapés dans la même
file. La durée de vie est270s par défaut,480s maximum, sans recycler les
reçus d’effets bornés à256. Le mode ne promeut jamais la maison étrangère.

Le MQTT du banc écoute une adresse VPN précise et n’accepte qu’un pair
déclaré. Les contrôles HTTP acceptent uniquement l’observateur déclaré :
`GET /status`, `POST /fault` avec `mode: up|cut|blackhole`, `POST /freeze`
avec `enabled: bool`, `POST /stop`. Les corps sont bornés à1Kio. Ces contrôles
restent joignables pendant les défauts MQTT ; ils ne constituent pas un
protocole d’authentification de production. `blackhole → up` reprend les
mêmes sockets tant que les échéances naturelles ne les ont pas fermées.

État du27octobre2026 :67tests Rust et Clippy stricts passent localement.
Les deux hôtes réels acceptent les échos TCP53141 du Mac, mais les quatre
tentatives entre maisons expirent pendant les fenêtres d’écoute. Le transfert
SCP échoue aussi. Le binaire Linux n’a pas été construit ; les sites autonomes
ne sont pas déployés. Les coupures8s/3s/unidirectionnelle8s/longue120s restent
**non prouvées**. Aucune règle VPN ou configuration HA n’a été changée.

Les écoutes temporaires ont été fermées, le conteneur de préparation et son
image retirés. La passerelle1414 et le service1416 sont restés actifs.
Le serveur optionnel `wan-artifacts.py` distribue seulement les fichiers
déclarés depuis le dossier privé du banc, sur l’adresse VPN de l’observateur,
avec pairs autorisés et arrêt automatique. Il est arrêté ; le téléchargement
depuis la maison n’a pas fonctionné. Le chemin normal reste le transfert
SCP/SFTP fourni par l’application.

Constats horodatés et critères restant à exécuter :
[`ha-multimaison-wan.html`](../../docs/proposition/analyse/v6/ha-multimaison-wan.html).

## Pannes du pont entre deux instances Mac — quatre cas réussis

```sh
python3 tools/ha-multihome-poc/run-link-faults.py
```

Le lanceur construit et vérifie le binaire, normalise les deux exports,
démarre deux instances autonomes et deux courtiers loopback sur des ports
libres, puis applique fermeture8s, silence3s, arrêt du seul flux B→A8s et
silence120s. Chaque primaire continue à produire ses propres changements ;
le contrôleur ne publie aucun état à leur place. La convergence est vérifiée
sans gel par trois lectures source/copie/source au même rang et au même hash.

Résultat mesuré : quatre PASS ; reprise par deltas dans les trois cas courts,
mêmes connexions conservées sur le silence3s, un instantané par maison après
120s. Pendant le cas long, chaque primaire avance de60rangs avec60effets
factices admis. Le retour des copies identiques prend respectivement
1393,706,1331et1198ms sur cette exécution debug locale.

Les67tests Rust, le formatage et Clippy stricts passent. `--skip-check` exige
le reçu natif du binaire courant ; les empreintes sont comparées aux sources.
Un contrôle dédié refuse le reçu manquant. `--quick` réduit le cas long à12s
et ne qualifie pas120s ; `--fixture` utilise un petit jeu synthétique et ne
qualifie pas la campagne sur les deux exports. `--measure-runner` mesure
séparément le contrôleur. RSS/CPU sont échantillonnés par PID, allocations
mesurées dans les instances, et tous les enfants sont arrêtés puis récoltés.

Les résultats publics sont dans
[`ha-multimaison-pont-local.json`](../../docs/proposition/analyse/v6/ha-multimaison-pont-local.json),
avec [rapport HTML](../../docs/proposition/analyse/v6/ha-multimaison-pont-local.html)
et [protocole](../../docs/proposition/analyse/v6/ha-multimaison-pont-local-protocole.md).
Le code exact du lanceur exécuté est archivé dans le dossier V6 ; le lanceur
courant ajoute ensuite uniquement le refus du reçu natif absent, vérifié
séparément. Les quatre chemins de panne sont inchangés.
