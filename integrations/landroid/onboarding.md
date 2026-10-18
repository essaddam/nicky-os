# Mettre une tondeuse Worx Landroid dans Moli

La tondeuse ne parle qu'au cloud Worx : Moli s'y connecte avec le **compte Worx de l'appli
Landroid** (le même que Home Assistant utilisait). Rien n'écoute dans la maison.

1. **Vérifier le modèle** dans l'appli Landroid (ou sous la tondeuse) : Moli pilote les
   modèles filaires (L1000 WR147E.1 et voisins). Une Landroid Vision est ignorée.
2. **Ranger les identifiants dans le coffre**, par un humain, jamais dans un fichier ni dans
   une conversation avec un agent :

   ```sh
   printf %s "$EMAIL" | docker exec -i moli-os /moli-os secrets set tondeuse username
   printf %s "$MOT_DE_PASSE" | docker exec -i moli-os /moli-os secrets set tondeuse password
   ```

   (`tondeuse` est l'`id` de l'instance déclarée ci-dessous.)
3. **Déclarer l'instance** dans `moli.toml` (aucune option : tout vient du compte) :

   ```toml
   [[driver]]
   id = "tondeuse"
   kind = "landroid"
   ```

4. Redémarrer Moli. Chaque tondeuse du compte apparaît sous son nom Worx, identifiée par son
   numéro de série (stable). Lui donner sa pièce (le jardin) dans l'Atelier.

## Vérifier

- Le pilote passe « en marche ». S'il attend, le message dit quoi faire : identifiants
  manquants ou refusés (le pilote ne réessaie pas tout seul : Worx bloque les comptes qui
  insistent), trop d'appels (nouvel essai dans une heure), cloud injoignable.
- L'état, la batterie et l'erreur suivent la tondeuse en direct ; le dernier état connu du
  cloud est montré dès le démarrage.

## Un seul propriétaire

Tant que Home Assistant pilote la tondeuse, il reste le propriétaire de ses automatismes
(planning, pluie) : ne pas recréer les mêmes dans Moli avant d'avoir validé la version Moli
et retiré celle de HA. Les deux peuvent lire en même temps.

## Essayer une commande (jalon humain)

Seulement avec le propriétaire présent et la pelouse dégagée (ni enfant, ni animal, ni
jouet), **jamais pendant une heure calme**, et en remettant la tondeuse dans l'état où on l'a
trouvée (à la base, planning tel qu'il était).

1. Depuis l'Atelier, `control` → `start` : la tondeuse quitte sa base, `state` passe à
   `leaving_home` puis `mowing`.
2. `control` → `home` : elle rentre ; `state` passe à `going_home` puis `home`.
3. Un agent (MCP, Moli) qui demande `start` crée une demande d'approbation qu'un humain
   libère ; les deux tentatives sont au journal.

## Ce que Moli ne fait pas ici

- Pas de carte, de zones, ni d'édition des créneaux du planning (seulement le mode :
  actif, arrêté, fête).
- Pas de fonctionnement sans Internet : la tondeuse n'a pas d'API locale.
