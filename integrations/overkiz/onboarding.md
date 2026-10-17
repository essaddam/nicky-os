# Onboarding : Somfy TaHoma et Connectivity Kit

1. Demander quelle box : une **TaHoma** (route locale possible) ou un **Connectivity Kit**
   (cloud seulement : Somfy n'ouvre pas d'API locale sur ce modèle).
2. Il faut une clé maître (`MOLI_MASTER_KEY_FILE`) : les secrets vivent dans le coffre.
3. **Route locale** (TaHoma) :
   - le mode développeur s'active sur le compte Somfy de la maison (somfy.com, rubrique
     développeur) ; le jeton est créé avec ce compte. Si Home Assistant pilote déjà la box en
     local, réutiliser son jeton, sans l'afficher :
     ```sh
     docker exec homeassistant python3 -c 'import json;d=json.load(open("/config/.storage/core.config_entries"));e=[x["data"] for x in d["data"]["entries"] if x["domain"]=="overkiz" and x["data"].get("api_type")=="local"];print(e[0]["token"],end="")' \
       | docker exec -i moli-os /moli-os secrets set <instance> token
     ```
   - sinon l'humain le range par un tube : `read -rs V; printf %s "$V" | moli-os secrets set <instance> token; unset V` ;
   - demander l'adresse IP de la box (fixe de préférence) ; déclarer `kind = "overkiz"`,
     `transport = "local"`, `host = "<ip>"` (`port = 8443` par défaut). Le pilote épingle le
     certificat de la box au premier contact, avant d'envoyer le jeton.
4. **Route cloud** (Connectivity Kit) : l'humain range l'adresse e-mail puis le mot de passe
   du compte Somfy, chacun par un tube (`moli-os secrets set <instance> username`, puis
   `password`) ; déclarer `kind = "overkiz"`, `transport = "cloud"` (aucune adresse : celle du
   cloud Somfy est fixée dans le pilote).
5. `moli-os check-config`, redémarrer. Le tableau de bord dit ce qui manque s'il manque
   quelque chose (statut `waiting`).
6. Tant que Home Assistant pilote ces appareils, il reste le seul propriétaire des
   automatismes : Moli ne reprend une fonction qu'après validation.
7. Tester, humain présent, sur une cible neutre (jamais une pièce protégée) : un volet de
   pièce de vie, `open` puis `close` (ou `my`), et remettre l'état d'avant ; vérifier que
   l'ordre répond « fait » quand la box confirme. Le portail : seulement avec quelqu'un qui le
   voit.
