# Onboarding : télé Samsung Tizen

1. Demander l'adresse locale de la télé et, pour le réveil, sa MAC. Vérifier que
   l'API Tizen v2 est disponible et que le contrôle distant est autorisé.
2. Activer le coffre chiffré de Moli avec la clé maître existante ; ne jamais la
   régénérer. Aucun jeton dans la configuration.
3. Déclarer `kind = "samsung"`, `host`, éventuellement `mac` ; ports par défaut
   `port = 8002` (TLS obligatoire), `upnp_port = 9197`, `every = 30` secondes.
   Lancer `moli-os check-config`, puis redémarrer Moli.
4. Pour le premier appairage, allumer la télé et accepter **Moli** à l'écran.
   Le pilote épingle et stocke d'abord le certificat, puis obtient le jeton et
   l'enregistre dans le coffre. L'identité native est conservée dans le coffre.
5. Une fois appairée, la télé peut démarrer en veille : l'identité conservée
   permet de lui adresser le réveil, sans inventer un état « allumée ».
6. Certificat changé : vérifier physiquement l'appareil avant d'oublier le secret
   `cert` avec `moli-os secrets forget <instance> cert`, puis redémarrer.
   Jeton révoqué : oublier `token`, redémarrer et accepter Moli à nouveau.
7. Qualification réelle uniquement après H1, humain présent, sur une cible
   neutre, jamais une pièce protégée ; remettre l'état initial. Cette livraison
   utilise uniquement des serveurs simulés et des fixtures synthétiques.
