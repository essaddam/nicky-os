# Vérification des orientations mémoire

Banc synthétique autonome, en Rust et bibliothèque standard seulement. Il mesure des
tailles et vérifie quelques invariants de la **proposition**, sans modifier Moli,
ouvrir une installation, lire une donnée privée ni commander un appareil. Ce n’est
ni un benchmark du système complet ni une preuve d’autorisation ou de sûreté du moteur.

Le layout `Row96` reprend exactement les champs `Row64` + `Numeric32` du
[`alias-banc`](../alias-banc/main.rs). Les autres enveloppes utilisent `repr(C)` :
alias à trois identités stables et cache slot/génération, appartenance durable à
deux identités, arête de cache à deux indices et paire `(u32, u16)`.
Cette dernière occupe **8 octets**, padding compris, et non les 4 d’un indice `u32`.
Les tailles de pointeur et `usize` sont mesurées sur la cible de compilation.

Le JSON de sortie donne les tailles et deux calculs sans allocation géante : CSR
pour un million de nœuds et quatre millions d’arcs, puis budget brut pour un million
de lignes et d’alias et quatre millions d’appartenances et d’arêtes de cache.
CSR compte bien `n + 1` offsets. Les offsets `u32` supposent que le nombre d’arcs et
chaque destination restent représentables ; une structure mutable, ses capacités,
les index, chaînes, allocateurs et services ne sont pas compris dans ces budgets.

Les 18 tests couvrent :

- tailles, padding, offsets CSR et débordement de la formule ;
- famille → maison → étage → pièce, pièce directement sous maison si étage absent,
  appareil sous pièce ou maison ;
- zones logiques qui se chevauchent, sans remplacer la contenance ; refus des identités
  dupliquées, parents absents, cycles, mauvais types et appartenances entre maisons ;
- distinction explicite avec les zones géographiques existantes de `people.rs`
  (latitude, longitude, rayon) et les étages dessinés de `plan.rs` ;
- calcul de l’ancêtre maison sans copier la valeur et identité source d’alias séparée
  du placement de l’appareil ; une contenance ambiguë est refusée, jamais corrigée ;
- perte d’une transition `false → true → false` après coalescence, péremption sans
  nouvelle valeur, `null`/NaN/infini distincts de zéro, dérive d’une accumulation
  flottante naïve, incompatibilité d’unités et renouvellement sans changement de valeur.

Le validateur miniature n’a aucune notion de droits : une appartenance logique
n’accorde rien et ne peut servir de preuve d’accès. Il n’est pas un schéma de migration.
Les exemples de moyenne utilisent de petites valeurs ordinaires et ne remplacent
pas l’algorithme numérique courant, qui traite aussi les valeurs extrêmes et les
conversions explicites. Le test d’unités illustre un contrat conservateur ; il ne
reproduit pas les conversions du produit. Le contre-exemple de somme teste une idée
naïve, pas une opération livrée : `AggregateOp` expose actuellement `Min`, `Max`,
`Mean` dans `crates/moli-core/src/aggregate.rs`.

Exécuter depuis la racine (sources montées en lecture seule, sorties dans `/tmp`) :

```sh
docker run --rm --network none -v "$PWD/tools/memoire-orientations-banc:/bench:ro" moli-os-dev:latest sh -c 'rustc --edition=2024 -D warnings -C opt-level=s /bench/main.rs -o /tmp/orientations && /tmp/orientations'
docker run --rm --network none -v "$PWD/tools/memoire-orientations-banc:/bench:ro" moli-os-dev:latest sh -c 'rustc --edition=2024 -D warnings -C opt-level=s --test /bench/main.rs -o /tmp/orientations-tests && /tmp/orientations-tests'
```

Les résultats valent pour ce modèle et ces contre-exemples. Ils ne prouvent ni coût
RSS réel, ni latence de propagation, ni tenue d’une installation à cette échelle.
