# Nuki (pilote natif)

- `crates/moli-nuki`. Serrures connectées appairées à un pont Nuki matériel, par l'API HTTP
  locale du pont. Une serrure = un appareil `<instance>:<numéro Nuki en hexadécimal>`.
- Points : `locked` (binaire, lecture et écriture, sémantique `lock` : vrai = verrouillée),
  `state` (énumération des états Nuki), `unlatch` (écriture seule, sémantique `lock` : tire le
  pêne), `battery_low`, `observed` (dernière fois que le pont a entendu la serrure) ; selon ce que la serrure rapporte : `battery` (%), `charging`, `door`
  (capteur de porte, contact : vrai = fermée), `keypad_battery_low`.
- **Garde** : un point de sémantique `lock` qui ouvre (déverrouiller, tirer le pêne) est retenu
  pour un humain **quelle que soit la pièce**, pour un agent comme pour le dashboard sans code
  (`crates/moli-runtime/src/hub.rs`, `Hub::sensitive`). Verrouiller passe sans friction. Tout
  ordre, retenu ou non, est au journal.

## Repris

- La documentation publique de l'API (Bridge HTTP API 1.13.2) : `/list`, `/lockAction`,
  jeton haché (`ts`, `rnr`, `hash` = SHA-256 de `ts,rnr,jeton` ; le test reprend l'exemple
  chiffré de la doc), types d'appareils, états, actions, états du capteur de porte, codes
  d'erreur (401/403 refus, 503 pont occupé : une seule requête à la fois).
- Le choix de lire `/list` (état mis en cache par le pont) plutôt que `/lockState`, comme
  l'intégration HA `nuki` : la doc avertit que `/lockState` vide les piles.

## Deviné

- `fixtures/list.json` et `fixtures/lockAction.json` sont **construits sur les exemples de la
  documentation**, avec des valeurs fictives : aucune capture d'un pont réel n'a encore été
  faite. À remplacer par une capture anonymisée au premier essai réel.
- Le modèle affiché selon `deviceType` (0, 3, 4) ; un type inconnu est ignoré, jamais deviné.
- `nowait=1` sur `/lockAction` : l'ordre est accepté par le pont, l'état réel arrive par la
  lecture suivante (toutes les 3 s pendant 30 s après un ordre).
- « En ligne » = le pont répond et connaît un état de la serrure ; ce n'est **pas** une
  confirmation récente de la serrure elle-même : c'est ce que dit `observed`. Les valeurs ne
  sont republiées que quand le pont a vu du nouveau.

## Reste

- **Qualification sur le pont réel** (jalon humain H1) : jeton haché accepté par le
  micrologiciel actuel, délais réels, comportement de « déverrouiller » sur une serrure
  réglée pour tirer le pêne au déverrouillage.
- Le **jeton chiffré** (`ctoken`, XSalsa20-Poly1305, ponts ≥ 1.22.1 / 2.14.0) : la doc
  déclare le jeton haché « déprécié ». À ajouter si le pont réel le refuse (une dépendance
  de chiffrement en plus).
- Les rappels du pont (`/callback/add`) pour un état instantané ; l'Opener ; le pont
  logiciel des Smart Lock 3.0 Pro sans pont matériel (Nuki Web / MQTT, autre transport).
- Une carte « porte » dédiée dans la Maison ; aujourd'hui l'Atelier montre l'interrupteur
  `locked` et le dashboard demande le code pour ouvrir.
