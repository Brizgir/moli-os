# Onboarding : station météo Ecowitt

1. Demander le modèle (console ou passerelle GW1000/GW1100/GW2000…) et si elle est déjà sur
   le Wi-Fi de la maison (application WS View Plus ou Ecowitt).
2. Déclarer l'instance, sans option :

   ```toml
   [[driver]]
   id = "meteo"
   kind = "ecowitt"
   # [driver.options]
   # port = 49199                 # défaut
   # path = "/data/report/"       # défaut : n'importe quel chemin
   # stations = ["<PASSKEY>"]     # recommandé une fois la station vue (journal)
   ```

   `moli-os check-config`, redémarrer. Statut `waiting` : « en attente d'un envoi ».
3. Faire faire par la personne, sur la station (WS View Plus › appareil › « Weather
   Services », ou la page web de la passerelle) › **Customized** : activé, protocole
   **Ecowitt**, serveur = adresse de la machine de Moli, port = `port`, chemin = `path`
   (ou `/data/report/`), intervalle 60 s. Enregistrer.
4. Au premier envoi : journal `ecowitt: station heard` avec la `passkey` ; le statut passe à
   `running`. Recopier la passkey dans `stations` (seule cette station est alors écoutée).
5. Tester : l'appareil « Station météo … » a `temperature`, `humidity`, `pressure` ; les
   capteurs en plus apparaissent comme appareils à placer dans leurs pièces.
6. Rien n'arrive : la machine de Moli doit accepter les connexions entrantes sur `port`
   (pare-feu), et la station être sur le réseau local (les adresses publiques sont refusées).
