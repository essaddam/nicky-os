# Revue Codex — les ordres des appareils

Revue de [l'analyse des ordres des appareils](ordres-appareils.md), faite le 24 octobre 2026 par Codex (`gpt-6.1-sol`, effort `max`, session `01a16d7c-2e5b-7a22-8924-cdf2f5f8b84b`), en lecture seule : aucun fichier du dépôt modifié. Texte de Codex, non retouché.

## Verdict

L’idée tient : séparer les valeurs, les capacités de commande et les demandes en cours clarifie le modèle.
La proposition ne peut toutefois pas être validée en l’état : certaines garanties d’idempotence, de sécurité et de reprise sont fausses.
Les quatre choix du § 20 restent conservables, avec un contrat MQTT et des transitions plus précis.
Le banc démontre plusieurs mécanismes utiles ; ses comparaisons ne prouvent pas encore les gains annoncés à fonctionnalités égales.
Je commencerais par le catalogue des ordres et leur suivi, en réutilisant les autorisations et les coordinateurs existants.

## Périmètre, hypothèses et vérification

Référence du code : `a835893`, également présent en tête du dépôt lors de la revue. Lecture de la proposition Markdown, du banc, des fichiers qu’il inclut et des passages nécessaires des quatre propositions liées. Aucun fichier source modifié.

Hypothèses retenues : plusieurs demandes peuvent viser simultanément le même ordre ; une redéclaration peut survenir pendant une demande ; une composition peut contenir plusieurs ordres et des équipements virtuels. Si certaines possibilités doivent être interdites, leur refus explicite fait partie du contrat.

Exécutions réalisées, hors ligne :

- `cargo test --release --offline --manifest-path tools/ordres-banc/Cargo.toml` : **28 réussites, aucun échec, un test ignoré** ;
- sections `catalogue 1`, `clients 1`, `demandes 100` et `soiree` : terminées ;
- nombres fonctionnels reproduits, notamment les **300 000 demandes**, **121 demandes simultanées**, **42 envois de la soirée** et **3 344 validations identiques**.

Ces exécutions natives ne reproduisent pas les chronométrages Linux musl publiés. Les contre-exemples ci-dessous sont des lectures de code et des scénarios déduits de ses transitions ; aucun nouveau test n’a été ajouté.

## Constats bloquants

### B1 — Une ACL sur `req/<client>` n’autorise pas la cible contenue dans la demande

**§§ 9.3, 14.2–14.4.** L’ordre cible est dans la charge, tandis que les ACL décrites portent sur les sujets. Le contrat doit également expliquer comment le hub obtient une identité authentifiée : accepter `origine` ou `qui` déclarés par le client permettrait de contourner la garde.

**Preuve.** `Req` contient directement `origine` et `qui` dans `tools/ordres-banc/src/hub.rs:118`. `demander` vérifie l’existence, la forme et une garde dépendant de `r.origine`, sans autorisation invitée sur la cible (`hub.rs:453`, `hub.rs:478`). Un invité autorisé à publier sur son sujet peut donc y nommer une serrure située hors de son périmètre. Le code actuel contrôle cette portée séparément : `crates/moli-api/src/rest.rs:119`.

**Correction.** Attribuer chaque sujet de demande à un principal côté serveur ; en déduire l’origine ; autoriser chaque cible dans le hub, filles comprises. Réserver au hub les publications `cmd/`, `ord/`, `val/` et les réponses. MQTT direct reste le transport choisi.

### B2 — La réplication asynchrone du reçu ne garantit pas l’absence de double effet après bascule

**§§ 10.4, 15, 18.** Une trace critique ne protège pas un effet déjà exécuté si le secours ne l’a pas encore reçue. La garantie « un appui retenté après une bascule ne part pas deux fois » dépasse celle de la HA v2.

**Preuve.** La HA v2 reconnaît une perte possible des dernières sorties en vol (`docs/proposition/haute-disponibilite-v2.md:575`). Dans la soirée, les commandes passent au kit avant les traces au secours (`tools/ordres-banc/src/soiree.rs:394`, `soiree.rs:417`).

Scénario : appui exécuté → actif perdu avant copie du reçu → secours promu → même clé retentée → reçu absent → second appui. Aucune panne du banc ne coupe cet intervalle.

**Correction.** Pour garantir un effet au plus une fois, faire acquitter l’intention et sa clé par le support de reprise avant l’effet. Définir le comportement sans secours disponible et après restauration d’un ancien instantané. À défaut, annoncer une garantie bornée et refuser les retries appartenant à une période ambiguë.

### B3 — La nouvelle table de reçus réautorise des appuis anciens

**§ 10.4.** La fenêtre de 32 reçus oublie aussi ce qui permettait de refuser un ancien rejeu. Elle supprime en outre les reçus d’un appelant lorsque le soixante-cinquième arrive.

**Preuve.**

- Recherche limitée aux reçus présents : `tools/ordres-banc/src/demandes.rs:294`.
- Éviction du premier appelant : `demandes.rs:284`.
- Une clé retrouvée pour un autre ordre ne produit aucun conflit : `tools/ordres-banc/src/hub.rs:469`.

Après les séquences 1 à 33, rejouer 1 la fait repartir. Le code actuel conserve `highest`, refuse les séquences trop anciennes, les conflits et la saturation : `crates/moli-runtime/src/occurrences.rs:302`.

L’identifiant `id` ne déduplique pas non plus les demandes : seule la clé optionnelle `rid` intervient.

**Correction.** Conserver un seuil de séquence par principal, refuser les clés anciennes et les conflits, et traiter la saturation sans effacer une preuve de non-rejeu. Donner à `id` et `rid` une sémantique commune explicite, couvrant également les doublons du transport.

### B4 — Le numéro entier d’une demande est effectivement redonné

**§§ 10.1, 18.** La génération sur 16 bits boucle ; la garantie centrale « le numéro entier jamais » est donc fausse. Le bit supérieur est aussi employé simultanément par la génération et par les clés des cartes d’approbation.

**Preuve.** `Demandes::prendre` utilise `wrapping_add(1)` puis remplace zéro par un (`tools/ordres-banc/src/demandes.rs:117`). Après 65 535 réutilisations d’une même case, une ancienne référence redevient valide. Le microbanc effectue lui-même 102 000 prises et libérations successives (`tools/ordres-banc/src/banc.rs:470`).

Dès la génération 32 768, `x.0 | 1 << 31 == x.0` : la carte écrite dans `hub.rs:500` peut écraser les paramètres rangés sous cette même clé (`hub.rs:590`).

**Correction.** Choisir une identité de livraison suffisamment large, incluant son domaine de reprise, et définir son épuisement. Séparer les clés des cartes et des paramètres par un type distinct. Reprendre la règle du nettoyage qui condamne une génération épuisée, sans prétendre que sa fréquence de réutilisation est comparable à celle des demandes.

### B5 — Les paramètres privés circulent dans les commandes et les traces

**§§ 9.1, 10, 15 ; V7 § 5.1.** Le drapeau `TEXTE_PRIVE` est déclaré mais n’influence pas le traitement des paramètres. Le banc transporte donc le contenu qu’il annonce remplacer par une référence.

**Preuve.** La règle figure dans `tools/ordres-banc/src/catalogue.rs:56`. Les arguments sont clonés dans `Cmd` (`hub.rs:518`), les textes rangés en clair (`hub.rs:570`), puis recopiés dans `Sortie::Trace.params` (`hub.rs:756`). La soirée possède bien des formes privées Sonos et Telegram (`soiree.rs:218`, `soiree.rs:230`).

La V7 impose une référence dans l’ordre et le contenu sur le canal de services (`docs/proposition/isolation-reactivite-runtime-v7.md:394`).

**Correction.** Appliquer cette séparation avant publication. Définir les destinataires autorisés du contenu lors d’une réplication et sa rétention. Vérifier qu’un texte sentinelle privé est absent des commandes MQTT publiques, des traces clientes et du journal.

## Constats importants

### I1 — L’authentification initiale ne remplace pas une autorisation vivante

**§§ 10.3, 10.5, 14.3.** « La garantie est la même, la vérification se fait une fois » est trompeur. Une invitation, une session ou un périmètre peuvent expirer pendant une connexion, une retenue ou une attente en file.

**Preuve.** Le SSE actuel réévalue les droits avant les données et chaque seconde au repos (`crates/moli-api/src/sse.rs:101`, `sse.rs:115`). Le permis de commande est revérifié lors de l’envoi (`crates/moli-runtime/src/hub.rs:1798`) et avant le transport (`driver.rs:157`). Ces exigences sont explicites dans `docs/components/reservations.md:63`.

**Correction.** Conserver le principal et un permis réévaluable jusqu’au premier envoi. Révoquer les abonnements et commandes devenus interdits, y compris après approbation et reprise. Une retenue persistée conserve une demande, jamais une autorisation acquise définitivement.

### I2 — Les échéances ne sont pas contrôlées à toutes les frontières

**§§ 9.3, 10.1, 10.3, 10.5.** Le banc ignore l’échéance d’entrée montrée sur `req/`, puis accorde systématiquement cinq secondes à réception. Il peut aussi approuver une retenue déjà échue si son nettoyage n’a pas encore été exécuté.

**Preuve.** `Req` ne contient aucune échéance (`tools/ordres-banc/src/hub.rs:118`) ; `demander` fixe `maintenant + 5 s` (`hub.rs:505`). `decider` ne compare pas l’heure à `d.echeance_ms` (`hub.rs:607`). Le code actuel contrôle explicitement l’expiration lors de la décision (`crates/moli-runtime/src/hub.rs:1341`).

**Correction.** Refuser une demande déjà échue ; employer `min(échéance reçue, maintenant + délai autorisé, fin du permis)`. Contrôler l’expiration dans `decider` lui-même. Seule une approbation encore valide peut ouvrir une nouvelle échéance d’envoi.

### I3 — Une demande ne conserve pas la forme sous laquelle elle a été acceptée

**§§ 9.2, 10.3, 15, 16.** L’immuabilité d’une forme ne suffit pas : la demande retrouve toujours la forme courante de l’ordre. Une redéclaration peut ainsi modifier l’interprétation d’une demande en vol ou de ce qu’un humain approuve.

**Preuve.** `Demande` ne conserve pas de référence de forme (`tools/ordres-banc/src/demandes.rs:70`). `args_de` reconstruit les arguments selon les paramètres actuels (`hub.rs:647`) ; une suppression de paramètre peut donc faire disparaître un argument au lieu de provoquer un refus. La confirmation utilise aussi la forme actuelle (`module.rs:281`, `hub.rs:825`).

**Correction.** Conserver la forme et les arguments typés d’origine pendant toute la demande. Lors d’une approbation, vérifier leur compatibilité avec la définition actuelle sans les réinterpréter. Lier la décision à la version montrée, aux paramètres et, pour un groupe, aux cibles approuvées.

### I4 — Une valeur reçue après la bascule peut être trop ancienne pour confirmer

**§§ 10.5, 15, 18.3.** Connaître la forme permet de comparer une valeur ; cela ne garantit pas que cette valeur constitue une observation suffisamment récente. Le nouveau chemin de confirmation perd une protection déjà présente dans le code.

**Preuve.** `Hub::valeur` confirme une demande héritée avec le point, la valeur et le mouvement, sans critère de fraîcheur de lecture (`tools/ordres-banc/src/hub.rs:810`). Actuellement, `Confirmations::observe` exige une lecture commencée après l’acquittement (`crates/moli-runtime/src/orders.rs:272`).

Scénario : une lecture commence avant l’ordre, son résultat arrive après la promotion et correspond à la cible ; le hub conclut « confirmé ».

**Correction.** Conserver un critère explicite d’observation postérieure à l’envoi ou imposer une lecture fraîche après reprise. Les rapports anciens et les caches de démarrage ne doivent pas lever une incertitude à eux seuls.

### I5 — `EXPLIQUE` établit une compatibilité temporelle, pas une causalité

**§§ 2.1, 6.3, 13.** L’égalité de valeur et la comparaison d’instants ne permettent pas de garantir « éteindre ce que la maison a allumé, pas ce qu’un humain a rallumé ». Une commande identique à un état préexistant peut également s’attribuer cet état.

**Preuve.** `explique` accepte notamment `EN_COURS` et `ENVOYE`, puis vérifie `change_ms <= fiche.instant_ms` (`tools/ordres-banc/src/moteur.rs:482`). Or chaque issue repousse `fiche.instant_ms` (`hub.rs:742`).

Scénario : ordre automatique → geste humain éteindre/rallumer → confirmation tardive de l’ordre. L’instant de la confirmation devient postérieur au geste humain ; `EXPLIQUE` redevient vrai.

**Correction.** Nommer ce prédicat « compatible avec la dernière commande », avec un résultat inconnu lorsque la cause manque. Une règle promettant de respecter les gestes humains exige un contexte causal ou une politique conservatrice sur les transitions observées. Employer des révisions pour leur ordre, plutôt que des comparaisons d’horloge murale.

### I6 — La dernière commande et l’état « en cours » sont confondus

**§§ 11, 13.** Une issue ancienne peut remplacer une demande plus récente dans la fiche. Inversement, l’issue ne suffit pas à savoir si une demande existe encore.

**Preuve.** `tracer` réécrit la fiche à chaque événement sans vérifier que `fiche.demande == x` (`tools/ordres-banc/src/hub.rs:735`). Si A est suivie, puis B demandée et conclue, une suite tardive de A rétablit A comme « dernière commande ».

`EN_COURS` teste seulement les issues `EN_COURS`, `RETENUE`, `ACCEPTE` (`moteur.rs:265`). Il devient faux pendant un suivi `ENVOYE` ou `INCERTAIN`, et peut rester vrai après la clôture finale d’un suivi demeuré `ACCEPTE`.

**Correction.** Définir « dernière » par l’ordre d’admission des demandes. Ne mettre à jour cette fiche que pour la demande correspondante ; conserver séparément la présence de demandes actives et leur phase.

### I7 — Un groupe retenu puis approuvé ne passe plus par le répartiteur

**§§ 10.3, 12, 18.** Le chemin d’approbation transforme le groupe en commande ordinaire adressée à l’appareil virtuel. Aucune demande fille n’est créée.

**Preuve.** L’admission normale appelle `repartir` lorsque la forme porte `GROUPE` (`tools/ordres-banc/src/hub.rs:507`). `decider` pousse directement un `Cmd` (`hub.rs:641`). Le test du groupe protégé vérifie uniquement son entrée en retenue (`soiree.rs:1041`).

**Correction.** Employer un seul chemin de dispatch après autorisation, commun aux admissions directes et aux approbations. L’autorisation humaine doit se transmettre aux filles précisément approuvées, avec leurs permis, sans provoquer de nouvelles retenues identiques.

### I8 — L’agrégation d’un groupe perd sa progression au secours et peut surévaluer sa preuve

**§§ 12, 15, 18.** Le compteur de réussites de la mère n’est pas répliqué pendant la progression. De plus, le succès de toutes les filles produit toujours `CONFIRME`, même lorsque leurs retours prouvent seulement une exécution par un pont.

**Preuve.** `fille_conclue` incrémente `m.faites` sans trace tant qu’il reste une fille (`tools/ordres-banc/src/hub.rs:557`). L’application d’une trace fille au secours ne recalcule pas ce compteur (`hub.rs:943`). Après trois réussites copiées et une bascule avant la quatrième, la mère peut donc finir avec un compte incomplet.

Les filles `EXECUTE` comptent comme réussies (`hub.rs:729`), puis toutes les réussites donnent `CONFIRME` (`hub.rs:565`).

**Correction.** Répliquer chaque progression de la mère ou conserver assez de résultats pour la reconstruire exactement. Définir l’agrégation des niveaux de preuve : quatre volets RTS exécutés ne deviennent pas quatre positions confirmées.

### I9 — Le cycle de la mère peut conclure trop tôt ou provoquer des réveils continus

**§§ 10.1, 12.** Le cycle suppose implicitement que les filles ne terminent pas pendant leur création. Il conserve aussi une échéance expirée sur une mère dont les filles poursuivent leur suivi.

**Preuve.** Les filles sont créées successivement par `repartir` (`tools/ordres-banc/src/hub.rs:538`). Une première fille exécutée immédiatement par le hub peut appeler `fille_conclue` et fermer la mère avant la création des suivantes (`hub.rs:512`, `hub.rs:562`).

Par ailleurs, `prochaine` inclut l’échéance de la mère (`hub.rs:840`), alors que `tic` l’exclut tant qu’elle possède une fille (`hub.rs:853`). Après cinq secondes, une fille suivie pendant deux minutes peut donc laisser un réveil immédiatement dû sans travail à accomplir.

**Correction.** Enregistrer toutes les cibles avant de lancer les filles et compter explicitement les résultats terminaux attendus. Retirer l’échéance d’envoi de la mère lorsqu’elle attend ses filles ; conserver uniquement une borne globale définie.

### I10 — La composition ne dit pas quel ordre de groupe commande quelles cibles

**§§ 9.1, 12.** Une cible est attachée à l’appareil virtuel, sans lien avec un ordre source particulier. Le modèle ne définit pas non plus la validité des compositions récursives.

**Preuve.** `Membre` contient un numéro, un rang, une sorte et un rôle (`tools/ordres-banc/src/hub.rs:75`). `repartir` sélectionne toutes les entrées `ORDRE/CIBLE` de l’appareil pour chaque ordre de groupe (`hub.rs:527`).

Un groupe possédant `on` et `brightness` ne peut donc pas distinguer leurs cibles. Une déclaration ciblant son propre ordre peut aussi conduire à des appels récursifs de `demander`, sans détection de cycle.

**Correction.** Pour la première version, limiter explicitement les groupes à des cibles physiques compatibles et à une relation définie par ordre. Si les compositions récursives sont conservées, valider le graphe, sa profondeur et son expansion maximale avant toute exécution.

### I11 — Les plages contiguës et les rangs relatifs ne couvrent pas une vraie redéclaration

**§§ 8, 9.2, 16.** Le document traite les changements de bornes, mais pas l’ajout, le retrait ou le réordonnancement des valeurs et des ordres d’un appareil. Ces opérations doivent préserver simultanément les identifiants, les références et les correspondances de confirmation.

**Preuve.** `redeclarer` associe les nouvelles formes aux anciennes places par position et ignore les clés (`tools/ordres-banc/src/hub.rs:392`). Les confirmations retrouvent les points par rang relatif (`hub.rs:436`).

Exemple : `[on, brightness]` devient `[on, energy, brightness]`. Le rang 1 ne désigne plus la même donnée ; une simple forme immuable ne protège pas cette correspondance.

**Correction.** Définir une mise à jour atomique par clés stables. Choisir entre des listes de références stables et une règle d’évolution des plages avec trous, déplacement et invalidation explicites. Recompiler les programmes et mettre à jour les caches au même rang critique.

### I12 — Plusieurs limites de capacité sont des troncatures silencieuses

**§§ 9–12, 16, 17.** Les tailles fixes imposent des plafonds qui ne sont ni complètement annoncés ni contrôlés. Le banc ne fait notamment pas respecter le plafond annoncé de 50 retenues.

**Preuve.**

- Case convertie en `u16` sans refus au-delà de 65 536 : `tools/ordres-banc/src/demandes.rs:124`.
- Nombre de points, d’ordres et de membres converti en `u8` : `hub.rs:415`.
- Nombre de cibles converti en `u8` : `hub.rs:534`.
- Numéro de forme converti en `u16` : `catalogue.rs:304`.
- Allocation d’une retenue sans contrôle de quota : `hub.rs:492`.

Une composition de 256 membres devient une composition de longueur zéro. Une rafale de retenues peut dépasser le modèle de mémoire annoncé.

**Correction.** Valider toutes ces cardinalités avant mutation, avec une erreur explicite. Fixer des quotas globaux et par principal, réserver les ressources d’un groupe avant son lancement et définir l’épuisement du catalogue de formes.

### I13 — Une copie filtrée ne peut pas détecter ses pertes avec le seul rang global

**§§ 14.2–14.3.** La preuve d’une copie intégrale ne se transpose pas directement à un écran filtré. Un saut de `s` peut correspondre à une perte ou à un message volontairement invisible.

**Preuve.** `s` numérote toutes les sorties critiques (`docs/proposition/haute-disponibilite-v2.md:281`). Un écran du salon recevant 10 puis 12 ignore si 11 concernait le salon ou une chambre interdite. Lui fournir le tronçon global divulgue des données ; filtrer ce tronçon conserve l’ambiguïté.

Le format client du banc ne porte d’ailleurs pas le couple époque/rang de coupure annoncé (`tools/ordres-banc/src/hub.rs:1150`).

**Correction.** Ajouter une progression vérifiable de la vue filtrée : séquence propre à cette vue ou messages attestant les plages couvertes. Définir l’état initial au rang `(E, s0)`, les changements de droits et la reprise filtrée. Cela conserve MQTT direct.

### I14 — La validation commune ne vérifie pas toute la structure annoncée

**§§ 5, 9.1, 9.3, 18.** Le validateur commun couvre surtout arité, longueur et intervalle numérique. Il ne démontre pas la validation d’un dictionnaire d’énumération ou d’un objet conforme au schéma déclaré.

**Preuve.** `valider_params` ne consulte pas `dico` et traite un objet comme un texte borné (`tools/ordres-banc/src/catalogue.rs:421`). Un dictionnaire contenant les codes 1 et 3 autorise donc aussi 2 si ses bornes sont `[1,3]`. Une chaîne non JSON respecte également la validation d’un objet si sa longueur convient.

`json_forme` omet le schéma objet (`catalogue.rs:498`). Enfin, deux consommateurs appelant le même validateur incomplet peuvent produire 3 344 décisions identiques.

**Correction.** Vérifier les codes réellement déclarés et les schémas aux frontières. Transporter les définitions nécessaires et tester la conformité attendue indépendamment de l’égalité hub/écran. Les branches non prises en charge doivent être refusées explicitement.

### I15 — Le témoin « aujourd’hui » est reconstruit depuis le nouveau modèle

**§§ 2.2, 9.2, 14, 17, annexe A.** Les structures recopiées ne suffisent pas à rendre la comparaison représentative : les déclarations et les états utilisés diffèrent du code de référence. Certains choix gonflent le témoin actuel.

**Preuve.** Le banc crée un point `Button` par nouveau nom d’ordre (`tools/ordres-banc/src/banc.rs:49`). Un volet RTS reçoit ainsi quatre anciens points, alors que le pilote réel possède un seul `control: Enum` (`crates/moli-overkiz/src/model.rs:145`) ; son test exige `["control"]` et aucun état (`model.rs:604`).

Le générateur lui attribue aussi quatre ou cinq valeurs génériques (`tools/memoire-banc/src/maison.rs:119`). Le témoin crée ensuite un état pour tous les points, y compris en écriture seule (`banc.rs:145`, `banc.rs:546`).

**Correction.** Partir de déclarations réelles ou de fixtures communes et définir le mapping nouvel ordre → ancien couple `(point, valeur)`. Recalculer les **828 Kio**, **317 Kio de copie** et **631 Ko** sur ce corpus équivalent.

### I16 — Les 114 ns et 39 ns ne chronomètrent pas des décisions équivalentes

**§§ 2.1, 9.2.** La comparaison de validation mélange des types et des paramètres différents. Elle ne mesure donc pas proprement le gain du registre ou de la forme partagée.

**Preuve.** Les anciens types dépendent d’une nature tirée au hasard (`tools/ordres-banc/src/banc.rs:34`, `tools/memoire-banc/src/maison.rs:124`). Les nouveaux ordres dépendent de la classe d’appareil (`maison_ordres.rs:22`). `cibles_de` fournit les minima valides au nouveau modèle, mais des valeurs anciennes arbitraires, telles que `Float(1.0)` (`banc.rs:173`).

Les chronométrages totalisent seulement les succès, sans assertion d’équivalence (`banc.rs:133`, `banc.rs:198`).

**Correction.** Construire les deux représentations d’une même intention, vérifier préalablement leurs décisions sur un corpus valide et invalide commun, puis chronométrer.

### I17 — Les économies client et assistant retirent aussi de l’information

**§§ 2.2, 14, 17.** Les chiffres comparent des contenus différents et attribuent au nouveau chemin des économies venant aussi du format binaire et d’informations supprimées. Ils ne mesurent pas l’apport propre de MQTT ou des formes partagées.

**Preuve.** Le JSON ancien contient libellés, types, unités, accès, noms et disponibilité (`tools/ordres-banc/src/aujourdhui.rs:285`). Le format de 118 Ko omet notamment les types et unités des valeurs, la disponibilité, les dictionnaires, les schémas et une partie du suivi des demandes (`hub.rs:1150`). Son chronométrage n’appelle aucun transport (`banc.rs:555`).

L’inventaire proposé supprime aussi unités et choix (`banc.rs:295`). Une consigne canonique `50–300`, d’échelle `−1`, apparaît sans expliquer qu’elle signifie `5–30 °C` (`maison_ordres.rs:57`). Les jetons sont estimés par division du nombre de caractères par quatre.

**Correction.** Comparer des états et inventaires contenant les mêmes informations. Isoler les gains de contenu, de partage, d’encodage, de filtrage et de transport. Conserver la qualification « estimation » pour les jetons.

### I18 — Les tailles de deux fiches sont fausses et plusieurs « mesures » sont des forfaits

**§§ 2.2, 9.1, 10.1, 17.** Les fiches annoncées ne correspondent pas à leur déclaration ; le total proposé mélange comptage d’allocations et calculs théoriques. Les valeurs doivent être corrigées avant de servir d’argument de décision.

**Preuve.** Par calcul d’alignement de la déclaration `#[repr(C)]` :

| Fiche | Annoncé | Taille calculée |
|---|---:|---:|
| Appareil | 16 o | **20 o** |
| Forme | 24 o | **20 o** |

Les compteurs utilisent pourtant 16 et 24 (`tools/ordres-banc/src/hub.rs:1264`, `catalogue.rs:515`).

Les `Def` sont ajoutés par formule (`banc.rs:111`), les 50 retenues valent arbitrairement `50 × (64 + 64)` (`banc.rs:530`), et le journal `500 × 40` (`banc.rs:457`). Les tables de hachage sont évaluées par leur capacité utile, sans leur allocation complète (`tools/nettoyage-banc/src/numeros.rs:389`).

Le chiffre **131,3 Kio** se reconstitue ainsi :

```text
58 752 + 34 036 + 4 224 + 8 320 + 20 000 + 6 400 + 2 720
= 134 452 octets
```

Il constitue une addition de postes et de forfaits. Les **8,1 Kio de reçus** correspondent à huit appelants ; 64 fenêtres pleines demandent au moins **66 560 octets**, avant surcapacité.

**Correction.** Employer `size_of`, les capacités effectivement allouées et le compteur par poste. Séparer mesures, calculs et hypothèses. Présenter les 32 Mio comme un budget à vérifier sur le cœur intégré : la somme de résultats provenant de prototypes distincts ne le certifie pas.

### I19 — Le temps « processeur de toute la journée » exclut du travail du hub et du kit

**§ 10.1, annexe A.** Le chiffre annoncé est un temps écoulé sur une partie de la boucle simulée. L’ordonnancement et le traitement des sorties sont partiellement hors chronomètre.

**Preuve.** Le chronomètre commence après `hub.prochaine` et `kit.prochaine` (`tools/ordres-banc/src/banc.rs:379`, `banc.rs:387`) et s’arrête avant le traitement des sorties (`banc.rs:438`). Ces fonctions parcourent demandes, voies ou suivis (`hub.rs:840`, `module.rs:182`). `Instant::elapsed` mesure du temps écoulé, pas du temps CPU.

**Correction.** Chronométrer la boucle complète et préciser séparément les coûts exclus : sérialisation, courtier, réseau et pilotes réels. Les **8,7 ms / 1 279 ms** restent des résultats partiels du prototype.

### I20 — Le redémarrage de la soirée conserve les baux hors de l’instantané

**§§ 15, 18.1–18.2.** Le test de redémarrage ne prouve pas la persistance complète annoncée. Il conserve une partie de la mémoire de l’ancien moteur dans des variables locales.

**Preuve.** `redemarrer` clone les baux avant la coupure (`tools/ordres-banc/src/soiree.rs:585`) puis les injecte directement dans le nouveau moteur (`soiree.rs:601`). `Hub::ranger` ne les sérialise pas (`hub.rs:1053`). Les dictionnaires sont également recopiés depuis l’ancienne machine (`soiree.rs:599`).

La règle d’architecture peut être correcte et déjà étudiée ailleurs ; cette soirée ne l’éprouve pas par son instantané.

**Correction.** Faire repartir la machine uniquement des octets persistés, de la configuration et des nouvelles observations. Sérialiser les sections nécessaires du moteur et vérifier leur restitution, sans objet vivant transféré depuis l’ancien processus.

## Ce qui tient

Le § 4 est globalement solide sur le large échantillon vérifié : `Hub::prepare`, la garde, les canaux, les approbations volatiles, le délai de cinq secondes, le journal et les points en écriture seule correspondent au code. La perte des retenues au redémarrage et la copie d’état pour l’assistant sont bien des problèmes réels. Cette dernière partage toutefois les descriptions par `Arc` ; elle ne duplique pas tous les `PointSpec`.

La séparation entre valeurs et ordres reste justifiée. Le code distingue déjà certaines opérations des états échantillonnés (`crates/moli-core/src/spec.rs:82`), mais il ne possède pas le catalogue explicite et le suivi partagé proposés.

Les formes immuables, les références à génération, la distinction consigne/appui et les issues bornées par le retour sont de bons principes. Le registre repris du nettoyage traite correctement l’épuisement des générations ; cette protection manque dans la table des demandes.

Composer sans posséder évite les doubles propriétaires. Le résultat partiel d’un groupe corrige aussi un défaut réel du luminaire actuel, qui réussit dès qu’un membre réussit (`crates/moli-lights/src/lib.rs:498`).

Enfin, les résultats publiés correspondent aux sorties conservées. L’annexe reconnaît honnêtement plusieurs limites, notamment le corpus hypothétique et l’absence de MQTT ; les formulations du verdict doivent conserver ces réserves.

## Alternatives plus simples

1. **Livrer d’abord un catalogue typé et un suivi central.** Ajouter `OrdreSpec`, garder les permis vivants et les chemins de livraison existants, exposer les demandes et dernières commandes. Reporter l’objectif strict de 32/64 octets jusqu’à stabilisation des invariants de sécurité et de reprise.

2. **Partager les définitions en interne avant de créer un registre public de formes.** Des définitions immuables partagées peuvent suffire au départ. Le registre MQTT de formes devient utile lorsque son gain est établi sur un catalogue réel ; les scores du § 7 ne démontrent pas à eux seuls que sa complexité est nécessaire.

3. **Réutiliser la coordination des scènes pour les groupes.** Le code possède déjà coordination, portées, baux et autorisations. Un groupe peut être une courte exécution de ce coordinateur, avec répartition par le hub ou un module selon le choix conservé du § 20.

4. **Limiter les premières règles de workflow aux faits observables.** Dernière demande, origine, issue et présence d’une demande active suffisent à de nombreux usages. Garder l’attribution causale à « inconnue » lorsque le protocole de l’appareil ne la fournit pas.

5. **Garder MQTT direct avec un service explicite de synchronisation des vues.** Le courtier transporte ; le hub autorise, construit l’état filtré et atteste sa progression. La disparition du module WebSocket ne supprime pas ces responsabilités.

## Questions ouvertes pour le propriétaire

Ces arbitrages sont à consigner ultérieurement ; la revue retient les recommandations suivantes :

- **Quelle garantie pour les appuis sans secours disponible ou après restauration ancienne ?** Retenir une intention durable avant effet ; refuser les retries ambigus lorsque cette garantie manque.
- **Quelle portée donner à « confirmé » et « expliqué » ?** Confirmer une observation fraîche ; conserver « cause inconnue » lorsque l’appareil ne fournit aucun lien causal.
- **Jusqu’où autoriser les groupes dans la première version ?** Commencer par des cibles physiques, une relation explicite par ordre et une expansion bornée.
- **Que devient une approbation si la forme ou les cibles changent ?** Conserver la demande, invalider le consentement devenu différent et présenter une nouvelle carte.
- **Sur quelle installation certifier les 32 Mio ?** Garder le budget et mesurer le cœur intégré à l’échelle cible ; utiliser ×100 comme test de croissance et de saturation.

Non vérifié : transport MQTT réel, courtier modifié, navigateur, matériel physique et nouvelle série Linux de mutations ; les garanties distribuées restent à démontrer.
**Total : 5 constats bloquants, 20 importants, 0 mineur.**