# Étude mémoire — Sarah Home : des numéros qui sont des adresses

> **Version remplacée.** Cette v1 est gardée comme historique. La [v2](memoire-v2.md) la reprend en
> entier, la corrige (deux horloges, attente de 24 octets, fraîcheur tenue) et ajoute le temps :
> l'équivalent d'un `cron`, les agendas, la précision.

Date : 23 octobre 2026. Statut : **proposition, aucune modification du code**.
Base analysée : `main` au commit `bd1714d`. Complète l'[analyse V7](isolation-reactivite-runtime-v7.md)
(§ 10.5 la santé des appareils, § 11 la mémoire, § 12.1 le journal de reprise, § 13 le moteur de
workflow).

L'idée de départ est la tienne : **chaque valeur a un numéro fixe ; les fiches ont toutes la même
taille et sont rangées dans l'ordre des numéros ; l'adresse d'une valeur se déduit donc de son
numéro, et on n'a jamais à la transmettre.** Ce mémoire la confronte au code d'aujourd'hui, la
vérifie par un banc, et répond à tes trois questions :

1. retrouve-t-on une valeur sans la chercher ?
2. trouve-t-on facilement, par le calcul, les valeurs qui ont expiré ?
3. comment ranger les workflows en mémoire, à part de celle des valeurs, et comment calculer leurs
   conditions ? Avec quel rapport entre l'état d'un flux à la n8n et la mémoire des valeurs ?

**Mise à jour du 23 octobre** : tes quatre réponses sont intégrées (§ 11).

- Une condition lit la maison au moment où elle passe. Le flux garde l'état qui l'a déclenché pour
  toute la suite, et une étape franchie ne se rejoue pas.
- Les exécutions en attente reprennent après un redémarrage si elles sont à l'heure.
- Les déclencheurs composés entrent dès la V7.
- Le numéro voyage sur MQTT, en plus du nom.

Méthode, et techniques employées, nommées au fil du texte :

- **premiers principes** : ce que fait réellement le processeur quand on lit une fiche ;
- **contre-épreuve des hypothèses** : chercher où l'idée casse (noms, textes, suppressions,
  redémarrage, expiration, lecteurs multiples) ;
- **analyse morphologique** : pour chaque question, toutes les options côte à côte ;
- **analogies** : l'automate industriel (cycle), les systèmes experts (réseau incrémental), les
  moteurs de jeu (entités numérotées rangées en colonnes), n8n (le flux porte ses données) ;
- **sélection à critères visibles** : simplicité, vitesse, mémoire, reprise, explicabilité ;
- **un banc** : Linux musl, conteneur limité à 2 CPU et 128 Mio comme la production, chaque mesure
  dans un processus neuf ; les structures d'aujourd'hui recopiées à l'identique depuis le code
  ([annexe A](#annexe-a--le-banc)) ;
- **une vérification de bout en bout** : un moteur de référence complet, une soirée de la maison
  rejouée seconde par seconde, six tests et sept fautes injectées pour prouver qu'ils savent échouer
  (§ 13). Le banc, le moteur et les tests sont rangés dans le dépôt : `tools/memoire-banc/`.

Vocabulaire :

- un **numéro** est un entier de 4 octets (`u32`) donné à un point, un appareil, un nœud de
  workflow ou une exécution ;
- une **fiche** est un enregistrement de taille fixe (40 octets pour la valeur vive d'un point) ;
- une **colonne** est un tableau de fiches d'une même sorte, rangées par numéro ;
- l'**annuaire** traduit un nom (`z2m:0x00158d00a1b2c3d4/temperature`) en numéro, et
  inversement ;
- une **échéance** est l'instant où quelque chose expire : un appareil muet, une valeur trop vieille,
  une attente qui se termine ;
- un **passage** est la lecture complète d'une colonne d'échéances ;
- le **programme** est la forme compilée des workflows ; une **exécution** est un workflow en train
  de tourner.

## 1. Ton idée, reformulée, et ce qu'il fallait vérifier

Reformulée en trois questions « Comment pourrions-nous… » (*How Might We*), la technique qui
ouvre toute séance d'idéation :

| Question | Ce qu'elle demande à la mémoire |
|---|---|
| **Comment retrouver une valeur sans la chercher ?** | que le numéro suffise à calculer l'adresse, sans table de hachage ni comparaison de chaînes |
| **Comment savoir ce qui a expiré sans tout relire ?** | que les échéances se trouvent par le calcul, ou par une lecture si rapide qu'elle ne coûte rien |
| **Comment ranger les workflows avec le même principe, dans leur propre mémoire ?** | que le programme, les exécutions et les conditions soient aussi des fiches numérotées, sans recopier les valeurs |

Ce qu'il fallait vérifier, point par point :

1. le prix d'une lecture par numéro, face aux lectures d'aujourd'hui ;
2. une taille fixe pour des valeurs qui ne le sont pas (textes, énumérations) ;
3. des numéros qui restent fixes : au redémarrage, à la suppression d'un appareil, sur la machine de
   secours ;
4. l'expiration : le calcul donne une adresse ; donne-t-il aussi la liste de ce qui a expiré ?
5. plusieurs lecteurs (hub, moteur, écrans, API) sans se gêner ;
6. les workflows : le programme, les exécutions, les conditions, et l'état du flux face à la
   mémoire des valeurs.

## 2. Le verdict en bref

- **Ton idée est juste, et c'est déjà la forme du prototype de la V7** (`Vec<Vif>` indexé par un
  `u32`, § 11.4.4). Lire une valeur par son numéro coûte **0,32 ns**, contre **92 ns** par
  `Hub::state` aujourd'hui et **33 ns** par le miroir du moteur : environ 290 fois moins.
- **L'adresse se déduit, le numéro suffit.** Il pèse 4 octets au lieu des 8 d'une adresse, et il
  survit là où une adresse meurt : quand le tableau grandit, au redémarrage, sur disque, au journal
  de reprise, sur la machine de secours. Ton « index où tous les ID sont à la suite » existe :
  c'est l'annuaire, les noms bout à bout dans l'ordre des numéros (§ 3, § 4).
- **« Où » se calcule ; « qui a expiré » se lit.** Le numéro donne la place d'une fiche connue,
  pas la liste des fiches périmées. Mais une colonne d'échéances de 4 octets, contiguë et de taille
  fixe, se lit en **0,71 µs pour toute la maison**. Avec le **réveil au plus proche**, on ne la lit
  que **26 fois par heure** au lieu de 3 600, pour **0,03 ms de processeur par heure** (§ 6).
- **Le nom reste l'identité.** L'annuaire traduit une fois, au bord, en 34 ns ; dedans, plus aucune
  chaîne n'est comparée (§ 4.3). Sur MQTT, le numéro voyage en plus du nom : `n` dans `meta/`, `i`
  dans `val/` (§ 4.4).
- **Un numéro de point n'est jamais redonné** ; une case d'exécution, si, avec une génération qui
  écarte les réponses tardives (§ 4, § 8.4).
- **Les workflows ont leur mémoire à part, en deux parties.**
  - Le programme dit ce que les workflows sont. Il est figé, et recompilé à chaque validation en
    0,23 ms.
  - Les exécutions disent ce qu'ils font, et chacune garde sa cause.

  Pour 400 automatismes, **0,23 Mio au lieu de 3,45 Mio**, déclencheurs composés compris au pire.
  Un changement de valeur réveille les déclencheurs de son point en **1 à 5,5 ns**. Le balayage de
  tous les automatismes coûte **0,8 à 1,5 µs** (§ 8).
- **Les conditions se calculent à la demande, par numéro** : un « si » de trois règles en 4,6 ns.
  Une durée (« ouvert depuis 10 min ») est un calcul sur l'instant « changé », pas une minuterie.
  Les déclencheurs composés (« toutes les fenêtres fermées »), que tu veux dès la V7, passent par un
  réseau incrémental. Son état se reconstruit d'un coup en 3,6 µs (§ 9.5).
- **Un flux n'emporte pas la maison avec lui, mais il garde sa cause.** Après une attente, une
  condition lit la mémoire des valeurs, pas une copie emportée comme le ferait n8n. L'état qui a
  déclenché le flux (point, valeur, précédente, instant) reste dans sa fiche pour toute la suite, et
  une étape franchie ne se rejoue pas (§ 8.4).
- **Vérifié de bout en bout.** Une soirée de la maison est rejouée contre un moteur de référence,
  avec un automatisme de 23 nœuds à deux entrées, des vérifications de nouvelles valeurs, une panne,
  une modification en cours et un capteur muet. Les six tests passent, et sept fautes injectées sont
  toutes détectées. 400 maisons rejouées ensemble se comportent comme la première, pour 54 ms de
  processeur sur toute la soirée (§ 13).
- **Trois défauts d'aujourd'hui, trouvés en chemin** (§ 8.1) :
  - chaque « attendre que » se réveille à chaque événement de la maison : 29 µs par événement avec
    400 attentes ;
  - chaque durée armée est une tâche Tokio de 629 octets, perdue au redémarrage ;
  - chaque changement de valeur parcourt tous les déclencheurs de tous les automatismes.

<!-- FIGURE:memoire -->

## 3. Le principe : le numéro est l'adresse

### 3.1 Ce que fait le processeur

Une **analogie** d'abord : un couloir de casiers tous de la même largeur. Pour trouver le casier
1234, on ne lit pas les étiquettes : on fait 1234 pas de la largeur d'un casier depuis l'entrée.

La mémoire, c'est le couloir ; une fiche, c'est un casier. Le calcul de l'adresse tient en une
ligne :

```text
adresse de la fiche n = début de la colonne + n × taille de la fiche
fiche Vif n° 1234      = début + 1234 × 40 = début + 49 360
```

Le processeur le fait en une ou deux instructions (une multiplication-addition), plus une
comparaison qui vérifie que le numéro est dans le tableau. En Rust, c'est exactement ce que fait
`vifs[n]` sur un `Vec<Vif>` : aucune ligne `unsafe`, aucune adresse manipulée à la main. Le
tableau garantit que ses fiches sont contiguës et que leur pas est `size_of::<Vif>()`, alignement
compris.

<!-- FIGURE:principe -->

### 3.2 Pourquoi un numéro plutôt qu'une adresse

| | Numéro (`u32`) | Adresse (pointeur) | Nom (`z2m:…/temperature`) |
|---|---|---|---|
| Taille | **4 o** | 8 o | ≈ 39 o, plus le hachage |
| Le tableau grandit (il est déplacé en mémoire) | **reste valable** | pend : l'ancienne adresse ne vaut plus rien | reste valable |
| Redémarrage | **reste valable**, si l'annuaire est relu | change à chaque lancement | reste valable |
| Écrit sur disque, au journal, envoyé à la machine de secours | **oui** | non | oui, mais 10 fois plus gros |
| En Rust | un index : vérifié, sans verrou ni `unsafe` | une référence ne peut pas être gardée pendant qu'on écrit dans le tableau | une recherche à chaque usage |
| Lire la valeur | **0,32 ns** | la même lecture, sans la vérification | 33 à 92 ns |

Ce n'est pas une astuce isolée. Les moteurs de jeu rangent leurs entités ainsi (des colonnes
indexées par un numéro d'entité), et les compilateurs, dont celui de Rust, rangent leurs nœuds
dans des tableaux indexés. La base historique de Sarah Home fait déjà la même chose sur disque : la
clé d'un relevé est calculée, `samples.id = point_id · 2^44 + ts` (D13). La place sur disque se
déduit du numéro du point, sans index secondaire.

### 3.3 Ce que le banc mesure

<!-- FIGURE:lecture -->

| Lire une valeur (ns, Linux musl) | ×1 | ×3 | ×10 | ×100 |
|---|---|---|---|---|
| `Hub::state` aujourd'hui : alloue un `DeviceId`, verrou, deux recherches, copie | 92 | 90 | 91 | — |
| Miroir du moteur aujourd'hui : `HashMap<String, Value>` | 33 | 38 | 43 | — |
| Annuaire : nom → numéro, une fois au bord | 34 | 39 | 48 | 74 |
| Par numéro, fiche de 40 o | **0,32** | 0,40 | 0,55 | 0,49 |
| Par numéro, fiche de 32 o | 0,34 | 0,43 | 0,44 | 0,56 |
| Par numéro, colonne de 16 o | 0,36 | 0,36 | 0,48 | 0,56 |
| Par numéro, avec la qualité de l'appareil (trois lectures) | 0,73 | 1,2 | 1,6 | 2,1 |
| Par numéro, derrière un verrou (lecteur d'un autre fil, sans rival) | 3,5 | 3,5 | 3,5 | 3,2 |
| Écrire une nouvelle : valeur, précédente, instants, santé, échéances, en place | 2,0 | 3,5 | 4,3 | 6,3 |

Lectures dans un ordre tiré au hasard, comme une condition lit un point quelconque. À ×100
(226 400 points), les structures d'aujourd'hui ne tiennent plus dans 128 Mio avec le reste : le banc
ne les construit plus.

### 3.4 La taille de la fiche et le cache du processeur

Le processeur ne lit pas un octet : il lit une **ligne de cache** de 64 octets. Une fiche de
40 octets est à cheval sur deux lignes une fois sur deux ; la lire peut alors coûter deux accès à
la mémoire au lieu d'un. À 32 ou 16 octets, jamais.

<!-- FIGURE:lignes -->

Sur le banc, aucun écart : à ×1, la colonne de 40 o (88 Kio) tient dans le cache, et même à ×100 le
cache de l'Apple M4 est assez grand. Sur un Pi Zero 2 W (cache L2 de 512 Kio, la cible D10), ×10
(884 Kio) n'y tient plus. La variante à 32 octets de la V7 existe (§ 11.2) ; **la mesure est à faire
sur la carte** avant de choisir.

## 4. Les numéros : qui les donne, comment ils restent fixes

### 4.1 La règle

<!-- FIGURE:cycle -->

1. **Le hub donne le numéro**, à la première déclaration d'un point : le suivant, au bout
   (n = nombre de points déjà connus). Les points d'un appareil arrivent ensemble à sa
   déclaration : leurs numéros se suivent.
2. **Il l'écrit dans l'annuaire persistant avant de s'en servir** : la table des numéros de
   `maison.db`, ou en attendant un fichier d'état écrit par `StateFile` (fichier, puis dossier, avec
   `fsync`). Un arrêt brutal ne peut donc pas donner le même numéro à deux points.
3. **Au démarrage, l'annuaire relu remet chaque nom à son numéro** : les colonnes se reconstruisent
   dans le même ordre ; même numéro, même place, d'un lancement à l'autre.
4. **Un point retiré garde sa case, condamnée ; son numéro n'est jamais redonné.** Un trou coûte
   sa fiche, environ 120 octets : mille points retirés, 0,1 Mio. Avec 4 milliards de numéros, aucune
   chance d'épuisement.
5. **Les appareils ont leur numéro**, par la même règle ; la définition d'un point porte celui de
   son appareil (`Def.appareil`), qui ouvre la colonne de santé.

Les colonnes grandissent si la maison grandit : un tableau Rust se déplace alors en mémoire, une
fois de temps en temps (copier 100 Kio prend environ 12 µs). Le numéro reste valable, puisque
personne ne garde d'adresse. Au démarrage, on réserve la taille relue plus une marge, et le tableau
ne bouge plus en marche normale.

### 4.2 Pourquoi ils doivent être fixes

Ce n'est pas seulement pratique. Le **journal de reprise** de la V7 (§ 12.1) écrit une ligne par
changement : `seq u64, point u32, charge, instant u64`. L'**instantané binaire** range l'état vif par
numéro. La **machine de secours** rejoue le même journal. Si le numéro 1234 désignait la
température du salon avant un redémarrage et la porte d'entrée après, le journal rejoué mentirait.
**Le numéro fixe est une condition de la reprise et du secours**, et ta règle « les ID doivent être
fixes » est la bonne.

### 4.3 Ce que le numéro n'est pas

- **Pas une identité.** Les humains, les profils, les définitions d'automatismes, les sujets MQTT,
  `moli.db` (qui a son propre `point_id`) et l'archive gardent les noms. Deux installations
  numérotent différemment ; un import ou un export passe par les noms.
- **Jamais montré à un humain**, jamais tapé, jamais dans une URL.
- **Typé** : un numéro de point, un numéro d'appareil et un numéro de nœud sont trois types Rust
  différents (`Point(u32)`, `Appareil(u32)`, `Noeud(u32)`). Le compilateur refuse alors d'ouvrir la
  colonne des santés avec un numéro de point. C'est la seule protection à ajouter, et elle ne coûte
  rien à l'exécution.

Ton « index où tous les ID sont à la suite » existe donc : **c'est l'annuaire**. Les noms y sont
bout à bout, dans l'ordre des numéros, avec une table de hachage qui ne garde que des numéros. Il
pèse **52 octets par nom** (39 caractères en moyenne), une seule fois pour toute la maison.

### 4.4 Sur MQTT : faut-il faire voyager le numéro ?

Dans le binaire, le kit donne au module un numéro à la déclaration (`Point(n)`). Aucun nom ne
circule sur le chemin chaud ; la V7 le prévoyait (§ 11.4.5).

Sur MQTT, les abonnés de `val/` (moteur, module WebSocket, base de données) reçoivent un sujet par
nom. Deux options :

| | A. Les noms seulement | B. Le numéro voyage aussi |
|---|---|---|
| Contrat | inchangé | `meta/` donne à chaque point son numéro `n` ; `val/` porte `"i":1234` |
| Abonné | traduit le nom (34 à 48 ns), tient sa propre table | indexe un tableau directement |
| Mémoire par abonné | ≈ 52 o par nom suivi | aucune table |
| Coût | aucun | + 9 octets par message `val/` (115 → 124 o) |
| Risque | aucun | un client extérieur prend le numéro pour une identité |

Le coût en processeur ne départage pas : 48 ns × 239 valeurs par seconde à la pointe, c'est 11 µs
par seconde. Le choix porte sur la simplicité des abonnés et sur le contrat.

**Décidé le 23 octobre : B, le numéro voyage en plus du nom.**

Bloc technique — le numéro dans le contrat MQTT (V7 § 7.3 et § 7.4) :

```text
meta/salon_thermo     {"points": {"temperature": {"n": 1234, "kind": "numeric", "unit": "°C", "echelle": 2, …}, …}}
val/salon_thermo/temperature    {"v":2150,"o":1792675624123,"r":1792675624125,"c":1792675624123,"s":918273,"i":1234}
```

Les règles du numéro sur MQTT :

- **`i` est un rang de l'installation** : stable, jamais redonné, jamais une identité. Le sujet
  garde le nom, et un client extérieur s'en tient aux noms.
- **Un abonné qui reçoit un `i` qu'il ne connaît pas redemande `meta/`** ; il ne devine jamais.
- **Le message `val/` passe de 115 à 124 octets** (125 à ×10, où le numéro a cinq chiffres).

## 5. La fiche : une taille fixe pour des valeurs qui ne le sont pas

### 5.1 Les fiches et les colonnes

| Colonne | Par | Octets | Contenu |
|---|---|---|---|
| `Def` | point | 16 | appareil, libellé, clé, dictionnaire, nature, unité, échelle, sens |
| `Vif` | point | 40 | valeur, précédente, observé (ms), changé (ms), reçu (s), genres, qualité |
| `perime` | point | 4 | reçu + fraîcheur exigée ; « jamais » si aucune (**nouveau**) |
| `fraicheur` | point | 4 | `fraicheur_s` de la méta (**nouveau**) |
| `Sante` | appareil | 20 | contact, changé, rythme, rythme appris, pertes tolérées, vie |
| `echeance` | appareil | 4 | contact + rythme × (pertes + 1) ; « jamais » pour un appareil qui n'émet qu'à l'action (**nouveau**) |
| `delai` | appareil | 4 | rythme × (pertes + 1), calculé une fois (**nouveau**) |
| annuaire | point | ≈ 52 | le nom, une fois, et sa case dans la table de hachage |

Les quatre colonnes nouvelles sont ce que ce mémoire ajoute à la V7 : 8 octets par point et par
appareil, qui rendent l'expiration presque gratuite (§ 6).

| Mémoire des valeurs (octets demandés) | ×1 | ×3 | ×10 | ×100 |
|---|---|---|---|---|
| Aujourd'hui : `States` du hub + miroir du moteur | 0,68 Mio | 1,82 Mio | 6,39 Mio | trop gros pour le banc |
| **Proposé : colonnes, santé, échéances, annuaire** | **0,27 Mio** | **0,79 Mio** | **2,66 Mio** | **26,2 Mio** |
| … dont les colonnes seules | 0,15 Mio | 0,45 Mio | 1,51 Mio | 15,1 Mio |
| … dont l'annuaire | 0,11 Mio | 0,32 Mio | 1,10 Mio | 10,6 Mio |
| Par point, tout compris | **124 o** | 122 o | 123 o | 121 o |

La forme proposée porte bien plus (la précédente, trois instants, la qualité, la santé, les
échéances) et pèse deux à trois fois moins que la valeur seule d'aujourd'hui. Les définitions
d'aujourd'hui (`Device`, `PointSpec`) ne sont pas comptées ici ; la V7 les a mesurées : 719 octets
par point, contre 96 (§ 11.4.4).

### 5.2 Les valeurs de taille variable

- **Un texte** : la fiche porte son rang ; le texte vit dans un **tiroir à part**. 5 % des points
  sont des textes ; leur taille varie, pas celle de la fiche.
- **Une énumération** : un code. Le dictionnaire est dans la définition et sur `meta/`, jamais dans
  la valeur.
- **Un objet court** (borné) : comme un texte.
- **Rien** (`null`, inconnu) : un genre dans la fiche, la valeur à zéro.

Toutes les fiches gardent donc leur taille ; seul le tiroir varie. Écrire une valeur numérique
n'alloue rien : 40 octets réécrits en place, en 2 ns.

### 5.3 Ranger ensemble ce qui se lit ensemble

Deux façons de ranger les mêmes champs :

- **fiches complètes** : un `Vif` de 40 o par point, tous les champs ensemble ;
- **colonnes** : ce qu'une condition lit (valeur, changé, qualité : 16 o) dans une colonne,
  le reste ailleurs.

Pour lire une valeur au hasard, la mesure ne les départage pas (0,32 à 0,36 ns à ×1). Pour parcourir
une seule information sur tous les points, la colonne gagne : c'est le cas des échéances. D'où la
recommandation : **les fiches de la V7 (`Vif` 40 o, `Sante` 20 o), plus des colonnes de 4 octets
pour les échéances**.

## 6. Trouver ce qui a expiré

### 6.1 Trois sortes d'expiration

| Ce qui expire | Échéance | Par | Tenu par |
|---|---|---|---|
| Un appareil devient muet | dernier contact + rythme × (pertes + 1) | appareil | le hub (§ 10.5 de la V7) |
| Une valeur trop vieille pour une règle | reçu + `fraicheur_s` | point | le hub |
| Une attente du moteur | « depuis 10 min », « attendre 5 min », délai d'« attendre que », bail « pendant », suivi d'un ordre (`suivi_s`), pause d'une source (`pause_jusqu_a`) | déclencheur, exécution, ordre, module | le moteur et le hub |

### 6.2 Le calcul dit « où », la lecture dit « qui »

C'est la nuance centrale, et la réponse directe à ta question.

- **Pour une valeur donnée, l'expiration est un calcul**, sans recherche :
  `maintenant − reçu ≤ fraîcheur`. Une condition qui lit la fiche le sait en même temps que la
  valeur (moins d'une nanoseconde de plus).
- **Pour savoir lesquelles ont expiré**, le numéro ne suffit pas : il donne la place d'une fiche
  qu'on connaît, pas la liste des fiches périmées. Il faut regarder chaque échéance, ou les garder
  triées.

Or regarder chaque échéance est précisément ce que la taille fixe rend le moins cher. Une colonne de
4 octets, contiguë, se lit dans l'ordre : le processeur précharge la suite et compare 4 à 16
échéances par instruction. **C'est le cas où ton idée brille le plus.**

<!-- FIGURE:echeances -->

### 6.3 Quatre façons, jouées sur une heure de la maison

Le banc rejoue une heure de la maison. Chaque appareil émet à son rythme : capteurs Zigbee, API
interrogées toutes les 3 min, prises Wi-Fi, appareils qui n'émettent qu'à l'action, capteurs sur
pile. Cela fait 7 042 nouvelles par heure à ×1, soit 1,96 par seconde, la cadence réelle des deux
maisons. 2 % des appareils se taisent à la 20e minute.

| Méthode | Principe |
|---|---|
| **A. Calcul au passage** (le prototype de la V7) | chaque seconde, lire la santé (20 o) de chaque appareil et calculer contact + rythme × (pertes + 1) |
| **B. Colonne d'échéances** | chaque nouvelle écrit `échéance = maintenant + délai` (une addition) ; chaque seconde, le passage compare seulement |
| **R. Colonne + réveil au plus proche** | le passage rend la plus petite échéance restante ; l'échéancier dort jusque-là ; une nouvelle dont l'échéance tomberait avant avance le réveil |
| **C. File de priorité** (un tas) | chaque nouvelle empile (échéance, appareil, génération) ; les entrées dépassées sont écartées quand elles arrivent en haut |

Une **roue de minuteries** (celle de Tokio) appartient à la même famille que C : chaque nouvelle y
déplace l'entrée de l'appareil. Elle sert pour des millions de minuteries, ce que n'a pas une maison.

<!-- FIGURE:heure -->

| Une heure de la maison (Linux musl) | ×1 | ×3 | ×10 | ×100 |
|---|---|---|---|---|
| Appareils ; nouvelles dans l'heure | 496 ; 7 042 | 1 488 ; 20 014 | 4 960 ; 66 879 | 49 600 ; 686 364 |
| Appareils déclarés muets (identiques pour les quatre méthodes) | 9 | 21 | 74 | 701 |
| A : processeur pour l'heure | 2,5 ms | 8,3 ms | 29 ms | 360 ms |
| B : processeur pour l'heure | 1,0 ms | 3,0 ms | 8,7 ms | 87 ms |
| **R : processeur pour l'heure** | **0,03 ms** | **0,06 ms** | **0,20 ms** | **6,7 ms** |
| C : processeur pour l'heure | 0,45 ms | 1,2 ms | 3,6 ms | 50 ms |
| R : passages dans l'heure (au lieu de 3 600) | **26** | 38 | 84 | 395 |
| Coût d'une nouvelle : A / B / C | 0,31 / 0,59 / 4,6 ns | 0,34 / 0,62 / 4,9 ns | 0,35 / 0,64 / 4,8 ns | 0,98 / 1,1 / 5,9 ns |
| Mémoire : colonnes (B, R) / tas (C) | 3,9 / 18,7 Kio | 11,6 / 54 Kio | 39 / 181 Kio | 388 / 1 855 Kio |

<!-- FIGURE:passage -->

| Un passage, rien d'expiré | 2 264 | 22 640 | 226 400 | 2,26 millions |
|---|---|---|---|---|
| Colonne, par blocs de 16 (réglage de production) | **0,71 µs** | 7,2 µs | 72 µs | 661 µs |
| Colonne, une case après l'autre | 1,2 µs | 12 µs | 112 µs | 1 099 µs |
| Colonne, par blocs, réglée pour la vitesse (`opt-level = 3`) | 0,17 µs | 1,3 µs | 13 µs | 130 µs |

Ce que disent les mesures :

- **À l'échelle d'une maison, tout est négligeable** : même à ×100, la méthode la plus chère prend
  360 ms sur une heure, soit 0,01 % d'un cœur. Le choix se fait donc sur la simplicité et la
  justesse.
- **R est la plus économe et presque la plus simple** : une colonne, une addition par nouvelle, un
  passage quand une échéance peut tomber. L'échéancier connaît en plus l'instant exact du prochain
  réveil, sans tourner chaque seconde.
- **C économise les passages mais coûte partout ailleurs** : 8 fois plus cher par nouvelle, 5 fois
  plus de mémoire, une structure de plus à tenir juste. Le tas grossit avec le débit des nouvelles,
  pas avec le nombre d'appareils.
- **Le réglage « taille » de la production** (`opt-level = "s"`) coûte un facteur 4 à 6 sur ce
  passage : le compilateur y vectorise moins. C'est sans importance ici (0,71 µs). S'il le fallait
  un jour, Cargo permet de régler une seule crate pour la vitesse
  (`[profile.release.package.<crate>]`).

**Recommandation : R**, une colonne d'échéances de 4 octets par sorte d'expiration, lue au réveil
le plus proche. Les colonnes concernées sont celles des appareils, des points à fraîcheur exigée, des
attentes du moteur et des exécutions (§ 8). C'est un seul mécanisme, écrit une fois.

### 6.4 Ce qui se passe à l'échéance

- **Un appareil devient muet** :
  - sa santé passe à `muet`, et `avail/` le dit avec sa raison (« aucune nouvelle depuis 6 min ») ;
  - ses valeurs deviennent mauvaises **par calcul à la lecture** : qualité effective = qualité de
    la fiche, sauf si l'appareil est muet. Aucune fiche n'est réécrite. Le prototype de la V7
    réécrivait chaque point de l'appareil ; la lecture coûte 0,73 ns au lieu de 0,32 ;
  - le hub publie une fois la mauvaise qualité de ses points sur `val/`. Leurs numéros se suivent :
    c'est une plage, pas une recherche.
- **Une valeur trop vieille pour une règle** : la règle la lit « inconnue », calculé à la lecture.
- **La première nouvelle** réarme l'échéance par l'écriture elle-même (l'addition de la méthode B).

### 6.5 Deux horloges

- **Les échéances** sont en secondes d'une horloge **monotone** depuis le démarrage (`u32` :
  136 ans). Une horloge murale qui saute (NTP, réglage à la main) ne doit ni déclarer un appareil
  muet, ni allonger une attente.
- **Les instants affichés et conservés** (observé, changé) restent en millisecondes d'horloge
  murale (`u64`) : ce que voient les humains et l'historique.
- **Au démarrage**, les échéances se recalculent depuis les instants persistés, avec une grâce :
  `contact = max(contact persisté, démarrage)`. Un appareil reçoit un délai complet après un
  redémarrage : si le hub n'écoutait pas, ce n'est pas la faute de l'appareil.

## 7. Lire à plusieurs sans se gêner

Qui lit les valeurs :

- le hub, seul rédacteur, et l'échéancier ;
- le moteur de workflow, par son miroir tenu par `val/` (V7 § 13) ;
- le module WebSocket, l'API et les agents (MCP) ;
- le journal et l'instantané.

| Façon de partager | Coût mesuré | Risque |
|---|---|---|
| **Un seul propriétaire** (le fil du hub) ; les autres reçoivent des messages ou des copies | lecture interne 0,32 ns ; copie de tout l'état vif (valeurs et santé) : 12 µs à ×1, 115 µs à ×10 | aucun |
| Verrou lecteur-rédacteur | 3,5 ns par lecture, sans rival | un lecteur lent retient le rédacteur |
| Instantané partagé (copie publiée à chaque lot) | une copie par lot : 12 µs à ×1 | mémoire doublée, plus de code |
| Lectures sans verrou (`seqlock`, atomiques) | ≈ 1 ns | code `unsafe`, lectures déchirées à prévenir |

**Recommandation : un seul propriétaire, et des copies.** Le fil du hub possède les colonnes,
écrit et passe les échéances, sans verrou sur le chemin chaud. Un écran, l'API ou un agent reçoit
une copie : copier toute la maison coûte 12 µs. Le moteur garde son miroir (16 o par point lu,
V7 § 13), indexé par le même numéro si le numéro voyage (§ 4.4). **Copier plutôt que partager**,
c'est possible justement parce que tout est compact et de taille fixe.

## 8. Les workflows : une mémoire à part

### 8.1 Ce qui existe aujourd'hui

Faits relevés dans le code de `main` (`bd1714d`) :

| Sujet | Aujourd'hui | Où |
|---|---|---|
| Définitions | `automations.json` chargé entier : graphes de `Node` et `Edge` identifiés par des chaînes, version validée recopiée (`approved_version`) | `model.rs`, `store.rs` |
| Copie des actifs | le moteur garde en plus la liste des automatismes actifs | `engine.rs:336`, `lib.rs:796` |
| Mémoire mesurée | **1,9 Mio** pour 400 automatismes, plus **1,5 Mio** de copie des actifs : près de 9 Kio par automatisme | banc |
| Plafond | 400 automatismes au plus | `lib.rs:994` |
| Un changement de valeur | `on_state` parcourt tous les automatismes actifs et tous leurs déclencheurs, et compare les noms de points | `engine.rs:351` |
| Suivre un lien | `Graph::next` parcourt toutes les arêtes, par nom | `model.rs:125` |
| Une règle d'état | `holds` relit le hub par nom : `Hub::state` alloue un `DeviceId`, prend le verrou, deux recherches, une copie ; 70 à 91 ns | `run.rs:594` |
| Une durée (`for_s`) | une tâche Tokio qui dort, plus une entrée dans une table de chaînes ; **629 octets**, perdue au redémarrage | `engine.rs:458-476` |
| « Attendre » | une attente Tokio dans la tâche de l'exécution ; perdue au redémarrage | `run.rs:396` |
| « Attendre que » | chaque exécution qui attend s'abonne à **tout** le bus du hub, et réévalue sa règle à chaque événement de la maison, quel que soit le point | `run.rs:410-429` |
| Données entre nœuds | seulement le texte écrit par Sarah (`{{texte}}`) | `run.rs:124` |
| Historique des exécutions | 500 exécutions gardées en mémoire, avec le détail de chaque étape en texte | `store.rs:13` |
| Modifier la logique | arrête les exécutions et les durées en cours ; la nouvelle version attend la validation d'un humain | `lib.rs:436-474` |

### 8.2 Trois mémoires, pas une

Ta remarque est juste : **la mémoire des workflows est à part de celle des valeurs**. Elle se divise
elle-même en deux, selon ce qui change et à quel rythme.

<!-- FIGURE:trois -->

| Mémoire | Ce qu'elle dit | Qui l'écrit | Change | Survit au redémarrage |
|---|---|---|---|---|
| **Les valeurs** | ce que la maison est | le hub | à chaque nouvelle | journal de reprise et instantané |
| **Le programme** | ce que les workflows sont | le compilateur du moteur | à chaque validation humaine | jamais écrit : recompilé (0,23 ms) depuis les définitions |
| **Les exécutions** | ce que les workflows font, et la vérité des conditions composées | le moteur | à chaque pas, à chaque changement d'une feuille | journal du moteur ; reprise si l'échéance n'est pas dépassée de plus de 2 min (§ 10) |

Quatre règles les tiennent séparées :

1. **Le programme cite les valeurs par numéro, il ne les copie jamais.** Une règle porte le numéro
   de son point ; elle lit la fiche quand elle s'évalue.
2. **Une exécution cite le programme par numéro**, jamais l'inverse : le programme est figé et
   partagé.
3. **Les valeurs ne savent rien des workflows.** Le seul pont est l'index par point, qui appartient
   au programme : « le point n a des déclencheurs ici ».
4. **Chaque mémoire a sa durée de vie et sa persistance** ; aucune ne dépend du format d'une autre.

### 8.3 Le programme : des nœuds numérotés

Le même principe, appliqué au graphe :

- les **automatismes** ont un numéro fixe, par un annuaire des automatismes, jamais redonné ;
- un **plan** (12 o) par automatisme dit où commencent ses nœuds ;
- les **nœuds** (32 o, deux par ligne de cache) sont rangés bout à bout ; un nœud dit ce qu'il fait
  (`op`), sur quel point, avec quelle valeur, et où commencent ses liens ;
- les **liens** (8 o) sont groupés par nœud, dans l'ordre validé : le port et le nœud suivant ;
- les **règles** (16 o) : une comparaison, une plage horaire, le soleil ;
- les **textes** (gabarits de messages, consignes de Sarah) sont bout à bout, comme l'annuaire ;
- l'**index par point** : deux tableaux. `debut[n]` dit où commencent les déclencheurs du point n
  dans `index`, et `debut[n+1]` où ils finissent. Le numéro du point donne la place, encore.

<!-- FIGURE:programme -->

| Le programme, mesuré | ×1 : 400 automatismes | ×3 | ×10 |
|---|---|---|---|
| Définitions en mémoire aujourd'hui (liste + copie des actifs) | 3,45 Mio | 10,6 Mio | 33,9 Mio |
| **Programme compilé** | **0,14 Mio** | 0,43 Mio | 1,44 Mio |
| … nœuds / liens / règles / textes / index | 75 / 16 / 19 / 23 / 10 Kio | | |
| Par automatisme | **376 o** au lieu de 9 Kio | | |
| Tout recompiler | **0,23 ms** | 0,74 ms | 2,7 ms |
| Relire `automations.json` au démarrage | 4,1 ms | 13,6 ms | 41 ms |

Recompiler tout le programme coûte moins d'une milliseconde à ×1. Il se rebâtit donc **entier** à
chaque validation, et il remplace l'ancien d'un coup. Aucune mise à jour partielle n'est à écrire,
ni à tester.

Une exécution désigne son nœud par **(numéro d'automatisme, rang du nœud dans l'automatisme)**, pas
par sa place globale : après une recompilation, `plans[a].premier + rang` donne la nouvelle place.
Les exécutions d'un automatisme inchangé continuent donc sans rien savoir. Celles d'un automatisme
dont la logique a changé s'arrêtent, comme aujourd'hui (`lib.rs:436-474`) : la nouvelle version
attend la validation d'un humain, et l'ancienne ne doit plus agir.

<!-- FIGURE:evenement -->

| Un changement de valeur, côté moteur (ns) | ×1 | ×3 | ×10 |
|---|---|---|---|
| Aujourd'hui, point quelconque (balayage, rien ne se déclenche) | 826 | 1 100 | 1 608 |
| Aujourd'hui, point déclencheur | 1 467 | 3 836 | 11 074 |
| **Index, point quelconque** (`debut[n] = debut[n+1]` : deux lectures) | **1,1** | 1,1 | 1,1 |
| **Index, point déclencheur** | **5,5** | 5,5 | 5,6 |
| Une exécution, du déclencheur jusqu'à l'attente de 5 min (un « si » de trois règles, un ordre) | 11,6 | 12,5 | 13,8 |

Le balayage d'aujourd'hui grandit avec le nombre d'automatismes ; l'index ne dépend que des
déclencheurs du point. Le banc mesure le balayage d'aujourd'hui au mieux : sans la traduction de la
valeur attendue (`canonical_expected`), ni le texte de la cause, ni la tâche lancée par
déclenchement.

### 8.4 Les exécutions : l'état du flux, à la n8n

Dans **n8n**, chaque nœud passe des données (des *items*) au suivant, et l'exécution les garde ; un
nœud « Wait » range l'exécution et la reprend plus tard. C'est un flux **avec état** : il emporte
ses données.

Dans Sarah Home, ce qu'un flux emporte est petit et connu à la compilation :

- sa **cause** : le point, la valeur, l'instant qui l'ont déclenché ;
- le **texte** qu'a écrit Sarah (`{{texte}}`) ;
- demain, une valeur lue dans l'historique, la branche choisie par « décider ».

D'où une **fiche d'exécution de 64 octets**, une ligne de cache, plus un tiroir pour les seuls
textes. Une moitié dit où elle en est. L'autre garde **l'état qui l'a lancée, pour toute la
suite**, comme tu l'as demandé :

| Champ | Octets | Rôle |
|---|---|---|
| automatisme | 4 | son plan |
| curseur | 4 | le rang du nœud où elle en est |
| réveil | 4 | son échéance (attendre, délai d'« attendre que ») ; « jamais » sinon |
| départ | 4 | quand elle a commencé |
| demande | 4 | l'ordre en vol dont elle attend l'issue sur `res/` |
| génération | 2 | la case est réutilisée : une issue tardive, destinée à l'occupante d'avant, est écartée |
| état | 1 | en cours, attend une durée, attend une condition, attend une issue |
| version | 1 | version du programme |
| **cause : point** | 4 | le point qui a déclenché, ou la feuille qui a fait basculer une condition composée |
| **cause : déclencheur** | 4 | le nœud déclencheur qui est parti |
| **cause : instant** | 8 | quand (ms, horloge murale) |
| **cause : valeur, précédente** | 8 + 8 | la transition, au canon |
| registre | 8 | une donnée produite en route : le rang du texte de Sarah, une moyenne lue |

Ici, à l'inverse des points, **la case est réutilisée** : une exécution est éphémère, il y en aura
des millions par an. La génération remplace la règle « jamais redonné » : une réponse qui porte
(case 12, génération 7) ne touche pas l'exécution (case 12, génération 8). Finir une exécution et en
lancer une autre coûte **4,3 ns** (fiche de 64 o, mesuré). Les modes d'aujourd'hui se traduisent sans structure de plus :
le plan garde la case de son exécution en cours ; `restart` la finit et relance, `single` ignore,
`queued` garde une petite file.

**La question que pose le flux avec état**, et ta décision : après « attendre 5 min », que lit un
« si » ?

<!-- FIGURE:n8n -->

- **Une condition lit la mémoire des valeurs**, au moment où elle passe : la maison telle qu'elle
  est, pas une copie emportée au déclenchement.
- **La cause est gardée pour toute la suite.** Un nœud peut la lire explicitement (« le point qui a
  déclenché », « la valeur qui a déclenché ») pour régler, décider ou écrire. Dans une règle
  compilée, une opérande est soit un point, lu dans la mémoire des valeurs, soit la cause, lue dans
  la fiche d'exécution ; l'éditeur dit laquelle.
- **Une étape franchie ne se rejoue pas.** Si la valeur change après le « si », le flux ne revient
  pas en arrière : il avance. Seuls un nouveau déclenchement (mode `restart`) ou un « attendre que »
  réagissent au changement.

Aujourd'hui, le moteur lit déjà la mémoire des valeurs (`holds` relit le hub). De la cause, il ne
garde que la phrase affichée (`Run.why`), et seul le texte de Sarah voyage entre les nœuds. La
proposition garde la lecture actuelle et ajoute la cause à la fiche, typée et au canon.

### 8.5 Une durée est un calcul

« La porte est ouverte depuis 10 minutes » se calcule :
`valeur = ouverte ∧ maintenant − changé ≥ 600 s`. **Dans un « si », aucune minuterie** : la fiche du
point porte l'instant « changé ».

Comme **déclencheur** (`for_s`), il faut un réveil au bon moment. À la transition, le moteur écrit
une **attente de 16 octets** dans une case fixée par le compilateur (une par déclencheur à durée) :
`échéance = maintenant + 600`, `depuis = changé`. À l'échéance, trouvée par le même passage qu'au
§ 6, une seule vérification : **l'instant « changé » du point est-il toujours `depuis` ?**

- oui : la valeur n'a pas bougé depuis 10 minutes, le déclencheur part ;
- non : elle est partie, peut-être revenue. Cette attente ne vaut plus rien, et la nouvelle
  transition a armé la sienne.

**Aucun désarmement, aucune annulation, aucune tâche** : l'instant « changé » sert de numéro de
version à la valeur. Une attente coûte 16 octets au lieu de 629, et elle survit au redémarrage si on
la range, puisque l'instant « changé » est lui-même conservé.

Une condition composée en « et » suit la même règle : elle est vraie depuis le plus grand des
instants « changé » de ses feuilles. « Toutes les fenêtres fermées depuis 10 min » se revérifie donc
sur cet instant, sans rien ranger de plus (§ 9.5).

« Attendre que » suit le même chemin. Le compilateur sait quels points lit la règle ; un événement
sur un de ces points, et seulement lui, fait relire la colonne des exécutions en attente (400
fiches : 0,17 µs). Aujourd'hui, chaque attente se réveille à chaque événement de la maison (§ 9.2).

| Mémoire des workflows (octets demandés) | ×1 | ×3 | ×10 |
|---|---|---|---|
| Exécutions en cours, au pire une par automatisme (fiches de 64 o) | 32 Kio | 128 Kio | 256 Kio |
| Attentes des déclencheurs à durée (un sur quatre) | 1,6 Kio | 4,7 Kio | 15,6 Kio |
| Réseau des conditions composées, au pire (§ 9.5) | 55 Kio | 132 Kio | 475 Kio |
| Passer les réveils des exécutions | 0,17 µs | 0,33 µs | 1,0 µs |
| **Total : programme, exécutions, attentes, réseau** | **0,23 Mio** | **0,69 Mio** | **2,2 Mio** |
| Aujourd'hui : définitions et copie des actifs, sans les exécutions | 3,45 Mio | 10,6 Mio | 33,9 Mio |

## 9. Calculer les conditions : cinq façons confrontées

### 9.1 Les cinq façons

Une **analyse morphologique** : toutes les façons connues de calculer une condition, côte à côte,
chacune empruntée à un domaine (les **analogies**).

| # | Façon | D'où elle vient | Ce qu'elle fait |
|---|---|---|---|
| 1 | **Balayage à chaque événement** | le moteur d'aujourd'hui | chaque événement parcourt tous les déclencheurs ; une condition relit l'état par nom |
| 2 | **Index par point, calcul à la demande** | la V7 (§ 13), les bases de données (index) | l'événement réveille les déclencheurs de son point ; un « si » se calcule quand l'exécution y arrive, en lisant les fiches par numéro |
| 3 | **Réseau incrémental** | les systèmes experts (l'algorithme RETE) | chaque règle garde sa vérité ; un changement ne recalcule que les règles de son point ; un « si » est déjà calculé quand on y arrive ; un déclencheur peut porter sur une condition composée |
| 4 | **Cycle d'automate** | les automates industriels (API, PLC) | toutes les 100 ms, toutes les conditions sont recalculées et comparées au tour d'avant |
| 5 | **Durées par les instants** | transversale | « depuis X » se calcule sur l'instant « changé » ; une attente se revérifie à son échéance (§ 8.5) |

### 9.2 Ce que disent les mesures

<!-- FIGURE:conditions -->

| Mesuré (Linux musl) | ×1 | ×3 | ×10 |
|---|---|---|---|
| 1. Une règle d'état aujourd'hui (`holds` : relire le hub par nom) | 70 ns | 91 ns | 91 ns |
| 2. Un « si » de trois règles, à la demande, par numéro | **4,6 ns** | 5,8 ns | 7,7 ns |
| 3. Réseau : un changement sur un point quelconque / lu par une règle | 1,7 / 4,1 ns | 1,8 / 4,1 ns | 1,8 / 4,1 ns |
| 3. Réseau : la bascule « nuit », lue par tous les automatismes | 0,77 µs | 2,0 µs | 6,5 µs |
| 3. Réseau : mémoire de l'état de vérité | 52 Kio | 123 Kio | 443 Kio |
| 4. Cycle d'automate : un tour, toutes les conditions | 1,8 µs | 7,0 µs | 31 µs |
| 5. Une durée armée : aujourd'hui (tâche Tokio) / proposé (attente) | 629 / 16 o | 594 / 16 o | 612 / 16 o |
| « Attendre que » aujourd'hui : coût d'un événement avec 10 / 100 / 400 attentes | 0,72 / 6,6 / 29 µs | 0,70 / 6,7 / 27 µs | 0,64 / 7,0 / 25 µs |

Ramené à une seconde de pointe (239 changements par seconde, ×1) :

- le balayage d'aujourd'hui : 197 µs par seconde (0,02 % d'un cœur) ;
- **« attendre que » d'aujourd'hui avec 400 attentes : 6,9 ms par seconde (0,7 % d'un cœur)**. Sur
  un Pi Zero 2 W, plusieurs fois plus lent, cela ferait plusieurs pour cent (estimé) ;
- le cycle d'automate à 10 tours par seconde : 18 µs par seconde ;
- le réseau incrémental : 0,4 µs par seconde ; l'index : 0,3 µs par seconde.

Le seul coût visible d'aujourd'hui est donc « attendre que », qui réveille chaque attente à chaque
événement. Tout le reste est négligeable : **le choix se fait sur d'autres critères**.

### 9.3 Le choix, à critères visibles

| Critère | 1. Balayage | **2. Index, à la demande** | 3. Réseau | 4. Cycle |
|---|---|---|---|---|
| Simplicité (code à écrire et à tenir) | 3 | **3** | 1 : un état de vérité à reconstruire au démarrage et après une perte de messages | 3 |
| Coût (processeur, mémoire) | 1 : grandit avec les automatismes | **3** | 3 | 2 |
| Latence | 3 | **3** | 3 | 1 : jusqu'à un tour (100 ms) |
| Reprise après redémarrage | 3 | **3** : rien à reconstruire | 1 | 3 |
| Explicabilité (« pourquoi il est parti, pourquoi pas ») | 2 | **3** : on rejoue la règle sur les fiches et leurs instants | 3 | 2 |
| Puissance (déclencheur sur une condition composée) | 1 | 2 : un point par déclencheur, le reste dans un « si » | **3** | 3, mais il rate une impulsion plus courte qu'un tour |
| **Total (sur 18)** | 13 | **17** | 14 | 14 |

**Retenu : 2 et 5 pour les « si » et les déclencheurs simples, 3 pour les déclencheurs composés.**
C'est la V7, précisée et complétée.

- **Le réseau incrémental (3)** sert aux déclencheurs sur une condition composée (« quand toutes
  les fenêtres sont fermées »). **Tu les veux dès la V7** : il s'ajoute au-dessus de l'index, sans le
  remplacer (§ 9.5). Le défaut noté dans ce tableau, la reprise, ne coûte en fait qu'une
  reconstruction, mesurée à 3,6 µs.
- **Le cycle d'automate (4)** est écarté comme mécanisme principal : il rate une impulsion plus
  courte qu'un tour (un appui de bouton), qui doit de toute façon rester un événement. La V7 détecte
  déjà une valeur perdue par le rang `s` de `val/`.

### 9.4 Les plages horaires et le soleil deviennent des valeurs

Une idée tirée du réseau incrémental : « il fait nuit » et « entre 22 h et 6 h en semaine »
deviennent des **points tenus par le hub** (`proprietaire = hub`, V7 § 5.1). L'échéancier les bascule
à l'heure dite. Alors :

- toute règle est une règle sur une valeur, lue par numéro ; le moteur ne convertit plus l'heure
  locale à chaque évaluation (`holds` le fait aujourd'hui pour chaque règle horaire) ;
- un déclencheur horaire est un déclencheur sur un point, dans le même index ;
- l'écran explique : « nuit : oui depuis 21 h 12 ».

Le coût mesuré est une bascule « nuit » lue par les 400 automatismes : 0,77 µs, deux fois par jour.
Il y a un point par plage distincte, quelques-uns par maison.

### 9.5 Les déclencheurs composés

Décidé le 23 octobre : dès la V7, un automatisme peut partir quand une **condition composée**
devient vraie : « quand toutes les fenêtres sont fermées », « quand personne n'est là et que
l'alarme est armée ».

<!-- FIGURE:compose -->

Avec les mêmes principes :

- **la structure est dans le programme**, figée : les feuilles (une comparaison sur un point, ou sur
  un point virtuel « nuit », « 22 h–6 h »), les conditions (« et », « ou », imbriquées), et un index
  par point, comme pour les déclencheurs simples ;
- **la vérité vit à part**, dans la mémoire des exécutions : un octet par feuille, un compteur de
  2 octets par condition. Un « et » est vrai quand son compteur atteint le nombre de ses feuilles ;
  un « ou », dès qu'il dépasse zéro ;
- **un changement ne recalcule que les feuilles de son point** (1,7 à 4,1 ns), puis remonte aux
  conditions dont le compteur bouge ;
- **le déclencheur part sur la bascule** de fausse à vraie, jamais sur un état : une condition déjà
  vraie ne repart pas à chaque changement ;
- **sa cause est la feuille qui a fait basculer la condition** : son point, sa valeur, la précédente,
  l'instant ;
- **la durée composée se calcule** : une condition en « et » est vraie depuis le plus grand des
  instants « changé » de ses feuilles. L'attente de 10 min se revérifie à l'échéance sur cet
  instant, comme pour un point seul ;
- **les changements d'un même message sont appliqués ensemble** avant de lire les bascules : une
  fenêtre qui se ferme pendant qu'une autre s'ouvre ne fait pas partir le déclencheur entre les
  deux ;
- **une feuille sur une valeur mauvaise est fausse.** Quand un appareil devient muet, ses feuilles
  sont relues (ses points sont une plage de numéros), et rien ne part sur une valeur qu'on ne connaît
  plus.

Le défaut du réseau, un état de vérité à reconstruire, ne coûte presque rien. On le recalcule d'un
coup depuis la mémoire des valeurs : au démarrage, après une recompilation, et après une perte de
messages sur `val/`.

| Réseau, au pire (un déclencheur composé de trois feuilles par automatisme) | ×1 | ×3 | ×10 |
|---|---|---|---|
| Feuilles ; conditions | 1 200 ; 400 | 3 600 ; 1 200 | 12 000 ; 4 000 |
| Structure figée (dans le programme) | 49 Kio | 113 Kio | 415 Kio |
| Vérité : un octet par feuille, compteur et instant par condition | 6,6 Kio | 19 Kio | 59 Kio |
| Reconstruire toute la vérité | **3,6 µs** | 12 µs | 46 µs |
| Un changement : point quelconque / lu par une feuille | 1,7 / 4,1 ns | 1,8 / 4,1 ns | 1,8 / 4,1 ns |

## 10. Ce qui survit à un redémarrage

| Mémoire | Au redémarrage | Coût mesuré |
|---|---|---|
| Annuaire des points et des appareils | relu ; les numéros sont les mêmes | — |
| Valeurs | instantané binaire puis journal de reprise (V7 § 12.1) ; échéances recalculées avec une grâce (§ 6.5) | instantané des valeurs : 29 µs à ×1, 273 µs à ×10 |
| Programme | recompilé depuis les définitions | 0,23 ms + 4,1 ms de lecture du JSON à ×1 |
| Attentes des déclencheurs à durée | encore valables si l'instant « changé » du point n'a pas bougé | 16 o chacune |
| Exécutions en attente (« attendre », « attendre que ») | reprennent, avec leur cause, si leur échéance n'est pas dépassée de plus de 2 min ; sinon arrêtées, notées « interrompue » (décidé) | 64 o chacune |
| Vérité des conditions composées | reconstruite d'un coup depuis les valeurs | 3,6 µs à ×1, 46 µs à ×10 |
| Baux « pendant » | rétablissent la valeur à leur terme, même en retard : revenir à l'état d'avant est le sens sûr | — |
| Historique des exécutions | sur disque et sur `evt/`, en codes ; en mémoire, une file courte de fiches fixes pour l'écran | non mesuré |

## 11. Les choix à faire

Décidé ici, sans question, parce que ta demande ou le code le tranche :

| Sujet | Décision | Pourquoi |
|---|---|---|
| Numéros des points et des appareils | fixes, jamais redonnés, écrits avant usage | ta règle ; la reprise et le secours l'exigent (§ 4.2) |
| Numéros des exécutions | cases réutilisées, avec une génération | éphémères, des millions par an (§ 8.4) |
| Modifier la logique d'un automatisme | arrête ses exécutions en cours ; la nouvelle version attend un humain | le code le fait déjà (`lib.rs:436-474`) ; règle de validation |
| Expiration | colonnes de 4 o, réveil au plus proche | mesuré (§ 6.3) |
| Partage | un seul propriétaire, des copies | mesuré (§ 7) |
| Conditions | index par point, calcul à la demande, durées par les instants | mesuré et noté (§ 9.3) |
| Taille des colonnes | réservée au démarrage, avec une marge ; elles peuvent grandir | le numéro survit au déplacement (§ 4.1) |
| Taille de la fiche `Vif` | 40 o, la variante 32 o prête | à mesurer sur le Pi (§ 3.4) |

Décidé par toi le 23 octobre :

| Question | Ta réponse | Ce qui en découle |
|---|---|---|
| Après une attente, une condition lit-elle la valeur actuelle ou celle du déclenchement ? | **La valeur actuelle**, avec ta remarque : une étape franchie ne se rejoue pas, et la suite a besoin de l'état qui a déclenché | lecture dans la mémoire des valeurs ; cause complète dans la fiche d'exécution (64 o) ; une opérande « cause » dans les règles ; le flux avance seulement (§ 8.4) |
| Après un redémarrage, une exécution qui attendait reprend-elle ? | **Oui, si elle est à l'heure** | reprise si l'échéance n'est pas dépassée de plus de 2 min, sinon « interrompue » ; baux « pendant » toujours rétablis ; fiches d'exécution et attentes au journal du moteur (§ 10) |
| Faut-il des déclencheurs sur une condition composée ? | **Oui, dès la V7** | réseau incrémental au-dessus de l'index ; vérité à part, reconstruite en 3,6 µs (§ 9.5) |
| Le numéro voyage-t-il sur MQTT ? | **Oui, en plus du nom** | `n` dans `meta/`, `i` dans `val/` ; rang de l'installation, jamais une identité (§ 4.4) |

## 12. Ce que ce mémoire change dans la V7

| Section de la V7 | Changement |
|---|---|
| § 10.5 la santé | l'échéance est écrite dans une colonne de 4 o à chaque nouvelle, au lieu d'être calculée au passage ; réveil au plus proche ; qualité effective calculée à la lecture, sans réécrire les fiches ; grâce au démarrage |
| § 11.2 la valeur | quatre colonnes de 4 o (échéance et délai par appareil, fraîcheur et péremption par point) |
| § 11.4.4 les types compacts | l'annuaire est persistant, écrit avant usage ; numéros jamais redonnés ; numéros typés (`Point`, `Appareil`, `Noeud`) |
| § 12.1 le journal de reprise | confirmé : il cite les points par numéro, ce qui exige des numéros fixes |
| § 13.1 le modèle | un déclencheur « quand une condition composée devient vraie », avec ou sans durée ; la cause d'une exécution, lisible par tout nœud ; un flux qui avance seulement |
| § 13.2 le moteur | `Noeud` devient une fiche de 32 o, rangée dans une table ; liens, règles et textes en tables ; index par point en deux tableaux ; exécutions de 64 o à génération, qui gardent leur cause ; attentes de 16 o revérifiées par l'instant « changé » ; « attendre que » indexé ; plages horaires et soleil en points du hub ; réseau incrémental pour les déclencheurs composés, vérité à part, reconstruite d'un coup |
| § 7.3 et § 7.4 le contrat | `n` dans `meta/`, `i` dans `val/` (décidé) ; un message `val/` passe de 115 à 124 octets |

Trois défauts du moteur actuel se corrigent indépendamment de la V7, et peuvent faire l'objet de
tickets :

- « attendre que » réveille chaque attente à chaque événement de la maison (`run.rs:427`) ;
- une durée armée est une tâche Tokio perdue au redémarrage (`engine.rs:476`) ;
- chaque changement de valeur parcourt tous les déclencheurs (`engine.rs:351`).

## 13. La vérification de bout en bout : une soirée de la maison

Tu as demandé de vérifier la conception sur un workflow long, avec état, plusieurs conditions
d'entrée, des attentes et la vérification des nouvelles valeurs. C'est fait par un **moteur de
référence complet** (`tools/memoire-banc/src/moteur.rs`), qui suit exactement les règles de ce
mémoire. Une **soirée de la maison** (`src/scenario.rs`) le rejoue seconde par seconde, de 18 h à
23 h 15, et **six tests** en vérifient le comportement. Tout est rangé dans le dépôt pour être
rejoué (`tools/memoire-banc/README.md`).

<!-- FIGURE:soiree -->

### 13.1 Les automatismes de la soirée

| Automatisme | Ce qu'il fait | Ce qu'il éprouve |
|---|---|---|
| **A · Coucher de la maison** (23 nœuds, mode `single`) | **deux entrées** : fenêtres et porte fermées la nuit depuis 10 min (condition composée de six feuilles), ou le bouton « bonne nuit ». Puis : un « si » de **quatre règles** (personne au salon, alarme désarmée, entre 21 h et 2 h, maison pas déjà en nuit) ; la température **lue dans le registre** ; éteindre le salon ; attendre 5 min ; un « si » **relu après l'attente** ; fermer le volet et **vérifier sa nouvelle valeur** ; armer l'alarme et **vérifier** ; passer la maison en **mode nuit** ; attendre 30 min ; une consigne de chauffage **jamais confirmée** ; une fin **selon la cause** | le déclencheur composé avec durée, les entrées multiples, l'état produit en route, le flux qui n'avance que vers l'avant, les vérifications réussies et échouées, la cause gardée à travers une panne et une recompilation |
| **B · Couloir** (8 nœuds, mode `restart`) | au mouvement : la veilleuse si la maison est en nuit, la lumière sinon, pendant 2 min | la relance, l'issue tardive, l'interruption au redémarrage, l'arrêt par modification, la version modifiée |
| **C · Porte ouverte** | la porte passée de fermée à ouverte depuis 5 min : une alerte si la maison est en nuit | une attente qui échoit pendant la panne, revérifiée au redémarrage |
| **D · Extinction auto** | une lumière allumée depuis 10 min s'éteint, **par sa cause** (« régler le point qui a déclenché ») | une attente qui tombe parce que la valeur a bougé |
| **E · Alerte** | l'alarme passée de 2 à 3 depuis 1 min | une valeur revenue à l'identique sans la transition attendue : seule la revérification par l'instant « changé » empêche le départ |

Les points sont ceux d'une maison : quatre fenêtres, la porte, deux détecteurs, un bouton, un
thermomètre, un volet io, trois lumières, l'alarme, un radiateur, le mode de la maison (tenu par le
hub), et l'horloge du hub (la nuit, la plage 21 h–2 h). Les capteurs parlent toutes les 5 min ; le
volet passe par 50 avant 0 ; le radiateur ne confirme jamais ; le premier ordre du couloir revient
100 s plus tard.

### 13.2 Ce qui arrive dans la soirée

| Heure | Événement | Ce que le moteur doit faire |
|---|---|---|
| 19:00 → 19:55 | la lumière du salon allumée, éteinte, rallumée | D l'éteint à 19:10 par sa cause ; abandonne l'attente de 19:30 (la lumière est éteinte à 19:35) ; l'éteint à 19:55 |
| 20:30 | le bouton « bonne nuit », trop tôt | A part par sa seconde entrée ; le « si » échoue (avant 21 h) : « conditions non réunies », avec le bouton pour cause |
| 21:00 → 21:03 | mouvement, puis nouveau mouvement à 21:01:30 | B allume ; son issue tarde ; il est **relancé** ; l'issue de 21:01:40, destinée à l'exécution d'avant, est **écartée** ; extinction à 21:03:31 |
| 21:42 | **un seul message** : la fenêtre du bureau se ferme, celle du salon s'ouvre | la condition de A ne bascule pas entre les deux valeurs : **rien ne s'arme** |
| 21:45 → 21:51 | le bureau fermé, rouvert, refermé | la condition est vraie depuis 21:45, fausse, vraie depuis 21:51 : l'attente est réarmée jusqu'à 22:01 |
| 22:01 → 22:07 | A part ; un mouvement au salon à 22:03 | le « si » est franchi : le mouvement ne fait **pas revenir A en arrière** ; après 5 min, le second « si » relit le salon (vide) ; le volet à 50 **ne suffit pas**, à 0 il est vérifié ; l'alarme confirme ; la maison passe en nuit |
| 22:02 | la température passe de 21,30 à 20,90 °C | le registre de A garde 2130 : la donnée produite en route ne bouge pas |
| 22:10 | le bouton, pendant l'exécution | « déclenchement ignoré, déjà en cours » (mode `single`) |
| 22:17 → 22:19 | la porte s'ouvre ; un mouvement au couloir | C arme son attente (22:22) ; B allume la veilleuse (la maison est en nuit) |
| **22:20 → 22:25** | **panne du hub**, instantané rangé | au redémarrage : A reprend (réveil 22:36:48) ; B est **interrompu** (en retard de 239 s) ; l'attente de C, échue pendant la panne, est **revérifiée** et part : alerte de nuit |
| 22:30 → 22:31 | B repart, puis **on modifie B** | B est arrêté (logique modifiée) ; A continue au même nœud, bien que ses nœuds aient glissé d'une place |
| 22:36 → 22:37 | fin de l'attente de 30 min | la consigne n'est **jamais confirmée** : après 60 s, la branche d'échec, puis la fin selon la cause (la fenêtre du bureau), avec le registre |
| 22:44 → 22:45 | l'alarme 2 → 3 → 1 → 3 | E abandonne son attente : la valeur est revenue à 3, mais depuis 1 |
| 22:50 → 23:05 | la porte se ferme ; le capteur de la cuisine devient **muet** à 22:55:01, puis revient à 23:05 | la condition de A tombe (valeur plus sûre) ; l'attente de 23:00 est abandonnée ; au retour, la condition est vraie depuis 22:50 et part tout de suite. Le « si » refuse (alarme armée) : « conditions non réunies » |
| 23:08 → 23:12 | mouvement au couloir ; l'alarme repasse en alerte | B, modifié, va jusqu'à son nouvel avis ; E part à 23:12 |

### 13.3 Ce que les tests exigent, et la preuve qu'ils savent échouer

| Test | Ce qu'il exige |
|---|---|
| `les_ordres_de_la_soiree` | les 14 ordres, à la seconde |
| `les_avis_et_les_causes` | les 7 avis, chacun avec la cause et le registre de son exécution |
| `le_chemin_de_a_avance_seulement` | le chemin de A, nœud par nœud, de `tout_ferme` à `fin_fenetre`, sans retour en arrière |
| `les_etats_releves` | les fiches d'exécution et la vérité de la condition relevées avant et après la panne, après la modification de B, et l'état de la maison à la fin |
| `le_journal_dit_pourquoi` | les raisons, au journal, à la seconde : attente abandonnée, issue absente, relance, issue tardive écartée, condition vraie depuis, déclenchement ignoré, interruption, reprise, arrêt, capteur muet, capteur revenu |
| `cent_maisons_identiques` | cent copies rejouées ensemble se comportent exactement comme la première |

Les six passent, en natif et dans le conteneur Linux musl. Un test qui passe ne prouve rien s'il ne
sait pas échouer. `scripts/mutations.py` injecte donc sept fautes dans le moteur. **Chacune fait
échouer au moins un test** :

| Faute injectée dans le moteur | Tests en échec |
|---|---|
| une attente ne revérifie plus l'instant « changé » | 2 : les avis, le journal (E part à tort à 22:45) |
| le redémarrage reprend tout, même très en retard | 4 : les ordres, les états, le journal, les cent maisons |
| les bascules sont lues à chaque valeur, pas à la fin du message | 1 : le journal (le message de 21:42 arme une attente) |
| le registre ne garde plus la valeur : le message relit la température en direct | 2 : les avis, les états (2090 au lieu de 2130) |
| une recompilation arrête toutes les exécutions | 5 |
| la cause n'est plus gardée par l'exécution | 2 : les avis, le chemin (la fin « selon la cause » change) |
| une issue est acceptée sans vérifier sa génération ni sa demande | 2 : les ordres, le journal |

La première version du scénario n'avait que D pour éprouver la revérification par l'instant
« changé ». La mutation ne faisait alors échouer aucun test : avec un déclencheur « vers allumée »,
tout retour à « allumée » réarme l'attente, et la revérification ne sert jamais. E, ajouté pour
cela, rend le cas réel : une valeur revenue à l'identique **sans** la transition attendue.

### 13.4 Ce que la soirée a appris

- **Les règles tiennent ensemble.** Les mêmes ordres, les mêmes avis et les mêmes chemins sortent
  avec un redémarrage, une recompilation et un capteur muet au milieu.
- **Allumer, attendre, éteindre gagne à devenir un bail.** B, interrompu par le redémarrage (la règle
  « reprendre si à l'heure »), laisse la veilleuse allumée jusqu'à son exécution suivante. Écrit
  avec un bail (« veilleuse pendant 2 min »), il l'aurait éteinte au redémarrage : un bail rétablit
  toujours la valeur, même en retard (§ 10). L'éditeur peut proposer cette forme.
- **Une condition qui redevient connue** avec le même « vraie depuis » part, si elle n'était pas déjà
  partie pour cette vérité. À 23:05, le capteur de la cuisine revient, la condition est vraie depuis
  22:50, et A part aussitôt ; la cause est la feuille revenue. Le « si » de A l'arrête ensuite,
  puisque l'alarme est armée. C'est cohérent avec « une durée se compte depuis l'instant de la
  vérité », et le test le fige.
- **La vérification d'une nouvelle valeur est un « attendre que » après l'ordre**, avec un délai et
  une branche d'échec. Une valeur intermédiaire (le volet à 50) ne suffit pas. Un appareil qui ne
  confirme jamais (le radiateur) fait passer par la branche d'échec, sans bloquer le flux.

### 13.5 Les mêmes soirées, en nombre

Le scénario rejoue N maisons à la fois, dans un seul moteur, avec la même soirée (Linux musl, 2 CPU,
128 Mio).

| La soirée, de 18 h à 23 h 15 | ×1 | ×100 | ×400 |
|---|---|---|---|
| Automatismes ; nœuds | 5 ; 40 | 500 ; 4 000 | 2 000 ; 16 000 |
| Points ; messages traités | 18 ; 679 | 1 602 ; 67 702 | 6 402 ; 270 802 |
| **Processeur du moteur, toute la soirée** | **0,17 ms** | **11,8 ms** | **54 ms** |
| Par message | 249 ns | 175 ns | 200 ns |
| Programme compilé (dont noms et gabarits) | 5,5 Kio (1,9) | 452 Kio (189) | 1,77 Mio (0,74) |
| Les mêmes définitions sous la forme d'aujourd'hui (une copie ; le moteur actuel en garde deux) | 13 Kio | 1,24 Mio | 4,98 Mio |
| Exécutions simultanées au plus ; leur mémoire | 2 ; 128 o | 200 ; 12,5 Kio | 800 ; 50 Kio |
| Attentes des déclencheurs à durée ; vérité des conditions composées | 208 o ; 17 o | 20 Kio ; 1,7 Kio | 81 Kio ; 6,6 Kio |
| Ordres par copie ; copies identiques à la première | 14 | 14 ; 100 sur 100 | 14 ; 400 sur 400 |

Le programme pèse plus par nœud que dans le § 8.3 (116 o contre 63 o), parce que les noms des nœuds
et les gabarits des messages y sont comptés. A, à lui seul, porte sept textes. Sans ces textes, une
copie de la maison (cinq automatismes, 40 nœuds) pèse 2,7 Kio. Toute une soirée de 400 maisons,
270 802 messages, coûte au moteur 54 ms de processeur.

## Annexe A — Le banc

### A.1 Comment il a été joué

- Le banc est rangé dans le dépôt, hors workspace : `tools/memoire-banc/` (son `README.md` dit comment
  tout rejouer). Il n'a aucune dépendance aux crates de Sarah Home. Les types d'aujourd'hui y sont
  recopiés à l'identique : `Value`, `Sample`, `States` du hub, `Hub::state`, le miroir du moteur,
  tout le modèle de `moli-automation`, la boucle `on_state`, `holds` et la tâche de `hold_until`. Trois
  ajouts proposés au modèle y sont marqués : `WhenRules`, `ReadValue`, `Rule::Cause`.
- Compilé en Linux musl par `rust:1-alpine` (rustc 1.99.1), réglé comme la production :
  `opt-level = "s"`, LTO complet, une unité de code. Un second binaire est réglé pour la vitesse
  (`opt-level = 3`), pour la seule comparaison du passage.
- Chaque mesure tourne dans un **conteneur neuf, limité à 2 CPU et 128 Mio, sans échange**, un
  processus par section et par échelle : la mémoire comptée est celle de la mesure seule.
- Mémoire : octets demandés à l'allocateur (le compteur enveloppe l'allocateur de musl). Temps :
  médiane de 15 à 201 tours, lectures dans un ordre tiré au hasard, résultats passés par
  `black_box`.
- Maison : la forme des deux maisons (496 appareils, 2 264 points), répétée ×3, ×10, ×100. Elle a
  50 % de mesures, 25 % de binaires, 15 % d'énumérations, 5 % de textes et 5 % de points sans
  valeur, comme le banc mémoire de la V7. Les classes d'appareils donnent 1,96 nouvelle par seconde.
- Automatismes : l'automatisme type de la maquette (quand, si de trois règles, régler, attendre 5 min,
  régler, prévenir), un sur quatre avec une durée ; 2 470 octets de JSON chacun.
- Machine : Apple M4, Docker 29.2.1 (Colima, Ubuntu 24.04, noyau 6.8), conteneur `alpine:3`.
  **Le Pi Zero 2 W (Cortex-A53) sera plusieurs fois plus lent** : les rapports entre méthodes
  tiennent, les valeurs absolues sont à refaire sur la carte.

Après tes réponses, deux sections ont été ajoutées et jouées seules, en Linux :

- `decisions`, à ×1, ×3 et ×10 : la fiche d'exécution de 64 octets et le réseau des déclencheurs
  composés (structure, vérité, reconstruction) ;
- `scenario`, à 1, 100 et 400 copies : la soirée du § 13, avec ses six tests joués dans le conteneur
  de compilation.

Les autres sections n'ont pas changé, et leurs mesures restent celles de la première série.

Trois mesures ont d'abord été tuées par la limite de 128 Mio : workflows ×10, conditions ×10 et
valeurs ×100. Ce n'étaient pas les formes proposées, mais les copies que le banc gardait des formes
d'aujourd'hui : le fichier JSON de 4 000 automatismes gardé en arbre, et `States` à ×100. Le banc a
été corrigé pour ne garder qu'une copie à la fois. Les 17 mesures finales sont passées sans
débordement.

### A.2 Les limites

- Les répliques d'aujourd'hui sont **au mieux**. La traduction de la valeur attendue
  (`canonical_expected`), la conversion d'heure locale (`jiff`), le texte des causes et la tâche
  lancée à chaque déclenchement ne sont pas comptés. Le code réel est donc plus lent que le banc.
- Le banc mesure des structures, pas Sarah Home entière : ni pilotes, ni courtier, ni HTTP.
- « Attendre que » d'aujourd'hui est mesuré par 4 096 événements diffusés à 10, 100 ou 400 tâches
  Tokio abonnées (2 fils), médiane de trois essais.

### A.3 Les structures, en Rust

Bloc technique — l'annuaire et l'état vif (extraits du banc, sans `unsafe`) :

```rust
/// Le seul endroit où un nom devient un numéro. Chaque nom une fois, bout à bout ;
/// la table de hachage ne garde que des numéros. Un numéro n'est jamais redonné.
pub struct Annuaire { texte: String, fins: Vec<u32>, table: hashbrown::HashTable<u32>, hacheur: RandomState }

impl Annuaire {
    pub fn numero(&self, nom: &str) -> Option<u32>;    // au bord : 34 à 48 ns
    pub fn attribuer(&mut self, nom: &str) -> u32;     // le suivant, au bout
    pub fn nom(&self, n: u32) -> &str;                 // l'inverse, pour l'écran et les fichiers
}

/// Une colonne par sorte de fiche, toutes indexées par le même numéro.
pub struct Etat {
    pub annuaire: Annuaire,
    pub defs: Vec<Def>,        // 16 o par point
    pub vifs: Vec<Vif>,        // 40 o par point
    pub sante: Vec<Sante>,     // 20 o par appareil
    pub delai: Vec<u32>,       // par appareil : rythme × (pertes + 1), calculé une fois
    pub echeance: Vec<u32>,    // par appareil : contact + délai (u32::MAX : jamais)
    pub fraicheur: Vec<u32>,   // par point : fraicheur_s de la méta (0 : aucune)
    pub perime: Vec<u32>,      // par point : reçu + fraîcheur (u32::MAX : jamais)
    pub textes: Vec<Box<str>>, // le tiroir : la fiche porte le rang
}

impl Etat {
    /// Une nouvelle : en place, sans allocation ; l'échéance se réarme par une addition.
    pub fn rapport(&mut self, n: u32, valeur: i64, maintenant_ms: u64) -> bool {
        let (s, i) = ((maintenant_ms / 1_000) as u32, n as usize);
        let v = &mut self.vifs[i];
        let change = v.valeur != valeur;
        if change { v.precedente = v.valeur; v.valeur = valeur; v.change_ms = maintenant_ms; }
        v.observe_ms = maintenant_ms;
        v.recu_s = s;
        let d = self.defs[i].appareil as usize;
        self.sante[d].contact_s = s;
        self.sante[d].vie = vie::VIVANT;
        if self.delai[d] != 0 { self.echeance[d] = s.saturating_add(self.delai[d]); }
        if self.fraicheur[i] != 0 { self.perime[i] = s.saturating_add(self.fraicheur[i]); }
        change
    }

    /// Ce qu'une condition lit : un appareil muet rend ses valeurs mauvaises sans réécrire une fiche.
    pub fn lire(&self, n: u32) -> (i64, u8) {
        let i = n as usize;
        let muet = self.sante[self.defs[i].appareil as usize].vie == vie::MUET;
        (self.vifs[i].valeur, if muet { qualite::MAUVAISE } else { self.vifs[i].qualite })
    }
}
```

Bloc technique — le passage par blocs et le réveil au plus proche :

```rust
/// Rend la plus petite échéance restante : l'échéancier dort jusque-là.
pub fn passage_blocs(ech: &mut [u32], maintenant: u32, sortie: &mut Vec<u32>) -> u32 {
    let mut prochaine = u32::MAX;
    for (b, bloc) in ech.chunks_mut(16).enumerate() {
        let mini = bloc.iter().copied().fold(u32::MAX, u32::min);   // vectorisé
        if mini < maintenant {
            for (i, e) in bloc.iter_mut().enumerate() {
                if *e < maintenant { sortie.push((b * 16 + i) as u32); *e = u32::MAX; }
                else { prochaine = prochaine.min(*e); }
            }
        } else {
            prochaine = prochaine.min(mini);
        }
    }
    prochaine
}
// À chaque nouvelle : ech[d] = s + delai[d] ; prochaine = prochaine.min(ech[d]).
// L'échéancier : si maintenant > prochaine { prochaine = passage_blocs(...) }.
```

Bloc technique — le programme, l'index par point et les attentes revérifiées :

```rust
#[repr(C)]
pub struct Noeud {          // 32 o, deux par ligne de cache
    pub op: u8, pub drapeaux: u8, pub nb_liens: u16,
    pub a: u32,             // point (quand, régler), début des règles (si), texte (prévenir)
    pub b: u32,             // durée en s, nombre de règles, canaux
    pub c: u32,             // case d'attente d'un déclencheur à durée
    pub liens: u32,         // début de ses liens
    pub _r: u32,
    pub v: i64,             // valeur au canon : « vers », valeur à régler, seuil
}
pub struct Lien { pub port: u8, pub cible: u32 }                                      // 8 o
pub struct Regle { pub genre: u8, pub op: u8, pub jours: u8, pub point: u32, pub valeur: i64 } // 16 o
pub struct Plan { pub premier: u32, pub nb: u16, pub mode: u8, pub definition: u32 } // 12 o

pub struct Programme {
    pub plans: Vec<Plan>, pub noeuds: Vec<Noeud>, pub liens: Vec<Lien>, pub regles: Vec<Regle>,
    pub textes: String, pub fins: Vec<u32>,
    pub index_debut: Vec<u32>,  // par numéro de point (un de plus à la fin)
    pub index: Vec<u32>,        // les nœuds déclencheurs, rangés par point
}

/// Un changement du point n : seuls ses déclencheurs se réveillent.
pub fn sur_changement(p: &Programme, etat: &Etat, n: u32, avant: i64, valeur: i64,
                      maintenant: u32, attentes: &mut Attentes, sortie: &mut Vec<u32>) {
    let (d, f) = (p.index_debut[n as usize] as usize, p.index_debut[n as usize + 1] as usize);
    for &t in &p.index[d..f] {
        let nd = &p.noeuds[t as usize];
        if nd.op == op::QUAND_ETAT && (nd.drapeaux & drapeau::A_VERS == 0 || nd.v == valeur) && avant != valeur {
            if nd.b == 0 { sortie.push(t) } else {
                let c = nd.c as usize;                          // la case est fixée par le compilateur
                attentes.echeance[c] = maintenant + nd.b;
                attentes.depuis[c] = etat.vifs[n as usize].change_ms;
                attentes.noeud[c] = t;
            }
        }
    }
}
// À l'échéance d'une attente : elle part si et seulement si vifs[point].change_ms == depuis.
// Aucun désarmement : l'instant « changé » sert de numéro de version à la valeur.

#[repr(C)]
pub struct Execution64 {    // 64 o, une ligne de cache ; la case est réutilisée, la génération écarte les issues tardives
    // où elle en est : 24 o
    pub automatisme: u32, pub curseur: u32, pub reveil_s: u32, pub depuis_s: u32, pub demande: u32,
    pub generation: u16, pub etat: u8, pub version: u8,
    // l'état qui l'a lancée, pour toute la suite : 32 o
    pub cause_point: u32, pub cause_declencheur: u32,
    pub cause_instant_ms: u64, pub cause_valeur: i64, pub cause_precedente: i64,
    // une donnée produite en route : 8 o
    pub registre: i64,
}
```

Bloc technique — le réseau des déclencheurs composés : la vérité, reconstruite d'un coup :

```rust
pub struct Reseau {
    // figé, dans le programme
    pub point: Vec<u32>, pub op: Vec<u8>, pub valeur: Vec<i64>, pub parent: Vec<u32>,
    pub nb: Vec<u16>, pub debut: Vec<u32>, pub index: Vec<u32>,
    // la vérité, à part
    pub vrai: Vec<bool>, pub vraies: Vec<u16>, pub parent_vrai: Vec<bool>,
    pub depuis: Vec<u64>,   // « et » : le plus grand des « changé » de ses feuilles
}

impl Reseau {
    /// Un changement du point n : ses feuilles, puis les conditions dont le compteur bouge.
    pub fn changement(&mut self, n: u32, valeur: i64, bascules: &mut Vec<u32>) {
        let (d, f) = (self.debut[n as usize] as usize, self.debut[n as usize + 1] as usize);
        for &feuille in &self.index[d..f] {
            let i = feuille as usize;
            let t = comparer(valeur, self.op[i], self.valeur[i]);
            if t != self.vrai[i] {
                self.vrai[i] = t;
                let p = self.parent[i] as usize;
                if t { self.vraies[p] += 1 } else { self.vraies[p] -= 1 }
                let pv = self.vraies[p] == self.nb[p];
                if pv != self.parent_vrai[p] { self.parent_vrai[p] = pv; bascules.push(p as u32); }
            }
        }
    }

    /// Au démarrage, après une recompilation ou une perte de messages : tout, d'un coup.
    pub fn reconstruire(&mut self, etat: &Etat) {
        self.vraies.fill(0);
        self.depuis.fill(0);
        for i in 0..self.point.len() {
            let (valeur, change) = (etat.lire(self.point[i]).0, etat.vifs[self.point[i] as usize].change_ms);
            let t = comparer(valeur, self.op[i], self.valeur[i]);
            self.vrai[i] = t;
            let p = self.parent[i] as usize;
            if t { self.vraies[p] += 1; self.depuis[p] = self.depuis[p].max(change); }
        }
        for p in 0..self.nb.len() { self.parent_vrai[p] = self.vraies[p] == self.nb[p]; }
    }
}
```

### A.4 Les résultats bruts

Sortie brute — les lignes RESULTAT du banc, par section, à ×1 (Linux musl, 2 CPU, 128 Mio) :

```text
valeurs     points 2264 · appareils 496 · nom moyen 39,0 car
            mémoire : states 414 408 o · miroir moteur 302 312 o · état proposé 281 396 o
                      (annuaire 117 824 o · colonnes 158 784 o)
            lire : hub_state 91,6 ns · miroir 33,4 · annuaire 34,1 · vif40 0,315 · vif32 0,336
                   chaud16 0,356 · qualité effective 0,732 · derrière verrou 3,50 · écrire 2,01
            copier : vifs+santé 11,8 µs · colonne chaude 5,7 µs · instantané binaire 29,2 µs
echeances   7 042 nouvelles/h · 9 muets · accord des quatre méthodes : oui
            heure : A 2,549 ms · B 1,040 · R 0,030 (26 passages) · C 0,451 (tas 1 428 entrées)
            nouvelle seule : A 0,308 ns · B 0,586 · C 4,574
            passage : blocs 167 ns (appareils) · colonne 292 · calculé 708 · blocs 708 (points)
workflows   400 automatismes · 2 400 nœuds · 2 483 o de JSON chacun
            mémoire : définitions 2 003 410 o · copie des actifs 1 613 010 o · programme 150 550 o
                      exécutions 16 384 o · attentes 1 600 o (100)
            compiler 0,229 ms · relire le JSON 4,091 ms
            événement : aujourd'hui 825,6 / 1 467,3 ns · index 1,099 / 5,473 ns
            exécuter jusqu'à l'attente 11,6 ns · finir et relancer 5,73 ns
conditions  règle d'état aujourd'hui 70,1 ns · si de trois règles 4,58 ns
            réseau : 53 292 o · 1,709 / 4,130 ns · bascule nuit 771 ns · cycle 1,750 µs
            durée : tâche Tokio 628,6 o · attente 16 o
            attendre que aujourd'hui : 0,721 / 6,553 / 28,839 µs (10 / 100 / 400 attentes)
passage     blocs 0,708 / 7,208 / 71,958 / 660,663 µs (2 264 → 2,26 millions)
            vitesse 0,166 / 1,291 / 13,083 / 130,333 µs
decisions   exécution 64 o : 32 768 o pour 400 · finir et relancer 4,27 ns
            réseau : 56 492 o (structure 49 708 o, vérité 6 784 o) · 1 200 feuilles, 400 conditions
            reconstruire 3,625 µs (×3 : 11,625 µs ; ×10 : 46,291 µs)
scenario    ×1 : 5 automatismes, 40 nœuds, 679 messages, 14 ordres · moteur 0,169 ms · programme 5 630 o
            ×100 : 67 702 messages · moteur 11,824 ms · programme 462 382 o · définitions 1 300 976 o
            ×400 : 270 802 messages · moteur 54,152 ms (199,969 ns par message) · programme 1 851 070 o
                   (textes 775 450 o) · définitions 5 217 104 o · 800 exécutions au plus · copies identiques
tests       6 passed, 0 failed (natif et rust:1-alpine) · mutations : 7 détectées sur 7
```
