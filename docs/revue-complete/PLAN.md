# Revue complète : méthode et réalisation

## Demande retenue

La revue sert à comprendre et décider. Elle recoupe les documents avec le code et les preuves déjà disponibles et inclut les sources du dépôt citées par la documentation. Précision finale du propriétaire : retenir une seule meilleure idée récente, résoudre les contradictions et achever la conception présentée. Aucun document existant n'est supprimé ou réécrit.

## Livrables

- `index.html` : une cible française unique et cohérente, illustrée, consultable hors ligne, avec vues globales, fonctionnement détaillé, navigation, recherche, filtres d'avancement et impression.
- `donnees/*.json` : synthèse éditoriale structurée et références vérifiables.
- `corpus.json` : inventaire daté des fichiers, empreintes, titres de section et rattachement aux thèmes.
- `construire.py`, `presentation.css`, `interaction.js` : reconstruction sans dépendance externe.
- `VERIFICATION.md` : contrôles de couverture, références, conservation des sources et rendu.

## Étapes

1. Figer l'inventaire initial de `docs/`, sans inclure ce nouveau dossier ; relever les versions et les preuves citées.
2. Lire les familles documentaires en parallèle, conserver les idées héritées et distinguer décision, recommandation, prototype et intégration.
3. Recouper les affirmations structurantes avec le code actuel et les rapports ; signaler les références externes ou absentes, sans les inventer.
4. Construire une cible par thèmes, résoudre les contradictions par des choix explicites et motivés, compléter les contrats manquants, ajouter une galerie des figures retenues et un index exhaustif des sources.
5. Vérifier les liens locaux et les empreintes, tester la recherche et les filtres, contrôler le rendu grand écran et mobile et documenter les limites.

## Choix éditoriaux

- La version récente explicitement substituée fait référence pour la cible ; elle ne prouve pas une livraison.
- Les précisions héritées restent conservées lorsqu'aucun texte ne les remplace.
- Un « décidé ici » d'auteur n'est pas assimilé à une décision explicitement attribuée au propriétaire.
- Les choix nouveaux nécessaires à la cohérence sont attribués à cette consolidation. Ils sont retenus dans sa cible, sans être présentés comme déjà implémentés ni comme des décisions antérieures du propriétaire.
- La lecture principale ne présente pas plusieurs variantes contradictoires. L'historique sert uniquement à expliquer la provenance des choix retenus et à retrouver les documents d'origine.
- Le code vérifié décrit l'existant, les propositions décrivent la cible. Une différence entre eux est un écart de réalisation, pas forcément une contradiction de conception.
- Les résultats de banc sont datés et rattachés à leur environnement. Aucun banc n'est rejoué pour cette revue ; aucune action n'est envoyée aux appareils.
- Les images des versions remplacées restent accessibles dans leurs documents d'origine ; les vues principales utilisent seulement des images compatibles avec la cible retenue.

## Critères de vérification

- Chaque document du corpus initial apparaît dans l'index ; chaque proposition a une synthèse et une filiation.
- Chaque idée a des sources ; chaque affirmation sur le code a une référence actuelle ou une limite explicite.
- Chaque contradiction recensée aboutit à un contrat unique, ses conséquences et son motif de choix.
- Les figures de la revue sont embarquées ; aucune ressource réseau n'est nécessaire à sa lecture.
- Aucun fichier initial n'a été supprimé ou modifié par la construction.

## Présentation par thèmes, 24 octobre 2026

Conserver intégralement les données de la revue et regrouper ses 31 sous-thèmes dans 11 onglets. Chaque onglet introduit l’architecture, le fonctionnement, un chemin principal et un exemple. Garder une vue « Liste complète », la recherche globale, les liens directs, la navigation au clavier et l’impression de tous les thèmes. Le contrôle d’empreintes vérifie que les 220 idées, la consolidation et le corpus initial ne changent pas.
