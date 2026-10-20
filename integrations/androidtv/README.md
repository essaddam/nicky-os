# Android TV et Google TV

Pilote générique distinct de Philips JointSpace. Android TV Remote v2 (TLS
6466) fournit les touches, le texte, l’alimentation, le volume observé et
l’application au premier plan. Polo v2 (TLS 6467) effectue l’appairage humain.
Google Cast (TLS 8009) fournit les métadonnées et play/pause/stop lorsqu’une
session média existe. Aucun serveur ADB ni compte fournisseur nécessaire.

L’identité de l’appareil est `<instance>:tv`, indépendante de son adresse et
de son nom affiché. Le constructeur valide `host` sans connexion. Une instance
désactivée n’a ni tâche, ni passerelle HTTP. Les valeurs inconnues sont `null` ;
la perte de Remote ne signifie jamais « éteinte ». Cast peut perdre ses
métadonnées tout en laissant la télécommande disponible.

## Identité et ordres

Certificat RSA 2048 et clé générés dans Moli, hors de la boucle asynchrone.
Le code hexadécimal de six caractères affiché par la télé lie la preuve à ses
clés publiques et à l’identité cliente. Une tentative par instance, 120 s au
total, échanges de 4 s et trames de 64 Kio. L’identité et l’empreinte Remote
sont enregistrées ensemble dans une entrée chiffrée `identity`. L’empreinte
Cast est indépendante (`cast_pin`). Aucun code, certificat, clé ou preuve
n’apparaît dans la réponse HTTP ou l’audit. Les identifiants Philips existants
`atv_cert` et `atv_key` ne sont pas importés ni modifiés.

Une écriture acceptée au coffre se termine même si la session redémarre ;
la session suivante attend cette écriture avant de charger l’identité ou les
empreintes, afin de ne pas réutiliser une ancienne valeur en mémoire.

Une file FIFO de quatre requêtes, ordre en cours compris, sérialise Remote
et Cast. Le délai absolu du hub et l’abandon suivent l’ordre jusqu’à l’écriture.
Une perte après émission produit un résultat incertain, sans rejeu. Un volume
absolu se règle par touches successives séparées par les observations reçues ;
un volume ou une sourdine inconnus ne déclenchent pas de toggle supposé.

## Services et niveau de preuve

Le point `play_media` reçoit du texte JSON `{service,kind,target}`. Le résolveur
partagé `moli-media-target` accepte uniquement une cible explicite et valide ses
hôtes et son format ; il ne cherche aucun catalogue et ne contacte aucun site.
Les fixtures sont fictives et ne désignent pas des comptes ou favoris réels.
La cible `kind: "app"` ouvre le paquet Android du fournisseur. Les variantes
du même fournisseur ne sont essayées qu’après un refus explicite ; aucune
ouverture de navigateur n’est utilisée pour cette cible.

| Service | Cible précise acceptée | Limite |
|---|---|---|
| Netflix | ID numérique ou lien titre/épisode | Le premier plan Netflix ne prouve pas quel épisode joue. |
| Crunchyroll | ID explicite ou lien série/épisode | Un ID de série ne devient jamais un ID d’épisode. |
| Free TV | ID interne de chaîne ; lien web explicite de programme replay | Numéro de bouquet et ID interne sont distincts ; portail replay refusé. |
| Prime Video | ID explicite ou lien de détail `gti` | Lien à qualifier sur l’application Android installée. |
| Disney+ | UUID de titre ou lien de lecture d’épisode | Un titre n’invente pas un épisode. |
| Molotov | Ouverture de l’application | Chaîne et contenu précis non qualifiés : refus explicite. |
| Apple TV | Lien web complet de film, série ou épisode compatible | Aucun replay générique fabriqué. |

Une cible précise refusée ne retombe jamais sur l’accueil du service.
« Lien transmis », application observée et programme en lecture sont trois
preuves différentes. Les formats web non qualifiés restent signalés comme
tels ; un HTTP 200 du fournisseur ne qualifie pas le lien Android TV.
Recherche, catalogues et guide TV appartiennent au domaine transverse suivi
séparément et pourront fournir ces mêmes cibles explicites à d’autres lecteurs.

## Qualification

Les tests utilisent des transports simulés, le vrai hub, TLS sur boucle locale
et des identités générées en mémoire. Aucune qualification sur téléviseur réel
ni modification d’une maison n’est revendiquée. Après autorisation H1, vérifier
l’appairage, les empreintes 6467/6466, le volume, le Cast et chaque application
sur une cible neutre, puis restaurer son état. Qualifier séparément chaîne Free
TV, portail replay et programme replay sur la version installée.
