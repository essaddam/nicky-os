# Cast générique — conception

Décision propriétaire du 21 octobre : activation
explicite de chaque appareil découvert, sans publication automatique ni arbitrage
automatique entre lecteurs. Mandat : finaliser la conception et implémenter.

## Périmètre

Pilote natif `cast`, une instance par récepteur vidéo/écran. La musique vers les
enceintes reste à Music Assistant ; une télé déjà couverte garde son pilote.
Commandes : lecture/pause/arrêt, volume récepteur 0–100 et sourdine ; métadonnées
Cast réellement observées. Aucun chargement d’URL libre ou recherche de contenu.
Les interfaces publiques legacy Philips et Android TV restent compatibles.

## Découverte et identité

Option `host` : IPv4 privée explicite, proposée sans connexion au démarrage.
Sinon `interface` IPv4 privée et `prefix` 16–30 fixent le sous-réseau mDNS.
Recherche `_googlecast._tcp.local` uniquement sur demande humaine, trois secondes,
256 datagrammes maximum, 9 000 octets par paquet, 128 records, 64 candidats.
Réponses privées du même sous-réseau ; adresse A annoncée égale à l’émetteur,
port Cast 8009, records PTR/SRV/TXT/A cohérents. Compression DNS bornée et boucles
refusées. ID TXT stable de 32 chiffres hexadécimaux ; noms contrôlés et bornés.
Les enceintes/groupes audio seuls ne sont pas activables par ce pilote vidéo.
Pas de balayage réseau et aucune connexion TLS vers un appareil non validé.

Une instance ne valide qu’un récepteur à la fois ; ID natif issu du TXT, ou
`receiver` constant pour l’adresse manuelle explicitement configurée. Il ne
dérive ni d’un nom affiché ni de l’adresse. Une nouvelle sélection impose une
validation ; révoquer n’efface jamais silencieusement une empreinte changée.

## Activation et propriété

Passerelle partagée avec l’API. État/candidats publics au foyer non invité,
mutations PIN humain, interdites en démo. POST découverte, PUT validation de
`{generation, candidate, exclusive:true}`, DELETE révocation. Aucun hôte libre
dans le corps : la sélection désigne le candidat exact de cette génération.
Maximum 1 Kio, JSON fermé, aucune empreinte/secret publiée.

Les candidats restent hors du hub tant que la validation et sa persistance
n’ont pas réussi. Une réservation explicite par identité/adresse empêche deux
instances Cast de publier le même récepteur. Les hôtes/identités réservés aux
pilotes TV ou déclarés externes sont exclus, sans réattribution automatique.
L’onboarding demande de laisser Music Assistant et les pilotes TV propriétaires
lorsqu’ils couvrent déjà l’usage ; les réservations externes préparent ce contrat.

Activation durable et pin TOFU chiffrés dans une seule entrée. Après validation,
TLS apprend le premier pin et le persiste avant toute observation/commande.
Un pin différent bloque. Les écritures acceptées survivent au moniteur ; les
reprises attendent leur fin, comme pour Android TV. La révocation ferme la liaison
et invalide génération/file avant de retirer la publication.

## Ordres

FIFO de quatre ordres, actif compris. Boucle hub sans attente réseau, travail
possédé en arrière-plan. Délais absolus et abandon suivis jusqu’au premier octet ;
après émission acquisition bornée du résultat, sans rejeu. Les commandes attendues
portent la génération d’activation et de connexion ; reconnexion ne les réactive
pas. Sans session média, lecture/pause/arrêt refusés ; volume récepteur indépendant.

SET_VOLUME du récepteur utilise le namespace receiver et son requestId ; la
réponse doit provenir de receiver-0 et porter le même id et la valeur observée.
Le contrôle ne synthétise pas l’état. Silence/perte après émission incertain ;
refus corrélé explicite distinct. Les métadonnées disparaissent sur perte.

## Livraison et validation

Crate `moli-cast`, module mDNS pur/testable, passerelle et transport injectables,
API/carte Système, paquet integrations/cast, traductions fr/en. Tests du vrai hub,
trames simulées et TLS local seulement ; aucun appareil réel avant H1.
Tests ciblés natifs Rust 1.99.1, deux jobs, fmt et Clippy workspace avant revue
Database. Complet unique au tour de fusion, FF local après approbation, aucun push.
