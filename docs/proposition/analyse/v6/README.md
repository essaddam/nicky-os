# Dossier V6 — ponts MQTT et budget mémoire

Édition du 27 octobre 2026. Ouvrir [index.html](index.html) dans un navigateur.
La page est autonome, sans bibliothèque distante, sans connexion à une maison
et sans commande d'appareil. Les liens de sources sont relatifs au dépôt.

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
