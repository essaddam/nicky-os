# Redémarrage à chaud — Sarah Home : redémarrer le moteur sans rien perdre

Date : 23 octobre 2026. Statut : **proposition, aucune modification du code**. Base analysée : `main` au
commit `a835893`, et les sources de `rustix` 1.1.5 pour ce qui se fait sans `unsafe`.

Proposition à part, née du [monitoring](monitoring.md). Elle s'appuie sur l'[étude de haute disponibilité
v2](haute-disponibilite-v2.md) (l'état complet, l'arrêt figé, les modes, le pont maison), l'[analyse
V7](isolation-reactivite-runtime-v7.md) (les modules isolés, le courtier, le bail), l'[étude mémoire
v2](memoire-v2.md) (les deux horloges) et la [critique des workflows](critique-workflow.md) (les clés des
engagements).

Ta demande : **« si on démarre un module en mode isolé, il faut arriver à redémarrer le moteur sans ne
rien perdre ; à chaud, tout en conservant la mémoire. »**

Ce que j'entends par **le moteur** : le processus du cœur de Sarah Home (`moli-os serve`), c'est-à-dire
le hub, le courtier, le moteur des workflows, l'API, et les modules qui tournent dans le cœur. Le moteur
des workflows seul n'a pas besoin de redémarrer quand un automatisme change : il recharge déjà ses
définitions (`refresh_live`, `moli-automation/src/lib.rs:782-790`). Ce qui impose un
redémarrage du cœur :

- une nouvelle version ;
- un plantage, par exemple une panique du routeur du courtier (V7 § 9.4.3) ;
- un réglage qui ne se recharge pas : aujourd'hui, ranger un secret au coffre se termine par
  « puis redémarrer » (exploitation.md) ;
- passer au binaire de mesure du [monitoring](monitoring.md) (§ 6).

Les questions, reformulées (« comment pourrait-on… ») :

| # | Comment pourrait-on… | § |
|---|---|---|
| 1 | redémarrer le cœur **sans qu'un module isolé ne perde ses connexions**, ses appareils, ni ce qu'il voit pendant la coupure ? | 7.4 |
| 2 | faire passer **la mémoire du cœur** (valeurs, exécutions, zone de travail, demandes en attente, courbes) à l'instance suivante, **sans repasser par la carte SD** ? | 7.2 |
| 3 | qu'**aucun client ne voie une porte fermée** pendant la coupure : ni un écran, ni un module, ni la machine de secours ? | 7.3, 7.6 |
| 4 | **ne rien perdre même quand le cœur plante**, sans écrire de journal sur le disque ? | 7.7, 7.8 |
| 5 | faire la même chose quand **le nouveau binaire est une autre version**, et revenir en arrière ? | 7.9 |
| 6 | **prouver**, après coup, que rien n'a été perdu ? | 7.6, 10 |

Méthode, et techniques employées, nommées au fil du texte :

- **lecture du code** : ce que fait un redémarrage aujourd'hui (§ 2, annexe B) ;
- **premiers principes** : qu'est-ce qui doit survivre, au juste ? (§ 3) ;
- **analogies** : nginx, HAProxy, Envoy, le dépôt de descripteurs de systemd, Erlang, `shellflip` de
  Cloudflare, Home Assistant, CRIU (§ 4) ;
- **pré-mortem** : « le redémarrage a mal tourné : qu'a-t-on perdu, et pourquoi ? » ; vingt réponses,
  vingt parades (§ 5) ;
- **analyse morphologique** : où garder la mémoire, qui garde les portes, qui lance les modules, options
  côte à côte (§ 6) ;
- **convergence à critères visibles** (§ 6, § 12).

Vocabulaire :

- un **redémarrage à froid** est ce qui se passe aujourd'hui : tout s'arrête, on repart des fichiers ;
- un **redémarrage à chaud** : le cœur s'arrête et repart ; ce qui doit survivre ne passe ni par le
  disque, ni par l'arrêt des modules isolés ;
- le **gardien** est un petit processus, le même binaire (`moli-os gardien`). Il lance le cœur et les
  modules isolés, garde les portes et le relais, et ne fait rien d'autre ;
- les **portes** sont les sockets d'écoute : l'API et les écrans (8790), les écouteurs du courtier ;
- le **relais** est la mémoire du cœur, écrite en octets, confiée au gardien dans un `memfd` scellé. Son
  format est l'**état complet** de l'étude HA v2 (§ 16.2) : ce qui voyage par MQTT s'appelle l'état
  complet, ce qui s'écrit sur le disque s'appelle l'**instantané**, ce que garde le gardien s'appelle le
  relais ;
- la **file** est celle du pont maison (HA v2 § 6.4) : les 10 000 dernières sorties critiques, chacune
  avec son rang `s` ;
- un **accusé** dit à un module isolé que tout ce qu'il a donné jusqu'à son numéro `k` est rangé dans le
  relais ou dans la file : il peut l'oublier ;
- l'**époque** `E` et le **bail** sont ceux de la V7 (§ 18.1.7) : la seule machine autorisée à agir, et le
  droit qu'elle renouvelle auprès du témoin.

## 1. Le verdict en bref

- **Oui, à une condition : qu'un petit processus reste quand le cœur s'arrête.** C'est le gardien. Il
  garde trois choses pendant le redémarrage :
  - **les portes** : un client qui se connecte attend dans la file du noyau, il n'est pas refusé ;
  - **la mémoire du cœur** : le relais, dans un `memfd` scellé ;
  - **les modules isolés** : ce sont ses enfants, pas ceux du cœur.

  Sous systemd, sur le Pi (D10), le dépôt de descripteurs de systemd peut en plus garder ceux du gardien
  lui-même.
- **On conserve le contenu de la mémoire, pas la mémoire.** Un nouveau binaire ne peut pas reprendre
  l'ancien tas :
  - les adresses changent à chaque lancement ;
  - Rust ne promet pas la disposition d'une structure d'une compilation à l'autre ;
  - chaque `Vec` ou `HashMap` pointe dans l'ancien processus.

  Ce qui passe, c'est l'état complet de l'étude HA v2 : 233 Kio à ×1. Il se construit en 132 µs et se
  relit en 272 µs (banc HA v2). Le passage de la mémoire coûte donc moins d'une milliseconde à ×1, et
  quelques millisecondes à ×10.
- **Le gain n'est pas la vitesse : le disque ne coûte que 3 ms** (l'instantané avec `fsync`, HA v2).
  Le gain est ailleurs :
  - **ce que le disque n'a jamais pu garder** : les portes ouvertes et les modules vivants ;
  - **le plantage** : un relais rafraîchi et la file du pont tenue par le gardien font qu'un plantage du
    cœur ne perd plus que le dernier lot de 10 ms. Avec l'instantané seul, il perdrait jusqu'à sa
    période, de 15 min à 1 h ;
  - **la carte SD** : aucune écriture de plus.
- **Les modules isolés ne redémarrent pas.**
  - Ils gardent leurs connexions aux appareils et aux clouds ; seul leur lien avec le courtier se coupe.
  - Ce qu'ils publient attend dans leur tampon jusqu'à l'accusé.
  - Ils se reconnectent à une porte qui n'a jamais fermé.
  - C'est ta remarque, et elle donne une troisième raison d'isoler, après « contenir » et « mesurer » :
    **isoler pour durer**.
- **Les modules du cœur redémarrent avec lui.** Ce que cela leur coûte dépend de leur façon de parler à
  l'appareil (§ 8) :
  - rien, pour un sondage ;
  - les événements de la coupure, pour un flux comme celui de Hue ;
  - une connexion de plus au cloud du fabricant, pour Landroid.
- **Un redémarrage prévu ne perd rien, par construction.**
  - Le nouveau cœur s'initialise **avant** que l'ancien s'arrête, comme Envoy, dans le mode « reprise »
    de l'étude HA v2.
  - L'ancien se fige au rang `N` et confie son état. Le nouveau le relit, compare l'empreinte et reprend
    au rang `N`.
  - Coupure visée : une seconde au plus sur la machine de production, deux sur un Pi Zero 2 W. C'est
    sous les 2 s au bout desquelles l'actif se rétrograde (V7 § 18.1.7) : la secours ne bascule pas.
- **Un plantage ne perd presque rien.**
  - Les modules isolés renvoient tout ce qui n'a pas d'accusé.
  - Le nouveau cœur reprend le dernier relais, puis rejoue la file jusqu'au dernier lot.
  - Ce qui avait été fait est refait **sous les mêmes clés** (critique § 3.11) : un module ou un service
    qui reconnaît la clé ne le refait pas.
  - Un doublon reste possible pour un ordre non idempotent d'un module du cœur, s'il n'a pas été écrit
    avant au relais. D'où la règle : **un engagement s'écrit avant, au moins au relais**.
- **Trois étages de durée de vie** :
  - la mémoire du cœur meurt avec le processus ;
  - le relais et la file survivent au redémarrage et au plantage du cœur, pas à une coupure de courant ;
  - le disque et les pairs survivent à une coupure de courant (HA v2).

  Chaque donnée choisit son étage.
- **Le temps continue.** Sur le même allumage (`boot_id`), l'horloge monotone ne repart pas : les
  échéances restent monotones et aucune grâce n'est nécessaire. Sur un autre allumage, c'est la règle de
  la mémoire v2 : l'heure murale, puis la grâce.
- **Une autre version.**
  - Les sections du relais sont typées et versionnées. Une version lit les anciennes, jamais l'inverse,
    et un relais illisible est refusé. On repart alors des pairs, puis de l'instantané, **jamais à
    vide** (HA v2 § 11.3).
  - Si le nouveau binaire n'est pas sain en 30 s, le gardien relance l'ancien avec le même relais, comme
    le vieux maître de nginx.
- **Docker et systemd** :
  - sous systemd (le Pi, D10), tout est à chaud, mise à jour du binaire comprise ;
  - sous Docker, tout est à chaud sauf une nouvelle image. Celle-ci recrée le conteneur : la mise à jour
    est alors **à froid, mais sans perte**, par l'arrêt figé de l'étude HA v2.

## 2. Ce que fait un redémarrage aujourd'hui

| Ce qui est en jeu | Aujourd'hui | Preuve |
|---|---|---|
| Le signal | SIGTERM ou Ctrl-C : `wait_for_signal`, puis l'annulation de toutes les tâches | `moli-os/src/main.rs:488-491`, `:1640-1652` |
| Les pilotes | annulés d'un coup ; au départ suivant, chacun refait sa connexion depuis zéro | `main.rs:494-496` |
| L'historique, l'énergie | leur dernier lot est écrit | `main.rs:499-504` |
| Les dernières valeurs | `state.json` réécrit à l'arrêt, et chaque minute si une valeur a changé, avec `fsync` | `main.rs:505`, `:710-712`, `:950-955` |
| Les automatismes en cours | annulés : ils ne reprennent jamais | HA v2 § 2.4 (`moli-automation/src/engine.rs:99-113`) |
| Les demandes retenues par la garde, en attente d'un humain | **en mémoire seulement** (50 au plus, 10 min chacune) : perdues | `moli-runtime/src/hub.rs:173`, `:176`, `:245` |
| Les écrans | leur flux se coupe ; ils se reconnectent et reçoivent un instantané | `moli-api/src/sse.rs` |
| Les portes | fermées pendant tout l'arrêt ; rouvertes par `TcpListener::bind` au départ suivant | `main.rs:448` |
| La mise à jour | reconstruire l'image, puis `docker compose up -d --force-recreate` ; Docker attend 10 s (`stop_grace_period`) avant de tuer | exploitation.md |
| Le processus principal du conteneur | le binaire lui-même : `ENTRYPOINT ["/moli-os"]`, `CMD ["serve"]` | `Dockerfile` |
| Le démarrage | budget : 2 s jusqu'à « sain » | PERFORMANCE.md |

Aujourd'hui, un redémarrage perd :
- les exécutions et leurs attentes ;
- les demandes retenues par la garde ;
- les courbes du monitoring, quand elles existeront ;
- ce que les appareils ont dit pendant la coupure.

Les portes se ferment, et les appareils passent par « hors ligne ». L'étude HA v2 corrige déjà la mémoire
par l'arrêt figé (§ 16). Ce document y ajoute ce qu'un disque ne garde pas.

## 3. Premiers principes : qu'est-ce qui doit survivre ?

Cinq choses, et pas une de plus :

1. **Le contenu de la mémoire**, pas la mémoire elle-même. Quatre raisons empêchent de reprendre le tas
   de l'ancien processus :
   - **les dispositions** : Rust ne promet pas la disposition d'une structure ordinaire (`repr(Rust)`)
     d'une compilation à l'autre ; un champ ajouté, un compilateur plus récent, et les octets ne veulent
     plus dire la même chose ;
   - **les adresses** changent à chaque lancement (ASLR) ; chaque `Vec`, `String`, `HashMap` ou `Arc`
     pointe dans l'ancien processus ;
   - **les octets de remplissage** : même une structure `#[repr(C)]` a des trous dont le contenu n'est
     pas défini ; les copier tels quels n'a pas de sens ;
   - **le code** : Rust ne charge pas du code neuf dans un processus qui tourne sans une ABI stable
     (`abi_stable`) ou sans WASM, que D2 a écarté. Erlang le fait (§ 4) ; Rust remplace le processus.

   Donc : le contenu, **encodé champ par champ**, comme l'instantané de l'étude HA v2.
2. **Les portes.** Une socket d'écoute qui reste ouverte garde les connexions entrantes dans la file du
   noyau. Le client attend au lieu d'être refusé.
3. **Les processus qui n'ont pas à mourir** : les modules isolés, avec leurs connexions.
4. **Le temps** : l'horloge monotone de la machine continue tant que la machine ne redémarre pas.
5. **La parole donnée** :
   - l'époque et le bail ;
   - les échéances des ordres ;
   - les clés des engagements ;
   - les issues attendues.

Ce qui ne peut pas survivre, et ne doit pas faire semblant :

- les connexions TCP que tient le cœur : les flux des écrans, les liens MQTT des modules ;
- les requêtes HTTP en vol des modules du cœur vers un cloud ;
- les sessions TLS.

On les ferme proprement, on les rouvre. Rien d'important ne repose sur elles seules : un écran reçoit un
instantané, un module renvoie ce qui n'a pas d'accusé.

## 4. Ce qu'on fait ailleurs

**Analogies**, vérifiées dans la documentation de chacun (annexe A) :

| Système | Ce qui survit | Comment | Ce qu'on en garde |
|---|---|---|---|
| **nginx** | les sockets d'écoute | `USR2` : l'ancien maître lance le nouveau binaire ; `WINCH` puis `QUIT` à l'ancien ; pour revenir en arrière, `HUP` à l'ancien maître, qui « ne ferme pas ses sockets d'écoute » | garder l'ancien prêt à repartir tant que le nouveau n'a pas fait ses preuves |
| **HAProxy** | les sockets d'écoute | `expose-fd listeners` sur la socket d'administration, puis `-x` au rechargement ; en maître-ouvrier, le maître passe les sockets par sa paire de sockets interne | passer les descripteurs par une socket Unix |
| **Envoy** | les sockets, les compteurs | le nouveau processus s'initialise entièrement, puis demande des copies des sockets d'écoute à l'ancien, puis lui dit de vider ; les compteurs passent par la socket Unix ; « les connexions existantes ne sont pas transférées » | **s'initialiser avant de prendre la main** ; les compteurs passent, les connexions non |
| **systemd**, le dépôt de descripteurs | des descripteurs, dont un `memfd` | `FDSTORE=1` et `FDNAME=` par `sd_notify` ; rendus au redémarrage par `LISTEN_FDS` et `LISTEN_FDNAMES` ; `FileDescriptorStoreMax=` vaut 0 par défaut (éteint) ; un redémarrage garde le dépôt, un arrêt puis un départ séparés « probablement pas » ; un `memfd` « peut servir à y sérialiser l'état du service » | **le modèle du relais** ; et ce dépôt est « généralement recommandé » plutôt que de garder l'ancien processus en vie |
| **Erlang** | le code et l'état | deux versions d'un module à la fois, `current` et `old` ; en charger une troisième purge la plus ancienne et tue les processus restés dedans ; un processus passe au nouveau code par un appel qualifié ; `code_change` transforme l'état | **versionner l'état, et le transformer à la reprise** |
| **`shellflip`** de Cloudflare (Rust), et `ecdysis` | les sockets et un petit état | un `fork`, un peu d'état passé de l'ancien au nouveau, une grâce pour s'initialiser ; « crashing during initialisation is OK » | l'ancien garde la main si le nouveau échoue |
| **Home Assistant** | les dernières valeurs | `core.restore_state`, écrit toutes les 15 min (`STATE_DUMP_INTERVAL`) et à l'arrêt | ce que fait déjà `state.json` : un redémarrage à froid |
| **CRIU** | tout le processus | figé, puis repris | écarté par l'étude HA v2 (§ 16.2) : connexions mortes, droits `root`, image liée au binaire exact |

Deux leçons en sortent :

- **Envoy et nginx** gardent les portes ouvertes et ne lâchent l'ancien qu'une fois le nouveau prêt.
- **systemd et Erlang** passent un état versionné, et non un processus.

Aucun ne transfère les connexions vivantes. Personne ne sait le faire proprement, et Sarah Home n'en a
pas besoin.

## 5. Pré-mortem : le redémarrage a mal tourné, qu'a-t-on perdu ?

**Inversion** : on suppose l'échec, et l'on cherche ce qui l'a causé. Chaque cause devient une parade.

| # | Ce qu'on a perdu | Comment | La parade |
|---|---|---|---|
| P1 | un appui sur un bouton pendant la coupure | le module était dans le cœur, et il est mort avec lui | les modules d'appuis et de capteurs importants sont isolés, ou leurs appareils redisent leur état (§ 8) ; un module isolé garde ses occurrences, dans l'ordre, jusqu'à l'accusé (§ 7.4) |
| P2 | l'issue d'un ordre | l'ancien cœur l'a donné, le module l'a exécuté, l'issue arrive au nouveau cœur qui ne connaît pas l'ordre | la table des ordres en vol (point, clé, échéance) voyage dans le relais |
| P3 | rien, mais l'ordre a été exécuté deux fois | le nouveau cœur a refait une étape déjà faite | la clé de l'engagement (critique § 3.11) : le module reconnaît une clé déjà vue et rend la même issue ; jamais d'envoi après l'échéance |
| P4 | une attente | son échéance est tombée pendant la coupure | l'horloge monotone continue (§ 7.5) : l'échéance part au redémarrage, avec le retard de la coupure, et la règle des 2 min de la mémoire v2 ne joue pas |
| P5 | tout : deux cœurs ont tourné ensemble | le nouveau a démarré avant que l'ancien ne lâche | le nouveau reste en reprise, sans pilote ni porte ni base en écriture, jusqu'à ce que l'ancien sorte ; le verrou d'instance (`flock`, HA v2 § 5) ; Turso refuse de toute façon un second écrivain (D13) |
| P6 | l'époque : la secours a pris la main pendant le redémarrage | la coupure a dépassé le bail | une coupure sous 2 s, ou annoncée par `sys/arret` avec son retour attendu (§ 7.6) ; le gardien ne renouvelle jamais le bail à la place du cœur : ce serait mentir |
| P7 | un clic sur un écran | la porte était fermée | la porte reste ouverte : la connexion attend dans la file du noyau, et le nouveau cœur la sert ; au-delà de la patience du client, le clic échoue visiblement, jamais en silence |
| P8 | la mémoire : le nouveau binaire ne sait pas lire le relais | un format d'une version future | refusé ; on repart des pairs, puis de l'instantané ; jamais à vide (HA v2 § 11.3) |
| P9 | la maison : le nouveau binaire plante au démarrage | un défaut de la version | le gardien relance le binaire précédent avec le même relais, qu'il sait lire (§ 7.9) |
| P10 | le relais | le gardien est mort, ou le conteneur a été recréé | l'instantané sur disque, que l'arrêt figé écrit avant une nouvelle version (HA v2 § 16) ; le redémarrage est alors à froid, sans perte |
| P11 | une réponse de Sarah | elle arrivait pendant la coupure | Sarah isolée garde sa réponse jusqu'à l'accusé (critique § 3.11) ; dans le cœur, l'appel est refait : c'est une lecture, sans effet, qui coûte des jetons |
| P12 | les courbes du monitoring | elles vivaient en mémoire | une section « mesures » du relais |
| P13 | une demande retenue par la garde | elle n'existe qu'en mémoire (`hub.rs:245`) | une section « garde » du relais ; l'instantané devrait la porter aussi |
| P14 | un lot d'historique | il était en vol | le dernier lot est écrit à l'arrêt (`main.rs:499-501`), en 100 ms au plus (D13) |
| P15 | des déclencheurs, dans la rafale du retour | les modules et Zigbee2MQTT renvoient tout d'un coup | le tampon d'entrée du hub (V7 § 20.5) : 0 perdu sur une rafale de 5 000 (V7 § 17.1.1) |
| P16 | des occurrences, au-delà du tampon d'un module | la coupure a duré trop longtemps | des tampons bornés (§ 7.4) ; leur débordement est **signalé**, jamais muet |
| P17 | tout ce qui était en mémoire | le gardien lui-même est mort | tout repart à froid, par Docker ou systemd, et l'on suit l'étude HA v2 ; le gardien est petit et sans logique métier, et sous systemd le dépôt de descripteurs garde ses portes et son relais |
| P18 | l'instantané | la carte est pleine | le relais ne passe pas par la carte ; une alerte (monitoring § 10) |
| P19 | des échéances | l'heure murale a sauté pendant le redémarrage (NTP) | les échéances sont monotones ; l'heure murale ne sert qu'à afficher (mémoire v2 § 6.5) |
| P20 | un ordre d'un cœur qui n'avait plus le droit | un ancien cœur, revenu, a parlé à un module | l'époque dans chaque ordre ; un module refuse un ordre d'une époque ancienne (HA v2, test 46) |

## 6. Les choix, à critères visibles

**Analyse morphologique**, trois axes.

**Où garder la mémoire pendant le redémarrage :**

| Option | Survit au redémarrage | au plantage du cœur | à une coupure de courant | Écritures sur la carte | Retenue |
|---|---|---|---|---|---|
| Rien, sauf `state.json` (aujourd'hui) | les valeurs seules | les valeurs seules | les valeurs seules | 2 880 `fsync` et 236 Mo par jour (HA v2 § 10) | non |
| L'instantané sur disque (HA v2) | oui | jusqu'au dernier instantané (15 min à 1 h) | oui | 3 ms et 233 Kio par instantané | **oui, à l'heure, à l'arrêt, avant une nouvelle version** |
| **Le relais et la file, gardés par le gardien** | oui | **oui, au dernier lot de 10 ms près** | non | 0 | **oui** |
| Le dépôt de descripteurs de systemd | oui | oui, un redémarrage demandé par systemd garde le dépôt | non | 0 | **en plus, sous systemd** : il garde aussi ceux du gardien |
| Un fichier sur un tmpfs de l'hôte, monté dans le conteneur | oui, même un conteneur recréé | oui | non | 0 | **pour une nouvelle image Docker** |
| La mémoire d'un pair : le secours (HA v2 § 11.2) | oui | oui | oui, s'il vit | 0 | **oui, quand il existe** |
| CRIU | oui | non | non | le processus entier | non |

**Qui garde les portes :**
- personne (aujourd'hui) ;
- les unités de sockets de systemd ;
- **le gardien** ;
- deux cœurs qui écoutent ensemble (`SO_REUSEPORT`).

Retenu : le gardien, partout. Sous systemd, il peut recevoir les siennes d'une unité de sockets. Deux
cœurs ensemble seraient deux écrivains : non.

**Qui lance les modules isolés :**
- le cœur : non, ils mourraient avec lui ;
- une unité systemd par module : sur le Pi seulement, et il faudrait piloter systemd depuis le cœur ;
- **le gardien**.

Retenu : le gardien, sous Docker comme sous systemd. Sous systemd, il reçoit un sous-arbre de cgroups
(`Delegate=yes`) pour donner à chaque module isolé sa limite.

**Un seul mécanisme, deux hôtes.** Le gardien fait la même chose sous Docker et sous systemd. systemd
n'y ajoute qu'une chose : garder les descripteurs du gardien lui-même.

## 7. La conception

### 7.1 Le gardien

<!-- FIGURE:gardien -->

Le gardien est le même binaire, lancé par `moli-os gardien`. L'image ne grossit pas. Ce qu'il fait :

1. **ouvrir les portes, une fois**, à son démarrage : l'API et les écrans (8790), les écouteurs du
   courtier (local 1883 ; réseau 8883, quand il est activé) ;
2. **lancer le cœur**, lui passer les portes et le dernier relais, et le relancer s'il sort ;
3. **lancer les modules isolés**, à la demande du cœur (monitoring § 11.5), avec leur jeton de lancement
   par un tube hérité, et les relancer, de 1 s à 60 s ;
4. **garder le relais et la file** que le cœur lui confie ;
5. **relayer les signaux** : SIGTERM vers l'arrêt figé du cœur et vers les modules. Dans le conteneur, en
   processus principal, il récolte aussi les processus terminés ;
6. **garder le dernier souffle** : les 64 dernières lignes de la sortie d'erreur du cœur, et le signal
   qui l'a arrêté (monitoring § 11.2) ;
7. **garder le binaire précédent** et y revenir si le nouveau n'est pas sain en 30 s (§ 7.9).

Ce qu'il ne fait jamais :
- parler MQTT ;
- tenir un secret : le relais n'en contient aucun, comme l'instantané de l'étude HA v2 ;
- décider quoi que ce soit pour la maison ;
- renouveler le bail à la place du cœur.

Bloc technique — ce que tient le gardien (esquisse) :

```rust
/// `moli-os gardien` : le même binaire. Ce qu'il tient, et rien d'autre.
struct Gardien {
    portes: Vec<(&'static str, OwnedFd)>,  // ("http", 8790), ("mqtt-local", 1883)… ouvertes une fois
    relais: Option<OwnedFd>,               // le dernier état complet confié par le cœur : un memfd scellé
    file: Option<OwnedFd>,                 // la file du pont maison, que le cœur écrit au fil de l'eau
    coeur: Option<Child>,                  // le cœur qui tourne
    precedent: PathBuf,                    // le binaire d'avant, pour revenir en arrière
    modules: Vec<(String, Child)>,         // les modules isolés : ses enfants, pas ceux du cœur
    souffle: VecDeque<String>,             // les 64 dernières lignes d'erreur du cœur
}
```

### 7.2 Le relais : la mémoire du cœur, en octets

**Le format est celui de l'étude HA v2** (§ 16.2) : un en-tête (`SHI1`, version, raison, rang, époque,
instant mural, version du programme, empreinte de l'annuaire, CRC), puis des sections. Les sections de
l'étude HA v2 sont l'annuaire, les valeurs, la santé, les échéances, les exécutions, les attentes, les
horaires, les minuteries, les agendas et les aides. Le redémarrage à chaud en ajoute :

| Section | Ce qu'elle porte | Pourquoi à chaud | Taille |
|---|---|---|---|
| Ordres en vol | point, clé, échéance, origine de chaque ordre donné et sans issue | une issue qui arrive au nouveau cœur retrouve son ordre (P2) | quelques dizaines d'octets par ordre |
| Garde | les demandes retenues, en attente d'un humain | aujourd'hui perdues à chaque redémarrage (P13) | 50 au plus (`hub.rs:176`) |
| Zone de travail | les dossiers des démarches en cours (critique § 3.8) | ce que Sarah ou un service a déjà rapporté | bornée par le plafond de la zone |
| Temps | l'origine de l'horloge monotone du cœur, et le `boot_id` | § 7.5 | 24 o |
| Accusés | pour chaque module isolé, le dernier numéro `k` rangé | § 7.4 | 8 o par module |
| Mesures | les courbes et les compteurs du monitoring | P12 ; les compteurs continuent au lieu de repartir de zéro, comme chez Envoy | ≈ 342 Kio (monitoring § 8) |

Le relais est **écrit dans un `memfd`, puis scellé** (`F_SEAL_SHRINK`, `F_SEAL_GROW`, `F_SEAL_WRITE`) :
une fois confié, personne ne peut plus le modifier, le gardien compris. **Aucun secret** n'y entre : le
nouveau cœur relit le coffre, avec la clé maître que lui donne l'environnement, comme aujourd'hui.

**Où comptent ses pages.** Les pages d'un `memfd` sont de la mémoire partagée (`shmem`) dans le cgroup,
pas de la mémoire anonyme. Elles ne prennent rien au budget de 32 Mio de PERFORMANCE.md, mais elles
comptent dans le plafond de 128 Mio du conteneur.

**Quand il s'écrit :**
- **à l'arrêt prévu**, une fois, au rang `N` du gel ;
- **en marche**, quand la moitié de la file a été écrite depuis le dernier relais, ou toutes les 15 min,
  et **seulement si quelque chose a changé**. Au rythme moyen des maisons (1,9 changement par seconde),
  cela fait un relais toutes les 45 min environ ; à la pointe de 200 par seconde, toutes les 25 s. Le
  coût est de 132 µs à ×1 et 1,2 ms à ×10 par relais (banc HA v2), et nul au repos.

Bloc technique — l'en-tête du relais (proposé ; encodé champ par champ, petit-boutiste, jamais une copie de la RAM) :

```rust
/// L'en-tête de l'instantané de l'étude HA v2 (`SHI1`), plus ce qu'un redémarrage à chaud ajoute.
struct EnTeteRelais {
    magie: [u8; 4],          // b"SHI1"
    version: u16,            // du format ; une version plus récente lit les anciennes, jamais l'inverse
    raison: u8,              // arrêt prévu, rafraîchissement, avant un engagement
    rang: u64,               // s : la mémoire est figée à ce rang
    epoque: u64,             // E
    mur_ms: u64,             // l'heure murale du gel
    mono_origine_ns: u64,    // l'origine de l'horloge monotone de ce cœur (CLOCK_MONOTONIC)
    boot_id: [u8; 16],       // /proc/sys/kernel/random/boot_id : même allumage, même horloge monotone
    programme: [u8; 32],     // l'empreinte du binaire qui l'a écrit
    empreinte: [u8; 32],     // l'empreinte de la mémoire critique, comparée à la reprise
    crc: u32,
}
// Puis les sections : (type, version, obligatoire ou non, longueur, CRC) et leurs octets.
```

Bloc technique — le cœur confie son relais au gardien, sans `unsafe` (`rustix` 1.1.5, fonctions `fs` et `net`) :

```rust
fn confier(etat: &[u8], gardien: &UnixStream) -> std::io::Result<()> {
    let fd = memfd_create("sarah-relais", MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING)?;
    let mut f = std::fs::File::from(fd);
    f.write_all(etat)?;                                  // en-tête, sections, CRC
    fcntl_add_seals(&f, SealFlags::SHRINK | SealFlags::GROW | SealFlags::WRITE)?;
    let fds = [f.as_fd()];
    let mut place = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
    let mut annexe = SendAncillaryBuffer::new(&mut place);
    annexe.push(SendAncillaryMessage::ScmRights(&fds));  // le descripteur voyage avec le message
    sendmsg(gardien, &[IoSlice::new(b"relais")], &mut annexe, SendFlags::empty())?;
    Ok(())
}
```

`memfd_create`, `fcntl_add_seals`, `sendmsg` et `recvmsg` sont des fonctions sûres de `rustix` 1.1.5
(`fs/memfd_create.rs:20`, `fs/fcntl.rs:73`, `net/send_recv/msg.rs:712`, `:789`). Le dépôt n'active
aujourd'hui que les fonctions `fs` et `std` de `rustix` (`Cargo.toml:86`) : il faut y ajouter `net`. La
règle `unsafe_code = "forbid"` tient.

### 7.3 Les portes

- **Le gardien ouvre les portes et les passe à chaque cœur** qu'il lance, en suivant le protocole de
  systemd (`LISTEN_FDS`, `LISTEN_FDNAMES`). Le cœur les lit de la même façon, qu'il soit lancé par systemd
  ou par le gardien ; la crate `listenfd` (1.0.2, `take_tcp_listener`) le fait déjà.
- **Ce que cela change dans le code** :
  - `TcpListener::bind` (`main.rs:448`) devient « prendre la porte héritée, sinon l'ouvrir », et `axum`
    la sert telle quelle ;
  - le courtier doit accepter un écouteur déjà ouvert. `rumqttd` 0.20 ouvre lui-même le sien
    (`TcpListener::bind(&self.config.listen)`, `server/broker.rs:383`) : un changement de plus dans
    `moli-courtier`.
- **Pendant la coupure, personne n'appelle `accept`.** Un client qui se connecte attend dans la file du
  noyau, à concurrence de sa taille, et le nouveau cœur le sert.
- **Les connexions longues des écrans** (le flux SSE, demain le WebSocket) sont fermées par l'ancien cœur
  avec un avis « redémarrage ». Les écrans se reconnectent et reçoivent un instantané, ce que `sse.rs`
  fait déjà pour un abonné en retard (V7 § 4.2).

### 7.4 Les modules isolés

- **Ce sont les enfants du gardien**, pas ceux du cœur (monitoring § 11.5).
- **Pendant la coupure**, ils gardent leurs connexions aux appareils et aux clouds ; seul leur lien avec
  le courtier se coupe. Le kit garde :
  - **la dernière valeur de chaque point**, la dernière gagnant, comme le tampon d'entrée du hub : un
    tampon borné par le nombre de points du module ;
  - **les occurrences** (appuis, sonnette, passages), dans l'ordre, jamais fusionnées, 1 024 au plus ;
    au-delà, une perte **signalée** ;
  - **les issues des ordres**, jusqu'à ce que le hub les accuse.
- **Chaque publication d'un module porte son numéro `k`**, propre au module et croissant.
  - Le hub garde, pour chaque module, le dernier `k` qu'il a rangé dans le relais ou dans la file ; cette
    table voyage dans le relais.
  - Le hub publie `sys/accuse/<module> {k}` : le module oublie tout ce qui est en deçà.
  - C'est la règle du pont de la critique des workflows : celui qui envoie garde jusqu'à l'accusé, celui
    qui reçoit reconnaît ce qu'il a déjà vu.
- **Au retour**, le module se reconnecte à la même porte, qui n'a jamais fermé, lit son dernier accusé,
  renvoie ce qui le dépasse, et republie `$vu`. Le hub écarte un doublon par son numéro.
- **La rafale du retour** est absorbée par le tampon d'entrée du hub (V7 § 20.5).
- **Un ordre en cours d'exécution** dans un module isolé quand le cœur s'arrête va à son terme. Son issue
  attend dans le tampon du module. Le nouveau cœur la rattache à son ordre par la table des ordres en vol.

### 7.5 Le temps

- **`CLOCK_MONOTONIC` compte depuis l'allumage de la machine**, pas depuis le lancement du processus.
  Le même allumage se reconnaît à `/proc/sys/kernel/random/boot_id`, un identifiant tiré à chaque
  allumage. On le lit dans un conteneur ordinaire : vérifié.
- **Le relais porte l'origine monotone de l'ancien cœur et son `boot_id`.** Sur le même allumage, le
  nouveau cœur garde la même origine. La mémoire v2 compte en secondes depuis le démarrage, dans un
  entier de 32 bits : ce compteur continue, au lieu de repartir de zéro. Les échéances restent monotones :
  rien n'est converti, et aucune grâce n'est nécessaire, car rien n'a cessé d'écouter les modules isolés.
- **Sur un autre allumage, ou sur une autre machine** (la secours), c'est la règle de la mémoire v2
  (§ 6.5) : l'heure murale dans l'instantané, la conversion, puis la grâce.
- `std::time::Instant` ne se sérialise pas. `rustix::time::clock_gettime(ClockId::Monotonic)` donne
  l'instant brut, sans `unsafe`.

### 7.6 Pas à pas : un redémarrage prévu

<!-- FIGURE:redemarrage -->

**La leçon d'Envoy : le nouveau s'initialise avant que l'ancien ne lâche.** Le nouveau cœur démarre dans
le mode « reprise » de l'étude HA v2 (§ 4) : pilotes arrêtés, moteur chargé sans rien faire partir, API
fermée. Il fait tout ce qui ne demande rien d'exclusif :
- lire la configuration ;
- compiler les automatismes ;
- charger le catalogue.

La coupure ne contient plus que le passage de la main.

Bloc technique — un redémarrage prévu, pas à pas (proposé) :

```text
avant la coupure : l'ancien cœur sert toujours
 0. le nouveau binaire est en place et vérifié (check-config, empreinte) ; le gardien le lit une fois,
    pour que ses pages soient en cache : sur un Pi, un binaire neuf se lit sinon sur la carte au départ
 1. le gardien lance le nouveau cœur en reprise : configuration, automatismes compilés, catalogue ;
    ni pilote, ni porte, ni base en écriture
 2. le nouveau cœur dit « prêt » ; l'ancien publie sys/arret {raison: redemarrage, retour_attendu_ms}
la coupure
 3. l'ancien cesse d'accepter : les nouvelles connexions attendent dans la file du noyau
 4. ordres en vol : ceux des modules isolés continuent, leur issue ira au nouveau cœur ;
    ceux des modules du cœur ont 1 s, sinon leur issue est « incertaine » (V7 § 20.7)
 5. les modules du cœur s'arrêtent, dans l'ordre inverse du démarrage (HA v2 § 16.1)
 6. le hub vide in/ ; la mémoire se fige au rang N ; l'état complet est construit et confié au gardien
 7. le dernier lot de l'historique est écrit ; la base, le verrou d'instance et les portes sont rendus
 8. l'ancien sort ; le gardien passe le relais et les portes au nouveau
 9. le nouveau prend le verrou et la base, lit le relais, compare l'empreinte, reprend au rang N à
    l'époque E : le bail court encore, il n'attend pas ses pairs ; il ouvre les portes
après la coupure
10. les clients en attente sont servis ; les modules isolés se reconnectent et renvoient ce qui dépasse
    leur dernier accusé ; les modules du cœur repartent et retrouvent leurs appareils
11. sys/etat : le secours continue au rang N + 1, sans état complet à recopier
12. le journal reçoit une ligne : « redémarrage à chaud, rang N, empreinte …, coupure de … ms »
```

| Étape | Durée | Nature |
|---|---|---|
| Construire l'état complet (6) | 132 µs à ×1, 1,2 ms à ×10 | mesuré (banc HA v2) |
| Relire, vérifier, reprendre (9) | 272 µs à ×1, 2,5 ms à ×10 | mesuré (banc HA v2) |
| Écrire le relais dans un `memfd` (6) | une copie en mémoire de 233 Kio à 2,3 Mio | estimé : sous la milliseconde |
| Le dernier lot de l'historique (7) | 100 ms au plus | la borne de D13 |
| Les ordres en vol d'un module du cœur (4) | 1 s au plus | la borne de HA v2 § 16.1 |
| Rendre et reprendre la base, sortir, passer la main (7 à 9) | à mesurer | exp. R1 |
| **La coupure, de l'étape 3 à l'étape 9** | **visée : 1 s au plus sur la machine de production, 2 s sur un Pi Zero 2 W** | à mesurer (R1) |

**La secours ne bascule pas.** L'actif renouvelle son bail toutes les secondes et se rétrograde au bout de
2 s sans réponse ; le bail dure 3 s (V7 § 18.1.7). Une coupure sous 2 s passe sans rien changer. Si elle
doit durer plus, l'annonce `sys/arret` porte son retour attendu, et la secours attend ce délai avant de
demander l'époque suivante. Si le cœur ne revient pas dans ce délai, la bascule normale reprend.

**La preuve.** L'empreinte de la mémoire critique, calculée au gel puis à la reprise, doit être la même.
Une ligne au journal la garde, avec le rang et la durée de la coupure.

### 7.7 Le plantage du cœur

Pas de gel, pas d'adieu. Ce que fait le nouveau cœur :

1. il prend **le dernier relais** que le gardien a reçu, au rang `N0` ;
2. il **rejoue la file**, de `N0 + 1` jusqu'au dernier lot écrit (§ 7.8). La file du pont maison est
   justement faite pour cela : la secours applique les sorties dans l'ordre de `s`, et sa mémoire finit
   identique à celle de l'actif, « exécutions comprises » (HA v2 § 1) ;
3. les **modules isolés** renvoient ce qui dépasse leur dernier accusé : rien de ce qu'ils ont vu n'est
   perdu ;
4. **ce qui avait été fait depuis le dernier lot** (au plus 10 ms) est refait, sous les mêmes clés. Un
   module isolé, ou un service qui garde la clé (l'identifiant d'un rendez-vous), reconnaît un
   engagement déjà fait et rend la même issue ;
5. **une alerte** part à l'astreinte, avec le dernier souffle (monitoring § 11.2).

Ce qui peut encore se perdre :
- ce qu'un module du cœur avait reçu de son appareil sans l'avoir encore remis au hub ;
- un ordre non idempotent (un appui, un message) envoyé par un module du cœur dans le dernier lot, s'il
  n'a pas été écrit avant (§ 7.8).

**Avant la file dans un `memfd`** (marche 7 du plan, qui attend le séquenceur de l'étude HA v2), le
nouveau cœur n'a que le dernier relais. Un plantage perd alors ce qui a changé depuis ce relais, soit au
plus 15 min. C'est déjà la période que l'étude HA v2 recommande pour l'instantané sans secours, mais
sans écriture sur la carte. Les modules isolés, eux, ne perdent rien dès la marche 4.

### 7.8 Trois étages de durée, et ce qui doit s'écrire avant

<!-- FIGURE:durees -->

| Étage | Survit à | Ne survit pas à | Coût d'une écriture | Ce qu'on y met |
|---|---|---|---|---|
| **La mémoire du cœur** | rien | à son arrêt | aucun | tout, d'abord |
| **Le relais et la file**, gardés par le gardien | un redémarrage, un plantage du cœur | une coupure de courant, la mort du gardien, un conteneur recréé | un `pwrite` par lot de 10 ms pour la file ; 132 µs à ×1 pour un relais | chaque sortie critique, par lots de 10 ms ; le relais quand la file s'est à moitié remplie |
| **Le disque et les pairs** | une coupure de courant (le disque) ; la perte de la machine (le pair) | une panne de tout, sans onduleur ni pair distant (HA v2 § 11.1) | 3 ms avec `fsync` pour l'instantané ; le délai du pont, 36 ms au 99e centile, pour le pair (HA v2) | l'instantané à l'heure, à l'arrêt et avant une nouvelle version ; la copie en direct vers les secours |

**La file dans un `memfd`.** Le pont maison garde déjà la file des 10 000 dernières sorties critiques, à
46 o chacune, soit 0,45 Mio (HA v2 § 6.5). Si cette mémoire est un `memfd` dont le gardien garde un
double, elle survit au cœur. Le cœur y écrit avec un `pwrite` par lot de 10 ms qui contient des sorties.
Il n'y a pas de projection en mémoire, qui demanderait `unsafe` : c'est un appel système par lot. Au repos,
rien. Il faut pour cela le séquenceur de l'étude HA v2, qui donne le rang `s` à chaque sortie : c'est
l'étape 4 de son plan.

**Ce qui doit s'écrire avant.** La critique des workflows demandait d'écrire sur le disque avant chaque
engagement (un message, un rendez-vous, une question). L'étude HA v2, plus récente, retire tout journal
sur le disque, et la durabilité vient des autres machines. Les deux se réconcilient ainsi :
- **un engagement s'écrit avant, au moins dans la file du gardien.** C'est un `pwrite` immédiat, de
  l'ordre de la microseconde, au lieu d'attendre la fin du lot. Il survit à un plantage du cœur ;
- **et chez le pair, quand il existe** (HA v2), pour survivre à la perte de la machine ;
- **le disque ne sert qu'à l'instantané.**

### 7.9 Une autre version

- **Les sections sont typées et versionnées** ; chacune dit si elle est obligatoire. Un lecteur saute une
  section facultative qu'il ne connaît pas, et refuse une section obligatoire inconnue. C'est la règle de
  l'étude HA v2 : une version plus récente lit les anciennes, jamais l'inverse ; un relais abîmé ou
  tronqué est refusé (CRC).
- **Un changement additif** passe dans les deux sens : une section facultative de plus, ou un champ
  ajouté à la fin d'un enregistrement qui porte sa longueur. Une version plus ancienne l'ignore, et le
  retour arrière reste possible.
- **Un changement qui casse** passe par une conversion à la reprise, comme `code_change` en Erlang. Pour
  le retour arrière, la nouvelle version écrit aussi l'ancienne forme de la section, le temps d'une
  version, ou bien elle se déclare **sans retour**. Dans ce cas, revenir en arrière est un redémarrage à
  froid, depuis l'instantané écrit avant la mise à jour (HA v2 § 16).
- **Le retour arrière est automatique.** Le gardien garde le binaire précédent. Si le nouveau cœur n'est
  pas « sain » en 30 s, ou s'il meurt au démarrage, le gardien relance le précédent, avec le relais de
  l'ancienne version, qu'il sait lire. C'est ce que fait le vieux maître de nginx, qui garde ses sockets
  et repart sur `HUP`, et ce que `shellflip` appelle « crashing during initialisation is OK ».
- **Un module isolé peut avoir une autre version que le cœur**, tant que le contrat `moli/v1/` tient : le
  numéro de version est dans les sujets. On essaie ainsi la version neuve d'un module, isolée, avant de
  mettre à jour le cœur (monitoring § 11.3).

### 7.10 Sous Docker, sous systemd

| | Sous systemd (le Pi, D10 : le binaire seul) | Sous Docker (aujourd'hui) |
|---|---|---|
| Le gardien | `moli-os gardien`, un service, avec `Delegate=yes` pour les cgroups des modules isolés | `moli-os gardien` en processus principal du conteneur (`CMD ["gardien"]` au lieu de `["serve"]`) : il relaie SIGTERM et récolte les processus terminés |
| Redémarrer le cœur sans changer de binaire (plantage, réglage, geste) | à chaud | à chaud |
| Mettre à jour le binaire | remplacer le fichier (un renommage atomique), puis « passe la main » : **à chaud** | une nouvelle image recrée le conteneur, et le gardien meurt avec lui : **à froid, sans perte**. L'arrêt figé écrit l'instantané (HA v2 § 16), sur le disque ou sur un tmpfs de l'hôte monté dans le conteneur (`/dev/shm/moli-os/`, comme la clé maître) ; les modules isolés repartent ; les portes se ferment le temps du départ |
| Le gardien lui-même peut redémarrer | oui : le dépôt de descripteurs de systemd garde ses portes et son relais (`FileDescriptorStoreMax=`) | non : sa mort recrée tout, à froid |
| Limites et comptes par module isolé | un cgroup par module, délégué | `setrlimit` et `/proc` : les cgroups sont en lecture seule dans le conteneur (vérifié, monitoring § 4.2) |
| Mettre le binaire dans le volume de données, pour mettre à jour sans recréer | — | **écarté** : l'image ne serait plus ce qui tourne |

**Recommandation.** Sur le Pi (D10), systemd et le gardien : tout est à chaud. Sous Docker, le gardien
donne le redémarrage à chaud sans nouvelle image, et la mise à jour d'image devient un redémarrage à
froid sans perte.

## 8. Ce qu'un redémarrage coûte à un module du cœur

**Isoler pour durer.** Un module isolé ne voit pas le redémarrage du cœur. Un module du cœur redémarre
avec lui, et ce que cela lui coûte dépend de sa façon de parler à l'appareil. Ce sont des exemples lus
dans le code ; le crible pilote par pilote reste à faire (exp. R13).

| Façon de parler | Exemples dans le code | Dans le cœur, un redémarrage lui coûte… | Isolé |
|---|---|---|---|
| Un sondage | Sonos, toutes les 2 s (`moli-sonos/src/lib.rs:19`) ; Roborock, 30 s au repos et 5 s en activité (`moli-roborock/src/lib.rs:33-34`) ; `moli-host`, 15 s | un sondage décalé ; rien de perdu, sauf un changement bref entre deux sondages | rien |
| Un flux d'événements | Hue : `GET /eventstream/clip/v2`, en `text/event-stream` (`moli-hue/src/bridge.rs:85-86`) | les événements de la coupure ; l'état se relit au retour, mais un appui bref sur un interrupteur Hue peut manquer | rien |
| Un écouteur enregistré chez la box | TaHoma : `/events/register`, puis `/events/{listener}/fetch` (`moli-overkiz/src/lib.rs:399`, `:405`) ; un écouteur qui expire se réenregistre (`:452`) | un nouvel écouteur : ce que la box gardait pour l'ancien est perdu. Si l'identifiant de l'écouteur passe dans l'état du module (monitoring § 11.4), la reprise relit ce que la box a gardé, tant qu'il n'a pas expiré | rien |
| Le MQTT d'un cloud | Landroid : MQTT 3.1.1 vers le courtier du fabricant (`moli-landroid/src/mqtt.rs:1-3`) | une connexion de plus au cloud à chaque redémarrage ; un fabricant peut limiter leur nombre (monitoring § 4.4) | rien |
| Un client extérieur de notre courtier | Zigbee2MQTT (`moli-z2m`) | il se reconnecte de lui-même ; ce qu'il publie pendant la coupure dépend de son client MQTT : à mesurer (R6) | — |
| Un envoi seul | Telegram (`moli-telegram`) : la Bot API, en envoi | un message en cours d'envoi peut partir deux fois ou pas du tout : « non confirmé » (critique § 3.11) | sa réponse reste au module jusqu'à l'accusé |

## 9. Ce que ça coûte

| Poste | Coût | Nature |
|---|---|---|
| Le gardien | un processus minimal : 577 Kio sous Linux musl (V3) ; il ne fait rien au repos, il attend | le processus minimal est mesuré ; le gardien est à mesurer (R7) |
| Le relais | l'état complet : 233 Kio à ×1, 2,3 Mio à ×10 (HA v2), plus les mesures (≈ 342 Kio) | pages `shmem` : hors du budget anonyme de 32 Mio, dans le plafond de 128 Mio |
| La file | 0,45 Mio, déjà comptés par le pont maison (HA v2 § 6.5) | elle change de place, pas de taille |
| Pendant un redémarrage prévu | deux cœurs, le temps que le nouveau s'initialise | à mesurer (R1) ; il ne tient ni pilote ni état vif |
| Rafraîchir le relais | 132 µs à ×1, 1,2 ms à ×10 (HA v2), quand la moitié de la file a été écrite, ou toutes les 15 min, si quelque chose a changé | 0 au repos |
| Écrire la file | un `pwrite` par lot de 10 ms qui contient des sorties | 0 au repos |
| La carte SD | rien de plus : l'instantané de l'étude HA v2 reste à l'heure, à l'arrêt et avant une nouvelle version | — |

## 10. Les tests

| # | Scénario | Réussi si |
|---|---|---|
| R1 | Redémarrage prévu sous la charge des maisons, avec rafales, sur la machine de production et sur un Pi Zero 2 W | coupure mesurée sous le budget (1 s, 2 s) ; aucune valeur, occurrence ni issue perdue ; aucun ordre rejoué ; même empreinte au gel et à la reprise |
| R2 | 100 redémarrages prévus à la suite, sous charge | aucune perte, aucun doublon, aucune fuite (la mémoire du gardien ne monte pas) |
| R3 | `kill -9` du cœur pendant une rafale | toutes les occurrences des modules isolés arrivent ; la mémoire reprise (relais, puis file) égale celle d'un jumeau qui n'a pas planté, au dernier lot près |
| R4 | Un ordre non idempotent d'un module du cœur, juste avant un `kill -9` | écrit avant : jamais refait ; sans l'écriture avant : refait (le test sait échouer) |
| R5 | Un écran qui se connecte pendant la coupure | servi par le nouveau cœur, aucune connexion refusée |
| R6 | Zigbee2MQTT publie pendant un redémarrage | compter ce qui arrive après la reconnexion, et décider s'il faut isoler davantage |
| R7 | Le gardien au repos, une journée | sa mémoire et son CPU mesurés ; zéro réveil inutile |
| R8 | Un relais d'une version future, ou abîmé | refusé ; reprise par les pairs, sinon par l'instantané ; jamais actif à vide |
| R9 | Un nouveau binaire qui plante au démarrage | le gardien relance le précédent avec le même relais ; rien de perdu |
| R10 | La secours présente : un redémarrage de 1,5 s ; un de 5 s annoncé ; un qui ne revient pas | pas de bascule ; pas de bascule ; bascule après le bail |
| R11 | Un redémarrage à travers un redémarrage de la machine | autre `boot_id` : conversion par l'heure murale et grâce (mémoire v2) ; même `boot_id` : échéances inchangées |
| R12 | Une nouvelle image Docker | mise à jour à froid, sans perte ; durée mesurée ; modules isolés relancés |
| R13 | Le crible : chaque pilote, redémarré dans le cœur | pour chacun, ce qu'il perd (§ 8), et la place recommandée |
| R14 | Un module isolé, coupé du courtier une minute pendant une rafale | sa dernière valeur par point arrive ; ses occurrences arrivent dans l'ordre ; au-delà de 1 024, une perte signalée |

## 11. Le plan

Chaque marche est utile seule, dans l'ordre :

1. **L'arrêt figé dans un relais** : l'instantané de l'étude HA v2 (étape 2 de son plan), écrit aussi
   dans un `memfd` et confié au gardien, avec ses sections « ordres en vol » et « garde ».
2. **Le gardien** : `moli-os gardien` en processus principal de l'image ; il ouvre les portes, lance et
   relance le cœur, garde le relais et le binaire précédent.
3. **Les portes héritées** : `axum` d'abord, puis le courtier dans `moli-courtier`.
4. **Les modules isolés, lancés par le gardien**, avec leurs tampons, leurs numéros `k` et les accusés
   (monitoring § 11).
5. **Le temps** : le `boot_id` et l'origine monotone dans le relais.
6. **La reprise prévue à la façon d'Envoy** : le nouveau cœur s'initialise en reprise, puis prend la main.
7. **Le plantage** : la file du pont dans un `memfd`, après le séquenceur de l'étude HA v2 ; l'écriture
   avant de chaque engagement.
8. **Les versions** : les règles des sections, le retour arrière automatique.
9. **Le Pi sous systemd** : le service du gardien, `Delegate=yes`, le dépôt de descripteurs.

## 12. Décidé ici, et à trancher

Décidé ici, parce que ta demande, le code ou une étude antérieure le tranche :

| Sujet | Décision | Pourquoi |
|---|---|---|
| Ce qui survit | le contenu de la mémoire, les portes, les modules isolés, le temps, la parole donnée | § 3 |
| La mémoire | l'état complet de l'étude HA v2, encodé champ par champ ; jamais une copie de la RAM | § 3, § 7.2 |
| Le mécanisme | un seul gardien, sous Docker et sous systemd | § 6 |
| Deux cœurs | jamais actifs ensemble : le nouveau attend en reprise | P5 |
| Le relais | un `memfd` scellé, sans secret, sans `unsafe` | § 7.2 |
| Le temps | même allumage : l'origine monotone passe ; sinon, la règle de la mémoire v2 | § 7.5 |
| Le retour arrière | automatique, après 30 s sans « sain » | § 7.9 |

À trancher :

| Question | Ma recommandation |
|---|---|
| Le gardien en processus principal de l'image Docker (`CMD ["gardien"]`) ? | oui : c'est la condition de tout le reste sous Docker |
| Sur le Pi : systemd et le gardien, `Delegate=yes`, le dépôt de descripteurs ? | oui, comme le prévoit D10 (le binaire seul) |
| Le budget de coupure | 1 s sur la machine de production, 2 s sur un Pi Zero 2 W ; à confirmer par R1 |
| La file du pont dans un `memfd`, pour ne rien perdre au plantage | oui, dès que le séquenceur de l'étude HA v2 existe |
| Écrire avant, dans la file, chaque engagement | oui : cela remplace « sur le disque » de la critique des workflows |
| Sous Docker, un tmpfs de l'hôte pour l'instantané d'une mise à jour d'image | oui : `/dev/shm/moli-os/`, déjà monté pour les secrets |
| Les modules à isoler d'abord, pour durer | Sarah, et les modules d'appuis et de flux (Hue) que R13 désignera |

## 13. Ce que cela changerait dans les autres études

Si elle est retenue ; rien n'est modifié dans ces documents aujourd'hui.

| Document, section | Changement |
|---|---|
| HA v2 § 16 (l'arrêt figé) | l'étape 5 écrit l'état complet dans le relais du gardien ; sur le disque, à l'heure, avant une nouvelle version, et au signal de l'onduleur |
| HA v2 § 11.2 (redémarrer) | une étape 0 : le relais du gardien, s'il porte le même `boot_id` et l'époque courante |
| HA v2 § 6.4 (le pont maison) | la file dans un `memfd` dont le gardien garde un double |
| HA v2 § 4 (les modes) | le mode « reprise » sert aussi au nouveau cœur d'un redémarrage prévu |
| V7 § 9.4.3 (si le courtier plante) | une panique du routeur ne redémarre plus Sarah Home à froid : le gardien relance le cœur à chaud |
| V7 § 9.4.2 (les modules à part) | les modules isolés sont lancés par le gardien |
| Mémoire v2 § 6.5 et § 11 | sur le même allumage, l'origine monotone passe ; sinon, la règle actuelle |
| Critique des workflows § 3.8 et § 3.11 | « ce qui engage s'écrit avant » : dans la file du gardien (plantage) et chez le pair (machine) |
| Monitoring § 8 | les courbes et les compteurs passent par le relais |

## Annexe A — Sources

- systemd, le dépôt de descripteurs : <https://systemd.io/FILE_DESCRIPTOR_STORE/> (« Storing file descriptors in the
  manager makes it easier to restart services without dropping connections or losing state » ; un `memfd` « may
  conveniently be used to serialize service state into » ; `FileDescriptorStoreMax=` « defaults to zero » ;
  `FileDescriptorStorePreserve=` ; `FDSTORE=1`, `FDNAME=`, `FDSTOREREMOVE=1` ; `LISTEN_FDS`, `LISTEN_FDNAMES`)
- Envoy, le redémarrage à chaud : <https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/operations/hot_restart>
  (le nouveau processus s'initialise avant de demander les sockets ; les compteurs passent par la socket Unix ;
  « does not transfer existing connections »)
- nginx, changer d'exécutable à la volée : <https://nginx.org/en/docs/control.html>
- HAProxy, les rechargements sans coupure : <https://www.haproxy.com/blog/hitless-reloads-with-haproxy-howto>, et le
  passage par la paire de sockets du maître :
  <https://git01.mediatek.com/plugins/gitiles/haproxy/+/2be557f7cb566e4fa8e3ea5a332e69698adc36c4%5E%21>
- Erlang, le remplacement du code : <https://www.erlang.org/doc/system/code_loading.html>
- `shellflip` (Cloudflare) : <https://docs.rs/crate/shellflip/latest> ; `ecdysis` : <https://docs.rs/ecdysis>
- Home Assistant, `restore_state.py` (`STATE_DUMP_INTERVAL = timedelta(minutes=15)`) :
  <https://github.com/home-assistant/core/blob/dev/homeassistant/helpers/restore_state.py>
- `listenfd` 1.0.2 : <https://docs.rs/listenfd/latest/listenfd/struct.ListenFd.html>
- `sd-notify` 0.5.0 (le protocole de notification ; le dépôt de descripteurs n'y figure pas dans la documentation
  publiée : un datagramme sur `$NOTIFY_SOCKET` avec `sendmsg` suffit) : <https://docs.rs/sd-notify/latest/sd_notify/>

## Annexe B — Preuves dans le code

| Fait | Où |
|---|---|
| Le moteur recharge ses définitions sans redémarrage | `crates/moli-automation/src/lib.rs:782-790` |
| L'arrêt : signal, annulation, tâches attendues, lots écrits, `state.json` | `crates/moli-os/src/main.rs:488-507`, `:1640-1652` |
| `rumqttd` 0.20.0 ouvre lui-même son écouteur | `src/server/broker.rs:383` |
| `state.json` toutes les minutes si une valeur a changé | `crates/moli-os/src/main.rs:710-712`, `:950-955` |
| Les portes ouvertes par `bind` au départ | `crates/moli-os/src/main.rs:448` |
| Les demandes retenues par la garde : en mémoire, 10 min, 50 au plus | `crates/moli-runtime/src/hub.rs:173`, `:176`, `:245` |
| Un pilote relancé : file vidée, appareils hors ligne | `crates/moli-runtime/src/supervisor.rs:50-51` |
| L'image : le binaire en processus principal | `Dockerfile` (`ENTRYPOINT ["/moli-os"]`, `CMD ["serve"]`) |
| `rustix` : fonctions `fs` et `std` seulement aujourd'hui | `Cargo.toml:86` |
| `unsafe` interdit dans l'espace de travail | `Cargo.toml:110` |
| Sonos interrogé toutes les 2 s | `crates/moli-sonos/src/lib.rs:19` |
| Roborock : 30 s au repos, 5 s en activité | `crates/moli-roborock/src/lib.rs:33-34` |
| Hue : le flux d'événements du pont | `crates/moli-hue/src/bridge.rs:85-86` |
| TaHoma : l'écouteur, sa lecture, son expiration | `crates/moli-overkiz/src/lib.rs:399`, `:405`, `:452` |
| Landroid : MQTT 3.1.1 vers le courtier du fabricant | `crates/moli-landroid/src/mqtt.rs:1-3` |
| `rustix` 1.1.5 : `memfd_create`, `fcntl_add_seals`, `sendmsg`, `recvmsg`, sans `unsafe` | `src/fs/memfd_create.rs:20`, `src/fs/fcntl.rs:73`, `src/net/send_recv/msg.rs:712`, `:789` |
