# Autoriser une station météo Netatmo

Le pilote lit la station `NAMain`, le module extérieur `NAModule1` et le pluviomètre
`NAModule3`. Il ne commande aucun appareil. Les modules intérieurs additionnels,
l'anémomètre, les caméras et le chauffage ne sont pas pris en charge par ce paquet.

## Préparer l'application et le coffre

Un humain crée son application dans le portail développeur Netatmo. Il choisit l'origine
publique par laquelle **son navigateur** joint Moli et enregistre exactement ce retour :
`<origine>/api/integrations/netatmo/meteo/callback`. Le pilote n'invente aucune URL
d'installation, ne découvre pas de serveur et ne contacte jamais cette origine.
L'accès au fournisseur est fixé à `api.netatmo.com:443` avec TLS.

L'origine est HTTPS, sans chemin, barre finale, utilisateur, requête ni fragment.
Le retour utilise l'origine canonique du navigateur : nom d'hôte en minuscules,
adresse IP normalisée et port par défaut omis (`:443` en HTTPS, `:80` en HTTP).
Enregistrer cette forme chez Netatmo, même si la configuration emploie une autre casse
ou écrit explicitement le port par défaut.
HTTP est accepté uniquement sur la boucle locale pour le développement. Si Moli est
derrière un accès distant, conserver sa protection existante, transmettre le `Host`
public et ajouter ce nom à `server.allowed_hosts`. Un alias différent est refusé pour
l'autorisation et son retour.

Configurer l'instance, sans secret :

```toml
[[driver]]
id = "meteo"
kind = "netatmo"

[driver.options]
public_origin = "https://weather.example.com" # à remplacer par l'origine choisie par l'humain
poll_seconds = 300
stale_seconds = 1800
```

Les bornes sont 60–3600 secondes pour la lecture et 300–86400 secondes pour la
fraîcheur, avec une fraîcheur au moins égale à deux périodes de lecture. L'identifiant
d'instance utilise 1–64 lettres ASCII, chiffres, tirets ou soulignements ; ne pas le
renommer après l'autorisation.

Un humain fournit `client_id` et `client_secret` au coffre de cette instance par un
tube depuis son gestionnaire de secrets : `moli-os secrets set meteo client_id`, puis
`moli-os secrets set meteo client_secret`. La valeur n'est jamais un argument de
commande, un fichier de configuration, un test ou un message. Conserver la clé maître
existante ; ne jamais la régénérer. Redémarrer Moli après ces apports.

## Consentement et renouvellement

Dans le système Moli, l'humain ouvre l'autorisation Netatmo avec sa session PIN.
Seul le périmètre `read_station` est demandé. Le retour enlève immédiatement les
paramètres de l'adresse affichée, puis remet le code au serveur par un POST de même
origine. La session PIN qui a commencé l'opération doit encore être valide : même
adresse IP ne signifie pas même session. Déconnexion, changement du PIN ou expiration
imposent de recommencer. L'état aléatoire expire après dix minutes et ne sert qu'une fois.

Les jetons sont écrits ensemble dans `oauth_tokens`, une entrée chiffrée gérée par le
pilote, **avant** toute lecture météo. Ne pas saisir ni modifier cette entrée à la main.
Le renouvellement est automatique avant expiration. Si un envoi de renouvellement a
pu atteindre Netatmo mais que son résultat est inconnu, aucun rejeu automatique n'est
tenté : une nouvelle autorisation humaine est nécessaire. Après une interruption pendant
la sauvegarde, le coffre contient l'ancien couple complet ou le nouveau ; le redémarrage
peut effectuer au plus un renouvellement effectif avant d'attendre l'humain en cas de refus.

## Disponibilité et validation

Un module absent, déclaré inaccessible ou dont la mesure est trop ancienne devient
hors ligne ; ses valeurs deviennent inconnues. Un horodatage fournisseur répété ne
prolonge pas leur fraîcheur. Une valeur inchangée ne crée pas d'écriture d'historique
supplémentaire. Les températures, humidités, CO₂ et pressions relèvent de `climate`,
la batterie de `diagnostic`, le bruit et les cumuls de pluie de `other` (politiques B1).
Le cumul de pluie garde un libellé neutre : sa période réelle reste à confirmer.

Avant H1, utiliser exclusivement les fixtures synthétiques et les contrats. Après H1,
qualifier avec un humain présent sur une cible neutre, jamais dans une pièce protégée,
et remettre l'état initial. Comparer modules, unités, fraîcheur, perte de réseau,
renouvellement et redémarrage. HA conserve la fonction jusqu'à validation Moli ;
le transfert doit laisser un seul propriétaire d'exécution par usage. Les caméras
exigent un sous-ticket distinct si leur usage est confirmé.
