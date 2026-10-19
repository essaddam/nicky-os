# Supervision des services Wyoming

## Contrat

Le pilote `wyoming` surveille les services locaux de synthèse (Piper), de
transcription (Whisper) et de mot d'éveil (openWakeWord). Home Assistant reste
propriétaire de l'usage voix jusqu'à validation humaine. Les appels audio de
`moli-assistant` et les annonces Sonos continuent d'utiliser `moli-net::wyoming`.

Une instance configurée représente un service, avec l'identité stable
`<instance>:service`. Changer son adresse ou son nom ne change pas cette identité.
`host` et `port` sont obligatoires, fournis et validés par le propriétaire (D5),
sans découverte ni adresse choisie par l'agent. Options prévues : `every`
(secondes, défaut 60, plage 10–3600) et `timeout` (secondes, défaut 4,
plage 1–10). Les options inconnues, ports nuls, hôtes vides, URL et adresses
contenant des identifiants sont refusés au chargement.
Les hôtes acceptés seront des littéraux IP locaux (privés, boucle locale ou
liaison locale), ou des noms DNS ASCII valides fournis par le propriétaire.
Un nom DNS sera limité à 253 octets et ses étiquettes à 63 ; aucun hôte ne
sera dérivé des informations annoncées par un service. Les noms et versions
des programmes ne participent jamais au choix de l'adresse.

Une sonde ouvre une connexion, envoie seulement `describe` avec l'encodeur
existant, lit un événement `info`, puis ferme la connexion. La résolution,
la connexion, l'écriture et la lecture partagent un seul délai. L'annulation
interrompt aussi une sonde en cours. Aucun nouvel essai immédiat ni rattrapage
des périodes manquées ; le prochain essai attend une période complète.

Le cadrage proposé borne séparément l'en-tête et le bloc JSON à 64 Kio.
L'en-tête doit finir par LF et être un objet ; les longueurs présentes doivent
être des entiers non négatifs. Les données en ligne et le bloc sont des objets,
fusionnés selon Wyoming (le bloc gagne). Une charge binaire non vide est
refusée avant allocation ou lecture : aucune réception audio. Les événements
inattendus, erreurs distantes, coupures et JSON mal formés échouent sans
réexposer leurs contenus. La profondeur JSON reste bornée par serde_json.

Les seules rubriques exploitées sont `asr`, `tts`, `wake` : au plus 16 programmes
par rubrique, 64 modèles ou voix par programme. Les noms et versions sont
limités à 128 caractères chacun ; les listes publiées à 2048 caractères.
Les champs inconnus, descriptions, liens et attributions ne sont pas exposés.
Les caractères de contrôle sont retirés des textes affichés. Les listes sont
dédupliquées et triées. Une rubrique reconnue mal formée ou aucune information
de programme exploitable fait échouer la sonde.

## Points prévus

| Clé | Type | Contenu |
|---|---|---|
| `available` | OnOff | Résultat de la dernière sonde, faux avant la première |
| `last_error` | texte | Diagnostic local traduit et expurgé ; vide après succès |
| `models` | texte | Modèles ASR et de mot d'éveil annoncés |
| `program` | texte | Noms des programmes annoncés |
| `version` | texte | Versions annoncées, vides si non fournies |
| `voices` | texte | Voix TTS annoncées |

Tous les points sont en lecture seule. La disponibilité de l'appareil suit
exactement la sonde ; une panne n'est pas un succès TCP. Les anciennes
métadonnées sont effacées après échec pour éviter de les présenter comme
actuelles. Le pilote n'émet ni audio, ni `detect`, ni commande à garder.

## Preuves et état

État : pilote, codec et paquet implémentés, garde-fou complet réussi le
19 octobre 2026 sur la base `835d9b8` : formatage, Clippy strict, 735 tests
(96 suites), catalogue et contrôle de fuites génériques, tous avec le code 0.
Le catalogue valide Wyoming et ses trois fixtures ; ses six avertissements
préexistants concernent d'autres profils. La liste noire privée n'était pas
présente. La revue architecte et la qualification réelle après H1 restent
distinctes de ces preuves ; aucune qualification d'une combinaison avec une
branche principale ultérieure n'est revendiquée.

Le premier contrôle sur la base figée `835d9b8` a compilé et exécuté les six
contrats initiaux : trois réussites, trois échecs attendus (LF absent, longueur
négative, données non objet). Ces échecs établissent les défauts du codec
d'origine, pas la validité de sa correction. Les boucles se sont arrêtées à
leur première assertion : les variantes suivantes n'ont pas été exercées.
Une erreur de séquencement du préflight est consignée au ticket ; ce contrôle
ne sera pas rejoué pour effacer cette réserve.

Le contrôle distinct du candidat corrigé, le 19 octobre 2026, a terminé avec
les codes 0 pour le formatage ciblé, Clippy `moli-net --all-targets` et les
16 tests Wyoming (14 contrats de supervision, 2 contrats audio simulés déjà
existants). Les trois tests de rejet initiaux réussissent désormais, variantes
incluses. Le préflight a terminé avec le code 0 avant le démarrage du conteneur,
et les 676 fichiers figés sont restés identiques après les vérifications.
Ce résultat concerne le codec sur la base `835d9b8`, pas le futur pilote,
le catalogue ou une combinaison avec la branche principale ultérieure.

Le premier contrôle de l'ossature configuration/métadonnées a compilé puis
exécuté onze contrats : deux réussites et neuf échecs comportementaux attendus,
code 101. Le préflight frais a terminé avec le code 0 avant Docker et les
680 fichiers figés sont restés identiques. Les boucles échouées n'ont pas
exercé leurs variantes après la première assertion. Ces résultats établissent
le point de départ du validateur et du parseur, sans valider leur correction.
Cinq contrats supplémentaires de cycle de vie sur faux serveur couvrent :
état initial et points en lecture seule, panne/reprise sans rafale, refus des
écritures avant la garde, délai et diagnostics expurgés, nettoyage lors de
l'abandon de la future du pilote. Leur premier passage, sur une interface
`Driver` volontairement inactive, a compilé puis exécuté les seize contrats :
les onze de configuration/métadonnées ont réussi ; les cinq de cycle de vie
ont échoué sur l'attente initiale de publication ou de sonde, avant leurs
autres assertions. Code 101, préflight 0 avant Docker, 681 sources inchangées.
Cela constitue leur point de départ, sans preuve des assertions suivantes.
La boucle a été implémentée après ce constat, sans nouveau contrôle à ce stade.

Le passage suivant a exécuté 16 contrats du pilote et 16 du codec, tous
réussis : les assertions de cycle de vie précédemment non atteintes sont
désormais exercées. Le formatage ciblé a réussi. Clippy a échoué sur
`single_match_else` (`runtime.rs`), donc le résultat global reste en échec.
Le remplacement par `if let … else` est préparé sans désactivation de règle.
Les sources ont conservé leurs 682 empreintes pendant ce contrôle, avec un
préflight code 0 avant Docker. Les preuves historiques restent distinctes
du contrôle complet du candidat final, de son catalogue et du binaire décrit ci-dessous.

Après correction du lint et enregistrement du pilote, le vrai
`sh scripts/check.sh` a terminé avec le code 0, sans nouvelle tentative :
735 tests réussis, aucun échec ou test ignoré, catalogue de 31 paquets valide.
Les 683 fichiers du candidat sont restés identiques avant et après le passage,
le préflight a terminé avec le code 0 avant Docker, et la durée mesurée était
de 880,315 secondes pour une borne de 900 secondes. Les dernières modifications
de documentation enregistrent uniquement ces résultats ; le code, le manifeste,
les traductions et les fixtures restent ceux du candidat contrôlé.

Les tests de contrat utilisent les fixtures d'`integrations/wyoming/fixtures/`
et des serveurs TCP de boucle locale. Ils couvrent fragmentation, fusion,
longueurs hostiles, EOF, délais globaux, indisponibilité, reprise, annulation
et refus d'écriture. Les tests audio existants sont rejoués en régression.

Les fixtures sont des exemples fictifs construits à partir du protocole,
pas des captures ni des preuves de compatibilité des services installés.
Après H1 seulement : le propriétaire fournit les adresses et autorise la
qualification de chaque service réel, sans audio ni déclenchement vocal.

Références consultées le 18 octobre 2026 :
[informations Wyoming](https://github.com/OHF-Voice/wyoming/blob/main/wyoming/info.py),
[framing Wyoming](https://github.com/OHF-Voice/wyoming/blob/main/wyoming/event.py).
Aucun code Python n'est importé ni exécuté par Moli.
