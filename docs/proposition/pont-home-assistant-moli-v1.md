# Pont Home Assistant ↔ Moli : premier lot de migration

Demande déléguée le28octobre2026 : créer les briques permettant de migrer
progressivement les appareils puis les automatismes, tout en conservant
l’accès depuis HA. Ce premier jalon conserve les automatismes dans HA et
réalise leur chemin de commande vers un volet natif Moli.

## Architecture retenue

MQTT relie le compagnon HA à une brique Rust `moli-home-assistant`.
Le compagnon importe les descriptions et états HA vers Moli, et expose dans
HA les volets natifs sélectionnés dans Moli. Les ordres entrants sont des
requêtes de service externes : `Origin::Api`, jamais `Ui`, `System` ou
`Automation` sur la seule foi d’un champ du message.

Le pilote Overkiz existant reste l’unique effecteur TaHoma côté Moli. Le pont
ne réimplémente pas son API et ne l’appelle pas hors du Hub. La garde, le
journal, l’échéance et le permis vivant du Hub s’appliquent jusqu’à la remise
HTTP de l’ordre, après les attentes de connexion et d’authentification.

Un registre `moli-migration` conserve les correspondances, révisions,
générations, empreintes de plan et preuves de bascule. La politique d’écriture
est appliquée dans le Hub à tous les chemins natifs, pas seulement au MQTT.
Une ancienne génération ne retrouve jamais son autorité après un aller-retour
HA→Moli→HA→Moli. Les écritures se ferment au redémarrage ou en cas de stockage
incertain, jusqu’à réconciliation explicite.

## Périmètre du premier lot

- Inventaire versionné : installation, identifiants de registre stables,
  appareil, canaux, unités, types et capacités. Température et humidité
  restent deux valeurs indépendantes, même dans un seul appareil.
- Commandes de volets : ouvrir, fermer, arrêter ; favori séparé si déclaré.
  Le RTS sans retour ne fabrique ni position ni état ouvert/fermé.
- Bascule durable à phases : préparer, constater la neutralisation de
  l’ancien écrivain, accorder le nouveau propriétaire, vérifier ; retour
  inverse selon les mêmes règles. L’absence de connexion n’est pas une
  preuve de neutralisation.
- Les champs de provenance des automatismes sont préparés dans le registre,
  mais aucun graphe n’est activé par cette brique. Les automatismes demeurent
  dans HA pour la première qualification.

Le changement réel de propriétaire exige une preuve du compagnon concernant
le périmètre complet de l’ancien écrivain. Désactiver un seul miroir Moli
ne coupe pas l’intégration HA qui connaît encore la box. Une preuve absente
ou incomplète bloque la bascule ; aucun booléen fourni par l’interface ne la
remplace. L’adaptateur de neutralisation physique doit être qualifié avant
d’être utilisé sur une installation réelle.

## Contrat MQTT v1

Un préfixe configuré par instance et une installation HA explicite séparent
les maisons. Les topics utilisent des identifiants stables, les noms restent
dans les descriptions. Exemples de suffixes :

| Suffixe | Producteur | Contenu |
| --- | --- | --- |
| `ha/inventory` | HA | Catalogue retained versionné et borné |
| `ha/state/<appareil>/<canal>` | HA | Valeur différentielle et disponibilité |
| `ha/availability` | HA | Session courante et disponibilité |
| `ha/command` | Moli | Action typée sur un volet encore détenu par HA |
| `ha/result/<requête>` | HA | Résultat de cette action, sans texte sensible |
| `moli/exports` | Moli | Volets natifs exportés et capacités |
| `moli/state/<correspondance>` | Moli | État connu, qualité et disponibilité |
| `moli/availability` | Moli | Session courante et disponibilité |
| `moli/command` | HA | Action sur un volet désormais détenu par Moli |
| `moli/result/<requête>` | Moli | Accepté, validation requise, refusé ou incertain |
| `moli/migration/request` | Moli | Phase et défi exact du plan approuvé |
| `ha/migration/proof` | HA | Preuve observée et liée au défi |

Commandes QoS0 non retained, session du destinataire, requête unique,
révision et génération attendues, échéance absolue de5s maximum. Rejeter
commandes retained, sessions anciennes, doublons, révisions périmées et
actions absentes du catalogue. Aucune file d’actions hors ligne ni rejeu
après reconnexion. Une réponse réseau ne prouve pas une position physique.

Descriptions ≤1Mio,1024appareils et4096canaux au total ; commandes ≤1Kio ;
textes d’état ≤4096octets UTF-8 ;16commandes en attente et256reçus par session,
sans éviction qui permettrait une réexécution. Les tables enum et leurs
libellés appartiennent aux descriptions ; les états transportent leur code.

Les deux comptes MQTT doivent avoir des ACL distinctes pour les topics de
leur producteur. Les origines de payload ne sont jamais une autorité. Les
secrets de Moli restent dans son coffre, ceux de HA dans son intégration MQTT.

## Qualification

Tests synthétiques des générations, de la garde, des échéances après attente
réseau, du stockage avant/après renommage, de la réconciliation, des deux
directions MQTT et des volets RTS. La migration des automatismes et les
adaptateurs de bascule réels constituent des jalons suivants, avec preuve
distincte d’absence de double exécution.

Une qualification physique ultérieure utilisera une cible neutre, un humain
présent et un état initial rétabli, conformément à `AGENTS.md`. Aucun appareil
ni HA réel n’est modifié pour développer ce premier lot.

Sources de contrat consultées le28octobre2026 :
[API WebSocket HA](https://contributors.home-assistant.io/core/internals/websocket-api/),
[entités cover](https://contributors.home-assistant.io/core/entities/cover/),
[client MQTT HA](https://github.com/home-assistant/core/blob/dev/homeassistant/components/mqtt/client.py).
