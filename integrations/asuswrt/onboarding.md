# Ajouter ASUSWRT en lecture seule

Avant H1 : uniquement les contrats synthétiques. Après validation du pilote,
un humain présent fournit lui-même la cible neutre, l'hôte, le port et HTTP/HTTPS.
L'agent ne choisit aucune adresse/URL, ne scanne pas le réseau et n'intervient
jamais dans une pièce protégée. HA garde ses usages jusqu'à validation séparée.

1. Choisir un identifiant d'instance stable et un compte autorisé par le
   propriétaire. L'authentification peut affecter les sessions : le propriétaire
   valide ce point après H1 ; aucun logout ni forçage d'une session distante.
2. Ajouter une configuration **privée** ; l'exemple ci-dessous est fictif et ne
   désigne aucun appareil réel. Aucun identifiant d'authentification dans ce fichier.
3. L'humain saisit `username` et `password` directement au coffre de cette instance
   (`moli-os secrets set <instance> <nom>`, valeur sur stdin depuis la saisie humaine,
   jamais dans un fichier, une commande historisée ou une conversation d'agent).
   Ne pas modifier ou régénérer la clé maître existante.
4. Valider hors réseau avec `moli-os check-config`. Après H1, le pilote épingle le
   certificat et persiste `cert_sha256` **avant** l'authentification, puis conserve
   `device_identity` au coffre avant de publier quoi que ce soit.
5. Laisser `selected = []`, puis sélectionner seulement les clients utiles par
   leur identité native fournie/validée par le propriétaire, au plus32. Ne pas
   transformer la liste DHCP/ARP en présence ni retirer la présence existante.
6. Contrôler les points de lecture, les unités et la charge ; remettre la cible
   dans son état initial. Une seule solution possède un usage après validation.

```toml
[[driver]]
id = "network-diagnostic"
kind = "asuswrt"
[driver.options]
host = "router.example.invalid"
port = 8443
transport = "https"
every = 60
inventory_every = 300
info_every = 3600
selected = []
```

`host`, `port`, `transport` sont obligatoires. Noms DNS ASCII et IPv4 privées/loopback
acceptés ; URL, userinfo, chemin, IPv6 et options inconnues refusés. Périodes30–86400 s,
inventaire/identité au moins aussi lents que la télémétrie. HTTP exige le choix
explicite `transport = "http"` **et** `allow_plain_auth = true` ; sinon configuration
refusée. HTTPS n'est jamais dégradé automatiquement vers HTTP.

Un refus/captcha/verrouillage arrête l'authentification automatique : vérifier
humainement le compte/firmware puis redémarrer l'instance. Une empreinte ou identité
changée reste bloquée : vérifier la cible avant toute suppression manuelle d'une
empreinte au coffre, jamais l'oublier automatiquement. Aucun corps distant ni
secret n'est recopié dans les diagnostics.
