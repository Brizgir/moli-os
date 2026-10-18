# Chauffage Netatmo Energy (thermostat, vannes)

Moli lit et règle le chauffage Netatmo : le thermostat connecté, les vannes de radiateur
connectées et leur relais. Chaque pièce chauffée de l'appli Netatmo devient un appareil de
Moli (sa température, sa consigne, sa demande de chauffe), la maison a son mode de chauffage
(planning, absent, hors-gel), et chaque module dit sa batterie et son signal radio.

**Par le cloud de Netatmo.** Le relais branché sur la box ne propose aucune interface locale
utilisable : Moli passe par l'**API officielle** de Netatmo (dev.netatmo.com), avec une app
que vous créez vous-même. Sans internet, Moli ne voit plus le chauffage, mais le thermostat et
les vannes continuent de suivre leur planning.

## 1. Créer votre app chez Netatmo (une fois, 2 minutes)

Netatmo donne l'accès à son API par des « apps » que chacun crée gratuitement avec son
compte habituel. Personne d'autre que vous n'a les clés de la vôtre.

1. Ouvrez <https://dev.netatmo.com> et connectez-vous avec **le compte de l'appli Netatmo**
   (celui qui voit votre thermostat).
2. Menu de votre compte → **My apps** → **Create** (ou directement
   <https://dev.netatmo.com/apps/createanapp>).
3. Remplissez :
   - **app name** : ce que vous voulez, par exemple `Moli` ;
   - **description** : par exemple `Domotique de la maison` ;
   - **data protection officer name** et **email** : vous-même ;
   - acceptez les conditions d'utilisation de l'API Netatmo.
4. Enregistrez (**Save**). La page de l'app affiche alors deux valeurs : **client ID** et
   **client secret**. Laissez-la ouverte.
5. Dans le champ **Redirect URI** de l'app, mettez exactement l'adresse que montre la carte
   « Chauffage Netatmo » de Moli (par défaut `http://localhost/moli-netatmo`), puis
   enregistrez à nouveau. Cette page n'a pas besoin d'exister : c'est l'adresse où Netatmo
   renverra votre navigateur, et vous n'aurez qu'à la copier.
6. Laissez **Webhook URI** vide : Moli n'en a pas besoin.

## 2. Donner les clés de l'app à Moli

Deux façons, au choix ; jamais dans `moli.toml`, jamais dans un message ou un chat.

- **Par le tableau de bord** (le plus simple) : Système → carte **Chauffage Netatmo** → collez
  le client ID et le client secret → Enregistrer. Votre code est demandé ; les clés partent au
  coffre chiffré de Moli et ne réapparaissent nulle part. Le pilote les prend dans les
  5 secondes.
- **En ligne de commande**, sur la machine de Moli, chaque valeur par un tube (jamais au
  clavier ni à l'écran), puis redémarrer Moli :

  ```
  read -rs V; printf %s "$V" | moli-os secrets set netatmo client_id; unset V
  read -rs V; printf %s "$V" | moli-os secrets set netatmo client_secret; unset V
  ```

  (`netatmo` est l'`id` de l'instance dans `moli.toml`.)

## 3. Autoriser Moli

Dans la carte **Chauffage Netatmo** :

1. **Ouvrir la page Netatmo** : c'est la vraie page de Netatmo. Connectez-vous si besoin et
   acceptez : Moli demande seulement `read_thermostat` et `write_thermostat` (lire et régler le
   chauffage ; rien sur les caméras, la météo ou le reste du compte).
2. Netatmo renvoie le navigateur vers `http://localhost/moli-netatmo?state=…&code=…` : la page
   ne s'affiche pas, **c'est normal**. Copiez son adresse complète.
3. Collez-la dans Moli → **Connecter**. Le code ne sert qu'une fois et expire vite : collez-le
   sans attendre. Coller le code seul marche aussi.

Moli échange le code contre un jeton, qu'il range chiffré dans son coffre. Netatmo change ce
jeton à chaque renouvellement (toutes les 3 heures environ) : Moli range toujours le dernier.
Pour couper l'accès : supprimez l'app sur dev.netatmo.com, ou retirez l'autorisation depuis
votre compte Netatmo ; Moli redemandera alors une autorisation.

## Configuration

```toml
[[driver]]
id = "netatmo"
kind = "netatmo"
[driver.options]
# redirect_uri = "http://localhost/moli-netatmo"  # la même que dans l'app de dev.netatmo.com
# setpoint_minutes = 180    # durée d'une consigne donnée par Moli ; sans : celle de la maison
#                           # dans l'appli Netatmo, sinon 3 h (1 à 720)
# poll_seconds = 300        # relecture de l'état (120 au moins)
```

## Ce que Moli voit

**Une pièce chauffée** (thermostat ou vanne ; sa pièce Moli suggérée est la pièce Netatmo,
donc la garde des **pièces protégées** s'y applique) :

| Point | Sens |
|---|---|
| `temperature` | température mesurée (°C) |
| `setpoint` ✎ | consigne (°C), de 7 à 30 par pas de 0,5 (les bornes de Netatmo ; hors bornes : refusé). Écrire une consigne la rend **manuelle pour `setpoint_minutes`** (3 h par défaut), puis la pièce revient d'elle-même à son planning |
| `setpoint_mode` ✎ | `schedule` (planning), `manual`, `max` (boost), `away`, `hg` (hors-gel), `off`. Seul `schedule` s'écrit : la pièce revient au planning |
| `setpoint_end` | fin de la consigne manuelle en cours (horodatage en ms), vide sinon |
| `heating` | la pièce demande de la chauffe (vannes : demande de puissance ; pièce à thermostat seul : chaudière en marche) |

**La maison** (un appareil par maison Netatmo ; son ordre touche toutes les pièces, la garde
les vérifie toutes) :

| Point | Sens |
|---|---|
| `mode` ✎ | `schedule` (planning), `away` (absent), `hg` (hors-gel), sans limite de durée jusqu'au prochain changement. Lu d'après les pièces : `away` ou `hg` quand toutes le sont |
| `schedule` ✎ | le planning suivi, par son nom (la liste des plannings de la maison). En écrire un autre bascule la maison dessus. Comme le mode, il touche toutes les pièces : un agent attend l'accord d'une personne dès qu'une pièce est protégée |

**Chaque module** (thermostat, vannes, relais) :

| Point | Sens |
|---|---|
| `reachable` | le module répond au relais |
| `rf_signal` | signal radio vers le relais (dBm : −60 bon, −90 faible) |
| `battery`, `battery_low`, `battery_voltage` | thermostat et vannes : niveau dit par Netatmo (`max` … `very_low`), alerte (`low`, `very_low`), tension en mV |

## Les plannings de chauffe

Le tableau de bord a une page **Planning de chauffe** (`#/chauffage`), ouverte depuis la
pastille de chaque pièce chauffée (Pièces) ou depuis la carte « Chauffage Netatmo » (Système).
Sans passer par l'appli Netatmo, on y :

- choisit le planning suivi (« Utiliser ce planning ») ;
- voit la semaine en grille, un réglage (Confort, Nuit, Éco…) par couleur ; on fait glisser
  une limite pour changer l'heure (par quart d'heure), on touche une plage pour lui donner un
  autre réglage ou en ajuster le début et la fin, on touche un jour pour le recopier sur
  d'autres ;
- règle la température de chaque réglage dans chaque pièce (de 7 à 30 °C, par demi-degré), et
  celles d'absence et de hors-gel ;
- duplique un planning pour en essayer un autre sans toucher à celui qu'on suit.

**Enregistrer demande le code de la maison** : changer un planning est le geste d'une
personne, jamais celui d'un agent, de Moli ni d'un automatisme. Avant l'envoi, Moli vérifie
tout (températures de 7 à 30 par 0,5, premier créneau le lundi à 0 h, créneaux dans l'ordre et
dans la semaine, réglages et pièces qui existent, chaque pièce chauffée a sa température dans
chaque réglage, 20 réglages et 1 000 créneaux au plus, nom de 60 caractères au plus) ; puis il
envoie une seule requête à Netatmo (`synchomeschedule`, `createnewhomeschedule` ou
`switchhomeschedule`) et relit aussitôt les plannings chez Netatmo : ce que la page affiche
ensuite est ce que Netatmo garde.

Une **consigne manuelle** en cours (donnée dans Moli ou l'appli Netatmo) passe avant le
planning jusqu'à sa fin ; la page le dit en haut, pièce par pièce, avec « Revenir au
planning ». De même, une maison en absence ou en hors-gel ne suit pas son planning tant que
dure ce mode.

## Prudence

- **Quotas** : l'état est relu toutes les 5 minutes, jamais deux requêtes à moins d'une
  seconde d'écart. Quand Netatmo dit que le quota est atteint, Moli se tait 1 minute, puis 2,
  4… jusqu'à 30 minutes, et le dit dans la carte.
- **Un ordre part tout de suite ou jamais** : s'il n'a pas pu partir dans les 5 secondes
  (Netatmo lent, quota), il est refusé et n'est jamais envoyé plus tard (un radiateur qui
  chauffe des heures après « échec », c'est pire qu'un refus).
- Après chaque ordre, Moli relit aussitôt l'état chez Netatmo. Les vannes reçoivent la
  consigne par radio à leur rythme (souvent quelques minutes).

## Tester chez soi

1. **Lecture d'abord** : après la connexion, vérifier que chaque pièce chauffée apparaît avec
   la bonne température (comparer avec l'appli Netatmo), que le mode de la maison et les
   batteries des vannes sont justes. Rien n'est écrit à cette étape.
2. **Une consigne, sur une pièce neutre** (bureau, salon : **jamais une chambre**, et jamais
   une pièce protégée), la personne présente : noter la consigne actuelle, en donner une de
   0,5 °C de plus depuis Moli, vérifier qu'elle apparaît dans l'appli Netatmo (manuelle,
   avec sa fin), puis remettre la pièce sur `schedule` (`setpoint_mode`) et vérifier qu'elle
   suit à nouveau son planning.
3. Le mode de la maison (`away`, `hg`) touche toutes les pièces, chambres comprises : ne le
   tester qu'avec l'accord de la famille, et le remettre sur `schedule` aussitôt.
4. **Les plannings, en lecture d'abord** : ouvrir Planning de chauffe, comparer la semaine, les
   températures de chaque réglage et le planning « en cours » avec l'appli Netatmo.
5. **Une modification réversible** : « Dupliquer pour essayer », changer une heure ou une
   température dans la copie, « Créer ce planning » (le code est demandé) ; vérifier que la
   copie apparaît dans l'appli Netatmo, identique à ce que montre Moli. Ne pas l'utiliser :
   le planning suivi n'a pas bougé. La supprimer ensuite depuis l'appli Netatmo (Moli ne
   supprime pas de planning). Changer de planning suivi touche toutes les pièces, chambres
   comprises : seulement avec l'accord de la famille, et en revenant aussitôt au précédent.
