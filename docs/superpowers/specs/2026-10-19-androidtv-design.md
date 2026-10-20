# Pilote Android TV générique — proposition de conception

Statut : conception initiale acceptée ; extension contenus demandée le 19 octobre 2026.
Le plan d’implémentation et les limites par service restent à relire.
Aucune implémentation ni qualification acquise.

## Objectif et périmètre

Ajouter le pilote natif `androidtv` aux téléviseurs Android TV / Google TV
équipés du service Android TV Remote v2. Il fonctionne sans Home Assistant,
sans ADB et sans API constructeur. Il reprend les clés de points utilisées
par la télécommande Philips et complète l'état par Google Cast.

Contraintes du ticket : conserver l'API consommée par `moli-philips`, appliquer
la note `pilotes-ordres`, fournir le paquet `integrations/androidtv/`, traduire
les textes en français et en anglais et obtenir un garde-fou complet vert.
La qualification physique est distincte et attend H1.

## Choix d'architecture

Trois approches ont été considérées : ajouter ce fonctionnement au pilote
Philips ; mêler le contrat Driver aux protocoles extraits ; ajouter une crate
`moli-androidtv-driver` qui consomme `moli-androidtv`.

La troisième est retenue comme proposition : la bibliothèque reste un
transport réutilisable, le pilote possède les états, le coffre et les ordres.
L'API historique de `remote` et `cast` reste disponible pour Philips ; les
échanges avec échéance et résultat utilisent une API additionnelle.

Une passerelle typée d'appairage est partagée entre le pilote et l'API, sur le
modèle des passerelles téléphones/ordinateurs déjà présentes dans `moli-os`.
Elle ne connaît que les instances activées ; aucune adresse libre ne vient du
navigateur. Pas de tâche ni de registre alloué pour une instance désactivée.

## Configuration et identité

Une instance correspond à une télé, avec `kind = "androidtv"` et l'option
obligatoire `host`. Les options inconnues sont refusées. `host` est un hôte,
jamais une URL ; les ports des protocoles sont fixes : 6467 pour l'appairage,
6466 pour Remote et 8009 pour Cast. Aucun balayage réseau.

L'identité est `<instance>:tv`, indépendante du nom, de l'adresse DHCP et du
certificat renouvelé. Changer de télé dans cette instance exige un nouvel
appairage explicite ; une empreinte différente est refusée.

## Appairage humain

Le pilote sans identité utilisable attend une action humaine. Le dashboard,
dans Système, affiche les instances et un bouton pour appairer. Le démarrage
et la saisie exigent une session humaine PIN ; un invité et une démo refusent
ces écritures. Aucun essai sur une télé réelle pendant le développement.

Le pilote produit une identité RSA 2048 bits et un certificat autosigné en
Rust, hors du chemin asynchrone principal. Aucun sous-processus OpenSSL ni
fichier PEM temporaire. La paire cert/clé est gardée dans une seule entrée
du coffre pour éviter une identité partiellement écrite ; le chemin Philips
`atv_cert` / `atv_key` ne change pas.

L'échange Polo v2 négocie le rôle de saisie et six caractères hexadécimaux.
La preuve SHA-256 associe les clés publiques RSA des deux extrémités aux
deux derniers octets du code ; le premier octet vérifie le code avant envoi.
Le code ne passe ni par un point du hub, ni par l'historique, ni par le
journal, ni par une notice ou un message d'erreur.

Une seule tentative par instance, une échéance humaine absolue de 120 s,
des échanges réseau bornés à 4 s et des trames bornées à 64 Kio. Une génération
de session empêche un ancien code ou résultat de valider une nouvelle
tentative. La déconnexion du navigateur ne relance rien ; annulation, arrêt
et expiration ferment la socket. Une tentative échouée exige un nouveau
geste humain, sans remplacer une identité antérieure encore valide.

L'empreinte de l'interlocuteur d'appairage est conservée avant la validation
de l'identité ; Remote doit retrouver cet interlocuteur. Cast possède son
épinglage distinct. Aucun changement d'empreinte n'est accepté silencieusement.
La qualification H1 devra vérifier cette correspondance sur le matériel.

## Contrat HTTP proposé

Préfixe : `/api/integrations/androidtv/{instance}/pairing`.

| Méthode | Fonction | Résultat |
|---|---|---|
| GET | État public de cette tentative, sans secret | 200 : état, génération, expiration, code d'erreur traduit |
| POST | Démarrer une tentative après geste humain | 202 : génération ; 409 si déjà en cours ou déjà appairée |
| PUT | Soumettre `{generation, code}` | 200 après accusé de la télé ; 422 code invalide ; 409 tentative périmée |
| DELETE | Annuler la tentative identifiée par génération | 204 ; aucun effacement de l'identité durable |

Toutes les routes héritent des gardes Host, origine et foyer. Écritures :
PIN humain obligatoire, instance activée connue, corps JSON plafonné à 1 Kio,
champs inconnus refusés. GET ne déclenche aucun réseau. Instance inconnue :
404 ; coffre indisponible : 503 ; réponse réseau invalide : 502 ; délai : 504.
Un audit d'appairage conserve acteur, instance, opération et résultat sans
code, clé, certificat ou preuve. Le contrat sera décrit dans `api.md` et
un fragment OpenAPI versionné avec les schémas et erreurs.

## Points et comportement des commandes

Points : `power`, `volume`, `mute`, `key`, `app`, `text`, `media_control`,
`media_app`, `media_state`, `media_title`, `media_subtitle`, `media_image`,
`media_duration`, `media_position`. Les accès suivent Philips, sans Ambilight.
Le maximum de volume vient du message Remote ; son absence interdit un
réglage absolu. La perte de connexion donne un état inconnu, pas une télé
présumée éteinte ; la disponibilité du pilotage dépend de Remote.

Alimentation : touches Android distinctes réveil/veille, jamais un toggle
aveugle. Pas de réveil garanti depuis une veille profonde avec port fermé.
Sourdine : aucun toggle si l'état est inconnu ; aucun envoi si déjà conforme.
Volume : différence avec le dernier état confirmé, touches successives
bornées par le maximum annoncé et l'échéance de l'ordre. Attendre une
observation entre deux corrections ; pas de rafale aveugle. L'échec après
un réglage partiel est explicite, avec l'état réellement observé.

Les touches conservent les noms Philips, traduits en codes Android. Lancement
d'appli : réutiliser `app_links`, sans nouveau catalogue divergent. Le lien
suivant n'est essayé qu'après refus explicite, dans la même échéance ; une
liaison perdue ne provoque aucun nouvel essai. Cast refuse une commande sans
session média et corrèle ses réponses, sans annoncer un succès sur mise en file.

Tous les ordres physiques passent par le hub. Une file FIFO de quatre ordres
au maximum, commande en vol comprise, commune à Remote et Cast ; saturation
refusée immédiatement. Un seul ordre physique en vol, sans attente réseau
dans la boucle qui gère les échéances et l'arrêt.

L'enveloppe de transport porte échéance absolue, abandon et numéro de
génération jusqu'au dernier contrôle avant écriture sur la socket. Revoir
ces conditions après chaque attente. Une écriture commencée n'est jamais
rejouée ; le résultat reste suivi jusqu'à sa borne même si l'appelant part.
Si la réponse se perd, le résultat dit « envoyé, non confirmé ». À la panne,
vider la file de cette session avant toute reconnexion.

## Films, séries, épisodes et chaînes : extension demandée

Services demandés : Netflix, Crunchyroll, Free TV/OQEE, Prime Video, Disney+,
Molotov et l'application Apple TV installée sur Android TV. Le pilotage d'un
boîtier Apple TV est une autre intégration, hors de ce pilote.

Ajouter un point `play_media` en écriture seule. Il accepte un objet JSON
encodé en texte : `{"service":"netflix","kind":"title","target":"12345678"}`.
Services stables : `netflix`, `crunchyroll`, `freetv`, `primevideo`,
`disneyplus`, `molotov`, `appletv`. Types : `app`, `title`, `episode`,
`channel`, `replay`. `target` est un identifiant du service ou un lien de partage
explicite ; aucun nom libre n'est transformé en identifiant par supposition.

| Service | Cible précise et règle de construction |
|---|---|
| Netflix | Identifiant numérique → `https://www.netflix.com/title/<id>?source=30`. L'identifiant d'une série et celui d'un épisode sont distincts. Un lien `/watch/<id>` est normalisé vers ce titre précis. |
| Crunchyroll | Série → lien `/series/<id>` ; épisode → `/watch/<id>`. Accepter les liens de partage avec préfixe de langue, valider le domaine exact et conserver la cible. La réception par l'application TV reste à qualifier. |
| Free TV/OQEE | Identifiant interne de chaîne → `https://oq.ee/channel/<id>/play`. Un numéro affiché dans le bouquet n'est jamais pris pour cet identifiant. Les liens web de chaîne reconnus sont normalisés ; pas de table de numérotation figée. |
| Prime Video | Identifiant de titre/épisode → `https://app.primevideo.com/detail?gti=<id>`. Accepter les liens `primevideo.com/detail/...` sans suivre de raccourcisseur dans le moteur. |
| Disney+ | Identifiant UUID → `https://www.disneyplus.com/browse/entity-<id>` ; un lien explicite `/play/<id>` vise un contenu de lecture, sans assimiler l'UUID d'une série à celui de son épisode. |
| Apple TV | Lien de partage `https://tv.apple.com/...` de film, série ou épisode avec identifiant `umc.cmc...`, conservé après validation. Aucun schéma réservé à iOS n'est substitué à un lien Android TV. |
| Molotov | Ouverture de l'application officielle `tv.molotov.app`. Aucun lien de titre/chaîne universel fonctionnel n'a été établi pour cette application : une cible précise non qualifiée est refusée explicitement, jamais convertie en simple ouverture de l'appli avec faux succès. Un lien propre à une version pourra être qualifié après H1. |

`play_media` utilise la même garde, la même file FIFO et la même échéance
que les autres ordres. Un objectif précis ne retombe jamais sur la page
d'accueil après refus. Pas de séquence aveugle de touches pour choisir
profil, abonnement, achat ou épisode ; pas de lecture d'identifiants de
services ni de requête de catalogue externe dans le pilote.

Validation avant tout envoi : JSON plafonné à 4096 octets, champs inconnus
refusés, liens à 2048 octets au plus, pas de caractères de contrôle,
userinfo, port personnalisé, hôte ressemblant ou schéma `intent:`, `file:`,
`javascript:`. Les paramètres de suivi inutiles sont retirés ; les paramètres
sémantiques nécessaires sont reconstruits, sans concaténation de texte libre.

La commande réussie signifie « lien transmis à la télé », sans prétendre
prouver quel épisode joue. Le point `app` constate l'application au premier
plan ; Cast renseigne la lecture quand elle est observable. Un simple retour
dans la même application ne confirme pas l'identifiant du contenu demandé.
L'interface affiche ces niveaux de résultat avec leurs limites.

Le dashboard propose ces sept services, la saisie d'un lien/identifiant et
le type de cible. Des favoris nommés permettent de réutiliser une cible
connue ; ils restent des données locales de la maison, jamais un catalogue
personnel embarqué dans le dépôt. Les demandes libres par nom utilisent le
favori explicite ; sans correspondance, Moli demande la cible précise.

Constats de recherche : le bundle public Free TV du 19 octobre reconnaît
explicitement `https://oq.ee/channel/<id>/play`, et les associations Android
publiées par `oq.ee` désignent les paquets TV. Cela prouve la forme du lien,
pas son effet sur un téléviseur précis. L'association Crunchyroll n'a pas
pu être inspectée (HTTP 403) : l'ouverture d'une URL publique n'est pas
présentée comme une qualification Android TV.

### Replay Free TV et recherche transverse

La correction du propriétaire porte sur le replay **Free TV/OQEE**, pas sur
une fonction replay d'Apple TV. Il faut viser le programme ou l'épisode
souhaité, et non le simple portail replay de la chaîne.

Le bundle Free TV inspecté distingue le portail `https://oq.ee/replay/<id>`
d'un programme : la version web utilise `/replay/<program-id>/detail`, la
version smart TV `/play/replay/<program-id>`. Ce ne sont pas des preuves que
l'application Android accepte ces mêmes chemins. Un lien de portail n'est
jamais présenté comme le lancement du programme. Accepter un lien de
programme explicite seulement avec le niveau de qualification annoncé ;
aucun chemin Android de lecture n'est inventé à partir d'un identifiant.

Le ticket transverse **0983449d-040b-4248-ba9e-aa600db39e70**, créé à la demande
du propriétaire, possède la recherche par nom, les catalogues fournisseurs,
le guide TV, les identifiants de programmes/replays et leur résolution.
Il est lié au ticket Android TV par `relates_to`, sans le bloquer.
Le pilote reste un consommateur de cibles explicites : pas d'API privée de
catalogue, de scraping ou de recherche autonome dans sa boucle d'exécution.
La table des formats ci-dessus sert de contrat et de fixtures initiales ;
la résolution réutilisable doit vivre hors de la bibliothèque Android.

## Intégration et validation

Fichiers concernés : nouvelle crate pilote, ajouts compatibles dans
`moli-androidtv`, enregistrement et passerelles `moli-os`, routes `moli-api`,
carte d'appairage Système, catalogues fr/en, paquet d'intégration, docs.
Les listes partagées modifiées sont triées par ordre alphabétique.

Les fixtures sont synthétiques, indiquées comme telles. Tests sur transports
simulés et vrai hub : négociation et preuve, mauvais code, trame fragmentée
ou excessive, expiration et ancienne génération, persistance interrompue,
autorisation HTTP et absence de fuite, FIFO, saturation, abandon avant et
après émission, panne/reconnexion sans rejeu, réglage partiel du volume,
arrêt et libération des tâches. Les tests Philips existants restent requis.

Progression : contrats rouges ciblés, implémentation, validations ciblées,
puis `sh scripts/check.sh` dans l'image de référence et build web/i18n.
Les contrôles lourds attendent un créneau explicitement alloué selon les
règles de la machine partagée. Aucune preuve de réussite n'est déduite de
tests simplement préparés. Le budget dormant est vérifié sans équipement ;
les mesures réelles, l'appairage physique et le déploiement attendent H1.

## Sources de protocole

Confrontation au code amont, sans dépendance Python embarquée :

- [androidtvremote2 : appairage](https://github.com/tronikos/androidtvremote2/blob/main/src/androidtvremote2/pairing.py).
- [Polo : schéma](https://github.com/tronikos/androidtvremote2/blob/main/src/androidtvremote2/polo.proto).
- [Remote : schéma](https://github.com/tronikos/androidtvremote2/blob/main/src/androidtvremote2/remotemessage.proto).
- [Génération d'identité amont](https://github.com/tronikos/androidtvremote2/blob/main/src/androidtvremote2/certificate_generator.py).

Avant toute reprise de code, figer la révision et conserver les obligations
Apache-2.0 de provenance. Ce document décrit un contrat à implémenter et
tester, pas une qualification matérielle.

### Sources des liens de contenu

- [Android TV Remote : liens et limites](https://www.home-assistant.io/integrations/androidtv_remote/).
- [Free TV : associations Android publiées](https://oq.ee/.well-known/assetlinks.json).
- [Free TV : route publique de chaîne](https://oq.ee/channel/273/play), inspectée sans connexion à un compte.
- [Free TV : bundle public inspecté le 19 octobre](https://tv.free.fr/assets/index-CCrbGUxu.js).
- [Multistream : formats Prime Video et Disney+ et limites Molotov](https://github.com/renaudallard/multistream#deep-links), témoignage du développeur, pas un contrat des fournisseurs.
- [Crunchyroll : applications prises en charge](https://help.crunchyroll.com/hc/en-us/articles/24181606177300-What-devices-are-supported-by-Crunchyroll).
