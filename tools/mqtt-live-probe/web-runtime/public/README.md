# Coque PWA du prototype

Le serveur Rust sert ces chemins de même origine :

| Chemin | Fichier ou source | Type |
|---|---|---|
| `/` | `docs/proposition/analyse/poc-interface-mqtt.html` | `text/html; charset=utf-8` |
| `/mqtt.min.js` | MQTT.js 5.10.3 local | `application/javascript` |
| `/service-worker.js` | `public/service-worker.js` | `application/javascript` |
| `/manifest.webmanifest` | `public/manifest.webmanifest` | `application/manifest+json` |
| `/icons/moli.svg` | `public/icons/moli.svg` | `image/svg+xml` |
| `/icons/moli-192.png` | `public/icons/moli-192.png` | `image/png` |
| `/icons/moli-512.png` | `public/icons/moli-512.png` | `image/png` |
| `/icons/apple-touch-icon.png` | `public/icons/apple-touch-icon.png` (180 × 180) | `image/png` |

Servir le worker sans cache HTTP permanent (`Cache-Control: no-cache`), à la racine
pour sa portée `/`. L’enregistrement nécessite HTTPS ou localhost. Un accès initial
connecté charge la coque et installe le cache ; un premier accès hors connexion ne
peut pas installer l’application. Safari iOS utilise l’icône PNG dédiée pour
l’ajout à l’écran d’accueil. Les PNG sont des rendus locaux du SVG par `rsvg-convert`.

Le cache versionné contient uniquement HTML générique, bibliothèque MQTT,
manifeste et icônes. Incrémenter `CACHE_VERSION` après une modification de la coque
ou des assets. L’activation supprime les anciennes versions portant notre préfixe,
sans toucher aux caches d’autres applications.

Les API, WebSockets, URL avec requête, origines tierces et requêtes portant un
en-tête d’authentification sont exclus. Les redirections, réponses opaques et
pages HTML sans le marqueur de coque ne sont jamais mises en cache. Aucune donnée
MQTT, aucun identifiant saisi, aucun état ni aucune description de maison ne sont
persistés côté PWA. Les descriptions initiales stockées en RAM ou sur disque
relèvent du serveur et restent hors de ce cache. Le mode fichier garde son repli
MQTT.js externe ; le mode HTTP utilise exclusivement la bibliothèque locale.

Hors connexion, un bandeau distingue les anciennes valeurs de session des états
actuels. Toute commande exige le réseau et un client MQTT connecté. Les commandes
UI utilisent QoS 0 avec `queueQoSZero: false` : aucune file ni répétition au retour
du réseau. La réussite d’écriture ne confirme jamais l’état physique. Seul un
retour d’état peut confirmer une commande ; la reconnexion recharge les retained
sans rejouer les commandes.

Références : [cycle de vie du service worker](https://developer.mozilla.org/en-US/docs/Web/API/Service_Worker_API/Using_Service_Workers),
[icônes de manifeste](https://developer.mozilla.org/en-US/docs/Web/Progressive_web_apps/Manifest/Reference/icons).
