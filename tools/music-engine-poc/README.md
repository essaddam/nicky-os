# Moteur musical Rust : verticale locale T0

Cette tranche exécute **source synthétique → catalogue → file révisionnée →
décodeur Rust → null ou WAV**. Elle constitue le premier moteur local, pas un
équivalent complet de Music Assistant.

Elle est indépendante du POC de transport `tools/music-poc/`. Aucune dépendance
Music Assistant, HTTP, MQTT, sortie audio système, FFmpeg ou processus musical
Python n'entre dans ce paquet. La source n'est jamais jouée sur un appareil.

## Frontière avec le master et les hôtes

Le master minimal reste externe : ce paquet ne le modifie pas et ne lui ajoute
ni broker ni logique audio. L'API `Engine<S>` peut être détenue par un hôte de
module, regroupé ou isolé. Le binaire fourni est un exécutable de qualification
qui termine après sa démonstration, pas encore un service de production.

Le contrôle est une API typée ; le futur adaptateur MQTT transportera seulement
commandes et événements. Les blocs PCM ont un autre type et ne sont pas des
payloads de contrôle. Aucun raccord à la garde de production n'est revendiqué.

## Sources et catalogue

`Catalog::synthetic()` génère deux WAV privés, `tone-a` et `tone-b`, à
48 kHz, deux canaux, PCM signé 16 bits. Chaque source contient 4 800 trames,
soit 0,1 seconde ; les deux canaux et les deux sources ont des valeurs connues
différentes. Il n'y a ni enregistrement ni œuvre tierce.

Les fichiers sont écrits par blocs. Le catalogue et ses libellés résident dans
un manifeste privé temporaire, relu lors d'une consultation. Le moteur ne garde
ni les libellés ni un PCM de toute la bibliothèque. Ces fichiers sont supprimés
à la fin du propriétaire `Catalog`.

Une commande sélectionne uniquement un `SourceId` déclaré : 1 à 64 caractères
ASCII alphanumériques, tirets ou underscores. Chemins, URI, séparateurs et
identifiants absents du catalogue sont refusés. Il n'existe pas de commande
« ouvrir cette adresse ».

## API et transitions

```rust
let mut engine = Engine::new(Catalog::synthetic()?, NullSink::new())?;
let sources = engine.catalog().list()?.into_iter().map(|entry| entry.id).collect();
engine.apply(0, Command::ReplaceQueue { sources })?;
engine.apply(1, Command::Play { index: 0 })?;
let event = engine.render_next(512)?;
```

| Opération | Comportement |
| --- | --- |
| Création / restauration | `stopped`, aucune ouverture de décodeur ni lecture automatique |
| Remplacement de file | Validation complète, nouvelle révision, arrêt et purge des blocs en attente |
| `play(index)` | Sélection explicite d'une entrée de la file, depuis sa première trame |
| `pause` | Plus aucun bloc produit ; la position du producteur reste stable |
| `resume` | Reprise d'un état `paused` seulement |
| `seek(frame)` | Position exacte dans la source active ; conserve l'état en pause ou en lecture |
| `stop` | Ferme le décodeur, purge les blocs en attente, revient à zéro et rend le WAV partiel valide |
| EOF d'une piste | Passe à l'entrée suivante de la file, sans perdre la fin déjà en attente |
| EOF de la file | Attend les derniers blocs de la sortie bornée, finalise et passe à `ended` |
| Erreur de lecture ou de sortie | `faulted`, aucun rejeu implicite |

Toutes les commandes comparent la révision attendue. Un remplacement invalide
ne change pas la file existante. Une révision ancienne ne déclenche aucune
lecture. `QueueSnapshot` est sérialisable ; `Engine::restore` revalide ses
sources et crée une nouvelle identité de session, toujours à l'arrêt.
Il n'y a pas encore de service de persistance durable de la file.

`command_bytes` accepte une enveloppe JSON de 4 Kio au maximum :

```json
{
  "session": "identite-de-la-session-courante",
  "expected_revision": 1,
  "command": { "operation": "seek", "frame": 1500 }
}
```

Une session ancienne et les champs inconnus sont refusés. L'identité de session
est un marqueur de génération, pas un jeton d'autorisation. L'adaptateur futur
devra aussi fournir authentification, garde, identifiant d'ordre, échéance,
déduplication et journalisation.

Les blocs portent une `epoch`. Play, seek, stop et remplacement de file
invalident les blocs en attente ; un consommateur externe devra aussi vérifier
la session et cette génération. Pause conserve les blocs déjà admis : le
consommateur doit respecter l'état de pause avant de les rendre. Les octets
déjà consommés par null ou WAV ne sont pas retirés du passé. Après EOF, une
sortie WAV finalisée est fermée : une nouvelle lecture nécessite une nouvelle
sortie, au lieu de rester indéfiniment en contre-pression.

## Mémoire et contre-pression

- Une file contient au plus 16 identifiants, avec une seule source décodée à la fois.
- Une lecture rend de 1 à 1 024 trames : au plus 4 096 octets PCM stéréo 16 bits.
- La sortie bornée garde au plus quatre blocs, soit **16 384 octets de PCM**.
- Quand la sortie est pleine, `render_next` ne lit ni ne décode un bloc de plus
  et ne modifie pas la position du producteur.
- Symphonia utilise un tampon d'entrée de 64 Kio et un paquet PCM de
  1 152 trames au maximum dans ce sous-ensemble. Son état interne reste à compter.
- Les sources sont des fichiers réguliers de 10 Mio au maximum et d'au plus
  120 secondes ; les deux plafonds s'appliquent. La sortie WAV est limitée
  à 60 secondes et exige un fichier régulier déjà ouvert par l'appelant.
- Le seek exact réouvre le lecteur et écarte des petits blocs jusqu'à la trame
  voulue. Son travail est borné par la taille de la source ; sa latence n'est
  pas présentée comme constante.

Ces nombres bornent des buffers identifiés, pas le tas entier ni le RSS. Les
allocations des bibliothèques, structures et fichiers du système restent à
mesurer. Aucun RSS exact par module fusionné n'est annoncé.

La position exposée désigne les trames **admises à la sortie**, pas une position
mesurée sur un DAC. Les codecs FLAC et autres structures RIFF sont refusés dans
T0, même si la dépendance pourrait en décoder : seul le WAV canonique de cette
fixture est qualifié ici.

## Vérification reproductible

```sh
sh tools/music-engine-poc/check.sh
```

La commande AgentsRoom « Musique — moteur local T0 » lance le même contrôle :
format, Clippy avec avertissements refusés, tests, puis démonstration. Rust
1.99.1, deux jobs et `target/music-engine-poc` sont imposés ; aucun autre
workspace n'est compilé. Le `Cargo.lock` fige les dépendances du paquet.

La démonstration compare chaque canal et chaque trame, sans garder le PCM
complet. Elle rend les deux sources vers null puis WAV, vérifie l'en-tête et
le condensat des octets PCM, et supprime le WAV temporaire après contrôle.
Le résultat attendu est **9 600 trames, 19 200 échantillons et un WAV de
38 444 octets**, avec le même condensat pour les deux sorties.

Les contrats couvrent ordre et contenu PCM, pause/reprise/seek/stop/EOF,
restauration à l'arrêt, révision périmée, source non déclarée, enveloppe
malformée, codec refusé, troncature, quotas et erreur du consommateur.
La contre-pression est éprouvée sur 1 000 tentatives sans avancement du
producteur ; le programme exécutable est aussi testé.

Le reçu [results.json](results.json) distingue les contrôles exécutés des
limites restantes. Le code utilise [Symphonia 0.6.1, MPL-2.0](https://docs.rs/crate/symphonia/0.6.1/source/Cargo.toml).

## Suite du moteur complet

1. Adaptateur de module et de contrôle MQTT sous la garde, avec transport PCM
   séparé et arrêt supervisé de bout en bout.
2. Catalogue musical durable, bibliothèque incrémentale, entrées stables de file,
   édition atomique et reprise sans son automatique.
3. FLAC, autres codecs, seek natif, transitions sans blanc, DSP et formats de sortie.
4. Fournisseurs natifs Apple/YouTube : comptes, droits, résolution et renouvellement
   des flux à qualifier séparément ; les clients de catalogue sont un autre lot.
5. Sorties Cast/AirPlay/Alexa puis horloges et multiroom : aucune compatibilité
   physique ou synchronisation acoustique n'est déduite de T0.
