# Cibles de médias explicites

Cette crate valide une cible choisie et produit son lien. Elle ne dépend
d'aucun lecteur, ne recherche pas dans un catalogue et ne fait aucun accès
réseau. La recherche transverse est suivie séparément dans le backlog.

Entrée : `ContentRequest { service, kind, target }`. La méthode `parse`
borne le JSON à 4096 octets et refuse les champs inconnus ou dupliqués.
`resolve` borne la cible à 2048 octets et produit un `LaunchTarget` : URI,
paquets Android attendus à titre d'indication et niveau de documentation.
Les autres transports peuvent ignorer les indications de paquets Android.
Pour une cible `app`, le pilote Android TV utilise ces paquets pour ouvrir
l’application native ; les cibles précises conservent leur lien validé.

`Documented` décrit le **format du lien**, jamais un test matériel ou une
preuve que la lecture a commencé. `Unqualified` signale un lien dont
l'interprétation par l'application TV reste à vérifier. Un simple retour
dans l'application déjà ouverte ne prouve pas le changement de contenu.

| Service | Cibles reconnues |
|---|---|
| Netflix | Application ; titre ou identifiant d'épisode explicite, avec `source=30` reconstruit |
| Crunchyroll | Application ; série `/series/<id>` ou épisode `/watch/<id>` ; préfixe de langue accepté |
| Free TV/OQEE | Application ; chaîne par identifiant interne ; lien web explicite de programme replay `/replay/<id>/detail`, non qualifié pour Android TV |
| Prime Video | Application ; titre/épisode explicite par identifiant ou lien `detail`, normalisé en `gti` |
| Disney+ | Application ; UUID de titre vers `browse/entity-…` ; épisode seulement depuis un lien explicite `play/…` |
| Apple TV | Application ; lien de film, série ou épisode conservé après validation, sans paramètres de suivi |
| Molotov | Application officielle ; cible précise refusée tant qu'aucun format fiable n'est qualifié |

Un identifiant Free TV de portail replay n'est jamais transformé en
identifiant de programme. Un numéro de chaîne affiché n'est pas résolu
automatiquement en identifiant interne. Aucun nom libre n'est recherché
ou remplacé par un identifiant supposé. Une cible refusée ne retombe
jamais silencieusement sur l'accueil de l'application.

La grammaire reconnaît des formes précises : autorités HTTPS exactes,
chemins bornés, sans userinfo, port explicite, antislash, fragment ou
composante `.`/`..`. Les identifiants ne sont pas décodés depuis des
échappements ambigus. Les slugs cosmétiques peuvent contenir des lettres
Unicode encodées, après validation. Les paramètres de suivi reconnus
sont retirés ; les paramètres sémantiques inconnus sont refusés.

## Provenance et qualification

- Netflix : [documentation Android TV Remote de Home Assistant](https://www.home-assistant.io/integrations/androidtv_remote/), notamment le paramètre `source=30`.
- Free TV : [associations Android publiées par le fournisseur](https://oq.ee/.well-known/assetlinks.json) et [bundle public inspecté le 19 octobre 2026](https://tv.free.fr/assets/index-CCrbGUxu.js). Il distingue chaîne, portail et programme replay ; ses routes web ne prouvent pas leur réception par Android TV.
- Prime Video et Disney+ : formes de liens relevées par le [développeur de Multistream](https://github.com/renaudallard/multistream#deep-links), sans contrat fournisseur ni qualification sur la télé ciblée.
- Crunchyroll : liens publics de série et d'épisode ; l'inspection des associations Android a renvoyé HTTP 403. Compatibilité de chaque cible TV à qualifier.
- Apple TV : [fiche officielle de l'application Android](https://play.google.com/store/apps/details?id=com.apple.atve.androidtv.appletv) et liens publics `tv.apple.com`. La présence de l'application ne garantit pas l'ouverture d'un épisode par lien.
- Molotov : [application officielle](https://play.google.com/store/apps/details?id=tv.molotov.app). Aucun format universel de lien vers un contenu précis établi dans ce lot.

Les treize contrats initiaux ont été observés en échec fonctionnel avant
implémentation (1 réussi, 12 échoués). Ils ont ensuite été observés au vert (treize réussis). Les validations
d'intégration du pilote sont suivies dans son ticket ; aucun test sur
appareil réel.
