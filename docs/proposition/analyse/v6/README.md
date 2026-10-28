# Dossier V6 — ponts MQTT et budget mémoire

Édition du 27 octobre 2026. Ouvrir [index.html](index.html) dans un navigateur.
La page est autonome, sans bibliothèque distante, sans connexion à une maison
et sans commande d'appareil. Les liens de sources sont relatifs au dépôt.

Le [guide HTML de migration Home Assistant → Moli](../../migration-home-assistant.html)
présente les parcours par composant et par workflow, les impacts mémoire,
les retours arrière et les conditions de validation de chaque lot.

- [mesures.json](mesures.json) : mesures agrégées, unités, périmètres et limites.
- [preuves.json](preuves.json) : registre des tests, observations et points non prouvés.
- [empreintes.json](empreintes.json) : empreintes des fichiers d'implémentation relus.
- [bancs/README.md](bancs/README.md) : reproduction synthétique des allocations et layouts.

Les résultats antérieurs consignés sont distingués des vérifications rejouées
pour le dossier. Aucune commande physique n'est envoyée. Les snapshots privés
et les journaux qui contiennent des identités de maison restent hors dépôt.

Le rapport recommande un virtuel d'ambiance multicanal par pièce et des groupes
métier distincts pour lumières et volets. Le format inline pour les textes courts
et une réserve bornée pour les textes longs restent à qualifier. La consolidation
du design et les raccords d'actions HA manquants ne sont pas présentés comme livrés.

Le [complément de charge](charge.html) présente vingt maisons et quarante workflows
simulés, avec [résultats bruts agrégés](charge-resultats.json), succès et échecs.
Le [reçu des snapshots](heap-snapshot-recu.json) distingue le budget RSS surveillé
de 1 Gio d’une limite système stricte. Les captures brutes restent hors dépôt.

L’[analyse des instantanés sous budget de 1 Gio](memoire-1g.html) attribue
les principales allocations aux descriptions, aux buffers et au cache des
automatismes. Les nouvelles empreintes et validations sont dans
[charge-empreintes.json](charge-empreintes.json) ; empreintes.json conserve
l’état historique de la première édition du dossier.

Le [complément modules Rust et master minimal](modules.html) présente les modes
fusionné, par groupes et isolé, leurs mesures par processus et le premier chemin
audio Rust à sortie muette. [Résultats](modules-resultats.json) et
[preuves](modules-preuves.json) distinguent ce banc des essais de vingt maisons.

Le [complément haute disponibilité entre deux maisons](ha-multimaison.html)
présente le mode actif chez soi/passif pour l’autre, son paramétrage, les essais
de processus et les observations MQTT réelles séparées du rejeu.

Le [test de perte de liaison TCP](ha-multimaison.html#perte-connexion-tcp)
complète les coupures logiques : connexions fermées, puis sockets ouvertes
sans transmission, dans les deux sens. [Résultats](ha-multimaison-tcp.json),
[protocole](ha-multimaison-tcp-protocole.md) et
[preuves courantes](ha-multimaison-tcp-preuves.json) distinguent ce banc
loopback d’une panne WAN réelle entre des serveurs déployés.

La [tentative de qualification sur les deux hôtes réels](ha-multimaison-wan.html)
consigne les vérifications d’accès, les délais TCP entre maisons, le mode
autonome préparé et les quatre cas encore **non prouvés**. Le transfert et la
liaison directe doivent fonctionner avant d’injecter une perte temporaire ou
prolongée. [Constats](ha-multimaison-wan.json) et
[preuves](ha-multimaison-wan-preuves.json) sont distincts du banc local précédent.

Après précision du propriétaire, la [campagne du pont entre deux instances sur
le Mac](ha-multimaison-pont-local.html) qualifie quatre défauts : fermeture8s,
silence3s avec les mêmes connexions, arrêt du flux B→A8s et silence120s suivi
d’une reprise par instantané. Deux processus autonomes et deux courtiers,
aucun gel des primaires pour la convergence, zéro commande physique.
[Mesures](ha-multimaison-pont-local.json),
[protocole](ha-multimaison-pont-local-protocole.md) et
[preuves](ha-multimaison-pont-local-preuves.json) conservent les temps,
allocations, RSS, CPU et conditions de qualification. Le VPN n’est pas requis
pour ce périmètre local.

Le [premier lot du pont HA↔Moli et de migration](pont-ha-moli.html), ajouté le
28octobre, décrit les deux briques Rust, le compagnon HA, la garde par
génération et le chemin MQTT→Hub→Overkiz simulé. [Résultats](pont-ha-moli-tests.json)
et [preuves](pont-ha-moli-preuves.json) distinguent les487tests Rust, les22tests
Python purs et les5contrats exécutés avec Home Assistant2026.10.4. La
neutralisation réelle du writer HA et la migration de ses automatismes
restent non implémentées ; le vérificateur de transfert est fermé par défaut.
