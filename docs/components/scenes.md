# Scènes : état voulu et confirmations (AUTO-S1)

Une scène est une liste ordonnée d’affectations de points lisibles et inscriptibles.
Elle applique des valeurs scalaires typées, jamais un toggle, un bouton, un incrément
ou une commande de minuteur. Le moteur est déterministe et n’appelle aucun LLM.
Les aides de date et d’heure utilisent leur normalisation habituelle, notamment
l’instant canonique avec fuseau pour une date-heure.

## Configuration et validation

```toml
[[automations.scene]]
id = "scenes:desired"
[automations.scene.definition]
parallel = 2
timeout_s = 60
confirmation_s = 5
targets = [{ point = "helpers:lamp/on", value = true }]
```

Les points sont des exemples fictifs. Ils doivent être déclarés par un pilote.
La configuration initialise un **brouillon**, sans jamais approuver ni remplacer un
courant déjà enregistré. La portée réelle doit exister et être compatible avant
publication et lancement. Un point absent ou devenu incompatible refuse cette
prévalidation, avant tout ordre ; une cible explicitement hors ligne échoue dans
son résultat d’exécution, sans empêcher les autres cibles indépendantes.

Les scènes étendent le propriétaire de dépendances d’AUTO-H5 : mêmes deux têtes
brouillon/courant, CAS, fichier `dependencies.json`, rédacteur unique, sauvegarde et
verrou commun avec les graphes. L’aperçu de publication lie aussi les cibles, leurs
membres déclarés, leurs pièces et les empreintes des descriptions/étiquettes.
La portée du courant est enregistrée atomiquement avec ses versions. Un changement
de cette portée ou des graphes concernés invalide l’aperçu (**409**). Si les membres,
pièces ou descriptions diffèrent ensuite de cette portée validée, aucune nouvelle
exécution ne peut l’élargir silencieusement, y compris depuis un graphe. Enregistrer
un nouveau brouillon puis le publier avec PIN autorise explicitement la nouvelle
portée ; réapprouver l’ancienne révision ne la modifie pas. Après reprise, une portée
identique à celle enregistrée reste utilisable sans rejouer un ancien lancement.
Seule une session PIN existante peut publier cet aperçu exact ; un en-tête `ui`
ne prouve pas une validation humaine. Enregistrer un brouillon, humain compris,
ne modifie jamais le courant ni les graphes déjà validés.

La publication n’envoie aucun ordre. Un nœud
`{"id":"apply","type":"scene","scene":"scenes:desired"}` référence l’identité stable,
couverte par l’empreinte du graphe. Il résout le courant validé à chaque étape.
Sans courant, sa validation est refusée. Publier un nouveau courant avec les
consommateurs montrés ne change pas l’empreinte du graphe. Une exécution déjà admise
conserve son instantané ; les étapes suivantes résolvent le nouveau courant.

## Exécution bornée

Lire l’aperçu du courant puis le transmettre au lancement : identité, version,
portée et empreinte sont revérifiées avant admission. L’origine et l’acteur restent
ceux de l’appelant ; un lancement depuis graphe validé est une action d’automatisme.
Chaque ordre passe par le hub, sa garde et son permis vivant. Les commandes dérivées
aux membres conservent cette autorité ; une annulation, un changement de portée ou
une intention extérieure concurrente peut encore empêcher un envoi non engagé.

Les portées qui partagent un appareil sont regroupées dans une même voie séquentielle,
en conservant l’ordre déclaré. Les voies indépendantes peuvent avancer en parallèle.
Au début d’une cible suivante de même portée, les continuations non émises de la
cible précédente sont révoquées : elles ne peuvent pas la dépasser. Un nouveau
lancement chevauchant une scène encore active est refusé, sans file cachée.

| Borne | Valeur |
|---|---|
| Définitions communes aux scènes et plannings | 128, fichier d’état de 4 Mio |
| Cibles déclarées par scène | 1–32, doublons égaux normalisés, contradictions refusées |
| JSON d’une définition / d’une portée | 64 Kio chacun |
| Taille d’un point / d’une valeur texte | 256 / 2 000 octets |
| Appareils dans une portée, racines et membres compris | 128 au total, membres directs seulement |
| Pièce dans l’aperçu | 240 octets au plus |
| Scènes actives | 4, sans file supplémentaire |
| Voies simultanées | 1–4 par scène, 4 au total dans le moteur de scènes |
| Durée totale, attentes comprises | 1–120 s |
| Attente de confirmation après acquittement | 1–30 s, au plus la durée totale |
| Résultats terminés gardés en mémoire | 64, détails d’erreur de 512 caractères au plus |
| Portées de continuations encore vivantes | 512 appareils au plus ; références faibles, échéance d’origine |

La concurrence borne les commandes de cibles de scène ; un pilote composé conserve
son propre contrat de distribution interne. Une confirmation porte sur le **point
cible**, selon la sémantique publiée par ce pilote. Pour un groupe, elle ne prouve
pas individuellement l’état de chacun de ses membres. Les membres et les pièces
restent visibles dans l’aperçu et soumis à la garde.

Les durées et permis utilisent une horloge monotone. Un effet déjà remis au
transport n’est pas réversible. Une continuation composée confirmée peut survivre
à la réponse du parent jusqu’à l’échéance d’origine ; elle reste révocable par une
nouvelle intention sur un membre, une annulation explicite ou une suspension des
dépendances. Les références faibles expirées sont retirées à la prochaine admission
ou commande sur leur portée. Aucun ordonnanceur de scène ne tourne au repos.

## Résultats et confirmation

Chaque cible a un état propre :

- `already_conform` : son échantillon courant correspond déjà à la valeur voulue,
  n’est pas futur et a au plus `confirmation_s` secondes ; aucun ordre n’est envoyé,
  aucun acquittement n’est fabriqué ;
- `held` : la garde attend un humain ; l’identité de la demande est fournie ;
- `acknowledged` : le pilote a acquitté l’ordre ; ce n’est pas une confirmation d’état ;
- `confirmed` : après admission de l’ordre et dans le délai de confirmation, une
  observation correspond à la cible ;
- `failed`, `confirmation_expired`, `superseded` ou `cancelled` : résultat terminal
  explicite, avec l’acquittement éventuel conservé séparément.

Le suivi reçoit aussi les observations de valeur égale, que le bus déduplique.
L’instant de capture précède l’attente du verrou du modèle : une ancienne observation
retardée ne devient pas une confirmation nouvelle. Une observation capturée après
le délai ne valide pas non plus un consommateur réveillé tardivement. Les demandes
retenues n’autorisent aucun envoi après annulation ou expiration ; elles sont retirées
et journalisées comme expirées. Après approbation, le délai de confirmation part de
l’acquittement, dans la borne totale initiale.

Le lancement est `done` seulement si chaque cible est confirmée ou déjà conforme.
Sinon il est `partial`, ou `cancelled` après annulation explicite. Aucun rollback
physique ni transaction multi-appareils n’est promis. Les résultats décrivent les
observations de chaque cible, pas un état simultané garanti de toute la scène.

## Reprise, journal et surfaces

Les définitions ont la durabilité d’AUTO-H5 : temporaire, fsync, rename et fsync du
répertoire avant acquittement ; pas de réinitialisation silencieuse d’un fichier
corrompu. Une issue incertaine ferme les dépendances et révoque les intentions de
scène encore non émises. La reprise choisit le courant enregistré et expose le
brouillon séparément.

Les lancements sont éphémères. **Aucun lancement ni ordre n’est rejoué au redémarrage**.
Un ancien identifiant d’exécution devient inconnu ; consulter le journal pour son
historique et une éventuelle interruption entre effet et confirmation.

Début, états intermédiaires, résultats par cible, fin et annulation entrent au
journal (`Action::Scene`). Les commandes et approbations gardent leurs traces
habituelles. L’identifiant d’exécution corrèle toute la scène ; `parent_correlation`
relie un lancement de graphe à son occurrence d’origine lorsqu’elle existe.

REST et erreurs : [OpenAPI des scènes](../api/scenes.openapi.yaml) et
[des dépendances](../api/dependencies.openapi.yaml). Les invités n’y accèdent pas ;
les mutations de démonstration sont refusées. MCP expose `preview_scene`, `run_scene`,
`get_scene_run`, `cancel_scene_run`, et les outils communs de dépendances avec
`kind: "scene"`. Ils ne peuvent pas publier une définition.

```text
moli-os dependency get scenes:desired --kind scene
moli-os dependency draft scenes:desired 1 definition.json --kind scene
moli-os scene preview scenes:desired
moli-os scene run scenes:desired preview.json
moli-os scene status <identifiant>
moli-os scene cancel <identifiant>
```

Les fichiers CLI sont bornés à 256 Kio. Le JSON de définition est
`{"type":"scene","value":{"targets":[{"point":"helpers:lamp/on","value":true}],"parallel":2,"timeout_s":60,"confirmation_s":5}}`.
Le fichier de lancement est l’aperçu exact lu sur le serveur, pas une liste d’ordres
libres. La publication se fait depuis l’éditeur de l’onglet Automatismes ou REST,
avec la session humaine existante ; aucun PIN n’est passé sur la ligne de commande.
