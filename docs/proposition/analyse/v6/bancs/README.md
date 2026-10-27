# Microbancs reproductibles de la V6

Ce dossier mesure les allocations de texte, les tailles de types et une fixture de
17 appareils à deux points comparée à 34 appareils à un point. Il utilise directement
`moli_core::Value`, `Sample`, `Device`, `PointSpec` et `Kind` du dépôt. Leur implémentation
n’est pas copiée ici. Les seuls types candidats sont les prototypes de disposition inline.

Les entrées sont entièrement synthétiques. Le programme ne lit aucun export de maison,
ne reçoit aucun secret, n’ouvre aucun service et n’envoie aucune commande.

## Exécuter

Depuis la racine du dépôt, avec Rust 1.99.1 :

```sh
RUSTUP_TOOLCHAIN=1.99.1 CARGO_TARGET_DIR=target/v6-microbancs CARGO_BUILD_JOBS=2 cargo run --offline --locked --release --manifest-path docs/proposition/analyse/v6/bancs/Cargo.toml -- verifier
RUSTUP_TOOLCHAIN=1.99.1 CARGO_TARGET_DIR=target/v6-microbancs CARGO_BUILD_JOBS=2 cargo run --offline --locked --release --manifest-path docs/proposition/analyse/v6/bancs/Cargo.toml -- disposition
RUSTUP_TOOLCHAIN=1.99.1 CARGO_TARGET_DIR=target/v6-microbancs CARGO_BUILD_JOBS=2 cargo run --offline --locked --release --manifest-path docs/proposition/analyse/v6/bancs/Cargo.toml -- textes
RUSTUP_TOOLCHAIN=1.99.1 CARGO_TARGET_DIR=target/v6-microbancs CARGO_BUILD_JOBS=2 cargo run --offline --locked --release --manifest-path docs/proposition/analyse/v6/bancs/Cargo.toml -- virtuels
```

`verifier` effectue un contrôle court : conservation du texte UTF-8, une allocation à la
création ou au remplacement, aucune au clone ou à l’égalité inchangée, et exactement
34 points dans les deux fixtures. Il n’impose aucun seuil de temps ni taille portable.

Les commandes hors ligne supposent que la chaîne Rust et les dépendances du fichier
`Cargo.lock` sont déjà disponibles. Sur une machine neuve, préparer le cache avec
`cargo fetch --locked --manifest-path docs/proposition/analyse/v6/bancs/Cargo.toml`
nécessite un accès au registre public des dépendances.

Les compteurs ajoutent leur propre coût aux allocations. Pour chronométrer sans eux,
utiliser une seconde compilation :

```sh
RUSTUP_TOOLCHAIN=1.99.1 CARGO_TARGET_DIR=target/v6-microbancs CARGO_BUILD_JOBS=2 cargo run --offline --locked --release --no-default-features --manifest-path docs/proposition/analyse/v6/bancs/Cargo.toml -- textes
RUSTUP_TOOLCHAIN=1.99.1 CARGO_TARGET_DIR=target/v6-microbancs CARGO_BUILD_JOBS=2 cargo run --offline --locked --release --no-default-features --manifest-path docs/proposition/analyse/v6/bancs/Cargo.toml -- virtuels
```

Les champs d’allocations sont alors `null`, car ils ne sont pas mesurés. Les sorties sont
des lignes JSON sur la sortie standard. Les résultats historiques restent inchangés.

Par défaut : 50 000 opérations par échantillon, cinq répétitions, avec 1 000 opérations
d’échauffement par cas texte. Un passage court est possible :

```sh
RUSTUP_TOOLCHAIN=1.99.1 CARGO_TARGET_DIR=target/v6-microbancs CARGO_BUILD_JOBS=2 cargo run --offline --locked --release --manifest-path docs/proposition/analyse/v6/bancs/Cargo.toml -- textes --iterations 1000 --repetitions 3
```

Les itérations sont bornées entre 1 et 5 000 000, les répétitions entre 1 et 31 et doivent
être impaires. Le profil Cargo publié utilise `opt-level=3`. Pour un nouveau passage avec
optimisation propre à la machine, ajouter `RUSTFLAGS='-C target-cpu=native'` à la commande
et conserver ce réglage avec ses résultats ; ne pas comparer silencieusement les deux modes.

## Ce que mesure chaque commande

- `textes` : textes de 16, 64, 256 et 4 096 **octets UTF-8**, avec `é` et `à` répétés.
  Création puis destruction, clone puis destruction, changement, reconstruction identique
  et comparaison permettant de sauter une reconstruction. La chaîne source et la valeur
  initiale sont créées avant la mesure. Le pic est relatif à ce point de départ ; pendant
  un remplacement, l’ancienne et la nouvelle allocation peuvent coexister brièvement.
- `disposition` : tailles et alignements du vrai Core ; puis candidats inline de 15, 16,
  24 et 32 octets, avec toutes les variantes scalaires actuelles. Une variante texte
  imbriquée est comparée à l’enum direct. Ces prototypes ne sont pas raccordés au Core.
- `virtuels` : 17 appareils ambiance T°/humidité contre 34 appareils séparés. Il reste
  17 points température et 17 points humidité dans les deux cas. Les identifiants et
  libellés sont fictifs. La sérialisation des descriptions est mesurée en taille après
  les allocations de construction. Le parcours chronométré lit seulement les longueurs
  des 34 clés de points ; ce n’est pas un calcul métier ni un appel au Hub.

Les compteurs utilisent les tailles des `Layout` demandés à l’allocateur système.
`alloc`, `alloc_zeroed` et les réallocations réussies sont comptés. Le programme tourne
sur un seul fil d’exécution, sans charge asynchrone. Les résultats incluent les allocations
transitoires de construction dans le total, mais seulement les allocations encore vivantes
dans le compteur vivant. Les sorties JSON et leurs allocations arrivent après les mesures.

## Résultats historiques conservés

[Le JSON du 27 octobre 2026](resultats-historiques-2026-10-27.json) conserve les mesures
antérieures : Apple M4, macOS ARM64, Rust 1.99.1, compilation directe `rustc` en O3 avec
`target-cpu=native`. Le banc texte recompilait alors directement le fichier public
`value.rs` ; le banc d’appareils utilisait la bibliothèque Core déjà compilée. Le package
Cargo de ce dossier constitue désormais la procédure reproductible sur le Core courant.
Les nouvelles exécutions peuvent donc différer, même sur la même machine.

| Texte UTF-8 | Allocation demandée par création | Création sans compteurs | Clone sans compteurs |
| ---: | ---: | ---: | ---: |
| 16 o | 32 o, une allocation | 33,1 ns | 4,6 ns, aucune allocation |
| 64 o | 80 o, une allocation | 26,4 ns | 3,9 ns, aucune allocation |
| 256 o | 272 o, une allocation | 28,2 ns | 3,6 ns, aucune allocation |
| 4 096 o | 4 112 o, une allocation | 64,1 ns | 3,4 ns, aucune allocation |

Le clone partage les octets de l’`Arc<str>`. Une comparaison complète n’est pas toujours
plus rapide qu’une reconstruction : à 4 096 octets, le cas historique inchangé parcouru
pour comparaison prenait environ 82,8 ns, contre 65,4 ns pour le remplacement changé.
Ces chiffres ne décrivent pas le dédoublonnage ni le débit du moteur complet.

Tailles historiques : `Value` 24 o, `Sample` 32 o, `Device` 160 o, `PointSpec` 112 o et
`Kind` 48 o. Les candidats inline 15/16 tenaient dans 24 o ; les candidats 24 et 32 prenaient
respectivement 32 et 40 o, y compris avec la variante texte imbriquée.

Dans la fixture historique, 17 appareils occupaient 9 408 octets demandés vivants contre
13 624 pour 34 appareils ; les vecteurs de points occupaient 3 808 octets dans les deux
cas. Le tableau des appareils passait de 5 440 à 2 720 octets. Les descriptions JSON
faisaient 6 121 contre 7 634 octets. Ces différences sont celles des identifiants, libellés
et allocations de cette fixture, pas une économie universelle du Hub.

## Limites

- `Value` 24 o et `Sample` 32 o sont les conteneurs, pas la capacité inline du texte actuel.
  Les octets du texte et l’allocation de l’`Arc` s’ajoutent. `Sample` contient déjà `Value` :
  il ne faut pas additionner deux fois sa taille.
- Le banc ne mesure ni RSS, pages, comptabilité interne de l’allocateur, index du Hub,
  disponibilité, historique, lecteurs concurrents, broker ni commandes physiques.
- Les temps sont des médianes de cinq échantillons d’un même processus. Ils ne sont ni
  des percentiles de latence du produit, ni des mesures sur la machine cible d’une maison.
- Les layouts `repr(Rust)` ne sont pas une garantie ABI. Un prototype qui tient dans 24 o
  ne valide ni Serde, ni égalité, ni vérification UTF-8, ni durée de vie ou gestion d’une annexe.
- La taille synthétique de 4 096 octets ne prouve pas qu’un trajet d’entrée complet accepte
  un titre de cette longueur : ses limites et l’encodage JSON doivent être contrôlés séparément.
- Les 34 points restent indépendants. Grouper leurs descriptions ne fusionne pas leurs
  valeurs, unités ou dates et n’impose aucun paquet MQTT unique.
