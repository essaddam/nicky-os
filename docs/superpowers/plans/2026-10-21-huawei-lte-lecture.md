# Huawei LTE : XML HiLink de lecture

Exécution directe du cadrage réseau relu, auteur unique, branche dédiée depuis
`0dbf5b44949a532e9e0be409ae6a7291d22e6a13`. Relecture finale par Rust.
Rebase avant raccordement sur c5f2002ee1cc17d190ae541b741bdea5e9d8856d ;
branche build_driver A–M conservée, insertion alphabétique après son découpage. Les corrections
ASUSWRT priment. Aucun contrôle complet, aucune fusion ni appareil réel par l'auteur.

## Contrat

- Crate native `moli-huawei-lte`, paquet `integrations/huawei-lte/` (D2).
  Source protocolaire : huawei-lte-api f416c63d2a01d7e1d743a2d039ae2d1ff6b71df5
  (LGPL-3.0, connaissance du protocole seulement, aucun code tiers copié).
  Unités et identité recoupées dans HA core cbbd58b4ca6b657168339661a638136ce4387a02
  (Apache-2.0). Fixtures intégralement synthétiques ; aucune analyse privée reprise.
- `host`, `port`, `transport`, `auth` obligatoires, options inconnues refusées.
  HTTP exige allow_plain_auth explicite ; TLS épinglé et persisté avant secrets.
  Période60 s, information3600 s, bornes30–86400 s, fin de cycle sans rattrapage.
- `session = "sestok"` par défaut ou `"token"` explicitement ; aucune tentative
  de variantes en cascade ni lecture de page HTML pour découvrir une route.
- GET SesTokInfo OU token ; GET user/state-login seulement pour auth sha256-v4 ;
  seul POST user/login (password_type4). State0 réutilise la session sans POST,
  State-1 permet ce login ; variante/type inconnu ou autre session refusée = attente.
  auth none n'envoie jamais de POST. Aucun logout ni forçage de session humaine.
- GET device/information, monitoring/status, device/signal, monitoring/traffic-statistics.
  Aucune URL/config dans une réponse n'est suivie. Pas de SMS, SIM/cellule/numéro,
  opérateur, administration, reboot, scan/ping ou test de débit dans les points.
- SerialNumber natif requis et stable, au coffre avant publication ; pas de repli
  IP/IMEI/nom. Noms génériques, aucune métadonnée personnelle en point ou erreur.
- XML strict : 1 Mio avant parsing, profondeur16, 10000 éléments, textes/attributs
  <=4 Kio, doublons directs refusés ; ni DTD, entité déclarée/externe ou réparation.
  Erreurs XML interprétées même sous HTTP200/500. Refus explicite prioritaire sur401.
- Cookies limités à l'hôte, <=8 /4 Kio cumulés ; jetonsASCII <=1 Kio chacun, file2
  au plus, remplacée atomiquement à chaque rotation one/two/single, sans accumulation.
  Matériel de session en mémoire sans Debug ; aucun secret fonctionnel dans un test.
- Une requête en vol, 5 s/échange, 30 s et16 requêtes/cycle, un rétablissement/rejeu
  de la seule lecture expirée. Refus/captcha/verrouillage/variante/identité => humain.
  Panne de transport => superviseur/backoff. Annulation rapide, commandes refusées.
- Au plus11 points : connexion, technologie (code HiLink brut), RSSI/RSRP/RSRQ/SINR,
  octets reçus/émis, débits reçus/émis et durée de connexion. Diagnostic explicite,
  SignalStrength pour les quatre signaux ; Family::Diagnostic vérifiée sur les specs.
- B/B/s/s selon les champs amont ; signal seulement avec unité dBm/dB attestée.
  Valeurs censurées/sentinelles/absentes => inconnu, jamais zéro inventé.
  Compteurs u64 conservés exacts en interne ; publication Int seulement <=2^53-1.
  Débit rapporté en B/s prioritaire, sinon delta entier vérifié sur durée monotone ;
  baisse/temps/session réinitialisés => pas de delta à travers la rupture.
- SIM absente : pas de requête signal/trafic ; connexion de données non prouvée vraie :
  pas de trafic frais publié. Retour SIM/réseau réévalué au prochain relevé de statut.
  Cycle validé avant publication, absences en Null après observation, déduplication.

## TDD et preuve

1. Tests config/XML/modèles/compteurs : observer les contrats rouges, puis implémenter.
2. Session : login figé, cookies/jetons hostiles/rotation, erreurs200, refus sous401,
   un seul rejeu, identité changée, variante inconnue sans POST, capacités absentes.
3. Serveurs loopback HTTP/TLS : bornes, délai/annulation, pin durable avant secrets.
4. Vrai hub/coffre éphémère : 11points RO/famille, cadence/déduplication, SIM/réseau,
   commandes sans réseau, publication atomique, effacement à l'arrêt/échec.
5. Raccordement alphabétique binaire/catalogue/docs/fr/en ; adapter le raccordement
   au découpage build_driver lorsqu'il sera publié, sans toucher à la logique d'autrui.
6. Rust1.99.1 natif, target propre au worktree, -j2 : fmt, Clippy workspace strict,
   tests des crates touchées, catalogue, i18n et fuites. Sources figées/empreintes,
   commit français puis backlog_submit_for_review. Contrôle complet par le fusionneur.

Les tests synthétiques ne qualifient aucun firmware, unité matérielle ni budget réel.
H1 demeure requis, humain présent sur cible neutre et état rétabli ; HA garde ses usages.
