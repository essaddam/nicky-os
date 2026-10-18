# Recette : ballon d'eau chaude sur relais

Un ballon électrique ordinaire, alimenté par un relais (un disjoncteur ou une prise
Zigbee, vus par `z2m`), et une maison qui choisit quand il chauffe. `ballon.json` reprend le
paquet HA de trois automatismes qui le faisait : un **sélecteur** et trois **brouillons**.
Ce n'est pas un pilote : le relais a déjà le sien, le sélecteur est une aide.

## Ce qui est repris

| HA | Moli |
|---|---|
| `input_select` « Mode Chauffe Eau » : `Éteint`, `Automatique`, `Forcer` | aide `select` `ballon_mode`, mêmes choix, `initial = "Éteint"` (le premier choix, comme HA sans `initial`) |
| « Ballon Mode Automatique » : au choix `Automatique` et à 01:20, 07:20, 11:50, 13:50 ; si toujours `Automatique` : relais fermé de 01:20 à 07:20 et de 11:50 à 13:50, ouvert sinon | brouillon du même nom : quatre `at_time` et un `when_state`, puis deux `if` (`time` : début inclus, fin exclue, comme HA) |
| « Ballon Mode Forcer » : au choix `Forcer`, relais fermé | brouillon du même nom |
| « Ballon Mode Éteint » : au choix `Éteint`, relais ouvert | brouillon du même nom |
| capteurs gabarits « chauffe active », « conso instantanée », « conso cumulée » | rien à écrire : ce sont les points du relais lui-même (`state`, `power`, `energy`), son **état réel** tel que le pont le rapporte |

Mode `single`, comme les automatismes HA par défaut.

## Mettre en place

1. Déclarer l'aide dans l'instance des aides (`kind = "helpers"`) :
   ```toml
   [[driver.options.helper]]
   id = "ballon_mode"
   name = "Mode Chauffe Eau"
   kind = "select"
   options = ["Éteint", "Automatique", "Forcer"]
   initial = "Éteint"
   ```
   Reprendre la valeur que HA avait au moment de la bascule (la choisir une fois dans le
   tableau de bord) : Moli la garde ensuite à travers les redémarrages.
2. Dans `ballon.json`, remplacer `{aides}` par l'id de l'instance des aides et `{relais}` par
   le point du relais (`<appareil>/state`, par exemple `z2m:0x00124b00aabbccdd/state`).
3. Enregistrer chacun des trois comme **brouillon** (outil MCP `save_automation_draft`, ou
   `POST /api/automations` sans session) : un agent ne peut rien de plus.
4. Un humain relit chaque brouillon dans l'éditeur (phrase, appareils, heures), ajuste les
   heures creuses au contrat de la maison s'il le faut, puis **valide** avec son code.
   **Au même moment**, il désactive l'automatisme HA du même nom : un seul propriétaire par
   usage, sinon HA et Moli commandent le même relais.
5. Ne rien fusionner avec les modes de la maison, l'armement de l'alarme ou les
   réservations : le ballon a son propre sélecteur. Un mode « Absent » qui devrait couper
   l'eau chaude est un **autre** automatisme, à décider à part.

## Redémarrage et état réel

- **Aucune action implicite au démarrage**, comme dans HA : pas de déclencheur `on_start`,
  et une valeur vue pour la première fois (le sélecteur rechargé, le relais relu) n'est pas
  un changement. Le test `crates/moli-automation/tests/ballon.rs` le vérifie : sélecteur sur
  `Automatique` au démarrage, aucun ordre au relais.
- Conséquence, la même que dans HA : si Moli est arrêté à 07:20, le relais reste fermé
  jusqu'à 11:50. Ajouter un rattrapage au démarrage serait un choix nouveau, à faire valider
  par un humain, pas une reprise.
- Le relais lui-même décide de son état après une coupure de courant (réglage de l'appareil
  dans Zigbee2MQTT) : le vérifier à la qualification.
- Ce que montre Moli est l'état rapporté par le relais, jamais celui demandé.

## Qualifier (après le jalon H1)

Humain présent, ballon visible : passer en `Forcer` (le relais ferme, la puissance monte),
`Éteint` (il ouvre), puis `Automatique` hors et dans une plage d'heures creuses ; remettre
le choix d'avant. Ce n'est qu'ensuite que HA cède la fonction.
