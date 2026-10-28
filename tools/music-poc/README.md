# POC musique : contrôle MQTT et transport média Rust

Prototype autonome, hors workspace principal. Il ne démarre aucune lecture sur
une enceinte et ne modifie pas une instance Music Assistant existante.

## Ce qui fonctionne et ce qui reste à raccorder

| Élément | État du prototype |
| --- | --- |
| Contrôle MQTT sur TCP | Broker local réutilisé du POC MQTT, authentification, petits messages JSON, commande non retained |
| Transport audio/vidéo HTTP | Tickets privés révocables, GET/HEAD, plages, fichiers ouverts lus par blocs |
| Décodage Rust | FLAC synthétique mono 48 kHz, comparaison de chaque échantillon, sortie nulle |
| Isolation | Même parcours en un processus ou avec récepteur dans un second processus |
| Music Assistant | Client d'inventaire conservé pour comparaison ; absent du parcours natif choisi |
| Apple Music natif | REST direct : catalogue et vérification d'autorisation/storefront ; jetons requis |
| YouTube Music natif | Compte et recherche directe qualifiés temporairement avec une requête authentifiée complète ; API non officielle |
| Flux complets Apple/YouTube | Résolution/déchiffrement non implémentés ; distincts du catalogue et de l'autorisation |
| Google Cast | Pilote de contrôle existant dans le dépôt ; lancement d'un flux à raccorder |
| AirPlay | Émetteur audio non implémenté dans ce POC |
| Alexa | Non raccordé ; le parcours MA utilise une Skill et un service HTTPS séparé |
| Garde Moli | Pas de raccord au Hub dans ce banc ; seule l'opération `decode-to-null` est autorisée |

Un succès du banc ne prouve donc ni l'accès aux morceaux Apple/YouTube ni une
lecture sur Cast, AirPlay ou Alexa. Le choix retenu est celui de fournisseurs
natifs Rust ; le client MA conservé pour comparaison ne sert pas de passerelle
aux nouveaux clients directs.

## Parcours et frontière commune

```text
                   petits messages / états / références
 source ───────────────── MQTT/TCP ─────────────────► récepteur
   │                                                     │
   └──── HTTP authentifié, octets FLAC ou autre média ──────┘
                                                         │
                                               décodage Rust → null
```

Le processus récepteur s'abonne avant l'admission de la commande. Les états
froids décrivent le format ; la commande porte une identité de session, une
génération et un identifiant média. Le secret HTTP passe par un tube privé
parent/enfant, jamais dans les topics retained, les arguments ou le rapport.
Les identités et le format de commande restent identiques dans les deux modes.
Le processus source héberge aussi le broker et le serveur média du banc.

Pour les modules audio **et vidéo**, le transport déplace les octets sans
interpréter le codec : un module peut produire un média, un autre le décoder,
le transformer ou le servir à un appareil. Une vraie entrée live, un transfert
entre machines avec TLS et le relais progressif d'une URL fournisseur ne sont
pas encore implémentés. Aucun fichier privé ni URL de fournisseur n'est accepté
par une requête HTTP cliente.

## Bornes du transport

- Quatre tickets et quatre réponses actives au maximum.
- Huit Mio de payloads retenus au total, corps en cours compris ; ce n'est pas
  un plafond RSS, ni celui des tampons TCP du système.
- Fichiers réguliers déjà ouverts par l'appelant, de 1 octet à 4 Gio ; lectures
  positionnelles par blocs de 64 Kio. Le fichier doit rester immuable pendant
  sa publication. Son contenu complet n'est pas gardé en mémoire.
- Durée d'un ticket : plus de zéro à 60 secondes. La purge fonctionne tant
  que le serveur tourne. Une session musicale longue nécessitera une politique
  explicite de renouvellement, encore absente.
- Révocation, expiration et arrêt empêchent les nouveaux accès et les prochains
  blocs ; les octets déjà transmis ne peuvent pas être rappelés.
- Arrêt gracieux attendu deux secondes. Un appel système de lecture déjà bloqué
  ne peut pas être interrompu ; la durée de vie du runtime reste déterminante.
- Le client de démonstration limite son entrée FLAC à 1 Mio et son décodage à
  480 000 échantillons. Le vecteur PCM entier sert uniquement à la comparaison
  de cette courte fixture ; ce n'est pas encore un moteur musical continu.
- Pas de mesure RSS attribuée à un module fusionné. Le rapport indique le
  nombre de processus du banc et les compteurs logiques du broker.

Le transport intermodules utilise un en-tête Bearer. Un lecteur Cast standard
qui récupère une URL, AirPlay qui reçoit du PCM et une Skill Alexa ne consomment
pas tous ce contrat directement : leurs adaptateurs de sortie devront convertir
le protocole et porter une autorisation limitée à la session et à la cible.

## Exécution

```sh
sh tools/music-poc/check.sh
```

Cette commande vérifie le format, Clippy, les contrats HTTP/MQTT et les deux
démonstrations. Elle utilise Rust 1.99.1, deux jobs et `target/music-poc`.
La commande réutilisable « Musique — POC et contrôles » existe aussi dans
AgentsRoom. Les ports sont éphémères et limités à la boucle locale ; le serveur
POC existant n'est pas utilisé ni redémarré.

### Fournisseurs natifs

```sh
target/music-poc/debug/moli-music-poc native-status
target/music-poc/debug/moli-music-poc native-search apple_music 'Recherche de test'
target/music-poc/debug/moli-music-poc native-search ytmusic 'Recherche de test'
target/music-poc/debug/moli-music-poc native-account apple_music
target/music-poc/debug/moli-music-poc native-account ytmusic
```

Apple utilise `APPLE_MUSIC_DEVELOPER_TOKEN` pour le catalogue et ajoute
`APPLE_MUSIC_USER_TOKEN` pour les requêtes personnelles.
`APPLE_MUSIC_STOREFRONT` vaut `fr` par défaut. Un identifiant de connexion
« Sign in with Apple » ne remplace pas le Music User Token.
Le [parcours de préparation MusicKit](APPLE-MUSICKIT.md) décrit aussi la
signature native ES256 depuis une clé fournie par le coffre, sans affichage
du jeton généré.

YouTube utilise `YTMUSIC_COOKIE`, issu d'une session que le propriétaire a
autorisée ; sa présence n'est pas une preuve d'abonnement ni d'accès audio.
Les valeurs doivent venir du coffre par environnement, jamais du dépôt, d'un
argument de commande ou d'une conversation. Le programme ne les enregistre
pas dans des fichiers et marque les en-têtes d'authentification sensibles.

Les deux clients parlent directement à leur origine HTTPS fixe ; aucune URL
arbitraire n'est acceptée. Leurs résultats sont bornés à cinq éléments, leurs
réponses JSON à 1 Mio et leurs requêtes à dix secondes. YouTube récupère la
version du client depuis son bootstrap au lieu de figer une version datée.
Les redirections, y compris vers le consentement, ne sont pas suivies.

Les tests utilisent des serveurs HTTP isolés et des jetons fictifs. L'essai
public YouTube du 27 octobre 2026 a reçu **HTTP 302 vers le consentement** avant
la recherche. Après import explicite d'une requête authentifiée complète, le
fournisseur Rust a confirmé le compte directement auprès de YouTube, puis
rendu cinq résultats de recherche. La session n'est conservée qu'en mémoire,
dans le processus temporaire ; aucun identifiant de compte ou secret n'est
inclus dans le reçu. Le résultat `full_stream: not_implemented` reste explicite
pour les deux services : cette preuve ne qualifie ni les morceaux complets ni
une lecture audible.

### Association YouTube temporaire

`pair-youtube` prépare un service local borné à dix minutes. L'import doit être
approuvé explicitement avant d'envoyer une session du navigateur au fournisseur
Rust. Aucun import n'est déclenché automatiquement par le lancement.

Le navigateur envoie alors sa session directement au processus local, sans
valeur passant par la conversation, les arguments, un fichier ou un topic
MQTT. L'autorisation du compte est vérifiée avant la conservation en RAM ;
aucun nom de compte n'est retourné. Le service ne persiste rien dans le coffre
et ne remplace pas son intégration durable future.

L'essai réel a montré que la seule session lisible depuis JavaScript ne
suffisait pas : le navigateur valide le compte, mais le même accès Rust reste
non confirmé. L'initialisation sans contexte de navigateur recevait aussi une
redirection de consentement. Le fournisseur peut désormais recevoir la version
du client et les données visiteur de l'onglet déjà autorisé, sans fabriquer de
consentement ; il vérifie toujours le compte par une requête Rust directe.

Pour l'import complet, le propriétaire peut copier une requête POST de lecture
authentifiée, **Copy as fetch (Node.js)**, depuis les outils développeur de
Chrome. La commande dédiée lit ensuite le presse-papiers par un tube directement
dans le programme Rust :

```sh
pbpaste | target/music-poc/debug/moli-music-poc pair-youtube-stdin
```

Cette commande ne doit être lancée qu'après confirmation que la bonne requête
est copiée. Ne pas coller son contenu dans un terminal, un fichier ou la
conversation. Le texte JavaScript n'est jamais exécuté : seuls l'origine, la
méthode, les en-têtes et le contexte JSON sont analysés, avec une limite de
64 Kio. Les autres origines et le code ajouté sont refusés. L'en-tête
Authorization copié n'est pas réutilisé ; Rust calcule lui-même sa signature.
La [méthode d'authentification navigateur documentée par le client de référence](https://ytmusicapi.readthedocs.io/en/stable/setup/browser.html)
explique pourquoi les en-têtes de la requête authentifiée sont nécessaires.

L'écoute est limitée à `127.0.0.1`, avec Host exact et Origin
`https://music.youtube.com`. Le corps est limité à 17 Kio, l'authentification
à cinq tentatives et la recherche à cinq appels, un seul à la fois. L'arrêt
ou l'expiration supprime la session de l'état actif. Les réponses n'exposent
que le statut et le nombre de résultats. CORS ne constitue pas une
authentification de production : ce mécanisme est réservé au POC local.

Chrome peut demander au propriétaire l'autorisation d'accès au réseau local ;
le programme ne la valide pas à sa place. Aucun morceau n'est lancé lors de
cette association.

### Client MA de référence

Pour l'inventaire ou la recherche MA, fournir `MUSIC_ASSISTANT_URL` et
`MUSIC_ASSISTANT_TOKEN` par environnement, via le coffre. Ne pas mettre les
valeurs dans une commande, un fichier ou une conversation.

```sh
target/music-poc/debug/moli-music-poc inspect-ma
target/music-poc/debug/moli-music-poc search-ma apple_music 'Recherche de test'
target/music-poc/debug/moli-music-poc search-ma ytmusic 'Recherche de test'
```

Le client accepte uniquement cinq commandes MA de lecture. Il refuse les
redirections, borne les réponses à 1 Mio et n'expose pas les noms de lecteurs,
leurs identifiants, les URLs ou le jeton dans l'inventaire. Une source absente
reste `not_configured`, sans résultat inventé.

## Constat distant du 27 octobre 2026

Inventaire en lecture seule d'une instance existante : MA **2.11.0b6**, schéma
99, six lecteurs Cast, un AirPlay et sept lecteurs universels. Apple Music,
YouTube Music et Alexa ne figurent pas dans les fournisseurs configurés.
Le jeton est resté sur l'hôte. Les lectures n'ont changé ni file ni lecteur.
Le propriétaire a confirmé cette instance comme référence pour la suite.
Aucune adresse, nom de lieu ou identité d'appareil privée n'est publiée ici.

## Références et limites des fournisseurs

- [Apple Music dans MA](https://www.music-assistant.io/music-providers/apple-music/) :
  authentification et décodage propres au fournisseur, à qualifier ; une API de
  catalogue ne garantit pas un flux exploitable par le moteur Rust.
- [YouTube Music dans MA](https://www.music-assistant.io/music-providers/youtube-music/) :
  interfaces non officielles, cookies et résolution des flux dépendants de la
  version. Le code amont courant utilise notamment `ytmusicapi`, `yt-dlp` et
  un générateur de jetons PO. Ne pas confondre sa branche courante avec la
  version installée observée.
- [AirPlay dans MA](https://github.com/music-assistant/airplay-cli) :
  émetteur séparé `cliairplay` en C ; ce n'est pas un émetteur Rust livré ici.
- [Alexa dans MA](https://www.music-assistant.io/player-support/alexa/) :
  service Skill/HTTPS et configuration de compte ; pas de sortie LAN universelle.
- [Synchronisation MA](https://www.music-assistant.io/faq/tech-info/) :
  les horloges, timestamps de frames et latences sont une responsabilité séparée.
  Le banc ne mesure aucune synchronisation acoustique.

Suites nécessaires : authentification humaine des deux sources, implémentation
et qualification de leur résolution audio en processus isolés, file musicale et pipeline
continu, puis une sortie neutre Cast ou AirPlay. Alexa et la synchronisation
multiroom doivent être qualifiés séparément. Le choix d'un module externe
expérimental reste distinct d'une implémentation native Rust.
