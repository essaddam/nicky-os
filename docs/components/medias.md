# Médias : domaine pur de découverte R1

## État

La tranche A fournit la crate pure `moli-media-catalog` : schémas, validation de
requêtes/pages et contrôles de sélection sur métadonnées. Aucun service de
recherche, route REST/MCP, outil de Moli ou interface de lancement n'est raccordé.
Les dix contrats utilisent exclusivement des fixtures synthétiques. Compilation,
ROUGE initial et passages correctifs sont distingués dans les preuves du lot.

R1 cible la France et des métadonnées françaises. La découverte réelle attend
les droits d'usage, les attributions et une clé fournie humainement. Aucun
compte, achat, abonnement partenaire, guide ou replay intégré n'est livré ici.

## Contrat

`ContentKey { source, id }` identifie un objet dans sa source. Un ID TMDB,
un numéro de chaîne affiché, une collection ou un portail ne deviennent jamais
un ID fournisseur lançable par simple changement de type.

`ContentRef` conserve titre, type, année et langue réellement connus, parents
série/saison/chaîne et numéros d'affichage facultatifs. Ses références fournisseur
gardent leur namespace et leur provenance. Saison, épisode spécial, chaîne,
diffusion, portail, collection et programme replay restent distincts.

`Offer` est séparée de l'identité : fournisseur, pays, mode de monétisation,
formule éventuelle, dates et entitlement. `Unknown` reste inconnu.
Une offre d'abonnement ne devient pas une location ; une fin inconnue n'est
pas une preuve d'absence d'expiration.

`Provenance` conserve source et identifiant du relevé ; `Freshness` distingue
récupération, date amont éventuelle et limite de fraîcheur. Les horodatages sont
des millisecondes UTC. Une URL d'image ou de preuve est une donnée non fiable,
sans accès réseau ni rendu dans cette crate.

`SearchPage` associe objets, offres et états par source/fournisseur.
Indisponibilité, quota, délai, absence de configuration et recherche réussie sans
résultat sont des états distincts. Un cursor éventuel reste opaque ; aucun
snapshot ou cache avec état n'est implémenté dans A.

`Intent` n'a que `OpenDetails` et `PlayExact`, sans repli automatique en
application. Les `ReaderCapabilities` sont déclaratives, sans autorité.
`SelectionCheck::RequiresAdapterEvidence` est le seul résultat positif prévu ;
il n'autorise ni envoi ni lecture. Une qualification fournie par le client ne
peut créer une preuve officielle ou matérielle.

## Interfaces et validation

- `SearchQuery::parse(&[u8]) -> Result<SearchQuery, CatalogError>` :
  refuse les champs inconnus/dupliqués, borne l'entrée avant décodage, normalise
  les espaces de bord et l'alias `fr`, puis contrôle les filtres France/français.
- `SearchPage::validate(&self, &SearchQuery, now_ms) -> Result<(), CatalogError>` :
  contrôle les bornes, identités distinctes, parents déclarés, provenance,
  fraîcheur, offres et états de sources. `NoMatch` reste distinct de l'absence
  de source utilisable. Une information absente n'est pas inventée.
- `check_selection(&ContentRef, &Offer, Intent, &ReaderCapabilities, now_ms)` :
  contrôle l'offre, le fournisseur, le type et l'intention demandés par le lecteur,
  la référence fournisseur explicite, les dates et la fraîcheur. `OpenDetails`
  peut seulement retourner `RequiresAdapterEvidence` ; `PlayExact` reste refusé
  même si les capacités déclarées le proposent.

Les contrôles appliquent les bornes suivantes : requête 4 Kio, texte 2 à 200
caractères et 800 octets, sept fournisseurs, page 20 par défaut/50 maximum ;
clé 256 octets, titre 512, résumé 4 000, référence externe 2 048, réponse
256 Kio. Le budget de réponse est compté par un écrivain de sérialisation borné,
sans construire une seconde réponse complète. Les offres restent limitées à
50 × 7 et les états de sources à sept. Les comparaisons d'identité et de parent
portent toujours sur la paire source/ID, jamais sur le titre ou le numéro affiché.

Les références sont restreintes à une syntaxe HTTPS avec nom DNS explicite,
sans identifiants, port, adresse IP littérale ou suffixe local/réservé. Cette
restriction ne prouve ni l'origine officielle, ni la résolution vers une adresse
publique, ni la sécurité d'un futur accès réseau. Aucun accès n'est effectué ici ;
l'adaptateur de découverte devra définir ses propres origines autorisées.
Les dix contrats et la procédure de contrôle sont détaillés dans
[le paquet](../../crates/moli-media-catalog/README.md).

Le membre du workspace a été ajouté à `Cargo.lock` par la préparation conteneur
coordonnée, sans mise à niveau d'autre paquet. Un problème de lock, de fixture
ou de compilation reste un défaut de SETUP, distinct d'un échec fonctionnel.

## Frontières

Aucune dépendance Hub/runtime, réseau, Android, résolveur média, lecteur ou
Music Assistant. Aucun `ContentRequest` ni `LaunchTarget` construit.
Les fixtures sont incluses dans les tests exclusivement, jamais servies comme
un catalogue réel. Aucun LLM n'entre dans un chemin de résolution ou d'envoi.

La suite B concernera la découverte de lecture/API avec provenance et
indisponibilité explicites. La suite C, séparée, devra conserver le contexte
serveur d'intention, d'offre et d'accès jusqu'à l'émission après une garde 202.
Elle devra préserver Claim/Delivery/Observer et les filtres de confidentialité
de la base courante. A ne modifie ni cette garde ni les ordres existants.

Le cache de 1 Mio et l'enveloppe active cible de 3 968 Kio de l'étude ne sont
ni implémentés ni mesurés par A ; le budget global de
[performance](../PERFORMANCE.md) reste celui du dépôt.
