# Onboarding : télé Hisense VIDAA

1. Trouver son adresse : la box, ou une recherche SSDP (un `MediaRenderer` Hisense). Lui donner
   une adresse fixe. Vérifier les ports : 36669 (MQTT TLS) ouvert, et 38400 (ou 18400) qui sert
   `http://<télé>:38400/MediaServer/rendererdevicedesc.xml` avec son `transport_protocol`.
2. Facultatif, pour l'allumer depuis la veille profonde : son adresse MAC (option `mac`), et
   sur la télé « Paramètres › Système › Alimentation » : réveil par le réseau, et une veille qui
   garde le réseau (sinon la télé oublie aussi l'appairage en veille profonde).
3. **Certificat client** (télés récentes, `transport_protocol` ≥ 3000 ; inutile sur les
   anciennes) : il vient de l'appli VIDAA et reste la propriété de Hisense, Moli ne l'embarque
   pas. Le tirer soi-même de l'APK de l'appli (un keystore PKCS#12 dans ses ressources :
   `client_mobile_android.p12` ou `El.p12` ; le mot de passe du keystore est documenté par
   pyvidaa et hisense_vidaa), le convertir en PEM :
   `openssl pkcs12 -in El.p12 -clcerts -nokeys -out cert.pem` et
   `openssl pkcs12 -in El.p12 -nocerts -nodes -out key.pem`, puis le ranger par un tube :
   `cat cert.pem | moli-os secrets set <instance> client_cert` et
   `cat key.pem | moli-os secrets set <instance> client_key`. Sans lui, le pilote essaie quand
   même : si la télé refuse toute connexion (statut « la télé refuse tous les identifiants »),
   c'est lui qui manque.
4. Déclarer l'instance (`kind = "hisense"`, `host`, éventuellement `mac`, `name`, `auth`) ;
   `moli-os check-config`, redémarrer. Le coffre des secrets doit être ouvert
   (`MOLI_MASTER_KEY`) : le pilote y écrit `pin`, `client_mac` et `session`.
5. **Appairage**, devant la télé allumée : au démarrage, Moli demande un code par lui-même.
   Sinon, tableau de bord › Système › « Appairer une télé » (ou la télécommande de sa pièce) ›
   « Afficher le code ». La télé affiche 4 chiffres : les taper dans le champ, puis
   « Appairer ». Le code est demandé au code de la maison (c'est une connexion, jamais un
   agent).
   - « ce n'est pas le code affiché » : le retaper ;
   - « la télé n'a pas répondu au code » ou le code a disparu : « Afficher le code » de nouveau ;
   - l'appairage refait surface plus tard (statut « télé à appairer ») : la télé a oublié la
     session (veille profonde) ; refaire le code, et revoir le réglage de veille du point 2.
6. Tester : une touche (`key` = `CursorUp`), le volume, une source (`source` = `HDMI 1`), une
   appli (`app` = `Netflix`) ; éteindre et rallumer (`power`).
7. La ranger dans sa pièce : sa télécommande apparaît dans la carte de la pièce (et au Salon).
