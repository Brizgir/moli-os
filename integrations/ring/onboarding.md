# Onboarding : sonnettes et carillons Ring

1. Prévenir la personne : interface non officielle de Ring (celle de l'appli), qui peut casser ;
   Moli apparaîtra parmi les « appareils clients autorisés » de son compte Ring ; les appuis
   sont relevés toutes les 10 s (pas de notification instantanée).
2. Déclarer `kind = "ring"` (aucune option obligatoire) ; `moli-os check-config`, redémarrer.
   Le pilote attend la connexion (« Connecte le compte Ring… »).
3. La personne fait le jeton **elle-même, sur son ordinateur** : `npx -p ring-client-api
   ring-auth-cli` (Node.js). Ring y demande l'e-mail, le mot de passe et le code reçu ; la
   commande affiche un jeton. Elle le colle dans le tableau de bord (Système, carte Ring).
   **Ne jamais lui demander son mot de passe, son code ni son jeton dans une conversation,
   ni le faire passer par un fichier, un message ou un agent** : le jeton se colle dans la
   carte, et nulle part ailleurs. Si le tableau de bord demande le code de la maison, c'est
   normal : une connexion de compte est l'affaire d'une personne.
4. Tester : la sonnette et le carillon apparaissent, batterie et Wi-Fi remplis ; un appui sur la
   sonnette passe `doorbell` à vrai dans les 10 à 20 s, puis il retombe.
5. Redémarrer Moli : `doorbell` reste faux, rien ne se déclenche ;
   un appui d'avant le démarrage n'est jamais annoncé.

## Premier essai contre un vrai compte

Le pilote a été écrit d'après l'interface de l'appli et rejoué sur des réponses fictives ; il n'a
pas encore tourné contre les serveurs Ring. Au premier essai, lancer Moli avec
`RUST_LOG=info,moli_ring=debug` et regarder le journal du pilote :

- `POST /clients_api/session` : un échec est noté (`ring session`), il ne bloque pas la suite.
- Les événements : `locations/<loc>/devices/<id>/events` d'abord, repli automatique sur
  `doorbots/<id>/history` si Ring répond 404. Si aucun appui n'est vu alors que l'appli Ring le
  montre, c'est ici qu'il faut regarder (forme de la réponse).
- La santé (`doorbots|chimes/<id>/health`, notée `ring health` si elle échoue) : sans elle,
  `wifi_signal` reste vide, le reste marche.
- `status = slow_down` : Ring a répondu 429, le pilote attend 10 min avant de reprendre.
- `status = sign_in` après avoir été connecté : le jeton a été refusé (mot de passe changé, Moli
  retiré du Centre de contrôle) ; la personne refait un jeton et le colle depuis Système.

Une réponse inattendue se capture d'abord telle quelle dans un fichier `*.real.json` (ignoré
par git, jamais commité), puis **anonymisée** (numéros de série, MAC, adresse, coordonnées,
identifiants de lieu et d'appareil, noms, e-mails et jetons remplacés par des valeurs
fictives) dans `integrations/ring/fixtures/`, avec un test qui la rejoue, et
`scripts/leak-check.sh` avant le commit.
