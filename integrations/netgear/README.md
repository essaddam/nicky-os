# Netgear : SOAP de lecture

Contrats synthétiques uniquement ; aucun modèle ou firmware réel qualifié.
La porte H1 de `docs/components/reseau-qualification.md` reste humaine.

## Repris

Connaissance du protocole [pynetgear, révision figée
 d9b34149b110cd91fbccc5fff7a285a1221af5dd](https://github.com/MatMaul/pynetgear/tree/d9b34149b110cd91fbccc5fff7a285a1221af5dd),
MIT : router.py, const.py et helpers.py. Aucun code tiers copié.
Les fixtures sont entièrement synthétiques (licence Apache-2.0 du dépôt), identités et
noms fictifs, sans capture privée ni secret fonctionnel. Les cookies de tests sont créés
en mémoire. `cargo test -p moli-netgear` les rejoue, le catalogue valide leur présence.

Une seule destination fournie par l'humain, POST `/soap/server_sa/` seulement :

| Service | Action |
|---|---|
| DeviceConfig:1 | SOAPLogin (login v2), GetTrafficMeterStatistics |
| DeviceInfo:1 | GetInfo, GetSystemInfo, GetAttachDevice (inventaire v1), GetAttachDevice2 (v2) |
| ParentalControl:1 | Authenticate (login v1) |
| WANEthernetLinkConfig:1 | GetEthernetLinkStatus |

Pas de détection de ports, de login alternatif automatique, d'activation du compteur,
de session de modification, de Set*, de speedtest, de blocage ou de redémarrage.
Les connexions ne suivent ni redirection ni adresse reçue dans un document.

## Conservateur et explicite

- `SerialNumber` est l'identité native du routeur ; empreinte conservée dans son coffre
  sous `device_identity`. Toute disparition ou substitution empêche la publication.
- Inventaires v1 comptés et v2 XML : validation complète avant tout changement ; au plus
  512 entrées, sans troncature, doublon ou réparation. Les MAC natives des clients sont
  des identités locales. Aucun point nom, MAC, adresse IP, SSID ou présence.
- Sélection humaine de 32 clients au plus, vide par défaut (aucune lecture d’inventaire). Les appareils sélectionnés
  ne sont créés que lorsqu'un inventaire valide les observe. Une absence n'est pas une
  déconnexion prouvée : leurs mesures deviennent inconnues, jamais `associated=false`.
- Type de lien selon une liste fermée. Le signal reste brut sans unité inventée ; les
  débits nus (`866`, `1000`) ne prouvent pas une unité. Seul un débit explicitement
  marqué `Mbit/s` peut devenir une mesure de débit de lien.
- Le format source des compteurs de trafic ne prouve pas leur unité/base pour chaque
  firmware. Les nombres nus sont donc ignorés ; seule la durée `HH:MM` documentée par
  le parseur amont est convertie en secondes. `--`/champ absent signifie inconnu.
  La collecte n'active jamais Traffic Meter ; inactif ne devient pas zéro.
- Une liste « attachée » ne prouve pas l'association actuelle d'un appareil. Ce lot
  n'expose donc pas de point d'association ; la présence existante reste inchangée.
- Pas de points dynamiques issus des champs inconnus. Les capacités facultatives
  absentes sont mémorisées pour la session. Une nouvelle authentification les réévalue.

## Déduit, à qualifier

Le `SessionID` XML est créé au hasard (20 caractères hexadécimaux), puis renouvelé à
l'authentification. L'amont utilise une constante issue d'une capture ; cette valeur
n'est pas reprise. La tolérance de chaque firmware à cet identifiant reste à qualifier.
L'en-tête `Content-Type` reprend `multipart/form-data` du client amont, bien que le corps
soit du SOAP XML. Aucun format alternatif n'est essayé contre un équipement.
Les valeurs de CPU/mémoire sont les champs d'utilisation ; leur interprétation physique
reste à comparer à la source existante à H1. Ni le signal brut ni les débits nus ne sont
convertis par supposition.

## Bornes

1 Mio et 5 s par échange, 30 s/16 échanges par cycle (login compris), un en vol.
XML : profondeur 16, 10 000 éléments, champ 4 Kio, aucun DTD ou résolveur d'entité.
8 cookies/4 Kio au total, destinés au seul hôte déclaré, aucun contrôle dans les en-têtes.
Un seul relogin par cycle, puis attente humaine sur refus ; aucun retry de login refusé.
Mesures 60 s, inventaire 300 s, informations 3600 s ; plancher 30 s et aucun rattrapage.
Arrêt annulable, aucune tâche si l'instance est absente/désactivée. Points en lecture
seule ; valeurs identiques sans événement d'état. Maximum contractuel 144 points
(16 équipement + 4 par client), le sous-ensemble qualifié ici reste inférieur.

## Reste à qualifier

Formes exactes par matériel/firmware, cookies, TLS, unité des compteurs et débits nus,
sens et unité du signal, association actuelle, satellites possédant leur propre identité.
Aucune publication de satellite comme routeur de substitution ; aucune lecture
`GetAllSatellites` ni connexion à un satellite découverte par l'inventaire.
Les tests de contrat ne prouvent ni le budget CPU/mémoire du matériel, ni une reprise HA.
