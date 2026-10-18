**Titre proposé :** Pilote natif Netatmo Energy (thermostat, vannes, relais) par l'API cloud officielle

---

## Le besoin

Le chauffage Netatmo Energy (thermostat connecté, vannes de radiateur connectées, leur relais)
est très répandu, et Moli ne sait aujourd'hui ni le lire ni le régler. Le relais branché sur la
box n'expose **aucune interface locale utilisable** : la seule voie propre est l'API cloud
officielle de Netatmo (`api.netatmo.com`), documentée et ouverte à chacun sur dev.netatmo.com.

Ce qu'on attend dans Moli :

- chaque pièce chauffée comme un appareil (température, consigne, mode, demande de chauffe),
  rangée dans la bonne pièce Moli pour que **la garde des pièces protégées s'applique** ;
- le mode de la maison (planning, absent, hors-gel) ;
- la batterie et le signal de chaque module, pour être prévenu avant qu'une vanne ne s'éteigne ;
- une consigne donnée depuis Moli (tableau de bord, automatisme, agent sous la garde), et le
  retour au planning.

## La proposition

Un pilote natif `crates/moli-netatmo` (`kind = "netatmo"`) et son paquet
`integrations/netatmo/`, sur le modèle des pilotes cloud existants (Tuya cloud, Alexa).

- **Connexion** : la personne crée **sa propre app** sur dev.netatmo.com (gratuit, deux
  minutes), donne le client ID et le client secret à Moli par une carte « Chauffage Netatmo »
  dans Système (ou `moli-os secrets set`), puis autorise Moli par OAuth2 *authorization code* :
  la carte ouvre la vraie page de Netatmo, la personne colle l'adresse de retour (la page de
  retour n'a pas besoin d'exister : `http://localhost/moli-netatmo` par défaut).
- **Appareils** : le compte (statut de connexion), une pièce chauffée par pièce Netatmo, la
  maison (son mode, ses pièces pour membres), chaque module.
- **Écritures** : consigne d'une pièce (7 à 30 °C par 0,5, manuelle pour une durée bornée,
  3 h par défaut), retour de la pièce au planning, mode de la maison.

## Choix d'architecture

- **Cloud officiel, pas de rétro-ingénierie** : seule l'API publique de Netatmo, avec les
  scopes `read_thermostat write_thermostat` et rien d'autre (ni caméras, ni météo, ni le reste
  du compte). `[origin]` : aucune ligne reprise d'un autre projet.
- **Une app par maison** : pas de clé partagée par le projet, pas de serveur relais. Chacun
  garde la main sur son accès et peut le couper chez Netatmo.
- **Quotas respectés** : `homestatus` toutes les 5 min (120 s au moins), `homesdata` toutes les
  15 min, jamais deux requêtes à moins d'une seconde, silence qui double (1 → 30 min) quand
  Netatmo dit stop.
- **Un ordre part tout de suite ou jamais** : s'il n'a pas pu partir dans les 5 s de
  l'échéance du hub, il n'est jamais envoyé plus tard. L'état est relu juste après un ordre.

## Sécurité

- Client ID, client secret et refresh token vivent **dans le coffre chiffré** de Moli, jamais
  dans `moli.toml`, jamais affichés ni journalisés. Netatmo fait tourner le refresh token :
  le nouveau est rangé avant tout usage.
- `PUT /api/integrations/netatmo-app` (les clés) est **réservé aux humains** (session du
  tableau de bord, code demandé), comme la route Tuya cloud ; `GET` ne rend jamais les clés.
- Le `state` OAuth2 est aléatoire et vérifié ; un code périmé ou refusé est dit clairement.
- Les pièces Netatmo portent leur nom comme pièce suggérée : la garde retient l'ordre d'un
  agent sur une pièce protégée ; la maison a toutes ses pièces pour membres, donc un changement
  de mode demandé par un agent est retenu dès qu'une pièce protégée en fait partie.

## Ce qui ne change pas pour les autres maisons

Rien tant qu'on ne déclare pas `kind = "netatmo"` : un crate de plus, une route d'intégration
de plus, une carte dans Système qui ne s'affiche qu'avec un pilote netatmo. Aucun changement
du cœur, du hub ni de la garde.

## État

Écrit et vérifié sur une installation réelle (thermostat, trois vannes, relais) : lecture,
consigne d'une pièce neutre puis retour au planning, maison hors-gel puis planning. Une pull
request suivra, puis une seconde, empilée, pour les plannings de chauffe (issue séparée).

---

*In English: a native driver for Netatmo Energy (thermostat, radiator valves, relay) through
Netatmo's official cloud API, with each household's own dev.netatmo.com app and an OAuth2
sign-in from the System page. Rooms become devices (temperature, setpoint, mode, heating
demand) placed in Moli rooms so the guard applies; the home's mode and each module's battery
and signal are read. Keys and tokens stay in the encrypted store; nothing changes for homes
that do not declare the driver. Tried on a real installation.*
