# Samsung Tizen — WebSocket TLS et UPnP

Pilote local natif `moli-samsung`. Aucun cloud, aucune dépendance Python.

- Télécommande : WebSocket TLS `/api/v2/channels/samsung.remote.control`, port
  8002. Appairage accepté à l'écran. TOFU : poignée de main sans données
  applicatives, certificat persisté avant tout jeton. Un autre certificat est
  refusé. Jeton numérique borné à 256 caractères, coffre obligatoire.
- Identité : `device.id` de `/api/v2/`, sans le préfixe `uuid:`, jamais le nom ou
  l'adresse. L'identité persistée permet de redémarrer télé en veille. Un relevé
  présentant une autre identité est refusé.
- Points compatibles télécommande : `power`, `volume` (0–100), `mute`, `key`,
  `app` (écriture seulement). Touches Philips traduites par liste autorisée ;
  aucune touche Samsung brute, touche de maintenance ou URL d'application.
  Pas de « piste suivante/précédente » émulée par avance/retour rapide.
- Allumer : un paquet WoL, MAC requise. Éteindre : `KEY_POWEROFF`, jamais une
  bascule d'alimentation. Volume et sourdine absolus : SOAP RenderingControl sur
  l'hôte déclaré, port 9197, chemin fixe `/upnp/control/RenderingControl1`.
  Aucun SSDP, redirection ou adresse donnée par un appareil n'est suivi.
- Une connexion WebSocket authentifiée par ordre : aucune socket inactive à
  maintenir. Les connexions ne rejouent jamais un ordre précédent.
- Une FIFO de 4 ordres en attente et un en vol, échéance absolue du hub vérifiée
  après connexion/verrou et avant émission. Pas de rejeu automatique. Une
  écriture WebSocket réussie signifie « remis au transport », pas une preuve
  d'effet sur l'écran. Un échec ambigu est annoncé comme non confirmé.
- Abandon avant émission : rien ne part. Après émission : acquisition du résultat
  jusqu'à l'échéance, journal conservé même si le caller part ; notice sur échec.
- Relevés indépendants toutes les 30 s (10–3600), sans rattrapage. `PowerState`
  est publié seulement s'il est explicite. UPnP peut être indisponible selon le
  modèle : volume/sourdine restent alors inconnus, sans émulation par touches.
  L'application courante n'est pas observable par ce contrat : aucun état
  d'application n'est inventé à partir d'une demande de lancement.
- Bornes : événement JSON 64 Kio (transport WS borné à 1 Mio), 32 événements
  d'appairage, 60 s d'attente humaine, 4 s pour reconnecter une télécommande,
  64 Kio par réponse SOAP ; relevé global 12 s. Arrêt : tâches annulées, appareil
  hors ligne. Aucune donnée brute du réseau ou du jeton dans les diagnostics.

Les fixtures sont **reconstruites et synthétiques**, aucune capture de maison.
Les tests emploient le vrai hub, un serveur TLS à certificat éphémère, et un
serveur SOAP sur loopback. Aucun appareil réel n'a été qualifié.

Références de protocole (consultées le 20 octobre 2026, sans code repris) :
- [samsung-tv-ws-api : remote.py](https://github.com/xchwarze/samsung-tv-ws-api/blob/master/samsungtvws/remote.py)
- [samsung-tv-ws-api : connection.py](https://github.com/xchwarze/samsung-tv-ws-api/blob/master/samsungtvws/connection.py)
- [SamsungTV Smart : UPnP](https://github.com/ollo69/ha-samsungtv-smart/blob/master/custom_components/samsungtv_smart/api/upnp.py)
