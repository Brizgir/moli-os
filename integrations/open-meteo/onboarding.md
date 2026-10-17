# Onboarding : Open-Meteo

1. Demander la commune (ou la position) de la maison ; arrondir à ~1 km (`lat`, `lon` avec
   deux décimales suffisent pour la météo).
2. Aucun identifiant, aucun appareil : API web publique.
3. Déclarer le pilote ; `profile`, `host`, `port` et `vars` vont **sous**
   `[driver.options]`, jamais à côté de `kind` (sinon « unknown field `profile` ») :

   ```toml
   [[driver]]
   id = "meteo"
   kind = "profile"

   [driver.options]
   profile = "open-meteo"
   vars = { lat = "…", lon = "…" }
   ```

   Puis `moli-os check-config`, redémarrer.
4. Tester : `temperature`, `sunrise` et `sunset` ont une valeur dans le tableau de bord.
