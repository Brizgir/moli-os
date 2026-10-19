# Télé Android TV / Google TV

- Pilote générique (`kind = "androidtv"`, crate `moli-androidtv`) pour toute télé ou boîtier
  sous Android TV ou Google TV, sans API du fabricant : tout passe par les deux protocoles
  ouverts de Google.
  - **Android TV Remote v2** (TLS 6466, certificat client) : touches, marche/veille, volume et
    sourdine (lus ; réglés par appuis successifs, le protocole n'a pas de volume absolu),
    application au premier plan, ouverture d'une appli (son nom, son paquet Android ou un
    lien), texte dans le champ ouvert (une recherche).
  - **Appairage intégré** (« polo », TLS 6467) : Moli crée son certificat client (RSA 2048,
    `rsa` + `rcgen`), la télé affiche un code de 6 caractères hexadécimaux, la personne le tape
    dans le tableau de bord ; le secret envoyé est le SHA-256 des deux clés publiques et du
    code. Un code mal tapé est reconnu avant tout envoi (premier octet) : on peut le retaper.
  - **Google Cast** (TLS 8009, rien à appairer) : ce qui joue (`media_*`) et `media_control`.
- Points : `power`, `volume`, `mute`, `app`, `text`, `key` (mêmes noms que les autres télés :
  `CursorUp`, `Confirm`, `Back`, `Home`, `Options`, `PlayPause`…), `media_app`, `media_state`,
  `media_title`, `media_subtitle`, `media_image`, `media_duration`, `media_position`,
  `media_control`, `pairing` (`paired`, `code`, `needed`, `waiting` ; écrire `code` demande un
  nouveau code) et `pairing_code` (sémantique `Secret` : seulement depuis le tableau de bord,
  jamais journalisé ni gardé).
- Marche : la touche POWER est une bascule, appuyée seulement si l'état connu diffère. Télé en
  veille profonde (télécommande injoignable) : Wake-on-LAN sur l'option `mac`.
- Secrets écrits par le pilote : `atv_cert`, `atv_key` (l'appairage), `atv_pin`, `cast_pin`
  (les certificats de la télé, épinglés au premier contact). Un appairage de Home Assistant
  (`.storage/androidtv_remote_cert.pem` / `_key.pem`) peut être repris sous ces noms.
- Le même appairage sert au pilote `philips` (module `pairer.rs`).
- Identité = l'adresse `host` (comme Philips) : une adresse fixe dans la box.
- Fixture : `fixtures/pairing.txt`, l'échange d'appairage encodé à la main d'après
  `polo.proto` ; les tests le rejouent octet par octet, et appairent une fausse télé en TLS.
- **À vérifier sur une vraie télé** (TCL Google TV) : l'appairage de bout en bout ; que
  `power` suit la veille (message `RemoteStart`) ; le pas du volume (appuis successifs) ;
  que le réveil par Wake-on-LAN fonctionne (souvent désactivé : « Allumer via le réseau »).
