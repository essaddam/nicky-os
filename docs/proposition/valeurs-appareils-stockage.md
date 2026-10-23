# Valeurs, appareils et stockage : proposition de consolidation

> **Version 1, remplacée par la [V3](valeurs-appareils-stockage-v3.md)** (21 octobre 2026, après une
> [V2](valeurs-appareils-stockage-v2.md) intermédiaire). La V3 met le long terme dans l'archive
> Parquet, fait vieillir chaque valeur selon sa nature (segments, paliers, codes) et mesure ce
> que pèse chaque forme. Ce texte reste en historique ; ses parties encore valables sont listées
> au § 13 de la V3.

> **Proposition, non livrée.** Réflexion du 21 octobre 2026 sur `main`, références au code
> revérifiées sur `d0ce806`. Aucun code n'a été modifié pour l'écrire. Les tailles marquées
> *estimation* sont calculées sur le format d'enregistrement SQLite, que Turso reprend ; elles se
> mesurent avec `tools/storage-bench` avant toute décision. L'étude de la double écriture
> (§ 13.7 à 13.9) s'appuie sur une mesure datée : la série `2026-10-21T114647Z`, produite par
> [`tools/storage-writes`](../../tools/storage-writes/README.md), qui rejoue une vraie journée de
> relevés sur SQLite. Chaque chiffre mesuré de ce document en vient :
> [rapport de la série](../reports/mesure-ecritures-2026-10-21T114647Z.md).
>
> Page d'explication illustrée : [valeurs-appareils-stockage.html](valeurs-appareils-stockage.html).
> Inventaire de toutes les données persistées :
> [inventaire-persistance](../reports/inventaire-persistance-2026-10-20T060728Z.md). Décision de
> stockage : D13 dans [ARCHITECTURE.md](../ARCHITECTURE.md). Proposition voisine : [modes.md](modes.md).

## En bref

Objectif : une structure de données plus rapide à lire, moins coûteuse à écrire et plus petite
sur disque, sans perdre d'information. Chaque donnée a une seule copie, sauf un recouvrement
voulu et borné : la dernière valeur de chaque point, dans l'état courant et dans l'historique
(§ 13.7).

1. **Une valeur canonique, jamais un libellé.** On transforme une seule fois, au bord : sens d'un
   contact, échelle, unité, code d'énumération. On stocke le code et on traduit à l'affichage.
2. **Trois instants distincts** : observé (à la source), reçu (dernier rapport), changé (dernier
   changement). Aujourd'hui, un seul `ts` désigne le dernier rapport dans l'instantané et le
   dernier changement dans le flux.
3. **Une politique par valeur** : un défaut par famille, une surcharge possible par point. Elle
   fixe la bande morte et l'intervalle minimal *au stockage*, l'anti-rebond *à l'entrée* là où
   il est prouvé, la fraîcheur attendue, les durées et la fonction de consolidation.
4. **Consolider plutôt qu'éclaircir.** Les agrégats horaires sont tenus au fil de l'eau avec
   `moli-rollup`, déjà écrit mais utilisé seulement par l'archive. La moyenne est pondérée par
   le temps partout.
5. **Valeurs cumulées : l'agrégat est une différence, pas une moyenne.** On étend à tous les index
   (eau, gaz, production, impulsions) ce que `moli-energy` fait déjà pour l'électricité : delta
   réparti dans le temps, remise à zéro détectée, périodes additives, sous-compteurs non comptés
   deux fois.
6. **Fraîcheur et qualité.** Chaque appareil a un rapport attendu (Linky ≈ 3 min, contact de porte
   24 h) et chaque valeur une qualité : bonne, incertaine ou mauvaise. Une panne remonte à sa
   cause : intégration, puis appareil, puis équipement.
7. **Des équipements logiques par liaisons**, en généralisant le luminaire D12. Un portail, c'est
   la commande TaHoma plus l'état du contact Zigbee : aucune copie, et un mode dégradé explicite.
8. **Des conditions avec durée qui survivent.** `when_threshold … for_s` existe déjà. Il lui manque
   la fraîcheur, l'hystérésis et la reprise après redémarrage, via un échéancier unique.
9. **Des algorithmes revus** (§ 12) : réception d'une valeur en O(1) sans écriture disque pour un
   rapport identique, rédacteur filtré, purge après consolidation, moyennes pondérées par le temps,
   échéancier unique, export compact.
10. **Deux bases** (§ 13) : l'historique (`moli.db` de D13) d'un côté, le fonctionnement direct
    (`maison.db`) de l'autre. Chacune a son rédacteur et tourne en parallèle de l'autre : une
    rafale de relevés ne retarde jamais un clic. C'est le parallélisme de Turso, obtenu sans
    fonction expérimentale.
11. **Des changements de base** (§ 14) :
    - un relevé amaigri (*estimation* : ≈ 39 → ≈ 21 octets), un vocabulaire de l'historique, des
      dictionnaires et des agrégats ;
    - l'état métier et les équipements en tables ;
    - un journal d'export compact, à soumettre à la revue D13.
12. **Double écriture et travaux de nuit** (§ 13.7 à 13.9) :
    - `maison.db` garde l'état courant complet de chaque point (valeur et trois instants), écrit
      par lots de 60 s ; elle suffit seule au démarrage.
    - `moli.db` reçoit l'historique en parallèle : un journal en ajout le jour, rangé par point à
      chaque heure close.
    - La nuit consolide le jour, purge et sauvegarde.
    - Mesure sur une vraie journée (série `2026-10-21T114647Z`, `tools/storage-writes`) : ≈ 0,3 Go
      écrits par jour, double écriture comprise, contre ≈ 3,1 Go pour le chemin d'écriture de D13
      tel que décidé (*proxy* SQLite, à confirmer sur Turso et sur le Pi Zero 2 W de D10).

## 1. Le besoin

Reformulé depuis la demande :

- La valeur **reçue** n'est pas la valeur **stockée**, qui n'est pas la valeur **affichée**. Une
  énumération se stocke en code compact. Pour « 0 = fermé », on stocke 0 et on affiche « Fermé ».
- La **dernière mise à jour** n'est pas le **dernier changement**.
- Un **anti-rebond** peut être utile, valeur par valeur.
- Pour chaque valeur, il faut décider :
  - combien de temps on garde l'historique ;
  - combien de temps la dernière valeur reste utilisable ;
  - comment on lisse les anciennes valeurs : éclaircir ou moyenner.
- Les **usages** guident la structure. Exemple : « si la luminosité reste sous 200 plus de
  10 minutes, alors agir ». Cette question doit avoir une réponse simple et rapide.
- Il existe des **appareils physiques** (API, Zigbee…) et des **appareils logiques** qui en
  combinent plusieurs. Exemple : un portail commandé par TaHoma, dont l'état vient d'un contact
  Zigbee.
- La **fraîcheur attendue** varie : Linky ≈ toutes les 3 min, ouverture de porte au moins une fois
  par 24 h. Une panne peut toucher un appareil ou toute une API, avec plusieurs appareils derrière.
  Un appareil logique doit savoir qu'il est en mode dégradé.
- **Pas de double stockage** : une donnée physique transformée pour un appareil logique reste
  stockée une fois, et la liaison se voit.
- **Valeurs cumulées** : leur agrégation demande des précautions particulières.
- **Algorithmes** : les expliquer et voir comment les améliorer.
- **Historique dans une base à part**, et les données du fonctionnement direct dans une autre.
- **Double écriture** : la base de production garde la dernière valeur avec tout ce qu'il faut
  (dernière mise à jour, etc.) ; l'historique s'écrit en parallèle ; des travaux de nuit
  consolident et purgent. Est-ce intéressant, dans le fonctionnement réel ?
- **Turso** a été choisi pour l'embarqué et les vecteurs (embeddings) d'une part, et pour le
  multithread d'autre part : il faut en tirer parti.
- **Cas particuliers** :
  - les notes scolaires, avec moyennes par semaine et par trimestre ;
  - les informations texte, comme les derniers contenus vus sur la télévision.
- **Normaliser au maximum.** Le but : rapidité, performance, place. On change la base si nécessaire.

**Questions de cadrage (« Comment pourrions-nous… »)**

1. … stocker chaque information une seule fois, sous sa forme la plus compacte, tout en affichant
   ce que l'humain attend ?
2. … savoir à tout instant si une valeur est fiable (fraîche, source vivante) sans balayer la base ?
3. … garder des années d'historique utiles sans que le fichier grossisse au rythme des capteurs
   bavards ?
4. … décrire un équipement réel composé de plusieurs appareils, sans copie et avec un mode dégradé
   explicite ?
5. … que l'historique ne ralentisse jamais la maison, et inversement ?
6. … écrire l'état courant et l'historique en parallèle, sans transaction commune, sans que les
   deux copies se contredisent durablement ?

**Critères de choix** :
- octets par relevé et par jour ;
- écritures par seconde et coût d'un rapport identique ;
- latence d'une courbe, d'un résumé, d'un clic et du démarrage ;
- aucune perte d'information utile ;
- une seule autorité par donnée ;
- garde, confidentialité et validation humaine intactes ;
- budgets de [PERFORMANCE.md](../PERFORMANCE.md) tenus : 32 Mio anonymes, image ≤ 40 Mo, p95 API
  ≤ 25 ms, démarrage ≤ 2 s.

## 2. Ce qui existe déjà (réanalyse du 21 octobre)

| Sujet | Aujourd'hui | Écart |
| --- | --- | --- |
| Types de valeurs | `Value` = Null, Bool, Int, Float, Text ([value.rs:6](../../crates/moli-core/src/value.rs#L6)). Une énumération est un texte pris dans une liste ([spec.rs:47](../../crates/moli-core/src/spec.rs#L47)). La précision n'est pas dans le `PointSpec`. | Énumération stockée en texte et affichée brute. Une valeur publiée par un pilote n'est pas validée contre son `Kind` ([hub.rs:2371](../../crates/moli-runtime/src/hub.rs#L2371)) : un mélange Int/Float crée de faux changements. |
| Transformation | Profils : `map`, `scale`, `round`, binaire, énumération fermée, `missing` ([profile.rs:1179](../../crates/moli-profile/src/profile.rs#L1179)). Pilotes natifs : conversions codées en dur. | Ni inversion ni conversion d'unité. Le sens d'un contact dépend du pilote : Zigbee `contact` vrai = fermé, Tuya `doorcontact_state` vrai = ouvert ([tuya/model.rs:182](../../crates/moli-tuya/src/model.rs#L182)). L'interface le corrige à plusieurs endroits (`formatValue`, `doorOpen`, `WORDS`). |
| Instants | `Sample{value, ts}` ; `ts` = heure de réception, renouvelée à chaque rapport, même identique. L'événement n'est émis que sur changement ([hub.rs:2416](../../crates/moli-runtime/src/hub.rs#L2416)). Un pilote ne peut pas donner d'horodatage source ([driver.rs:559](../../crates/moli-runtime/src/driver.rs#L559)). | Un seul `ts` pour deux sens : l'instantané donne le dernier rapport, le flux le dernier changement. Le panneau Sécurité recalcule le vrai changement depuis l'historique. |
| Filtres | Dédoublonnage par égalité. Compactage du rédacteur seulement en saturation. Règle écrite : ne pas arrondir arbitrairement une mesure pour réduire les écritures ([capteurs-lumieres.md:108](../components/capteurs-lumieres.md)). | Ni bande morte, ni anti-rebond, ni intervalle minimal. Zigbee2MQTT relaie chaque champ. |
| Conservation | Familles `power`, `climate`, `energy`, `states`, `diagnostic`, `other` ([policy.rs:14](../../crates/moli-history/src/policy.rs#L14)). Sans `[storage]`, tout est gardé `history_days` = 30 j. Planchers et consentement par PIN (B1). Énergie jamais purgée. Fenêtres D13 décidées mais pas activées. | Pas d'horizon « agrégat ». Famille déduite de l'unité en dernier recours : un résumé en kWh devient `energy` ([policy.rs:92](../../crates/moli-history/src/policy.rs#L92)). |
| Lissage | `moli-rollup` : moyenne pondérée par le temps, somme, nombre, durées par état, couverture, fraîcheur ([rollup/lib.rs:34](../../crates/moli-rollup/src/lib.rs#L34)), utilisé seulement par l'archive. Courbes du cœur : seaux à moyenne simple ([store.rs:846](../../crates/moli-history/src/store.rs#L846)). Statistiques MCP pondérées par nombre de relevés ([mcp.rs:641](../../crates/moli-api/src/mcp.rs#L641)). | Moyennes biaisées sur des relevés irréguliers. Aucun agrégat local. Plus de 64 états distincts dans une période empoisonnent la publication d'archive ([rollup/lib.rs:236](../../crates/moli-rollup/src/lib.rs#L236)). |
| Valeurs cumulées | `moli-energy` traite un index correctement ([meter.rs:1](../../crates/moli-energy/src/meter.rs#L1)) : delta réparti sur les heures (31 j au plus), relevé suspect retenu, remise à zéro confirmée, recalage après 3 relevés cohérents, totaux jour/mois additifs. L'archive refuse de sommer un index d'énergie ([protocol.rs:277](../../crates/moli-archive/src/protocol.rs#L277)). | Réservé aux compteurs d'électricité. Eau, gaz, production, octets ou impulsions n'en profitent pas. Une valeur calculée Min/Max/Mean accepte un compteur en entrée. |
| Fraîcheur | Disponibilité par appareil `online: Option<bool>` ; statut par pilote ; un pilote en panne met tous ses appareils hors ligne ([supervisor.rs:45](../../crates/moli-runtime/src/supervisor.rs#L45)). Silence détecté par quelques pilotes : Tuya 35 s, profils après 3 sondes ratées, Netatmo `valid_until`, `/availability` de Zigbee2MQTT. `max_age_s` seulement pour les valeurs calculées. | Pas d'intervalle de rapport attendu générique, pas de qualité de valeur, disponibilité non persistée. L'interface déduit un « périmé » à 26 h pour la sécurité ([SecurityPanel.svelte:14](../../web/src/maison/ui/SecurityPanel.svelte#L14)). |
| Appareils logiques | Aides = appareils virtuels à état propre. `Device.members` sert aux groupes Hue. Seul vrai composite : le luminaire D12 (ampoules + relais, état dérivé, ordres redistribués) ([ARCHITECTURE.md:168](../ARCHITECTURE.md)). Valeurs calculées Min/Max/Mean publiées comme points et historisées. | Pas de composition générique : un portail Overkiz RTS n'expose qu'une commande, sans position ([pilotes.md:603](../components/pilotes.md)). Aucun lien vers un capteur d'une autre intégration, pas d'état « dégradé ». |
| Usages | `when_threshold{below, for_s}` couvre « < 200 pendant 10 min » ; `when_state(to, from, for_s)` aussi ([automation/model.rs:183](../../crates/moli-automation/src/model.rs#L183)). | Décompte en mémoire, perdu au redémarrage, et une valeur déjà sous le seuil au redémarrage ne relance rien ([engine.rs:341](../../crates/moli-automation/src/engine.rs#L341)). Ni fraîcheur ni disponibilité testées pendant le décompte ([engine.rs:476](../../crates/moli-automation/src/engine.rs#L476)), alors que [hub.rs:2733](../../crates/moli-runtime/src/hub.rs#L2733) l'exige. Pas d'hystérésis. |
| Texte | `MAX_TEXT` = 4 000 caractères, pour les ordres seulement ([spec.rs:351](../../crates/moli-core/src/spec.rs#L351)). Titres de lecture publiés en points texte et historisés à chaque changement. | Texte d'un pilote non borné. Au-delà de 4 Mio, le lot entier échoue et les purges sont suspendues ([store.rs:376](../../crates/moli-history/src/store.rs#L376)). |
| Familles à part | Notes scolaires : ni code ni plan. Pronote est prévu pour l'emploi du temps et les devoirs, notes exclues. « Récemment regardé » : absent. | À concevoir hors de l'historique ordinaire. |
| État courant | `data/state.json` : dernière valeur et `ts` de chaque point ([state_cache.rs](../../crates/moli-runtime/src/state_cache.rs)). Le fichier entier est réécrit toutes les 60 s dès qu'un rapport est arrivé, même identique, puis à l'arrêt ([main.rs:705](../../crates/moli-os/src/main.rs#L705)). Au démarrage, chaque valeur relue est réémise comme un changement avec son ancien `ts` ([hub.rs:2134](../../crates/moli-runtime/src/hub.rs#L2134)). | C'est déjà une double écriture, mais coûteuse : ≈ 81 Ko réécrits chaque minute pour 1 512 points (*estimation*). Valeur relue sans marque d'incertitude. Environ un relevé en double par point à chaque redémarrage. Aucune réconciliation : après une coupure, l'historique peut avoir 60 s d'avance. |
| État métier | Une vingtaine de fichiers JSON ou TOML. `StateFile` réécrit l'état entier à chaque changement, jusqu'à 4 Mio, avec fsync ([state_file.rs](../../crates/moli-runtime/src/state_file.rs)). La sauvegarde suit une liste `FILES` qui omet `people.json`, `settings.json` et `assistant.json` ([backup.rs:22](../../crates/moli-os/src/backup.rs#L22)). | Voir l'[inventaire](../reports/inventaire-persistance-2026-10-20T060728Z.md). |
| Turso (D13) | Un seul fichier `moli.db` pour l'historique et l'énergie. Un rédacteur, et des lecteurs sur leurs connexions, sans blocage. `BEGIN CONCURRENT` (MVCC) fonctionne mais n'est pas utilisé, car expérimental ([ARCHITECTURE.md:209](../ARCHITECTURE.md)). Vecteurs compilés par défaut ; le schéma n'emploie rien que SQLite ne sache lire ([ARCHITECTURE.md:212](../ARCHITECTURE.md)). | L'état métier reste en fichiers. Le parallélisme entre une rafale de relevés et une décision humaine n'est pas exploité. |

## 3. Le modèle proposé

```text
Intégration (pilote, API)        santé : en marche, en attente, en panne (statut existant)
 └─ Appareil physique            dernier contact, rapport attendu                 maison.db
     └─ Point source             type, nature, unité, échelle, dictionnaire, politique
         ├─ État courant         valeur, observé, reçu, changé, qualité            mémoire + maison.db
         ├─ Relevés              changements seulement                              moli.db
         ├─ Agrégats             heure → jour → mois                                moli.db
         └─ Archive              détail ou résumés                                  Parquet (D13)
Équipement logique               rôles → points sources : liaisons sans copie      maison.db
Usages                           seuils, durées, hystérésis : échéancier unique    mémoire + maison.db
```

Principe : **le point source porte la donnée ; tout le reste la référence.** L'historique et le
fonctionnement direct vivent dans deux bases (§ 13).

## 4. La valeur : reçue, canonique, stockée, affichée

### 4.1 Types canoniques

| Type | Exemples | Stocké | Affiché |
| --- | --- | --- | --- |
| Booléen | contact, présence, prise | `0` ou `1` (aucun octet de valeur en SQLite) | libellé de la sémantique : Ouvert/Fermé, Allumé/Éteint |
| Énumération | mode de chauffage, programme, état d'un portail | code du dictionnaire du point (1 octet) | libellé traduit de la clé |
| Entier | niveau, impulsions | entier | unité d'affichage |
| Décimal | température, puissance, luminosité | entier mis à l'échelle de la source (21,57 °C → `2157`, échelle 0,01) ; réel de 8 octets si aucune échelle ne convient | précision d'affichage du point |
| Index cumulé | kWh, m³, impulsions totales | comme un décimal, avec sa nature déclarée (§ 7.5) | consommation sur la période |
| Texte | titre en cours, message | texte borné à 4 000 caractères ; dictionnaire s'il se répète | tel quel |
| Instant ou durée | prochaine échéance, durée de lecture | entier en ms | date locale, durée lisible |
| Inconnu | valeur non fournie | `Null`, jamais `0` ni texte vide | « — » |

### 4.2 Transformer une fois, au bord

- Le profil ou le pilote traduit la valeur reçue en valeur canonique. Il s'occupe :
  - du sens : « ouvert » vaut vrai pour tout contact de porte, quel que soit le pilote ;
  - de l'échelle et de l'unité canonique (W, Wh, °C, lx, %) ;
  - du code d'énumération ;
  - de la borne de texte : 4 000 caractères, comme les ordres et le contrat d'archive ;
  - du contrôle du `Kind` : un entier reste un entier.
- Le hub, l'historique et l'archive ne voient que la valeur canonique.
- L'affichage la traduit avec le libellé de la sémantique et de la langue, plus l'unité et la
  précision d'affichage du point. `formatValue`, `doorOpen` et `WORDS` deviennent une seule table
  de libellés par sémantique.
- Les profils gagnent `invert` et `offset`. En écriture, ils appliquent l'échelle inverse.

### 4.3 Énumérations et dictionnaires

- Chaque énumération a un dictionnaire en ajout seul : clé stable → code (0, 1, 2…). Un code n'est
  jamais réutilisé ni renuméroté.
- Une valeur inconnue reçue (nouvelle option d'une API) reçoit un nouveau code, au lieu de devenir
  `Null` avec un avertissement comme aujourd'hui dans les profils. Elle s'affiche brute en
  attendant sa traduction.
- Les textes répétitifs passent par le même mécanisme : applications, sources, chaînes, modes. Les
  textes libres ou privés en sont exclus : messages, échanges avec l'assistant.
- À l'import depuis Home Assistant, certaines énumérations ont déjà des mots français pour valeurs.
  On les traduit en clés une fois, à l'import.
- Les dictionnaires appartiennent à la base d'historique, seule à stocker des codes (§ 13.3).
- Gains :
  - un octet au lieu d'un mot ;
  - des comparaisons entières ;
  - un historique stable quand un libellé change ;
  - un nombre d'états connu d'avance, ce qui supprime le risque des « plus de 64 états » à la
    publication d'archive.

### 4.4 Décimaux : l'échelle de la source, jamais plus grossière

L'échelle d'un point est la résolution réelle de sa source. Zigbee2MQTT donne la température au
centième : l'échelle est 0,01. Ce n'est pas un arrondi, et la règle « ne pas arrondir
arbitrairement une mesure » reste respectée. Changer l'échelle ou l'unité d'un point en ouvre une
nouvelle version, comme le fait déjà l'invalidation des valeurs calculées lors d'un changement
d'unité ([hub.rs:2078](../../crates/moli-runtime/src/hub.rs#L2078)).

## 5. Trois instants par valeur

| Instant | Sens | Où il vit | Coût |
| --- | --- | --- | --- |
| Observé le (`observed_at`) | instant de la mesure selon la source : cloud, compteur, import | clé du relevé, donc la chronologie | aucun |
| Reçu le (`received_at`) | dernier rapport, même identique | mémoire, persisté par lots dans `maison.db`, au rythme de l'écriture existante toutes les 60 s ([main.rs:705](../../crates/moli-os/src/main.rs#L705)) | une écriture groupée par minute au plus |
| Changé le (`changed_at`) | dernier changement de valeur | mémoire et `point_state` de `maison.db`, écrit avec la valeur ; l'historique le porte aussi pour un point historisé (§ 13.7) | aucun en plus : même lot que « reçu le » |

- Un rapport identique ne crée aucun relevé : il avance seulement « reçu le ».
- L'historique n'écrit jamais une valeur égale au dernier relevé du point. Les ré-énoncés du
  démarrage cessent donc d'ajouter des lignes.
- Chaque appareil a un « dernier contact » : tout message, même sans valeur. C'est lui qui mesure
  la vie de l'appareil.
- `DriverCtx` gagne un horodatage source facultatif, pour les clouds, les compteurs et les imports.
- Après un redémarrage, une valeur relue reste incertaine jusqu'au premier rapport frais.
- Une valeur relue n'est jamais réémise comme un changement : elle ne crée ni relevé ni
  déclenchement. Aujourd'hui, elle l'est, avec son ancien `ts`
  ([hub.rs:2134](../../crates/moli-runtime/src/hub.rs#L2134)).

## 6. Filtrer sans perdre de vrai

| Filtre | Pour | Où | Effet |
| --- | --- | --- | --- |
| Anti-rebond | rebonds physiques d'un contact, détecteur qui se réarme | au bord (pilote ou profil), avant le hub | tout le monde voit la valeur stabilisée ; à n'activer que là où le rebond est prouvé, car une porte ouverte puis fermée en une seconde est un vrai événement |
| Bande morte | bruit d'un capteur (±1 W, ±0,05 °C) | au stockage seulement | le hub et les automatismes voient la valeur exacte ; l'historique n'écrit que les écarts significatifs |
| Intervalle minimal | capteurs bavards (puissance chaque seconde) | au stockage seulement | au plus un relevé par intervalle ; le dernier gagne ; min et max de l'intervalle restent dans l'agrégat |

- Un filtre ne doit jamais cacher un franchissement de seuil utilisé par un automatisme. Bande morte
  et intervalle s'appliquent donc au stockage, pas à l'entrée.
- La bande morte n'est pas un arrondi arbitraire. Elle est déclarée par point, visible, réservée à
  l'historique, et l'agrégat reçoit toutes les valeurs, filtrées comprises (§ 12.3) : ses
  extrêmes et sa moyenne restent exacts.
- La position de lecture d'un média ne s'historise pas : elle se déduit de l'état de lecture et de
  son début. Selon la fréquence de publication du pilote, c'est peut-être aujourd'hui le point le
  plus bavard de la maison.
- Index cumulés : voir § 7.5.

## 7. Garder longtemps sans grossir

### 7.1 Horizons

| Famille | Aujourd'hui | Cible D13 (détail local / archive) | Proposé en plus |
| --- | --- | --- | --- |
| `states` | 30 j | 31 j / détail 365 j | durées par état, chaque heure |
| `power` | 30 j | 7 j / détail 30 j, heures 10 ans | moyenne pondérée, min, max chaque heure |
| `climate` | 30 j | 14 j / détail 90 j, heures 5 ans | moyenne pondérée, min, max chaque heure |
| `energy` | jamais purgée | heures 2 mois civils, totaux 62 j et 13 mois / heures 10 ans | même traitement pour tous les index (§ 7.5) |
| `diagnostic` | 30 j | 7 j / pas d'archive | min, max, dernière chaque heure |
| `other` | 30 j | 7 j / détail 30 j | selon la nature du point |

La dernière valeur reste toujours en état courant, même pour un point exclu de l'historique.

### 7.2 Fonction de consolidation selon la nature

| Nature | Agrégat d'une heure, d'un jour, d'un mois | Pourquoi |
| --- | --- | --- |
| Mesure continue (température, luminosité, humidité, puissance) | moyenne pondérée par le temps, min, max | les relevés sont irréguliers : une moyenne simple des relevés est fausse |
| Index cumulé (kWh, m³, impulsions) | delta réparti, total additif | on ne moyenne jamais un index (§ 7.5) |
| Binaire, énumération | durée par état, nombre de transitions | « porte ouverte 2 h 10 hier, 14 ouvertures » |
| Événement (bouton, sonnette) | nombre | — |
| Texte | aucun : événements gardés selon leur rétention | — |

- Les fonctions existent déjà dans `moli-rollup` : TimeWeighted, Sum, Count, Durations. Il manque
  l'association nature → fonction ; aujourd'hui, l'archive range en `other` ce qui n'a pas de règle.
- Chaque agrégat porte sa couverture : la part de la période couverte par des données fraîches. Un
  trou n'est pas zéro.

### 7.3 Éclaircir ou moyenner ?

- Éclaircir, c'est-à-dire garder un point sur N, perd les extrêmes et biaise les moyennes : c'est
  déconseillé au stockage.
- Moyenner par période garde min, max et couverture : c'est recommandé.
- Pour afficher une longue courbe, on réduit à la lecture, un point par pixel, pas sur le disque.

### 7.4 Agrégats tenus au fil de l'eau

- On reprend le principe de `energy_total` dans D13 : des totaux de période tenus dans la même
  transaction que l'heure.
- L'agrégat de l'heure en cours vit en mémoire et s'écrit une fois, à la fin de l'heure. Après un
  arrêt brutal, l'heure partielle se recalcule depuis le détail récent, encore présent.
- Une courbe de 30 jours lit 720 lignes au lieu de dizaines de milliers. Le détail peut garder une
  fenêtre courte.
- Les courbes du cœur et les statistiques MCP passent à la moyenne pondérée par le temps (§ 12.5).

### 7.5 Valeurs cumulées : l'agrégat est une différence

Un index (compteur Linky, eau, gaz, production solaire, impulsions, octets transférés) ne se
moyenne pas. Son maximum sur une période n'est que son dernier relevé. Ce qui compte, c'est la
quantité consommée entre deux relevés.

1. **Nature déclarée, jamais devinée par l'unité.**

   | Nature | Exemple | Agrégat |
   | --- | --- | --- |
   | Mesure | puissance, température | moyenne pondérée, min, max |
   | Index monotone | Linky, compteur d'eau | delta réparti |
   | Index à remise à zéro | consommation du jour, compteur qui redémarre | delta avec remise détectée |
   | Résumé de période | `energie:maison/today_kwh` | aucun : déjà agrégé par sa source |
   | Occurrences | appuis, ouvertures | nombre |

   Aujourd'hui, un résumé en kWh est classé `energy` par son unité ([policy.rs:92](../../crates/moli-history/src/policy.rs#L92)),
   alors que [points.rs:163](../../crates/moli-energy/src/points.rs#L163) précise que ces points ne
   sont pas des compteurs à additionner.
2. **Delta réparti dans le temps.** La quantité entre deux relevés se répartit sur les heures
   qu'ils couvrent, au prorata, et sur 31 jours au plus après un long trou. C'est déjà le cas dans
   [meter.rs](../../crates/moli-energy/src/meter.rs).
3. **Remise à zéro et changement de compteur.** Une baisse est retenue comme suspecte. Une remise
   à zéro confirmée compte le nouveau relevé depuis le précédent. Trois relevés anormaux cohérents
   recalent l'index sans rien compter. Jamais de delta négatif.
4. **Périodes additives.** Jour = somme des heures, mois = somme des jours. Jamais « index de fin −
   index de début » : c'est faux dès qu'il y a une remise à zéro, un changement de compteur ou un
   trou.
5. **Plusieurs compteurs.** On somme des deltas, jamais des index. Un sous-compteur, par exemple une
   prise mesurée sur le chauffe-eau, est déjà compté par le compteur principal. Une relation
   « inclus dans » évite de le compter deux fois. « Autres usages » = principal − somme des
   sous-compteurs, borné à zéro et signalé s'il devient négatif.
6. **Filtrage.** Sauter des relevés intermédiaires d'un index monotone ne perd aucune quantité,
   seulement de la résolution temporelle : l'intervalle minimal y est sûr. Mais sur un index à
   remise à zéro, le dernier relevé avant la remise doit toujours être gardé, sinon la fin de
   période est perdue. Pas de bande morte sur un index.
7. **Valeurs calculées.** Une moyenne d'index n'a pas de sens : un index n'alimente qu'un calcul
   de deltas.
8. **Puissance et énergie.** Intégrer la puissance donne une estimation. Quand un index existe, il
   l'emporte. On n'additionne jamais les deux.
9. **Coût.** Prix × delta par période. Un coût inconnu reste inconnu, comme aujourd'hui.

## 8. Fraîcheur, qualité et pannes

### 8.1 Rapport attendu

| Appareil | Rapport attendu (exemple) | Perdu après (proposition) |
| --- | --- | --- |
| Linky (téléinformation) | ≈ 3 min | 3 rapports manqués |
| Zigbee sur secteur | 10 min, défaut de Zigbee2MQTT | 3 rapports manqués |
| Zigbee sur pile (contact de porte) | 24 h ; Zigbee2MQTT prend 25 h | dépassement de l'intervalle |
| Intégration cloud interrogée | l'intervalle d'interrogation | 3 interrogations sans réponse |
| Valeur calculée | fraîcheur de ses entrées (`max_age_s` existe) | entrées toutes périmées |

- Un défaut par type d'appareil vient du profil ou du pilote ; on peut le surcharger par appareil.
- Trois états se déduisent à la lecture, sans balayage : frais, en retard, perdu. Perdu veut dire
  indisponible, cause « silence ».
- L'échéancier tient une seule échéance par appareil ; chaque rapport la repousse (§ 12.7).

### 8.2 Qualité d'une valeur

| Qualité | Quand | Usage par défaut |
| --- | --- | --- |
| Bonne | source fraîche et disponible | tout usage |
| Incertaine | en retard, ou relue après redémarrage sans confirmation | affichée avec un badge ; un automatisme décide selon sa règle |
| Mauvaise | perdue, indisponible, intégration en panne | jamais utilisée pour déclencher |

### 8.3 Cascade

- Intégration → appareils → points → équipements : chaque cause est portée une fois, à son niveau.
- Une intégration en panne rend ses appareils indisponibles « par héritage » : une alerte, pas une
  par appareil. C'est déjà le comportement du superviseur ; il manque la cause affichée et la
  persistance de la disponibilité.
- Les équipements calculent leur mode (normal, dégradé, indisponible) à partir de leurs rôles (§ 9).
- Les automatismes ne déclenchent jamais sur une valeur de mauvaise qualité et revérifient la
  qualité à l'échéance d'un `for_s`.

## 9. Appareils physiques et équipements logiques

- **Appareil physique** : ce qu'une intégration publie, avec une identité native stable
  (`DeviceId`) et ses points sources.
- **Équipement** : un objet de la maison (portail, chauffe-eau, voiture) décrit par des **rôles** :
  commande, état, mesure, consigne…
- Un rôle est une **liaison** vers un point source, avec une transformation sans perte : inversion,
  correspondance de codes, échelle.
- Lire un point d'équipement, c'est lire la valeur et l'historique du point source, transformation
  appliquée à la lecture. Rien n'est recopié. L'interface montre la provenance : « état fourni
  par : contact portail, Zigbee ».
- Une commande d'équipement est routée vers le point source de commande, par le même chemin que
  toute commande : hub, garde, journal.
- Une **valeur calculée** (moyenne de plusieurs capteurs, agrégat) apporte une information
  nouvelle. Elle se stocke une fois, comme point dérivé, avec sa définition versionnée et ses
  entrées. C'est déjà le cas des valeurs calculées publiées par les aides. Une simple liaison ne se
  stocke jamais.
- **Mode de l'équipement**, exemple du portail :

  | Commande (TaHoma) | État (contact Zigbee) | Portail |
  | --- | --- | --- |
  | saine | saine | normal |
  | saine | perdue | dégradé : commande sans retour d'état |
  | perdue | saine | dégradé : lecture seule |
  | perdue | perdue | indisponible |

- **Point de départ** : le luminaire D12 fait déjà cela pour les lumières : état dérivé des
  membres, ordres redistribués. On le généralise.
  - Déclaration : `[[equipment]]` dans `moli.toml`, à l'image de `[[fixture]]`.
  - Édition depuis l'interface plus tard : brouillon et version validée par PIN, comme les
    dépendances.
- **Lien avec [modes.md](modes.md)** : un profil d'équipements y est un ensemble de valeurs cibles
  (une scène), pas une composition. Les deux se complètent : une scène peut viser le rôle « commande »
  d'un équipement. La qualité d'information et la politique de péremption que demande modes.md
  sont celles du § 8.

## 10. Usages : conditions avec durée

- « Luminosité < 200 pendant 10 min » existe déjà : `when_threshold{below: 200, for_s: 600}`.
  - Sur franchissement, une attente démarre.
  - Si la valeur sort de la zone ou devient `Null`, l'attente s'annule.
  - À l'échéance, la valeur est relue.
  - Le coût est constant par changement, sans lecture d'historique : c'est la bonne forme.
- Propositions, détaillées au § 12.7 :
  1. **Survivre au redémarrage** : « vraie depuis » se reconstruit depuis le détail récent (dernier
     franchissement dans la fenêtre). Aujourd'hui, une valeur déjà sous le seuil au redémarrage ne
     relance aucun décompte.
  2. **Qualité** : annuler le décompte si la valeur devient mauvaise, et revérifier la qualité à
     l'échéance.
  3. **Hystérésis** : un seuil de sortie distinct (< 200 pour allumer, > 250 pour revenir) évite le
     battement autour du seuil.
  4. **Échéancier unique** pour les minuteurs, les plannings, les échéances de fraîcheur, les durées
     de conditions et les expirations (invitations, sessions).
     - Les minuteurs persistent déjà une échéance UTC absolue, reconstruite au démarrage
       ([timers.rs:87](../../crates/moli-runtime/src/timers.rs#L87)) ; on réutilise ce modèle.
     - Les échéances durables se persistent ; les échéances dérivées se recalculent.

## 11. Familles particulières

### 11.1 Énergie (Linky)

- Le Linky arrive aujourd'hui par des points Zigbee2MQTT ([moli.example.toml](../../moli.example.toml)).
- Index et heures sont déjà tenus par `moli-energy` (§ 7.5) ; la puissance apparente (PAPP, en VA)
  est une mesure continue.
- Rapport attendu ≈ 3 min : une coupure se voit vite.
- Le Linky doit déclarer sa nature (index monotone, puissance) plutôt que de la laisser déduire de
  l'unité.

### 11.2 Notes scolaires

Ce ne sont pas des mesures d'un capteur, mais des enregistrements. Aujourd'hui, le plan Pronote les
exclut ; s'ils entrent un jour, ils vont dans `maison.db` :

```text
eleve(id, personne)                              -- lien vers la personne de la maison
periode(id, eleve, nom, debut, fin)              -- trimestres ou semestres du calendrier scolaire
matiere(id, nom)                                 -- dictionnaire
evaluation(id, eleve, matiere, periode, date, note, bareme, coefficient, non_comptee)
moyenne_officielle(eleve, matiere, periode, valeur, publiee_le)   -- telle que publiée par la source
```

- Les moyennes par semaine ou par trimestre se calculent à la demande : quelques centaines de notes
  par an, en millisecondes, sur les périodes du calendrier scolaire et non du calendrier civil.
- La moyenne officielle publiée par la source peut différer d'un recalcul (arrondis, notes non
  comptées) : c'est une information différente, pas un doublon.
- Ce sont des données d'enfants : accès par personne, rétention limitée à l'année scolaire, pas
  d'archive par défaut. Le préalable de confidentialité F0 du plan famille s'applique.

### 11.3 Informations texte et médias

- Les points de lecture en cours (titre, application, état) restent des points texte pour l'état
  courant.
- Leur historique devient un journal d'événements :
  `lecture(appareil, contenu, debut, fin)`, avec `contenu(id, titre, type, application)` dans un
  dictionnaire. Un titre revu dix fois n'est stocké qu'une fois.
- Position et durée de lecture ne s'historisent pas (§ 6).
- Un historique de visionnage est personnel : rétention courte et effaçable.

### 11.4 Textes libres

Borne à 4 000 caractères au bord, avec un indicateur de troncature. Un texte trop long ne doit plus
faire échouer tout un lot d'historique ni suspendre les purges.

## 12. Les algorithmes : aujourd'hui, limites, améliorations

### 12.1 Vue d'ensemble

| Algorithme | Aujourd'hui | Limite | Amélioration |
| --- | --- | --- | --- |
| Réception d'une valeur (hub) | O(1) : filtre privé, comparaison à la valeur précédente, événement si changement, `ts` renouvelé à chaque rapport ([hub.rs:2371](../../crates/moli-runtime/src/hub.rs#L2371)) | `ts` à double sens, pas de contrôle du `Kind`, texte non borné | valeur canonique au bord, deux instants, qualité (§ 12.2) |
| Rédacteur de l'historique | lots de 2 s ou 1 000 lignes. En saturation (4 000 en attente), il garde le dernier changement par point, puis jette les plus anciens ([lib.rs:384](../../crates/moli-history/src/lib.rs#L384)) | ré-énoncés au démarrage ; en saturation, extrêmes perdus ; une page par point et par lot (§ 13.8) | filtres de politique, jamais une valeur égale au dernier relevé, agrégat alimenté par toutes les valeurs (§ 12.3) ; journal en ajout, rangé par point à l'heure close (§ 13.8) |
| Purge ancrée | toutes les 6 h, par point, lots de 5 000, pause de 2 ms. Elle garde le dernier relevé avant la coupe et vérifie plancher, suspension et génération à chaque suppression (B1) | solide | ne purger que ce qui est consolidé et publié ; plages de rowid en D13 (§ 12.4) ; la nuit (§ 13.9) |
| Courbes du cœur | brut si ≤ N points, sinon seaux de largeur fixe en moyenne simple ; non numérique : les N derniers ([store.rs:846](../../crates/moli-history/src/store.rs#L846)) | moyenne biaisée, états tronqués | moyenne pondérée par le temps, agrégats au-delà de quelques jours, états en segments (§ 12.5) |
| Statistiques MCP | pondérées par nombre de relevés ([mcp.rs:641](../../crates/moli-api/src/mcp.rs#L641)) | biais vers les périodes bavardes | pondération par le temps (§ 12.5) |
| Consolidation `moli-rollup` | moyenne pondérée, somme, nombre, durées par état, couverture ; archive seulement | absente du cœur ; plus de 64 états bloque la publication | agrégats horaires incrémentaux dans le cœur (§ 12.5) |
| Index (`meter.rs`) | delta réparti, relevé suspect, remise à zéro, recalage, 31 j au plus | électricité seulement | toute nature cumulée, relation « inclus dans » (§ 7.5) |
| Valeurs calculées | Min/Max/Mean de 32 sources fraîches au plus ; publiée dès une source valide ; `ts` = source la plus ancienne ([aggregate.rs](../../crates/moli-core/src/aggregate.rs)) | pas de quorum : la dernière source restante fait « la moyenne » sans le dire ; accepte un index | quorum minimal, qualité exposée, somme explicite, index refusés (§ 12.6) |
| Durées des automatismes | franchissement → tâche `sleep(for_s)`, annulée en sortie de zone ou sur `Null`, revérifiée à l'échéance ([engine.rs:458](../../crates/moli-automation/src/engine.rs#L458)) | perdue au redémarrage ; ni qualité ni hystérésis ; une tâche par décompte | échéancier unique, reprise, qualité, hystérésis (§ 12.7) |
| Disponibilité | un pilote en panne met ses appareils hors ligne, avec un backoff de 1 à 60 s ; quelques pilotes détectent le silence ([supervisor.rs:45](../../crates/moli-runtime/src/supervisor.rs#L45)) | pas d'échéance générique ; cause non portée ; disponibilité non persistée | échéance par appareil dans l'échéancier, cause racine, persistance (§ 8, § 12.7) |
| État courant (`state.json`) | tout le fichier réécrit toutes les 60 s dès qu'un rapport est arrivé ; relu et réémis au démarrage | ≈ 81 Ko par minute (*estimation*, 1 512 points) ; un relevé en double par point à chaque redémarrage ; ni incertitude ni réconciliation | `point_state` par lots de 60 s, réconciliation silencieuse au démarrage (§ 13.7) |
| Écriture de l'état métier (`StateFile`) | sérialise tout l'état, l'écrit à côté, fsync, renomme, fsync du dossier ; `commit_unknown` après le renommage | coût proportionnel à la taille totale (jusqu'à 4 Mio) à chaque changement | une ligne par changement dans `maison.db` (§ 13) |
| Journal des actions | ajout ; au-delà de 8 Mio, réécrit les 2 000 dernières lignes | réécriture complète, aucune requête possible | table en ajout seul, purge par plage de rowid |
| Export (D13) | outbox et journal NDJSON immuable par opération, lots de 5 000 au plus, en gzip | ≈ 128 octets par opération ; lots ≤ 100 ms : presque une validation et un fsync par relevé | opération compacte figée, lot rendu une fois puis figé (§ 12.8) ; elle sert aussi de journal du jour ; lots de 2 s (§ 13.8) |
| Sauvegarde | liste `FILES` et `VACUUM INTO` par base, à la main ; manifeste avec les tailles | oublis, pas d'empreintes, aucune planification | deux instantanés, empreintes SHA-256, plus aucune liste (§ 13.6) ; `moli.db` la nuit (§ 13.9) |

### 12.2 Réception d'une valeur

```text
au bord (pilote ou profil), une fois :
    v     ← canonique(brut)            -- Kind contrôlé, sens, échelle, unité, code, texte ≤ 4 000
    t_obs ← horodatage source s'il existe, sinon maintenant

dans le hub, en O(1) :
    p.reçu_le ← maintenant ; appareil.dernier_contact ← maintenant
    échéancier.repousser(appareil, maintenant + attendu(appareil))
    si v ≠ p.valeur :
        p.valeur ← v ; p.changé_le ← t_obs ; p.qualité ← bonne
        émettre État(p, v, t_obs)
    sinon si p.qualité ≠ bonne :          -- confirmée après un retard ou un redémarrage
        p.qualité ← bonne ; émettre Qualité(p)
```

Un rapport identique coûte deux affectations et une opération sur le tas des échéances : aucune
écriture disque immédiate, aucun événement. Le contrôle du `Kind` au bord supprime les faux
changements Int/Float.

### 12.3 Rédacteur de l'historique

```text
pour chaque État(p, v, t) :
    agrégat[p].ajouter(v, t)                              -- toujours : l'agrégat voit tout
    si v = dernier_écrit[p] : ignorer                     -- plus aucun ré-énoncé
    si nature(p) = mesure et |v − dernier_écrit[p]| < bande_morte(p) : ignorer
    si t − dernier_t[p] < intervalle_min(p) :
        en_attente[p] ← (v, t) ; continuer                -- le dernier gagne
    si nature(p) = index à remise et v < dernier_écrit[p] :
        écrire d'abord en_attente[p]                      -- la fin de période n'est jamais perdue
    lot.ajouter(p, v, t) ; dernier_écrit[p] ← v ; dernier_t[p] ← t
toutes les 2 s ou 1 000 lignes : une transaction, une opération compacte en ajout par relevé (§ 13.8)
à l'heure close : ranger l'heure par point dans samples et écrire son agrégat, une transaction
en cas d'erreur : rollback, puis suspension des purges (B1, inchangé)
```

- La bande morte compare au dernier relevé **écrit**, pas au dernier reçu : une dérive lente finit
  toujours par s'écrire.
- L'agrégat reçoit toutes les valeurs, même celles que le détail n'écrit pas. Moyenne, min et max
  de l'heure restent exacts, même quand le rédacteur sature.

### 12.4 Purge

```text
chaque nuit (§ 13.9 ; aujourd'hui toutes les 6 h), par point, lots de 5 000 :
    coupe ← maintenant − durée_effective(p)               -- plancher et consentement B1 inclus
    coupe ← min(coupe, consolidé_jusqu'à(p))              -- jamais avant l'agrégat de l'heure
    si famille archivée : coupe ← min(coupe, instant(published_upto))
    ancre ← dernier relevé < coupe ; supprimer [plus ancien, ancre)
    à chaque lot : vérifier plancher, suspension et génération (B1, inchangé)
```

En D13, chaque suppression est une plage de rowid : `[(p << 44) + début, (p << 44) + ancre)`.

### 12.5 Consolidation et courbes

```text
en mémoire, par point, pour l'heure en cours (fonctions de moli-rollup) :
    mesure : Σ v·Δt, min, max, couverture      -- la valeur tient jusqu'au changement suivant
    état   : durée par code, nombre de transitions
    index  : delta réparti (meter.rs)
à chaque heure pleine : une transaction écrit agg_hour / state_hour de l'heure close
                       et range l'heure du journal par point dans samples (§ 13.8)
la nuit : agg_day depuis les heures du jour clos (23 ou 25 lors d'un changement d'heure, § 13.9)
courbe demandée : ≤ 2 j → relevés (les états en segments début–fin)
                  ≤ 90 j → agg_hour ; au-delà → agg_day
```

La moyenne pondérée corrige le biais actuel des courbes et des statistiques MCP : une période
bavarde ne pèse plus plus lourd qu'une période calme. Un état se rend en segments exacts, au lieu
des N derniers relevés tronqués.

### 12.6 Valeurs calculées

- La fenêtre de fraîcheur `[obs, obs + max_age)` est bonne et reste.
- **Quorum** : un nombre minimal de sources, par exemple 2 sur 3. En dessous, la qualité devient
  incertaine, ou la valeur `Null` selon la politique.
- **Qualité exposée** : `valid` et les sources utilisées deviennent visibles sur le point, et pas
  seulement dans le journal.
- **Fonctions** : un index n'alimente jamais Min, Max ou Mean. On ajoute une `Sum` explicite de
  mesures instantanées, par exemple la puissance totale de plusieurs prises.

### 12.7 Échéancier unique

```text
un tas binaire d'échéances (instant, type, référence, génération)
une seule tâche, réveillée à l'échéance la plus proche
types : fraîcheur d'appareil, durée de condition, minuteur, planning, expiration (invitation, session)
repousser : O(log n) ; annuler : incrémenter la génération (l'ancienne entrée est ignorée)
persister seulement les échéances durables (maison.db)

au démarrage :
    recharger les échéances durables
    fraîcheur : échéance ← dernier_contact + attendu
    condition avec durée, si la valeur est déjà dans la zone :
        vraie_depuis ← dernier franchissement, lu dans les relevés récents
        reste ← for_s − (maintenant − vraie_depuis)
        si reste > 0 : programmer à maintenant + reste
        sinon : ne pas rattraper par défaut, comme les horaires manqués aujourd'hui
```

Cet échéancier remplace une tâche tokio par décompte. Il survit au redémarrage et se teste avec
une horloge injectée.

### 12.8 Export vers l'archive (à soumettre à la revue D13)

```text
dans la transaction du lot : opération compacte figée(seq, point, t, valeur), en ajout   -- ≈ 15–25 o,
                             elle tient lieu d'outbox (seq contiguë) et de journal du jour (§ 13.8)
à la formation d'un lot (5 000 opérations au plus) : rendre le NDJSON, gzip, stocker les octets du lot
envoi et réessais : toujours ces mêmes octets ; après publication, purge 7 jours plus tard, par préfixes
```

- La garantie de D13, un réessai identique au octet près, tient : les octets sont figés dès la
  formation du lot.
- L'opération est figée dès sa transaction, avec ses valeurs et non une référence à une ligne
  mutable.
- Seul le rendu change de place. L'expurgation de confidentialité doit alors s'appliquer aux
  opérations compactes et aux lots non publiés : c'est à vérifier par la revue.
- L'opération compacte, écrite en ajout dans l'ordre du temps, sert aussi de journal du jour :
  `samples` est rempli depuis elle à l'heure close, au lieu d'être écrit relevé par relevé
  (§ 13.8). Une ligne d'environ 20 octets par relevé remplace alors trois écritures : le relevé,
  l'outbox et le NDJSON.

## 13. Deux bases : l'historique à part, le fonctionnement direct à part

### 13.1 Pourquoi

| | Fonctionnement direct (`maison.db`, nom proposé) | Historique (`moli.db`, nom de D13) |
| --- | --- | --- |
| Taille | quelques Mo | des centaines de Mo |
| Écritures | petites, décidées par un humain ou un automatisme ; état courant par lots de 60 s | journal en ajout par lots de 2 s, rangement et agrégats à l'heure close, travaux de nuit |
| Lectures | ponctuelles, surtout au démarrage : le hub tient l'état en mémoire | des plages : courbes, résumés, export |
| Valeur | précieuse : c'est la maison | en partie remplaçable : l'archive garde le long terme |
| Sauvegarde | fréquente et rapide | chaque nuit |
| Panne | ne doit jamais attendre l'historique | réparable (confidentialité, purge) sans bloquer la maison |

- Un rédacteur par base : les deux rédacteurs tournent en parallèle. Une rafale de relevés ne
  retarde jamais un clic, la validation d'un automatisme ou la modification d'une personne.
- La réparation de confidentialité de D13 (DELETE, `VACUUM INTO`, remplacement atomique) ne
  réécrit que la base d'historique.
- La garde B1 garde son rédacteur unique dans la base d'historique : rien ne change pour elle.

### 13.2 Contenu

- **`maison.db`, fonctionnement direct** :
  - état courant complet de chaque point, historisé ou non : valeur canonique, observé, reçu,
    changé (§ 13.7) ;
  - appareils : dernier contact, disponibilité, rapport attendu ;
  - équipements et rôles ;
  - état métier : personnes, zones, invitations, téléphones, étiquettes, pièces, tableau de bord,
    plan, automatismes, dépendances, aides, réglages ;
  - échéances durables, journal des actions.
- **`moli.db`, historique** :
  - vocabulaire de l'historique : points (identifiant interne, `PointId`, type, nature, échelle,
    version) et dictionnaires ;
  - relevés, agrégats ;
  - énergie : `meter_state`, `energy_hour`, `energy_total` ;
  - garde de conservation B1 : planchers, journal, garde, consentements ;
  - export : opérations compactes (journal du jour, § 13.8), lots figés, `archive_ack` ; méta ;
  - progrès des travaux de nuit (§ 13.9).
- **À part**, sans changement : le coffre `secrets.enc`, chiffré, et `moli.toml`, le fichier
  d'amorçage.

### 13.3 Règle des clés : des entiers dans une base, des noms entre les bases

- Dans une base : des clés entières compactes (rowid calculé).
- Entre les deux bases : des noms stables (`PointId`, `DeviceId`), jamais des entiers internes.
- Conséquences :
  - chaque base se restaure ou se reconstruit seule ;
  - vider l'historique ne touche pas à la maison ;
  - aucune clé étrangère entre fichiers.
- `maison.db` garde ses valeurs sous forme canonique, sans code : elle ne dépend pas des
  dictionnaires de l'historique.
- **Double écriture voulue, recouvrement borné** (§ 13.7) :
  - l'état courant (une ligne par point, dans `maison.db`) et l'historique (une ligne par
    changement retenu, dans `moli.db`) ne sont pas la même donnée : ce qui est vrai maintenant,
    et ce qu'on a gardé du passé ;
  - la dernière ligne de l'historique est souvent égale à l'état courant, mais pas toujours :
    bande morte, intervalle minimal, point non historisé, lot pas encore validé ;
  - le recouvrement tient en une ligne par point, quelle que soit la durée d'historique ;
  - « reçu le » vit seulement dans `maison.db` ;
  - le vocabulaire de l'historique n'est pas une copie : il fige l'encodage des relevés, alors
    que `maison.db` porte les descriptions modifiables.
- La règle « pas de double stockage » du § 1 reste entière pour les équipements : une valeur
  physique n'est jamais recopiée dans un équipement logique (§ 9).

### 13.4 L'algorithme de bout en bout

1. **Bord** : le pilote ou le profil produit la valeur canonique (§ 12.2).
2. **Hub, en mémoire** : il met à jour « reçu le », « changé le », la qualité et l'échéancier.
3. **Rédacteur direct (`maison.db`)** :
   - une décision humaine ou d'automatisme s'écrit tout de suite, en petite transaction ;
   - l'état courant de chaque point touché (valeur, observé, reçu, changé), le dernier contact
     et la disponibilité s'écrivent par lots toutes les 60 s, et à l'arrêt (§ 13.7).
4. **Rédacteur de l'historique (`moli.db`)** :
   - filtres de politique, puis lots de 2 s : une opération compacte en ajout par relevé retenu
     (§ 12.3, § 13.8) ;
   - à chaque heure close, une transaction : l'agrégat de l'heure (§ 12.5) et le rangement de
     l'heure par point dans `samples` (§ 13.8) ;
   - la nuit : heures à vérifier, agrégats du jour, purge B1, sauvegarde (§ 13.9).
5. **Export** : lots vers l'archive Parquet (§ 12.8).
6. **Lectures** :
   - l'état courant se lit en mémoire ;
   - les courbes se lisent dans `moli.db`, sur des connexions de lecture, une par requête ou par
     fil d'exécution : une plage de `samples`, plus le journal de l'heure en cours ; le dernier
     point d'une courbe est la valeur en mémoire, si bien qu'une courbe ne retarde jamais d'un lot.
7. **Démarrage** :
   - ouvrir `maison.db`, qui est petite : maison, équipements, échéances, état courant complet ;
   - toutes les valeurs relues sont incertaines et ne déclenchent rien ; la maison repart sans
     attendre `moli.db` ;
   - en arrière-plan, la réconciliation compare chaque point au dernier relevé de `moli.db`
     (§ 13.7) ;
   - l'échéancier est reconstruit (§ 12.7).
8. **Sauvegarde** : `VACUUM INTO` des deux bases par le processus qui tourne, `maison.db` souvent
   et `moli.db` chaque nuit, plus un manifeste avec empreintes SHA-256.

### 13.5 Turso : embarqué, multithread, vecteurs

- **Embarqué** : les deux bases tournent dans le processus, sans serveur, dans un seul binaire.
  D13 a accepté le surcoût de Turso (+11,4 Mo) dans un budget d'image de 40 Mo.
- **Multithread** :
  - Les lecteurs ont chacun leur connexion. Ils ne bloquent pas et gardent leur instantané : D13
    l'a vérifié. Courbes et API tournent donc en parallèle des écritures.
  - Deux bases, ce sont deux rédacteurs réellement parallèles, sans fonction expérimentale.
  - `BEGIN CONCURRENT` (MVCC) fonctionne mais reste expérimental, et D13 ne l'utilise pas. On le
    réévaluera pour que consolidation et purge écrivent en même temps que le rédacteur des relevés,
    mais pas avant qu'il soit stable : la garde B1 repose sur un rédacteur unique.
- **Vecteurs (embeddings)** :
  - Les types `vector32`, `vector8` et `vector1bit` et les distances sont compilés par défaut.
    Turso ne calcule pas les vecteurs, et son seul index (`toy_vector_sparse_ivf`) est
    expérimental : une recherche lit tous les vecteurs du sous-ensemble filtré.
  - Les vecteurs vont dans `maison.db`, ou dans une base de l'assistant, rangés en BLOB : le
    fichier reste lisible par SQLite, comme l'exige D13.
  - Usages possibles :
    - la mémoire de l'assistant, avec les faits structurés d'abord et les vecteurs en complément,
      comme déjà décidé pour l'assistant ;
    - retrouver un appareil ou une pièce par le sens (« la lampe près du canapé ») à partir des
      libellés.
  - Jamais dans `moli.db` : les relevés n'en ont pas besoin.

### 13.6 Sauvegarde, restauration, migration

- **Sauvegarde** : deux instantanés, le coffre et un manifeste d'empreintes. Plus de liste `FILES`
  à tenir pour l'état métier : tout est dans `maison.db`.
- **Restauration** : `maison.db` seule rend une maison intacte, avec les dernières valeurs de
  chaque point, sans historique. `moli.db` seule rend les courbes ; les noms stables permettent
  de les rattacher.
- **Migration** :
  - `history.db` et `energy.db` (SQLite) passent dans `moli.db` (Turso), selon C2 et D13 ;
  - les fichiers JSON passent dans `maison.db`, domaine par domaine, importés au premier démarrage ;
  - les fichiers d'origine sont renommés et gardés 30 jours, comme la période de retour arrière
    de D13.

### 13.7 Double écriture : l'état courant complet dans `maison.db`

**Question** : la base de production garde la dernière valeur de chaque point avec tous ses
instants, l'historique s'écrit en parallèle, et la nuit consolide et purge. Est-ce intéressant ?

**Réponse : oui, à trois conditions** (plus bas). C'est déjà ce que fait Moli, sous une forme
coûteuse : `state.json` est réécrit en entier chaque minute, à côté de l'historique (§ 2). La
proposition garde l'idée et en corrige le coût et les défauts. Elle remplace aussi la règle
« jamais dans les deux » de la version précédente de ce document (§ 13.3).

| | Version précédente | Double écriture |
| --- | --- | --- |
| Valeur d'un point historisé | dernier relevé de `moli.db` | `point_state` de `maison.db`, et relevés dans `moli.db` |
| « Changé le » | lu dans le dernier relevé | colonne de `point_state` |
| Après une bande morte | le dernier relevé écrit, pas la vraie dernière valeur | la vraie dernière valeur |
| Point exclu de l'historique | cas à part | même ligne que les autres |
| Démarrage | `maison.db`, puis une descente de b-arbre par point dans `moli.db` | `maison.db` seule ; `moli.db` s'ouvre ensuite, en arrière-plan |
| `moli.db` illisible, en réparation ou en migration | dernières valeurs perdues | maison intacte |

**Le fonctionnement, situation par situation** :

| Situation | Ce qui se passe | Effet |
| --- | --- | --- |
| Rapport identique (un contact se réannonce) | mémoire : « reçu le » avance et la ligne est marquée à écrire ; `moli.db` : rien | aucune écriture immédiate |
| Changement (porte ouverte) | mémoire et événement tout de suite ; les automatismes lisent la mémoire ; relevé dans le lot de 2 s ; ligne d'état dans le lot de 60 s | aucun disque sur le chemin d'une décision |
| Petite variation (21,03 → 21,05 °C) | l'état courant prend la valeur exacte ; l'historique l'ignore (bande morte) ; l'agrégat la compte | l'historique maigrit sans fausser l'état courant |
| Rafale (jusqu'à 241 valeurs en une seconde, observé chez Home Assistant) | la mémoire absorbe ; un lot d'historique ; dix rapports d'un point = une ligne d'état | aucune base n'en ralentit une autre |
| Coupure entre deux écritures | une base a au plus un lot de retard sur l'autre | la réconciliation du démarrage prend la plus récente |
| Redémarrage | `maison.db` rend tout l'état, en qualité incertaine ; la maison repart sans attendre l'historique | démarrage indépendant de la taille de l'historique (budget ≤ 2 s) |
| Nuit | consolidation, purge et sauvegarde dans `moli.db`, par tranches (§ 13.9) | une nuit lente ne retarde aucun clic |
| Restauration | `maison.db` seule : la maison et ses dernières valeurs ; `moli.db` seule : les courbes | chaque base se suffit |
| Historique corrompu ou plein | le rédacteur d'historique s'arrête et le signale ; la maison continue | panne isolée |

Chaque rapport prend aujourd'hui le verrou d'écriture du hub
([hub.rs:2371](../../crates/moli-runtime/src/hub.rs#L2371)). Une écriture disque à cet endroit
sérialiserait tous les pilotes : les deux rédacteurs reçoivent donc leur travail par canal, comme
l'historique aujourd'hui.

**Fenêtres de perte sur coupure** :

| Donnée | Base | Écrite | Perte au plus | Rattrapage |
| --- | --- | --- | --- | --- |
| Décision (humain, automatisme), échéance durable | `maison.db` | tout de suite | rien | — |
| Relevé retenu | `moli.db` | lot de 2 s | 2 s | — |
| État courant : valeur, instants, disponibilité | `maison.db` | lot de 60 s, et à l'arrêt | 60 s | réconciliation avec `moli.db`, puis rapports suivants |
| Agrégat de l'heure en cours | mémoire | à l'heure close | l'heure en cours | recalcul depuis les relevés (§ 7.4, § 13.9) |

Les deux bases se couvrent : pour une valeur historisée, la perte réelle est celle du lot le plus
court, 2 s.

**Réconciliation au démarrage** :

```text
état ← point_state (maison.db) ; qualité ← incertaine ; aucun événement
la maison démarre sans attendre moli.db
en arrière-plan, quand moli.db est ouverte, pour chaque point historisé :
    d ← dernier relevé du point                        -- une descente de b-arbre
    si d.instant > p.changé_le :                        -- maison.db avait un lot de retard
        p.valeur ← d.valeur ; p.changé_le ← d.instant
    sinon si d.valeur ≠ p.valeur :                      -- le dernier relevé s'est perdu
        proposer p au rédacteur d'historique           -- il repasse par les filtres
en silence : aucun événement ; la qualité reste incertaine jusqu'au premier rapport frais (§ 12.2)
```

Le silence compte : une porte ouverte juste avant la coupure ne doit pas redéclencher « porte
ouverte » au redémarrage. Aujourd'hui, chaque valeur relue est réémise comme un changement, ce qui
ajoute environ un relevé en double par point à chaque redémarrage
([hub.rs:2134](../../crates/moli-runtime/src/hub.rs#L2134)).

**Trois conditions** :

1. **La mémoire reste la seule vérité vive.** Les deux bases en sont des copies datées : on ne les
   relit qu'au démarrage, jamais pour décider.
2. **L'état courant s'écrit par lots, jamais à chaque rapport.** C'est ce qui rend la double
   écriture presque gratuite (mesure ci-dessous).
3. **Aucune transaction commune aux deux fichiers.** En WAL, SQLite garantit l'atomicité dans
   chaque fichier, mais pas entre fichiers attachés : un arrêt pendant la validation peut laisser
   l'un à jour et l'autre non ([documentation d'`ATTACH`](https://www.sqlite.org/lang_attach.html)).
   Les fenêtres bornées et la réconciliation remplacent cette transaction.

**Coût mesuré** de `point_state` (1 512 points, une vraie journée ; série `2026-10-21T114647Z`,
méthode au § 13.8) :

| Écriture de l'état courant | Validations par jour | Écrit par jour |
| --- | --- | --- |
| Aujourd'hui : `state.json` entier chaque minute | 1 440 (2 fsync chacune) | ≈ 117 Mo (*estimation* : ≈ 81 Ko × 1 440) |
| `point_state`, lots de 2 s | 42 557 | ≈ 495 Mo |
| `point_state`, lots de 60 s (proposé) | 1 440 | ≈ 65 Mo |
| `point_state`, lots de 15 min | 96 | ≈ 5,9 Mo |

**Analogie** : Home Assistant fait le même découpage. Son état vit en mémoire ;
`core.restore_state` est réécrit toutes les 15 min et à l'arrêt ; l'enregistreur valide
l'historique toutes les 5 s, dans son propre fil ; la purge passe chaque nuit à 04 h 12 ; les
statistiques horaires ne sont jamais purgées automatiquement. La proposition reprend ce découpage
avec une table au lieu d'un fichier réécrit, et des instants que Home Assistant n'a pas tous.

### 13.8 Ce que coûte vraiment une écriture : des pages, pas des octets

Pour chiffrer la double écriture, il a fallu mesurer ce qu'écrit chaque base. Le résultat dépasse
la question posée.

**Le mécanisme.** SQLite et Turso écrivent le WAL par pages de 4 Kio. Une validation réécrit
chaque page qu'elle a touchée, même pour une ligne de 20 octets. Or `samples` est rangé par point
(`(point << 44) + ts`) et les relevés arrivent dans l'ordre du temps : chaque point présent dans
un lot salit sa propre page. Le coût suit le nombre de points distincts par validation, pas le
volume des données. À elle seule, la table `samples` rangée par point écrit ≈ 880 Mo par jour en
lots de 2 s : 4,9 pages par validation pour 3,9 relevés en moyenne. Des lots plus longs n'y
changent presque rien : ≈ 842 Mo en lots de 10 s, ≈ 502 Mo en lots de 60 s.

**Mesure.**

- *Série* : `2026-10-21T114647Z`, produite par
  [`tools/storage-writes/measure.py`](../../tools/storage-writes/README.md). Son
  [rapport daté](../reports/mesure-ecritures-2026-10-21T114647Z.md) donne l'environnement, les
  empreintes de l'outil et de l'entrée, tous les scénarios et les données brutes. Relancer l'outil
  écrit une nouvelle série ; les chiffres de ce document restent ceux de celle-ci.
- *Méthode* : SQLite 3.53, *proxy* du format de pages que reprend Turso ; pages de 4 Kio ; on
  compte les octets écrits dans le WAL et par les points de contrôle.
- *Charge* : une vraie journée de 164 126 relevés, rejouée sur 1 512 séries. Ce sont les comptes
  horaires réels, anonymisés, de deux installations Home Assistant, la même charge « ×1 » que le
  banc D13. Cinq jours d'historique sont pré-remplis.
- *Ordre de grandeur* : une maison entièrement passée de Home Assistant à Moli. L'installation
  Moli actuelle est bien plus petite : ≈ 2,5 Mo pour 70 appareils sur 30 jours
  ([historique-energie.md](../components/historique-energie.md)).

| Chemin d'écriture de l'historique | Validations par jour | Écrit par jour |
| --- | --- | --- |
| D13 tel que décidé : relevé + outbox + journal NDJSON + `meta`, lots ≤ 100 ms | 150 311 | ≈ 3,1 Go |
| D13 avec des lots de 2 s | 42 638 | ≈ 1,75 Go |
| Proposé, lots ≤ 100 ms, rangement la nuit | 150 313 | ≈ 659 Mo |
| **Proposé : opération compacte en ajout, lots de 2 s, rangement à l'heure close** | 42 663 | **≈ 258 Mo** |
| Proposé, lots de 2 s, rangement la nuit | 42 640 | ≈ 218 Mo |
| Proposé, lots de 10 s, rangement la nuit | 8 723 | ≈ 79 Mo |

Pour ≈ 3,4 Mo de données utiles par jour.

- **Deux leviers.**
  - *La disposition* : écrire la journée en ajout, dans l'ordre du temps, puis la ranger par point
    d'un coup. L'opération compacte du § 12.8 sert de journal : une ligne d'environ 20 octets par
    relevé remplace trois écritures (relevé, outbox, NDJSON).
  - *Le rythme* : une validation écrit au moins une page. Avec ≈ 2 relevés par seconde, des lots
    ≤ 100 ms valident presque chaque relevé seul, avec un fsync chacun en `FULL`.
- **Ranger à l'heure close plutôt que la nuit.** La nuit écrit un peu moins (218 contre 258 Mo).
  Mais une courbe du jour devrait alors balayer tout le journal du jour. Pour le point le plus
  bavard (14 136 relevés en 24 h), sur la machine de mesure :
  - une plage de `samples` : 9,8 ms ;
  - le journal du jour entier : 19,5 ms, et ce coût s'applique à chaque courbe, même pour un point
    calme ;
  - le journal d'une heure : 0,46 ms.
  Le rangement de l'heure se fait dans la même transaction que son agrégat.
- **Lots de 2 s plutôt que ≤ 100 ms.** Rien sur le chemin vif n'attend l'historique (§ 13.7). `FULL`
  reste : une séquence n'est envoyée à l'archive qu'après sa validation. Un lot plus long retarde
  l'export de 2 s au plus, sans affaiblir la garantie de D13. C'est déjà le rythme du recorder
  actuel ([lib.rs:39](../../crates/moli-history/src/lib.rs#L39)).
- **Ensemble.** Historique ≈ 258 Mo + état courant ≈ 65 Mo ≈ 0,32 Go par jour. D13 tel que décidé
  écrit ≈ 3,1 Go. La proposition écrit donc environ dix fois moins et fait 3,4 fois moins de
  validations, double écriture comprise.
- **Pourquoi c'est important.** D10 vise un Pi Zero 2 W, donc une carte microSD. De petites
  écritures synchrones y usent la carte bien plus que leur volume ne le laisse croire. Sur le
  SSD d'un mini-PC, ces volumes ne posent pas de problème d'usure ; ils restent du temps disque.
- **Contraintes relevées dans le banc D13** :
  - la déduplication `ON CONFLICT (id)` se fait au rangement ;
  - `outbox.ref` disparaît : l'export lit l'opération ;
  - la purge et son ancre ne voient que des heures déjà rangées ;
  - les lectures restent des plages de rowid, plus le journal de l'heure. Un rowid ordinaire avec
    un index `(point_id, ts)` a été mesuré 2,8 à 6 fois plus lent : on ne l'ajoute pas au journal.
- **À confirmer sur Turso et sur Pi.** Le banc D13 relève déjà la taille du WAL
  ([perf.rs:155](../../tools/storage-bench/src/perf.rs#L155)), mais aucun résumé ne la reprend, et
  la taille d'un fichier ne donne pas les octets écrits. Il faut y porter les scénarios de
  `tools/storage-writes`, qui comptent les octets écrits (§ 17).

### 13.9 La nuit : consolider, purger, sauvegarder

Principe : **ce qui a besoin de toutes les valeurs reste au fil de l'eau ; ce qui peut attendre
passe la nuit.**

| Tâche | Quand | Pourquoi |
| --- | --- | --- |
| État courant | lots de 60 s | § 13.7 |
| Relevés retenus | lots de 2 s, en ajout | § 13.8 |
| Agrégat de l'heure et rangement de l'heure | à l'heure close, en une transaction | l'agrégat voit toutes les valeurs, filtrées comprises, alors que la nuit ne verrait que les relevés écrits |
| Énergie (`meter.rs`) | au fil de l'eau | la consommation du jour s'affiche en direct |
| Export | lots de 5 000 au plus | inchangé |
| Vérifier les heures du jour clos ; recalculer celles qu'un arrêt a laissées partielles | la nuit | les relevés sont encore là |
| Agrégats du jour (`agg_day`, `state_day`) | la nuit | 24 lignes par point à lire |
| Purge B1 | la nuit, au lieu de toutes les 6 h et au démarrage | une coupe au jour près suffit ; garde B1 inchangée |
| Sauvegarde `VACUUM INTO` de `moli.db` | la nuit | aujourd'hui, la sauvegarde ne se lance qu'à la main ; `maison.db` se sauvegarde plus souvent |
| Point de contrôle `TRUNCATE` | la nuit | WAL vide pour la journée |

```text
chaque nuit à 03 h 30, heure locale (après l'heure qui saute ou se répète) :
    pour chaque tâche, dans l'ordre : heures du jour clos, agrégats du jour, purge, sauvegarde,
                                      point de contrôle
        reprendre à progrès[tâche]                      -- une ligne par tâche dans moli.db
        par tranches courtes (≤ 50 ms), écrites par le rédacteur de l'historique entre deux lots
        progrès[tâche] ← où on en est                   -- dans la transaction de la tranche
    à 06 h 00 : s'arrêter ; la tâche reprendra la nuit suivante
nuit manquée (machine arrêtée) : rattrapage au démarrage suivant, après 10 min, jour le plus ancien
                                 d'abord
```

- **Reprenable et idempotente** : recalculer un jour remplace les mêmes lignes, puisque leur
  identifiant est calculé.
- **Un seul rédacteur** : les tranches passent par le rédacteur de l'historique. La garde B1 ne
  change pas, et MVCC reste inutile.
- **Après un arrêt non propre**, B1 suspend les purges jusqu'à une validation par PIN
  ([store.rs:649](../../crates/moli-history/src/store.rs#L649)). La nuit saute alors la purge et
  l'interface le dit ; les autres tâches continuent.
- **Planification** : il n'existe aujourd'hui que des intervalles tokio, sans heure fixe. La nuit
  est une échéance de plus dans l'échéancier unique (§ 12.7).
- **Place** : la place libérée par la purge est réutilisée par les nouvelles lignes, et le fichier
  se stabilise. Le banc D13 le constate : le fichier garde sa taille après purge. Aucun `VACUUM` du
  fichier vivant n'est nécessaire en régime établi.

## 14. Changements de base proposés

Ces changements visent la cible D13. Les décider **avant l'intégration de C2** évite une migration
de données réelles.

### 14.1 Un relevé amaigri (`moli.db`)

```sql
CREATE TABLE samples (
    id  INTEGER PRIMARY KEY,  -- (point << 44) + ts en ms, comme D13 : courbe et purge = plages
    seq INTEGER NOT NULL,     -- à garder seulement si l'export l'exige (question ouverte)
    num ANY,                  -- 0/1, code d'énumération, entier mis à l'échelle ou réel
    txt TEXT,                 -- texte seulement
    q   INTEGER               -- NULL normalement (1 octet d'en-tête) ; marqueur rare : texte tronqué
) STRICT;
```

- `samples` n'est plus écrit relevé par relevé : il est rempli à l'heure close, trié par point,
  depuis le journal du jour (§ 13.8, § 14.6).
- `point_id` et `ts` sont retirés : ils sont déjà dans la clé calculée (`id >> 44`,
  `id & (2^44 − 1)`).
- `kind` est retiré : il est dans le vocabulaire (§ 14.2).
- `num ANY` garde les entiers entiers : SQLite écrit 0 et 1 sans octet de valeur, un petit entier
  sur 1 à 3 octets, alors qu'un réel coûte toujours 8 octets. Il faut vérifier au banc que Turso
  en fait autant.

| Relevé | Octets (*estimation*, pointeur de cellule compris) |
| --- | --- |
| `history.db` actuel (`WITHOUT ROWID`) | ≈ 26 |
| Cible D13 actuelle, décimal | ≈ 39 |
| Proposé, décimal mis à l'échelle (2 octets) | ≈ 22 |
| Proposé, binaire ou énumération | ≈ 20 à 21 |

Au banc ×2, la cible D13 sans journal d'export pèse 151,74 Mo pour 2,32 millions de relevés et
127 452 heures d'énergie. Le relevé amaigri réduit la part des relevés d'environ 45 %, à confirmer
au banc.

### 14.2 Le vocabulaire de l'historique (`moli.db`)

```sql
CREATE TABLE catalog (            -- étend le catalog de D13
    id      INTEGER PRIMARY KEY,
    point   TEXT NOT NULL UNIQUE, -- PointId : le nom stable partagé avec maison.db
    type    INTEGER NOT NULL,     -- booléen, énumération, entier, décimal, texte, instant
    nature  INTEGER NOT NULL,     -- mesure, index, index à remise, résumé, occurrences (§ 7.5)
    scale   INTEGER,              -- puissance de 10 : -2 = centièmes ; fixe pour une version
    dict_id INTEGER,              -- dictionnaire de l'énumération
    family  TEXT NOT NULL,
    unit    TEXT,
    version INTEGER NOT NULL DEFAULT 1
) STRICT;
CREATE TABLE dict_entry (
    id   INTEGER PRIMARY KEY,     -- (dict << 16) + code : décoder = une recherche de rowid
    dict INTEGER NOT NULL,
    key  TEXT NOT NULL,
    seen INTEGER NOT NULL,        -- première apparition, pour l'audit
    UNIQUE (dict, key)
) STRICT;
```

Les dictionnaires sont chargés en mémoire au démarrage. L'encodage à l'écriture se fait en
mémoire ; l'index d'unicité ne sert qu'à l'ajout d'une nouvelle clé.

### 14.3 Les agrégats (`moli.db`)

```sql
CREATE TABLE agg_hour (
    id      INTEGER PRIMARY KEY,      -- (point << 24) + heures depuis 1970 : une courbe = une plage
    mean    ANY, min ANY, max ANY,    -- mesures (moyenne pondérée par le temps)
    delta   ANY,                      -- index (§ 7.5)
    covered INTEGER NOT NULL          -- secondes couvertes par des données fraîches
) STRICT;
CREATE TABLE state_hour (
    id      INTEGER PRIMARY KEY,      -- (point << 32) + (heure << 8) + code
    ms      INTEGER NOT NULL,         -- durée passée dans l'état
    entries INTEGER NOT NULL          -- transitions vers cet état
) STRICT;
```

Les tables `agg_day` et `state_day` suivent le même modèle, au jour local. L'énergie garde
`energy_hour` et `energy_total` de D13.

### 14.4 La base de fonctionnement direct (`maison.db`)

```sql
CREATE TABLE point_state (            -- état courant complet, historisé ou non (§ 13.7)
    id          INTEGER PRIMARY KEY,
    point       TEXT NOT NULL UNIQUE,  -- PointId
    num         ANY,                   -- valeur canonique, sans code de dictionnaire
    txt         TEXT,
    observed_at INTEGER,               -- instant de la mesure selon la source (§ 5)
    received_at INTEGER NOT NULL,      -- dernier rapport
    changed_at  INTEGER                -- dernier changement
) STRICT;
CREATE TABLE device (
    id           INTEGER PRIMARY KEY,
    device       TEXT NOT NULL UNIQUE, -- DeviceId
    integration  TEXT NOT NULL,        -- InstanceId
    expected_ms  INTEGER,              -- rapport attendu (§ 8.1)
    last_contact INTEGER,              -- persisté par lots
    online       INTEGER               -- dernière disponibilité connue, aujourd'hui perdue au redémarrage
) STRICT;
CREATE TABLE equipment (
    id      INTEGER PRIMARY KEY,
    name    TEXT NOT NULL,
    kind    TEXT NOT NULL,             -- portail, chauffe-eau…
    room_id INTEGER
) STRICT;
CREATE TABLE equipment_role (
    id        INTEGER PRIMARY KEY,     -- (equipment << 8) + rôle
    point     TEXT NOT NULL,           -- PointId de la source, jamais une copie
    transform TEXT                     -- inversion, correspondance de codes, échelle
) STRICT;
CREATE TABLE deadline (
    id         INTEGER PRIMARY KEY,
    kind       INTEGER NOT NULL,       -- minuteur, planning, expiration, condition (§ 12.7)
    ref        TEXT NOT NULL,
    at         INTEGER NOT NULL,       -- échéance UTC absolue
    generation INTEGER NOT NULL
) STRICT;
```

`point_state` remplace `state.json`, aujourd'hui entièrement réécrit chaque minute. Chaque point a
sa ligne complète, qu'il soit historisé ou non : `maison.db` suffit au démarrage, et `moli.db` ne
sert qu'à la réconciliation, en arrière-plan (§ 13.7). La qualité n'est pas stockée : après un
redémarrage, elle est incertaine jusqu'au premier rapport.

### 14.5 Les politiques

Recommandé : étendre `[storage]` de `moli.toml`, déjà validé par B1, pour garder une seule
autorité. On y ajoute bande morte, intervalle minimal, anti-rebond, rapport attendu, validité de la
dernière valeur et consolidation, avec des défauts par famille et des surcharges par point, comme
les durées aujourd'hui. Une table ne se justifie que si l'interface doit les éditer. Baisser une
durée garde la validation humaine B1.

### 14.6 Un journal d'export compact (à soumettre à la revue D13)

- Au banc ×2, le contenu immuable du journal d'export ajoute 313,4 Mo pour 2,45 millions
  d'opérations, soit ≈ 128 octets par opération : du NDJSON avec le nom complet du point à chaque
  ligne.
- Proposition (algorithme au § 12.8) : une opération compacte figée dans la transaction métier,
  puis le lot rendu une fois et figé à sa formation. Le protocole de l'archive (NDJSON gzip,
  opérations nommées) ne change pas.
- *Estimation* : 15 à 25 octets par opération en attente, soit 80 à 90 % de moins sur ce poste.
- En régime établi, le journal ne garde que 7 jours après publication. Le gain de place compte
  surtout quand l'archive est injoignable.
- L'opération compacte sert aussi de journal du jour (§ 13.8) : c'est elle qui économise le plus
  d'écritures, bien plus que de place.

```sql
CREATE TABLE ops (                 -- opération compacte figée : export, journal du jour, rangement
    seq   INTEGER PRIMARY KEY,     -- séquence d'export contiguë ; ajout en fin de b-arbre
    kind  INTEGER NOT NULL,        -- relevé ou heure d'énergie, comme l'outbox de D13
    point INTEGER NOT NULL,        -- catalog.id (ou compteur pour une heure d'énergie)
    ts    INTEGER NOT NULL,
    num   ANY,                     -- valeur, ou kWh d'une heure d'énergie
    txt   TEXT,
    cost  ANY                      -- coût d'une heure d'énergie ; NULL (1 octet) pour un relevé
) STRICT;
CREATE TABLE progress (            -- reprise des travaux de nuit et du rangement (§ 13.9)
    task INTEGER PRIMARY KEY,
    upto INTEGER NOT NULL          -- dernière séquence rangée, dernier jour consolidé…
) STRICT;
```

- Le rangement de l'heure lit `ops` par plage de `seq`, trie par point et insère dans `samples`
  avec `ON CONFLICT DO NOTHING`, dans la même transaction que l'agrégat de l'heure.
- Une ligne de `ops` n'est supprimée qu'une fois rangée, publiée, puis gardée 7 jours, par
  préfixes de `seq`, comme le journal de D13.
- `ops` remplace l'`outbox` et l'`export_journal` de D13 ; `archive_ack` et `meta` restent.
- La séquence suivante se lit à la fin de `ops`. `meta.next_seq` ne s'écrit donc qu'avant une
  purge qui viderait le journal, et non à chaque lot comme dans le banc D13 : une page de moins
  par validation. Une séquence n'est jamais réattribuée, comme l'exige D13.

### 14.7 L'état métier en tables (`maison.db`)

- Les fichiers JSON d'état se regroupent en six formes, chacune avec ses tables :
  - registres : personnes, téléphones, étiquettes, Tuya, profils approuvés ;
  - documents validés en bloc : plan, tableau de bord ;
  - définitions versionnées : automatismes, dépendances ;
  - états d'exécution : compteurs, minuteurs, dates, occurrences ;
  - journaux bornés ;
  - réglages.
- Certaines entités sont aujourd'hui recopiées sans lien et deviennent des tables pivots :
  - **pièce** : `room` dans `labels.toml`, disposition dans `home.json`, géométrie dans
    `plan.json` ;
  - **personne** : `people.json`, `personnes:<id>` dans `phones.json`, mots de passe et sessions
    dans le coffre.
- Gains :
  - une écriture = une ligne, au lieu de réécrire tout le fichier avec fsync ;
  - des transactions entre domaines ;
  - une sauvegarde complète par construction, en un instantané, sans liste à tenir.
- Le coffre reste chiffré et séparé.

### 14.8 Synthèse

| Changement | Vitesse | Place | Coût, risque | Mesure à faire |
| --- | --- | --- | --- | --- |
| Deux bases | un clic n'attend jamais une rafale de relevés | — | deux rédacteurs, règle des noms stables | p95 d'un clic pendant une rafale, avec et sans séparation |
| Relevé amaigri | moins de pages par courbe | ≈ −45 % par relevé (*estimation*) | faible avant l'intégration de C2 | banc ×2 : taille, p95 d'une courbe de 7 j |
| Bande morte et intervalle au stockage | moins d'écritures | fort sur les capteurs bavards | réglage par famille | part des points qui changent plus d'une fois par minute |
| Agrégats au fil de l'eau | courbe de 30 j ou d'un an en quelques centaines de lignes | détail plus court possible | écriture horaire, recalcul après panne | p95 d'une courbe de 30 j |
| Moyennes pondérées par le temps | — | — | corrige un biais actuel | comparer sur une journée réelle |
| Index généralisés (§ 7.5) | totaux sans relecture | — | réutilise `meter.rs` | cas : remise, trou, changement de compteur |
| Dictionnaires | comparaisons entières | 1 octet par état | ajout seul | — |
| État courant en table, double écriture | démarrage sans attendre l'historique | +1 ligne par point ; ≈ 65 Mo écrits par jour (mesuré) contre ≈ 117 pour `state.json` (*estimation*) | réconciliation silencieuse au démarrage | coupure entre deux lots de chaque base |
| Journal du jour, rangé à l'heure close | courbe = une plage + une heure de journal | ≈ 3,1 Go → ≈ 0,26 Go écrits par jour (mesuré, *proxy* SQLite) | revue D13 ; déduplication au rangement | octets écrits sur Turso, puis sur Pi Zero 2 W |
| Lots de 2 s au lieu de ≤ 100 ms | — | 3,5 fois moins de validations et de fsync | export retardé de 2 s au plus | idem |
| Travaux de nuit | journée plus calme | — | reprise, rattrapage, purge suspendue après un arrêt non propre | durée d'une nuit à charge ×2 |
| Échéancier unique | aucun balayage, une seule tâche | — | regroupe l'existant | reprise après redémarrage |
| Équipements par liaisons | lecture en mémoire | zéro copie | généralise D12 | trois équipements sur papier |
| Journal d'export compact | — | −80 à −90 % du poste (*estimation*) | revue D13 | banc ×2 |
| État métier en tables | une ligne au lieu d'un fichier | — | migration par domaine | — |

## 15. Ce qu'on ne fait pas

Exercice d'inversion : comment rater cette consolidation à coup sûr ? Chaque réponse devient un
garde-fou.

- Stocker un libellé, une unité d'affichage ou une traduction.
- Recopier la valeur d'un appareil physique dans un équipement logique.
- Moyenner un index, ou calculer un mois par « index de fin − index de début ».
- Éclaircir l'historique au hasard pour gagner de la place.
- Appliquer bande morte ou intervalle à l'entrée, et cacher un seuil à un automatisme.
- Traiter une erreur de lecture comme une absence, ce qui écrase des données.
- Relier les deux bases par des entiers internes.
- Activer MVCC avant qu'il soit stable, au détriment de la garde B1.
- Chercher une transaction commune aux deux bases : en WAL, elle n'est pas atomique.
- Écrire l'état courant à chaque rapport, ou le relire pour décider.
- Réémettre une valeur relue au démarrage comme un changement.
- Écrire chaque relevé directement à sa place par point, lot après lot.
- Faire la nuit en une seule grosse transaction, qui bloquerait le rédacteur.
- Une table clé/valeur universelle.
- Un secret en clair dans une table.
- Toucher au code avant que ces choix soient tranchés.

## 16. Décisions à prendre

1. Anti-rebond : seulement là où un rebond est prouvé (proposé) ou par défaut sur tous les
   contacts ?
2. `seq` dans `samples` : nécessaire à l'export et à l'expurgation, ou supprimable ?
3. Journal d'export compact : le présenter à la revue D13 ?
4. Politiques : dans `[storage]` de `moli.toml` (proposé) ou dans la base ?
5. Deux bases : noms définitifs (`maison.db` proposé), frontière exacte, et place des vecteurs
   (`maison.db` ou une base de l'assistant) ?
6. Reprise d'une condition avec durée après redémarrage : reprendre le temps restant sans jamais
   rattraper (proposé), ou rattraper ?
7. Équipements : `[[equipment]]` à l'image de D12 d'abord (proposé), puis édition validée par PIN ?
8. Notes scolaires : les faire entrer dans le périmètre Pronote ? Garder aussi les moyennes
   officielles ?
9. Sous-compteurs : qui déclare la relation « inclus dans », et où ?
10. MVCC (`BEGIN CONCURRENT`) : quels critères pour l'adopter un jour ?
11. Double écriture de l'état courant dans `maison.db` : l'adopter (proposé) ? Avec des lots de
    60 s (proposé, comme aujourd'hui) ou de 15 min comme Home Assistant (≈ 5,9 Mo par jour) ?
12. Rythme des lots de l'historique : 2 s comme le recorder actuel (proposé), ou ≤ 100 ms comme
    écrit dans D13 ?
13. Journal du jour en ajout, rangé à l'heure close (proposé) ou la nuit ? À présenter à la revue
    D13 avec l'export compact (décision 3).
14. Nuit : fenêtre 03 h 30 – 06 h 00 (proposé) ? Purge B1 déplacée de « toutes les 6 h » à la
    nuit ?

## 17. Expériences proposées, sans toucher au code de production

- **Banc `tools/storage-bench`** :
  - variante « relevé amaigri » sur la charge ×2 : taille du fichier, débit d'écriture, p95 d'une
    courbe de 7 j ; vérifier aussi que Turso écrit 0 et 1 sans octet de valeur et garde les
    entiers en `ANY` ;
  - variante « deux bases » : une rafale de relevés pendant des écritures métier ; on mesure le
    p95 d'une écriture métier avec et sans séparation ;
  - **octets écrits par jour** : porter les scénarios de `tools/storage-writes` (chemin D13
    actuel, lots de 2 s, journal compact rangé à l'heure close) dans ce banc, sur Turso, en
    comptant les octets écrits et non la taille du WAL.
- **`tools/storage-writes` sur le matériel cible** : l'outil n'a besoin que de Python. Le relancer
  sur un Pi Zero 2 W avec sa carte microSD donne une nouvelle série datée, à comparer à
  `2026-10-21T114647Z`.
- **Coupure** : `kill -9` entre deux lots de chaque base, puis vérifier la réconciliation : la
  valeur la plus récente gagne, aucun événement n'est émis, aucun relevé n'est dupliqué.
- **Nuit** : durée de chaque tâche à charge ×2, et rattrapage de trois nuits manquées.
- **Sur un export réel anonymisé** : la part des points qui changent plus d'une fois par minute, et
  le volume de la position de lecture des médias. Cela donne le gain réel de la bande morte et de
  l'exclusion.
- **Exercice papier n° 1** : faire passer dix points réels dans les quatre états et la table des
  politiques : contact Zigbee, contact Tuya, Linky (index et puissance), thermostat en énumération,
  luminosité, titre en cours.
- **Exercice papier n° 2** : décrire trois équipements en rôles (portail, chauffe-eau, voiture) et
  vérifier chaque mode dégradé.
- **Exercice papier n° 3** : rejouer une journée de compteur du jour avec une remise à minuit et un
  trou de deux heures. Le total jour = somme des heures doit être exact.

## 18. Méthode

- Premiers principes : ce que chaque valeur doit permettre (lire, décider, consolider), avant le
  moteur de stockage.
- Analyse morphologique : pour chaque décision, les options et la proposition retenue (tableaux
  ci-dessus).
- Analogies :
  - historiens industriels : OPC UA distingue qualité bonne, incertaine ou mauvaise, et horodatage
    source ou serveur ; PI fait exception puis compression ;
  - Zigbee2MQTT : disponibilité à 10 min sur secteur et 25 h sur pile ;
  - Prometheus : clause `for:` d'une règle d'alerte ;
  - RRDtool : fonctions de consolidation par archive ;
  - Home Assistant : `last_changed`, `last_updated` et `last_reported`, registres en JSON et
    historique en SQLite ; état en mémoire, `core.restore_state` toutes les 15 min, enregistreur
    validé toutes les 5 s, purge chaque nuit à 04 h 12, statistiques horaires jamais purgées ;
  - arbres LSM : écrire en ajout, ranger plus tard d'un seul geste.
- Parcours de scénarios : rapport, rafale, coupure, redémarrage, nuit, restauration (§ 13.7).
- Mesure sur une journée réelle plutôt qu'une estimation, dès que la question en dépend (§ 13.8) :
  un outil versionné (`tools/storage-writes`), une série datée, un rapport généré
  ([`2026-10-21T114647Z`](../reports/mesure-ecritures-2026-10-21T114647Z.md)).
- Inversion : comment rater (§ 15).
- Convergence : vitesse, place, risque et mesure à faire (§ 14.8).
