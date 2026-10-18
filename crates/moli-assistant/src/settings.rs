//! Moli's settings from the dashboard: who thinks (Anthropic or OpenAI, which
//! model), the API keys (tested, then filed in the encrypted store, never
//! sent back) and the voice (which one, which tone), kept in
//! `data/assistant.json` and applied at once.

use std::path::{Path, PathBuf};
use std::sync::PoisonError;

use moli_core::InstanceId;
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

use crate::{Assistant, AssistantError, CLOUD_VOICES, Provider, claude, pricing};

/// OpenAI's models offered besides the configured one.
const OPENAI_MODELS: [&str; 3] = ["gpt-4.1-mini", "gpt-4.1", "gpt-5-mini"];

/// Tones a person can pick: id, then the catalogue keys of its label and of
/// its instructions for the voice (both in the house's language).
pub(crate) const STYLES: [(&str, &str, &str); 3] = [
    (
        "enjoue",
        "assistant.voice.style.enjoue.label",
        "assistant.voice.style.enjoue.instructions",
    ),
    (
        "pose",
        "assistant.voice.style.pose.label",
        "assistant.voice.style.pose.instructions",
    ),
    (
        "doux",
        "assistant.voice.style.doux.label",
        "assistant.voice.style.doux.instructions",
    ),
];

/// Paces a person can pick: id, catalogue key of its label, OpenAI's `speed`.
pub(crate) const PACES: [(&str, &str, f64); 3] = [
    ("normal", "assistant.voice.pace.normal", 1.0),
    ("rapide", "assistant.voice.pace.rapide", 1.2),
    ("tres_rapide", "assistant.voice.pace.tres_rapide", 1.4),
];

/// What a person chose in the dashboard: the voice, and who thinks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Prefs {
    pub voice: String,
    pub style: String,
    /// A pace from `PACES`; none = `speech_speed` of `moli.toml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pace: Option<String>,
    /// `anthropic` or `openai`; none = `moli.toml`'s.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// One of the provider's models offered; none = its default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

fn pace_speed(id: &str) -> Option<f64> {
    PACES
        .iter()
        .find(|(i, ..)| *i == id)
        .map(|(.., speed)| *speed)
}

/// Which pace `speed` is; `perso` for a speed of `moli.toml`'s own.
fn pace_id(speed: f64) -> &'static str {
    PACES
        .iter()
        .find(|(.., s)| (s - speed).abs() < 0.001)
        .map_or("perso", |(i, ..)| *i)
}

/// The instructions of tone `id`, in the house's language.
fn style_text(id: &str) -> Option<String> {
    STYLES
        .iter()
        .find(|(i, ..)| *i == id)
        .map(|(.., key)| moli_i18n::tr(key))
}

/// Which tone says `text` (in the house's language); `perso` for a text of
/// the person's own.
fn style_id(text: &str) -> &'static str {
    STYLES
        .iter()
        .find(|(.., key)| moli_i18n::tr(key) == text)
        .map_or("perso", |(i, ..)| *i)
}

/// The tone `moli.toml` asks for. The built-in default is the cheerful tone,
/// said in the house's language; any other text is the person's own.
fn configured_style(speech_style: &str) -> String {
    if speech_style == crate::default_speech_style() {
        style_text(STYLES[0].0).unwrap_or_default()
    } else {
        speech_style.to_owned()
    }
}

/// A key (OpenAI's `sk-…`, Anthropic's `sk-ant-…`), at first sight: no
/// spaces, a sensible length.
pub(crate) fn plausible_key(key: &str) -> bool {
    (20..=300).contains(&key.len()) && key.chars().all(|c| c.is_ascii_graphic())
}

fn read_prefs(path: &Path) -> Option<Prefs> {
    let mut prefs: Prefs = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    // A pace this build does not know: the configured one.
    prefs.pace = prefs.pace.filter(|p| pace_speed(p).is_some());
    // A provider it does not know: `moli.toml`'s, with its model.
    if prefs
        .provider
        .as_deref()
        .and_then(Provider::from_id)
        .is_none()
    {
        prefs.provider = None;
        prefs.model = None;
    }
    // `perso`: the tone of `moli.toml`, kept when only the model was chosen.
    (CLOUD_VOICES.contains(&prefs.voice.as_str())
        && (style_text(&prefs.style).is_some() || prefs.style == "perso"))
        .then_some(prefs)
}

impl Assistant {
    /// Reads the voice chosen in the dashboard (`data/assistant.json`);
    /// without it, `moli.toml` decides.
    pub fn load_voice_prefs(&self, path: PathBuf) {
        let prefs = read_prefs(&path);
        *self.0.prefs.write().unwrap_or_else(PoisonError::into_inner) = prefs;
        *self
            .0
            .prefs_path
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Some(path);
    }

    /// The cloud voice and its instructions, as chosen now.
    pub(crate) fn voice_and_style(&self) -> (String, String) {
        let c = &self.0.config;
        match &*self.0.prefs.read().unwrap_or_else(PoisonError::into_inner) {
            Some(p) => (
                p.voice.clone(),
                style_text(&p.style).unwrap_or_else(|| configured_style(&c.speech_style)),
            ),
            None => (c.speech_voice.clone(), configured_style(&c.speech_style)),
        }
    }

    /// How fast the cloud voice speaks, as chosen now.
    pub(crate) fn speed(&self) -> f64 {
        self.0
            .prefs
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .and_then(|p| p.pace.as_deref())
            .and_then(pace_speed)
            .unwrap_or(self.0.config.speech_speed)
    }

    /// The models a person may pick for `provider`: Anthropic's current ones,
    /// OpenAI's usual ones, and the one `moli.toml` names. A model on the
    /// local network: that one only.
    pub(crate) fn models(&self, provider: Provider) -> Vec<String> {
        let c = &self.0.config;
        let configured = c
            .model
            .clone()
            .filter(|m| !m.is_empty() && c.provider() == provider);
        let mut list: Vec<String> = configured.into_iter().collect();
        let offered: &[&str] = match provider {
            Provider::Anthropic => &claude::MODELS,
            Provider::Openai if self.endpoint(provider).is_local() => &[],
            Provider::Openai => &OPENAI_MODELS,
        };
        for m in offered {
            if !list.iter().any(|l| l == m) {
                list.push((*m).to_owned());
            }
        }
        if list.is_empty() {
            list.push(provider.default_model().to_owned());
        }
        list
    }

    /// What the settings card shows (never a key): who thinks, which keys
    /// are filed, the voice, and today's estimated cost.
    pub fn settings(&self) -> Json {
        let c = &self.0.config;
        let prefs = self
            .0
            .prefs
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let (voice, style) = match prefs {
            Some(p) => (p.voice, p.style),
            None => (
                c.speech_voice.clone(),
                style_id(&configured_style(&c.speech_style)).to_owned(),
            ),
        };
        let brain = self.brain();
        let local_model = self.endpoint(brain.provider).is_local();
        let anthropic_key = self.secret(Provider::Anthropic).is_some();
        let openai_key = self.secret(Provider::Openai).is_some();
        let providers: Vec<Json> = [Provider::Anthropic, Provider::Openai]
            .iter()
            .map(|p| json!({ "id": p, "models": self.models(*p) }))
            .collect();
        let rate = |model: &str| {
            pricing::rates(model)
                .map(|r| json!({ "input": r.input, "cached": r.cached, "output": r.output }))
        };
        json!({
            "provider": brain.provider,
            "providers": providers,
            "key": self.secret(brain.provider).is_some(),
            "keys": { "anthropic": anthropic_key, "openai": openai_key },
            "local_model": local_model,
            "model": brain.model,
            "rates": rate(&brain.model),
            // Anthropic has no voice: OpenAI's key gives it one, otherwise
            // the house's (Whisper, Piper).
            "voice_cloud": self.voice_key().is_ok() && !c.speech_model.is_empty(),
            "today": self.today(),
            "cloud_voice": !c.speech_model.is_empty(),
            "local_voice": crate::voice::host_port(&c.local_speech).is_some(),
            "local_listen": crate::voice::host_port(&c.local_listen).is_some(),
            "voice": voice,
            "style": style,
            "pace": pace_id(self.speed()),
            "voices": CLOUD_VOICES,
            "styles": STYLES.iter().map(|(id, label, _)| json!({ "id": id, "label": moli_i18n::tr(label) })).collect::<Vec<_>>(),
            "paces": PACES.iter().map(|(id, label, _)| json!({ "id": id, "label": moli_i18n::tr(label) })).collect::<Vec<_>>(),
        })
    }

    /// Today's exchanges (since midnight, the house's time) and what they
    /// cost, estimated at each model's rates.
    fn today(&self) -> Json {
        let midnight = jiff::Timestamp::now()
            .to_zoned(self.0.tz.clone())
            .start_of_day()
            .map_or(0, |d| d.timestamp().as_millisecond());
        let today = self.0.exchanges.since(midnight);
        let cost: f64 = today.iter().map(|e| e.cost.total).sum();
        json!({
            "exchanges": today.iter().filter(|e| e.kind == crate::ExchangeKind::Turn).count(),
            "dollars": (cost * 10_000.0).round() / 10_000.0,
        })
    }

    /// Who thinks, and with which model, chosen from the dashboard (a person
    /// with the code: it changes what Moli costs), kept for the next start.
    pub async fn set_brain(
        &self,
        provider: &str,
        model: Option<&str>,
    ) -> Result<Json, AssistantError> {
        let Some(provider) = Provider::from_id(provider) else {
            return Err(AssistantError::Invalid("unknown provider".into()));
        };
        let models = self.models(provider);
        if model.is_some_and(|m| !models.iter().any(|o| o == m)) {
            return Err(AssistantError::Invalid("unknown model".into()));
        }
        let current = self
            .0
            .prefs
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let c = &self.0.config;
        let prefs = Prefs {
            provider: Some(provider.id().to_owned()),
            // None picked: the provider's default, or `moli.toml`'s.
            model: model.map(str::to_owned),
            ..current.unwrap_or_else(|| Prefs {
                voice: c.speech_voice.clone(),
                style: style_id(&configured_style(&c.speech_style)).to_owned(),
                pace: None,
                provider: None,
                model: None,
            })
        };
        self.save_prefs(prefs).await?;
        let brain = self.brain();
        tracing::info!(provider = brain.provider.id(), model = %brain.model, "assistant model chosen from the dashboard");
        Ok(self.settings())
    }

    async fn save_prefs(&self, prefs: Prefs) -> Result<(), AssistantError> {
        let path = self
            .0
            .prefs_path
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(path) = path {
            let tmp = path.with_extension("json.tmp");
            let body = serde_json::to_vec_pretty(&prefs)
                .map_err(|e| AssistantError::Upstream(e.to_string()))?;
            tokio::fs::write(&tmp, body)
                .await
                .and(tokio::fs::rename(&tmp, &path).await)
                .map_err(|e| AssistantError::Upstream(format!("cannot save the settings: {e}")))?;
        }
        *self.0.prefs.write().unwrap_or_else(PoisonError::into_inner) = Some(prefs);
        Ok(())
    }

    /// Changes the voice, the tone and/or the pace, kept for the next start.
    pub async fn set_voice_prefs(
        &self,
        voice: Option<&str>,
        style: Option<&str>,
        pace: Option<&str>,
    ) -> Result<Json, AssistantError> {
        if voice.is_some_and(|v| !CLOUD_VOICES.contains(&v)) {
            return Err(AssistantError::Invalid("unknown voice".into()));
        }
        if style.is_some_and(|s| style_text(s).is_none()) {
            return Err(AssistantError::Invalid("unknown tone".into()));
        }
        if pace.is_some_and(|p| pace_speed(p).is_none()) {
            return Err(AssistantError::Invalid("unknown pace".into()));
        }
        let (current_voice, _) = self.voice_and_style();
        let current = self
            .0
            .prefs
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let (current_style, current_pace) = current.as_ref().map_or_else(
            || (STYLES[0].0.to_owned(), None),
            |p| (p.style.clone(), p.pace.clone()),
        );
        let prefs = Prefs {
            voice: voice.map_or(current_voice, str::to_owned),
            style: style.map_or(current_style, str::to_owned),
            pace: pace.map(str::to_owned).or(current_pace),
            provider: current.as_ref().and_then(|p| p.provider.clone()),
            model: current.and_then(|p| p.model),
        };
        tracing::info!(voice = %prefs.voice, style = %prefs.style, pace = ?prefs.pace, "voice chosen from the dashboard");
        self.save_prefs(prefs).await?;
        Ok(self.settings())
    }

    /// Tests a key against its provider (`None`: whoever thinks), then
    /// files it (encrypted). OpenAI's is tested where the voice goes when
    /// Anthropic thinks.
    pub async fn set_key(&self, key: &str, provider: Option<&str>) -> Result<(), AssistantError> {
        let key = key.trim();
        let provider = match provider {
            Some(p) => Provider::from_id(p)
                .ok_or_else(|| AssistantError::Invalid("unknown provider".into()))?,
            None => self.brain().provider,
        };
        if !plausible_key(key) {
            return Err(AssistantError::Invalid(moli_i18n::tr!(
                "assistant.settings.key_invalid"
            )));
        }
        let endpoint = match provider {
            Provider::Anthropic => &self.0.anthropic,
            Provider::Openai if self.brain().provider == Provider::Openai => &self.0.openai,
            Provider::Openai => &self.0.voice,
        };
        // A model in plain http never sees a key: not to check it either.
        if endpoint.is_local() {
            return Err(AssistantError::Invalid(moli_i18n::tr!(
                "assistant.settings.key_in_clear"
            )));
        }
        let accepted = endpoint
            .check(key)
            .await
            .map_err(|e| AssistantError::Upstream(e.to_string()))?;
        if !accepted {
            return Err(AssistantError::Invalid(moli_i18n::tr!(
                "assistant.settings.key_refused",
                provider = provider_name(provider)
            )));
        }
        self.0
            .hub
            .store_secret(&InstanceId::from("assistant"), provider.secret(), key)
            .await
            .map_err(|e| {
                AssistantError::Upstream(moli_i18n::tr!("assistant.settings.vault", error = e))
            })?;
        tracing::info!(
            provider = provider.id(),
            "assistant API key filed from the dashboard"
        );
        Ok(())
    }
}

/// A provider's name, as people know it.
fn provider_name(provider: Provider) -> &'static str {
    match provider {
        Provider::Anthropic => "Anthropic",
        Provider::Openai => "OpenAI",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_at_first_sight() {
        assert!(plausible_key("sk-proj-abcdefghijklmnopqrstuvwxyz0123"));
        assert!(!plausible_key("sk-court"));
        assert!(!plausible_key("sk-proj abcdefghijklmnopqrstuvwxyz"));
    }

    #[test]
    fn prefs_are_read_only_when_valid() {
        let dir = std::env::temp_dir().join(format!("moli-prefs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("assistant.json");
        std::fs::write(&path, r#"{"voice":"nova","style":"doux"}"#).unwrap();
        assert_eq!(
            read_prefs(&path),
            Some(Prefs {
                voice: "nova".into(),
                style: "doux".into(),
                pace: None,
                provider: None,
                model: None,
            })
        );
        // Who thinks is kept; an unknown provider is dropped with its model.
        std::fs::write(&path, r#"{"voice":"nova","style":"perso","provider":"anthropic","model":"claude-sonnet-5-5"}"#).unwrap();
        let read = read_prefs(&path).unwrap();
        assert_eq!(
            (read.provider.as_deref(), read.model.as_deref()),
            (Some("anthropic"), Some("claude-sonnet-5-5"))
        );
        std::fs::write(
            &path,
            r#"{"voice":"nova","style":"doux","provider":"robot","model":"r2"}"#,
        )
        .unwrap();
        let read = read_prefs(&path).unwrap();
        assert_eq!((read.provider, read.model), (None, None));
        // An unknown pace is dropped, the rest kept.
        std::fs::write(&path, r#"{"voice":"nova","style":"doux","pace":"fusee"}"#).unwrap();
        assert_eq!(read_prefs(&path).unwrap().pace, None);
        std::fs::write(&path, r#"{"voice":"nova","style":"doux","pace":"normal"}"#).unwrap();
        assert_eq!(read_prefs(&path).unwrap().pace.as_deref(), Some("normal"));
        std::fs::write(&path, r#"{"voice":"robot","style":"doux"}"#).unwrap();
        assert_eq!(read_prefs(&path), None);
        assert_eq!(read_prefs(&dir.join("absent.json")), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn every_tone_has_words() {
        for (id, label, text) in STYLES {
            assert!(!id.is_empty() && !label.is_empty() && text.len() > 30);
        }
    }

    #[test]
    fn the_french_tones_are_what_they_always_were() {
        assert_eq!(STYLES.len(), crate::before::STYLES.len());
        for ((id, label, text), (old_id, old_label, old_text)) in
            STYLES.iter().zip(crate::before::STYLES)
        {
            assert_eq!(*id, old_id);
            assert_eq!(moli_i18n::tr(label), old_label);
            assert_eq!(moli_i18n::tr(text), crate::before::reworded(old_text));
        }
    }

    #[test]
    fn the_default_tone_speaks_the_houses_language() {
        let cheerful = style_text("enjoue").unwrap();
        // moli.toml's built-in default is the cheerful tone, whatever the language.
        assert_eq!(crate::default_speech_style(), cheerful);
        assert_eq!(configured_style(&crate::default_speech_style()), cheerful);
        assert_eq!(configured_style("Voix de sorcière."), "Voix de sorcière.");
        assert_eq!(style_id(&style_text("doux").unwrap()), "doux");
        assert_eq!(style_id("Voix de sorcière."), "perso");
        let en = crate::tests::english();
        for (_, label, text) in STYLES {
            assert!(en[label].is_ascii() && en[text].is_ascii(), "{label}");
        }
    }

    #[tokio::test]
    async fn the_pace_is_chosen_and_kept() {
        let hub = moli_runtime::Hub::new(moli_runtime::HubOptions::default()).unwrap();
        let config = serde_json::from_value(json!({})).unwrap();
        let moli = Assistant::new(hub, None, None, None, config).unwrap();
        // Nothing chosen: moli.toml's speed, 1.2 by default, is « rapide ».
        assert_eq!(moli.settings()["pace"], "rapide");
        assert_eq!(moli.settings()["paces"].as_array().unwrap().len(), 3);
        let s = moli
            .set_voice_prefs(None, None, Some("normal"))
            .await
            .unwrap();
        assert_eq!(s["pace"], "normal");
        assert!((moli.speed() - 1.0).abs() < f64::EPSILON);
        // Another voice keeps the pace.
        let s = moli
            .set_voice_prefs(Some("sage"), None, None)
            .await
            .unwrap();
        assert_eq!(
            (s["voice"].as_str(), s["pace"].as_str()),
            (Some("sage"), Some("normal"))
        );
        assert!(matches!(
            moli.set_voice_prefs(None, None, Some("fusée")).await,
            Err(AssistantError::Invalid(_))
        ));
        // A file from before the pace still loads.
        let old: Prefs = serde_json::from_str(r#"{"voice":"coral","style":"enjoue"}"#).unwrap();
        assert_eq!(old.pace, None);
        assert_eq!(pace_id(1.33), "perso");
    }

    #[tokio::test]
    async fn a_refused_key_is_said_in_the_houses_words() {
        let hub = moli_runtime::Hub::new(moli_runtime::HubOptions::default()).unwrap();
        let config = serde_json::from_value(json!({})).unwrap();
        let moli = Assistant::new(hub, None, None, None, config).unwrap();
        match moli.set_key("sk-court", None).await {
            Err(AssistantError::Invalid(why)) => assert_eq!(
                why,
                "ce n'est pas une clé d'API (elle commence par sk-, sans espace)"
            ),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            moli_i18n::tr!("assistant.settings.key_refused", provider = "Anthropic"),
            "Anthropic refuse cette clé : vérifie-la, ou qu'elle n'a pas été supprimée"
        );
        assert_eq!(
            moli_i18n::tr!("assistant.settings.vault", error = "disque plein"),
            "coffre : disque plein"
        );
    }

    /// A model in plain http: a key given in the dashboard is refused before
    /// it goes anywhere.
    #[tokio::test]
    async fn a_key_is_never_sent_to_a_model_in_clear() {
        let hub = moli_runtime::Hub::new(moli_runtime::HubOptions::default()).unwrap();
        let (url, task) = crate::llm::tests::fake_http(200, br#"{"data":[]}"#).await;
        let config = serde_json::from_value(json!({ "base_url": url })).unwrap();
        let moli = Assistant::new(hub, None, None, None, config).unwrap();
        match moli
            .set_key("sk-proj-abcdefghijklmnopqrstuvwxyz", None)
            .await
        {
            Err(AssistantError::Invalid(why)) => {
                assert_eq!(why, moli_i18n::tr!("assistant.settings.key_in_clear"));
            }
            other => panic!("{other:?}"),
        }
        assert!(!task.is_finished(), "nothing was sent");
        task.abort();
    }
}
