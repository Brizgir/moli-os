Titre proposé : **Ring : pilote natif des sonnettes et caméras, en lecture, connexion par jeton collé**

Branche : `Brizgir/moli-os:contrib/ring` → `rudy-io/moli-os:main`. Issue : #… (à ouvrir d'abord
avec `contrib/ring/ISSUE.md`).

## What and why

*In English, in short: a native Ring driver (`crates/moli-ring`, `integrations/ring/`), read
only: doorbell presses, motion, battery, Wi-Fi, chime volume, through the Ring app's unofficial
cloud API (the only way: Ring devices open no port on the home network). Sign-in without the
password ever going through Moli: the person makes a token on their own computer with Ring's
client (`ring-auth-cli`) and pastes it in the dashboard (System, Ring card). A small core
addition, `Semantic::Secret`, makes that sign-in a person's only (origin `ui`), refused with 403
to any agent, never held for approval, `null` in the journal, never kept as state. Verified on a
real installation (a battery doorbell and two Stick Up Cam Mini): presses seen within about ten
seconds, motion seen. Nothing changes for homes without a `ring` driver.*

### Ce que ça change, et pourquoi

Un appareil Ring ne répond à rien sur le réseau de la maison ; Moli ne savait donc pas qu'on
sonne. Cette PR ajoute un pilote natif, en lecture seule, sur l'interface de l'appli Ring (non
officielle, la même que python-ring-doorbell et ring-client-api) :

| Appareil | Point | Sens |
|---|---|---|
| sonnette | `doorbell` | vrai 10 s après un appui |
| sonnette, caméra | `motion` | vrai 60 s après le dernier mouvement |
| sonnette, caméra | `battery` | % |
| tous | `wifi_signal` | dBm |
| carillon | `volume` | 0 à 11, lecture |

Deux commits :

1. **Cœur : la sémantique `secret`** (`moli-core/src/spec.rs`, `moli-runtime/src/hub.rs`,
   `moli-api/src/rest.rs`, `docs/components/coeur.md`). Un point `Semantic::Secret` n'est
   accepté que de l'origine `ui` ; les autres reçoivent `CommandError::HumanOnly` (403). Jamais
   retenu pour approbation (la demande garderait la valeur), `null` au journal, jamais gardé
   comme état. Test : `a_sign_in_is_a_persons_and_never_journaled`.
2. **Le pilote** (`crates/moli-ring`, `integrations/ring/`, carte `RingAccount.svelte` dans
   Système, `docs/components/pilotes.md`, textes `fr`/`en`, `*.real.json` dans `.gitignore`).

### Sécurité (ce qui mérite la relecture la plus attentive)

- **Le mot de passe Ring ne passe jamais par Moli.** Ring n'a pas de page de connexion pour une
  autre application. La première version de ce pilote (jamais publiée) demandait e-mail et mot de
  passe dans le tableau de bord ; une revue de sécurité interne l'a jugé bloquant, et le flux a
  été remplacé par un **jeton collé** : la personne fait le jeton chez elle avec le client de
  Ring (`ring-auth-cli`), qui lui demande e-mail, mot de passe et code de vérification ; Moli ne
  voit que le jeton.
- Le jeton collé est vérifié **une fois** (`grant_type=refresh_token`) ; Ring le remplace
  aussitôt et seul le jeton rendu va au coffre, **écrit avant d'être utilisé** (s'il ne peut pas
  être écrit, aucun appel ne part avec ; test `a_token_that_cannot_be_kept_is_not_used`). Sans
  coffre, rien n'est envoyé à Ring. Un texte qui n'a pas la forme d'un jeton (vide, deux mots,
  non ASCII, trop long, balises) n'est jamais envoyé (test
  `what_is_not_a_token_never_reaches_ring_and_a_refused_one_says_so`). Aucune requête ne
  contient `password` ni `username` (test `only_a_refresh_token_ever_goes_to_ring`).
- `refresh_token` et `hardware_id` ne vivent qu'au coffre et en mémoire (`Debug` caviardé) ;
  `moli.toml` refuse toute option inconnue.
- Hôtes fixes (`oauth.ring.com`, `api.ring.com`), TLS vérifié par `moli-net`, aucune URL tirée
  d'une réponse, **aucune dépendance nouvelle** (`Cargo.lock` : seulement `moli-ring`).
- Sûreté : un événement est dit une fois, s'il a moins de 90 s, jamais à la première lecture
  après un démarrage ; Ring injoignable → `doorbell`/`motion` à faux et appareils indisponibles
  (sinon un « on sonne » pouvait rester figé pendant tout le recul exponentiel) ; 429 → pause de
  10 min ; un 401 ne renouvelle qu'une fois, un jeton révoqué renvoie à la connexion sans
  boucle.
- Résumé de la revue interne (pas de fichier de revue dans la PR) : **R1** bloquant, le mot de
  passe transitait par Moli → corrigé (jeton collé). **R2** « on sonne » figé quand Ring tombe →
  corrigé. **R3** amélioration non appliquée : un échec HTTP cite au journal (niveau `warn`) le
  chemin de la requête, donc l'identifiant numérique de l'appareil ou l'UUID du lieu Ring, et
  200 octets de la réponse ; aucun jeton (il est dans l'en-tête, jamais cité). **R4** captures
  réelles : `*.real.json` ignoré, onboarding qui impose l'anonymisation. Vérifié aussi : aucune
  route nouvelle dans `moli-api`, la connexion passe par `POST /api/command` (l'origine `ui`
  exige une session humaine liée à l'appelant ; l'extérieur est refusé avant la route).

### Guide pas à pas (installation et essai)

Il faut une instance Moli avec son coffre (`MOLI_MASTER_KEY` dans l'environnement) et le code
du tableau de bord.

1. **Déclarer le pilote** dans `moli.toml`, puis `moli-os check-config` et redémarrer :

   ```toml
   [[driver]]
   id = "ring"
   kind = "ring"
   # [driver.options]
   # poll = 10   # secondes entre deux lectures des événements (5 au moins)
   ```

   Dans Système apparaît la carte « Sonnette et carillon Ring », qui attend un jeton.

2. **Faire le jeton, sur son ordinateur, sans rien installer** (Docker suffit) :

   ```sh
   docker run --rm -it node:22-alpine npx -y -p ring-client-api ring-auth-cli
   ```

   (ou `npx -p ring-client-api ring-auth-cli` si Node.js est installé). Ring y demande l'e-mail
   du compte, le mot de passe, puis le code reçu par SMS, e-mail ou appli. La commande affiche à
   la fin un jeton sur une longue ligne. Le conteneur est jeté à la sortie (`--rm`) : il ne
   garde rien. **Ne jamais coller ce jeton ailleurs que dans la carte Ring** (ni conversation,
   ni fichier, ni agent).

3. **Le coller dans Moli** : tableau de bord, Système, carte Ring, champ « Jeton affiché par
   ring-auth-cli », « Connecter ». Le code de la maison est demandé si la session n'en a pas
   (c'est voulu : une connexion de compte est l'affaire d'une personne). La carte passe à
   « Compte Ring connecté : N appareils lus par Moli ». Un jeton déjà utilisé ou périmé est
   refusé avec un message clair : en refaire un (étape 2).

4. **Vérifier les appareils** : la sonnette et les caméras apparaissent dans les appareils de
   Moli (fabricant Ring), avec batterie et Wi-Fi remplis. Dans l'appli Ring, Centre de
   contrôle, « appareils clients autorisés » : Moli y figure (c'est là qu'on le retire).

5. **Tester un appui** : sonner. `doorbell` passe à vrai dans les 10 à 20 s et retombe 10 s
   plus tard ; un passage devant une caméra passe `motion` à vrai. Le journal (`RUST_LOG=
   info,moli_ring=debug`) ne montre ni jeton ni valeur de `sign_in` (`null`).

6. **Redémarrer Moli** : `doorbell` reste faux, rien ne se déclenche (un appui d'avant le
   démarrage n'est jamais annoncé).

7. **Vérifier la garde** (facultatif) : un `POST /api/command` sur `<compte>/sign_in` sans
   `x-moli-origin: ui` (ou par MCP) reçoit 403, et rien n'apparaît dans les approbations.

### Vérifié sur une installation réelle

Octobre 2026, une sonnette Ring sur batterie et deux Stick Up Cam Mini sur un même lieu :
connexion par un jeton de ring-auth-cli collé dans Système, appareils lus, appui vu dans Moli
en une dizaine de secondes, mouvements des caméras vus. Pas encore vu : un carillon
(`chimes/<id>/health`), un compte à plusieurs lieux.

### Ordre de fusion

1. Cette PR (`contrib/ring`).
2. Puis la PR « images à la demande et modes de surveillance » (`contrib/ring-images-modes`),
   construite **sur** cette branche : à relire après celle-ci (son diff propre est celui de
   `contrib/ring..contrib/ring-images-modes`).

Aucune dépendance aux autres contributions proposées en parallèle.

### Limites connues

- **Interface non officielle** : Ring peut la changer ou la fermer sans préavis ; c'est dit
  dans la carte, le README et l'onboarding.
- Pas instantané : un appui est vu 10 à 20 s après (sondage, pas de notification push FCM).
- Lecture seule dans cette PR (pas d'image, pas de mode : PR suivante ; pas de volume du
  carillon, de sirène ni de lumière).
- R3 ci-dessus : identifiants du compte Ring (pas de secret) dans les messages d'échec HTTP.
- Changer le mot de passe Ring, ou retirer Moli du Centre de contrôle, oblige à refaire un
  jeton.
- `clippy` 1.97 (stable actuelle) échoue sur `main` lui-même (`clippy::float_cmp` dans des
  tests de `moli-energy`, lint inconnu `clippy::unused_async_trait_impl` dans
  `moli-api/src/mcp.rs`) : hors de cette PR, qui ne touche pas ces fichiers. Avec ces deux
  lints autorisés, `clippy -D warnings` est propre sur tout l'espace de travail.

## Checked

- [x] `sh scripts/check.sh` (fmt, clippy, tests, catalogue, leak check) — chaque étape lancée ;
  clippy propre avec `-A unknown_lints -A clippy::float_cmp` (dette de toolchain de `main`,
  voir plus haut) ; tous les tests verts ; `catalogue check` : `ring` reconnu ; leak-check propre
- [x] `npm run build` in `web/` and `node scripts/i18n-check.mjs` (2 790 clés, aucun texte en dur)
- [x] New texts in `locales/fr` **and** `locales/en`
- [x] No personal data (names, addresses, MACs, serials, keys): fake values only
  (`10000001`, `00000000-0000-4000-8000-000000000001`, `user@example.com`, « 1 rue de
  l'Exemple »)
- [x] Tried on real hardware? Ring battery doorbell and two Stick Up Cam Mini, October 2026
  (in `docs/components/pilotes.md` and `integrations/ring/onboarding.md`)
