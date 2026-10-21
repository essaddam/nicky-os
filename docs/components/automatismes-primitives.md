# Automatismes : cadrage des primitives et des calculs

Statut : proposition à relire avant implémentation. Le présent lot livre l'audit et le
découpage ; aucune primitive ci-dessous n'est ajoutée par ce document. Les sous-tickets
du backlog portent chacun une tranche complète, vérifiable sans appareil physique.
Sa publication ne modifie implicitement aucune règle de validation ou d'exécution.

## Objectif et périmètre

Représenter les usages utiles de HA en aides, scènes, graphes ou points calculés
déterministes. Chaque résultat d'agent ou d'import reste un brouillon désactivé jusqu'à
validation humaine de sa version exacte. HA garde la commande de chaque usage tant que
sa relève n'est pas qualifiée et validée ; deux moteurs ne possèdent jamais simultanément
le même usage.

Cette extension ne dépend pas du remplacement du stockage d'historique. Elle ne porte
ni migration Recorder, ni services tiers, alarme, shell arbitraire ou orchestration
externe. La capture privée reste en lecture seule ; sources, noms, identifiants et
correspondances de reprise ne vont jamais dans le dépôt public.

## État vérifié dans le code

| Composant | Existant | Limite pour cette extension |
|---|---|---|
| `moli-helpers` | `HelperKind::{Select, Switch, Number, Text}`, un point inscriptible par aide | Pas de minuteur, incrément atomique, date/heure, bouton ou planning |
| Cache du hub | Derniers échantillons avec leur horodatage ; sauvegarde périodique et à l'arrêt | Ce cache ne garantit pas la durabilité de chaque commande acquittée ; un JSON invalide est remplacé par `{}` si la réparation réussit ; une erreur d’accès ou de réparation empêche le démarrage |
| Bus | `DriverCtx::pulse_state` diffuse aussi deux valeurs égales | `engine::on_state` ignore les valeurs identiques ; un bouton ne peut pas se limiter à cette méthode |
| Graphes | États, seuils, horaires, attentes, notifications ; modes `restart`, `single`, `queued` | Pas de scène ni de déclencheur d'occurrence ; attentes et files en mémoire |
| Horloge du moteur | Minute courante marquée au démarrage pour éviter un second déclenchement | Ce mécanisme n'est pas une reprise persistante des minuteurs et ne rattrape pas les horaires manqués |
| `moli-energy` | Suivi de compteur avec reset, intégration de puissance, prix par période et profils lumineux | Pas un moteur générique de points calculés ou de compteurs à cycles ; la couverture des modèles doit être vérifiée |
| Import HA | Traduction en brouillons, empreinte humaine, provenance dans une note | Pas de correspondance privée structurée durable ni de preuve automatique d'équivalence |

Points de lecture : `crates/moli-helpers/src/lib.rs`, `moli-runtime/src/{driver,hub,state_cache}.rs`,
`moli-automation/src/{engine,model,run,store}.rs` et `moli-energy/src/{config,meter,estimate}.rs`
(tous sous `crates/`).

## Contrats communs proposés

### Définition, validation et commande

- Garder les configurations des quatre aides existantes compatibles. Ajouter des types
  explicites plutôt que déguiser un événement en texte ou un incrément en affectation.
- Séparer définition, version validée et état d'exécution. Le contrat proposé pour les
  dépendances exécutables est décrit ci-dessous ; il ne découle pas du modèle actuel.
- Aucun LLM dans le chemin d'exécution des aides, scènes, calculs et usages repris.
  La création, la traduction et l'explication par l'IA restent hors de cette exécution.
- Toute commande passe par le hub, sa garde, ses échéances et son journal ; aucune
  nouvelle route ne permet à un appelant de choisir `Origin::Automation` ou `System`.
  L'origine et l'acteur suivent les commandes dérivées. Une cible retenue par la garde
  reste en attente, jamais présentée comme appliquée.
- Distinguer commande acceptée, état confirmé et résultat partiel. Les scènes et
  occurrences portent un identifiant de corrélation ; les mutations et leurs échecs
  ont une trace exploitable au journal. Les changements de définition sont aussi tracés.
- Exposer les opérations par REST, MCP et CLI avec les mêmes règles. Documenter leurs
  schémas, erreurs et limites dans la documentation d'API. Toute interface ajoutée
  affiche brouillon, validation et erreurs ; chaque texte visible existe en français
  et en anglais dès son écriture.

### Versions des dépendances : proposition à arbitrer

Aujourd'hui, `Automation::fingerprint` couvre le mode, les nœuds et les liens du graphe,
et `Version` ne conserve que mode et graphe. Les étapes nomment leurs références par
identifiant : l'empreinte couvre donc déjà ces identités. Il n'existe pas de registre
de versions validées de dépendances exécutables. Le contrat ci-dessous étend les validations
actuelles : il est validé à la revue du premier lot qui l'implémente (relecteur du lot et
architecte) ; seul un conflit avec la garde actuelle remonte au propriétaire. La publication
de ce document n'assouplit aucune vérification existante. Cet arbitrage concerne ce contrat
seulement et ne réduit aucun autre arbitrage réservé au propriétaire.

- Chaque planning, scène ou calcul possède un brouillon et une version validée courante.
  Un graphe validé utilise cette version validée courante, jamais le brouillon.
- Les nouvelles dépendances sont aussi référencées par identifiant stable dans les
  étapes. L'empreinte couvre ainsi structure, mode et identités, sans figer les versions
  de ces dépendances. Leur résolution à la version validée courante et la propagation
  de leurs mises à jour validées doivent être définies et testées.
- Une modification par agent reste un brouillon sans effet sur la version courante et
  sur les graphes qui l'utilisent. Elle ne remplace ni ne valide cette version.
- Une édition humaine destinée à remplacer la version courante est explicitement
  autorisée et validée, avec les graphes concernés montrés avant confirmation et la
  version attendue vérifiée pour éviter une validation périmée. Son effet est journalisé.
- « Humaine » signifie autorisée par les mécanismes existants de garde, de session et
  de validation. Un appel UI quelconque ne prouve pas cette autorisation ; le PIN reste
  exigé partout où les règles actuelles le demandent. Aucun nouveau chemin de confiance.

Le premier lot qui introduit une dépendance exécutable référencée par un graphe
(`AUTO-H5`, `AUTO-S1` ou `AUTO-K1`, selon l'ordre de passage) réalise ce mécanisme commun ;
les suivants le réutilisent. Les valeurs des aides actuelles restent de l'état, sans
version : ce contrat n'impose pas une validation de définition à chaque changement de valeur.

### Persistance et temps

- Une mutation annoncée durable n'est acquittée qu'après écriture de son état métier.
  Le cache périodique du hub ne constitue pas cet acquittement. Une erreur de stockage
  doit être visible et ne doit pas créer un succès ni une réinitialisation silencieuse.
- Contrat commun : un fichier d'état métier par brique, versionné et borné, avec un seul
  rédacteur. Écrire l'état entier dans un fichier temporaire, le synchroniser (`fsync`),
  puis le renommer atomiquement et synchroniser le répertoire avant acquittement. Fournir
  une restauration validée et des tests de coupure avant/après écriture et renommage.
  Aucun lot ne choisit une nouvelle base SQLite ; aucune dépendance à `moli.db` ni à la
  migration historique. Ce contrat ne redéfinit pas la cible de stockage de D13.
- En exécution, les durées utilisent une horloge monotone ; une échéance persistée est
  un instant absolu. Tester le calcul du restant après redémarrage, les sauts d'horloge
  et les dépassements arithmétiques. Les calendriers utilisent le fuseau configuré.
- Chaque occurrence durable a une identité stable et une politique de consommation
  persistée. Rejouer un événement du bus ou restaurer l'état ne doit pas déclencher
  deux fois un usage. Une interruption entre effet physique et confirmation reste
  ambiguë : la signaler, sans promettre une transaction entre appareils ni rejouer
  aveuglément une commande.
- Une perte du bus peut se réparer par relecture pour un **état** ; elle ne reconstitue
  pas des **occurrences**. Boutons et expirations doivent définir la livraison, le
  rattrapage et la déduplication côté moteur, avec tests de retard du consommateur.

### Bornes et coût

Chaque lot fixe et teste ses plafonds avant livraison : nombre de définitions, sources,
créneaux ou cibles, taille d'état, plage de durée, concurrence et file. Aucune allocation
ou tâche par entrée non bornée. Réutiliser les limites des graphes lorsqu'elles
s'appliquent ; les nouvelles limites doivent être documentées dans l'API.

Les calculs ont un index de dépendances, un ordre stable, une détection de cycles et
un budget de propagation. Aucun balayage global à chaque événement ; aucun travail
si la brique n'est pas configurée. Mesurer les chemins actifs contre
[PERFORMANCE.md](../PERFORMANCE.md), sur sources simulées.

## Lots verticaux

Tous les codes de ce chantier portent le préfixe `AUTO-`, y compris dans les
références entre tickets. `AUTO-H1` désigne le minuteur ; il est distinct du jalon
humain `H1` d’installation et de qualification. Les calculs utilisent `AUTO-K1` à
`AUTO-K4` et la reprise `AUTO-M1` à `AUTO-M2`, sans réutiliser les codes du stockage.

Chaque lot inclut modèle, validation, exécution, persistance nécessaire, surfaces de
configuration/lecture/commande, documentation, traductions et tests. Les estimations
ci-dessous sont des jours de développement, hors qualification physique et revue.

| Lot | Livrable | Effort | Prérequis |
|---|---|---|---|
| AUTO-H1 | Minuteur durable : démarrer, annuler, restant, expiration et reprise | 3–5 j | Aucun |
| AUTO-H2 | Compteur : incrément atomique et remise à zéro durables | 1–3 j | Aucun |
| AUTO-H3 | Aides date, heure et date-heure validées | 1–2 j | Aucun |
| AUTO-H4 | Bouton : occurrence distincte par appui et déclencheur de graphe | 2–3 j | Aucun |
| AUTO-H5 | Planning hebdomadaire éditable, versionné et validé | 2–4 j | Aucun |
| AUTO-S1 | Scène d'état voulu, confirmation et concurrence bornée | 3–5 j | Aucun |
| AUTO-K1 | Points calculés : min, max, moyenne avec fraîcheur | 2–4 j | Aucun |
| AUTO-K2 | Compteur périodique : cycle, reset, correction et tarif | 3–5 j | AUTO-K1 |
| AUTO-K3 | Temps passé dans un état avec couverture connue | 2–4 j | AUTO-K1 |
| AUTO-K4 | Estimation par profils des seuls modèles requis | 2–5 j | Aucun |
| AUTO-M1 | Gabarits utiles traduits en calculs ou graphes, brouillons | 2–5 j | AUTO-H1–AUTO-H5, AUTO-K1–AUTO-K4 |
| AUTO-M2 | Scripts utiles traduits en graphes, brouillons | 2–3 j | AUTO-H1–AUTO-H5, AUTO-S1, AUTO-M1 |

Les prérequis de reprise sont conservateurs : ils évitent de lancer une conversion
avant que ses primitives existent. Ils pourront être réduits, preuves à l'appui, lors
du tri privé des usages. Aucun lot n'attend le chantier de stockage historique.

### AUTO-H1 — Minuteur

Définir les états inactif, actif, annulé et terminé, les commandes de démarrage et
d'annulation, et le restant en secondes, borné à zéro. Redémarrer un minuteur remplace
sa génération ; annuler invalide l'ancienne expiration. Restaurer une échéance passée
termine une fois la génération, avec une occurrence identifiable, sans rafale de
rattrapage. Le protocole de livraison durable au moteur doit être établi dans ce lot.

Preuves : limites de durée, annulation à l'échéance, démarrages concurrents, coupure
avant/après expiration et consommation, plusieurs redémarrages après échéance, échec
d'écriture et retard du bus. Aucun graphe en brouillon ne reçoit un droit d'exécution.

### AUTO-H2 — Compteur

Un incrément est une opération sérialisée par compteur, jamais une lecture puis une
affectation côté client. Définir la valeur initiale, le pas entier, les bornes et le
refus de dépassement ; la remise à zéro revient à la valeur initiale définie. Distinguer
la répétition volontaire d'un incrément du rejeu d'une même requête identifiée.

Preuves : incréments simultanés sans perte, ordre incrément/reset, overflow, bornes,
rejet sans mutation, état restauré, échec d'écriture et requête répétée après reconnexion.

### AUTO-H3 — Date et heure

Types explicites pour date seule, heure seule et date-heure. Valider calendrier et
format ; refuser les dates impossibles. Définir comment une date-heure locale ambiguë
ou inexistante est rejetée ou désambiguïsée, et restituer le fuseau. Une simple aide
de saisie ne déclenche pas à elle seule un automatisme.

Preuves : année bissextile, changement de jour, heure d'été/hiver, sérialisation et
restauration, configuration antérieure toujours lisible, erreurs localisées.

Le [contrat détaillé proposé](dates-heures.md) précise les formats, la distinction
civil/instant, le refus des ambiguïtés et la durabilité. Sa préparation ne livre
aucun comportement ; il reste soumis à la revue AUTO-H3 avant implémentation.

### AUTO-H4 — Bouton

Chaque commande d'appui produit une occurrence distincte, y compris deux appuis de
même contenu dans la même milliseconde. Fournir un déclencheur d'occurrence qui ne
repasse pas par la déduplication des états ; l'initialisation d'un pilote ou la
restauration de son dernier état n'est jamais un appui. Définir explicitement la
politique de coupure, de livraison et de déduplication d'une requête rejouée.

Preuves : deux appuis identiques déclenchent deux fois un graphe validé, aucun au
redémarrage, aucun en brouillon, garde respectée, perte du bus traitée sans inventer
une occurrence et compteur de séquence sans collision.

### AUTO-H5 — Planning

Créneaux hebdomadaires dans le fuseau de configuration ; jours 1 à 7 et intervalles
semi-ouverts `[début, fin)`. Découper explicitement un créneau traversant minuit ;
refuser les chevauchements. Édition avec contrôle de version : une écriture périmée
renvoie un conflit. Une modification par agent reste un brouillon et n'affecte pas
la version validée courante utilisée par les graphes. Une édition humaine de cette
version suit le contrat proposé des dépendances, avec autorisation, validation et
présentation des graphes concernés.

Preuves : dimanche/lundi, minuit, limites et chevauchements, heure sautée/doublée,
édition concurrente, restauration de la bonne version et transition sans doublon.
Prouver aussi qu'un brouillon d'agent n'affecte aucun graphe actif et qu'un simple appel
UI ne peut remplacer une validation humaine requise.

### AUTO-S1 — Scène

La scène décrit des valeurs cibles typées, pas des bascules. Prévalider toutes les
cibles, refuser les doublons contradictoires et montrer la portée réelle à l'humain.
Exécuter avec une concurrence plafonnée et des échéances, en conservant l'ordre pour
des commandes visant le même appareil. Rendre par cible : retenue, confirmée, échouée
ou confirmation expirée. Un acquittement de pilote ne prouve pas toujours l'état voulu.

Preuves : cible déjà conforme, cible absente ou hors ligne, garde sur un membre,
confirmation ancienne, commande concurrente extérieure, annulation, panne partielle
et reprise sans rejouer une commande physique. Pas de rollback physique automatique.

### AUTO-K1 — Min, max et moyenne

Créer une définition déclarative bornée avec sources numériques compatibles, unité,
durée de fraîcheur et publication en lecture seule. Exclure absences, valeurs nulles,
non finies, périmées et sources hors ligne ; une liste vide produit indisponible,
jamais zéro. Une expiration de fraîcheur recalcule même sans nouvel événement.
Garder l'horodatage des données sources lors d'une restauration.

Preuves : moyenne sur les seules sources valides, unité incompatible, échéance exacte
de fraîcheur, disparition/retour d'une source, cycle de dépendances, propagation
déterministe et limites de coût. Ce lot fournit le socle de publication utilisé par AUTO-K2/AUTO-K3.

### AUTO-K2 — Compteurs périodiques

Définir les cycles calendaires, leur fuseau et les frontières, la source cumulative
ou delta, les resets de source, les corrections signées explicites et la ventilation
par tarif. Réutiliser le suivi de compteur énergétique là où son contrat convient ;
ne pas confondre une correction avec une consommation nouvelle ou un reset.
Conserver la dernière lecture consommée, le cycle, le tarif et les corrections durables.

Preuves : changement de cycle et tarif, baisse de source, lecture dupliquée ou en
retard, correction, trou de disponibilité, jours de 23/25 heures, redémarrages autour
de la frontière et absence de double comptage. Pas d'import de l'historique dans ce lot.

### AUTO-K3 — Temps dans un état

Accumuler sur des intervalles connus et bornés, avec instant d'entrée, durée acquise
et couverture. Un état absent, périmé ou hors ligne termine la couverture connue ;
une période d'arrêt ne doit pas devenir du temps observé. Reconstituer la durée sans
compter deux fois l'intervalle ouvert lors d'un redémarrage.

Preuves : événements égaux ou désordonnés, changement à la frontière, panne et reprise,
trou de télémétrie, horloge reculée et écriture échouée. La durée et les périodes
inconnues restent distinguables pour l'utilisateur.

### AUTO-K4 — Profils d'estimation

Établir hors dépôt la correspondance des entités utiles et de leurs modèles ; seules
les courbes ou règles publiques de modèles effectivement requis entrent au catalogue.
Vérifier les profils déjà présents avant tout ajout. Conserver source publique,
version et licence des données ; aucune récupération dynamique ni estimation LLM.
Un modèle inconnu reste non couvert, sans profil voisin choisi implicitement.

Preuves : veille, extinction, luminosité, mode blanc/couleur et entrée manquante ;
une absence de télémétrie n'est pas à elle seule une mesure de zéro watt. Marquage
explicite « estimation », unités correctes et absence d'addition aux totaux mesurés.

### AUTO-M1 et AUTO-M2 — Reprise utile en brouillons

Trier chaque source privée : utile et représentable, dépendance manquante, service
externe à conserver ou décision humaine nécessaire. Le relevé exhaustif et la
correspondance source/version/empreinte/cible restent dans un espace privé inscriptible
distinct de la capture en lecture seule ; rien n'est écrit dans cette capture.

Traduire les gabarits vers des expressions déclaratives bornées ou des graphes : pas
d'interpréteur Jinja général, `eval`, shell ou appel distant arbitraire à l'exécution.
Traduire les scripts en conservant ordre, attentes, annulations, conditions, modes
et effets indirects. Une logique non représentable est signalée et reste chez HA.

Preuves : fixtures synthétiques publiques, comparaison privée sur entrées identiques,
import répété sans doublon, graphe modifié redevenu brouillon, dépendance manquante
visible. Aucun original privé ne devient fixture publique, même simplement renommé.
La qualification réelle et la bascule ont lieu seulement après validation humaine :
désactiver l'usage HA, vérifier son arrêt, puis autoriser sa version Moli. Le retour
arrière éteint et annule Moli avant de réactiver HA. Conserver la preuve du propriétaire
unique et du résultat, sans inclure de données privées dans le dépôt.
Toute qualification physique se fait sur une cible neutre, avec un humain présent,
jamais dans une pièce protégée, et remet la cible dans son état initial.

## Validation d'une livraison

Chaque sous-ticket doit établir ses tests de contrat avant son implémentation, puis
livrer un commit relisible sur sa branche. Le garde-fou reste `sh scripts/check.sh`
dans l'image de développement, sans modifier les sources pendant son exécution.
Si l'interface change : `cd web && npm ci && npm run build`, puis
`node scripts/i18n-check.mjs` à la racine. Un contrôle ciblé ne remplace pas le garde-fou.

La revue vérifie aussi les délais et les plafonds, la reprise après coupure, le retard
du bus, l'usage exclusif des versions validées, la confidentialité et le budget de
performance. Elle distingue livraison des primitives, conversion privée et autorisation
d'exécuter : aucune de ces étapes ne vaut automatiquement pour les suivantes.

## Livraison AUTO-H2

Le contrat du [compteur durable](compteurs.md) décrit l'implémentation ajoutée :
valeur entière en lecture seule, opérations `increment` et `reset` à null, propriétaire
unique au hub, valeurs et reçus atomiques, garde/journal et interfaces communes.
Les autres lots de ce cadrage restent des propositions tant que leur propre livraison
n'est pas décrite. Ce lot n'introduit pas de registre de dépendances exécutables.

## Livraison AUTO-H1

Le contrat des [minuteurs](minuteurs.md) décrit commandes entières, restant monotone,
échéance UTC, générations, reçus atomiques et expiration récupérable. Les lignées
figées au démarrage protègent contre une nouvelle validation après crash. Le moteur
persiste les consommations sans cible avant acquittement. Aucun LLM, nouveau chemin
de confiance ou changement de la sémantique live-only AUTO-H4.
