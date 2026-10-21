# Station météo Ecowitt

- La station **pousse** ses relevés : Moli écoute sur un port à lui (49199 par défaut), la
  station y envoie un formulaire HTTP (`POST`, `application/x-www-form-urlencoded`) toutes les
  16 s à 10 min. C'est le réglage « Customized » de la station, protocole **Ecowitt**.
- Un appareil par capteur : la station (extérieur, vent, pluie, soleil, éclairs, pression), la
  console à l'intérieur, et chaque capteur en plus (thermo-hygromètres WN31, sondes de sol
  WH51/WH52, sondes WN34, fuites WH55, particules WH41, feuilles WN35, hauteur WH54, air
  WH45). Chacun peut aller dans sa pièce.
- Unités de la maison : la station envoie en °F, inHg, pouces et mph ; Moli convertit en °C,
  hPa, mm et km/h. Calculés comme dans Home Assistant : points de rosée, température
  ressentie, refroidissement éolien, luminosité estimée.
- Lecture seule. Hors ligne après 3 intervalles sans nouvelles (5 min au moins).
- Origine : l'intégration `ecowitt` de Home Assistant et la bibliothèque `aioecowitt`
  (Apache-2.0) pour les noms des champs ; aucun code repris.
- **Pas encore vu sur une vraie station.** La fixture `fixtures/gw2000a.kv` est construite
  d'après le format documenté (les relevés de test d'`aioecowitt` pour une GW2000A), avec une
  clé fictive : ce n'est pas une capture. À remplacer par une capture anonymisée.
