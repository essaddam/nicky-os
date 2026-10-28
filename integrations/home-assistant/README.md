# Home Assistant (pont MQTT)

`moli-home-assistant` relie un Home Assistant existant au broker MQTT partagé. Il publie dans Moli le catalogue HA et ses états typés, puis reçoit les commandes des volets déclarés par Moli. Les commandes passent ensuite par le Hub et le pilote propriétaire ; le pont ne pilote aucun équipement directement.

Les identifiants du broker sont lus dans le coffre chiffré Moli sous l’instance configurée (`mqtt_username`, `mqtt_password`). Un hôte distant exige TLS. Le registre de migration unique est conservé dans `data/migration.json` et inclus dans les sauvegardes officielles.

La première version prépare le registre et des gateways, puis expose l’état de migration et les opérations API `prepare`, `advance`, `reconcile` et `reload`. Le compagnon HA ne désactive aucune config entry. Son vérificateur de quiescence est fermé par défaut : faute d’observation fiable que l’ancien rédacteur est arrêté, non chargé, sans ordre en attente et sur la portée attendue, aucune preuve de transfert n’est envoyée. Ce paquet ne prétend donc pas avoir réalisé une migration réelle.

## Limites

- Un seul registre durable est partagé par les instances HA d’un serveur Moli.
- Une liaison cible un pilote natif Moli Overkiz activé, une entité de volet déclarée et les actions présentes dans son contrat.
- Les confirmations de migration arrivent du compagnon MQTT ; les routes HTTP ne peuvent pas acquitter une preuve.
- Les mutations HTTP exigent une session humaine Moli et sont désactivées en mode démo.
- Le transport n’a pas été qualifié sur une instance Home Assistant ou une box réelle.

`fixtures/v1.json` contient uniquement des identifiants et valeurs fictifs pour le contrat versionné.
