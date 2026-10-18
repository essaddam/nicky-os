# Onboarding : Voltalis

1. Demander si la maison a un compte Voltalis (appli « myVoltalis ») et ce qu'il pilote :
   des radiateurs sur **fil pilote** (`VX_WIRE`), un **chauffe-eau** derrière un relais
   (`VX_RELAY`). Le chauffe-eau est affiché, jamais commandé (voir le README).
2. Il faut une clé maître (`MOLI_MASTER_KEY_FILE`) : les secrets vivent dans le coffre.
3. L'humain range l'adresse e-mail puis le mot de passe du compte, chacun par un tube, sans
   les afficher : `read -rs V; printf %s "$V" | moli-os secrets set <instance> username; unset V`,
   puis de même pour `password`. Jamais dans un fichier, un test ou une conversation.
4. Déclarer l'instance, sans option (l'adresse du cloud est fixée dans le pilote) :
   ```toml
   [[driver]]
   id = "chauffage"
   kind = "voltalis"
   ```
5. `moli-os check-config`, redémarrer. Identifiants absents ou refusés : statut `waiting`
   qui dit quoi faire ; le pilote ne réessaie pas en boucle.
6. Ranger chaque radiateur dans sa pièce (étiquettes) : la garde s'en sert. Les programmes
   (toute la maison à la fois) n'ont pas de pièce : un agent qui les change attend un humain.
7. Tant que Home Assistant pilote ce chauffage, il reste le seul propriétaire de ses
   automatismes : Moli ne reprend une fonction qu'après validation, et l'automatisme HA
   correspondant est désactivé au même moment.
8. Tester, humain présent, sur une cible neutre (un radiateur de pièce de vie, **jamais une
   pièce protégée**) : noter le régime, le préréglage et la consigne ; changer la consigne
   d'un demi-degré, attendre qu'elle se relise (2 min au plus, sinon une notice le dit) ;
   remettre le régime et la consigne d'avant, et vérifier qu'ils se relisent.
