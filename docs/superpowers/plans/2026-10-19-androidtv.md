# Plan d'implémentation — Android TV et contenus

> Pour l'exécution : appliquer `superpowers:executing-plans` tâche par tâche.
> Méthode proposée : réalisation dans cette session, sans nouvel agent.

**But :** appairer une Android TV depuis Moli et piloter la télé ainsi que
les contenus Netflix, Crunchyroll, Free TV, Prime Video, Disney+, Molotov et
Apple TV selon les capacités observables des applications.

**Architecture :** protocoles dans `moli-androidtv`, pilote séparé dans
`moli-androidtv-driver`, passerelle d'appairage partagée avec `moli-api`.
Les contenus passent par le même hub que toute commande physique.

**Technologies :** Rust, Tokio, TLS épinglé, protobuf existant, Svelte 5.

**Spécification :** [conception et extension contenus](../specs/2026-10-19-androidtv-design.md).

## Contraintes globales

- Conserver les signatures et les variantes publiques consommées par Philips.
- File FIFO : quatre requêtes au maximum, un seul ordre physique en vol.
- Échéance absolue du hub : cinq secondes, abandon vérifié avant émission.
- Appairage : une tentative par instance, 120 s, étapes réseau de 4 s,
  trames de 64 Kio maximum ; identité conservée atomiquement dans le coffre.
- JSON d'appairage : 1 Kio ; requête de contenu : 4096 octets ; URL : 2048 octets.
- Les sept services sont proposés ; les capacités non établies restent
  explicitement indisponibles. Transmission, appli ouverte et lecture observée
  sont trois résultats différents.
- Pas de donnée personnelle, d'identifiant de compte ou de catalogue privé
  dans les fixtures ; textes fr/en dès leur introduction.
- Pas de commande physique ni déploiement avant H1. Aucun push.
- Décision propriétaire du 20 octobre : tests natifs Rust 1.99.1 dans le
  worktree, deux jobs depuis la décision du 20 octobre à 20:56 UTC ;
  itérations ciblées libres, sans octroi, préflight ni manifeste.
  Les quatre tests réseau `moli-profile` Linux restent réservés au tour de
  fusion sur `main`, dans un target neuf et avec un seul job Docker sur la machine ;
  ils ne bloquent ni la soumission ni la fusion. Le
  contrôle natif complet reste obligatoire une seule fois à la fusion, un
  seul à la fois selon la file du coordinateur. La soumission exige fmt et
  Clippy workspace, puis les tests ciblés verts des crates touchées. Le
  test `authenticated_d13_transport_handles_replay_conflict_gap_and_read`
  est réintégré au contrôle complet depuis la fusion du correctif archive
  dans `main` (`c5f2002`, décision du 21 octobre). Aucun skip D13 ni du test
  invité HTTP pour cette branche, qui modifie `crates/moli-api/tests/`.
- Après approbation du ticket (`done`), rebaser sur `main`, refaire le contrôle
  complet, puis fusionner localement par avance rapide. Ne pas pousser. Un
  conflit dans la logique d’un autre ticket revient au coordinateur. Les
  listes partagées gardent la version de main et reçoivent seulement les
  nouvelles entrées du pilote, à leur place alphabétique.

## Points de relecture prioritaires

1. Netflix déjà au premier plan : cet état ne prouve pas le changement d'épisode.
2. Free TV : l'identifiant interne de chaîne ne désigne pas son numéro de bouquet.
3. Série et épisode : ne jamais inventer l'identifiant d'un épisode depuis celui d'une série.
4. Appairage remplacé ou annulé : une ancienne réponse ne doit jamais activer la nouvelle session.
5. Perte de liaison après émission : journaliser l'incertitude et ne pas rejouer l'ordre.

## Task 1: Contrat de lancement partagé

La recherche et la résolution de catalogues sont suivies dans le ticket
`0983449d-040b-4248-ba9e-aa600db39e70`, indépendamment des lecteurs.
Ce lot fournit le contrat de cible et les tests des formats explicites ;
il n'implémente aucune recherche de catalogue ni de guide TV.


Fichiers : `crates/moli-media-target/{Cargo.toml,src/lib.rs,src/tests.rs}`,
workspace et dépendances serde/URL uniquement si nécessaires au parseur.
Cette petite crate de valeurs ne dépend ni d'Android TV ni d'un fournisseur
réseau ; le ticket transverse pourra réutiliser ce contrat.

Interface : `ContentRequest { service, kind, target }`, énumérations
`Service` et `ContentKind`, `resolve(&ContentRequest) -> Result<LaunchTarget,
ContentError>`. `LaunchTarget` porte le lien, les paquets attendus et la
capacité visée ; le moteur n'effectue aucune requête réseau de résolution.

- [x] Écrire les contrats avec les URL attendues de la spécification :
  Netflix `12345678` → `/title/12345678?source=30` ; Crunchyroll série/épisode
  et langue ; Free TV ID `123` → `https://oq.ee/channel/123/play` ; Prime Video
  `B000TEST00` → lien `gti` ; UUID Disney+ ; lien Apple TV complet.
- [x] Ajouter `ContentKind::Replay` pour Free TV. Tester séparément chaîne,
  portail et programme : `https://oq.ee/replay/123` est un portail, jamais
  une preuve de lancement du replay 123 ; aucun chemin de lecture Android
  n'est fabriqué. Conserver le lien de programme explicitement fourni avec
  sa qualification, ou refuser si son type ne peut pas être établi.
- [x] Tester séparément les sept ouvertures d'application ; refuser la cible
  Molotov précise non qualifiée avec `UnsupportedTarget`, sans repli accueil.
- [x] Tester les hôtes ressemblants, userinfo, ports, caractères de contrôle,
  liens mal formés, JSON inconnu, limites 2048/4096, type de cible incompatible,
  paramètres de suivi et conservation de l'identifiant d'épisode.
- [x] Constater les échecs attendus, puis
  implémenter le parseur et rejouer les mêmes tests jusqu'au vert.
- [x] Documenter provenance et niveau de preuve de chaque format ; ne pas
  qualifier un lien Android TV par un simple HTTP 200 du site web.

## Task 2: Appairage Polo et identité

Fichiers : `crates/moli-androidtv/src/pairing/{mod,identity,wire,tests}.rs`,
`src/lib.rs`, manifestes et verrou des dépendances.

Interfaces : `Identity::generate() -> Result<Identity>`,
`PairingSession::start(tls, host, identity) -> Result<PairingSession>`,
`finish(&mut self, code: &str) -> Result<()>`. La session possède la socket,
le certificat distant et son échéance absolue. Le parsing ne journalise
jamais le code. Certificat RSA et clé sont générés hors de la boucle pilote.

- [x] Écrire les contrats de preuve SHA-256 avec clés publiques connues,
  rejet des codes invalides et des mauvais octets de vérification.
- [x] Préparer le faux serveur TLS : négociation complète, fragmentation,
  ordre de messages invalide, refus, mauvaise version, dépassement de taille,
  déconnexion et silence. Aucun PEM privé ajouté au dépôt.
- [x] Constater les échecs ; implémenter les
  échanges et la génération Rust, puis vérifier le vert ciblé.
- [x] Vérifier l'arrêt de chaque échange et l'expiration humaine à 120 s
  même si des réponses arrivent ; vérifier que les erreurs restent sans secrets.

## Task 3: Transport avec échéance et résultat

Fichiers : `crates/moli-androidtv/src/{remote,cast,tracked}.rs`, tests associés.

Interface additionnelle : `TrackedOrder<T>` avec charge, échéance absolue,
prédicat d'abandon et canal de résultat ; `tracked_session` pour Remote et
Cast. Garder les anciens `Order`, `Event` et `session` utilisables par Philips.

- [x] Tests sur écritures capturées : abandon/expiration avant le dernier
  octet d'attente = zéro émission ; résultat conservé après abandon post-envoi.
- [x] Tester saturation, ordre d'émission, panne et reconnexion sans rejeu,
  trame partiellement écrite et fermeture/libération de la tâche de lecture.
- [x] Tester Cast sans session média, erreur corrélée et réponse perdue ;
  succès sur émission seulement ne doit pas devenir « lecture confirmée ».
- [x] Après rouge, implémenter puis valider ces contrats et tous les
  tests `moli-philips` et protocoles préexistants dans un créneau distinct.

## Task 4: Pilote et passerelle

Fichiers : `crates/moli-androidtv-driver/{Cargo.toml,src/lib.rs,src/config.rs,
src/model.rs,src/commands.rs,src/pairing.rs,src/tests.rs}`.

Interfaces : `Config`, `AndroidTv::new(Config) -> Result<AndroidTv>`,
`AndroidTv::gateway() -> PairingGateway`, implémentation de `Driver` pour
`kind = "androidtv"`. Passerelle : état, démarrage, soumission et annulation
par génération ; aucun secret renvoyé aux surfaces.

- [x] Tests de configuration hors réseau et identité `<instance>:tv` stable.
- [x] Tests du vrai hub : points et accès, file de quatre en vol compris,
  FIFO unique Remote/Cast, délais et appelants abandonnés, refus immédiat plein.
- [x] Tests d'état : perte de Remote = hors ligne et états inconnus ; Cast
  perdu = métadonnées inconnues ; ne pas publier « éteinte » par supposition.
- [x] Tests volume par observations successives et limite absolue, sourdine
  inconnue, réveil/veille distincts, réglage partiel et retour d'erreur explicite.
- [x] Tests de lancement : même appli déjà ouverte, refus explicite du lien,
  aucun repli accueil d'une cible précise, contenu envoyé non confirmé par Cast.
- [x] Tests appairage : ancienne génération, annulation puis nouveau code,
  coffre indisponible, panne de persistance, identité durable précédente conservée.
- [x] Après rouge, implémenter la boucle supervisée, la passerelle,
  l'entrée de coffre atomique et les traductions, puis obtenir le vert ciblé.

## Task 5: Câblage, HTTP et interface

Fichiers : `Cargo.toml`, `crates/moli-os/{Cargo.toml,src/main.rs}`,
`crates/moli-api/{Cargo.toml,src/lib.rs,src/androidtv.rs,tests/http.rs}`,
`web/src/maison/ui/AndroidTvPairing.svelte`, `MediaLauncher.svelte`,
`web/src/maison/pages/{Systeme,Salon}.svelte`, `web/src/maison/lib/media.js`,
`media.test.js`, `locales/{fr,en}/*.json`.

- [x] Enregistrer la construction hors réseau, les passerelles des seules
  instances actives et `KINDS` ; trier les listes partagées touchées.
- [x] Avant les handlers, tests HTTP pour GET/POST/PUT/DELETE selon la spec,
  PIN, invité, démo, instance inconnue/désactivée, JSON excessif ou inconnu,
  génération périmée, réseau expiré et réponses dépourvues de secret.
- [x] Après rouge, implémenter les routes et l'audit sans code/PEM ;
  ajouter la description dans `docs/components/api.md` et OpenAPI versionné.
- [x] Carte Système : état, démarrage explicite, six caractères, annulation,
  demande PIN avec mécanisme existant, erreurs et effacement de la saisie.
- [x] Carte Salon : sept services, type de contenu, lien/ID, favoris locaux
  avec version, maximum 50 entrées, texte 80 caractères ; commande via `act`
  et le point `play_media`, jamais un appel direct vers la télé.
- [x] Tests Node du stockage des favoris : version inconnue, données invalides,
  doublons, suppression, plafond ; les entrées restent propres à chaque télé.
- [x] Afficher uniquement les capacités de la télé sélectionnée ; aucune
  commande `play_media` envoyée à une Philips qui ne déclare pas ce point.
  Masquer Ambilight si absent. Conserver la télécommande Philips existante.
- [x] Vérifier lecture clavier, libellés fr/en, sortie d'erreur et absence de
  faux état « lecture démarrée ». Rejouer tests Node puis build web/i18n.

## Task 6: Catalogue, garde-fou et remise

Fichiers : `integrations/androidtv/{integration.toml,README.md,onboarding.md,
fixtures/*}`, `docs/components/pilotes.md`, documentation de contrat OpenAPI.

- [x] Compléter le paquet avec fixtures synthétiques, besoins `host`/coffre,
  ports, étapes humaines et restrictions par service, sans donnée de maison.
- [x] Expliquer comment récupérer un lien de titre/épisode/replay et un ID interne
  Free TV ; ne pas figer de catalogue de chaînes ou de correspondance personnelle.
- [x] Exécuter le contrôle complet natif une seule fois au tour de fusion,
  après approbation ; relever les résultats et les éventuelles exclusions.
- [x] Relever séparément `npm test`, `npm run build` et i18n pour l'interface,
  ainsi que le périmètre exact du contrôle de fuites (liste privée disponible ou non).
- [x] Vérifier qu'une instance désactivée ne démarre aucune tâche ; conserver
  la qualification et les mesures sur appareil réel pour après H1.
- [ ] Après H1, qualifier séparément Free TV direct, portail replay et
  programme replay sur la version Android installée ; conserver les liens
  acceptés et les refus, sans confondre les routes web et Android.
- [x] Présenter les fichiers, preuves, limites Molotov et liens non qualifiés
  au relecteur du ticket. Fusion locale après approbation selon la file ; aucun push.

## État de réalisation (21 octobre)

Implémentation des six étapes prête pour revue dans ce worktree : résolveur
partagé, identité RSA/Polo, transports suivis, pilote et coffre, routes HTTP,
interface traduite, câblage binaire et paquet de catalogue.

Validation ciblée avant le rebase final, Rust 1.99.1 avec deux jobs : fmt et Clippy workspace stricts
verts ; 218 tests ciblés réussis, aucun échec : bibliothèque Android TV 41,
pilote 31, API 66 (dont 20 HTTP), cibles partagées 13, binaire 13, Philips 5,
runtime 49 (unitaires, hub et doctest). Les tests couvrent aussi la persistance
acceptée avant redémarrage et l’ouverture native des sept applications.
Le client HTTP de test accepte un reset TCP après réception d’une réponse
complète avec Content-Length ; une réponse tronquée ne passe pas.

Web après rebase : 35 tests, build et traductions verts ; saisie/effacement du code,
annulation pendant vérification et favoris vérifiés dans le navigateur sur
réponses simulées. Aucun test sur téléviseur réel. Molotov précis et liens
non qualifiés restent explicitement limités. Recherche transverse suivie
séparément au ticket 0983449d.

Catalogue : 32 paquets, dont le pilote Android TV, contrôle réussi avec six
avertissements préexistants sur les fixtures des profils. Contrôle de fuites
propre sur les motifs génériques ; aucune liste noire privée disponible.

Revue corrective : les demandes en attente sont liées à leur génération
Remote/Cast dès l’admission et avant le premier poll du job. Les régressions
sur vrai hub refusent une Pause reçue avant/pendant une perte Cast et
autorisent le témoin sain après reconnexion. STOP accepte un statut vide
strictement corrélé à cette opération, conserve la connexion et ne se rejoue
pas ; faux identifiant/source/namespace, statuts incomplets et erreurs
restent distincts. Aucun changement du chemin Philips.

Après approbation et rebase sur `main` incluant Chaleur, contrôle complet
natif exécuté une seule fois sur les sources finales : fmt, Clippy workspace
strict, 1 143 tests Rust réussis, aucun échec ni test ignoré. Seuls les quatre
tests `moli-profile` de bind Linux sont filtrés ; D13 et le test invité HTTP
sont inclus. Catalogue : 38 paquets conformes, six avertissements préexistants.
Fuites : motifs génériques propres, aucune liste privée disponible. Le client
HTTP corrigé et le fsync coffre de `main` sont conservés ; les fabriques de
pilotes découpées reçoivent Android TV à sa place alphabétique.

Les procédures historiques de collecteurs, octrois et manifestes ont été
remplacées par les décisions natives du propriétaire. Aucune qualification
sur téléviseur réel ni publication distante.
