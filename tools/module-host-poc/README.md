# Hôtes modulaires Rust et master minimal — POC

Prototype autonome, hors du démarrage de `moli-os`. Aucun pilote physique, aucun
compte fournisseur et aucune connexion aux brokers d’une maison. Le serveur 1414
est indépendant. Le dossier V6 présente les résultats dans
[`modules.html`](../../docs/proposition/analyse/v6/modules.html).

## Démarrage et découpage

`moli-master-poc` ouvre son HTTP loopback en premier, puis demande un plan au
binaire adjacent `moli-module-host-poc`. Il lance et surveille uniquement ses
propres enfants. Son code n’importe aucun adaptateur métier, broker MQTT ou codec
audio. `/health` reste disponible lorsque la configuration échoue ou qu’un
processus enfant tombe. `/status` et `/` renvoient le même état JSON ; seuls les
GET sans corps sont admis. Ce POC n’a pas de commandes d’administration HTTP.

Le fichier TOML choisit :

- `merged` : tous les modules actifs et le broker dans un seul enfant du master ;
- `grouped` : broker séparé et un processus par groupe déclaré ;
- `isolated` : broker séparé et un processus par module ;
- `enabled = false` : le module n’est ni construit ni lancé à l’exécution.

Le **master reste toujours séparé**. Les modules d’un même processus partagent
son tas et sa connexion MQTT. Deux processus ont des tas distincts : ce n’est pas
un `mmap`, un conteneur OCI ou une isolation de permissions. Il n’y a ni
redémarrage automatique ni reconfiguration à chaud. Une source active possède
un seul propriétaire `(kind, source_ref)` ; IDs, affectations et bornes sont
validés au démarrage (32 modules, 8 groupes au maximum pour le master).

## Reproduire

Sur ce poste : Rust 1.99.1, deux tâches, répertoire cible dédié. Les commandes
longues se lancent dans la commande Dev « Modules Rust — groupes et master minimal ».

```sh
RUSTUP_TOOLCHAIN=1.99.1 CARGO_TARGET_DIR=target/module-host-poc \
  cargo test --offline --manifest-path tools/module-host-poc/Cargo.toml -j2
RUSTUP_TOOLCHAIN=1.99.1 CARGO_TARGET_DIR=target/module-host-poc \
  cargo clippy --offline --manifest-path tools/module-host-poc/Cargo.toml --all-targets -j2 -- -D warnings
python3 tools/module-host-poc/run-poc.py
python3 tools/module-host-poc/render-report.py
```

Le banc compile les deux exécutables en release, compare trois topologies,
vérifie les désactivations, joue une sélection musicale sur `NullSink`, injecte
une panne dans son propre enfant et vérifie l’arrêt des enfants. Il conserve
`modules-resultats.json`. Sa reprise dépend des empreintes des deux binaires ;
les essais incomplets ne valent pas un passage réussi. Les tests défaillants
initiaux, puis les corrections, sont consignés dans `modules-preuves.json`.

Exemple de supervision continue, à lancer comme commande Dev distincte après
le banc :

```sh
target/module-host-poc/release/moli-master-poc \
  --config tools/module-host-poc/examples/continuous.toml --port 1416
```

`seconds = 0` signifie continu pour un groupe lancé par le master. Les autres
exemples durent quelques secondes. SIGINT/SIGTERM arrêtent le master et récoltent
ses enfants ; aucun autre processus n’est ciblé. Le processus qui supervise le
master lui-même, les limites OS de mémoire et son lancement au boot restent à
intégrer au déploiement réel.

## Adaptateurs disponibles

| Module | Ce qui s’exécute | Limite |
| --- | --- | --- |
| TaHoma | Décodeur et encodeur natifs `moli-overkiz` sur fixtures | Ordres RTS ouvrir/fermer/stop/my selon capacités ; aucune position inventée ni requête physique |
| Ariston | Modèle natif `moli-ariston`, état multicanal et mode enum | Corps de requêtes simulés, bornes vérifiées ; pas de compte cloud |
| Météo / soleil | 14 points météo fictifs et 24 points solaires calculés | Horloge et coordonnées de test ; aucune prévision réelle ; [méthode et précision](../../docs/proposition/analyse/v6/sources-solaire.md) |
| Apple TV | Instantanés normalisés, canaux secondaires, catalogue d’applications et commandes | Pas de transport natif Apple TV ni raccord HA vivant dans cet outil |
| Musique | Moteur Rust local `tools/music-engine-poc`, décodage WAV vers `NullSink` | Lecture par blocs à chaque tick d’essai, pas un rendu temps réel ni une sortie physique |

Le moteur musical commence à l’arrêt. Son catalogue froid fournit deux pistes
synthétiques. Le banc envoie explicitement `play` par MQTT, puis vérifie les
frames et échantillons décodés. Aucun PCM ne passe dans MQTT. Les services
musicaux, les vrais lecteurs, le DSP et la synchronisation multiroom restent des
travaux séparés ; ce T0 ne constitue pas une parité Music Assistant.

## Contrat MQTT et mesures

Topics stables : `moli-probe/module/<id>/config`, `state/<PointId>`,
`availability`, `simulate`, `command-result`. L’affectation de processus ne
figure pas dans l’identité du topic ou du point. Descriptions/catalogues retained
précèdent les états différentiels ; chaque état transporte la génération de
l’instance et sa révision de description. Les clients attendent la description
correspondante avant d’interpréter un code. Changer de génération invalide les
valeurs et la disponibilité de l’ancienne instance.

Les enums arrivent dans le cœur comme `Value::EnumCode`, avec tables à la
frontière. Les descriptions sont transférées une fois depuis l’adaptateur.
Les textes libres sont bornés à 4 096 octets UTF-8, sans troncature silencieuse ;
la sérialisation des deltas emprunte les valeurs. Le POC garde encore les textes
libres courants et les dernières valeurs dans le tas du groupe : il ne démontre
pas encore un stockage intégralement froid ou un layout inline de production.

Commandes d’essai uniquement : enveloppe ≤1 024 octets, requête native ≤256,
ID/génération/expiration (≤5 s), déduplication bornée à128 reçus, aucune commande
retained. Une surcharge de la file d’admission est comptée ; elle ne bloque pas
la boucle MQTT et n’exécute pas les commandes rejetées avant admission. Ce n’est
pas le raccord à la garde de production. Les fichiers TOML de cet outil sont des
fixtures de confiance locales.

Les résultats donnent RSS et CPU par **processus**, allocations vivantes du tas
instrumenté de l’hôte, compteurs logiques par module et temps moyen de traitement.
Aucune RSS fictive n’est attribuée à un module partageant un processus. Les
compteurs excluent les en-têtes d’allocateur sauf indication ; la somme des RSS
peut compter plusieurs fois des pages partagées. Les petits pourcentages CPU
proviennent de compteurs `ps` peu précis sur ces courtes fenêtres. Le coût des processus `ps` de mesure n’est pas inclus. Ils ne prouvent
ni un budget de production ni un gain sur la campagne précédente de 20 maisons.

Le master expose ses mesures par HTTP ; la télémétrie complète du master n’est
pas republiée en MQTT dans ce lot. Un arrêt brutal est visible dans le master,
mais ne publie pas encore l’indisponibilité MQTT des appareils : un bail de
fraîcheur de données reste à raccorder. Les heartbeats attestent la boucle du
processus, pas la fraîcheur d’un appareil.
