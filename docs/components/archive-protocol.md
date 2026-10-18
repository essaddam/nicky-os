# Contrat HTTP du service d’archive

Complément d’implémentation de [D13](../ARCHITECTURE.md#d13--stockage--le-récent-sur-turso-le-long-terme-dans-un-service-darchive-17-octobre-2026).
Le cœur et l’archive ne partagent aucun fichier. C2 reste limité à l’envoi des lots et au
statut ; D1 branchera le catalogue et les lectures. Un service possède une installation
(identifiant de 128 bits, 32 caractères hexadécimaux minuscules). Une autre installation
utilise un autre volume/service ; une identité différente est refusée.

## Lots et accusés — D13 inchangé

`POST /v1/installations/{installation}/batches/{first}-{last}`, corps gzip NDJSON.
Chaque ligne est une opération `sample` ou `energy_hour` du format D13. Les séquences
sont strictement contiguës ; aucune opération n’est reconstruite depuis une table mutable.
Les limites sont de 5 000 opérations, 1 Mio compressé et 8 Mio décompressés. Les temps
sont des entiers positifs en millisecondes UTC ; une heure énergie est alignée sur
3 600 000 ms. Les identifiants de point/compteur sont des chaînes bornées, jamais des chemins.
Les nombres doivent être finis ; un coût absent reste inconnu et ne devient pas zéro.

La clé d’idempotence est l’installation, les bornes et SHA-256 des octets NDJSON
originaux après décompression (retours à la ligne compris). Le serveur calcule le hash.
Aucun en-tête supplémentaire n’est demandé. Même lot : statut courant, sans nouvelle
écriture. Même plage avec autre contenu, chevauchement ou trou : HTTP 409 et statut
actuel. Après une erreur réseau, le client lit le statut puis repart de `durable_upto+1`.
Un doublon ne fait jamais reculer l’accusé. Les reçus publiés sont conservés au plus sept
jours, et peuvent être évincés plus tôt sous quota. Hors de cette fenêtre, HTTP 409 avec
le statut courant : l’identité ancienne ne peut plus être vérifiée.

`GET /v1/installations/{installation}/status` et accusé de lot :

```json
{"durable_upto":5000,"published_upto":0,"generation":123}
```

`generation` est l’époque de restauration, stable lors des publications, redémarrages et
déplacements. La révision renvoyée par le catalogue et les lectures identifie un manifeste publié dans cette époque ; elle ne figure pas dans le statut D13. Les deux sont
des entiers signés positifs (révision initiale zéro). Restaurer une ancienne sauvegarde
impose une nouvelle époque. Le journal durable ne prouve pas la publication : seul le
manifeste intégralement validé autorise l’avancée de `published_upto`.

Les événements bruts de séquences différentes sont conservés, y compris à horodatage
identique. Pour `energy_hour`, la séquence maximale remplace **l’état complet** de la même
paire compteur/heure (kWh et coût), sans addition des versions. Un import répété avec de
nouvelles séquences doit donc renvoyer les heures énergie comme des remplacements ; la
déduplication métier des événements importés appartient au producteur.

## Catalogue et lectures — complément C1/D1

Ces routes ne changent pas le journal D13. Le catalogue fait partie du manifeste figé
pour une lecture ; `catalog.json` est une copie de consultation, jamais l’autorité de reprise.
Le catalogue est borné à 4 096 points et 2 Mio de JSON ; un point indique `id`, `family`,
`unit`, `aggregation`, `freshness_ms`, `origin`. `aggregation` peut être absente : le brut
est conservé dans la famille `other` (30 jours par défaut B1), sans inventer de résumé. Un index énergétique cumulatif ne prend
jamais l’agrégation `sum`. La consommation par heure est portée par `energy_hour`.

`PUT /v1/installations/{installation}/catalog` reçoit un catalogue complet et
`expected_revision`. Un catalogue tardif ou modifié provoque le recalcul des résumés
concernés, sans modifier les opérations reçues ni leur hash. Le nouveau catalogue et ses
résumés deviennent visibles ensemble dans un manifeste. Une révision devenue ancienne
renvoie 409. Le fuseau IANA sert aux jours et mois locaux ; les heures restent UTC.
`GET .../catalog` renvoie le catalogue du manifeste courant et sa révision.

`POST /v1/installations/{installation}/query` :

```json
{"source":"samples","points":["demo.power"],"from":0,"to":3600000,"resolution":"hour","max_points":200}
```

`source` distingue `samples` et `energy`, même si un point et un compteur portent le même identifiant.

Résolutions : `raw`, `hour`, `day`, `month`. Les bornes des résumés doivent coïncider avec les limites de leur résolution (heures UTC, jours/mois locaux). Une période partielle demande le brut ou une résolution plus fine ; aucun total de période entière n’est renvoyé comme s’il était tronqué. Période demi-ouverte `[from,to)`, au plus
128 identifiants et 5 000 valeurs au total, réponse JSON au plus 1 Mio. Une réponse
porte l’époque, la révision, les bornes réellement couvertes, l’origine et la qualité
par série. Un résultat qui dépasserait le budget est refusé explicitement ; aucune
troncature silencieuse ni agrégation arbitraire. `raw` conserve les versions des événements
et résout les heures énergie ; les résumés exposent les valeurs A3 et leur couverture.
Un point inconnu ou sans règle indique `unknown`, jamais un zéro synthétique.

Chaque réponse contient aussi le `published_upto` de son manifeste figé : D1 peut ainsi superposer les opérations locales plus récentes sans se fier à un statut lu à un autre instant.

Chaque requête épingle un manifeste avant sa lecture. Une publication concurrente
n’ajoute ni ne retire une partie de son résultat. Les fichiers d’une génération épinglée
par un lecteur ou une future sauvegarde ne sont pas collectés.

## Ressources et erreurs

Bearer obligatoire pour toutes les routes, jeton lu depuis un fichier secret. Bouclage
par défaut ; sur VPS, TLS est assuré par le mandataire inverse et le port du service
reste privé. Aucun jeton ou corps d’opération dans les journaux.

Un travail de données à la fois, admission avant allocation du corps. Les dépassements de
corps renvoient 413, entrées invalides 400, conflits 409, saturation 503. Une erreur I/O
rend les mutations indisponibles jusqu’à réouverture, pour éviter de publier un état
ambigu. Le statut reste rapide ; les limites de stockage et les générations conservées
sont des erreurs explicites. La purge du détail relève d’E1.

Les statistiques de somme dans les métadonnées de fichiers vérifient les lignes physiques. Elles ne se somment pas entre fichiers versionnés : les lectures et résumés résolvent d’abord la dernière version de chaque heure.
