# Home Assistant : pont MQTT et migration progressive

Premier lot expérimental,28octobre2026. Les briques et les chemins simulés
sont implémentés ; aucune installation HA ni box réelle n’a été migrée.

Le [guide HTML de migration](../proposition/migration-home-assistant.html)
explique les étapes par composant, les impacts mémoire, les sauvegardes,
les retours arrière et la migration distincte des workflows. Il sépare
les briques qualifiées en simulation des étapes restant à implémenter.

## Les briques

| Composant | Responsabilité |
| --- | --- |
| `moli-home-assistant` | Catalogue et états HA typés, commandes MQTT dans les deux sens, sessions et échéances |
| `moli-migration` | Correspondances stables, phases durables, révisions, empreintes et réconciliation |
| `moli-runtime` | Autorité d’écriture par génération sur tous les chemins du Hub |
| `moli-overkiz` / `moli-net` | Permis vivant jusqu’à la dernière remise HTTP, y compris après OAuth/TLS |
| Compagnon HA `custom_components/moli` | Inventaire HA, proxies cover Moli, bouton favori et résultats MQTT |

Un appareil HA peut publier plusieurs canaux : chacun garde sa valeur, son
type, son unité et sa sémantique. Les noms ne servent pas de clés. Les choix
enum sont décrits dans le catalogue ; l’état utilise leur code. Les proxies
Moli sont exclus de l’inventaire retourné par HA pour éviter les boucles.

Le premier lot de commandes couvre ouvrir, fermer, arrêter et le favori
quand le pilote le déclare. Un volet RTS sans retour n’a jamais de position
inventée. L’import HA est également conservateur pour les covers annoncés
comme supposés ou sans position observée.

## Configuration explicite

```toml
[[driver]]
id = "ha-main"
kind = "home-assistant"
[driver.options]
host = "127.0.0.1"
port = 1883
installation = "ha-main"
native_instances = ["covers-native"]
```

`covers-native` doit déjà être une instance Overkiz activée et configurée.
Elle est placée sous gestion de migration : ses appareils commencent fermés
en écriture, même avant leur découverte. Une génération confirmée ouvre
ensuite seulement les appareils transférés. Avec `native_instances=[]`, le
pont peut importer HA sans déclarer de transfert natif.

Le préfixe par défaut devient `moli/ha-bridge/ha-main`. Un préfixe explicite
doit rester sous `moli/` ou le domaine neutre de banc `moli-probe/`. Les
installations et détenteurs natifs dupliqués sont refusés au démarrage. Un
broker distant exige `tls=true` ; TCP sans TLS est limité à loopback.

Les identifiants MQTT se placent dans le coffre chiffré de cette instance,
sous `mqtt_username` et `mqtt_password`, par le mécanisme standard
`moli-os secrets set` avec valeur passée par un tube. Aucun identifiant n’est
mis dans le TOML. Ne pas régénérer la clé maître existante.

Le compagnon à installer ultérieurement est dans
`tools/home-assistant/custom_components/moli/`. Il nécessite l’intégration
MQTT de HA. Son formulaire demande le même préfixe et le même identifiant
d’installation. Il réutilise la connexion MQTT HA ; les ACL du broker doivent
séparer les producteurs HA et Moli. L’identité de service HA devient
`Origin::Api`, jamais une identité humaine Moli fournie par le payload.

## Contrôle et persistance

Les options sont chargées au démarrage. Le registre global
`data/migration.json` est ouvert et ses fermetures installées avant le
lancement des pilotes natifs. Un seul écrivain de ce fichier est partagé
par les instances HA. Il est inclus dans la sauvegarde/restauration Moli.

L’API expose sous `/api/integrations/home-assistant/{instance}` :

- `GET` : état du pont, catalogue et phases de migration ; accès foyer.
- `POST /prepare` : plan exact, révision attendue, opération et empreinte.
- `POST /advance` : demande de preuve pour la phase courante.
- `POST /reconcile` : nouveau défi après redémarrage ou état incertain.
- `POST /reload` : recharge le registre partagé ; ses écritures restent
  fermées jusqu’à réconciliation. Cette opération concerne le registre global.

Les mutations exigent une session humaine Moli, refusent les invités et le
mode démo, et limitent les corps à8Kio. Aucune route HTTP n’acquitte une
preuve sur la foi d’un booléen du navigateur. Les réponses de preuve sont
réservées au transport du compagnon configuré.

Chaque commande porte la session du destinataire, une requête unique, la
génération, la révision pertinente et une échéance de5s maximum. Les rejeux
retained, anciennes sessions, doublons et générations périmées sont refusés.
Pas de file d’actions hors connexion ni retry après reconnexion. Les
publications retained de description/état peuvent être reprises ; elles ne
sont pas des actions. Les battements de disponibilité sont émis toutes les5s
et expirent après15s ; une ancienne annonce retained ne prouve pas la vie.

Le registre est borné à512correspondances,128opérations actives et1Mio.
Le protocole limite le catalogue à1Mio,1024appareils,4096canaux, les textes
à4096octets et les commandes à1Kio. Chaque session garde au plus256reçus sans
éviction silencieuse ; cette limite et son renouvellement devront être
qualifiés avant un usage continu en production.

## Ce qui reste volontairement fermé

Le compagnon ne neutralise pas automatiquement les config entries HA.
Son observateur de quiescence n’est pas branché sur une installation réelle :
sans preuve fiable de l’ancien écrivain désactivé, déchargé, sans attente et
sur la bonne portée, il répond `quiescence_not_proven`. La bascule reste
fermée. Une perte de réseau ne transfère jamais le rôle d’écrivain.

Les proxies HA ont leurs propres identifiants stables. Ce lot ne réécrit
pas les références des automatismes, scripts et scènes existants. Leur
analyse, leur transfert d’autorité et une neutralisation HA réelle restent
des jalons distincts. La provenance d’un futur brouillon peut être conservée
dans le registre ; aucun moteur d’automatisme n’est activé par cette brique.

La garde Moli reste applicable aux commandes venues de HA : une commande
retenue pour validation est signalée comme telle, pas comme un effet effectué.
Une réponse Overkiz ne constitue pas une mesure de position RTS.

## Vérifier

```sh
python3 -m venv target/ha-companion-venv
target/ha-companion-venv/bin/python -m pip install -r tools/home-assistant/requirements-test.txt
npm ci --prefix crates/moli-overkiz/test-fixtures --ignore-scripts
python3 tools/home-assistant/check-moli-bridge.py
```

Le script utilise Rust1.99.1, le broker local de test, une box Overkiz
simulée et le framework Home Assistant2026.10.4 dans un environnement isolé.
Les maisons réelles ne sont pas contactées. Le dossier de qualification V6
relie les résultats et les empreintes aux sources effectivement vérifiées.

Le broker synthétique des tests Rust utilise Aedes1.2.0, dépendance Node
verrouillée dans `crates/moli-overkiz/test-fixtures/package-lock.json`.
L’image de développement l’installe pour que ces tests ne dépendent pas du
POC web local. En natif : `npm ci --prefix crates/moli-overkiz/test-fixtures`.
Cette dépendance de test n’est pas ajoutée au serveur Moli de production.
