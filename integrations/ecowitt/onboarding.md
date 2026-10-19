# Onboarding : station météo Ecowitt (GW1210 : passerelle GW1200 + capteur WS90)

Rien à ranger au coffre : la station envoie, Moli écoute (port 49199, réseau de la maison
seulement).

## Avant de poser le capteur

1. Questions : quel kit (GW1210 = GW1200 + WS90 ?), où ira le capteur (dégagé, vent et soleil,
   panneau solaire vers le sud), la passerelle à l'intérieur loin du chauffage (elle mesure
   l'intérieur et la pression), sur le Wi-Fi **2,4 GHz** de la maison.
2. Le WS90 : panneau solaire et supercondensateur, plus 2 piles AA de secours (non fournies,
   conseillées). Mettre les piles, puis brancher la passerelle (USB) **à moins de 2 m** du
   capteur pour le premier appairage (quelques minutes), puis les éloigner.

## Régler la station (appli Ecowitt, ou page web de la passerelle)

3. Appli **Ecowitt** (ou WS View Plus) : ajouter la passerelle GW1200, lui donner le Wi-Fi.
   Elle prend une adresse : la fixer dans la box (bail statique), elle servira pour `allow`.
4. Réglages de la passerelle → **Weather Services** → **Customized** (aussi dans la page web
   de la passerelle : `http://<adresse de la passerelle>`, menu « Weather Services ») :
   - **Enable** ;
   - **Protocol Type Same As** : **Ecowitt** (pas Wunderground : il met un mot de passe dans
     l'URL, Moli le refuse) ;
   - **Server IP / Hostname** : l'adresse de la machine de Moli sur le réseau (pas `localhost`,
     pas un nom en `.ts.net`) ;
   - **Path** : `/data/report/` (l'appli ajoute parfois elle-même le premier `/` : le chemin
     doit finir en `/data/report/`, avec ou sans le `/` final) ;
   - **Port** : `49199` ;
   - **Upload Interval** : `60` secondes (de 16 à 60 ; plus court n'apporte rien au tableau de
     bord, le WS90 émet toutes les 8,5 s mais la passerelle n'envoie qu'à cet intervalle).
   - Enregistrer. Les autres services (Ecowitt.net, Wunderground) peuvent rester coupés : rien
     ne passe par le cloud.
5. Unités : sans importance, l'envoi Ecowitt est toujours en unités impériales, Moli convertit.

## Déclarer l'instance (`moli.toml`)

6. Ajouter, puis `moli-os check-config` et redémarrer Moli :

   ```toml
   [[driver]]
   id = "meteo"            # jamais renommé : préfixe des appareils
   kind = "ecowitt"
   # [driver.options]
   # port = 49199          # celui donné à la station
   # path = "/data/report/"
   # name = "Station météo"
   ```

   Le port doit être joignable depuis la passerelle : Moli tourne en réseau `host`, rien à
   publier ; un pare-feu sur la machine doit laisser entrer le TCP 49199 depuis le réseau local.
7. Statut du pilote : « en attente de la station … port 49199, chemin /data/report/ » tant que
   rien n'est arrivé.

## Vérifier

8. Dans la minute : le statut passe à `running`, trois appareils apparaissent :
   `meteo:outdoor` (« Station météo » : `temperature`, `humidity`, `dew_point`, `wind_speed`,
   `wind_gusts`, `wind_gusts_max`, `wind_direction`, `wind_direction_avg`, `raining`,
   `rain_rate`, `rain_today`, `rain_event`, `rain_hour`, `rain_24h`, `rain_week`,
   `rain_month`, `rain_year`, `solar_radiation`, `illuminance`, `uv_index`,
   `battery_voltage`, `capacitor_voltage`) et `meteo:indoor` (« Passerelle météo » :
   `temperature`, `humidity`, `pressure`, `pressure_abs`). Ranger la passerelle dans sa pièce.
9. Le journal de Moli dit une fois : `ecowitt gateway heard: add passkey = "…"` et
   `ecowitt station heard` (modèle, firmware, champs non lus). Recopier le `PASSKEY` dans les
   options, avec l'adresse de la passerelle, puis redémarrer :

   ```toml
   [driver.options]
   passkey = "<les 32 caractères du journal>"
   allow = ["192.168.1.x"]  # l'adresse fixée à l'étape 3
   ```

   Toute autre passerelle (ou un autre appareil) est alors refusée (403, une ligne au journal
   par minute au plus).
10. Comparer avec l'appli Ecowitt : température extérieure, pression relative (hPa), vent
    (km/h), pluie du jour (mm). Une pression qui diffère : la **relative** dépend du réglage
    d'altitude/décalage fait dans l'appli Ecowitt (Calibration), Moli la reprend telle quelle.
11. Rien n'arrive : depuis une autre machine du réseau,
    `curl -sS http://<moli>:49199/data/report/` doit répondre `Method Not Allowed` (le port
    est joignable ; un GET ne change rien). Pas de réponse : pare-feu, port, ou Moli pas
    démarré. S'il répond mais que la station ne poste pas : revoir l'étape 4 (protocole
    Ecowitt, chemin, adresse, port), et que la passerelle est sur le même réseau que Moli (pas
    un Wi-Fi invité isolé). Le journal (`ecowitt post refused`) dit pourquoi un envoi est
    refusé (chemin, protocole, PASSKEY), sans jamais citer son contenu.

## À vérifier avec la vraie station (jamais vu en vrai)

- Le `model` envoyé (attendu `GW1200A`, `freq=868M`) et que le WS90 donne bien la pluie
  piézo (`*_piezo`) et `srain_piezo`.
- L'intervalle réel et la marge de 3 envois manqués (hors ligne après 3 min sans envoi).
- `ws90cap_volt` par beau temps (3,5 à 5,5 V attendus ; sous 2,5 V, nettoyer le panneau) et
  `wh90batt` (piles de secours, ~3 V neuves).
- Que l'appli accepte le chemin tel quel (avec ou sans `/` initial).
