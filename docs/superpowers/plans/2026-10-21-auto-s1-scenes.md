# AUTO-S1 — Scènes d’état voulu

Base : socle des dépendances AUTO-H5 intégré. Aucun nouveau stockage, aucune IA
à l’exécution, aucune cible réelle. Le chef autorise l’implémentation sans attente
intermédiaire du cadrage ; Rust relit cadrage et livraison ensemble.

## Contrat retenu

Une scène contient au plus 32 affectations scalaires typées, jamais de toggle ni
d’opération (bouton, incrément, commande temporelle). Les doublons égaux sont
normalisés, les contradictions refusées. Ordre déclaré conservé par appareil,
4 envois simultanés au plus, 4 scènes actives et aucun débordement en file.
Délais monotones : 1–120 s pour le lancement, 1–30 s pour la confirmation.

Définitions : enveloppe, fichier dependencies.json, rédacteur, verrou de graphes,
CAS, aperçu, validation PIN et sauvegarde de H5 réutilisés. La configuration
initialise seulement un brouillon. Publication et lancement montrent les cibles,
leurs membres et leurs pièces ; un changement invalide l’aperçu exact.
Un graphe nomme la scène stable et résout son courant validé à chaque étape.

Lancements éphémères bornés, résultats par cible et journal corrélé. Aucun rejeu
au redémarrage. Un état récent déjà conforme évite un ordre et un faux acquittement. Sinon,
l’acquittement pilote ne vaut pas confirmation d’état : une observation capturée
après admission et avant échéance est nécessaire, y compris si la valeur est
égale et dédupliquée sur le bus. Une commande extérieure sur la portée
invalide l’intention encore en attente. Annulation et échéance suivent le permis
vivant jusque dans les commandes composées et le transport détaché.
Pas de rollback ni de transaction physique multi-appareils.

## Séquence et preuves

1. Contrats purs et tests de bornes/doublons/valeurs.
2. Extension additive du propriétaire H5, reprise et aperçu de portée.
3. Admission, ordonnancement, résultats et confirmation indépendante du bus.
4. Nœud de graphe et index de consommateurs ; annulation transmise.
5. REST/MCP/CLI, éditeur fr/en et OpenAPI.
6. Régressions simulées : conforme, ancien état, absent/hors ligne, erreur
   partielle, concurrence, garde, annulation avant émission, publication/reprise.
7. Fmt et Clippy workspace stricts, tests ciblés -j 2, web build/tests/i18n,
   catalogue et fuites ; commit puis soumission à Rust. Le contrôle complet
   unique appartient au tour d’intégration, sans fusion ni push par l’auteur.

## Précisions d’implémentation

La portée publiée est durable avec le courant et doit toujours correspondre à
la description actuelle. Une portée différente demande une nouvelle révision
humaine ; la vérification à l’exécution ferme aussi la course avec fsync. Les
aperçus affichent leurs noms natifs et unités enregistrés. Les continuations
composées confirmées survivent à l’acquittement parent jusqu’à leur échéance,
avec révocation par une nouvelle cible de même portée, une commande extérieure,
une annulation ou la suspension du propriétaire. L’état déjà observé n’est pas
présenté comme un état simultané garanti de toutes les cibles.
