# Enceintes Amazon Echo (Alexa)

Moli pilote les enceintes Echo d'un compte Amazon comme ses autres enceintes : musique par
recherche (« du jazz », « la playlist Apéro », « France Inter »), lecture et pause, volume, une
phrase dite, une annonce, une commande Alexa. Les groupes multi-pièces de l'appli Alexa
apparaissent comme des enceintes : la musique y joue partout à la fois.

**À savoir avant de l'installer.** Amazon ne propose aucune interface officielle pour piloter
la musique des Echo. Moli utilise celle de l'appli Alexa, comme le font Home Assistant (Alexa
Media Player), ioBroker et Node-RED : elle peut changer ou cesser sans préavis.

## Connexion

Depuis le tableau de bord (Système, carte Alexa) : un bouton ouvre la vraie page de connexion
d'Amazon ; on s'y connecte (mot de passe, code de vérification, captcha : tout se passe chez
Amazon, Moli ne voit jamais le mot de passe) ; la page finit sur une page d'erreur « introuvable »
dont l'adresse commence par `https://www.amazon.com/ap/maplanding` : c'est normal, on la copie et
on la colle dans Moli. Moli s'inscrit alors comme un appareil du compte (« Moli » dans la liste
des appareils Amazon) et garde un jeton chiffré dans son coffre. Le code de la page expire en
quelques minutes : coller l'adresse sans attendre.

Conseils : se connecter depuis un ordinateur (sur un téléphone où l'appli Alexa est installée,
elle peut intercepter la fin de la connexion) ; une double authentification par appli est plus
sûre qu'un code par SMS.

## Configuration

```toml
[[driver]]
id = "alexa"
kind = "alexa"
[driver.options]
domain = "amazon.fr"   # le site Amazon du compte : amazon.fr, amazon.de, amazon.com…
music = ""             # le service par défaut : vide = celui du compte (AMAZON_MUSIC, SPOTIFY, DEEZER, TUNEIN…)
```

## Ce que Moli voit

| Point | Sens |
|---|---|
| `playing` ✎ | lecture en cours (vrai) ou en pause ; **nul** quand Amazon ne dit rien de l'enceinte (voir plus bas) |
| `volume` ✎ | 0 à 100 (pas pour un groupe : ses membres ont le leur) |
| `title`, `artist`, `state` | ce qui joue |
| `play` ✎ | une recherche musicale, jouée avec le service du compte |
| `say` ✎ | une phrase dite (250 caractères au plus ; un groupe : par chaque membre) |
| `announce` ✎ | une annonce (son d'annonce, puis la phrase ; à activer sur l'Echo) |
| `command` ✎ | une commande Alexa écrite, comme si on lui parlait |

## Ce qui joue : d'où Moli le sait

Toutes les 30 s, Moli demande à Amazon les sessions média en cours (celles que montre l'appli
Alexa), par la première forme de requête que le site accepte (voir les pièges). Pour une
enceinte dont aucune session ne dit rien, il interroge son lecteur à elle (`np/player`) ; si
ce lecteur répond 400 ou un état vide, il lit son ancien état média (`media/state`). Puis il
regarde les groupes : quand un groupe multi-pièces joue, Amazon répond « inactif » pour chacun
de ses membres. Moli affiche alors chaque membre en lecture, avec le titre du groupe et le
volume de ce membre quand Amazon le donne.

Une enceinte dont **aucune** source ne dit rien est **inconnue** : `playing` est nul, `state`
vaut `unknown`, le tableau de bord écrit « État inconnu ». Moli n'affirme jamais « à l'arrêt »
sur une simple absence de réponse : sur un vrai compte, un Echo Dot qui joue répond 400 ou
un état vide à son lecteur, et un tableau de bord qui le croyait arrêté faisait envoyer
« lecture » en boucle par-dessus la musique.

Après un ordre de volume accepté, le curseur prend la valeur demandée tout de suite, et les
volumes sont relus 5 s plus tard (puis toutes les 2 min comme d'habitude).

Pour comprendre une enceinte qui joue mais que Moli voit inconnue ou inactive, il faut passer
le journal au niveau debug pour ce pilote (ajouter `moli_alexa=debug` à `RUST_LOG`, par
exemple `info,moli_alexa=debug`). À chaque changement, une ligne `alexa playing` décrit la
**forme** des réponses : la forme de requête qui a répondu (`form=device`, `plain`,
`routines`), les champs présents, le nombre de sessions, les états, et pour chaque enceinte
son type, son état et la source de cet état (`sessions`, `sessions(device)`, `player`,
`media`, `group`, `error`). Chaque réponse refusée (400, 429…) donne une ligne `alexa:
refused` avec le chemin sans ses paramètres, le statut et le message d'Amazon. Une enceinte
y est désignée par son rang et son type, jamais par son nom, son numéro de série ou le titre
joué. Ces lignes peuvent donc être copiées telles quelles dans un rapport de bug.

## Pièges

- **`list-media-sessions` sans paramètre est refusé (400)** par alexa.amazon.fr sur un
  compte récent. Moli essaie donc, dans l'ordre, et retient la première qui répond :
  `GET /api/np/list-media-sessions?deviceSerialNumber=…&deviceType=…` (la forme
  d'alexa-remote2), puis sans paramètre, puis sans paramètre avec l'en-tête
  `Routines-Version`. Une forme refusée n'est plus demandée jusqu'au renouvellement de la
  session (tous les 4 jours). Le journal `info` dit `alexa: media sessions list answers
  form=…` quand l'une répond.
- **`np/player` répond 400 ou `state: null` pour un Echo Dot**, même en lecture. Moli lit
  alors `/api/media/state?deviceSerialNumber=…&deviceType=…` ; si lui aussi refuse, l'enceinte
  est inconnue (ci-dessus), jamais « à l'arrêt ».
- **Amazon refuse les ordres rapprochés** (`HTTP 429 Rate exceeded` sur `behaviors/preview`,
  trois volumes en quelques secondes). Les ordres partent un par un, au moins 1 s d'écart
  (file par compte) ; plusieurs valeurs en attente pour un même point ne gardent que la
  dernière ; un 429 est réessayé une fois après 2 s, et seule la seconde refusée remonte à
  la personne.

## Limites

- Pause et lecture ne marchent que si la musique vient d'Alexa (pas de Spotify Connect lancé
  depuis un téléphone).
- Un changement de mot de passe Amazon peut obliger à refaire la connexion.
- L'état de lecture est relu toutes les 30 s, le volume toutes les 2 min (5 s après un ordre) ;
  la liste des enceintes (une ajoutée, une **renommée** dans l'appli Alexa) toutes les 3 min :
  un nouveau nom apparaît dans Moli en 3 min au plus (tant que l'enceinte n'a pas de nom donné
  dans Moli, qui prime), sans rien perdre de son historique. Si Amazon demande une pause
  (HTTP 429), la prochaine relecture attend 15 min.
