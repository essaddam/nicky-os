# Énumérations : conversion API → MQTT et coût mémoire

Date : 27 octobre 2026. Statut : analyse initiale conservée, puis contrat compact implémenté dans le cœur et le pont du POC.

## Mise à jour après implémentation

Le [dossier HTML V6](analyse/v6/index.html) et ses [mesures](analyse/v6/mesures.json)
font foi pour le résultat courant. `Value::EnumCode(u8)`, `Kind::EnumCode`, les
tables partagées et versionnées du pont, leur cache privé et la résolution côté
client sont implémentés et testés. L'ancien `Kind::Enum` reste compatible avec
les pilotes historiques : tous les pilotes natifs ne sont pas migrés. Les
commandes HA manquantes et la projection complète de la seconde source restent
à raccorder.

La mesure des 96 listes donne **43 catalogues partagés et 8 014 octets
d'allocations demandées**, Arc compris, après libération des temporaires, hors
RSS, JSON et index. Les 6 Kio ci-dessous sont l'ancienne estimation de layout.
Les codes livrés vont de **0 à 255** ; l'indisponible est `null` avec qualité
explicite, jamais un code réservé. `Value` reste à 24 octets et `Sample` à 32 sur
la cible mesurée ; un sample complet ne coûte donc pas un seul octet.

Les anciennes lettres A/B de l'inventaire provenaient du tri des fichiers.
Alignées sur les brokers du POC, **A contient 65 listes / 259 choix / 1 644
octets UTF-8** ; **B contient 31 listes / 159 choix / 1 193 octets**. Les totaux
restent 96 / 418 / 2 837. Après conversion, 494 valeurs texte et 5 260 octets
UTF-8 restent dans la projection hors ligne. Les titres médias ne constituent
pas un catalogue enum.

Les sections suivantes documentent le raisonnement **avant implémentation**.
En particulier, « code actuel » ci-dessous décrit le constat initial, pas le
code livré depuis. Les exemples JSON restent illustratifs ; le contrat livré
est décrit dans le [README de la sonde](../../tools/mqtt-live-probe/README.md#catalogues-enum-et-états-codés).

Orientation demandée : les listes d’énumération et leurs libellés font partie de
la description initiale, hors mémoire permanente des valeurs du cœur. L’état
courant conserve un code sélectionné. La conversion dans les plugins métier
API → MQTT est la proposition étudiée ici ; son raccordement reste à réaliser.

## 1. Reconnaître une énumération avant de conserver du texte

Une chaîne reçue par une API n’est pas nécessairement du texte libre. Si la
source déclare un ensemble de choix, sa valeur est une sélection dans cet
ensemble. Le schéma doit être examiné **avant** une conversion numérique :
`"1"` peut désigner une option, sans être une mesure entière.

| Donnée | Source du domaine de choix | Représentation cible |
|---|---|---|
| `select`, `input_select` HA | Attribut `options` | Code d’option |
| Capteur HA de classe `enum` | Classe déclarée et attribut `options` | Code d’état |
| Mode de chauffe-eau | `operation_list`, mode courant | Code de mode |
| Modes de climatisation | `hvac_modes`, autres listes de modes lorsqu’elles sont déclarées | Code par sélection |
| Application ou source TV | Catalogue d’applications, identifiants natifs et noms ; `source_list` si disponible | Code d’application ou de source |
| Titre, message ou saisie | Aucun catalogue de choix déclaré | Texte borné, selon un contrat distinct |

Un petit nombre de textes observés ne prouve pas une énumération fermée.
Inversement, une valeur indisponible ne supprime pas le type déclaré par la source.
Les identifiants d’applications et les libellés d’affichage restent distincts.

## 2. Ce que fait le code actuel

- Le [modèle Ariston](../../crates/moli-ariston/src/model.rs) définit déjà deux
  modes : `manuel` ↔ valeur native `1`, `programme` ↔ valeur native `5`.
  Le point est déclaré `Kind::Enum`.
- Le [pont HA du POC](../../tools/mqtt-live-probe/src/house.rs) conserve
  `source_list` dans la description sous `catalogue_sources`, mais ignore notamment
  `options`, `operation_list` et `hvac_modes`. Les sélections restent des chaînes JSON.
- La [projection vers le Hub](../../tools/mqtt-live-probe/src/hub_projection.rs)
  convertit les chaînes restantes en `Value::Text` et leur attribue `Kind::Text`.
- Dans le cœur, [Kind::Enum](../../crates/moli-core/src/spec.rs) contient une
  liste de chaînes et valide encore une sélection sous forme de `Value::Text`.
  [Value](../../crates/moli-core/src/value.rs) ne possède pas de variante dédiée
  au code enum.

Il faut donc distinguer le typage métier déjà présent dans certains pilotes de
la représentation compacte proposée. Déclarer `Kind::Enum` aujourd’hui ne retire
pas automatiquement les chaînes de la mémoire du cœur.

## 3. Répartir les responsabilités

```text
API native                         Plugin métier API → MQTT
valeur « manuel » ou nombre 1  ───► conversion déterministe vers le code 1
                                               │
                           description initiale │ état différentiel
                           codes et libellés    │ code sélectionné
                                               ▼
                                        cœur et clients
```

| Emplacement | Données nécessaires |
|---|---|
| Plugin métier | Correspondance valeur native ↔ code, catalogue pris en charge, validation des entrées |
| Description initiale | Identité du catalogue, révision, codes et libellés ; capacités de sélection déclarées |
| Cœur | Code courant, qualité et référence compacte au contrat applicable ; métadonnées nécessaires aux droits et à la validation |
| Client d’affichage | Correspondance code ↔ libellé pour les catalogues qu’il utilise |

Le plugin transforme chaque observation native en code. Dans l’autre sens, un
ordre autorisé portant un code est traduit vers la valeur attendue par l’API.
Cette traduction ne contourne ni le Hub, ni les droits, ni la garde. Un état
observé ne constitue pas, à lui seul, une capacité de commande.

**L’envoi initial n’élimine pas la table de conversion.** Elle reste accessible
au plugin pour traiter les prochaines observations et commandes. Si sa
bibliothèque possède déjà le catalogue, réutiliser cette structure peut éviter
une seconde copie ; ce partage doit être vérifié dans chaque intégration.

Une table fixe peut être compilée dans les données en lecture seule du binaire.
Une table dynamique, par exemple les applications installées sur une TV, demande
un stockage actualisable. Une représentation froide peut réduire la mémoire
permanente, au prix de lectures lors des conversions. Le coût du plugin reste à
compter dans le budget du processus s’il tourne avec le cœur.

## 4. Description initiale et changements d’état

Exemple illustratif, **pas un nouveau schéma MQTT livré**. Les identifiants sont
fictifs. Les libellés produits par Moli sont traduits ; un nom d’application
fourni par un éditeur reste une donnée du catalogue.

Description :

```json
{
  "enum_id": "mode-chauffe-eau",
  "revision": 1,
  "choix": [
    {"code": 1, "libelle": {"fr": "Manuel", "en": "Manual"}},
    {"code": 2, "libelle": {"fr": "Programmé", "en": "Scheduled"}}
  ]
}
```

État différentiel, sans recopier les libellés :

```json
{
  "canal": "chauffe-eau-mode",
  "enum_id": "mode-chauffe-eau",
  "revision": 1,
  "valeur": 1,
  "qualite": "observe"
}
```

La référence et la révision peuvent être portées par une enveloppe commune si
le contrat permet de les retrouver sans ambiguïté. La version exacte du protocole
et la taille de ces références restent à fixer. Un nombre JSON tel que `1` n’est
pas nécessairement un seul octet sur le transport ni dans une structure Rust.

## 5. Inventaire et estimation mémoire

Les deux instantanés HA privés analysés contiennent les listes `options`
suivantes. Seuls des comptages et des longueurs agrégées sont publiés.

| Mesure | Source A | Source B | Total |
|---|---:|---:|---:|
| Listes `options` | 31 | 65 | 96 |
| Choix, sans déduplication entre listes | 159 | 259 | 418 |
| Octets UTF-8 des chaînes | 1 193 | 1 644 | 2 837 |
| Taille maximale d’une liste | 37 | 12 | 37 |

Ces 96 entités comprennent 77 `select`, 4 `input_select` et 15 capteurs de classe
`enum`. Le [classificateur précédent](analyse/inventorier-ha-memoire.py) les
répartit en 41 `enum_candidate`, 15 textes, 5 entiers et 35 valeurs inconnues.
Les 61 sélections connues appartiennent toutes à leur liste déclarée. Les 35
inconnues gardent leur contrat enum sans devenir une option « inconnu ».

### Table compacte estimée à 6 Kio

Hypothèse de calcul : 8 octets par liste ; par choix, un offset `u16`, une longueur
`u16`, les octets UTF-8 et un octet terminal ; chaque entrée est alignée sur
4 octets. Le code découle d’un emplacement stable dans la table. Les offsets
`u16` supposent un bloc adressé de moins de 64 Kio ; au-delà, changer de format.

```text
taille = 8 × nombre_de_listes
       + somme(alignement_4(4 + longueur_UTF8_du_choix + 1))

Source A : 2 436 octets
Source B : 3 716 octets
Total    : 6 152 octets = environ 6,0 Kio
```

C’est le coût théorique de ce rangement des chaînes natives observées, avec
capacité exacte et stockage regroupé. Il ne comprend pas les libellés traduits
supplémentaires, un index de recherche éventuel, l’allocateur, les buffers réseau,
la sérialisation JSON, les copies de bibliothèques ou la mémoire des clients.
Ce n’est ni une mesure RSS, ni une économie démontrée sur le programme actuel.

À titre de comparaison, un rangement en `Vec<Vec<String>>` sur une cible 64 bits
avec des en-têtes de 24 octets représenterait déjà **15 173 octets**, chaînes
comprises, avant surcoûts d’allocation et capacités inutilisées.

Les listes observées permettent un code `u8`, en réservant zéro et en attribuant
les codes 1 à 255. Les 96 valeurs sélectionnées représenteraient donc 96 octets
de charge utile, hors fiches, références, qualités, dates et alignement. Une
liste dépassant 255 choix demanderait un code plus large ou un refus explicite.

### Périmètre de l’estimation

Les 6 Kio ne couvrent que les 96 listes `options` relevées. Les catalogues TV,
les modes de chauffe-eau, les autres listes de capacités et leurs traductions
restent à inventorier séparément. Les exports sont expurgés et peuvent raccourcir
certaines chaînes. Le coût final dépendra des catalogues réels et du partage
effectif dans les plugins.

Les listes sortent de la mémoire permanente **des valeurs du cœur** ; leur
stockage dans le plugin, le serveur de descriptions, le broker et les clients
ne disparaît pas. Les pics de chargement et de sérialisation restent également
dans le budget mémoire global.

## 6. Catalogues dynamiques et valeurs inconnues

Une application nouvellement installée peut apparaître après l’initialisation.
Un catalogue dynamique reste une énumération à une révision donnée, avec une
mise à jour explicite de son domaine de choix.

1. Le plugin construit et valide la nouvelle table dans des bornes déclarées.
2. Les codes existants gardent leur signification ; aucun tri par nom ne les
   réattribue. Un retrait ne permet pas de réutiliser silencieusement un code.
3. Il publie la nouvelle description avant les états utilisant les nouveaux codes.
   Comme un client peut recevoir un état avant sa description, la référence de
   révision reste nécessaire ; l’attente côté client doit être bornée.
4. Un code sans description connue n’est ni affiché comme une autre option, ni
   interprété comme une commande valide. Une commande fondée sur un contrat
   périmé est refusée si sa signification n’est plus garantie.
5. Les correspondances sont conservées au redémarrage, ou un changement de
   génération invalide explicitement les anciennes références, états et ordres.

Une valeur native non reconnue ne crée pas automatiquement un texte permanent
dans le cœur et ne fait pas grossir le catalogue sans limite. Le plugin demande
une actualisation si la source la permet ; entre-temps, il publie une qualité
inconnue avec une valeur absente. Une indisponibilité de la source reste distincte
d’une valeur hors catalogue, même si toutes deux interdisent une sélection inventée.

## 7. Points à vérifier avant réalisation

- Reconnaître les schémas enum avant les heuristiques de texte ou de nombre,
  y compris lorsque l’observation vaut `unknown` ou `unavailable`.
- Raccorder les listes HA au descripteur initial et introduire une représentation
  de sélection compacte jusqu’au cœur, aux règles et à l’historique.
- Vérifier la conversion aller-retour de chaque plugin, l’identité des applications,
  les révisions, les retraits, le redémarrage et les valeurs hors catalogue.
- Mesurer les allocations permanentes et transitoires, le RSS et le temps de
  conversion du chemin complet. La table de 6 Kio ne justifie pas, seule, un
  budget de mémoire ou de performance pour l’intégration entière.

## Sources

- Code Moli et POC : fichiers liés dans les sections 2 et 5. Aucun pilote n’a été
  modifié et aucune commande physique n’a servi à l’inventaire.
- [HA : chauffe-eau](https://github.com/home-assistant/core/blob/dev/homeassistant/components/water_heater/entity.py)
  expose `operation_list` et le mode courant ; la sélection est validée contre la liste.
- [HA : Apple TV](https://github.com/home-assistant/core/blob/dev/homeassistant/components/apple_tv/media_player.py)
  construit une table noms ↔ identifiants d’applications et en expose les sources.
- [HA : Android TV Remote](https://github.com/home-assistant/core/blob/dev/homeassistant/components/androidtv_remote/media_player.py)
  rapproche l’application courante d’un catalogue, tout en acceptant un identifiant
  observé absent de celui-ci. Ce catalogue peut venir des
  [options de l’intégration](https://github.com/home-assistant/core/blob/dev/homeassistant/components/androidtv_remote/entity.py) ;
  il ne prouve pas une découverte exhaustive des applications installées.

Les liens HA visent la branche évolutive `dev`, consultée le 27 octobre 2026 ;
ils décrivent le mécanisme amont, pas la version installée d’une maison.
