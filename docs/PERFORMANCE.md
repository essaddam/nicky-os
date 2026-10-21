# Performances : la règle

Moli OS n'a de sens que s'il reste **très loin devant Home Assistant**, brique après brique.
Chaque livraison est mesurée : après le déploiement, un script côté hôte, **hors de ce
dépôt**, compare Moli à HA sur la même machine, au même moment, et signale tout dépassement
de budget ou toute dégradation de plus de 20 % depuis la mesure précédente. Il ajoute une
ligne JSON par mesure à `data/perf.jsonl` ; Moli ne fait que lire ce fichier (les 20
dernières lignes, telles quelles, servies par `/api/system` et affichées sur la page
Système). Les mesures ci-dessous viennent d'une installation réelle, à côté de son Home
Assistant.

**Une alerte se corrige avant de passer à la suite**, ou s'explique noir sur blanc ici.

## Budget

| Mesure | Budget | Pourquoi |
|---|---|---|
| Mémoire (mémoire **anonyme** du cgroup : tas et piles, aucun cache de fichiers ; **la même définition pour Moli et HA** ; le RSS de Moli est noté à côté) | ≤ 32 Mio (plafond conteneur 128) | HA : ~2,1 Gio. Rester sous 1/50. |
| CPU **au repos** (compteur du cgroup sur 30 s, aucun client connecté) | ≤ 0,5 % | HA : 1,5–2,5 %. Rien ne tourne pour rien. Une mesure faite avec un dashboard ouvert est notée « en usage » (nombre de connexions) et n'est pas jugée contre ce budget. |
| Image | ≤ 40 Mo (25 jusqu'au 17 octobre 2026) | HA : 2,3 Go. Relevé par le propriétaire pour Turso (ADR D13) : les sondes mesurent un delta de +11,4 Mo, projetant le binaire linux/arm64 actuel de 23,1 à 34,4 Mo (35,7 pendant le retour arrière avec SQLite) ; C1 mesurera l’image réelle, symboles gardés. Réévalué à chaque montée de version de Turso. |
| API, p95 | ≤ 25 ms (listes), ≤ 5 ms (santé) | Le dashboard doit répondre au doigt. |
| Démarrage | ≤ 2 s jusqu'à « sain » | Un redémarrage ne doit rien coûter. |

### Réserves de la bascule Turso (18 octobre 2026)

Le [banc D13](annexe-d13-stockage.md) a terminé trois tours avec journal
immuable, en linux/arm64, limité à un CPU : énergie sur 7 jours à
**30,37–31,83 ms** au p95 avant HTTP ; rafale de **949,24–965,17 op/s**
pour 974 visées avec cinq lecteurs. C1/C2 doivent prouver le coût du graphique
et la capacité requise avant bascule. Pic RSS du processus de comparaison :
**53,21–54,32 Mio**, options écartées comprises ; les 32 Mio anonymes du
cœur intégré restent à mesurer sur les seuls chemins retenus. La projection
d'image reste 34,43 Mo (35,69 Mo avec les deux moteurs), à vérifier
sur l'image réelle en C1. Les budgets du tableau restent inchangés.

Le journal conservé sept jours après publication porte le fichier du banc à
**465,14 Mo**, contre 151,74 Mo avant correction ; le chargement de
2 322 732 relevés prend 213,38–251,49 s contre 141,80–161,20 s.
Ce coût d'écriture et de disque assure les réessais identiques et la reprise
après restauration, y compris les imports anciens dont les lignes ont été purgées.

## Principes de conception qui tiennent le budget

- Rien ne se recalcule « au cas où » : ce qui sert à chaque seconde ou à chaque événement
  est mis en cache et recalculé seulement quand il change (ex. : la liste des automatismes
  actifs, voir ci-dessous).
- Une brique inactive ne coûte rien : pas de tâche, pas de minuterie, pas de mémoire.
- AUTO-H4 : l'index `when_occurrence` est construit lors des changements de
  définitions, à partir de la même liste active partagée. Un appui consulte les
  seuls déclencheurs de son point ; aucune empreinte globale n'est recalculée.
  Les fichiers métier restent bornés à 1 Mio chacun ; fenêtres de reçus et de
  consommation bornées, chargement paresseux, aucun sondage ni nouvelle tâche
  permanente. Les synchronisations avant publication/admission font partie de
  la latence de l'appui ; aucune mesure de latence physique ou de production
  n'est déduite des tests simulés.
- AUTO-H2 : propriétaire unique chargé paresseusement, 128 valeurs et 64 fenêtres
  de 32 reçus au plus, fichier plafonné à 1 Mio. Aucune minuterie ni tâche permanente.
  Les mutations attendent le commit atomique ; les tests simulés ne constituent pas
  une mesure de latence sur l'installation réelle.
- Les pilotes écoutent (MQTT, SSE, websockets) plutôt que d'interroger quand l'appareil le
  permet ; sinon intervalle le plus long qui reste utile.
- Le LLM n'est jamais sur un chemin chaud : seulement à la demande d'une personne ou d'un
  automatisme validé (texte).

## Historique

| Date | RSS | CPU | Image | Note |
|---|---|---|---|---|
| 2026-10-02 | 4,3 Mio | 0,00 % | 5,5 Mo | MVP, 7 appareils Zigbee |
| 2026-10-03 | 7,9 Mio | — | — | 38 appareils |
| 2026-10-03 | 12,5 Mio | **0,58 %** | 12 Mo | 71 appareils, 87 brouillons d'automatismes : **régression** |
| 2026-10-03 | 12,0 Mio | 0,25 % | 12 Mo | Corrigé : le moteur hachait (SHA-256) chaque brouillon à chaque seconde et à chaque événement pour savoir lesquels sont actifs ; la liste active est désormais en cache. HA au même instant : 2,14 Gio, 2,4 % CPU. |
| 2026-10-03 | 12,2 → 12,7 Mio | 0,51 → **0,23 %** | 12,1 Mo | Première alerte automatique (0,51 % > 0,5) : mesurée 15 s après le redémarrage, pendant la connexion des pilotes. À froid : 0,23 %, HA 1,92 % (÷8), mémoire ÷360. La mesure attend désormais 2 min 30 de fonctionnement (régime normal). |
| 2026-10-03 | 12,6 Mio | **2,09 %** (en usage) | 12,2 Mo | Alerte expliquée : un tableau de bord resté ouvert montrait une caméra **Reolink en direct** (une image par seconde, 430 Ko/s). Mesuré au cgroup : une image Reolink/s coûte **+1,45 %** (~15 ms par image), une image Frigate/s +0,04 %, un client SSE de plus ~0. Pas une régression du déploiement (le changement ne touche que le traitement des requêtes). Conséquences : le script de mesure prend le CPU au cgroup sur 30 s et note les clients connectés (une mesure « en usage » n'est plus jugée contre le budget de repos) ; le chemin d'image Reolink est à optimiser (connexion réutilisée, cache d'une seconde partagé entre clients). |
| 2026-10-04 | 12,2 Mio (5,1 cgroup) | 0,42 % | 15,7 Mo | Deux dérives attrapées par la mesure : `/api/automations` p95 28,4 ms (> 25) car la recherche des enceintes copiait tout le modèle pour chacun des 87 automatismes → lecture sans copie, capacités calculées une fois par liste (13,5 ms) ; CPU au repos 0,42 % dont ~0,13 % pour le healthcheck Docker (39 ms par appel, toutes les 30 s) → retiré, le chien de garde interroge `/api/health` depuis l'hôte. Image +3 Mo : symboles gardés (`strip = "debuginfo"`). |
| 2026-10-04 | 12,3 Mio | 0,24 % | 15,8 Mo | Fausse alerte « mémoire +39 % » : la mesure façon `docker stats` compte le cache de fichiers actif, qui variait de 4,9 à 8,5 Mio selon ce qui venait d'être lu, RSS stable à 12,3 Mio. La mémoire mesurée est désormais la mémoire anonyme du cgroup, des deux côtés. |
| 2026-10-04 | 4,0 Mio anonyme (12,6 RSS) | 0,24 % | 16,1 Mo | Corrections de la contre-relecture (inconnus refusés, budget de codes, file d'annonces, sauvegarde du plan). Rien de mesurable : HA 2 138 Mio, 1,5 % (mémoire ÷534, CPU ÷6). p95 `/api/automations` 12,5 ms. |
| 2026-10-04 | — | — | 6,5 Mo | arm64 : même `Dockerfile` en `linux/arm64` (musl, `ring`, `rusqlite` bundled) : compile, 21 min sous émulation QEMU sur un PC x86. Pas encore lancé sur un Pi. |
| 2026-10-04 | 4,2 → 4,8 Mio anonyme | 0,33 % | 16,2 Mo | Pilote Telegram natif. Alerte `/api/automations` p95 27,5 ms (> 25) **passagère** : `/api/energy`, non touché, avait doublé au même instant (hôte chargé). Remesure 3 min après : 14,1 ms et 7,6 ms, budget OK. |
| 2026-10-04 | 7,0 → 7,2 Mio anonyme (16,9 RSS) | 0,26 % (en usage) | 17,0 Mo | Pilote téléphones + appli mobile. Alerte « mémoire +43 % » mesurée avec **11 à 12 clients connectés** (réseau local et tunnel : appli mobile, tableaux de bord), bien plus que d'ordinaire ; latences `/api/automations` 28,4 ms à la 1re mesure (hôte chargé, charge 1,7), 14,8 ms à la remesure. Mémoire suivie 2 min 30 : 7,2 Mio stable (7 à 11 connexions), pas de croissance. **À revérifier au repos** (mesure de nuit) : si elle reste au-dessus de 5 Mio sans client, chercher. |
| 2026-10-04 | 6,1 Mio anonyme (15,4 RSS) | 7 % → **0,32 %** | 17,2 Mo | Ambiances, cadran de clim, conseils d'énergie. Alerte au déploiement expliquée : un onglet de vérification était resté sur l'Accueil (image de la sonnette rafraîchie, comme le 3 oct.) : 4 à 9 % tant qu'il était ouvert, 0,1 à 0,5 % une fois fermé. Remesure : budget OK, `/api/energy` p95 7,9 ms. |
| 2026-10-04 | 6,1 Mio anonyme (15,9 RSS) | 0,36 % | 17,5 Mo | Alerte `/api/automations` p95 35,1 ms (> 25), mesurée pendant le démarrage ; hors démarrage 16,4 ms, en hausse depuis 12 ms avec 10 brouillons de plus (103 automatismes). Profil local sur les vraies données : l'**empreinte** (graphe sérialisé puis SHA-256) recalculée 4 fois par automatisme et par listing était le gros du temps, puis la vérification qui copiait tout l'état de chaque appareil cité. Corrigé : `Hub::describe` (description + étiquette, sans l'état), une seule empreinte par vue, puis **empreintes gardées entre deux écritures** (oubliées sous le verrou d'écriture ; valider et lancer recalculent toujours). `/api/automations` **16,4 → 8,5 ms** (p95 9,6). |

## Budget des minuteurs AUTO-H1

Un propriétaire, fichier métier de 1 Mio, 128 minuteurs, 64 clients et 32 reçus par
client. Ordonnanceur sans poll au repos ; projections hors cache périodique et sans
fsync à chaque seconde. En activité, au plus 128 variations de restant par seconde,
visibles en SSE et à l'historique selon B1 : jusqu'à 11 059 200 par jour au plafond.
La rétention B1 n'est pas modifiée. Ces limites ne sont pas une mesure de charge
ou de latence physique et ne remplacent pas une qualification de l'installation.

## Budget des aides temporelles AUTO-H3

128 aides au niveau du hub, fichier de 1 Mio, un seul rédacteur ; aucune éviction
automatique d'une valeur durable. Aucun ordonnanceur, poll ou calcul au repos.
Normalisation aux mutations et comparaisons explicites, publication aux changements ;
aucune écriture du cache périodique pour ces projections. Ces limites ne remplacent
pas une mesure de charge sur une installation réelle.

### AUTO-H5 : chemins simulés et bornes

Les définitions sont chargées paresseusement : sans planning configuré, aucun
ordonnanceur supplémentaire ne démarre. Un seul ordonnanceur couvre les plannings
courants ; les brouillons ne produisent aucune échéance. Le battement de 60 secondes
ne parcourt les prédicats qu’en cas de saut d’horloge ; sinon seuls les plannings dus
sont recalculés. Aucune empreinte de graphe n’est recalculée à chaque tick de planning.
L’index des consommateurs est rafraîchi aux mutations de définitions ; un aperçu humain
revérifie son instantané directement auprès du propriétaire des graphes, sans dépendre
du retard du bus. Le fichier métier est plafonné à 4 Mio et 128 définitions, avec deux
têtes et 256 créneaux chacune. Le test `auto_h5_calendar_budget_at_maximum_slots` rend
mesurables 100 recherches à la borne maximale ; cette mesure native debug ne remplace
ni les mesures cgroup ni la qualification de production, hors périmètre de ce lot.


### AUTO-K1 : propagation indexée simulée

Les calculs réutilisent le registre borné de versions d’AUTO-H5. Une observation ne
réveille que les racines de son index ; les calculs touchés sont triés par rang
précalculé, sans balayage global des définitions. Les échéances ont deux files
ordonnées (monotone et timestamps de cache futurs). La file sale contient 128
identités au plus, les observations/index 4096 sources, et une propagation reste
bornée à 8192 arêtes/lectures sur une profondeur maximale de 16.

Les tests simulés mesurent le cas 128 calculs × 32 sources (4096 lectures), vérifient
qu’un point non indexé ne provoque aucune lecture et qu’une chaîne ne renouvelle
pas la fraîcheur. Ces mesures natives debug ne constituent pas une mesure cgroup
ni une qualification de production. Sans calcul courant, aucune échéance ni
observation de calcul n’est entretenue. Les valeurs dérivées ne sont pas écrites
dans le cache périodique ; les définitions restent dans le fichier métier commun.
### Scènes (AUTO-S1)

Aucune tâche ni horloge de scène sans lancement. Un index des points n’observe
que les cibles actives ; les portées de continuations utilisent des références
faibles bornées à 512 appareils. Quatre scènes et quatre voies au total ; 32 cibles
et 128 appareils de portée par scène. Les portées communes partagent une voie
séquentielle. Les validations, empreintes et écritures de définitions restent
hors du chemin d’observation ; le suivi d’une valeur égale ne fabrique pas un
événement State. Les seules attentes actives sont bornées par le lancement
(120 s au plus). Les 64 résultats éphémères ne déclenchent aucun rejeu.

### AUTO-K3 — couverture et durée bornées

Sans courant `duration`, aucun ordonnanceur ni travail de capture n’est activé.
L’index inverse visite au plus 128 durées dépendantes d’un même point, sans file de
transitions à croissance libre. Chaque durée garde 256 segments au plus ; la perte
par saturation est explicitement de la couverture inconnue. Le rédacteur sérialise
au plus un fichier de 4 Mio par checkpoint ordinaire d’une seconde.

Sonde simulée `auto_k3_bounded_capture_and_checkpoint_budget` : 128 définitions,
256 segments chacune, JSON de **1 263 914 octets**. Sur le poste natif en debug,
une passe a mesuré **50,65 ms** de sérialisation et **86,54 ms** pour 1 000 captures
visant chacune les 128 définitions. Ces chiffres décrivent cette fixture et ce
poste, sans extrapolation à une machine de déploiement. Le test rejoue la borne et
imprime les temps ; il n’impose pas une limite murale instable en CI. L’écriture
réelle inclut fsync et reste hors du verrou de capture. Le bilan conserve sa date
pour rendre un checkpoint lent visible ; les échecs rendent la projection indisponible.
