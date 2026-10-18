**Titre proposé :** Netatmo Energy : pilote natif sur l'API cloud officielle

**Branche :** `Brizgir:contrib/netatmo-energy` → `rudy-io:main` (un commit, sur `d54c3de`)

*Ferme l'issue « Pilote natif Netatmo Energy » (à lier après l'avoir ouverte : `Closes #…`).*

---

## What and why

**En bref (English summary).** A native driver for Netatmo Energy — smart thermostat,
smart radiator valves and their relay — through Netatmo's **official cloud API**, since the
relay offers no usable local interface. Each household creates its own free app on
dev.netatmo.com, files its keys from a new « Chauffage Netatmo » card in System (encrypted
store, human-only route), then signs in with OAuth2 by pasting the return address. Each
heated room becomes a device (temperature, setpoint 7–30 °C, mode, heating demand) in its
Moli room so the guard applies; the home has its heating mode; each module reports battery
and radio signal. Quotas are respected, an order that could not leave within 5 s is never
sent late, tokens rotate safely. Tested with anonymised fixtures and on a real installation.
Nothing changes for homes that do not declare `kind = "netatmo"`.

### Ce que ça change

- **Nouveau crate `crates/moli-netatmo`** (`kind = "netatmo"`) :
  - `auth.rs` : OAuth2 *authorization code* (page d'autorisation, `state` aléatoire vérifié,
    échange du code, refresh token rangé au coffre **avant** tout usage car Netatmo le fait
    tourner ; accès renouvelé 5 min avant sa fin et une fois de plus s'il est refusé) ;
  - `api.rs` : client HTTP, espacement d'une seconde, quota (code 26 / HTTP 429) → silence
    1 min doublé jusqu'à 30 min ;
  - `model.rs` : `homesdata` + `homestatus` → appareils et points ; ordres
    (`setroomthermpoint`, `setthermmode`) avec bornes vérifiées ;
  - `lib.rs` : la boucle du pilote (statut `waiting` traduit tant que l'app ou l'autorisation
    manquent, relecture après chaque ordre, ordre tardif jamais envoyé).
- **Appareils publiés** : le compte (`status`, `sign_in_url`, `redirect_uri`, `sign_in` ✎),
  une **pièce chauffée** par pièce Netatmo (`temperature`, `setpoint` ✎, `setpoint_mode` ✎,
  `heating` ; `native_room` = son nom), la **maison** (`mode` ✎ `schedule|away|hg`, membres =
  ses pièces), chaque **module** (`reachable`, `rf_signal`, batterie).
- **API** : `GET|PUT /api/integrations/netatmo-app` (les clés de l'app ; `PUT` humain
  seulement, comme Tuya cloud ; `GET` ne rend jamais les clés).
- **Tableau de bord** : carte « Chauffage Netatmo » dans Système (`NetatmoAccount.svelte`),
  affichée seulement quand un pilote netatmo est déclaré : créer l'app, coller les clés,
  autoriser, coller l'adresse de retour.
- **`integrations/netatmo/`** : `integration.toml`, `README.md` (mode d'emploi pas à pas,
  points, prudence, ce qui a été vérifié), `onboarding.md` (pour un agent).
- **Docs** : `pilotes.md` (§ netatmo et tableau), `api.md` (la route). **Traductions** fr et en
  (`pilotes`, `serveur`, `systeme`).

### Pourquoi ainsi

- Pas d'interface locale sur le relais : l'API officielle est la seule voie stable et
  autorisée. Scopes limités à `read_thermostat write_thermostat`.
- Une app par maison : pas de clé commune au projet, pas de serveur intermédiaire ; chacun
  peut couper l'accès depuis son compte Netatmo.
- Un radiateur qui chauffe des heures après un « échec » est pire qu'un refus : d'où l'ordre
  tout de suite ou jamais.

## Guide détaillé : mettre en route et essayer

> Toute la procédure est aussi dans `integrations/netatmo/README.md`. Prévoir 10 minutes.

**0. Déclarer le pilote.** Dans `moli.toml` :

```toml
[[driver]]
id = "netatmo"        # les secrets sont rangés sous cet id : ne plus le renommer
kind = "netatmo"
[driver.options]
# redirect_uri = "http://localhost/moli-netatmo"   # défaut
# setpoint_minutes = 180                           # durée d'une consigne donnée par Moli
# poll_seconds = 300                               # 120 au moins
```

`moli-os check-config`, redémarrer. La carte **Système → Chauffage Netatmo** apparaît et
invite à créer l'app.

**1. Créer l'app sur dev.netatmo.com.**

1. Ouvrir <https://dev.netatmo.com>, se connecter avec **le compte de l'appli Netatmo** (celui
   qui voit le thermostat).
2. Menu du compte → **My apps** → **Create** (ou <https://dev.netatmo.com/apps/createanapp>,
   aussi derrière le bouton « Créer l'app » de la carte).
3. *app name* et *description* au choix (par exemple `Moli`, `Domotique de la maison`) ;
   *data protection officer* : soi-même ; accepter les conditions ; **Save**.
4. La page de l'app affiche le **client ID** et le **client secret** : la garder ouverte.

**2. Redirect URI.** Dans le champ **Redirect URI** de l'app, mettre **exactement** l'adresse
que montre la carte (par défaut `http://localhost/moli-netatmo`), enregistrer. Cette page n'a
pas besoin d'exister : c'est là que Netatmo renverra le navigateur. Laisser *Webhook URI* vide.

**3. Les clés dans Système.** Carte **Chauffage Netatmo** → coller client ID et client secret →
**Enregistrer**. Le code du tableau de bord est demandé ; les clés partent au coffre chiffré et
ne réapparaissent nulle part. Le pilote les prend en moins de 5 s. (Variante sans tableau de
bord : `read -rs V; printf %s "$V" | moli-os secrets set netatmo client_id; unset V`, idem
pour `client_secret`, puis redémarrer.)

**4. Autoriser.** **Ouvrir la page Netatmo** (de préférence sur un ordinateur) → se connecter si
besoin → accepter (lecture et réglage du chauffage seulement). Le navigateur arrive sur
`http://localhost/moli-netatmo?state=…&code=…` qui **ne s'affiche pas : c'est normal**. Copier
l'adresse complète, la coller dans la carte → **Connecter**, sans attendre (le code expire
vite). La carte dit « Compte Netatmo connecté : *n* pièces chauffées suivies par Moli ».

**5. Premiers essais, en lecture seulement.** Rien n'est écrit à cette étape.

- chaque pièce chauffée apparaît, avec la même température et la même consigne que l'appli
  Netatmo ;
- le mode de la maison est juste (`schedule` en temps normal) ;
- les modules ont leur batterie et leur signal ;
- les pièces Netatmo tombent dans les bonnes pièces Moli (c'est ce qui fait jouer la garde
  des pièces protégées) ; sinon, les ranger.

**6. Une consigne, sur une pièce neutre.** Bureau ou salon, **jamais une chambre ni une pièce
protégée**, quelqu'un sur place :

1. noter la consigne actuelle ;
2. donner 0,5 °C de plus depuis Moli ;
3. vérifier dans l'appli Netatmo : la pièce est en manuel, avec sa fin (3 h par défaut) ;
4. remettre la pièce sur `schedule` (`setpoint_mode`) et vérifier qu'elle suit de nouveau son
   planning.

Le **mode de la maison** (`away`, `hg`) touche toutes les pièces, chambres comprises : ne
l'essayer qu'avec l'accord de la famille, et revenir à `schedule` aussitôt.

> ⚠️ **Piège vécu : « revenir au planning » suit le mode de la maison.** Remettre une pièce sur
> `schedule` demande à Netatmo de lui faire suivre la maison (`mode=home`), pas le planning
> lui-même. Si la maison est en hors-gel (ou absente), la pièce repart en **hors-gel**. Pour
> retrouver le planning partout, remettre d'abord le `mode` de la maison sur `schedule`.

**Couper l'accès** : supprimer l'app sur dev.netatmo.com, ou retirer l'autorisation depuis le
compte Netatmo ; Moli redemande alors une autorisation.

## Vérifié sur une installation réelle

Octobre 2026, une maison Netatmo avec **un thermostat connecté (NATherm1), trois vannes de
radiateur (NRV) et leur relais (NAPlug)** ; firmware tel que livré, non relevé.

- création de l'app, clés par la carte de Système, autorisation par l'adresse collée, jeton
  renouvelé seul depuis ;
- lecture des pièces : températures et consignes identiques à l'appli Netatmo ; mode de la
  maison ; batteries et signal des modules ;
- une consigne sur une pièce neutre, vue manuelle avec sa fin dans l'appli Netatmo, puis
  retour de la pièce au planning, vérifié dans l'appli ;
- maison passée de **hors-gel à planning** (c'est là qu'est apparu le piège ci-dessus).

Aucune donnée de cette installation n'est dans la PR : les fixtures
(`crates/moli-netatmo/src/fixtures/`) sont fictives (maison `home-1`, pièces `1001…`, modules
`aa:bb:cc:dd:ee:0x`, `user@example.com`).

## Ordre de fusion

1. **Cette PR** (`contrib/netatmo-energy`), indépendante, directement sur `main`.
2. Ensuite la PR des plannings (`contrib/netatmo-plannings`), empilée sur celle-ci.

## Limites connues

- **Cloud seulement** : sans internet, Moli ne voit plus le chauffage (le thermostat et les
  vannes continuent leur planning).
- Relecture toutes les 5 min : une consigne changée dans l'appli Netatmo se voit dans Moli au
  plus 5 min après ; les vannes reçoivent un ordre par radio à leur rythme (quelques minutes).
- Le piège du retour au planning ci-dessus (comportement de l'API Netatmo, documenté).
- OpenTherm (`OTH`, `OTM`) est reconnu comme le thermostat mais n'a pas été essayé sur du vrai
  matériel ; une seule maison Netatmo essayée (le code en gère plusieurs, comme la fixture).
- Pas de webhook : Moli lit, il n'écoute pas les événements poussés par Netatmo.

## Checked

- [x] `sh scripts/check.sh` (fmt, clippy, tests, catalogue, leak check) — Rust 1.99 / clippy
  pédant, tous les tests verts
- [x] `npm run build` in `web/` and `node scripts/i18n-check.mjs`
- [x] New texts in `locales/fr` **and** `locales/en`
- [x] No personal data (names, addresses, MACs, serials, keys): fake values only
- [x] Tried on real hardware? Which model, firmware, date (in the integration's README) —
  NATherm1, 3 × NRV, NAPlug, octobre 2026, firmware non relevé
