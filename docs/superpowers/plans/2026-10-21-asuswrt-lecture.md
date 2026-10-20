# ASUSWRT : client JSON authentifié de lecture

Exécution directe dans la session Réseau ; revue finale par Database, sans étape de
relecture par l'architecte (consigne du chef : mêmes règles que Netgear).
Base : main local `87691c54e560b4cb4a9db524dc47f96bff048201` ; aucun contenu non commité
du checkout principal n'est repris. Netgear demeure indépendant et ses corrections
passent en priorité.

## Contrat retenu

Crate `moli-asuswrt`, paquet `integrations/asuswrt/`, dépendances standards du workspace.
Source publique figée : AsusRouter a897f28e1e07229a16f3fb553035575fbf2b2328,
Apache-2.0 ; connaissance du protocole seulement, aucun code tiers copié.

- Hôte/port/transport fournis par l'humain, HTTP authentifié uniquement sur choix explicite.
  TLS épinglé et persistance au coffre avant l'envoi d'identifiants. Pas de redirection,
  détection, variante automatique, SSH/Telnet, administration ou déconnexion humaine.
- POST `/login.cgi`, login_authorization en base64, puis cookie asus_token issu du JSON.
  Token ASCII borné à 1 Kio ; pas de cookie jar ni adoption des attributs Set-Cookie.
  Le format de corps suit l'encodage de la bibliothèque amont ; aucune valeur de secret
  ne figure dans les fixtures, messages ou journaux.
- POST `/appGet.cgi` seulement, trois corps fixes : identité (label_mac, productid et
  firmware par nvram_get), télémétrie (cpu_usage(appobj), memory_usage(appobj),
  netdev(appobj)), inventaire vivant (get_clientlist). Aucun hook fourni par la config.
- L'identité label_mac doit être présente et stable ; elle est conservée au coffre.
  Aucun repli IP/nom ni publication de métadonnées personnelles comme points.
- JSON strict uniquement : 1 Mio avant parsing, profondeur 16, 10 000 valeurs,
  textes/clefs 4 Kio, doublons refusés (dont interfaces WAN ambiguës). HTML/JS refusés.
- Un échange en vol, 5 s par échange, 30 s et 16 échanges par cycle. Un relogin au plus
  par cycle avec rejeu de la seule lecture expirée. Refus/captcha/verrouillage en attente
  humaine ; panne de transport au superviseur. Annulation et refus des commandes.
- Cadences 60 s/300 s/3600 s (min 30 s), sans rattrapage. Inventaire <=512 entrées,
  sélection <=32 vide par défaut, 16 points routeur et 4 par client au maximum.
- CPU dérivé des compteurs ticks entre relevés compatibles ; mémoire KiB vers octets,
  compteur WAN INTERNET en octets, débits B/s par delta monotone. Réinitialisation ou
  changement de session => pas de débit inventé. Les conversions suivent les schémas
  amont ; un autre schéma reste non qualifié. Clients : type de lien, signal brut,
  taux de lien selon la convention amont explicitée. Aucune présence ou association
  déduite de isOnline/isWL. Clients absents => inconnu, jamais faux par simple absence.
- Température non exposée dans ce premier contrat JSON : les routes amont connues
  renvoient du JavaScript. Pas d'interpréteur ni de conversion supposée.
- Publication atomique du cycle, points fixes en lecture seule, Diagnostic/SignalStrength,
  valeurs absentes en Null et valeurs inchangées sans nouvel événement. HA garde ses usages.

## Séquence TDD

1. Config/JSON/modèles : fixtures synthétiques et assertions des bornes, identité,
   compteurs/réinitialisations, inventaire vide/duplication/513 entrées, unités et champs
   absents. Observer le rouge, implémenter le contrat, puis vert.
2. Protocole/session et réseau : requêtes figées, encodage, token hostile, erreur JSON
   dans HTTP 200, un seul relogin, erreur d'identité/capacités, budgets, taille/délai,
   annulation et TLS avant secrets ; serveurs locaux éphémères, clés générées en mémoire.
3. Runtime/hub : secrets absents sans réseau, panne initiale récupérable, cadences,
   atomicité/Null, clients sélectionnés seulement, absence de présence et de commandes,
   stabilité sans événements supplémentaires.
4. Enregistrement alphabétique dans le binaire/workspace/catalogue, documentation,
   onboarding et textes fr/en. Pas de nouveau point d'accès REST ni modification web.
5. Validation native Rust 1.99.1, deux jobs : fmt, Clippy workspace strict, tests ciblés
   des crates modifiées, catalogue, i18n, fuites. Commit puis soumission à Database.
   Aucun contrôle complet avant approbation ; pas de Docker ni appareil réel.
   La fusion attend le tour explicite ; aucune publication distante.

## Risques de revue

Distinguer expiration et refus/captcha ; traiter une liste invalide comme une erreur,
jamais une déconnexion ; refuser les clefs JSON dupliquées ; ne pas calculer de débit
sur un changement d'identité/session ou un compteur en baisse ; garder l'identité hors
points et logs. Les fixtures ne qualifient aucun matériel, firmware ou budget physique.
