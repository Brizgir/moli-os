//! The generic Android / Google TV driver (kind `androidtv`): everything
//! goes through Android TV Remote (keys, power, volume, the app on screen)
//! and Google Cast (what plays), with no maker API beside them.
//!
//! Pairing is built in: unpaired, the driver asks the TV for a code (it
//! shows 6 hexadecimal characters) and waits for a person to type it at the
//! dashboard (the `pairing_code` point, « secret »: never journaled nor
//! kept). The certificate Moli made is then stored (`atv_cert`, `atv_key`).

use std::time::Duration;

use anyhow::Context as _;
use moli_core::{Access, Device, DeviceId, Kind, PointSpec, Semantic, Unit, Value};
use moli_runtime::{BoxFuture, CommandRequest, Driver, DriverCtx};
use serde::Deserialize;

use crate::link::{Android, Waiting};
use crate::pairer::Pairer;

const TICK: Duration = Duration::from_secs(5);
/// Volume steps one order may press (a slider moves by a few).
const MAX_STEPS: u64 = 40;

const POWER: u32 = 26;
const VOLUME_UP: u32 = 24;
const VOLUME_DOWN: u32 = 25;
const VOLUME_MUTE: u32 = 164;

/// The keys, by the names Moli's remotes send (the same for every TV), and
/// their Android key codes.
pub const KEYS: [(&str, u32); 34] = [
    ("CursorUp", 19),
    ("CursorDown", 20),
    ("CursorLeft", 21),
    ("CursorRight", 22),
    ("Confirm", 23),
    ("Back", 4),
    ("Home", 3),
    ("Options", 82),
    ("Info", 165),
    ("PlayPause", 85),
    ("Play", 126),
    ("Pause", 127),
    ("Stop", 86),
    ("Next", 87),
    ("Previous", 88),
    ("FastForward", 90),
    ("Rewind", 89),
    ("VolumeUp", VOLUME_UP),
    ("VolumeDown", VOLUME_DOWN),
    ("Mute", VOLUME_MUTE),
    ("ChannelStepUp", 166),
    ("ChannelStepDown", 167),
    ("Source", 178),
    ("Settings", 176),
    ("Digit0", 7),
    ("Digit1", 8),
    ("Digit2", 9),
    ("Digit3", 10),
    ("Digit4", 11),
    ("Digit5", 12),
    ("Digit6", 13),
    ("Digit7", 14),
    ("Digit8", 15),
    ("Digit9", 16),
];

fn code_of(key: &str) -> Option<u32> {
    KEYS.iter().find(|(k, _)| *k == key).map(|(_, c)| *c)
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub host: String,
    /// For Wake-on-LAN from deep standby, `02:00:00:00:00:02`.
    #[serde(default)]
    pub mac: Option<String>,
    /// The device's name (default: « Télé »).
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug)]
pub struct AndroidTv {
    config: Config,
}

impl AndroidTv {
    #[must_use]
    pub fn new(config: Config) -> Self {
        Self { config }
    }
}

impl Driver for AndroidTv {
    fn kind(&self) -> &'static str {
        "androidtv"
    }

    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(run(&self.config, ctx))
    }
}

fn spec(key: &str, label: &str, kind: Kind, access: Access, semantic: Semantic) -> PointSpec {
    PointSpec {
        key: key.into(),
        label: label.into(),
        kind,
        access,
        unit: None,
        semantic,
    }
}

const RW: Access = Access {
    read: true,
    write: true,
};
const R: Access = Access {
    read: true,
    write: false,
};
/// An order, not a state: nothing to read back.
const W: Access = Access {
    read: false,
    write: true,
};

fn enumeration(values: &[&str]) -> Kind {
    Kind::Enum {
        values: values.iter().map(|v| (*v).into()).collect(),
    }
}

#[allow(clippy::too_many_lines)] // one entry per point
fn device(ctx: &DriverCtx, id: &DeviceId, name: &str, max_volume: u64) -> Device {
    let tr = |key: &str| moli_i18n::tr!(key);
    #[allow(clippy::cast_precision_loss)]
    let max = max_volume as f64;
    let seconds = |key: &str, label: &str| PointSpec {
        unit: Some(Unit::Second),
        ..spec(
            key,
            label,
            Kind::Numeric {
                min: Some(0.0),
                max: None,
                step: None,
            },
            R,
            Semantic::Other,
        )
    };
    Device {
        id: id.clone(),
        instance: ctx.instance().clone(),
        native_name: name.into(),
        manufacturer: None,
        model: Some("Android TV".into()),
        description: None,
        native_room: None,
        members: Vec::new(),
        points: vec![
            spec(
                "power",
                &tr("pilotes.androidtv.allumee"),
                Kind::Binary,
                RW,
                Semantic::OnOff,
            ),
            spec(
                "volume",
                &tr("pilotes.androidtv.volume"),
                Kind::Numeric {
                    min: Some(0.0),
                    max: Some(max),
                    step: Some(1.0),
                },
                RW,
                Semantic::infer("volume", None),
            ),
            spec(
                "mute",
                &tr("pilotes.androidtv.sourdine"),
                Kind::Binary,
                RW,
                Semantic::infer("mute", None),
            ),
            // Written: opens that app (its name, Android package) or link.
            spec(
                "app",
                &tr("pilotes.androidtv.application"),
                Kind::Text,
                RW,
                Semantic::Other,
            ),
            spec(
                "text",
                &tr("pilotes.androidtv.texte"),
                Kind::Text,
                W,
                Semantic::Other,
            ),
            spec(
                "key",
                &tr("pilotes.androidtv.touche"),
                Kind::Enum {
                    values: KEYS.iter().map(|(k, _)| (*k).into()).collect(),
                },
                W,
                Semantic::Other,
            ),
            spec(
                "media_app",
                &tr("pilotes.androidtv.appli_en_lecture"),
                Kind::Text,
                R,
                Semantic::Other,
            ),
            spec(
                "media_state",
                &tr("pilotes.androidtv.lecture"),
                enumeration(&["playing", "paused", "buffering", "idle"]),
                R,
                Semantic::Other,
            ),
            spec(
                "media_title",
                &tr("pilotes.androidtv.titre"),
                Kind::Text,
                R,
                Semantic::Other,
            ),
            spec(
                "media_subtitle",
                &tr("pilotes.androidtv.serie_artiste"),
                Kind::Text,
                R,
                Semantic::Other,
            ),
            spec(
                "media_image",
                &tr("pilotes.androidtv.image"),
                Kind::Text,
                R,
                Semantic::Other,
            ),
            seconds("media_duration", &tr("pilotes.androidtv.duree")),
            seconds("media_position", &tr("pilotes.androidtv.position")),
            spec(
                "media_control",
                &tr("pilotes.androidtv.lecture_commande"),
                enumeration(&["play", "pause", "stop"]),
                W,
                Semantic::Other,
            ),
        ]
        .into_iter()
        .chain(crate::pairer::points())
        .collect(),
    }
}

/// The key presses that bring the volume from `level` to `target`.
fn volume_steps(level: u64, target: u64) -> Vec<u32> {
    let (code, n) = if target >= level {
        (VOLUME_UP, target - level)
    } else {
        (VOLUME_DOWN, level - target)
    };
    let n = usize::try_from(n.min(MAX_STEPS)).unwrap_or(0);
    vec![code; n]
}

/// Until `at`; forever without one.
async fn until(at: Option<tokio::time::Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

struct Tv<'a> {
    config: &'a Config,
    id: DeviceId,
    name: String,
    max: u64,
    link: Android,
    pairer: Pairer,
}

impl Tv<'_> {
    async fn order(&mut self, command: CommandRequest, ctx: &DriverCtx) {
        let Some(command) = self
            .pairer
            .order(command, ctx, &self.id, &mut self.link)
            .await
        else {
            return;
        };
        match (&*command.key, &command.value) {
            ("app", Value::Text(target)) => {
                let target = target.to_string();
                self.link.launch(&target, Waiting::order(command));
            }
            _ if !self.link.is_paired() => {
                command.reply(Err(moli_i18n::tr!("pilotes.androidtv.appairage")));
            }
            ("power", Value::Bool(true)) if !self.link.remote_up() => {
                command.reply(wake(self.config).await);
            }
            _ => {
                let result = self.apply(&command).map_err(|e| format!("{e:#}"));
                command.reply(result);
            }
        }
    }

    fn apply(&self, command: &CommandRequest) -> anyhow::Result<()> {
        let link = &self.link;
        match (&*command.key, &command.value) {
            // POWER toggles: pressed only when it changes something.
            ("power", Value::Bool(on)) => {
                if link.power() != Some(*on) {
                    link.press(&[POWER])?;
                }
            }
            ("volume", v) => {
                let (level, _, _) = link
                    .volume()
                    .with_context(|| moli_i18n::tr!("pilotes.androidtv.volume_inconnu"))?;
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let target = v.as_f64().context("volume")?.round().clamp(0.0, 1000.0) as u64;
                link.press(&volume_steps(level, target.min(self.max)))?;
            }
            ("mute", Value::Bool(muted)) => {
                if link.volume().is_none_or(|(_, _, now)| now != *muted) {
                    link.press(&[VOLUME_MUTE])?;
                }
            }
            ("key", Value::Text(key)) => {
                let code = code_of(key).with_context(|| {
                    moli_i18n::tr!("pilotes.androidtv.touche_inconnue", key = key)
                })?;
                link.press(&[code])?;
            }
            ("text", Value::Text(text)) => link.write(text)?,
            ("media_control", Value::Text(word)) => link.control(word)?,
            (key, _) => anyhow::bail!(moli_i18n::tr!(
                "pilotes.androidtv.ne_se_regle_pas",
                point = key
            )),
        }
        Ok(())
    }
}

/// The remote is down: the TV sleeps deeply (or is gone). Wake-on-LAN.
async fn wake(config: &Config) -> Result<(), String> {
    let Some(mac) = &config.mac else {
        return Err(moli_i18n::tr!("pilotes.androidtv.sans_mac"));
    };
    moli_net::wol::wake(mac, Some(&config.host))
        .await
        .map_err(|e| moli_i18n::tr!("pilotes.androidtv.reveil", error = format!("{e:#}")))
}

async fn run(config: &Config, ctx: &mut DriverCtx) -> anyhow::Result<()> {
    let id = ctx.device_id(&config.host);
    let name = config
        .name
        .clone()
        .unwrap_or_else(|| moli_i18n::tr!("pilotes.androidtv.tele"));
    let max = 100;
    ctx.upsert_device(device(ctx, &id, &name, max));
    let (link, mut ups) = Android::start(ctx, &config.host);
    let paired = link.is_paired();
    let mut tv = Tv {
        config,
        id,
        name,
        max,
        link: link.alone(),
        pairer: Pairer::new(&config.host, true, paired),
    };
    tv.pairer
        .show(ctx, &tv.id, if paired { "paired" } else { "needed" });
    let mut online = None;
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            command = ctx.next_command() => {
                let Some(command) = command else { return Ok(()) };
                tv.order(command, ctx).await;
            }
            Some(up) = ups.recv() => {
                tv.link.on(up, ctx, &tv.id).await;
                if let Some((_, seen, _)) = tv.link.volume()
                    && seen > 0
                    && seen != tv.max
                {
                    tv.max = seen;
                    ctx.upsert_device(device(ctx, &tv.id, &tv.name, tv.max));
                }
            }
            () = until(tv.link.due()) => tv.link.settle(),
            result = tv.pairer.started() => tv.pairer.on_started(result, ctx, &tv.id),
            _ = tick.tick() => {
                tv.link.tick(true);
                tv.pairer.tick(ctx, tv.link.is_paired());
            }
        }
        // Unpaired, the device is there to be paired.
        let now = tv.link.remote_up() || !tv.link.is_paired();
        if online != Some(now) {
            online = Some(now);
            ctx.set_availability(&tv.id, now);
            if !now && tv.link.is_paired() {
                ctx.set_state(&tv.id, "power", Value::Bool(false));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keys_are_the_ones_every_remote_sends() {
        for key in [
            "CursorUp",
            "CursorDown",
            "CursorLeft",
            "CursorRight",
            "Confirm",
            "Back",
            "Home",
            "Options",
            "PlayPause",
            "Previous",
            "Next",
            "VolumeUp",
            "VolumeDown",
            "Mute",
        ] {
            assert!(code_of(key).is_some(), "{key}");
        }
        // Android's own codes (KeyEvent).
        assert_eq!(code_of("Confirm"), Some(23));
        assert_eq!(code_of("Back"), Some(4));
        assert_eq!(code_of("Mute"), Some(164));
        assert_eq!(code_of("Digit9"), Some(16));
        assert_eq!(code_of("Explode"), None);
    }

    #[test]
    fn the_volume_is_reached_by_steps() {
        assert_eq!(volume_steps(10, 13), [VOLUME_UP; 3]);
        assert_eq!(volume_steps(10, 8), [VOLUME_DOWN; 2]);
        assert!(volume_steps(10, 10).is_empty());
        assert_eq!(volume_steps(0, 100).len(), 40, "at most 40 presses");
    }
}
