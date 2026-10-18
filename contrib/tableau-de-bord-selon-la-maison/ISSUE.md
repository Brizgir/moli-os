# Le tableau de bord n'affiche que ce que la maison possède

*In English: the family dashboard shows every page and every card to every house, so a home
without a TV, a meter or a printer still gets « Living room », « Energy » and « 3D printing »
in its menu, and cards full of « — » on the home page. This proposes that each page declares
what justifies it (a pure, tested function), that pages and cards without their data are
**hidden, never removed** (the route still answers, and the page comes back by itself when a
device of that kind arrives), a « Pages shown » setting in System (automatic / always / never
per page, kept in `home.json`), and a bottom tab bar on phones. A house that has everything
sees the same pages as today.*

## Le besoin

Le menu du tableau de bord famille liste toutes les pages, quelle que soit la maison : Salon
sans télé, Énergie sans compteur, Impression sans imprimante, Plan sans plan, Carte sans zone
ni téléphone. L'Accueil fait de même avec ses cartes (télécommande, électricité, « Dehors »
avec des tirets). Pour une famille, c'est du bruit : des pages vides, un menu qui défile sur
téléphone, et l'impression que quelque chose manque.

On ne veut pas pour autant **supprimer** ces pages : une maison change (une télé arrive, un
compteur est posé), et une personne peut vouloir garder une page à portée même vide.

## La proposition

1. **Chaque page déclare ce qui la justifie** : `web/src/maison/lib/pages.js`, fonction pure
   `justified(facts)` calculée depuis les appareils (sans les `hidden`), `home.json`,
   `/api/plan`, `/api/energy`, les zones et téléphones, et la voix de Moli. Par exemple :
   Salon = une télé ou `home.json` `salon` ; Énergie = `/api/energy` répond ; Impression =
   une imprimante ; Plan = au moins un étage ; Carte = une zone ou un téléphone ; Parler = un
   boîtier vocal ou la voix de Moli ; Dehors = la météo. Pièces, Automatismes et Moli sont
   toujours là ; Infra n'apparaît pas d'elle-même (c'est l'affaire de l'Atelier).
2. **Masquer, jamais supprimer** : une page sans justification sort du menu, mais sa route
   reste servie (`#/energie` tapé ouvre la page, avec son état vide). Dès qu'un appareil de
   ce type arrive, la page revient d'elle-même.
3. **Un réglage humain**, Système → « Pages affichées » : pour chaque page, Automatique /
   Toujours / Jamais, avec la raison du choix automatique (« Masquée : pas de télé »). Rangé
   dans `home.json` (`"pages": { "energie": "never" }`) par `PUT /api/home/pages`, session
   humaine seulement.
4. **Les cartes suivent la même règle** : sur l'Accueil, télécommande, électricité,
   chauffage/clims et « Dehors » seulement avec leur donnée ; sans alerte, une bande
   « Tout va bien : N services en marche » ; une carte « Aujourd'hui » liste les automatismes
   horaires du jour. Dehors : le ciel et l'air seulement s'ils existent.
5. **Téléphone : une barre d'onglets en bas** (Accueil, Pièces, Moli au centre, la page
   suivante du menu, « Plus » pour le reste, Système et l'Atelier) au lieu du rail qui
   défile ; tablette : rail étroit ; ordinateur : menu avec le nom de chaque page.

## Choix d'architecture

- **Une fonction pure, testée** (`node --test`, ajoutée à `npm test`) : facile à relire, à
  étendre et à faire évoluer sans toucher au rendu. Les pages acceptées par le serveur
  (`PAGES` de `home.rs`) sont celles du tableau de bord (`PAGE_IDS`), vérifié par un test
  Rust qui lit `pages.js`.
- **Le réglage vit dans `home.json`**, à côté des pièces et des favoris, écrit par le même
  module que la contribution « pièces » (écriture atomique, le reste du fichier intact,
  `home_changed` pour les autres écrans). Pas de nouvelle configuration côté serveur.
- **Générique** : aucune page n'est câblée sur une marque. Une nouvelle page s'ajoute en trois
  lignes (`PAGE_IDS` + sa justification, `PAGES` côté Rust, son entrée de menu) et deux
  phrases traduites. Les pages qui arriveront par d'autres contributions (un planning de
  chauffage, par exemple) s'y brancheront de la même façon ; cette proposition n'en dépend
  pas.
- **Rien de nouveau côté pilotes ni côté moteur** ; deux lectures de plus au chargement du
  tableau de bord (`/api/plan`, `/api/energy`), une fois.

## Ce qui ne change pas pour les autres maisons

- Une maison qui a tout (télé, compteur, imprimante, plan, zones…) voit **les mêmes pages**
  qu'aujourd'hui, dans le même ordre (Moli passe en fin de menu sur ordinateur, au centre de
  la barre sur téléphone).
- Toutes les routes restent servies ; aucun lien existant ne casse.
- Deux changements visibles par défaut, réversibles depuis « Pages affichées » : **Infra**
  sort du menu (elle reste dans l'Atelier et à son adresse) et l'**Atelier** s'ouvre depuis
  Système (première carte) et depuis « Plus » sur téléphone, plus depuis le pied du menu.
- Sans `home.json`, tout est automatique.

Cette proposition s'appuie sur celle des pièces créées depuis le tableau de bord (même module
`home.rs`, même fiche), à fusionner d'abord.
