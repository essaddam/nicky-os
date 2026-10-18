# Worx Landroid (pilote natif)

- `crates/moli-landroid`. Tondeuses Landroid **filaires** (protocole 0 du cloud Worx, dont la
  L1000 WR147E.1), par le compte Worx de l'appli Landroid. Une tondeuse = un appareil
  `<instance>:<numéro de série>`.
- Chemin : session OAuth (`id.worx.com`), liste des tondeuses (`api.worxlandroid.com`,
  `product-items`), puis **MQTT sur websocket** vers le broker du compte (AWS IoT,
  `*.worxlandroid.com`, chemin `/mqtt`), le jeton d'accès porté par le nom d'utilisateur.
  L'état arrive sur le sujet `commandOut` de la tondeuse ; les ordres partent sur `commandIn`.
- Points : `control` (écriture seule : `start`, `pause`, `home` ; sémantique `control`),
  `state`, `error`, `battery` (%), `charging`, `battery_temperature`, `rain_delay` (min,
  écrivable, sémantique `config`), `rain` et `rain_remaining` (capteur de pluie, si le modèle
  le dit), `schedule` (`active`, `off`, `party` ; écrivable, sémantique `control`),
  `schedule_week` (les tontes de la semaine en clair), `wifi_signal`, `blade_hours`,
  `mowing_hours`.
- **Garde** : démarrer, mettre en pause, rentrer et changer le planning sont des ordres de
  sémantique `control` : un agent qui les demande attend un humain, le dashboard sans code
  demande le code. Le délai de pluie est un réglage.
- **Ordres** : file de 4 au plus derrière l'ordre en vol, un seul à la fois, dans l'ordre reçu ;
  revérifié juste avant de partir (appelant toujours là, avant l'échéance) ; publié **une
  fois** en QoS 1, jamais rejoué, ni après une reconnexion (session MQTT propre). Réponse :
  accusé du broker reçu → fait ; pas d'accusé avant l'échéance → « peut-être parti, non
  confirmé » ; liaison perdue → « peut-être parti » pour l'ordre en vol, « non envoyé » pour
  la file. L'effet est suivi : sans état de la tondeuse dans la minute, la maison est
  prévenue (notice).

## Repris

- Les **faits** du protocole, lus dans pyworxcloud (GPL-3.0, aucune ligne recopiée) et
  l'intégration `landroid_cloud` de HA : adresses, client OAuth public de l'appli Worx
  (`password` puis `refresh_token`, `scope=*`), `product-items?status=1` et ses champs
  (`serial_number`, `protocol`, `online`, `mqtt_endpoint`, `mqtt_topics`, `user_id`,
  `last_status`), nom d'utilisateur MQTT (`bot?jwt=…&x-amz-customauthorizer-…`), identifiant
  client `WX/USER/<compte>/…`, websocket `/mqtt` sur 443, enveloppe des messages du protocole 0
  (`id`, `sn`, `tm`, `dt`), numéros d'ordres (`cmd` 1 démarrer, 2 pause, 3 rentrer), délai de
  pluie (`rd`, 0 à 1 440 min), modes du planning (`sc.m` 0, 1, 2 = fête), tables d'états
  (`ls`) et d'erreurs (`le`).
- Le format du statut (`cfg`, `dat`) : d'après l'exemple documenté de pyworxcloud.

## Deviné

- `fixtures/product-items.json` et `fixtures/status-mowing.json` sont **reconstruits**
  (numéros de série, compte et adresse fictifs) d'après les formats publics : aucune capture
  du compte réel. À remplacer par une capture anonymisée au premier essai réel.
- L'en-tête `Sec-WebSocket-Protocol: mqtt` (exigé par AWS IoT en websocket) et le jeton
  `Bearer` sur l'API : usage courant, pas vu sur le compte réel.
- Le planning renvoyé **entier** avec seulement son mode changé (comme le fait pyworxcloud) ;
  `distm` remis à 0 quand il existe.
- « En ligne » = le cloud dit la tondeuse joignable, ou elle vient d'envoyer un état.

## Reste

- **Qualification sur le compte et la tondeuse réels** (jalon humain H1) : connexion au broker,
  forme réelle des messages de la L1000, délais, limites d'appels de Worx.
- Hors MVP : zones, verrouillage, entraînement de zone, « rentrer sans tondre » (`cmd` 9),
  édition des créneaux du planning, modèles Vision (protocole 1), plusieurs brokers sur un
  même compte.
