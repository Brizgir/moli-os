# Pièces : les créer, les renommer et leur donner d'autres noms depuis le tableau de bord

*Titre en anglais, si besoin : « Rooms: create, rename and alias them from the dashboard »*

Branche : `Brizgir/moli-os:contrib/pieces-depuis-le-tableau-de-bord` → `rudy-io/moli-os:main`
Issue : #… (à remplir une fois l'issue ouverte)

## What and why

**En bref (English summary).** Rooms of the family dashboard could only be declared by
editing `data/home.json` by hand. This adds a human-only API (`POST /api/home/rooms`,
`PUT` / `DELETE /api/home/rooms/{name}`) that rewrites `rooms[]` atomically and leaves the
rest of the file untouched, a `Hub::rename_room` that moves the devices' `room` labels when a
room is renamed (the old name stays as an alias), a `home_changed` event so open dashboards
reload the layout, and a « New room » sheet plus a ⚙ on each room card in the Rooms tab.
Agents get 403; protecting a room stays in `moli.toml`. One commit, French and English texts,
HTTP and hub tests.

**Ce que ça change.**

- **Cœur** (`moli-runtime`, `moli-core`) : `Hub::rename_room(from, to, origin, actor)` réécrit
  l'étiquette `room` de chaque appareil rangé dans `from` (même comparaison que la garde :
  accents, casse et espaces ignorés), écrit `labels.toml` une fois, émet un `label_changed` et
  une ligne de journal par appareil ; `Hub::home_changed()` émet le nouvel
  `Event::HomeChanged`. `guard::same_room` devient `pub` pour cela.
- **API** (`moli-api`, nouveau module `home.rs`) :
  - `POST /api/home/rooms` `{ name, icon?, aliases? }` → `201` + le fichier entier ;
  - `PUT /api/home/rooms/{name}` (nom ou alias ; champ absent = inchangé, `icon: ""` = sans
    icône) → `200` ; renommer appelle `rename_room` et garde l'ancien nom en alias ; une
    pièce que seuls les appareils nommaient est déclarée au passage ;
  - `DELETE /api/home/rooms/{name}` → `200`, `404` si elle n'est pas dans la liste.
  Session humaine seulement (`x-moli-origin: ui` + cookie du code) : `403` sinon. Bornes :
  60 caractères par nom, pas de doublon (nom ou alias d'une autre pièce), icône de
  `ROOM_ICONS`, 10 alias, 100 pièces ; `422` avec un message traduit ; `503` sans dossier de
  données. Écriture atomique sous verrou, puis `home_changed` sur `/api/events` (laissé
  passer aux invités : la disposition est déjà publique par `GET /api/home`).
- **Tableau de bord** (`web/src/maison`) : bouton « Nouvelle pièce » dans la barre de
  l'onglet Pièces, fiche `ui/RoomForm.svelte` (nom, icône, autres noms ; « Retirer de la
  liste » pour une pièce existante ; rappel que la protection se règle dans `moli.toml`),
  ⚙ sur chaque carte de pièce. Le code de la maison est demandé une fois si la session n'est
  pas humaine (`asHuman`, qui prend maintenant la raison affichée en paramètre). Une pièce
  déclarée vide s'affiche avec une invitation à y ranger un appareil (lien vers l'Atelier).
  `home.svelte.js` relit `/api/home` sur `home_changed`.
- **Docs** : `docs/components/api.md` (trois routes, l'événement) et
  `docs/components/dashboard.md` (la fiche). **Traductions** fr et en (`commun`, `maison`,
  `serveur`).

**Pourquoi.** Dans une maison où personne n'ouvre `home.json`, on ne pouvait ni créer une
pièce pour y ranger ses appareils, ni la renommer sans réétiqueter chaque appareil, ni dire
que « Chambre 2 » (Hue) et « Chambre des enfants » (le plan) sont la même. Détails et choix
dans l'issue.

## Guide détaillé : l'essayer

Prérequis : une instance construite depuis cette branche, le code du tableau de bord, un
dossier de données (`data_dir`) inscriptible. Les exemples sont fictifs.

1. **Construire et lancer.**
   ```sh
   sh scripts/dev.sh sh scripts/check.sh            # ou .\scripts\dev.ps1 sous Windows
   cd web && npm ci && npm run build && cd ..
   docker build -t moli-os . && docker compose up -d   # ou votre façon habituelle
   ```
   La villa de démonstration (`demo/villa`) montre la fiche et le ⚙, mais refuse toute
   écriture (« pas dans la démo », comme pour le reste) : pour enregistrer, une instance
   normale, même sans appareil.
2. **Sauvegarder l'existant** (si vous avez déjà un `home.json`) :
   `cp data/home.json data/home.json.avant`.
3. **Créer une pièce.** Ouvrir le tableau de bord → **Pièces** → **Nouvelle pièce**. Nom :
   `Buanderie`, icône : la goutte (`water`) ; autres noms : `Laverie, Laundry`.
   **Enregistrer** : le code de la maison est demandé si besoin.
   Attendu : une carte « Buanderie » avec « Aucun appareil ici pour l'instant » et le lien
   « Ranger un appareil dans cette pièce (Atelier) ». Dans `data/home.json`, `rooms[]`
   contient la pièce ; le reste du fichier (`hidden`, `favorites`…) n'a pas bougé.
4. **La voir arriver ailleurs.** Avec un second onglet (ou un téléphone) ouvert sur Pièces
   avant l'étape 3, la carte y apparaît sans recharger (`home_changed`).
5. **Y ranger un appareil.** Atelier → un appareil neutre (une prise, une lampe de couloir) →
   pièce `Buanderie`. Retour à Pièces : la carte montre l'appareil.
6. **Renommer.** ⚙ de la carte → nom `Cellier` → Enregistrer. Attendu : la carte s'appelle
   « Cellier », l'appareil y est toujours ; `home.json` a
   `"aliases": ["Laverie", "Laundry", "Buanderie"]` ; `labels.toml` porte
   `room = "Cellier"` ; le journal a une ligne par appareil déplacé.
7. **Refus attendus.**
   - Créer `cellier` → 422 « … désigne déjà la pièce « Cellier » ».
   - Un nom de 61 caractères → 422.
   - Depuis un terminal, sans session humaine :
     ```sh
     curl -i -X POST http://<hôte>:8790/api/home/rooms \
       -H 'content-type: application/json' -H 'x-moli-origin: ui' \
       -d '{"name":"Essai"}'
     ```
     → `403` et `home.json` inchangé. Même chose par le MCP : aucun outil n'écrit les pièces.
8. **Retirer.** ⚙ → **Retirer de la liste** → confirmer. La déclaration disparaît de
   `home.json` ; l'appareil garde son étiquette `Cellier` et la carte reste (nommée par
   l'appareil), sans icône ni alias.
9. **Remettre comme avant** : `cp data/home.json.avant data/home.json` et remettre la pièce
   d'origine de l'appareil dans l'Atelier.

## Vérifié sur une installation réelle

<!-- À relire avant publication : garder seulement ce qui a vraiment été fait chez vous. -->

Cette branche reprend, seule, une partie du code en service depuis le déploiement du
18 octobre 2026 dans une maison réelle (image Docker construite depuis le fork ; pilotes Hue,
Tuya, Alexa, Netatmo, Ring, météo et box ; pas de télé, de compteur ni d'imprimante) :

- pièces créées depuis l'onglet Pièces, apparues sur un autre écran ouvert sans recharger ;
- une pièce renommée : ses appareils ont suivi, l'ancien nom est resté en alias ;
- une pièce retirée de la liste : ses appareils ont gardé leur étiquette ;
- sans session humaine, l'écriture est refusée (403) et `home.json` reste intact (c'est aussi
  ce que vérifie le test HTTP `rooms_are_declared_by_a_person_and_a_rename_moves_the_labels`).

La version en service contenait aussi une relecture plus rapide de la liste des enceintes
Alexa (un Echo renommé dans l'appli suit en quelques minutes) ; elle est **retirée de cette
branche** et relève d'une autre contribution. Aucun nom de pièce, d'appareil ni de lieu de
cette maison n'apparaît dans le code, les tests ou ce texte.

Côté code, en plus de `check.sh` : la villa de démonstration lancée depuis cette pile de
branches, l'onglet Pièces regardé à 390 px et à 1400 px de large.

## Ordre de fusion

À fusionner **en premier** : la pull request « Tableau de bord : seulement ce que la maison
possède » (`contrib/tableau-de-bord-selon-la-maison`) est construite par-dessus (elle étend
`home.rs` avec `PUT /api/home/pages` et ajoute une carte « Nouvelle pièce » qui ouvre la
fiche de celle-ci).

## Limites connues

- Protéger une pièce reste dans `moli.toml` (`[guard] protected_rooms`, lu au démarrage) ; la
  fiche le rappelle sans le proposer. Renommer une pièce protégée ne renomme pas sa
  protection : il faut mettre `moli.toml` à jour (la garde ignore accents et casse, mais ne
  connaît pas les alias de `home.json`).
- Le renommage ne touche qu'aux étiquettes Moli (`labels.toml`), pas aux noms dans les
  applications des fabricants ; c'est pourquoi l'ancien nom reste en alias.
- La liste d'icônes est fermée (28 icônes) ; en ajouter une se fait dans `ROOM_ICONS` (Rust et
  JS) et `lib/icons.js`, un test vérifie l'accord.
- L'ordre des pièces suit `rooms[]` ; pas encore de glisser-déposer pour le changer.

## Checked

- [x] `sh scripts/check.sh` (fmt, clippy, tests, catalogue, leak check) — chaque étape lancée
      dans le cloud avec la toolchain stable à jour : vert
- [x] `npm run build` in `web/` and `node scripts/i18n-check.mjs`
- [x] New texts in `locales/fr` **and** `locales/en`
- [x] No personal data (names, addresses, MACs, serials, keys): fake values only
- [ ] Tried on real hardware? — pas une intégration : essayé sur une installation réelle
      (voir plus haut), aucun README d'intégration concerné

---
🤖 Generated with [Claude Code](https://claude.com/claude-code)
