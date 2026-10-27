# Charge synthétique de vingt maisons — protocole

Objectif demandé : rejouer dix copies de chacune des deux installations HA,
avec des variations, et observer mémoire, CPU, délais et faux workflows.

Le générateur lit les exports privés existants et remplace les identités et
textes par des données synthétiques en RAM. Ses processus et mesures restent
distincts du système testé. Aucun secret n'est nécessaire. Le worker testé
emploie les vrais House::run, Hub, broker POC et moteur moli-automation ; il
ne possède aucun pilote physique et n'accepte que des connexions loopback.

## Plan d'exécution

1. Créer et vérifier les fixtures : mêmes comptes de registres, sémantiques,
   unités et catalogues ; identités uniques entre copies. Les états absents du
   registre restent comptés comme tels, sans inventer d'entité.
2. Transporter les états par de vrais sockets TCP MQTT locaux ; conserver les
   limites du broker existant. Comparer un broker partagé avec des brokers
   isolés par maison. Aucun plafond de production n'est relevé pour réussir.
3. Ajouter par maison trois entrées synthétiques pour les workflows : température,
   ouverture, mode texte. Elles passent elles aussi par MQTT puis la projection.
   Deux graphes par maison utilisent le vrai moteur : seuil + condition + attente
   + sortie simulée, et ouverture + notification locale. Aucun envoi externe.
4. Séparer démarrage, échauffement, activité mesurée et vidage des workflows.
   Mesurer RSS, allocations demandées, CPU rapporté à un cœur, messages,
   retained, erreurs, graphes terminés et compteurs attendus/obtenus.
5. Comparer deux maisons, vingt maisons, stockage RAM/disque et débit amplifié.
   Publier résultats agrégés et limites, y compris les échecs, dans le dossier V6.

## Critères et limites

- Aucun socket dirigé vers un appareil, HA ou broker réel ; le 1414 courant reste actif.
- Zéro commande physique ; aucune sortie simulée ne possède de route réseau.
- Les identités et valeurs privées ne sortent pas dans les fichiers de résultats.
- Une limite de retained ou une déconnexion est un résultat de charge, pas un succès.
- Les comptes de workflows doivent être rapprochés des vagues réellement émises.
- L'export historique n'est pas une capture exhaustive du trafic réel : la cadence
  et les variations sont fixées et documentées. Le générateur doit suivre la cadence.
- CPU debug sur le poste de développement, mémoire RSS et instrumentations locales
  ne qualifient pas un budget release sur la machine cible.
- La condition de mode texte ne démontre pas le support de conditions EnumCode.

Fichiers : `load_fixture.rs` (données), `load_wire.rs` (source MQTT),
`load_workflows.rs` (graphes et sorties simulées), `load_test.rs` (campagne).

## Complément : instantanés sous budget de 1 Gio

La demande suivante donne 1 Gio de budget au POC de charge. Sur macOS sans
Docker installé, `snapshot-load.py` surveille le RSS du worker toutes les
0,5 seconde et arrête son groupe en cas de franchissement. Cette garde
échantillonnée n'est pas un plafond cgroup strict et ne préalloue pas 1 Gio.

Deux captures natives sont réalisées avec MallocStackLogging lite et leaks,
puis analysées hors ligne par heap, vmmap et malloc_history. Les graphes et
sorties brutes restent dans un répertoire privé hors dépôt. Le reçu public
et les agrégats ne contiennent ni adresses de blocs, ni PID, ni chemins privés.

L'inspection perturbe l'exécution : le passage profilé ne remplace pas les
mesures CPU ni les validations fonctionnelles non instrumentées. Une
détection leaks doit être rapprochée de l'ownership Rust et de la libération
des connexions avant d'être qualifiée comme une fuite.
