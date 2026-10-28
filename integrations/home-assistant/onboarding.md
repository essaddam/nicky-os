# Onboarding : Home Assistant (pont MQTT)

1. Garder le broker MQTT existant de la maison et activer le chiffrement TLS s’il est distant du serveur Moli. Le pont Moli ne crée ni broker, ni identifiants de service.
2. Installer manuellement le compagnon `tools/home-assistant/custom_components/moli/` dans Home Assistant, puis configurer l’intégration MQTT Home Assistant sur le même broker.
3. Ajouter un driver Moli ; `native_instances` ne peut contenir que des IDs de drivers Overkiz activés :

   ```toml
   [[driver]]
   id = "home-assistant"
   kind = "home-assistant"

   [driver.options]
   host = "127.0.0.1"
   port = 1883
   installation = "ha-fixture-1"
   base_topic = "moli/ha-bridge/ha-fixture-1"
   tls = false
   native_instances = ["overkiz-fixture"]
   ```

   Utiliser l’adresse du broker joignable depuis le conteneur Moli. Si le broker est distant, `tls = true` est obligatoire. L’identifiant `installation` est stable et doit être identique dans le formulaire du compagnon HA ; le préfixe doit aussi correspondre exactement.

4. Ranger le compte MQTT du broker dans le coffre chiffré, sans écrire sa valeur dans `moli.toml` :

   ```sh
   printf '%s' "$MQTT_USERNAME" | moli-os secrets set home-assistant mqtt_username
   printf '%s' "$MQTT_PASSWORD" | moli-os secrets set home-assistant mqtt_password
   ```

5. Vérifier `moli-os check-config`, sauvegarder, puis redémarrer Moli. L’instance attend ces deux secrets avant d’ouvrir le transport MQTT.
6. Dans l’API Moli, consulter `GET /api/integrations/home-assistant/{instance}`. Les mutations `prepare`, `advance`, `reconcile` et `reload` requièrent une session humaine PIN et reçoivent uniquement des corps JSON de 8 Kio au plus.
7. La préparation n’arrête pas l’intégration Overkiz de Home Assistant et ne prouve pas que ses automatisations ou tâches sont arrêtées. Tant qu’un observateur de quiescence fiable n’est pas disponible et que le transfert n’a pas reçu sa preuve via MQTT, ne pas considérer la propriété du volet comme transférée.

Les IDs de l’exemple sont fictifs. Aucune qualification sur un Home Assistant ou une box réelle n’est incluse à cette étape.
