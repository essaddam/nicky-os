# Twinkly local : marche, luminosité et RGB statique

Pilote natif `moli-twinkly`, API HTTP locale **v1**, un appareil par instance.
Il ne découvre aucun appareil et ne contacte aucun compte cloud. L'adresse IPv4
privée et l'identité MAC sont choisies et validées par un humain. L'adresse et le
nom n'entrent pas dans l'identité du cœur : `<instance>:<MAC canonique>`.
Un contrôle de MAC sur HTTP n'est pas une preuve cryptographique contre l'usurpation.
L'API locale transporte le jeton en clair : ce contrat suppose le réseau local
validé par l'humain, sans revendiquer une authentification cryptographique du serveur.

## Contrat

| Point | Contrat du cœur | Observation |
|---|---|---|
| `on` | booléen, `OnOff`, `states` | mode relu, tout mode autre que `off` est allumé |
| `brightness` | entier de 0 à 100 %, `Brightness`, `states` | dimming relu ; `disabled` signifie 100 % |
| `color` | texte `#rrggbb`, `Color`, `states` | RGB relu ; disponible seulement pour profil RGB et firmware ≥ 2.7.1 avec route directe reconnue |

Une luminosité demandée de zéro éteint : certaines variantes désactivent le dimming
si on leur envoie zéro. Une luminosité positive règle le niveau sans choisir une
scène. Allumer utilise un mode compatible réellement observé (`color`, `movie`,
`playlist`) ou explicitement choisi par l'humain dans `resume_mode` ; jamais un
mode `movie` arbitraire par défaut. Les modes temps réel, musique, démo et effet ne
sont pas des modes de reprise. Le mode mémorisé n'est pas persisté au redémarrage.
Le point couleur représente la couleur statique : il vaut `null` quand un autre
mode est actif, même si la route conserve un ancien RGB.

Aucun sélecteur d'effets, choix de film/playlist, téléversement, synchronisation,
musique ou temps réel. Les variantes RGBW et API v2 sont hors du contrat initial.
Pas de téléversement de film monochrome quand la couleur directe manque.

L'état demandé n'est jamais publié comme une observation. Après un envoi, Moli
relit les valeurs ; une contradiction publie la valeur relue. Un acquittement
signifie HTTP réussi **et** `code:1000`, pas une qualification physique. Sans
réponse, l'effet reste incertain et la commande n'est jamais répétée. Lectures utiles toutes les 15 s ; après un échec, pause de 60 s. Chaque échange
a un plafond de 4 s ; une lecture comporte au plus six échanges, deux de plus
si la session est renouvelée. Le délai de commande reste les 5 s absolues du hub :
une attente longue peut faire refuser un ordre, jamais l'envoyer en retard. Les erreurs
de lecture rendent l'appareil indisponible et ne rafraîchissent aucun point.

## Bornes et session

Le JSON est limité à 8 Kio dès le transport, y compris sans Content-Length ; textes
64 caractères, profil 16, version 24, MAC canonique de 12 chiffres hexadécimaux.
Le challenge contient 32 octets aléatoires générés à l'exécution. Jeton de 12 à
128 caractères base64, réponse de challenge de 40 caractères hexadécimaux et durée
entière de 1 à 86400 secondes. La durée de la réponse est utilisée, jamais 14400
supposé. Le jeton est transitoire en RAM ; aucune valeur de session au coffre,
dans les fichiers ou dans les journaux. Les erreurs ne recopient aucun corps.
Les destinations et chemins sont fixes, sans redirection ni URL issue du réseau.

Une seule opération réseau en cours ; huit ordres au maximum (actif compris),
en plus du canal borné du hub ; FIFO et échéance
absolue du hub et abandon contrôlés avant chaque émission, authentification
comprise. Une réponse déjà en vol reste acquise de façon bornée après le départ
de l'appelant ; aucune deuxième étape non envoyée ne part après son abandon.
Un renouvellement de session précède une opération non envoyée ; jamais de rejeu
après une erreur d'envoi. Un 401/403 invalide la session qui avait déjà fourni une lecture complète : la
lecture suivante refait un login/verify, sans répéter l'écriture refusée. Un jeton
neuf refusé avant sa première lecture complète met l'instance en attente humaine.
Les récupérations après refus sont limitées à trois dans une heure glissante ;
un nouveau refus au-delà de ce budget demande une intervention plutôt que de
mener un duel avec HA/l'application. Les expirations normales suivent la durée
retournée, distinctement de ce budget de récupération.

## Provenance, licences et choix

Références publiques figées, étudiées sans embarquer leur code :

- [XLED docs, révision 6e88f64](https://github.com/xled/xled-docs/blob/6e88f64e382245a2aef3b9468f81c6707b2035b9/docs/rest_api.rst),
  licence MIT : login/verify, modes, dimming, couleur directe depuis 2.7.1.
- [Home Assistant, 2026.10.2](https://github.com/home-assistant/core/blob/8d7fd8b68677c2585be36ec86043308f403875b8/homeassistant/components/twinkly/light.py),
  Apache-2.0 : échelles et différence entre couleur directe et ancien repli film.
- [ttls 1.11.1](https://pypi.org/project/ttls/1.11.1/), BSD-2-Clause,
  source SHA-256 `4b4142f595edf8bcb2c6ad5a7179eaa674c366b027aa89fc189ecc12dc1a2956` :
  challenge aléatoire, durée retournée et séquence d'authentification.

Il s'agit d'un protocole communautaire, sans garantie fabricant. Moli ne reprend
ni le réessai automatique après 401 de ttls, ni son mode d'allumage par défaut,
ni le repli film de l'ancien chemin HA. Les limites de taille, de durée, la mise
en attente sur refus et le périmètre RGB v1 sont des choix conservateurs Moli,
pas des propriétés prouvées de tout firmware. Pas de preuve privée incorporée.

## Repris, supposé, restant

Repris : les routes et formats documentés, les conversions explicites.
Les fichiers `fixtures/` sont **synthétiques**, pas des captures ; identité, nom et
modèle sont fictifs. Les tests génèrent challenge et jeton en mémoire, aucun
secret même de test n'est commité. Ils font tourner de vrais échanges HTTP avec
un serveur local simulé et les vrais points du cœur.

Restant avant usage réel : jalon humain H1, modèle et firmware réellement relevés,
propriétaire d'exécution unique, couleur directe prouvée, état observé après chaque
commande et restauration de l'état initial sur une cible neutre. Aucun matériel
n'est qualifié par ces fixtures. Voir [onboarding.md](onboarding.md).

À l'arrêt, un nettoyage propre à Twinkly met ses seuls appareils hors ligne,
y compris si sa future est détruite. Le superviseur des autres pilotes est inchangé.
