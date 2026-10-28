# Banc des transformations réelles

Ce banc hors workspace appelle les crates de production par dépendances de chemin.
Il sépare les **valeurs entrantes**, les **actions sortantes**, et les propositions
de descripteur affine et d'index d'identités. Aucun broker, serveur HTTP, équipement,
secret, LLM ni commande physique n'est utilisé. Toutes les données sont fictives.

```sh
cargo test --release --offline --manifest-path tools/transformations-banc/Cargo.toml
cargo run --release --offline --manifest-path tools/transformations-banc/Cargo.toml -- 20000 7
cargo run --release --offline --manifest-path tools/transformations-banc/Cargo.toml --bin index-memory
```

Le runner conserve tests, mesures JSONL, métadonnées et empreintes des sources :

```sh
python3 -I tools/transformations-banc/executer.py --iterations 20000 --samples 7
```

`--output <dossier>` choisit un dossier neuf. Les commandes utilisent le profil release
du produit (`opt-level=s`, LTO fat, une unité de compilation). La première commande
génère le verrou de dépendances de ce banc ; le runner utilise `--locked` s'il existe.
Rust 1.88 minimum, comme le dépôt ; les dépendances doivent déjà être présentes pour
`--offline`. Aucun outil n'est installé par le runner.

## Ce qui est réellement mesuré

| Mesure | Frontière |
| --- | --- |
| `mqtt_bytes_route_decode` | classification du topic, vrai `Catalog::by_name`, parsing JSON depuis les octets, vrai `Entry::decode`, 8 valeurs ; catalogues de 1, 100 et 1 000 appareils |
| `action_api_json_lookup_validate_encode` | JSON d'une commande scalaire, vrai `PointId::split`, vrai `Catalog::by_id`, accès, vrai `validate`, vrai `Entry::encode`, sérialisation JSON |
| `experimental_action_api_json_index_validate_encode` | même chemin, avec un index supplémentaire ID→Entry à la place du balayage `by_id` ; **proposition de banc** |
| `experimental_action_index_rebuild` | construction puis destruction de cet index complet ; allocations, hachage et clonage d'Arc compris |
| `action_codec_only` | vrai codec sortant sur une entrée résolue et une valeur validée à l'avance : frontière volontairement plus étroite |
| `profile_json_parse_and_compiled_read` | vrai `Answer::parse` JSON, puis les 5 vrais `CompiledPoint::read` d'un profil fictif : échelle, arrondi, binaire, enum, pourcentage |
| `profile_compiled_read_only` | les mêmes lectures sur une réponse déjà parsée |
| `profile_recompile_each_message_naive_baseline` | mêmes lectures, mais TOML et profil recompilés à chaque message : **baseline volontairement naïve, pas le produit actuel** |
| `profile_action_api_json_validate_render` | parsing JSON scalaire, séparation du point, recherche du point et de l'écriture compilés, vraie validation et vrai `CompiledWrite::render` Daikin depuis une réponse KV fictive conservée en cache |
| `profile_action_validate_render_only` | validation + rendu, point/écriture/réponse déjà résolus |
| `core_unit_ratio_lookup` | vraie sélection de ratio d'unités compatibles du cœur et multiplication |
| `experimental_affine_inbound/outbound` | **proposition de banc**, bornée et déterministe, 0..254↔0..100 ; jamais raccordée au produit |

Les topics et commandes parcourent **tous** les appareils de chaque catalogue.
32 réponses JSON sont réutilisées, leurs valeurs varient. La mesure d'action inclut
le balayage linéaire actuellement effectué par `Catalog::by_id` ; le chemin entrant
utilise déjà des tables de hachage. Les catalogues, topics, octets des messages et
profils compilés sont préparés avant chronométrage, sauf pour les mesures de compilation
et de construction explicitement nommées.

`index-memory` mesure séparément les octets de layout réellement demandés à
l'allocateur et encore vivants après construction de l'index, puis exige leur libération
à sa destruction. Ce binaire seul utilise un allocateur compteur : les mesures de temps
n'ont **aucun compteur d'allocation**. Le coût additionnel exclut les données `Entry`
et les chaînes d'identité partagées par Arc, la mémoire déjà détenue par le catalogue,
les métadonnées de malloc et le RSS. L'en-tête sur pile est fourni à part. L'index
emprunte les entrées : il doit être reconstruit à chaque remplacement du catalogue.

Les durées sont des **moyennes par lot**, avec une médiane, un meilleur et un pire lot.
Elles ne sont pas des percentiles de latence par message. Les sorties indiquent les
effectifs et chaque moyenne brute ; elles ne doivent pas servir seules à annoncer
un budget p99 ou des performances réseau. Le checksum et `black_box` maintiennent
les conversions observables.

## Contrats et limites

Les tests couvrent les codecs binaires et enums numériques dans les deux sens, la couleur
imbriquée, les 255 luminosités natives, les bornes et types des commandes, les entrées
absentes/nulles/non finies, la mise à l'échelle et l'arrondi des profils, la conversion
d'unités, les paramètres repris dans une écriture Daikin et le refus si une réponse
nécessaire manque. Les écritures de composites Z2M restent refusées par le modèle actuel.

La luminosité Z2M de production reste **native 0..254**, sans conversion implicite en
pourcentage. Un profil peut appliquer explicitement `scale = 100/254` à la lecture ;
cela ne crée **aucune fonction inverse automatique** pour les écritures. Le descripteur
affine expérimental traite séparément les deux directions, accepte une plage native
décalée et une inversion explicite, refuse les valeurs hors bornes au lieu de les
écrêter, puis arrondit la commande native si elle est entière. Les tests vérifient
tous les allers-retours natifs et l'erreur de quantification d'un demi-pas.

Le contrat d'action existant est un **point typé et une valeur scalaire** : booléen,
nombre, texte enum/texte, ou `null` pour un bouton. Le banc teste le vrai validateur
de boutons ; le profil déclaratif et les exposes Z2M ne créent pas de bouton de ce
type. Un objet de paramètres arbitraires dans `value` est refusé par le modèle actuel.
Il n'est pas simulé comme une fonctionnalité disponible.

La sélection du topic de `on_publish` et la vérification d'accès de `Hub::prepare`
sont privées ; le banc possède donc deux petits adaptateurs de sélection, clairement
nommés dans `src/lib.rs`. Les conversions et validations de type sont les fonctions
de production. Les tests des adaptateurs refusent les identités absentes/malformées,
les points inconnus ou en lecture seule, les JSON invalides, les échos `/set` et `/get`.
Cela **ne teste pas les routes REST/MCP ni le Hub complet** : garde, approbations,
délais, file du pilote, journal, publication d'état et messages du réseau sont hors
frontière. La réponse KV des écritures est préparée : le rafraîchissement avant/après
commande et l'échange HTTP réel ne sont pas chronométrés.

## Provenance du code appelé

- `crates/moli-z2m/src/catalog.rs` : catalogue, décodage et encodage réels.
- `crates/moli-z2m/src/lib.rs` : frontière du gestionnaire privé `on_publish`.
- `crates/moli-profile/src/profile.rs` : compilation, lecture et rendu réels.
- `crates/moli-profile/src/lib.rs` : moteur et frontière d'envoi HTTP non exécutée.
- `integrations/daikin-brp069/profile.toml` : véritable contrat de sortie du catalogue.
- `crates/moli-core/src/id.rs`, `value.rs`, `spec.rs`, `aggregate.rs` : identités,
  valeurs, validation et ratios réels.
- `crates/moli-runtime/src/hub.rs` : accès, routage et garde ; **non exécuté** ici.

Le runner conserve le commit et les empreintes SHA-256 de ces fichiers, des manifestes,
du verrou du banc et des sources du banc. Un écart d'empreinte signifie qu'on ne rejoue
plus exactement la même version. Les instantanés des sources du banc sont conservés
dans la sortie, sans embarquer de données d'une maison.
