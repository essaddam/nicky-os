# Banc de l'étude mémoire

Le banc et le moteur de référence de l'[étude mémoire v2](../../docs/proposition/memoire-v2.md)
(+ [`.html`](../../docs/proposition/memoire-v2.html)). La [v1](../../docs/proposition/memoire.md) est
gardée comme historique. Ils servent à trois choses :

- **mesurer** la mémoire des valeurs, des workflows et du temps, l'expiration, le calcul des
  conditions et la précision des réveils, dans la forme d'aujourd'hui (recopiée à l'identique du code)
  et dans la forme proposée ;
- **vérifier de bout en bout** le moteur de workflows et le moteur du temps proposés, par une soirée
  de la maison rejouée seconde par seconde et par 29 tests ;
- **reproduire** les chiffres et les figures de l'étude.

Crate hors workspace (`[workspace]` vide), sans dépendance aux crates de Sarah Home : `serde`,
`serde_json`, `hashbrown` 0.17, `tokio` et `jiff` (fuseaux embarqués, comme le dépôt) sont déjà dans
le `Cargo.lock` du dépôt.

## Contenu

| Fichier | Rôle |
| --- | --- |
| `src/maison.rs` | la maison du banc : la forme des deux maisons (496 appareils, 2 264 points), répétée ×N |
| `src/valeurs.rs` | la mémoire des valeurs : `States` du hub et miroir du moteur d'aujourd'hui ; annuaire, colonnes et échéances proposés, aux deux horloges |
| `src/echeances.rs` | trouver ce qui a expiré : quatre méthodes jouées sur une heure simulée de la maison |
| `src/workflows.rs` | le modèle des automatismes, recopié de `moli-automation`, plus les ajouts proposés et marqués (`WhenRules`, `ReadValue`, `Rule::Cause`, `WhenCalendar`) ; le balayage d'aujourd'hui ; le programme compilé |
| `src/conditions.rs` | cinq façons de calculer les conditions, et les « attendre que » d'aujourd'hui |
| `src/decisions.rs` | ce que les décisions du 23 octobre ajoutent : exécutions de 64 o, réseau des déclencheurs composés |
| `src/calendrier.rs` | **le moteur du temps** : fiches calendaires de 24 o, changements d'heure, rattrapage, sauts d'horloge, heure sûre, agendas ; 19 tests |
| `src/minuteries.rs` | les minuteries du hub, aux deux horloges ; 4 tests |
| `src/precision.rs` | la précision : lire l'heure, le retard d'un réveil (fil, Tokio, au repos, sous charge), le balayage d'aujourd'hui, le moteur du temps sur une journée, les agendas |
| `src/moteur.rs` | **le moteur de référence complet** : programme, exécutions à génération, attentes revérifiées, réseau, fiches horaires, échéancier aux deux horloges, recompilation, redémarrage |
| `src/scenario.rs` | **la soirée rejouée** et ses six tests |
| `src/outils.rs`, `src/main.rs` | compteur d'octets, chronomètre, horloges, sorties `RESULTAT\|…` |
| `scripts/linux.sh` | rejoue toute la série en Linux musl, chaque mesure dans trois conteneurs neufs de 2 CPU et 128 Mio |
| `scripts/mediane.py` | fusionne les trois essais d'une mesure : la médiane, et le pire des trois pour un pire cas (`_max`) |
| `scripts/mutations.py` | vérifie que les tests savent échouer : 17 fautes injectées dans le moteur et le moteur du temps |
| `sorties/linux/` | la série publiée, citée par la v2 ; `sorties/mutations.txt` |
| `sorties/v1/linux/` | la série de la v1, gardée telle quelle |
| `rendu/` | `figures.py` (lit `sorties/linux/`), `build.py`, `gabarit.html` (v1) et `gabarit-v2.html` : les pages HTML de l'étude |

## Lancer

```
# Les 29 tests (quelques secondes) :
cargo test --release --offline --manifest-path tools/memoire-banc/Cargo.toml

# Une section du banc, en natif (l'échelle : ×N la maison ; pour « scenario », N copies) :
cargo run --release --offline --manifest-path tools/memoire-banc/Cargo.toml -- scenario 1
cargo run --release --offline --manifest-path tools/memoire-banc/Cargo.toml -- precision 1

# La série complète, comme la production (Docker ou Colima ; environ 20 min) :
sh tools/memoire-banc/scripts/linux.sh
# … ou seulement quelques sections, à leurs échelles habituelles :
sh tools/memoire-banc/scripts/linux.sh precision scenario

# Les mutations : chaque faute injectée doit faire échouer au moins un test :
python3 -I tools/memoire-banc/scripts/mutations.py

# Les figures, puis la page de la v2 :
python3 -I tools/memoire-banc/rendu/figures.py
python3 -I tools/memoire-banc/rendu/build.py docs/proposition/memoire-v2.md docs/proposition/memoire-v2.html tools/memoire-banc/rendu/gabarit-v2.html
```

Il faut Rust 1.88 ou plus récent (édition 2024) ; le dépôt valide avec 1.99.1
(`RUSTUP_TOOLCHAIN=1.99.1`). La page demande en plus `pandoc` et Python 3 ; `linux.sh` demande
Python 3 pour fusionner les essais.

La page de la v1 n'est plus reconstruite : ses figures viennent de la série v1. Pour les revoir,
`python3 -I tools/memoire-banc/rendu/figures.py tools/memoire-banc/sorties/v1/linux <dossier>`.

## Les sections du banc

| Section | Ce qu'elle mesure |
| --- | --- |
| `valeurs` | lire une valeur (par nom aujourd'hui, par numéro), écrire une nouvelle, copier tout l'état ; la mémoire des deux formes |
| `echeances` | une heure de la maison : calcul au passage, colonne d'échéances, réveil au plus proche, file de priorité ; accord des quatre méthodes |
| `passage` | un passage sur une colonne d'échéances, de 2 264 à 2,26 millions de cases |
| `workflows` | la mémoire des définitions d'aujourd'hui et du programme compilé ; un changement de valeur côté moteur ; une exécution ; les attentes de 24 o |
| `conditions` | les cinq façons de calculer les conditions ; une durée armée (tâche Tokio contre attente) ; « attendre que » d'aujourd'hui |
| `decisions` | l'exécution de 64 o ; le réseau des déclencheurs composés et sa reconstruction |
| `scenario` | la soirée en N copies : processeur du moteur, mémoire (fiches horaires comprises), et l'identité de chaque copie avec la première |
| `precision` | lire l'heure ; le retard d'un réveil de 1 ms à 1 s ; un tour du balayage d'aujourd'hui ; une journée du moteur du temps ; un agenda d'un jour, d'une semaine, d'un an |

## Ce que le scénario vérifie

Une soirée de 18 h à 23 h 15, huit automatismes par copie de la maison, dont « Coucher de la maison »
(23 nœuds, deux entrées, un « si » de quatre règles, une valeur lue dans le registre, des attentes,
trois vérifications de nouvelle valeur dont une qui échoue, une fin selon la cause), une ronde à
22:30:00, deux tâches qui tombent pendant la panne, et des rappels 10 min avant chaque rendez-vous de
l'agenda. Pendant la soirée : un message qui ferait basculer une condition en deux temps, une issue
tardive, un bouton pressé pendant l'exécution, une panne de 5 min et le redémarrage (l'horloge
monotone repart de zéro), la modification d'un automatisme pendant qu'il tourne, un capteur qui
devient muet puis revient, un thermomètre dont la valeur devient trop vieille, un rendez-vous ajouté
au dernier moment, deux rendez-vous à la même heure. Le détail et les résultats sont au § 14 de
l'étude.

| Test | Ce qu'il exige |
| --- | --- |
| `les_ordres_de_la_soiree` | les 14 ordres, à la seconde |
| `les_avis_et_les_causes` | les 14 avis, avec la cause et le registre de chaque exécution, horaires comprises |
| `le_chemin_de_a_avance_seulement` | le chemin de A, nœud par nœud, sans retour en arrière |
| `les_etats_releves` | les fiches d'exécution et la vérité de la condition avant et après la panne, après la modification, à 23:00 (valeur trop vieille), à la fin |
| `le_journal_dit_pourquoi` | les raisons au journal : attente abandonnée, issue tardive écartée, relance, interruption, occurrence manquée, arrêt, valeur trop vieille, capteur muet |
| `cent_maisons_identiques` | cent copies rejouées ensemble se comportent exactement comme la première |

Le moteur du temps a 19 tests (un par règle : changements d'heure, plages, soleil, rattrapage,
sauts d'horloge, heure sûre, valeurs qui portent une heure, seconde et milliseconde, agendas), les
minuteries 4 ; la liste est au § 14.3 de l'étude.

## Règles

- La série publiée est celle de `sorties/linux/` : chaque mesure y est la médiane de trois essais
  dans trois conteneurs neufs, et un pire cas (`_max`), le pire des trois. Relancer
  `scripts/linux.sh` écrit une nouvelle série à la place : les temps bougent d'une série à l'autre,
  surtout si la machine sert à autre chose pendant la mesure. Un document cite la série qui a produit
  ses chiffres ; on ne corrige jamais une sortie à la main.
- Le moteur de référence n'est pas le moteur de Sarah Home : c'est la conception de l'étude, écrite
  pour être vérifiée. Ses règles sont celles que le portage devra tenir.
- Les mesures sont faites sur un Apple M4 (`sorties/linux/machine.txt`). Un Pi Zero 2 W sera
  plusieurs fois plus lent : les rapports tiennent, les valeurs absolues sont à refaire sur la carte.
