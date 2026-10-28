# Moteur réel, transformations et fenêtres temporelles

Cette campagne complète les microbancs V2–V4. Elle distingue trois preuves :

- le garde-fou du dépôt réel (`scripts/check.sh`), avec les tests de toutes les crates ;
- les chemins réels du hub, des calculs, des automatismes et des codecs, exercés par
  des pilotes et messages synthétiques ;
- les algorithmes de fenêtres glissantes proposés, distincts des fonctions déjà
  raccordées au produit.

Les expériences sont séparées dans `../moteur-integre-banc/`,
`../transformations-banc/` et `../fenetres-temporelles-banc/`. Chaque dossier décrit
son contrat et ses limites. Aucune installation ni aucun appareil réel n'est contacté.
Les commandes sortantes aboutissent à un pilote en mémoire. Aucun LLM ne transforme
une valeur ou une commande dans le chemin d'exécution.

La comparaison d'adaptateurs mémoire alimentant le même hub ne remplace pas les
structures internes du hub : les résultats distinguent leur coût du processus réel.
Une moyenne entre capteurs, une moyenne d'échantillons et une moyenne pondérée par
la durée ne répondent pas à la même question. Un délai de fraîcheur n'est pas une
fenêtre temporelle.

Les fichiers `sorties/` conservent les commandes, résultats, erreurs éventuelles,
empreintes des sources et paramètres de conteneur. Les campagnes figées gardent les
sources des bancs et une référence au dépôt de production utilisé. Une mesure de
latence sur ces données synthétiques ne qualifie pas les appareils et transports réels.

Vérifier les empreintes, les sorties, les agrégats, les oracles et l'absence de chemins
utilisateur dans l'archive v5 :

```sh
python3 tools/moteur-suite-banc/verifier.py
```


## Rejouer depuis la racine

Docker utilise l’image de développement du projet et les volumes `moli-cargo`
(registre) et `moli-target` (compilation). Les locks sont conservés. Le mode offline
suppose ces dépendances déjà disponibles dans le cache, comme pendant cette campagne.
Aucune toolchain Rust locale n’est nécessaire.

```sh
python3 tools/moteur-suite-banc/executer.py workspace
python3 tools/moteur-suite-banc/executer.py build
python3 tools/moteur-suite-banc/executer.py runtime-modes
python3 tools/moteur-suite-banc/campagnes.py
python3 tools/moteur-suite-banc/rapport.py
python3 tools/moteur-suite-banc/verifier.py
python3 tools/moteur-suite-banc/archiver.py
```

Les mesures sont séquentielles pour éviter la concurrence entre nos compilations et
nos mesures. Elles utilisent Linux ARM64, un CPU et 512 Mio ; ce plafond de conteneur
n’est pas le budget de 32 Mio du produit. Le garde-fou complet autorise 6 Gio et un
seul job de compilation : le lien de la suite de tests a dépassé 3 Gio à la première
passe. Cela décrit la compilation, pas la mémoire du moteur en fonctionnement.

`campagnes.py` vérifie les empreintes des binaires avant/après et crée une nouvelle
date de campagne. L’index `sorties/campagnes-v5.json` pointe les trois expériences.
Les binaires compté et sans compteurs sont préservés séparément. Les sources mesurées
et résultats sont conservés ; `archiver.py` ajoute les entrées publiques de compilation
(locales, profils, fixtures) et les contrôles finaux. Son archive doit être extraite
à la racine d’une copie du dépôt au commit enregistré. Les archives existantes ne
sont pas écrasées.

Le débit moteur `raw_observations_per_second` compte les mises à jour numériques
contrôlées, pas tous les événements internes. Les p95 sont empiriques sur dix opérations
par processus, avec trois processus. Les temps de transformation sont des moyennes de
lots puis médianes de sept lots et trois processus, pas des p95 par message.
