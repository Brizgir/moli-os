# Ring

- **Cloud** : Ring n'a pas d'API locale. Hôtes joints : `oauth.ring.com` (jetons) et
  `api.ring.com` (appareils, historique, images, ordres), en TLS vérifié.
- **Compte** : comme l'appli et Home Assistant, e-mail et mot de passe une fois, puis le code
  que Ring envoie (SMS, e-mail, appli). Moli ne garde ensuite que le **jeton de
  renouvellement** (que Ring fait tourner : chaque nouveau est rangé dans le coffre) ; le mot
  de passe et le code sont effacés du coffre. Un `hardware_id` aléatoire, gardé, fait voir à
  Ring un seul appareil « moli-os/ring » parmi les appareils autorisés du compte.
- **Repris de Home Assistant** (`homeassistant/components/ring`, Apache-2.0) : la forme
  (liste des appareils toutes les minutes, historique et santé par caméra, réauthentification
  quand le jeton est refusé, relecture après un ordre, un capteur « sonné » qui retombe tout
  seul, les capacités par modèle). Les points d'API et les listes de modèles viennent de
  python-ring-doorbell ; le code est réécrit, rien n'est copié.
- **Deviné, pas encore vu sur un vrai compte** : tout. Les fixtures ont la forme des réponses
  de Ring (valeurs fictives). À vérifier en premier : la connexion avec code, la rotation du
  jeton, le délai d'un coup de sonnette.
- **Reste** :
  - le **temps réel** : Ring pousse les coups de sonnette par Firebase Cloud Messaging (ce que
    fait HA). Ici l'historique est lu toutes les `history_seconds` (15 s par défaut) : un coup
    de sonnette arrive avec ce délai au plus. Le canal FCM (enregistrement GCM/FCM, connexion
    MCS en protobuf) est la suite logique ;
  - les interphones (`other`), le volume des carillons et sonnettes, la vidéo en direct
    (WebRTC) et les enregistrements ;
  - une saisie du compte et du code depuis le tableau de bord (aujourd'hui : coffre par tube
    puis redémarrage).
