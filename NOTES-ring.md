# Pilote Ring : où on en est (note de reprise, à supprimer une fois fini)

## Fait

- **Cœur** : `Semantic::Secret` (`moli-core/src/spec.rs`). `Hub::command` (`moli-runtime/src/hub.rs`) :
  un point `secret` n'accepte que l'origine `ui` (sinon `CommandError::HumanOnly`, 403 en REST),
  n'est jamais retenu pour approbation, est journalisé avec `null` ; `record_state` ne garde
  jamais sa valeur. Test : `a_sign_in_is_a_persons_and_never_journaled`
  (`moli-runtime/tests/hub.rs`), **passe**.
  Pourquoi : l'e-mail et le mot de passe Ring passent par `/api/command`, et le journal garde
  la valeur de chaque commande (`journal.jsonl`).
- **Crate `crates/moli-ring`** : `auth.rs` (OAuth `oauth.ring.com`, `ring_official_android`,
  2FA par en-tête `2fa-code`, 412 = code demandé), `api.rs` (jeton d'accès, renouvellement 5 min
  avant ou sur 401, **nouveau refresh token rangé avant usage**), `signin.rs` (identifiants
  puis code, en mémoire 10 min au plus), `model.rs` (appareils, santé, événements, `Watch` :
  un événement dit une fois, < 90 s, jamais à la première lecture), `net.rs` (transport réel
  et rejeu pour les tests), `lib.rs` (pilote), `fixtures/` (réponses anonymisées).
  17 tests du crate **passent**.
- Câblage `moli-os` (`KINDS`, `build_driver`), workspace `Cargo.toml`.
- Carte `web/src/maison/ui/RingAccount.svelte` dans Système ; traductions fr/en
  (`pilotes.ring.*`, `systeme.ring.*`) ; `npm run build` et `i18n-check` **passent**.
- Docs : `integrations/ring/{integration.toml,onboarding.md,README.md}`,
  `docs/components/pilotes.md` (section `ring`, tableau, secrets, tests), `docs/components/coeur.md`
  (connexions de compte).

## Reste à faire

1. **Clippy** (`single_match_else`, `crates/moli-ring/src/lib.rs`, début de la boucle de `run`) :
   remplacer le `match (ctx.secret(REFRESH), ctx.secret(HARDWARE_ID))` par
   `if let (Some(refresh), Some(hardware_id)) = (…) { Client::new(…) } else { sign_in… }`.
2. Relancer `sh scripts/check.sh` en entier : catalogue (`moli-os catalogue check integrations`)
   et `scripts/leak-check.sh` pas encore vus passer.
3. `fmt` passait juste avant la dernière retouche ; relancer `cargo fmt --all`.

## À savoir

- `moli-profile` : `tests::an_address_a_person_changed_is_obeyed` échoue **aussi sur le commit de
  départ** (d54c3de), sans ces changements : défaut existant, instable (1 réussite sur 7), hors
  de ce chantier.
- Rien n'a été essayé contre les vrais serveurs Ring (pas de compte ici). À surveiller au premier
  essai : `POST /clients_api/session` (non bloquant), la forme de
  `locations/<loc>/devices/<id>/events` (repli automatique sur `doorbots/<id>/history` si 404),
  `chimes/<id>/health`.
- Décisions : lecture seule (pas de volume du carillon en écriture), pas d'image, sondage des
  événements toutes les 10 s au lieu des notifications push FCM (suite documentée) ;
  disponibilité de l'appareil pour « en ligne » plutôt qu'un point `online` ; `doorbell` vrai
  10 s, `motion` vrai 60 s ; `hardware_id` tiré une fois et gardé au coffre.
- Test chez l'utilisateur : voir `integrations/ring/onboarding.md`. Il se connecte lui-même dans
  Système ; ne jamais lui demander son mot de passe dans le chat.
