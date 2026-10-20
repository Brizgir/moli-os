# Station météo Ecowitt (envoi local)

- Pilote natif `ecowitt` (`crates/moli-ecowitt`) : la passerelle **poste** ses mesures à Moli
  (« Customized », protocole Ecowitt : formulaire HTTP en unités impériales, toutes les 60 s).
  Pas de cloud, pas de compte, rien n'est demandé à la station.
- Un profil déclaratif ne suffit pas : un profil interroge (`GET`), il ne sait pas recevoir.
  L'API locale `get_livedata_info` des GW1200/GW2000 rend des valeurs avec leur unité dans le
  texte (`"0.6 m/s"`), illisibles par un profil.
- Repris : les noms de champs, unités et codes de piles d'aioecowitt (Home Assistant), sans
  code. Conversions en °C, hPa, km/h, mm ; point de rosée (Magnus) et luminosité estimée
  (W/m² ÷ 0,0079, comme aioecowitt) calculés.
- Écrit pour le kit **GW1210** (passerelle GW1200 + capteur 7-en-1 WS90). Fixtures : vrais
  envois d'une GW1200B et de GW2000A avec WS90 (même famille de firmware). **Jamais vu tourner
  contre une vraie station** : à vérifier le jour de l'installation (`onboarding.md`).
- Aussi lus, un appareil chacun : foudre WH57, globe noir WN38 et `vpd` (vus dans les vrais
  envois des fixtures) ; qualité de l'air WH45, sondes de sol WH51, sondes WN34, fuites
  WH55, particules WH41, feuilles WN35, CO₂ de la console (d'après les noms de champs
  d'aioecowitt, **sans capture** : à vérifier quand un de ces capteurs arrive). Protocole Wunderground refusé (mot de passe dans l'URL).
