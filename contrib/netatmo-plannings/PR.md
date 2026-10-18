**Titre proposé :** Netatmo : lire et modifier les plannings de chauffe depuis Moli

**Branche :** `Brizgir:contrib/netatmo-plannings`, empilée sur `contrib/netatmo-energy` (un
commit au-dessus du pilote).

> Tant que la PR du pilote n'est pas fusionnée, cette PR montre aussi son commit. Pour la
> relire seule : comparer `contrib/netatmo-energy...contrib/netatmo-plannings`. Une fois le
> pilote fusionné, elle ne contient plus que le commit des plannings.

*Ferme l'issue « Plannings de chauffe Netatmo » (à lier : `Closes #…`). Dépend de la PR du
pilote Netatmo Energy.*

---

## What and why

**En bref (English summary).** Read and edit Netatmo heating schedules from Moli's
dashboard. A small, generic **per-device service** is added to the runtime (a document to
read, actions on it): `GET /api/devices/{id}/service/{name}` is free to read (a guest: its
rooms only), `POST …/{action}` is a person's gesture only (dashboard + their code), never an
agent, script or automation. The Netatmo driver provides `schedules`, `sync`, `create` and
`activate`, validates every draft strictly before anything is sent, sends one request at a
time and re-reads the home afterwards. A new « Heating schedule » page shows the week as a
grid (drag a boundary, tap a slot, copy a day, temperatures per setting and per room,
away and frost-guard, the followed schedule, duplicate to try). Nothing changes for homes
without Netatmo.

### Ce que ça change

- **Runtime** (`crates/moli-runtime/src/service.rs`, `driver.rs`, `hub.rs`) : le trait
  `Service` (lire un document, faire une action), `DriverCtx::provide_service`,
  `Hub::service_read` / `service_write` (sans file, 25 s au plus), chaque action journalisée
  (`Action::Service` dans `moli-core`).
- **API** (`crates/moli-api/src/service.rs`, `limit.rs`) : les deux routes ci-dessus ; noms
  `[a-z_]{1,32}` ; écriture réservée à `x-moli-origin: ui` + session humaine, refusée à un
  invité ; 10 écritures / 10 s. Test HTTP : lecture libre ; écriture refusée à un agent, à un
  script, au tableau de bord sans le code, à une session sans l'en-tête du tableau de bord ;
  acceptée pour une personne ; brouillon invalide → `422`, nom inconnu → `404`. (Le refus à
  un invité est dans le code, sans test dédié.)
- **Pilote netatmo** (`src/schedule.rs`, `model.rs`, `lib.rs`, `api.rs`) : plannings lus dans
  `homesdata` ; point `schedule` ✎ sur la maison (le planning suivi, par la garde) ;
  `setpoint_end` sur chaque pièce (fin de la consigne manuelle) ; le service avec
  **validation stricte avant envoi** (nom 1–60, 7–30 °C par 0,5, zones distinctes et connues,
  chaque pièce chauffée dans chaque zone, ≤ 20 zones, ≤ 1 000 créneaux, premier à lundi 0 h,
  strictement croissants, < 10 080) ; corps JSON exact pour `synchomeschedule` et
  `createnewhomeschedule` ; `homesdata` relu après chaque changement et quand la page lit des
  plannings vieux de plus d'une minute.
- **Tableau de bord** : page **Planning de chauffe** (`#/chauffage`,
  `web/src/maison/pages/Chauffage.svelte`), logique de la semaine pure et testée
  (`web/src/maison/lib/heating.js`, `heating.test.js`, ajouté à `npm test`) ; lien depuis la
  carte de chaque pièce chauffée ; action de service lisible dans le Journal ; lien depuis la
  carte Système.
- **Docs** : `api.md` (§ 2.1 ter, les services), `pilotes.md`, `dashboard.md`, README et
  onboarding Netatmo. **Traductions** fr et en (`maison`, `pilotes`, `serveur`, `systeme`).

### Pourquoi ainsi

- Un planning (zones, températures par pièce, créneaux) n'est pas une valeur de point : le
  service le porte sans tordre le modèle, et resservira à d'autres pilotes.
- Écrire un planning change le chauffage de toute la maison pour des semaines : c'est un geste
  humain, comme `PUT /api/plan`. Le planning **suivi**, lui, reste un point ordinaire sous la
  garde (la maison a toutes ses pièces pour membres).
- Rien d'invalide ne part chez Netatmo, et la page montre ce que Netatmo **garde**, relu après
  l'envoi.

## Guide détaillé : essayer les plannings

> Prérequis : le pilote Netatmo Energy connecté (guide de la PR précédente : app sur
> dev.netatmo.com, Redirect URI, clés dans Système, autorisation, essais en lecture puis une
> consigne sur une pièce neutre). Aucune configuration de plus.

**1. Ouvrir la page.** Depuis la carte d'une pièce chauffée (« *Pièce* · *x*° visés ») ou
directement `#/chauffage`. Sans maison Netatmo, la page renvoie vers Système.

**2. Lecture d'abord.** Comparer avec l'appli Netatmo :

- le planning marqué **en cours** est celui que suit la maison ;
- la semaine (heures de chaque plage, jour par jour) ;
- les **températures** de chaque réglage (Confort, Nuit, Éco…) pièce par pièce, et celles
  d'absence et de hors-gel ;
- en haut, ce qui passe avant le planning : une consigne manuelle en cours (« *Pièce* est
  réglée à la main sur *x*° jusqu'à *hh:mm* ») ou la maison en absence / hors-gel.

**3. Une modification réversible, sur une copie.**

1. Choisir un planning → **Dupliquer pour essayer** ;
2. dans la copie, glisser une limite d'un quart d'heure, ou changer une température d'un
   demi-degré ;
3. **Créer ce planning** : le code est demandé ;
4. vérifier dans l'appli Netatmo que la copie existe, identique à ce que montre Moli ;
5. **ne pas l'utiliser** : le planning suivi n'a pas bougé ;
6. la supprimer depuis l'appli Netatmo (Moli ne supprime pas de planning).

**4. Changer le planning suivi** (« Utiliser ce planning ») touche toutes les pièces, chambres
comprises : seulement avec l'accord de la famille, et revenir aussitôt au précédent.

> ⚠️ **Piège vécu : « Revenir au planning » suit le mode de la maison.** Le bouton remet la
> pièce sur `mode=home` chez Netatmo : si la maison est en **hors-gel** (ou en absence), la
> pièce repart en hors-gel, pas sur le planning affiché. La page le signale (« La maison est en
> hors-gel : le planning reprendra quand elle repassera sur le planning ») ; remettre d'abord
> la maison sur `schedule`.

## Vérifié sur une installation réelle

Octobre 2026, la même installation que pour le pilote (un thermostat, trois vannes, un
relais) :

- **lecture des plannings** : semaine, réglages et températures par pièce, planning suivi,
  conformes à l'appli Netatmo ;
- la consigne manuelle en cours et son retour au planning, la maison passée de hors-gel à
  planning (vérifiés avec le pilote).

**Pas encore essayé sur du vrai matériel** : l'enregistrement d'un planning modifié (`sync`),
la création (`create`) et le changement de planning suivi (`activate`). Ils sont couverts par
les tests (validation, corps exact envoyé, formulaire de `switchhomeschedule`) mais un essai
réel suivant le guide ci-dessus, étape 3, serait bienvenu avant la fusion.

Fixture fictive (`crates/moli-netatmo/src/fixtures/homesdata.json` : plannings `sched-1…3`,
pièces `1001…`, modules `aa:bb:cc:dd:ee:0x`) ; aucun identifiant réel.

## Ordre de fusion

1. La PR du pilote Netatmo Energy (`contrib/netatmo-energy`) ;
2. **puis celle-ci** (`contrib/netatmo-plannings`). Après la fusion de la première, cette
   branche s'applique sans conflit sur `main` (un seul commit au-dessus).

## Limites connues

- Moli ne **supprime** pas de planning (à faire dans l'appli Netatmo).
- Écriture des plannings non essayée en vrai (voir plus haut).
- Le piège du retour au planning en hors-gel (comportement de Netatmo, signalé par la page).
- Un seul service existe aujourd'hui (Netatmo) ; il n'est pas exposé par MCP, volontairement :
  l'écriture est un geste humain.
- Les plannings modifiés dans l'appli Netatmo se voient à la prochaine lecture (15 min, ou dès
  l'ouverture de la page si la dernière lecture a plus d'une minute).

## Checked

- [x] `sh scripts/check.sh` (fmt, clippy, tests, catalogue, leak check) — Rust 1.99 / clippy
  pédant, tous les tests verts
- [x] `npm run build` in `web/` and `node scripts/i18n-check.mjs` (et `npm test` : 23 tests,
  dont la logique de la semaine)
- [x] New texts in `locales/fr` **and** `locales/en`
- [x] No personal data (names, addresses, MACs, serials, keys): fake values only
- [x] Tried on real hardware? Which model, firmware, date (in the integration's README) —
  lecture des plannings seulement ; écriture couverte par les tests, pas encore essayée
