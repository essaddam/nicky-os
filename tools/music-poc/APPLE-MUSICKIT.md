# Préparer MusicKit pour le fournisseur Rust

Le fournisseur Rust appelle directement l'API Apple Music. Ce document prépare
sa configuration ; aucun identifiant, clé privée ou jeton réel n'est conservé
dans le dépôt.

## Configuration Apple à créer

Dans [Certificates, Identifiers & Profiles](https://developer.apple.com/account/resources/identifiers/list),
un propriétaire de compte ou un administrateur peut créer un **Media ID** et
lui associer une clé **Media Services** avec MusicKit activé. La description
est présentée à l'utilisateur lors de la demande d'accès à Apple Music.

Configuration proposée, à vérifier avant son enregistrement :

| Champ | Proposition |
| --- | --- |
| Description | Moli Music POC |
| Media ID | media.org.moli.music.poc, si cet identifiant est disponible dans l'équipe ; le portail ajoute le préfixe media. |
| Service | MusicKit uniquement pour ce prototype |
| Nom de clé | Moli Music POC |

La clé privée doit être prise en charge par le propriétaire puis rangée dans
le coffre. Elle ne doit jamais être copiée dans le chat, le dépôt, une commande
ou un journal. Ne pas réutiliser la clé maître Moli, une clé APNs ou une clé
App Store Connect : les droits et la destination sont différents.

La procédure primaire est celle d'[Apple : Media ID et clé privée](https://developer.apple.com/help/account/capabilities/create-a-media-identifier-and-private-key/).
Le Media ID `media.org.moli.music.poc` a été enregistré avec l'accord du
propriétaire, avec MusicKit uniquement. La clé `Moli Music POC`, associée à ce
Media ID, a également été créée avec son accord. Son téléchargement n'a pas
été déclenché ; aucun secret Apple réel n'est disponible dans le POC.

Apple permet de différer le téléchargement. Une fois la clé téléchargée, la
copie serveur est supprimée et elle ne peut plus être téléchargée. Préparer
la destination dans le coffre et le transfert avant cette récupération.

## Signature locale en Rust

Le programme prend les entrées suivantes dans son environnement, injectées
depuis le coffre :

- `APPLE_MUSIC_PRIVATE_KEY_P8` : clé PKCS#8/PEM P-256 ;
- `APPLE_MUSIC_TEAM_ID` : identifiant de l'équipe ;
- `APPLE_MUSIC_KEY_ID` : identifiant de la clé MusicKit.

Il signe un jeton développeur **ES256** valable quinze minutes, utilisé
directement dans l'en-tête de la requête Apple. La clé et le jeton ne sont
jamais affichés. La signature JOSE contient les 64 octets `r || s` et les
segments sont encodés en Base64 URL sans padding.

Un `APPLE_MUSIC_DEVELOPER_TOKEN` déjà injecté peut aussi être utilisé ; il a
priorité sur la signature locale. Les identifiants manquants, partiels ou
malformés sont refusés. La signature est vérifiée dans les tests avec une
clé fictive créée uniquement en mémoire.

La dépendance de signature est [p256 0.13.2](https://docs.rs/crate/p256/0.13.2/source/Cargo.toml),
sous Apache-2.0 ou MIT. Les champs des jetons suivent la
[documentation Apple](https://developer.apple.com/documentation/applemusicapi/generating-developer-tokens).

## Consentement du compte Apple Music

Le jeton développeur donne accès au catalogue. Les requêtes personnelles
exigent ensuite un **Music User Token**, délivré après le consentement du
propriétaire via MusicKit. Un jeton « Sign in with Apple » ne le remplace pas.

Le fournisseur accepte ce jeton sous `APPLE_MUSIC_USER_TOKEN`, également injecté
depuis le coffre. La commande `native-account apple_music` valide l'accès
personnel et ne retourne que le storefront, pas les coordonnées du compte.

Le parcours interactif MusicKit Web et l'import du Music User Token ne sont
pas encore raccordés dans cette tranche. Leur mise en place suit la création
de la configuration développeur.

## Limite audio à préserver

La signature du jeton, une recherche réussie et l'autorisation du compte ne
constituent pas une preuve d'accès à un flux complet décodable par le moteur
Rust. Le résultat reste `full_stream: not_implemented`. Résolution du flux,
protection éventuelle et droits de lecture nécessitent leur propre intégration
et leur qualification ; les aperçus ne doivent pas être présentés comme des
morceaux complets.
