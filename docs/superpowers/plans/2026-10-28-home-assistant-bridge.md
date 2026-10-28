# Home Assistant bridge Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Préparer les briques de migration progressive et qualifier le premier chemin HA→MQTT→Hub→Overkiz sur transports simulés.

**Architecture:** Un compagnon HA et un pilote Rust échangent sur MQTT. Un registre durable détient les correspondances et phases ; le Hub applique la génération d’autorité et Overkiz la recontrôle jusqu’à l’émission.

**Tech Stack:** Rust1.99.1, Tokio, rumqttc0.25, StateFile, Python du compagnon HA, tests synthétiques locaux.

**Spec:** `docs/proposition/pont-home-assistant-moli-v1.md`.

## Global Constraints

- Aucun ordre physique ni modification de HA pendant le développement.
- Pas de secret dans un fichier ou une sortie ; identités fictives dans les tests.
- Messages visibles traduits FR/EN ; origines déterminées par le serveur.
- Échéance absolue, pas de rejeu, entrées et files bornées.
- Préserver l’arbre partagé et les changements préexistants ; aucun staging global.

## Review Focus

- Ancienne commande réactivée après plusieurs changements de propriétaire.
- Révocation pendant OAuth, connexion ou TLS, avant la remise HTTP.
- Erreur de stockage après renommage et redémarrage sur opération incertaine.
- Message retained, dupliqué ou émis avant la session actuelle du destinataire.
- Commande HA assimilée à tort à une action humaine Moli ou feedback RTS inventé.

### Task 1: Autorité native dans le Hub

Files: `moli-runtime/src/{write_authority,hub,lib}.rs`.

- [x] Capturer et composer un permis de génération dans tous les chemins de commande et d’approbation.
- [x] Tester révocation, clôture d’urgence, non-résurrection, composition avec la garde et absence de cycle de rétention.

### Task 2: Dernière frontière d’émission Overkiz

Files: `moli-net/src/lib.rs`, `moli-overkiz/src/{api,lib,fake,tests}.rs`.

- [x] Transmettre SendPermit jusqu’au dernier handoff HTTP, sans retry d’action.
- [x] Tester abandon et révocation pendant connexion/OAuth, délai absolu et réponse après émission.

### Task 3: Registre durable de migration

Files: nouvelle crate `moli-migration`.

- [x] Implémenter phases, empreintes de plan, révisions attendues et preuves exactes.
- [x] Fermer les écritures avant toute décision incertaine et au redémarrage ; réconcilier avec défi neuf.
- [x] Tester retour inverse, bornes, erreurs pré/postcommit et provenance inactive des automatismes.

### Task 4: Protocole et pilote MQTT

Files: nouvelle crate `moli-home-assistant`, paquet `integrations/home-assistant` et raccord de démarrage explicite.

- [x] Importer des canaux indépendants et exposer les volets sélectionnés.
- [x] Valider chaque commande, session, échéance, révision et génération avant le Hub sous Origin::Api.
- [x] Qualifier les refus et les échanges réels avec un broker et des effecteurs simulés.

### Task 5: Compagnon HA

Files: `tools/home-assistant/custom_components/moli/` et ses tests.

- [x] Fournir configuration, inventaire MQTT, cover sans optimisme RTS et bouton favori distinct.
- [x] Refuser toute preuve de neutralisation qui n’est pas réellement observée par un adaptateur qualifié.
- [x] Tester le codec, les états inconnus, le contrôle des sessions et les commandes sans rejeu.

### Task 6: Intégration et preuves

- [x] Raccorder les options et sauvegardes nécessaires, avec activation explicite.
- [x] Exécuter fmt, Clippy strict, tests ciblés, catalogue, traductions et contrôle de fuites.
- [x] Publier les résultats simulés et les limites de qualification réelle, puis répondre à la délégation.

Qualification du premier lot : voir `docs/proposition/analyse/v6/pont-ha-moli-tests.json`. Les étapes réelles de neutralisation HA et de migration du moteur d’automatismes restent hors de ce jalon ; elles ne sont pas marquées comme exécutées.
