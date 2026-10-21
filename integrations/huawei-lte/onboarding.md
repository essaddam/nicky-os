# Ajouter la lecture Huawei HiLink

Avant H1 : uniquement les contrats sur fixtures/serveurs synthétiques. Après validation
et H1, humain présent sur cible neutre, jamais une pièce protégée, état remis à l'identique.
HA garde ses fonctions ; aucune activation ni bascule d'usage implicite.

1. Le propriétaire fournit lui-même hôte, port, transport et contrat de firmware.
   L'agent ne cherche pas d'URL/adresse, ne scanne pas et ne choisit pas de variante.
2. Choisir un identifiant d'instance stable. Dans une configuration privée, sélectionner
   explicitement `auth = "sha256-v4"` ou `"none"` (lectures publiques seulement).
   Session `sestok` par défaut ou `token` si le contrat a été vérifié ; pas de fallback.
3. En mode sha256-v4, l'humain saisit `username` et `password` dans le coffre de
   l'instance (`moli-os secrets set <instance> <nom>`, valeur sur stdin issue de sa
   saisie, jamais dans un fichier, une commande historisée, un test ou la conversation).
   En mode none, ces deux secrets ne sont pas nécessaires. Le coffre reste requis
   pour identité et épinglage ; ne jamais régénérer sa clé maître.
4. `moli-os check-config` valide hors réseau. Après H1 seulement, la poignée de main
   HTTPS est épinglée et `cert_sha256` persistée avant session ou identifiants ;
   `device_identity` reçoit SerialNumber avant la première publication.
5. Comparer signal, unités et compteurs avec la source existante sans provoquer
   d'arrêt radio/WAN, de retrait SIM, de changement de mot de passe ou de logout humain.
   Contrôler charge, rythme et nombre de points ; remettre la cible à son état initial.
6. Valider séparément chaque usage avant d'en changer le propriétaire. Aucun point
   de présence ou SMS n'est fourni ; ne pas modifier la présence existante.

Exemple **fictif** ; remplacer la cible uniquement par celle fournie par le propriétaire :

```toml
[[driver]]
id = "mobile-network-diagnostic"
kind = "huawei-lte"
[driver.options]
host = "modem.example.invalid"
port = 8443
transport = "https"
auth = "sha256-v4"
session = "sestok"
every = 60
info_every = 3600
```

Hôte/port/transport/auth obligatoires. DNS ASCII et IPv4 privées/loopback, aucune URL,
userinfo, IPv6, query ou chemin libre. HTTP exige `transport = "http"` **et**
`allow_plain_auth = true`, même sans login ; aucun repli automatique de HTTPS.
Périodes30–86400 s, info_every au moins every. Aucun secret dans le TOML.

Un refus/verrouillage/type inconnu attend un humain et ne tente pas de variantes.
Vérifier le compte et le contrat avant de redémarrer. Un changement de certificat
ou d'identité exige une vérification humaine ; ne rien oublier automatiquement au
coffre. Les identités/cache déjà connus empêchent l'admission d'un autre modem sous
la même instance. Les erreurs ne recopient ni corps distant ni matériel de session.
