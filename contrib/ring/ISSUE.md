# Pilote natif Ring : sonnettes et caméras en lecture, connexion par jeton sans mot de passe

*In English: a native driver for Ring doorbells and cameras (presses, motion, battery, Wi-Fi),
read only, through the Ring app's unofficial cloud API. The person makes a token on their own
computer with Ring's client (`ring-auth-cli`) and pastes it in Moli: the Ring password and
2FA code never go through Moli. A small core addition, `Semantic::Secret`, makes such a sign-in
a person's only (dashboard with the code), never journaled, never held for approval, never
kept as state. Nothing changes for homes without Ring.*

## Le besoin

Une sonnette Ring (et ses caméras) est souvent le seul appareil « de la porte » d'une maison.
Aujourd'hui Moli ne sait rien d'elle : ni qu'on sonne, ni qu'il y a du mouvement devant la
maison, ni que sa batterie s'épuise. On voudrait qu'un appui puisse lancer un automatisme comme
n'importe quel bouton (une annonce sur les enceintes, une lumière), et voir la sonnette dans le
tableau de bord.

Un appareil Ring n'ouvre aucun port sur le réseau de la maison : tout passe par les serveurs de
Ring, et Ring ne publie aucune interface officielle. Home Assistant (python-ring-doorbell) et
Homebridge (ring-client-api) utilisent l'interface de l'appli Ring ; c'est le seul chemin.

## La proposition

Un pilote natif `crates/moli-ring` et une intégration `integrations/ring/`, en **lecture
seule** dans cette première étape :

- sonnette : `doorbell` (vrai 10 s après un appui), `motion` (vrai 60 s), `battery`,
  `wifi_signal` ; caméra : les mêmes sans `doorbell` ; carillon : `volume` (lecture) ;
- derniers événements relus toutes les 10 s (option `poll`, 5 s au moins) ;
- connexion depuis Système (une carte Ring), par un **jeton collé**.

Une suite (issue séparée) ajoute les images à la demande et le mode de surveillance du lieu.

## Choix d'architecture

- **Pilote natif plutôt que profil déclaratif** : OAuth avec rotation du jeton, sondage avec
  mémoire des événements déjà dits, repli entre deux adresses d'événements : hors de portée d'un
  profil.
- **Sondage plutôt que notifications push** : les notifications de l'appli passent par Firebase
  (FCM) ; ne pas en dépendre coûte 10 à 20 s de retard sur un appui. Les notifications sont une
  suite possible.
- **Jamais de « on sonne » périmé** : un événement est dit une fois, seulement s'il a moins de
  90 s, et jamais à la première lecture après un démarrage ou une reconnexion. Si Ring devient
  injoignable, `doorbell` et `motion` retombent à faux et les appareils passent indisponibles
  jusqu'au retour de Ring.
- **Aucune dépendance nouvelle** : HTTP et TLS par `moli-net` (racines Mozilla, vérification
  complète), hôtes fixes `oauth.ring.com` et `api.ring.com`, aucune URL construite depuis une
  réponse.
- « En ligne » par la disponibilité de l'appareil, pas par un point `online` ; 429 : pause de
  10 min ; un 401 ne renouvelle le jeton qu'une fois.

## Sécurité

- **Le mot de passe Ring ne passe jamais par Moli.** Ring n'offre pas de page de connexion pour
  une autre application (pas de code d'autorisation OAuth, à la différence d'Amazon). La
  personne fait donc le jeton **sur son ordinateur** avec le client de Ring lui-même
  (`npx -p ring-client-api ring-auth-cli`, par exemple dans un conteneur Node jetable) : Ring y
  demande e-mail, mot de passe et code de vérification. Elle colle le jeton affiché dans la
  carte Ring de Système.
- **Le jeton collé** (brut, ou la forme `{"rt","hid"}` que ring-auth-cli affiche, dont le
  `hardware_id` est repris) est vérifié **une fois** auprès de Ring, qui le remplace aussitôt ;
  seul le jeton rendu par Ring va au coffre chiffré de Moli, **rangé avant d'être utilisé**,
  puis renouvelé par le pilote. Ce qui a été collé n'est ni affiché, ni journalisé, ni gardé.
  Sans coffre, rien n'est envoyé à Ring (le jeton collé serait brûlé pour rien) ; un texte qui
  n'a pas la forme d'un jeton n'est jamais envoyé.
- **Une petite addition au cœur : `Semantic::Secret`.** Un point de cette sémantique (ici
  `sign_in`) n'est accepté que de l'origine `ui` (une personne avec son code) ; toute autre
  origine (agent MCP, API, assistant, tableau de bord sans code) reçoit 403 (`HumanOnly`). Il
  n'est **jamais retenu pour approbation** (la demande garderait la valeur), il est écrit `null`
  au journal, et jamais gardé comme état du point (ni SSE, ni historique).
- **Interface non officielle** : elle peut changer ou cesser sans préavis ; c'est dit dans la
  carte de connexion, le README et l'onboarding. Moli apparaît parmi les « appareils clients
  autorisés » du compte Ring, d'où la personne peut le retirer à tout moment.
- Captures : `*.real.json` est ignoré par git, et l'onboarding demande d'anonymiser toute réponse
  réelle avant d'en faire une fixture (les réponses Ring contiennent adresse, coordonnées,
  identifiants de lieu et d'appareil, que `leak-check.sh` ne sait pas tous reconnaître).

## Ce qui ne change pas pour les autres maisons

- Rien sans `[[driver]] kind = "ring"` dans `moli.toml` : le pilote ne démarre pas, aucune
  requête ne part vers Ring.
- Dans le cœur, `Semantic::Secret` est une variante nouvelle, qu'aucun pilote existant
  n'utilise ; `CommandError::HumanOnly` donne 403 comme `ReadOnly`. La garde, les approbations
  et le journal ne changent pas pour les autres sémantiques.
- Une carte « Ring » dans Système, qui n'apparaît que si le pilote Ring est déclaré (comme celle
  d'Alexa) ; textes en `fr` et `en`.

## Vérifié

Sur une installation réelle (octobre 2026) : une sonnette Ring sur batterie et deux Stick Up Cam
Mini sur un même lieu ; connexion par un jeton de ring-auth-cli, appareils lus, appui vu dans
Moli en une dizaine de secondes, mouvements vus. Pas encore vu : un carillon, un
compte à plusieurs lieux.
