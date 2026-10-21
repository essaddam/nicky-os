# Google Cast : récepteurs vidéo

Pilote natif `cast` construit sur `moli-androidtv::cast`. Il observe la session
média existante et fournit play/pause/stop, volume du récepteur (0–100 %) et
sourdine absolue. Une application Cast reste nécessaire pour lancer un contenu ;
aucun compte fournisseur, catalogue, télécommande Android, alimentation ou
lancement d’application n’est inventé.

## Activation et identité

Une instance gère un seul récepteur vidéo. Au démarrage sans validation durable,
il n’existe aucun appareil dans le hub, aucune découverte et aucune connexion.
Dans **Système**, une personne lance la découverte sur l’interface déclarée,
vérifie le nom, l’adresse et l’identité, confirme la propriété exclusive et valide
avec son PIN. La sélection exacte est enregistrée au coffre avant publication.
Une validation durable est restaurée au redémarrage, sans nouvelle découverte.
La désactivation retire l’appareil et invalide les commandes non parties.

La découverte utilise l’ID natif TXT Cast (32 caractères hexadécimaux). Pour une
adresse manuelle, l’identité est `<instance>:receiver`, indépendante de l’IP ou
du nom ; réserver cette instance à un seul appareil et désactiver avant de la
réaffecter. La sélection et le pin TLS sont réunis dans l’entrée chiffrée
`approval`. Le pin TOFU est enregistré avant la première trame Cast ; un
certificat différent bloque la connexion sans effacer automatiquement le pin.
Aucun pin n’est exposé dans les vues ou les audits HTTP.

## Propriété explicite

Les instances Cast partagent leurs réservations par ID et IPv4. Les hôtes des
pilotes TV activés `androidtv`, `philips` et `samsung` sont réservés avant leur
démarrage ; leurs alias DNS sont vérifiés au moment de la validation. Un alias
non résolu fait refuser la validation, sans essayer de prendre la place du TV.
Les identités gérées par Music Assistant peuvent être déclarées dans
`external_ids`. Moli ne découvre pas automatiquement cette propriété dans MA :
la confirmation du propriétaire reste obligatoire. Les appareils audio seuls
ne sont pas activables et restent à gérer par Music Assistant.

## Transport et limites

mDNS RFC 6762, requête à réponse unicast sur l’interface IPv4 privée choisie,
sous-réseau /16 à /30 : 3 s, 256 datagrammes de 9 000 octets au maximum,
128 records DNS par paquet et 64 candidats. PTR/SRV/TXT/A doivent former une
réponse cohérente dans un même paquet, port 8009 et adresse A identique à
l’émetteur. Les réponses fragmentées entre paquets ne sont pas réassemblées.
Aucune découverte IPv6, hors sous-réseau, en arrière-plan ou cloud.

TLS 8009 utilise le certificat épinglé ; une coupure efface les métadonnées,
le volume et la disponibilité observés. Reconnexion après 5 s, aucun rejeu.
Le volume est celui du **récepteur**, pas le volume du flux média : SET_VOLUME
sur `urn:x-cast:com.google.cast.receiver`, destination `receiver-0`, sans
mediaSessionId. La confirmation exige la source, le namespace et requestId
exacts ainsi que la valeur observée attendue. Une réponse négative diffère
d’une réponse perdue, qui reste incertaine.

FIFO bornée à quatre requêtes (active comprise), un envoi à la fois. La requête
originale porte l’échéance absolue et l’abandon jusqu’au premier octet. Une
ancienne génération de validation, de connexion ou de session média ne peut
pas devenir une nouvelle commande après reconnexion. Après émission, le suivi
et le journal conservent le résultat même si l’appelant quitte l’écran.

## Sources et preuves

- [RFC 6762, multicast DNS](https://www.rfc-editor.org/rfc/rfc6762).
- [Contrôleur de récepteur PyChromecast](https://github.com/home-assistant-libs/pychromecast/blob/master/pychromecast/controllers/receiver.py), pour le namespace et le volume du récepteur ; aucune dépendance Python ni code Python embarqué.
- [Messages média Google Cast](https://developers.google.com/cast/docs/media/messages), pour la lecture et sa session distincte du volume récepteur.

Les contrats emploient des paquets synthétiques, des flux duplex et le vrai hub
avec un coffre temporaire. Les tests HTTP n’atteignent aucun appareil réel.
La fixture JSON est synthétique. Aucune qualification matérielle n’est acquise.
Un essai réel exige une autorisation H1 distincte, une cible neutre, un humain
présent et la remise en état initial.
