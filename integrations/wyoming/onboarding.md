# Ajouter la supervision d'un service Wyoming

Ce paquet est en attente de validation finale : ne pas activer de pilote réel
avant cette validation et H1.
Avant H1, seuls les tests de contrat synthétiques sur boucle locale sont autorisés.

Après H1 et validation du pilote :

1. Le propriétaire indique le service à surveiller : Piper, Whisper ou
   openWakeWord ; il fournit explicitement son hôte et son port. L'agent ne
   cherche pas d'adresse et ne choisit pas de port à sa place (D5).
2. Choisir un identifiant d'instance stable, distinct pour chaque service.
   L'appareil sera `<instance>:service`, indépendamment de son nom ou de son hôte.
3. Ajouter une instance dans la configuration privée ; aucun nom personnel,
   hôte réel ou identifiant de maison dans le dépôt public.
4. Valider la configuration hors réseau avec `moli-os check-config`.
5. Après autorisation de qualification, vérifier seulement `describe` → `info` :
   disponibilité, programme, version, modèles et voix annoncés. Ne pas émettre
   d'audio, de commande `detect`, de test d'annonce ou de mot d'éveil.
6. Vérifier avec le propriétaire la panne puis la reprise sur un service de test
   autorisé ; ne jamais arrêter un service utilisé par la maison pour ce contrôle.

## Configuration privée, après validation et H1

Exemple entièrement fictif : remplacer l'hôte et le port uniquement par ceux
fournis par le propriétaire, sans conserver cet exemple comme cible réelle.

```toml
[[driver]]
id = "voice-synthesis"
kind = "wyoming"

[driver.options]
host = "service.example.invalid"
port = 12345
every = 60
timeout = 4
```

Créer une instance distincte pour chaque service. Le port est obligatoire et
ne sera jamais déduit du type Piper, Whisper ou openWakeWord. Les hôtes admis
sont des IP locales ou des noms DNS ASCII valides ; aucune URL, aucun identifiant
dans l'hôte, aucune découverte automatique.

Options : `host` et `port` obligatoires ; `every = 60` secondes
(10–3600), `timeout = 4` secondes (1–10). Tous les points sont en lecture seule.
Une disponibilité vraie atteste seulement une réponse de description valide,
pas le bon fonctionnement de la synthèse, de la transcription ou du mot d'éveil.

## Diagnostic

Une description invalide, une erreur distante ou un délai dépassé met le
service hors ligne et efface ses anciennes métadonnées. `last_error` expose
un diagnostic local expurgé, sans adresse, jeton ou texte reçu du service.
Une sonde valide remet la disponibilité à vrai et efface l'erreur ; aucun
nouvel essai ne part avant la période complète après le résultat précédent.
Les points `available`, `last_error`, `models`, `program`, `version` et `voices`
refusent tous les écritures, y compris celles d'un agent : aucune approbation
de la garde n'est créée. L'arrêt de l'instance ferme la sonde et remet le
service hors ligne.

Les appels vocaux existants de Moli et Home Assistant gardent leur configuration.
Ne pas modifier le pipeline voix ni saisir de secret pour ce pilote de supervision.
