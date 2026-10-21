# Ajouter la supervision publique SMLIGHT

Avant H1 : fixtures et serveurs simulés seulement. Aucun agent ne sonde un coordinateur.
Après H1, humain présent, cible neutre hors pièces protégées, état laissé identique.

1. Le propriétaire fournit lui-même hôte/port et confirme que `/ha_info` et `/ha_sensors`
   sont déjà publics. Ne désactiver aucune sécurité et ne saisir aucun identifiant.
   401/403/HTML de connexion : conserver la supervision actuelle, variante hors périmètre.
2. Créer une instance privée `kind = "profile"`, options `profile = "smlight"`, `host`
   et éventuellement `port` (défaut80). Aucun secret, URL, query, chemin ou commande.
   `moli-os check-config` vérifie la forme sans contacter le coordinateur.
3. Après H1, comparer seulement identité native, sept liaisons, unités et cadences.
   Une MAC absente/invalide ou différente de l'identité déjà connue bloque la publication ;
   faire vérifier la cible par l'humain, sans effacer automatiquement cache ou identités.
   Les adresses apprises ne sont pas suivies : seul l'hôte fourni est utilisé.
4. Vérifier la charge sans scan, débit provoqué, radio, redémarrage ou retrait réseau.
   La mémoire reste brute sans unité ; une mesure absente reste inconnue. Le premier tour
   comporte deux départs espacés d'une seconde, après une pause initiale d'une seconde.
5. Laisser Zigbee2MQTT propriétaire de la radio et HA de la supervision jusqu'à validation
   explicite de chaque usage. Aucune bascule ni mise en production implicite par une revue.

Exemple fictif, remplacer la cible uniquement par celle du propriétaire :

```toml
[[driver]]
id = "coordinator-diagnostic"
kind = "profile"
[driver.options]
profile = "smlight"
host = "coordinator.example.invalid"
port = 80
```

HTTP seulement selon le contrat constructeur ; ne jamais placer de secret dans ce profil.
Le changement de cible/identité doit être une décision humaine explicite. Les erreurs
invalident les mesures de la cible fixe ; les nouvelles lectures restent soumises au backoff.

## English

Before H1, use synthetic fixtures only. The owner supplies the host/port and confirms both
monitoring endpoints are already public. Never disable authentication to use this profile.
Configure `kind = "profile"`, `profile = "smlight"`, host and optional port80, without any
secret or URL. Missing/changed MAC identity blocks publication; learned addresses are ignored.
Seven static diagnostic bindings remain declared with missing measurements at Null, including
cold-cache restart. RAM remains raw. Reads are spaced by one second including startup/restarts,
with 5 s / 1 MiB exchange bounds. No commands, radio, scan, reboot or credential handling.
Human qualification uses a neutral target outside protected rooms and preserves its state.
Zigbee2MQTT and HA retain ownership until each usage is explicitly validated.
