# Perte de liaison : protocole du test TCP

Le test étend le banc de rôles croisés avec un relais TCP réel en loopback.
Il ne coupe ni le réseau des HA ni leur VPN. Le contrôle local, la source de
données du banc et le témoin central restent joignables. Ce périmètre est
séparé d’une panne totale de site ou d’une panne du coordinateur.

## Dispositif

Deux processus portent chacun la copie des deux structures HA anonymisées.
Pour ce mode de test seulement, chaque nœud a deux connexions MQTT :

- canal local direct : sa maison primaire, statut et battements ;
- canal passant par le relais : mutations et réponses de l’autre maison.

Les opérations d’une maison étrangère sont refusées sur le canal direct,
même lorsque le relais fonctionne. La coupure ne peut donc pas être évitée
par ce canal. Aucun flag d’isolation logique d’Engine n’est utilisé.

## Cas et critères

1. **TCP fermé** : fermeture des flux existants, refus des reconnexions.
2. **TCP silencieux** : sockets laissées ouvertes mais aucune lecture/écriture
   transmise dans les deux sens. Au retour, les anciens flux sont fermés afin
   de jeter les octets en attente et de repartir avec une session propre. La
   reprise du trafic sur les mêmes sockets reste à qualifier.

Après une courte stabilisation du défaut, une mutation distante dans chaque
sens doit expirer. Pendant au moins cinq secondes d’exercice local, les deux
primaires doivent avancer, garder leur bail et admettre leurs effets factices.
Les copies croisées doivent rester au rang et à l’empreinte précédents ; le
compteur d’octets transmis du relais doit rester constant. Une promotion
manuelle du passif périmé doit être refusée.

Au retour du lien, le test attend une **nouvelle génération de connexion MQTT**
pour chaque nœud, pas seulement un ancien booléen « connecté ». Les deux copies
sont remises au rang courant et leurs empreintes comparées. Un compteur worker
hors document, conservé malgré les instantanés, doit prouver qu’aucune sonde
ayant expiré n’a été appliquée, même brièvement. Un test positif vérifie que
ce compteur peut réellement augmenter puis survivre au remplacement du document.

Le rapport distingue durée totale du défaut, intervalle de progression locale,
reconnexion et resynchronisation complète. Les effets sont cumulés depuis le
début du test. Les mesures loopback/debug ne sont pas des RTO intersites.

## Reproduction

Commande Dev « Maisons — coupure TCP et silence réseau » :

```sh
python3 tools/ha-multihome-poc/run-poc.py --network-loss
```

`--fixture` permet un essai sans exports privés. Le mode ordinaire des nœuds
est aussi exercé par un smoke test afin de vérifier que l’ajout des canaux
optionnels conserve le comportement précédent. Aucun service1414/1416 en cours
ni appareil physique n’est arrêté par cette commande.

Résultats : `ha-multimaison-tcp.json` ; empreintes et contrôles :
`ha-multimaison-tcp-preuves.json`. Les preuves antérieures restent datées et
ne sont pas réattribuées rétroactivement au nouveau binaire.
