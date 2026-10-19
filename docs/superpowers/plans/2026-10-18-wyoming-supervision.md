# Plan d'implémentation : supervision Wyoming

**But :** surveiller Piper, Whisper et openWakeWord sans posséder la chaîne voix.
**Architecture :** un appareil `<instance>:service`, une sonde TCP `describe`
avec le codec partagé, un pilote dormant sans configuration.
**Technologies :** Rust, Tokio, serde_json, catalogue natif et traductions fr/en.
**Contrat :** [supervision Wyoming](../../components/wyoming.md).
**Exécution :** dans ce worktree, sans délégation supplémentaire ni publication.

## Contraintes de validation

Aucun Cargo, build ni contrôle lourd avant allocation explicite de
l'Orchestrateur. Sécurité Z2M est prioritaire ; un seul contrôle lourd sur la
machine. Aucun équipement réel, secret, audio ou mot d'éveil avant H1.
Les tests préparés ne valent jamais preuve tant qu'ils n'ont pas été exécutés.
Pas de commit ni push demandé ; préparer seulement le message pour révision.

## 1. Codec partagé et contrat TCP

Fichiers : `crates/moli-net/src/wyoming.rs` et
`crates/moli-net/src/wyoming/supervision_tests.rs`.

- [x] Lire les décisions D1/D5, le codec et le protocole amont.
- [x] Préparer les fixtures synthétiques des trois familles.
- [x] Écrire les tests de rejet du framing incomplet et des longueurs invalides.
- [x] Obtenir le créneau et constater les échecs sur le codec actuel : 3 PASS / 3 FAIL
  parmi six tests, code 101, base figée `835d9b8`. Réserve de séquencement du
  préflight consignée ; aucun rejeu de ce premier rouge.
- [x] Préparer `describe(host: &str, port: u16, limit: Duration) -> anyhow::Result<serde_json::Value>`
  utilisant `send`, `receive` et un délai global ; imposer `info` sans payload.
- [x] Préparer le durcissement de `receive` : LF obligatoire, objets JSON, longueurs typées, bornes
  vérifiées avant allocation ; conserver la fusion et l'encodage audio existants.
- [x] Préparer les faux serveurs vérifiant la requête unique, la fragmentation,
  les blocs tronqués, la réponse inattendue, l'erreur hostile et le délai global.
- [x] Exécuter le candidat figé sous allocation distincte : formatage ciblé,
  Clippy `--locked --offline -p moli-net --all-targets -- -D warnings` et
  `cargo test --locked --offline -p moli-net wyoming -- --test-threads=1`.
  Le 19 octobre 2026 : codes 0/0/0, 16 PASS, préflight 0 avant Docker,
  676 sources inchangées, créneau libéré. Portée : codec sur base `835d9b8`.

Le candidat codec est validé sur les seuls contrats ci-dessus. Pour toute
vérification future, avant demande d'allocation : figer
sources et lanceur, obtenir un préflight achevé code 0, parser le vrai JSON-lines
Docker et vérifier l'exclusivité des contrôles et caches Moli sans exiger un
Docker vide. Chaque échec arrête le parcours ; aucun lancement pendant l'attente
du préflight. Les phases pilote, catalogue et garde-fou complet demandent
des allocations séparées ; aucun élargissement implicite du créneau codec.

## 2. Pilote en lecture seule

Créer `crates/moli-wyoming/{Cargo.toml,src/lib.rs,src/info.rs,src/tests.rs}`.
Consomme `moli_net::wyoming::describe`, produit `Wyoming::new(Config) -> Result<Self>`.

- [x] Écrire les contrats de parsing des trois fixtures et des données hostiles :
  bornes 16 programmes/64 modèles, 128 caractères/champ, 2048/liste ; Unicode,
  contrôles, champs absents, null, types invalides, tri et déduplication.
- [x] Préparer une première ossature non enregistrée, les options typées et
  11 contrats configuration/métadonnées dans `src/tests.rs`. Avant implémentation,
  le validateur accepte tout et le parseur renvoie une description vide :
  constater les échecs comportementaux sous allocation distincte. Ces stubs
  ne sont pas un pilote fonctionnel ; aucun câblage `moli-os` à ce stade.
- [x] Constater le premier rouge configuration/métadonnées : onze contrats
  exécutés, 2 PASS / 9 FAIL comportementaux, compilation réussie, code 101,
  préflight 0 avant Docker et 680 sources inchangées. Créneau libéré.
- [x] Préparer le validateur et le parseur après ce rouge, sans les exécuter.
  Préparer cinq contrats de cycle de vie et un `Driver` inactif pour leur
  premier rouge. Aucun appareil publié, aucune connexion dans cette ossature.
- [x] Exécuter le passage distinct des seize contrats : configuration et
  métadonnées 11 PASS ; cycle de vie 5 FAIL à l'attente initiale du `Driver`
  inactif, sans atteindre leurs assertions suivantes. Compilation réussie,
  code 101, préflight 0 avant Docker, 681 sources inchangées, créneau libéré.
- [x] Préparer la boucle de supervision après ce rouge : publication RO,
  sonde `describe`, période après résultat et nettoyage à l'abandon de la
  future. Traductions fr/en ajoutées. Aucune validation de cette boucle acquise.
- [x] Exécuter le ciblé suivant : 16 contrats pilote et 16 contrats codec PASS,
  formatage 0, Clippy 101 (`single_match_else`), bilan global 1. Sources et
  tests inchangés pendant ce passage ; préflight 0 avant Docker, créneau libéré.
- [x] Préparer la correction de forme Clippy, sans désactivation de règle ni
  nouveau test miroir ; la prochaine validation doit confirmer son effet.
- [x] Écrire les tests configuration, identité stable, tous points non inscriptibles,
  disponibilité initiale fausse, succès/panne/reprise et annulation pendant lecture.
- [x] Exécuter les échecs attendus dans le créneau alloué, puis implémenter.
- [x] Publier les diagnostics locaux traduits seulement, jamais le texte d'une
  erreur réseau ni une réponse serveur. Effacer les métadonnées après panne.
- [x] Vérifier périodicité sans rafales, arrêt et fermeture de socket côté faux serveur.
- [x] Exécuter fmt, Clippy pédant et tests de la crate dans le créneau alloué.

## 3. Enregistrement, paquet et validation

Modifier `Cargo.toml`, `Cargo.lock`, `crates/moli-os/Cargo.toml`,
`crates/moli-os/src/main.rs`, `locales/{fr,en}/pilotes.json`,
`docs/{README.md,components/pilotes.md}` ; compléter `integrations/wyoming/`.

- [x] Enregistrer le pilote ; trier `KINDS` par ordre alphabétique conformément
  au ticket ; conserver l'ordre des autres entrées partagées existantes.
- [x] Ajouter manifeste, besoins `host`/`port`, aucune découverte automatique.
- [x] Ajouter traductions fr/en, documentation d'onboarding et provenance des fixtures.
- [x] Tester la construction hors réseau et la correspondance catalogue/pilotes.
- [x] Après allocation : tests `moli-os` inclus dans `cargo test --workspace`,
  puis `cargo run --locked -p moli-os -- catalogue check integrations`.
- [x] Contrôles légers : `node scripts/i18n-check.mjs`, `sh scripts/leak-check.sh`.
- [x] Obtenir l'allocation du garde-fou complet `sh scripts/check.sh` ; conserver
  base Git, empreinte du diff, journaux et codes de sortie séparés.
- [x] Mettre à jour l'état de la documentation avec les seules preuves acquises.
- [ ] Présenter le dossier au relecteur architecte désigné via le ticket ;
  qualification réelle toujours distincte et conditionnée à H1.

Résultat du garde-fou complet du 19 octobre 2026 : tous les codes à 0,
735 tests réussis sur 96 suites, catalogue de 31 paquets valide (six
avertissements préexistants hors Wyoming), fuites génériques propres sans
liste privée. Préflight 0 avant Docker, 683 sources inchangées pendant les
880,315 secondes du passage, aucun dépassement ni nouvelle tentative.
La base contrôlée est `835d9b8` ; aucune fusion ou qualification réelle faite.

## Points de relecture

- Ne jamais déduire une adresse des noms reçus dans `info`.
- Une connexion TCP acceptée avec un `info` invalide reste hors ligne.
- Un `error` distant ne doit révéler ni hôte, ni chemin, ni jeton dans les surfaces.
- `receive` est partagé par l'audio : rejouer les contrats existants Piper/Whisper.
- Les fixtures ne doivent jamais être décrites comme des captures de vrais services.
