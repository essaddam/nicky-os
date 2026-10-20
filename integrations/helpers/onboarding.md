# Onboarding : aides de la maison

1. Demander quelles valeurs la maison doit retenir (mode, alarme, budget…) ; à la reprise de
   HA, lister ses `input_*` utilisés par les automatisations et garder leurs noms.
2. Pour chacune : `id` (jamais renommé), `name`, `kind` (`select`, `switch`, `number`,
   `text`, `button`, `counter`), `options` ou `min` / `max` / `step`, `initial`, `room` si elle appartient à une pièce.
3. Déclarer `kind = "helpers"` et un `[[driver.options.helper]]` par aide ;
   `moli-os check-config`, redémarrer.
4. Tester : changer une valeur depuis le tableau de bord, redémarrer, elle est gardée.

Pour un compteur, `initial`, `step`, `min` et `max` sont des entiers exacts dans
±9007199254740991. Le pas signé est non nul ; `initial` reste entre les bornes.
La valeur est en lecture seule : tester `increment` et `reset` (à null) via le hub,
puis la reprise, sur cette seule aide virtuelle. Un retry garde son `request_id` ;
une répétition volontaire en reçoit un nouveau. Les valeurs et reçus vivent ensemble
dans `helper-counters.json`, inclus dans la sauvegarde. Ne pas le supprimer pour
résoudre une erreur. Voir [le contrat compteur](../../docs/components/compteurs.md).
