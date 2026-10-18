# Capteurs et lumières : cadrage des intégrations

> Étude du 18 octobre 2026. Contrats proposés pour quatre sous-lots indépendants ;
> aucun de ces pilotes n'est livré par cette note. Le catalogue courant reste
> inchangé. La relecture du cadrage ne vaut ni validation du matériel ni bascule.

L'objectif est de reprendre des mesures physiques et des commandes élémentaires
sans modifier le chantier de stockage. Les règles [D1, D2 et D5](../ARCHITECTURE.md)
et le [contrat des pilotes](pilotes.md) s'appliquent à chaque sous-lot.
Les inventaires, correspondances d'identités et configurations d'installation
restent hors du dépôt. Aucun extrait de capture privée n'est repris ici.

## Découpage

| Sous-lot | Livrable attendu | Effort indicatif |
|---|---|---|
| EnOcean MQTT | conserver le pont, adapter ses sujets et EEP documentés | 3 à 8 jours |
| Netatmo météo | OAuth, station, modules et fraîcheur des mesures | 5 à 12 jours |
| Twinkly | session HTTP locale, marche, luminosité et couleur statique | 3 à 7 jours |
| Xiaomi BLE | réception d'annonces, décodage local, disponibilité et familles B1 | 3 à 8 jours |

Ces fourchettes sont des références de préparation, sans engagement de calendrier.
Chaque lot livre `integrations/<id>/` : manifeste, profil **ou** crate native,
fixtures anonymisées, onboarding et README distinguant repris, deviné et restant.
Un sous-dossier vide ou un manifeste pointant vers une crate absente ne constitue
pas un paquet. Aucun paquet provisoire ne doit rendre le catalogue trompeur.

Le moteur de profils actuel traite HTTP avec des lectures et écritures déclarées ;
il ne propose ni abonnement MQTT, ni BLE, ni machine OAuth, ni challenge de session.
La cible proposée est donc un pilote natif par protocole. EnOcean pourra employer
un profil si, lors de son implémentation, le moteur sait exprimer **tout** son contrat
sans contourner la garde ; étendre le moteur général n'est pas un prérequis implicite.

## Xiaomi BLE : radio et décodage

**Voie proposée : proxy ESPHome recevant les annonces BLE, décodage dans Moli.**
Cela évite de lier le binaire portable à BlueZ, D-Bus et aux droits radio de l'hôte.
Le proxy est un service extérieur, à inventorier et à autoriser : son existence ou
sa couverture radio n'est pas présumée. Aucun achat ni flash n'est lancé par ce choix.
Si cette voie n'est pas disponible, réexaminer le lot avant développement plutôt que
d'ajouter silencieusement une dépendance à la radio de l'hôte.

| Voie | Intérêt | Coût ou limite |
|---|---|---|
| Proxy ESPHome (proposition) | radio placée près des capteurs, cœur sans BlueZ | client d'annonces à ajouter ; proxy et liaison à qualifier |
| BLE de l'hôte | pas de proxy supplémentaire si la radio est disponible | dépendance Linux/BlueZ, droits, conteneurisation et portée à établir |
| Deux voies dès le premier lot | plusieurs récepteurs possibles | double contrat de transport et déduplication ; différé |

Le [proxy ESPHome](https://esphome.io/components/bluetooth_proxy/) transmet des
annonces sans nécessiter de connexion GATT au capteur. Son mode de connexion active
et le scan actif sont deux réglages distincts. Le lot vise la réception d'annonces,
sans GATT, appairage, flash ou changement de configuration du capteur.
Le [schéma API ESPHome 2026.9.0](https://github.com/esphome/esphome/blob/2026.9.0/esphome/components/api/api.proto)
sert de référence initiale à vérifier contre la version effectivement utilisée.
Le client `moli-esphome` actuel est orienté audio ; ses messages dans
`crates/moli-esphome/src/api.rs` ne couvrent pas les annonces Bluetooth.
Partager un codec borné est possible ; faire passer un capteur pour un satellite
audio ne l'est pas. La session du proxy doit être chiffrée, sa clé au coffre.

Le décodeur pur doit accepter uniquement les modèles et objets justifiés par des
contrats publics et des fixtures. UUID de service, version, bits de contrôle,
identité embarquée, longueur et objets sont vérifiés avant lecture d'un champ.
Modèle ou objet inconnu : ignoré sans créer de capacité. Une trame tronquée,
un dépassement de taille ou un échec d'authentification ne rafraîchit aucune mesure.
Les annonces chiffrées exigent une clé humaine au coffre ; aucun extracteur de compte
cloud ni secret issu d'une capture n'entre dans le pilote. Une variante non prise en
charge doit être signalée et rester sans valeur valide.

La [référence Xiaomi BLE](https://github.com/Bluetooth-Devices/xiaomi-ble) fournit
un parseur et des tests à étudier avec leurs licences. Son
[traitement de batterie](https://github.com/Bluetooth-Devices/xiaomi-ble/blob/main/src/xiaomi_ble/parser.py)
peut calculer une tension depuis le pourcentage : une telle valeur est une estimation,
jamais une mesure physique indépendante. Le README du futur paquet doit préciser
la provenance et la formule de tout point dérivé ; ne pas inventer un relevé manquant.

Chaque capteur a son identité stable, sa dernière réception valide et un délai de
silence réglé selon son contrat. Une annonce valide inchangée maintient la disponibilité
sans produire de faux changement de température. Le numéro de trame ne suffit pas
seul à dédupliquer : retour à zéro, débordement et redémarrage doivent être testés.
Un même paquet reçu par plusieurs voies ne devient pas plusieurs mesures.
La fraîcheur d'une température ne se renouvelle pas parce qu'une batterie ou un RSSI
vient d'arriver. Un lien proxy opérationnel ne suffit pas à déclarer tous les capteurs
en ligne ; une perte du lien rend indisponibles ceux dont il est l'unique source.

## Familles B1 et bruit

Le contrat repose sur [`Family::of`](../../crates/moli-history/src/policy.rs),
qui classe la sémantique puis l'unité. Il ne demande aucun nouveau moteur de stockage.

| Point proposé | Sémantique / unité | Famille actuelle |
|---|---|---|
| température | `Temperature`, °C | `climate` |
| humidité | `Humidity`, % | `climate` |
| pression | `Pressure`, hPa | `climate` |
| CO₂ | `Other`, ppm | `climate` |
| niveau de pile | `Battery`, % | `diagnostic` |
| tension de pile, clé `battery_voltage` | `Voltage`, V ou mV | `diagnostic` |
| signal reçu | `SignalStrength`, dBm | `diagnostic` |
| marche, luminosité, couleur d'une lampe | `OnOff`, `Brightness`, `Color` | `states` |
| pluie, vent, bruit acoustique | `Other`, mm, km/h, dB | `other` |

Le bruit acoustique en dB n'est pas un signal radio. Une mesure en % sans sémantique
ne suffit pas à obtenir la famille `climate`. La tension de pile en V ne doit pas être
nommée simplement `voltage`, ce qui la classerait dans la puissance secteur.
Tester cette table sur les `PointSpec` réellement construits par chaque pilote.

`set_state` diffuse les changements ; `pulse_state` est réservé aux occurrences
réelles (deux appuis identiques restent deux appuis). Les compteurs de trames et dates
de réception ne doivent pas transformer toute annonce en événement d'historique.
Dédupliquer les transports, ne pas arrondir arbitrairement une mesure pour réduire
les écritures. La limitation du nombre de lignes dépend aussi des changements reçus
et du filtre, pas de la seule durée de conservation.

Les durées suivent [B1](historique-energie.md#politiques-de-conservation-storage-cratesmoli-historysrcpolicyrs) :
règle de point, puis famille, puis `history_days`. Sans `[storage]`, la durée générale
reste appliquée ; les fenêtres cibles de D13 ne sont pas activées par cette note.
L'onboarding doit proposer une politique explicite, sans l'appliquer à la place de
l'humain. Réduire un plancher déjà acquis exige la validation humaine B1 ; aucune
réduction automatique, désactivation de filtre protecteur ou purge propre au pilote.
La conservation et la fraîcheur sont deux contrats distincts.

## EnOcean : conserver le pont MQTT

L'intégration se branche sur le broker et le pont radio existants. Elle ne prend pas
le port série, ne réassocie pas les appareils et ne remplace pas ce pont par Z2M.
La publication du [pont EnOcean MQTT](https://github.com/embyt/enocean-mqtt)
documente plusieurs modes de sujets, canaux et formats ; elle ne prouve pas le schéma
d'une autre version ou d'un autre dérivé. Aucun sujet d'écriture n'est à deviner.

Avant le mapping : relever **hors dépôt** version du pont, EEP (`RORG/FUNC/TYPE`),
canal, identité radio, sujets d'état et de commande, type/échelle, retain, QoS,
horodatage et signal de vie. Produire ensuite un contrat public fictif explicitement
distinct d'une capture. Si les fichiers de mapping ou les messages manquent, rester
en étude ; un add-on démarré ne prouve aucun EEP ni retour d'état.
L'identité radio est séparée du libellé de sujet, qui peut changer.

Un acquittement broker atteste la publication MQTT, jamais le mouvement de l'appareil.
Les capacités sans retour vérifiable restent sans état confirmé ; ne pas recopier
la consigne envoyée comme mesure. Retours tardifs, commandes concurrentes et canaux
doivent être corrélés selon le protocole. Pas de commande retenue par le broker,
pas de rejeu d'ordre après reconnexion ; un `PUBACK` manquant n'autorise pas un doublon
physique. Les messages retenus sans date fiable ne prouvent pas la présence actuelle.
Un interrupteur sans télégramme périodique ne doit pas être déclaré en panne au seul
motif qu'il n'a pas été actionné ; sa couverture reste explicitement non prouvée.

## Netatmo : météo et OAuth

Périmètre : station et modules de mesure, en lecture seule. Ni commande de chauffage,
ni caméra, projecteur, webhook ou média. Une entrée de registre caméra ne prouve pas
son usage : créer un sous-ticket caméra seulement après confirmation de l'usage.
Open-Meteo ne remplace pas les mesures physiques.

L'[OAuth officiel](https://dev.netatmo.com/apidocumentation/oauth) et l'[API météo](https://dev.netatmo.com/apidocumentation/weather)
sont les références à figer lors du lot. Autorisation humaine, permission minimale
`read_station`, destinations fournisseur fixes ; pas d'URL reçue du modèle, d'une réponse
ou d'une redirection. Le retour OAuth doit vérifier un `state` aléatoire, borné,
à usage unique et à expiration ; le code n'est jamais journalisé.
Les identifiants d'application et le jeton de renouvellement entrent au coffre par
l'humain. Aucun mot de passe de compte n'est demandé par le pilote.

Un seul renouvellement en vol. Conserver le nouveau jeu de jetons durablement avant
de remplacer le précédent en mémoire ; une rotation dont la sauvegarde échoue ne
doit pas être déclarée saine. Durée calculée depuis la réponse, renouvellement avec
marge bornée ; jamais de boucle infinie sur 401, `invalid_grant` ou consentement retiré.
Ces derniers cas passent en attente d'intervention humaine. Temporiser les erreurs
réseau, 429 et 5xx sans rafale. Tests de logs et d'erreurs : ni jeton, ni code OAuth,
ni corps d'authentification, même sur une réponse malformée.

Utiliser l'identité station/module du fournisseur, pas sa position dans un tableau
ni son nom. La date de mesure, le signal d'accessibilité et la présence du module
doivent décider de sa fraîcheur, séparément du succès HTTP. HTTP 200 contenant une
ancienne mesure ne réanime pas le module ; module absent ne veut jamais dire zéro.
Une disparition prolongée et sa réapparition gardent une identité cohérente.

## Twinkly : session et état observé

Le [protocole décrit par XLED](https://xled-docs.readthedocs.io/en/latest/rest_api.html)
est une référence communautaire, pas une garantie du fabricant : challenge aléatoire,
login, vérification et jeton temporaire. Sa vérification peut invalider une autre
session. **Même le login réel attend H1**, après désignation d'un seul propriétaire
d'exécution ; aucun scan authentifiant en arrière-plan durant la coexistence HA.

Une session liée à l'identité et à l'hôte validés, jamais de redirection. Jeton uniquement
en mémoire, effacé à l'arrêt ; aucun jeton dans un TOML, fixture ou message d'erreur.
Durée lue dans la réponse avec bornes de validité, pas une constante supposée par
firmware. Un renouvellement ne doit jamais rejouer automatiquement un ordre physique.

Marche, luminosité et couleur statique ne sont publiées comme confirmées qu'après
relecture. Distinguer pourcentage et échelle du cœur. Une réponse HTTP réussie dont
le code applicatif refuse l'action reste un échec. Une lecture périmée, malformée
ou en échec ne laisse pas une lampe éternellement joignable.
Les capacités de couleur doivent être annoncées par un contrat de modèle/firmware :
les chemins qui exigent un téléversement de film ne sont pas un repli implicite.
Effets, sélecteur d'effets, playlists, synchronisation, temps réel et téléversements
sont hors périmètre. Allumer n'autorise pas à remplacer un effet sauvegardé par un autre.

Le pilote natif `moli-twinkly` et son [paquet](../../integrations/twinkly/README.md)
implémentent ce sous-lot pour API v1 RGB sur contrats simulés : marche, luminosité
et couleur directe, sans effet ni upload. Les autres sous-lots gardent leur état
propre. Cette livraison de développement ne lève pas H1 et ne clôture pas le parent
fonctionnel. Les modèles/firmwares réellement installés restent à qualifier par
l'humain selon [l'onboarding](../../integrations/twinkly/onboarding.md).

## Vérification par sous-lot

Avant toute qualification réelle, les contrats sont exercés sur des fixtures publiques
ou synthétiques : provenance, licence, transformations et statut sont documentés.
Ne jamais qualifier une donnée synthétique de capture d'appareil. Aucun secret
réel ou de test n'est commité ; pour tester une clé, la générer à l'exécution.

| Surface | Cas obligatoires |
|---|---|
| BLE | troncatures à chaque champ, longueur incohérente, type inconnu, compteur rebouclé, doublon, silence et reprise, clé absente/invalide si applicable |
| MQTT EnOcean | mauvais sujet/EEP/canal, payload excessif, retained ancien, doublon broker, appuis répétés, perte de connexion, absence de retour physique |
| Netatmo | rotation et redémarrage, sauvegarde refusée, 401/429/5xx, retrait du consentement, module absent, mesure ancienne, absence de fuite |
| Twinkly | challenge et durée invalides, session expirée, code applicatif en erreur, écriture envoyée sans réponse, relecture contradictoire, mode couleur indisponible |
| Hub et cycle de vie | lecture seule refusée, garde 202 sans envoi, file pleine, ordre abandonné avant chaque envoi, aucun rejeu, arrêt/reconnexion hors ligne |
| B1 | familles des vrais points, rafale inchangée sans inflation, variation valide conservée, fraîcheur indépendante et baisse de plancher sans purge non autorisée |

Les lots doivent chiffrer leurs limites avant implémentation : octets par message,
nombre d'objets et d'appareils, longueur des sujets/textes, taille et politique de file,
délais réseau, reprises et rythme de publication. Refuser l'excès avant allocation
proportionnelle, ne pas accumuler de corps chunked ou décompressé sans borne.
Mesurer une source bavarde et son effet sur le bus sans changer le budget global de
[performance](../PERFORMANCE.md). Une brique désactivée ne crée aucune tâche.

Chaque livraison passe `sh scripts/check.sh` sous Linux dans son worktree, catalogue
compris ; tout texte visible nouveau existe en français et en anglais. Ajouts aux
listes partagées dans l'ordre alphabétique. Les contrôles lourds sont coordonnés avant
lancement. Les journaux, codes de sortie et empreintes se conservent hors dépôt.

Après H1 seulement : humain présent, cible neutre, jamais de pièce protégée, état
initial relevé et rétabli. HA garde la fonction jusqu'à cette validation, avec un seul
propriétaire d'exécution par usage. La clôture du cadrage ne ferme pas les sous-lots ;
le parent fonctionnel reste ouvert tant que leurs critères de livraison ne sont pas
satisfaits et relus.
