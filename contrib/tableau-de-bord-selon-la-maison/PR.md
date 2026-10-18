# Tableau de bord : seulement ce que la maison possède

*Titre en anglais, si besoin : « Dashboard: only what the house has »*

Branche : `Brizgir/moli-os:contrib/tableau-de-bord-selon-la-maison` → `rudy-io/moli-os:main`
Issue : #… (à remplir une fois l'issue ouverte)
**Dépend de** la pull request « Pièces : les créer, les renommer et leur donner d'autres noms
depuis le tableau de bord » (`contrib/pieces-depuis-le-tableau-de-bord`), à fusionner d'abord :
cette branche est construite dessus et ses deux commits propres viennent après le sien.

## What and why

**En bref (English summary).** Each page of the family menu now declares what justifies it
(`web/src/maison/lib/pages.js`, a pure function with `node --test` tests): a TV for the
living room, a meter for energy, a printer, a floor plan, zones or phones, a voice satellite,
the weather. Without it, the page leaves the menu but its route still answers, and it comes
back by itself when such a device arrives. A « Pages shown » card in System lets a person
force each page (automatic / always / never), stored in `home.json` `pages` through
`PUT /api/home/pages` (human session only, atomic write, `home_changed`). Home-page and
Outside cards only show with their data; « All is well » replaces an empty alert row; a
« Today » card lists the day's timed automations; phones get a bottom tab bar with a « More »
sheet. A house that has everything sees the same pages as today. Plus a small commit: the
home page's climate section is titled « Heating » when it only lists radiators.

**Ce que ça change (deux commits).**

1. *Accueil : la section climat s'appelle « Chauffage » quand elle ne liste que des
   radiateurs.* Le titre et l'icône suivent `favorites.climate` : un point `heating` partout
   → « Chauffage » et un radiateur ; aucun → « Climatisation » ; un mélange → « Chauffage et
   climatisation ».
2. *Tableau de bord : seulement ce que la maison possède.*
   - **Menu** (`Maison.svelte`, `lib/pages.js`, `lib/menu.svelte.js`) : chaque entrée porte un
     `id` ; `visiblePages(facts, choices)` décide. Tant que la disposition n'est pas lue, seules
     Pièces, Automatismes et Moli s'affichent (pas de menu qui saute). Ordinateur (≥ 1100 px) :
     menu de 248 px avec le nom de chaque page ; tablette : rail étroit ; téléphone
     (< 768 px) : **barre d'onglets** Accueil, Pièces, Moli au centre, la première autre page
     affichée, **Plus** (feuille : le reste du menu, Système, Atelier, thème ; Échap ou un tap
     à côté la ferme ; elle se ferme aussi en changeant de page).
   - **Justifications** (`justified`) : Salon = une télé (`deviceKind` `tv`) ou `home.json`
     `salon` ; Plan = un étage dans `/api/plan` ; Carte = une zone ou un téléphone ;
     Caméras = une caméra ou une caméra favorite ; Parler = un boîtier vocal (point `state`
     qui connaît `listening`) ou la voix de Moli ; Énergie = `/api/energy` répond ou
     `home.json` `energy` ; Solaire (le lien de la page Énergie) = une production (`solar`,
     `pv_`, `production`) ; Impression = une imprimante 3D ou papier ; Dehors = la météo ;
     Pièces, Automatismes, Moli : toujours ; Infra : jamais d'elle-même.
   - **Réglage « Pages affichées »** (`ui/PagesSetting.svelte`, dans Système, sous un lien vers
     l'Atelier) : le nombre de pages montrées, puis par page Automatique / Toujours / Jamais
     et la raison du choix automatique. Enregistrer demande le code si besoin.
   - **API** : `PUT /api/home/pages` dans `home.rs` (fusion dans `pages`, `auto` efface le
     choix, plus de choix = plus de clé ; page ou choix inconnu → 422) ; tests unitaires et
     HTTP ; test qui vérifie que `PAGES` (Rust) = `PAGE_IDS` (JS).
   - **Accueil** : bande verte « Tout va bien : N services en marche, rien à surveiller »
     quand il n'y a pas d'alerte (N : les pilotes dont un appareil répond) ; lumières : les
     favorites, sinon une tuile par pièce qui a des lumières ; chauffage/clims seulement pour
     les favoris qui existent ; télécommande seulement avec `salon.tv` ; électricité seulement
     avec `/api/energy` ; « Dehors » seulement avec l'air, l'UV ou la piscine ; nouvelle carte
     **« Aujourd'hui »** (`ui/Today.svelte`, `lib/today.js`, `lib/timed.svelte.js`) : les
     automatismes horaires du jour, cochés une fois passés, lus toutes les deux minutes.
   - **Dehors** : la carte du ciel seulement avec la météo (sinon une phrase qui dit où
     l'ajouter), l'air seulement s'il est mesuré.
   - **Énergie** : le lien « simulation solaire » seulement si la page Solaire est affichée.
   - **Pièces** : en sous-titre « 6 pièces · 2 allumées » ; la température dans la ligne
     d'état de la carte, « sans thermomètre » sinon ; une pièce protégée le dit en une
     phrase ; une carte en pointillés **« Nouvelle pièce »** en fin de liste ouvre la fiche
     de la PR « pièces ».
   - **Docs** : `docs/components/dashboard.md` (nouvelle section « Ce qui s'affiche selon la
     maison », navigation), `docs/components/api.md` (la route). **Traductions** fr et en.

**Pourquoi.** Voir l'issue : un menu et un Accueil qui montrent tout à tout le monde
font, dans une maison sans télé ni compteur ni imprimante, des pages vides et des tirets.
Rien n'est supprimé : le code des pages et des cartes reste, seule leur condition
d'affichage change.

## Guide détaillé : l'essayer

Prérequis : une instance construite depuis cette branche (qui contient la PR « pièces »), le
code du tableau de bord. Les exemples sont fictifs.

1. **Construire et vérifier.**
   ```sh
   sh scripts/dev.sh sh scripts/check.sh            # ou .\scripts\dev.ps1 sous Windows
   cd web && npm ci && npm test && npm run build && cd ..
   node scripts/i18n-check.mjs
   docker build -t moli-os .
   ```
   `npm test` passe maintenant aussi `pages.test.js` et `today.test.js`.
2. **Une maison qui a tout : rien ne disparaît.** Lancer la villa de démonstration :
   ```sh
   cd demo/villa && MOLI_UI_PIN=123456 cargo run -p moli-os -- --config moli.toml serve
   ```
   Ouvrir `http://localhost:8790`. Sur ordinateur, le menu montre Accueil, Pièces, Salon,
   Plan, Carte, Caméras, Énergie, Impression, Dehors, Automatismes, Demander à Moli (Parler
   n'y est pas : la villa n'a ni boîtier vocal ni voix pour Moli ; Infra est masquée par
   défaut). En pied de menu : le thème et Système.
3. **Le téléphone.** Outils de développement → mode appareil, 390 × 844. En bas : Accueil,
   Pièces, la bulle Moli, Salon, Plus. **Plus** ouvre la feuille : Plan, Carte, Caméras,
   Énergie, Impression, Dehors, Automatismes, Système, Atelier, le thème. Échap la ferme.
4. **Le réglage.** Système → **Pages affichées** → Choisir. Chaque page dit pourquoi elle est
   là ou pas (« Masquée : ni boîtier vocal ni voix pour Moli », « Masquée : elle sert dans
   l'Atelier »…). Sur une instance normale (la démo refuse toute écriture : « pas dans la
   démo »), passer **Énergie** à **Jamais** : le code est demandé, Énergie sort du menu,
   `data/home.json` contient `"pages": { "energie": "never" }`, et un autre écran ouvert suit
   sans recharger. Repasser à **Automatique** : la clé `pages` disparaît. Passer **Infra** à
   **Toujours** : elle revient dans le menu.
5. **Une maison qui n'a pas tout : le menu se réduit.** Sur une instance avec seulement des
   lumières, une prise et la météo (ou la villa en retirant de `demo/villa/moli.toml` les
   instances télé, imprimantes, caméras et énergie), le menu se réduit à Accueil, Pièces,
   Dehors, Automatismes, Moli (plus Carte s'il y a des téléphones appairés ou des zones).
   L'Accueil n'a ni télécommande, ni électricité, ni tirets dans « Dehors ».
6. **Une page qui revient d'elle-même.** Ajouter un appareil du type manquant (une télé
   Philips ou Android TV, une imprimante IPP…) : dès qu'il est publié, la page correspondante
   revient dans le menu, sans recharger. Le retirer (ou le masquer par `hidden`) : elle
   repart.
7. **Une page masquée reste servie.** Taper `#/impression` dans la barre d'adresse d'une
   maison sans imprimante : la page s'ouvre, avec son état vide.
8. **Refus attendus.**
   ```sh
   curl -i -X PUT http://<hôte>:8790/api/home/pages \
     -H 'content-type: application/json' -H 'x-moli-origin: ui' \
     -d '{"energie":"never"}'
   ```
   → `403` (pas de session humaine). Avec une session humaine, `{"atelier":"never"}` ou
   `{"salon":"parfois"}` → `422`, `home.json` inchangé.
9. **Remettre comme avant** : repasser chaque page modifiée à **Automatique** (ou retirer
   `pages` de `home.json`).

## Vérifié sur une installation réelle

<!-- À relire avant publication : garder seulement ce qui a vraiment été fait chez vous. -->

Le code d'origine tourne depuis le déploiement du 18 octobre 2026 dans une maison réelle :
Hue, Tuya, Alexa, Netatmo, Ring, météo et box ; ni télé, ni compteur, ni imprimante.

- **Menu réduit** comme attendu : Salon, Énergie, Impression, Plan et Infra hors du menu ;
  Pièces, Dehors, Automatismes, Moli présents (dans la version en service, une page de
  planning de chauffage et une entrée « Sonnette et caméras » s'y ajoutent : elles viennent
  de contributions séparées et ne sont **pas** dans cette branche) ;
- **retour automatique** d'une page quand un appareil de son type arrive, sans recharger ;
- **forçage** dans Système → Pages affichées (Toujours / Jamais), suivi par les autres écrans ;
- barre d'onglets et feuille « Plus » sur téléphone.

Cette branche est la version **générique** de ce code : les cartes propres à des pilotes qui
ne sont pas encore dans le projet (carte Chauffage Netatmo avec planning et mode absence,
sonnette Ring sans image, carte Musique des enceintes) en ont été retirées, et le calcul des
pages ne cite aucune marque. Aucun nom de pièce, de personne ni de lieu de cette maison
n'apparaît dans le code, les tests (une maison fictive) ou ce texte.

Côté code, en plus de `check.sh` : la villa de démonstration lancée depuis cette branche,
regardée à 390 px (barre d'onglets, feuille « Plus », Système → Pages affichées) et à 1400 px
(menu avec les noms, Accueil), sans erreur JavaScript.

## Ordre de fusion

1. `contrib/pieces-depuis-le-tableau-de-bord` (pièces depuis l'onglet Pièces) ;
2. cette branche. Si la première change à la relecture, je rebase celle-ci dessus.

## Limites connues

- Le plan, les compteurs et les zones/téléphones sont lus **une fois** au chargement du
  tableau de bord : un plan ajouté ou un téléphone appairé fait revenir sa page au
  rechargement suivant. Les appareils, eux, sont suivis en direct.
- Les justifications sont des heuristiques simples (une télé = `deviceKind` `tv`, une
  production solaire = un nom de point) ; un cas non reconnu se règle par « Toujours ».
- Les pages qui arriveront par d'autres contributions (planning de chauffage, sonnette sans
  image…) devront s'ajouter à `PAGE_IDS`/`PAGES`, au menu et aux deux phrases
  `systeme.pages.oui|non.<id>` ; la doc l'explique. La 4ᵉ case de la barre d'onglets prend
  aujourd'hui la première page affichée après Pièces ; une page de chauffage pourra s'y
  réserver la place.
- Le menu d'ordinateur passe de 104 px (icônes) à 248 px (noms) : c'est voulu, mais visible.

## Checked

- [x] `sh scripts/check.sh` (fmt, clippy, tests, catalogue, leak check) — chaque étape lancée
      dans le cloud avec la toolchain stable à jour : vert
- [x] `npm run build` in `web/` and `node scripts/i18n-check.mjs` (et `npm test` : 24 tests)
- [x] New texts in `locales/fr` **and** `locales/en`
- [x] No personal data (names, addresses, MACs, serials, keys): fake values only
- [ ] Tried on real hardware? — pas une intégration : essayé sur une installation réelle
      (voir plus haut), aucun README d'intégration concerné

---
🤖 Generated with [Claude Code](https://claude.com/claude-code)
