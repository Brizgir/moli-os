//! Claude, by Anthropic's Messages API (`POST /v1/messages`), without SDK:
//! Rust has none, so plain JSON over `llm::Endpoint`.
//!
//! Moli builds its conversation one way (OpenAI-shaped messages and tools,
//! see `lib.rs`); this module translates it to Anthropic's shapes and the
//! answer back, so the loop, the tools and the guard stay the same for
//! every provider:
//!
//! - the system messages become `system` blocks, a `cache_control`
//!   breakpoint after the stable ones (the instructions, the inventory),
//!   the date and time after the last breakpoint;
//! - tools become `{name, description, input_schema, strict}`, the schema
//!   cut to what strict mode takes;
//! - an assistant turn with tool calls becomes `text` + `tool_use` blocks,
//!   the tool results one `user` message of `tool_result` blocks (all of
//!   them together: parallel calls stay parallel);
//! - an answer is read only after its `stop_reason`: a refusal executes
//!   nothing, a cut answer runs none of its calls. Its blocks (thinking
//!   included) are kept as they came and sent back unchanged while the tool
//!   loop goes on.
//!
//! Thinking is adaptive (never disabled, never a budget: both are refused
//! by the current models); `output_config.effort` sets its depth. No
//! forced `tool_choice`, no prefilled answer: both are refused too.

use std::time::Duration;

use serde_json::{Map, Value as Json, json};

use crate::llm::Endpoint;

/// The models offered, the default first. Exact ids, never a dated suffix.
pub(crate) const MODELS: [&str; 4] = [
    "claude-opus-5-5",
    "claude-sonnet-5-5",
    "claude-haiku-5-5",
    "claude-fable-5-1",
];
pub(crate) const DEFAULT_MODEL: &str = MODELS[0];
pub(crate) const DEFAULT_URL: &str = "https://api.anthropic.com";
/// Effort levels Anthropic knows, lightest first.
pub(crate) const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];
/// Room for thinking and the answer, without streaming.
const MAX_TOKENS: u32 = 16_000;
/// A draft is thought through (`high`): longer than a conversation's turn.
pub(crate) const BUILD_LIMIT: Duration = Duration::from_secs(120);
/// `fallbacks: "default"`: a declined request is run again, server-side, on
/// the model Anthropic recommends for that kind of refusal.
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
/// A web search the server paused this many times is given up.
const MAX_CONTINUATIONS: usize = 3;
/// Web searches one question may make.
const MAX_SEARCHES: u32 = 3;
/// At most four breakpoints per request.
const MAX_BREAKPOINTS: usize = 4;

/// What one round trip asks for, whatever the provider.
pub(crate) struct Ask<'a> {
    pub model: &'a str,
    /// OpenAI-shaped: `system` (its `blocks` when Moli split it), `user`,
    /// `assistant` (its `tool_calls`, its `claude` blocks), `tool`.
    pub messages: &'a [Json],
    /// OpenAI-shaped functions.
    pub tools: Option<&'a Json>,
    /// No more tool calls this time (the turn's last round trip).
    pub last: bool,
    pub effort: &'a str,
    /// The answer must be this JSON (`output_config.format`).
    pub schema: Option<&'a Json>,
}

/// Whether `model` can be run again elsewhere when declined (Haiku cannot:
/// the parameter would be refused).
fn has_fallbacks(model: &str) -> bool {
    ["claude-opus-5", "claude-fable-5", "claude-sonnet-5-5"]
        .iter()
        .any(|m| model.starts_with(m))
}

/// The request's body, and the betas it needs (`anthropic-beta`).
/// `fallbacks`: the endpoint is Anthropic's own (a proxy may refuse them).
pub(crate) fn request(ask: &Ask, fallbacks: bool) -> (Json, Vec<&'static str>) {
    let mut body = json!({
        "model": ask.model,
        "max_tokens": MAX_TOKENS,
        "messages": messages(ask.messages),
        "output_config": { "effort": ask.effort },
    });
    let system = system(ask.messages);
    if !system.is_empty() {
        body["system"] = Json::Array(system);
    }
    if let Some(tools) = ask.tools.and_then(Json::as_array) {
        body["tools"] = tools.iter().map(tool).collect();
        // `auto` or `none` only: forcing a call is refused (400).
        body["tool_choice"] = json!({ "type": if ask.last { "none" } else { "auto" } });
    }
    if let Some(schema) = ask.schema {
        body["output_config"]["format"] =
            json!({ "type": "json_schema", "schema": strict_schema(schema) });
    }
    let mut betas = Vec::new();
    if fallbacks && has_fallbacks(ask.model) {
        body["fallbacks"] = json!("default");
        betas.push(FALLBACK_BETA);
    }
    (body, betas)
}

/// The system messages as text blocks. A block Moli marked `cache` ends a
/// stable stretch: a breakpoint there (four at most). None marked: one at
/// the end.
fn system(messages: &[Json]) -> Vec<Json> {
    let mut blocks = Vec::new();
    let mut marked = false;
    for m in messages.iter().filter(|m| m["role"] == "system") {
        let Some(parts) = m["blocks"].as_array() else {
            let text = m["content"].as_str().unwrap_or_default();
            if !text.trim().is_empty() {
                blocks.push(json!({ "type": "text", "text": text }));
            }
            continue;
        };
        for part in parts {
            let text = part["text"].as_str().unwrap_or_default();
            if text.trim().is_empty() {
                continue;
            }
            let mut block = json!({ "type": "text", "text": text });
            if part["cache"] == true {
                block["cache_control"] = json!({ "type": "ephemeral" });
                marked = true;
            }
            blocks.push(block);
        }
    }
    if !marked && let Some(last) = blocks.last_mut() {
        last["cache_control"] = json!({ "type": "ephemeral" });
    }
    let mut seen = 0;
    for block in &mut blocks {
        if block.get("cache_control").is_some() {
            seen += 1;
            if seen > MAX_BREAKPOINTS
                && let Some(b) = block.as_object_mut()
            {
                b.remove("cache_control");
            }
        }
    }
    blocks
}

/// The conversation as Anthropic wants it: `user` first, roles taking
/// turns, the results of one answer's calls in one message.
fn messages(messages: &[Json]) -> Vec<Json> {
    let mut out: Vec<(String, Vec<Json>)> = Vec::new();
    for m in messages {
        let (role, blocks) = match m["role"].as_str() {
            Some("user") => ("user", text_blocks(&m["content"])),
            Some("assistant") => ("assistant", assistant_blocks(m)),
            Some("tool") => (
                "user",
                vec![json!({
                    "type": "tool_result",
                    "tool_use_id": m["tool_call_id"],
                    "content": m["content"].as_str().unwrap_or_default(),
                })],
            ),
            _ => continue,
        };
        if blocks.is_empty() {
            continue;
        }
        match out.last_mut() {
            Some((last, kept)) if last == role => kept.extend(blocks),
            _ => out.push((role.to_owned(), blocks)),
        }
    }
    // The dashboard's greeting may come first: the model starts with a person.
    let start = out
        .iter()
        .position(|(r, _)| r == "user")
        .unwrap_or(out.len());
    out.into_iter()
        .skip(start)
        .map(|(role, blocks)| {
            // One plain text stays a string, as people write it.
            let content = match blocks.as_slice() {
                [only] if only["type"] == "text" && only.get("cache_control").is_none() => {
                    only["text"].clone()
                }
                _ => Json::Array(blocks),
            };
            json!({ "role": role, "content": content })
        })
        .collect()
}

/// A text as blocks: none when empty (Anthropic refuses an empty text).
fn text_blocks(content: &Json) -> Vec<Json> {
    match content.as_str().map(str::trim) {
        Some(text) if !text.is_empty() => vec![json!({ "type": "text", "text": text })],
        _ => Vec::new(),
    }
}

/// An assistant turn: its own blocks when Claude wrote it (thinking
/// included, unchanged), otherwise its words and its calls.
fn assistant_blocks(m: &Json) -> Vec<Json> {
    if let Some(raw) = m["claude"].as_array() {
        return raw.clone();
    }
    let mut blocks = text_blocks(&m["content"]);
    for call in m["tool_calls"].as_array().into_iter().flatten() {
        let input: Json = call["function"]["arguments"]
            .as_str()
            .and_then(|a| serde_json::from_str(a).ok())
            .filter(Json::is_object)
            .unwrap_or_else(|| json!({}));
        blocks.push(json!({
            "type": "tool_use",
            "id": call["id"],
            "name": call["function"]["name"],
            "input": input,
        }));
    }
    blocks
}

/// An OpenAI function as an Anthropic tool, strict: its arguments then
/// always match the schema.
fn tool(function: &Json) -> Json {
    let f = &function["function"];
    let schema = if f["parameters"].is_object() {
        strict_schema(&f["parameters"])
    } else {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    };
    json!({
        "name": f["name"],
        "description": f["description"],
        "input_schema": schema,
        "strict": true,
    })
}

/// Keywords strict mode does not take (bounds are checked by Moli itself).
const UNSUPPORTED: [&str; 9] = [
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "multipleOf",
    "minLength",
    "maxLength",
    "minItems",
    "maxItems",
];

/// A schema as strict mode wants it: `additionalProperties: false` on every
/// object, no numeric or length bounds, a list of types as `anyOf` (an
/// `enum` alone when it lists the values).
pub(crate) fn strict_schema(schema: &Json) -> Json {
    match schema {
        Json::Object(map) => {
            let mut out = Map::new();
            for (k, v) in map {
                if UNSUPPORTED.contains(&k.as_str()) {
                    continue;
                }
                let v = match k.as_str() {
                    "properties" => Json::Object(
                        v.as_object()
                            .into_iter()
                            .flatten()
                            .map(|(name, s)| (name.clone(), strict_schema(s)))
                            .collect(),
                    ),
                    "items" | "not" => strict_schema(v),
                    "anyOf" | "allOf" | "oneOf" => Json::Array(
                        v.as_array()
                            .into_iter()
                            .flatten()
                            .map(strict_schema)
                            .collect(),
                    ),
                    _ => v.clone(),
                };
                out.insert(k.clone(), v);
            }
            if out.get("type").and_then(Json::as_array).is_some() {
                return type_list(out);
            }
            if out.get("type").and_then(Json::as_str) == Some("object")
                || out.contains_key("properties")
            {
                out.entry("properties").or_insert_with(|| json!({}));
                out.insert("additionalProperties".into(), json!(false));
            }
            Json::Object(out)
        }
        other => other.clone(),
    }
}

/// `{"type": ["string", "null"], …}` as `anyOf`, each branch with what
/// belongs to its type.
fn type_list(mut out: Map<String, Json>) -> Json {
    let types: Vec<Json> = out
        .remove("type")
        .and_then(|t| t.as_array().cloned())
        .unwrap_or_default();
    if out.contains_key("enum") {
        return Json::Object(out);
    }
    let mut shared = Map::new();
    let mut branches = Vec::new();
    for t in &types {
        let mut branch = Map::new();
        branch.insert("type".into(), t.clone());
        match t.as_str() {
            Some("array") => {
                if let Some(items) = out.get("items") {
                    branch.insert("items".into(), items.clone());
                }
            }
            Some("object") => {
                branch.insert(
                    "properties".into(),
                    out.get("properties").cloned().unwrap_or_else(|| json!({})),
                );
                if let Some(required) = out.get("required") {
                    branch.insert("required".into(), required.clone());
                }
                branch.insert("additionalProperties".into(), json!(false));
            }
            _ => {}
        }
        branches.push(Json::Object(branch));
    }
    for (k, v) in out {
        if !matches!(
            k.as_str(),
            "items" | "properties" | "required" | "additionalProperties"
        ) {
            shared.insert(k, v);
        }
    }
    shared.insert("anyOf".into(), Json::Array(branches));
    Json::Object(shared)
}

/// How an answer ended, read before its content.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Ending {
    /// Finished, or stopped for its tool calls.
    Done,
    /// Declined by the model's safeguards: nothing in it is run.
    Refused(String),
    /// Cut by `max_tokens` (or the context window): its calls are not run.
    Cut,
    /// The server paused its own tools' loop: send it back as it is.
    Paused,
}

pub(crate) fn ending(answer: &Json) -> Ending {
    match answer["stop_reason"].as_str() {
        Some("refusal") => Ending::Refused(
            answer["stop_details"]["category"]
                .as_str()
                .unwrap_or("unspecified")
                .to_owned(),
        ),
        Some("max_tokens" | "model_context_window_exceeded") => Ending::Cut,
        Some("pause_turn") => Ending::Paused,
        _ => Ending::Done,
    }
}

/// The blocks to send back: as they came, but after a server-side
/// fallback, only the words of what came before it.
fn replayed(content: &[Json]) -> Vec<Json> {
    let Some(boundary) = content.iter().rposition(|b| b["type"] == "fallback") else {
        return content.to_vec();
    };
    content[..boundary]
        .iter()
        .filter(|b| b["type"] == "text")
        .chain(&content[boundary + 1..])
        .cloned()
        .collect()
}

/// The usage as Moli counts it: every prompt token (fresh, read from the
/// cache, written to it), those read, those written, and the answer's.
pub(crate) fn usage(usage: &Json) -> Json {
    let n = |k: &str| usage[k].as_u64().unwrap_or(0);
    let read = n("cache_read_input_tokens");
    let written = n("cache_creation_input_tokens");
    json!({
        "prompt_tokens": n("input_tokens") + read + written,
        "completion_tokens": n("output_tokens"),
        "prompt_tokens_details": { "cached_tokens": read },
        "cache_written_tokens": written,
    })
}

/// An answer in Moli's shape (`choices[0].message`, `usage`), whatever
/// Claude said. A refusal becomes `refused` (its category) and no words; a
/// cut answer keeps its words and drops its calls (`cut`).
pub(crate) fn answer(answer: &Json) -> Json {
    let content = answer["content"].as_array().cloned().unwrap_or_default();
    let ending = ending(answer);
    let mut out = json!({ "model": answer["model"], "usage": usage(&answer["usage"]) });
    if let Ending::Refused(category) = &ending {
        out["refused"] = json!(category);
        out["choices"] = json!([{ "message": { "role": "assistant", "content": "" } }]);
        return out;
    }
    let text = content
        .iter()
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned();
    let mut message = json!({ "role": "assistant", "content": text });
    let calls: Vec<Json> = content
        .iter()
        .filter(|b| b["type"] == "tool_use")
        .map(|b| {
            json!({
                "id": b["id"],
                "type": "function",
                "function": { "name": b["name"], "arguments": b["input"].to_string() },
            })
        })
        .collect();
    if ending == Ending::Cut {
        out["cut"] = json!(true);
    } else {
        if !calls.is_empty() {
            message["tool_calls"] = Json::Array(calls);
        }
        message["claude"] = Json::Array(replayed(&content));
    }
    out["choices"] = json!([{ "message": message }]);
    out
}

/// One round trip: `ask` translated, sent, its answer translated back.
pub(crate) async fn complete(
    endpoint: &Endpoint,
    key: Option<&str>,
    ask: &Ask<'_>,
    limit: Duration,
) -> anyhow::Result<Json> {
    let (body, betas) = request(ask, endpoint.is_anthropic_api());
    let raw = endpoint.messages(key, &betas, &body, limit).await?;
    let out = answer(&raw);
    log(&raw);
    Ok(out)
}

/// What the cache did, and why an answer stopped short.
fn log(raw: &Json) {
    let u = &raw["usage"];
    tracing::debug!(
        cache_read = u["cache_read_input_tokens"].as_u64().unwrap_or(0),
        cache_written = u["cache_creation_input_tokens"].as_u64().unwrap_or(0),
        input = u["input_tokens"].as_u64().unwrap_or(0),
        output = u["output_tokens"].as_u64().unwrap_or(0),
        stop = raw["stop_reason"].as_str().unwrap_or_default(),
        "claude answered"
    );
    match ending(raw) {
        Ending::Refused(category) => tracing::warn!(category, "claude declined the request"),
        Ending::Cut => tracing::warn!("claude's answer was cut short"),
        Ending::Done | Ending::Paused => {}
    }
}

/// The web search tool (run by Anthropic). Models before the current ones
/// know only the basic variant.
fn search_tool(model: &str, timezone: &str) -> Json {
    let kind = if model.starts_with("claude-haiku") {
        "web_search_20250305"
    } else {
        "web_search_20260209"
    };
    json!({
        "type": kind,
        "name": "web_search",
        "max_uses": MAX_SEARCHES,
        "user_location": { "type": "approximate", "timezone": timezone },
    })
}

/// The request of a web question.
pub(crate) fn search_request(model: &str, timezone: &str, question: &str) -> Json {
    json!({
        "model": model,
        "max_tokens": MAX_TOKENS,
        "messages": [{ "role": "user", "content": question }],
        "tools": [search_tool(model, timezone)],
        "output_config": { "effort": "low" },
    })
}

/// What a web question cost: its tokens (Moli's usage shape) and how many
/// searches the server ran.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct SearchUsage {
    pub prompt: u64,
    pub cached: u64,
    pub written: u64,
    pub completion: u64,
    pub searches: u64,
}

impl SearchUsage {
    fn add(&mut self, usage: &Json) {
        let u = self::usage(usage);
        self.prompt += u["prompt_tokens"].as_u64().unwrap_or(0);
        self.cached += u["prompt_tokens_details"]["cached_tokens"]
            .as_u64()
            .unwrap_or(0);
        self.written += u["cache_written_tokens"].as_u64().unwrap_or(0);
        self.completion += u["completion_tokens"].as_u64().unwrap_or(0);
        self.searches += usage["server_tool_use"]["web_search_requests"]
            .as_u64()
            .unwrap_or(0);
    }
}

/// A question answered from the web: Claude searches (on Anthropic's side)
/// and writes. A pause is resumed by sending the paused turn back as it
/// is; the search's own errors come in the answer (HTTP 200), an object
/// where the results would be.
pub(crate) async fn search(
    endpoint: &Endpoint,
    key: Option<&str>,
    model: &str,
    timezone: &str,
    question: &str,
    spent: &mut SearchUsage,
) -> anyhow::Result<String> {
    let mut body = search_request(model, timezone, question);
    let mut words = Vec::new();
    for _ in 0..=MAX_CONTINUATIONS {
        let raw = endpoint
            .messages(key, &[], &body, crate::llm::SEARCH_LIMIT)
            .await?;
        spent.add(&raw["usage"]);
        log(&raw);
        let content = raw["content"].as_array().cloned().unwrap_or_default();
        for failure in search_errors(&content) {
            tracing::warn!(
                error = failure,
                "the web search failed on the provider's side"
            );
        }
        match ending(&raw) {
            Ending::Refused(_) => anyhow::bail!("the search was declined"),
            Ending::Paused => {
                words.extend(texts(&content));
                if let Some(list) = body["messages"].as_array_mut() {
                    list.push(json!({ "role": "assistant", "content": content }));
                }
            }
            Ending::Done | Ending::Cut => {
                words.extend(texts(&content));
                let text = words.join("").trim().to_owned();
                anyhow::ensure!(!text.is_empty(), "the search gave no answer");
                return Ok(text);
            }
        }
    }
    anyhow::bail!("the search did not finish")
}

fn texts(content: &[Json]) -> Vec<String> {
    content
        .iter()
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str().map(str::to_owned))
        .collect()
}

/// The error codes of the searches that failed (`max_uses_exceeded`…).
fn search_errors(content: &[Json]) -> Vec<String> {
    content
        .iter()
        .filter(|b| b["type"] == "web_search_tool_result" && b["content"].is_object())
        .map(|b| {
            b["content"]["error_code"]
                .as_str()
                .unwrap_or("unknown")
                .to_owned()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn functions() -> Json {
        json!([
            { "type": "function", "function": { "name": "set", "description": "Change a value.",
              "parameters": { "type": "object",
                "properties": { "point": { "type": "string" },
                  "value": { "anyOf": [{ "type": "boolean" }, { "type": "number" }] } },
                "required": ["point", "value"] } } },
            { "type": "function", "function": { "name": "forecast", "description": "Weather.",
              "parameters": { "type": "object",
                "properties": { "days": { "type": "integer", "minimum": 1, "maximum": 7 } },
                "required": [] } } }
        ])
    }

    fn conversation() -> Vec<Json> {
        vec![
            json!({ "role": "system", "content": "joined", "blocks": [
                { "text": "Tu es Moli.", "cache": true },
                { "text": "Inventaire : lampe.", "cache": true },
                { "text": "Maintenant : samedi 18 h." }
            ] }),
            json!({ "role": "assistant", "content": "Bonjour !" }),
            json!({ "role": "user", "content": "Allume le salon" }),
        ]
    }

    #[test]
    fn a_turn_is_asked_the_way_anthropic_wants_it() {
        let messages = conversation();
        let tools = functions();
        let (body, betas) = request(
            &Ask {
                model: "claude-opus-5-5",
                messages: &messages,
                tools: Some(&tools),
                last: false,
                effort: "low",
                schema: None,
            },
            true,
        );
        assert_eq!(
            body,
            json!({
                "model": "claude-opus-5-5",
                "max_tokens": 16_000,
                "system": [
                    { "type": "text", "text": "Tu es Moli.", "cache_control": { "type": "ephemeral" } },
                    { "type": "text", "text": "Inventaire : lampe.", "cache_control": { "type": "ephemeral" } },
                    { "type": "text", "text": "Maintenant : samedi 18 h." }
                ],
                // The greeting before the person's first words is left out.
                "messages": [{ "role": "user", "content": "Allume le salon" }],
                "tools": [
                    { "name": "set", "description": "Change a value.", "strict": true,
                      "input_schema": { "type": "object", "additionalProperties": false,
                        "properties": { "point": { "type": "string" },
                          "value": { "anyOf": [{ "type": "boolean" }, { "type": "number" }] } },
                        "required": ["point", "value"] } },
                    { "name": "forecast", "description": "Weather.", "strict": true,
                      "input_schema": { "type": "object", "additionalProperties": false,
                        "properties": { "days": { "type": "integer" } }, "required": [] } }
                ],
                "tool_choice": { "type": "auto" },
                "output_config": { "effort": "low" },
                "fallbacks": "default"
            })
        );
        assert_eq!(betas, ["server-side-fallback-2026-07-01"]);
        // Never a thinking switch, a budget, a forced call or a prefill.
        let text = body.to_string();
        for refused in ["thinking", "budget_tokens", "\"any\"", "temperature"] {
            assert!(!text.contains(refused), "{refused}");
        }
    }

    #[test]
    fn no_fallback_where_it_would_be_refused() {
        let messages = conversation();
        for (model, anthropic) in [("claude-haiku-5-5", true), ("claude-opus-5-5", false)] {
            let (body, betas) = request(
                &Ask {
                    model,
                    messages: &messages,
                    tools: None,
                    last: false,
                    effort: "low",
                    schema: None,
                },
                anthropic,
            );
            assert!(
                body.get("fallbacks").is_none() && betas.is_empty(),
                "{model}"
            );
            assert!(body.get("tool_choice").is_none());
        }
    }

    #[test]
    fn the_last_round_trip_calls_nothing_and_one_block_is_cached_by_default() {
        let messages = vec![
            json!({ "role": "system", "content": "Tu es Moli." }),
            json!({ "role": "user", "content": "Et alors ?" }),
        ];
        let tools = functions();
        let (body, _) = request(
            &Ask {
                model: "claude-sonnet-5-5",
                messages: &messages,
                tools: Some(&tools),
                last: true,
                effort: "high",
                schema: None,
            },
            false,
        );
        assert_eq!(body["tool_choice"], json!({ "type": "none" }));
        assert_eq!(
            body["system"],
            json!([{ "type": "text", "text": "Tu es Moli.", "cache_control": { "type": "ephemeral" } }])
        );
    }

    /// Two calls in one answer: both run, their results go back together,
    /// after the answer's own blocks (its thinking unchanged).
    #[test]
    fn parallel_calls_go_there_and_back() {
        let raw = json!({
            "id": "msg_test", "type": "message", "role": "assistant", "model": "claude-opus-5-5",
            "stop_reason": "tool_use",
            "content": [
                { "type": "thinking", "thinking": "", "signature": "c2lnbmF0dXJlLXRlc3Q=" },
                { "type": "text", "text": "J'allume." },
                { "type": "tool_use", "id": "toolu_test_1", "name": "set", "input": { "point": "lampe-1/on", "value": true } },
                { "type": "tool_use", "id": "toolu_test_2", "name": "set", "input": { "point": "lampe-2/on", "value": true } }
            ],
            "usage": { "input_tokens": 120, "cache_read_input_tokens": 5_000, "cache_creation_input_tokens": 0, "output_tokens": 80 }
        });
        let translated = answer(&raw);
        let message = &translated["choices"][0]["message"];
        assert_eq!(message["content"], "J'allume.");
        let calls = message["tool_calls"].as_array().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1]["id"], "toolu_test_2");
        let args: Json =
            serde_json::from_str(calls[0]["function"]["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(args, json!({ "point": "lampe-1/on", "value": true }));
        assert_eq!(translated["usage"]["prompt_tokens"], 5_120);
        assert_eq!(
            translated["usage"]["prompt_tokens_details"]["cached_tokens"],
            5_000
        );

        // As lib.rs carries on: the answer, then each call's result.
        let messages = vec![
            json!({ "role": "user", "content": "Allume les lampes" }),
            json!({ "role": "assistant", "content": message["content"], "tool_calls": message["tool_calls"], "claude": message["claude"] }),
            json!({ "role": "tool", "tool_call_id": "toolu_test_1", "content": "{\"status\":\"done\"}" }),
            json!({ "role": "tool", "tool_call_id": "toolu_test_2", "content": "{\"status\":\"held\"}" }),
        ];
        let (body, _) = request(
            &Ask {
                model: "claude-opus-5-5",
                messages: &messages,
                tools: None,
                last: false,
                effort: "low",
                schema: None,
            },
            false,
        );
        let sent = body["messages"].as_array().unwrap();
        assert_eq!(sent.len(), 3);
        assert_eq!(
            sent[1]["content"], raw["content"],
            "the blocks as they came"
        );
        assert_eq!(
            sent[2],
            json!({ "role": "user", "content": [
                { "type": "tool_result", "tool_use_id": "toolu_test_1", "content": "{\"status\":\"done\"}" },
                { "type": "tool_result", "tool_use_id": "toolu_test_2", "content": "{\"status\":\"held\"}" }
            ] })
        );
    }

    /// An assistant turn another provider wrote (no blocks of Claude's):
    /// its words and calls become blocks.
    #[test]
    fn calls_without_claude_blocks_are_translated() {
        let blocks = assistant_blocks(&json!({
            "role": "assistant", "content": "",
            "tool_calls": [{ "id": "call_1", "type": "function", "function": { "name": "show", "arguments": "{\"cards\":[]}" } }]
        }));
        assert_eq!(
            blocks,
            [
                json!({ "type": "tool_use", "id": "call_1", "name": "show", "input": { "cards": [] } })
            ]
        );
    }

    #[test]
    fn a_refusal_runs_nothing() {
        let raw = json!({
            "model": "claude-opus-5-5", "stop_reason": "refusal",
            "stop_details": { "type": "refusal", "category": "cyber", "explanation": null },
            "content": [{ "type": "tool_use", "id": "toolu_x", "name": "set", "input": {} }],
            "usage": { "input_tokens": 10, "output_tokens": 0 }
        });
        let translated = answer(&raw);
        assert_eq!(translated["refused"], "cyber");
        assert!(translated["choices"][0]["message"]["tool_calls"].is_null());
        assert_eq!(translated["choices"][0]["message"]["content"], "");
    }

    #[test]
    fn a_cut_answer_keeps_its_words_not_its_calls() {
        let raw = json!({
            "model": "claude-opus-5-5", "stop_reason": "max_tokens",
            "content": [
                { "type": "text", "text": "Je regarde" },
                { "type": "tool_use", "id": "toolu_x", "name": "set", "input": { "point": "lampe-1/on" } }
            ],
            "usage": { "input_tokens": 10, "output_tokens": 16_000 }
        });
        let translated = answer(&raw);
        assert_eq!(translated["cut"], true);
        assert_eq!(translated["choices"][0]["message"]["content"], "Je regarde");
        assert!(translated["choices"][0]["message"]["tool_calls"].is_null());
    }

    #[test]
    fn after_a_fallback_only_the_words_before_it_go_back() {
        let content = [
            json!({ "type": "thinking", "thinking": "", "signature": "x" }),
            json!({ "type": "text", "text": "Je " }),
            json!({ "type": "fallback", "from": { "model": "claude-opus-5-5" }, "to": { "model": "claude-opus-5" } }),
            json!({ "type": "tool_use", "id": "toolu_y", "name": "set", "input": {} }),
        ];
        assert_eq!(replayed(&content), [content[1].clone(), content[3].clone()]);
    }

    #[test]
    fn schemas_become_strict() {
        let schema = json!({
            "type": "object",
            "required": ["op", "days", "value", "rule"],
            "properties": {
                "op": { "type": ["string", "null"], "enum": ["eq", null] },
                "days": { "type": ["array", "null"], "items": { "type": "integer", "maximum": 6 } },
                "value": { "type": ["string", "number", "null"] },
                "rule": { "anyOf": [{ "type": "object", "properties": { "x": { "type": "string" } }, "required": ["x"] }, { "type": "null" }] }
            }
        });
        assert_eq!(
            strict_schema(&schema),
            json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["op", "days", "value", "rule"],
                "properties": {
                    "op": { "enum": ["eq", null] },
                    "days": { "anyOf": [{ "type": "array", "items": { "type": "integer" } }, { "type": "null" }] },
                    "value": { "anyOf": [{ "type": "string" }, { "type": "number" }, { "type": "null" }] },
                    "rule": { "anyOf": [
                        { "type": "object", "properties": { "x": { "type": "string" } }, "required": ["x"], "additionalProperties": false },
                        { "type": "null" }
                    ] }
                }
            })
        );
    }

    #[test]
    fn a_structured_answer_is_asked_in_output_config() {
        let messages = vec![json!({ "role": "user", "content": "Chaque soir à 22 h" })];
        let schema = json!({ "type": "object", "properties": { "name": { "type": "string" } }, "required": ["name"] });
        let (body, _) = request(
            &Ask {
                model: "claude-opus-5-5",
                messages: &messages,
                tools: None,
                last: false,
                effort: "high",
                schema: Some(&schema),
            },
            false,
        );
        assert_eq!(
            body["output_config"],
            json!({ "effort": "high", "format": { "type": "json_schema", "schema": {
                "type": "object", "properties": { "name": { "type": "string" } },
                "required": ["name"], "additionalProperties": false } } })
        );
    }

    #[test]
    fn a_web_question_uses_the_server_search() {
        let body = search_request("claude-opus-5-5", "Europe/Paris", "Qui a gagné hier ?");
        assert_eq!(
            body["tools"],
            json!([{ "type": "web_search_20260209", "name": "web_search", "max_uses": 3,
                     "user_location": { "type": "approximate", "timezone": "Europe/Paris" } }])
        );
        assert!(!body.to_string().contains("code_execution"));
        assert_eq!(
            search_request("claude-haiku-5-5", "Europe/Paris", "?")["tools"][0]["type"],
            "web_search_20250305"
        );
    }

    /// A paused search is sent back as it is, then finishes; a failed search
    /// comes as an object in a 200.
    #[tokio::test]
    async fn a_paused_search_is_resumed() {
        let paused = json!({
            "model": "claude-opus-5-5", "stop_reason": "pause_turn",
            "content": [
                { "type": "server_tool_use", "id": "srvtoolu_test", "name": "web_search", "input": { "query": "match" } },
                { "type": "web_search_tool_result", "tool_use_id": "srvtoolu_test", "content": { "type": "web_search_tool_result_error", "error_code": "max_uses_exceeded" } }
            ],
            "usage": { "input_tokens": 900, "output_tokens": 30, "server_tool_use": { "web_search_requests": 1 } }
        });
        let done = json!({
            "model": "claude-opus-5-5", "stop_reason": "end_turn",
            "content": [{ "type": "text", "text": "Lyon a gagné 2 à 1." }],
            "usage": { "input_tokens": 1_200, "cache_read_input_tokens": 300, "output_tokens": 20, "server_tool_use": { "web_search_requests": 1 } }
        });
        let (url, asked) =
            crate::llm::tests::fake_provider(vec![paused.to_string(), done.to_string()]).await;
        let endpoint = Endpoint::anthropic(&url).unwrap();
        let mut spent = SearchUsage::default();
        let text = search(
            &endpoint,
            None,
            "claude-opus-5-5",
            "Europe/Paris",
            "Le match ?",
            &mut spent,
        )
        .await
        .unwrap();
        assert_eq!(text, "Lyon a gagné 2 à 1.");
        assert_eq!(
            spent,
            SearchUsage {
                prompt: 2_400,
                cached: 300,
                written: 0,
                completion: 50,
                searches: 2
            }
        );
        let asked = asked.lock().unwrap();
        assert_eq!(asked.len(), 2);
        assert!(asked[0].starts_with("POST /v1/messages "), "{}", asked[0]);
        assert!(asked[0].contains("anthropic-version: 2023-06-01"));
        let second: Json =
            serde_json::from_str(asked[1].split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(
            second["messages"][1],
            json!({ "role": "assistant", "content": paused["content"] })
        );
        assert_eq!(
            second["messages"].as_array().unwrap().len(),
            2,
            "no « continue » added"
        );
    }
}
