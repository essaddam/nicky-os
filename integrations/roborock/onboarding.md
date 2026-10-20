# Mettre un aspirateur Roborock (Mi Home) dans Moli

L'aspirateur répond sur le réseau de la maison, en miIO, à qui connaît son **jeton** (32
chiffres hexadécimaux). Moli n'utilise aucun compte et aucun cloud.

1. **Vérifier l'appli** : l'aspirateur doit être appairé dans **Mi Home**. Appairé dans
   l'appli Roborock, il parle un autre protocole : ce pilote ne s'applique pas.
2. **Réserver son adresse** sur la box (bail fixe), et la noter.
3. **Récupérer le jeton**, par le propriétaire, sur sa machine : par exemple avec l'outil
   libre « Xiaomi Cloud Tokens Extractor » et son compte Xiaomi. Le jeton change si le Wi-Fi
   de l'aspirateur est réinitialisé.
4. **Ranger le jeton dans le coffre**, par un humain, jamais dans un fichier ni dans une
   conversation avec un agent :

   ```sh
   printf %s "$JETON" | docker exec -i moli-os /moli-os secrets set aspirateur token
   ```

   (`aspirateur` est l'`id` de l'instance déclarée ci-dessous.)
5. **Déclarer l'aspirateur** dans `moli.toml` :

   ```toml
   [[driver]]
   id = "aspirateur"
   kind = "roborock"
   [driver.options]
   host = "192.168.1.x"
   # name = "Aspirateur"
   ```

6. Redémarrer Moli. L'aspirateur apparaît ; lui donner sa pièce dans l'Atelier.

## Vérifier

- Le pilote passe « en marche » dès la première lecture. S'il attend : jeton manquant ou
  illisible, aspirateur injoignable (adresse, Wi-Fi, mauvais jeton : un mauvais jeton se voit
  comme un silence), ou appareil qui n'est pas un aspirateur Roborock.
- L'état suit l'aspirateur en 30 s au plus, en 5 s quand il travaille.

## Un seul propriétaire

Tant qu'un autre système pilote l'aspirateur (automatismes de Mi Home ou de HA), ne pas
recréer les mêmes dans Moli avant d'avoir validé la version Moli et retiré l'autre.

## Essayer une commande (jalon humain)

Seulement avec le propriétaire présent, dans une pièce neutre, **jamais dans une pièce
protégée** ni pendant une heure calme, et en remettant l'aspirateur à sa base.

1. Depuis l'Atelier, `control` → `start` : il quitte sa base, `state` passe à `cleaning`.
2. `control` → `pause`, puis `home` : il rentre, `state` passe à `returning_home` puis
   `charging`.
3. Un agent (MCP, Moli) qui demande `start` crée une demande d'approbation qu'un humain
   libère ; les deux tentatives sont au journal.
