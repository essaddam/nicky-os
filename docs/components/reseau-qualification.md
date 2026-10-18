# Réseau : preuves de contrat et qualification

Matrice à appliquer aux lots du [cadrage](reseau.md). Elle ne rapporte aucun
test exécuté et ne vaut pas une qualification sur matériel.

## Fixtures et traces de tests

Construire des fixtures synthétiques depuis les schémas amont, sans ouvrir une sauvegarde
privée pour en dériver un exemple. Noms génériques, identités fictives explicites ; aucun
SSID, client découvert, numéro, adresse, cookie ou jeton capturé dans une installation.
Les séquences d'authentification de tests utilisent des valeurs factices générées en
mémoire ; aucun secret réel ni secret d'accès fonctionnel dans un fichier ou un test.
Les cas invalides peuvent être construits en mémoire pour ne pas livrer de gros fichiers.
Le README de chaque paquet indique origine, licence, version et limites des fixtures.

Pour chaque cas : entrée, requêtes attendues (méthode/chemin/action), requêtes interdites,
valeurs ou erreur attendues, disponibilité, nombre de points et famille B1. Les corps
contenant des informations d'authentification ne sont jamais affichés lors d'un échec.
Un test négatif doit échouer contre une implémentation qui effectue l'action interdite.

## Matrice préalable, sans appareil

| Lot | Cas | Preuve attendue |
|---|---|---|
| N0 | ancienne configuration, anciens points et unités | mêmes familles et mêmes durées avant/après ; lecture des données existantes |
| N0 | `diagnostic` avec B, B/s, s, %, °C ; signal avec dBm/dB | famille diagnostic prouvée pour les points réseau ; énergie/climat existants inchangés |
| N0 | profil avec nouvelle sémantique/unités, unité erronée | nouveau profil valide ; faute refusée ; catalogue historique valide |
| Tous | instance absente/désactivée | aucune tâche, requête, allocation périodique ni point |
| Natifs | options inconnues, URL au lieu de host, hôte absent, port nul, période trop courte | refus avant connexion ; aucun hôte ni port choisi par le pilote |
| Natifs | secret manquant, empreinte modifiée, HTTP non autorisé | `waiting`/refus avant l'envoi d'identifiants ; aucune rétrogradation TLS |
| Natifs | mauvais mot de passe, verrouillage, boucle de login | nombre de requêtes borné ; attente explicite, aucun verrouillage provoqué par retries |
| Natifs | cookie/jeton trop long, surnombre, CR/LF, rotation | rejet avant en-tête ou remplacement borné ; aucun secret dans erreurs/états/logs |
| Natifs | 3xx vers hôte tiers, adresse dans XML/JSON | aucune redirection, résolution d'entité ni connexion aux adresses retournées |
| Natifs | 1 Mio + 1, réponse sans fin, lenteur, profondeur/texte XML excessifs, DTD/XXE | erreur bornée en taille/temps ; pas de parsing partiel ni d'accès externe |
| Natifs | annulation pendant login/lecture, panne, reconnexion répétée | arrêt rapide, sessions libérées, hors ligne, backoff et aucun rattrapage de lectures |
| Natifs | capacité optionnelle absente, champ inconnu, champ manquant, HTML 200 | pas de point dynamique ni zéro inventé ; pas de stale présenté comme mesure fraîche |
| Inventaire | liste vide sélectionnée, clients inconnus, adresses/noms modifiés | zéro appareil client par défaut ; identité sélectionnée stable ; pas d'accès aux clients |
| Inventaire | 513 clients, doublon d'identité, réponse partielle, échec | cycle inventaire rejeté ; aucune déconnexion inférée à partir du rejet |
| Inventaire | 33 sélections, 32 sélections et capacités maximales | option excessive refusée ; au plus 144 points par instance |
| Inventaire | bail conservé, Wi-Fi dormant, association prouvée, VLAN absent | aucune déduction de présence ; `associated` seulement avec preuve, sinon `Null` |
| ASUSWRT | login réussi/expiré, lecture JSON, HTML/JavaScript inattendu | cookie limité à l'instance ; une reconnexion au plus ; aucune évaluation de code |
| ASUSWRT | injection de paramètre/hook, commande sur chaque point | paramètres fixes ; aucune route d'administration ni requête en réponse à l'ordre |
| Huawei | XML normal, code d'erreur XML en HTTP 200, jeton expiré/one/two | valeurs typées ou erreur ; file de jetons bornée ; un seul rejeu de lecture |
| Huawei | absence de SIM/réseau, sentinelles, unités signal/trafic | absence de mesure explicite ; aucun identifiant mobile publié |
| Huawei | compteur en baisse, u64 excessif, plus de 2^53−1, coupure entre deux lectures | référence remise à zéro ; aucune conversion silencieuse erronée ou débit négatif |
| Netgear | authentifications v1/v2, SOAP Fault, méthode absente | contrats distincts et bornés ; capacité retirée sans appels répétés par cycle |
| Netgear | liste v1/v2, compteurs désactivés, namespace différent | inventaire contrôlé ; absence de trafic distincte de zéro ; aucune activation |
| SMLIGHT | info/sensors publics, identité absente, multichip, champ absent | profil rejoué ; pas de repli IP/nom ; aucun contrôle radio |
| SMLIGHT | 401/403/login HTML, redirect, unité de mémoire inconnue | erreur de compatibilité ; aucun secret ni tentative d'API alternative |
| SMLIGHT | démarrage simultané des deux lectures | espacement conforme à la recommandation constructeur, pas de rafale initiale |
| Tous | relevés identiques pendant plusieurs cycles | aucun événement `state` supplémentaire, aucun `pulse_state` |
| Tous | contrôle des requêtes sortantes | seules les routes/actions figées ; zéro administration/SMS/scan/test de débit |

## Validation de livraison par lot

Sur les octets définitifs et dans le créneau explicitement alloué :

```sh
sh scripts/check.sh
```

Ce garde-fou inclut format, Clippy strict, tests, catalogue et détection de fuites.
Si l'interface a changé :

```sh
cd web
npm ci
npm run build
cd ..
node scripts/i18n-check.mjs
```

Mesurer le coût au repos et sous rejeu de fixtures contre les budgets de
[PERFORMANCE.md](../PERFORMANCE.md) : périodes, requêtes par cycle, points publiés,
événements par cycle et mémoire bornée. Ne pas extrapoler un volume Recorder à un
nombre d'actions physiques ou à un budget CPU Moli.
Le journal précise la base Git, le diff, les empreintes avant/après, la commande réelle,
son code de sortie et les limites de la preuve. Pas de modification pendant le contrôle.
Un check documentaire ne valide pas les clients qui seront livrés dans d'autres lots.

## Porte H1 : uniquement avec un humain

Avant H1 : aucun appel vers les équipements réels, même de simple télémétrie.
Après H1 : le propriétaire fournit l'hôte, le port et les secrets dans le coffre,
confirme le matériel/firmware et les clients à sélectionner. L'onboarding explique les
requêtes lues et l'éventuel HTTP sans TLS ; il n'active aucune fonction sur le routeur.

Sur une cible neutre, humain présent et hors pièces protégées, comparer les valeurs à
la source existante : unités, identité, disponibilité, inventaire sélectionné et volume.
Ne provoquer ni coupure WAN, redémarrage, changement radio ou changement de mot de passe.
Les erreurs/reconnexions sont déjà éprouvées sur le simulateur de contrat ; ne pas
transformer la qualification en test destructif du réseau habité.

Documenter séparément compatibilité firmware prouvée, limites et usages validés. Pour
une présence éventuelle, sommeil, VLAN et distinction bail/association exigent des
preuves spécifiques et un autre accord : les points de ce lot ne remplacent pas la
présence. Après validation seulement, décider du propriétaire unique de chaque usage.
Vérifier l'état initial en fin de qualification ; si un état a changé, le remettre à
l'identique. Aucun déploiement ni bascule de propriété implicite à la revue du code.
