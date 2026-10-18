# HTTPS natif : que Moli serve lui-même son tableau de bord en HTTPS (`[server.tls]`), pour la voix depuis un téléphone

*In English, in short: through Tailscale without a subnet router, a phone reaches Moli at its
`100.x` address over plain HTTP. The dashboard works, but browsers only open the microphone on a
secure page (HTTPS or `localhost`), so voice does not. The docs rightly forbid a reverse proxy on
the same machine (everybody would look local). Proposal: Moli serves HTTPS itself on a second port
(`[server.tls]`, rustls), with a certificate from `tailscale cert`; caller classification stays
based on the peer's real address only, so nothing changes security-wise, and nothing changes for
homes that do not add the section.*

## Le besoin

Depuis l'extérieur, `installation.md` propose deux voies sûres : Tailscale en **routeur de
sous-réseau**, ou Cloudflare Tunnel + Access. Beaucoup de foyers utilisent Tailscale **sans**
routeur de sous-réseau : le téléphone joint alors Moli à son adresse Tailscale (`100.x`), en
HTTP. C'est l'extérieur, la personne se connecte par mot de passe, et le tableau de bord marche.

Mais pas la voix : un navigateur (Chrome sur Android, Safari sur iOS) n'ouvre
`getUserMedia` (le micro) que dans un **contexte sécurisé**, c'est-à-dire une page HTTPS ou
`localhost`. En `http://100.x.y.z:8790`, le bouton micro ne peut rien faire.

La réponse habituelle (mettre Caddy, Nginx ou `tailscale serve` devant) est **interdite à juste
titre** par la doc : un proxy inverse sur la même machine fait arriver toutes les requêtes de
`127.0.0.1`, donc tout le monde paraîtrait local, donc de la maison. Moli refuse d'ailleurs une
requête locale qui dit avoir été relayée. Il n'y a donc aujourd'hui aucune façon sûre d'avoir la
voix depuis un téléphone sans routeur de sous-réseau ni Cloudflare.

## La proposition

Moli sert **lui-même** le HTTPS, sur un **second port** (8791 par défaut), à côté du HTTP qui ne
change pas :

```toml
[server.tls]
cert_file = "/data/tls/moli.crt"
key_file = "/data/tls/moli.key"
port = 8791
```

- Le **même routeur** (mêmes sessions, mêmes personnes, mêmes règles) répond sur les deux ports.
- Le certificat vient de `tailscale cert` (Let's Encrypt, 90 jours, nom MagicDNS
  `machine.tailnet-example.ts.net`) ; un script (`scripts/tailscale-cert.sh`) et une minuterie
  systemd hebdomadaire le renouvellent. N'importe quel autre certificat PEM marche aussi.
- Moli **relit** le certificat quand ses fichiers changent (chaque minute, ou aussitôt sur
  `SIGHUP`) : le renouvellement ne demande aucun redémarrage.
- Section absente : rien ne change, ni écouteur, ni comportement.

## Choix d'architecture

- **rustls** (avec `ring`, déjà dans l'espace de travail, comme `tokio-rustls`) : aucune
  nouvelle crate, rien de lié à OpenSSL, l'image `scratch` reste statique. TLS 1.2 et 1.3,
  suites AEAD seulement (les valeurs sûres de rustls), ALPN `http/1.1`.
- **Un second écouteur**, pas un remplacement : le HTTP 8790 reste celui de la maison (le
  contrôle `health`, les scripts, les appareils du réseau local). Ni redirection ni HSTS, qui
  casseraient le HTTP local sans rien apporter.
- **Lecteur DER minimal** pour la validité (`notBefore`/`notAfter`) et les noms (SAN) du
  certificat, sans dépendance x509 de plus.
- **État visible** : `/api/health` et `/api/system` portent un champ `tls` (`null` sans la
  section ; sinon `open`, `names`, `expires`, `expires_in_days`, `reason`, jamais la clé) ; la
  carte Système affiche une ligne « HTTPS ».
- **Pannes franches** : certificat absent, illisible, expiré ou pas encore valable → l'écouteur
  HTTPS refuse les connexions, la raison est dans `tls.reason`, la maison reçoit une notice
  (fermeture, réouverture, et 14 jours avant l'échéance) ; le HTTP continue. Moli ne s'arrête
  jamais pour un certificat.
- Poignées de main bornées (10 s, 64 en parallèle) : un client lent ou hostile ne bloque pas
  l'écouteur.

## Sécurité

C'est le point central, et **rien ne s'assouplit** :

- **Le classement des appelants est inchangé** : il ne regarde que l'adresse **réelle** du pair
  TCP, jamais la façon d'arriver (HTTP ou HTTPS) ni un en-tête. Une adresse Tailscale (`100.x`)
  reste **l'extérieur** en HTTPS comme en HTTP : la personne se connecte avec son mot de passe,
  la garde s'applique comme avant. Une adresse du réseau de la maison reste le foyer.
- **Aucun en-tête de relais n'est cru** (`X-Forwarded-For`, `Forwarded`, `X-Real-IP`,
  `Tailscale-User-Login`) : c'est justement pour ne pas avoir de relais que Moli termine le TLS
  lui-même. Un test le vérifie avec une adresse `100.x` qui prétend venir du réseau local.
- **Garde DNS (rebinding)** : les noms du certificat (SAN) s'ajoutent à `allowed_hosts`, tels
  qu'ils sont au moment de la requête (un certificat renouvelé pour d'autres noms les remplace).
  Rien d'autre n'est ajouté.
- **Cookies** : `Secure` quand la requête est arrivée en HTTPS (l'écouteur marque ses requêtes),
  sans `Secure` en HTTP (le navigateur le refuserait) ; `HttpOnly; SameSite=Strict` et la
  liaison à la clé de l'appelant ne changent pas.
- **La clé privée** reste un fichier (`0600`, propriétaire l'utilisateur du conteneur), jamais
  dans le coffre, le journal, `/api/health` ni la sortie du script.
- Le port 8791 n'est pas plus fait pour Internet que le 8790 : on le joint par le VPN.

## Ce qui ne change pas pour les autres maisons

- Sans `[server.tls]` : aucun second écouteur, aucun changement de comportement ; seul
  `/api/health` gagne un champ `tls: null`.
- Les deux voies existantes (routeur de sous-réseau, Cloudflare Access) sont intactes.
- La règle « jamais de proxy inverse sur la même machine » reste ; cette proposition est
  justement ce qui permet de la garder.

## Prêt à proposer

L'implémentation est faite et tourne sur une installation réelle (certificat Tailscale servi sur
8791, micro du navigateur ouvert sur Android, adresse Tailscale toujours vue comme l'extérieur,
renouvellement par la minuterie systemd). Si l'approche vous convient, j'ouvre la pull request.
