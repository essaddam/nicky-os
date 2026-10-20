# Ajouter un calendrier de réservations

1. Le propriétaire choisit l’export ICS HTTPS de son calendrier. Cette URL est un secret :
   la transmettre directement au coffre `moli-os secrets set sejours url` par un tube,
   jamais dans le TOML, une conversation, une ligne de commande ou le journal du shell.
   Un agent ne choisit pas ce fournisseur ni cette URL (décision D5).
2. Déclarer l’instance et son fuseau explicite :

   ```toml
   [[driver]]
   id = "sejours"
   kind = "reservations"
   [driver.options]
   timezone = "Europe/Paris"
   # poll_seconds = 300   # de 30 à 3600 s
   ```

3. Redémarrer Moli. Le calendrier et cinq emplacements de prochaines réservations sont
   visibles dans l’Atelier. Un emplacement vide est hors ligne. Une réservation publie
   sa référence opaque, son début/sa fin (millisecondes Unix) et sa fenêtre active.
4. Pour limiter un invité, choisir sa réservation dans **Personnes**. Ses pièces et sa
   date de fin restent nécessaires : la liaison ne crée ni personne ni droit. Une
   réservation absente, annulée, expirée ou un calendrier hors ligne ferme cet accès.
   L’API accepte également une référence opaque déjà publiée et son instance ; ne jamais
   utiliser le UID privé du fournisseur. Les identifiants d’un séjour plus lointain seront
   proposés lorsqu’il entrera parmi les cinq prochains séjours.
5. Vérifier sur les fixtures, puis faire qualifier le calendrier réel par le propriétaire.
   HA reste propriétaire de l’installation tant que cette qualification n’est pas faite.

Les modifications du calendrier sont découvertes à la prochaine lecture. Un échec ferme
les accès dès qu’il est constaté ; une lecture perdue ne maintient pas indéfiniment un
ancien instantané. Changer l’URL au coffre demande de redémarrer l’instance.
