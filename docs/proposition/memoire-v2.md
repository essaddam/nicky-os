# Étude mémoire v2 — Sarah Home : des numéros qui sont des adresses, et le temps

Date : 23 octobre 2026. Statut : **proposition, aucune modification du code**. Base analysée :
`main` au commit `bd1714d`. **Remplace la [v1](memoire.md)**, gardée comme historique. Complète
l'[analyse V7](isolation-reactivite-runtime-v7.md) (§ 10.5 la santé des appareils, § 11 la mémoire,
§ 12.1 le journal de reprise, § 13 le moteur de workflow).

L'idée de départ est la tienne : **chaque valeur a un numéro fixe ; les fiches ont toutes la même
taille et sont rangées dans l'ordre des numéros ; l'adresse d'une valeur se déduit donc de son
numéro, et on n'a jamais à la transmettre.** La v1 a confronté cette idée au code d'aujourd'hui, l'a
vérifiée par un banc et a répondu à tes trois questions :

1. retrouve-t-on une valeur sans la chercher ?
2. trouve-t-on facilement, par le calcul, les valeurs qui ont expiré ?
3. comment ranger les workflows en mémoire, à part de celle des valeurs, et comment calculer leurs
   conditions ? Avec quel rapport entre l'état d'un flux à la n8n et la mémoire des valeurs ?

Tu as ensuite posé quatre questions, auxquelles cette v2 répond :

4. reste-t-il des choses à corriger et à affiner ?
5. comment le moteur qui tient l'équivalent d'un cron s'intègre-t-il à cette mémoire ?
6. à quelle précision peut-on tenir une tâche planifiée (la minute, la seconde, la milliseconde), et
   que coûte chacune ?
7. les agendas : « 10 minutes avant un événement » ; faut-il garder en mémoire l'agenda du jour, ou
   ce qui arrive bientôt, et à part des valeurs ou dedans ?

Ton rappel sert de fil conducteur : **le but est d'optimiser la mémoire et l'usage des workflows.**
Ce document est complet : il reprend toute la v1, corrigée, chiffrée par une nouvelle série de
mesures, et ajoute le temps (§ 10). Ce qui a changé depuis la v1 est résumé d'abord (§ 1).

Tes décisions du 23 octobre sont intégrées (§ 12). Les quatre de la v1 :

- Une condition lit la maison au moment où elle passe. Le flux garde l'état qui l'a déclenché pour
  toute la suite, et une étape franchie ne se rejoue pas.
- Les exécutions en attente reprennent après un redémarrage si elles sont à l'heure.
- Les déclencheurs composés entrent dès la V7.
- Le numéro voyage sur MQTT, en plus du nom.

Les quatre de la v2 :

- L'agenda est gardé sur 7 jours glissants, avec les événements en cours, à part des valeurs.
- Les déclencheurs horaires et les tâches planifiées sont tenus à la seconde.
- « Le lundi, de 22 h à 6 h » désigne la nuit du lundi au mardi, comme dans les plannings.
- Un rappel d'agenda en retard part tant que son rendez-vous n'a pas commencé.

Méthode, et techniques employées, nommées au fil du texte :

- **premiers principes** : ce que fait réellement le processeur quand on lit une fiche, et quand il
  dort jusqu'à une échéance ;
- **contre-épreuve des hypothèses** : chercher où l'idée casse (noms, textes, suppressions,
  redémarrage, expiration, lecteurs multiples, changements d'heure, horloge qui saute) ;
- **contre-épreuve de la v1** : relire la v1 contre son propre moteur de référence, et corriger ce
  qu'elle affirmait sans le tenir (§ 1.1) ;
- **analyse morphologique** : pour chaque question, toutes les options côte à côte ;
- **analogies** : l'automate industriel (cycle), les systèmes experts (réseau incrémental), les
  moteurs de jeu (entités numérotées rangées en colonnes), n8n (le flux porte ses données), `cron`
  et les roues de minuteries (le temps) ;
- **sélection à critères visibles** : simplicité, vitesse, mémoire, reprise, explicabilité ;
- **un banc** : Linux musl, conteneur limité à 2 CPU et 128 Mio comme la production, chaque mesure
  dans un processus neuf ; les structures d'aujourd'hui recopiées à l'identique depuis le code
  ([annexe A](#annexe-a--le-banc)) ;
- **une vérification de bout en bout** : un moteur de référence complet, une soirée de la maison
  rejouée seconde par seconde, 29 tests et 17 fautes injectées pour prouver qu'ils savent échouer
  (§ 14). Le banc, le moteur et les tests sont rangés dans le dépôt : `tools/memoire-banc/`.

Vocabulaire :

- un **numéro** est un entier de 4 octets (`u32`) donné à un point, un appareil, un nœud de workflow
  ou une exécution ;
- une **fiche** est un enregistrement de taille fixe (40 octets pour la valeur vive d'un point) ;
- une **colonne** est un tableau de fiches d'une même sorte, rangées par numéro ;
- l'**annuaire** traduit un nom (`z2m:0x00158d00a1b2c3d4/temperature`) en numéro, et inversement ;
- une **échéance** est l'instant où quelque chose expire : un appareil muet, une valeur trop
  vieille, une attente qui se termine, une heure qui sonne ;
- un **passage** est la lecture complète d'une colonne d'échéances ;
- le **programme** est la forme compilée des workflows ; une **exécution** est un workflow en train
  de tourner ;
- l'**horloge monotone** compte les secondes depuis le démarrage et ne saute jamais ; l'**horloge
  murale** donne la date et l'heure, et peut sauter (réglage, synchronisation NTP) ;
- une **fiche calendaire** est une règle de temps de 24 octets (« à 22:30 », « au coucher −
  30 min », « toutes les 15 min », « 10 min avant chaque rendez-vous ») ; une **occurrence** est
  l'un de ses instants ; un **niveau** est une règle qui vaut vrai ou faux sur une durée (« il fait
  nuit », « entre 22 h et 6 h », « en rendez-vous ») ;
- **rattraper** une occurrence, c'est la faire partir en retard ; la **manquer**, c'est la noter
  sans la faire partir.

## 1. Ce que la v2 corrige et affine

### 1.1 Ce qui était faux ou incomplet dans la v1

La v1 affirmait des règles que son moteur de référence ne tenait pas toujours. Relue contre son
propre code, voici ce qui ne tenait pas, et ce que la v2 en fait :

| # | La v1 disait | Ce qui n'allait pas | La v2 |
|---|---|---|---|
| 1 | « Les échéances sont en secondes d'une horloge monotone » (§ 6.5) | le moteur de référence comptait tout en heure murale : un réglage de l'heure aurait allongé ou raccourci une attente, sans qu'aucun test le voie | deux horloges partout (`Temps { mono, mur_ms }`) ; la faute injectée « une attente compte en heure murale » fait échouer 4 tests (§ 6.5, § 10.3) |
| 2 | « Une attente coûte 16 octets » (§ 8.5) | deux champs manquaient (« déjà partie », le point de la cause), et le moteur gardait une copie de la cause : 52 octets en réalité | **24 octets** ; la cause n'est plus copiée, elle se relit dans la fiche du point, que la revérification par l'instant « changé » garantit intacte. La soirée le mesure : 208 octets pour ses 4 attentes en v1, 96 en v2 (§ 8.5) |
| 3 | « Une valeur trop vieille : la règle la lit inconnue, calculé à la lecture » (§ 6.4) | juste pour un « si », faux pour un déclencheur composé, dont la vérité est gardée : il ne l'aurait jamais su. Le moteur de référence ne tenait pas la fraîcheur | l'échéance de fraîcheur est un événement : la valeur devient mauvaise et ses feuilles sont relues, comme pour un appareil muet. La soirée le vérifie à 22:52:31 (§ 6.4) |
| 4 | « Au démarrage, les échéances se recalculent depuis les instants persistés » (§ 6.5) | l'horloge monotone repart de zéro à chaque démarrage : une échéance monotone rangée telle quelle ne veut plus rien dire | l'instantané range les échéances en heure murale ; le redémarrage les reconvertit, et la règle des 2 min décide (§ 6.5, § 11) |
| 5 | « Les plages horaires et le soleil deviennent des valeurs ; l'échéancier les bascule à l'heure dite » (§ 9.4) | sans dire comment : changements d'heure, panne, réglage de l'heure, heure pas encore sûre au démarrage | le moteur du temps : des fiches de 24 octets, le réveil au plus proche, des règles écrites et testées (§ 10) |
| 6 | Trois défauts du moteur d'aujourd'hui (§ 8.1) | quatre manquaient, tous dans les déclencheurs horaires | l'heure relue chaque seconde ; les changements d'heure (« toutes les 15 min » saute quatre occurrences en octobre, 02:30 disparaît en mars) ; aucun rattrapage après une panne ; une plage de nuit rattachée au mauvais jour (§ 10.1) |
| 7 | La série de mesures de la v1, un essai par mesure | le code du banc a changé (deux horloges, attente de 24 octets, fraîcheur) ; un seul essai laisse passer la charge du moment | toute la série est rejouée avec le code de la v2, chaque mesure trois fois (la médiane) : les mémoires ne bougent pas, certains temps bougent du simple au double, aucune conclusion ne change (annexe A) |

### 1.2 Ce que la v2 ajoute

- **Le moteur du temps** (§ 10), l'équivalent d'un `cron` rangé comme le reste de la mémoire : des
  fiches calendaires de 24 octets, une colonne d'échéances, le réveil au plus proche. Il tient les
  heures, le soleil, les périodes, les heures portées par une valeur, les plages et les plannings,
  les changements d'heure, le rattrapage après une panne et les sauts d'horloge.
- **Les agendas** (§ 10.9) : « 10 minutes avant chaque rendez-vous », une table d'événements à part
  des valeurs, son horizon, et ce qu'elle donne aux valeurs.
- **Les minuteries** (§ 10.8), tenues par le hub en fiches de 24 octets.
- **La précision** (§ 10.10) : ce que coûtent la minute, la seconde, le dixième et la milliseconde,
  mesuré.
- **La soirée étendue** (§ 14) : trois automatismes horaires de plus (une ronde à la seconde, des
  tâches qui tombent pendant la panne, des rappels d'agenda) et une valeur qui devient trop vieille.
  29 tests au lieu de 6 ; 17 fautes injectées au lieu de 7, toutes détectées.

### 1.3 Le but : moins de mémoire, des workflows plus simples à tenir

Ce que la v2 gagne, objet par objet :

| Objet | Aujourd'hui | v1 | **v2** |
|---|---|---|---|
| Une durée armée (« ouvert depuis 10 min ») | une tâche Tokio, 629 o, perdue au redémarrage | 16 o annoncés, 52 o dans le moteur | **24 o**, rangés, revérifiés |
| Un déclencheur horaire | l'heure relue en texte et le soleil recalculé à chaque seconde : 979 ms de processeur par jour pour 400 déclencheurs | non traité | **49 o** (fiche 24 o, état 25 o) ; 0,51 ms de processeur par jour pour 400 |
| Un rappel « 10 min avant » | n'existe pas | — | **une fiche de 49 o par rappel**, quel que soit le nombre de rendez-vous ; 24 o par rendez-vous, sans titre |
| « Il fait nuit », « entre 22 h et 6 h » | l'heure locale convertie à chaque règle qui les lit | un point du hub | un point du hub, tenu par une fiche du moteur du temps et lu par tous les automatismes |
| La cause d'un départ horaire ou d'agenda | une phrase | — | dans la fiche d'exécution de 64 o, sans un octet de plus |
| 400 automatismes en mémoire | 3,45 Mio | 0,23 Mio | **0,23 Mio**, déclencheurs composés compris au pire |

## 2. Le verdict en bref

- **Ton idée est juste, et c'est déjà la forme du prototype de la V7** (`Vec<Vif>` indexé par un
  `u32`, § 11.4.4). Lire une valeur par son numéro coûte **0,35 ns**, contre **95 ns** par
  `Hub::state` aujourd'hui et **35 ns** par le miroir du moteur : environ 270 fois moins.
- **L'adresse se déduit, le numéro suffit.** Il pèse 4 octets au lieu des 8 d'une adresse, et il
  survit là où une adresse meurt : quand le tableau grandit, au redémarrage, sur disque, au journal
  de reprise, sur la machine de secours. Ton « index où tous les ID sont à la suite » existe : c'est
  l'annuaire, les noms bout à bout dans l'ordre des numéros (§ 3, § 4).
- **« Où » se calcule ; « qui » se lit.** Le numéro donne la place d'une fiche connue, pas la liste
  des fiches périmées. Mais une colonne d'échéances de 4 octets, contiguë et de taille fixe, se lit
  en **0,71 µs pour toute la maison**. Avec le **réveil au plus proche**, on ne la lit que **26 fois
  par heure** au lieu de 3 600, pour **0,04 ms de processeur par heure** (§ 6).
- **Le nom reste l'identité.** L'annuaire traduit une fois, au bord, en 35 ns ; dedans, plus aucune
  chaîne n'est comparée (§ 4.3). Sur MQTT, le numéro voyage en plus du nom : `n` dans `meta/`, `i`
  dans `val/` (§ 4.4).
- **Un numéro de point n'est jamais redonné** ; une case d'exécution, si, avec une génération qui
  écarte les réponses tardives (§ 4, § 8.4).
- **Les workflows ont leur mémoire à part, en deux parties.**
  - Le programme dit ce que les workflows sont. Il est figé, et recompilé à chaque validation en
    0,45 ms.
  - Les exécutions disent ce qu'ils font, et chacune garde sa cause.

  Pour 400 automatismes, **0,23 Mio au lieu de 3,45 Mio**, déclencheurs composés compris au pire. Un
  changement de valeur réveille les déclencheurs de son point en **1,4 à 5,9 ns**. Le balayage de
  tous les automatismes coûte **0,9 à 1,6 µs** (§ 8).
- **Les conditions se calculent à la demande, par numéro** : un « si » de trois règles en 4,8 ns.
  Une durée (« ouvert depuis 10 min ») est un calcul sur l'instant « changé », pas une minuterie.
  Les déclencheurs composés (« toutes les fenêtres fermées ») passent par un réseau incrémental. Son
  état se reconstruit d'un coup en 3,6 µs (§ 9.5).
- **Un flux n'emporte pas la maison avec lui, mais il garde sa cause.** Après une attente, une
  condition lit la mémoire des valeurs, pas une copie emportée comme le ferait n8n. L'état qui a
  déclenché le flux reste dans sa fiche pour toute la suite, et une étape franchie ne se rejoue pas
  (§ 8.4).
- **Le temps est une mémoire comme les autres.** Un déclencheur horaire devient une fiche de
  24 octets ; son état, 25 de plus. Le réveil au plus proche remplace le balayage à la seconde :
  **0,51 ms de processeur par jour** pour 400 déclencheurs, au lieu de 979 ms aujourd'hui. Les
  changements d'heure, une panne, un saut d'horloge et une heure pas encore sûre ont chacun leur
  règle et leur test (§ 10).
- **Les agendas sont à part des valeurs.** Un événement est un intervalle à venir, pas l'état d'un
  point : il vit dans sa propre table (24 octets, sans titre), sur 7 jours glissants. Le hub en tire
  quelques valeurs (« en rendez-vous », « prochain rendez-vous »). « 10 min avant chaque
  rendez-vous » est une seule fiche, quel que soit le nombre de rendez-vous (§ 10.9).
- **La seconde partout, sans rien payer ; la milliseconde dans les calculs, jamais garantie au
  réveil.** Un réveil arrive d'ordinaire avec quelques millisecondes de retard, au pire mesuré avec
  27 ms. Viser la milliseconde en balayant coûterait mille fois la seconde ; par le réveil au plus
  proche, rien de plus (§ 10.10).
- **Vérifié de bout en bout.** Une soirée de la maison est rejouée contre un moteur de référence :
  huit automatismes, dont un de 23 nœuds à deux entrées, des vérifications de nouvelles valeurs, une
  panne, une modification en cours, un capteur muet, une valeur trop vieille, une ronde à la
  seconde, des tâches qui tombent pendant la panne et des rappels d'agenda. Les 29 tests passent, et
  les 17 fautes injectées sont toutes détectées. 400 maisons rejouées ensemble se comportent comme
  la première, pour 60 ms de processeur sur toute la soirée (§ 14).
- **Sept défauts d'aujourd'hui, trouvés en chemin** (§ 8.1, § 10.1) :
  - chaque « attendre que » se réveille à chaque événement de la maison : 31 µs par événement avec
    400 attentes ;
  - chaque durée armée est une tâche Tokio de 629 octets, perdue au redémarrage ;
  - chaque changement de valeur parcourt tous les déclencheurs de tous les automatismes ;
  - chaque seconde, tous les déclencheurs horaires relisent leur heure en texte et recalculent le
    soleil ;
  - aux changements d'heure, des occurrences sont sautées ou perdues ;
  - une occurrence tombée pendant une panne est perdue, sans trace ;
  - « le lundi, de 22 h à 6 h » ne veut pas dire la même chose dans une règle et dans un planning.

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
`vifs[n]` sur un `Vec<Vif>` : aucune ligne `unsafe`, aucune adresse manipulée à la main. Le tableau
garantit que ses fiches sont contiguës et que leur pas est `size_of::<Vif>()`, alignement compris.

<!-- FIGURE:principe -->

### 3.2 Pourquoi un numéro plutôt qu'une adresse

| | Numéro (`u32`) | Adresse (pointeur) | Nom (`z2m:…/temperature`) |
|---|---|---|---|
| Taille | **4 o** | 8 o | ≈ 39 o, plus le hachage |
| Le tableau grandit (il est déplacé en mémoire) | **reste valable** | pend : l'ancienne adresse ne vaut plus rien | reste valable |
| Redémarrage | **reste valable**, si l'annuaire est relu | change à chaque lancement | reste valable |
| Écrit sur disque, au journal, envoyé à la machine de secours | **oui** | non | oui, mais 10 fois plus gros |
| En Rust | un index : vérifié, sans verrou ni `unsafe` | une référence ne peut pas être gardée pendant qu'on écrit dans le tableau | une recherche à chaque usage |
| Lire la valeur | **0,35 ns** | la même lecture, sans la vérification | 35 à 95 ns |

Ce n'est pas une astuce isolée. Les moteurs de jeu rangent leurs entités ainsi (des colonnes
indexées par un numéro d'entité), et les compilateurs, dont celui de Rust, rangent leurs nœuds dans
des tableaux indexés. La base historique de Sarah Home fait déjà la même chose sur disque : la clé
d'un relevé est calculée, `samples.id = point_id · 2^44 + ts` (D13). La place sur disque se déduit
du numéro du point, sans index secondaire.

### 3.3 Ce que le banc mesure

<!-- FIGURE:lecture -->

| Lire une valeur (ns, Linux musl) | ×1 | ×3 | ×10 | ×100 |
|---|---|---|---|---|
| `Hub::state` aujourd'hui : alloue un `DeviceId`, verrou, deux recherches, copie | 95 | 93 | 95 | — |
| Miroir du moteur aujourd'hui : `HashMap<String, Value>` | 35 | 41 | 44 | — |
| Annuaire : nom → numéro, une fois au bord | 35 | 40 | 51 | 78 |
| Par numéro, fiche de 40 o | **0,35** | 0,56 | 0,54 | 0,66 |
| Par numéro, fiche de 32 o | 0,33 | 0,45 | 0,49 | 0,53 |
| Par numéro, colonne de 16 o | 0,36 | 0,36 | 0,49 | 0,58 |
| Par numéro, avec la qualité de l'appareil (trois lectures) | 0,84 | 1,2 | 1,6 | 2,1 |
| Par numéro, derrière un verrou (lecteur d'un autre fil, sans rival) | 3,5 | 3,5 | 3,6 | 3,5 |
| Écrire une nouvelle : valeur, précédente, instants, santé, échéances, en place | 2,5 | 3,4 | 4,4 | 6,0 |

Lectures dans un ordre tiré au hasard, comme une condition lit un point quelconque. À ×100 (226 400
points), les structures d'aujourd'hui ne tiennent plus dans 128 Mio avec le reste : le banc ne les
construit plus.

### 3.4 La taille de la fiche et le cache du processeur

Le processeur ne lit pas un octet : il lit une **ligne de cache** de 64 octets. Une fiche de
40 octets est à cheval sur deux lignes une fois sur deux ; la lire peut alors coûter deux accès à la
mémoire au lieu d'un. À 32 ou 16 octets, jamais.

<!-- FIGURE:lignes -->

Sur le banc, aucun écart net : à ×1, la colonne de 40 o (88 Kio) tient dans le cache, et même à ×100
le cache de l'Apple M4 est assez grand. Sur un Pi Zero 2 W (cache L2 de 512 Kio, la cible D10), ×10
(884 Kio) n'y tient plus. La variante à 32 octets de la V7 existe (§ 11.2) ; **la mesure est à faire
sur la carte** avant de choisir.

## 4. Les numéros : qui les donne, comment ils restent fixes

### 4.1 La règle

<!-- FIGURE:cycle -->

1. **Le hub donne le numéro**, à la première déclaration d'un point : le suivant, au bout (n =
   nombre de points déjà connus). Les points d'un appareil arrivent ensemble à sa déclaration :
   leurs numéros se suivent.
2. **Il l'écrit dans l'annuaire persistant avant de s'en servir** : la table des numéros de
   `maison.db`, ou en attendant un fichier d'état écrit par `StateFile` (fichier, puis dossier, avec
   `fsync`). Un arrêt brutal ne peut donc pas donner le même numéro à deux points.
3. **Au démarrage, l'annuaire relu remet chaque nom à son numéro** : les colonnes se reconstruisent
   dans le même ordre ; même numéro, même place, d'un lancement à l'autre.
4. **Un point retiré garde sa case, condamnée ; son numéro n'est jamais redonné.** Un trou coûte sa
   fiche, environ 120 octets : mille points retirés, 0,1 Mio. Avec 2 milliards de numéros (le bit de
   poids fort est réservé, § 8.4), aucune chance d'épuisement.
5. **Les appareils ont leur numéro**, par la même règle ; la définition d'un point porte celui de
   son appareil (`Def.appareil`), qui ouvre la colonne de santé.

Les colonnes grandissent si la maison grandit : un tableau Rust se déplace alors en mémoire, une
fois de temps en temps (copier 100 Kio prend environ 12 µs). Le numéro reste valable, puisque
personne ne garde d'adresse. Au démarrage, on réserve la taille relue plus une marge, et le tableau
ne bouge plus en marche normale.

C'est aussi pourquoi **un rendez-vous ne peut pas être un point** : il y en a des milliers par an,
et un numéro de point n'est jamais redonné. Les événements d'un agenda ont leur propre table, à part
des valeurs (§ 10.9).

### 4.2 Pourquoi ils doivent être fixes

Ce n'est pas seulement pratique. Le **journal de reprise** de la V7 (§ 12.1) écrit une ligne par
changement : `seq u64, point u32, charge, instant u64`. L'**instantané binaire** range l'état vif
par numéro. La **machine de secours** rejoue le même journal. Si le numéro 1234 désignait la
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
| Abonné | traduit le nom (35 à 51 ns), tient sa propre table | indexe un tableau directement |
| Mémoire par abonné | ≈ 52 o par nom suivi | aucune table |
| Coût | aucun | + 9 octets par message `val/` (115 → 124 o) |
| Risque | aucun | un client extérieur prend le numéro pour une identité |

Le coût en processeur ne départage pas : 51 ns × 239 valeurs par seconde à la pointe, c'est 12 µs
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
| `perime` | point | 4 | reçu + fraîcheur exigée, en secondes monotones ; « jamais » si aucune (**nouveau**) |
| `fraicheur` | point | 4 | `fraicheur_s` de la méta (**nouveau**) |
| `Sante` | appareil | 20 | contact, changé, rythme, rythme appris, pertes tolérées, vie |
| `echeance` | appareil | 4 | contact + rythme × (pertes + 1), en secondes monotones ; « jamais » pour un appareil qui n'émet qu'à l'action (**nouveau**) |
| `delai` | appareil | 4 | rythme × (pertes + 1), calculé une fois (**nouveau**) |
| annuaire | point | ≈ 52 | le nom, une fois, et sa case dans la table de hachage |

Les quatre colonnes nouvelles sont ce que l'étude ajoute à la V7 : 8 octets par point et par
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
- **Une date ou une heure** (« prochain rendez-vous », « réveil du téléphone ») : un nombre, en
  millisecondes murales ou en secondes du jour. Le moteur du temps sait s'armer dessus (§ 10.6).
- **Rien** (`null`, inconnu) : un genre dans la fiche, la valeur à zéro.

Toutes les fiches gardent donc leur taille ; seul le tiroir varie. Écrire une valeur numérique
n'alloue rien : 40 octets réécrits en place, en 2,5 ns.

### 5.3 Ranger ensemble ce qui se lit ensemble

Deux façons de ranger les mêmes champs :

- **fiches complètes** : un `Vif` de 40 o par point, tous les champs ensemble ;
- **colonnes** : ce qu'une condition lit (valeur, changé, qualité : 16 o) dans une colonne, le reste
  ailleurs.

Pour lire une valeur au hasard, la mesure ne les départage pas (0,33 à 0,36 ns à ×1). Pour parcourir
une seule information sur tous les points, la colonne gagne : c'est le cas des échéances. D'où la
recommandation : **les fiches de la V7 (`Vif` 40 o, `Sante` 20 o), plus des colonnes de 4 octets
pour les échéances**.

## 6. Trouver ce qui a expiré

### 6.1 Quatre sortes d'expiration

| Ce qui expire | Échéance | Par | Tenu par | Horloge |
|---|---|---|---|---|
| Un appareil devient muet | dernier contact + rythme × (pertes + 1) | appareil | le hub (§ 10.5 de la V7) | monotone |
| Une valeur trop vieille pour une règle | reçu + `fraicheur_s` | point | le hub | monotone |
| Une attente du moteur | « depuis 10 min », « attendre 5 min », délai d'« attendre que », bail « pendant », suivi d'un ordre (`suivi_s`), pause d'une source (`pause_jusqu_a`), minuterie | déclencheur, exécution, ordre, module | le moteur et le hub | monotone |
| Une heure qui sonne | « à 22:30 », « au coucher − 30 min », « toutes les 15 min », « 10 min avant chaque rendez-vous », début et fin d'une plage | fiche calendaire | le moteur du temps (§ 10) | murale |

Les trois premières sont des **durées** : elles comptent du temps écoulé, et aucun réglage de
l'heure ne doit les toucher. La quatrième est un **rendez-vous avec le calendrier** : elle suit
l'heure qu'affiche la maison, changements d'heure compris. D'où deux horloges (§ 6.5).

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
déplace l'entrée de l'appareil. Elle sert pour des millions de minuteries, ce que n'a pas une
maison.

<!-- FIGURE:heure -->

| Une heure de la maison (Linux musl) | ×1 | ×3 | ×10 | ×100 |
|---|---|---|---|---|
| Appareils ; nouvelles dans l'heure | 496 ; 7 042 | 1 488 ; 20 014 | 4 960 ; 66 879 | 49 600 ; 686 364 |
| Appareils déclarés muets (identiques pour les quatre méthodes) | 9 | 21 | 74 | 701 |
| A : processeur pour l'heure | 2,0 ms | 9,4 ms | 28 ms | 320 ms |
| B : processeur pour l'heure | 1,2 ms | 5,5 ms | 11 ms | 90 ms |
| **R : processeur pour l'heure** | **0,04 ms** | **0,06 ms** | **0,23 ms** | **6,8 ms** |
| C : processeur pour l'heure | 0,45 ms | 1,1 ms | 4,5 ms | 53 ms |
| R : passages dans l'heure (au lieu de 3 600) | **26** | 38 | 84 | 395 |
| Coût d'une nouvelle : A / B / C | 0,31 / 0,59 / 4,8 ns | 0,33 / 0,62 / 5,8 ns | 0,35 / 0,62 / 4,7 ns | 0,96 / 1,09 / 6,0 ns |
| Mémoire : colonnes (B, R) / tas (C) | 3,9 / 18,7 Kio | 11,6 / 54 Kio | 39 / 181 Kio | 388 / 1 855 Kio |

<!-- FIGURE:passage -->

| Un passage, rien d'expiré | 2 264 | 22 640 | 226 400 | 2,26 millions |
|---|---|---|---|---|
| Colonne, par blocs de 16 (réglage de production) | **0,71 µs** | 7,2 µs | 72 µs | 687 µs |
| Colonne, une case après l'autre | 1,2 µs | 12 µs | 115 µs | 1 134 µs |
| Colonne, par blocs, réglée pour la vitesse (`opt-level = 3`) | 0,17 µs | 1,2 µs | 13 µs | 134 µs |

Ce que disent les mesures :

- **À l'échelle d'une maison, tout est négligeable** : même à ×100, la méthode la plus chère prend
  320 ms sur une heure, soit 0,01 % d'un cœur. Le choix se fait donc sur la simplicité et la
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

**Recommandation : R**, une colonne d'échéances de 4 octets par sorte d'expiration, lue au réveil le
plus proche. Les colonnes concernées sont celles des appareils, des points à fraîcheur exigée, des
attentes du moteur, des exécutions et des minuteries ; le calendrier en ajoute une, en heure murale
(§ 10.3). C'est un seul mécanisme, écrit une fois.

### 6.4 Ce qui se passe à l'échéance

- **Un appareil devient muet** :
  - sa santé passe à `muet`, et `avail/` le dit avec sa raison (« aucune nouvelle depuis 6 min ») ;
  - ses valeurs deviennent mauvaises **par calcul à la lecture** : qualité effective = qualité de la
    fiche, sauf si l'appareil est muet. Aucune fiche n'est réécrite. Le prototype de la V7
    réécrivait chaque point de l'appareil ; la lecture coûte 0,84 ns au lieu de 0,35 ;
  - le hub publie une fois la mauvaise qualité de ses points sur `val/`. Leurs numéros se suivent :
    c'est une plage, pas une recherche ;
  - le moteur relit les feuilles des conditions composées qui lisent ces points (§ 9.5).
- **Une valeur devient trop vieille** (corrigé en v2) : la fiche passe à la qualité « mauvaise » à
  son échéance, le hub le publie, et le moteur relit les feuilles qui lisent ce point. Pourquoi un
  événement, et pas seulement un calcul à la lecture comme le disait la v1 ? Un « si » relit la
  fiche quand il passe, et le calcul lui suffirait. Mais la vérité d'un déclencheur composé est
  gardée (§ 9.5) : sans événement, « toutes les fenêtres fermées » resterait vraie sur une valeur
  que plus personne ne garantit. La soirée le vérifie : le thermomètre se tait après 22:37:31 ; avec
  une fraîcheur exigée de 15 min, sa valeur devient mauvaise à 22:52:31, bien avant que l'appareil
  ne soit déclaré muet (23:07:31).
- **La première nouvelle** réarme l'échéance par l'écriture elle-même (l'addition de la méthode B),
  et rend à la fiche sa qualité.

### 6.5 Deux horloges

- **Les durées** (santé, fraîcheur, attentes, exécutions, minuteries) sont en secondes d'une horloge
  **monotone** depuis le démarrage (`u32` : 136 ans). Une horloge murale qui saute (NTP, réglage à
  la main) ne doit ni déclarer un appareil muet, ni allonger une attente. **La v1 le disait ; son
  moteur de référence ne le faisait pas** : il comptait tout en heure murale. En v2, le moteur
  reçoit les deux à chaque appel (`Temps { mono, mur_ms }`), et la faute injectée « une attente
  compte en heure murale » fait échouer quatre tests.
- **Le calendrier et ce qu'on montre** (observé, changé, l'heure d'une occurrence) sont en
  millisecondes d'horloge murale (`u64`) : ce que voient les humains et l'historique.
- **Au redémarrage**, l'horloge monotone repart de zéro. Une échéance monotone rangée telle quelle
  ne veut donc plus rien dire : **l'instantané range chaque échéance en heure murale**, et le
  redémarrage la reconvertit (`échéance monotone = maintenant monotone + (échéance murale −
  maintenant mural)`). Une exécution en retard de plus de 2 min est arrêtée, notée « interrompue »
  (§ 11).
- **La grâce** : au démarrage, chaque appareil reçoit un délai complet
  (`contact = max(contact persisté, démarrage)`). Si le hub n'écoutait pas, ce n'est pas la faute de
  l'appareil.
- **Un saut de l'horloge murale se voit** : à chaque réveil, le hub compare ce qu'ont avancé les
  deux horloges. Un écart de plus d'une seconde est un saut ; le calendrier se recale alors (§ 10.3,
  § 10.6). Les durées ne le voient pas.

## 7. Lire à plusieurs sans se gêner

Qui lit les valeurs :

- le hub, seul rédacteur, et l'échéancier ;
- le moteur de workflow, par son miroir tenu par `val/` (V7 § 13) ;
- le module WebSocket, l'API et les agents (MCP) ;
- le journal et l'instantané.

| Façon de partager | Coût mesuré | Risque |
|---|---|---|
| **Un seul propriétaire** (le fil du hub) ; les autres reçoivent des messages ou des copies | lecture interne 0,35 ns ; copie de tout l'état vif (valeurs et santé) : 12 µs à ×1, 144 µs à ×10 | aucun |
| Verrou lecteur-rédacteur | 3,5 ns par lecture, sans rival | un lecteur lent retient le rédacteur |
| Instantané partagé (copie publiée à chaque lot) | une copie par lot : 12 µs à ×1 | mémoire doublée, plus de code |
| Lectures sans verrou (`seqlock`, atomiques) | ≈ 1 ns | code `unsafe`, lectures déchirées à prévenir |

**Recommandation : un seul propriétaire, et des copies.** Le fil du hub possède les colonnes, écrit
et passe les échéances, sans verrou sur le chemin chaud. Un écran, l'API ou un agent reçoit une
copie : copier toute la maison coûte 12 µs. Le moteur garde son miroir (16 o par point lu, V7 § 13),
indexé par le même numéro puisque le numéro voyage (§ 4.4). La même règle vaut pour les agendas : le
hub possède la table des événements, le moteur en garde une copie pour ses rappels (§ 10.9).
**Copier plutôt que partager**, c'est possible justement parce que tout est compact et de taille
fixe.

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
| Une règle d'état | `holds` relit le hub par nom : `Hub::state` alloue un `DeviceId`, prend le verrou, deux recherches, une copie ; 67 à 96 ns | `run.rs:594` |
| Une durée (`for_s`) | une tâche Tokio qui dort, plus une entrée dans une table de chaînes ; **629 octets**, perdue au redémarrage | `engine.rs:458-476` |
| « Attendre » | une attente Tokio dans la tâche de l'exécution ; perdue au redémarrage | `run.rs:396` |
| « Attendre que » | chaque exécution qui attend s'abonne à **tout** le bus du hub, et réévalue sa règle à chaque événement de la maison, quel que soit le point | `run.rs:410-429` |
| Les déclencheurs horaires | une minuterie Tokio réveille le moteur **chaque seconde** ; `on_clock` parcourt alors tous les automatismes actifs et tous leurs déclencheurs, relit l'heure en texte (`"HH:MM"`), recalcule le lever ou le coucher du soleil, et garde dans une table de chaînes la dernière minute partie de chaque déclencheur | `engine.rs:66`, `engine.rs:499-537`, `lib.rs:180` |
| Données entre nœuds | seulement le texte écrit par Sarah (`{{texte}}`) | `run.rs:124` |
| Historique des exécutions | 500 exécutions gardées en mémoire, avec le détail de chaque étape en texte | `store.rs:13` |
| Modifier la logique | arrête les exécutions et les durées en cours ; la nouvelle version attend la validation d'un humain | `lib.rs:436-474` |

Le détail des déclencheurs horaires, et de ce qu'ils font aux changements d'heure, est au § 10.1.

### 8.2 Quatre mémoires, pas une

Ta remarque est juste : **la mémoire des workflows est à part de celle des valeurs**. Elle se divise
elle-même en deux, selon ce qui change et à quel rythme. La v2 y ajoute une quatrième mémoire, celle
des agendas, tenue par le hub à part des valeurs (§ 10.9).

<!-- FIGURE:memoires -->

| Mémoire | Ce qu'elle dit | Qui l'écrit | Change | Survit au redémarrage |
|---|---|---|---|---|
| **Les valeurs** | ce que la maison est | le hub | à chaque nouvelle | journal de reprise et instantané |
| **Les agendas** | ce qui va arriver | le hub, à chaque lecture d'un agenda | toutes les quelques minutes, d'un bloc | instantané ; relus à la lecture suivante |
| **Le programme** | ce que les workflows sont | le compilateur du moteur | à chaque validation humaine | jamais écrit : recompilé (0,45 ms) depuis les définitions |
| **Les exécutions** | ce que les workflows font, la vérité des conditions composées, l'état des fiches horaires | le moteur | à chaque pas, à chaque changement d'une feuille, à chaque occurrence | journal du moteur ; reprise si l'échéance n'est pas dépassée de plus de 2 min (§ 11) |

Quatre règles les tiennent séparées :

1. **Le programme cite les valeurs par numéro, il ne les copie jamais.** Une règle porte le numéro
   de son point ; elle lit la fiche quand elle s'évalue.
2. **Une exécution cite le programme par numéro**, jamais l'inverse : le programme est figé et
   partagé.
3. **Les valeurs ne savent rien des workflows.** Le seul pont est l'index par point, qui appartient
   au programme : « le point n a des déclencheurs ici ». De même, l'agenda ne sait rien des
   rappels : ce sont des fiches du programme qui le lisent.
4. **Chaque mémoire a sa durée de vie et sa persistance** ; aucune ne dépend du format d'une autre.

### 8.3 Le programme : des nœuds numérotés

Le même principe, appliqué au graphe :

- les **automatismes** ont un numéro fixe, par un annuaire des automatismes, jamais redonné ;
- un **plan** (12 o) par automatisme dit où commencent ses nœuds ;
- les **nœuds** (32 o, deux par ligne de cache) sont rangés bout à bout ; un nœud dit ce qu'il fait
  (`op`), sur quel point, avec quelle valeur, et où commencent ses liens ;
- les **liens** (8 o) sont groupés par nœud, dans l'ordre validé : le port et le nœud suivant ;
- les **règles** (16 o) : une comparaison sur un point ; une plage horaire ou la nuit deviennent une
  comparaison sur un point du hub (§ 9.4) ;
- les **fiches horaires** (24 o) : un déclencheur « à 22:30 », « au coucher », « toutes les 15 min »
  ou « 10 min avant chaque rendez-vous » est compilé en fiche du moteur du temps (§ 10.4) ;
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
| Tout recompiler | **0,45 ms** | 1,16 ms | 7,4 ms |
| Relire `automations.json` au démarrage | 9,3 ms | 18 ms | 58 ms |

Recompiler tout le programme coûte moins d'une milliseconde à ×1. Il se rebâtit donc **entier** à
chaque validation, et il remplace l'ancien d'un coup. Aucune mise à jour partielle n'est à écrire,
ni à tester.

Une exécution désigne son nœud par **(numéro d'automatisme, rang du nœud dans l'automatisme)**, pas
par sa place globale : après une recompilation, `plans[a].premier + rang` donne la nouvelle place.
Les exécutions d'un automatisme inchangé continuent donc sans rien savoir. Celles d'un automatisme
dont la logique a changé s'arrêtent, comme aujourd'hui (`lib.rs:436-474`) : la nouvelle version
attend la validation d'un humain, et l'ancienne ne doit plus agir. Les fiches horaires suivent la
même règle : à la recompilation, chacune reprend sa dernière occurrence partie par (automatisme,
rang), et ne repart donc pas deux fois.

<!-- FIGURE:evenement -->

| Un changement de valeur, côté moteur (ns) | ×1 | ×3 | ×10 |
|---|---|---|---|
| Aujourd'hui, point quelconque (balayage, rien ne se déclenche) | 865 | 1 204 | 2 495 |
| Aujourd'hui, point déclencheur | 1 583 | 4 358 | 14 431 |
| **Index, point quelconque** (`debut[n] = debut[n+1]` : deux lectures) | **1,4** | 1,4 | 1,4 |
| **Index, point déclencheur** | **5,9** | 5,8 | 6,0 |
| Une exécution, du déclencheur jusqu'à l'attente de 5 min (un « si » de trois règles, un ordre) | 11,7 | 13,0 | 14,8 |

Le balayage d'aujourd'hui grandit avec le nombre d'automatismes ; l'index ne dépend que des
déclencheurs du point. Le banc mesure le balayage d'aujourd'hui au mieux : sans la traduction de la
valeur attendue (`canonical_expected`), ni le texte de la cause, ni la tâche lancée par
déclenchement.

### 8.4 Les exécutions : l'état du flux, à la n8n

Dans **n8n**, chaque nœud passe des données (des *items*) au suivant, et l'exécution les garde ; un
nœud « Wait » range l'exécution et la reprend plus tard. C'est un flux **avec état** : il emporte
ses données.

Dans Sarah Home, ce qu'un flux emporte est petit et connu à la compilation :

- sa **cause** : le point, la valeur, l'instant qui l'ont déclenché, ou l'heure qui a sonné ;
- le **texte** qu'a écrit Sarah (`{{texte}}`) ;
- demain, une valeur lue dans l'historique, la branche choisie par « décider ».

D'où une **fiche d'exécution de 64 octets**, une ligne de cache, plus un tiroir pour les seuls
textes. Une moitié dit où elle en est. L'autre garde **l'état qui l'a lancée, pour toute la suite**,
comme tu l'as demandé :

| Champ | Octets | Rôle |
|---|---|---|
| automatisme | 4 | son plan |
| curseur | 4 | le rang du nœud où elle en est |
| réveil | 4 | son échéance (attendre, délai d'« attendre que »), en secondes monotones ; « jamais » sinon |
| départ | 4 | quand elle a commencé |
| demande | 4 | l'ordre en vol dont elle attend l'issue sur `res/` |
| génération | 2 | la case est réutilisée : une issue tardive, destinée à l'occupante d'avant, est écartée |
| état | 1 | en cours, attend une durée, attend une condition, attend une issue |
| version | 1 | version du programme |
| **cause : point** | 4 | le point qui a déclenché, ou la feuille qui a fait basculer une condition composée ; pour une heure : l'horloge ; pour un rappel : l'agenda |
| **cause : déclencheur** | 4 | le nœud déclencheur qui est parti |
| **cause : instant** | 8 | quand (ms, horloge murale) ; pour une heure, l'occurrence |
| **cause : valeur, précédente** | 8 + 8 | la transition, au canon ; pour une heure, le retard ; pour un rappel, l'événement |
| registre | 8 | une donnée produite en route : le rang du texte de Sarah, une moyenne lue |

Ici, à l'inverse des points, **la case est réutilisée** : une exécution est éphémère, il y en aura
des millions par an. La génération remplace la règle « jamais redonné » : une réponse qui porte
(case 12, génération 7) ne touche pas l'exécution (case 12, génération 8). Finir une exécution et en
lancer une autre coûte **4,3 ns** (fiche de 64 o, mesuré). Les modes d'aujourd'hui se traduisent
sans structure de plus : le plan garde la case de son exécution en cours ; `restart` la finit et
relance, `single` ignore, `queued` garde une petite file.

**La cause d'un départ horaire tient dans les mêmes 64 octets.** Le point vaut alors une marque
réservée : `HORLOGE` (`u32::MAX`) pour une heure, `AGENDA | n° d'agenda` (bit de poids fort) pour un
rappel. L'instant est l'occurrence, la valeur le retard en secondes ou le numéro de l'événement.
L'écran l'explique sans rien stocker de plus : « l'horloge, 22:24:00, 60 s de retard » ; « agenda 1,
événement 2, rappel de 22:21:00 ». Les numéros de point restent donc sous 2³¹.

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
une **attente** dans une case fixée par le compilateur (une par déclencheur à durée) :
`échéance = maintenant + 600` (en secondes monotones), `depuis = changé`. À l'échéance, trouvée par
le même passage qu'au § 6, une seule vérification : **l'instant « changé » du point est-il toujours
`depuis` ?**

- oui : la valeur n'a pas bougé depuis 10 minutes, le déclencheur part ;
- non : elle est partie, peut-être revenue. Cette attente ne vaut plus rien, et la nouvelle
  transition a armé la sienne.

**Aucun désarmement, aucune annulation, aucune tâche** : l'instant « changé » sert de numéro de
version à la valeur. L'attente survit au redémarrage si on la range, puisque l'instant « changé »
est lui-même conservé.

**Corrigé en v2 : une attente pèse 24 octets, pas 16.** La v1 annonçait 16 octets (échéance 4,
depuis 8, nœud 4), mais son moteur gardait en plus une copie de la cause (36 octets) : 52 octets en
réalité. La v2 range exactement ce qu'il faut :

| Champ | Octets | Rôle |
|---|---|---|
| échéance | 4 | en secondes monotones |
| depuis | 8 | l'instant « changé » (ou « vraie depuis » d'une condition composée) quand l'attente s'est armée |
| partie | 8 | le dernier « depuis » parti : une vérité ne fait partir le déclencheur qu'une fois, même après un redémarrage ou une valeur revenue |
| point | 4 | le point, ou la feuille qui a fait basculer la condition : sa fiche donne la cause au départ |

Le nœud n'est pas rangé : la case est fixée par le compilateur, et le programme sait quel nœud elle
sert. La cause n'est pas copiée : au départ, elle se relit dans la fiche du point (valeur,
précédente, instant « changé »). La revérification garantit qu'elle n'a pas bougé depuis
l'armement : c'est la même condition. Une attente coûte donc **24 octets au lieu des 629 d'une tâche
Tokio**.

Une condition composée en « et » suit la même règle : elle est vraie depuis le plus grand des
instants « changé » de ses feuilles. « Toutes les fenêtres fermées depuis 10 min » se revérifie donc
sur cet instant, sans rien ranger de plus (§ 9.5).

« Attendre que » suit le même chemin. Le compilateur sait quels points lit la règle ; un événement
sur un de ces points, et seulement lui, fait relire la colonne des exécutions en attente (400
fiches : 0,12 µs). Aujourd'hui, chaque attente se réveille à chaque événement de la maison (§ 9.2).

| Mémoire des workflows (octets demandés) | ×1 | ×3 | ×10 |
|---|---|---|---|
| Programme compilé | 147 Kio | 441 Kio | 1 474 Kio |
| Exécutions en cours, au pire une par automatisme (fiches de 64 o) | 32 Kio | 128 Kio | 256 Kio |
| Attentes des déclencheurs à durée (un sur quatre, 24 o) | 2,3 Kio | 7,0 Kio | 23,4 Kio |
| Réseau des conditions composées, au pire (§ 9.5) | 55 Kio | 132 Kio | 475 Kio |
| Passer les réveils des exécutions | 0,12 µs | 0,33 µs | 1,0 µs |
| **Total : programme, exécutions, attentes, réseau** | **0,23 Mio** | **0,69 Mio** | **2,2 Mio** |
| Aujourd'hui : définitions et copie des actifs, sans les exécutions | 3,45 Mio | 10,6 Mio | 33,9 Mio |

Si chaque automatisme avait en plus un déclencheur horaire, il faudrait y ajouter 49 octets par
fiche : 19,1 Kio pour 400 (§ 10.11).

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
| 1. Une règle d'état aujourd'hui (`holds` : relire le hub par nom) | 67 ns | 96 ns | 92 ns |
| 2. Un « si » de trois règles, à la demande, par numéro | **4,8 ns** | 5,9 ns | 8,0 ns |
| 3. Réseau : un changement sur un point quelconque / lu par une règle | 1,9 / 4,7 ns | 1,9 / 4,6 ns | 2,0 / 4,6 ns |
| 3. Réseau : la bascule « nuit », lue par tous les automatismes | 0,83 µs | 2,4 µs | 8,0 µs |
| 3. Réseau : mémoire (structure et vérité) | 55 Kio | 132 Kio | 475 Kio |
| 4. Cycle d'automate : un tour, toutes les conditions | 1,8 µs | 6,9 µs | 32 µs |
| 5. Une durée armée : aujourd'hui (tâche Tokio) / proposé (attente) | 629 / 24 o | 594 / 24 o | 612 / 24 o |
| « Attendre que » aujourd'hui : coût d'un événement avec 10 / 100 / 400 attentes | 0,73 / 6,6 / 31 µs | 0,70 / 6,6 / 27 µs | 0,65 / 7,0 / 27 µs |

Ramené à une seconde de pointe (239 changements par seconde, ×1) :

- le balayage d'aujourd'hui : 207 µs par seconde (0,02 % d'un cœur) ;
- **« attendre que » d'aujourd'hui avec 400 attentes : 7,5 ms par seconde (0,7 % d'un cœur)**. Sur
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

- **Le réseau incrémental (3)** sert aux déclencheurs sur une condition composée (« quand toutes les
  fenêtres sont fermées »). **Tu les veux dès la V7** : il s'ajoute au-dessus de l'index, sans le
  remplacer (§ 9.5). Le défaut noté dans ce tableau, la reprise, ne coûte en fait qu'une
  reconstruction, mesurée à 3,6 µs.
- **Le cycle d'automate (4)** est écarté comme mécanisme principal : il rate une impulsion plus
  courte qu'un tour (un appui de bouton), qui doit de toute façon rester un événement. La V7 détecte
  déjà une valeur perdue par le rang `s` de `val/`.

### 9.4 Les plages horaires, le soleil et les agendas deviennent des valeurs

Une idée tirée du réseau incrémental : « il fait nuit », « entre 22 h et 6 h en semaine » et « en
rendez-vous » deviennent des **points tenus par le hub** (`proprietaire = hub`, V7 § 5.1). Le moteur
du temps les bascule à l'heure dite (§ 10.7). Alors :

- toute règle est une règle sur une valeur, lue par numéro ; le moteur ne convertit plus l'heure
  locale à chaque évaluation (`holds` le fait aujourd'hui pour chaque règle horaire) ;
- un déclencheur « quand la nuit tombe » est un déclencheur sur un point, dans le même index ;
- l'écran explique : « nuit : oui depuis 21 h 12 » ;
- une plage distincte est un seul point, **partagé** par tous les automatismes qui la lisent : le
  compilateur ne crée pas deux fois « 22 h–6 h en semaine ».

Le coût mesuré est une bascule « nuit » lue par les 400 automatismes : 0,83 µs, deux fois par jour.
Il y a un point par plage distincte, quelques-uns par maison.

### 9.5 Les déclencheurs composés

Décidé le 23 octobre : dès la V7, un automatisme peut partir quand une **condition composée**
devient vraie : « quand toutes les fenêtres sont fermées », « quand personne n'est là et que
l'alarme est armée ».

<!-- FIGURE:compose -->

Avec les mêmes principes :

- **la structure est dans le programme**, figée : les feuilles (une comparaison sur un point, ou sur
  un point du hub « nuit », « 22 h–6 h », « en rendez-vous »), les conditions (« et », « ou »,
  imbriquées), et un index par point, comme pour les déclencheurs simples ;
- **la vérité vit à part**, dans la mémoire des exécutions : un octet par feuille, un compteur de
  2 octets par condition. Un « et » est vrai quand son compteur atteint le nombre de ses feuilles ;
  un « ou », dès qu'il dépasse zéro ;
- **un changement ne recalcule que les feuilles de son point** (1,9 à 4,7 ns), puis remonte aux
  conditions dont le compteur bouge ;
- **le déclencheur part sur la bascule** de fausse à vraie, jamais sur un état : une condition déjà
  vraie ne repart pas à chaque changement ;
- **sa cause est la feuille qui a fait basculer la condition** : son point, sa valeur, la
  précédente, l'instant ;
- **la durée composée se calcule** : une condition en « et » est vraie depuis le plus grand des
  instants « changé » de ses feuilles. L'attente de 10 min se revérifie à l'échéance sur cet
  instant, comme pour un point seul ;
- **les changements d'un même message sont appliqués ensemble** avant de lire les bascules : une
  fenêtre qui se ferme pendant qu'une autre s'ouvre ne fait pas partir le déclencheur entre les
  deux ;
- **une feuille sur une valeur mauvaise est fausse.** Quand un appareil devient muet, ou qu'une
  valeur devient trop vieille, ses feuilles sont relues, et rien ne part sur une valeur qu'on ne
  connaît plus.

Le défaut du réseau, un état de vérité à reconstruire, ne coûte presque rien. On le recalcule d'un
coup depuis la mémoire des valeurs : au démarrage, après une recompilation, et après une perte de
messages sur `val/`.

| Réseau, au pire (un déclencheur composé de trois feuilles par automatisme) | ×1 | ×3 | ×10 |
|---|---|---|---|
| Feuilles ; conditions | 1 200 ; 400 | 3 600 ; 1 200 | 12 000 ; 4 000 |
| Structure figée (dans le programme) | 49 Kio | 113 Kio | 415 Kio |
| Vérité : un octet par feuille, compteur et instant par condition | 6,6 Kio | 19 Kio | 59 Kio |
| Reconstruire toute la vérité | **3,6 µs** | 12 µs | 47 µs |
| Un changement : point quelconque / lu par une feuille | 1,9 / 4,7 ns | 1,9 / 4,6 ns | 2,0 / 4,6 ns |

## 10. Le temps : l'équivalent d'un `cron`, rangé dans la mémoire

Ta question : comment le moteur qui tient l'équivalent d'un `cron` s'intègre-t-il à cette mémoire ?
La réponse courte : **comme le reste**.

- Une règle de temps est une **fiche de taille fixe** (24 octets), numérotée, rangée dans le
  programme ou chez le hub.
- Son état (la prochaine occurrence, la dernière partie) est une **colonne d'échéances**, lue au
  **réveil le plus proche**, le même que pour les durées (§ 6).
- Ce qu'elle produit est soit une **valeur** (un niveau : « il fait nuit »), soit un **départ
  d'exécution**, avec sa cause dans la fiche de 64 octets (§ 8.4).

Rien ne tourne chaque seconde. Le reste de la section dit ce que fait le code aujourd'hui, les
options, la forme retenue, et les règles qui la rendent juste : changements d'heure, panne, horloge
qui saute, heure pas encore sûre, agendas, précision.

### 10.1 Ce que fait le code aujourd'hui

Faits relevés dans `main` (`bd1714d`) :

| Sujet | Aujourd'hui | Où |
|---|---|---|
| Le réveil | une minuterie Tokio réveille le moteur **chaque seconde** (`interval(1 s)`) | `moli-automation/src/engine.rs:66`, `:88` |
| Le tour | pour chaque automatisme actif et chaque déclencheur horaire : l'heure relue depuis son texte `"HH:MM"`, le lever ou le coucher du soleil recalculé (approximation de la NOAA), comparés à la minute locale | `engine.rs:499-537`, `:539` |
| La précision | la minute pour « à heure fixe », « au soleil » et « toutes les N minutes » ; la seconde pour « attendre » et les durées | `model.rs` |
| Ne pas partir deux fois | un tampon `"date minute"` par (automatisme, déclencheur), dans une table de chaînes | `lib.rs:180` |
| Après une panne | rien n'est rattrapé : au démarrage, la minute courante est marquée partie (`mark_this_minute`), et ce qui est tombé pendant l'arrêt est perdu, sans trace | `engine.rs:319` |
| Retour à l'heure d'hiver | de 02:00 à 02:59, chaque minute locale existe deux fois et porte le même tampon : « à 02:30 » part une fois, à la première copie, ce qu'il faut ; mais « toutes les 15 min » saute les quatre occurrences de l'heure répétée | `engine.rs:499-537` |
| Passage à l'heure d'été | de 02:00 à 02:59, aucune minute locale n'existe : « à 02:30 » ne part pas ce jour-là | `engine.rs:499-537` |
| « Le lundi, de 22 h à 6 h » | dans une **règle** d'automatisme : le lundi de 0 h à 6 h et de 22 h à minuit (`day_ok` lit le jour courant). Dans un **planning hebdomadaire** : du lundi 22 h au mardi 6 h (`normalize` coupe la plage à minuit et rattache la suite au mardi). Le même texte, deux sens | `run.rs:651-661`, `moli-core/src/weekly.rs` |
| Les minuteries | échéance vive en horloge monotone, échéance murale rangée ; au redémarrage, une reprise impossible (il resterait plus que la durée) met la minuterie en « horloge invalide » | `moli-runtime/src/timers.rs` |
| Les plannings | prédicat civil juste aux deux copies de l'heure répétée ; recherche bornée de la prochaine limite, les sauts de décalage comptant comme des réveils | `moli-runtime/src/dependencies/calendar.rs` |
| L'agenda | le pilote ICS lit l'agenda toutes les 30 s à 1 h (5 min par défaut), 512 événements au plus, sur ±366 jours ; il publie un appareil « calendrier » et cinq séjours ; ni titre, ni participant | `moli-reservations` |

Ce qui est déjà juste, et que la v2 garde : les minuteries (deux horloges, refus d'une reprise
impossible), les plannings (heure répétée, sauts), le pilote d'agenda (bornes, aucun titre). Ce qui
ne l'est pas : les déclencheurs horaires du moteur, et le sens de « 22 h à 6 h » dans une règle.

Le banc rejoue le tour d'aujourd'hui (heure relue en texte, soleil recalculé, trois déclencheurs sur
cinq à heure fixe, un au soleil, un périodique) : **11 µs** pour 400 déclencheurs, 112 µs pour
4 000. Chaque seconde, cela fait **979 ms de processeur par jour** pour 400 déclencheurs, et 9,7 s
pour 4 000 : peu, mais pour rien, et proportionnel à la fréquence du balayage (§ 10.10).

### 10.2 Les options

La question, reformulée (**Comment pourrions-nous…**) : *comment faire partir chaque règle de temps
à l'heure juste, une seule fois, sans rien relire chaque seconde, et sans rien perdre à une panne ?*
Une **analyse morphologique**, et une **analogie** par option :

| Option | D'où elle vient | Principe | Mémoire par règle | Processeur | Ce qui reste à écrire juste |
|---|---|---|---|---|---|
| Balayage | le moteur d'aujourd'hui, le premier `cron` | chaque seconde, chaque règle est comparée à l'heure | aucune | grandit avec le nombre de règles et la fréquence | le tampon anti-doublon, les changements d'heure, le rattrapage |
| Une tâche par règle | la roue de minuteries de Tokio | chaque règle dort jusqu'à sa prochaine occurrence | ≈ 600 o, perdue au redémarrage | faible | tout, dans chaque tâche |
| Liste triée des prochaines occurrences | `cron` moderne, minuteries de `systemd` | chaque règle calcule sa prochaine occurrence ; on dort jusqu'à la plus proche | une échéance | une occurrence calculée par départ | les règles, une fois |
| **Fiches calendaires et colonne d'échéances** | la liste triée, rangée comme les valeurs | une fiche de 24 o par règle, une colonne d'échéances, le réveil au plus proche du § 6 | **49 o** | **0,51 ms par jour** pour 400 règles | les règles, une fois, testées (§ 10.5, § 10.6) |

**Retenu : la dernière.** C'est la liste triée de `cron` et de `systemd`, rangée comme le reste de
la mémoire : des fiches numérotées, une colonne d'échéances, lues au même réveil que les durées. Pas
de tas : quelques centaines de règles se parcourent plus vite qu'on ne tient un tas à jour (§ 6.3).
Le code est écrit et vérifié dans le banc (`tools/memoire-banc/src/calendrier.rs`).

### 10.3 Deux horloges, un seul réveil

<!-- FIGURE:temps -->

- **Les durées vivent en horloge monotone** (§ 6.5) ; **le calendrier, en horloge murale**, en
  millisecondes (`u64`) : « à 22:30 » est un rendez-vous avec l'heure qu'affiche la maison.
- **Un seul réveil.** L'échéancier prend la plus petite des échéances, toutes familles confondues.
  Une échéance calendaire se convertit au moment où l'on s'endort : `réveil monotone = maintenant
  monotone + (prochaine murale − maintenant mural)`.
- **Un saut de l'heure murale se voit au réveil** : si l'heure murale a avancé de plus d'une seconde
  de plus (ou de moins) que l'horloge monotone, elle a sauté. Le calendrier se recale alors (`saut`,
  § 10.6). Les durées, elles, ne voient rien.
- **L'heure doit être sûre.** Un Pi Zero 2 W n'a pas d'horloge à pile : au démarrage, il reprend
  l'heure du dernier arrêt, jusqu'à la première synchronisation NTP. Le noyau sait si l'heure est
  synchronisée (`adjtimex`, ce que lit `timedatectl`). **Tant qu'elle ne l'est pas, rien de
  calendaire ne s'arme** ; les durées, si. Quand elle le devient, les règles s'arment depuis
  maintenant : rien ne part et rien n'est compté manqué pour une période où l'heure n'était pas
  sûre.

### 10.4 La fiche calendaire

Une règle de temps tient dans une fiche de **24 octets**, comme un nœud tient dans 32 :

| Champ | Octets | Rôle |
|---|---|---|
| genre | 1 | heure, soleil, période, valeur, agenda ; ou un niveau : plage, nuit, en rendez-vous |
| jours | 1 | un bit par jour de la semaine ; 0 : tous les jours |
| bascule | 1 | soleil : lever ou coucher ; agenda : début ou fin |
| — | 1 | réserve |
| a | 4 | l'heure (seconde du jour), la période (s), le point qui porte une heure, ou l'agenda |
| b | 4 | la fin d'une plage (seconde du jour) |
| décalage | 4 | en secondes : « coucher − 30 min », « 10 min avant » (−600) |
| cible | 4 | ce que l'occurrence déclenche (le nœud déclencheur, par son rang) |
| point | 4 | pour un niveau : le point du hub qu'il tient |

Son **état** est rangé à part, en colonnes, comme les échéances du § 6 : la prochaine occurrence
(8 o, ms murales), la dernière partie (8 o), la valeur d'un niveau (1 o), et pour un agenda,
l'événement de la prochaine et de la dernière occurrence (4 + 4 o). **49 octets par règle en tout.**

| Genre | Exemple | Ce que la fiche porte |
|---|---|---|
| Heure | « à 22:30:00, en semaine » | la seconde du jour, les jours |
| Soleil | « au coucher − 30 min » | lever ou coucher, le décalage, les jours |
| Période | « toutes les 15 min » | la période en secondes |
| Instant porté par une valeur | « à l'heure de livraison annoncée » | le point qui porte l'instant (ms), un décalage |
| Heure portée par une valeur | « chaque jour à l'heure du réveil réglé, − 10 min » | le point qui porte l'heure (seconde du jour), un décalage, les jours |
| Agenda | « 10 min avant chaque rendez-vous » | l'agenda, début ou fin, le décalage |
| Niveau : plage | « entre 22 h et 6 h, en semaine » | début, fin, jours ; le point tenu |
| Niveau : nuit | « il fait nuit » | le point tenu |
| Niveau : en rendez-vous | « un rendez-vous est en cours, ou commence dans 10 min » | l'agenda, le décalage ; le point tenu |

**Où vivent les fiches.** Le même moteur du temps tourne chez deux propriétaires :

- **les niveaux sont tenus par le hub.** Ce sont des valeurs, lues par tous les automatismes, par
  les écrans et par Sarah : une fiche par niveau distinct, qui bascule un point du hub (§ 9.4) ;
- **les déclencheurs sont dans le programme du moteur.** Le compilateur fait une fiche par
  déclencheur horaire ; une occurrence lance une exécution. La fiche est recompilée avec le
  programme ; son état (dernière partie) est repris par (automatisme, rang), et rangé avec les
  exécutions.

**Et le reste d'un `cron` ?** Un `cron` dit « minute, heure, jour du mois, mois, jour de la
semaine ». Aujourd'hui, Sarah Home connaît l'heure et les jours de la semaine, le soleil et la
période. La v2 couvre les mêmes, à la seconde, plus les heures portées par une valeur et les
agendas. Le jour du mois et le mois (« le 1er de chaque mois », « de novembre à mars ») tiendraient
dans la même fiche (31 bits dans `b`, 12 dans `point`), sans un octet de plus ; ils ne sont ni
écrits ni testés, faute d'usage connu. Les jours fériés et les vacances sont un agenda (§ 10.9).

### 10.5 Les changements d'heure

<!-- FIGURE:dst -->

Les règles, chacune tenue par un test du moteur du temps :

- **Une heure qui n'existe pas** (passage à l'heure d'été, le 28 mars 2027 : de 02:00 à 03:00 en
  heure locale) : l'occurrence « à 02:30 » a lieu **au saut**, à 03:00. Une plage qui enjambe le
  saut s'ouvre au saut ; une plage tout entière dans le saut ne s'ouvre pas.
- **Une heure qui existe deux fois** (retour à l'heure d'hiver, le 25 octobre 2026 : 02:00 à 03:00
  revient) : une occurrence part **une fois, à la première** ; un niveau vaut pour les **deux
  copies**, comme les plannings d'aujourd'hui.
- **Une période compte le temps réel**, alignée sur l'UTC : « toutes les 15 min » part toutes les
  15 min, changement d'heure ou pas (quatre fois dans chaque copie de l'heure répétée).
- **Le soleil** se calcule par jour local ; le décalage s'applique en temps réel (le coucher du
  25 octobre recule d'une heure en heure locale, pas en temps réel).

Aujourd'hui, la première et la troisième sont fausses dans le moteur (§ 10.1). La deuxième y est
juste, par le tampon de la minute qui empêche la seconde copie de partir ; les plannings du runtime
la tiennent aussi.

### 10.6 Rattraper, manquer, ne jamais partir deux fois

<!-- FIGURE:rattrapage -->

Ta décision du 23 octobre pour les exécutions (« reprendre si l'on est à l'heure ») s'applique aux
occurrences, avec le même seuil :

- **une occurrence en retard de 2 min au plus part**, avec son retard (dans sa cause : « 60 s de
  retard ») ;
- **au-delà, elle est manquée** : le journal le dit (« occurrence de 22:21:30 manquée (1), retard au
  delà de 2 min »), et plusieurs occurrences manquées d'une même règle font **une seule alerte**,
  avec leur nombre : une heure de panne sur « toutes les 5 min » fait une alerte pour 11
  occurrences, pas 11 départs ;
- **jamais deux fois la même occurrence** : la dernière partie est gardée, et rangée. Une horloge
  qui recule de 8 min ne fait pas repartir 7:00 ; un redémarrage non plus ;
- **une panne et un saut en avant sont le même cas** : au réveil, les occurrences enjambées sont
  échues ; leur retard décide ;
- **un saut en arrière** ne fait rien repartir ; les niveaux se recalculent.

Les heures **portées par une valeur** (« à l'heure de livraison annoncée », « chaque jour à l'heure
du réveil réglé ») se réarment quand la valeur change, et seulement les fiches de ce point. Un
rappel d'agenda suit une règle à lui (§ 10.9) : il part en retard tant que son rendez-vous n'a pas
commencé.

La soirée de la maison le vérifie de bout en bout (§ 14) : deux occurrences de G tombent pendant la
panne de 22:20 à 22:25 ; au redémarrage, celle de 22:21:30 (3 min 30 de retard) est manquée, celle
de 22:24:00 (60 s) part.

### 10.7 Les plages, la nuit, les plannings

Un **niveau** est une fiche qui tient un point du hub à vrai ou faux. À l'armement, il calcule sa
valeur et sa prochaine limite (une borne de la plage, le lever ou le coucher, ou un changement
d'heure, qui compte comme une limite) ; à la limite, il bascule son point et se réarme. Les règles
le lisent par numéro, comme n'importe quelle valeur (§ 9.4).

**« Le lundi, de 22 h à 6 h » veut dire la nuit du lundi au mardi.** La partie après minuit
appartient au jour où la plage commence, comme dans les plannings hebdomadaires d'aujourd'hui. C'est
la seule règle de la v2 qui **change le sens d'un automatisme existant** : une règle horaire limitée
à certains jours, avec une plage qui passe minuit, était vraie la nuit d'avant (§ 10.1). **Décidé le
23 octobre** ; la migration le signale sur chaque automatisme concerné.

Les **plannings hebdomadaires** du runtime sont déjà justes ; la v2 leur donne la même forme (un
créneau par fiche de plage, le planning étant le « ou » de ses créneaux), sans changer leur sens. Le
banc ne les rejoue pas.

### 10.8 Les minuteries

Les minuteries d'aujourd'hui (`moli-runtime/src/timers.rs`) ont déjà la bonne règle ; la v2 leur
donne la forme du reste :

- **échéance vive en horloge monotone** : un réglage de l'heure ne la touche pas ;
- **rangée : 24 octets** (échéance murale 8, durée 4, génération 4, état 1, réserve 7) ;
- **au redémarrage, le reste se recalcule sur l'heure murale**, et une reprise impossible (il
  resterait plus que la durée : l'heure murale a reculé) met la minuterie en « horloge invalide » :
  elle ne repart pas seule ;
- vivante, en colonnes : durée, échéance monotone, échéance murale, génération, état (21 octets).

Quatre tests le vérifient (`src/minuteries.rs`) : l'échéance suit l'horloge monotone malgré un saut
d'une heure ; la reprise après 7 min d'arrêt finit à la seconde ; une heure murale qui a reculé de
2 h rend la reprise impossible ; une minuterie annulée ne finit pas.

### 10.9 Les agendas : « 10 min avant », et la mémoire de ce qui arrive

Tes deux questions : « 10 minutes avant un événement », et que garder en mémoire, à part des valeurs
ou dedans ? **À part des valeurs ; ni le seul jour, ni tout l'agenda : les 7 prochains jours, plus
ce qui est en cours** (décidé le 23 octobre).

<!-- FIGURE:agenda -->

**Pourquoi à part des valeurs.** Quatre raisons, dont une qui suffit :

1. **un numéro de point n'est jamais redonné** (§ 4.1). Des milliers de rendez-vous par an en
   consommeraient autant, pour rien ;
2. un rendez-vous n'est pas l'état d'un point : c'est un **intervalle à venir**, avec deux
   instants ;
3. un agenda se **relit d'un bloc** (toutes les 5 min par défaut) : la table est remplacée, pas
   modifiée valeur par valeur. Le journal de reprise, fait pour les changements de valeurs, n'a rien
   à en dire ;
4. **ni titre ni participant** ne doivent circuler, comme aujourd'hui (le pilote ICS ne les publie
   pas) : la table ne garde que des instants et des numéros.

**Ce qu'on garde : 24 octets par événement**, `{début ms, fin ms, agenda, numéro}`. Le numéro est
stable d'une lecture à l'autre : le pilote garde la correspondance avec l'identifiant de
l'événement, qu'il ne publie pas. La table appartient au hub ; le moteur en garde une copie pour ses
rappels (§ 7).

**L'horizon**, mesuré :

| Ce qu'on garde | Événements | Mémoire (réservée) | Relire l'agenda et réarmer 10 rappels |
|---|---|---|---|
| Le jour | 20 | 768 o | 0,54 µs |
| **7 jours glissants** (une semaine chargée) | 150 | 6 Kio | 5,1 µs |
| Tout un an | 3 000 | 96 Kio | 54 µs |

La mémoire réservée est celle du vecteur (une puissance de deux) ; utile : 24 octets par événement,
3,5 Kio pour la semaine chargée. **Le coût ne départage pas** : même tout l'agenda que lit le pilote
(512 événements au plus) tient en 12 Kio. Ce qui départage :

- **le jour seul ne suffit pas** : « 1 h avant » un rendez-vous de 00:30 doit partir la veille à
  23:30 ; « la veille au soir » a besoin du lendemain ;
- **tout l'agenda n'apporte rien** : un rendez-vous dans trois mois sera relu des milliers de fois,
  déplacé ou annulé peut-être, avant de compter ;
- **7 jours glissants, plus les événements en cours** couvrent « la veille », « le lundi d'avant »
  et tous les rappels d'usage. La règle exacte : l'horizon couvre au moins le plus grand décalage
  des rappels, plus une période de lecture.

**Les rappels.** « 10 min avant chaque rendez-vous » est **une seule fiche** (genre agenda, décalage
−600), quel que soit le nombre de rendez-vous. Sa prochaine occurrence est le premier événement,
dans l'ordre (début + décalage, numéro), qui suit la dernière occurrence partie. Les règles :

- **un rappel en retard part tant que son rendez-vous n'a pas commencé** (décidé le 23 octobre) :
  rendez-vous ajouté au dernier moment, panne. Après le début, il est manqué. C'est plus large que
  la règle des 2 min, parce qu'un rappel reste utile jusqu'au rendez-vous ;
- **deux rendez-vous à la même heure font deux rappels** (l'ordre départage par le numéro) ;
- **un rendez-vous déplacé ou annulé réarme le rappel** à la relecture suivante ;
- **au redémarrage**, la table, la dernière occurrence partie et son événement sont dans
  l'instantané : le rappel tombé pendant la panne part au redémarrage, si le rendez-vous n'a pas
  commencé.

Cinq tests du moteur du temps et l'automatisme H de la soirée le vérifient (§ 14) : rappel à l'heure
à 21:20 ; rappel de 22:21 tombé pendant la panne, parti à 22:25 ; rendez-vous ajouté à 22:56 pour
23:00, rappelé aussitôt ; deux rendez-vous à 23:14, deux rappels à 23:04.

**Ce que l'agenda donne aux valeurs.** Le hub en tire, par agenda, deux points : **« en
rendez-vous »**, un niveau (vrai du début, moins un décalage, à la fin), et **« prochain
rendez-vous »**, une date. Les règles les lisent par numéro, comme les autres : « si en rendez-vous,
ne pas parler à voix haute ». C'est ce que fait déjà le pilote ICS avec son appareil
« calendrier » : des valeurs bornées, tirées de l'agenda.

**Sur MQTT** (proposé) : le hub publie la table d'un agenda en un seul message gardé, remplacé à
chaque lecture (`{"g": génération, "e": [[numéro, début, fin], …]}`, environ 5 Ko pour une semaine
chargée), et ses deux points comme toutes les valeurs, sur `val/`.

**Les jours fériés et les vacances** sont un agenda comme un autre : un niveau « en congé » de plus,
sans une ligne de code de plus.

### 10.10 La précision : la minute, la seconde, la milliseconde

Ta question : à quelle précision peut-on tenir un `cron` ou une tâche planifiée, que coûte chacune,
qu'est-ce qui est possible, et qu'est-ce qui est raisonnable ? Trois choses bornent la précision :
le réveil (de combien il arrive en retard), le stockage (la résolution des échéances) et l'ordre
lui-même (le réseau jusqu'à l'appareil).

**Le réveil, mesuré.** Le banc demande une attente de 1 ms, 10 ms, 100 ms ou 1 s, par un fil qui
dort ou par Tokio, au repos puis avec deux fils qui occupent les deux CPU, et mesure le retard :

<!-- FIGURE:precision -->

| Retard du réveil (Linux musl, conteneur de 2 CPU) | Médiane au repos | Pire au repos | Médiane sous charge | Pire sous charge |
|---|---|---|---|---|
| Un fil qui dort, 1 ms demandée | 341 µs | 574 µs | 51 µs | 9,7 ms |
| Un fil qui dort, 1 s demandée | 3,5 ms | 5,2 ms | 63 µs | 88 µs |
| Tokio, 1 ms demandée | 1,6 ms | 2,8 ms | 1,1 ms | 24 ms |
| Tokio, 1 s demandée | 3,4 ms | 6,5 ms | 1,1 ms | 1,2 ms |
| Le pire de tous les cas | | 27 ms | | 24 ms |

- **Au repos, un réveil arrive avec quelques millisecondes de retard.** Tokio compte en
  millisecondes entières et arrondit au-dessus : environ 1 ms de plus.
- **Sous charge, la médiane baisse** (le processeur ne s'endort plus entre deux réveils). **Le
  pire** dépend de ce que fait la machine : 27 ms au repos, 24 ms sous charge, le pire de trois
  essais. Une série précédente, non gardée, a vu un réveil sous charge arriver avec 0,27 s de
  retard : le fil attendait son tour.
- Ce sont des mesures dans la VM Colima d'un Apple M4. **Un Pi Zero 2 W n'a pas d'hyperviseur, mais
  des cœurs plus lents** : l'ordre de grandeur tient (quelques millisecondes d'ordinaire, quelques
  dizaines au pire, davantage sous forte charge), les valeurs sont à refaire sur la carte.

**L'ordre, ensuite** (ordres de grandeur connus, non mesurés ici) : une lampe Zigbee répond en
quelques dizaines à quelques centaines de millisecondes, une prise Wi-Fi en quelques dizaines, et
l'heure de la maison n'est juste qu'à quelques millisecondes près, une fois synchronisée par NTP.
Une milliseconde au réveil ne se verrait pas sur la lampe.

**Le coût, mesuré.** Deux façons de tenir des règles de temps :

<!-- FIGURE:cron -->

- **par balayage**, le coût suit la fréquence : chaque seconde, 979 ms de processeur par jour pour
  400 règles ; dix fois par seconde, 9,8 s ; mille fois par seconde, **16,3 min par jour** (2,7 h
  pour 4 000 règles), au-delà du pour cent d'un cœur d'un Apple M4 ;
- **par le réveil au plus proche**, le coût suit le nombre d'occurrences, pas la précision : 161
  réveils et 8 000 départs par jour pour 400 règles, **0,51 ms de processeur par jour**, que les
  échéances soient à la seconde ou à la milliseconde.

**La mémoire**, enfin : les échéances du calendrier sont déjà en millisecondes (`u64`) ; rien de
plus. Les durées sont en secondes (`u32`, 136 ans) : les passer à la milliseconde demande un `u64`
(un `u32` en millisecondes déborde au bout de 49 jours), soit **4 octets de plus par échéance**.
L'attente passe de 24 à 28 octets ; la fiche d'exécution, avec deux instants, de 64 à 72 : elle ne
tient plus dans une ligne de cache.

| Précision | Ce qu'il faut | Mémoire | Processeur (400 règles) | Tenue mesurée | Verdict |
|---|---|---|---|---|---|
| La minute | ce que fait le moteur aujourd'hui | — | 979 ms par jour en balayant | oui | **c'est aujourd'hui** ; rien ne l'impose |
| **La seconde** | « HH:MM:SS », périodes en secondes ; réveil au plus proche | **rien de plus** | **0,51 ms par jour** | **oui, largement** : le pire mesuré est 27 ms (une fois 0,27 s, dans une série précédente) | **décidée, partout** |
| Le dixième (100 ms) | les mêmes échéances en `u64` ms pour les durées | + 4 o par échéance | idem | oui dans cette série (le pire : 27 ms) ; pas la fois où un réveil a pris 0,27 s | possible, sans garantie |
| La milliseconde | idem, et un fil temps réel dédié (`SCHED_FIFO`) | + 4 o par échéance, l'exécution à 72 o | idem, plus un fil | **non** : quelques millisecondes de retard au repos | dans les calculs oui (§ 10.4), au réveil non ; une impulsion de 500 ms sur un relais appartient au pilote ou à l'appareil |

**Décidé le 23 octobre : la seconde, partout**, sans rien payer. Les instants portés par une valeur
gardent leur milliseconde dans les calculs (un test le vérifie : l'occurrence part à 07:00:30,250),
mais aucune garantie n'est donnée sous la seconde.

### 10.11 Ce que le temps coûte

| Objet | Mémoire | Processeur |
|---|---|---|
| Lire l'heure | — | monotone 33 ns, murale 19 ns |
| Une fiche horaire | 24 o, plus 25 o d'état | calculer sa prochaine occurrence : 165 ns |
| 400 fiches horaires (×1) ; 4 000 (×10) | 24,5 Kio ; 196 Kio (réservés) | une journée : 8 000 départs, 0,51 ms ; 80 000 départs, 3,7 ms |
| Un événement d'agenda | 24 o | relire une semaine et réarmer 10 rappels : 5,1 µs |
| Une minuterie | 21 o vivants, 24 o rangés | — |
| Une attente de durée | 24 o | revérifiée à l'échéance : une comparaison |
| Le moteur du temps de la soirée, 400 maisons | 98 Kio réservés pour 1 600 fiches | compris dans les 60 ms du moteur (§ 14.5) |

### 10.12 Ce que le temps donne aux workflows

- **Des déclencheurs horaires justes** : à la seconde, au soleil, toutes les N secondes, à l'heure
  portée par une valeur, avant ou après chaque rendez-vous ; rattrapés ou manqués selon leur retard,
  jamais deux fois, aux changements d'heure comme les autres jours.
- **Des règles qui lisent le temps comme une valeur** : « nuit », « 22 h–6 h en semaine », « en
  rendez-vous », un point partagé par tous ; plus aucune conversion d'heure dans une condition.
- **Une cause qui dit l'heure** : « l'horloge, 22:24:00, 60 s de retard » ; « agenda 1, événement 2,
  rappel de 22:21:00 ».
- **Un « pourquoi pas » qui répond** : manquée (retard de plus de 2 min, et de combien), heure pas
  encore sûre, déjà partie pour cette occurrence.
- **Trois ajouts au modèle**, marqués dans le banc (`src/workflows.rs`) : « avant ou après chaque
  événement d'un agenda » (`WhenCalendar`), les secondes dans « à heure fixe » (`"HH:MM:SS"`), et
  des périodes en secondes.

## 11. Ce qui survit à un redémarrage

L'horloge monotone repart de zéro à chaque démarrage : tout ce qui est rangé l'est en heure murale,
et se reconvertit au redémarrage (§ 6.5).

| Mémoire | Au redémarrage | Coût mesuré |
|---|---|---|
| Annuaire des points et des appareils | relu ; les numéros sont les mêmes | — |
| Valeurs | instantané binaire puis journal de reprise (V7 § 12.1) ; échéances recalculées en horloge monotone, avec une grâce (§ 6.5) | instantané des valeurs : 30 µs à ×1, 290 µs à ×10 |
| Programme | recompilé depuis les définitions | 0,45 ms + 9,3 ms de lecture du JSON à ×1 |
| Attentes des déclencheurs à durée | encore valables si l'instant « changé » du point n'a pas bougé ; celles qui sont échues pendant l'arrêt sont revérifiées au redémarrage | 24 o chacune |
| Exécutions en attente (« attendre », « attendre que ») | reprennent, avec leur cause, si leur échéance n'est pas dépassée de plus de 2 min ; sinon arrêtées, notées « interrompue » (décidé) | 64 o chacune |
| Vérité des conditions composées | reconstruite d'un coup depuis les valeurs | 3,6 µs à ×1, 47 µs à ×10 |
| Fiches horaires du moteur | recompilées ; leur état (prochaine occurrence, dernière partie, événements) est rangé : une occurrence échue pendant l'arrêt part si elle a 2 min de retard au plus, sinon elle est manquée ; aucune ne part deux fois | 25 o d'état par fiche |
| Niveaux du hub (nuit, plages, en rendez-vous) | recalculés depuis l'heure, dès qu'elle est sûre | — |
| Agendas | la table des événements est rangée, puis remplacée à la lecture suivante | 24 o par événement |
| Minuteries | le reste se recalcule sur l'heure murale ; une reprise impossible les met en « horloge invalide » | 24 o rangés chacune |
| Baux « pendant » | rétablissent la valeur à leur terme, même en retard : revenir à l'état d'avant est le sens sûr | — |
| Historique des exécutions | sur disque et sur `evt/`, en codes ; en mémoire, une file courte de fiches fixes pour l'écran | non mesuré |

## 12. Les choix

Décidé ici, sans question, parce que ta demande, une décision déjà prise ou le code le tranche :

| Sujet | Décision | Pourquoi |
|---|---|---|
| Numéros des points et des appareils | fixes, jamais redonnés, écrits avant usage ; le bit de poids fort réservé | ta règle ; la reprise et le secours l'exigent (§ 4.2) ; les causes horaires (§ 8.4) |
| Numéros des exécutions | cases réutilisées, avec une génération | éphémères, des millions par an (§ 8.4) |
| Modifier la logique d'un automatisme | arrête ses exécutions en cours ; la nouvelle version attend un humain | le code le fait déjà (`lib.rs:436-474`) ; règle de validation |
| Expiration | colonnes de 4 o, réveil au plus proche | mesuré (§ 6.3) |
| Deux horloges | durées en monotone, calendrier en murale ; l'instantané range tout en murale | corrigé (§ 6.5) ; une faute injectée le vérifie |
| Attente d'un déclencheur à durée | 24 o ; la cause relue dans la fiche du point | corrigé (§ 8.5) |
| Fraîcheur exigée | un événement à l'échéance : la valeur devient mauvaise, ses feuilles sont relues | corrigé (§ 6.4) ; une faute injectée le vérifie |
| Partage | un seul propriétaire, des copies | mesuré (§ 7) |
| Conditions | index par point, calcul à la demande, durées par les instants | mesuré et noté (§ 9.3) |
| Déclencheurs horaires | des fiches de 24 o, une colonne d'échéances, le réveil au plus proche | mesuré (§ 10.2, § 10.10) |
| Changements d'heure | heure absente : au saut ; heure répétée : une fois, à la première ; un niveau vaut pour les deux copies ; une période compte le temps réel | la règle des plannings d'aujourd'hui, étendue (§ 10.5) |
| Occurrences en retard | la règle des 2 min, comme les exécutions ; plusieurs manquées font une alerte ; jamais deux fois | ta décision du 23 octobre, appliquée au temps (§ 10.6) |
| Heure pas encore sûre | rien de calendaire ; les durées, si | un Pi n'a pas d'horloge à pile (§ 10.3) |
| Minuteries | la règle d'aujourd'hui, en fiches de 24 o | `timers.rs` est déjà juste (§ 10.8) |
| Agendas | une table à part des valeurs, 24 o par événement, sans titre ; projetée en deux valeurs par agenda | un numéro de point n'est jamais redonné (§ 10.9) |
| Taille des colonnes | réservée au démarrage, avec une marge ; elles peuvent grandir | le numéro survit au déplacement (§ 4.1) |
| Taille de la fiche `Vif` | 40 o, la variante 32 o prête | à mesurer sur le Pi (§ 3.4) |

Décidé par toi le 23 octobre, après la v1 :

| Question | Ta réponse | Ce qui en découle |
|---|---|---|
| Après une attente, une condition lit-elle la valeur actuelle ou celle du déclenchement ? | **La valeur actuelle**, avec ta remarque : une étape franchie ne se rejoue pas, et la suite a besoin de l'état qui a déclenché | lecture dans la mémoire des valeurs ; cause complète dans la fiche d'exécution (64 o) ; une opérande « cause » dans les règles ; le flux avance seulement (§ 8.4) |
| Après un redémarrage, une exécution qui attendait reprend-elle ? | **Oui, si elle est à l'heure** | reprise si l'échéance n'est pas dépassée de plus de 2 min, sinon « interrompue » ; baux « pendant » toujours rétablis ; la même règle pour les heures (§ 10.6, § 11) |
| Faut-il des déclencheurs sur une condition composée ? | **Oui, dès la V7** | réseau incrémental au-dessus de l'index ; vérité à part, reconstruite en 3,6 µs (§ 9.5) |
| Le numéro voyage-t-il sur MQTT ? | **Oui, en plus du nom** | `n` dans `meta/`, `i` dans `val/` ; rang de l'installation, jamais une identité (§ 4.4) |

Décidé par toi le 23 octobre, après la v2 :

| Question | Ta réponse | Ce qui en découle |
|---|---|---|
| Que garder de l'agenda en mémoire ? | **7 jours glissants**, avec les événements en cours | une table à part des valeurs, 24 o par événement ; l'horizon couvre au moins le plus grand décalage des rappels plus une période de lecture ; une semaine chargée tient en 3,5 Kio (§ 10.9) |
| À quelle précision tenir les tâches planifiées ? | **La seconde** | « HH:MM:SS » et périodes en secondes ; durées en secondes `u32`, calendrier en ms ; aucune garantie sous la seconde, les impulsions courtes restent au pilote (§ 10.10) |
| « Le lundi, de 22 h à 6 h » dans une règle | **La nuit du lundi au mardi** | comme les plannings ; la migration signale chaque automatisme dont une règle horaire, limitée à certains jours, passe minuit (§ 10.7) |
| Un rappel d'agenda en retard | **Il part tant que le rendez-vous n'a pas commencé** | après le début, il est manqué ; les heures gardent la règle des 2 min (§ 10.9) |

## 13. Ce que l'étude change dans la V7

| Section de la V7 | Changement |
|---|---|
| § 10.5 la santé | l'échéance est écrite dans une colonne de 4 o à chaque nouvelle, en horloge monotone, au lieu d'être calculée au passage ; réveil au plus proche ; qualité effective calculée à la lecture, sans réécrire les fiches ; grâce au démarrage |
| § 11.2 la valeur | quatre colonnes de 4 o (échéance et délai par appareil, fraîcheur et péremption par point) ; la péremption est un événement : la valeur devient mauvaise, ses feuilles sont relues |
| § 11.4.4 les types compacts | l'annuaire est persistant, écrit avant usage ; numéros jamais redonnés ; numéros typés (`Point`, `Appareil`, `Noeud`) ; bit de poids fort réservé |
| § 12.1 le journal de reprise | confirmé : il cite les points par numéro, ce qui exige des numéros fixes. L'instantané range les échéances en heure murale, l'état des fiches horaires et la table des agendas ; le journal n'a rien à dire des agendas |
| § 13.1 le modèle | un déclencheur « quand une condition composée devient vraie », avec ou sans durée ; la cause d'une exécution, lisible par tout nœud ; un flux qui avance seulement ; « avant ou après chaque événement d'un agenda » ; les secondes dans « à heure fixe » et les périodes ; « 22 h à 6 h » appartient au jour où la plage commence |
| § 13.2 le moteur | `Noeud` devient une fiche de 32 o, rangée dans une table ; liens, règles et textes en tables ; index par point en deux tableaux ; exécutions de 64 o à génération, qui gardent leur cause, horaire comprise ; attentes de 24 o revérifiées par l'instant « changé » ; « attendre que » indexé ; plages horaires, soleil et agendas en points du hub ; réseau incrémental pour les déclencheurs composés, vérité à part, reconstruite d'un coup ; déclencheurs horaires en fiches du moteur du temps |
| **Nouveau : le moteur du temps** | un module du runtime, partagé par le hub (niveaux, minuteries, agendas) et par le moteur de workflow (déclencheurs) : fiches de 24 o, deux horloges, réveil au plus proche, changements d'heure, rattrapage, heure sûre (§ 10) |
| § 7.3 et § 7.4 le contrat | `n` dans `meta/`, `i` dans `val/` (décidé) ; un message `val/` passe de 115 à 124 octets ; proposé : la table d'un agenda en un message gardé, remplacé à chaque lecture |

Sept défauts du moteur actuel se corrigent indépendamment de la V7, et peuvent faire l'objet de
tickets :

- « attendre que » réveille chaque attente à chaque événement de la maison (`run.rs:427`) ;
- une durée armée est une tâche Tokio perdue au redémarrage (`engine.rs:476`) ;
- chaque changement de valeur parcourt tous les déclencheurs (`engine.rs:351`) ;
- chaque seconde, chaque déclencheur horaire relit son heure en texte et recalcule le soleil
  (`engine.rs:66`, `engine.rs:499-537`) ;
- aux changements d'heure, le tampon `"date minute"` saute l'heure répétée et perd l'heure absente
  (`engine.rs:499-537`, `lib.rs:180`) ;
- après une panne, rien n'est rattrapé ni signalé (`engine.rs:319`) ;
- « le lundi, de 22 h à 6 h » n'a pas le même sens dans une règle et dans un planning
  (`run.rs:651-661`, `moli-core/src/weekly.rs`).

## 14. La vérification de bout en bout : une soirée de la maison

Tu as demandé de vérifier la conception sur un workflow long, avec état, plusieurs conditions
d'entrée, des attentes et la vérification des nouvelles valeurs. C'est fait par un **moteur de
référence complet** (`tools/memoire-banc/src/moteur.rs`), qui suit exactement les règles de cette
étude, et par le **moteur du temps** (`src/calendrier.rs`) qu'il utilise. Une **soirée de la
maison** (`src/scenario.rs`) le rejoue seconde par seconde, de 18 h à 23 h 15. La v2 y ajoute trois
automatismes horaires et une valeur qui devient trop vieille. **29 tests** en vérifient le
comportement : 6 sur la soirée, 19 sur le moteur du temps, 4 sur les minuteries. Tout est rangé dans
le dépôt pour être rejoué (`tools/memoire-banc/README.md`).

<!-- FIGURE:soiree -->

### 14.1 Les automatismes de la soirée

| Automatisme | Ce qu'il fait | Ce qu'il éprouve |
|---|---|---|
| **A · Coucher de la maison** (23 nœuds, mode `single`) | **deux entrées** : fenêtres et porte fermées la nuit depuis 10 min (condition composée de six feuilles), ou le bouton « bonne nuit ». Puis : un « si » de **quatre règles** (personne au salon, alarme désarmée, entre 21 h et 2 h, maison pas déjà en nuit) ; la température **lue dans le registre** ; éteindre le salon ; attendre 5 min ; un « si » **relu après l'attente** ; fermer le volet et **vérifier sa nouvelle valeur** ; armer l'alarme et **vérifier** ; passer la maison en **mode nuit** ; attendre 30 min ; une consigne de chauffage **jamais confirmée** ; une fin **selon la cause** | le déclencheur composé avec durée, les entrées multiples, l'état produit en route, le flux qui n'avance que vers l'avant, les vérifications réussies et échouées, la cause gardée à travers une panne et une recompilation |
| **B · Couloir** (8 nœuds, mode `restart`) | au mouvement : la veilleuse si la maison est en nuit, la lumière sinon, pendant 2 min | la relance, l'issue tardive, l'interruption au redémarrage, l'arrêt par modification, la version modifiée |
| **C · Porte ouverte** | la porte passée de fermée à ouverte depuis 5 min : une alerte si la maison est en nuit | une attente qui échoit pendant la panne, revérifiée au redémarrage |
| **D · Extinction auto** | une lumière allumée depuis 10 min s'éteint, **par sa cause** (« régler le point qui a déclenché ») | une attente qui tombe parce que la valeur a bougé |
| **E · Alerte** | l'alarme passée de 2 à 3 depuis 1 min | une valeur revenue à l'identique sans la transition attendue : seule la revérification par l'instant « changé » empêche le départ |
| **F · Ronde** (v2) | à **22:30:00**, à la seconde : la porte d'entrée est-elle ouverte ? | un déclencheur horaire à la seconde, compilé en fiche du moteur du temps ; sa cause, l'horloge |
| **G · Tâches de nuit** (v2) | à 22:21:30 et à 22:24:00, une tâche | deux occurrences qui tombent pendant la panne : l'une manquée (plus de 2 min de retard), l'autre rattrapée |
| **H · Rendez-vous** (v2) | **10 min avant** chaque rendez-vous de l'agenda, un rappel | un rappel à l'heure, un rappel tombé pendant la panne, un rendez-vous ajouté au dernier moment, deux rendez-vous à la même heure |

Les points sont ceux d'une maison : quatre fenêtres, la porte, deux détecteurs, un bouton, un
thermomètre, un volet io, trois lumières, l'alarme, un radiateur, le mode de la maison (tenu par le
hub), et les niveaux du hub (la nuit, la plage 21 h–2 h), tenus par le moteur du temps du hub. Les
capteurs parlent toutes les 5 min ; le volet passe par 50 avant 0 ; le radiateur ne confirme
jamais ; le premier ordre du couloir revient 100 s plus tard ; le thermomètre exige une fraîcheur de
15 min. L'agenda est lu à 18 h (deux rendez-vous, à 21:30 et à 22:31), puis relu à 22:56 avec trois
de plus (à 23:00, et deux à 23:14). **L'horloge monotone repart de zéro au redémarrage**, comme sur
la carte : le test vérifie donc aussi la conversion des échéances rangées.

### 14.2 Ce qui arrive dans la soirée

| Heure | Événement | Ce que le moteur doit faire |
|---|---|---|
| 19:00 → 19:55 | la lumière du salon allumée, éteinte, rallumée | D l'éteint à 19:10 par sa cause ; abandonne l'attente de 19:30 (la lumière est éteinte à 19:35) ; l'éteint à 19:55 |
| 20:30 | le bouton « bonne nuit », trop tôt | A part par sa seconde entrée ; le « si » échoue (avant 21 h) : « conditions non réunies », avec le bouton pour cause |
| 21:00 → 21:03 | mouvement, puis nouveau mouvement à 21:01:30 | B allume ; son issue tarde ; il est **relancé** ; l'issue de 21:01:40, destinée à l'exécution d'avant, est **écartée** ; extinction à 21:03:31 |
| 21:20 | 10 min avant le rendez-vous de 21:30 | H rappelle, **à l'heure** ; la cause : « agenda 1, événement 1, rappel de 21:20:00 » |
| 21:42 | **un seul message** : la fenêtre du bureau se ferme, celle du salon s'ouvre | la condition de A ne bascule pas entre les deux valeurs : **rien ne s'arme** |
| 21:45 → 21:51 | le bureau fermé, rouvert, refermé | la condition est vraie depuis 21:45, fausse, vraie depuis 21:51 : l'attente est réarmée jusqu'à 22:01 |
| 22:01 → 22:07 | A part ; un mouvement au salon à 22:03 | le « si » est franchi : le mouvement ne fait **pas revenir A en arrière** ; après 5 min, le second « si » relit le salon (vide) ; le volet à 50 **ne suffit pas**, à 0 il est vérifié ; l'alarme confirme ; la maison passe en nuit |
| 22:02 | la température passe de 21,30 à 20,90 °C | le registre de A garde 2130 : la donnée produite en route ne bouge pas |
| 22:10 | le bouton, pendant l'exécution | « déclenchement ignoré, déjà en cours » (mode `single`) |
| 22:17 → 22:19 | la porte s'ouvre ; un mouvement au couloir | C arme son attente (22:22) ; B allume la veilleuse (la maison est en nuit) |
| **22:20 → 22:25** | **panne du hub**, instantané rangé ; pendant la panne tombent le rappel de 22:21 (H), les tâches de 22:21:30 et de 22:24:00 (G) | au redémarrage, l'horloge monotone repart de zéro : A reprend (réveil 22:36:48) ; B est **interrompu** (en retard de 239 s) ; l'attente de C, échue pendant la panne, est **revérifiée** et part : alerte de nuit ; G : 22:21:30 est **manquée** (3 min 30 de retard), 22:24:00 **part** (60 s) ; H : le rappel de 22:21 **part** (le rendez-vous de 22:31 n'a pas commencé) |
| 22:30 | la ronde | F part **à 22:30:00** : la porte est ouverte, alerte |
| 22:30 → 22:31 | B repart, puis **on modifie B** | B est arrêté (logique modifiée) ; A continue au même nœud, bien que ses nœuds aient glissé d'une place ; les fiches horaires de F, G et H gardent leur dernière occurrence |
| 22:36 → 22:37 | fin de l'attente de 30 min | la consigne n'est **jamais confirmée** : après 60 s, la branche d'échec, puis la fin selon la cause (la fenêtre du bureau), avec le registre |
| 22:37:31 | dernière nouvelle du thermomètre, qui se tait | rien encore |
| 22:44 → 22:45 | l'alarme 2 → 3 → 1 → 3 | E abandonne son attente : la valeur est revenue à 3, mais depuis 1 |
| 22:50 → 22:55 | la porte se ferme ; le capteur de la cuisine devient **muet** à 22:55:01 | la condition de A redevient vraie à 22:50, puis tombe (valeur plus sûre) ; l'attente de 23:00 sera abandonnée |
| **22:52:31** | le thermomètre n'a rien dit depuis 15 min | sa valeur devient **trop vieille** : mauvaise, publiée, ses feuilles relues ; l'appareil n'est pas encore muet |
| 22:56 | l'agenda est relu : un rendez-vous ajouté à 23:00, deux à 23:14 | H rappelle **aussitôt** le rendez-vous de 23:00 (son rappel de 22:50 est passé, le rendez-vous n'a pas commencé) |
| 23:04 | 10 min avant les deux rendez-vous de 23:14 | H rappelle **deux fois**, un rappel par rendez-vous |
| 23:05 | le capteur de la cuisine revient | la condition est vraie depuis 22:50 et part tout de suite. Le « si » refuse (alarme armée) : « conditions non réunies » |
| 23:07:31 | le thermomètre, toujours muet | l'appareil est déclaré **muet** |
| 23:08 → 23:12 | mouvement au couloir ; l'alarme repasse en alerte | B, modifié, va jusqu'à son nouvel avis ; E part à 23:12 |

### 14.3 Ce que les tests exigent, et la preuve qu'ils savent échouer

| Test | Ce qu'il exige |
|---|---|
| `les_ordres_de_la_soiree` | les 14 ordres, à la seconde |
| `les_avis_et_les_causes` | les 14 avis, chacun avec la cause et le registre de son exécution : la ronde, la tâche rattrapée et les cinq rappels compris |
| `le_chemin_de_a_avance_seulement` | le chemin de A, nœud par nœud, de `tout_ferme` à `fin_fenetre`, sans retour en arrière |
| `les_etats_releves` | les fiches d'exécution et la vérité de la condition relevées avant et après la panne, après la modification de B, et à 23:00 (le thermomètre : valeur mauvaise, appareil pas encore muet) ; l'état de la maison à la fin |
| `le_journal_dit_pourquoi` | les raisons, au journal, à la seconde : attente abandonnée, issue absente, relance, issue tardive écartée, condition vraie depuis, déclenchement ignoré, interruption, reprise, occurrence manquée, arrêt, valeur trop vieille, capteur muet, capteur revenu |
| `cent_maisons_identiques` | cent copies rejouées ensemble se comportent exactement comme la première |

Le moteur du temps a ses propres tests (`src/calendrier.rs`), un par règle :

| Test | Ce qu'il exige |
|---|---|
| `retour_a_l_heure_d_hiver_une_seule_fois_a_la_premiere` | « à 02:30 » part une fois le 25 octobre, à la première copie |
| `passage_a_l_heure_d_ete_au_saut` | « à 02:30 » part à 03:00 le 28 mars |
| `une_periode_compte_le_temps_reel` | « toutes les 15 min » part 8 fois de 00:00 à 02:00 UTC le 25 octobre, à travers l'heure répétée |
| `une_plage_vaut_pour_les_deux_copies_de_l_heure_repetee` | une plage de 02:30 à 02:45 s'ouvre deux fois le 25 octobre |
| `une_plage_dans_le_saut_ne_s_ouvre_jamais_celle_qui_l_enjambe_s_ouvre_au_saut` | le 28 mars, une plage de 02:15 à 02:45 ne s'ouvre pas ; une de 02:30 à 03:30 s'ouvre à 03:00 |
| `une_plage_de_nuit_appartient_au_jour_ou_elle_commence` | « le lundi, de 22 h à 6 h » vaut le lundi à 23 h 30 et le mardi à 3 h, pas le lundi à 3 h |
| `le_coucher_du_soleil_suit_les_jours_et_le_decalage` | « au coucher − 30 min » suit le soleil jour après jour, à travers le changement d'heure |
| `rattrapee_si_moins_de_2_min_de_retard_sinon_manquee` | après une panne : 1 min de retard part, 4 min est manquée |
| `une_longue_panne_ne_fait_qu_une_alerte` | une heure de panne sur « toutes les 5 min » : une seule alerte pour 11 occurrences manquées ; la dernière part, 1 min en retard |
| `un_saut_d_horloge_en_avant_rattrape_ou_manque` | l'heure saute d'une heure en avant : 7:00 est manquée, 7:59 part avec 1 min de retard |
| `un_saut_d_horloge_en_arriere_ne_fait_pas_partir_deux_fois` | 7:00 est partie, l'horloge recule de 8 min : rien ne repart jusqu'au soir |
| `rien_de_calendaire_tant_que_l_horloge_n_est_pas_sure` | rien ne part ni n'est compté manqué avant que l'heure soit sûre |
| `une_heure_portee_par_une_valeur_se_rearme_quand_elle_change` | une heure lue dans une valeur se réarme quand la valeur change |
| `la_seconde_et_la_milliseconde_sont_tenues` | « à 07:00:30 » part à la seconde ; « chaque seconde » part dix fois en dix secondes ; un instant porté par une valeur part à la milliseconde (07:00:30,250) |
| `dix_minutes_avant_chaque_evenement` | un rappel 10 min avant chaque rendez-vous, dans l'ordre |
| `un_rappel_tardif_part_tant_que_l_evenement_n_a_pas_commence` | un rappel en retard part avant le rendez-vous, pas après |
| `deux_evenements_a_la_meme_heure_font_deux_rappels` | deux rendez-vous à la même heure : deux rappels |
| `un_evenement_deplace_ou_annule_rearme_le_rappel` | un rendez-vous déplacé ou annulé réarme le rappel |
| `un_niveau_en_rendez_vous` | « en rendez-vous » vaut vrai du début à la fin du rendez-vous, puis faux |

Les 29 passent, en natif et dans le conteneur Linux musl (`rust:1-alpine`). Un test qui passe ne
prouve rien s'il ne sait pas échouer. `scripts/mutations.py` injecte donc **17 fautes**, dans le
moteur et dans le moteur du temps. **Chacune fait échouer au moins un test** :

| Faute injectée | Tests en échec |
|---|---|
| une attente ne revérifie plus l'instant « changé » | 2 : les avis, le journal (E part à tort à 22:45) |
| le redémarrage reprend tout, même très en retard | 4 : les ordres, les états, le journal, les cent maisons |
| les bascules sont lues à chaque valeur, pas à la fin du message | 1 : le journal (le message de 21:42 arme une attente) |
| le registre ne garde plus la valeur : le message relit la température en direct | 2 : les avis, les états (2090 au lieu de 2130) |
| une recompilation arrête toutes les exécutions | 5 |
| la cause n'est plus gardée par l'exécution | 2 : les avis, le chemin (la fin « selon la cause » change) |
| une issue est acceptée sans vérifier sa génération ni sa demande | 2 : les ordres, le journal |
| **une attente compte en heure murale au lieu de l'horloge monotone** (v2) | 4 : les ordres, les avis, le journal, les cent maisons |
| **une valeur trop vieille reste bonne** (v2) | 1 : les états (le relevé de 23:00) |
| **une heure répétée part à la seconde copie** (v2) | 1 : le retour à l'heure d'hiver |
| **une heure qui n'existe pas est sautée** (v2) | 1 : le passage à l'heure d'été |
| **tout se rattrape, quel que soit le retard** (v2) | 4 : le rattrapage, le saut en avant, le journal, les avis (G part deux fois) |
| **une occurrence repart après un recul de l'horloge** (v2) | 1 : le saut en arrière |
| **le calendrier s'arme sur une horloge pas encore sûre** (v2) | 1 : l'horloge pas sûre |
| **un rappel en retard ne part plus, même avant le rendez-vous** (v2) | 2 : le rappel tardif, les avis (les rappels de 22:25 et de 22:56 disparaissent) |
| **la partie après minuit d'une plage appartient au jour même** (v2) | 1 : la plage de nuit |
| **deux rendez-vous à la même heure ne font qu'un rappel** (v2) | 2 : les deux rappels, les avis |

Deux fautes passaient d'abord inaperçues, et les tests ont été renforcés :

- « une valeur trop vieille reste bonne » : à la fin de la soirée, le thermomètre est aussi muet, ce
  qui masquait la fraîcheur. Un relevé à 23:00, entre la péremption (22:52:31) et le silence déclaré
  (23:07:31), la rend visible ;
- « le calendrier s'arme sur une horloge pas encore sûre » : le test vérifiait que rien ne partait,
  pas que rien n'était compté manqué. Il exige maintenant une sortie vide.

C'était déjà la leçon de la v1, où la première version du scénario laissait passer la suppression de
la revérification par l'instant « changé » : avec un déclencheur « vers allumée », tout retour à
« allumée » réarme l'attente, et la revérification ne sert jamais. E, ajouté pour cela, rend le cas
réel : une valeur revenue à l'identique **sans** la transition attendue.

### 14.4 Ce que la soirée a appris

- **Les règles tiennent ensemble.** Les mêmes ordres, les mêmes avis et les mêmes chemins sortent
  avec un redémarrage, une horloge monotone remise à zéro, une recompilation, un capteur muet et une
  valeur trop vieille au milieu.
- **Allumer, attendre, éteindre gagne à devenir un bail.** B, interrompu par le redémarrage (la
  règle « reprendre si à l'heure »), laisse la veilleuse allumée jusqu'à son exécution suivante.
  Écrit avec un bail (« veilleuse pendant 2 min »), il l'aurait éteinte au redémarrage : un bail
  rétablit toujours la valeur, même en retard (§ 11). L'éditeur peut proposer cette forme.
- **Une condition qui redevient connue** avec le même « vraie depuis » part, si elle n'était pas
  déjà partie pour cette vérité. À 23:05, le capteur de la cuisine revient, la condition est vraie
  depuis 22:50, et A part aussitôt ; la cause est la feuille revenue. Le « si » de A l'arrête
  ensuite, puisque l'alarme est armée. C'est cohérent avec « une durée se compte depuis l'instant de
  la vérité », et le test le fige.
- **La vérification d'une nouvelle valeur est un « attendre que » après l'ordre**, avec un délai et
  une branche d'échec. Une valeur intermédiaire (le volet à 50) ne suffit pas. Un appareil qui ne
  confirme jamais (le radiateur) fait passer par la branche d'échec, sans bloquer le flux.
- **Une heure manquée se dit.** G ne fait pas semblant : 22:21:30 est notée manquée, avec son
  retard ; 22:24:00 part avec le sien. Aujourd'hui, les deux seraient perdues sans trace.
- **Un rappel reste utile jusqu'au rendez-vous.** Les rappels de 22:25 et de 22:56 sont en retard,
  et utiles ; la règle des 2 min les aurait manqués.
- **La fraîcheur parle avant la santé.** Le thermomètre devient douteux à 22:52:31, un quart d'heure
  avant d'être déclaré muet (23:07:31) : une règle qui exige une valeur fraîche le sait d'abord.

### 14.5 Les mêmes soirées, en nombre

Le scénario rejoue N maisons à la fois, dans un seul moteur, avec la même soirée (Linux musl, 2 CPU,
128 Mio).

| La soirée, de 18 h à 23 h 15 | ×1 | ×100 | ×400 |
|---|---|---|---|
| Automatismes ; nœuds | 8 ; 49 | 800 ; 4 900 | 3 200 ; 19 600 |
| Points ; messages traités | 18 ; 672 | 1 602 ; 67 002 | 6 402 ; 268 002 |
| **Processeur du moteur, toute la soirée** | **0,23 ms** | **13 ms** | **60 ms** |
| Par message | 347 ns | 192 ns | 222 ns |
| Programme compilé (dont noms et gabarits) | 6,1 Kio (2,4) | 702 Kio (239) | 2,74 Mio (0,94) |
| Les mêmes définitions sous la forme d'aujourd'hui (une copie ; le moteur actuel en garde deux) | 15,5 Kio | 1,57 Mio | 6,30 Mio |
| Exécutions simultanées au plus ; leur mémoire | 3 ; 256 o | 201 ; 25 Kio | 801 ; 100 Kio |
| Attentes des déclencheurs à durée (24 o) | 96 o | 9,4 Kio | 37,5 Kio |
| Fiches horaires du moteur (49 o) | 4 ; 196 o | 400 ; 24,5 Kio | 1 600 ; 98 Kio |
| Vérité des conditions composées | 17 o | 1,7 Kio | 6,6 Kio |
| Ordres par copie ; copies identiques à la première | 14 | 14 ; 100 sur 100 | 14 ; 400 sur 400 |

Le programme pèse plus par nœud que dans le § 8.3, parce que les noms des nœuds et les gabarits des
messages y sont comptés : A, à lui seul, porte sept textes. Toute une soirée de 400 maisons, 268 002
messages, coûte au moteur 60 ms de processeur, le temps compris.

## Annexe A — Le banc

### A.1 Comment il a été joué

- Le banc est rangé dans le dépôt, hors workspace : `tools/memoire-banc/` (son `README.md` dit
  comment tout rejouer). Il n'a aucune dépendance aux crates de Sarah Home ; il prend `jiff` (les
  fuseaux embarqués, comme le dépôt) pour le moteur du temps. Les types d'aujourd'hui y sont
  recopiés à l'identique : `Value`, `Sample`, `States` du hub, `Hub::state`, le miroir du moteur,
  tout le modèle de `moli-automation`, la boucle `on_state`, le tour `on_clock`, `holds` et la tâche
  de `hold_until`. Les ajouts proposés au modèle y sont marqués : `WhenRules`, `ReadValue`,
  `Rule::Cause`, `WhenCalendar`.
- Compilé en Linux musl par `rust:1-alpine` (rustc 1.99.1), réglé comme la production :
  `opt-level = "s"`, LTO complet, une unité de code. Un second binaire est réglé pour la vitesse
  (`opt-level = 3`), pour la seule comparaison du passage.
- Chaque mesure tourne dans un **conteneur neuf, limité à 2 CPU et 128 Mio, sans échange**, un
  processus par section et par échelle : la mémoire comptée est celle de la mesure seule.
- **Nouveau en v2 : chaque mesure est jouée trois fois**, dans trois conteneurs neufs. La série
  retient la médiane des trois essais ; pour un pire cas (le pire retard d'un réveil), le pire des
  trois (`scripts/mediane.py`). Une première série à un seul essai par mesure donnait des écarts du
  simple au double sur quelques temps (`Hub::state` à ×3, la recompilation à ×10) : le portable
  servait à autre chose pendant la mesure (charge moyenne autour de 6).
- Mémoire : octets demandés à l'allocateur (le compteur enveloppe l'allocateur de musl). Temps :
  médiane de 11 à 201 tours, lectures dans un ordre tiré au hasard, résultats passés par
  `black_box`. Retards de réveil : 200 réveils de 1 ms, 100 de 10 ms, 30 de 100 ms, 5 de 1 s, par
  cas.
- Maison : la forme des deux maisons (496 appareils, 2 264 points), répétée ×3, ×10, ×100. Elle a
  50 % de mesures, 25 % de binaires, 15 % d'énumérations, 5 % de textes et 5 % de points sans
  valeur, comme le banc mémoire de la V7. Les classes d'appareils donnent 1,96 nouvelle par seconde.
- Automatismes : l'automatisme type de la maquette (quand, si de trois règles, régler, attendre
  5 min, régler, prévenir), un sur quatre avec une durée ; 2 470 octets de JSON chacun. Déclencheurs
  horaires (section `precision`) : trois sur cinq à heure fixe, un au soleil, un périodique.
- Machine : Apple M4, Docker 29.2.1 (Colima, Ubuntu 24.04, noyau 6.8), conteneur `alpine:3`. **Le Pi
  Zero 2 W (Cortex-A53) sera plusieurs fois plus lent** : les rapports entre méthodes tiennent, les
  valeurs absolues sont à refaire sur la carte.
- Les 29 tests passent en natif et dans le conteneur de compilation
  (`sorties/linux/tests-scenario.txt`) ; les 17 mutations sont dans `sorties/mutations.txt`.
- La série de la v1 est gardée dans `sorties/v1/linux/`. D'une série à l'autre, les temps bougent
  (la recompilation à ×1 : 0,23 ms en v1, 0,45 ms ici). Les mémoires sont identiques, sauf ce que le
  code a changé : l'attente (16 puis 24 octets dans les sections du banc ; dans la soirée,
  208 octets pour ses 4 attentes en v1, 96 en v2), le réseau de la section `conditions` (qui compte
  maintenant l'instant « vraie depuis » de chaque condition, comme la section `decisions`) et la
  soirée (8 automatismes au lieu de 5). Aucune conclusion ne change.

### A.2 Les limites

- Les répliques d'aujourd'hui sont **au mieux**. La traduction de la valeur attendue
  (`canonical_expected`), le texte des causes et la tâche lancée à chaque déclenchement ne sont pas
  comptés. Le code réel est donc plus lent que le banc.
- Le banc mesure des structures, pas Sarah Home entière : ni pilotes, ni courtier, ni HTTP.
- « Attendre que » d'aujourd'hui est mesuré par 4 096 événements diffusés à 10, 100 ou 400 tâches
  Tokio abonnées (2 fils), médiane de trois essais.
- Les retards de réveil sont ceux d'une VM sur un portable : un Pi n'a pas d'hyperviseur, mais des
  cœurs plus lents. L'ordre de grandeur tient, pas les valeurs.
- Le moteur du temps est vérifié par ses tests et par la soirée ; **la détection d'un saut
  d'horloge** (comparer l'avance des deux horloges) et **la synchronisation de l'heure**
  (`adjtimex`) sont décrites, pas jouées : les tests appellent directement ce qui doit suivre
  (`saut`, horloge sûre ou non).
- **Ne sont ni écrits ni testés** : les plannings hebdomadaires en fiches de plage (§ 10.7), le jour
  du mois et le mois (§ 10.4), le message MQTT d'un agenda (§ 10.9).

### A.3 Les structures, en Rust

Bloc technique — l'annuaire et l'état vif, aux deux horloges (extraits du banc, sans `unsafe`) :

```rust
/// Le seul endroit où un nom devient un numéro. Chaque nom une fois, bout à bout ;
/// la table de hachage ne garde que des numéros. Un numéro n'est jamais redonné.
pub struct Annuaire { texte: String, fins: Vec<u32>, table: hashbrown::HashTable<u32>, hacheur: RandomState }

/// Les deux horloges : la monotone (s depuis le démarrage, jamais de saut) pour les durées,
/// la murale (ms, peut sauter) pour le calendrier et ce qu'on montre.
pub struct Temps { pub mono: u32, pub mur_ms: u64 }

/// Une colonne par sorte de fiche, toutes indexées par le même numéro.
pub struct Etat {
    pub annuaire: Annuaire,
    pub defs: Vec<Def>,        // 16 o par point
    pub vifs: Vec<Vif>,        // 40 o par point
    pub sante: Vec<Sante>,     // 20 o par appareil
    pub delai: Vec<u32>,       // par appareil : rythme × (pertes + 1), calculé une fois
    pub echeance: Vec<u32>,    // par appareil : contact + délai, en s monotones (u32::MAX : jamais)
    pub fraicheur: Vec<u32>,   // par point : fraicheur_s de la méta (0 : aucune)
    pub perime: Vec<u32>,      // par point : reçu + fraîcheur, en s monotones (u32::MAX : jamais)
    pub textes: Vec<Box<str>>, // le tiroir : la fiche porte le rang
}

impl Etat {
    /// Une nouvelle : en place, sans allocation ; les échéances se réarment par une addition,
    /// en horloge monotone ; les instants montrés restent en heure murale.
    pub fn rapport(&mut self, n: u32, valeur: i64, mur_ms: u64, mono_s: u32) -> bool {
        let (s, i) = ((mur_ms / 1_000) as u32, n as usize);
        let v = &mut self.vifs[i];
        let change = v.valeur != valeur;
        if change { v.precedente = v.valeur; v.valeur = valeur; v.change_ms = mur_ms; }
        v.observe_ms = mur_ms;
        v.recu_s = s;
        v.qualite = qualite::BONNE;                 // une valeur trop vieille redevient bonne
        let d = self.defs[i].appareil as usize;
        self.sante[d].contact_s = s;
        self.sante[d].vie = vie::VIVANT;
        if self.delai[d] != 0 { self.echeance[d] = mono_s.saturating_add(self.delai[d]); }
        if self.fraicheur[i] != 0 { self.perime[i] = mono_s.saturating_add(self.fraicheur[i]); }
        change
    }

    /// Ce qu'une condition lit : un appareil muet rend ses valeurs mauvaises sans réécrire une fiche.
    pub fn lire(&self, n: u32) -> (i64, u8) {
        let i = n as usize;
        let muet = self.sante[self.defs[i].appareil as usize].vie == vie::MUET;
        (self.vifs[i].valeur, if muet { qualite::MAUVAISE } else { self.vifs[i].qualite })
    }
}

/// Une échéance murale rangée devient une échéance monotone, au redémarrage ou pour s'endormir.
fn vers_mono(t: Temps, mur_ms: u64) -> u32 {
    t.mono.saturating_add(mur_ms.saturating_sub(t.mur_ms).div_ceil(1_000) as u32)
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

Bloc technique — le programme, l'index par point et les attentes de 24 octets :

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

/// Les attentes des déclencheurs à durée, en colonnes : 24 o. La case est fixée par le
/// compilateur, qui sait quel nœud elle sert ; la cause n'est pas copiée.
pub struct Attentes {
    pub echeance: Vec<u32>, // s monotones
    pub depuis: Vec<u64>,   // l'instant « changé » (ou « vraie depuis ») à l'armement
    pub parti: Vec<u64>,    // le dernier « depuis » parti : une vérité, un départ
    pub point: Vec<u32>,    // la fiche qui donne la cause au départ
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
                attentes.point[c] = n;
            }
        }
    }
}
// À l'échéance : elle part si et seulement si vifs[point].change_ms == depuis, que la valeur est
// bonne, et que ce « depuis » n'est pas déjà parti. La cause se relit alors dans vifs[point].

#[repr(C)]
pub struct Execution64 {    // 64 o, une ligne de cache ; la case est réutilisée, la génération écarte les issues tardives
    // où elle en est : 24 o
    pub automatisme: u32, pub curseur: u32, pub reveil_s: u32, pub depuis_s: u32, pub demande: u32,
    pub generation: u16, pub etat: u8, pub version: u8,
    // l'état qui l'a lancée, pour toute la suite : 32 o (HORLOGE ou AGENDA | n pour un départ horaire)
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

Bloc technique — le moteur du temps : la fiche, l'événement, le passage (`src/calendrier.rs`) :

```rust
pub const RATTRAPAGE_MS: u64 = 120_000;   // au-delà : manquée (décision du 23 octobre)

#[repr(C)]
pub struct Fiche {          // 24 o
    pub genre: u8,          // HEURE, SOLEIL, PERIODE, INSTANT_VALEUR, HEURE_VALEUR, AGENDA ; niveaux : PLAGE, NUIT, AGENDA_EN_COURS
    pub jours: u8,          // un bit par jour ; 0 : tous
    pub coucher: u8,        // soleil : lever ou coucher ; agenda : début ou fin
    pub _r: u8,
    pub a: u32,             // heure (s du jour), période (s), point source, agenda
    pub b: u32,             // fin d'une plage (s du jour)
    pub decalage_s: i32,    // « coucher − 30 min », « 10 min avant » : −600
    pub cible: u32,         // ce que l'occurrence déclenche
    pub point: u32,         // niveau : le point du hub qu'il tient
}

#[repr(C)]
pub struct Evenement { pub debut_ms: u64, pub fin_ms: u64, pub agenda: u32, pub numero: u32 } // 24 o, sans titre

pub struct Calendrier {
    pub tz: jiff::tz::TimeZone, pub lat: f64, pub lon: f64,
    pub fiches: Vec<Fiche>,
    pub prochaine: Vec<u64>,     // ms murales ; u64::MAX : aucune
    pub derniere: Vec<u64>,      // la dernière occurrence partie : jamais deux fois
    pub niveau: Vec<i8>,
    pub prochaine_evt: Vec<u32>, pub derniere_evt: Vec<u32>,
    pub horloge_sure: bool,      // faux : rien de calendaire
    pub evenements: Vec<Evenement>,
}

impl Calendrier {
    /// Ce qui est échu : la plus récente occurrence part si elle a 2 min de retard au plus ;
    /// celles d'avant, et elle-même au-delà, sont manquées, en une seule sortie avec leur nombre.
    pub fn passage(&mut self, maintenant_ms: u64, valeur: &dyn Fn(u32) -> Option<i64>) -> Vec<Sortie> {
        let mut sorties = Vec::new();
        if !self.horloge_sure { return sorties; }
        for k in 0..self.fiches.len() {
            let p = self.prochaine[k];
            if p > maintenant_ms { continue; }
            let f = self.fiches[k];
            // (niveaux : basculer et se réarmer ; agenda : un départ par événement, en retard tant
            //  que le rendez-vous n'a pas commencé — voir le code)
            let (mut occ, mut avant) = (p, 0u32);
            while let Some(n) = self.suivante(&f, occ, valeur).filter(|n| *n <= maintenant_ms) {
                occ = n;
                avant += 1;
            }
            let retard = maintenant_ms - occ;
            let manquees = avant + u32::from(retard > RATTRAPAGE_MS);
            if manquees > 0 { sorties.push(Sortie::Manquee { fiche: k as u32, occurrence_ms: p, nombre: manquees }); }
            if retard <= RATTRAPAGE_MS { sorties.push(Sortie::Part { fiche: k as u32, occurrence_ms: occ, retard_ms: retard }); }
            self.derniere[k] = occ;
            self.prochaine[k] = self.suivante(&f, maintenant_ms.max(occ), valeur).unwrap_or(u64::MAX);
        }
        sorties
    }
}
// `suivante` calcule la prochaine occurrence d'une fiche après un instant, en heure locale
// (jiff) : une heure absente a lieu au saut, une heure répétée à sa première copie, une période
// en temps réel, le soleil par jour local.
```

Bloc technique — une minuterie rangée (`src/minuteries.rs`) :

```rust
#[repr(C)]
pub struct Rangee {         // 24 o
    pub echeance_mur_ms: u64, pub duree_s: u32, pub generation: u32, pub etat: u8, pub _r: [u8; 7],
}
// Au redémarrage : reste = échéance murale − maintenant ; plus que la durée → HORLOGE_INVALIDE,
// sinon échéance monotone = maintenant monotone + reste.
```

### A.4 Les résultats bruts

Sortie brute — les lignes RESULTAT du banc, par section, à ×1 (Linux musl, 2 CPU, 128 Mio, médiane
de trois essais) :

```text
valeurs     points 2 264 · appareils 496 · nom moyen 39,0 car
            mémoire : states 414 408 o · miroir moteur 302 312 o · état proposé 281 396 o
                      (annuaire 117 824 o · colonnes 158 784 o)
            lire : hub_state 95,0 ns · miroir 34,9 · annuaire 35,2 · vif40 0,346 · vif32 0,325
                   chaud16 0,356 · qualité effective 0,844 · derrière verrou 3,51 · écrire 2,49
            copier : vifs+santé 12,0 µs · colonne chaude 6,4 µs · instantané binaire 29,8 µs
echeances   7 042 nouvelles/h · 9 muets · accord des quatre méthodes : oui
            heure : A 2,017 ms · B 1,162 · R 0,035 (26 passages) · C 0,446 (tas 1 428 entrées)
            nouvelle seule : A 0,308 ns · B 0,592 · C 4,840
            passage : blocs 167 ns (appareils) · colonne 291 · calculé 541 · blocs 750 (points)
workflows   400 automatismes · 2 400 nœuds · 2 483 o de JSON chacun
            mémoire : définitions 2 003 410 o · copie des actifs 1 613 010 o · programme 150 550 o
                      exécutions 16 384 o · attentes 2 400 o (100)
            compiler 0,448 ms · relire le JSON 9,281 ms
            événement : aujourd'hui 865,1 / 1 583,3 ns · index 1,363 / 5,900 ns
            exécuter jusqu'à l'attente 11,7 ns · finir et relancer 6,46 ns
conditions  règle d'état aujourd'hui 67,1 ns · si de trois règles 4,79 ns
            réseau : 56 492 o · 1,852 / 4,659 ns · bascule nuit 834 ns · cycle 1,750 µs
            durée : tâche Tokio 628,6 o · attente 24 o
            attendre que aujourd'hui : 0,73 / 6,6 / 31 µs (10 / 100 / 400 attentes)
passage     blocs 0,708 / 7,208 / 71,874 / 687,169 µs (2 264 → 2,26 millions)
            vitesse 0,166 / 1,250 / 12,875 / 134,207 µs
decisions   exécution 64 o : 32 768 o pour 400 · finir et relancer 4,27 ns
            réseau : 56 492 o (structure 49 708 o, vérité 6 784 o) · 1 200 feuilles, 400 conditions
            reconstruire 3,625 µs (×3 : 11,875 µs ; ×10 : 46,542 µs)
precision   lire l'heure : monotone 33,3 ns · murale 18,8 ns
            réveil 1 ms, au repos : fil 341 µs (pire 574 µs) · Tokio 1,6 ms (pire 2,8 ms)
            réveil 1 s, au repos : fil 3,5 ms (pire 5,2 ms) · Tokio 3,4 ms (pire 6,5 ms)
            le pire de tous les réveils : 27 ms au repos, 24 ms sous charge
            balayage d'aujourd'hui : 11,3 µs le tour (400) · 112,4 µs (4 000)
            moteur du temps : 161 réveils, 8 000 départs, 0,509 ms par jour · une occurrence 165,1 ns · fiches 25 088 o
            agenda 20 / 150 / 3 000 événements : 768 / 6 144 / 98 304 o · relire et réarmer 0,54 / 5,12 / 54,1 µs
scenario    ×1 : 8 automatismes, 49 nœuds, 672 messages, 14 ordres · moteur 0,233 ms · programme 6 268 o
            ×100 : 67 002 messages · moteur 12,889 ms · programme 718 452 o · définitions 1 646 582 o
            ×400 : 268 002 messages · moteur 59,501 ms (222,0 ns par message) · programme 2 876 340 o
                   (textes 981 520 o) · définitions 6 601 838 o · 801 exécutions au plus · copies identiques
tests       29 passed, 0 failed (natif et rust:1-alpine) · mutations : 17 détectées sur 17
```
