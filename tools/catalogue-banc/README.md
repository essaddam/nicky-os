# Banc isolé du catalogue

Expérience Rust `std` seule, sans modification du catalogue de Moli, sans réseau et sans
commande physique. Les charges publiques conservent les formes, types, nombres et liens
des exports ; les valeurs, dates et identifiants sont synthétiques. Le manifeste précise
les hypothèses de transformation. `HAS_SET` reste un **candidat**, avec le bit 16
`ACCESS_UNRESOLVED` : aucune autorisation réelle n'est déduite de ce banc.

## Rejouer

Depuis la racine du dépôt, avec l'image de développement déjà construite :

```sh
python3 tools/catalogue-banc/executer.py
```

Ce lanceur compile avec les options inscrites dans ses métadonnées, exécute les tests,
puis démarre un conteneur neuf par essai. Ses résultats sont dans `sorties/`.

Pour compiler et lancer un cas manuellement :

```sh
docker run --rm \
  -v "$PWD/tools/catalogue-banc:/study:ro" \
  -w /study moli-os-dev:latest sh -c '
  rustc --edition=2024 -D warnings --test main.rs -o /tmp/tests && /tmp/tests &&
  rustc --edition=2024 -D warnings -C opt-level=s -C codegen-units=1 main.rs -o /tmp/banc &&
  /tmp/banc --bench charges/somme-detail.tsv uniform64 1 775115'
```

Interface : `--bench INPUT.tsv uniform96|uniform64|split SCALE [ARENA_BYTES_PER_WORLD]`.
Le dernier argument est multiplié par l'échelle. `--help` décrit les douze champs TSV.
Le programme écrit une seule ligne JSON ; une entrée invalide ou une différence de
contenu provoque une sortie en erreur. Chaque essai compare également chaque ligne
décodée à son entrée, au-delà de la simple somme de contrôle.

## Ce qui est mesuré

- `uniform96` : rangées de 96 octets, quartet numérique réservé dans chaque rangée.
- `uniform64` : rangées de 64 octets, quartet numérique de 32 octets seulement pour les
  rangées portant `NUMERIC`, adressé par `aux`.
- `split` : tableau chaud de 32 octets (charge utile et trois dates), tableau froid de
  32 octets (identité, parent, métadonnées), même tableau numérique externe.
- Chaque variante possède le même index `HashMap<u64,u32>` et une adjacence CSR
  (`u32` pour les offsets et les références). Les parents peuvent être des valeurs
  composées ; ils ne sont pas supposés être tous des équipements.

`allocations_requested_bytes` est le delta **vivant** de tailles demandées à `System`
par l'allocateur global, pris juste avant/après la construction. Il inclut les capacités
réelles des tableaux, les buckets de l'index et le pool synthétique de données variables.
Le programme vérifie l'égalité avec la somme de ces postes. Ce compteur exclut les
en-têtes et la fragmentation de `malloc`, les pages de pile et le fichier du programme.
Le pool est réellement alloué et touché, mais son contenu est factice ; son volume vient
du manifeste. Les schémas sont des références vers un schéma commun vide.

Les valeurs RSS et RssAnon sont des lectures absolues de `/proc/self/status`, avec leur
delta avant/après la construction. Leurs granularités de pages, les entrées du banc
conservées en mémoire et le comportement de l'allocateur interdisent de les confondre
avec le budget mémoire du processus Moli. Hors Linux elles valent `null`.

Les temps sont des médianes de sept passages après échauffement. Les lectures aléatoires
utilisent des slots précalculés et 1 048 576 opérations par passage. La lecture scalaire
et les mises à jour parcourent les booléens, entiers et flottants ; la lecture numérique
inclut aussi les états inconnus dont le schéma est numérique. Les boucles séquentielles
lisent charge utile et flags pour toutes les rangées. Les snapshots chronomètrent
allocation, copie et destruction des seuls tableaux de rangées et de bornes ; les index
et le pool commun sont exclus. Un snapshot utilise entre 1 et 128 copies par passage,
selon la taille de charge. Aucun temps de calcul aléatoire n'est inclus.

## Limites de portée

Les mises à jour comparent les flottants par égalité `f64` (les zéros signés sont égaux,
les NaN sont différents), en conservant les bits reçus. Les autres scalaires comparent
leurs bits. Pour les textes et records, le handle doit être immuable et versionné : un
contenu changé exige un autre handle ; aucune arène mutable n'est implémentée ici.
Une réception identique avance
`received_ms`, une modification avance aussi `changed_ms`. `observed_ms` reste inchangé
sans observation explicite, y compris sa valeur inconnue zéro. Le banc mesure cette
opération de stockage, pas la validation d'une commande, la garde ou les autorisations.
Un état d'action artificiel et un `has_set` sur une action sont refusés à l'entrée.
Une rangée d'équipement peut porter un état de santé booléen ou entier et ses trois
dates, sans `HAS_SET` ni quartet numérique. Elle participe aux lectures et mises à jour
scalaires comme les rangées de valeurs scalaires.

Les identifiants sont locaux à un espace d'installation. Une identité globale exige
aussi un identifiant d'installation, par exemple 128 bits. Le petit allocateur testé
garantit croissance et absence de réutilisation avec un high-water mark ; il refuse
l'épuisement et un recul de ce marqueur. Le stockage durable de ce marqueur et des
correspondances avec les identités natives, les tombstones et les suppressions de
rangées ne sont pas implémentés. Les liens de la charge répliquée restent dans leur
copie du monde.

Ce banc ne mesure ni moteur de schémas fonctionnel, ni sérialisation, ni transactions,
ni verrouillage, ni diffusion d'événements, ni journal, ni vraie arène de textes/records,
ni accès matériel. Une comparaison de tailles ou de nanosecondes ici ne constitue pas
à elle seule une validation de l'architecture de production.
