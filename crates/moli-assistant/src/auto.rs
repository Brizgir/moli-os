//! Moli and the automations: drafting a graph from words (or reworking
//! one), and writing the texts « Moli écrit » nodes ask for.
//!
//! Drafts are only drafts: a person approves them before anything runs.
//! Writing is words only: the engine never lets a model decide.

use moli_automation::{Actor, Author, Automation, Automations, Graph, Intent, Mode, Node};
use moli_runtime::BoxFuture;
use serde::Deserialize;
use serde_json::{Value as Json, json};

use crate::claude::{self, Ask};
use crate::house::{self, Layout};
use crate::{Assistant, AssistantError, OpenAiKnobs, Provider, pricing};

/// What Moli proposes.
#[derive(Clone, Debug)]
pub struct Draft {
    pub name: String,
    pub mode: Mode,
    pub graph: Graph,
    /// What Moli could not do as asked, in words.
    pub note: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Flat {
    name: String,
    mode: Mode,
    note: Option<String>,
    nodes: Vec<Json>,
    edges: Vec<moli_automation::Edge>,
}

fn nullable(t: &str) -> Json {
    json!({ "type": [t, "null"] })
}

fn any_value() -> Json {
    json!({ "type": ["string", "number", "boolean", "null"] })
}

fn rule_schema() -> Json {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["kind", "point", "op", "value", "after", "before", "days", "is"],
        "properties": {
            "kind": { "type": "string", "enum": ["state", "time", "sun"] },
            "point": nullable("string"),
            "op": { "type": ["string", "null"], "enum": ["eq", "ne", "gt", "ge", "lt", "le", null] },
            "value": any_value(),
            "after": nullable("string"),
            "before": nullable("string"),
            "days": { "type": ["array", "null"], "items": { "type": "integer" } },
            "is": { "type": ["string", "null"], "enum": ["day", "night", null] }
        }
    })
}

fn schema() -> Json {
    let fields = [
        "id",
        "type",
        "point",
        "value",
        "to",
        "from",
        "for_s",
        "above",
        "below",
        "at",
        "days",
        "event",
        "offset_min",
        "minutes",
        "seconds",
        "timeout_s",
        "all",
        "rules",
        "rule",
        "title",
        "message",
        "channels",
        "prompt",
    ];
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["name", "mode", "note", "nodes", "edges"],
        "properties": {
            "name": { "type": "string" },
            "mode": { "type": "string", "enum": ["restart", "single", "queued"] },
            "note": nullable("string"),
            "nodes": { "type": "array", "items": {
                "type": "object",
                "additionalProperties": false,
                "required": fields,
                "properties": {
                    "id": { "type": "string" },
                    "type": { "type": "string", "enum": [
                        "when_state", "when_threshold", "at_time", "at_sun", "every", "on_start", "manual",
                        "if", "set", "toggle", "wait", "wait_for", "notify", "write"
                    ] },
                    "point": nullable("string"),
                    "value": any_value(),
                    "to": any_value(),
                    "from": any_value(),
                    "for_s": nullable("integer"),
                    "above": nullable("number"),
                    "below": nullable("number"),
                    "at": nullable("string"),
                    "days": { "type": ["array", "null"], "items": { "type": "integer" } },
                    "event": { "type": ["string", "null"], "enum": ["dawn", "rise", "set", "dusk", null] },
                    "offset_min": nullable("integer"),
                    "minutes": nullable("integer"),
                    "seconds": nullable("integer"),
                    "timeout_s": nullable("integer"),
                    "all": nullable("boolean"),
                    "rules": { "type": ["array", "null"], "items": rule_schema() },
                    "rule": { "anyOf": [rule_schema(), { "type": "null" }] },
                    "title": nullable("string"),
                    "message": nullable("string"),
                    "channels": { "type": ["array", "null"], "items": { "type": "string", "enum": ["maison", "voix", "telephone", "telegram"] } },
                    "prompt": nullable("string")
                }
            } },
            "edges": { "type": "array", "items": {
                "type": "object",
                "additionalProperties": false,
                "required": ["from", "port", "to"],
                "properties": {
                    "from": { "type": "string" },
                    "port": { "type": "string", "enum": ["out", "yes", "no", "ok", "timeout"] },
                    "to": { "type": "string" }
                }
            } }
        }
    })
}

/// Nulls out: the engine's format has optional fields, not null ones.
fn strip_nulls(v: &Json) -> Json {
    match v {
        Json::Object(map) => Json::Object(
            map.iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k.clone(), strip_nulls(v)))
                .collect(),
        ),
        Json::Array(items) => Json::Array(items.iter().map(strip_nulls).collect()),
        other => other.clone(),
    }
}

impl Assistant {
    /// Drafts an automation from words, or reworks `current`. Validated
    /// against the house; one correction round if the checker objects.
    pub async fn draft_automation(
        &self,
        autos: &Automations,
        request: &str,
        current: Option<&Automation>,
    ) -> Result<Draft, AssistantError> {
        // What is reworked comes from the client: bounded like a saved
        // automation before it reaches the prompt.
        let current = current
            .cloned()
            .map(|mut a| {
                a.name = moli_automation::tidy_name(&a.name);
                moli_automation::tidy(&mut a.graph).map(|()| a)
            })
            .transpose()
            .map_err(|e| AssistantError::Invalid(e.to_string()))?;
        let key = self.key()?;
        let _permit = self.admit()?;
        let request: String = request.chars().take(crate::MAX_MESSAGE_CHARS).collect();
        // Not an exchange with Moli: its cost is not kept.
        self.draft_with(key.as_deref(), autos, &request, current.as_ref(), &mut 0.0)
            .await
    }

    /// Drafting, inside an admitted turn (the conversation's tool). What the
    /// builder model's calls cost is added to `spent` (dollars), failed
    /// drafts included.
    pub(crate) async fn draft_with(
        &self,
        key: Option<&str>,
        autos: &Automations,
        request: &str,
        current: Option<&Automation>,
        spent: &mut f64,
    ) -> Result<Draft, AssistantError> {
        // Imports carry a whole YAML; words typed by a person are capped
        // earlier (2 000 characters).
        let request: String = request.trim().chars().take(12_000).collect();
        if request.is_empty() {
            return Err(AssistantError::Invalid(
                "say what the automation should do".into(),
            ));
        }
        let layout = Layout::load(self.0.layout_path.as_deref()).await;
        let snapshot = self.0.hub.snapshot();
        let now = jiff::Timestamp::now().to_zoned(self.0.tz.clone());
        let house_now = builder_context(
            &crate::house_date(&now),
            &snapshot.guard.protected_rooms.join(", "),
            &house::inventory(
                &snapshot.devices,
                &layout,
                &snapshot.guard.protected_rooms,
                house::Reader::Builder,
            ),
            &self.meters().await,
        );
        let guide = moli_i18n::tr!("assistant.auto.guide");
        let user = request_message(&request, current);
        let brain = self.brain();
        // The guide is long and never changes: Claude keeps it in its cache.
        let mut messages = vec![
            json!({ "role": "system", "content": format!("{guide}\n\n{house_now}"),
                    "blocks": [{ "text": guide, "cache": true }, { "text": house_now }] }),
            json!({ "role": "user", "content": user }),
        ];
        let schema = schema();
        let mut last_problems = String::new();
        for attempt in 0..2 {
            let (content, said) = self
                .draft_round(key, &brain, &messages, &schema, spent)
                .await?;
            let draft = match parse(&content, current) {
                Ok(d) => d,
                Err(e) => {
                    last_problems = e;
                    messages.push(said);
                    messages.push(json!({ "role": "user", "content": moli_i18n::tr!("assistant.auto.unreadable", problems = last_problems) }));
                    continue;
                }
            };
            let check = autos.check(&draft.graph);
            let errors: Vec<String> = check
                .problems
                .iter()
                // Errors, and loose ends: a step nothing leads to is a
                // forgotten link.
                .filter(|p| p.level == moli_automation::Level::Error || p.is_unreached())
                .map(|p| {
                    format!(
                        "{}{}",
                        p.node
                            .as_deref()
                            .map(|n| format!("[{n}] "))
                            .unwrap_or_default(),
                        p.message
                    )
                })
                .collect();
            if errors.is_empty() || attempt == 1 {
                tracing::info!(
                    nodes = draft.graph.nodes.len(),
                    errors = errors.len(),
                    "automation drafted"
                );
                return Ok(draft);
            }
            last_problems = errors.join(" ; ");
            messages.push(said);
            messages.push(json!({ "role": "user", "content": moli_i18n::tr!("assistant.auto.refused", problems = last_problems) }));
        }
        Err(AssistantError::Upstream(moli_i18n::tr!(
            "assistant.auto.unusable",
            problems = last_problems
        )))
    }

    /// One round trip of a draft: the answer's text, and the message that
    /// keeps it in the conversation (Claude's blocks as they came, its
    /// thinking too). What it cost is added to `spent`.
    async fn draft_round(
        &self,
        key: Option<&str>,
        brain: &crate::Brain,
        messages: &[Json],
        schema: &Json,
        spent: &mut f64,
    ) -> Result<(String, Json), AssistantError> {
        let model = &brain.builder_model;
        let answer = self
            .complete(
                key,
                brain.provider,
                &Ask {
                    model,
                    messages,
                    tools: None,
                    last: false,
                    effort: &self.0.config.builder_effort,
                    schema: Some(schema),
                },
                OpenAiKnobs {
                    reasoning_tokens: 8_000,
                    temperature: 0.2,
                    max_tokens: 3_000,
                },
                claude::BUILD_LIMIT,
            )
            .await?;
        *spent += pricing::chat(model, &answer["usage"]);
        if answer.get("refused").is_some() {
            return Err(AssistantError::Upstream(moli_i18n::tr!(
                "assistant.reply.refused"
            )));
        }
        let message = &answer["choices"][0]["message"];
        let content = message["content"].as_str().unwrap_or_default().to_owned();
        let said = if brain.provider == Provider::Anthropic {
            json!({ "role": "assistant", "content": content, "claude": message["claude"] })
        } else {
            json!({ "role": "assistant", "content": content })
        };
        Ok((content, said))
    }

    /// A text for a « Moli écrit » node: words for people, from the house's
    /// current state. Never a decision.
    pub async fn compose(&self, prompt: &str) -> Result<String, AssistantError> {
        let key = self.key()?;
        // Not counted against the conversation: automations are limited on
        // their own (20 runs a minute), and a busy chat must not silence them.

        let layout = Layout::load(self.0.layout_path.as_deref()).await;
        let snapshot = self.0.hub.snapshot();
        let now = jiff::Timestamp::now().to_zoned(self.0.tz.clone());
        let power = self.power_now().await;
        let system = compose_prompt(
            &crate::house_date(&now),
            &power,
            &house::inventory(
                &snapshot.devices,
                &layout,
                &snapshot.guard.protected_rooms,
                house::Reader::Chat,
            ),
        );
        let brain = self.brain();
        let messages = [
            json!({ "role": "system", "content": system }),
            json!({ "role": "user", "content": prompt }),
        ];
        let answer = self
            .complete(
                key.as_deref(),
                brain.provider,
                &Ask {
                    model: &brain.model,
                    messages: &messages,
                    tools: None,
                    last: false,
                    effort: &self.0.config.effort,
                    schema: None,
                },
                OpenAiKnobs {
                    reasoning_tokens: 3_000,
                    temperature: 0.6,
                    max_tokens: 400,
                },
                crate::llm::CHAT_LIMIT,
            )
            .await?;
        if answer.get("refused").is_some() {
            return Err(AssistantError::Upstream(moli_i18n::tr!(
                "assistant.reply.refused"
            )));
        }
        Ok(answer["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .trim()
            .to_owned())
    }
}

/// What the drafting model is told after the guide: the house as it is
/// now. The inventory comes last among the words filled in (see `moli_i18n`).
fn builder_context(date: &str, protected: &str, inventory: &str, meters: &str) -> String {
    moli_i18n::tr!(
        "assistant.auto.context",
        date = date,
        protected = protected,
        meters = meters,
        inventory = inventory
    )
}

/// What writes the message of a « Moli écrit » node: who Moli is talking to,
/// the time, the live power, the house.
fn compose_prompt(date: &str, power: &str, inventory: &str) -> String {
    moli_i18n::tr!(
        "assistant.auto.compose",
        date = date,
        power = power,
        inventory = inventory
    )
}

/// What Moli is asked: the words, and the automation being reworked.
fn request_message(request: &str, current: Option<&Automation>) -> String {
    let Some(a) = current else {
        return moli_i18n::tr!("assistant.auto.request", request = request);
    };
    let flat: Vec<Json> = a
        .graph
        .nodes
        .iter()
        .map(|n| json!({ "id": n.id, "step": n.step }))
        .collect();
    moli_i18n::tr!(
        "assistant.auto.rework",
        mode = format!("{:?}", a.mode),
        nodes = Json::Array(flat),
        edges = serde_json::to_string(&a.graph.edges).unwrap_or_default(),
        name = a.name,
        request = request
    )
}

/// The JSON object in a model's words, when it wrapped it (a code fence, a
/// sentence before): from the first `{` to the last `}`.
fn json_object(content: &str) -> Option<&str> {
    let start = content.find('{')?;
    let end = content.rfind('}')?;
    (end > start).then(|| &content[start..=end])
}

/// The model's flat JSON → the engine's graph (canvas positions kept from
/// the current version for nodes that stay).
fn parse(content: &str, current: Option<&Automation>) -> Result<Draft, String> {
    let flat: Flat = serde_json::from_str(content)
        .or_else(|e| serde_json::from_str(json_object(content).ok_or(e)?))
        .map_err(|e| e.to_string())?;
    let mut nodes = Vec::new();
    for raw in &flat.nodes {
        let mut raw = strip_nulls(raw);
        // Words in the other field: a writer's prompt in `message`, a
        // message in `prompt`.
        match raw["type"].as_str() {
            Some("write") if raw.get("prompt").is_none() => {
                if let Some(m) = raw.get("message").cloned() {
                    raw["prompt"] = m;
                }
            }
            Some("notify") if raw.get("message").is_none() => {
                if let Some(p) = raw.get("prompt").cloned() {
                    raw["message"] = p;
                }
            }
            _ => {}
        }
        let mut node: Node = serde_json::from_value(raw.clone()).map_err(|e| {
            moli_i18n::tr!(
                "assistant.auto.node_error",
                error = e,
                id = raw["id"].as_str().unwrap_or("?")
            )
        })?;
        if let Some(old) = current.and_then(|a| a.graph.node(&node.id)) {
            node.x = old.x;
            node.y = old.y;
        }
        nodes.push(node);
    }
    Ok(Draft {
        name: flat.name.trim().chars().take(80).collect(),
        mode: flat.mode,
        graph: Graph {
            nodes,
            edges: flat.edges,
        },
        note: flat.note.filter(|n| !n.trim().is_empty()),
    })
}

/// The electricity meters by their meaning, one line each (the names the
/// house gave them come last among the words filled in).
fn meters_text(s: &moli_energy::Summary) -> String {
    let mut out = moli_i18n::tr!("assistant.auto.meters_head");
    for m in &s.meters {
        let role = match m.role {
            moli_energy::Role::Total => moli_i18n::tr!("assistant.auto.role_total"),
            moli_energy::Role::Grid => moli_i18n::tr!("assistant.auto.role_grid"),
            moli_energy::Role::Circuit => moli_i18n::tr!("assistant.auto.role_circuit"),
            moli_energy::Role::Appliance => moli_i18n::tr!("assistant.auto.role_appliance"),
        };
        let line = match &m.power {
            Some(power) => moli_i18n::tr!(
                "assistant.auto.meter_power",
                role = role,
                power = power,
                name = m.name
            ),
            None => moli_i18n::tr!(
                "assistant.auto.meter_energy",
                role = role,
                point = m.point,
                name = m.name
            ),
        };
        out.push_str(&line);
        out.push('\n');
    }
    out
}

impl Assistant {
    /// The electricity meters by their meaning: what « the consumption » is,
    /// for the builder.
    async fn meters(&self) -> String {
        let Some(energy) = &self.0.energy else {
            return String::new();
        };
        let Ok(s) = energy.summary().await else {
            return String::new();
        };
        meters_text(&s)
    }

    pub fn set_automations(&self, automations: Automations) {
        *self
            .0
            .automations
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(automations);
    }

    pub(crate) fn automations(&self) -> Option<Automations> {
        self.0
            .automations
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// The conversation's `automation` tool: a draft saved for a person to
    /// approve, shown as a card.
    pub(crate) async fn automation_tool(&self, args: &Json, run: &mut crate::Run) -> Json {
        let Some(autos) = self.automations() else {
            return json!({ "error": moli_i18n::tr!("assistant.auto.not_running") });
        };
        let Some(request) = args["request"].as_str() else {
            return json!({ "error": "request is required" });
        };
        let key = match self.key() {
            Ok(k) => k,
            Err(e) => return json!({ "error": e.to_string() }),
        };
        if run.drafts == crate::MAX_DRAFTS {
            return crate::over_budget("automation", crate::MAX_DRAFTS);
        }
        run.drafts += 1;
        let request: String = request.chars().take(crate::MAX_MESSAGE_CHARS).collect();
        let draft = match self
            .draft_with(key.as_deref(), &autos, &request, None, &mut run.draft_cost)
            .await
        {
            Ok(d) => d,
            Err(e) => return json!({ "error": e.to_string() }),
        };
        let a = Automation {
            id: String::new(),
            name: draft.name,
            enabled: false,
            mode: draft.mode,
            graph: draft.graph,
            author: moli_automation::Author::Assistant,
            approved: None,
            approved_version: None,
            note: draft.note.clone(),
            created: 0,
            updated: 0,
        };
        let by = moli_automation::Actor {
            human: false,
            author: moli_automation::Author::Assistant,
        };
        match autos.save(a, by).await {
            Ok(saved) => {
                let check = autos.check(&saved.graph);
                run.cards
                    .push(json!({ "kind": "automation", "id": saved.id }));
                let problems: Vec<&str> =
                    check.problems.iter().map(|p| p.message.as_str()).collect();
                json!({
                    "status": moli_i18n::tr!("assistant.auto.saved"),
                    "summary": check.summary,
                    "problems": problems,
                    "note": draft.note,
                })
            }
            Err(e) => json!({ "error": e.to_string() }),
        }
    }
}

/// How many automations the model is shown at once (the most recently
/// changed first), and how long each sentence.
const LISTED: usize = 120;
const LISTED_SUMMARY: usize = 200;

/// A name as people say it: lower case, no accents, words only.
fn folded(name: &str) -> String {
    let mut out = String::new();
    for c in name.to_lowercase().chars() {
        let c = match c {
            'à' | 'â' | 'ä' | 'á' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' | 'í' => 'i',
            'ô' | 'ö' | 'ó' => 'o',
            'ù' | 'û' | 'ü' | 'ú' => 'u',
            'ç' => 'c',
            c => c,
        };
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with(' ') {
            out.push(' ');
        }
    }
    out.trim_end().to_owned()
}

/// The automation someone means: its id, its name, or the only one whose
/// name holds the words. Never a guess between two.
pub(crate) fn find_automation(list: &[Automation], wanted: &str) -> Result<Automation, String> {
    let wanted = wanted.trim();
    if let Some(a) = list.iter().find(|a| a.id == wanted) {
        return Ok(a.clone());
    }
    let w = folded(wanted);
    if w.is_empty() {
        return Err(moli_i18n::tr!("assistant.auto.unknown", name = wanted));
    }
    let exact: Vec<&Automation> = list.iter().filter(|a| folded(&a.name) == w).collect();
    let found = if exact.is_empty() {
        list.iter()
            .filter(|a| {
                let n = folded(&a.name);
                !n.is_empty() && (n.contains(&w) || w.contains(&n))
            })
            .collect()
    } else {
        exact
    };
    match found.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(moli_i18n::tr!("assistant.auto.unknown", name = wanted)),
        many => Err(moli_i18n::tr!(
            "assistant.auto.ambiguous",
            names = many
                .iter()
                .map(|a| format!("« {} »", a.name))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// The `automations` tool: what the house has, to find the one asked about.
pub(crate) fn automations_tool() -> Json {
    json!({
        "type": "function",
        "function": {
            "name": "automations",
            "description": moli_i18n::tr!("assistant.tools.automations.description"),
            "parameters": { "type": "object", "properties": {}, "required": [] }
        }
    })
}

/// The `change_automation` tool: a change to an existing automation,
/// proposed for a person to confirm.
pub(crate) fn change_automation_tool() -> Json {
    json!({
        "type": "function",
        "function": {
            "name": "change_automation",
            "description": moli_i18n::tr!("assistant.tools.change_automation.description"),
            "parameters": {
                "type": "object",
                "properties": {
                    "automation": { "type": "string", "description": moli_i18n::tr!("assistant.tools.change_automation.automation") },
                    "action": { "type": "string", "enum": ["update", "pause", "resume", "delete", "run"], "description": moli_i18n::tr!("assistant.tools.change_automation.action") },
                    "request": { "type": "string", "description": moli_i18n::tr!("assistant.tools.change_automation.request") }
                },
                "required": ["automation", "action", "request"]
            }
        }
    })
}

impl Assistant {
    /// The `automations` tool: each automation, its state and what it does
    /// (the checker's sentence, never the model's).
    pub(crate) fn automations_list(&self) -> Json {
        let Some(autos) = self.automations() else {
            return json!({ "error": moli_i18n::tr!("assistant.auto.not_running") });
        };
        let can = autos.abilities();
        let mut all = autos.list_fingerprinted();
        all.sort_by_key(|(a, _)| std::cmp::Reverse(a.updated));
        let items: Vec<Json> = all
            .iter()
            .take(LISTED)
            .map(|(a, fingerprint)| {
                let check = autos.check_with(&a.graph, can);
                let approved = a.approved.as_deref() == Some(fingerprint.as_str());
                let state = if a.enabled && approved {
                    "live"
                } else if !check.ok() {
                    "to_fix"
                } else if approved {
                    "off"
                } else {
                    "draft"
                };
                json!({
                    "id": a.id,
                    "name": a.name,
                    "state": state,
                    "summary": check.summary.chars().take(LISTED_SUMMARY).collect::<String>(),
                    "protected": check.protected,
                })
            })
            .collect();
        json!({ "automations": items, "total": all.len() })
    }

    /// The `change_automation` tool: change, pause, resume, delete or run an
    /// existing automation. Nothing happens here: a proposal waits for a
    /// person to confirm it on the card (the code when there is no
    /// session). A spoken « yes » confirms nothing.
    pub(crate) async fn change_automation(&self, args: &Json, run: &mut crate::Run) -> Json {
        let Some(autos) = self.automations() else {
            return json!({ "error": moli_i18n::tr!("assistant.auto.not_running") });
        };
        let Some(wanted) = args["automation"].as_str() else {
            return json!({ "error": "automation is required" });
        };
        let current = match find_automation(&autos.list(), wanted) {
            Ok(a) => a,
            Err(why) => return json!({ "error": why }),
        };
        let mut note = None;
        let intent = match args["action"].as_str().unwrap_or_default() {
            "pause" => Intent::Pause,
            "resume" => Intent::Resume,
            "delete" => Intent::Delete,
            "run" => Intent::Run,
            "update" => {
                let request = args["request"].as_str().map(str::trim).unwrap_or_default();
                if request.is_empty() {
                    return json!({ "error": moli_i18n::tr!("assistant.auto.say_what") });
                }
                let key = match self.key() {
                    Ok(k) => k,
                    Err(e) => return json!({ "error": e.to_string() }),
                };
                // Drafting costs the builder model: one a turn, new or reworked.
                if run.drafts == crate::MAX_DRAFTS {
                    return crate::over_budget("change_automation", crate::MAX_DRAFTS);
                }
                run.drafts += 1;
                let request: String = request.chars().take(crate::MAX_MESSAGE_CHARS).collect();
                let draft = match self
                    .draft_with(
                        key.as_deref(),
                        &autos,
                        &request,
                        Some(&current),
                        &mut run.draft_cost,
                    )
                    .await
                {
                    Ok(d) => d,
                    Err(e) => return json!({ "error": e.to_string() }),
                };
                note = draft.note;
                Intent::Update(Box::new(Automation {
                    name: draft.name,
                    mode: draft.mode,
                    graph: draft.graph,
                    ..current.clone()
                }))
            }
            other => {
                return json!({ "error": format!("action {other:?}: update, pause, resume, delete or run") });
            }
        };
        let by = Actor {
            human: false,
            author: Author::Assistant,
        };
        match autos.propose(&current.id, intent, by) {
            Ok(p) => {
                let card = json!({ "kind": "automation", "id": p.automation });
                if !run.cards.contains(&card) {
                    run.cards.push(card);
                }
                json!({
                    "status": moli_i18n::tr!("assistant.auto.proposed"),
                    "change": p.kind,
                    "name": p.name,
                    "summary": p.summary,
                    "protected": p.protected,
                    "note": note,
                })
            }
            Err(e) => json!({ "error": e.to_string() }),
        }
    }
}

impl moli_automation::Writer for Assistant {
    fn write(&self, prompt: String) -> BoxFuture<'_, anyhow::Result<String>> {
        Box::pin(async move {
            self.compose(&prompt)
                .await
                .map_err(|e| anyhow::anyhow!(e.to_string()))
        })
    }

    /// What the model never reads, in a prompt either (`house::private`).
    fn withholds(&self, key: &str) -> bool {
        house::private(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_drafts_become_graphs() {
        let content = json!({
            "name": "Entrée la nuit", "mode": "restart", "note": null,
            "nodes": [
                { "id": "t1", "type": "when_state", "point": "z:1/contact", "value": null, "to": false, "from": null, "for_s": 0,
                  "above": null, "below": null, "at": null, "days": null, "event": null, "offset_min": null, "minutes": null,
                  "seconds": null, "timeout_s": null, "all": null, "rules": null, "rule": null, "title": null, "message": null,
                  "channels": null, "prompt": null },
                { "id": "c1", "type": "if", "rules": [{ "kind": "sun", "is": "night", "point": null, "op": null, "value": null,
                  "after": null, "before": null, "days": null }], "all": true, "point": null },
                { "id": "a1", "type": "set", "point": "hue:2/on", "value": true }
            ],
            "edges": [{ "from": "t1", "port": "out", "to": "c1" }, { "from": "c1", "port": "yes", "to": "a1" }]
        })
        .to_string();
        let d = parse(&content, None).unwrap();
        assert_eq!(d.graph.nodes.len(), 3);
        assert_eq!(d.graph.nodes[0].step.kind(), "when_state");
        assert_eq!(d.graph.next("c1", "yes")[0].id, "a1");
        assert!(d.note.is_none());
    }

    /// A « Moli écrit » prompt never carries a position or a Wi-Fi either.
    #[test]
    fn the_writer_withholds_what_the_model_never_reads() {
        let hub = moli_runtime::Hub::new(moli_runtime::HubOptions::default()).unwrap();
        let moli = Assistant::new(hub, None, None, None, crate::Config::default()).unwrap();
        let writer: &dyn moli_automation::Writer = &moli;
        for key in ["latitude", "longitude", "accuracy", "wifi"] {
            assert!(writer.withholds(key), "{key}");
        }
        for key in ["zone", "home", "battery", "on"] {
            assert!(!writer.withholds(key), "{key}");
        }
    }

    #[test]
    fn the_schema_lists_every_field_as_required() {
        let s = schema();
        let node = &s["properties"]["nodes"]["items"];
        let required = node["required"].as_array().unwrap().len();
        let properties = node["properties"].as_object().unwrap().len();
        assert_eq!(
            required, properties,
            "strict mode needs every property required"
        );
    }

    /// The draft's schema is past Claude's strict limits (24 optional, 16
    /// unions): it goes to Claude in words, not as `output_config.format`,
    /// and a draft wrapped in a fence or a sentence still reads.
    #[test]
    fn the_draft_goes_to_claude_in_words_and_a_wrapped_answer_reads() {
        let strict = crate::claude::strict_schema(&schema());
        assert!(!crate::claude::fits_strict(&strict));
        let messages = [json!({ "role": "user", "content": "lampe" })];
        let schema = schema();
        let ask = crate::claude::Ask {
            model: "claude-opus-5-5",
            messages: &messages,
            tools: None,
            last: false,
            effort: "high",
            schema: Some(&schema),
        };
        let (body, _) = crate::claude::request(&ask, false);
        assert!(body["output_config"].get("format").is_none());
        let words = body["system"].as_array().unwrap().last().unwrap()["text"]
            .as_str()
            .unwrap();
        assert!(words.starts_with("Answer with one JSON object only"));
        let wrapped = "Voici :\n```json\n{\"name\":\"Entrée\",\"mode\":\"restart\",\"nodes\":[],\"edges\":[]}\n```";
        assert_eq!(
            json_object(wrapped),
            Some("{\"name\":\"Entrée\",\"mode\":\"restart\",\"nodes\":[],\"edges\":[]}")
        );
    }

    use crate::before;

    #[test]
    fn the_french_drafting_prompts_are_what_they_always_were() {
        assert_eq!(moli_i18n::tr!("assistant.auto.guide"), before::GUIDE);
        let inventory = "## Salon\n- hue:1 · Lampe · on✎=true\n## Sans pièce\n- x:1 · Truc\n";
        let meters = "Électricité (pour « la consommation ») :\n- Maison (toute la maison) : puissance en W = t/p\n";
        let date = "samedi 3 octobre 2026, 21h05";
        for protected in ["Chambre des enfants, Bureau", ""] {
            assert_eq!(
                format!(
                    "{}\n\n{}",
                    moli_i18n::tr!("assistant.auto.guide"),
                    builder_context(date, protected, inventory, meters)
                ),
                before::builder_prompt(date, protected, inventory, meters)
            );
        }
        for power in ["Électricité en direct : 940 W.", ""] {
            assert_eq!(
                compose_prompt(date, power, inventory),
                before::compose_prompt(date, power, inventory)
            );
        }
    }

    #[test]
    fn the_french_request_is_what_it_always_was() {
        assert_eq!(
            request_message("allume le garage", None),
            before::request_message("allume le garage", None)
        );
        let current: Automation = serde_json::from_value(json!({
            "name": "Garage la nuit",
            "graph": { "nodes": [{ "id": "a1", "type": "set", "point": "hue:2/on", "value": true }] },
            "author": "human"
        }))
        .unwrap();
        assert_eq!(
            request_message("plus tard", Some(&current)),
            before::request_message("plus tard", Some(&current))
        );
    }

    #[test]
    fn the_french_meters_and_messages_are_what_they_always_were() {
        for full in [true, false] {
            let summary = crate::tests::sample_summary(full);
            assert_eq!(meters_text(&summary), before::meters(&summary));
        }
        assert_eq!(
            moli_i18n::tr!("assistant.auto.unreadable", problems = "x"),
            before::unreadable("x")
        );
        assert_eq!(
            moli_i18n::tr!("assistant.auto.refused", problems = "x ; y"),
            before::refused("x ; y")
        );
        assert_eq!(
            moli_i18n::tr!("assistant.auto.unusable", problems = "x"),
            before::unusable("x")
        );
        assert_eq!(
            moli_i18n::tr!("assistant.auto.not_running"),
            "les automatisations ne tournent pas"
        );
        assert_eq!(
            moli_i18n::tr!("assistant.auto.saved"),
            "brouillon enregistré : un adulte doit le valider (bouton sur la carte)"
        );
        let bad = json!({ "name": "x", "mode": "restart", "note": null,
            "nodes": [{ "id": "t1", "type": "nope" }], "edges": [] })
        .to_string();
        let why = parse(&bad, None).unwrap_err();
        assert!(why.starts_with("nœud t1 : "), "{why}");
    }

    /// The model is told the engine's own words (node types, ports,
    /// channels, tokens) in both languages: only the prose is translated.
    #[test]
    fn the_english_guide_keeps_the_engines_words() {
        let (fr, en) = (crate::tests::french(), crate::tests::english());
        let (fr, en) = (&fr["assistant.auto.guide"], &en["assistant.auto.guide"]);
        let s = schema();
        let node = &s["properties"]["nodes"]["items"]["properties"];
        let mut words: Vec<String> = node.as_object().unwrap().keys().cloned().collect();
        for list in [
            &node["type"]["enum"],
            &node["channels"]["items"]["enum"],
            &s["properties"]["edges"]["items"]["properties"]["port"]["enum"],
            &rule_schema()["properties"]["kind"]["enum"],
        ] {
            words.extend(
                list.as_array()
                    .unwrap()
                    .iter()
                    .map(|w| w.as_str().unwrap().to_owned()),
            );
        }
        words.extend(["{{texte}}", "{{heure}}", "restart", "single", "queued"].map(String::from));
        for word in &words {
            assert_eq!(
                fr.contains(word.as_str()),
                en.contains(word.as_str()),
                "{word}"
            );
        }
        assert!(
            en.contains("{{texte}}") && en.contains("when_state") && en.contains("\"telegram\"")
        );
        assert!(en.contains("message (English, short)") && en.contains("short, in English"));
    }

    // ---- Moli acts on the automations, a person decides -------------------

    const HUMAN: Actor = Actor {
        human: true,
        author: Author::Human,
    };
    const MOLI: Actor = Actor {
        human: false,
        author: Author::Assistant,
    };

    /// A door that reads, a hall light that obeys.
    async fn hall() -> (moli_runtime::Hub, Automations) {
        use moli_core::{Kind, Value};
        let level = Kind::Numeric {
            min: Some(0.0),
            max: Some(100.0),
            step: None,
        };
        let hub = crate::tests::hub_with(crate::tests::Fake(vec![
            (
                "porte",
                vec![crate::tests::point("contact", Kind::Binary, false)],
                vec![("contact", Value::Bool(true))],
            ),
            (
                "entree",
                vec![
                    crate::tests::point("on", Kind::Binary, true),
                    crate::tests::point("brightness", level, true),
                ],
                vec![("on", Value::Bool(false)), ("brightness", Value::Int(100))],
            ),
        ]))
        .await;
        let (autos, _engine) = Automations::start(
            &hub,
            moli_automation::Config::default(),
            None,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .unwrap();
        (hub, autos)
    }

    /// Moli over the hall, its provider answering `answers` in turn.
    async fn moli(
        hub: &moli_runtime::Hub,
        autos: &Automations,
        answers: Vec<String>,
    ) -> (Assistant, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let (url, asked) = crate::llm::tests::fake_provider(answers).await;
        let config: crate::Config = serde_json::from_value(json!({ "base_url": url })).unwrap();
        let moli = Assistant::new(hub.clone(), None, None, None, config).unwrap();
        moli.set_automations(autos.clone());
        (moli, asked)
    }

    fn calls(calls: &[(&str, Json)]) -> String {
        let calls: Vec<Json> = calls
            .iter()
            .enumerate()
            .map(|(i, (name, args))| {
                json!({ "id": format!("c{i}"), "type": "function",
                    "function": { "name": name, "arguments": args.to_string() } })
            })
            .collect();
        json!({ "choices": [{ "message": { "role": "assistant", "content": null, "tool_calls": calls } }],
            "usage": { "prompt_tokens": 100, "completion_tokens": 10 } })
        .to_string()
    }

    fn words(text: &str) -> String {
        json!({ "choices": [{ "message": { "role": "assistant", "content": text } }],
            "usage": { "prompt_tokens": 100, "completion_tokens": 10 } })
        .to_string()
    }

    /// What the builder model answers: a graph in the flat format.
    fn built(name: &str, level: u32, light: &str) -> String {
        let content = json!({
            "name": name, "mode": "restart", "note": null,
            "nodes": [
                { "id": "t1", "type": "when_state", "point": "fake:porte/contact", "to": false },
                { "id": "c1", "type": "if", "all": true, "rules": [{ "kind": "sun", "is": "night" }] },
                { "id": "a1", "type": "set", "point": format!("{light}/on"), "value": true },
                { "id": "a2", "type": "set", "point": format!("{light}/brightness"), "value": level },
                { "id": "w1", "type": "wait", "seconds": 300 },
                { "id": "a3", "type": "set", "point": format!("{light}/on"), "value": false }
            ],
            "edges": [
                { "from": "t1", "port": "out", "to": "c1" },
                { "from": "c1", "port": "yes", "to": "a1" },
                { "from": "a1", "port": "out", "to": "a2" },
                { "from": "a2", "port": "out", "to": "w1" },
                { "from": "w1", "port": "out", "to": "a3" }
            ]
        });
        json!({ "choices": [{ "message": { "content": content.to_string() } }],
            "usage": { "prompt_tokens": 1_000, "completion_tokens": 10 } })
        .to_string()
    }

    /// The builder's graph, as the engine reads it.
    fn built_graph(level: u32, light: &str) -> Graph {
        let answer: Json = serde_json::from_str(&built("x", level, light)).unwrap();
        parse(
            answer["choices"][0]["message"]["content"].as_str().unwrap(),
            None,
        )
        .unwrap()
        .graph
    }

    /// « Entrée la nuit », saved by a person; approved when `approve`.
    async fn entree_la_nuit(autos: &Automations, approve: bool) -> Automation {
        let a = autos
            .save(
                Automation {
                    id: String::new(),
                    name: "Entrée la nuit".into(),
                    enabled: false,
                    mode: Mode::Restart,
                    graph: built_graph(30, "fake:entree"),
                    author: Author::Human,
                    approved: None,
                    approved_version: None,
                    note: None,
                    created: 0,
                    updated: 0,
                },
                HUMAN,
            )
            .await
            .unwrap();
        if approve {
            autos.approve(&a.id, &a.fingerprint(), HUMAN).await.unwrap()
        } else {
            a
        }
    }

    fn turn(question: &str) -> crate::Turn {
        serde_json::from_value(json!({
            "spoken": true,
            "messages": [{ "role": "user", "content": question }],
        }))
        .unwrap()
    }

    /// The example: a sentence said aloud becomes a valid automation, saved
    /// as a draft. Nothing runs and nothing is commanded until a person
    /// approves it; Moli cannot approve it.
    #[tokio::test]
    async fn a_spoken_request_becomes_a_valid_draft_and_nothing_runs() {
        let (hub, autos) = hall().await;
        let asked_for =
            "quand la porte d'entrée s'ouvre la nuit, allume l'entrée à 30 % pendant 5 minutes";
        let (moli, asked) = moli(
            &hub,
            &autos,
            vec![
                calls(&[("automation", json!({ "request": asked_for }))]),
                built("Entrée la nuit", 30, "fake:entree"),
                words("La nuit, l'entrée s'allumera à 30 % cinq minutes quand la porte s'ouvre : valide-le à l'écran."),
            ],
        )
        .await;
        let reply = moli.turn(turn(asked_for)).await.unwrap();
        assert!(reply.reply.contains("valide"), "{}", reply.reply);
        assert!(reply.actions.is_empty(), "no order given");

        let list = autos.list();
        assert_eq!(list.len(), 1);
        let a = &list[0];
        assert_eq!(
            reply.cards,
            vec![json!({ "kind": "automation", "id": a.id })]
        );
        assert_eq!(a.author, Author::Assistant);
        assert!(!a.enabled && a.approved.is_none(), "a draft, off");
        let check = autos.check(&a.graph);
        assert!(check.ok(), "{:?}", check.problems);
        // What the person reads is the checker's sentence, also given to Moli.
        assert!(check.summary.contains("30"), "{}", check.summary);
        assert!(
            asked.lock().unwrap()[2].contains("30"),
            "the summary went back to Moli"
        );
        let light = moli_core::PointId::from("fake:entree/on");
        assert_eq!(
            hub.state(&light).map(|s| s.value),
            Some(moli_core::Value::Bool(false)),
            "nothing commanded"
        );

        // Only a person approves.
        assert!(matches!(
            autos.approve(&a.id, &a.fingerprint(), MOLI).await,
            Err(moli_automation::AutomationError::NeedsHuman)
        ));
        let live = autos.approve(&a.id, &a.fingerprint(), HUMAN).await.unwrap();
        assert!(live.is_live());
    }

    /// An approved automation is paused by voice: Moli finds it, proposes,
    /// and it keeps running until a person confirms on the card.
    #[tokio::test]
    async fn pausing_by_voice_is_a_proposal_a_person_confirms() {
        let (hub, autos) = hall().await;
        let a = entree_la_nuit(&autos, true).await;
        let (moli, asked) = moli(
            &hub,
            &autos,
            vec![
                calls(&[("automations", json!({}))]),
                calls(&[(
                    "change_automation",
                    json!({ "automation": "entree la nuit", "action": "pause", "request": "" }),
                )]),
                words("Je propose de couper « Entrée la nuit » : confirme à l'écran."),
            ],
        )
        .await;
        let reply = moli
            .turn(turn("coupe l'automatisme de l'entrée la nuit"))
            .await
            .unwrap();
        assert!(
            asked.lock().unwrap()[1].contains(&a.id),
            "the list reached Moli"
        );
        assert_eq!(
            reply.cards,
            vec![json!({ "kind": "automation", "id": a.id })]
        );
        assert!(autos.get(&a.id).unwrap().is_live(), "still running");
        let p = autos.proposal_for(&a.id).unwrap();
        assert_eq!(p.kind, "pause");
        assert_eq!(p.proposed_by, Author::Assistant);

        assert!(
            autos.confirm(p.id, MOLI).await.is_err(),
            "Moli never confirms"
        );
        autos.confirm(p.id, HUMAN).await.unwrap();
        assert!(!autos.get(&a.id).unwrap().enabled);
    }

    /// Changing an approved automation by voice: the builder reworks it, the
    /// new version waits as a proposal, the approved one is untouched.
    #[tokio::test]
    async fn changing_by_voice_keeps_the_approved_version_until_confirmed() {
        let (hub, autos) = hall().await;
        let a = entree_la_nuit(&autos, true).await;
        let (moli, _asked) = moli(
            &hub,
            &autos,
            vec![
                calls(&[(
                    "change_automation",
                    json!({ "automation": a.id, "action": "update", "request": "50 % au lieu de 30 %" }),
                )]),
                built("Entrée la nuit", 50, "fake:entree"),
                words("Je propose 50 % au lieu de 30 % : confirme à l'écran."),
            ],
        )
        .await;
        moli.turn(turn("mets l'entrée la nuit à 50 %"))
            .await
            .unwrap();
        let now = autos.get(&a.id).unwrap();
        assert_eq!(now.fingerprint(), a.fingerprint(), "untouched");
        assert!(now.is_live());
        let p = autos.proposal_for(&a.id).unwrap();
        assert_eq!(p.kind, "update");
        assert!(p.summary.contains("50"), "{}", p.summary);

        let moli_automation::Confirmed::Automation(done) =
            autos.confirm(p.id, HUMAN).await.unwrap()
        else {
            panic!("an update gives the automation back");
        };
        assert!(done.is_live());
        assert!(autos.check(&done.graph).summary.contains("50"));
    }

    /// Refusals: an unknown automation, an unknown action, a change that
    /// reaches an unknown device. Nothing is proposed, nothing changes.
    #[tokio::test]
    async fn impossible_changes_are_refused() {
        let (hub, autos) = hall().await;
        let a = entree_la_nuit(&autos, false).await;
        // The builder insists on a garage that does not exist (two tries).
        let (moli, _asked) = moli(
            &hub,
            &autos,
            vec![
                built("Entrée la nuit", 30, "fake:garage"),
                built("Entrée la nuit", 30, "fake:garage"),
            ],
        )
        .await;
        let mut run = crate::Run::default();
        let unknown = moli
            .change_automation(
                &json!({ "automation": "salle de bain", "action": "pause" }),
                &mut run,
            )
            .await;
        assert!(
            unknown["error"].as_str().unwrap().contains("salle de bain"),
            "{unknown}"
        );
        let forbidden = moli
            .change_automation(
                &json!({ "automation": a.id, "action": "approve" }),
                &mut run,
            )
            .await;
        assert!(forbidden["error"].is_string(), "{forbidden}");
        let garage = moli
            .change_automation(
                &json!({ "automation": a.id, "action": "update", "request": "allume le garage" }),
                &mut run,
            )
            .await;
        assert!(
            garage["error"].as_str().unwrap().contains("garage"),
            "{garage}"
        );
        assert!(autos.proposals().is_empty());
        assert_eq!(autos.get(&a.id).unwrap().fingerprint(), a.fingerprint());
        assert!(run.cards.is_empty());
    }

    /// A new automation that reaches an unknown device is kept as a draft to
    /// fix: even a person cannot approve it as it is.
    #[tokio::test]
    async fn a_draft_reaching_an_unknown_device_cannot_be_approved() {
        let (hub, autos) = hall().await;
        let (moli, _asked) = moli(
            &hub,
            &autos,
            vec![
                built("Garage la nuit", 30, "fake:garage"),
                built("Garage la nuit", 30, "fake:garage"),
            ],
        )
        .await;
        let mut run = crate::Run::default();
        let said = moli
            .automation_tool(&json!({ "request": "allume le garage la nuit" }), &mut run)
            .await;
        assert!(!said["problems"].as_array().unwrap().is_empty(), "{said}");
        let a = &autos.list()[0];
        assert!(matches!(
            autos.approve(&a.id, &a.fingerprint(), HUMAN).await,
            Err(moli_automation::AutomationError::Invalid(_))
        ));
    }

    #[test]
    fn the_automation_meant_is_found_by_its_name_never_guessed() {
        let named = |id: &str, name: &str| Automation {
            id: id.into(),
            name: name.into(),
            enabled: false,
            mode: Mode::Restart,
            graph: Graph::default(),
            author: Author::Human,
            approved: None,
            approved_version: None,
            note: None,
            created: 0,
            updated: 0,
        };
        let list = [
            named("entree-la-nuit", "Entrée la nuit"),
            named("extinction-23-h", "Extinction 23 h"),
            named("entree-le-matin", "Entrée le matin"),
        ];
        let find = |w: &str| find_automation(&list, w).map(|a| a.id);
        assert_eq!(find("entree-la-nuit").unwrap(), "entree-la-nuit");
        assert_eq!(find("  ENTREE LA NUIT ").unwrap(), "entree-la-nuit");
        assert_eq!(find("« Entrée la nuit »").unwrap(), "entree-la-nuit");
        assert_eq!(find("extinction").unwrap(), "extinction-23-h");
        let both = find("entrée").unwrap_err();
        assert!(
            both.contains("Entrée la nuit") && both.contains("Entrée le matin"),
            "{both}"
        );
        assert!(find("garage").unwrap_err().contains("garage"));
        assert!(find("").is_err());
    }

    /// The new tools are offered only when automations run, and speak the
    /// engine's words in both languages.
    #[test]
    fn the_new_tools_say_nothing_happens_without_a_person() {
        let (fr, en) = (crate::tests::french(), crate::tests::english());
        for (words, nothing) in [(&fr, "Rien ne change"), (&en, "Nothing changes")] {
            let text = &words["assistant.tools.change_automation.description"];
            assert!(text.contains(nothing), "{text}");
            for action in ["update", "pause", "resume", "delete", "run"] {
                assert!(text.contains(action), "{action}");
            }
        }
        let tool = change_automation_tool();
        assert_eq!(
            tool["function"]["parameters"]["properties"]["action"]["enum"],
            json!(["update", "pause", "resume", "delete", "run"])
        );
        assert_eq!(automations_tool()["function"]["name"], "automations");
    }
}
