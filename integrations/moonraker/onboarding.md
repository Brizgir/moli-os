# Onboarding : Klipper / Moonraker

1. Demander : l'adresse IP de l'imprimante et le port de Moonraker (souvent 7125).
2. Aucun identifiant sur un LAN de confiance (Moonraker sans authentification).
3. Tester : `curl http://<ip>:<port>/printer/info` répond un `hostname`. Il devient
   l'identité de l'appareil : il ne doit plus changer.
4. Déclarer le pilote ; `profile`, `host`, `port` et `vars` vont **sous**
   `[driver.options]`, jamais à côté de `kind` (sinon « unknown field `profile` ») :

   ```toml
   [[driver]]
   id = "imprimante"
   kind = "profile"

   [driver.options]
   profile = "moonraker"
   host = "<ip>"
   port = 7125
   ```

   Puis `moli-os check-config`, redémarrer, vérifier `state` et les températures.
