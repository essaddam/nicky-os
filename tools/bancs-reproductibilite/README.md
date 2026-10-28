# Campagnes séparées et reproductibles

Les expériences restent indépendantes :

| Dossier | Question testée | Charge / portée |
|---|---|---|
| `../catalogue-banc/` | lignes96, lignes64+annexes, colonnes | formes anonymisées des deux exports HA |
| `../alias-banc/` | miroir, alias96, ID, cache, pointeur | source96 unique et vues virtuelles synthétiques |
| `../memoire-orientations-banc/` | layouts et contre-exemples | tests de modèle, pas un benchmark de latence |
| `../organisation-banc/` | listes, liens plats et CSR | organisation synthétique, compteurs agrégés HA |
| `../mqtt-canonicalisation-banc/` | lookup allouant, emprunté, cache | scalaires déjà décodés, aucune connexion MQTT |

Chaque dossier contient son `main.rs`, un README de contrat, un `executer.py` et ses
sorties JSON/tests. Rejouer depuis la racine :

```sh
python3 tools/organisation-banc/executer.py
python3 tools/mqtt-canonicalisation-banc/executer.py
python3 tools/memoire-orientations-banc/executer.py
```

Les scripts compilent des sources figées pendant la campagne. Les mesures de performance
utilisent trois conteneurs neufs par cas et sept passages internes après chauffe ;
les tests de modèle n'inventent pas des mesures de latence.

Vérifier les preuves déjà produites, sans relancer les campagnes, compiler ou accéder
au réseau (Python, bibliothèque standard uniquement) :

```sh
python3 tools/bancs-reproductibilite/verifier.py
```

Le vérificateur contrôle les empreintes des cinq sources, les 72 essais/24 médianes
d’organisation, les 90 essais/30 médianes MQTT, leurs dimensions, la parité des sommes
de contrôle et les décompositions du tas. Il vérifie aussi les trois essais de chaque
médiane, les deux modes MQTT, les 18 tests et exemples de tailles/protocoles V4, le
corpus de cinq conversations (43 messages, dont 23 messages utilisateur), les 32 affirmations
uniques, le besoin MQTT-CANON-01 et les liens/ancres locaux du rapport HTML V4.
Si l’index des campagnes existe, ses cinq archives doivent correspondre aux sorties
courantes ; une campagne en attente d’archivage échoue explicitement à ce contrôle.
Les compteurs nuls du mode `timing_plain` sont traités comme non mesurés : le tas
persistant est validé exclusivement à partir du mode `counted`.

Avant de faire évoluer une source ou remplacer des résultats, figer la campagne :

```sh
python3 tools/bancs-reproductibilite/archiver.py
```

Les archives sont dans `sorties/campagnes/<date>-<hash-source>/` de chaque expérience.
Elles conservent la source réellement compilée, les scripts, commandes, README,
versions Rust/Alpine/cible, résultats et tests. L'archiveur refuse d'écraser une campagne
existante avec des données différentes. Les fixtures TSV publiques et synthétiques
sont également figées dans `charges/`, avec empreinte du manifeste.

Une ancienne source `source-mesuree.rs` peut être recompilée directement avec Rust,
les options et l'image indiquées par son `campagne.json`/`resultats.json`; pour les
bancs TSV, fournir les fichiers de l'archive de charge correspondante. Les scripts
archivés gardent leurs hypothèses de chemins du dossier d'expérience d'origine : les
replacer dans une copie de ce dossier pour rejouer automatiquement une ancienne version.
Une empreinte d'image locale ne garantit pas que cette image existera partout ; le
fichier `toolchain.txt` documente la cible et les versions à reconstruire si nécessaire.

Les performances ne sont pas des constantes universelles. Ne pas additionner les
mémoires totales de ces bancs : leurs catalogues de référence se recouvrent. Les coûts
de bus, JSON, TLS, verrouillage, journal, droits, géométrie, vrais abonnés et Moli complet
ne sont pas couverts par tous les prototypes. Aucun export privé ou chat brut n'est archivé.
