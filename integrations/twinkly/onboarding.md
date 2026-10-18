# Ajouter une Twinkly locale

**Avant H1 : aucune connexion réelle, même pour lire le modèle ou se connecter.**
`verify` peut invalider la session HA ou de l'application. HA conserve la fonction
jusqu'à la qualification humaine Moli ; ne pas activer deux propriétaires concurrents.

1. Le propriétaire désigne un seul responsable d'exécution. Il relève hors dépôt
   l'adresse locale, la MAC stable, le modèle et le firmware. Aucun relevé privé
   ne doit rejoindre ce paquet public. Réserver l'adresse sur le réseau.
2. Après H1 seulement, déclarer une instance dans la configuration privée validée
   par l'humain, puis redémarrer Moli :

   ```toml
   [[driver]]
   id = "guirlande"
   kind = "twinkly"
   [driver.options]
   host = "192.168.1.x" # remplacer par l'adresse IPv4 privée validée
   mac = "aa:bb:cc:dd:ee:ff" # remplacer par la MAC réelle, hors dépôt
   # port = 80
   # resume_mode = "color" # uniquement choisi par l'humain si souhaité
   ```

   Aucun mot de passe, challenge ou jeton à saisir : le pilote crée une session
   temporaire uniquement dans sa mémoire. Il refuse les URL, adresses Internet,
   identités invalides et options inconnues. Il ne suit aucun déménagement réseau.
3. Vérifier l'identité publiée et les capacités. Sans RGB direct pris en charge,
   aucun point couleur n'est exposé. L'API v2/RGBW reste hors périmètre. Une lampe
   initialement éteinte sans mode de reprise connu peut être éteinte mais refuse
   l'allumage jusqu'à observation d'un mode compatible ou choix humain explicite.
4. Si la session est refusée, arrêter les propriétaires concurrents puis faire
   réexaminer l'instance par l'humain avant redémarrage. Moli ne mène aucun duel
   de reconnexion et ne rejoue pas les commandes échouées.

## Vérification humaine, après H1

Humain présent, cible neutre, **jamais une pièce protégée**. Relever l'état initial.
Observer puis tester marche, luminosité et, si annoncé, RGB statique ; vérifier la
relecture effective, y compris après extinction et remise en marche. Un refus
HTTP/applicatif ou une réponse perdue ne prouve pas l'absence d'effet : regarder
l'appareil, sans répéter aveuglément. Remettre l'état initial et consigner les
preuves dans l'espace privé. Les commandes passent par le hub et sa garde : une
demande d'agent en pièce protégée reste une approbation humaine, sans envoi.

Les points relèvent de `states`. Proposer une politique B1 explicite à l'humain,
sans la changer automatiquement ni réduire un plancher déjà acquis. La fraîcheur
vient des lectures utiles ; elle est indépendante de la conservation historique.
