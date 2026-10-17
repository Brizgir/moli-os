# Onboarding : couleur Tempo

1. Demander si la maison a un contrat EDF Tempo (sinon, inutile).
2. Aucun identifiant, aucune variable.
3. Déclarer le pilote ; `profile`, `host`, `port` et `vars` vont **sous**
   `[driver.options]`, jamais à côté de `kind` (sinon « unknown field `profile` ») :

   ```toml
   [[driver]]
   id = "tempo"
   kind = "profile"

   [driver.options]
   profile = "tempo"
   ```

   Puis `moli-os check-config`, redémarrer.
4. Tester : `today` vaut `bleu`, `blanc` ou `rouge` ; `tomorrow` reste `inconnu` avant ~11 h.
