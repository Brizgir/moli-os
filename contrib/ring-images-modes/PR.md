Titre proposé : **Ring : images des caméras à la demande et mode de surveillance Désactivé / Domicile / Absent**

Branche : `Brizgir/moli-os:contrib/ring-images-modes` → `rudy-io/moli-os:main`, **après** la
PR « Ring : pilote natif » (`contrib/ring`), sur laquelle elle est construite. Issue : #… (à
ouvrir avec `contrib/ring-images-modes/ISSUE.md`).

> Tant que la PR du pilote n'est pas fusionnée, le diff de celle-ci sur GitHub contient aussi
> ses deux commits : seuls les deux derniers commits sont propres à cette PR
> (`contrib/ring..contrib/ring-images-modes`).

## What and why

*In English, in short: on top of the read-only Ring driver, each Ring doorbell and camera now
gives its last image to the hub (`SnapshotSource`), fetched from Ring only when a dashboard
shows it, one at a time per camera, at most once a minute on battery, and shown with the time
the camera took it (`image_at`), never as fresh when Ring gives an old one back. Each Ring
location becomes a « Ring watch » device whose `mode` (disarmed, home, away) is the only thing
Moli writes to Ring. That point is `Semantic::Control`: the guard holds any order that is not
from `ui` (a person with the code), `automation` or `system` — the dashboard without a code,
the assistant, MCP, the API and the CLI get an approval request. Three buttons on the Cameras
page and on the home page. Verified on a real installation: the doorbell's image, and a mode
change from Moli seen in the Ring app.*

### Ce que ça change, et pourquoi

Deux commits :

1. **Pilote** (`crates/moli-ring` : `images.rs`, `api.rs`, `model.rs`, `lib.rs`, `net.rs` ;
   `moli-runtime` : `SecretKeeper` ; fixtures `snapshots-timestamps.json`,
   `mode-location.json` ; README, onboarding, `pilotes.md`, `api.md` ; textes `pilotes` et
   `serveur` en `fr`/`en`).
   - Images : `PUT snapshots/update`, puis `POST snapshots/timestamps` toutes les 2 s (5 fois au
     plus) jusqu'à un horodatage plus récent que l'image gardée, puis `GET
     snapshots/image/<id>`. 404/410 : rien de disponible, l'image gardée est rendue. Même
     horodatage : pas de nouvelle lecture.
   - Modes : un appareil `ring:location-<lieu>` par lieu, points `mode` (enum
     `disarmed|home|away`, `Semantic::Control`) et `mode_since`. Relu toutes les 30 s et après
     chaque ordre ; l'ordre échoue si Ring ne montre pas le mode demandé. `readOnly` (alarme
     Ring) : lu, refus d'écrire.
   - Tests : pilote complet rejoué sur un faux transport
     (`an_image_is_asked_seldom_and_never_told_newer_than_it_is`,
     `the_mode_is_a_person_s_call_and_read_again_after_the_order` : refus `assistant`, `mcp`,
     `household`, `api`, accepté `ui` et `automation`, relecture après ordre, changement vu
     depuis l'appli) ; test de la garde du hub étendu à `mcp`, `household`, `api`.
2. **Tableau de bord** (`RingModes.svelte` nouveau ; `Cameras.svelte`, `Accueil.svelte`,
   `CameraTile.svelte` ; `dashboard.md` ; textes `salon` en `fr`/`en`).
   - `RingModes` : trois boutons, le mode actuel en évidence et depuis quand. L'appui passe par
     `act` : sans session, la garde répond 202 et la feuille du code s'ouvre.
   - Page Caméras : les boutons sous le panneau Sécurité ; une caméra avec un point `battery`
     n'est demandée qu'au toucher.
   - Accueil : les boutons dans la carte de la porte ; sans caméra favorite, cette carte prend
     une sonnette qui donne une image (Ring) ; sans sonnette, les boutons seuls.
   - `CameraTile` : l'heure de l'image (`image_at`) plutôt que l'heure de lecture.

Interface : tout est fait sur le tableau de bord de `main` (aucune dépendance à une autre
refonte de l'interface).

### Sécurité (à relire en priorité)

- **La garde du mode** : `Semantic::Control` existe déjà sur `main` (pause / annulation d'une
  impression) et la règle ne change pas : un ordre qui ne vient pas de `ui`, `automation` ou
  `system` est retenu pour un humain **quelle que soit la pièce**. Pour le mode Ring, cela veut
  dire que **le tableau de bord sans code (`household`), l'assistant de Moli, un agent MCP, un
  script sur l'API ou la CLI ne peuvent ni armer ni désarmer** : ils obtiennent une demande
  d'approbation (202). Un automatisme validé par une personne le peut (c'est son intention).
  Seul le texte de la raison change (`serveur.garde.machine`).
- **Seul le mode s'écrit** chez Ring ; tout autre point Ring reste en lecture
  (`pilotes.ring.lecture_seule`). Un lieu avec une alarme Ring n'est jamais écrit.
- **Pas d'image en boucle** : à la demande d'une surface seulement, une par caméra à la fois,
  au plus une par minute sur batterie ; pas de vidéo en direct. Les caméras d'une pièce
  protégée restent réservées à une personne, comme toute caméra.
- Le jeton renouvelé pendant une demande d'image est rangé au coffre par `SecretKeeper` avant
  d'être utilisé, comme dans la boucle du pilote.
- **Interface non officielle** : les adresses des instantanés et des modes (dont l'hôte de
  l'appli, `prd-api-us.prd.rings.solutions`, `app.ring.com` autrefois) viennent de
  ring-client-api et python-ring-doorbell ; elles peuvent changer sans préavis.

### Guide pas à pas (essai)

Le pilote Ring est installé et connecté (guide de la PR précédente : jeton fait avec
`docker run --rm -it node:22-alpine npx -y -p ring-client-api ring-auth-cli`, collé dans
Système, carte Ring).

1. **Vérifier les appareils** : en plus de la sonnette et des caméras, un appareil
   « Surveillance Ring » par lieu, avec `mode` et `mode_since` remplis, le même mode que l'appli
   Ring.
2. **Activer les instantanés** dans l'appli Ring pour chaque caméra qui doit donner une image
   (réglages de l'appareil, « Instantanés » / « Snapshot Capture ») : activés par défaut sur une
   sonnette, souvent coupés sur une Stick Up Cam Mini. Sans eux, Ring n'a aucune image à rendre
   et la tuile reste « Image indisponible ».
3. **Voir une image** : onglet Caméras. Chaque sonnette et caméra Ring a sa tuile (« Batterie ·
   touche pour voir »). Toucher : une image arrive en quelques secondes, avec son heure. Toucher
   de nouveau dans la minute : l'image gardée revient, pas de nouvelle demande à Ring (la
   batterie est ménagée). Sur l'accueil, sans caméra favorite, la sonnette prend la carte de la
   porte.
4. **Changer le mode, avec la personne présente** : sur la page Caméras (ou l'accueil), toucher
   « Désactivé » alors que le mode est « Domicile ». Sans session, la feuille du code s'ouvre ;
   après le code, le bouton passe en évidence. Vérifier dans l'appli Ring que le mode a changé,
   puis **remettre le mode d'origine**. Jamais cet essai sans la personne : armer ou désarmer la
   surveillance d'une maison habitée est son affaire.
5. **Changer le mode dans l'appli Ring** : Moli le montre en moins d'une minute.
6. **Vérifier la garde** (facultatif) : un ordre `mode` par MCP ou par l'API (sans
   `x-moli-origin: ui`) reçoit 202 et une demande apparaît dans les approbations ; rien ne part
   vers Ring tant qu'une personne ne l'a pas validée.

### Vérifié sur une installation réelle

Octobre 2026, une sonnette Ring sur batterie et deux Stick Up Cam Mini sur un même lieu : image
de la sonnette obtenue dans Moli ; changement de mode depuis Moli vu dans l'appli Ring. Les Stick
Up Cam Mini ne donnent une image qu'avec les « Instantanés » activés dans l'appli Ring. Pas
encore vu : un lieu avec une alarme Ring (`readOnly`), plusieurs lieux.

### Ordre de fusion

1. « Ring : pilote natif des sonnettes et caméras » (`contrib/ring`).
2. Cette PR (`contrib/ring-images-modes`). Une fois la première fusionnée, son diff ne montre
   plus que ses deux commits.

### Limites connues

- Interface non officielle de Ring (voir plus haut).
- Pas de vidéo en direct ; une image prend quelques secondes, au plus une par minute sur
  batterie, aucune pendant qu'une caméra enregistre (l'ancienne reste, avec son heure).
- Une caméra sans « Instantanés » activés dans l'appli Ring n'a pas d'image.
- L'hôte des modes est celui des comptes actuels de l'appli ; un compte rattaché à une autre
  région n'a pas été vu.
- Volume du carillon, sirène, lumière : toujours en lecture (suites possibles).
- Même remarque que la PR précédente sur `clippy` 1.97 et `main` (`float_cmp`,
  `unknown_lints`), hors de cette PR.

## Checked

- [x] `sh scripts/check.sh` (fmt, clippy, tests, catalogue, leak check) — chaque étape lancée ;
  clippy propre avec `-A unknown_lints -A clippy::float_cmp` (dette de toolchain de `main`) ;
  tous les tests verts ; `catalogue check` : `ring` reconnu avec ses 8 fixtures ; leak-check
  propre
- [x] `npm run build` in `web/` and `node scripts/i18n-check.mjs` (2 810 clés, aucun texte en dur)
- [x] New texts in `locales/fr` **and** `locales/en`
- [x] No personal data (names, addresses, MACs, serials, keys): fake values only
- [x] Tried on real hardware? Ring battery doorbell and two Stick Up Cam Mini, October 2026
  (in `docs/components/pilotes.md` and `integrations/ring/README.md`)
