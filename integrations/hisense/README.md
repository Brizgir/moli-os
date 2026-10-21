# Télé Hisense VIDAA

- Pilote `kind = "hisense"` (crate `moli-hisense`) pour une télé Hisense sous VIDAA : le
  protocole de la télécommande de l'appli VIDAA, c'est-à-dire **MQTT sur TLS, servi par la
  télé elle-même** (port 36669). Rien ne passe par le cloud.
  - Sujets : `/remoteapp/tv/<service>/<client_id>/actions/<action>` en envoi (`sendkey`,
    `gettvstate`, `sourcelist`, `changesource`, `applist`, `launchapp`, `getvolume`,
    `changevolume`…), `/remoteapp/mobile/<client_id>/#` et `/remoteapp/mobile/broadcast/#` en
    réception (plus les sujets exacts : certains firmwares refusent les jokers). Charges JSON,
    nombres parfois en chaînes : lecture tolérante.
  - **Certificats** : celui de la télé (auto-signé) est épinglé au premier contact (secret `pin`).
    Les télés récentes exigent en plus le **certificat client de l'appli VIDAA** : il n'a de
    licence dans aucun dépôt, Moli ne l'embarque pas ; on le fournit en secrets `client_cert`
    et `client_key` (PEM), voir `onboarding.md`.
  - **Identifiants** : les anciens firmwares prennent `hisenseservice` / `multimqttservice` ;
    les récents calculent `client_id`, `username` et `password` d'une adresse MAC cliente
    (Moli s'en invente une, locale, secret `client_mac`), de l'heure en secondes et de
    constantes (`credentials.rs`, algorithme de l'appli relu par pyvidaa, vecteur de test
    vérifié). Le schéma suit `transport_protocol` lu dans
    `http://<télé>:38400/MediaServer/rendererdevicedesc.xml` (repli 18400) : ≥ 3290 `modern`,
    3000 à 3285 `middle`, sinon `static` ; option `auth` pour forcer (`auto`, `static`,
    `legacy`, `middle`, `modern`). Un refus (CONNACK 4 ou 5) fait essayer le schéma suivant.
  - **Appairage** : `vidaa_app_connect` (ou `gettvstate` sur l'ancien firmware) fait afficher
    un code à 4 chiffres ; `authenticationcode` `{"authNum": N}` ; réponse `{"result":1}` ou
    `{"result":100,"info":"Wrong authNum!!"}` ; puis `gettoken` donne `accesstoken` /
    `refreshtoken` et leurs durées (lues, pas supposées) sur `tokenissuance`. Les connexions
    suivantes gardent le **même `client_id` et le même `username`**, avec le jeton d'accès en
    mot de passe, puis le jeton de rafraîchissement et `gettoken` quand il arrive à terme. Tout
    est dans le secret `session` (JSON). Un refus de connexion avec une session = la télé l'a
    oubliée (veille profonde sur certains firmwares) : ré-appairage, demandé une fois par le
    pilote lui-même. Les anciens firmwares ne donnent pas de jetons : le code seul suffit.
- Points : `power`, `volume` (0 à 100), `mute`, `source` (énumération de `sourcelist`, les
  noms affichés), `app` (nom de `applist`, ou paquet Android envoyé par le Salon :
  `com.netflix.ninja` → Netflix…), `screen`, `key` (mêmes noms que les autres télés :
  `CursorUp` → `KEY_UP`, `Confirm` → `KEY_OK`, `Back` → `KEY_RETURNS` — `KEY_BACK` rembobine —,
  `Options` → `KEY_MENU`, `PlayPause` → `KEY_PLAY`, `Previous` → `KEY_BACK`, `Next` →
  `KEY_FORWARDS`, `Digit0`…`Digit9`), `pairing` (`paired`, `code`, `needed`, `waiting`) et
  `pairing_code` (sémantique `Secret`, numérique 0 à 9999 : le tableau de bord en déduit 4
  chiffres).
- Marche : `statetype` `fake_sleep_0` ou `poweroff` = éteinte (le courtier répond encore), le
  reste = allumée, `screen` = ce que la télé dit (`sourceswitch` : l'entrée, `livetv` : chaîne
  et programme, `app` : l'appli). `KEY_POWER` est une bascule, appuyée seulement si l'état
  connu diffère. Veille profonde = courtier injoignable = hors ligne ; `power = true` envoie
  alors un Wake-on-LAN sur l'option `mac`, puis `KEY_POWER` si la télé revient en veille.
- Identité = l'adresse `host` (comme Philips et Android TV) : une adresse fixe dans la box.
- Fixtures : ce que la télé répond, tel que ces projets le documentent, noms et identifiants
  fictifs (`fixtures/`), rejouées par les tests de la crate ; vecteur d'identifiants pour une
  MAC fictive (`02:00:00:00:00:01`), produit par l'algorithme vérifié de pyvidaa.
- Sources (MIT, aucun code repris) : newAM/hisensetv, sehaas/ha_hisense_tv,
  stevene1919/hisense_vidaa, warrenrees/pyvidaa.
- **À vérifier sur la vraie télé** : le certificat client est-il exigé (connexion refusée
  sans lui ?) ; le schéma d'identifiants détecté (`transport_protocol`) ; l'appairage de bout
  en bout et les durées des jetons ; les jetons survivent-ils à la veille profonde ; le
  Wake-on-LAN (souvent à activer : « Réveil par le réseau ») ; `getvolume` répond-il, et sur
  quel sujet ; les `appId` de `applist` ; `changevolume` accepte-t-il un niveau absolu.
