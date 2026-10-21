# ASUSWRT — lecture authentifiée

Client natif `moli-asuswrt`, `kind = "asuswrt"`. Aucun profil TOML : les sessions,
l'identité obligatoire, les erreurs HTTP/JSON et les deltas de compteurs demandent
un état natif (D2). [Installation](onboarding.md) ; [cadrage](../../docs/components/reseau.md).

## Repris

Contrat public [AsusRouter a897f28](https://github.com/Vaskivskyi/asusrouter/tree/a897f28e1e07229a16f3fb553035575fbf2b2328),
notamment `connection.py`, `modules/hooks.py`, `modules/system_status/source.py`,
`modules/traffic/interface/source.py` et `modules/clients/source.py` (Apache-2.0).
Aucun code tiers copié et aucune capture d'installation utilisée.

- POST `/login.cgi`, base64 ASCII `username:password` ; puis POST `/appGet.cgi`.
  Encodage percent de l'intégralité du corps comme `quote` amont (slash conservé),
  Content-Type formulaire, User-Agent `moli-DUTUtil-1`.
- Trois hooks constants : `nvram_get(label_mac/productid/firmver/buildno)` (quatre
  appels distincts), `cpu_usage(appobj);memory_usage(appobj);netdev(appobj)` et
  `get_clientlist()`. Ni hook ni chemin ne proviennent de la configuration.
- Un seul cookie `asus_token`, issu du JSON, ASCII <=1 Kio ; aucun cookie jar ni
  adoption de Set-Cookie, donc sous les plafonds communs 8 cookies / 4 Kio.
- `label_mac` obligatoire et stable au coffre ; noms génériques. Aucun nom/IP/SSID
  découvert publié comme point ; la MAC native ne sert qu'à l'identité Moli.
- JSON strict : corps <=1 Mio avant parsing, profondeur16, 10000 valeurs,
  textes/clefs <=4 Kio, doublons et contrôles refusés. HTML/JS jamais exécutés.
- Un échange en vol, 5 s par échange, 30 s et 16 échanges par cycle. Un seul
  rafraîchissement de session, puis rejeu de la seule lecture expirée. Refus,
  captcha, verrouillage ou changement d'identité/certificat attendent un humain.
  Les pannes réseau passent au superviseur. Aucun logout de sessions humaines.
- HTTP403/423/429 et erreurs JSON3–7/9–11 : refus terminal. Un refus JSON
  reste terminal même sous HTTP401/500. HTTP401 avec JSON valide sans refus ou
  codes de session1/2/8 autorisent la reprise unique ; HTML200 est invalide.
- Par défaut 60 s télémétrie, 300 s inventaire, 3600 s identité ; minimum30 s,
  maximum86400 s, sans rattrapage. Arrêt annule les requêtes et efface les valeurs.

## Points et unités

Au plus **11 points routeur +4 par client**, 32 clients sélectionnés au plus :
**139 points**, sous le plafond144. L'inventaire reste interne (512 entrées max,
rejet intégral au-delà) ; sélection vide par défaut, seul `client_count` est agrégé.
Les identités déjà connues (cache et anciennes sélections inclus) réservent leurs
places : au plus32 clients et un routeur par instance sur les relances. Une sélection
qui dépasserait cette capacité reste en attente, avant toute requête. Aucune suppression
automatique ne libère une place, notamment après une panne ou un inventaire invalide.
Les nœuds AiMesh `amesh_isRe` sont exclus de ce compte. Ce compte décrit les
entrées inventoriées, pas des personnes présentes ni des associations Wi-Fi.

| Points | Contrat |
|---|---|
| `firmware` | texte `firmver` |
| `cpu` | pourcentage issu des deltas de ticks sur le même ensemble de cœurs |
| `memory` | `mem_used / mem_total * 100` |
| `memory_total`, `memory_used`, `memory_free` | KiB amont ×1024 → `B` |
| `rx_bytes`, `tx_bytes` | compteurs hexadécimaux `INTERNET_rx/tx`, en `B` |
| `rx_rate`, `tx_rate` | deltas monotones par durée monotone, en `B/s` |
| `client_count` | nombre d'entrées clients de l'inventaire valide |
| `link`, `signal`, `rate_rx`, `rate_tx` (clients sélectionnés) | type de lien, signal brut, débits de liaison |

Tous les points sont RO et `Diagnostic`, sauf `signal` (`SignalStrength`, sans
unité supposée). Aucun point `associated` ou de présence : `isOnline/isWL` ne
prouve pas une association. `curRx/curTx` suivent la convention **Mibit/s** amont,
convertie en **Mbit/s** (×1,048576) ; cela ne mesure pas un débit Internet.
Les champs absents deviennent Null après observation ; aucun point inventé pour
une métrique jamais prouvée. Un reset de compteur/session ou un changement de
cœurs empêche un faux delta. Les valeurs identiques n'émettent pas de nouvel état.
Un cycle est entièrement validé avant publication ; une liste invalide n'est
jamais publiée comme un inventaire vide.

## Deviné et limites

Les tests figent le contrat amont public, **pas un firmware réellement qualifié**.
La disponibilité des hooks, l'acceptation de l'encodage/User-Agent, les unités
sur chaque firmware et le comportement des sessions simultanées restent à prouver.
Les clefs WAN dupliquées (certains dual-WAN) sont refusées : aucune agrégation
ou sélection d'interface devinée. TLS utilise TOFU sur une cible fournie par
l'humain ; empreinte persistée avant tout identifiant, sans redirection ni repli.

## Reste

Qualification H1 avec humain présent sur cible neutre ; budget CPU/mémoire/réseau
réel ; variantes de firmware et dual-WAN. La température n'est pas exposée dans
ce premier contrat JSON : les routes connues renvoient du JavaScript. Aucun SSH,
Telnet, administration (`apply*.cgi`, netool, shell), scan/ping ou test de débit.
HA garde chaque fonction jusqu'à validation de son remplacement par usage.
