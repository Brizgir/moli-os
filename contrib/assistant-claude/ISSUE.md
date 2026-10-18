# L'assistant de Moli fonctionne aussi avec Claude (API Messages d'Anthropic, en natif)

*In English: a proposal to let Moli, the assistant, think with Claude through Anthropic's
Messages API, natively (no SDK, no OpenAI-compatibility layer), next to the OpenAI-compatible
path that stays the default. The tool loop, the guard and the safety nets stay shared; the voice
keeps using OpenAI's key, or local Whisper and Piper. Nothing changes for a house that does not
opt in. A pull request is ready; this issue is to agree on the "what" first, as CONTRIBUTING asks
for a large change.*

## Le besoin

Aujourd'hui, Moli ne parle qu'à un modèle **compatible OpenAI** (OpenAI, Mistral, un Ollama
local…). Une maison qui préfère Claude — pour la qualité des automatismes dessinés, pour le suivi
des consignes de la garde, ou simplement parce qu'elle a déjà un compte Anthropic — n'a pas de
chemin propre :

- l'API d'Anthropic n'est pas compatible OpenAI sur ce qui compte ici : appels d'outils
  parallèles, blocs de réflexion à renvoyer tels quels, `stop_reason` (`refusal`,
  `max_tokens`, `pause_turn`), cache du prompt, recherche web côté serveur, sortie JSON ;
- une passerelle de traduction (proxy « OpenAI → Anthropic ») ajouterait un service à faire
  tourner et perdrait justement ces nuances (un refus traité comme une réponse vide, des appels
  exécutés après une réponse coupée…).

## La proposition

Un second fournisseur, **natif**, choisi dans `[assistant]` ou depuis la page Système :

```toml
[assistant]
provider = "anthropic"          # "anthropic" | "openai" (défaut : openai)
model = "claude-opus-5-5"       # ou claude-sonnet-5-5, claude-haiku-5-5, claude-fable-5-1
builder_model = "claude-opus-5-5"
effort = "low"                  # conversation
builder_effort = "high"         # brouillons d'automatismes
```

- **`crates/moli-assistant/src/claude.rs`** (nouveau) traduit à l'aller et au retour la
  conversation que Moli construit déjà (messages et outils au format OpenAI) vers
  `POST /v1/messages`. La boucle d'outils, la garde, les limites d'appels et les filets de
  sécurité restent **un seul code** pour les deux fournisseurs.
- **Clé à part dans le coffre** : `assistant/anthropic_api_key`, posée par un tube
  (`moli-os secrets set assistant anthropic_api_key`) ou depuis Système (humain, code demandé,
  vérifiée par `GET /v1/models` avant d'être rangée). La clé OpenAI (`assistant/api_key`) reste
  ce qu'elle est.
- **La voix** : Anthropic ne transcrit ni ne parle. Avec Claude, l'écoute et la voix passent par
  la clé OpenAI si elle est là, sinon par Whisper et Piper locaux (déjà pris en charge). La carte
  Système le dit clairement.
- **Page Système** : « Qui réfléchit » (Claude ou OpenAI) et le modèle, avec le barème du modèle
  choisi et le coût estimé du jour. Changer de fournisseur ou de modèle change ce que Moli coûte :
  c'est un geste humain (code demandé), gardé dans `data/assistant.json`.
- **Coûts** : `pricing.rs` connaît les modèles Claude actuels, le cache relu et écrit, et la
  recherche web (0,01 $).

## Choix d'architecture

- **Pas de SDK** : il n'y a pas de SDK Rust officiel ; du JSON (`serde_json`) sur le client
  HTTP que Moli utilise déjà pour OpenAI (`llm.rs`). Aucune nouvelle dépendance, `Cargo.lock` inchangé.
- **Sans streaming** (`max_tokens` 16 000) : Moli parle par phrases entières, comme aujourd'hui.
- **Cache du prompt** : le système est découpé en trois blocs (consignes stables, la maison, le
  moment) avec deux points de cache ; la date et l'heure sont après le dernier, pour ne pas
  invalider le cache à chaque minute.
- **Réflexion adaptative** : `thinking` n'est jamais envoyé ; la profondeur se règle par
  `output_config.effort` (`low` par défaut pour la latence d'une maison, `high` pour un
  brouillon d'automatisme).
- **Outils en mode strict** (`strict: true`), schémas ramenés à ce que ce mode accepte. Au-delà
  des limites d'Anthropic (16 paramètres à plusieurs types, 24 facultatifs), le schéma passe en
  consigne au lieu d'être imposé — c'est le cas du brouillon d'automatisme, trouvé à l'essai.
- **Recherche web** par l'outil serveur d'Anthropic (`web_search`), pour la même question
  « sur le monde » que l'outil web d'OpenAI aujourd'hui.

## Sécurité

- **Jamais de LLM dans le chemin d'exécution** : rien ne change ; Claude crée et explique, le
  moteur reste déterministe, un automatisme reste un brouillon jusqu'à la validation d'un humain.
- **`stop_reason` lu avant le contenu** : `refusal` → aucun appel de la réponse n'est exécuté ;
  `max_tokens` → les mots arrivés sont dits, aucun appel n'est exécuté.
- **Pas de `tool_choice` forcé, pas de réponse pré-remplie** : le prompt dit quand appeler un
  outil, la garde décide.
- **La clé** : chiffrée dans le coffre, jamais dans `moli.toml`, jamais au journal ni renvoyée
  par l'API ; envoyée seulement en https (un `base_url` en `http://` ne la reçoit jamais) ;
  posée ou remplacée seulement par une personne (code).
- **Choisir le modèle** est un geste humain (code), comme donner une clé : un agent ne peut pas
  faire passer la maison sur un modèle plus cher.
- Ce que Claude voit est ce que voit déjà le modèle OpenAI : ni image de caméra, ni secret, ni
  position précise.

## Ce qui ne change pas pour les autres maisons

- **Sans `provider`, c'est OpenAI**, comme aujourd'hui (sauf si `base_url` pointe déjà vers
  `api.anthropic.com`). Un `moli.toml` existant se comporte exactement pareil.
- La clé OpenAI garde son nom (`assistant/api_key`), la voix ses réglages, Whisper/Piper leur
  rôle de repli.
- Les modèles OpenAI « à raisonnement », le modèle local en `http://`, l'import des
  automatismes de Home Assistant : inchangés.
- Seul changement visible pour une maison OpenAI : la carte Système montre en plus « Qui
  réfléchit », le barème et le coût du jour, et quelques textes disent « crédit » au lieu de
  « crédit OpenAI ».

Une pull request est prête (trois commits, testée sur une installation réelle) ; je l'ouvre dès
qu'on est d'accord sur le principe.
