# Banc de la haute disponibilité

Le banc des propositions haute disponibilité : la [v2](../../docs/proposition/haute-disponibilite-v2.md)
(+ [`.html`](../../docs/proposition/haute-disponibilite-v2.html)), une copie conforme sans journal sur
disque, et la [v1](../../docs/proposition/haute-disponibilite.md) qu'elle remplace. Il sert à trois
choses :

- **vérifier** les règles de la proposition : une copie de la mémoire par les sorties du hub est
  exacte, une copie par les entrées ne l'est pas ; le pont maison ne perd rien sous des coupures, même
  plus longues que sa file (l'état complet prend le relais), celui de `rumqttd` et celui de Mosquitto
  si ; deux renouveleurs d'un jeton qui tourne se font révoquer ; une base qui dérive son historique
  ne peut pas reprendre l'export ;
- **mesurer** le pont (délai, rattrapage, octets), le retour d'un secours après une absence (v1 : en
  rejouant un journal ; v2 : en recopiant l'état complet), sur un réseau local et sur un lien distant
  qui bat, et l'instantané d'arrêt ;
- **reproduire** les chiffres et les figures de la proposition.

Crate hors workspace (`[workspace]` vide), sans dépendance aux crates de Sarah Home. Il **inclut tels
quels**, par `#[path]`, trois fichiers du [banc mémoire](../memoire-banc/) : `outils.rs`, `maison.rs`
et `valeurs.rs`. La réplique tient donc exactement la mémoire des valeurs de l'étude mémoire v2.
Toutes ses dépendances (`rumqttd` 0.20, `rumqttc` 0.25, `tokio`, `sha2`, `crc32fast`,
`chacha20poly1305`, `lz4_flex`) sont dans le registre local.

## Contenu

| Fichier | Rôle |
| --- | --- |
| `src/flux.rs` | les sorties critiques (`val/`, `vu/`, et `wf/` : l'état d'une exécution) dans un seul rang `s` ; l'actif ; la réplique ; l'état complet (le construire, le relire) ; l'empreinte par blocs, exécutions comprises |
| `src/copie.rs` | une heure de la maison copiée par les sorties et par les entrées ; les attentes de la v2 revérifiées après une bascule |
| `src/relais.rs` | un relais TCP qui imite le lien : coupure, délai, gigue, débit |
| `src/pont.rs` | deux vrais courtiers `rumqttd` ; le pont maison à deux bouts : côté actif, la file en mémoire, les tronçons, « hors file », l'état complet en morceaux de 4 Kio, le battement ; côté secours, la remise en ordre, les demandes, la mémoire du secours et sa file ; une réplique abonnée au secours ; le pont de `rumqttd` pour comparer ; le réseau local, une coupure plus longue que la file, le lien distant, le retour après une absence (file sans limite, comme un journal, ou bornée) |
| `src/instantane.rs` | l'instantané d'arrêt : format, écriture avec `fsync`, relecture, reprise sur une horloge neuve |
| `src/jetons.rs` | un fournisseur qui suit la RFC 9700 § 4.14.2 ; quatre façons de tenir un compte à deux machines |
| `src/stockage.rs` | le rang d'export de D13 : une base dérivée contre une base qui reçoit les opérations |
| `scripts/linux.sh` | la série en Linux musl, chaque mesure dans trois conteneurs neufs de 2 CPU et 128 Mio |
| `scripts/mosquitto.sh` | un vrai pont Mosquitto 2 entre deux conteneurs, coupé 10 s : sa file en mémoire, par défaut, grande, puis courtier redémarré |
| `sorties/linux/` | la série publiée, citée par la v2 (avec `mosquitto.txt`) |
| `sorties/linux-v1/` | la première série, citée par la v1 (extraite de son annexe) |
| `rendu/` | v2 : `relief.py` (la boîte à outils des schémas en relief : cartes ombrées, blocs en perspective, flux animés) et `figures_v2.py` (lit `sorties/linux/`), `gabarit-v2.html` ; v1 : `figures.py` (lit `sorties/linux-v1/`), `gabarit.html` ; `build.py` (celui du banc mémoire, recopié, qui remplace aussi les images du Markdown par le schéma intégré) |
| `../../docs/proposition/haute-disponibilite-v2/` | les schémas de la v2 en SVG autonomes (légende comprise), que le Markdown affiche comme des images |
| `../../docs/proposition/haute-disponibilite/` | les 21 schémas de la v1 |

## Lancer

```
# Les tests (quelques secondes) :
cargo test --release --offline --manifest-path tools/haute-dispo-banc/Cargo.toml

# Une section, en natif :
cargo run --release --offline --manifest-path tools/haute-dispo-banc/Cargo.toml -- pont 1
# Un seul cas d'une section, avec les traces du pont et du relais :
HD_DIAG=1 HD_CAS=coupures cargo run --release --offline --manifest-path tools/haute-dispo-banc/Cargo.toml -- pont 1

# La série complète (Docker ou Colima ; environ 30 min), puis le pont Mosquitto (3 min) :
sh tools/haute-dispo-banc/scripts/linux.sh
sh tools/haute-dispo-banc/scripts/mosquitto.sh
# … ou seulement quelques sections :
sh tools/haute-dispo-banc/scripts/linux.sh pont resync

# v2 : les schémas (fragments dans rendu/fig2, SVG autonomes dans docs/proposition/haute-disponibilite-v2), puis la page :
python3 -I tools/haute-dispo-banc/rendu/figures_v2.py
python3 -I tools/haute-dispo-banc/rendu/build.py docs/proposition/haute-disponibilite-v2.md docs/proposition/haute-disponibilite-v2.html tools/haute-dispo-banc/rendu/gabarit-v2.html tools/haute-dispo-banc/rendu/fig2 haute-disponibilite-v2

# v1 : depuis sa propre série
python3 -I tools/haute-dispo-banc/rendu/figures.py tools/haute-dispo-banc/sorties/linux-v1
python3 -I tools/haute-dispo-banc/rendu/build.py docs/proposition/haute-disponibilite.md docs/proposition/haute-disponibilite.html tools/haute-dispo-banc/rendu/gabarit.html
```

Il faut Rust 1.88 ou plus récent (édition 2024) ; le dépôt valide avec 1.99.1. La page demande
`pandoc` et Python 3.

## Les sections

| Section | Ce qu'elle mesure |
| --- | --- |
| `copie` | une heure de la maison, pont coupé 30 s : la copie par les sorties et par les entrées ; appliquer une sortie ; l'empreinte |
| `pont` | réseau local, 200 messages par seconde : le régime ; deux coupures (5 s, 10 s), dans la file ; une coupure de 60 s, plus longue que la file (l'état complet) ; le pont de `rumqttd` sous les coupures courtes |
| `distant` | 40 ms ± 20 ms à 1 Mbit/s, coupé 3 s toutes les 15 s ; la cadence moyenne de la maison sur le même lien |
| `rattrapage` | v1 : le secours revient après une heure, une journée (réseau local, 10 Mbit/s, 1 Mbit/s) et rejoue tout (une file sans limite, comme un journal) |
| `resync` | v2 : les mêmes retours, avec la file bornée à 10 000 sorties ; au-delà, l'état complet |
| `instantane` | l'instantané d'arrêt à ×1 et ×10 : taille, figer, écrire avec `fsync`, relire et reprendre ; le JSON d'aujourd'hui pour comparer |
| `jetons` | deux renouveleurs ; un renouveleur avec copie (local, distant hors ligne 1 h par jour) ; une autorisation par machine ; sceller un jeton |
| `stockage` | une heure d'opérations de `moli.db` : base dérivée contre base qui reçoit ; reprise de l'export |

## Ce que les leçons de `rumqttd` ont coûté au banc

Elles sont dans la proposition (v2 § 6.9), et chacune a d'abord fait échouer le banc :

- l'ordre entre deux filtres n'est pas gardé : le rattrapeur attend 200 ms avant de demander un trou ;
- un abonné local en retard saute en avant sans le savoir : la réplique comble par la copie du
  journal ;
- un lien local mis en pause (`Unschedule`) ne reçoit plus rien tant qu'il ne répond pas `wake` ;
- une boucle `rumqttc` interrompue en pleine reconnexion laisse des connexions à moitié ouvertes,
  jusqu'à `max_connections` : la boucle vit dans sa propre tâche ;
- un trou ne se voit qu'à la sortie suivante : un secours revenu par l'état complet attendait une
  sortie qui ne venait pas ; d'où le battement (`repl/bout`, toutes les 500 ms).

## Règles

- La série publiée est celle de `sorties/linux/` (la v1 cite `sorties/linux-v1/`). Chaque mesure y est la médiane de trois essais dans
  trois conteneurs neufs, et le pire des trois pour un pire cas (`_max`). Relancer `scripts/linux.sh`
  écrit une nouvelle série à la place. Un document cite la série qui a produit ses chiffres ; on ne
  corrige jamais une sortie à la main.
- Le banc n'est pas Sarah Home : c'est la conception de la proposition, écrite pour être vérifiée.
- Les deux courtiers, le pont et le relais partagent un processus et 2 CPU : le délai local mesure
  le logiciel, pas un réseau. Les mesures sont faites sur un Apple M4 partagé (`sorties/linux/machine.txt`).
