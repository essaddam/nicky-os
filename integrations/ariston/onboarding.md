# Onboarding : Ariston (chauffe-eau Velis)

1. Demander le modèle du chauffe-eau et s'il est relié à l'appli **Ariston NET** (passerelle
   Wi-Fi intégrée). Pilotés : **Velis Evo 2** et **Andris 2**. Un autre modèle (Lydos,
   Nuos, chaudière) est ignoré par le pilote : le dire, ne pas le forcer.
2. Il faut une clé maître (`MOLI_MASTER_KEY_FILE`) : les secrets vivent dans le coffre.
3. L'humain range l'adresse e-mail puis le mot de passe du compte Ariston NET, chacun par un
   tube, sans les afficher : `read -rs V; printf %s "$V" | moli-os secrets set <instance> username; unset V`,
   puis de même pour `password`. Jamais dans un fichier, un test ou une conversation.
4. Déclarer l'instance, sans option (l'adresse du cloud est fixée dans le pilote) :
   ```toml
   [[driver]]
   id = "eau-chaude"
   kind = "ariston"
   ```
5. `moli-os check-config`, redémarrer. Identifiants absents ou refusés : statut `waiting`
   qui dit quoi faire ; le pilote ne réessaie pas en boucle. Le cloud limite les appels : ne
   pas faire tourner en même temps deux clients du même compte plus que nécessaire (Home
   Assistant le temps de la bascule).
6. Ranger le chauffe-eau dans sa pièce (étiquette) : la garde s'en sert.
7. Tant que Home Assistant pilote ce chauffe-eau, il reste le seul propriétaire de ses
   automatismes : Moli ne reprend une fonction qu'après validation, et l'automatisme HA
   correspondant est désactivé au même moment.
8. Tester, humain présent : noter la consigne et le mode ; changer la consigne d'un degré,
   attendre qu'elle se relise (3 min au plus, sinon une notice le dit) ; remettre la consigne
   et le mode d'avant et vérifier qu'ils se relisent. Ne pas couper le chauffe-eau ni sa
   protection anti-légionelle pendant le test.
