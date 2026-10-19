# Netatmo météo

Pilote natif `moli-netatmo` : station (`NAMain`), module extérieur (`NAModule1`) et
pluviomètre (`NAModule3`), en lecture seule. Il n’est pas qualifié sur une installation réelle.
Intérieur/vent non établis dans l’inventaire de référence : à confirmer à la qualification humaine,
sans promettre leur prise en charge. Caméras, chauffage, projecteurs et médias hors périmètre.

- **Repris** : aucun code ni capture privée. Références : [OAuth officiel](https://dev.netatmo.com/apidocumentation/oauth),
  [API météo](https://dev.netatmo.com/apidocumentation/weather),
  [SDK officiel 9694823](https://github.com/Netatmo/Netatmo-API-PHP/tree/9694823b9dee4effd25ce4b43a2431fb2447ab15),
  sémantique des capteurs de [HA 2026.10.0](https://github.com/home-assistant/core/tree/2026.10.0/homeassistant/components/netatmo).
- **Synthétique** : `fixtures/weather.json` est entièrement construit pour les tests, avec identités fictives,
  horodatages fixes et valeurs choisies ; ce n’est pas une réponse d’appareil capturée. Aucun jeton ou secret.
- **Deviné / à confirmer** : compatibilité des réponses réelles, période du cumul de pluie de la station,
  comparaison des capacités et unités après H1. Aucun modèle supplémentaire n’est promis.
  Les indices radio/Wi-Fi sans unité physique confirmée ne deviennent pas des dBm ou des pourcentages inventés.
- **Temps** : les échantillons du hub portent l’heure de réception Moli ; `dashboard_data.time_utc`
  décide de la fraîcheur et ne sert pas à antidater des écritures d’historique.

L’autorisation exige une origine de retour configurée par un humain, joignable par son navigateur
et enregistrée à l’identique chez Netatmo : HTTPS hors boucle locale, par exemple via son accès
distant déjà protégé. Aucun compte réel ni accès fournisseur ne fait partie des tests de développement.

## Fonctionnement

L’instance activée expose une carte de connexion dans le système Moli. Elle ne reçoit
aucun secret d’application : un humain fournit ceux-ci au coffre. Le consentement est
lié à sa session PIN exacte, limité à `read_station`, et valide dix minutes. Le retour
GET est statique, sans écho des paramètres ; son script les enlève de l’adresse puis
les transmet par POST de même origine. Aucune URL de consentement n’est conservée dans
le stockage du navigateur. Déconnexion, expiration ou changement de PIN invalident le retour.

Une seule tâche supervisée possède les jetons et les échanges fournisseur. Elle conserve
le couple complet versionné dans `oauth_tokens`, via le hub, avant de l’utiliser. Aucun
rejeu d’un renouvellement dont l’envoi est ambigu : nouvelle autorisation humaine.
L’arrêt n’attend pas une sauvegarde asynchrone ; le coffre garde l’ancien ou le nouveau
couple complet. À la reprise, un refus du renouvellement de l’ancien couple attend
l’humain. Aucun marqueur de reprise ni rédacteur détaché n’est ajouté au runtime.

Les requêtes TLS vont exclusivement à `api.netatmo.com:443`, sans redirection ni retry
implicite, en dix secondes au plus. Réponse OAuth ≤ 16 Kio, météo ≤ 512 Kio ; 16 stations,
16 modules par station et 128 identités connues par instance. Une file de quatre contrôles
humains évite d’accumuler les demandes ; les contrôles abandonnés sont refusés avant envoi.

Une mesure absente, inaccessible ou périmée devient inconnue et le module hors ligne.
Un horodatage répété ne rajeunit pas les valeurs ; une valeur stable ne crée pas
d’écriture d’historique supplémentaire. Les champs absents deviennent inconnus même lorsque
l’horodatage n’a pas changé. L’expiration continue pendant un échange réseau en attente.

| Points | Famille B1 |
|---|---|
| température, humidité, CO₂, pression | `climate` |
| batterie | `diagnostic` |
| bruit, pluie et cumuls | `other` |

## Preuves et reste

Les contrats synthétiques couvrent décodeur, état OAuth à usage unique, classification
avant/après envoi, réponse hostile, sauvegarde avant activation, annulation, renouvellement,
fraîcheur et disponibilité. Les routes sont exercées avec le vrai routeur, les sessions PIN,
un coffre temporaire chiffré et une instance native ; aucun échange fournisseur valide.
Le binaire vérifie construction, callback, activation/désactivation et paquet de catalogue.
Les tests web couvrent les destinations de consentement et la taille des réponses.

Commandes de vérification : `sh scripts/check.sh`, `cd web && npm ci && npm test && npm run build`,
puis `node scripts/i18n-check.mjs` à la racine. Ces vérifications logicielles ne constituent
pas une qualification fournisseur ou appareil.

**Restant après livraison logicielle** : autorisation, renouvellement et comparaison des modules
sur un compte réel après le jalon humain H1, puis transfert explicite de la fonction depuis HA.
Le statut « connecté » confirme une session fournisseur fonctionnelle ; la disponibilité des
mesures dépend séparément de leurs dates et de la joignabilité des modules.

Installation : [onboarding.md](onboarding.md). Contrat HTTP : [openapi.yaml](openapi.yaml).
