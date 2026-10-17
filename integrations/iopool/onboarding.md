# Onboarding : iopool

1. Demander si la sonde est reliée à l'appli iopool, et combien de bassins a le compte (seul
   le premier est lu).
2. Clé personnelle : appli iopool → Réglages → clé API. L'humain la range lui-même, par un
   tube : `moli-os secrets set <instance> api_key`. Elle ne part que vers `api.iopool.com`.
3. Déclarer le pilote ; `profile`, `host`, `port` et `vars` vont **sous**
   `[driver.options]`, jamais à côté de `kind` (sinon « unknown field `profile` ») :

   ```toml
   [[driver]]
   id = "piscine"
   kind = "profile"

   [driver.options]
   profile = "iopool"
   ```

   Puis `moli-os check-config`, redémarrer.
4. Tester : `temperature`, `ph`, `orp` ont une valeur ; sans clé, statut `waiting`.
