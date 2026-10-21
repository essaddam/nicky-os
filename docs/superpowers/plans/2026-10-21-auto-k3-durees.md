# AUTO-K3 — temps dans un état et couverture connue

Base : main d0ce806. Tranche complète, uniquement sources simulées.

- Domaine pur : définition typée, fenêtre glissante 1–604800 s, fraîcheur identique,
  intervalles demi-ouverts, durée acquise/connue/inconnue, entrée, limite de 256
  segments par calcul. Les segments écartés redeviennent inconnus et sont signalés.
- Réutiliser les têtes computed, CAS/PIN, bindings et consommateurs transitifs.
  Vérifier cycles et profondeur sur le graphe mixte agrégats/durées ; aucune unité
  implicite de la source vers la durée de sortie, toujours en secondes.
- Capturer chaque état utile, même égal, dans un index inverse synchrone borné.
  Aucun coalescement de transitions. Un seul rédacteur/ordonnanceur durable,
  checkpoint au plus une fois par seconde, fichier versionné de 4 Mio avec StateFile.
  Seuls les checkpoints durables sont projetés. En cas d’erreur, projection nulle,
  journal et suspension ; aucune réponse durable fictive.
- Reprise : conserver les segments acquis jusqu’au dernier checkpoint, fermer
  l’intervalle ouvert à cet instant ; cache, indisponibilité, interruption et
  événements anciens ne rouvrent pas une couverture. Nouvelle observation requise.
  Horloge monotone en processus ; recul civil à la reprise : attendre le dernier
  instant durable avant de compter à nouveau.
- Surfaces communes REST/MCP/CLI de définitions et lecture du bilan typé ; éditeur
  fr/en, couverture visible, OpenAPI, sauvegarde et documentation.
- Tests rouges puis verts : frontières, égalité/désordre, péremption/hors ligne,
  horloge, troncature, panne/reprise répétée, coupures avant/après rename, validation
  humaine, mélange avec K1 et budget. fmt/Clippy strict et tests ciblés ; complet
  réservé à l’intégration après revue Rust. Commits français, aucune donnée privée.
