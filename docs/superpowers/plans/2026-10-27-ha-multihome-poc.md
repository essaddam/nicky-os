# POC actif chez soi et passif pour l’autre — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Deux serveurs portent chacun deux maisons : actif pour sa maison, passif pour l’autre, avec une interface commune et une réplication optionnelle configurable.

**Architecture:** Un POC Rust autonome supervise des processus de nœuds, un transport MQTT de test et une autorité de baux par maison. Le master minimal et les pilotes physiques restent indépendants. L’interface passe par le même serveur web que le POC existant via un proxy loopback fixe ; les seules actions sont des simulations sur les enfants du banc.

**Tech Stack:** Rust 1.99.1, Tokio, rumqttc, broker POC existant, Axum, serde, SHA-256 ; HTML/JS autonome, catalogues FR/EN.

**Spec:** Demande utilisateur du 27 octobre et précisions : « maison A active pour elle, passive pour B », symétriquement pour B ; accès aux deux maisons. Décisions conservées de `docs/proposition/haute-disponibilite-v2.md` : pas de journal de réplication sur disque, sorties et horodatages de l’actif recopiés, secours distant limité au cloud sur geste humain.

## Global Constraints

- Aucun ordre physique, arrêt de HA, appairage, copie de coffre ou renouvellement de jeton.
- IDs de maison et de nœud stables ; rôle par maison, jamais un unique rôle global du serveur.
- Topologie optionnelle : aucune copie, copie locale, copie cloud, deux passifs ou secours croisés.
- Maximum 2 maisons, 8 nœuds, 4 détenteurs par maison ; instantanés bornés, messages par morceaux, files bornées.
- Activation automatique uniquement pour un secours du même site et avec témoin ; distant manuel/cloud seulement.
- Sans témoin, reprise uniquement après arrêt confirmé de l’ancien processus du banc ; perte de lien seule insuffisante.
- Données initiales : deux exports HA privés, normalisés par le vrai pont existant et anonymisés ; distinction explicite entre rejeu et transport vivant observé.
- Pas de secret/donnée personnelle dans les sources et preuves. Deux processus locaux ne prouvent pas deux sites réels, un VPN, un quorum distribué ou la qualification des horloges.
- Le serveur 1414 conserve ses commandes et son proxy WebSocket authentifié ; le nouveau service écoute seulement en loopback.

## Review Focus

- Partition réseau : pas de double autorité d’effet, aucun ancien epoch accepté.
- Passif en retard ou état tronqué : pas de promotion à jour fictive ; contrôle d’intégrité avant publication atomique.
- Deux maisons : compteurs, baux, copies et effets indépendants, dont après arrêt d’un seul nœud.
- Secours distant : aucune capacité radio/LAN déduite de la promotion.
- Configuration et interface : doublons/IDs absents/bornes refusés, maisons toujours distinguables, erreur visible sans brouillon perdu.

### Task 1 — Modèle et contrat HA pur

Files: `tools/ha-multihome-poc/src/{config,authority}.rs`.
- [x] Configurations et validation ; presets sans HA/local/cloud/deux-passifs/croisé.
- [x] Baux et epochs indépendants par maison, reprise après expiration/fencing et copie au rang requis.
- [x] Validation d’effets simulés : propriétaire, epoch, bail, portée locale/cloud, ID dédupliqué.
- [x] Tests de partition, absence de témoin, copie périmée et identités partagées sans confusion.

### Task 2 — Transport et répliques en processus

Files: `tools/ha-multihome-poc/src/transport.rs` et point d’entrée worker.
- [x] Broker loopback et enfants détenus ; instantanés SHA-256 découpés, application atomique.
- [x] Deltas ordonnés, refus de trou/stale epoch, remise à jour par instantané au redémarrage.
- [x] Mesures RSS/CPU par nœud, octets logiques par copie ; arrêt/récolte même après erreur.

### Task 3 — Scénario et service interactif

Files: `tools/ha-multihome-poc/src/{main,engine,server}.rs`, `tools/mqtt-live-probe/src/ha_seed.rs`.
- [x] Exporter les deux structures HA par SourceFixture + House/Hub existants, sans identité privée en sortie.
- [x] Faire vivre la topologie croisée ; arrêt/reprise de nœud, coupure/rétablissement, promotion contrôlée et workflows factices.
- [x] API bornée de simulation et état agrégé ; republier l’état du banc sur MQTT virtuel.

### Task 4 — Interface commune aux deux maisons

Files: page POC HA, catalogues `locales/{fr,en}/ha-poc.json`, POC principal et proxy de passerelle.
- [x] Sélection et accès A/B ; matrice serveur × maison, état de synchronisation et paramètres optionnels.
- [x] Presets local/cloud/deux-passifs/croisé ; contrôles de simulation clairement identifiés.
- [x] Ajouter un sélecteur de maison au POC existant sans retirer les canaux secondaires ni les commandes.

### Task 5 — Preuves et mesure

Files: script de banc et complément V6 HTML/JSON.
- [x] Tests unitaires et campagnes réelles de processus : croisé, zéro copie, deux secours, partition, ancien actif, rattrapage et refus de copie périmée.
- [x] Mesurer mémoire/CPU par nœud et coût d’une seconde copie à jeux de données comparables.
- [x] Vérifier l’interface en Chrome AgentsRoom bureau/mobile et les catalogues FR/EN.
- [x] Documenter ce qui fonctionne et ce qui manque ; ne pas présenter le témoin central de test comme un quorum distribué prêt pour la production.

Aucun commit automatique prévu pour ce lot ; le précédent travail est sauvegardé dans 077aebb.
