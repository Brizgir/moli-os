# Le dashboard (`web/`)

Svelte 5 (runes) + Vite, **embarqué dans le binaire** (`rust-embed`, ~210 Ko JS / 66 Ko CSS,
compressé à l'envoi). Deux visages sur le même hub, routés par le hash (`web/src/App.svelte`) :

| Visage | Route | Pour qui |
|---|---|---|
| **Maison** | `#/…` (défaut) | la famille : un membre de la famille qui n'est pas technicien doit pouvoir s'en servir sans aide |
| **Atelier** | `#/atelier` | bricoler : tous les appareils, toutes les valeurs, journal, approbations, bench |

Le flux SSE `/api/events` est ouvert **une seule fois** et partagé (`web/src/lib/hub.svelte.js`).

## Maison : pages (`web/src/maison/pages/`)

| Page | Route | Contenu |
|---|---|---|
| Accueil | `#/` | **une liste de cartes que chacun dispose à sa main** (bouton Modifier, voir « Disposer l'accueil soi-même ») ; par défaut : date, salut, heure, météo ; alertes seulement quand il y a un souci (fumée, eau, portes, sonnette, piscine, approbations) ; lumières favorites, et dessous **Ambiance** : le choix de la pièce (pastilles, la dernière choisie est retenue sur l'appareil) et le bandeau d'ambiances ; électricité (jauge du jour vs hier, et à côté la jauge **en direct** du Linky sur l'abonnement, `[energy.live]` : sa couleur dit combien on tire, pas la distance à l'abonnement, verte au calme, orange dès `warn` (par exemple 2 kVA), rouge au double ; mot « calme / modérée / soutenue / forte ») ; clims (cadran **tactile** : on fait glisser le doigt sur l'anneau pour la consigne, le point gris = la température de la pièce, la perle = la consigne, visibles même clim éteinte ; une consigne glissée clim éteinte part tout de suite et la clim démarre dessus ; marche/arrêt, consigne ± 0,5 °C regroupée en un seul ordre, mode, dès que le profil sait écrire) ; sonnette (image + dernier passage réel, lu dans l'historique) ; télécommande ; dehors en bref **Mise à jour du 7 oct.** : une tuile de lumière peut en commander plusieurs (`favorites.lights[].also` : par exemple une pièce et son couloir) ; Électricité : les plus gros consommateurs du moment en légende ; Climatisation : coût du jour et du mois par clim (compteur de son circuit, sinon « coût non mesuré ») ; Dehors : l'humidité à la place de la couleur Tempo. |
| Moli | `#/moli` | l'assistant plein écran : la conversation à gauche, **l'écran se compose** à droite avec les cartes choisies par Moli (voir [moli.md](moli.md)) |
| Salon | `#/salon` | télécommande porcelaine (touches ou **pavé tactile** : glisser = flèches, toucher = OK ; clavier), « à l'écran », barre de son, Ambilight, lampes |
| Pièces | `#/pieces` | une carte par pièce : groupe Hue, lampes (curseur de luminosité ; lampe couleur : pastille qui ouvre une palette, trois blancs par température et huit couleurs + « autre couleur » ; en mode couleur l'ampoule de la tuile prend la teinte ; la tuile « Toute la pièce » a la même palette, appliquée à chaque lampe de la pièce qui sait la prendre) ; bouton **Ambiances** : un bandeau (Feu de cheminée, Bougies, Fin de journée, Cocooning, Fête, Océan, Forêt, Aurore boréale, Romantique, Cinéma, Lecture, Veilleuse), aussi sur la page Salon, volets, clims, prises, médias, capteurs en pastilles ; « Protégée » sur les chambres ; « Tout éteindre » **Depuis le 7 oct., cartes compactes** (`ui/RoomCard.svelte`) : en-tête = température de la pièce (moyenne de ses sondes et clims, jauge froid → chaud), ambiances (pastille), petit interrupteur de toutes ses lumières (jamais télé ni prises) ; lumières, télé, enceinte, prises en petites pastilles côte à côte, icônes parlantes (suspension, spot, lampe, ventilateur, prise…), ⚙ pour les réglages d'une lampe (luminosité, couleurs, ampoule par ampoule, ventilateur) ; une clim sur une ligne (le cadran au toucher) ; affichage « Aa » (noms) ou icônes seules, retenu par appareil (`pieces-look`). **Enceintes (17 oct.)** : un appui sur la pastille d'une enceinte (Sonos, Echo) ouvre sa carte sur place (`ui/SpeakerCard.svelte`, `inline`), comme le ⚙ d'une lampe : ce qui joue (titre, artiste), lecture/pause (`playing`), volume et sourdine si l'enceinte les a, et un champ « Mets… » (`play` : un genre, un artiste, une radio) / « Dis… » (`say`, 250 caractères) pour celles qui prennent des mots ; plusieurs enceintes par pièce, une carte à la fois. La pastille dit « En lecture », « En pause » (`state` `paused` d'Alexa ou `paused_playback` de Sonos, `speakerState`) ou « À l'arrêt ». Un groupe multi-pièces Alexa (« Partout ») est une enceinte sans volume (`speakerGroup`, icône à plusieurs enceintes) ; les enceintes et groupes sans pièce sont réunis dans une carte « Toute la maison » en bas de la page (`roamingSpeakers`). « Le son de la télé » n'est dit que par l'enceinte `salon.speaker` (la barre de son). |
| Lumières (D12) | partout | une lampe injoignable est « Coupée à l'interrupteur » (jamais comptée allumée, pas de halo ; un appui l'explique) ; une pièce Hue est « Toute la pièce », jamais une lampe ni une pastille du plan ; un **luminaire** (`lumieres:…`) remplace ses ampoules et son relais dans les vues de la famille, avec un bouton « ampoule par ampoule » (une couleur chacune) ; « Tout éteindre » une pièce = ses lampes et luminaires une à une, jamais la télé, l'enceinte ni les prises ; les ambiances visent les ampoules d'un luminaire. `home.svelte.js` : `isOn` (injoignable = éteint, groupe = une de ses lampes joignables), `isFixture`, `isGroup`, `fixtureOf`, `unpowered` |
| Plan | `#/plan` | **le plan vivant** : un onglet par étage. Étage **dessiné** (`plan/Drawing.svelte`) : la maison redessinée en SVG depuis la donnée, zoom molette / pincement / double tap / boutons, 4 habillages (Plan, Blueprint, Nuit, Aquarelle ; choix par appareil, défaut de la maison en modification), chaque lampe allumée projette un halo découpé par sa pièce, les pièces se teintent ; bouton **3D** (`plan/House3D.svelte` + `plan/scene3d.js`, three.js chargé à la demande) : étages empilés, murs coupés à 60 % sur l'étage regardé, meubles modélisés (`plan/furniture3d.js` : lits, canapés, chaises, voitures, trampoline, transat…), murs de biais et murets, toit de tuiles sur l'étage qui en a un (le regarder = la maison de dehors), vraies lumières, toucher une perle = l'appareil. En modification d'un étage dessiné, un calque à la fois répond au doigt (« Meubles et appareils », « Pièces », « Murs », « Portes et fenêtres ») : toucher choisit, glisser ce qui est choisi le déplace (glisser ailleurs fait toujours défiler le plan) ; **meubles** : ajouter / déplacer / tourner / redimensionner (lit, canapé, voiture…), « poser les appareils dans leur pièce » ; **pièces** : glisser les coins (poignées), le « + » au milieu d'un côté ajoute un coin, « Retirer ce coin », nom, type, pièce de Moli (celles de `home.json`, ou aucune), créer (rectangle de 3 × 3 m) et supprimer ; **murs** : ajouter (horizontal ou vertical), déplacer par la poignée du milieu, allonger ou raccourcir par les bouts, épaisseur, longueur et hauteur (vide = toute la hauteur, sinon muret) dans la fiche, supprimer (avec ses ouvertures) ; ses ouvertures le suivent ; **portes et fenêtres** (porte, fenêtre, baie, portail) : ajoutées dans le mur le plus proche, elles y glissent (et passent au mur voisin le plus proche), type et largeur dans la fiche. **Magnétisme** (activé par défaut) : grille de 5 cm, faces des murs et coins des pièces voisines (angles droits), à 12 px près à l'écran. **Défaire / Rétablir** (50 dernières actions, Ctrl+Z / Ctrl+Y, Suppr retire ce qui est choisi). La fiche de ce qui est choisi s'ouvre sous le plan, avec les **mesures en cm** à taper : X et Y depuis le coin en haut à gauche de l'étage (coin haut-gauche d'un mur ou d'un meuble, coin choisi d'une pièce), largeur et profondeur d'un meuble, épaisseur et longueur d'un mur (X et Y restent fixes), largeur d'une ouverture et sa distance depuis le début de son mur (bout gauche ou haut) ; la fiche d'un mur ajoute une porte ou une fenêtre dans ce mur. **Mesurer** : « Mesurer depuis… » puis toucher le repère (mur, porte, meuble ou pièce, quel que soit le calque) ; la fiche de tout ce qu'on touche ensuite donne l'écart en cm entre les faces, à droite / à gauche et au-dessus / en dessous du repère (négatif : ils se chevauchent), et le plan trace les cotes ; taper un écart déplace ce qui est choisi (pas le repère), ses ouvertures avec un mur, une ouverture restant dans son mur. C'est aussi la façon de placer un mur ou un meuble par rapport à un autre (`editing.js` : `footprint`, `gaps`, `withGap`). La **mesure de température** d'une pièce s'affiche à son pied par défaut ; dans le calque Pièces, un thermomètre sur la pastille se glisse ailleurs (ou X, Y dans la fiche, « Température au pied de la pièce » pour revenir) : `gauge` de la pièce. **Taille des appareils et des mesures** (Petits, Moyens, Grands) : un choix par écran, à côté de l'habillage, gardé sur l'appareil comme lui. Tout reste dans le brouillon jusqu'à « Enregistrer ». Logique pure et testée : `plan/editing.js` (`editing.test.js`). Étage **image** (ancien mode) : l'image du plan, une lumière allumée brille, une porte ouverte ou une présence se colore, une fumée ou une fuite clignote ; toucher = allumer/éteindre (lumières, prises), caméras et télé ouvrent leur page, une enceinte ouvre ses commandes sous le plan (comme dans sa pièce), le reste ouvre une fiche. « Modifier » : ajouter un étage avec son image (photo du plan acceptée), poser un appareil (le choisir dans la liste, toucher le plan), glisser pour déplacer ; enregistrer demande le code. Données : `data/plan.json`, images dans `data/plan/` (voir [api.md](api.md)). |
| Planning de chauffe | `#/chauffage`, `#/chauffage/<appareil maison>` | les plannings Netatmo (`pages/Chauffage.svelte`, logique pure et testée dans `lib/heating.js`) : en haut, ce qui passe avant le planning (consigne manuelle d'une pièce avec sa fin et « Revenir au planning », maison en absence ou hors-gel) ; les plannings (celui « en cours », « Utiliser ce planning », « Dupliquer pour essayer », nom) ; les **températures** : un réglage par couleur (Confort, Nuit, Éco, Confort +, d'après le type Netatmo) et sa température par pièce (± 0,5, 7 à 30), absence et hors-gel ; **la semaine** : 7 barres de 0 h à 24 h, glisser une limite (par quart d'heure, flèches au clavier), toucher une plage (réglage, début et fin ± 15 min), toucher un jour pour le recopier sur d'autres. « Enregistrer » / « Créer ce planning » ouvre la feuille du code (403 → code → même requête), puis montre ce que Netatmo garde. **Courbes** (`ui/HeatingCurves.svelte` et `ui/HeatingPlot.svelte`, SVG, logique pure et testée dans `lib/heating-curves.js`) : une carte par pièce chauffée (nom de la pièce Moli, même recherche que l'accueil : `heatedRooms()`), lue dans `/api/history` : température relevée (ligne), consigne (escalier, depuis `before` puis chaque changement), demande de chauffe (bande sous la courbe ; sur 7 ou 30 jours, un seau mêlé est pâli selon sa part de « chauffe ») ; 24 h, 7 jours ou 30 jours (24 h par défaut), un seul axe des températures pour toutes les pièces, au toucher ou au survol l'heure, la température, la consigne et la chauffe sur toutes les cartes à la fois, sinon le dernier relevé ; « Toutes les pièces » superpose les températures relevées, « Extérieur » ajoute en pointillé la température de la météo (`outdoor.weather` de `home.json`). Période, vue et extérieur sont gardés sur l'appareil ; relu toutes les 5 min. Accès : pastille « … · 19° visés » d'une pièce chauffée (Pièces), bouton de la carte Netatmo (Système). |
| Caméras | `#/cameras` | mur d'images (rafraîchies si visibles ; caméras sur batterie, dont Ring, au toucher seulement, l'heure de l'image telle que la caméra la donne, `image_at`) ; sous une caméra qui les dit, son dernier appui et son dernier mouvement ; plein écran avec les passages du jour (sonnette, personne, véhicule, animal, colis, mouvement) et, avec `[server] captures_days`, la photo que Moli a gardée de chacun et celles des 7 derniers jours (`/api/captures`) : une photo touchée s'affiche en grand à la place du direct ; les modes de surveillance Ring (`RingModes.svelte` : Désactivé, Domicile, Absent, le code demandé à l'appui) |
| Énergie | `#/energie` | en ce moment (Linky en direct coloré selon la conso, repère à `warn`, prochain changement heures creuses / pleines), aujourd'hui (au rythme d'hier), mois + projection vs mois dernier + part en heures creuses, Tempo ; **conseils du moment** (`lib/energy-advice.js` : porte ouverte clim allumée, grosse conso en heures pleines et qui tire, clim alors qu'il fait plus frais dehors, consignes, pompe de piscine en heures pleines, heures creuses en cours ou proches, talon (heure la plus calme de chaque jour, médiane sur 7 jours) en €/an) ; qui consomme maintenant ; où ça part (aujourd'hui / hier / ce mois, par circuit, %) ; **journée type** (moyenne heure par heure sur 7 jours, heures creuses **apprises** de l'historique du point PTEC, talon) ; graphe 24 h / 30 j / 12 mois. Mobile : une colonne, chiffres deux par deux **Journée type** : la journée en cours par-dessus (repère sur chaque heure passée, « aujourd'hui jusqu'à X h : … kWh, d'habitude … (± %) ») ; lumières estimées « ≈ estimé » dans « qui consomme » et « où ça part ». |
| Impression 3D | `#/impression` | une carte par imprimante (Klipper, Bambu) : état, vignette du travail, progression, couche, temps restant et heure de fin, buse / plateau / enceinte, têtes et filaments (couleurs ; AMS pour Bambu), lumière, **pause / reprise / annulation** (annuler demande une confirmation) ; totaux (impressions, heures, km de filament). Sur l'Accueil, une pastille tant qu'une impression tourne, est en pause ou en erreur ; dans Pièces, une pastille sur la carte de la pièce |
| Dehors | `#/dehors` | météo (soleil calculé), air/UV, piscine (pH, chlore, filtration), jardin, caméras extérieures |
| Automatismes | `#/automatismes`, `#/automatismes/<id>` | liste, idées, éditeur à canevas (voir [automatismes.md](automatismes.md)) |
| Système | `#/systeme` | accès à l'**Atelier**, **Pages affichées** (voir « Ce qui s'affiche selon la maison »), **carte vivante** du système et comparaison avec HA (voir plus bas) |

Navigation (refonte d'octobre 2026, maquette `docs/design/maquette-2026-10/`) : menu latéral
avec le nom de chaque page sur grand écran (≥ 1100 px), rail étroit icône + nom sur tablette,
et sur téléphone (< 768 px) une **barre d'onglets en bas** : Accueil, Pièces, Moli au centre,
Chauffage (ou la page suivante du menu s'il n'y en a pas), **Plus** (une feuille avec le reste
du menu, Système, Atelier et le thème ; cibles ≥ 44 px). Pied du menu : thème, Système.
L'Atelier s'ouvre depuis Système (sa première carte). Bulle « Demander à Moli » sur toutes les
pages sauf Moli (sur téléphone, Moli est au centre de la barre).

## Ce qui s'affiche selon la maison

Principe : **rien n'est supprimé, ce qui ne sert pas est masqué, et revient de lui-même** dès
qu'un appareil de ce type arrive. Le code des pages et des cartes reste ; seule leur condition
d'affichage change.

**Le menu.** Chaque page déclare ce qui la justifie (`web/src/maison/lib/pages.js`,
`justified`, fonction pure testée par `pages.test.js`, `npm test`), calculé depuis
`hub.devices` (sans les appareils `hidden`), `home.json`, `/api/plan`, `/api/energy`, les
zones et téléphones (`/api/people`) et la voix de Moli :

| Page | Apparaît quand… |
|---|---|
| Pièces, Automatismes, Demander à Moli | toujours |
| Chauffage | une maison Netatmo à plannings, ou une pièce chauffée (`setpoint` + `setpoint_mode`) |
| Salon | une télé (`deviceKind` `tv`) ou `home.json` `salon` |
| Plan | `data/plan.json` a au moins un étage |
| Carte | une zone ou un téléphone déclaré |
| Caméras | une caméra (`camera`), une caméra favorite, ou une sonnette / caméra Ring ; s'appelle **« Sonnette et caméras »** quand toutes les caméras sont des Ring (`camerasOnlyRing`) |
| Parler | un boîtier vocal (ESPHome, point `state` à `listening`) ou la voix de Moli (`/api/assistant` `speak`/`listen`) ; une Echo qui sait dire une phrase ne suffit pas |
| Énergie | `/api/energy` répond (section `[energy]`) |
| Solaire (lien de la page Énergie) | une production (point de puissance ou d'énergie `solar`, `pv_`, `production`) |
| Impression 3D | une imprimante 3D ou papier |
| Dehors | la météo (`outdoor.weather`, ou un point `weather_code`) |
| Infra | jamais d'elle-même : c'est l'affaire de l'Atelier |

Sans justification, la page sort du menu **mais sa route reste servie** (`#/energie` tapé
affiche la page avec son état vide). Réglage humain dans Système, carte **« Pages
affichées »** : trois états par page (Automatique / Toujours / Jamais), rangés dans
`data/home.json` :

```json
{ "pages": { "energie": "never", "infra": "always" } }
```

écrits par `PUT /api/home/pages` (`crates/moli-api/src/home.rs`, comme les pièces : session
humaine, code demandé au besoin, écriture atomique, `home_changed`). Le corps ne contient que
les pages à changer ; `auto` efface le choix ; une page ou un choix inconnu est refusé (422).
Les pages acceptées (`PAGES` côté Rust) sont celles de `PAGE_IDS` (`pages.js`), vérifié par un
test.

**Les cartes des pages gardées.** Une carte ne s'affiche que si sa donnée existe :

- Accueil : télécommande (une télé `salon.tv`), électricité (`/api/energy`), clims (les
  favorites `climate` qui existent et ne sont pas une pièce Netatmo), dehors en bref (qualité
  de l'air, UV ou piscine ; l'humidité s'y ajoute si elle est là), caméra de la sonnette (une
  caméra favorite). Lumières : les favorites, ou à défaut une tuile par pièce qui a des
  lumières (sa pièce Hue sinon ses lampes ensemble). Sans alerte, une bande verte
  **« Tout va bien : N services en marche, rien à surveiller »** (N : les pilotes dont un
  appareil répond) remplace la zone d'alertes.
- **Chauffage** (`ui/HeatingCard.svelte`, dès qu'une pièce chauffée existe) : chaque pièce
  avec sa température et sa consigne (« Consigne 18° », « … jusqu'à 14:00 » ou « … en
  permanence » si elle est manuelle, « Hors-gel », « Arrêté ») et le cadenas d'une pièce
  protégée ; en tête le **prochain changement** d'après les automatismes horaires actifs
  (approuvés, en marche, déclencheur `at_time` du jour, qui règlent un point du chauffage :
  `lib/today.js` `nextHeating`, sinon le premier de demain) ; boutons **« Voir le planning »**
  (`#/chauffage`) et **« Mode absence »** (point `mode` de la maison Netatmo : `away` ↔
  `schedule`).
- **Sonnette sans image** (`ui/DoorbellEvents.svelte`, Ring) : dernier appui et dernier
  mouvement lus dans l'historique (7 jours), batteries ; un espace réservé tient la place de
  l'image tant que le pilote Ring n'en donne pas. La page Caméras montre la même chose
  appareil par appareil.
- **Musique** (`ui/MusicCard.svelte`, dès qu'une enceinte existe) : celle qui joue (sinon en
  pause, sinon la première), lecture/pause, le nombre d'enceintes et de groupes.
- **Aujourd'hui** (`ui/Today.svelte`) : les automatismes horaires du jour, cochés une fois
  passés ; aucun : pas de carte. Les automatismes sont lus une fois toutes les deux minutes
  pour toutes les cartes (`lib/timed.svelte.js`).
- Dehors : la carte du ciel seulement avec la météo, l'air seulement s'il est mesuré, la
  piscine seulement si elle est déclarée.
- Pièces : en tête de carte, « 20,2° → 18° » (température, et consigne d'une pièce Netatmo),
  « sans thermomètre » dans la ligne d'état d'une pièce qui n'en a pas ; une pièce protégée
  dit « Protégée : un automatisme ou Moli y attend ton code » ; une carte en pointillés
  **« Nouvelle pièce »** en fin de liste ouvre la fiche (`ui/RoomForm.svelte`).

## Disposer l'accueil soi-même : `data/dashboards.json`

**Pour la famille.** Sur l'accueil, le bouton **Modifier** (crayon, en haut à droite) ouvre
le mode de modification sur la page même, au doigt comme à la souris. Chaque carte montre
alors sa poignée : la prendre pour la déplacer (ou les flèches ▲▼), choisir sa largeur
**S / M / L** (un tiers, deux tiers, toute la rangée ; sur téléphone une colonne, la taille
règle ce que la carte dit), l'œil la **masque** (elle reste en bas, un appui la remet), la
roue ouvre ses **réglages** : titre, taille, ce qu'elle affiche (par exemple les pièces de la
carte Chauffage et leur ordre, les tuiles de lumières, la caméra de la sonnette, les faits de
Dehors), **Dupliquer**, **Retirer**. **Ajouter** ouvre le catalogue de ce que la maison
expose : ses pièces, ses enceintes, diffuseurs, caméras, imprimantes, et les cartes de la
maison (présence, assistant, électricité, état de la machine…) ; ce qui est déjà là le dit.
**Défaire** annule la dernière action, **Par défaut** remet la page de Moli dans le
brouillon, rien n'est enregistré avant **Terminé** (le code est demandé une fois) ;
**Abandonner** jette le brouillon. Deux dispositions par appareil, **grand écran** (≥ 768 px)
et **téléphone**, choisies par la largeur et modifiées chacune de leur côté ; des
**dispositions nommées** (« Matin », « Soir », « Absent »…, douze au plus) se créent dans la
barre et se choisissent à la main (le choix est celui de l'appareil, comme le thème). « Oublier
tout ce qui est enregistré » revient à la page de Moli. Une installation sans disposition
enregistrée affiche la page telle qu'elle a toujours été.

**Qui.** La disposition **de la maison** est celle du propriétaire (même règle que les
personnes : rôle `owner`, propriétaire Access, ou le code depuis le foyer tant qu'aucun
propriétaire ne peut se connecter). Une **personne connectée** (mot de passe, Access) qui a
donné le code garde **la sienne**, qui remplace celle de la maison pour elle ; un membre part
d'une copie de ce qu'il voit, le propriétaire choisit dans la barre « la mienne » ou « celle
de la maison ». Un invité voit le défaut, sans bouton (la garde lui refuse `/api/dashboard`). La disposition ne donne aucun droit :
une pièce protégée reste gardée carte par carte.

**Comment.** `pages/Accueil.svelte` est une liste de cartes posée sur une grille de trois
colonnes qui comble les trous (`grid-auto-flow: dense`) : une carte a une largeur, jamais une
position, donc rien ne se chevauche et rien à calculer. Chaque carte garde sa condition
d'affichage (pas de télé : pas de télécommande), si bien que la liste par défaut, calculée
par `lib/layout.js` (`defaultLayout`, pure, testée par `layout.test.js`), redonne la page
d'avant. `accueil/cards.js` dit ce qu'est chaque type de carte (`CARD_TYPES` : nom, icône,
réglages et leurs choix lus dans la maison), `accueil/Card.svelte` monte le composant
(`SkyHero`, `HeatingCard`, `MusicCard`… et les nouveaux : pièce, enceinte, diffuseur, caméra,
présence, assistant, état de la machine, imprimante). L'état et les appels sont dans
`lib/dashboard.svelte.js` ; le brouillon y porte son historique (50 pas). Serveur :
`crates/moli-api/src/dashboard.rs` (`GET /api/dashboard`, `PUT|DELETE /api/dashboard/house`
et `/me`, voir [api.md](api.md)), fichier `data/dashboards.json` versionné (`version`, migré
à la lecture ; un fichier plus récent est lu tel quel, une carte d'un type inconnu est gardée
sans être affichée), écriture atomique, `dashboard_changed` sur le flux. Le fichier :

```json
{ "version": 1,
  "house":  { "layouts": { "default": { "desktop": { "cards": [ { "id": "k7f2", "type": "heating", "size": "l", "options": { "only": ["Cuisine", "Bureau"] } } ] }, "phone": { "cards": [] } } } },
  "people": { "<id>": { "layouts": { "default": …, "soir": { "name": "Soir", "desktop": …, "phone": … } } } } }
```

## La mise en page de la maison : `data/home.json`

Servie telle quelle par `/api/home` depuis le dossier de données (`data/home.json`, écrite à
la main ou par un agent pour chaque maison ; les **pièces** s'y déclarent aussi depuis l'onglet
Pièces, voir plus bas). Sans elle, le dashboard reste utilisable (pièces telles que les
appareils les nomment).

```json
{
  "title": "Maison",
  "rooms": [{ "name": "Chambre des enfants", "icon": "bed-outline", "aliases": ["Chambre 2"] }],
  "favorites": {
    "lights":  [{ "id": "hue:…", "name": "Salon", "icon": "sofa-outline" }],
    "climate": [{ "id": "clim-salon:…", "name": "Salon" }],
    "cameras": [{ "id": "reolink:…", "name": "Sonnette", "refresh": 10 }, { "id": "reolink:…", "name": "Allée", "refresh": 60, "battery": true }]
  },
  "salon":   { "tv": "tele:…", "speaker": "sonos:…", "room_light": "hue:…", "lamps": ["hue:…"] },
  "doors":   [{ "id": "tuya:…", "name": "Entrée" }],
  "outdoor": { "weather": "meteo:…", "air": "air:…", "tempo": "tempo:…", "pool": "piscine:…", "pool_pump": "tuya:…", "garden_lights": [{ "id": "tuya:…", "name": "Lumière du jardin" }] },
  "safety":  ["z2m:…"],
  "hidden":  ["tuya:…", "compteur:…"],
  "quiet":   ["tuya:…"],
  "outlets": { "tuya:…": { "switch_1": "Prise 1", "switch_2": "Prise 2", "switch_3": "Prise 3" } }
}
```

- `rooms[].aliases` : les noms que HA, Hue ou Tuya donnent à la même pièce (le dashboard et
  Moli les ramènent au nom canonique).
- **Pièces depuis l'onglet Pièces (17 oct.)** : le bouton « Nouvelle pièce » ouvre une fiche
  (`ui/RoomForm.svelte`) : le nom, l'icône (choix parmi `ROOM_ICONS` de `lib/home.svelte.js`,
  la même liste que `crates/moli-api/src/home.rs`), ses autres noms. Enregistrer demande le
  code de la maison une fois (`asHuman`) puis `POST /api/home/rooms` ; le serveur réécrit
  `rooms[]` de `home.json` sans toucher au reste, et prévient les tableaux de bord ouverts
  (`home_changed` sur le flux d'événements : `home.config` est relu). Le ⚙ d'une carte ouvre la
  même fiche pour **renommer** la pièce (les étiquettes `room` de ses appareils suivent,
  `labels.toml` réécrit ; l'ancien nom devient un alias, pour que ce que Hue ou Alexa appellent
  ainsi y reste), changer l'icône, ajouter un alias, ou la retirer de la liste (ses appareils
  gardent leur pièce). Une pièce déclarée **sans appareil** s'affiche quand même (`roomList` :
  `declared`, `empty`) avec « Aucun appareil ici pour l'instant » et un lien vers l'Atelier
  pour y ranger quelque chose. Protéger une pièce reste dans `moli.toml` (`[guard]
  protected_rooms`, lu au démarrage) : la fiche le dit, ne le propose pas.
- `hidden` : masqué du dashboard **et** de l'inventaire de la conversation (pas de celui
  qui sert à créer les automatismes). Par exemple un compteur EM06 (sinon Moli montre une fiche
  « em06p » brute).
- `quiet` : rangé dans sa pièce mais **jamais un bouton** (lave-vaisselle, machine à laver,
  recharge de la voiture : un appui de travers coupe un cycle). La carte de la pièce dit ce qu'il
  consomme (« Lave-vaisselle · 1 850 W », dès 3 W) ; débranché, il ne dit rien (sauf dans une
  vue filtrée). Moli et les automatismes peuvent toujours le commander.
- `outlets` : une multiprise dont chaque prise a son bouton, dans l'ordre donné, plus un bouton
  pour toute la multiprise (« 2 sur 3 » ; un appui allume tout, ou éteint tout si tout est
  allumé). Seuls les points inscriptibles comptent.

## Conception visuelle

- Thème **porcelaine** le jour, **nuit** après le coucher du soleil (point `daylight` de la
  météo) ; bascule Auto / Jour / Nuit (mémorisée dans le navigateur).
- Ambre = allumé, bleu = climat/froid, corail = attention. Grandes cibles tactiles, peu de mots,
  aucun jargon. Police Figtree Variable, icônes Material Design (sélection dans
  `lib/icons.js` : seules celles-ci sont dans le paquet).
- Jetons CSS sous `.maison` dans `maison.css` (l'Atelier garde son propre style).
- Règle de style : **pas de bordure gauche colorée** sur les cartes.
- Une lampe injoignable n'est jamais affichée allumée (sa dernière valeur est une supposition).
- Les ordres retenus par la garde ouvrent la feuille « C'est moi » (code PIN), qui sert aussi
  à valider, lancer ou supprimer un automatisme.

## Bibliothèques partagées (`web/src/maison/lib/`)

- `home.svelte.js` : état de la maison (config, ordres retenus, toasts, thème), lecture des
  valeurs, `powerKey` (le point qui allume/éteint), `deviceKind`, `roomList`, `act` (envoie
  un ordre, gère le 202 de la garde), `confirmHeld` (code PIN), formats français.
- `moli.svelte.js` : la conversation (partagée bulle/page), voix (enregistrement si contexte
  sécurisé, lecture des réponses à voix haute), approbations depuis la conversation,
  suggestions selon l'heure et la maison.
- `auto.svelte.js` : catalogue des nœuds, textes des nœuds, mise en page automatique, API des
  automatismes (avec empreintes), `asHuman`.
- `energy-live.svelte.js` : résumé énergie partagé (une seule requête par minute quel que soit
  le nombre de cartes).

## La carte vivante « Comment ça marche » (`pages/Systeme.svelte`)

Construite **uniquement** à partir de `/api/system` et du flux d'événements : pilotes (état,
appareils, injoignables) → cœur (appareils, valeurs, changements par minute) → briques
(actives ou non, chiffres clés) → surfaces. Les liens d'un pilote s'allument quand ses
appareils changent. En tête : Moli contre HA (mémoire, processeur, taille, réponse, budget,
tendance des livraisons) lu dans `data/perf.jsonl`. Un nouveau pilote ou une nouvelle brique y
apparaît sans toucher à la page.

## Vérifier visuellement

Pour un agent, plutôt que son navigateur intégré (chaque clic y demande une autorisation) :
`puppeteer-core` sur le Chrome installé, avec de petits scripts de capture écrits pour la
session ; `chrome --screenshot` ne rend jamais la main à cause du flux SSE.

## Limites connues

- **Voix** : le micro exige HTTPS ; en `http://<hôte>:8790` le bouton est grisé.
- Télé : applis et clavier non disponibles (JointSpace ne le permet pas ; protocole
  Android TV Remote v2 à faire).
- Clims : lecture seule tant que les écritures Daikin n'ont pas été testées physiquement.
- Enceintes : un groupe Alexa multi-pièces (« Partout ») n'a pas de pièce ; la garde ne retient
  donc pas une commande sur lui, alors qu'il joue aussi sur l'Echo d'une pièce protégée. À régler
  côté serveur (pilote ou garde), pas dans le tableau de bord.
