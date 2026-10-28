# Pannes du pont entre deux instances autonomes sur le Mac

L’utilisateur a précisé le périmètre : simuler la perte du pont entre deux
instances sur le Mac. Ce test ne nécessite aucun accès VPN ni déploiement
sur les hôtes Home Assistant.

## Dispositif

Deux processus Rust exécutent `--site-bench`, chacun avec son courtier MQTT,
son primaire et son autorité locale pour sa propre maison, et une copie
passive de l’autre. Les structures des deux exports HA passent par la
projection et l’anonymisation existantes. Les seuls changements produits
pendant le test sont synthétiques. Aucun pilote physique n’est chargé.

Les deux instances utilisent `127.0.0.1` avec des ports distincts :
`mqtt_port` est l’entrée de réplication de l’instance, `peer_mqtt_port` la
destination du pont. L’option explicite `loopback_fixture` exige exclusivement des adresses
loopback. Chaque primaire produit ses propres changements toutes les2s ; le
contrôleur Python n’envoie aucun changement d’état. Les contrôles de statut
et de panne sont séparés de la liaison MQTT interrompue.

## Critères annoncés avant exécution

| Cas | Durée visée | Critères spécifiques |
| --- | ---: | --- |
| Fermeture des deux ponts TCP | 8s | Anciennes connexions fermées, copies figées, reprise par deltas après reconnexion |
| Silence bref des deux ponts | 3s | Aucun octet transmis après stabilisation ; reprise sans changement de génération ni nouvelle connexion |
| Arrêt du seul flux de réplication B→A | 8s | Copie de B chez A figée ; copie de A chez B continue ; deux primaires actifs |
| Silence prolongé des deux ponts | 120s | Retard supérieur au seuil de16deltas ; reprise par instantané et convergence |

Pendant chaque cas : baux locaux valides, rôles propres actifs et rôles
étrangers passifs, époque1, progression des rangs et du nombre d’effets
factices, zéro commande physique. La file reste sous256deltas/1Mio, le nombre
de connexions entrantes de pair sous16. Le mécanisme de suspension n’ajoute
pas de tampon applicatif ; les files MQTT, instantanés et buffers du noyau
ont leur propre coût.

La convergence se mesure sans geler les primaires : lire la source, la copie
distante, puis la source à nouveau ; les trois rangs et empreintes doivent
être identiques. La reprise doit satisfaire ce critère pour les deux maisons
dans une échéance annoncée de30s après rétablissement. Les durées mesurées
sont locales, en debug, et ne constituent pas des RTO intersites.

## Mesures et limites

Le rapport conserve les états avant, pendant et après, les compteurs de
connexion et de reprise, ainsi que les allocations vivantes. La RSS et le
temps CPU sont relevés par PID avec `ps` sur macOS ; le pourcentage CPU est
rapporté à un cœur. Le contrôleur est mesuré séparément. La RSS ne s’ajoute
pas aux allocations vivantes et une somme de RSS ne mesure pas des pages
physiques uniques. Les maximums RSS sont des maximums échantillonnés.

Les effets vérifient l’admission déterministe, pas une action d’appareil.
L’égalité finale prouve la convergence de l’état ; elle ne prouve pas la
livraison de chaque état intermédiaire. La campagne n’exécute aucune
promotion étrangère et ne qualifie pas un consensus distribué, une panne
électrique, la perte du Mac entier ni les appareils réels.

Chaque instance a une durée de vie bornée ; le contrôleur arrête et récolte
ses deux enfants, y compris après un échec. Les services1414 et1416 en cours
ne sont pas reconfigurés par cette commande.

## Reproduction

```sh
python3 tools/ha-multihome-poc/run-link-faults.py
```

`--skip-check` réutilise les contrôles du binaire courant ; il ne les remplace
pas. `--fixture` remplace les deux exports par une petite fixture synthétique.
`--quick` raccourcit le cas long et ne qualifie pas une interruption de120s.

Résultats : `ha-multimaison-pont-local.json` ; rapport HTML et reçu de preuves
distincts des campagnes locales et interhôtes antérieures.
