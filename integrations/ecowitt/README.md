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
- Pas encore lus (signalés une fois au journal) : foudre WH57, qualité de l'air, sol, fuites,
  WN38, `vpd`. Protocole Wunderground refusé (mot de passe dans l'URL).
