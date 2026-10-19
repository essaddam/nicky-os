# Netatmo météo — plan d’implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans.
> Exécution dans cette session, sans nouveau worker, puis revue architecturale.
> Plan à relire avant implémentation ; aucun accès fournisseur réel pendant le développement.

**Goal:** Livrer le pilote natif de station météo Netatmo et de ses modules extérieur et pluie,
avec autorisation humaine, renouvellement durable et mesures dont la fraîcheur est vérifiable.

**Architecture:** Une tâche supervisée par instance possède la session OAuth et sérialise
les échanges fournisseur. Une passerelle bornée relie cette tâche aux routes d’autorisation
humaine ; seul le pilote persiste les jetons par `DriverCtx::store_secret`. Le décodeur météo
est pur et ne transforme ni une réponse HTTP réussie en mesure fraîche, ni une absence en zéro.

**Tech Stack:** Rust 2024, Tokio, Hyper via `moli-net`, Axum, Serde, Ring, Svelte 5 et
catalogues fr/en existants ; aucun nouveau SDK OAuth ni moteur de stockage.

**Spec:** `docs/components/capteurs-lumieres.md`, sections Netatmo, B1 et vérification ;
`docs/ARCHITECTURE.md`, D1/D2/D5 ; sous-ticket Netatmo et précision `DriverCtx::store_secret`.
Base de préparation : `34b03b2`, branche `integ-capteurs-netatmo`.

## Global Constraints

- Lecture seule : station, module extérieur et pluviomètre (`NAMain`, `NAModule1`, `NAModule3`).
  Autres types ignorés sans capacité inventée ; ni caméra, projecteur, chauffage, webhook,
  média, station publique tierce, reprise d’historique ou substitution Open-Meteo.
- `read_station` seul ; `api.netatmo.com:443`, chemins fixes `/oauth2/authorize`,
  `/oauth2/token` et `/api/getstationsdata?get_favorites=false`. Pas de destination configurable
  pour le fournisseur, ni redirection HTTP suivie.
- L’humain fournit `client_id` et `client_secret` au coffre de l’instance. Le pilote conserve
  un seul secret `oauth_tokens` versionné contenant le jeu de jetons et son expiration.
  Aucun jeton, code ou état OAuth dans les journaux, messages d’erreur ou fixtures commitées.
- Le retour enregistré chez Netatmo est une option d’installation humaine `public_origin`
  (origine HTTP(S) seule, sans chemin, userinfo, query ni fragment). Le code y ajoute le chemin
  fixe `/api/integrations/netatmo/<instance>/callback`. HTTP seulement en boucle locale ;
  HTTPS ailleurs. Cette configuration ne déclenche aucune requête vers cette origine.
- Défauts de fonctionnement proposés : interrogation toutes les 300 s, donnée périmée après
  1 800 s. Options strictes : `poll_seconds` entre 60 et 3 600 ; `stale_seconds` entre 300 et
  86 400 et au moins deux périodes de lecture. Tolérance d’horloge future : 300 s.
- Bornes proposées : corps météo 512 Kio ; réponse OAuth 16 Kio ; délai réseau global 10 s ;
  16 stations, 16 modules par station et 128 appareils connus par instance ; identifiant
  ASCII sans `/`, au plus 64 octets ; nom nettoyé de ses contrôles et limité à 128 caractères.
- Autorisation : aléa de 32 octets, état hexadécimal de 64 caractères ; une tentative en attente
  par instance, expiration de 600 s ; boîte de contrôle de quatre requêtes au plus, requête
  abandonnée avant envoi refusée. Code OAuth ≤ 2 048 octets, jeton ≤ 4 096 octets,
  corps des routes d’autorisation ≤ 8 Kio ; durée de jeton reçue entre 60 et 86 400 s.
- Aucune compilation ni vérification lourde avant allocation de la coordination (chef après reprise). Validation
  Linux sur sources figées, un CPU/un job/tests séquentiels ; `sh scripts/check.sh` avant commit.
- Tous les messages visibles passent par `t`/`tr!`, en fr/en. Listes partagées insérées dans
  l’ordre alphabétique. Aucun changement du budget de performance ni de la conservation B1.
- Développement avec données synthétiques et faux serveurs seulement ; aucun compte réel,
  secret humain, contact de l’API fournisseur, matériel ou déploiement avant qualification autorisée.

## Review Focus

- Deux sessions humaines ouvertes depuis la même adresse : l’une ne termine jamais l’autorisation
  ouverte par l’autre ; logout, changement de PIN et expiration invalident le retour.
- Retour depuis un site extérieur avec cookie `SameSite=Strict` : ne pas affaiblir les cookies
  existants ; la page de retour effectue une seconde requête de même origine.
- Réponse perdue après rotation côté fournisseur : l’ancien jeton peut déjà être invalidé ;
  ne pas rejouer aveuglément le renouvellement ni promettre une récupération sans nouvelle autorisation.
  Échec avant envoi (DNS, connexion, TLS) : conserver les jetons et retenter après backoff.
- HTTP 200 répété avec le même `time_utc`, champ supprimé ou station inaccessible : la fraîcheur
  ne recule jamais, la valeur absente devient inconnue et l’appareil périmé passe hors ligne.
- Changement de nom ou d’ordre des modules, identifiants dupliqués et champs fournisseur ajoutés :
  identités conservées, réponse ambiguë rejetée, champs inutiles ignorés sans publication de données personnelles.

## Sources à figer

- [OAuth officiel](https://dev.netatmo.com/apidocumentation/oauth), consulté le 18 octobre 2026 :
  Authorization Code, vérification de `state`, rotation et jetons précédents invalidés.
- [API météo officielle](https://dev.netatmo.com/apidocumentation/weather).
- [Client météo officiel, révision 9694823](https://github.com/Netatmo/Netatmo-API-PHP/blob/9694823b9dee4effd25ce4b43a2431fb2447ab15/src/Netatmo/Clients/NAWSApiClient.php) :
  `getstationsdata`, choix explicite de ne pas demander les favoris publics.
- [Intégration HA 2026.10.0](https://github.com/home-assistant/core/tree/2026.10.0/homeassistant/components/netatmo) :
  référence de sémantique, dépendance déclarée `pyatmo==9.9.0`, sans reprendre les fonctions hors périmètre.
- La documentation dynamique météo ne fournit pas à elle seule des captures : les fixtures seront
  synthétiques, avec provenance, schéma utilisé et limites déclarés ; aucun extrait de l’inventaire privé.

## Fichiers et interfaces

| Fichier | Responsabilité et contrat |
|---|---|
| `crates/moli-netatmo/src/config.rs` | `Config`, validation des options et construction du callback fixe pour une instance |
| `crates/moli-netatmo/src/model.rs` | `decode(bytes: &[u8], now_s: u64, stale_s: u64) -> Result<Weather, Error>` ; descriptions et mesures typées, disponibilité et dates d’expiration |
| `crates/moli-netatmo/src/oauth.rs` | `PendingAuthorization`, validation/consommation liée à une empreinte de session ; `TokenSet`, décodage et expiration, `Debug` expurgé |
| `crates/moli-netatmo/src/api.rs` | client privé à destination fixe ; réponses et erreurs classées sans conserver les textes d’erreur externes |
| `crates/moli-netatmo/src/gateway.rs` | `Gateway::authorize(binding)` et `Gateway::complete(binding, callback)`, asynchrones, file de quatre et échéance ; statut sans secret |
| `crates/moli-netatmo/src/lib.rs` | `Netatmo::new(Config) -> Result<Netatmo, Error>`, `gateway() -> Gateway`, `Driver::run` ; un acteur possède les jetons et le coffre |
| `crates/moli-netatmo/src/tests.rs` | tests du vrai pilote avec transport simulé privé et coffre réel temporaire ; injecter les pannes par les surfaces de test, pas par des options réseau de production |
| `crates/moli-api/src/netatmo.rs` | routes humaines, page de retour statique, association à la session PIN et traduction des erreurs sûres |
| `crates/moli-api/src/session.rs` | empreinte de la session réellement validée, réutilisant cookie et `Caller::key`, sans changer la politique des cookies |
| `crates/moli-os/src/main.rs` | `KINDS`, `build_driver`, carte des passerelles des instances actives vers `moli_api::Options` |
| `web/src/maison/ui/NetatmoAuth.svelte` | état et bouton de connexion, confirmation PIN existante, aucune saisie ni restitution de jeton |
| `integrations/netatmo/` | manifeste, fixtures synthétiques sans secret, onboarding, README et spécification OpenAPI des routes ajoutées |

Les cartes de passerelles suivent le précédent des téléphones et machines : `moli-api`
consomme `moli-netatmo::Gateway`, jamais la tâche de lecture directement. Aucun nouveau contrat
générique du hub n’est introduit. Sans instance Netatmo activée : carte vide, aucune tâche propre.

### 1. Décodage météo et familles des points

- [ ] Écrire les tests `station_and_modules_keep_identity`, `old_http_success_is_offline`,
  `missing_field_is_null`, `duplicate_identity_is_rejected`, `unknown_types_are_ignored` et
  `real_point_specs_have_b1_families`. Assertions : même id après renommage/permutation ;
  `online == false` à `time_utc + stale_seconds + 1` ; absence ≠ `Value::Number(0.0)` ;
  type caméra sans appareil publié ; aucune réponse partiellement appliquée si identités ambiguës.
- [ ] Exécuter les tests rouges ciblés dans le créneau Linux accordé ; conserver le journal
  prouvant le comportement manquant avant le décodeur final.
- [ ] Implémenter `decode` et les descriptions : température °C, humidité %, CO₂ ppm, pression
  hPa, bruit dB, pluie et sommes 1 h/24 h en mm, batterie %. Ajouter un indice radio/Wi-Fi brut
  seulement si sa représentation est documentée ; ne jamais le présenter en dBm ou en % inventé.
  Ne pas publier de tension de pile à partir de `battery_vp` sans contrat d’unité vérifié.
- [ ] Valider nombres finis et plages de mesure, horodatages, tailles et doublons avant publication.
  Une clé absente/null/incorrecte donne `Null` pour le point concerné ; des métadonnées d’identité
  ou de temps invalides empêchent de déclarer le module disponible.
- [ ] Vérifier les tests verts et les limites à chaque frontière ; `Temperature/Humidity/Pressure`
  et ppm → `climate`, `Battery/SignalStrength` → `diagnostic`, pluie/bruit → `other`.

### 2. Autorisation humaine et échange initial

- [ ] Écrire les tests `state_is_single_use_and_expires`, `same_ip_different_session_is_refused`,
  `logout_invalidates_callback`, `callback_without_cookie_does_not_exchange`,
  `authorize_never_calls_provider` et `callback_html_never_embeds_code`. Vérifier notamment
  refus avant transport, absence d’écho des données reçues et un seul échange lors de deux retours concurrents.
- [ ] Implémenter `PendingAuthorization` et la passerelle ; conserver uniquement le hash de l’état
  avec l’empreinte de session, l’instance et une échéance monotone. Consommer avant l’échange ;
  un mauvais état ne consomme pas la tentative valide. Une nouvelle ouverture remplace celle de l’instance.
- [ ] Ajouter `GET /api/integrations/netatmo` pour le statut sans secret et
  `POST /api/integrations/netatmo/{instance}/authorize` réservé à la session PIN ; retourner
  l’URL d’autorisation de destination fixe et son expiration. Vérifier l’hôte de retour contre
  l’origine configurée, et l’existence d’une instance active ; aucun URL ni scope fourni par le corps.
- [ ] Ajouter `GET` et `POST /api/integrations/netatmo/{instance}/callback`. Le GET sert une page
  statique, sans traiter de code ni de jeton côté serveur ; le script retire immédiatement la query
  de l’historique puis soumet code/état en JSON par POST de même origine. Le POST exige la session
  humaine exacte, vérifie l’état et déclenche l’échange une seule fois. Aucun assouplissement de la garde Host/CSRF.
- [ ] Fixer `Cache-Control: no-store`, `Referrer-Policy: no-referrer`, CSP limitée à cette page,
  sans script/image externe. Ne journaliser ni URI de callback avec query, ni corps, ni URL d’autorisation.
  Tester en particulier l’arrivée initiale sans cookie puis le POST avec cookie valide.
- [ ] Vérifier les tests verts ; ajouter les statuts 403/404/409/410/413/422/429/502/503 aux contrats OpenAPI.

### 3. Renouvellement, coffre et erreurs réseau

- [ ] Écrire `rotation_persists_before_use`, `storage_failure_never_uses_new_tokens`,
  `restart_reads_rotated_bundle`, `simultaneous_refresh_is_single_flight`,
  `lost_refresh_response_is_not_replayed`, `refresh_failure_before_send_keeps_tokens_and_retries`
  et `errors_and_debug_never_contain_credentials`.
  Générer les clés et jetons à l’exécution ; vérifier les sorties avec les valeurs générées.
- [ ] Implémenter `TokenSet` versionné et son unique écriture par `DriverCtx::store_secret`.
  Le résultat d’échange devient utilisable seulement après retour réussi du coffre. Le jeu est
  sérialisé sous une seule clé pour ne pas sauvegarder un accès nouveau avec un ancien renouvellement.
- [ ] Une réponse de rotation perdue ou une sauvegarde refusée rend l’instance non saine et les
  appareils hors ligne. Ne pas réutiliser l’ancien jeton possiblement invalidé ; demander une nouvelle
  autorisation si le jeu ne peut être conservé. Aucun mécanisme de récupération par un second fichier.
- [ ] Implémenter les requêtes fixes avec `moli_net::web_with_headers`, TLS public, limites globales
  et aucun retry implicite. Les corps d’erreur fournisseur sont classés puis jetés, jamais repris dans `anyhow`.
  Refus de consentement/`invalid_grant` : attente humaine. Lecture 401 : au plus un renouvellement,
  puis attente si nouvel échec ; lecture 429/5xx/réseau : backoff borné de 60 à 900 s, `Retry-After`
  numérique borné, pas de rafale. Ne pas rejouer les POST de jetons après résultat ambigu.
  Ajouter un transport ponctuel typé dans `moli-net` pour distinguer l’échec avant envoi de
  l’échec après remise de la requête à HTTP ; les API réseau existantes gardent leur comportement.
- [ ] Vérifier les tests verts sur transport simulé et faux serveur borné : réponse chunked excessive,
  compression refusée, redirection refusée, mauvais JSON, expiration hors bornes, auth hostile sans fuite.

### 4. Cycle de vie et fraîcheur réelle

- [ ] Écrire `stable_value_new_measurement_does_not_inflate_history`,
  `repeated_source_timestamp_does_not_refresh`, `expiry_fires_during_backoff`,
  `missing_module_goes_offline_and_returns_same_id`, `disabled_creates_no_task` et
  `commands_are_read_only`. Vérifier le bus réel du hub, les états, l’horloge simulée et la file bornée.
- [ ] Implémenter la boucle d’instance : un seul appel fournisseur en vol, commandes refusées,
  contrôle OAuth et échéances de fraîcheur traités même lorsqu’une lecture attend. Un module absent
  ou inaccessible passe hors ligne ; ses valeurs ne deviennent pas zéro. Au-delà de son âge maximal,
  invalider les points périmés. Une batterie récente ne rafraîchit pas une mesure climatique.
- [ ] Ne rappeler `set_state` pour une mesure que si la date source progresse ; une mesure inchangée
  à date nouvelle reste un état valide sans faux événement. Une date source répétée ne renouvelle pas
  l’âge ; `set_availability` ne dépend jamais du seul HTTP 200. Ne pas réinjecter un cache de redémarrage comme mesure fraîche.
- [ ] Vérifier annulation en lecture, durant rotation et après envoi : les jetons reçus sont
  persistés avant tout usage. Si l'arrêt interrompt cette sauvegarde, l'écriture atomique laisse
  le coffre lisible avec l'ancien ou le nouveau jeu ; la reprise tente au plus un renouvellement,
  puis attend une autorisation humaine en cas de refus. Aucun changement du runtime ni marqueur
  avant POST. Tester cette frontière ; aucune nouvelle requête après annulation, appareils hors ligne.
  À l’arrêt du pilote, refuser/vider la boîte de contrôle et invalider les états OAuth en attente.

### 5. Raccordement, installation et livraison

- [ ] Raccorder la crate au workspace et au binaire, déclarer `kind = "netatmo"`, construire les
  passerelles uniquement pour les instances actives et les transmettre aux options API. Couvrir
  `check-config` et le contrat catalogue de chaque `KINDS`.
- [ ] Ajouter la carte d’autorisation Netatmo dans `Systeme.svelte`, reprenant la confirmation PIN
  existante et les styles locaux. Afficher attente, connecté, reconnexion et échec sûr ; ne mettre
  aucun identifiant d’application, code ou état OAuth en `localStorage`, dans une notification ou un lien persistant.
- [ ] Compléter `integrations/netatmo/integration.toml`, `onboarding.md`, `README.md`, fixtures
  météo synthétiques et OpenAPI des routes OAuth. Documenter coffre humain, origine de retour enregistrée,
  portée météo, limites, panne de sauvegarde, état inconnu, familles B1 et absence de qualification H1.
  Toute fixture citée au manifeste est rejouée par un test ; aucune fixture de jeton commité.
  L’inventaire privé confirme station, module extérieur et pluie ; intérieur/vent non observés
  dans cette capture, à confirmer à H1. Ne pas présenter ces variantes comme prises en charge.
  La CSP du retour épingle le script par SHA-256, sans `unsafe-inline` ; aucune preuve de PKCE
  S256 dans le contrat officiel consulté, ne pas prétendre l’utiliser. Les échantillons du hub
  sont horodatés à réception par Moli ; `time_utc` sert à la validité, sans antidater l’historique.
- [ ] Actualiser `docs/components/pilotes.md`, `docs/components/api.md` et les catalogues fr/en.
  Les clés visibles nouvelles sont couvertes par le contrôle i18n.
- [ ] Après créneau explicite, lancer `sh scripts/check.sh` sous Linux ; puis
  `cd web && npm ci && npm run build`, et `node scripts/i18n-check.mjs` à la racine.
  Exiger codes 0, pas de test en échec, catalogue sans erreur et leak-check propre ; conserver
  source, diff, logs et empreintes avant/après hors dépôt. Aucune affirmation de fonctionnement matériel.
- [ ] Commiter les seuls fichiers Netatmo sur `integ-capteurs-netatmo`, message public en français ;
  soumettre le sous-ticket à l’architecte par `backlog_submit_for_review`, sans fusion, push ni clôture du parent.

## Porte d’exécution

Le plan sépare les décisions logicielles proposées des étapes d’installation humaine. Aucun secret
réel ni choix de matériel n’est requis pour les tests simulés. Le skill `writing-plans` demande une
relecture humaine de ce document avant l’implémentation ; méthode retenue : exécution dans cette session,
sans délégation. Les créneaux de compilation restent soumis séparément à l’Orchestrateur.
