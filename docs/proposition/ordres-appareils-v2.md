# Les ordres des appareils, v2 — Sarah Home : ce que chacun en connaît, et où c'est rangé

**Version remplacée.** Cette v2 est remplacée par la [v3](ordres-appareils-v3.md), du même jour,
qui y ajoute les droits sur les ordres et corrige les vingt-trois constats de la [seconde revue
Codex](ordres-appareils-revue-droits.md) : une connexion n'est pas une personne (la tablette, la
voix, l'agent), des droits par classe d'ordre, un approbateur habilité, les droits revérifiés jusqu'à
la remise au module, `ord/` sans identité, désarmer jamais par les ordres, et ce qui manquait à la
mémoire et au moteur des automatismes. Elle reste pour l'historique ; ses chiffres viennent de la
série v2 du banc, gardée dans `tools/ordres-banc/sorties/linux-v2/`, et ses fautes injectées dans
`tools/ordres-banc/sorties/mutations-v2.txt`. Le banc, lui, a été repris pour la v3 : les extraits
de code ci-dessous sont ceux de la v2.

Date : 24 octobre 2026. Statut : **analyse v2, non validée**. Elle remplace la
[v1](ordres-appareils.md), du même jour, gardée pour l'historique, et corrige les 25 constats de la
[revue Codex](ordres-appareils-revue-codex.md) (5 bloquants, 20 importants). Aucune modification du
code ; aucune des propositions précédentes n'est modifiée. Base analysée : `main` au commit
`a835893`. S'appuie sur l'[étude mémoire v2](memoire-v2.md), la [haute disponibilité
v2](haute-disponibilite-v2.md), le [nettoyage des équipements](nettoyage-equipements.md) et
l'[analyse V7](isolation-reactivite-runtime-v7.md). Le § 21 dit ce qu'elle leur changerait si tu la
valides ; le § 22 garde tes quatre choix du 24 octobre, qui tiennent.

Ce que tu as demandé, dans l'ordre où tu l'as dit :

1. **Il manque un truc** : on ne voit pas où sont rangées en mémoire les actions sur les appareils.
   Les intégrer à la mémoire du hub et des autres, en restant cohérent avec le reste.
2. **« Il faut qu'ils connaissent l'appareil, l'ordre et la structure de l'ordre. »**
3. **Les ordres et les valeurs sont deux choses différentes** : les séparer.
4. Avec les actions en mémoire, **les workflows pourront mieux les interroger**.
5. **Des appareils physiques, et des appareils virtuels** qui assemblent des informations des
   physiques : une valeur ou un ordre peut appartenir à l'un, à l'autre ou aux deux. L'avoir en
   mémoire, puisque les interfaces et les sites en auront besoin ; et **revérifier ce que tout cela
   consomme**, par rapport aux assistants, aux autres composants et aux anciennes versions.
6. **Un passage qui démontre l'intérêt**, en montrant la différence de mémoire.
7. **Les clients se rechargeraient peut-être mieux directement par MQTT** que par un module dédié,
   maintenant que tout est en mémoire : les équipements, les ordres et les valeurs.
8. **Revoir l'idée avec un agent Codex** (`gpt-6.1-sol`, effort maximal), puis **une v2** qui
   corrige ce qu'il a trouvé.

Méthode, et techniques employées, nommées au fil du texte :

- **la revue adversariale** de Codex : 25 constats, chacun avec sa preuve ; la v2 les reprend un à
  un, et vérifie chacun dans le code avant de le corriger (§ 3) ;
- **« Comment pourrait-on… »** : la demande reformulée en questions ouvertes, et ce qui prouvera
  chaque réponse (§ 4) ;
- **premiers principes** : ce qu'est un ordre, et ce qu'il faut savoir de lui, du moment où il est
  demandé jusqu'à ce que la maison l'oublie (§ 9) ;
- **inventaire** : tout ce qui garde quelque chose d'un ordre, dans la conception et dans le code
  d'aujourd'hui, fichier par fichier (§ 5) ;
- **analogies**, **SCAMPER** et **brainstorming inversé** (« comment rater la mémoire des
  ordres ? »), complété par ce que la revue a trouvé (§ 7) ;
- **analyse morphologique**, puis **sélection à critères visibles** (§ 8) ;
- **un banc** : le catalogue, les demandes, un hub, un kit de modules, un moteur et des écrans
  réduits, mesurés à côté des structures d'aujourd'hui recopiées, à la convention des pilotes réels.
  Linux musl, 2 CPU et 128 Mio, trois essais par mesure. Rangé dans `tools/ordres-banc/` (annexe
  A) ;
- **une vérification de bout en bout** : une soirée de la maison, avec une bascule (le pont vers le
  secours coupé juste avant), un redémarrage, un module mort, un retrait, un lot, deux
  redéclarations, un invité, un permis qui finit et des textes privés témoins ; des essais ciblés
  pour le reste ; 52 tests, et 53 fautes injectées, au moins une par constat de conception de la
  revue (§ 20).

Vocabulaire :

- une **valeur** dit ce que l'appareil est (« la lampe est allumée ») ; c'est un point de la mémoire
  v2 ;
- un **ordre** dit ce que la maison peut lui demander (« allume-toi », « ouvre ») ; il a son numéro,
  à part de ceux des valeurs ;
- la **forme** d'un ordre est sa structure : ses paramètres, leurs bornes, son effet, son retour,
  son suivi. Elle est partagée par tous les ordres qui ont la même structure ;
- une **demande** est un ordre que quelqu'un vient de demander, avec ses paramètres, jusqu'à son
  **issue** (confirmé, exécuté, accepté, envoyé, incertain, non envoyé, échec, refusé, retenu : les
  neuf de la V7) ;
- le **retour** est ce que l'appareil sait dire après un ordre (une valeur qui revient, le pont qui
  dit « émis », un accusé, rien) ; l'**effet** dit si l'ordre est une **consigne** (la dernière
  gagne, la rejouer ne change rien) ou un **appui** (chacun compte, le rejouer le refait) ;
- le **suivi** est la fenêtre, après l'issue, pendant laquelle la suite peut encore arriver ;
- un **principal** est celui qui demande : un compte (un écran, l'appli, un agent, un script), le
  moteur, le hub. Le serveur le déduit de la connexion ; un client ne le déclare jamais. Son
  **permis** dit jusqu'à quand, et dans quelles pièces, il commande ;
- une **retenue** est une demande que la garde garde pour un humain ; sa **carte** est ce que
  l'humain voit et approuve ;
- un **reçu** retient la clé d'une demande (son principal et sa séquence) et son issue, pour qu'une
  demande renvoyée ne parte pas deux fois ;
- un **bail** (« pendant 2 min ») ouvre par un ordre et rétablit la valeur d'avant par un autre ;
- la **dernière commande** d'un ordre : la dernière demande admise pour lui, ce qu'elle demandait,
  quand, qui, et son issue ; sa **preuve** dit si la valeur suit cette commande ;
- une **reprise** est une bascule ou un redémarrage ; elle est **sûre** quand aucun appui n'a pu
  partir sans que la mémoire qui reprend le sache ;
- un appareil **physique** tient ses valeurs et ses ordres de son module ; un appareil **virtuel**
  assemble des valeurs et des ordres d'autres appareils (sa **composition**), et peut en avoir à lui
  (une moyenne, un ordre de groupe) ;
- un **client** est ce qui montre la maison : un écran, l'appli, un site, une tablette au mur.

## 1. Le verdict en bref

- **L'idée tient** ; Codex le dit aussi : séparer les valeurs, les ordres et les demandes en cours
  clarifie le modèle. Mais la v1 n'était **pas validable en l'état** : certaines garanties
  d'idempotence, de sécurité et de reprise étaient fausses, et plusieurs chiffres flattaient la
  proposition. La revue a trouvé **5 constats bloquants et 20 importants**. Je les ai tous vérifiés
  dans le code ; ils étaient tous fondés. La v2 les corrige tous : chaque correction de conception a
  son test et au moins une faute injectée ; les cinq corrections de mesure sont refaites dans le
  banc (§ 3).
- **Les cinq bloquants, corrigés** :
  - **qui demande** est fixé par le serveur, d'après la connexion, jamais par le contenu ; chaque
    cible est autorisée, filles d'un groupe comprises (§ 11) ;
  - **un appui n'est jamais refait par une reprise** : il attend l'accusé du secours avant de
    partir ; une reprise non sûre refuse une nouvelle tentative ambiguë (§ 12.6, § 17) ;
  - **les reçus** gardent la plus haute séquence de chaque principal et refusent ce qui est plus
    ancien que leur fenêtre, ou en conflit (§ 12.5) ;
  - **le numéro d'une demande** porte l'époque de l'actif et une génération de 32 bits : il n'est
    jamais redonné, ni par l'actif, ni par celui qui lui succède (§ 12.1) ;
  - **un texte privé** ne voyage que par le canal privé, vers le module de l'appareil et le pair ;
    jamais sur `cmd/`, `ord/`, au journal ni dans l'état d'un client (§ 12.7).
- **Les ordres et les valeurs, séparés** (§ 9). Deux registres, chacun avec ses numéros, ses noms et
  ses fiches. Le seul lien est une référence, portée par la forme de l'ordre, **par la clé de la
  valeur** : une redéclaration qui ajoute, retire ou réordonne des valeurs ne déplace rien (la v1
  citait un rang).
- **L'appareil, l'ordre et la structure de l'ordre, en trois fiches** (§ 10), dont les tailles sont
  maintenant vérifiées à la compilation :
  - l'appareil, 16 octets : sa liste (les numéros de ses valeurs, puis de ses ordres), sa
    composition, sa pièce ;
  - l'ordre, 40 octets : sa forme, sa dernière commande, ses demandes vivantes, et sa preuve ;
  - la forme, 24 octets, plus 20 par paramètre et 4 par valeur attendue, partagée.

  Dans la maison ×1 du banc, 802 ordres partagent 16 formes ; à ×100, 82 592 ordres en partagent
  toujours 16.
- **Les demandes ont leur fiche de 72 octets, dans une case réutilisée** (§ 12). Le permis,
  l'échéance et la forme sont revérifiés à chaque frontière : à l'arrivée, à l'approbation, à
  l'envoi. Une demande garde la forme sous laquelle elle a été acceptée.
- **Les workflows interrogent les ordres** (§ 15) : la dernière commande en 0,37 ns ; « la valeur
  suit-elle la dernière commande ? » en 1,3 ns, avec trois réponses : **oui**, **non**, ou **on ne
  sait pas**. La v1 prétendait dire la cause ; ce n'est qu'une compatibilité, et le document dit
  maintenant où elle s'arrête.
- **Les appareils virtuels composent sans posséder** (§ 14). Un ordre de groupe passe par le même
  chemin, approuvé ou non ; la mère compte ses filles par niveau de preuve (« 3 volets exécutés » ne
  font pas « 3 confirmés ») ; sa progression passe la bascule ; chaque cible dit de quel ordre de
  groupe elle dépend.
- **Les clients se chargent par MQTT**, ton choix (§ 16). À contenu égal à celui de `/api/snapshot`
  (types, unités, disponibilité, libellés, retenues), un écran charge 113 Ko au lieu de 593 Ko, et
  un écran de deux pièces 20,5 Ko. Chaque sortie porte aussi le rang de sa pièce : un écran filtré
  voit ses trous, ce que le seul rang `s` ne permettait pas.
- **Ce que tout cela consomme reste petit** (§ 19) : la mémoire des ordres (registre, catalogue,
  demandes, reçus, journal, retenues, compositions) pèse 130 Kio à ×1. Comparée honnêtement, à
  information égale et au même corpus :
  - la description de la maison passe de 775 à 357 Kio ;
  - le journal en mémoire, de 138 à 23,4 Kio ;
  - valider une demande, de 105 ns à 39 ns par le nom, 5,7 ns par le numéro ;
  - l'assistant ne recopie plus 317 Kio à chaque requête.
- **Vérifié de bout en bout** (§ 20) : la même mémoire des ordres sur l'actif, sa copie et une copie
  reprise par l'état complet, à chacun des 3 relevés ; 52 tests ; 53 fautes injectées.
- **Tes choix du 24 octobre tiennent** (§ 22) : les clients par MQTT direct ; une retenue gardée au
  redémarrage ; les traces sur `ord/` ; un ordre de groupe réparti par le hub, ou par un module
  quand il faut une stratégie. La revue les jugeait conservables ; la v2 leur ajoute ce qu'il leur
  manquait (un permis vivant, un rang par pièce, des filles autorisées une à une).

![Ce que la revue a trouvé, et ce que la v2 en fait](ordres-appareils-v2/revue.svg)

## 2. L'intérêt, démontré

Tu as demandé un passage qui montre pourquoi ranger en mémoire les ordres, et les appareils
physiques et virtuels qui les portent avec les valeurs, et ce que cela change à la mémoire. Le
voici, en deux tableaux : ce que cela permet, puis ce que cela coûte. Chaque ligne a sa preuve au
banc.

### 2.1 Ce que cela permet

| Ce que la maison sait faire | Aujourd'hui | Avec la mémoire des ordres | La preuve |
|---|---|---|---|
| Dire ce qu'un volet RTS a reçu en dernier (il n'a aucune valeur) | rien sur l'appareil ; le journal global, 60 lignes à l'écran | la fiche de l'ordre : « ouvrir, exécuté par la box, 18:20, par l'assistant », la même sur l'actif, le secours et chaque écran | test `la_derniere_commande_se_lit_partout_pareil` |
| Dire si une valeur suit la dernière commande, ou un geste à la main | impossible : rien ne relie un ordre à la valeur qui suit (V7 § 5.8) | oui, non, ou on ne sait pas, en 1,3 ns : « éteindre ce que la maison a allumé, pas ce qu'un humain a rallumé » | test `les_workflows_interrogent_les_ordres` |
| Réagir à un ordre, pas à une valeur | impossible | un déclencheur sur l'ordre : « quand on ferme le volet RTS, fermer celui de la chambre » | idem |
| Refuser une demande hors de sa structure, avant de l'envoyer | le hub, en 105 ns ; l'écran lit le `Kind` du point, sans effet ni retour | le hub, l'écran et l'agent, avec la même forme et les mêmes dictionnaires : 39 ns par le nom, 5,7 ns par le numéro, sur le même corpus et avec les mêmes décisions | `catalogue`, corpus commun ; test du corpus fixe |
| Ne laisser un invité commander que ses pièces | une vérification par requête REST (`rest.rs:119`) | le principal est celui de la connexion ; chaque cible est autorisée, groupes compris | test `un_invite_ne_commande_que_ses_pieces` |
| Une issue tardive ne touche jamais la mauvaise demande | les canaux le garantissent ; une exécution n'a pas de numéro de demande | la génération et l'époque du numéro | test `chaque_issue_tombe_sur_sa_demande`, et ses fautes injectées |
| Une retenue passe un redémarrage et une bascule, sans garder une autorisation échue | perdue au redémarrage, sans une ligne de journal | gardée ; à l'approbation, l'échéance, le permis et la forme sont revérifiés | tests `une_retenue_n_approuve_que_ce_qu_elle_montre`, `une_retenue_ne_garde_pas_une_autorisation_echue`, `une_carte_ne_s_approuve_que_sous_sa_forme` |
| Un appui renvoyé ne part qu'une fois, même après une bascule | reçus par fichier, sur une seule machine | l'appui attend l'accusé du secours ; les reçus sont dans la trace ; une reprise non sûre refuse la nouvelle tentative | tests `un_appui_n_est_ni_perdu_ni_double_par_la_bascule`, `un_appui_retente_ne_part_qu_une_fois`, essais de la reprise |
| Un ordre en vol à la bascule n'est jamais renvoyé, et sa vérité est dite | — (aucun secours) | « incertain », puis confirmé par le nouvel actif, par une lecture faite après la reprise | tests `aucun_ordre_n_est_rejoue`, `une_lecture_d_avant_la_reprise_ne_confirme_pas` |
| Un appareil virtuel montre et commande sans posséder | `members` : des appareils seulement ; un luminaire dit « fait » dès qu'une ampoule obéit (`moli-lights/src/lib.rs:498`) | une composition ; « 3 sur 4 », par niveau de preuve ; la garde lit les pièces des membres | tests `un_appareil_virtuel_compose_sans_posseder`, `un_groupe_approuve_se_repartit`, `un_groupe_coupe_par_la_bascule_garde_son_compte` |
| Un texte privé reste privé | il voyage avec la commande | il ne voyage que par le canal privé, et s'efface à la conclusion | test `un_texte_prive_ne_sort_que_par_le_canal_prive` |
| Un écran se recharge sans tout relire, et voit ce qu'il a perdu | `/api/snapshot` : toute la maison, en JSON | son état, filtré, par MQTT ; puis le flux de ses pièces, au rang de chacune | test `un_ecran_de_piece_voit_ses_trous` ; section `clients` |
| Un appareil retiré ne garde rien, sa place non plus | son étiquette et son état restent (nettoyage § 4.3) | la fiche de l'ordre est remise à zéro au lot ; un vieux numéro ne commande jamais l'appareil neuf | test `un_appareil_retire_n_est_plus_commande_et_sa_place_ne_garde_rien` |

### 2.2 Ce que cela coûte, et ce que cela remplace

Chaque chiffre est mesuré à information égale : le témoin d'aujourd'hui suit la convention des
pilotes réels (§ 19.1), et chaque taille vient du compteur d'allocations ou de `size_of`.

![La mémoire, avant et après](ordres-appareils-v2/memoire.svg)

| Ce qui est gardé, maison ×1 (×10) | Aujourd'hui | Avec la mémoire des ordres |
|---|---|---|
| La description des appareils, de leurs valeurs et de leurs ordres | 775 Kio (7,6 Mio) : `Device` et `PointSpec` | 357 Kio (3,3 Mio) : annuaires, `Def` de la mémoire v2, registre des ordres, catalogue, listes et compositions |
| … dont la structure et les fiches des ordres | dans les `PointSpec`, incomplète : ni effet, ni retour, ni suivi | 32,0 Kio : 16 formes et une fiche de 40 o par ordre |
| Une demande en vol | 590 o au mieux, en une dizaine d'allocations, sans compter les canaux ni les tâches | 72 o, dans une case ; au pic de la journée, 39 demandes en cours (64 à ×10) |
| Le journal en mémoire, 500 lignes | 138 Kio de textes | 23,4 Kio de lignes de 48 o ; les noms se résolvent à l'affichage |
| 50 retenues (le plafond) | 21,6 Kio | 11,2 Kio, mesuré : les demandes, leurs cartes, le reçu |
| Les reçus | trois fichiers métier (boutons, compteurs, minuteries), jusqu'à 1 Mio chacun | 10,2 Kio dans la journée du banc ; au plus 65,0 Kio pour 64 principaux |
| Les compositions des appareils virtuels | `members`, des noms | 2,7 Kio : 340 entrées de 8 o |
| Ce que l'assistant copie à chaque requête | 317 Kio (`Hub::snapshot` : les états de chaque appareil ; les descriptions, elles, sont partagées par `Arc`), en 127 µs | rien : l'inventaire se lit dans les colonnes |
| L'inventaire que lit le modèle | 17 988 jetons, estimés | 15 070 jetons, estimés : chaque structure écrite une fois, en unités humaines, avec ses choix |
| Ce qu'un écran charge pour démarrer, à contenu égal | 593 Ko de JSON | 113 Ko par MQTT, 20,5 pour un écran de deux pièces |

Trois lectures :

- **La mémoire des ordres coûte moins que ce qu'elle remplace.** Elle range des choses qui
  n'existent pas aujourd'hui (la structure complète, la dernière commande et sa preuve, les
  compositions, les permis), et la description entière reste 2,2 fois plus légère que celle
  d'aujourd'hui. La v1 annonçait 2,5 fois ; le témoin y était gonflé (§ 3.3).
- **Le gain le plus net est hors du hub** : la copie que fait l'assistant à chaque requête, et ce
  que charge un écran.
- **Rien de tout cela ne pèse à l'échelle d'une maison** : moins d'un Mio à ×1. Le choix se fait sur
  ce que la maison sait faire en plus (§ 2.1), et sur ce qu'elle ne fait plus de travers (§ 3), pas
  sur les octets.

## 3. Ce que la revue a changé

Codex a lu la v1, le banc, les quatre propositions liées et le code d'aujourd'hui, puis rendu 25
constats, chacun avec sa preuve (`fichier:ligne`, un scénario, un calcul). Je les ai repris un à un
dans le banc et dans le code : **aucun n'était faux**. Le tableau dit, pour chacun, ce que la v2
fait, le test qui le prouve, et combien de fautes injectées gardent la correction (§ 20.3). Les cinq
constats de mesure (I15 à I19) n'en ont pas : la preuve est la mesure refaite, et deux d'entre eux
sont tenus par des assertions (les mêmes décisions sur le corpus commun, les tailles des fiches).

### 3.1 Les cinq bloquants

| Constat | Ce que la v2 fait | La preuve |
|---|---|---|
| **B1** — Une ACL sur `req/<client>` n'autorise pas la cible nommée dans la demande ; l'origine et la personne étaient déclarées par le client | le **principal** est déduit du sujet, que l'ACL du courtier réserve à son compte ; l'origine et la personne viennent de lui ; chaque cible est autorisée par le hub, filles d'un groupe comprises ; seuls le hub et ses pairs publient `cmd/`, `ord/`, `res/` (§ 11) | test `un_invite_ne_commande_que_ses_pieces` ; 2 fautes |
| **B2** — Une trace copiée après coup ne garantit pas qu'un appui retenté après une bascule ne part pas deux fois | un **appui attend l'accusé du secours** avant de partir (≈ un aller-retour sur le réseau local, quelques centaines de µs) ; sans accusé au bout de 200 ms, l'actif inscrit chez le témoin que le secours a décroché, puis envoie ; une reprise après décrochage, ou sur un instantané ancien, n'est **pas sûre** : une nouvelle tentative d'un appui antérieur à la reprise est refusée, « incertain, ambiguë », jamais refaite (§ 12.6, § 17) | test `un_appui_n_est_ni_perdu_ni_double_par_la_bascule` ; essais de la reprise non sûre et du secours décroché ; 3 fautes |
| **B3** — La fenêtre de 32 reçus réautorisait un appui plus ancien qu'elle, et cédait la fenêtre d'un appelant au 65e | une fenêtre **par principal**, avec sa **plus haute séquence** : une clé plus ancienne que la fenêtre est refusée (`perimee`), la même clé pour un autre ordre ou d'autres arguments aussi (`conflit`) ; aucune fenêtre n'est jamais cédée. C'est la règle d'aujourd'hui (`occurrences.rs:302`), que la v1 avait perdue ; et **toute** demande externe porte sa clé, pas seulement les appuis (§ 12.5) | test des reçus ; 2 fautes |
| **B4** — La génération de 16 bits faisait le tour ; la clé d'une carte se confondait avec celle des paramètres | le numéro tient sur 64 bits : la case (16), **l'époque de l'actif** (16) et la génération (32) ; une case épuisée est condamnée ; cartes, paramètres et textes privés ont chacun leur tiroir (§ 12.1) | tests des cases condamnées et des époques ; 2 fautes, dont celle de la v1 sur la génération |
| **B5** — Le drapeau « texte privé » ne changeait rien : le texte voyageait sur `cmd/` et `ord/` | le texte est remplacé par sa longueur partout ; il ne voyage que par le **canal privé**, vers le module de l'appareil et le pair ; il s'efface à la conclusion ; l'état d'un client et l'instantané d'un actif sans demande vivante n'en ont aucun (§ 12.7) | test `un_texte_prive_ne_sort_que_par_le_canal_prive` : deux textes témoins, cherchés dans chaque sortie, chaque commande, chaque état ; 2 fautes |

### 3.2 Les vingt importants

| Constat | Ce que la v2 fait | La preuve |
|---|---|---|
| **I1** — Une authentification faite une fois ne vaut pas une autorisation vivante | le permis est revérifié à l'arrivée, à l'approbation et à l'envoi ; à sa fin (ou à une révocation), les retenues tombent, ce qui attend dans une voie en est retiré, les abonnements sont coupés (§ 11) | tests `une_retenue_ne_garde_pas_une_autorisation_echue`, essais de la révocation et du permis à l'approbation ; 3 fautes |
| **I2** — L'échéance de l'appelant était ignorée ; une retenue échue s'approuvait tant que le balayage n'était pas passé | l'échéance d'une demande est la plus courte de : 5 s, celle de l'appelant, la fin de son permis ; une demande déjà échue est refusée ; la décision vérifie elle-même l'échéance de la retenue (§ 12.2) | test `une_echeance_se_verifie_a_chaque_frontiere` ; 2 fautes |
| **I3** — Une demande relisait la forme courante de l'ordre | elle garde sa **forme d'admission** jusqu'au bout ; une approbation se lie à la forme et aux cibles montrées ; si elles ont changé, la demande est revalidée : refusée, ou une nouvelle carte à décider (§ 12.3) | test `une_carte_ne_s_approuve_que_sous_sa_forme` ; 1 faute |
| **I4** — Après une bascule, une valeur lue avant pouvait confirmer | seule une lecture **commencée après l'envoi** prouve un ordre, comme aujourd'hui (`orders.rs:273`) ; après une reprise, seule compte une lecture faite après la reprise (§ 13.2) | test `une_lecture_d_avant_la_reprise_ne_confirme_pas`, essai de la lecture d'avant l'envoi ; 2 fautes |
| **I5** — « La valeur vient-elle d'un ordre ? » donnait une compatibilité pour une causalité | trois réponses, tenues par l'ordre des événements (jamais en comparant des horloges) : **oui** si la première variation après l'envoi est celle demandée et que rien n'a changé depuis ; **non** si elle a changé depuis, ou si la commande a échoué ; **on ne sait pas** sinon. Un geste humain pendant la latence de l'appareil reste indécidable sans cause donnée par l'appareil, et le document le dit (§ 15) | test `les_workflows_interrogent_les_ordres` : les quatre cas ; 3 fautes |
| **I6** — La dernière commande et « en cours » se confondaient ; une suite tardive remplaçait une demande plus récente | la dernière commande est la **dernière demande admise** ; seule elle écrit la fiche ; « en cours » se compte à part (`actives`) (§ 13.1) | test `la_derniere_commande_est_la_derniere_admise` ; 1 faute |
| **I7** — Un groupe approuvé ne passait plus par le répartiteur | **un seul chemin d'envoi**, après la garde, pour une demande admise comme approuvée ; les filles d'une mère approuvée ne sont pas retenues de nouveau, et chacune est autorisée (§ 14) | test `un_groupe_approuve_se_repartit` ; 1 faute |
| **I8** — La progression de la mère n'était pas copiée ; « exécuté » comptait comme « confirmé » | la copie rejoue la progression à partir des traces des filles ; la mère compte par niveau de preuve et prend le plus faible (§ 14) | test `un_groupe_coupe_par_la_bascule_garde_son_compte`, essai des filles du hub ; 2 fautes |
| **I9** — La mère pouvait conclure pendant la création de ses filles, et gardait une échéance sans travail | elle connaît toutes ses cibles avant la première fille ; elle n'a pas d'échéance à elle, et ne réveille personne (§ 14) | essais ; 2 fautes |
| **I10** — Une cible ne disait pas de quel ordre de groupe elle dépendait | chaque cible porte le rang de son ordre de groupe ; une cible est l'ordre d'un appareil **physique** (pas de groupe de groupes), de forme compatible ; 64 cibles au plus (§ 14) | essais des compositions ; 2 fautes |
| **I11** — Des plages contiguës et des rangs ne couvraient pas une vraie redéclaration | une redéclaration garde les numéros **par clé** ; les listes d'un appareil sont des numéros ; ce qui n'est plus déclaré est retiré ; les formes citent des clés (§ 9, § 18) | test `une_redeclaration_garde_les_numeros_par_cle` ; 1 faute |
| **I12** — Des plafonds tronquaient en silence ; aucun ne bornait les retenues | chaque plafond (255 valeurs, ordres ou membres par appareil ; 64 cibles ; 50 retenues ; 4 096 demandes en cours ; 16 paramètres) est vérifié avant toute mutation, avec une erreur (§ 12.4) | essais ; 1 faute |
| **I13** — Une copie filtrée ne voyait pas ses pertes avec le seul rang `s` | chaque sortie porte aussi **le rang de sa pièce** ; un écran vérifie la continuité de chacune de ses pièces ; une époque nouvelle le fait se recharger (§ 16.3) | test `un_ecran_de_piece_voit_ses_trous` ; 1 faute |
| **I14** — La validation commune ne vérifiait ni les dictionnaires, ni les objets | un code d'énumération doit être au dictionnaire, pas seulement entre les bornes ; un objet doit suivre son schéma plat ; un schéma imbriqué est refusé à la déclaration ; un corpus fixe écrit chaque décision attendue (§ 10.4) | test du corpus fixe ; 2 fautes |
| **I15** — Le témoin d'aujourd'hui était gonflé | il suit la convention des pilotes réels : un seul point `control` pour les appuis d'un appareil, aucun état pour un point en écriture seule (§ 19.1) | section `catalogue` |
| **I16** — 114 ns contre 39 ns ne mesuraient pas les mêmes décisions | un corpus commun de 1 025 intentions, valides et hors bornes ; les deux validations décident pareil sur les 1 025, puis sont chronométrées (§ 19.1) | section `catalogue` : une assertion arrête le banc si une décision diffère |
| **I17** — Les gains des écrans et de l'assistant venaient aussi d'informations retirées | l'état d'un client porte ce que porte `/api/snapshot` ; le gain est décomposé en partage, encodage et filtrage ; l'inventaire garde les unités et les choix ; les jetons sont dits estimés (§ 16.4, § 19) | section `clients` |
| **I18** — Deux tailles de fiches étaient fausses ; plusieurs postes étaient des forfaits | chaque fiche a sa taille vérifiée à la compilation ; chaque poste est mesuré par le compteur d'allocations, ou dit d'où il vient (§ 19) | `size_of` vérifié à la compilation ; `taille_*` dans la série |
| **I19** — « Le processeur de la journée » laissait du travail hors du chronomètre | la boucle entière, hub, kit et appareils simulés, en temps processeur (`getrusage`) (§ 12.8) | section `demandes` |
| **I20** — Le redémarrage de la soirée reprenait les baux à l'ancien moteur | la machine repart des seuls octets de l'instantané (hub et moteur) et de la configuration (§ 17.3) | test `un_bail_se_retablit_apres_la_bascule_et_le_redemarrage` ; 1 faute |

### 3.3 Ce que la v1 annonçait, ce que la v2 mesure

| Chiffre | v1 | v2 | Pourquoi l'écart |
|---|---|---|---|
| Fiche d'un appareil | 16 o annoncés (20 réels) | 16 o, vérifiés | une liste de numéros au lieu de deux plages |
| Fiche d'un ordre | 32 o | 40 o | le numéro de 64 bits, les demandes vivantes, la preuve |
| Forme ; paramètre | 24 o annoncés (20 réels) ; 16 o | 24 o ; 20 o | les clés sur 16 bits |
| Demande ; ligne de journal | 64 o ; 40 o | 72 o ; 48 o | la séquence, le principal, l'époque, la forme d'admission, les compteurs d'un groupe |
| La description, aujourd'hui ×1 | 828 Kio | 775 Kio | le témoin à la convention des pilotes |
| La description, proposée ×1 | 337 Kio | 357 Kio | la `Def` de la mémoire v2 comptée pour ce qu'elle est (16 o), les listes |
| Valider : aujourd'hui, par le nom, par le numéro | 114 ; 39 ; 4,1 ns | 105 ; 39 ; 5,7 ns | le même corpus, les mêmes décisions ; le dictionnaire vérifié |
| 50 retenues, proposé | 6,2 Kio (un forfait) | 11,2 Kio, mesurés | les cartes, le reçu, la table |
| Les reçus | 8,1 Kio | 10,2 Kio ; borne 65,0 Kio | une fenêtre par principal ; la borne dite |
| Un écran démarre | 631 → 118 Ko | 593 → 113 Ko | à contenu égal, décomposé (§ 16.4) |
| L'inventaire de l'assistant | 17 885 → 14 227 jetons | 17 988 → 15 070 jetons, estimés | unités et choix gardés |
| La journée ×1 | 8,7 ms (temps écoulé, partiel) | 4,7 ms de processeur, toute la boucle | la mesure refaite ; le hub v2 alloue moins |

### 3.4 Ce qui tenait

Codex le dit, et le banc le confirme : l'inventaire du code d'aujourd'hui (§ 5) était juste ; la
séparation des valeurs et des ordres, les formes immuables, les références à génération, la
distinction consigne et appui, les issues bornées par le retour, la composition sans propriétaire et
le « n/m » d'un groupe sont de bons principes. Une précision : la copie que fait l'assistant partage
les descriptions par `Arc` ; elle copie les états de chaque appareil, pas tous les `PointSpec`.

Ce que la revue n'a pas pu vérifier, et la v2 non plus : le transport MQTT réel, notre version du
courtier, le navigateur, le matériel. Les garanties distribuées sont démontrées par la soirée et les
essais, dans un seul processus ; elles restent à éprouver sur deux machines (§ 23, marche 7).

## 4. La question, reformulée

Technique **« Comment pourrait-on… »** :

1. Comment pourrait-on donner au hub, au moteur, aux modules, aux écrans, aux agents et au secours
   la même connaissance de l'appareil, de l'ordre et de sa structure, sans copier les définitions
   dix fois ?
2. Comment pourrait-on garder les ordres à part des valeurs, en ne les reliant que par référence, et
   par une référence qu'une redéclaration ne casse pas ?
3. Comment pourrait-on tenir chaque demande, de son arrivée à son issue et à sa suite, dans des
   fiches de taille fixe, sans qu'une issue tardive tombe jamais sur la mauvaise demande ?
4. Comment pourrait-on ne laisser commander que ce que chacun a le droit de commander, au moment où
   l'ordre part, et pas seulement au moment où il arrive ?
5. Comment pourrait-on laisser un workflow interroger les ordres comme il interroge les valeurs,
   sans lui faire croire plus qu'on ne sait ?
6. Comment pourrait-on assembler des appareils virtuels à partir des physiques, et les servir tels
   quels aux interfaces ?
7. Comment pourrait-on garder tout cela exact à travers une bascule, un redémarrage et un nettoyage,
   sans jamais rejouer un appui, ni le perdre en silence ?
8. Comment pourrait-on servir les clients sans module de traduction, et sans qu'un écran filtré
   ignore ce qu'il a perdu ?

Ce qui prouvera la réponse, fixé avant de chercher :

| Critère | La preuve |
|---|---|
| Chacun connaît l'appareil, l'ordre et sa structure, sans copie qui diverge | les copies du catalogue (secours, écran) égales à celle du hub ; un écran valide comme le hub |
| Une valeur n'est jamais un ordre | deux registres ; un volet RTS sans aucune valeur |
| Aucune issue ne tombe sur la mauvaise demande | des tests, et des fautes injectées qui les font échouer |
| Personne ne commande hors de ses droits, même après coup | un invité, un permis qui finit, une révocation, une approbation tardive |
| Aucun appui n'est rejoué, aucun n'est perdu sans le dire : module mort, bascule, redémarrage | chaque envoi compté, une fois ; une reprise non sûre refuse au lieu de deviner |
| Ce qui a été demandé en dernier se lit partout pareil, et dit sa preuve | la fiche de chaque ordre, comparée ; « oui, non, on ne sait pas » |
| La mémoire reste petite, et les chiffres sont honnêtes | mesurés à ×1, ×10, ×100, à information égale, à côté d'aujourd'hui |

## 5. Ce qui manque

![Ce que la mémoire range, et ce qui manque](ordres-appareils-v2/trou.svg)

### 5.1 Dans la conception

| Où | Ce qui y est | Ce qui n'y est pas |
|---|---|---|
| V7 § 5.1, § 5.6, § 20.2, § 20.7 | le contrat d'un ordre : la méta d'un point écrivable (effet, retour par code, bornes d'ordre, suivi, réveillable, sensibilité), les neuf issues, les voies, `req/`, `cmd/`, `res/` | une fiche qui garde la structure ; la dernière commande ; la mémoire des demandes |
| V7 § 11.4.4, mémoire v2 § 5.1 | `Def` de 16 o par point, dont **un octet `sens`** : sémantique, lecture, écriture | tout le reste de la structure d'un ordre |
| Mémoire v2 § 8.4 | l'exécution de 64 o porte `demande`, « l'ordre en vol dont elle attend l'issue » | la demande elle-même : rien ne dit où elle vit ni ce qu'elle pèse |
| HA v2 § 3, § 7 | les retenues et les baux sont critiques, sur `wf/` ; un ordre envoyé sans issue devient « incertain » à la bascule | comment le secours sait quels ordres ont pu partir, et ce qu'il fait d'un appui retenté |
| Nettoyage § 9 | un appareil retiré n'est plus commandé ; ses retenues et ses baux se terminent | la fiche d'un ordre, et ce qu'en garde une place libérée |

La structure est donc **décrite** (sur MQTT) mais **rangée nulle part** ; l'état d'un ordre en cours
n'a pas de place ; et les ordres, comme dans le code, sont restés des points.

### 5.2 Dans le code d'aujourd'hui

**Inventaire** de tout ce qui garde quelque chose d'un ordre, relevé fichier par fichier
(`a835893`) ; la revue l'a recoupé sur un large échantillon :

| Ce qui est gardé | Où | Ce que c'est |
|---|---|---|
| La structure | `Device.points: Vec<PointSpec>` dans `Model.devices` (`moli-core/src/model.rs:10`, `moli-runtime/src/hub.rs:512`) | un ordre est un point écrivable (`Access { read, write }`, `spec.rs:119`) ; sa structure est le `Kind` du point : bornes, choix ; ni effet, ni retour, ni suivi |
| La validation | `Hub::prepare` (`hub.rs:1731`) | le `PointId` découpé (un `DeviceId` alloué), le `BTreeMap` des appareils, le point cherché en ligne, l'accès, `validate` |
| La garde | `verdict`, `sensitive`, `rooms_of` (`hub.rs:1095-1152`) | la sémantique du point (`Control`, `Lock`), les pièces de l'appareil et de ses `members` |
| Les droits | `guest_may` (`moli-api/src/rest.rs:119`), le permis d'un invité revérifié à l'envoi (`hub.rs:1798`) et au transport (`driver.rs:157`), et chaque seconde par le flux SSE (`sse.rs:101`) | **une autorisation vivante** : la v1 l'avait perdue en passant à MQTT ; la v2 la garde (§ 11) |
| Une demande en cours | `CommandRequest` (`driver.rs:66`) dans la file du pilote (64 places, `hub.rs:64`), un `oneshot`, un `AtomicU8`, et chez l'appelant `Delivery`, `Observer` et une fermeture qui écrira la ligne de journal (`hub.rs:343-431`) | l'état d'un ordre est réparti entre la tâche de l'appelant et celle du pilote ; il n'a pas de numéro |
| L'échéance | `command_timeout` 5 s (`hub.rs:122`) ; `REPLY_MARGIN` 400 ms, `MIN_LEFT` 1 s (`orders.rs:18-21`) | — |
| La voie d'un pilote | `OrderLine` (`orders.rs:25`) ; `Confirmations` : 32 valeurs attendues au plus, et **seule une lecture commencée après l'accusé confirme** (`orders.rs:209-228`, `orders.rs:273`) | dans chaque pilote qui l'utilise, pas dans tous |
| Les retenues | `Approvals { pending }`, 50 au plus, 10 min (`hub.rs:173-247`) ; la décision vérifie l'échéance et garde le permis de l'invité (`hub.rs:1313-1403`) | en mémoire seulement : **perdues au redémarrage**, sans ligne qui les conclue |
| Les reçus | boutons, compteurs, minuteries : 64 appelants, 32 reçus chacun, **la plus haute séquence**, les conflits et les clés périmées refusés (`occurrences.rs:133-171`, `occurrences.rs:302`, `counters.rs:11-14`) | durables, mais sur une seule machine |
| Les scènes | `Scenes { active, ended (64), points, leases }` (`scenes.rs:135-147`) : une scène coordonne ses appareils, 512 au plus | une dernière commande… le temps d'une scène |
| Le journal | 500 `Arc<JournalEntry>` en mémoire, en textes (`hub.rs:542-545`), et un fichier JSON Lines rogné à 8 Mio (`journal_log.rs:20-21`) | le seul endroit où l'on retrouve ce qu'un ordre a donné |
| La dernière commande | **nulle part dans le hub** ; l'écran garde la sienne, par point, 8 s (`web/src/lib/hub.svelte.js:17-18, 217-246`) | un volet RTS n'affiche rien de ce qu'il a reçu |
| Ce que lit l'assistant | `Hub::snapshot` à chaque requête (`moli-assistant/src/lib.rs:1139`), puis l'inventaire (`house.rs:115`) | les états de toute la maison copiés à chaque question ; les descriptions partagées par `Arc` |
| Ce que lisent les agents | `list_devices`, `get_device`, `send_command(point, valeur)` (`moli-api/src/mcp.rs:344-441`) | la structure d'un ordre se devine dans le `Kind` du point |

**Les ordres déguisés en valeurs.** Au moins neuf pilotes déclarent des points en écriture seule. Ce
sont des ordres sans valeur, rangés comme des valeurs :

| Pilote | Le « point » | Où |
|---|---|---|
| Telegram, téléphones | `notify`, `push` : un texte à envoyer | `moli-telegram/src/lib.rs:171`, `moli-phones/src/lib.rs:653` |
| Sonos | `announce` | `moli-sonos/src/lib.rs:201` |
| Philips | `text`, `key`, `media_control` | `moli-philips/src/lib.rs:337, 351, 402` |
| Roborock, Landroid | `control` (démarrer, pause, stop…) | `moli-roborock/src/model.rs:163`, `moli-landroid/src/model.rs:255` |
| Nuki | l'ouverture | `moli-nuki/src/model.rs:185` |
| TaHoma (RTS) | `control` : une énumération des gestes, aucun état (`moli-overkiz/src/model.rs:145`, et son test, `model.rs:604`) | `moli-overkiz/src/model.rs:124` |
| Aides | les commandes des minuteries et des compteurs | `moli-helpers/src/lib.rs:196, 267` |

### 5.3 Trois défauts trouvés en chemin

- **Une retenue disparaît au redémarrage, sans trace.** Les retenues ne vivent qu'en mémoire
  (`hub.rs:442`, aucune option de fichier dans `HubOptions`) ; le journal garde la ligne « en
  attente d'approbation », jamais sa fin. L'agent a pourtant reçu « elle expire dans 10 minutes »
  (`mcp.rs:431-437`).
- **Aucune dernière commande.** Un ordre et la valeur qui suit ne sont reliés nulle part (la V7 le
  notait déjà, § 5.8). L'écran garde sa propre attente, 8 s, par point (`hub.svelte.js:238-241`) ;
  un autre écran ne sait rien de l'ordre en cours.
- **L'assistant copie les états de toute la maison à chaque requête** (`Hub::snapshot`,
  `lib.rs:1139`) : 317 Kio alloués puis rendus, à ×1, pour en tirer quelques lignes par appareil.

## 6. Qui doit connaître quoi

Ta phrase, prise au pied de la lettre : **l'appareil, l'ordre, la structure de l'ordre**. Pour
chaque composant, ce qu'il en connaît, ce qu'il garde, et par où il l'apprend :

![Qui connaît quoi](ordres-appareils-v2/qui.svg)

| Qui | L'appareil | L'ordre | La structure | Ce qu'il garde des ordres | Il l'apprend par |
|---|---|---|---|---|---|
| **Le hub** | registre, fiche de 16 o, liste, composition | registre, fiche de 40 o | toutes les formes, les clés, les dictionnaires | tout : catalogue, demandes, reçus, retenues et cartes, journal, permis | les déclarations des modules ; la configuration des comptes |
| **Le moteur** | son numéro | le numéro de l'ordre, dans le nœud « commander » | la forme, à la compilation | ses exécutions (`demande`), ses baux, dans son instantané | `meta/`, `meta/forme/`, `ord/` |
| **Un module** | ses appareils (natif ↔ numéro) | le numéro, dans `cmd/` | celle de la demande, dans `cmd/` ; elle borne son issue | ses voies, ses ordres en vol, ses suivis | lui-même ; le canal privé pour un texte privé |
| **Un écran, l'appli, un site** | nom, numéro, pièce, composition, disponibilité | nom, numéro | la forme et les dictionnaires en cache, par numéro | les dernières commandes de ses pièces, le rang de chacune | MQTT : `etat/`, `meta/`, `meta/forme/`, `ord/`, `val/`, filtrés par ses droits (§ 16) |
| **Un agent, l'assistant** | le nom | le nom | un outil par ordre, tiré de la forme, en unités humaines | rien | MCP, REST |
| **Le secours** | le même registre | le même registre | les mêmes formes | tout ce que tient le hub, canal privé compris ; il accuse chaque rang | le flux critique et l'état complet (HA v2) |
| **La base, l'archive** | le nom | le nom | — | le journal des ordres, par les noms | `run/`, puis `hist/ops` |

Deux règles de la mémoire v2 suffisent à tenir tout cela :

- **un seul propriétaire, des copies** (§ 7) : le hub écrit, les autres reçoivent ;
- **un numéro inconnu se redemande, il ne se devine pas** (§ 4.4) : vrai pour une forme, un
  dictionnaire et un point.

## 7. Les idées

### 7.1 Ailleurs, comment fait-on ?

Technique des **analogies** :

| Où | Ce qu'ils font | Ce qu'on en retient |
|---|---|---|
| **Matter, Zigbee (ZCL)** | un cluster a des **attributs** (des valeurs) et des **commandes** (des ordres), numérotés à part ; une commande a des champs typés ; les commandes sensibles sont « minutées » ; chaque accès est contrôlé par une liste par nœud et par sujet | **séparer les valeurs et les ordres** ; **autoriser chaque commande**, pas seulement la connexion |
| **CANopen** | le dictionnaire d'objets : chaque entrée a un numéro, un type, un accès, des bornes ; tous les nœuds le connaissent par un fichier de description | **un catalogue partagé, par numéro**, publié une fois |
| **Home Assistant** | des services à schéma, les capacités de chaque entité, et un **contexte** attaché à chaque changement : qui, et par quel service | **la dernière commande et sa cause, quand l'appareil la donne** ; un outil par ordre pour les agents |
| **Le contrôle aérien** | une bande par vol, de taille fixe ; la clairance, puis son collationnement | **une fiche par demande** ; la confirmation par ce qui revient, et seulement après l'émission |
| **Les paiements** | une clé d'idempotence par requête, une fenêtre, et la règle : une requête qu'on ne peut plus départager est refusée, jamais rejouée | **les reçus, la plus haute séquence, la reprise non sûre** |
| **Les registres de schémas (Kafka)** | un message porte le numéro de son schéma ; le lecteur le demande une fois et le garde | **`meta/forme/<f>`**, en cache par numéro |

### 7.2 SCAMPER sur « un ordre est un point écrivable »

| Lettre | La question | L'idée |
|---|---|---|
| **S**ubstituer | Et si un ordre n'était pas un point ? | un registre des ordres, à part (retenu) |
| **C**ombiner | Et si les retenues étaient des demandes ? | un état `RETENUE` dans la même fiche (retenu) |
| **A**dapter | Et si on reprenait les exécutions de la v2 ? | cases réutilisées, génération, et l'époque de l'actif (retenu) |
| **M**odifier | Et si le retour par code de la V7 devenait un ordre par geste ? | ouvrir, fermer et stop ont chacun leur forme (retenu) |
| **P**roduire autre chose | Et si la fiche de l'ordre servait aux workflows ? | règles et déclencheurs sur un ordre (retenu) |
| **É**liminer | Et si l'écran n'avait plus de module de traduction ? | les clients par MQTT (§ 16, ton choix) |
| **R**enverser | Et si c'était la valeur qui citait l'ordre ? | écarté : une valeur ne sait rien des ordres (mémoire v2 § 8.2) |

### 7.3 Brainstorming inversé : comment rater la mémoire des ordres ?

La v1 avait sa liste ; la revue y a ajouté ce qu'elle avait laissé passer (en gras).

| Pour rater | Il suffirait de… | Le garde-fou |
|---|---|---|
| Une issue qui tombe sur la mauvaise demande | réutiliser la case sans génération | la génération ; une faute injectée le vérifie |
| **Un numéro redonné par celui qui reprend** | **repartir des générations de l'ancien actif** | **l'époque dans le numéro** |
| Un ordre rejoué après une bascule | renvoyer ce qui était remis à un module de l'ancien actif | « incertain », jamais renvoyé ; chaque envoi est compté |
| **Un appui fait deux fois par une reprise** | **croire qu'une trace copiée « après » protège un effet fait « avant »** | **l'accusé du secours avant l'effet ; la reprise non sûre refuse** |
| **Un vieil appui rejoué** | **ne garder que les 32 derniers reçus** | **la plus haute séquence ; « périmée »** |
| **Un invité qui ouvre la porte** | **autoriser le sujet, pas la cible ; croire l'origine que le client déclare** | **le principal de la connexion ; chaque cible autorisée** |
| **Une autorisation qui survit à son permis** | **la vérifier une fois** | **un permis revérifié à chaque frontière** |
| Une carte d'approbation qui approuve autre chose | décider par le numéro de case, **ou sur une forme qui a changé** | la carte porte l'ordre, **la forme et les cibles** montrés |
| Un ordre envoyé hors de sa structure | faire confiance au client, **ou ne vérifier que les bornes** | le hub valide toujours, **dictionnaire et schéma compris** |
| Un écran qui commande l'appareil neuf avec un vieux numéro | ignorer la génération de l'ordre | un numéro de l'ancienne génération est inconnu |
| Une copie dont les échéances dérivent | lui envoyer l'horloge monotone de l'actif | l'échéance voyage en heure murale |
| Une valeur prise pour l'œuvre d'un ordre | comparer la valeur seule, **ou des horloges** | **l'ordre des événements, et « on ne sait pas » quand on ne sait pas** |
| **Une confirmation par une vieille lecture** | **accepter toute valeur qui arrive après l'envoi** | **une lecture commencée après l'envoi** |
| Un groupe qui dit « fait » quand une ampoule a obéi | compter les réussites, pas les cibles, **ou confondre « exécuté » et « confirmé »** | « échec, partiel, n/m », **par niveau de preuve** |
| **Un texte privé publié** | **poser le drapeau sans s'en servir** | **le canal privé ; deux textes témoins cherchés partout** |
| **Un chiffre flatteur** | **construire le témoin depuis le nouveau modèle** | **le témoin à la convention des pilotes ; le même corpus ; `size_of`** |

## 8. Les options, et le choix à critères visibles

**Analyse morphologique** : six dimensions, et leurs options.

| Dimension | Options |
|---|---|
| D1. Où ranger la structure | S1 dans `Def` (48 o pour chaque point) · S2 une colonne par point · S3 une fiche par ordre · **S4 une fiche par ordre et des formes partagées** · S5 relire la méta à chaque demande |
| D2. Ce qu'est un ordre | N1 un point écrivable (aujourd'hui) · **N2 une entrée à lui, dans son registre, à paramètres** · N3 un point et un code |
| D3. Les demandes | E1 des tâches et des canaux (aujourd'hui) · **E2 des fiches, cases, génération et époque** · E3 une table par identifiant · E4 chacun garde la sienne |
| D4. Les retenues | R1 à part (aujourd'hui) · **R2 un état de la demande** |
| D5. La dernière commande | L1 nulle part (aujourd'hui) · L2 dans la valeur · **L3 dans la fiche de l'ordre** |
| D6. Ce qui passe au secours | H1 rien · **H2 chaque pas, sur `ord/`, et l'accusé avant un appui** · H3 la demande, pour la renvoyer |

**Sélection à critères visibles** : simplicité, mémoire, justesse (aucune mauvaise issue, aucun
rejeu), cohérence avec la mémoire v2, le nettoyage et la HA v2, lisibilité pour les écrans et les
agents. De 1 à 3 chacun :

| Option | Simplicité | Mémoire | Justesse | Cohérence | Lisibilité | Total |
|---|---|---|---|---|---|---|
| S1 la structure dans `Def` | 3 | 1 | 2 | 1 | 2 | 9 |
| S3 une fiche par ordre | 3 | 2 | 3 | 3 | 2 | 13 |
| **S4 fiche et formes partagées** | 2 | 3 | 3 | 3 | 3 | **14** |
| S5 relire la méta | 2 | 2 | 1 | 1 | 1 | 7 |
| N1 un point écrivable | 3 | 1 | 1 | 1 | 1 | 7 |
| **N2 un registre des ordres** | 2 | 3 | 3 | 3 | 3 | **14** |
| E1 tâches et canaux | 2 | 1 | 2 | 1 | 1 | 7 |
| **E2 cases, génération, époque** | 3 | 3 | 3 | 3 | 2 | **14** |
| L2 dans la valeur | 2 | 1 | 1 | 1 | 2 | 7 |
| **L3 dans la fiche de l'ordre** | 3 | 3 | 3 | 3 | 3 | **15** |
| H3 renvoyer après la bascule | 1 | 2 | 1 | 1 | 2 | 7 |
| **H2 chaque pas sur `ord/`, l'accusé avant un appui** | 2 | 3 | 3 | 3 | 3 | **14** |

Le choix : **S4, N2, E2, R2, L3, H2.** La revue rappelle qu'une grille de scores ne prouve pas
qu'une complexité est nécessaire ; le plan (§ 23) en tient compte : il livre d'abord le catalogue et
le suivi dans le code d'aujourd'hui, et ne serre les fiches à leur taille fixe qu'une fois les
règles de sécurité et de reprise éprouvées.

## 9. Les valeurs et les ordres, séparés

**Premiers principes.** Une valeur arrive de l'appareil, quand il veut ; elle a une qualité, une
fraîcheur, une précédente. Un ordre part de la maison, quand quelqu'un le demande ; il a des
paramètres, une échéance, une issue, une suite, et quelqu'un qui en répond. Ils ne vivent pas au
même rythme, n'ont pas la même durée de vie, et ne sont pas lus par les mêmes : une condition lit
une valeur, une action donne un ordre.

![Les valeurs et les ordres : deux registres](ordres-appareils-v2/separes.svg)

Les règles :

1. **Deux registres**, celui du nettoyage deux fois : les valeurs (`appareil/clé`) et les ordres
   (`appareil/ordre`). Chacun a ses numéros (place 24 bits, génération 7 bits), ses noms, ses
   fiches. Les numéros sont typés : `Point`, `Ordre`, `Appareil` (mémoire v2 § 4.3).
2. **Un appareil a une liste** : les numéros de ses valeurs, puis ceux de ses ordres. Un volet RTS a
   quatre ordres et aucune valeur ; un capteur a des valeurs et aucun ordre. La v1 voulait deux
   plages de places qui se suivent ; une redéclaration qui ajoute une valeur aurait dû tout
   déplacer.
3. **Le seul lien est une référence, portée par la forme de l'ordre, par la clé de la valeur** : la
   valeur qui le prouve (`confirme`) et celle que règle chaque paramètre (`regle`). La clé ne dépend
   ni de l'appareil, ni de l'ordre des déclarations : deux lampes du même modèle partagent la même
   forme, et une valeur ajoutée au milieu ne déplace rien.
4. **Une valeur ne sait rien des ordres**, comme elle ne sait rien des workflows (mémoire v2 § 8.2,
   règle 3). C'est l'ordre qui cite la valeur, jamais l'inverse.
5. **Sur MQTT, deux familles** : les valeurs sur `val/`, les ordres sur `ord/` (§ 10.5).

La migration ne réécrit aucun pilote (§ 23) :

| Aujourd'hui | Avec deux registres |
|---|---|
| un point lisible et écrivable (`on` d'une lampe) | une valeur `on`, et un ordre `on` à un paramètre, dont la forme dit : la valeur `on` le prouve, à la tolérance près |
| un point en écriture seule (`notify`, `control`, `unlatch`) | un ordre seul, sans valeur |
| une énumération d'ordres (`control` : ouvrir, stop, fermer) | d'abord un ordre à un paramètre, puis, quand le module le veut, un ordre par geste : chacun sa forme, son retour, son suivi |

## 10. L'appareil, l'ordre, la structure

![L'appareil, l'ordre, la structure](ordres-appareils-v2/trois.svg)

### 10.1 Les fiches

Bloc technique — les trois fiches (extraits du banc, `tools/ordres-banc/src/`, sans `unsafe` ;
chaque taille est vérifiée à la compilation par `const _: () = assert!(size_of::<…>() == …)`) :

```rust
/// Un appareil : sa liste (ses valeurs, puis ses ordres : des numéros), sa composition, sa pièce. 16 o.
#[repr(C)]
pub struct App {
    pub liste: u32, pub compo: u32,
    pub piece: u16,
    pub nb_points: u8, pub nb_ordres: u8, pub nb_compo: u8,
    pub genre: u8,            // physique, virtuel
    pub _r: u16,
}

/// Un ordre : sa forme, sa dernière commande, ses demandes vivantes, sa preuve. 40 o, une par ordre.
#[repr(C)]
pub struct Fiche {
    pub valeur: i64,          // le premier paramètre de la dernière commande
    pub instant_ms: u64,      // son dernier pas (murale)
    pub demande: Ref,         // la dernière demande ADMISE : une issue tardive d'une autre ne la remplace pas
    pub appareil: u32, pub qui: u32,
    pub forme: u16,
    pub issue: u8,            // celle de la dernière commande ; « en cours » ; 0 : jamais commandé
    pub origine: u8,
    pub actives: u8,          // combien de demandes de cet ordre vivent : « en cours », à part de l'issue
    pub changes: u8,          // les variations de la valeur prouvée depuis l'envoi de la dernière commande
    pub preuve: u8,           // prouvée par un changement ; le premier ; modifiée depuis
    pub _r: u8,
}

/// La structure d'un ordre, partagée. 24 o, plus ses paramètres et ses valeurs attendues.
#[repr(C)]
pub struct Forme {
    pub params: u32, pub attendus: u32,   // début de ses paramètres, de ses valeurs attendues
    pub confirme: u16,        // la CLÉ de la valeur qui prouve l'ordre
    pub suivi_s: u16, pub tolerance: u16, pub schema: u16,
    pub nb_params: u8, pub nb_attendus: u8,
    pub effet: u8,            // consigne (la dernière gagne) | appui (chacun compte)
    pub retour: u8,           // état | pont | accusé | aucun : l'issue ne le dépasse jamais
    pub drapeaux: u8,         // à l'arrêt, réveillable, ouvre, contrôle, texte privé, au hub, attendu = paramètre, groupe
    pub _r: [u8; 3],
}

/// Un paramètre, au canon : 20 o. Les bornes sont celles de l'ordre (V7 G24).
#[repr(C)]
pub struct Param {
    pub min: i32, pub max: i32, pub pas: u16,
    pub dico: u16,            // le dictionnaire d'une énumération
    pub regle: u16,           // la CLÉ de la valeur que ce paramètre règle
    pub nature: u8, pub echelle: i8, pub unite: u8,
    pub _r: [u8; 3],
}
```

![La forme d'un ordre](ordres-appareils-v2/forme.svg)

Ce que la forme contient est exactement la méta d'un ordre de la V7 (§ 5.1), mise en fiches :

| La V7, sur `meta/` | La forme |
|---|---|
| `effet` : consigne ou appui | `effet` |
| `retour` par code : `etat`, `pont`, `accuse`, `aucun` | `retour`, `confirme`, les valeurs attendues ; un code d'énumération qui avait son propre retour devient son propre ordre |
| `suivi_s` (180 s au plus) | `suivi_s` |
| bornes d'ordre, à part de la validité | `min`, `max`, `pas` de chaque paramètre |
| `reveillable`, `sensible: texte_prive`, `proprietaire: hub` | les drapeaux |
| `semantic` lu par la garde (`Control`, `Lock` qui ouvre) | les drapeaux « contrôle » et « ouvre » |
| objet court au schéma déclaré | `schema` : un objet plat (des clés, des textes, des entiers, des booléens) |

### 10.2 Une forme ne change jamais

Une forme est **immuable** : une structure nouvelle est une forme nouvelle, avec un numéro neuf. Un
écran, un agent ou le moteur qui l'a en cache n'a donc jamais à la vérifier ; il demande seulement
celles qu'il ne connaît pas. Une redéclaration (un micrologiciel qui change les bornes d'une
consigne) change la forme de l'ordre, pas la forme elle-même (§ 18) ; une demande en cours garde la
sienne (§ 12.3).

### 10.3 Le catalogue en mesures

| Le catalogue des ordres | ×1 | ×10 | ×100 |
|---|---|---|---|
| Appareils (dont virtuels) | 525 (29) | 5 243 (283) | 52 407 (2 807) |
| Valeurs | 2 264 | 22 640 | 226 400 |
| Ordres | 802 | 8 468 | 82 592 |
| Formes ; paramètres ; clés | 16 ; 12 ; 23 | 16 ; 12 ; 23 | 16 ; 12 ; 23 |
| Registre des ordres (noms, générations, statuts), mesuré | 44,4 Kio | 499 Kio | 4,6 Mio |
| Catalogue (formes, clés, dictionnaires, fiches de 40 o), mesuré | 33,9 Kio | 333 Kio | 3,2 Mio |
| Valider une demande, par le nom ; par le numéro | 39 ns ; 5,7 ns | 51 ns ; 6,0 ns | 59 ns ; 6,2 ns |
| Aujourd'hui, `Hub::prepare`, le même corpus | 105 ns | 162 ns | non construit |

Les ordres et les appareils virtuels de la maison du banc sont une **hypothèse** : un modèle par
classe d'appareil (annexe A.2). À ×10 et ×100, la maison répète les mêmes modèles ; une vraie maison
plus grande aurait un peu plus de formes, jamais une par ordre.

### 10.4 La validation commune

Le hub, l'écran et l'agent valident avec **la même fonction**, la même forme et les mêmes
dictionnaires : le nombre de paramètres, les bornes, le pas, **le code au dictionnaire** (un code
entre les bornes mais absent du dictionnaire est refusé), la longueur d'un texte, et **le schéma
d'un objet** : des clés déclarées, chacune une fois, au bon type. Un schéma que la validation ne
sait pas vérifier (un objet imbriqué, un tableau) est refusé dès la déclaration de la forme : rien
ne passe sans être vérifié.

L'égalité entre le hub et l'écran ne prouve rien si les deux appellent la même fonction incomplète :
un **corpus fixe** écrit chaque décision attendue (dix-sept cas, dont un code absent du
dictionnaire, une clé inconnue, une clé en double, un objet imbriqué, un texte qui n'est pas du
JSON). La soirée vérifie en plus que l'écran décide comme le hub sur 3 344 demandes tirées au
hasard.

### 10.5 Sur MQTT

Bloc technique — la méta d'un appareil, une forme, une demande, un ordre, une issue, une trace
(proposé) :

```text
meta/salon_lampe        {"points":{"on":{"n":1234,"kind":"binary"},"brightness":{"n":1235,…}},
                         "ordres":{"on":{"n":87,"f":3},"brightness":{"n":88,"f":4},"color_temp":{"n":89,"f":9}}}
meta/forme/4            {"f":4,"e":1,"r":1,"c":"brightness","d":64,"s":0,"t":2,"p":[[2,0,254,1,0,0,0,"brightness"]],"a":[]}
req/ecran-salon         {"q":4127,"o":"salon_lampe/brightness","v":[128],"e":1792675629123,"n":1}
cmd/z2m/salon_lampe/brightness   {"d":"4027/7/12","i":88,"f":4,"v":[128],"e":1792675629123,"E":7}
in/z2m/$issue           {"d":"4027/7/12","i":1}
res/ecran-salon         {"q":4127,"i":1,"ms":312}
ord/salon_lampe/brightness       {"d":"4027/7/12","e":"conclue","v":128,"i":1,"o":"ui","t":1792675624435,"E":7,"s":918301,"p":0,"sp":40211}
prive/hub-b             {"d":"4027/7/13","t":"…"}
```

| Ce qui change dans le contrat de la V7 (§ 7) | Pourquoi |
|---|---|
| `meta/<appareil>` sépare `points` et `ordres` ; chaque ordre porte son numéro `n` et sa forme `f` | deux registres ; le numéro voyage déjà (décision v2 § 4.4) |
| `meta/forme/<f>`, critique, publiée une fois ; elle cite des clés | la structure partagée, en cache par numéro |
| `req/<client>` : le sujet dit le principal ; le message porte sa séquence `q`, l'ordre `o`, les paramètres `v`, son échéance `e`, la tentative `n` | l'ACL du courtier réserve le sujet au compte ; la clé d'idempotence de toute demande ; l'échéance de l'appelant |
| `cmd/<module>/<appareil>/<ordre>` porte `d` (case, époque, génération), `i`, la forme `f` | la forme d'admission ; l'époque sépare deux actifs |
| `ord/<appareil>/<ordre>` : chaque pas d'une demande, au rang `s` et au rang de sa pièce `sp` | la dernière commande partout ; le secours sait ce qui a pu partir ; un écran filtré voit ses trous |
| `prive/<pair>` : le texte privé d'une demande vivante, vers le seul pair ; le module le lit à l'envoi | un texte privé ne passe ni par `cmd/`, ni par `ord/` |

Le message `meta/` d'une maison ×1 pèse 306 Ko quand chaque ordre porte sa structure, comme dans la
V7, et 242 Ko avec les formes, dont 1,5 Ko pour les 16 formes elles-mêmes. L'écart est modeste, car
les valeurs dominent `meta/` ; ce qui compte est qu'un client n'apprend chaque structure qu'une
fois.

## 11. Les principaux et leurs droits

**Premiers principes.** Qui demande est une question de sécurité, pas de contenu. Un client ne peut
pas dire qui il est ; c'est la connexion qui le dit.

- **Le principal vient de la connexion** : un compte MQTT par écran, par appli, par agent, par
  script, authentifié par le jeton que donne le PIN (comme aujourd'hui) ; le courtier ne laisse un
  compte publier que sur `req/<son compte>` ; le hub en déduit le principal, son origine et la
  personne qu'il représente. Ce qu'un message dit de son origine ou de sa personne est ignoré. Le
  moteur et le hub sont des principaux internes : seuls eux nomment une exécution.
- **Le permis** : jusqu'à quand le principal commande (une invitation a sa fin), et dans quelles
  pièces (un invité, un écran de pièce). **Chaque cible est autorisée** : toutes les pièces qu'un
  ordre atteint, celles des cibles et des membres d'un groupe comprises, et encore chaque fille au
  moment où elle naît.
- **Le permis est revérifié à chaque frontière** : à l'arrivée, à l'approbation d'une retenue, et à
  l'envoi (l'échéance d'une demande ne dépasse jamais la fin du permis : le kit ne l'envoie pas
  au-delà). À la fin d'un permis, ou à une révocation, ses retenues sont refusées, ce qui attend
  dans une voie en est retiré (« non envoyé, droits »), ses abonnements sont coupés par le courtier.
  Une retenue gardée garde une **demande**, jamais une autorisation acquise.
- **Seuls le hub et son pair publient** `cmd/`, `ord/`, `res/` et `prive/` ; les modules publient
  leurs valeurs et leurs issues sur leurs sujets ; les ACL du courtier le tiennent (V7 § 11.4.5 : 30
  à 44 ns par message vérifié).
- **Seul un humain, à l'écran, décide d'une retenue** (aujourd'hui : l'origine `Ui`,
  `hub.rs:1322`) ; il nomme l'ordre et la forme qu'il a vus.

Ce que la soirée rejoue : un invité, qui n'a que le bureau, se voit refuser la porte d'entrée et le
groupe du séjour (qui atteint le salon et la chambre), mais met en pause l'imprimante du bureau ; et
la demande porte l'invité, pas la personne qu'il prétendait être. L'agent d'un gardien, qui n'a que
l'entrée jusqu'à 18 h 40, voit sa retenue refusée à 18 h 40 précises, et l'approbation de 18 h 50
échoue. Deux essais : une révocation retire de la voie ce qui n'était pas parti, et une approbation
refuse une demande dont le principal a perdu la pièce entre-temps.

## 12. Les demandes

### 12.1 La fiche de 72 octets, et son numéro

![La demande : 72 octets](ordres-appareils-v2/demande.svg)

Bloc technique — la demande (extrait du banc) :

```rust
/// Une demande : 72 o, dans une case réutilisée.
#[repr(C)]
pub struct Demande {
    pub valeur: i64,        // le premier paramètre (un texte est au tiroir)
    pub seq: u64,           // la séquence de l'appelant : avec le principal, la clé d'idempotence
    pub echeance_ms: u64,   // monotone : la plus courte de 5 s, l'appelant, la fin du permis
    pub recue_ms: u64,      // murale : son arrivée
    pub parent: Ref,        // la demande mère d'un ordre de groupe
    pub ordre: u32, pub qui: u32,
    pub generation: u32,
    pub forme: u16,         // la forme d'admission : son interprétation ne change plus
    pub de: u16,            // le principal, jamais déclaré par le client
    pub etat: u8, pub issue: u8, pub origine: u8, pub raison: u8,
    pub cibles: u8, pub conclues: u8, pub confirmees: u8, pub executees: u8,   // un groupe
    pub drapeaux: u8,       // paramètres au tiroir, approuvée, texte privé
    pub _r: u8,
    pub epoque: u16,        // l'époque de l'actif qui l'a prise
    pub _s: [u8; 4],
}
/// Son numéro : la case (16 bits), l'époque (16 bits), la génération (32 bits). Il voyage dans
/// cmd/ et dans l'issue du module ; c'est le champ `demande` d'une exécution (mémoire v2 § 8.4).
pub struct Ref(pub u64);
```

Les règles :

- **la case se réutilise, le numéro entier jamais** : la génération (32 bits : quatre milliards de
  réutilisations d'une même case : 136 ans à une demande par seconde, 50 jours à mille) écarte une
  issue tardive ; une case qui atteindrait la dernière est condamnée, la règle du nettoyage, et une
  autre prend le relais (65 536 cases) ;
- **l'époque sépare deux actifs** : celui qui reprend ne peut pas redonner un numéro que l'ancien a
  pris sans le lui transmettre (une issue de l'ancien monde n'attrape jamais une demande du
  nouveau) ;
- **l'identité d'une demande, pour son appelant, est sa clé** (le principal et la séquence) : `res/`
  répond par la séquence ; une carte d'approbation porte le numéro ;
- **ce qui quitte la machine est en heure murale** (HA v2 § 6), l'échéance comprise.

### 12.2 Les échéances, à chaque frontière

- À l'arrivée : une demande dont l'échéance de l'appelant est passée est refusée (`echue`) ; sinon
  son échéance est **la plus courte de** 5 s, celle de l'appelant, et la fin du permis du principal.
- À la décision d'une retenue : la décision vérifie elle-même l'échéance (dix minutes), sans
  attendre que le balayage passe : une approbation à l'instant de l'échéance est refusée. C'est ce
  que fait le code d'aujourd'hui (`hub.rs:1341`).
- À l'approbation : seule une retenue encore valide ouvre une échéance neuve, elle-même bornée par
  le permis.
- Dans la voie : le kit n'envoie jamais un ordre au-delà de son échéance moins sa marge (`MIN_LEFT`,
  `REPLY_MARGIN`).

### 12.3 La forme d'admission

Une demande garde **la forme sous laquelle elle a été acceptée** : ses arguments, sa confirmation,
son suivi se lisent dans cette forme jusqu'au bout, même si l'ordre est redéclaré pendant qu'elle
vit. Une retenue se lie à **la forme et aux cibles montrées** sur sa carte. À l'approbation, si
elles ont changé, la demande est revalidée contre la forme du moment : refusée si elle ne tient
plus ; sinon, **une nouvelle carte** est présentée (« forme changée »), et c'est elle qu'un humain
approuve. Un écran qui approuve encore l'ancienne carte est refusé.

### 12.4 Les plafonds

Chaque plafond est vérifié avant toute mutation, avec une erreur : 255 valeurs, ordres ou membres
par appareil ; 64 cibles par ordre de groupe ; 16 paramètres et 32 valeurs attendues par forme ;
65 535 formes ; **50 retenues** (aujourd'hui : `MAX_PENDING`) ; 4 096 demandes en cours, un groupe
réservant ses filles avant de partir. Un refus ne prend aucune case et ne publie rien.

### 12.5 Les reçus

Une fenêtre **par principal** : sa **plus haute séquence**, et ses 32 derniers reçus de 32 octets. À
l'arrivée d'une demande externe :

| La clé | Le verdict |
|---|---|
| déjà prise, même ordre, mêmes arguments | la même demande, la même issue : elle ne part pas deux fois |
| déjà prise, un autre ordre ou d'autres arguments | refusée, `conflit` |
| plus ancienne que la fenêtre (séquence ≤ la plus haute − 32) | refusée, `perimee` : on ne peut plus savoir si elle est partie |
| neuve | elle est prise |

C'est la règle d'aujourd'hui (`occurrences.rs:302`) ; la v1 l'avait affaiblie. Une fenêtre n'est
jamais cédée à un autre principal : le nombre de principaux est borné par leur table, et la mémoire
des reçus avec (65,0 Kio pour 64). **Toute** demande externe porte sa clé, pas seulement les
appuis : un doublon du transport ne refait rien. Les reçus voyagent dans la trace : le secours les
a.

### 12.6 Un appui et le secours

Une consigne rejouée ne change rien (la même valeur une seconde fois) ; un appui rejoué le refait.
D'où une règle de plus pour les appuis que le hub ne fait pas lui-même :

- **un appui attend l'accusé du secours avant de partir** : sa trace (sa clé, son reçu) au rang `s`,
  puis l'accusé du secours pour ce rang, puis seulement l'envoi au module. Le prix : un aller-retour
  sur le réseau local, quelques centaines de microsecondes, pour les seuls appuis ;
- **sans accusé au bout de 200 ms**, l'actif inscrit chez le témoin (celui de la HA v2) que le
  secours a décroché, **puis** envoie ; le secours qui a décroché se refait par l'état complet ;
- **sans secours**, l'appui part aussitôt.

Ce que cela garantit est au § 17.2.

### 12.7 Les textes privés

Un ordre dont la forme dit « texte privé » (une annonce, un message) :

- le texte est remplacé par **sa longueur** dans les arguments publics : sur `cmd/`, dans la trace
  sur `ord/`, au journal, dans l'état d'un client ;
- il est gardé au **tiroir privé** du hub, et ne voyage que par le **canal privé** : vers le pair
  (au cas où une retenue serait décidée après une bascule), et vers le module de l'appareil, qui le
  lit au moment d'envoyer ;
- il **s'efface à la conclusion**, chez l'actif comme chez le pair ;
- l'instantané du hub, sur son disque, garde ceux des demandes encore vivantes.

La soirée envoie deux textes témoins, une annonce et un message de l'assistant, et les cherche dans
chaque sortie publique, chaque commande, l'état d'un client et l'instantané : trouvés 0 fois ; lus
par le module, tenus par le pair, puis effacés partout.

### 12.8 En mesures

| Une demande, mesurée | ×1 | ×10 | ×100 |
|---|---|---|---|
| Prendre une case et la rendre | 7,3 ns | 7,2 ns | 7,4 ns |
| Retrouver une demande par son numéro | 0,81 ns | 0,81 ns | 0,65 ns |
| Une journée : demandes ; renvoyées avec la même clé ; refusées | 3 000 ; 116 ; 26 | 30 000 ; 1 226 ; 322 | 300 000 ; 11 919 ; 3 277 |
| Au plus, en même temps | 39 | 64 | 114 |
| La table des demandes ; les reçus | 4,8 ; 10,2 Kio | 4,8 ; 10,2 Kio | 9,5 ; 10,2 Kio |
| Processeur de toute la boucle (hub, kit, appareils simulés), la journée | 4,7 ms | 50 ms | 1 024 ms |
| … par demande | 1,6 µs | 1,7 µs | 3,4 µs |
| Sorties critiques dans la journée | 10 924 (1,8 Mo) | 112 629 (18,2 Mo) | 1 125 535 (181 Mo) |

Le temps processeur est celui de `getrusage`, sur la boucle entière ; il ne compte ni la
sérialisation, ni le courtier, ni le réseau, ni de vrais pilotes. La journée du banc est une
**hypothèse** : 3 000 demandes par jour à ×1, dont 30 % en rafales de 40 (des scènes) ; 60 %
viennent du moteur, 35 % d'un écran, 5 % d'agents et de scripts ; 2 % sont hors de leur forme ; une
demande externe sur dix est renvoyée avec sa clé. Le hub se réveille au plus proche ; rien ne tourne
entre deux.

### 12.9 Le cycle

![Le cycle d'une demande](ordres-appareils-v2/cycle.svg)

| État | Ce qui l'y amène | Ce qui l'en sort |
|---|---|---|
| (aucune case) | hors de sa forme, ordre inconnu ou retiré, hors des droits, échue, clé en conflit ou périmée, tentative ambiguë, un plafond | — : une ligne de journal, une issue `refuse` (ou `incertain`, ambiguë) |
| **Retenue** | la garde : un agent sur un ordre qui ouvre ou qui arrête une machine, une pièce protégée | un humain décide, sur l'écran, en nommant l'ordre et la forme vus ; l'échéance, le permis, la forme revérifiés ; ou 10 min passent ; ou le permis finit |
| **Remise** | la garde laisse passer ; un appui attend l'accusé du secours ; puis `cmd/` au module | l'issue du module, avant l'échéance |
| **Mère** | un ordre de groupe : elle attend ses filles, sans échéance à elle | la dernière fille conclue |
| **Suivie** | `accepte`, `envoye` ou `incertain`, quand la forme a un suivi | la suite, une fois ; ou le hub conclut à la fin du suivi |
| Conclue | toute autre issue, ou la suite | la case se libère, ses tiroirs aussi ; la fiche de l'ordre garde la dernière commande |

### 12.10 Les voies, côté module

Le kit de la V7 (§ 20.2) ne change pas ; il range ses voies par **numéro** :

- **une voie par appareil, qui n'existe que tant qu'elle porte un ordre** ;
- un ordre en vol ; jusqu'à 8 en attente ;
- une consigne qui attend cède à la suivante **du même ordre**, si rien ne les sépare ; un appui
  n'est jamais fusionné ;
- l'échéance revérifiée juste avant l'envoi ; un ordre ne part **qu'une fois** ; ce que le hub
  retire (un permis révoqué) sort de la voie s'il n'est pas parti ;
- **l'issue est bornée par le retour de la forme** : un volet RTS ne rend jamais « confirmé » ;
- **seule une lecture commencée après l'envoi prouve l'ordre** (§ 13.2), et une valeur en mouvement
  ne confirme qu'à l'arrêt.

## 13. La dernière commande, et sa preuve

### 13.1 La dernière commande

Chaque ordre garde, dans sa fiche de 40 octets, **sa dernière commande** : la dernière demande
admise pour lui (l'ordre d'admission décide), sa valeur, son dernier pas, qui, par où, et son issue.
Une issue ou une suite tardive d'une demande plus ancienne ne la remplace jamais. **« En cours »**
se compte à part : le nombre de demandes de cet ordre qui vivent, retenues, remises ou suivies.

| À quoi elle sert | Exemple |
|---|---|
| Montrer l'état d'un appareil qui n'en a pas | « Ouvrir : exécuté par la box, 18:20, par l'assistant » sur un volet RTS |
| Montrer ce qui est en cours, sur tous les écrans | « Chauffe-eau : 54 °C, accepté ; une autre consigne encore suivie » |
| Dire si la valeur suit la commande | § 13.2 et § 15 |
| Interroger depuis un workflow | la dernière commande, son origine, son âge, « en cours » (§ 15) |

Elle remplace l'attente de 8 secondes que garde chaque écran (`hub.svelte.js:17-18`), et l'écho
« optimiste » que la V7 voulait retirer de deux pilotes (Tapo et Sonos, § 5.9). Une place d'ordre
libérée par un lot remet sa fiche à zéro (§ 18).

### 13.2 La preuve

- **Seule une lecture commencée après l'envoi confirme** : le kit compare l'heure de début de la
  lecture à celle de l'envoi, comme aujourd'hui (`orders.rs:273`) ; après une reprise, le hub
  n'accepte pour une demande héritée qu'une lecture commencée après la reprise (le module qui
  démarre relit ses appareils) ; une lecture ancienne qui arrive en retard ne confirme rien.
- **La fiche tient la preuve, dans l'ordre des événements** : à l'envoi, le compte des variations de
  la valeur prouvée repart de zéro ; à la confirmation, la fiche note si la preuve est une
  variation, et si c'est la première ; ensuite, toute variation la marque « modifiée depuis ».
  Aucune horloge n'est comparée : l'ordre est celui du hub, rejoué tel quel par chaque copie.

## 14. Les appareils physiques et virtuels

**Premiers principes** : une valeur ou un ordre n'a qu'**un propriétaire**, l'appareil dont le
module, ou le hub, le produit. Un autre appareil peut le **montrer**, s'en **servir pour calculer**,
ou le **commander** : il le cite, il ne le possède pas.

![Appareils physiques et virtuels](ordres-appareils-v2/virtuels.svg)

| Ce qu'un appareil virtuel fait d'un autre | Le rôle | Exemple |
|---|---|---|
| Il montre une de ses valeurs ou un de ses ordres, à sa place dans sa page | **affiche** | l'« Entrée » montre la serrure, le volet RTS, la lampe |
| Une de ses valeurs se calcule avec | **source** | la température moyenne d'une pièce |
| Un de ses ordres de groupe le commande | **cible**, rattachée à cet ordre de groupe | « allumer le séjour » commande les quatre « on » ; « luminosité du séjour », les luminosités |
| C'est un appareil membre : la garde lit sa pièce | **membre** | une ampoule du séjour est dans la chambre |

Bloc technique — la composition (extrait du banc) :

```rust
/// Une entrée de composition : 8 o. Le numéro garde sa génération.
#[repr(C)]
pub struct Membre {
    pub numero: u32,      // une valeur, un ordre ou un appareil, selon `sorte`
    pub rang: u8,         // sa place dans la page de l'appareil virtuel
    pub sorte: u8,        // valeur, ordre, appareil
    pub role: u8,         // affiche, source, cible, membre
    pub source: u8,       // pour une cible : le rang de son ordre de groupe dans l'appareil virtuel
}
```

Les règles :

- **Une cible dit de quel ordre de groupe elle dépend.** Un groupe qui a « allumer » et
  « luminosité » ne mélange pas leurs cibles. Une cible est l'ordre d'un appareil **physique** (pas
  de groupe de groupes, donc pas de cycle), de forme compatible avec son ordre de groupe (même
  effet, mêmes paramètres, des bornes de groupe comprises dans celles de la cible) ; 64 au plus. Une
  composition invalide est refusée à la déclaration.
- **Un ordre de groupe est une demande mère et une fille par cible.** La mère connaît toutes ses
  cibles avant la première fille ; elle n'a pas d'échéance à elle (ses filles sont bornées) ; chaque
  fille est une demande ordinaire, du même principal, autorisée, avec son module, sa voie et son
  issue. La mère conclut quand toutes ont conclu, **au niveau de preuve le plus faible** :
  `confirme` si toutes sont confirmées, `execute` si toutes ont au moins été exécutées (quatre
  volets RTS), sinon `echec`, cause `partiel`, avec n/m (V7 G17). Aujourd'hui, un luminaire répond
  « fait » dès qu'une ampoule a obéi (`moli-lights/src/lib.rs:498`).
- **Un groupe retenu puis approuvé passe par le même chemin** : la mère approuvée se répartit sur
  les cibles de sa carte, et ses filles ne sont pas retenues de nouveau.
- **La progression passe la bascule** : la copie rejoue le compte de la mère à partir des traces des
  filles ; le nouvel actif conclut la mère avec le bon n/m.
- **Qui répartit** (ton choix du 24 octobre, § 22) : le hub, pour un groupe simple ; un module
  virtuel, quand il faut une stratégie (le relais en dernier recours). Le code d'aujourd'hui a déjà
  un coordinateur, celui des scènes (`scenes.rs`) : la revue suggère d'y loger d'abord la
  répartition. Le banc ne mesure que la répartition par le hub.
- **La garde lit toutes les pièces qu'un ordre atteint** : celle de l'appareil virtuel, et celles
  des appareils de ses cibles et de ses membres (la règle d'aujourd'hui, `rooms_of`, étendue aux
  cibles).
- **Une interface lit la page d'un appareil d'un seul geste** : 204 ns par page à ×1, noms compris ;
  « cette valeur est aussi dans… », 510 ns en parcourant les compositions.
- **Une référence inerte se dit, elle ne se remplace pas** : « retiré », « membre retiré ».

Dans la maison ×1 du banc : 29 appareils virtuels et 340 entrées, soit 2,7 Kio.

## 15. Les workflows interrogent les ordres

Le programme cite les valeurs par numéro (mémoire v2 § 8.2, règle 1) ; il cite désormais aussi les
ordres. Une règle reste une fiche de 16 octets ; son `genre` dit ce qu'elle lit.

![Les workflows interrogent les ordres](ordres-appareils-v2/workflows.svg)

| Ce qu'un workflow demande | La règle | Ce qu'elle lit | Mesuré |
|---|---|---|---|
| Quelle est la dernière commande ? | `DERNIERE` | la fiche de l'ordre : le premier paramètre | 0,37 ns |
| A-t-elle abouti ? qui l'a donnée ? il y a combien de temps ? | `ISSUE`, `ORIGINE`, `DEPUIS` | l'issue, l'origine, l'instant | la même fiche (non chronométré à part) |
| Un ordre est-il en cours ? | `EN_COURS` | les demandes vivantes de l'ordre, à part de l'issue | la même fiche (non chronométré à part) |
| La valeur suit-elle la dernière commande ? | `VIENT_DE` | oui, non, ou on ne sait pas (ci-dessous) | 1,3 ns |
| Réagir quand un ordre part ou tombe | un déclencheur sur l'ordre | l'index par ordre, comme l'index par point | à la trace |

**`VIENT_DE` répond, et dit quand il ne sait pas** :

| La réponse | Quand | L'exemple de la soirée |
|---|---|---|
| **oui** | la dernière commande est confirmée, la première variation de la valeur après l'envoi est celle demandée, et rien n'a changé depuis | 20:16 : la maison rallume la lampe éteinte |
| **non** | la valeur a changé depuis la preuve ; ou la commande a échoué, a été refusée ou n'est pas partie ; ou jamais commandé | 20:12 : quelqu'un a éteint et rallumé à l'interrupteur |
| **on ne sait pas** | pas de preuve (exécuté, accepté, envoyé, incertain, en cours) ; la valeur était déjà là avant la commande ; plusieurs variations avant la preuve | 20:41 : une teinte déjà là ; 20:43 : un geste à la main pendant la latence |

C'est une **compatibilité**, pas une causalité : si un humain fait exactement le geste demandé
pendant la latence de l'appareil (quelques centaines de millisecondes), la première variation est la
sienne, et rien ne permet de la distinguer de celle de l'ordre. Un protocole qui donne la cause
(certains appareils Zigbee disent « physique » ou « numérique ») permettrait de répondre mieux ;
sans elle, la règle est conservatrice : un workflow qui veut « éteindre ce que la maison a allumé »
ne le fait que sur **oui**.

Trois usages que le banc rejoue :

- **Éteindre ce que la maison a allumé, pas ce qu'un humain a rallumé.** À 20:00, « Extinction
  auto » laisse la lampe : un humain l'avait allumée (`ORIGINE`). À 20:10, quelqu'un l'éteint et la
  rallume à l'interrupteur : non. À 20:15, la maison la rallume : oui ; à 20:40, l'extinction
  l'éteint.
- **Réagir à un ordre sans valeur.** « Quand on ferme le volet RTS du salon, fermer celui de la
  chambre » : seul un déclencheur sur son ordre peut y réagir.
- **Être prévenu d'un ordre qui n'aboutit pas.** « Si le verrouillage de la porte n'est pas
  confirmé » : un déclencheur sur l'ordre, avec les issues `envoye`, `incertain` et `echec`.

## 16. Les clients se chargent par MQTT

### 16.1 Ta remarque, et ton choix

Tu avais prévu que les clients passent par un module dédié : la V7 a son module WebSocket.
Maintenant que l'appareil, ses ordres et ses valeurs sont tous en mémoire, **un client peut se
charger et se recharger directement par MQTT**, comme le secours de la HA v2. Tu l'as choisi le 24
octobre. La revue l'a gardé, avec deux exigences que la v2 tient : une autorisation vivante (§ 11)
et une continuité vérifiable pour un écran filtré (§ 16.3).

![Les clients se chargent par MQTT](ordres-appareils-v2/clients.svg)

### 16.2 Trois façons, côte à côte

| | Aujourd'hui : SSE et REST | La V7 : un module WebSocket | **MQTT direct** |
|---|---|---|---|
| L'état au démarrage | `/api/snapshot`, toute la maison en JSON : 593 Ko à ×1 | l'état filtré, par blocs de 64 Kio | `etat/demande` → l'état du client, filtré par ses droits : 113 Ko, en 28 morceaux de 4 Kio ; 20,5 Ko pour deux pièces |
| Le direct | un événement SSE par changement | le module filtre `val/` et pousse des trames | l'abonnement aux sujets permis : `meta/`, `meta/forme/`, `ord/`, `val/`, `avail/` |
| Un trou | tout recharger | `retard`, puis un nouvel état | le rang de la pièce le montre ; l'état de la pièce, rechargé |
| Un ordre et son issue | `POST /api/command` ; l'issue par le flux | sur la connexion | `req/<compte>` ; l'issue sur `res/<compte>` |
| Les droits | une session par requête ; le permis d'un invité réévalué chaque seconde (`sse.rs:101`) | la session, à l'ouverture | le compte de la connexion ; le permis revérifié à chaque frontière ; à son changement, le courtier coupe et l'écran se recharge |
| L'historique, les médias | REST, compressé | par blocs | REST, comme aujourd'hui |
| Ce que le hub construit | tout, à chaque fois : 690 µs | le module garde ses écrans | l'état d'un client, à la demande : 85 µs à ×1 |

### 16.3 Un écran filtré voit ses trous

Le rang `s` numérote **toutes** les sorties critiques : un écran du salon qui reçoit 10 puis 12 ne
sait pas si 11 concernait une chambre qu'il ne voit pas ou s'il l'a perdu. La v2 donne à chaque
sortie **le rang de sa pièce** (`sp`) : un compteur par pièce, plus un pour ce qui concerne tout le
monde (les formes, le lot) ; le canal privé et les baux du moteur ont le leur, que seul le pair
voit. L'état d'un client porte, en tête, l'époque, le rang `s` et le rang de chacune de ses pièces.
Un trou dans une pièce fait recharger son état ; ce que l'état chargé contient déjà n'est pas un
trou ; une époque nouvelle (une bascule, un redémarrage) fait se recharger. Le prix : quatre octets
par sortie, un compteur par pièce.

Ce que la soirée rejoue : un écran du salon perd un message ; il voit le trou au message suivant de
sa pièce, et se recharge : 1 trou, aucun faux trou venu d'une autre pièce, 4 chargements en tout (le
premier, après le trou, après la bascule, après le redémarrage), et à la fin les mêmes dernières
commandes que le hub.

### 16.4 Ce qu'un écran charge, à contenu égal

L'état d'un client porte ce que porte `/api/snapshot` : les appareils, leur disponibilité, leurs
valeurs avec leur nature, leur unité, leur échelle et leur dictionnaire, leurs ordres avec leur
dernière commande et « en cours », leur composition, les retenues qu'il voit, les libellés. Le gain
se décompose :

| Ce qu'un écran charge, maison ×1 (×10) | Ko | Ce qui change |
|---|---|---|
| Aujourd'hui, `/api/snapshot`, JSON | 593 (5 971) | — |
| Le même contenu, proposé, en JSON | 140 (1 440) | **le partage** (chaque forme, chaque dictionnaire, chaque libellé une fois) et des tableaux au lieu de noms de champs répétés |
| Le même contenu, en binaire | 113 (1 133) | **l'encodage** |
| Un écran de deux pièces | 20,5 (189) | **le filtrage** |

Le transport (MQTT sur WebSocket, contre SSE) n'est pas mesuré ici.

### 16.5 Ce qu'il coûte, et ce qu'il faut régler

| Une connexion d'écran | Par écran | 64 écrans (le plafond) | D'où vient le chiffre |
|---|---|---|---|
| SSE d'aujourd'hui | 39,1 Kio | 2,5 Mio | V7 § 14.2.5, mesuré |
| WebSocket réglé (le module de la V7) | 11 Kio | 0,7 Mio | idem |
| MQTT, `rumqttd` tel quel | ≈ 62 Kio | ≈ 3,9 Mio | V7 § 11.4.2, mesuré : deux tampons de 200 éléments alloués d'avance par connexion |
| MQTT, notre version de `rumqttd`, tampons de 32 | ≈ 10 Kio, plus le cadre WebSocket | ≈ 0,7 Mio | **estimé**, à mesurer |

Ce qu'il faut, en plus de ce que la V7 prévoit déjà (des comptes et des ACL par sujet) :

- un **écouteur WebSocket** du courtier (`rumqttd` 0.20 l'a : la fonctionnalité `websocket`, active
  par défaut) ;
- **des tampons par connexion réglables** dans notre version de `rumqttd` (aujourd'hui une
  constante, `MAX_CHANNEL_CAPACITY` = 200) ;
- **l'état d'un client, filtré par ses droits**, construit par le hub à la demande ;
- **la coupure d'un abonnement** quand un permis change (le courtier déconnecte, l'écran se
  recharge) ;
- **un sujet de réponse par compte**, `res/<compte>` ;
- côté navigateur, **une bibliothèque MQTT sur WebSocket**, à choisir et à mesurer.

Sans le réglage des tampons, le choix tient encore : 3 Mio de plus à 64 écrans, dans le budget de
32 Mio (V7 § 11.3).

## 17. La haute disponibilité

![La bascule](ordres-appareils-v2/bascule.svg)

### 17.1 Ce qui passe au secours

| Ce qui passe | Comment | Pourquoi |
|---|---|---|
| Le catalogue : registres, formes, clés, fiches des ordres, listes, compositions | `meta/`, `meta/forme/`, au rang `s` | le secours connaît l'appareil, l'ordre et sa structure, comme l'actif |
| Chaque pas d'une demande remise ou retenue | `ord/`, au rang `s`, avec la fiche de la demande, celle de l'ordre, le reçu, la carte ; l'échéance en heure murale | le secours sait ce qui a pu partir, tient la dernière commande et les reçus |
| La progression d'un groupe | rejouée par la copie à partir des traces des filles | la mère conclut juste après une bascule |
| Les textes privés des demandes vivantes | `prive/`, au rang `s`, vers le seul pair | une retenue de message décidée après une bascule |
| Les retenues, les baux | `ord/` et `wf/`, comme le prévoit la HA v2 | une décision humaine et un rétablissement ne se perdent pas |
| Un appui | son accusé, avant l'envoi | § 17.2 |
| Les demandes refusées | jamais : une ligne d'historique seulement (`run/`) | elles n'ont jamais quitté le hub |

### 17.2 Un appui ne part ni deux fois, ni en silence

La HA v2 admet qu'une bascule perde les dernières sorties en vol (§ 6.5). Une trace copiée **après**
ne protège donc pas un appui fait **avant** : c'était le bloquant B2. La v2 :

| Le cas | Ce qui se passe | La garantie |
|---|---|---|
| Le secours est en phase | l'appui attend l'accusé de sa trace ; l'actif tombe avant l'accusé : l'appui n'est jamais parti | le secours promu connaît tout appui parti ; une nouvelle tentative inconnue de lui n'est jamais partie : elle part, une fois. **Reprise sûre** |
| Le secours décroche | au bout de 200 ms sans accusé, l'actif l'inscrit chez le témoin, **puis** envoie | le secours promu lit le témoin : **reprise non sûre** |
| Pas de secours | l'appui part aussitôt | un redémarrage sur un instantané pris avant l'appui est une **reprise non sûre** ; un arrêt figé, dont l'instantané est le dernier, est sûr |
| Une reprise non sûre | une nouvelle tentative d'un appui dont la première est antérieure à la reprise, et que la mémoire ne connaît pas, est refusée : « incertain, ambiguë » | **au plus une fois** : jamais refait ; jamais perdu en silence. Une consigne retentée, elle, repart (la même valeur deux fois ne change rien) ; une demande neuve aussi |

Le client porte, dans sa demande, la tentative et l'heure de la première : ce sont elles qui disent
qu'une tentative précède la reprise.

### 17.3 À la bascule, au redémarrage

| Ce qui était en cours | Après | Le banc |
|---|---|---|
| Une demande remise (partie ou encore dans la voie) | « incertain », cause `bascule` ; **jamais renvoyée** ; le nouvel actif la suit 30 s au moins, et la confirme sur `val/` par une lecture faite après la reprise | la luminosité partie 100 ms avant : confirmée ; la position restée dans la voie : incertaine, jamais partie, et une vieille lecture de 60 ne la confirme pas |
| Un appui qui attendait l'accusé | jamais parti ; une nouvelle tentative, sur une reprise sûre, part une fois | le « volume + » de 20:59:59.97 |
| Une demande suivie | sa suite arrive au nouvel actif, qui conclut | le chauffe-eau, accepté avant, confirmé après |
| Une mère | elle garde son compte ; elle conclut avec le bon n/m | le séjour éteint à 20:59:58 : 3 sur 4 |
| Une retenue | la carte reste ; l'humain décide sur le nouvel actif ; tout est revérifié | décidée à 21:05 ; l'autre expire à 21:09:40, à la seconde, même devant une approbation |
| Un bail | rétabli à son terme, par le nouvel actif | 21:01:30 |
| Les numéros | le nouvel actif prend une **époque** nouvelle ; un redémarrage aussi | une issue de l'ancien monde n'attrape rien |
| Les copies et les écrans | une époque nouvelle les fait se recharger par l'état complet | la copie reprise à 19 h 45 se refait sur le nouvel actif |

Le redémarrage suit la même règle (HA v2 § 16) : l'arrêt figé donne une seconde aux demandes
remises, puis les dit incertaines ; un appui qui attendait encore l'accusé ne part pas. L'instantané
range le catalogue, les demandes, les reçus, les retenues et leurs cartes, les textes privés des
demandes vivantes, les baux et les exécutions du moteur ; **la machine repart de ces seuls octets**
et de la configuration (les comptes, les pièces protégées). La v1 reprenait les baux à l'ancien
moteur.

## 18. Le nettoyage, et une structure qui change

| Ce qui arrive | Ce que deviennent les ordres |
|---|---|
| **Retirer** un appareil (nettoyage § 9) | ses ordres ne sont plus commandés : refus, cause `retire` ; ses retenues et ses baux se terminent ; les demandes déjà remises vont à leur issue ; les automatismes qui le citent passent « à revoir » |
| **Le lot** | les places de ses ordres sont libérées, comme celles de ses valeurs ; **la fiche de l'ordre est remise à zéro** ; un vieux numéro ne commande jamais l'appareil neuf ; les listes et les compositions mortes sont tassées, au même rang chez chaque copie |
| **Une redéclaration** | les valeurs et les ordres **gardent leur numéro par clé** ; une clé nouvelle prend une place ; une clé qui n'est plus déclarée est retirée ; une forme nouvelle remplace l'ancienne pour l'ordre, et les demandes en cours gardent la leur ; le programme se recompile ; un automatisme dont la valeur ne tient plus passe « à revoir » |
| **La succession** (nettoyage § 10) | les définitions sont récrites par les noms ; une forme incompatible met l'automatisme « à revoir » |
| **Le déménagement** | les ordres suivent comme les valeurs : fiches, listes, compositions, demandes, retenues, reçus et baux |

La soirée rejoue une redéclaration de la lampe du salon : une valeur ajoutée au milieu (`energy`),
une retirée (`linkquality`). Les trois autres gardent leur numéro, la nouvelle en prend un, la
retirée n'est plus vivante, et une luminosité demandée ensuite est prouvée par la luminosité, pas
par la valeur qui a pris son rang.

## 19. Ce que tout consomme : le budget revérifié

### 19.1 Un témoin honnête

La v1 construisait la maison d'aujourd'hui depuis le nouveau modèle : un point `Button` par ordre
d'un volet RTS, un état pour chaque point. La v2 suit **la convention des pilotes réels** :

- une consigne écrit sa valeur : le point est lisible et écrivable, avec les bornes de l'ordre ;
- les appuis d'un appareil partagent **un seul point `control`**, une énumération en écriture seule
  (le volet RTS d'`overkiz`, `model.rs:145`) ;
- un texte a son point en écriture seule ; un ordre à paramètre sans valeur réglée aussi ;
- **aucun état pour un point en écriture seule** (le test d'`overkiz` l'exige, `model.rs:604`).

La maison du banc donne aux volets RTS quelques valeurs génériques qu'un vrai n'a pas : elles sont
les mêmes des deux côtés. Les validations sont chronométrées sur **le même corpus** : 1 025
intentions (valides, et hors bornes pour un paramètre numérique), traduites dans la forme
d'aujourd'hui ; les deux décident pareil sur 1 025, avant tout chronomètre. Le pas, la longueur d'un
texte et le nombre d'arguments, qu'aujourd'hui ne vérifie pas, n'y sont pas.

### 19.2 Le budget

Tu as demandé de revérifier ce que cette revue de la mémoire consomme, par rapport aux assistants,
aux autres composants et aux anciennes versions. Chaque ligne dit d'où vient son chiffre.

| Ce qui est en mémoire, maison ×1 (×10) | Combien | D'où vient le chiffre |
|---|---|---|
| **Aujourd'hui, Sarah Home entière, installation réelle** | 4,0 à 7,2 Mio de mémoire anonyme, pour une maison de 71 appareils et plus | `docs/PERFORMANCE.md`, mesuré |
| Le budget du cœur | 32 Mio | idem |
| La V7 écrite avec les types d'aujourd'hui ; optimisée | 23,5 (136,6) ; 6,5 (27,0) Mio | V7 § 11.4.1, mesuré |
| … dont le courtier, optimisé | 4,30 (17,51) Mio | idem |
| … dont les valeurs, remesurées par la mémoire v2 (colonnes, santé, échéances, annuaire) | 0,27 (2,66) Mio | mémoire v2 § 5.1, mesuré |
| … dont les workflows, remesurés par la mémoire v2 | 0,23 (2,2) Mio | mémoire v2 § 8.5, mesuré |
| En plus de la V7 : la file du pont de la HA v2 (10 000 sorties de 46 o) | 0,45 Mio | HA v2 § 6.5, mesuré |
| **En plus : les ordres (registre, catalogue, demandes, reçus, journal, 50 retenues)** | 128 Kio (0,9 Mio) | ce banc, compteur d'allocations |
| **En plus : les listes et compositions des appareils** | 14,6 Kio (149 Kio) | ce banc : 4 o par valeur et par ordre, 8 o par entrée |
| La borne des reçus, 64 principaux | 65,0 Kio | calculée (`size_of`) |
| Les connexions des écrans, 64 au plus | 0,7 à 3,9 Mio selon le transport (§ 16.5) | V7, mesuré ; estimé pour MQTT réglé |
| **Ce que l'analyse retire** : le journal en textes, les retenues en textes, la copie de l'assistant | 125 Kio en permanence, 317 Kio par requête de l'assistant | ce banc |

Et à côté :

| Ce qui n'est pas le cœur | Combien | D'où |
|---|---|---|
| Home Assistant, sur la même machine | ≈ 2,1 Gio | `docs/PERFORMANCE.md`, mesuré |
| Le compagnon Hermes, hors du cœur, s'il est installé | 1 à 4 Go | la note « assistant-agents » du projet |
| L'inventaire que l'assistant envoie au modèle, à chaque requête | 17 988 jetons aujourd'hui ; 15 070 avec les formes, les unités et les choix | ce banc, **estimé** : un jeton pour quatre caractères, aucun tokeniseur hors ligne |

Ce que le tableau dit :

- **Toute la revue de la mémoire (v2, nettoyage, HA v2, ordres) tient dans le budget** : à ×1, la V7
  optimisée pèse 6,5 Mio ; les ordres, les listes et les compositions y ajoutent 0,14 Mio. Mais ces
  chiffres viennent de bancs distincts : **le budget de 32 Mio reste à vérifier sur le cœur
  intégré**, à l'échelle de la maison (§ 23, marche 7). ×100 sert d'essai de croissance, pas de
  certificat.
- **Par rapport aux anciennes versions**, la mémoire des ordres ne revient pas sur les gains : elle
  remplace une description 2,2 fois plus lourde, et des textes de journal et de retenues.
- **Par rapport aux assistants**, le cœur est minuscule : Sarah Home entière pèse un trois-centième
  de Home Assistant, et le compagnon Hermes, s'il existe, des centaines de fois le cœur. Pour
  l'assistant, la mémoire des ordres change surtout deux choses : il ne copie plus la maison à
  chaque question, et son inventaire perd 16 % de ses jetons, en gardant les unités et les choix. Le
  gain est modeste : les noms dominent l'inventaire.

## 20. La vérification de bout en bout

### 20.1 La soirée

Un actif, sa copie de secours (qui accuse chaque rang), une copie reprise par l'état complet à 19 h
45, un écran à cache, un écran du salon qui suit le rang de sa pièce ; huit principaux ; la maison
×1 du banc, plus quinze appareils nommés, dont deux virtuels ; neuf automatismes. En gras, ce que la
v2 ajoute.

| Heure | Ce qui arrive | Ce que la mémoire des ordres doit faire |
|---|---|---|
| 18:00 | la maison se déclare | catalogues identiques chez le secours et chez l'écran ; l'écran a demandé chaque forme une fois ; il valide comme le hub (3 344 décisions sur 3 344) |
| 18:05 | la lampe ; une luminosité de 300 ; puis 128 | confirmée ; refusée par le hub, jamais envoyée ; confirmée |
| 18:10 | un mouvement au couloir | un bail : la veilleuse, 2 min |
| 18:20 | l'assistant ouvre le volet RTS | « exécuté » par la box ; la dernière commande, sur un appareil sans valeur |
| **18:22** | **un invité, le bureau seulement** | **la porte et le groupe refusés (`droits`) ; l'imprimante du bureau, oui ; la demande porte l'invité** |
| 18:30 → 18:41 | un agent veut tirer le pêne, puis mettre l'imprimante en pause | deux retenues ; la mauvaise carte refusée ; la bonne approuvée (l'appui attend l'accusé du secours) ; l'autre expire |
| **18:35 → 18:50** | **l'agent d'un gardien, l'entrée jusqu'à 18 h 40, veut tirer le pêne** | **retenu ; refusé à 18:40 (`droits`) ; l'approbation de 18:50 échoue ; jamais tiré** |
| 18:40 → 18:43 | le groupe du séjour ; le même par un agent, **approuvé** | « échec, partiel », 3 sur 4 ; **l'approuvé se répartit, quatre filles, aucune retenue de nouveau, 3 sur 4** |
| 18:45 | trois « volume + », puis trois consignes ; **l'écran du salon perd un message** | les appuis partent dans l'ordre ; 20 et 25 remplacés ; 30 confirmé ; **l'écran voit le trou et se recharge** |
| **18:46 → 18:47** | **une annonce et un message privés** | **ni sur `cmd/`, ni sur `ord/`, ni dans un état ; lus par le module, tenus par le pair, effacés** |
| 18:50 | une sonnette, par un script qui retente avec la même clé | un seul appui, la même issue |
| **18:52** | **une demande déjà échue** | **refusée (`echue`)** |
| 19:00 → 19:20 | l'aspirateur ; le chauffe-eau, **puis une autre consigne depuis un écran** ; la télé endormie | accepté puis confirmé ; **la suite tardive de la première ne remplace pas la seconde dans la fiche, « en cours » vaut 1** ; envoyé puis confirmé |
| 19:30 | une pause de l'aspirateur, et son module meurt | « incertain », cause `module`, jamais renvoyé |
| 19:35 | une issue tardive pour une ancienne occupante d'une case | écartée par la génération |
| 20:00 → 20:43 | « Extinction auto », l'interrupteur, « Lumière du soir », **une teinte déjà là, un geste pendant la latence** | **non à 20:12, oui à 20:16 ; éteinte à 20:40 ; on ne sait pas à 20:41 et à 20:43** |
| 20:45 → 20:50 | on ferme le volet RTS ; la serrure ne confirme pas | « Volets du soir » réagit à l'ordre ; « Alerte serrure » part sur `envoye` |
| **20:59:58** | **le groupe du séjour, éteint** | **trois ampoules confirmées avant la bascule, comptées par la copie** |
| 21:00 | **le pont se coupe ; un « volume + » 30 ms avant** ; l'actif s'arrête net | le secours devient l'actif : l'ordre parti, incertain puis confirmé ; celui de la voie, incertain, jamais parti, **et une vieille lecture ne le confirme pas** ; **le « volume + » n'est jamais parti ; retenté à 21:06:30, il part une fois** ; **le groupe conclut 3 sur 4** ; le bail rétabli à 21:01:30 ; une retenue décidée à 21:05, **l'autre refusée à 21:09:40, même devant une approbation** ; la sonnette retentée ne part pas |
| 22:00 | arrêt figé, instantané, redémarrage **depuis les seuls octets** | la retenue, le suivi, l'ordre vers la télé et le bail passent ; le bail, échu pendant l'arrêt, est rétabli avec 3 s de retard |
| 22:20 | le volet RTS est retiré | refusé ; deux automatismes « à revoir » ; l'Entrée le montre « retiré » |
| un mois plus tard | le lot ; un volet io neuf ; le chauffe-eau se redéclare à 60 °C au plus ; **la lampe se redéclare avec une valeur en plus et une en moins** | fiches à zéro ; un vieux numéro refusé ; 62 °C refusé ; 63 °C revalidé et refusé ; **55 °C : une nouvelle carte, l'ancienne refusée, la nouvelle approuvée et confirmée** ; **les numéros suivent les clés** |

### 20.2 Ce que les tests exigent

Les 28 tests de la soirée :

| Test | Ce qu'il exige |
|---|---|
| `chacun_connait_l_appareil_l_ordre_et_sa_structure` | la même mémoire chez l'actif, le secours et la copie par état complet ; les mêmes dernières commandes chez l'écran ; l'écran valide comme le hub ; une forme demandée une fois |
| `les_valeurs_et_les_ordres_ne_se_melangent_pas` | un volet RTS sans valeur, avec quatre ordres |
| `une_valeur_hors_de_sa_forme_n_atteint_aucun_module` | 300 sur 0–254, 62 °C sur 40–60 : refusés au hub |
| `chaque_issue_tombe_sur_sa_demande` | les issues attendues ; l'issue tardive écartée |
| `les_consignes_se_fusionnent_les_appuis_jamais` | trois appuis exécutés, deux consignes remplacées |
| `une_retenue_n_approuve_que_ce_qu_elle_montre` | la mauvaise carte, l'expiration, la bascule, le redémarrage, la revalidation, l'expiration à la seconde |
| `un_invite_ne_commande_que_ses_pieces` | B1 |
| `une_retenue_ne_garde_pas_une_autorisation_echue` | I1 |
| `une_echeance_se_verifie_a_chaque_frontiere` | I2 |
| `une_carte_ne_s_approuve_que_sous_sa_forme` | I3 |
| `aucun_ordre_n_est_rejoue` | chaque envoi une fois ; module mort, bascule, arrêt |
| `une_lecture_d_avant_la_reprise_ne_confirme_pas` | I4 |
| `un_appui_n_est_ni_perdu_ni_double_par_la_bascule` | B2 |
| `la_derniere_commande_se_lit_partout_pareil` | les fiches de l'écran égales à celles du hub ; le volet RTS |
| `la_derniere_commande_est_la_derniere_admise` | I6 |
| `un_appui_retente_ne_part_qu_une_fois` | une sonnette, même retentée après la bascule |
| `un_bail_se_retablit_apres_la_bascule_et_le_redemarrage` | trois rétablissements, dont un en retard, depuis l'instantané seul (I20) |
| `les_workflows_interrogent_les_ordres` | I5 : non, oui, on ne sait pas, deux fois |
| `un_appareil_virtuel_compose_sans_posseder` | les pages, « 3 sur 4 », « aussi dans » |
| `un_groupe_approuve_se_repartit` | I7 |
| `un_groupe_coupe_par_la_bascule_garde_son_compte` | I8 |
| `un_appareil_retire_n_est_plus_commande_et_sa_place_ne_garde_rien` | refus, « à revoir », fiches à zéro, vieux numéro refusé |
| `une_structure_qui_change_met_l_automatisme_a_revoir` | la redéclaration ; l'écran à vieux cache |
| `une_redeclaration_garde_les_numeros_par_cle` | I11 |
| `la_copie_et_l_etat_complet_finissent_identiques` | les empreintes ; l'instantané relu identique |
| `un_texte_prive_ne_sort_que_par_le_canal_prive` | B5 |
| `un_ecran_de_piece_voit_ses_trous` | I13 |
| `le_journal_dit_pourquoi` | les avis, les départs |

Les dix essais ciblés (`essais.rs`) : la reprise non sûre (B2), le secours décroché (B2), les filles
du hub qui concluent aussitôt (I8, I9), la mère sans échéance (I9), les compositions invalides (I10,
I12), deux ordres de groupe et leurs cibles (I10), les plafonds (I12), la révocation (I1), le permis
revérifié à l'approbation (I1), la lecture d'avant l'envoi (I4). Et les tests unitaires : le
catalogue (dont le corpus fixe, I14), les demandes (les reçus, B3 ; les cases condamnées et les
époques, B4), et les six du registre repris du nettoyage. En tout, 52 tests.

### 20.3 La preuve qu'ils savent échouer

Un test vert ne prouve rien s'il ne sait pas échouer : **53 fautes injectées** : 18 de la v1,
reprises ; 35 nouvelles, au moins une par constat de conception de la revue (B1 à B5, I1 à I14,
I20), dont deux remplacent celles de la v1 sur la forme d'une retenue et sur « la valeur vient-elle
de l'ordre ». Chacune doit faire échouer au moins un test
(`tools/ordres-banc/sorties/mutations.txt`).

| Faute injectée | Tests en échec |
|---|---|
| une issue tardive touche la demande qui occupe la case (génération ignorée) | 2 : `une_case_reutilisee_change_de_generation`, `chaque_issue_tombe_sur_sa_demande` |
| le hub ne valide plus les arguments contre la forme | 1 : `une_valeur_hors_de_sa_forme_n_atteint_aucun_module` |
| la voie fusionne aussi les appuis | 1 : `les_consignes_se_fusionnent_les_appuis_jamais` |
| la voie fusionne une consigne par-dessus un autre ordre | 1 : `les_consignes_se_fusionnent_les_appuis_jamais` |
| une décision tombe sur une autre carte que celle montrée | 1 : `une_retenue_n_approuve_que_ce_qu_elle_montre` |
| la bascule renvoie ce qui était remis à un module | 2 : `aucun_ordre_n_est_rejoue`, `une_lecture_d_avant_la_reprise_ne_confirme_pas` |
| un module mort rend « non envoyé » au lieu d'« incertain » | 1 : `aucun_ordre_n_est_rejoue` |
| la copie ne tient pas la dernière commande des ordres | 3 : `la_copie_et_l_etat_complet_finissent_identiques`, `la_derniere_commande_se_lit_partout_pareil`, `chacun_connait_l_appareil_l_ordre_et_sa_structure` |
| la copie ne tient pas les reçus | 2 : `la_copie_et_l_etat_complet_finissent_identiques`, `chacun_connait_l_appareil_l_ordre_et_sa_structure` |
| la copie garde l'échéance monotone de l'actif | 3 : `aucun_ordre_n_est_rejoue`, `une_echeance_se_verifie_a_chaque_frontiere`, `une_retenue_n_approuve_que_ce_qu_elle_montre` |
| le lot ne remet pas à zéro la fiche d'un ordre libéré | 1 : `un_appareil_retire_n_est_plus_commande_et_sa_place_ne_garde_rien` |
| un client à vieux cache commande le nouvel occupant de la place | 1 : `un_appareil_retire_n_est_plus_commande_et_sa_place_ne_garde_rien` |
| la garde ne lit que la pièce de l'appareil virtuel, pas celles de ses cibles et membres | 1 : `un_groupe_approuve_se_repartit` |
| un ordre de groupe se dit confirmé dès qu'une cible a obéi | 3 : `un_groupe_coupe_par_la_bascule_garde_son_compte`, `un_groupe_approuve_se_repartit`, `un_appareil_virtuel_compose_sans_posseder` |
| le hub ne suit plus les demandes héritées d'une bascule ou d'un arrêt | 1 : `aucun_ordre_n_est_rejoue` |
| une valeur en mouvement confirme en passant par la cible | 1 : `aucun_ordre_n_est_rejoue` |
| un bail échu pendant un arrêt n'est plus rétabli | 1 : `un_bail_se_retablit_apres_la_bascule_et_le_redemarrage` |
| un écran garde l'ancienne structure d'un ordre redéclaré | 1 : `une_structure_qui_change_met_l_automatisme_a_revoir` |
| B1 · un invité commande hors de ses pièces | 2 : `une_approbation_reverifie_le_permis`, `un_invite_ne_commande_que_ses_pieces` |
| B1 · la demande prend la personne que le client déclare | 1 : `un_invite_ne_commande_que_ses_pieces` |
| B2 · un appui part sans attendre l'accusé du secours | 2 : `un_secours_qui_decroche_est_inscrit_avant_l_envoi`, `un_appui_n_est_ni_perdu_ni_double_par_la_bascule` |
| B2 · une tentative ambiguë est refaite après une reprise non sûre | 2 : `un_secours_qui_decroche_est_inscrit_avant_l_envoi`, `une_reprise_non_sure_refuse_une_tentative_ambigue` |
| B2 · l'actif envoie sans inscrire le décrochage chez le témoin | 1 : `un_secours_qui_decroche_est_inscrit_avant_l_envoi` |
| B3 · une clé plus ancienne que la fenêtre repart | 1 : `un_appel_retente_retrouve_son_recu_et_un_vieux_rejeu_est_refuse` |
| B3 · la même clé pour un autre ordre passe pour la même demande | 1 : `un_appel_retente_retrouve_son_recu_et_un_vieux_rejeu_est_refuse` |
| B4 · un numéro d'une autre époque désigne la demande de la case | 1 : `un_numero_d_une_autre_epoque_ne_designe_rien` |
| B5 · un texte privé voyage en clair sur cmd/ et ord/ | 1 : `un_texte_prive_ne_sort_que_par_le_canal_prive` |
| B5 · un texte privé reste en mémoire après la conclusion | 3 : `chacun_connait_l_appareil_l_ordre_et_sa_structure`, `la_copie_et_l_etat_complet_finissent_identiques`, `un_texte_prive_ne_sort_que_par_le_canal_prive` |
| I1 · l'approbation ne revérifie pas le permis | 1 : `une_approbation_reverifie_le_permis` |
| I1 · la fin d'un permis ne fait rien tomber | 1 : `une_retenue_ne_garde_pas_une_autorisation_echue` |
| I1 · une révocation laisse partir ce qui attend dans la voie | 1 : `une_revocation_retire_ce_qui_n_est_pas_parti` |
| I2 · l'échéance de l'appelant est ignorée | 1 : `une_echeance_se_verifie_a_chaque_frontiere` |
| I2 · une retenue échue s'approuve tant que le balayage n'est pas passé | 2 : `une_retenue_n_approuve_que_ce_qu_elle_montre`, `une_echeance_se_verifie_a_chaque_frontiere` |
| I3 · une approbation vaut pour une forme qu'elle n'a pas montrée | 2 : `une_carte_ne_s_approuve_que_sous_sa_forme`, `une_retenue_n_approuve_que_ce_qu_elle_montre` |
| I4 · une lecture d'avant la reprise confirme une demande héritée | 1 : `une_lecture_d_avant_la_reprise_ne_confirme_pas` |
| I4 · une lecture d'avant l'envoi prouve l'ordre en vol | 1 : `une_lecture_commencee_avant_l_envoi_ne_prouve_rien` |
| I5 · une variation avant la preuve passe pour l'œuvre de l'ordre | 1 : `les_workflows_interrogent_les_ordres` |
| I5 · une confirmation sans variation passe pour l'œuvre de l'ordre | 1 : `les_workflows_interrogent_les_ordres` |
| I5 · un geste à la main après la preuve passe inaperçu | 1 : `les_workflows_interrogent_les_ordres` |
| I6 · une issue tardive remplace la dernière commande | 1 : `la_derniere_commande_est_la_derniere_admise` |
| I7 · un groupe approuvé ne passe pas par le répartiteur | 1 : `un_groupe_approuve_se_repartit` |
| I8 · la copie ne rejoue pas la progression d'un groupe | 3 : `la_copie_et_l_etat_complet_finissent_identiques`, `chacun_connait_l_appareil_l_ordre_et_sa_structure`, `un_groupe_coupe_par_la_bascule_garde_son_compte` |
| I8 · des exécutions font une confirmation | 1 : `un_groupe_attend_toutes_ses_cibles_meme_conclues_aussitot` |
| I9 · la mère conclut dès sa première fille | 5 : `un_groupe_attend_toutes_ses_cibles_meme_conclues_aussitot`, `une_mere_n_a_pas_d_echeance`, `un_appareil_virtuel_compose_sans_posseder`, `un_groupe_coupe_par_la_bascule_garde_son_compte`, `un_groupe_approuve_se_repartit` |
| I9 · la mère garde une échéance à elle | 1 : `une_mere_n_a_pas_d_echeance` |
| I10 · un ordre de groupe commande les cibles des autres | 1 : `deux_ordres_de_groupe_ont_chacun_leurs_cibles` |
| I10 · un groupe cible l'ordre d'un autre groupe | 1 : `une_composition_invalide_est_refusee` |
| I11 · une clé qui n'est plus déclarée reste en service | 1 : `une_redeclaration_garde_les_numeros_par_cle` |
| I12 · la 51e retenue est acceptée | 1 : `les_plafonds_refusent_avant_de_muter` |
| I13 · une sortie ne fait pas avancer le rang de sa pièce | 1 : `un_ecran_de_piece_voit_ses_trous` |
| I14 · un code hors du dictionnaire passe entre les bornes | 1 : `la_validation_suit_la_forme_sur_un_corpus_fixe` |
| I14 · un objet hors de son schéma passe | 1 : `la_validation_suit_la_forme_sur_un_corpus_fixe` |
| I20 · l'instantané du moteur oublie les baux | 1 : `un_bail_se_retablit_apres_la_bascule_et_le_redemarrage` |

Bilan : 53 mutations détectées sur 53 ; code restauré.

### 20.4 Ce que le banc a appris

- **Ce qui quitte une machine est en heure murale, y compris dans une trace** (v1).
- **Une voie n'existe que tant qu'elle porte un ordre** (v1).
- **« A demandé la forme » ne prouve pas « s'en sert »** (v1).
- **Une règle juste fait échouer un scénario faux.** Les reçus refusent une séquence plus ancienne
  que leur fenêtre : la première soirée v2, dont un même écran numérotait ses demandes dans le
  désordre, se faisait refuser ses propres demandes. La règle avait raison ; un principal numérote
  dans l'ordre.
- **Une bascule a aussi des copies qui en savent plus que le secours.** La copie reprise par l'état
  complet avait reçu de l'ancien actif une sortie que le secours n'a jamais eue ; après la bascule,
  elle voyait un rang en double. D'où l'époque dans chaque sortie : une époque nouvelle fait se
  recharger.
- **Un écran qui se recharge au milieu d'un lot n'a pas perdu ce que son état contient déjà.**
- **Mesurer, c'est aussi savoir ce qu'on mesure** : 50 retenues comptaient d'abord le tampon des
  sorties, qui part sur MQTT.

## 21. Ce que l'analyse changerait, si tu la valides

Rien n'est modifié dans les propositions précédentes. Voici ce qu'il faudrait y reporter :

| Document | Ce qui changerait |
|---|---|
| V7 § 5.1, § 7 | un ordre n'est plus un point écrivable : `meta/<appareil>` sépare `points` et `ordres` ; `meta/forme/<f>` (des clés) ; `req/<compte>` porte la séquence, l'échéance et la tentative ; `cmd/` porte `d` (case, époque, génération), `i` et la forme ; l'issue du module revient avec `d` ; `ord/` remplace `evt/` pour les demandes ; `prive/<pair>` |
| V7 § 11.4.5 | les ACL : un compte ne publie que sur `req/<son compte>` ; seuls le hub et son pair publient `cmd/`, `ord/`, `res/`, `prive/` ; la coupure d'un abonnement quand un permis change |
| V7 § 14 | les clients par MQTT, ton choix : le module WebSocket disparaît ; le rang de chaque pièce |
| Mémoire v2 § 4.3, § 5.1 | un troisième type de numéro, `Ordre` ; l'octet `sens` ne dit plus « écriture » ; le registre des ordres et le catalogue, à part des valeurs ; les clés des valeurs, une fois |
| Mémoire v2 § 8 | les règles sur un ordre (`VIENT_DE` à trois réponses) ; l'index par ordre ; le nœud « commander » cite un ordre |
| HA v2 § 3, § 6, § 7, § 15 | `ord/`, `meta/forme/` et `prive/` au rang `s` ; l'époque et le rang de pièce dans chaque sortie ; les reçus dans la trace ; l'accusé du secours avant un appui ; le témoin garde le décrochage ; la reprise sûre ou non |
| Nettoyage § 9 à § 12 | les places d'ordres libérées avec celles des valeurs ; la fiche remise à zéro ; les listes tassées ; la redéclaration par clés ; la compatibilité des formes dans la succession |

## 22. Les choix

Décidé ici, parce que tes remarques, une décision déjà prise ou un constat de la revue le
tranchent :

| Sujet | Décision | Pourquoi |
|---|---|---|
| Valeurs et ordres | deux registres, un lien par référence, **par clé** | ta remarque ; I11 |
| La structure | des formes partagées et immuables, en cache par numéro | la règle du numéro inconnu (v2 § 4.4) |
| Les demandes | fiches de 72 o, cases, génération de 32 bits, **époque** | la règle des exécutions (v2 § 8.4) ; B4 |
| Qui demande | **le principal de la connexion ; un permis vivant ; chaque cible autorisée** | B1, I1 |
| Les reçus | **une fenêtre par principal, la plus haute séquence ; toute demande externe a sa clé** | B3 ; la règle d'aujourd'hui |
| Un appui et le secours | **l'accusé avant l'envoi ; le témoin garde le décrochage ; au plus une fois** | B2 |
| Un texte privé | **le canal privé, effacé à la conclusion** | B5 ; V7 § 5.1 |
| « La valeur vient-elle d'un ordre ? » | **oui, non, on ne sait pas ; conservatrice** | I5 |
| Un ordre de groupe | une mère, des filles, « n/m » **par niveau de preuve** ; des cibles physiques, rattachées, 64 au plus | V7 G17 ; I8, I10 |
| Un écran filtré | **le rang de chaque pièce** | I13 |

Décidé par toi le 24 octobre ; ces choix valent si tu valides l'analyse, et la v2 n'en change
aucun :

| Question | Ta réponse | Ce qui en découle |
|---|---|---|
| Les clients se chargent-ils par MQTT ? | **MQTT direct** | les écrans, l'appli et les tablettes par MQTT sur WebSocket ; REST et MCP pour les scripts et les agents ; les tampons de notre `rumqttd` à régler, puis à mesurer (§ 16.5, marche 8) |
| Une retenue passe-t-elle le redémarrage ? | **oui, gardée** | dans l'instantané, avec sa carte ; à l'approbation, l'échéance, le permis et la forme sont revérifiés (§ 12.2, § 12.3) |
| Le nom de la famille des traces | **`ord/`** | `evt/` reste aux occurrences des valeurs |
| Un ordre de groupe : qui le répartit ? | **les deux** | le hub pour un groupe simple ; un module quand il faut une stratégie (§ 14) |

Questions ouvertes, qui ne bloquent pas l'analyse :

| Question | Recommandation |
|---|---|
| L'attente de l'accusé du secours pour un appui (quelques centaines de µs sur le réseau local) te convient-elle, plutôt qu'une garantie plus faible ? | oui : elle ne touche que les appuis, et c'est ce qui rend « au plus une fois » vrai |
| Les protocoles qui disent la cause d'un changement (Zigbee « physique » ou « numérique », un bouton qui dit qui) méritent-ils d'être lus pour `VIENT_DE` ? | plus tard, pilote par pilote, quand un automatisme en a besoin |
| La bibliothèque MQTT du navigateur | à choisir et à mesurer à la marche 8 |

## 23. Le plan, si tu la valides

La revue propose de livrer d'abord le catalogue et le suivi dans le code d'aujourd'hui, et de ne
serrer les fiches à leur taille fixe qu'une fois les règles de sécurité et de reprise éprouvées. Le
plan la suit ; chaque marche est utile seule :

1. **Sans attendre la mémoire v2 : les trois défauts du § 5.3.** Garder les retenues au redémarrage
   (ton choix) et écrire la ligne de journal qui conclut chacune ; tenir la dernière commande de
   chaque point écrivable dans le hub, et la servir aux écrans ; donner à l'assistant un inventaire
   sans copier la maison.
2. **Le catalogue typé et le suivi central, dans le code d'aujourd'hui** : un type `Ordre` (une
   forme) à côté de `PointSpec`, la migration d'un point écrivable vers une valeur et un ordre faite
   par le hub à la déclaration, sans réécrire un pilote ; les permis et les chemins de livraison
   d'aujourd'hui gardés ; les demandes et les dernières commandes exposées.
3. **Les principaux, les permis, les reçus** : la clé de toute demande ; la fenêtre par principal ;
   le permis revérifié à chaque frontière (en reprenant `CommandPermit`).
4. **Le contrat MQTT** : `meta/` à deux familles, `meta/forme/`, `req/<compte>`, `cmd/` avec `d` et
   la forme, `ord/`, `prive/`, les ACL.
5. **Les workflows** : règles et déclencheurs sur un ordre, `VIENT_DE` à trois réponses.
6. **Les appareils virtuels** : la composition, les cibles rattachées, l'ordre de groupe (d'abord
   par le coordinateur des scènes), la garde des membres.
7. **La haute disponibilité, sur deux machines** : l'époque et le rang de pièce ; l'accusé avant un
   appui, le témoin ; la reprise sûre ou non ; **puis la mesure du cœur intégré**, qui certifie (ou
   non) le budget de 32 Mio.
8. **Les clients par MQTT** (ton choix) : l'écouteur WebSocket, les tampons réglés puis mesurés,
   l'état d'un client, la coupure d'un abonnement, la bibliothèque du navigateur.
9. **Les fiches de taille fixe** (72, 40, 24, 20, 16 o), une fois les règles éprouvées.
10. **Les pilotes, un par un** : un ordre par geste, quand leur module le veut (`control` → ouvrir,
    fermer, stop) ; la cause d'un changement, quand le protocole la donne.

## Annexe A — Le banc

### A.1 Comment il a été joué

- Le banc est rangé dans le dépôt, hors workspace : `tools/ordres-banc/` (son `README.md` dit
  comment tout rejouer). Il n'a aucune dépendance aux crates de Sarah Home. Il reprend tels quels la
  maison et les outils du banc mémoire, et le registre à génération du banc du nettoyage.
- Les structures d'aujourd'hui y sont recopiées : `PointSpec`, `Device`, `Hub::prepare`,
  `CommandRequest` et ce que garde son appelant, `JournalEntry`, `ApprovalRequest`, `Hub::snapshot`,
  l'inventaire de l'assistant, `/api/snapshot`. La maison d'aujourd'hui suit la convention des
  pilotes (§ 19.1).
- Compilé en Linux musl par `rust:1-alpine`, réglé comme la production (`opt-level = "s"`, LTO
  complet, une unité de code) ; chaque mesure dans un conteneur neuf limité à 2 CPU et 128 Mio, sans
  échange, trois fois ; la série garde la médiane.
- Mémoire : octets demandés à l'allocateur (le compteur du banc mémoire), et `size_of`, vérifié à la
  compilation. Temps : médiane de 11 à 201 tours ; la journée, en temps processeur (`getrusage`).
- Machine : Apple M4, Docker (Colima), conteneur `alpine:3`. Le Pi Zero 2 W sera plusieurs fois plus
  lent : les rapports tiennent, pas les valeurs absolues. La série v1 est gardée dans
  `sorties/linux-v1/`.

### A.2 Les limites

- **Les ordres de la maison du banc sont une hypothèse** : un modèle par classe d'appareil, un
  groupe pour 8 lampes et une « pièce » pour 10 capteurs. Le relevé des deux maisons reste à faire.
- **La journée de demandes est une hypothèse** : 3 000 par jour à ×1, aucune mesure réelle.
- **Les répliques d'aujourd'hui sont au mieux** : ni les canaux de Tokio, ni les tâches, ni la
  sérialisation ne sont comptés. Le code réel coûte plus.
- **À ×100, les structures d'aujourd'hui ne tiennent plus dans 128 Mio avec le reste** : le banc ne
  les construit qu'à ×1 et ×10.
- **Le banc ne joue ni le courtier, ni MQTT, ni deux machines** : les sorties critiques sont
  appliquées dans l'ordre, dans un seul processus ; l'accusé du secours y est immédiat ; le témoin
  est un drapeau. Les garanties distribuées sont démontrées par la soirée et les essais, pas
  éprouvées sur un réseau (§ 23, marche 7). Le coût d'une connexion MQTT est repris de la V7 ou
  estimé.
- **Le moteur est réduit** : un déclencheur, quelques pas ; la mémoire v2 a vérifié le reste.

### A.3 Les résultats bruts

Sortie brute — `catalogue-x1.txt` :

```text
RESULTAT|catalogue|1|appareils|525.000|
RESULTAT|catalogue|1|appareils_virtuels|29.000|
RESULTAT|catalogue|1|valeurs|2264.000|
RESULTAT|catalogue|1|ordres|802.000|
RESULTAT|catalogue|1|formes|16.000|
RESULTAT|catalogue|1|parametres|12.000|
RESULTAT|catalogue|1|membres|340.000|
RESULTAT|catalogue|1|cles|23.000|
RESULTAT|catalogue|1|taille_app_o|16.000|o
RESULTAT|catalogue|1|taille_fiche_ordre_o|40.000|o
RESULTAT|catalogue|1|taille_forme_o|24.000|o
RESULTAT|catalogue|1|taille_param_o|20.000|o
RESULTAT|catalogue|1|taille_demande_o|72.000|o
RESULTAT|catalogue|1|taille_recu_o|32.000|o
RESULTAT|catalogue|1|taille_membre_o|8.000|o
RESULTAT|catalogue|1|taille_ligne_o|48.000|o
RESULTAT|catalogue|1|propose_registre_ordres_o|45415.000|o
RESULTAT|catalogue|1|propose_catalogue_o|34674.000|o
RESULTAT|catalogue|1|propose_structure_ordres_o|32736.000|o
RESULTAT|catalogue|1|propose_appareils_listes_compositions_o|122265.000|o
RESULTAT|catalogue|1|propose_annuaire_valeurs_o|126880.000|o
RESULTAT|catalogue|1|memoire_v2_def_valeurs_o|36224.000|o
RESULTAT|catalogue|1|propose_description_totale_o|365458.000|o
RESULTAT|catalogue|1|hub_du_banc_total_o|523462.000|o
RESULTAT|catalogue|1|valider_par_nom_ns|38.903|ns
RESULTAT|catalogue|1|valider_par_numero_ns|5.692|ns
RESULTAT|catalogue|1|lire_derniere_commande_ns|0.366|ns
RESULTAT|catalogue|1|vient_de_ns|1.300|ns
RESULTAT|catalogue|1|page_virtuel_ns|204.034|ns
RESULTAT|catalogue|1|aussi_dans_parcours_ns|510.422|ns
RESULTAT|catalogue|1|meta_structure_dans_chaque_ordre_o|306345.000|o
RESULTAT|catalogue|1|meta_avec_formes_o|242058.000|o
RESULTAT|catalogue|1|meta_formes_seules_o|1462.000|o
RESULTAT|catalogue|1|aujourdhui_description_o|793240.000|o
RESULTAT|catalogue|1|aujourdhui_points|2389.000|
RESULTAT|catalogue|1|aujourdhui_points_ecrivables|556.000|
RESULTAT|catalogue|1|aujourdhui_points_en_ecriture_seule|114.000|
RESULTAT|catalogue|1|volets_rts|18.000|
RESULTAT|catalogue|1|corpus_intentions|1025.000|
RESULTAT|catalogue|1|corpus_acceptees|661.000|
RESULTAT|catalogue|1|corpus_decisions_identiques|1025.000|
RESULTAT|catalogue|1|valider_aujourdhui_ns|105.124|ns
RESULTAT|catalogue|1|assistant_copie_de_la_maison_o|324633.000|o
RESULTAT|catalogue|1|assistant_copie_us|127.294|µs
RESULTAT|catalogue|1|assistant_inventaire_aujourdhui_car|71952.000|car
RESULTAT|catalogue|1|assistant_inventaire_propose_car|60280.000|car
RESULTAT|catalogue|1|assistant_inventaire_aujourdhui_jetons_estimes|17988.000|jetons
RESULTAT|catalogue|1|assistant_inventaire_propose_jetons_estimes|15070.000|jetons
RESULTAT|catalogue|1|assistant_inventaire_propose_us|352.505|µs
RESULTAT|catalogue|1|rss_fin|872.000|Kio
CONTENEUR|false|0
CONTENEUR|false|0
CONTENEUR|false|0
```

Sortie brute — `catalogue-x10.txt` :

```text
RESULTAT|catalogue|10|appareils|5243.000|
RESULTAT|catalogue|10|appareils_virtuels|283.000|
RESULTAT|catalogue|10|valeurs|22640.000|
RESULTAT|catalogue|10|ordres|8468.000|
RESULTAT|catalogue|10|formes|16.000|
RESULTAT|catalogue|10|parametres|12.000|
RESULTAT|catalogue|10|membres|3490.000|
RESULTAT|catalogue|10|cles|23.000|
RESULTAT|catalogue|10|taille_app_o|16.000|o
RESULTAT|catalogue|10|taille_fiche_ordre_o|40.000|o
RESULTAT|catalogue|10|taille_forme_o|24.000|o
RESULTAT|catalogue|10|taille_param_o|20.000|o
RESULTAT|catalogue|10|taille_demande_o|72.000|o
RESULTAT|catalogue|10|taille_recu_o|32.000|o
RESULTAT|catalogue|10|taille_membre_o|8.000|o
RESULTAT|catalogue|10|taille_ligne_o|48.000|o
RESULTAT|catalogue|10|propose_registre_ordres_o|511300.000|o
RESULTAT|catalogue|10|propose_catalogue_o|341314.000|o
RESULTAT|catalogue|10|propose_structure_ordres_o|339376.000|o
RESULTAT|catalogue|10|propose_appareils_listes_compositions_o|1019020.000|o
RESULTAT|catalogue|10|propose_annuaire_valeurs_o|1246576.000|o
RESULTAT|catalogue|10|memoire_v2_def_valeurs_o|362240.000|o
RESULTAT|catalogue|10|propose_description_totale_o|3480450.000|o
RESULTAT|catalogue|10|hub_du_banc_total_o|5090502.000|o
RESULTAT|catalogue|10|valider_par_nom_ns|50.945|ns
RESULTAT|catalogue|10|valider_par_numero_ns|6.022|ns
RESULTAT|catalogue|10|lire_derniere_commande_ns|0.406|ns
RESULTAT|catalogue|10|vient_de_ns|1.302|ns
RESULTAT|catalogue|10|page_virtuel_ns|238.371|ns
RESULTAT|catalogue|10|aussi_dans_parcours_ns|4813.219|ns
RESULTAT|catalogue|10|meta_structure_dans_chaque_ordre_o|3153818.000|o
RESULTAT|catalogue|10|meta_avec_formes_o|2456997.000|o
RESULTAT|catalogue|10|meta_formes_seules_o|1462.000|o
RESULTAT|catalogue|10|aujourdhui_description_o|7986368.000|o
RESULTAT|catalogue|10|aujourdhui_points|23969.000|
RESULTAT|catalogue|10|aujourdhui_points_ecrivables|5836.000|
RESULTAT|catalogue|10|aujourdhui_points_en_ecriture_seule|1216.000|
RESULTAT|catalogue|10|volets_rts|200.000|
RESULTAT|catalogue|10|corpus_intentions|1024.000|
RESULTAT|catalogue|10|corpus_acceptees|654.000|
RESULTAT|catalogue|10|corpus_decisions_identiques|1024.000|
RESULTAT|catalogue|10|valider_aujourdhui_ns|162.233|ns
RESULTAT|catalogue|10|assistant_copie_de_la_maison_o|3247311.000|o
RESULTAT|catalogue|10|assistant_copie_us|1883.317|µs
RESULTAT|catalogue|10|assistant_inventaire_aujourdhui_car|736413.000|car
RESULTAT|catalogue|10|assistant_inventaire_propose_car|612967.000|car
RESULTAT|catalogue|10|assistant_inventaire_aujourdhui_jetons_estimes|184103.000|jetons
RESULTAT|catalogue|10|assistant_inventaire_propose_jetons_estimes|153241.000|jetons
RESULTAT|catalogue|10|assistant_inventaire_propose_us|3385.420|µs
RESULTAT|catalogue|10|rss_fin|1112.000|Kio
CONTENEUR|false|0
CONTENEUR|false|0
CONTENEUR|false|0
```

Sortie brute — `catalogue-x100.txt` :

```text
RESULTAT|catalogue|100|appareils|52407.000|
RESULTAT|catalogue|100|appareils_virtuels|2807.000|
RESULTAT|catalogue|100|valeurs|226400.000|
RESULTAT|catalogue|100|ordres|82592.000|
RESULTAT|catalogue|100|formes|16.000|
RESULTAT|catalogue|100|parametres|12.000|
RESULTAT|catalogue|100|membres|34451.000|
RESULTAT|catalogue|100|cles|23.000|
RESULTAT|catalogue|100|taille_app_o|16.000|o
RESULTAT|catalogue|100|taille_fiche_ordre_o|40.000|o
RESULTAT|catalogue|100|taille_forme_o|24.000|o
RESULTAT|catalogue|100|taille_param_o|20.000|o
RESULTAT|catalogue|100|taille_demande_o|72.000|o
RESULTAT|catalogue|100|taille_recu_o|32.000|o
RESULTAT|catalogue|100|taille_membre_o|8.000|o
RESULTAT|catalogue|100|taille_ligne_o|48.000|o
RESULTAT|catalogue|100|propose_registre_ordres_o|4826260.000|o
RESULTAT|catalogue|100|propose_catalogue_o|3306274.000|o
RESULTAT|catalogue|100|propose_structure_ordres_o|3304336.000|o
RESULTAT|catalogue|100|propose_appareils_listes_compositions_o|8781862.000|o
RESULTAT|catalogue|100|propose_annuaire_valeurs_o|12063048.000|o
RESULTAT|catalogue|100|memoire_v2_def_valeurs_o|3622400.000|o
RESULTAT|catalogue|100|propose_description_totale_o|32599844.000|o
RESULTAT|catalogue|100|hub_du_banc_total_o|48786630.000|o
RESULTAT|catalogue|100|valider_par_nom_ns|58.862|ns
RESULTAT|catalogue|100|valider_par_numero_ns|6.220|ns
RESULTAT|catalogue|100|lire_derniere_commande_ns|0.488|ns
RESULTAT|catalogue|100|vient_de_ns|1.300|ns
RESULTAT|catalogue|100|page_virtuel_ns|362.892|ns
RESULTAT|catalogue|100|aussi_dans_parcours_ns|46992.172|ns
RESULTAT|catalogue|100|meta_structure_dans_chaque_ordre_o|31579359.000|o
RESULTAT|catalogue|100|meta_avec_formes_o|24785651.000|o
RESULTAT|catalogue|100|meta_formes_seules_o|1462.000|o
RESULTAT|catalogue|100|rss_fin|2496.000|Kio
CONTENEUR|false|0
CONTENEUR|false|0
CONTENEUR|false|0
```

Sortie brute — `clients-x1.txt` :

```text
RESULTAT|clients|1|aujourdhui_api_snapshot_json_o|593342.000|o
RESULTAT|clients|1|aujourdhui_api_snapshot_us|690.218|µs
RESULTAT|clients|1|propose_meme_contenu_json_o|140281.000|o
RESULTAT|clients|1|mqtt_etat_client_complet_o|112567.000|o
RESULTAT|clients|1|mqtt_etat_client_morceaux_4kio|28.000|
RESULTAT|clients|1|mqtt_etat_client_deux_pieces_o|20518.000|o
RESULTAT|clients|1|mqtt_etat_client_complet_us|85.001|µs
RESULTAT|clients|1|mqtt_etat_client_deux_pieces_us|20.500|µs
RESULTAT|clients|1|rss_fin|856.000|Kio
CONTENEUR|false|0
CONTENEUR|false|0
CONTENEUR|false|0
```

Sortie brute — `clients-x10.txt` :

```text
RESULTAT|clients|10|aujourdhui_api_snapshot_json_o|5970534.000|o
RESULTAT|clients|10|aujourdhui_api_snapshot_us|7618.894|µs
RESULTAT|clients|10|propose_meme_contenu_json_o|1439681.000|o
RESULTAT|clients|10|mqtt_etat_client_complet_o|1132704.000|o
RESULTAT|clients|10|mqtt_etat_client_morceaux_4kio|277.000|
RESULTAT|clients|10|mqtt_etat_client_deux_pieces_o|188626.000|o
RESULTAT|clients|10|mqtt_etat_client_complet_us|679.135|µs
RESULTAT|clients|10|mqtt_etat_client_deux_pieces_us|142.376|µs
RESULTAT|clients|10|rss_fin|1064.000|Kio
CONTENEUR|false|0
CONTENEUR|false|0
CONTENEUR|false|0
```

Sortie brute — `clients-x100.txt` :

```text
RESULTAT|clients|100|propose_meme_contenu_json_o|14646137.000|o
RESULTAT|clients|100|mqtt_etat_client_complet_o|11223474.000|o
RESULTAT|clients|100|mqtt_etat_client_morceaux_4kio|2741.000|
RESULTAT|clients|100|mqtt_etat_client_deux_pieces_o|1877175.000|o
RESULTAT|clients|100|mqtt_etat_client_complet_us|7437.182|µs
RESULTAT|clients|100|mqtt_etat_client_deux_pieces_us|1614.146|µs
RESULTAT|clients|100|rss_fin|2492.000|Kio
CONTENEUR|false|0
CONTENEUR|false|0
CONTENEUR|false|0
```

Sortie brute — `demandes-x1.txt` :

```text
RESULTAT|demandes|1|demandes|3000.000|
RESULTAT|demandes|1|renvois_meme_cle|116.000|
RESULTAT|demandes|1|refusees|26.000|
RESULTAT|demandes|1|envois|3104.000|
RESULTAT|demandes|1|pic_demandes_en_cours|39.000|
RESULTAT|demandes|1|cpu_journee_ms|4.734|ms
RESULTAT|demandes|1|cpu_par_demande_us|1.578|µs
RESULTAT|demandes|1|mur_journee_ms|4.738|ms
RESULTAT|demandes|1|reveils|7598.000|
RESULTAT|demandes|1|sorties_critiques_par_jour|10924.000|
RESULTAT|demandes|1|sorties_critiques_par_jour_o|1764959.000|o
RESULTAT|demandes|1|memoire_demandes_o|4864.000|o
RESULTAT|demandes|1|memoire_recus_o|10448.000|o
RESULTAT|demandes|1|memoire_recus_borne_64_principaux_o|66560.000|o
RESULTAT|demandes|1|memoire_journal_o|24000.000|o
RESULTAT|demandes|1|etat_complet_catalogue_o|32069.000|o
RESULTAT|demandes|1|etat_complet_demandes_o|203.000|o
RESULTAT|demandes|1|etat_complet_total_o|277879.000|o
RESULTAT|demandes|1|prendre_et_liberer_une_case_ns|7.292|ns
RESULTAT|demandes|1|retrouver_une_demande_ns|0.812|ns
RESULTAT|demandes|1|propose_retenues|50.000|
RESULTAT|demandes|1|propose_retenues_50_o|11458.000|o
RESULTAT|demandes|1|aujourdhui_une_demande_en_vol_o|590.000|o
RESULTAT|demandes|1|aujourdhui_journal_500_o|140950.000|o
RESULTAT|demandes|1|aujourdhui_retenues_50_o|22100.000|o
RESULTAT|demandes|1|rss_fin|868.000|Kio
CONTENEUR|false|0
CONTENEUR|false|0
CONTENEUR|false|0
```

Sortie brute — `demandes-x10.txt` :

```text
RESULTAT|demandes|10|demandes|30000.000|
RESULTAT|demandes|10|renvois_meme_cle|1226.000|
RESULTAT|demandes|10|refusees|322.000|
RESULTAT|demandes|10|envois|32215.000|
RESULTAT|demandes|10|pic_demandes_en_cours|64.000|
RESULTAT|demandes|10|cpu_journee_ms|50.217|ms
RESULTAT|demandes|10|cpu_par_demande_us|1.674|µs
RESULTAT|demandes|10|mur_journee_ms|50.307|ms
RESULTAT|demandes|10|reveils|76157.000|
RESULTAT|demandes|10|sorties_critiques_par_jour|112629.000|
RESULTAT|demandes|10|sorties_critiques_par_jour_o|18214885.000|o
RESULTAT|demandes|10|memoire_demandes_o|4864.000|o
RESULTAT|demandes|10|memoire_recus_o|10480.000|o
RESULTAT|demandes|10|memoire_recus_borne_64_principaux_o|66560.000|o
RESULTAT|demandes|10|memoire_journal_o|24000.000|o
RESULTAT|demandes|10|etat_complet_catalogue_o|331043.000|o
RESULTAT|demandes|10|etat_complet_demandes_o|390.000|o
RESULTAT|demandes|10|etat_complet_total_o|2758227.000|o
RESULTAT|demandes|10|prendre_et_liberer_une_case_ns|7.209|ns
RESULTAT|demandes|10|retrouver_une_demande_ns|0.812|ns
RESULTAT|demandes|10|aujourdhui_une_demande_en_vol_o|590.000|o
RESULTAT|demandes|10|aujourdhui_journal_500_o|140950.000|o
RESULTAT|demandes|10|aujourdhui_retenues_50_o|22100.000|o
RESULTAT|demandes|10|rss_fin|1024.000|Kio
CONTENEUR|false|0
CONTENEUR|false|0
CONTENEUR|false|0
```

Sortie brute — `demandes-x100.txt` :

```text
RESULTAT|demandes|100|demandes|300000.000|
RESULTAT|demandes|100|renvois_meme_cle|11919.000|
RESULTAT|demandes|100|refusees|3277.000|
RESULTAT|demandes|100|envois|321298.000|
RESULTAT|demandes|100|pic_demandes_en_cours|114.000|
RESULTAT|demandes|100|cpu_journee_ms|1024.347|ms
RESULTAT|demandes|100|cpu_par_demande_us|3.414|µs
RESULTAT|demandes|100|mur_journee_ms|1029.388|ms
RESULTAT|demandes|100|reveils|760771.000|
RESULTAT|demandes|100|sorties_critiques_par_jour|1125535.000|
RESULTAT|demandes|100|sorties_critiques_par_jour_o|181466066.000|o
RESULTAT|demandes|100|memoire_demandes_o|9728.000|o
RESULTAT|demandes|100|memoire_recus_o|10448.000|o
RESULTAT|demandes|100|memoire_recus_borne_64_principaux_o|66560.000|o
RESULTAT|demandes|100|memoire_journal_o|24000.000|o
RESULTAT|demandes|100|etat_complet_catalogue_o|3221879.000|o
RESULTAT|demandes|100|etat_complet_demandes_o|764.000|o
RESULTAT|demandes|100|etat_complet_total_o|27194259.000|o
RESULTAT|demandes|100|prendre_et_liberer_une_case_ns|7.375|ns
RESULTAT|demandes|100|retrouver_une_demande_ns|0.652|ns
RESULTAT|demandes|100|aujourdhui_une_demande_en_vol_o|590.000|o
RESULTAT|demandes|100|aujourdhui_journal_500_o|140950.000|o
RESULTAT|demandes|100|aujourdhui_retenues_50_o|22100.000|o
RESULTAT|demandes|100|rss_fin|2516.000|Kio
CONTENEUR|false|0
CONTENEUR|false|0
CONTENEUR|false|0
```

Sortie brute — `machine.txt` :

```text
machine : Apple M4
docker : 29.2.1 · 4 CPU · 8309014528 o
conteneurs : 2 CPU, 128 Mio sans échange, trois essais par mesure (médiane)
charge à la fin de la série : 4.63 10.42 13.40
```

Sortie brute — `soiree-x1.txt` :

```text
RESULTAT|soiree|1|duree_ms|13.347|ms
RESULTAT|soiree|1|demandes_suivies|48.000|
RESULTAT|soiree|1|envois|59.000|
RESULTAT|soiree|1|sorties_critiques|823.000|
RESULTAT|soiree|1|sorties_critiques_o|143883.000|o
RESULTAT|soiree|1|instantane_o|281188.000|o
RESULTAT|soiree|1|ecran_formes_demandees|35.000|
RESULTAT|soiree|1|ecran_perime_formes_demandees|1.000|
RESULTAT|soiree|1|releves_identiques|3.000|
RESULTAT|soiree|1|releves|3.000|
RESULTAT|soiree|1|validations_identiques|3344.000|
RESULTAT|soiree|1|validations|3344.000|
RESULTAT|soiree|1|ordres_du_catalogue|836.000|
RESULTAT|soiree|1|formes_du_catalogue|35.000|
RESULTAT|soiree|1|ecran_piece_trous|1.000|
RESULTAT|soiree|1|ecran_piece_rechargements|4.000|
RESULTAT|soiree|1|secrets_trouves_hors_canal_prive|0.000|
RESULTAT|soiree|1|rss_fin|884.000|Kio
CONTENEUR|false|0
CONTENEUR|false|0
CONTENEUR|false|0
```

Sortie brute — `tests.txt` :

```text
test demandes::tests::la_table_relue_garde_les_generations ... ok
test catalogue::tests::deux_ordres_de_meme_structure_partagent_leur_forme ... ok
test catalogue::tests::le_catalogue_relu_est_identique ... ok
test catalogue::tests::la_validation_suit_la_forme_sur_un_corpus_fixe ... ok
test demandes::tests::un_appel_retente_retrouve_son_recu_et_un_vieux_rejeu_est_refuse ... ok
test demandes::tests::un_numero_d_une_autre_epoque_ne_designe_rien ... ok
test demandes::tests::une_case_epuisee_est_condamnee ... ok
test demandes::tests::une_case_reutilisee_change_de_generation ... ok
test essais::deux_ordres_de_groupe_ont_chacun_leurs_cibles ... ok
test essais::un_groupe_attend_toutes_ses_cibles_meme_conclues_aussitot ... ok
test essais::une_approbation_reverifie_le_permis ... ok
test essais::un_secours_qui_decroche_est_inscrit_avant_l_envoi ... ok
test essais::une_lecture_commencee_avant_l_envoi_ne_prouve_rien ... ok
test essais::une_mere_n_a_pas_d_echeance ... ok
test essais::une_reprise_non_sure_refuse_une_tentative_ambigue ... ok
test essais::une_revocation_retire_ce_qui_n_est_pas_parti ... ok
test numeros::tests::l_annuaire_relu_est_identique ... ok
test numeros::tests::les_plages_se_recollent_et_le_bout_se_rogne ... ok
test numeros::tests::une_copie_refuse_une_place_qui_n_est_pas_libre ... ok
test numeros::tests::le_tiroir_des_noms_se_tasse_sans_rien_perdre ... ok
test numeros::tests::une_place_liberee_sert_a_la_generation_suivante ... ok
test soiree::montrer::montrer ... ignored
test numeros::tests::une_generation_epuisee_condamne_la_place ... ok
test essais::les_plafonds_refusent_avant_de_muter ... ok
test essais::une_composition_invalide_est_refusee ... ok
test soiree::tests::aucun_ordre_n_est_rejoue ... ok
test soiree::tests::chaque_issue_tombe_sur_sa_demande ... ok
test soiree::tests::chacun_connait_l_appareil_l_ordre_et_sa_structure ... ok
test soiree::tests::la_copie_et_l_etat_complet_finissent_identiques ... ok
test soiree::tests::la_derniere_commande_est_la_derniere_admise ... ok
test soiree::tests::la_derniere_commande_se_lit_partout_pareil ... ok
test soiree::tests::le_journal_dit_pourquoi ... ok
test soiree::tests::les_consignes_se_fusionnent_les_appuis_jamais ... ok
test soiree::tests::les_valeurs_et_les_ordres_ne_se_melangent_pas ... ok
test soiree::tests::les_workflows_interrogent_les_ordres ... ok
test soiree::tests::un_appareil_retire_n_est_plus_commande_et_sa_place_ne_garde_rien ... ok
test soiree::tests::un_appareil_virtuel_compose_sans_posseder ... ok
test soiree::tests::un_appui_n_est_ni_perdu_ni_double_par_la_bascule ... ok
test soiree::tests::un_appui_retente_ne_part_qu_une_fois ... ok
test soiree::tests::un_bail_se_retablit_apres_la_bascule_et_le_redemarrage ... ok
test soiree::tests::un_ecran_de_piece_voit_ses_trous ... ok
test soiree::tests::un_groupe_coupe_par_la_bascule_garde_son_compte ... ok
test soiree::tests::un_groupe_approuve_se_repartit ... ok
test soiree::tests::un_invite_ne_commande_que_ses_pieces ... ok
test soiree::tests::un_texte_prive_ne_sort_que_par_le_canal_prive ... ok
test soiree::tests::une_carte_ne_s_approuve_que_sous_sa_forme ... ok
test soiree::tests::une_echeance_se_verifie_a_chaque_frontiere ... ok
test soiree::tests::une_lecture_d_avant_la_reprise_ne_confirme_pas ... ok
test soiree::tests::une_redeclaration_garde_les_numeros_par_cle ... ok
test soiree::tests::une_retenue_n_approuve_que_ce_qu_elle_montre ... ok
test soiree::tests::une_retenue_ne_garde_pas_une_autorisation_echue ... ok
test soiree::tests::une_structure_qui_change_met_l_automatisme_a_revoir ... ok
test soiree::tests::une_valeur_hors_de_sa_forme_n_atteint_aucun_module ... ok
test result: ok. 52 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.02s
rustc 1.99.1 (58cf8f9c3 2026-10-13)
```

