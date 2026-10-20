# Installer une Android TV

1. Faire activer par l’humain le service Android TV Remote sur le téléviseur.
   Relever son nom DNS ou son adresse locale ; ne pas activer ADB.
2. Déclarer une instance stable et un hôte dans la configuration privée :

   ```toml
   [[driver]]
   id = "television"
   kind = "androidtv"
   [driver.options]
   host = "tv.example.invalid"
   ```

3. Vérifier la configuration, puis activer l’instance selon la procédure
   d’exploitation. Le coffre Moli doit être disponible ; conserver la clé
   maître existante. Aucun secret ni code dans la configuration.
4. Dans Système, choisir cette télévision et démarrer l’appairage. Le PIN
   Moli identifie l’humain ; le code hexadécimal affiché par la télé sert
   uniquement à cette tentative. Le saisir dans les deux minutes. Annuler
   ferme la tentative sans effacer l’identité déjà enregistrée.
5. Après appairage, attendre les observations avant de régler volume et
   sourdine. Une veille profonde peut rendre le service réseau inaccessible ;
   aucune commande en attente n’est rejouée au retour de la connexion.
6. Dans Salon, choisir le service et le type de cible. Copier le lien de
   partage du titre ou de l’épisode depuis le service ; ne pas remplacer un
   lien d’épisode par celui de sa série. Free TV : utiliser l’ID interne fourni
   par un lien de chaîne, jamais déduire l’ID du numéro dans le bouquet.
   Pour un replay, fournir le lien du programme ; le portail `/replay/123`
   ne désigne pas ce programme. Molotov ne propose que l’ouverture de l’appli.
7. Les favoris sont locaux à ce navigateur et séparés par téléviseur. Ils
   ne contiennent aucun compte fournisseur et ne sont pas synchronisés.
8. Une empreinte différente bloque la connexion : vérifier le matériel avec
   l’humain. Ne pas effacer automatiquement les épingles pour contourner ce
   refus. La validation sur matériel réel exige l’autorisation H1 et une
   cible neutre ; aucune pièce protégée ne sert de cible d’essai.
