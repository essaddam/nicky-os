# Idées à reprendre de Moli OS

Relevé du 27/10/2026 dans [rudy-io/moli-os](https://github.com/rudy-io/moli-os) : 6 branches,
20 PR (dont 9 ouvertes). Le but n'est **pas de fusionner** : nikyOS a son propre moteur
(ordres, droits, archive, alarme…). On garde les **idées** — le problème, le mécanisme — et on
les réécrit à notre façon. Les références amont (`commit`, `#PR`) servent seulement à lire
comment ils ont fait.

Chaque idée : **problème** → **mécanisme amont** → **chez nous**.
Pertinence : ★★★ à faire, ★★ à étudier, ★ si le besoin apparaît.

---

## 1. Fiabilité des mesures et des appareils

### ★★★ Détecter un capteur qui se tait (`4c5c273`, PR #9 ; `b7ca706`)
- **Problème** : un seuil ne voit pas une mesure qui cesse d'arriver ; elle reste figée et plausible.
  Cas réel amont : une vanne a chauffé 14 h sur un 25,7 °C immobile.
- **Mécanisme** : nouvelle règle d'automatisme « périmé » : « depuis combien de temps ce point n'a
  pas changé / n'a pas été reçu » ; proposée aussi dans les brouillons de l'assistant.
- **Chez nous** : `moli-automation` n'a pas cette règle. `moli-rollup` calcule déjà couverture et
  fraîcheur (`features/historique-resumes`) : brique de base possible. Concerne surtout le
  chauffage (`moli-voltalis`, `moli-ariston`) et l'alarme.

### ★★★ Retrouver un appareil qui change d'adresse IP (`2cc3b97`, `d54c3de`, `53c1805`)
- **Problème** : nouveau bail DHCP → l'appareil est vu éteint alors qu'il fonctionne.
- **Mécanisme** : écouter ses annonces (Bambu : NOTIFY UDP 2021 avec numéro de série ; Tapo,
  Tuya : rediscovery), suivre par identité (série / MAC) et non par IP ; remettre le délai de
  reconnexion à zéro quand l'adresse est retrouvée.
- **Chez nous** : principe généralisable à tous nos pilotes LAN (bambu, tapo, tuya, roborock,
  samsung, twinkly…). Le suivi par MAC existe déjà côté routeurs (`asuswrt`, `netgear`).

### ★★ Tuya : reprise d'identité sans double connexion (`8ecfed2`, `b4e317f`)
- **Mécanisme** : réserver la connexion pendant qu'on reconfirme l'identité d'un appareil, pour
  ne pas ouvrir deux sessions vers un équipement qui n'en accepte qu'une.
- **Chez nous** : à relire avant la prochaine passe sur `moli-tuya`.

### ★★ Basculer de canal quand une API locale tombe (`a0db1ac`)
- **Problème** : JointSpace (Philips, port 1926) disparaît ~30 s par moments ; la TV passe
  « éteinte » et toutes les touches se grisent.
- **Mécanisme** : envoyer les touches par la session Android TV, plus stable.
- **Chez nous** : `moli-philips` utilise encore JointSpace et nous avons `moli-androidtv-driver`
  natif. Idée plus générale : un appareil joignable par deux canaux ne doit pas paraître éteint
  quand un seul tombe.

### ★★★ Zigbee2MQTT : deux pièges de décodage (#26, #29)
- `contact` est déclaré à l'envers (`value_on: false`) → une porte fermée s'affiche ouverte.
  Point sensible pour l'alarme.
- La luminosité va de 0 à 254, pas 0 à 100 → la tuile affiche « 254 % » et un curseur à 100
  n'envoie que 39 %.
- **Chez nous** : aucun des deux n'est traité dans `crates/moli-z2m/src`. À vérifier sur nos
  fixtures (la sonde `mqtt-live-probe` peut servir).

### ★★ Volets : sens « position » (PR #30, ouverte)
- **Mécanisme** : `Semantic::Position` (0–100 %, 100 = ouvert), `state open/close` traité comme
  ordre et non comme interrupteur ; tuile ouvrir / stop / fermer.
- **Chez nous** : pas de `Position` dans `moli-core/src/spec.rs` ; utile pour `moli-overkiz`
  (volets Somfy) et les volets Zigbee.

---

## 2. Assistant

### ★★★ Ne plus masquer pourquoi l'assistant n'a pas répondu (`5ea8610`, PR #22)
- **Problème** : la maison dit « Je n'ai pas su répondre » alors que le modèle avait répondu,
  ou avait été coupé (limite de tokens, nettoyage trop large).
- **Mécanisme** : `trim_filler` ne doit pas vider une réponse ; une coupure est dite comme telle.
- **Chez nous** : `trim_filler` est inchangé (`crates/moli-assistant/src/lib.rs:1454`).

### ★★★ Paramètres refusés selon le modèle (`2cecf7a`, PR #12)
- **Problème** : le point compatible OpenAI d'Anthropic renvoie HTTP 400 si `temperature` est
  envoyée aux modèles récents.
- **Chez nous** : même famille de contrainte que « Sign in with ChatGPT » (`temperature`,
  `max_output_tokens` refusés, voir `features/assistant-agents`). Idée : une table de capacités
  par fournisseur/modèle plutôt que des cas particuliers.

### ★★★ Niveaux de confidentialité des données (PR #34, ouverte ; issue #33)
- **Mécanisme** : fournisseurs nommés (`[[assistant.provider]]`) et niveaux :
  0 ne sort jamais (secrets, images, position) ; 1 reste à la maison (noms, valeurs) ;
  2 peut partir au cloud une fois pseudonymisé. Le fournisseur choisi dépend du niveau.
- **Chez nous** : rejoint directement le « F0 confidentialité » encore à construire
  (`features/assistant-agents`, `features/services-famille`).

### ★★★ Connexion par abonnement ChatGPT (PR #10, ouverte)
- **Mécanisme** : device code OAuth, jetons dans le coffre, conversation par l'API Responses.
- **Chez nous** : c'est exactement la piste « abonnements au lieu de clés API » étudiée le 19/10
  (`features/assistant-agents`). Le device code règle le problème « Moli tourne sans navigateur ».

### ★★ Arrêter une réflexion en cours (PR #18, ouverte)
- **Mécanisme** : un jeton d'annulation par tour, `POST /api/assistant/stop` qui répond
  `{"stopped": true|false}`.
- **Chez nous** : aucune route équivalente ; utile pour la voix (« stop ») et l'écran.

### ★ Modèle local affiné sur les outils de Moli (issue #33)
- **Constat amont** : sur CPU modeste, aucun modèle local du commerce ne dépasse 90 % d'appels
  d'outils corrects (banc de 48 cas). Proposition d'un petit modèle affiné (~14 000 lignes).
- **Chez nous** : à surveiller ; leur banc de cas est réutilisable pour mesurer nos modèles.

---

## 3. Énergie

### ★★ Production solaire et autoconsommation (`203e71c`, `1a55460`, PR #28)
- **Mécanisme** : rôles de compteur `solar` (production) et `export` (injection, prix de revente)
  → par période : production, injection, autoconsommé, consommation réelle, taux
  d'autoconsommation et d'autoproduction ; `flows` = où va la puissance en ce moment.
  L'interface n'affiche le solaire que si la maison a de vrais panneaux.
- **Chez nous** : `moli-energy` ne connaît pas le solaire (0 occurrence). À reprendre seulement
  si la maison a ou aura des panneaux ; le modèle de rôles se transpose à notre archive.
- Pilotes associés côté amont : `moli-solarman` (Deye, Modbus V5 local, lecture seule),
  `moli-izypower` (cloud, lecture seule).

---

## 4. Personnes et sécurité

### ★★★ Premier propriétaire bloqué sans Cloudflare Access (PR #31, ouverte)
- **Problème** : un e-mail compte comme « peut se connecter » même sans Access ; le code de la
  maison perd ses droits, le bouton « Inviter » disparaît, plus personne ne peut inviter.
- **Chez nous** : **même condition** dans `crates/moli-api/src/people.rs:588-592`
  (`!p.emails.is_empty() || mot de passe`). Correctif amont : un e-mail ne compte que derrière
  `[server.access]`.

### ★★ Famille : invités par pièce et par appareil (PR #20, ouverte)
- **Mécanisme** : page Famille, rôles Administrateur / Adulte / Invité, un invité reçoit une
  liste de pièces et d'appareils précis ; mots de passe Argon2id.
- **Chez nous** : recoupe les invitations datées (`features/invites`) et les droits de la v3
  « ordres des appareils » (non validée). À lire comme retour d'expérience d'interface.

### ★★ Jeton lié au certificat de l'appareil (`cfd1cb8`, Dirigera)
- **Mécanisme** : un jeton n'est utilisé qu'avec l'empreinte TLS sous laquelle il a été obtenu ;
  certificat changé → ré-appairage.
- **Chez nous** : déjà fait pour Overkiz et Samsung (certificats épinglés) ; idée à appliquer
  partout où un jeton local est envoyé.

### ★ Vérification TLS durcie (`767bbde`)
- À relire pour comparer avec notre `PinnedHttps`.

---

## 5. Interface

### ★★ Ne pas afficher ce que la maison n'a pas (#14)
- Cartes Clim, Piscine et tuile Ambilight affichées sans l'appareil. Règle simple :
  une carte n'existe que si l'appareil ou le point existe.

### ★★ Lumières : température de blanc et curseurs (#32)
- Curseur en kelvin borné par le point `color_temp`, dégradé chaud-froid, valeur envoyée
  **au relâchement** (pas de rafale d'ordres), pastilles d'état des capteurs, page Pièces sur
  téléphone.

### ★ Image de caméra par profil déclaratif (PR #16, ouverte)
- Une caméra qui expose une image en HTTP devient source d'aperçu sans pilote Rust.

---

## 6. Nouveaux domaines

### ★★ Cellier : courses du foyer (branche `cellier`, non fusionnée en amont)
- **Mécanisme** : `moli-pantry` (stocks, relevés de prix par enseigne, options de panier,
  écarts, achats), API REST, outils MCP `pantry_*`, 4 vues web, et une skill d'agent qui remplit
  les paniers des drives dans le Chrome de l'utilisateur **et s'arrête avant le paiement**.
- **Chez nous** : cadre bien avec le « majordome asynchrone » (lit, propose, ne commande pas)
  de `features/assistant-agents`.

### ★ Pilotes absents chez nous
- Shelly Gen2+ (websocket JSON-RPC, état poussé, digest SHA-256) ; IKEA Dirigera ;
  Eufy natif, Somfy Protect, pont HomeKit (PR #19). À reprendre seulement si le matériel est
  présent. Somfy Protect pourrait alimenter `moli-alarm`.

### Écarté
- Pilote TaHoma (PR #25) : doublon de notre `moli-overkiz`, plus complet (local + cloud, file
  FIFO à échéance, suivi sans rejeu). Leur gestion du jeton du mode développeur peut être lue.

---

## 7. L'appli cliente (« Moli Maison »)

**Ce qu'ils ont fait** (`mobile/`, Expo 57 / React Native 0.86, iOS en TestFlight, build 8,
dernier changement le 09/10) — identique au `mobile/` de nikyOS :
- **Coquille WebView** : l'appli affiche le tableau de bord web de la maison, pas d'écrans
  natifs. Seuls écrans natifs : choix de la maison (adresse, comme Home Assistant) et demande
  de localisation. Écran d'erreur qui propose de réessayer ou de changer de maison.
- **Téléphone = capteur** : appairage par `POST /api/mobile/register` lancé depuis le tableau de
  bord connecté, jeton dans le trousseau, puis `POST /api/phones/{id}/report` (batterie, charge,
  Wi-Fi, position) au premier plan et en arrière-plan (`moli-phones`). Rapport tous les 5 %
  de batterie seulement.
- **Notifications** par le push d'Expo ; canal « téléphone » des automatismes.
- **Micro** : la page parle à l'assistant via un pont `window.MoliNative`.
- **Accès distant** : Cloudflare Access, ou VPN (WireGuard) vu comme appelant `lan` (issue #11).
- **Vie privée** (`docs/PRIVACY.md`) : rien ne passe par un serveur de l'éditeur ; l'assistant
  cloud ne reçoit jamais la position précise ni le nom du Wi-Fi.

**Ce qu'ils avaient prévu** : rien de plus dans le dépôt (pas de feuille de route, pas de branche
après `appli-maisons`). Seul prérequis écrit : D7 « au-delà d'une maison » de
`docs/ARCHITECTURE.md` → durcissement des bornes et du débit, HTTPS local, **sessions
d'appareil**, avant une exposition plus large (appli, accès distant).

**Idées pour nous** :
- ★★ Le modèle « coquille + téléphone capteur » coûte peu : une seule interface à maintenir.
- ★★ Les sessions d'appareil (D7) rejoignent notre besoin d'un jeton par personne avec
  portées (`features/assistant-agents`).
- ★ Le VPN comme alternative à Cloudflare Access, à documenter.

---

## Annexe : d'où viennent ces idées

| Source amont | État | Déjà chez nous |
|---|---|---|
| `main` | 28 commits depuis l'ancêtre commun `d54c3de` (14/10) | non |
| `demo`, `personnes`, `appli-maisons` | anciennes (09–12/10) | oui |
| `kit-energie` | fusionnée dans `main` (#28) | non |
| `cellier` | 13 commits, hors `main` | non |
| PR ouvertes | #10, #16, #18, #19, #20, #25, #30, #31, #34 | non |
