# Onboarding : qualité de l'air (Open-Meteo)

1. Même position que la météo (`lat`, `lon`, arrondis à ~1 km).
2. Aucun identifiant : API web publique.
3. Déclarer le pilote ; `profile`, `host`, `port` et `vars` vont **sous**
   `[driver.options]`, jamais à côté de `kind` (sinon « unknown field `profile` ») :

   ```toml
   [[driver]]
   id = "air"
   kind = "profile"

   [driver.options]
   profile = "open-meteo-air"
   vars = { lat = "…", lon = "…" }
   ```

   Puis `moli-os check-config`, redémarrer.
4. Tester : `aqi` et `uv_index` ont une valeur (relus toutes les 30 min).
