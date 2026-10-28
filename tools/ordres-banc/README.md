# Banc des ordres des appareils

Le banc de l'analyse [« les ordres des appareils », v3](../../docs/proposition/ordres-appareils-v3.md)
(+ [`.html`](../../docs/proposition/ordres-appareils-v3.html)), **non validée** : où ranger en
mémoire ce que la maison peut demander à ses appareils, ce qu'elle est en train de leur demander, et
**qui a le droit de le demander**. La v3 ajoute les droits et corrige les 23 constats de la [seconde
revue Codex](../../docs/proposition/ordres-appareils-revue-droits.md), avec l'[inventaire des droits
d'aujourd'hui](../../docs/proposition/ordres-appareils-droits-aujourdhui.md) ; la
[v2](../../docs/proposition/ordres-appareils-v2.md) corrigeait les 25 de la [première
revue](../../docs/proposition/ordres-appareils-revue-codex.md). Les séries de la v1 et de la v2 sont
gardées dans `sorties/linux-v1/` et `sorties/linux-v2/` (leurs fautes injectées dans
`sorties/mutations-v1.txt` et `sorties/mutations-v2.txt`). Il sert à trois choses :

- **vérifier** la conception sur une soirée rejouée de bout en bout : un actif, sa copie de secours
  (qui accuse chaque rang), une copie reprise par l'état complet, un écran à cache, un écran de pièce
  qui suit le rang de sa pièce, et huit principaux (un écran, un second écran, un agent, l'assistant,
  un script, un invité, l'agent d'un gardien, le moteur) et six personnes. La soirée comprend une
  bascule (pont coupé juste avant), un redémarrage, un module mort, un retrait, un lot, une place
  réutilisée, deux redéclarations et des textes privés témoins ; des essais ciblés couvrent le reste,
  dont 26 pour les droits ;
- **mesurer** la mémoire et le temps de la proposition, à côté des structures d'aujourd'hui
  recopiées, à ×1, ×10 et ×100 : le catalogue des ordres, une journée de demandes, ce que charge un
  client, ce que lit l'assistant ;
- **reproduire** les chiffres et les schémas du document.

Crate hors workspace (`[workspace]` vide), sans dépendance aux crates de Sarah Home : seulement
`hashbrown` 0.17, déjà dans le `Cargo.lock` du dépôt. Il reprend tels quels, par inclusion de
fichiers (`#[path]`), la maison, la valeur d'aujourd'hui et les outils du
[banc mémoire](../memoire-banc/), et le registre à génération du [banc du nettoyage](../nettoyage-banc/).

## Contenu

| Fichier | Rôle |
| --- | --- |
| `src/catalogue.rs` | la forme d'un ordre (24 o, partagée ; elle cite les valeurs par leur **clé**), ses paramètres (20 o) et valeurs attendues (4 o), la fiche d'un ordre (40 o : sa forme, sa dernière commande, ses demandes vivantes, sa preuve), les dictionnaires, la validation commune (bornes, pas, dictionnaire, objet plat) ; 3 tests, dont un corpus fixe |
| `src/demandes.rs` | la demande (72 o, case réutilisée), son numéro (case 16 bits, époque 16 bits, génération 32 bits), les reçus (une fenêtre de 32 par principal, la plus haute séquence) ; 5 tests |
| `src/droits.rs` | v3 : le profil d'un compte (32 o), la personne (32 o), la session personnelle (16 o), le mandat d'un automatisme (16 o), la table rôles × classes ; `decider` (non, proposer, oui), `peut_approuver`, `peut_mandater` ; 3 tests, dont un corpus de 24 décisions |
| `src/hub.rs` | le hub : trois registres, les listes et compositions des appareils, les comptes, les personnes et leurs sessions, les droits (décidés à l'arrivée, à l'approbation et à la remise ; leur fin, une échéance), les retenues et leurs cartes à révision, l'approbateur habilité, l'accusé du secours avant un appui, la reprise sûre ou non et l'époque d'émission, les ordres de groupe, les textes privés, la preuve des ordres, les sorties critiques au rang `s` et au rang de leur pièce (`ord/` sans identité ; les droits, les sessions et l'identité des traces au pair), l'état complet (les comptes par leur nom), l'état d'un client |
| `src/module.rs` | le kit : une voie par appareil (un ordre en vol, des consignes fusionnées, jamais des appuis), l'issue bornée par le retour, le suivi, les lectures fraîches seules, l'annulation de ce qui n'est pas parti ; des appareils simulés |
| `src/moteur.rs` | le moteur : règles sur les valeurs et sur les ordres (dernière commande, en cours, origine, « la valeur suit-elle la commande ? » : oui, non, on ne sait pas), index par point et par ordre, baux ; v3 : le mandat (manifeste, responsable, adoption par le foyer, déclencheur ouvert), l'exécution appelée ou autonome, le bail ouvert au succès et qui ne rétablit que ce qu'il possède, l'empreinte du programme, les exécutions au pair (`wf/ex/`), 20 départs par minute ; son instantané |
| `src/maison_ordres.rs` | les ordres et les appareils virtuels de la maison du banc, par classe (hypothèse) |
| `src/aujourdhui.rs` | les structures d'aujourd'hui recopiées : `PointSpec`, `Device`, `Hub::prepare`, `CommandRequest`, `JournalEntry`, `ApprovalRequest`, `Hub::snapshot`, l'inventaire de l'assistant, `/api/snapshot` |
| `src/soiree.rs` | la soirée rejouée et ses 28 tests |
| `src/essais.rs` | 10 essais ciblés : reprise non sûre, secours décroché, groupes, compositions, plafonds, révocation, permis à l'approbation, lecture fraîche ; et `Mini`, le petit hub des essais |
| `src/essais_droits.rs` | v3 : 29 essais ciblés des droits, un par constat de la seconde revue (D1 à D9, M1 à M8, W1 à W6) et trois pour les choix du propriétaire (le groupe, l'urgence), avec un banc à moteur et à copie |
| `src/banc.rs` | les mesures : `catalogue`, `demandes`, `clients`, `soiree` ; le témoin d'aujourd'hui à la convention des pilotes, le corpus commun des validations, le temps processeur (`getrusage`) |
| `scripts/linux.sh` | la série en Linux musl, chaque mesure dans trois conteneurs neufs de 2 CPU et 128 Mio |
| `scripts/mutations.py` | 104 fautes injectées : les 53 de la v2 (B1 à B5, I1 à I14, I20, et 18 de la v1 ; six motifs suivent le code v3), 46 pour la seconde revue (D1 à D9, M1 à M8 sauf M4, W1 à W6) et 5 pour les choix du propriétaire (le groupe, l'urgence) ; chacune doit faire échouer au moins un test |
| `sorties/linux/` | la série v3, citée par le document ; `sorties/mutations.txt`, les fautes ; `*-v1`, `*-v2` : celles des versions précédentes |
| `rendu/figures_v3.py` | les schémas de la v3 (avec `tools/haute-dispo-banc/rendu/relief.py`), dont la table des droits lue dans la série ; `rendu/doc-v3.tpl.md` et `rendu/gabarit-v3.tpl.html`, le document et la couverture à jetons ; `rendu/rendre.py`, qui calcule chaque jeton depuis la série ; `figures_v2.py` (lit `sorties/linux-v2`), `figures.py` (lit `sorties/linux-v1`) et leurs gabarits : ceux des versions précédentes |

## Lancer

```
# Les 85 tests (dont 6 du registre repris du nettoyage ; `montrer`, ignoré, affiche la soirée) :
cargo test --release --offline --manifest-path tools/ordres-banc/Cargo.toml

# Une section, en natif :
cargo run --release --offline --manifest-path tools/ordres-banc/Cargo.toml -- soiree
cargo run --release --offline --manifest-path tools/ordres-banc/Cargo.toml -- demandes 10

# La série complète, comme la production (Docker ou Colima ; quelques minutes) :
sh tools/ordres-banc/scripts/linux.sh
sh tools/ordres-banc/scripts/linux.sh clients   # une section seulement

# Les fautes injectées (près d'une heure ; -u pour suivre l'avancement ; ne rien modifier dans src/ pendant ce temps) :
python3 -I -u tools/ordres-banc/scripts/mutations.py > tools/ordres-banc/sorties/mutations.txt

# Le document, la couverture, les schémas, puis la page :
R=tools/ordres-banc/rendu; S=tools/ordres-banc/sorties
python3 -I $R/rendre.py $R/doc-v3.tpl.md docs/proposition/ordres-appareils-v3.md $S/linux \
  --mutations=$S/mutations.txt --v2=$S/linux-v2
python3 -I $R/rendre.py $R/gabarit-v3.tpl.html $R/gabarit-v3.html $S/linux --mutations=$S/mutations.txt
python3 -I $R/figures_v3.py
python3 -I tools/haute-dispo-banc/rendu/build.py docs/proposition/ordres-appareils-v3.md \
  docs/proposition/ordres-appareils-v3.html $R/gabarit-v3.html $R/fig3 ordres-appareils-v3
```

## Pièges

- Ce qui quitte une machine est en heure murale : la première version de la copie gardait
  l'échéance monotone de l'actif. Après une bascule, ses suivis et ses retenues duraient une
  demi-heure de trop, et aucun test ne le voyait tant qu'il ne vérifiait pas les heures.
- Une voie n'existe que tant qu'elle porte un ordre (V7 § 20.2) : le premier kit les gardait toutes
  et les parcourait à chaque événement. La journée ×10 coûtait 64 s de processeur, contre 88 ms
  une fois corrigé.
- Un test qui vérifie qu'un écran « a demandé » une forme ne prouve pas qu'il s'en sert : la
  mutation « l'écran garde l'ancienne structure » passait, jusqu'à ce que le test lui fasse valider
  une valeur.
- À ×100, les structures d'aujourd'hui ne tiennent plus dans 128 Mio avec le reste : les sections
  `catalogue` et `clients` ne les construisent qu'à ×1 et ×10.
- `gen` est un mot réservé en édition 2024.
- v2 — les reçus refusent une séquence plus ancienne que la fenêtre de son principal : une soirée
  dont un même écran numérote ses demandes dans le désordre se fait refuser ses propres demandes. Un
  principal numérote dans l'ordre ; les scènes de la v2 ont leur propre écran.
- v2 — une copie qui a reçu de l'ancien actif une sortie que le secours n'a jamais eue voit, après la
  bascule, un rang en double : chaque sortie porte l'époque, et une époque nouvelle fait se recharger
  copies et écrans par l'état complet.
- v2 — un écran qui se recharge au milieu d'un lot de sorties reçoit ensuite des sorties déjà
  comprises dans son état : ce n'est pas un trou ; il les ignore (rang ≤ celui chargé).
- v2 — mesurer 50 retenues avec le compteur d'allocations compte aussi le tampon des sorties, qui
  part sur MQTT : le rendre avant la mesure.
- v2 — `getrusage` : sous macOS, `tv_usec` est un `i32` (Linux musl : un `i64`).
- v3 — une défense en double (les pièces du compte et celles de la personne) masque la faute qui en
  retire une : chaque défense a besoin d'un essai qui l'isole.
- v3 — le banc des essais doit remettre au kit ce que le moteur demande à ses échéances (le
  rétablissement d'un bail), comme le fait la soirée : sinon le bail « se rétablit » sans rien envoyer.
- v3 — une demande externe dit l'époque de la session sur laquelle elle a été émise : `Mini::req` et
  la soirée la donnent ; une requête neuve envoyée à un hub redémarré doit porter **son** époque.
- v3 — la faute « pas de quota » ferait tourner deux automatismes sans fin : le banc des essais borne
  le nombre de sorties, et le plafond des demandes arrête la chaîne.
