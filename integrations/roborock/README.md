# Roborock en Mi Home (pilote natif)

- `crates/moli-roborock`. Aspirateurs Roborock **appairés dans Mi Home** (S6 et voisins), par
  le protocole **miIO local** : UDP 54321, le jeton de l'appareil. Aucun cloud. Une instance =
  un aspirateur, appareil `<instance>:<identifiant miIO>`.
- Points : `control` (écriture seule : `start`, `pause`, `stop`, `home` ; sémantique
  `control`), `state`, `error`, `battery` (%), `cleaning`, `fan_power`, `clean_time` (min),
  `clean_area` (m²).
- **Garde** : un agent qui démarre, arrête ou renvoie l'aspirateur attend un humain ; le
  dashboard sans code demande le code. Une pièce protégée reste protégée par la pièce de
  l'aspirateur.
- **Ordres** : file de 4 au plus derrière un seul échange en cours, dans l'ordre reçu ;
  revérifié juste avant de partir (appelant toujours là, avant l'échéance) ; envoyé **une
  fois** (python-miio réessaie trois fois : Moli jamais). `["ok"]` → fait ; refus → sa raison ;
  pas de réponse 400 ms avant l'échéance du hub → « peut-être parti, non confirmé », puis
  nouveau `hello`. Ensuite l'état est relu toutes les 5 s pendant une minute.
- Un statut exploitable contient un état connu et une batterie entière entre 0 et 100 %.
  Une réponse vide ou invalide met le robot hors ligne ; les ordres encore en file sont
  refusés avant émission.

## Repris

- Les **faits** du protocole, lus dans python-miio (GPL-3.0, aucune ligne recopiée) :
  paquet `hello`, en-tête de 32 octets (`0x2131`, longueur, 0, identifiant, horodatage,
  somme MD5 de l'en-tête, du jeton et des données), clé = MD5(jeton), IV = MD5(clé ‖ jeton),
  AES-128-CBC avec bourrage PKCS7, `NUL` après le JSON, horodatage de l'appareil + 1 s.
- Les méthodes `miIO.info`, `get_status`, `app_start`, `app_pause`, `app_stop`, `app_charge`,
  les champs de `get_status` et les tables d'états et d'erreurs des aspirateurs Roborock.
- Le test du chiffrement compare à un paquet calculé à part (OpenSSL et `hashlib`).

## Deviné

- `fixtures/` est **reconstruit** (jeton, réseau et adresses fictifs) d'après le format
  public : aucune capture de l'aspirateur réel. À remplacer par une capture anonymisée au
  premier essai réel.
- `cleaning` : vrai dans les états où l'aspirateur est hors de sa base et travaille.
- L'identifiant miIO (`did`, donné par le `hello`) comme identité : stable tant que
  l'aspirateur n'est pas réinitialisé.

## Reste

- **Qualification sur l'aspirateur réel** (jalon humain H1) : jeton, réponses réelles du S6,
  délais.
- Hors MVP (sous-ticket si demandé) : cartes, pièces et zones, puissance d'aspiration en
  écriture, localiser, consommables, ne pas déranger, station. Les aspirateurs appairés dans
  l'appli Roborock (protocole local Roborock, TCP 58867, clé du compte) : autre pilote.
