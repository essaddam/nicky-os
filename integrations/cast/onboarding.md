# Ajouter un récepteur Cast

1. Garder les enceintes audio et groupes dans Music Assistant, les téléviseurs
   déjà configurés dans leur pilote TV. Choisir un récepteur vidéo dédié.
2. Créer une instance `cast` avec une seule des configurations suivantes :

```toml
[[driver]]
id = "screen-cast"
kind = "cast"
[driver.options]
host = "192.168.1.10" # adresse privée fixe ; proposition manuelle, aucun scan
```

Ou remplacer la ligne `host` par :

```toml
interface = "192.168.1.2" # adresse privée de l’interface de Moli
prefix = 24              # /16 à /30, défaut /24
# external_ids = ["00000000000000000000000000000001"] # ID appartenant à MA
```

3. Redémarrer Moli. Aucun nouvel appareil n’est publié automatiquement.
4. Dans **Système**, choisir **Rechercher les récepteurs**. Vérifier la cible
   physique, le nom, l’adresse et l’identité proposée. L’adresse manuelle est
   proposée sans découverte réseau et doit désigner un récepteur vidéo.
5. Confirmer que cet appareil n’appartient pas à un autre pilote ; choisir
   **Valider cet écran** et le PIN humain. Les conflits connus sont refusés.
6. Dans **Salon**, les commandes deviennent disponibles selon les observations
   réelles. Lancer un média depuis une application Cast ; Moli contrôle cette
   session existante. Un volume inconnu ou un récepteur hors ligne restent
   désactivés, sans valeur supposée.
7. Pour retirer ou remplacer la cible : **Désactiver ce récepteur** dans Système,
   attendre la confirmation, puis modifier la configuration. En cas de pin TLS
   changé, vérifier physiquement l’appareil avant cette nouvelle validation.

API détaillée : [openapi.json](openapi.json). Aucune URL, adresse, identité ou
empreinte n’est acceptée comme nouvelle cible dans le corps d’activation : seule
la sélection exacte issue de la génération découverte est autorisée.
