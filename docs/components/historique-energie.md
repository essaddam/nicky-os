# Historique et énergie

## Historique (`crates/moli-history`)

- SQLite `data/history.db`, enregistrement **au changement** (le hub n'émet que les
  changements), `[server] history_days` jours (30 par défaut), affinés par famille et par
  point (ci-dessous), purgé toutes les 6 h. La purge garde, pour
  chaque point, le **dernier échantillon antérieur au seuil** : une courbe qui commence au
  seuil connaît sa valeur de départ, et une valeur stable depuis des mois reste connue.
- Le recorder lit le bus sans jamais l'attendre et passe des lots (2 s ou 1 000 changements)
  à un rédacteur, par une file de 64 lots. **File pleine** : le lot en attente est gardé, dans
  la limite de 4 000 lignes ; au-delà, chaque point n'y garde que son dernier changement
  (`rows_merged`), et s'il reste plus de 2 000 points distincts, les plus anciens sont
  abandonnés (`rows_dropped`). La mémoire reste bornée quoi qu'il arrive. L'énergie a son
  propre abonné au bus : une saturation de l'historique ne la ralentit pas.
- Lecture : `series(point, from, to, max_points)` → valeurs brutes, ou seaux (moyenne, min,
  max, nombre) quand il y en a trop.
- Surfaces : `/api/history` (courbes du dashboard), outil MCP `history` (statistiques +
  série compacte) et **`history_sql`** pour les agents : une seule requête `SELECT`, `ATTACH`,
  `PRAGMA`, `BEGIN` interdits, 2 s et 200 lignes maximum, 64 Ko par valeur, sur la vue
  `history(point, ts, kind, num, txt)`.
- Utilisé par : les courbes, les passages du jour (caméras), « dernier passage » à la porte,
  Moli (questions sur le passé), le nœud d'automatisme… (les automatismes lisent l'état
  courant, pas l'historique).
- Ordre de grandeur : ~2,5 Mo pour environ 70 appareils sur 30 jours.

## Politiques de conservation (`[storage]`, `crates/moli-history/src/policy.rs`)

Sans section `[storage]`, rien ne change : tout est écrit et gardé `history_days` jours
(test `without_a_new_configuration_the_purge_is_the_one_before`, qui compare à l'ancienne
requête).

**Familles.** Chaque point appartient à une famille, déduite de sa sémantique puis de son
unité (`policy::family`), jamais du seul type : le type ne départage que les points sans
sémantique ni unité (un bouton, un texte → états ; un nombre → autre).

| Famille | Contient |
|---|---|
| `power` | puissance, puissance apparente, courant, tension secteur (V) |
| `climate` | température, humidité, pression, luminosité, ppm |
| `energy` | compteurs (kWh, Wh) ; leurs heures vivent aussi dans `energy.db` |
| `states` | marche/arrêt, contacts, présence, fuites, fumée, luminosité et couleur d'une lampe, boutons, textes |
| `diagnostic` | signal, link quality, batterie, tension de pile (`voltage` en mV d'un appareil sur pile, ou un nom de batterie) |
| `other` | le reste : un pourcentage sans sémantique, une mesure en mV qui n'est pas une pile (ORP d'une piscine) |

L'association famille → agrégation (moyenne pondérée, somme, durées, comptage) n'est pas
faite ici : le module de résumés n'est pas encore fusionné, elle revient à son ticket.

**Priorité** : la règle du point (`[[storage.point]]`, la première qui correspond), puis sa
famille (`[storage.policy.<famille>]`), puis `history_days`. Un motif est un identifiant de
point où `*` remplace n'importe quelle suite de caractères (`z2m:*/linkquality`).

```toml
[storage.history]
exclude = ["z2m:*/linkquality", "tapo:*/rssi"]   # plus jamais écrits
include = ["z2m:bureau_routeur/linkquality"]     # écrit même s'il est exclu

[storage.policy.climate]
days = 90

[storage.policy.diagnostic]
days = 7

[[storage.point]]
match = "z2m:cave_thermometre/temperature"
days = 365
```

Validation au démarrage, messages dans la langue de la maison : famille inconnue, durée hors
de 1 à 36 500 jours, motif vide, clé inconnue.

**Exclure n'est pas purger.** Un point exclu n'est plus écrit ; ce qui est déjà enregistré reste
et suit sa durée. L'énergie a son propre abonné au bus : un compteur exclu de l'historique
reste compté exactement (test `an_excluded_point_is_not_written_and_its_energy_stays_exact`).

**Purge par lots.** Point par point, au plus 5 000 lignes par requête, le rédacteur relâché
entre deux lots (2 ms) : les lots du recorder passent entre deux (test
`pruning_goes_by_lots_and_writes_pass_in_between`). SQL portable (pas de `DELETE … LIMIT`),
toujours par la clé primaire, ancrage conservé.

**Garde des réductions** (`crates/moli-history/src/retention.rs`). Chaque point a un
**plancher** dans `history.db` (table `retention`) : la durée pendant laquelle il est gardé
jusqu'ici. Au premier démarrage, tous les points déjà enregistrés reçoivent `history_days`
(Moli avant toute politique) ; un point apparu ensuite commence à sa durée configurée. Une
passe garde chaque point la plus longue de son plancher et de sa durée configurée :

- une durée **plus longue** s'applique tout de suite et **relève le plancher** aussitôt, même si
  une autre réduction attend : la ramener ensuite est une réduction comme une autre ;
- une durée **plus courte** n'efface rien. `GET /api/history/retention` compte, point par point,
  ce qu'elle supprimerait (`reduction.rows`, `points`, période `from`–`to`, points les plus
  touchés) et **garde ce plan** : points, familles, planchers, ancre et nombre de lignes de
  chacun. La page Système l'affiche dans « Stockage ». Une personne le valide avec son code :
  `POST /api/history/retention/validate` `{ "at", "fingerprint" }` (session PIN exigée, 403 sinon,
  même depuis un tableau de bord de confiance). La validation applique **ce plan seul** : elle
  abaisse les planchers de ses points et supprime leurs lignes antérieures à l'ancre comptée,
  **jamais plus que le nombre annoncé** (une écriture arrivée entre-temps n'élargit rien). Les
  autres points attendent leur passe ordinaire. Un décompte plus récent remplace celui affiché,
  un plancher changé entre-temps refuse la validation (409), un décompte de plus de 15 min aussi
  (410) : la page montre alors la nouvelle proposition et demande un nouveau clic ;
- validation et passes prennent le même verrou, tenu du contrôle du plan jusqu'à la dernière
  ligne supprimée : aucune passe ne s'intercale ;
- un agent (`api`, `mcp`, `cli`, `assistant`) ne valide jamais : `History::validate` refuse
  toute origine autre qu'une personne avec code, et aucun outil MCP ne l'expose ;
- un point dont l'appareil n'est pas (encore) publié, juste après un démarrage par exemple,
  est gardé la plus longue durée de toutes les familles et n'entre dans aucun plan ;
- chaque validation est journalisée (`retention_log` : quand, qui, points, lignes) ; la
  dernière est rendue par `GET /api/history/retention` (`last_validation`).

**Échantillonnage des courbes bavardes : non fait.** Il n'est prévu que si le relevé de la
maison montre des points à plus d'un changement par seconde. Ce relevé (mesure réelle après
déploiement) n'existe pas encore ; `top_24h` sur la page Système le donnera.

## Croissance visible (`/api/system`, page Système)

Les briques `history` et `energy` de `/api/system` donnent, dans `facts` :

- `db_bytes`, `wal_bytes` (journal), `free_bytes` (espace libre du dossier de données) ;
- `rows_24h` : lignes écrites sur 24 h (énergie : heures de compteur touchées) et `query_ms`,
  le coût de ce comptage ;
- historique : `top_24h` (les 10 points les plus bavards sur 24 h, `[point, lignes]`),
  `last_prune` (`at`, `ms`, `rows`), `batches_written` / `batches_failed`, `rows_merged` /
  `rows_dropped` (saturation, ci-dessus), `last_error` (`[ms, message]`) ;
- énergie : `writes_ok` / `writes_failed`, `last_error`.

Tout est calculé à la demande, sans minuterie. Les compteurs sont en mémoire depuis le
démarrage et toujours renvoyés. Si le comptage sur 24 h échoue, `rows_24h` (et ce qui
l'accompagne) est **absent** et `volume_error` dit pourquoi : la page affiche « indisponible »,
jamais zéro.

- Historique : le volume sur 24 h est recompté au plus une fois par minute, **un comptage à
  la fois** (les appels qui arrivent pendant un comptage l'attendent et le reprennent ; un
  échec n'est pas gardé). Une plage de clé primaire par point, jamais un parcours de toute la
  table : ~2 ms pour 100 000 lignes sur 24 h dans une base de 3 millions (mesure locale, à
  confirmer sur la maison avec `query_ms`). `counted_at` dit quand il a été compté.
- Énergie : pas de cache, le comptage passe par l'index `hour` sur quelques centaines de
  lignes (compteurs × 24 h) ; son coût est mesuré à chaque appel (`query_ms`).

La page Système les montre dans « Stockage ».

## Énergie (`crates/moli-energy`)

Les compteurs cumulés deviennent des **kWh et des euros heure par heure**, gardés à vie dans
`data/energy.db`.

- Configuration `[energy]` de `moli.toml` : tarif (période courante lue sur un point, prix
  par période : par exemple HC 0,14 €, HP 0,17 €), et `[[energy.meter]]` avec un rôle :
  - `grid` : ce que le fournisseur facture (index Linky HC / HP, au kWh près) ;
  - `total` : toute la maison mesurée finement (par exemple un compteur EM06 sur l'arrivée
    générale, au Wh) ;
  - `circuit` : un circuit du tableau (par exemple « Chauffe-eau »).
  - `appliance` : un **appareil** mesuré à sa prise (par exemple la télé) : affiché
    **dans** son circuit (`within = "<id du circuit>"`) ou dans « le reste de la maison »
    (sans `within`), jamais ajouté au total ni retranché du reste (pas de double compte).
  - `integrate = true` : `point` est une **puissance** (W) ; Moli en fait l'énergie
    (puissance tenue × temps, au plus 15 min d'un coup, un tic toutes les 5 min, rien de
    compté hors ligne ni pendant un arrêt de Moli). Pour les prises Tuya, dont `add_ele`
    repart de zéro à chaque envoi et n'est pas un compteur fiable.
  - `estimate = true` (appareil seulement, sans `integrate` ni `power`) : une **lumière connectée** qui ne mesure rien ; `point` = un point de la lampe ou d'un luminaire `lumieres:…`. Moli estime sa puissance d'après le profil mesuré de son modèle (powercalc, licence MIT, condensé par `scripts/light-profiles.py` dans `crates/moli-energy/src/estimate.rs` : LCA006, LTW001, LWA017, L530) : luminosité, blanc ou couleur, veille éteinte, **0 quand elle est coupée au mur** ; un luminaire = la somme de ses ampoules. Comptée dans le temps comme `integrate`, jamais ajoutée aux totaux, `estimated: true` dans l'API, « ≈ estimé » à l'écran. Modèle inconnu : pas d'estimation (jamais de chiffre inventé).
  - Pas repris de HA : ses capteurs « powercalc » (estimations, pas des mesures).
  - `feeds = ["<device id>"]` : les appareils qu'un compteur alimente (par exemple le
    circuit d'une clim → cette clim) : leur carte montre sa puissance (« circuit partagé »
    s'il alimente autre chose). Les adaptateurs Daikin BRP069 vérifiés ne mesurent **pas**
    leur conso (`get_day_power_ex` / `get_year_power_ex` à 0 même clim en marche,
    `cmpfreq=999`) : seul le tableau la donne.
  - `[energy.live]` (optionnel) : `power` = le point de puissance instantanée du compteur du
    fournisseur (Linky PAPP, en VA), `max` = l'abonnement dans la même unité (par
    exemple 9 000 pour un abonnement de 9 kVA), `warn` = à partir d'où la conso est
    « soutenue » (par exemple 2 000 ; défaut un quart de `max`) : la jauge vire à l'orange,
    puis au rouge au double. Sert la jauge « en direct » et la ligne du prompt de Moli.
  - `monthly_fee` (optionnel) : l'abonnement par mois TTC (par exemple 20 €) : la facture
    estimée = kWh du mois au rythme actuel + abonnement, comme le « Projection facture fin de
    mois » de HA. `budget` (optionnel) : le point qui tient le budget mensuel (par exemple
    une aide `maison:budget_elec_mensuel/value`).
- **Appareil « Énergie »** (`energie:maison`, `points.rs`) : Moli le fait tourner tout seul
  dès que `[energy]` existe (l'id de pilote `energie` est réservé). Points en lecture seule,
  recalculés toutes les 5 min : `today_kwh/_cost`, `yesterday_kwh/_cost`, `month_kwh/_cost`
  (kWh seulement), `projection_kwh`, `bill_projection` (abonnement compris, absent le premier
  jour du mois), `over_budget` (si `budget` : facture estimée > budget, jamais avant 5 jours
  écoulés, comme HA qui attendait le 6). Sémantique `other`, jamais `energy` : ce sont des
  résumés, pas des compteurs à additionner. Sert les automatismes (par exemple une alerte de
  budget mensuel) et les agents.
- Robustesse : saut impossible (> 36 kW) ou baisse = suspect ; remise à zéro comptée une fois
  confirmée ; nouveau socle après 3 relevés cohérents ; chaque heure au prix de sa période
  (l'index HC n'est étalé que sur du temps HC).
- Chiffre principal = la plus complète des deux mesures (`grid` et `total`, par exemple Linky
  et EM06).
- **Historique repris de HA** (statistiques long terme, lecture seule) :
  `tools/home-assistant/ha-energy-export.py … | docker exec -i moli-os /moli-os energy
  import`, relançable sans doublon.
- Surfaces : `/api/energy` (en direct, aujourd'hui, hier, mois + projection, mois dernier,
  part non mesurée), `/api/energy/series?step=hour|day|month`, outil MCP `energy`, page
  Énergie, carte Accueil ; la ligne « Électricité en direct » du prompt de Moli.
- **Brique future : production** (panneaux solaires, injection, batterie).
