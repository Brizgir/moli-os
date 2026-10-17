# Onboarding : sonnettes et carillons Ring

1. Prévenir la personne : interface non officielle de Ring (celle de l'appli), qui peut casser ;
   Moli apparaîtra parmi les « appareils clients autorisés » de son compte Ring ; les appuis
   sont relevés toutes les 10 s (pas de notification instantanée).
2. Déclarer `kind = "ring"` (aucune option obligatoire) ; `moli-os check-config`, redémarrer.
   Le pilote attend la connexion (« Connecte le compte Ring… »).
3. La personne connecte le compte depuis le tableau de bord (Système, carte Ring), elle-même :
   e-mail et mot de passe Ring, puis le code reçu (SMS, e-mail ou appli). **Ne jamais lui
   demander son mot de passe ni son code dans une conversation.** Si le tableau de bord demande
   le code de la maison, c'est normal : une connexion de compte est l'affaire d'une personne.
4. Tester : la sonnette et le carillon apparaissent, batterie et Wi-Fi remplis ; un appui sur la
   sonnette passe `doorbell` à vrai dans les 10 à 20 s, puis il retombe.
