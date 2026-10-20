# Mettre une serrure Nuki dans Moli

Le pont Nuki (le boîtier branché sur une prise, pas l'appli) a une API HTTP locale. Moli lit
l'état des serrures appairées au pont et peut les verrouiller ; **déverrouiller et ouvrir
sont toujours décidés par un humain** (voir plus bas).

1. **Activer l'API HTTP du pont**, par le propriétaire, dans l'appli Nuki : Pont → Gérer le
   pont → API HTTP → activer. L'appli montre l'adresse du pont, son port (8080 par défaut) et
   le **jeton** de l'API.
2. **Ranger le jeton dans le coffre**, par un humain, jamais dans un fichier ni dans une
   conversation avec un agent :

   ```sh
   printf %s "$JETON" | docker exec -i moli-os /moli-os secrets set serrure token
   ```

   (`serrure` est l'`id` de l'instance déclarée ci-dessous.)
3. **Déclarer le pont** dans `moli.toml` :

   ```toml
   [[driver]]
   id = "serrure"
   kind = "nuki"
   [driver.options]
   host = "192.168.1.x"   # l'adresse que montre l'appli ; réserver une adresse fixe sur la box
   # port = 8080
   ```

4. Redémarrer Moli. Chaque serrure apparaît sous son nom Nuki, identifiée par son numéro
   Nuki (stable). Lui donner sa pièce dans l'Atelier.

## Vérifier

- Le pilote passe « en marche ». S'il attend : le message dit quoi faire (jeton manquant,
  accès refusé, pont injoignable).
- **Accès refusé (401)** alors que le jeton est bon : l'horloge du pont et celle de Moli
  diffèrent de plus d'une minute (le jeton est haché avec l'heure). Remettre le pont à l'heure
  (il la prend sur Internet) ou celle de la machine de Moli.
- L'état suit la serrure en 30 s au plus (le pont garde le dernier état : interroger la
  serrure elle-même viderait ses piles).

## Essayer une commande (jalon humain)

Seulement avec le propriétaire présent, sur une porte qu'on peut rouvrir à la main, **jamais
celle d'une pièce protégée**, et en remettant la serrure dans l'état où on l'a trouvée.

1. Verrouiller depuis l'Atelier (`locked` → oui) : la serrure tourne, l'état passe à
   `locked`.
2. Déverrouiller depuis le dashboard sans code : l'ordre est **retenu** et le code est
   demandé ; un agent (MCP, Moli) qui le demande crée une demande d'approbation qu'un humain
   libère. Les deux tentatives sont au journal.
3. Remettre l'état initial.

## Ce que Moli ne fait pas ici

- L'Opener Nuki (interphone) n'est pas piloté : il est ignoré.
- Pas de rappel du pont vers Moli (callbacks) : la lecture est périodique.
- Le [pilote de réservations](../reservations/onboarding.md) peut limiter les accès d’un invité.
  Une coordination de serrure exige un automatisme explicitement validé par un humain ;
  le calendrier n’envoie jamais directement un ordre au pont.
