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

- [ ] Tests avant/au début/à la fin, expiration et suppression/remplacement du pilote.
- [x] Tests purs premier calcul, arrêt couvrant toute la fenêtre, recul d’horloge et réconciliation.
- [ ] Révocation des droits dans le vrai hub et les sessions.
- [ ] Implémentation minimale puis tests du vrai hub avec horloge contrôlée.

## 3. Client, paquet et binaire

Fichiers : `crates/moli-reservations/src/{lib.rs,client.rs,runtime.rs}`, `Cargo.toml`,
`crates/moli-os/{Cargo.toml,src/main.rs}`, `integrations/reservations/` et locales.

- [ ] Tests transport local simulé : corps trop grand, délai, redirection, secret absent,
  aucune fuite de l’URL dans les erreurs ; aucun contact d’URL hors du banc de test.
- [ ] Lire uniquement `url` au coffre ; publier les cinq prochaines réservations et cycle.
- [ ] Ajouter registre, manifeste, onboarding, fixtures et documentation de configuration.

## 4. Politique invitée

Fichiers : `crates/moli-api/src/people.rs`, tests HTTP et documentation d’API ; éditeur des
personnes si nécessaire pour une liaison humaine utilisable, avec traductions fr/en.

- [ ] Tests HTTP d’une session existante aux bornes, hors ligne et après annulation ;
  flux encore ouvert et commande suspendue au transport au moment de la fermeture.
- [ ] Ajouter la liaison facultative à une réservation ; préserver l’expiration existante,
  les pièces autorisées et la garde lock. Aucun droit créé depuis le seul calendrier.
- [ ] Vérifier les invités sans liaison et les comptes propriétaire/membre.

## 5. Livraison

- [ ] Tests ciblés natifs, format, Clippy, catalogue, fuites, web/i18n si modifié.
- [ ] Commit du ticket et soumission pour revue avec commandes et résumé PASS/FAIL.
- [ ] Après approbation : rebase, contrôle complet dans la file, fusion locale sans push.
