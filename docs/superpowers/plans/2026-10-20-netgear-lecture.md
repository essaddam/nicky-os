# Netgear : plan de lecture SOAP

> Exécution directe par l'auteur ; revue du diff par Database.

**But :** livrer N3 du ticket 26c60723 sans appel matériel, depuis main 892ccbb.
**Contrat :** docs/components/reseau.md et reseau-qualification.md, N0 déjà intégré.
**Architecture :** crate moli-netgear (configuration, XML borné, modèle, session SOAP,
publication supervisée), transport moli-net, paquet integrations/netgear. Aucune écriture.
**Source figée :** pynetgear d9b34149b110cd91fbccc5fff7a285a1221af5dd,
router.py / const.py / helpers.py ; connaissance du protocole seulement, aucun code copié.

## Contraintes

- Hôte, port, transport, login v1/v2 et inventaire v1/v2 explicitement choisis ; aucune
  recherche ni variante automatique. Options fermées ; période 60 s, inventaire 300 s,
  informations stables 3600 s ; plancher 30 s.
- HTTPS épinglé et empreinte au coffre avant identifiants ; HTTP seulement avec
  allow_plain_auth=true. Secrets username/password/cert_sha256, sessions en mémoire.
- POST /soap/server_sa/ seulement. Couples : ParentalControl:1/Authenticate,
  DeviceConfig:1/SOAPLogin, DeviceInfo:1/GetInfo, GetSystemInfo, GetAttachDevice(2),
  WANEthernetLinkConfig:1/GetEthernetLinkStatus,
  DeviceConfig:1/GetTrafficMeterStatistics. Aucun autre GetInfo, Set*, configuration,
  activation de Traffic Meter, scan, speedtest ni logout de session humaine.
- 1 Mio/5 s échange ; 30 s/16 échanges cycle ; un échange en vol ; 8 cookies/4 Kio,
  CR/LF et contrôles refusés ; relogin unique avec rejeu de la seule lecture refusée.
- XML : parseur maintenu, DTD/entités déclarées refusées, profondeur 16,
  10 000 éléments, champ 4 Kio ; identité par URI et non préfixe SOAP.
- Inventaire complet <=512, sélection <=32 vide par défaut ; aucun point nom/IP/MAC/SSID,
  aucune connexion découverte. Doublon, compte incohérent, séparateur inconnu, XML partiel
  ou identité absente rejettent le cycle avant publication.
- Identité routeur : SerialNumber, conservée au coffre et vérifiée à chaque authentification et lecture des informations stables ;
  satellite non identifié refusé, pas de repli IP/nom. Client : MAC native normalisée,
  identité locale privée. Une liste attachée ne prouve pas une association actuelle :
  associated reste absent sans preuve explicite, jamais faux sur simple absence.
- Système CPU/mémoire et lien Ethernet selon champs connus ; trafic seulement si unité
  prouvée dans la valeur, sinon inconnu (aucune supposition MB vers octets). Signal brut
  sans unité supposée, débit de lien seulement lorsque l'unité est explicite.
- <=16 points équipement +4/client, publication set_state uniquement ; champ absent ou
  erreur facultative => Null pour les points déjà déclarés ; identité invalide/échec
  global => retour superviseur, hors ligne. Refus auth => waiting, aucune boucle.

## Étapes et preuves

- [ ] 1. Tests rouges configuration/allowlist/XML (namespaces, DTD/XXE, profondeur,
  taille, champ, éléments, document tronqué), puis implémentation et tests verts.
  Fichiers : crates/moli-netgear/src/{lib,xml,protocol}.rs et tests associés ; Cargo.toml.
- [ ] 2. Fixtures et tests modèle : identité, liste v1 comptée et XML v2, doublons,
  513 clients, sélection 33, satellites, champs absents, trafic inactif et unités.
  Fichiers : model.rs, model_tests.rs, integrations/netgear/fixtures/.
- [ ] 3. Transport/session et tests serveur local : login choisi, en-têtes cookies,
  redirections interdites, fautes HTTP200/500, capacité mémorisée, relogin borné,
  délais, annulation. Ajouter à PinnedHttps un échange unique borné avec en-têtes,
  sans changer le comportement de ses méthodes existantes.
- [ ] 4. Publication et cycle supervisé : secrets manquants sans réseau, TLS avant
  secrets, refus des commandes, cadences distinctes sans rattrapage, offline au drop,
  aucun événement d'état sur relevé identique, budget de points et d'échanges.
- [ ] 5. Enregistrer kind netgear dans moli-os ; manifeste/onboarding/README (repris,
  supposé, restant), documentation des limites, traductions fr/en ; listes alphabétiques.
- [ ] 6. Tests ciblés natifs Rust 1.99.1 -j2 (CARGO_BUILD_JOBS=2). Soumission après fmt, clippy workspace
  et tests ciblés verts. Contrôle complet unique après approbation, seulement au tour
  attribué dans la file de fusion : check.sh
  natif sauf les quatre tests Linux moli-profile ; catalogue, i18n et fuites.
  Docker gelé : aucun run/exec/build ; les quatre tests Linux sont différés sur main
  après rétablissement de Docker, sans bloquer soumission ou fusion. Journaux hors
  dépôt. Commit français, soumission du diff et résultats pour revue.
  Test moli-archive authenticated_d13_transport_handles_replay_conflict_gap_and_read
  exclu provisoirement du contrôle complet puisque ce lot ne touche pas moli-archive ;
  mentionner cette exclusion au commentaire de fusion (décision propriétaire).

## Attention de revue

Namespaces trompeurs, cookie renouvelé avec attribut Domain hostile, inventaire malformé
présenté vide, routeur remplacé après reconnexion, fuite du corps SOAP dans une erreur :
chacun doit avoir un test négatif. Aucun firmware ni budget CPU matériel n'est qualifié.
