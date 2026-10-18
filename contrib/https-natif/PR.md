# HTTPS natif : Moli sert lui-même son tableau de bord en TLS (`[server.tls]`)

> Titre proposé : **HTTPS natif : Moli sert lui-même son tableau de bord en TLS (`[server.tls]`)**
> Issue liée : #<numéro de l'issue> (à remplacer après l'avoir ouverte)

## What and why

**In English.** Browsers only open the microphone on a secure page. A phone that reaches Moli
through Tailscale without a subnet router talks to it over plain HTTP at a `100.x` address, so
the dashboard works but voice does not, and the docs (rightly) forbid putting a reverse proxy on
the same machine. This PR lets Moli serve HTTPS itself on a second port (`[server.tls]`, rustls,
TLS 1.2/1.3, AEAD suites, HTTP/1.1), next to the unchanged HTTP port. The certificate (for example
from `tailscale cert`) is reloaded when its files change, its state is shown in `/api/health`,
`/api/system` and the System card, and the house is notified when HTTPS closes, reopens, or 14 days
before expiry. Security is unchanged: callers are still classified by the peer's real address only
(a Tailscale address stays "outside" over HTTPS too), no relay header is trusted, the certificate's
names join `allowed_hosts`, and cookies get `Secure` over HTTPS only. Without the section, nothing
changes. Includes `scripts/tailscale-cert.sh`, a weekly systemd timer and docs; tested on a real
installation with an Android phone.

**En français.**

### Le problème

Sans routeur de sous-réseau, un téléphone joint Moli par Tailscale à son adresse `100.x`, en HTTP.
Le tableau de bord marche (la personne se connecte par mot de passe), mais pas la voix : un
navigateur n'ouvre le micro (`getUserMedia`) que sur une page **sécurisée** (HTTPS ou
`localhost`). Et la solution habituelle, un proxy inverse sur la même machine (Caddy, Nginx,
`tailscale serve`), est interdite par la doc : tout le monde y paraîtrait local, donc de la maison.

### Ce que ça change

- **`[server.tls]`** dans `moli.toml` (`cert_file`, `key_file`, `port`, 8791 par défaut) : un
  **second écouteur** HTTPS servi par Moli lui-même (`crates/moli-api/src/tls.rs`), avec le
  **même routeur** que le HTTP (mêmes sessions, mêmes personnes, mêmes règles). Le HTTP 8790 ne
  change pas ; ni redirection ni HSTS.
- **rustls** (TLS 1.2 et 1.3, suites AEAD, ALPN `http/1.1`) sur `ring` : `rustls` et
  `tokio-rustls` étaient déjà dans l'espace de travail, **aucune nouvelle crate**, l'image reste
  statique. Poignées de main bornées (10 s, 64 en parallèle).
- **Rechargement à chaud** : le certificat et la clé sont relus quand leurs fichiers changent
  (vérification chaque minute) ou aussitôt sur `SIGHUP` (`docker kill -s HUP moli-os`).
- **Pannes franches, jamais d'arrêt** : certificat absent, illisible, expiré ou pas encore
  valable → l'écouteur HTTPS refuse les connexions, la raison est dans `tls.reason`, une notice
  prévient la maison ; le HTTP continue. Notice aussi à la réouverture et **14 jours** avant
  l'échéance.
- **État visible** : champ `tls` dans `/api/health` et `/api/system` (`null` sans la section ;
  sinon `listen`, `port`, `open`, `reason`, `names`, `expires`, `expires_in_days`, `cert_file`,
  jamais la clé) ; ligne « HTTPS » dans la carte Système.
- **Validation de la config** : le port HTTPS doit différer du port HTTP et ne pas valoir 0 ;
  `cert_file` et `key_file` sont tous deux requis ; un chemin relatif se lit depuis `data_dir`.
- **`scripts/tailscale-cert.sh`** : obtient le certificat par `tailscale cert`, l'écrit d'un coup
  (dossier temporaire puis déplacement, Moli ne lit jamais un fichier à moitié écrit), clé en
  `0600` pour l'utilisateur du conteneur, **jamais affichée**.
- **`docs/systemd/moli-tls.service` et `moli-tls.timer`** : renouvellement hebdomadaire.
- **Doc** : `installation.md` (pas à pas), `api.md` (second écouteur, garde DNS, cookies,
  classement), `exploitation.md`, `docs/README.md`, `moli.example.toml`.
- **Traductions** en `locales/fr` et `locales/en` (`serveur.tls.*`, `systeme.https.*`).

### Sécurité (inchangée)

- Le **classement des appelants** ne regarde que l'adresse réelle du pair TCP, jamais le
  transport ni un en-tête : une adresse Tailscale (`100.x`) reste **l'extérieur** en HTTPS comme
  en HTTP ; une adresse du réseau local reste le foyer.
- **Aucun en-tête de relais** n'est cru (`X-Forwarded-For`, `Forwarded`, `X-Real-IP`,
  `Tailscale-User-Login`) ; un test l'établit avec une `100.x` qui prétend venir du réseau local.
- **Garde DNS** : seuls les noms du certificat (SAN, DNS et adresses) s'ajoutent à
  `allowed_hosts`, tels qu'ils sont au moment de la requête.
- **Cookies** `moli_session` et cookie des personnes : `Secure` seulement quand la requête est
  arrivée en HTTPS ; `HttpOnly; SameSite=Strict` et la liaison à l'appelant ne changent pas.
- **Clé privée** : un fichier `0600`, jamais au coffre, au journal, dans l'API ni à l'écran.
- **Tests** : certificat forgé à l'exécution (aucune clé dans le dépôt), lecteur DER (validité,
  SAN, casse), rechargement sans redémarrage, fermeture/réouverture, notice d'échéance, cookies
  `Secure` en HTTPS seulement, écouteur testé de bout en bout (vraie poignée de main rustls,
  requête HTTP dessus, transport marqué chiffré, refus des connexions sans certificat).

## Guide détaillé : la voix depuis un téléphone, par Tailscale

Prérequis : Moli tourne par `docker compose` (réseau `host`, rien à publier pour le port 8791),
Tailscale est installé sur la machine et sur le téléphone, la personne qui utilisera le téléphone
a un compte (Système → Personnes) avec un mot de passe.

1. **Activer les certificats HTTPS dans Tailscale** (une fois par tailnet). Dans la console
   d'administration Tailscale : **DNS** → vérifier que **MagicDNS** est activé, puis **HTTPS
   Certificates** → *Enable HTTPS*. (Le nom de la machine et du tailnet apparaîtront dans le
   journal public de transparence des certificats : c'est le fonctionnement de Let's Encrypt.)

2. **Trouver le nom de la machine**, sans le point final :
   ```sh
   tailscale status --json | grep -m1 DNSName
   # "DNSName": "machine.tailnet-example.ts.net.",
   ```

3. **Donner le droit de demander un certificat.** `tailscale cert` veut root ou l'opérateur
   Tailscale. Au choix : lancer le script avec `sudo` (il donne alors les fichiers à l'utilisateur
   1000 du conteneur, ou à `MOLI_UID`), ou une fois pour toutes :
   ```sh
   sudo tailscale set --operator=$USER
   ```

4. **Obtenir le certificat**, depuis le dossier du dépôt (à côté de `data/`) :
   ```sh
   sudo scripts/tailscale-cert.sh machine.tailnet-example.ts.net ./data/tls
   ```
   Résultat : `data/tls/moli.crt` (`0644`) et `data/tls/moli.key` (`0600`, utilisateur du
   conteneur) ; le script affiche l'échéance, jamais la clé. Le certificat vaut 90 jours.

5. **Ajouter le bloc `[server.tls]`** à `moli.toml` (chemins vus du conteneur) :
   ```toml
   [server.tls]
   cert_file = "/data/tls/moli.crt"
   key_file = "/data/tls/moli.key"
   port = 8791
   ```
   Rien à ajouter à `allowed_hosts` : les noms du certificat en font partie. Puis, cette fois
   seulement :
   ```sh
   docker compose run --rm moli-os check-config
   docker compose restart moli-os
   ```
   Si la machine a un pare-feu, ouvrir le port 8791 sur l'interface Tailscale (`tailscale0`)
   seulement, jamais sur Internet.

6. **Installer la minuterie de renouvellement** :
   ```sh
   sudo cp docs/systemd/moli-tls.service docs/systemd/moli-tls.timer /etc/systemd/system/
   sudoedit /etc/systemd/system/moli-tls.service   # le nom MagicDNS et le chemin du dépôt
   sudo systemctl daemon-reload
   sudo systemctl enable --now moli-tls.timer
   sudo systemctl start moli-tls.service           # un premier essai, tout de suite
   systemctl status moli-tls.service               # « Deactivated successfully »
   systemctl list-timers moli-tls.timer            # la prochaine échéance
   ```
   Chaque semaine, `tailscale cert` rend le certificat en cours ou en demande un nouveau à
   l'approche de l'échéance ; Moli relit les fichiers dans la minute, sans redémarrage.

7. **Ouvrir l'adresse sur le téléphone** (Tailscale connecté) :
   `https://machine.tailnet-example.ts.net:8791`. Se connecter avec son mot de passe de personne :
   l'adresse Tailscale est l'extérieur, comme avant. Le navigateur demande l'accès au micro ; la
   voix marche.

8. **Vérifier** sur la machine :
   ```sh
   curl -s http://127.0.0.1:8790/api/health | jq .tls
   ```
   `open: true`, les `names` du certificat, `expires` et `expires_in_days`. Ou la carte Système,
   ligne « HTTPS ». Fermé : `reason` dit pourquoi (absent, illisible, expiré, pas encore valable).

Sans Tailscale, n'importe quel certificat PEM (chaîne complète) et sa clé conviennent ; seuls
les étapes 1 à 4 et 6 changent.

## Vérifié sur une installation réelle

Installation domestique en service (Moli en conteneur, réseau `host`), octobre 2026 :

- certificat Tailscale (Let's Encrypt) obtenu par `scripts/tailscale-cert.sh` et servi sur le
  port 8791 ; `/api/health` donne `open: true`, les noms et l'échéance ;
- un téléphone Android, par Tailscale, ouvre `https://<machine>.<tailnet>.ts.net:8791`, la
  connexion par mot de passe marche, **le micro du navigateur s'ouvre** ;
- l'adresse Tailscale du téléphone est toujours classée **extérieur** (pas foyer) en HTTPS ;
- le service de renouvellement lancé par la minuterie systemd s'est exécuté avec succès ;
- le HTTP 8790 sur le réseau local est inchangé.

## Limites connues

- Pas d'HTTP/2 sur l'écouteur HTTPS (HTTP/1.1 seulement), comme sur le HTTP.
- Un seul certificat (pas de SNI multiple) ; il faut un certificat dont les noms couvrent
  l'adresse ouverte sur le téléphone.
- Le renouvellement n'est pas fait par Moli lui-même : il dépend du script et de la minuterie
  (ou d'un autre outil qui réécrit les fichiers). La notice 14 jours avant l'échéance rattrape un
  oubli.
- Pas de redirection HTTP → HTTPS ni de HSTS, volontairement : le HTTP reste celui de la maison.
- Le nom MagicDNS de la machine devient public dans les journaux de transparence des
  certificats (propre à Let's Encrypt, rappelé dans le guide).
- Le premier passage à `[server.tls]` demande un redémarrage ; les renouvellements, non.

## Checked

- [x] `sh scripts/check.sh` (fmt, clippy, tests, catalogue, leak check)
- [x] `npm run build` in `web/` and `node scripts/i18n-check.mjs`
- [x] New texts in `locales/fr` **and** `locales/en`
- [x] No personal data (names, addresses, MACs, serials, keys): fake values only
- [x] Tried on real hardware? Which model, firmware, date (in the integration's README) — pas
  une intégration : essayé sur une installation réelle avec un téléphone Android, octobre 2026
  (voir ci-dessus)
