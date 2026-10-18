# Onboarding : sonnettes et carillons Ring

1. Prévenir la personne : interface non officielle de Ring (celle de l'appli), qui peut casser ;
   Moli apparaîtra parmi les « appareils clients autorisés » de son compte Ring (une seule fois :
   les alertes instantanées n'ajoutent pas de second appareil) ; les appuis arrivent par les
   notifications de Ring, avec un relevé régulier en plus par précaution. Le réseau doit laisser
   sortir le port TCP 5228 vers `mtalk.google.com` (serveur push de Google).
2. Déclarer `kind = "ring"` (aucune option obligatoire) ; `moli-os check-config`, redémarrer.
   Le pilote attend la connexion (« Connecte le compte Ring… »).
3. La personne fait le jeton **elle-même, sur son ordinateur** : `npx -p ring-client-api
   ring-auth-cli` (Node.js). Ring y demande l'e-mail, le mot de passe et le code reçu ; la
   commande affiche un jeton. Elle le colle dans le tableau de bord (Système, carte Ring).
   **Ne jamais lui demander son mot de passe, son code ni son jeton dans une conversation,
   ni le faire passer par un fichier, un message ou un agent** : le jeton se colle dans la
   carte, et nulle part ailleurs. Si le tableau de bord demande le code de la maison, c'est
   normal : une connexion de compte est l'affaire d'une personne.
4. Tester : la sonnette et le carillon apparaissent, batterie et Wi-Fi remplis ; la carte Ring de
   Système dit « Alertes instantanées : actives » (sinon : en repli, voir plus bas). Un appui sur
   la sonnette passe `doorbell` à vrai en une ou deux secondes, puis il retombe ; le journal dit
   `ring event … by=push delay_ms=…`.
5. Redémarrer Moli : `doorbell` reste faux, rien ne se déclenche ;
   un appui d'avant le démarrage n'est jamais annoncé.
6. Les images : ouvrir l'onglet Caméras ; chaque sonnette et caméra montre sa dernière image
   avec son heure (`image_at`). Toucher une image en demande une fraîche (quelques secondes ; sur
   batterie, pas plus d'une par minute, et aucune pendant un enregistrement : l'ancienne reste,
   avec son heure).
7. Le mode de surveillance, **avec la personne présente** : sur la page « Sonnette et caméras »,
   le mode actuel est celui de l'appli Ring. Passer de Domicile à Désactivé (le code est
   demandé), vérifier dans l'appli Ring, puis remettre le mode d'origine. Changer le mode dans
   l'appli : Moli le montre en moins d'une minute. Jamais ce test sans la personne : armer ou
   désarmer la surveillance d'une maison habitée est son affaire.

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
- Les images (`ring snapshot update`, `ring snapshot time` en debug) : `PUT snapshots/update`
  puis `POST snapshots/timestamps` jusqu'à un horodatage plus récent, puis `GET
  snapshots/image/<id>` (404/410 = rien de disponible, l'image gardée reste). Si l'onglet
  Caméras reste vide, c'est ici qu'il faut regarder.
- Le mode (`ring location`, `ring mode`) : `GET/POST prd-api-us.prd.rings.solutions/api/v1/mode/
  location/<lieu>` (l'hôte de l'appli, `app.ring.com` autrefois). Sans réponse, pas d'appareil
  « Surveillance Ring » ; `readOnly` (alarme Ring) : lu, jamais changé.
- `status = slow_down` : Ring a répondu 429, le pilote attend 10 min avant de reprendre.
- Les alertes instantanées (`ring push …`) : `ring push registered` (une fois, au premier
  démarrage), `ring push connected`, puis `ring instant alerts active=true`. `ring push down` dit
  pourquoi : `GCM check-in`, `GCM registration refused`, `Firebase installation`, `FCM
  registration` (Google), `Ring did not take the push token` (le `PATCH clients_api/device`),
  `cannot reach mtalk.google.com:5228` (port bloqué). En repli, rien ne casse : le sondage
  continue. Un appui vu `by=poll` alors que les alertes sont actives : Ring n'a rien envoyé
  (alertes coupées dans l'appli pour cet appareil ?) ; trois de suite, Moli se réenregistre.
  `ring push unreadable` en debug de temps en temps est normal (Firebase envoie parfois un
  message indéchiffrable). Pour couper les alertes : `push = false` dans les options.
- `status = sign_in` après avoir été connecté : le jeton a été refusé (mot de passe changé, Moli
  retiré du Centre de contrôle) ; la personne refait un jeton et le colle depuis Système.

Une réponse inattendue se capture d'abord telle quelle dans un fichier `*.real.json` (ignoré
par git, jamais commité), puis **anonymisée** (numéros de série, MAC, adresse, coordonnées,
identifiants de lieu et d'appareil, noms, e-mails et jetons remplacés par des valeurs
fictives) dans `integrations/ring/fixtures/`, avec un test qui la rejoue, et
`scripts/leak-check.sh` avant le commit.
