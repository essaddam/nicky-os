# Netgear : lectures SOAP bornées

Le paquet [integrations/netgear](../../integrations/netgear/README.md) met en œuvre N3
sans appel réel. Lire le [cadrage réseau](reseau.md) et la [porte humaine H1](reseau-qualification.md).
Ce contrat ne qualifie aucun matériel/firmware et ne transfère aucun usage depuis HA.

`moli-netgear` est un pilote natif supervisé, construit uniquement pour une instance
configurée et active. L'humain choisit hôte/port, transport et variantes login/inventaire.
Aucune adresse découverte, redirection ou URL XML n'est suivie ; aucun port n'est essayé.
L'identité routeur est le numéro de série, épinglé au coffre. Les clients sont des identités
natives sélectionnées, sans création automatique, rapprochement de présence ni métadonnée
personnelle en point. Leurs noms d'affichage initiaux sont génériques et traduits.

Lectures : `DeviceInfo:1/GetInfo`, `GetSystemInfo`, `GetAttachDevice` ou `GetAttachDevice2`,
`WANEthernetLinkConfig:1/GetEthernetLinkStatus`, `DeviceConfig:1/GetTrafficMeterStatistics`.
Authentifications distinctes : `ParentalControl:1/Authenticate` et `DeviceConfig:1/SOAPLogin`.
POST fixe `/soap/server_sa/` ; pas d'administration ou de configuration de session.

Les valeurs observées qualifiées sont CPU/mémoire en %, version du firmware, lien WAN,
durée de connexion du jour en secondes, et pour les clients choisis type de lien, signal
brut et débit avec unité explicitement fournie. Les compteurs nus de trafic ne prouvent
pas une unité et ne deviennent pas des octets. Les méthodes absentes sont mémorisées pour
la session ; un champ disparu efface sa mesure en `Null`. Les autres erreurs invalident
le cycle entier avant publication et mettent les appareils hors ligne. L'authentification
refusée ou l'identité modifiée réclame un humain, sans boucle de login.

Mesures toutes les 60 s, inventaire toutes les 5 min, informations stables à 1 h, plus une
vérification d'identité à chaque authentification. Délais après achèvement, aucun rattrapage.
Un échange en vol, 1 Mio/5 s, un cycle 30 s/16 échanges, un seul relogin. Sessions/cookies
locaux et en mémoire (8/4 Kio). XML borné et strict, DTD et entités externes interdits.
Sélection <=32, inventaire <=512 ; budget contractuel 144 points maximum, sous-ensemble
qualifié inférieur ici. Valeurs identiques via `set_state`, sans événement d'état ajouté.

Les preuves sont les tests natifs `cargo test -j 2 -p moli-netgear` sur fixtures synthétiques,
transport scripté et serveurs HTTP/TLS éphémères. Les certificats de test sont créés en
mémoire, sans clé privée au dépôt. Le statut de validation de livraison est consigné au
ticket ; la compatibilité réelle et les budgets matériels demandent H1.
