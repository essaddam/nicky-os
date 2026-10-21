# Codes et profils d’alarme — lot 3/9

Contrat de `moli-alarm-access`, en complément du moteur pur `moli-alarm` et de la
spécification d’alarme validée (§ 6, 12 et 20). Le pilote interne est le lot 4/9 ; les
routes REST/MCP, la CLI et le journal d’alarme sont le lot 7/9. La qualification réelle
appartient au lot 9/9 après H1 ; Alarmo conserve le contrôle opérationnel jusque-là.

## Frontières

Le service gère les profils, le coffre, les refus et leur persistance. Le moteur pur ne
reçoit qu’un `moli_alarm::Profiles` : une décision courante liée à un identifiant de
demande interne, une source, un droit et un mode. Il ne reçoit aucun code ni HMAC dans
ses événements, états, effets ou données de reprise.

Au démarrage d’une alarme configurée, le pilote obtient le service partagé par
`Household::alarm_profiles(audit)`. L’annuaire est celui des personnes déjà ouvert par
Moli. Ce constructeur ne crée ni route ni tâche de surveillance ; les opérations sont
sérialisées et les transactions admises suivent leur issue même si leur appelant part.

Les futures routes de gestion prennent l’extracteur `AlarmOwner`, avant le corps :
identité propriétaire **et** session humaine PIN valide liée à l’appelant. Le permis
fourni est revérifié après attente. Un propriétaire sans session PIN, un membre ou un
invité avec PIN et un PIN sans propriétaire identifié ne gèrent pas les profils. La
politique des routes ordinaires de personnes reste indépendante. `OwnerPermit::new`
est réservé aux adaptateurs internes de confiance ; aucun booléen ou rôle d’un corps
client ne constitue cette preuve.

## Profils

Identifiant aléatoire stable, nom (80 octets), personne facultative (64 octets),
`can_arm`, `can_disarm`, modes (`away`, `home`, `night`, `vacation`, tous par défaut),
`from` inclus et `until` exclu en millisecondes Unix. Une période inversée, un mode vide,
un texte vide ou un caractère de contrôle est refusé. Au plus 128 profils, révoqués
compris ; une capacité atteinte est explicite, aucun profil n’est évincé.

Un invité hérite de l’intersection avec `starts`/`expires`. Une fin est nécessaire pour
son profil : si le séjour ne fournit pas de fin, le propriétaire la fixe. Les droits du
compte et sa réservation éventuelle sont relus à chaque autorisation ; l’expiration, la
fermeture ou la disparition de la personne ferment aussi son code.

Révoquer est définitif pour cet identifiant. Les codes révoqués restent réservés : un
autre profil ne les récupère pas. Modifier un code invalide les reçus préparés avec son
ancien emplacement de coffre. Les autres changements de droits sont relus vivants.

Le retrait d’une personne révoque d’abord ses profils et garde le verrou du service
jusqu’à la suppression de la personne. Cela couvre aussi une alarme dormante, via ses
fichiers persistés. Une recréation portant le même identifiant de personne ne réactive
pas un profil révoqué. Un échec de cette révocation empêche la suppression ; les droits
ne continuent pas silencieusement avec des données incertaines.

## Secrets

`Code` est emprunté pour une demande, sans `Serialize`, `Clone` ou affichage du contenu.
Le format est strict : 4 à 8 **chiffres ASCII**, zéros initiaux conservés, aucun trim ni
conversion de chiffres Unicode. La copie privée d’une transaction est limitée à huit
octets et n’est pas journalisée.

Le coffre contient `alarm/code_key` (32 octets aléatoires) et `alarm/profile_codes`
(HMAC-SHA256 et références de profil). Les comparaisons passent par `ring::hmac::verify`.
Le fichier de profils ne contient qu’un emplacement opaque de coffre, aucun code ni
HMAC. La clé est créée au premier ajout autorisé ; une clé perdue avec des données
existantes n’est jamais régénérée automatiquement. Le namespace `alarm` est réservé
pour empêcher un pilote configurable d’obtenir ces secrets.

Les sorties publiques `ProfileView`, `Receipt` et les erreurs n’exposent aucun code ni
empreinte. Le reçu représente une admission, pas le verdict d’une commande physique :
la surface rend le verdict obtenu après l’exécution du moteur. Les données de test sont
fictives ; chaque personne saisira elle-même son code réel au jalon humain.

## Politique et sources

- Armer depuis `disarmed` sans code reste permis selon la source et le moteur.
- Tout code fourni est vérifié, y compris le droit d’armer ; un code faux n’est pas
  assimilé à une absence de code.
- Changer de mode exige le droit d’armer et un code. Exclure ou abréger la sortie utilise
  aussi le droit d’armer ; désarmer exige le droit de désarmer et un code **à chaque fois**,
  session PIN ou non.
- Le permis est lié à la demande interne et à la source : un identifiant client ne
  constitue jamais une autorisation. La source du tableau de bord est assignée par le
  serveur ; ouvrir une session PIN ne change pas son compteur. Chaque clavier a sa source.
- Le moteur conserve ses refus d’agent et d’automatisme ; les profils ne les contournent
  pas. Une politique retardée ne peut pas autoriser un instant antérieur à sa fin.

## Refus et fichiers

Cinq refus dans dix minutes verrouillent la source quinze minutes. Chaque récidive
double cette durée, plafonnée à vingt-quatre heures. Pendant le verrou, les tentatives
ne prolongent pas son échéance et ne produisent pas de nouvelle notification. Chaque
nouveau verrou émet une seule notification à la maison. Les autres sources restent
utilisables. Les succès n’effacent pas les refus encore dans la fenêtre.

Code inconnu, hors validité, révoqué ou sans droit : même sortie `invalid_code`. Le
motif détaillé va à `Audit`, à raccorder au journal par le lot 7/9, jamais à la réponse
publique. La notification traduit les textes en français et anglais et ne contient
aucun code.

`alarm-profiles.json` et `alarm.json` sont bornés à 512 Kio et écrits en 0600 par
fichier temporaire privé, synchronisation, renommage et synchronisation du répertoire.
Les liens symboliques sont refusés. Une donnée corrompue n’est pas remplacée par un
registre vide. Une écriture incertaine ferme le service jusqu’à sa réouverture contrôlée.
Les HMAC orphelins d’une publication interrompue ne donnent aucun droit.

Les verrous de source sont dans `alarm.json`, avec au plus 1 024 sources ; les identités
sont hachées séparément des codes. `write_runtime_state` et `runtime_state` donnent au
lot 4/9 une section de l’enveloppe partagée : ce lot doit réutiliser ce sérialiseur, pas
écraser le fichier depuis un second rédacteur. Les règles de `.bak`, d’état du moteur
et de reprise restent à raccorder dans ce lot. Les anciens verrous ne sont pas effacés
pour faire de la place ; une capacité atteinte est refusée explicitement. Un recul
d’horloge ne raccourcit pas un verrou.
