# Réservations ICS — plan d’implémentation

**Objectif :** un calendrier choisi par l’humain, ses occurrences et des fenêtres invitées
vérifiées à chaque accès, sans liaison directe avec une serrure.
**Architecture :** domaine pur borné, pilote en lecture seule, instantané éphémère du hub
et politique invitée facultative. Réutiliser Jiff, le coffre, moli-net et la garde.
**Pile :** Rust 1.99.1 en deux jobs pour les contrôles natifs, MSRV du dépôt conservée, Tokio/Jiff/Serde.
**Spécification :** [contrat ICS](../specs/2026-10-20-reservations-ics-design.md).
Exécution dans cette console ; revue finale indépendante avant intégration.

## Contraintes globales

- Aucun calendrier réel, URL choisie par un agent, commande physique ou push.
- Corps 1 Mio, ligne 4096 octets, 512 événements, UID 512 octets, 4096 occurrences,
  3660 pas d’expansion. Refus intégral en cas d’entrée temporelle inconnue ou trop grande.
- Accès `[début, fin)`, refus hors ligne/expiré ; pas d’événement de rattrapage au démarrage.
- Textes visibles fr/en, fixtures fictives, aucun titre ou participant publié.

## Points de revue

L’heure locale n’existe pas ou existe deux fois ; une exception annule une seule instance ;
un calendrier malformé remplace un calendrier valide ; une session invitée survit à la fin
de sa fenêtre ; un arrêt couvre entièrement une réservation. Chaque cas reçoit un test.

## 1. Parseur et fenêtres purs

Fichiers : `crates/moli-reservations/{Cargo.toml,src/lib.rs,src/calendar.rs,src/recurrence.rs}`,
`crates/moli-reservations/tests/calendar.rs` et fixtures du paquet.
Interface : `parse(&str, &str, Timestamp) -> Result<Vec<Reservation>, CalendarError>` ;
`Reservation { id, start, end }`, `allows(Timestamp) -> bool`.

- [x] Écrire et exécuter le rouge UTC/TZID/DATE et bornes exactes.
- [x] Implémenter dépliage/validation des composants et dates Jiff, faire passer ces tests.
- [x] Ajouter le rouge puis le vert DAILY/WEEKLY/COUNT/UNTIL/EXDATE/RDATE et annulations.
- [x] Tester limites, doublons/versions, exceptions, heures locales impossibles et ambiguës.

## 2. Instantanés et reprise

Fichiers : `crates/moli-core/src/access_window.rs`, `crates/moli-runtime/src/reservations.rs`,
`driver.rs` et `hub.rs` ; `crates/moli-reservations/src/{timeline.rs,runtime.rs}`.
Interfaces : fenêtre validée, remplacement atomique par instance et lecture refusant
un pilote hors ligne ou une échéance dépassée ; transitions sans rejeu initial.

- [x] Tests avant/au début/à la fin, expiration et suppression/remplacement du pilote.
- [x] Tests purs premier calcul, arrêt couvrant toute la fenêtre, recul d’horloge et réconciliation.
- [x] Révocation des droits dans le vrai hub et les sessions.
- [x] Tests du vrai hub pour statuts/restarts et du registre avec instants contrôlés.

## 3. Client, paquet et binaire

Fichiers : `crates/moli-reservations/src/{lib.rs,client.rs,runtime.rs}`, `Cargo.toml`,
`crates/moli-os/{Cargo.toml,src/main.rs}`, `integrations/reservations/` et locales.

- [x] Tests transport local simulé : corps trop grand, délai, redirection, secret absent,
  aucune fuite de l’URL dans les erreurs ; aucun contact d’URL hors du banc de test.
- [x] Lire uniquement `url` au coffre ; publier les cinq prochaines réservations et cycle.
- [x] Ajouter registre, manifeste, onboarding, fixtures et documentation de configuration.

## 4. Politique invitée

Fichiers : `crates/moli-api/src/people.rs`, tests HTTP et documentation d’API ; éditeur des
personnes si nécessaire pour une liaison humaine utilisable, avec traductions fr/en.

- [x] Tests HTTP d’une session existante avant la fenêtre, active, hors ligne et annulée ;
  flux encore ouvert et commande suspendue au transport au moment de la fermeture.
- [x] Ajouter la liaison facultative à une réservation ; préserver l’expiration existante,
  les pièces autorisées et la garde lock. Aucun droit créé depuis le seul calendrier.
- [x] Vérifier les invités sans liaison et les comptes propriétaire/membre.

## 5. Livraison

- [x] Tests ciblés natifs, format, Clippy, catalogue, fuites, web/i18n si modifié.
- [x] Préparer le commit du ticket et le dossier de revue avec commandes et résumé PASS/FAIL.
- [ ] Revue indépendante approuvée.
- [ ] Après approbation : rebase, contrôle complet dans la file, fusion locale sans push.

## Validation de soumission

- 231 tests Rust uniques couverts sur les sept crates concernées, en natif Rust 1.99.1,
  `-j 2` et un thread de test ; dont 37 dans `moli-reservations`.
- La première passe conserve deux échecs ConnectionReset des clients HTTP préexistants
  (demo et invité). Les deux mêmes binaires ont ensuite repassé leurs suites complètes
  (1 et 19 tests), sans exclusion. Les contrôles nouveaux de session/SSE/transport passent.
- Format et Clippy workspace/all-targets avec `-D warnings` verts ; 31 tests web, build et
  contrôle i18n 3003 clés verts. Catalogue 33 paquets, six avertissements antérieurs ;
  contrôle de fuites générique propre, sans liste privée disponible.
- Le contrôle complet du workspace est réservé au créneau de fusion après approbation,
  conformément à la règle propriétaire ; aucun push, calendrier réel ou appareil utilisé.

## Correction de revue ICS-R1-001

Le droit vivant du parent accompagne les enfants du luminaire et la reprise différée
après alimentation, borné aux membres déclarés. La portée du parent reste évaluée : une
ampoule physique sans pièce ne reçoit pas de nouveau droit indépendant. L’origine interne
System et le suivi/journal d’un ordre déjà émis restent conservés.

Sept tests du vrai Hub/Fixtures : quatre garanties étaient rouges (calendrier révoqué,
ampoule/relais à fin de fenêtre, continuation différée), trois témoins verts ; sept verts
après correction. Un test runtime vérifie la portée des membres et le droit après réponse.
84 tests runtime/luminaires/Nuki verts, Clippy workspace/all-targets et format verts,
fuites génériques propres. Tokio io-util est explicitement déclaré par le runtime pour
sa compilation isolée. Les contrôles web et domaine précédemment relus ne sont pas rejoués.
