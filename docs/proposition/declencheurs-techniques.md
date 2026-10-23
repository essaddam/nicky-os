# Proposition — Déclencheurs techniques des automatismes

Date : 21 octobre 2026. Statut : proposition de modèle technique à relire.
Cette proposition décrit une cible et des connecteurs possibles ; elle ne déclare pas
tous ces mécanismes livrés. Elle respecte les décisions de
[l’architecture](../ARCHITECTURE.md). Les exemples sont fictifs.

## 1. Objet et vocabulaire

Recenser les entrées techniques nécessaires à un moteur d’automatismes sans créer
un type de déclencheur pour chaque appareil ou chaque usage.

La proposition sépare quatre notions :

| Notion | Rôle | Exemple |
|---|---|---|
| Source | Produit l’information | Capteur, iPhone, récepteur Bluetooth, boîte mail, agenda, Teams, WhatsApp |
| Adaptateur | Acquiert et normalise l’information | Pilote matériel, polling HTTP, abonnement, webhook |
| Déclencheur | Décide à quel instant lancer | Changement d’état, occurrence reçue, échéance atteinte |
| Condition | Autorise ou écarte le lancement | Maison occupée, message issu d’un canal autorisé |

Le chemin proposé est :

```text
Sources → Adaptateurs → États / mesures / occurrences / échéances
                                  ↓
                       Déclencheur et conditions
                                  ↓
                 Scénario validé → hub → garde → pilote
```

Un lecteur NFC et un webhook nécessitent des adaptateurs différents, mais peuvent alimenter
le même mécanisme « occurrence reçue ». Un polling d’API est un mode d’acquisition :
il peut produire un état, une mesure inchangée ou plusieurs nouvelles occurrences.

## 2. Quatre mécanismes et une invocation explicite

| Mécanisme | Signal technique | Configuration minimale | Sémantique |
|---|---|---|---|
| Planification | Une échéance calculée est atteinte | Date, heure ou règle, fuseau, politique de reprise | Heure civile ou instant absolu |
| Minuterie | Une durée s’écoule après un départ | Durée, démarrage, remise à zéro, annulation | Temps écoulé |
| Mutation d’état | Une donnée passe de A à B | Référence stable, filtre sur ancienne/nouvelle valeur | Transition, pas simple réception |
| Réception d’événement | Une occurrence est publiée | Source, type, filtre, identité de l’occurrence | Deux occurrences identiques peuvent compter deux fois |
| Invocation explicite | Un appel demande un lancement | Scénario, arguments, identité et droits de l’appelant | Commande directe du moteur |

L’invocation explicite est une entrée d’exécution, pas nécessairement un abonnement.
Elle peut être représentée comme un événement interne si cela simplifie le contrat,
sans changer ses contrôles d’autorisation.

### 2.1 Planification

La famille comprend : date ponctuelle, horaire quotidien, expression cron,
récurrence de calendrier, intervalle aligné sur l’horloge, lever/coucher du soleil,
début/fin d’un rendez-vous et décalage par rapport à ce rendez-vous.

Chaque fournisseur calcule la prochaine échéance. L’ordonnanceur peut donc traiter
« à 08:00 » et « dix minutes avant le prochain rendez-vous » avec le même mécanisme,
même si les fournisseurs de dates sont différents.

Une récurrence cron et un intervalle écoulé ne sont pas interchangeables : « toutes
les heures à la minute 30 » et « toutes les 90 minutes depuis l’activation »
n’ont pas le même point de départ. Le modèle doit préciser l’ancrage.

### 2.2 Minuterie

Une minuterie peut être démarrée, réinitialisée, suspendue si le contrat le prévoit,
annulée ou remplacée. Une nouvelle génération invalide l’expiration de l’ancienne.
Cela sert au délai simple, au maintien d’un état et à l’absence de réponse.

Le temps écoulé est mesuré avec une horloge monotone pendant le fonctionnement.
Cette horloge ne traverse pas un redémarrage : toute persistance nécessite une
échéance durable et une politique explicite de restauration ou d’abandon.

### 2.3 Mutation d’état

Le moteur compare une valeur précédente et une nouvelle valeur typées. Les
variantes comprennent le passage vers une valeur, le franchissement d’un seuil,
l’entrée dans une plage ou la transition d’un résultat calculé de faux à vrai.

L’initialisation d’un état et sa restauration depuis un cache ne sont pas
automatiquement des transitions observées. Le contrat précise si une règle doit
évaluer la situation présente au démarrage. Une donnée inconnue ou périmée n’est
pas assimilée à une valeur métier, telle que « absent » ou « fermé ».

### 2.4 Réception d’événement

Un événement représente une occurrence : appui, scan, message reçu, fichier créé,
action terminée, décision humaine, démarrage ou déconnexion d’un composant.

Deux scans du même tag donnent deux identités d’occurrence. En revanche, la
retransmission réseau du même scan conserve son identité logique et doit pouvoir
être dédupliquée. Le contenu seul ne permet pas de distinguer ces situations.

## 3. Adaptateurs d’entrée à considérer

Ce catalogue décrit des possibilités techniques, pas une liste de connecteurs
actuellement disponibles ou qualifiés.

| Entrée | Acquisition possible | Sortie normalisée |
|---|---|---|
| Webhook HTTP | Requête entrante vérifiée et décodée | Occurrence ou invocation autorisée |
| API interrogée | Polling périodique, curseur ou lecture incrémentale | État, mesure et/ou occurrences |
| MQTT ou bus de messages | Abonnement à des topics ou files | État ou occurrence selon le contrat du message |
| WebSocket ou SSE | Connexion persistante, reprise avec curseur si disponible | État et/ou occurrence |
| TCP, UDP ou protocole propriétaire | Écoute réseau et décodeur | État et/ou occurrence |
| Zigbee, BLE, USB, série, GPIO | Pilote du bus ou passerelle | État, mesure ou occurrence |
| NFC, RFID, QR code, code-barres | Lecteur ou application mobile | Occurrence de lecture |
| GPS / localisation de l’iPhone | Application autorisée, mises à jour de position ou surveillance de zones | Mesure de position, état de zone, entrée/sortie de zone |
| Localisation Bluetooth par pièce | Récepteurs BLE fixes ou téléphone détectant des balises de pièce | Mesure de proximité, pièce estimée, entrée/sortie de pièce |
| Interface, raccourci, montre, CLI, API | Appel explicite | Invocation ou occurrence |
| Commande vocale reconnue | Intentions structurées d’un composant autorisé | Invocation ou occurrence |
| Fichier ou répertoire | Surveillance du système de fichiers | Création, modification, suppression |
| Base de données | Notifications, journal de changements ou polling | Mutation et/ou occurrence |
| Processus ou système d’exploitation | Supervision et notifications système | Démarrage, arrêt, erreur, changement réseau |
| Bus interne | Publication par un composant du moteur | Cycle de vie, approbation, résultat d’action ou de scénario |
| Mail | API du fournisseur, abonnement ou suivi de boîte compatible | Message reçu/modifié/supprimé, état calculé |
| Agenda | API, synchronisation incrémentale, export ICS ou protocole compatible | Rendez-vous normalisé et échéances dérivées |
| Teams | Connecteur autorisé à des ressources de messagerie et/ou de présence | Message, modification, mention structurée, état de présence |
| WhatsApp | Connecteur autorisé pour le compte et l’API retenus | Message entrant, réponse structurée, statut si exposé |

Le transport ne décide pas de la nature du signal. Un message MQTT peut être une
température à conserver ou un appui à compter. Le début d’un abonnement ou une
reconnexion ne prouve pas que chaque donnée reçue représente une nouvelle occurrence.

## 4. Trois signaux à distinguer absolument

Les noms suivants décrivent le contrat proposé ; ils ne sont pas tous des noms
d’API existants.

| Signal | Signification | Même valeur reçue deux fois |
|---|---|---|
| `sample_received` | Une nouvelle mesure a été acquise | Deux réceptions ; la fraîcheur peut être actualisée |
| `state_changed` | La valeur courante a réellement changé | Aucun deuxième changement si la valeur reste identique |
| `event_received` | Une nouvelle occurrence s’est produite | Deux événements si les identités d’occurrence diffèrent |

Exemple : le capteur transmet 21 °C, puis encore 21 °C. Deux mesures sont reçues,
mais la température n’a pas changé. Le capteur reste vivant. À l’inverse, deux
appuis « simple » restent deux occurrences, même avec le même contenu.

La fraîcheur doit distinguer instant de mesure à la source et instant de réception.
Recevoir de nouveau une ancienne mesure depuis un cache ne prouve pas une nouvelle
observation. Une règle de silence précise donc si elle suit le transport, les
mesures nouvelles ou la disponibilité métier de la source.

## 5. Mail, Teams et WhatsApp

Les messageries sont des sources d’événements. Elles ne nécessitent pas un moteur
distinct par fournisseur.

| Cas | Signal à normaliser | Filtre déterministe possible |
|---|---|---|
| Nouveau mail | `message.received` | Compte, dossier, expéditeur autorisé, étiquette |
| Mail déplacé ou marqué | `message.updated` | Ancienne/nouvelle propriété, révision |
| Nouveau message Teams | `message.received` | Conversation ou canal sélectionné |
| Mention Teams | Événement ou propriété structurée de message | Mention du compte suivi, si exposée |
| Message WhatsApp entrant | `message.received` | Compte et conversation autorisés |
| Réponse à un bouton de message | `interaction.received` | Référence de demande, choix et identité vérifiée |
| Absence de réponse | Expiration d’une minuterie | Demande toujours ouverte et non annulée |

Les noms ci-dessus sont indicatifs. Le connecteur conserve le type de changement :
recevoir une modification ou un accusé ne doit pas être confondu avec un nouveau
message. Il maintient son curseur et une identité de message pour éviter de relancer
sur tout l’historique lors d’une reconnexion.

Microsoft Graph fournit des notifications de changement pour les messages Outlook,
les événements Outlook, les messages Teams et certaines informations de présence.
Le choix exact des ressources, permissions et abonnements dépend du compte et de
l’organisation : voir la [documentation Microsoft](https://learn.microsoft.com/en-us/graph/change-notifications-overview).

WhatsApp doit faire l’objet d’un choix explicite de connecteur. L’offre officielle
documentée par Meta est la WhatsApp Business Platform, avec des API de messagerie ;
cela ne suffit pas à promettre l’accès à tous les échanges d’un compte personnel.
La compatibilité du compte et les événements disponibles restent à qualifier :
voir les [collections officielles Meta](https://www.postman.com/meta/whatsapp-business-platform/overview).

Le texte libre d’un message est une donnée, pas une autorisation d’exécuter des
commandes. Les règles utilisent des champs structurés ou des filtres explicites.
Aucune décision de commande physique ne dépend d’un LLM à l’exécution. La réception
d’un message n’autorise pas automatiquement l’envoi d’une réponse à un tiers.

## 6. Agenda et réunion : dix minutes avant

Un agenda expose deux choses différentes : les changements du rendez-vous et les
dates auxquelles des actions doivent être envisagées.

| Situation | Traitement technique |
|---|---|
| Rendez-vous créé | Normaliser l’objet et calculer ses échéances |
| Horaire ou durée modifiés | Invalider les anciennes échéances et recalculer |
| Rendez-vous annulé ou supprimé | Annuler ses échéances encore en attente |
| Début moins dix minutes atteint | Publier une occurrence à l’échéance calculée |
| Début ou fin atteints | Publier la frontière de calendrier correspondante |
| Participation effective à un appel | Consommer un signal distinct du client de réunion, s’il existe |

Le calendrier prévoit une réunion. Il ne prouve pas que la personne est en appel.
Un statut de présence Teams ou une télémétrie de session est une autre source,
dont la signification doit être définie par le connecteur.

### 6.1 Calcul et replanification

Exemple fictif, dans un fuseau explicite :

```text
Réunion prévue à 10:00 → anticipation -10 min → échéance à 09:50
Réunion déplacée à 10:30 → ancienne échéance annulée → nouvelle échéance à 10:20
Réunion annulée → échéance à 10:20 annulée
```

La règle est `échéance = début de l’occurrence + décalage`. Le décalage peut être
négatif, nul ou positif ; il peut aussi s’appliquer à la fin du rendez-vous.
La minuterie n’est pas armée « dix minutes après réception de l’invitation ».

Chaque échéance référence une source, une occurrence de rendez-vous, sa révision
et une règle. Une révision remplacée ne doit plus produire d’effet. La vérification
de génération et l’admission du lancement doivent être coordonnées afin de traiter
une modification qui arrive au même instant que l’échéance.

### 6.2 Situations à définir dans le contrat

- **Récurrence** : distinguer série, occurrence, exception déplacée et occurrence annulée.
  Développer les récurrences dans un horizon et un nombre d’occurrences bornés.
- **Fuseau** : conserver la zone de la source et les instants normalisés ; définir les
  heures ambiguës ou inexistantes lors d’un changement d’heure.
- **Journée entière** : ne pas inventer un horaire de réunion ; appliquer une règle
  distincte ou ignorer cette catégorie si elle n’a pas de sens pour l’automatisme.
- **Rendez-vous découvert tardivement** : choisir ignorer, prévenir maintenant dans une
  tolérance bornée, ou attendre la prochaine frontière. Ne pas rattraper implicitement.
- **Déplacement après un premier rappel** : préciser si le nouveau créneau autorise un
  nouveau rappel ; dédupliquer les transports sans effacer cette décision métier.
- **Événements simultanés** : définir une politique de regroupement ou de concurrence.
- **Panne de synchronisation** : conserver la dernière information en la marquant périmée ;
  n’agir que selon la politique de fraîcheur validée. Une annulation distante non encore
  synchronisée reste inconnue du moteur.
- **Redémarrage** : reconstruire les échéances futures et appliquer une politique explicite
  aux échéances dépassées ; une commande physique manquée n’est pas rejouée par défaut.

Les notifications push de Google Calendar signalent les modifications des ressources ;
elles ne remplacent pas l’ordonnanceur local chargé de l’anticipation de dix minutes.
Voir [Google Calendar : notifications push](https://developers.google.com/workspace/calendar/api/guides/push).

## 7. Localisation des personnes : GPS et Bluetooth par pièce

Deux échelles sont à distinguer : la zone géographique autour de la maison et la
pièce à l’intérieur. Le GPS et le Bluetooth sont des sources et des méthodes de
mesure ; les déclencheurs du moteur restent des mutations et des occurrences.

| Situation | Information normalisée | Mécanisme du moteur |
|---|---|---|
| Nouvelle position de l’iPhone | Position, précision et date de mesure | `sample_received` |
| Position qui change | Ancienne/nouvelle position, si son exposition est autorisée | `state_changed` |
| Personne entrant ou sortant d’une zone | Transition d’appartenance à une zone identifiée | Mutation puis occurrence d’entrée/sortie |
| Franchissement d’une distance définie | Distance calculée vers une zone ou un point de référence | Mutation et seuil |
| Personne localisée dans une autre pièce par Bluetooth | Ancienne/nouvelle pièce estimée, qualité | Mutation puis occurrence de passage |
| Maintien de présence dans une pièce | État de pièce stable pendant la durée définie | Mutation et minuterie |
| Signal GPS ou BLE devenu trop ancien | Qualité périmée ou localisation inconnue | Minuterie de fraîcheur et changement de qualité |

### 7.1 GPS de l’iPhone et zones géographiques

L’application acquiert une position ou un signal d’entrée/sortie de zone selon ses
capacités et autorisations. Le connecteur associe l’appareil à une personne par une
liaison explicite, puis normalise les données. La zone peut être calculée sur le
téléphone ou côté moteur ; il faut désigner la source faisant autorité pour éviter
de publier deux entrées pour la même transition.

Une zone a une identité stable, une géométrie privée et une politique de stabilité.
La précision de mesure, l’hystérésis aux frontières et une éventuelle durée de
confirmation évitent les allers-retours artificiels. Une donnée trop ancienne ou
imprécise ne permet pas de conclure à une nouvelle entrée ou sortie.

Sur iOS, les mises à jour en arrière-plan dépendent des autorisations et du cycle
de vie de l’application ; leur disponibilité ne doit pas être traitée comme un
flux continu garanti. Le connecteur expose les interruptions et la fraîcheur :
voir [Apple : localisation en arrière-plan](https://developer.apple.com/documentation/corelocation/handling-location-updates-in-the-background).

Les coordonnées brutes ne sont pas nécessaires dans chaque événement de scénario.
Une référence opaque de personne, une référence de zone, la transition et sa
qualité suffisent souvent. L’exposition et la conservation des positions précises
restent explicitement configurées et privées.

### 7.2 Bluetooth et estimation de la pièce

Deux architectures possibles : des récepteurs fixes dans les pièces observent un
appareil ou une balise portée ; ou le téléphone détecte des balises fixes associées
aux pièces et transmet son estimation. Ce choix appartient au connecteur, pas au
type de déclencheur du moteur.

La force du signal reçue (RSSI) est une observation, pas une preuve de présence dans
une pièce. L’estimation combine mesures récentes, écart entre récepteurs, durée
de stabilité et règles de départage. Elle doit pouvoir rendre « pièce inconnue »
ou « estimation incertaine ». Apple précise également que la proximité d’une balise
n’est pas une position précise et que les mesures fluctuent avec les interférences :
voir [Apple : précision d’une balise](https://developer.apple.com/documentation/corelocation/clbeacon/accuracy).

Un téléphone n’est pas supposé émettre en permanence une identité Bluetooth stable
et exploitable. Il faut qualifier le matériel et le mode d’émission, gérer les
identifiants changeants et définir une association autorisée à la personne. Une
balise BLE dédiée peut constituer une autre source. Le mode iBeacon annoncé par
une application iOS a notamment ses propres contraintes de premier plan :
voir [Apple : transformer un appareil iOS en iBeacon](https://developer.apple.com/documentation/corelocation/turning-an-ios-device-into-an-ibeacon-device).

### 7.3 Présence, identité et arbitrage

Une personne peut avoir plusieurs appareils ; un appareil peut rester dans une
pièce sans elle. Le modèle distingue observations des appareils et présence
estimée de la personne. Il conserve provenance, date et qualité de l’estimation.

Si les sources GPS, Bluetooth et Wi-Fi se contredisent, une politique déterministe
explicite décide : priorité, fraîcheur, corroboration ou résultat inconnu. Perdre
le Bluetooth dans une pièce ne prouve pas une sortie de la maison. Un retour
d’observation après panne ne constitue pas automatiquement une nouvelle arrivée.
Une estimation de localisation ne donne pas, à elle seule, le droit d’ouvrir une
porte ou de contourner la garde.

## 8. Règles composées à construire au-dessus

Ces règles réutilisent les mécanismes de base. Elles ajoutent de la mémoire et
des opérateurs ; elles ne créent pas un nouveau protocole d’entrée.

| Besoin | Composition technique |
|---|---|
| Seuil ou plage | Mutation d’état et comparaison ; hystérésis si nécessaire |
| Expression qui devient vraie | Suivi des dépendances et mémorisation du résultat précédent |
| État vrai pendant une durée | Mutation et minuterie annulable |
| Aucune nouvelle mesure | Minuterie réinitialisée sur la mesure pertinente |
| Aucune réponse à une demande | Occurrence initiale, corrélation, minuterie et annulation sur réponse |
| A puis B dans une durée | Automate de séquence et fenêtre temporelle |
| N événements dans une fenêtre | Compteur ou historique glissant borné |
| Hausse rapide ou moyenne dépassée | Valeur dérivée déterministe, puis mutation ou seuil |
| Première arrivée ou dernier départ | Agrégation des états de présence et transition du résultat |
| Entrée stable dans une zone GPS ou une pièce BLE | Estimation de localisation, hystérésis et confirmation temporelle |

Une expression surveille toutes ses dépendances : si `A ET B` devient vraie parce
que B change en dernier, elle doit aussi pouvoir déclencher. Des déclencheurs
indépendants reliés par OU et des conditions évaluées après lancement ne constituent
pas automatiquement ce mécanisme.

« À 22:00, si quelqu’un est présent » lance seulement à l’échéance. « Dès qu’il
est après 22:00 et que quelqu’un est présent » doit aussi réagir à une arrivée à
22:15. Le temps est alors une dépendance de l’expression.

## 9. Informations minimales et livraison

Un contrat normalisé doit pouvoir exprimer les informations suivantes, avec un
schéma versionné et des champs adaptés au signal.

| Information | Raison |
|---|---|
| Identité stable de source et de donnée | Éviter les références fondées sur un nom modifiable |
| Nature du signal et valeur typée | Distinguer mesure, mutation, occurrence et échéance |
| Identité d’occurrence ou de ressource/révision | Séparer répétition légitime et retransmission |
| Instant à la source et instant de réception | Fraîcheur, retard et ordre des observations |
| Provenance, cause et corrélation | Relier message, rendez-vous, déclenchement et exécution |
| Qualité et état de disponibilité | Signaler donnée inconnue, restaurée, périmée ou source hors ligne |
| Référence de règle et révision | Annuler les lancements d’une configuration remplacée |
| Échéance, génération et politique de reprise | Invalider une minuterie ou un rendez-vous remplacé |

L’ordre d’arrivée réseau n’est pas une garantie d’ordre métier. Les connecteurs
doivent traiter les révisions et curseurs disponibles. Sans version ordonnable,
ils peuvent devoir relire l’état courant du fournisseur.

La déduplication est bornée en mémoire et/ou persistée selon la politique de
livraison. Il faut distinguer déduplication du transport, admission d’un lancement
et effet physique : un identifiant d’événement ne garantit pas à lui seul un effet
« exactement une fois » après panne. Les commandes restent soumises à leurs
échéances et au suivi de résultat du hub.

## 10. Contraintes de fonctionnement

- **Déterminisme** : filtres et opérateurs explicites ; aucun LLM dans les décisions de commande.
- **Validation humaine** : un nouvel automatisme ou une nouvelle logique reste soumis au
  mécanisme de validation existant ; une source réseau n’accorde aucun droit implicite.
- **Autorité** : l’origine et l’acteur sont établis côté serveur. Un champ reçu dans un
  mail, un webhook ou un message ne prouve pas l’identité d’une personne.
- **Confidentialité** : ne publier sur le bus commun que les références opaques et champs
  nécessaires. Les corps de mail, titres privés, participants, liens de réunion et
  coordonnées précises ne deviennent pas des points publics de la maison. Les
  secrets restent au coffre.
- **Bornes** : limiter payloads, abonnements, récurrences développées, fenêtres,
  minuteries, déduplication et files. Traiter explicitement pertes et retard.
- **Reprise** : séparer la reconstruction des états du rejeu d’occurrences ; ne pas
  transformer une synchronisation initiale, un message conservé ou une restauration
  de cache en nouvelle action physique sans politique validée.
- **Boucles** : conserver la cause et limiter les chaînes, la concurrence et le débit
  afin qu’un scénario ne se relance pas indéfiniment sur ses propres effets.
- **Performance** : router par source, point ou type vers les abonnés concernés ;
  ne pas scanner tous les scénarios à chaque événement. Mutualiser les lectures
  d’un même connecteur et libérer abonnements/minuteries devenus inutiles.

## 11. Correspondance avec le dépôt actuel

Repérage au 21 octobre 2026 ; les fichiers de code font foi si la documentation
du composant présente un état plus ancien.

| Élément constaté | Référence | Conséquence pour la proposition |
|---|---|---|
| Horaires, soleil, intervalle, démarrage, manuel | [Modèle des automatismes](../../crates/moli-automation/src/model.rs) | Des mécanismes de base sont déjà représentés ; cela ne prouve pas un support cron général |
| Changements d’état, seuils et maintien `for_s` | [Documentation des automatismes](../components/automatismes.md) | Réutiliser ces sémantiques et préciser leur reprise |
| Déclencheur `when_occurrence` et événement `Occurrence` | [Modèle](../../crates/moli-automation/src/model.rs), [bus](../../crates/moli-core/src/event.rs) | La séparation état/occurrence existe déjà dans le code actuel |
| Traitement des occurrences et expirations de minuteries | [Moteur](../../crates/moli-automation/src/engine.rs) | Vérifier les contrats existants avant toute extension |
| Calendrier ICS de réservations | [Réservations](../components/reservations.md) | Source spécialisée déjà documentée, sans décalage d’arrivée/départ et sans calendrier de réunions général |

Les connecteurs mail, Teams, WhatsApp, l’anticipation générique de réunions et les
règles de localisation GPS/BLE décrites ici sont des pistes de cette proposition ;
ce document n’en affirme ni l’implémentation complète ni la qualification. Il ne
modifie aucun contrat existant de réservation ou d’accès.

## 12. Cas d’acceptation pour une future implémentation

| Vérification | Résultat attendu |
|---|---|
| Deux mesures nouvelles à 21 °C | Deux réceptions, aucun deuxième changement d’état |
| Deux scans distincts du même tag | Deux occurrences et deux lancements autorisés |
| Même occurrence retransmise | Pas de deuxième admission dans la fenêtre de déduplication définie |
| Mail ancien relu après reconnexion | Aucun lancement comme « nouveau mail » sans politique de rejeu explicite |
| Message modifié ou accusé reçu | Pas de confusion avec un nouveau message entrant |
| Réunion à 10:00, décalage -10 min | Échéance à 09:50 |
| Réunion déplacée à 10:30 avant le rappel | Ancienne échéance invalidée, nouvelle à 10:20 |
| Réunion annulée avant le rappel | Aucun lancement de cette échéance |
| Une occurrence d’une série est déplacée | Seule cette occurrence est replanifiée |
| Modification reçue au moment de l’échéance | Admission cohérente avec la révision retenue, trace explicable |
| Capteur transmettant une valeur stable | Pas d’alerte de silence tant que de nouvelles mesures fraîches arrivent |
| Réponse reçue avant expiration | Minuterie de non-réponse annulée |
| Redémarrage après l’échéance d’une commande physique | Pas de rattrapage implicite |
| Scénario modifié pendant une attente | Ancienne exécution annulée selon le contrat de validation |
| Expression `A ET B`, B change en dernier | Une transition vers vrai peut déclencher |
| Positions GPS oscillant à la frontière d’une zone | Pas d’alternance entrée/sortie tant que les règles de stabilité ne sont pas satisfaites |
| Position GPS reçue avec un ancien instant de mesure | Pas de présence déclarée fraîche sur la seule réception |
| Deux récepteurs BLE voient le même appareil | Une seule estimation de pièce, ou résultat incertain explicite |
| Bluetooth perdu alors que la présence maison reste établie | Pièce inconnue possible, pas de départ de la maison déduit automatiquement |
| Localisation restaurée après panne | Pas de nouvelle arrivée physique inventée par la restauration |

Ces cas décrivent des preuves à obtenir, pas des tests déclarés réussis.

## 13. Ordre proposé et références

Convergence selon deux critères : couverture de besoins distincts et réutilisation
des mécanismes existants. L’ordre ci-dessous n’est pas une estimation de charge.

1. **Stabiliser les contrats de signal et de livraison.** Vérifier mesures, états,
   occurrences et minuteries déjà présents ; éprouver répétition et retransmission.
2. **Généraliser les échéances dérivées.** Éprouver une réunion à -10 minutes,
   son déplacement et son annulation, sans effet physique.
3. **Ajouter les connecteurs prioritaires.** Choisir un fournisseur de mail/agenda,
   une messagerie et les sources de localisation utiles ; qualifier permissions,
   synchronisation, qualité et reprise avec des fixtures anonymisées ; ne pas
   promettre une compatibilité universelle.
4. **Étendre les opérateurs composés.** Ajouter les besoins réellement rencontrés,
   en bornant mémoire, fenêtres et coût de réévaluation.

Références de cadrage :

- [Architecture du projet](../ARCHITECTURE.md), [automatismes](../components/automatismes.md),
  [cœur](../components/coeur.md), [réservations](../components/reservations.md).
- [Home Assistant : événements](https://www.home-assistant.io/integrations/event/),
  pour la distinction entre état et signal ponctuel.
- [Microsoft Graph : notifications de changement](https://learn.microsoft.com/en-us/graph/change-notifications-overview),
  pour les ressources mail, agenda et Teams (référence consultée le 21 octobre 2026).
- [Google Calendar : notifications push](https://developers.google.com/workspace/calendar/api/guides/push),
  pour la synchronisation des modifications d’agenda.
- [Meta : collections officielles WhatsApp Business Platform](https://www.postman.com/meta/whatsapp-business-platform/overview),
  pour le périmètre de messagerie officielle.
- [Apple : localisation en arrière-plan](https://developer.apple.com/documentation/corelocation/handling-location-updates-in-the-background),
  [précision d’une balise](https://developer.apple.com/documentation/corelocation/clbeacon/accuracy) et
  [appareil iOS en iBeacon](https://developer.apple.com/documentation/corelocation/turning-an-ios-device-into-an-ibeacon-device),
  pour les limites des sources GPS et Bluetooth.

Les vidéos évoquées dans la discussion n’ont pas été consultées : aucun lien ou
transcrit n’était disponible dans ce dépôt. Leurs scénarios pourront compléter ce
catalogue sans changer la séparation entre source, adaptateur et mécanisme.
