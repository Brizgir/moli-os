//! The Orange TV decoder (Sagem / Arcadyan, the Livebox's TV box): its
//! local remote control, plain HTTP on port 8080, no pairing.
//!
//! - `GET /remoteControl/cmd?operation=10`: its state (standby or on, what
//!   is on screen, its MAC address, which is who it is);
//! - `GET /remoteControl/cmd?operation=01&key=<code>&mode=0`: one key press
//!   (codes are Linux input codes: 103 up, 352 OK, 116 power…).
//!
//! Power is a single toggle key: it is only pressed when the decoder is not
//! already where it was asked to be.

use std::time::Duration;

use moli_core::{Access, Device, DeviceId, Kind, PointSpec, Semantic, Value};
use moli_net::upnp;
use moli_runtime::{BoxFuture, CommandRequest, Driver, DriverCtx};
use serde::Deserialize;

const POLL: Duration = Duration::from_secs(5);
const DEFAULT_PORT: u16 = 8080;
const CMD: &str = "/remoteControl/cmd";
/// The power toggle.
const POWER: u16 = 116;

/// The keys the decoder takes, by the names Moli's remotes send, and their
/// codes. The same names as the other TVs' (`CursorUp`, `Confirm`…), so one
/// remote drives them all.
pub const KEYS: [(&str, u16); 27] = [
    ("CursorUp", 103),
    ("CursorDown", 108),
    ("CursorLeft", 105),
    ("CursorRight", 106),
    ("Confirm", 352),
    ("Back", 158),
    ("Home", 139),
    ("Options", 139),
    ("PlayPause", 164),
    ("Play", 164),
    ("Pause", 164),
    ("Next", 159),
    ("Previous", 168),
    ("FastForward", 159),
    ("Rewind", 168),
    ("Record", 167),
    ("VolumeUp", 115),
    ("VolumeDown", 114),
    ("Mute", 113),
    ("ChannelStepUp", 402),
    ("ChannelStepDown", 403),
    ("Digit0", 512),
    ("Digit1", 513),
    ("Digit2", 514),
    ("Digit3", 515),
    ("Digit4", 516),
    ("Digit5", 517),
];

/// The other digits, kept apart so [`KEYS`] reads as the remote does.
const MORE_DIGITS: [(&str, u16); 4] = [
    ("Digit6", 518),
    ("Digit7", 519),
    ("Digit8", 520),
    ("Digit9", 521),
];

fn code_of(key: &str) -> Option<u16> {
    KEYS.iter()
        .chain(MORE_DIGITS.iter())
        .find(|(k, _)| *k == key)
        .map(|(_, c)| *c)
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// The decoder's address on the LAN.
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_port() -> u16 {
    DEFAULT_PORT
}

#[derive(Debug)]
pub struct Orange {
    config: Config,
}

impl Orange {
    #[must_use]
    pub fn new(config: Config) -> Self {
        Self { config }
    }
}

impl Driver for Orange {
    fn kind(&self) -> &'static str {
        "orange"
    }

    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(run(&self.config, ctx))
    }
}

/// What `operation=10` says.
#[derive(Debug, PartialEq)]
struct Status {
    on: bool,
    screen: Option<String>,
    mac: Option<String>,
    name: Option<String>,
}

/// Reads a decoder's answer: `{"result": {"responseCode": "0", "data": {…}}}`.
fn parse_status(body: &str) -> anyhow::Result<Status> {
    let json: serde_json::Value = serde_json::from_str(body)?;
    let result = &json["result"];
    let code = result["responseCode"].as_str().unwrap_or("?");
    anyhow::ensure!(code == "0", "decoder answered {code}");
    let data = &result["data"];
    let text = |k: &str| {
        data[k]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty() && *s != "NA")
            .map(str::to_owned)
    };
    Ok(Status {
        // 0: active, 1: standby.
        on: text("activeStandbyState").as_deref() == Some("0"),
        screen: text("osdContext"),
        mac: text("macAddress"),
        name: text("friendlyName"),
    })
}

/// Whether a key press was taken.
fn parse_ack(body: &str) -> Result<(), String> {
    let json: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    match json["result"]["responseCode"].as_str() {
        Some("0") => Ok(()),
        other => Err(moli_i18n::tr!(
            "pilotes.orange.refus",
            code = other.unwrap_or("?")
        )),
    }
}

/// The MAC address as an identity: `9c24…`, no separators.
fn identity(mac: &str) -> String {
    mac.chars()
        .filter(char::is_ascii_hexdigit)
        .collect::<String>()
        .to_ascii_lowercase()
}

fn device(ctx: &DriverCtx, id: &DeviceId, name: &str) -> Device {
    let point =
        |key: &str, label: String, kind: Kind, read: bool, write: bool, semantic| PointSpec {
            key: key.into(),
            label: label.into(),
            kind,
            access: Access { read, write },
            unit: None,
            semantic,
        };
    Device {
        id: id.clone(),
        instance: ctx.instance().clone(),
        native_name: name.into(),
        manufacturer: Some("Orange".into()),
        model: Some(moli_i18n::tr!("pilotes.orange.decodeur").into()),
        description: None,
        native_room: None,
        members: Vec::new(),
        points: vec![
            point(
                "power",
                moli_i18n::tr!("pilotes.orange.allume"),
                Kind::Binary,
                true,
                true,
                Semantic::OnOff,
            ),
            point(
                "screen",
                moli_i18n::tr!("pilotes.orange.a_l_ecran"),
                Kind::Text,
                true,
                false,
                Semantic::Other,
            ),
            // A press, not a state: nothing to read back.
            point(
                "key",
                moli_i18n::tr!("pilotes.orange.touche"),
                Kind::Enum {
                    values: KEYS
                        .iter()
                        .chain(MORE_DIGITS.iter())
                        .map(|(k, _)| (*k).into())
                        .collect(),
                },
                false,
                true,
                Semantic::Other,
            ),
        ],
    }
}

struct Decoder<'a> {
    host: &'a str,
    port: u16,
}

impl Decoder<'_> {
    async fn status(&self) -> anyhow::Result<Status> {
        let body = upnp::fetch(self.host, self.port, &format!("{CMD}?operation=10")).await?;
        parse_status(&body)
    }

    async fn press(&self, code: u16) -> Result<(), String> {
        let path = format!("{CMD}?operation=01&key={code}&mode=0");
        let body = upnp::fetch(self.host, self.port, &path)
            .await
            .map_err(|e| moli_i18n::tr!("pilotes.orange.injoignable", error = format!("{e:#}")))?;
        parse_ack(&body)
    }
}

/// One order: a key, or power (pressed only if it changes something).
async fn order(
    decoder: &Decoder<'_>,
    on: Option<bool>,
    command: &CommandRequest,
) -> Result<(), String> {
    match (&*command.key, &command.value) {
        ("key", Value::Text(key)) => match code_of(key) {
            Some(code) => decoder.press(code).await,
            None => Err(moli_i18n::tr!(
                "pilotes.orange.touche_inconnue",
                key = key.to_string()
            )),
        },
        ("power", Value::Bool(wanted)) => {
            if on == Some(*wanted) {
                Ok(())
            } else {
                decoder.press(POWER).await
            }
        }
        _ => Err(moli_i18n::tr!("pilotes.orange.ordre_inconnu")),
    }
}

fn publish(ctx: &DriverCtx, id: &DeviceId, status: &Status) {
    ctx.set_state(id, "power", Value::Bool(status.on));
    let screen = status
        .screen
        .as_deref()
        .map_or(Value::Null, |s| Value::Text(s.into()));
    ctx.set_state(id, "screen", screen);
}

async fn run(config: &Config, ctx: &mut DriverCtx) -> anyhow::Result<()> {
    let decoder = Decoder {
        host: &config.host,
        port: config.port,
    };
    // Who it is comes from its first answer: wait for it, orders refused.
    let first = loop {
        match decoder.status().await {
            Ok(status) => break status,
            Err(e) => {
                ctx.wait_for(moli_i18n::tr!(
                    "pilotes.orange.injoignable",
                    error = format!("{e:#}")
                ));
                let pause = tokio::time::sleep(Duration::from_secs(30));
                tokio::pin!(pause);
                loop {
                    tokio::select! {
                        () = &mut pause => break,
                        command = ctx.next_command() => match command {
                            Some(c) => c.reply(Err(moli_i18n::tr!("pilotes.orange.injoignable", error = format!("{e:#}")))),
                            None => return Ok(()),
                        },
                    }
                }
            }
        }
    };
    let native = first
        .mac
        .as_deref()
        .map(identity)
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| config.host.replace([':', '/'], "_"));
    let id = ctx.device_id(&native);
    let name = first
        .name
        .clone()
        .unwrap_or_else(|| moli_i18n::tr!("pilotes.orange.decodeur"));
    ctx.upsert_device(device(ctx, &id, &name));
    ctx.set_availability(&id, true);
    publish(ctx, &id, &first);
    ctx.ready();
    let mut on = Some(first.on);
    let mut online = true;
    let mut tick = tokio::time::interval(POLL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            command = ctx.next_command() => {
                let Some(command) = command else { return Ok(()) };
                let result = order(&decoder, on, &command).await;
                let changed = result.is_ok() && &*command.key == "power";
                command.reply(result);
                if changed && let Ok(status) = decoder.status().await {
                    on = Some(status.on);
                    publish(ctx, &id, &status);
                }
            }
            _ = tick.tick() => match decoder.status().await {
                Ok(status) => {
                    if !online {
                        online = true;
                        ctx.set_availability(&id, true);
                    }
                    on = Some(status.on);
                    publish(ctx, &id, &status);
                }
                Err(e) => {
                    if online {
                        online = false;
                        on = None;
                        ctx.set_availability(&id, false);
                        tracing::warn!(instance = %ctx.instance(), error = %format!("{e:#}"), "orange decoder unreachable");
                    }
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ON: &str = include_str!("../../../integrations/orange/fixtures/status-on.json");
    const STANDBY: &str = include_str!("../../../integrations/orange/fixtures/status-standby.json");
    const WHD94: &str =
        include_str!("../../../integrations/orange/fixtures/status-homepage-whd94.json");

    #[test]
    fn reads_the_decoder_state() {
        assert_eq!(
            parse_status(ON).unwrap(),
            Status {
                on: true,
                screen: Some("LIVE".into()),
                mac: Some("02:00:00:00:00:01".into()),
                name: Some("Decodeur TV Orange".into()),
            }
        );
        let standby = parse_status(STANDBY).unwrap();
        assert!(!standby.on);
        assert_eq!(standby.screen, None, "« NA » is no value");
    }

    #[test]
    fn the_older_decoder_answers_the_same() {
        // A Sagem WHD94 (an older kernel), on its home page.
        assert_eq!(
            parse_status(WHD94).unwrap(),
            Status {
                on: true,
                screen: Some("HOMEPAGE".into()),
                mac: Some("02:00:00:00:00:02".into()),
                name: Some("Decodeur TV UHD".into()),
            }
        );
    }

    #[test]
    fn a_refusal_is_an_error() {
        assert!(parse_status(r#"{"result":{"responseCode":"-1","message":"ko"}}"#).is_err());
        assert!(parse_ack(r#"{"result":{"responseCode":"0","message":"ok","data":{}}}"#).is_ok());
        assert!(parse_ack(r#"{"result":{"responseCode":"-2","message":"ko"}}"#).is_err());
    }

    #[test]
    fn every_key_has_a_code_and_the_mac_is_the_identity() {
        assert_eq!(code_of("Confirm"), Some(352));
        assert_eq!(code_of("Digit9"), Some(521));
        assert_eq!(code_of("Source"), None);
        assert_eq!(identity("02:00:00:00:00:01"), "020000000001");
    }
}
