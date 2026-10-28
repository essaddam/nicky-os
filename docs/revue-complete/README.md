# Revue complète

Ouvrir `index.html` dans un navigateur. La revue expose une seule cible consolidée avec ses vues globales et son fonctionnement détaillé. Elle sépare l'état du code, les décisions rapportées et les choix de conception retenus pour résoudre les contradictions.

Les sources et les preuves restent dans leurs dossiers d'origine. Les figures utilisées sont intégrées à la page pour permettre une lecture hors ligne.

La lecture commence par 11 onglets thématiques. Chacun explique son architecture, son fonctionnement et un exemple concret avant les idées détaillées. L’onglet « Liste complète » conserve les 220 idées et les 31 sous-thèmes. La synthèse, la vue globale cliquable, les six parcours, les 75 figures, les 34 décisions de consolidation et l’index des 163 sources restent accessibles. Les détails et les sources s'ouvrent dans la page ; la recherche accepte les mots avec ou sans accents. L'impression développe les détails.

Reconstruction : `python3 docs/revue-complete/construire.py` depuis la racine du dépôt. Contrôle statique : `python3 docs/revue-complete/verifier.py`. Les contrôles de navigateur et leurs limites sont consignés dans `VERIFICATION.md`.

Voir `PLAN.md` pour les critères et les conventions de lecture.

Les onglets se parcourent aussi au clavier (flèches, début, fin). Un lien direct vers une idée ouvre son thème ; la recherche couvre tous les thèmes et l’impression les réunit. Les empreintes des données avant cette réorganisation sont dans `preservation-contenu.json`.
