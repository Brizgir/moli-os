<!-- Titre proposé : Moli parle aussi avec Claude (API Messages d'Anthropic, en natif) -->
<!-- Issue : Closes #<numéro de l'issue ouverte avec ISSUE.md> -->

## What and why

**Résumé (English).** Moli, the assistant, can now think with **Claude** through Anthropic's
Messages API, natively: `crates/moli-assistant/src/claude.rs` translates the conversation Moli
already builds (OpenAI-shaped messages and tools) to `POST /v1/messages` and back, so the tool
loop, the guard and the safety nets stay a single code path. Opt-in with
`[assistant] provider = "anthropic"` (or from the System page); without it nothing changes, OpenAI
stays the default. The key lives in the vault as `assistant/anthropic_api_key`. Anthropic has no
voice: the voice keeps using OpenAI's key if present, otherwise local Whisper and Piper. Tried
on a real installation: conversation with `claude-haiku-5-5`, automation drafts with
`claude-opus-5-5`, local voice. No new dependency.

### Ce que ça change

- **Un second fournisseur, natif** (`claude.rs`, nouveau) : pas de SDK ni de passerelle, du
  JSON sur le client HTTP déjà utilisé pour OpenAI. Système à part avec deux points de cache
  (la date après le dernier), outils en mode strict, appels d'outils en parallèle, blocs de
  réflexion renvoyés tels quels, recherche web par l'outil serveur d'Anthropic, brouillons
  d'automatismes en sortie JSON (`output_config.format`).
- **`[assistant]`** : `provider` (`"anthropic"` | `"openai"`, défaut `openai` sauf `base_url`
  d'Anthropic), modèles Claude (`claude-opus-5-5` par défaut, `claude-sonnet-5-5`,
  `claude-haiku-5-5`, `claude-fable-5-1`), `effort` (conversation, `low`) et `builder_effort`
  (brouillons, `high`), `base_url_voice` pour la voix.
- **Une clé à part** dans le coffre : `assistant/anthropic_api_key`. La clé OpenAI
  (`assistant/api_key`) garde son nom et son rôle (la réflexion d'OpenAI, et la voix).
- **Page Système → « Moli, l'assistant »** : « Qui réfléchit » (Claude ou OpenAI) et le
  modèle (geste humain, code demandé : `PUT /api/assistant/brain`), la clé de chaque
  fournisseur avec son pas-à-pas, une ligne « Clé OpenAI pour la voix (facultative) » qui dit ce
  que fera la voix sans elle, le barème du modèle et le coût estimé du jour.
- **`pricing.rs`** : les modèles Claude, le cache relu et écrit, la recherche web (0,01 $).
- **`leak-check.sh`** : les fausses clés des tests (`sk-ant-test-abcdefghij…`) sont admises,
  comme les `sk-abcdefghij…` d'OpenAI.
- Doc : `docs/components/moli.md`, `moli.example.toml`, README (fr, en). Textes en
  `locales/fr` et `locales/en`.

### Pourquoi

Une maison qui préfère Claude n'avait pas de chemin propre : l'API d'Anthropic n'est pas
compatible OpenAI sur ce qui compte ici (refus, réponse coupée, réflexion, cache, recherche web),
et une passerelle de traduction perdrait justement ces nuances. Voir l'issue pour la discussion.

### Sécurité

- Toujours **aucun LLM dans le chemin d'exécution** ; un automatisme reste un brouillon jusqu'à
  la validation d'un humain.
- **`stop_reason` lu avant le contenu** : `refusal` → rien de ce que la réponse demandait n'est
  exécuté (« Je préfère ne pas répondre à cette demande. ») ; `max_tokens` → les mots arrivés
  sont dits, aucun appel n'est exécuté.
- Jamais de `tool_choice` forcé, de réponse pré-remplie, de `thinking` désactivé ni de budget.
- La clé : chiffrée dans le coffre, jamais au journal ni renvoyée, envoyée seulement en https,
  posée seulement par une personne (code). Choisir le modèle aussi (il change ce que Moli coûte).

### Les commits

1. *Moli parle aussi avec Claude : API Messages d'Anthropic, en natif* — l'essentiel.
2. *« X ou rien » devient un champ facultatif dans les schémas stricts* — trouvé à l'essai :
   Anthropic refuse un schéma strict à plus de 16 paramètres à plusieurs types ; le brouillon
   d'automatisme, écrit pour OpenAI (tout requis, `null` partout), en comptait 31 (HTTP 400).
3. *Un schéma trop riche pour le mode strict passe en consigne* — trouvé à l'essai aussi :
   au-delà de 24 champs facultatifs, le schéma va dans les consignes au lieu de
   `output_config.format`, et le brouillon se relit même entouré d'un bloc de code.

## Guide : essayer Moli avec Claude, pas à pas

On part d'une installation qui tourne déjà (Docker, coffre ouvert par `MOLI_MASTER_KEY`, voir
`docs/components/exploitation.md`). Les commandes supposent le conteneur `moli-os` ; avec
Compose, remplacer `docker exec -i moli-os` par `docker compose exec -T moli-os`.

### 1. Construire l'image de cette branche

```sh
git fetch origin pull/<numéro>/head:assistant-claude && git checkout assistant-claude
docker build -t moli-os .
```

### 2. Obtenir une clé Anthropic (5 minutes)

1. Ouvrir <https://console.anthropic.com/> et créer un compte (ou se connecter).
2. **Billing** : ajouter un peu de crédit (quelques dollars couvrent longtemps une maison).
3. Conseillé, **Limits** : fixer une limite de dépense mensuelle.
4. **API keys** → « Create Key », la nommer « Moli », la copier (elle commence par `sk-ant-`).

### 3. Poser la clé dans le coffre, par un tube

Jamais au clavier dans une commande, jamais en argument, jamais dans `moli.toml` :

```sh
read -rs V; printf %s "$V" | docker exec -i moli-os /moli-os secrets set assistant anthropic_api_key; unset V
```

(`read -rs` lit la clé collée sans l'afficher ni la garder dans l'historique.)

*Autre chemin, sans terminal :* page **Système → Moli, l'assistant**, choisir « Claude
(Anthropic) » puis « Ajouter une clé » : le code est demandé, la clé est vérifiée auprès
d'Anthropic (`GET /v1/models`) avant d'être rangée, et prise en compte tout de suite.

### 4. Le bloc `[assistant]` de `moli.toml`

Le minimum :

```toml
[assistant]
provider = "anthropic"
```

Avec tous les réglages (ce sont les défauts, sauf mention) :

```toml
[assistant]
provider = "anthropic"            # "anthropic" | "openai" ; sans lui : openai
model = "claude-opus-5-5"         # la conversation ; claude-sonnet-5-5, claude-haiku-5-5, claude-fable-5-1
builder_model = "claude-opus-5-5" # les brouillons d'automatismes
effort = "low"                    # profondeur de réflexion en conversation : low … max
builder_effort = "high"           # … pour un brouillon d'automatisme
# search_model = "claude-opus-5-5"  # questions sur le monde (recherche web) ; défaut : model ; "" = pas de web
# base_url = "https://api.anthropic.com"         # avec ou sans /v1
# base_url_voice = "https://api.openai.com/v1"   # la voix (voir l'étape 6)
# timezone = "Europe/Paris"
```

Pour une maison, `claude-haiku-5-5` en conversation (rapide, peu coûteux) et
`claude-opus-5-5` pour les brouillons d'automatismes (rares, à soigner) est un bon équilibre :

```toml
model = "claude-haiku-5-5"
builder_model = "claude-opus-5-5"
```

Redémarrer Moli (`docker restart moli-os`).

### 5. Choisir le modèle dans Système, régler l'effort

- **Page Système → Moli, l'assistant → « Qui réfléchit »** : Claude (Anthropic) ou OpenAI,
  puis le modèle parmi ceux proposés. Le code est demandé (cela change ce que Moli coûte) ; le
  choix est gardé dans `data/assistant.json`, appliqué sans redémarrer, et il **prime** sur
  `moli.toml`. Sous les boutons : le barème du modèle choisi et les échanges du jour avec leur
  coût estimé.
- **L'effort** se règle dans `moli.toml` (`effort`, `builder_effort` : `low`, `medium`,
  `high`, `xhigh`, `max`). `low` garde la conversation rapide ; monter `effort` rend Moli plus réfléchi
  mais plus lent et plus cher. Les modèles de `moli.toml` appartiennent à son fournisseur :
  passé à OpenAI depuis Système, ce sont les défauts d'OpenAI qui s'appliquent.

### 6. La voix : OpenAI, ou Whisper et Piper chez soi

Anthropic ne transcrit ni ne parle. Avec `provider = "anthropic"`, Moli choisit seul :

- **une clé OpenAI est dans le coffre** (`assistant/api_key`) → l'écoute et la voix passent
  par OpenAI comme avant (`transcribe_model`, `speech_model`, `speech_voice` inchangés), vers
  `base_url_voice`. Claude réfléchit, OpenAI écoute et parle. Pour l'ajouter :

  ```sh
  read -rs V; printf %s "$V" | docker exec -i moli-os /moli-os secrets set assistant api_key; unset V
  ```

  ou, dans Système, la ligne « Clé OpenAI pour la voix (facultative) ».

- **pas de clé OpenAI** → **Whisper** (écoute) et **Piper** (voix) locaux, par le protocole
  Wyoming, déjà pris en charge : `local_listen` (défaut `127.0.0.1:10300`) et `local_speech`
  (défaut `127.0.0.1:10200`, voix `local_voice`, défaut `fr_FR-siwis-medium`). Par exemple, à
  côté de Moli dans le même `docker-compose.yml` (Moli est en `network_mode: host`) :

  ```yaml
    whisper:
      image: rhasspy/wyoming-whisper
      command: --model small-int8 --language fr
      ports: ["127.0.0.1:10300:10300"]
      volumes: ["./whisper:/data"]
      restart: unless-stopped
    piper:
      image: rhasspy/wyoming-piper
      command: --voice fr_FR-siwis-medium
      ports: ["127.0.0.1:10200:10200"]
      volumes: ["./piper:/data"]
      restart: unless-stopped
  ```

  Pour forcer le local même avec une clé OpenAI : `transcribe_model = ""` et
  `speech_model = ""`.

- **ni l'un ni l'autre** → pas de conversation à voix haute ; l'écrit marche. La carte
  Système dit dans lequel de ces trois cas on est.

### 7. Vérifier

1. **Système → Moli, l'assistant** : « Clé Anthropic enregistrée », « Qui réfléchit » sur
   Claude, le modèle attendu, et la ligne de la voix (« … passent par OpenAI » ou « … Whisper
   et Piper, sur ta machine »).
2. **Une conversation** : « Quelle température fait-il dans le salon ? », puis un ordre sur
   une cible neutre (une lampe d'une pièce non protégée) : il passe sous la garde comme avec
   OpenAI.
3. **Un brouillon d'automatisme** : « Quand la porte d'entrée s'ouvre après 23 h, envoie-moi
   un message » → un brouillon apparaît, rien n'est actif tant qu'un humain ne l'a pas validé.
4. **La voix** : le micro du tableau de bord (en HTTPS) ; Moli répond à voix haute.
5. **Le coût** : « Échanges avec Moli » montre le coût estimé de chaque échange ; le journal
   en `debug` montre `claude answered` avec les jetons lus et écrits dans le cache.

### Revenir en arrière

Retirer `provider = "anthropic"` (ou choisir OpenAI dans Système) : Moli repart sur OpenAI avec
`assistant/api_key`. La clé Anthropic peut rester dans le coffre, elle ne sert plus.

## Vérifié sur une installation réelle

Le 18 octobre 2026, sur une installation Moli en service dans une maison habitée
<!-- À COMPLÉTER : la machine (ex. « mini-PC x86_64, Linux, Docker »), sans rien d'identifiant -->,
image construite depuis ces commits :

- conversation écrite et parlée avec **`claude-haiku-5-5`** (lecture de l'inventaire, ordres
  sous la garde, réponses courtes à l'oral) ;
- brouillons d'automatismes avec **`claude-opus-5-5`** (`builder_effort = "high"`) : c'est là
  qu'ont été trouvées et corrigées les deux limites du mode strict (commits 2 et 3) ;
- voix **locale** : Whisper pour l'écoute, Piper pour la voix (sans clé OpenAI) ;
- la clé posée par un tube, le modèle choisi depuis Système (code demandé), le coût du jour
  affiché.

Contrôles : `sh scripts/check.sh` (fmt, tests de tout l'espace de travail, catalogue,
anti-fuite) ; `cd web && npm ci && npm run build` ; `node scripts/i18n-check.mjs` (0 texte en
dur). Des tests unitaires couvrent la traduction aller-retour (`claude.rs` : outils parallèles,
blocs de réflexion, `refusal`, `max_tokens`, `pause_turn`, recherche web, schémas stricts et
leurs limites), le choix du fournisseur, les clés et les coûts.

## Limites connues

- **Pas de streaming** : la réponse arrive entière (comme aujourd'hui avec OpenAI).
- **L'effort ne se règle pas depuis Système**, seulement dans `moli.toml` (le modèle, lui, se
  choisit dans Système).
- **Le barème est écrit en dur** dans `pricing.rs` (tarifs publics d'octobre 2026) : les coûts
  affichés sont des estimations, la facture exacte est sur la console d'Anthropic.
- **La liste des modèles proposés dans Système** est celle des modèles Claude actuels (plus
  celui de `moli.toml`) ; un nouveau modèle se met dans `moli.toml` en attendant.
- **Le brouillon d'automatisme** dépasse les limites du mode strict d'Anthropic : son schéma va
  dans les consignes plutôt qu'être imposé ; Moli relit et valide le brouillon comme avant (un
  brouillon invalide est refusé, jamais activé).
- **Le repli côté serveur** (`fallbacks`) n'est envoyé que vers l'API d'Anthropic elle-même,
  pas derrière un autre `base_url`, ni pour Haiku.
- Anthropic n'a **pas de voix** : sans clé OpenAI ni Whisper/Piper, Moli ne parle qu'à l'écrit.

## Checked

- [x] `sh scripts/check.sh` (fmt, clippy, tests, catalogue, leak check)
- [x] `npm run build` in `web/` and `node scripts/i18n-check.mjs`
- [x] New texts in `locales/fr` **and** `locales/en`
- [x] No personal data (names, addresses, MACs, serials, keys): fake values only
- [x] Tried on real hardware? Which model, firmware, date (in the integration's README) — pas
  une intégration : essayé sur une installation réelle le 18 octobre 2026 (voir plus haut)
