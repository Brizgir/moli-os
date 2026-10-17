# Revue de sécurité : branches Ring et Personnes (octobre 2026)

Revue en lecture, puis corrections, avant déploiement dans une maison habitée. Modèle de
menace de Moli (`AGENTS.md`, `docs/components/api.md` § 1.2 bis, `caller.rs`) : le réseau de la
maison est le foyer ; Internet, une adresse Tailscale `100.x` ou un relais sont l'extérieur et
n'obtiennent rien sans personne connectée ou Cloudflare Access ; aucun secret ne sort du coffre
chiffré ni n'apparaît dans un journal.

| Branche | Commit revu | Verdict |
|---|---|---|
| `claude/vigilant-gates-a65aab` (personnes) | `5a8e8ec` | **à retravailler** avant déploiement : un correctif (P1, patch ci-dessous), à appliquer sur sa branche |
| `claude/happy-chaum-36e30d` (pilote Ring) | `f3f327c` | **déployable après les corrections appliquées** sur cette branche (`claude/revue-ring-personnes`) |

Gravités : **bloquant** (ne pas déployer), **à corriger** (avant déploiement), **amélioration**.
Les numéros de ligne renvoient au commit revu, pas à cette branche.

---

## 1. Branche `claude/vigilant-gates-a65aab` — personnes

Correctif revu : sans Cloudflare Access, un e-mail dans la fiche ne fait plus un « propriétaire
qui peut se connecter » (`people.rs:535-543`, `has_owner`), si bien que la session ouverte avec
le code de la maison garde la gestion des personnes jusqu'à ce qu'un propriétaire ait un mot de
passe. Rien n'est modifié dans `caller.rs`. Le diff est petit (4 fichiers), lisible, et le test
`the_first_owner_is_invited_from_the_house` couvre le cas visé.

### Ce qui tient

- **Extérieur.** Toute requête `Via::Outside` est refusée par `identify` (`caller.rs:341-347`)
  avant tout gestionnaire, sauf les fichiers du dashboard, `POST /api/session` et
  `POST /api/invitation`. Une adresse publique, une `100.x` (CGNAT, ni privée ni ULA :
  `is_home` la classe dehors), une boucle locale relayée (`forwarded`, `x-forwarded-for`,
  `x-real-ip`, `tailscale-user-login`), ou des en-têtes Cloudflare venus du réseau local
  n'atteignent jamais `/api/people`. Le cookie du code de la maison est lié à `Caller::key`
  (`session.rs:423-432`) : volé, il ne sert pas depuis une autre adresse. Le test ajouté
  (`http.rs:1244-1246`, `X-Forwarded-For` + cookie du code → 403) vérifie bien ce point.
- **Pas de fenêtre durable.** `has_owner` est réévalué à chaque requête : dès qu'un
  propriétaire actif a un mot de passe (`core_secret(password_key)`), ou un e-mail avec
  `[server.access]` configuré (`behind_access(options.access.is_some())`, `lib.rs:239`), la
  session du code perd la gestion des personnes. Vérifié par le test (`http.rs:1264`, 403 après
  le mot de passe) et le test unitaire `an_e_mail_makes_an_owner_only_behind_access`.
- **Pas d'auto-promotion ordinaire.** `PUT /api/people/{id}` exige `is_owner` ; un invité ne
  passe pas le filtre `guest_allowed` (`caller.rs:64-90`) pour `POST/PUT /api/people…` ;
  `PUT /api/people/{id}/password` n'accepte que la personne elle-même (`people_api.rs:234`) ;
  `POST /api/people/{id}/invite` exige `is_owner`. Conforme à la doc.
- `docs/components/api.md` décrit la règle finale, et exactement celle du code.

### P1 — à corriger avant déploiement : le code de la maison « s'ouvre » depuis Internet pour une personne connectée, et donne les droits de propriétaire

- **Fichiers** : `crates/moli-api/src/people_api.rs:73-76` (troisième branche de `is_owner`,
  `caller.is_household()`), avec `crates/moli-api/src/caller.rs:136` (`is_household` inclut
  `Via::Person`) et `crates/moli-api/src/rest.rs:489` (`POST /api/session {pin}` n'exige que
  `is_household`).
- **Scénario, reproduit par un test** (sortie ci-dessous) : le premier propriétaire est créé
  avec son e-mail, sans Access, sans mot de passe (le cas que la branche veut rendre possible).
  Un **membre** invité a choisi son mot de passe et connaît le code de la maison (c'est la
  famille). Depuis Internet (adresse publique), il se connecte avec son mot de passe
  (`Via::Person`), puis tape le code de la maison sur `POST /api/session` : accepté (200),
  cookie lié à `person:<id>`. Avec les deux cookies, `is_owner` est vrai : il invite le
  propriétaire à sa place (il reçoit le code d'invitation du propriétaire, donc choisit son mot
  de passe), ou **se promeut propriétaire** (`PUT /api/people/elodie {role: owner}` → 200).

  ```
  PROBE pin login from Internet as member: 200 {"human":true}
  PROBE invite the owner from Internet as member with the house code: 200 {"code":"…"}
  PROBE promote self: 200 {"id":"elodie","name":"Élodie","role":"owner",…}
  ```

  La doc de la branche dit pourtant « le code ne s'ouvre jamais depuis Internet ni une adresse
  Tailscale ». Le test de la branche ne couvre que l'inconnu (`X-Forwarded-For` sans personne
  connectée). La faille préexiste sur `main` (`is_household` et le `login` sont inchangés), mais
  `main` la fermait de fait dès qu'un propriétaire avait un e-mail ; la branche l'ouvre pour
  tout le temps où le premier propriétaire n'a pas encore de mot de passe, c'est-à-dire
  précisément la période qu'elle crée.
- **Correction proposée** (validée dans un arbre de travail : les 16 tests `http` passent et la
  sonde ci-dessus répond 403, 403) : le code de la maison ne gère les personnes que **depuis le
  foyer**, jamais pour une personne connectée par mot de passe ni pour un visiteur de démo.

  ```diff
  --- a/crates/moli-api/src/people_api.rs
  +++ b/crates/moli-api/src/people_api.rs
  @@ -70,9 +70,12 @@ pub(crate) fn is_owner(
       {
           return true;
       }
  +    // The house's code manages the people from the house only: never a
  +    // person signed in from the Internet with their password (a member
  +    // who knows the code would make themself an owner from anywhere).
       !persons.has_owner()
           && !humans.has_owners()
  -        && caller.is_household()
  +        && matches!(caller.via, Via::Lan | Via::Loopback | Via::Access { .. })
           && humans.is_human(headers, &caller.key)
   }
  ```

  Et le test à ajouter dans `crates/moli-api/tests/http.rs`, à la suite de
  `the_first_owner_is_invited_from_the_house` (mêmes aides `serve`, `request_with`,
  `request_cookie`) :

  ```rust
  /// A member signed in from the Internet with their password, who knows the
  /// house's code: the code manages nobody from there, even while the first
  /// owner has no password yet.
  #[tokio::test]
  async fn the_house_code_manages_nobody_from_the_internet() {
      let dir = std::env::temp_dir().join(format!("moli-http-code-{}", std::process::id()));
      std::fs::create_dir_all(&dir).unwrap();
      let hub = Hub::new(HubOptions {
          secrets: Some((dir.join("secrets.enc"), moli_runtime::MasterKey::generate().unwrap())),
          ..HubOptions::default()
      })
      .unwrap();
      let options = moli_api::Options {
          ui_pin: Some(moli_api::UiPin::new("482915")),
          home_path: Some(dir.join("home.json")),
          ..moli_api::Options::default()
      };
      let addr = serve(&hub, &options).await;
      let outside = "X-Forwarded-For: 203.0.113.9\r\n";
      let (_, pin, _) = request_cookie(&addr, "POST", "/api/session", &json!({ "pin": "482915" }), "", "moli_session").await;
      let pin = format!("Cookie: {}\r\n", pin.unwrap());
      let owner = json!({ "name": "Moi", "role": "owner", "emails": ["user@example.com"] });
      assert_eq!(request_with(&addr, "POST", "/api/people", Some(&owner), &pin).await.0, 201);
      let member = json!({ "name": "Élodie", "role": "member" });
      assert_eq!(request_with(&addr, "POST", "/api/people", Some(&member), &pin).await.0, 201);
      let (_, body) = request_with(&addr, "POST", "/api/people/elodie/invite", None, &pin).await;
      let code = serde_json::from_str::<Json>(&body).unwrap()["code"].as_str().unwrap().to_owned();
      let invitation = json!({ "code": code, "password": "le jardin fleuri" });
      let (status, cookie, _) = request_cookie(&addr, "POST", "/api/invitation", &invitation, outside, "moli_person").await;
      assert_eq!(status, 200);
      let me = format!("{outside}Cookie: {}\r\n", cookie.unwrap());
      // The member, from the Internet, types the house's code.
      let (_, house, _) = request_cookie(&addr, "POST", "/api/session", &json!({ "pin": "482915" }), &me, "moli_session").await;
      let both = format!("{me}Cookie: {}\r\n", house.unwrap_or_default());
      let (status, _) = request_with(&addr, "POST", "/api/people/moi/invite", None, &both).await;
      assert_eq!(status, 403, "the house's code manages nobody from the Internet");
      let promoted = json!({ "name": "Élodie", "role": "owner" });
      let (status, _) = request_with(&addr, "PUT", "/api/people/elodie", Some(&promoted), &both).await;
      assert_eq!(status, 403, "nobody promotes themself");
      let _ = std::fs::remove_dir_all(&dir);
  }
  ```

  À compléter dans `docs/components/api.md` (§ Personnes) : « depuis le foyer » = réseau local,
  la machine ou Cloudflare Access, jamais une personne connectée par mot de passe depuis Internet.

### P2 — amélioration : le code de la maison s'accepte depuis Internet pour une personne connectée

- **Fichier** : `crates/moli-api/src/rest.rs:489` (`login`, `is_household`), préexistant sur
  `main`.
- **Scénario** : une personne de la maison connectée par mot de passe depuis Internet peut ouvrir
  une session avec le code de la maison (budget d'essais « Internet », verrou par
  `person:<id>`). Elle obtient alors l'origine `ui` depuis dehors : commander une pièce
  protégée, libérer la demande d'un agent, voir la caméra d'une chambre, depuis n'importe où,
  avec le mot de passe **et** le code. C'est peut-être voulu (« un téléphone bouge »), mais la
  doc dit l'inverse, et les deux facteurs se tapent dans le même navigateur. Un invité connecté
  (`guest_allowed` laisse passer `POST /api/session`) peut aussi essayer le code.
- **Proposition** : dans `login`, pour `LoginBody::Pin`, exiger `Via::Lan | Via::Loopback |
  Via::Access` (le code est celui du foyer), et le dire dans `api.md`. Décision du propriétaire
  de la maison : non appliquée ici.

### P3 — amélioration : un propriétaire « qui peut se connecter » mais dont la session a expiré

- `has_owner` compte un propriétaire avec mot de passe même si sa fiche a `expires` dépassé ?
  Non : `active(p)` est vérifié (`people.rs:539`). Rien à corriger ; noté pour mémoire.

---

## 2. Branche `claude/happy-chaum-36e30d` — pilote Ring

Nouveau crate `crates/moli-ring` (API cloud non officielle de l'appli Ring, lecture seule) et
`integrations/ring/`. Le crate n'ajoute **aucune dépendance externe** (`Cargo.lock` : seulement
`moli-ring` et des crates déjà présents) ; `cargo audit` sur la branche : 0 vulnérabilité, un
avertissement déjà présent sur `main` (`rustls-pemfile` non maintenu, RUSTSEC-2025-0134, hors
Ring). TLS : `moli_net::web(host, 443, true, …)` → `public_tls()` (racines Mozilla, vérification
rustls complète, `moli-net/src/lib.rs:194-209`), hôtes fixes `oauth.ring.com` et
`api.ring.com`, aucune URL construite depuis une réponse (`location_id` et `id` ne servent que
dans le chemin, sur le même hôte). Aucun `danger_accept_invalid_certs`.

### R1 — bloquant (corrigé ici) : le mot de passe Ring transitait par Moli

- **Fichiers** : `crates/moli-ring/src/signin.rs:20-26,81-104,147-152` (`Credentials`,
  `credentials()`, `Grant::Password`), `crates/moli-ring/src/auth.rs:89-107,140`
  (`grant_type = "password"`, `"password": password`, en-tête `2fa-code`),
  `crates/moli-ring/src/lib.rs:137-146` (points `sign_in` = `{"email","password"}` et `code`),
  `web/src/maison/ui/RingAccount.svelte:51,95` (`<input type="password">`, envoi par
  `POST /api/command`), `integrations/ring/README.md:14`.
- **Scénario** : la personne tape l'e-mail et le mot de passe de son compte Ring (souvent son
  compte Amazon) dans le tableau de bord ; ils voyagent dans `POST /api/command` (en clair sur
  le réseau local, `http://<hôte>:8790`), passent par le hub, restent en mémoire du pilote
  jusqu'à 10 minutes, et repartent vers `oauth.ring.com` à chaque essai de code. La branche fait
  un vrai effort pour ne pas les garder (sémantique `Secret` : `null` au journal, pas d'état,
  jamais retenu pour approbation, `Debug` redigé) ; mais la règle de la maison est que le mot de
  passe **ne transite jamais** par Moli, et Ring n'offre aucune page de connexion pour une autre
  application (pas de code d'autorisation OAuth, à la différence d'Amazon pour Alexa).
- **Correction appliquée** (flux « jeton collé », le plus proche du modèle Alexa possible) :
  la personne fait un jeton **sur son ordinateur** avec le client de Ring
  (`npx -p ring-client-api ring-auth-cli` : Ring y demande e-mail, mot de passe et code 2FA,
  rien n'arrive à Moli), puis colle ce jeton dans la carte Ring. Le point `sign_in`
  (`Semantic::Secret`, toujours réservé à une personne avec son code) reçoit le jeton ; Moli le
  vérifie **une fois** par `grant_type = refresh_token`, Ring le remplace aussitôt, et seul le
  jeton rendu va au coffre. Le texte collé n'est ni journalisé, ni gardé, ni affiché.
  - `auth.rs` : plus de `Grant::Password`, plus d'en-tête `2fa-code`, plus de variantes
    `Code`/`BadCode`/`BadPassword` ; test `only_a_refresh_token_ever_goes_to_ring` (le corps
    envoyé n'a ni `password` ni `username`).
  - `signin.rs` réécrit : `connect(net, pasted, ours)` accepte le jeton brut ou la forme
    `base64({"rt","hid"})` que ring-auth-cli affiche (le `hid` devient alors le `hardware_id`
    de Moli : le même « appareil client autorisé » que l'ordinateur, ce qui évite qu'un jeton
    lié à un autre appareil soit refusé) ; ce qui n'est pas un jeton (vide, deux mots, non
    ASCII, trop long, balises) n'est **jamais envoyé** à Ring ; `Connected` a un `Debug` redigé.
    Tests : `a_pasted_text_is_a_bare_token_or_what_the_cli_prints`,
    `the_pasted_token_is_checked_with_ring_and_its_hardware_id_taken`,
    `what_is_not_a_token_never_reaches_ring_and_a_refused_one_says_so`.
  - `lib.rs` : points `code_by` et `code` retirés, boucle `sign_in` simplifiée ; **sans coffre,
    rien n'est envoyé à Ring** (`ctx.secrets_problem()` avant tout : un jeton collé est consommé
    par sa première vérification, il ne faut pas le brûler pour rien) ; `hardware_id` puis
    `refresh_token` rangés **avant** l'usage du client.
  - `RingAccount.svelte` : trois étapes et un champ de collage (masqué), plus de champ e-mail
    ni mot de passe ; locales `fr` et `en` ; `README.md`, `onboarding.md`, `pilotes.md`,
    `coeur.md` ; fixtures `oauth-2fa`, `oauth-bad-code`, `oauth-bad-password` retirées (et du
    manifeste).
- **Reste vrai** : le jeton de rafraîchissement est lui-même un secret durable du compte
  (comme pour Alexa). Il ne vit qu'au coffre et en mémoire ; il ne transite par Moli qu'une
  fois, à la main de la personne, sur une session humaine.

### R2 — à corriger (corrigé ici) : « on sonne » ou « mouvement » figés quand Ring devient injoignable

- **Fichier** : `crates/moli-ring/src/lib.rs:324-387` (`connected`, table `resets` à la ligne 358), `lib.rs:156-205` (`run`).
- **Scénario** : `doorbell` passe à vrai (impulsion, retour à faux prévu 10 s plus tard par
  `resets`). Si, dans ces 10 s, un appel à Ring échoue (panne, 429, jeton révoqué), `connected`
  rend `Err`, la table `resets` est perdue, et `doorbell`/`motion` **restent à vrai** jusqu'à la
  prochaine reconnexion réussie : 30 s à 15 min de recul exponentiel, 10 min sur un 429, et
  indéfiniment si le jeton est révoqué (retour à la connexion). Un automatisme « on sonne →
  annonce » ne redéclenche pas (pas de nouvelle impulsion), mais le dashboard, Moli et la
  règle de la maison (« jamais de "on sonne" périmé, remise à faux quand le cloud est
  injoignable ») sont trahis ; les appareils restent « en ligne ».
- **Correction appliquée** : la boucle est isolée dans `watch()` ; sur toute erreur,
  `lost()` remet `doorbell` et `motion` à faux sur chaque sonnette et caméra suivie et passe
  tous les appareils **indisponibles** jusqu'à ce que Ring les redonne (comme le pilote
  Reolink quand la station est injoignable). Doc mise à jour (`README.md`, `pilotes.md`).

### R3 — amélioration (non appliquée) : identifiants de compte dans les journaux

- **Fichiers** : `crates/moli-ring/src/api.rs:177` (`"{API_HOST}{path}: HTTP … {text}"`),
  `lib.rs:197,498` (`error = %failure`, `error = %e`).
- **Scénario** : un échec HTTP inscrit au journal le chemin complet, donc le `location_id`
  (UUID du lieu Ring) et l'`id` numérique de l'appareil, plus 200 octets du corps de la
  réponse de Ring. Pas un secret (aucun jeton : il est dans l'en-tête `Authorization`, jamais
  cité ; `hardware_id` jamais journalisé), mais des identifiants de compte qui n'ont rien à
  faire dans un journal partagé pour un diagnostic.
- **Proposition** : ne citer que le chemin sans ses identifiants (`doorbots/<id>/health` →
  `doorbots/…/health`), et ne pas copier le corps au-delà du `error_description` JSON.

### R4 — amélioration (appliquée, bon marché) : une capture réelle commitée par mégarde

- `scripts/leak-check.sh` attrape les e-mails, MAC, clés et JWT, mais pas un `location_id`,
  un `id` d'appareil, un numéro de téléphone masqué, une adresse postale ou des coordonnées,
  tous présents dans les réponses Ring. Ajouté : `*.real.json` dans `.gitignore`, et
  `onboarding.md` demande de capturer d'abord dans un tel fichier, puis d'anonymiser.
- Fixtures de la branche relues : `ring-devices.json` (adresse « 1 rue de l'Exemple »,
  `user@example.com`, `aabbccddeeff`, coordonnées 0.0, ids `10000001`/`10000002`, UUID de lieu
  `00000000-0000-4000-8000-000000000001`), `events.json`, `history.json`,
  `doorbot-health.json`, `oauth-token.json` (`fake-…`), `oauth-bad-refresh.json` : rien de réel.
  `scripts/leak-check.sh` : propre.

### Ce qui tient (vérifié)

- **Secrets.** `refresh_token` et `hardware_id` ne vivent qu'au coffre (`ctx.store_secret`) et
  en mémoire (`Client`, `Debug` redigé ; `Tokens`, `Debug` redigé) ; jamais dans `moli.toml`
  (`Config` refuse tout champ inconnu, test `the_poll_has_a_floor_and_nothing_else_is_accepted`),
  dans `data/`, une fixture, ou un message d'erreur (`interpret` ne cite jamais la requête ;
  `Failure::Other` ne porte que le chemin et la réponse). Le jeton de rafraîchissement renouvelé
  est écrit au coffre **avant** d'être utilisé (`api.rs:89-102`, `Vault::keep` puis
  `self.refresh = …`) ; si l'écriture échoue, l'erreur remonte sans qu'aucun appel parte avec
  le nouveau jeton (test `a_token_that_cannot_be_kept_is_not_used`). Une panne entre la réponse
  de Ring et l'écriture perd au pire le jeton tout juste rendu : la personne refait un jeton ;
  le compte, lui, n'est jamais perdu.
- **Sémantique `Secret` dans le cœur** (`hub.rs:504-507,530,1583-1585`) : refusée (403
  `HumanOnly`) à toute origine autre que `ui` (test `a_sign_in_is_a_persons_and_never_journaled`
  : `mcp`, `api`, `household`, `assistant`), jamais retenue pour approbation (la demande
  garderait la valeur), `null` au journal, jamais gardée comme état (donc ni SSE d'état, ni
  historique SQLite). Un automatisme (`Origin::Automation`) ne peut pas l'écrire non plus.
- **Routes et origine.** Aucune route ajoutée dans `moli-api`. La connexion passe par
  `POST /api/command` : `origin()` ne donne `ui` qu'avec `x-moli-origin: ui` **et** une session
  humaine liée à l'appelant ; le dashboard de confiance sans code (`household`) reçoit 403 et
  ouvre la feuille du code. L'extérieur est refusé par `identify` avant la route. Rien de
  secret n'est renvoyé au navigateur : `GET /api/devices` montre `status` (texte) et la
  spécification du point `sign_in`, jamais une valeur.
- **Sûreté fonctionnelle.** Un événement est daté (`created_at`), dit une fois (`Watch::seen`),
  seulement s'il a moins de 90 s (`FRESH`) et jamais à la première lecture après un démarrage
  ou une reconnexion (`Watch::started`) ; `doorbell` impulsion de 10 s, `motion` 60 s, remis à
  faux à chaque démarrage, et désormais à chaque perte de Ring (R2). Recul exponentiel 30 s →
  15 min sur erreur, pause de 10 min sur 429 (`TooMany`, `SLOW_DOWN`), aucune boucle serrée :
  un 401 sur l'API ne renouvelle qu'**une** fois (`call`), un 401 sur le renouvellement oublie
  le jeton et remet le pilote en `waiting` (`ctx.wait_for`, statut `sign_in`) sans réessayer.
  Une demande de code de vérification (412) sur un renouvellement est traitée comme un jeton
  refusé : retour à la connexion, message clair, pas de boucle.
- **Hôtes et TLS** : voir l'en-tête de cette section.

### Vérifications sur cette branche

Après `cargo clean`, toolchain locale Rust 1.97 (l'image de dev `rust:1-alpine` suit la stable) :

| Vérification | Résultat |
|---|---|
| `cargo fmt --all -- --check` | propre |
| `cargo clippy --workspace --all-targets -- -D warnings` (pedantic) | propre pour `moli-ring` et tout ce que les deux branches touchent ; échoue seulement sur `clippy::float_cmp` dans des tests de `moli-energy` et `moli-history` et un `#[allow]` d'un lint que clippy 1.97 ne connaît plus (`moli-api/src/mcp.rs:543`), fichiers identiques à `main` : dette de version de toolchain, pas de ces branches. Avec ces deux lints seuls autorisés (`-A unknown_lints -A clippy::float_cmp`) : propre sur tout l'espace de travail |
| `cargo test --workspace` | tous verts (dont 17 tests `moli-ring`, 16 tests `http` de `moli-api`, le test `a_sign_in_is_a_persons_and_never_journaled` du cœur) |
| `cargo run -p moli-os -- catalogue check integrations` | `✓ ring … 6 files` |
| `scripts/leak-check.sh` | propre |
| `node scripts/i18n-check.mjs` | 2 790 clés, `fr` et `en` complets, aucun texte en dur |
| `cd web && npm run build` | construit |
| `cargo audit` (base RustSec du jour, 284 crates) | 0 vulnérabilité ; 1 avertissement déjà sur `main`, hors Ring (`rustls-pemfile` non maintenu, RUSTSEC-2025-0134) |

Non vérifié ici, comme la branche le dit déjà : le pilote n'a jamais tourné contre les
serveurs Ring. Le flux par jeton collé repose sur des faits d'interface des projets ouverts
(ring-client-api : forme `{"rt","hid"}` de `ring-auth-cli`, `grant_type = refresh_token` avec
en-tête `hardware_id`) ; `onboarding.md` décrit le premier essai, à faire avec la personne.

---

## 3. Verdicts

- **Personnes (`claude/vigilant-gates-a65aab`)** : à retravailler avant déploiement, le temps
  d'appliquer P1 (une ligne et un test) sur sa branche ; sans P1, un membre qui connaît le code
  de la maison se fait propriétaire depuis Internet tant que le premier propriétaire n'a pas
  choisi son mot de passe.
- **Ring (`claude/happy-chaum-36e30d`)** : déployable après les corrections appliquées ici
  (R1, R2, R4), c'est-à-dire en déployant `claude/revue-ring-personnes` plutôt que la branche
  d'origine, avec un premier essai contre un vrai compte selon `onboarding.md`.
