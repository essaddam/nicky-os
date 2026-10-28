# Alignement : mémoire, accès et écritures concurrentes

Banc autonome, sans modification du Hub ni commande sur un équipement. Les tailles
des huit arènes viennent du manifeste public A/B/somme ; valeurs, requêtes et types
de points sont synthétiques. Les anciennes campagnes ne sont pas réexécutées.

```sh
python3 tools/memoire-structure-v6/alignement-banc/executer.py --image rust:1-alpine
```

Le lanceur utilise l’image présente, figée par son ID, sous Docker : quatre CPU,
affinités logiques 0–3, 768 Mio, réseau désactivé. `--docker-context` permet de
sélectionner un environnement isolé. Aucun compilateur local n’est utilisé.

Six rangements conservent le même courant, précédent distinct, deux dates,
contraintes et Meta32 : Row96 naturelle ; quatre Row96 par bloc aligné sur 128
(pas de 96 entre objets) ; Row128 alignée sur 64 ; Row128 alignée sur 128 ; Row96+Meta32
dans 128 alignés ; Hot32+Cold32+Numeric32 avec Meta32 séparée. Le bloc de quatre
garde un pas de 96 octets et ajoute au plus trois lignes réservées en fin.

Le scalaire passe par la même transition logique, vérifiée contre le vrai Bank
sur 21→22→22→23. Le compteur d’allocation donne les réservations demandées pour
les tableaux ; capacités et alignements réels sont enregistrés, avec slots et
index communs. RSS, mémoire anonyme et pic du processus sont lus séparément.
Ces métriques ne sont pas interchangeables. La préparation et la construction
des index sont hors chronomètre. Les six variantes utilisent les mêmes requêtes.

Les scans lisent état ou ligne complète ; les accès aléatoires lisent état,
état+méta ou passent par l’index commun. Une mise à jour comprend lecture,
transition conservant le précédent, écriture et relecture. L’interface de rangement
est monomorphisée ; le bloc de quatre ajoute sa propre arithmétique d’accès.
Trois processus indépendants et trois rondes sont conservés par mesure, après
chauffe. L’ordre des variantes est inversé au deuxième processus.

## Sonde concurrente distincte

La sonde utilise des mots atomiques relaxés pour éviter toute course mémoire,
pas le verrou du Bank. Un, deux ou quatre threads travaillent chacun sur ses
propres points. Leur affinité et leur CPU de début/fin sont vérifiés. Une barrière
sépare préparation et mesure ; temps de libération et d’attente des barrières
restent dans le temps global. Aucune création de thread n’est chronométrée.

Deux répartitions visitent les mêmes nombres de points : alternance des threads
sur des lignes voisines et blocs contigus par thread. Les blocs commencent sur
des frontières de 256 points. La sonde normale écrit quatre mots aux offsets
payload/précédent/réception/changement (16,24,32,40 dans Row96). La sonde de stress
écrit les douze mots de 96 octets ; ce n’est pas une mise à jour domotique normale.
Hot32 groupé et Hot32 isolé dans 128 octets ne passent que la première sonde.

Ces structures atomiques reproduisent pas et offsets ; elles ne prouvent pas
une transaction atomique de plusieurs champs, ni une migration concurrente sûre
du Hub. Les performances mesurées dépendent de la VM, du placement hôte des vCPU,
des caches et de la charge. Les affinités de vCPU ne garantissent pas l’isolement
de cœurs physiques. Le cache annoncé par macOS est conservé, même si l’invité
Linux n’expose pas sa géométrie. Aucun compteur matériel de défauts de cache ou
d’invalidations n’est mesuré : un écart de temps ne constitue pas à lui seul une
preuve matérielle de faux partage.

Les temps sont des nanosecondes écoulées par opération, pas des cycles CPU ni
des latences réseau. Les minimum/maximum montrent la dispersion des neuf rondes.
Les échantillons, l’environnement et les empreintes restent dans `sorties/`.
Le temps concurrent est divisé par le nombre total d’opérations des threads :
il caractérise le débit agrégé, pas la latence isolée d’une écriture. Les barrières
et le réchauffement initial des caches restent dans ce temps. Les checksums de
chaque ronde sont comparés entre tous les rangements d’une même opération.
