# Modules activables et groupes de processus — POC

Demande du propriétaire : démarrer seulement les modules utiles, les isoler ou
les réunir dans des groupes, puis les fusionner quand la stabilité le permet.
MQTT conserve le contrat entre les groupes. Le moteur audio demandé est natif
Rust et couvre le moteur complet ; l'agent Music est consulté sur ce périmètre.

## Contrat retenu pour le premier essai

- Un groupe isolé est un processus Rust ; plusieurs modules d'un groupe
  partagent son tas et son client MQTT. Le mode fusionné accueille broker et
  modules dans le même processus. Un module non déclaré/activé n'est pas créé.
- Les identités et topics dépendent des modules et appareils, jamais du nom
  du groupe. Regrouper les processus ne renomme donc pas les points.
- Descriptions froides retained, valeurs typées différentielles, catalogues
  enum versionnés ; aucun nom ni liste répété dans chaque état.
- Les métriques RSS et CPU sont celles du processus/groupe. Les modules
  fusionnés disposent de compteurs logiques et temps de traitement ; aucun
  RSS individuel exact n'est inventé. Les coûts partagés restent explicites.
- Les groupes séparés communiquent par MQTT. Le partage du tas dans un groupe
  ne constitue pas encore une mémoire mmap partagée entre processus : ce
  transport, ses versions et sa reprise après panne restent distincts.

## Première verticale exécutable

Le banc de `tools/module-host-poc` réutilise les modèles métier purs TaHoma et
Ariston sur les fixtures publiques, et expose météo/astronomie et Apple TV
dans le même contrat. Les connexions API matérielles, l'appairage Apple TV
et les commandes physiques ne sont pas activés par ce banc.

Les commandes d'essai sont validées et encodées sans être transmises ; leur
résultat ne crée aucune confirmation optimiste. RTS ne publie pas de position.
Le futur exécuteur réel reste soumis à la garde centrale, à une échéance et à
un propriétaire unique ; une origine MQTT externe ne devient jamais Origin::Ui.

Essais : topologies fusionnée/groupée/isolée, identités et valeurs équivalentes,
activation sélective, rejet des propriétaires dupliqués, mesures par groupe,
arrêt d'un processus sans arrêt des autres. Les nouveautés sont un POC de
déploiement et de contrat, pas une bascule des appareils de la maison.
