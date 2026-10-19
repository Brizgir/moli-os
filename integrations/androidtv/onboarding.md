# Onboarding : télé Android TV / Google TV

1. Trouver son adresse : la box, ou une recherche mDNS (`_androidtvremote2._tcp`). Lui donner
   une adresse fixe. Vérifier les ports : 6466, 6467 et 8009 ouverts.
2. Facultatif, pour l'allumer depuis la veille profonde : son adresse MAC (option `mac`), et
   sur la télé « Paramètres › Système › Alimentation › Allumer via le réseau » (le nom varie).
3. Déclarer l'instance (`kind = "androidtv"`, `host`, éventuellement `mac` et `name`) ;
   `moli-os check-config`, redémarrer. Le coffre des secrets doit être ouvert
   (`MOLI_MASTER_KEY`).
4. **Appairage**, devant la télé allumée : au démarrage, Moli demande un code par lui-même
   (une fois). Sinon, tableau de bord › Système › « Appairer une télé » (ou la télécommande de
   sa pièce) › « Afficher le code ». La télé affiche 6 caractères (chiffres et A à F) : les
   taper dans le champ, puis « Appairer ». Le code est demandé au code de la maison (c'est une
   connexion, jamais un agent).
   - « ce n'est pas le code affiché » : le retaper, rien n'a été envoyé ;
   - « la télé a refusé le code » ou le code a disparu : « Afficher le code » de nouveau.
5. Tester : une touche (`key` = `CursorUp`), le volume ; ouvrir une appli (`app` = `YouTube`).
6. Le ranger dans sa pièce : sa télécommande apparaît dans la carte de la pièce (et au Salon).
