# Onboarding : Daikin BRP069

1. Demander : quelle unité (quelle pièce), son adresse IP (appli Daikin ou box). Une instance
   par unité ; son id ne se renomme jamais.
2. Aucun identifiant, aucun appui : l'adaptateur répond en HTTP local sans authentification.
3. Tester sans rien changer : `curl http://<ip>/common/basic_info` répond `ret=OK,…,mac=…`.
4. Déclarer le pilote ; `profile`, `host`, `port` et `vars` vont **sous**
   `[driver.options]`, jamais à côté de `kind` (sinon « unknown field `profile` ») :

   ```toml
   [[driver]]
   id = "clim-salon"
   kind = "profile"

   [driver.options]
   profile = "daikin-brp069"
   host = "<ip>"
   ```

   Puis `moli-os check-config`, redémarrer. L'identité est la MAC de l'adaptateur : si la box lui
   donne une autre adresse, Moli le retrouve seul (journal : « retrouvé à une nouvelle
   adresse »), sans toucher à la config. Une réservation d'adresse dans la box reste conseillée
   pour les autres logiciels qui parlent à l'unité.
5. Écriture : seulement avec un humain présent, jamais dans une chambre. Changer la consigne
   de 1 °C, la remettre, vérifier l'entrée `command` dans `/api/journal`.
