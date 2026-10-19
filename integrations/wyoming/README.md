# Wyoming : supervision locale

Un appareil par service explicitement configuré, six points en lecture seule,
une sonde `describe` → `info`. Aucun audio, téléchargement de modèle, commande
de mot d'éveil ou changement du pipeline vocal. Home Assistant conserve l'usage
voix jusqu'à validation humaine.

## Repris

- Le codec de `moli-net::wyoming`, également utilisé par l'assistant et les
  annonces Sonos : encodage partagé, API `describe` ajoutée, framing durci.
- Les rubriques `asr`, `tts`, `wake` du protocole
  [Wyoming](https://github.com/OHF-Voice/wyoming/blob/main/wyoming/info.py) et
  son [framing](https://github.com/OHF-Voice/wyoming/blob/main/wyoming/event.py).
- Aucun code Python ou Home Assistant copié ; aucune nouvelle dépendance
  externe d'exécution. La crate réutilise les composants Rust du projet.

## Deviné / simulé

Les trois fixtures `.wyoming` sont des trames synthétiques, anonymes dès leur
création. Noms de modèles et voix, versions et attributions sont fictifs.
Chaque trame contient un en-tête terminé par LF et un bloc JSON de longueur
exacte en octets UTF-8, sans audio. Elles sont rejouées par les contrats du
codec et du pilote : ce ne sont pas des captures de services installés.

L'identité fixe `<instance>:service` permet de modifier une adresse, un port ou
un nom sans créer un nouvel appareil. Les hôtes et ports proviennent toujours
du propriétaire ; ni découverte ni choix d'adresse par un agent.

## État vérifié

Le passage ciblé a exécuté 32 contrats réussis, dont les cinq de cycle de vie
sur faux serveur TCP. Après correction du lint signalé et enregistrement dans
le binaire, le garde-fou complet a réussi le 19 octobre 2026 : formatage,
Clippy strict, 735 tests, catalogue et contrôle de fuites génériques, codes 0.
Wyoming et ses trois fixtures sont validés par le catalogue ; aucun de ses six
avertissements préexistants ne concerne ce paquet. La liste noire privée
n'était pas présente. Détails : [contrat et état](../../docs/components/wyoming.md).

## Reste

- Faire relire le pilote et son paquet par l'architecte désigné. Les preuves
  obtenues sur la base `835d9b8` ne qualifient pas une future fusion sur une
  autre base.
- Après H1 seulement, qualifier chaque service installé avec les adresses
  fournies par le propriétaire, sans audio ni déclenchement de mot d'éveil.
- La disponibilité signifie qu'une description valide a été reçue. Elle ne
  prouve pas la qualité de la synthèse, de la transcription ou du mot d'éveil.

Le pilote ne possède ni microphone, ni satellite, ni pipeline vocal. Les
appels audio existants de Moli gardent leur configuration et leur implémentation.
