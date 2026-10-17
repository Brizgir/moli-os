//! Ring doorbells, cameras and chimes, through Ring's own cloud API
//! (unofficial: Ring publishes nothing, and its devices answer nothing on
//! the home network): a press at the door, motion, battery, Wi-Fi, a
//! chime's volume, read only. The account is connected once from the
//! dashboard: e-mail and password, then the code Ring sends; Moli keeps
//! only the refresh token Ring gives, in the encrypted secret store, and
//! writes it again each time Ring replaces it.
//!
//! Presses and motion are read from the devices' latest events every few
//! seconds (not Ring's push notifications): an event is told once, only if
//! it is less than [`model::FRESH`] old, and never from the first reading
//! after a start, so an old press never reads as someone at the door.

mod api;
mod auth;
mod model;
mod net;
mod signin;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use moli_core::{Access, Device, DeviceId, Kind, PointSpec, Semantic, Value};
use moli_runtime::{BoxFuture, Driver, DriverCtx};
use serde::Deserialize;
use tokio::time::{Instant, MissedTickBehavior};

use crate::api::{Client, Failure};
use crate::model::{Family, Happened, RingDevice, Watch};
use crate::net::{Transport, Web};
use crate::signin::{Refusal, SignIn};

const REFRESH: &str = "refresh_token";
const HARDWARE_ID: &str = "hardware_id";
/// The account's devices (one added, renamed, offline…).
const DEVICES: Duration = Duration::from_secs(5 * 60);
/// Wi-Fi and battery.
const HEALTH: Duration = Duration::from_secs(15 * 60);
/// A press shows this long on `doorbell`.
const PRESS: Duration = Duration::from_secs(10);
/// Motion stays on this long after the last one Ring saw.
const MOTION: Duration = Duration::from_secs(60);
const RETRY_MIN: Duration = Duration::from_secs(30);
const RETRY_MAX: Duration = Duration::from_secs(15 * 60);
/// Ring asked to slow down: this long before trying again.
const SLOW_DOWN: Duration = Duration::from_secs(10 * 60);
const SIGN_IN_AGAIN: &str = "Ring no longer accepts this sign-in";
const TOO_MANY: &str = "Ring asks to slow down";

fn default_poll() -> u64 {
    10
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Seconds between two readings of the latest events (5 at least).
    #[serde(default = "default_poll")]
    pub poll: u64,
}

impl Config {
    fn poll(&self) -> Duration {
        Duration::from_secs(self.poll.max(5))
    }
}

#[derive(Debug)]
pub struct Ring {
    config: Config,
}

impl Ring {
    #[must_use]
    pub fn new(config: Config) -> Self {
        Self { config }
    }
}

impl Driver for Ring {
    fn kind(&self) -> &'static str {
        "ring"
    }

    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(run(&self.config, ctx))
    }
}

/// The new refresh token, kept in the instance's encrypted store.
struct Store<'a>(&'a DriverCtx);

impl api::Vault for Store<'_> {
    fn keep<'a>(&'a self, refresh: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move { Ok(self.0.store_secret(REFRESH, refresh).await?) })
    }
}

fn text_point(key: &str, label: &str, write: bool, semantic: Semantic) -> PointSpec {
    PointSpec {
        key: key.into(),
        label: label.into(),
        kind: Kind::Text,
        access: Access { read: true, write },
        unit: None,
        semantic,
    }
}

/// The account: its state, and what the dashboard sends to sign it in.
fn account_device(ctx: &DriverCtx, id: &DeviceId) -> Device {
    Device {
        id: id.clone(),
        instance: ctx.instance().clone(),
        native_name: moli_i18n::tr!("pilotes.ring.compte").into(),
        manufacturer: Some("Ring".into()),
        model: Some("Ring".into()),
        description: Some(moli_i18n::tr!("pilotes.ring.description").into()),
        native_room: None,
        members: Vec::new(),
        points: vec![
            text_point(
                "status",
                &moli_i18n::tr!("pilotes.ring.etat_compte"),
                false,
                Semantic::Config,
            ),
            text_point(
                "code_by",
                &moli_i18n::tr!("pilotes.ring.code_par"),
                false,
                Semantic::Config,
            ),
            // What a person types: never journaled nor kept (`Secret`).
            text_point(
                "sign_in",
                &moli_i18n::tr!("pilotes.ring.identifiants"),
                true,
                Semantic::Secret,
            ),
            text_point(
                "code",
                &moli_i18n::tr!("pilotes.ring.code"),
                true,
                Semantic::Secret,
            ),
        ],
    }
}

fn set_text(ctx: &DriverCtx, id: &DeviceId, key: &str, text: &str) {
    ctx.set_state(id, key, Value::Text(text.into()));
}

async fn run(config: &Config, ctx: &mut DriverCtx) -> anyhow::Result<()> {
    let net: Arc<dyn Transport> = Arc::new(Web);
    let account = ctx.device_id("account");
    ctx.upsert_device(account_device(ctx, &account));
    ctx.set_availability(&account, true);
    set_text(ctx, &account, "code_by", "");
    let mut pause = RETRY_MIN;
    loop {
        let client = if let (Some(refresh), Some(hardware_id)) =
            (ctx.secret(REFRESH), ctx.secret(HARDWARE_ID))
        {
            Client::new(Arc::clone(&net), &hardware_id, &refresh)
        } else {
            let Some(client) = sign_in(ctx, &account, &net).await? else {
                return Ok(());
            };
            client
        };
        set_text(ctx, &account, "status", "connecting");
        let started = Instant::now();
        let failure = match connected(config, ctx, &account, client).await {
            Ok(()) => return Ok(()),
            Err(e) => format!("{e:#}"),
        };
        if failure.contains(SIGN_IN_AGAIN) {
            tracing::warn!(instance = %ctx.instance(), "ring: sign in again");
            ctx.forget_secret(REFRESH).await?;
            continue;
        }
        let wait = if failure.contains(TOO_MANY) {
            set_text(ctx, &account, "status", "slow_down");
            SLOW_DOWN
        } else {
            set_text(ctx, &account, "status", "error");
            if started.elapsed() > RETRY_MAX {
                pause = RETRY_MIN;
            }
            let wait = pause;
            pause = (pause * 2).min(RETRY_MAX);
            wait
        };
        tracing::warn!(instance = %ctx.instance(), error = %failure, "ring unreachable");
        tokio::select! {
            () = tokio::time::sleep(wait) => {}
            () = ctx.cancelled() => return Ok(()),
        }
    }
}

fn refusal(r: &Refusal) -> String {
    match r {
        Refusal::Typed => moli_i18n::tr!("pilotes.ring.identifiants_faux"),
        Refusal::BadPassword => moli_i18n::tr!("pilotes.ring.mauvais_identifiants"),
        Refusal::BadCode => moli_i18n::tr!("pilotes.ring.code_faux"),
        Refusal::NoCredentials => moli_i18n::tr!("pilotes.ring.code_attendu"),
        Refusal::TooMany => moli_i18n::tr!("pilotes.ring.trop"),
        Refusal::Other(why) => moli_i18n::tr!("pilotes.ring.refuse", why = why),
    }
}

/// Waits for the person to sign in from the dashboard; the client, signed
/// in and kept. `None`: stopped.
async fn sign_in(
    ctx: &mut DriverCtx,
    account: &DeviceId,
    net: &Arc<dyn Transport>,
) -> anyhow::Result<Option<Client>> {
    let hardware_id = match ctx.secret(HARDWARE_ID) {
        Some(id) => id,
        None => auth::hardware_id()?,
    };
    let mut pending = SignIn::new(hardware_id);
    let show = |ctx: &DriverCtx, pending: &SignIn| {
        let by = pending.code_by().unwrap_or_default();
        set_text(ctx, account, "code_by", by);
        set_text(
            ctx,
            account,
            "status",
            if by.is_empty() { "sign_in" } else { "code" },
        );
        ctx.wait_for(if by.is_empty() {
            moli_i18n::tr!("pilotes.ring.a_connecter")
        } else {
            moli_i18n::tr!("pilotes.ring.a_confirmer")
        });
    };
    show(ctx, &pending);
    loop {
        let forget_at = pending.deadline();
        let command = tokio::select! {
            command = ctx.next_command() => command,
            () = async {
                match forget_at {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending().await,
                }
            } => {
                pending.forget();
                show(ctx, &pending);
                continue;
            }
        };
        // `None` once Moli stops.
        let Some(command) = command else {
            return Ok(None);
        };
        let (Value::Text(typed), true) = (&command.value, command.device.id == *account) else {
            command.reply(Err(moli_i18n::tr!("pilotes.ring.pas_connecte")));
            continue;
        };
        let typed = typed.clone();
        let tokens = match &*command.key {
            "sign_in" => pending.credentials(&**net, &typed).await,
            "code" => pending.code(&**net, &typed).await.map(Some),
            _ => {
                command.reply(Err(moli_i18n::tr!("pilotes.ring.pas_connecte")));
                continue;
            }
        };
        drop(typed);
        match tokens {
            Ok(Some(tokens)) => {
                let kept = async {
                    ctx.store_secret(HARDWARE_ID, &pending.hardware_id).await?;
                    ctx.store_secret(REFRESH, &tokens.refresh).await
                };
                if let Err(e) = kept.await {
                    command.reply(Err(moli_i18n::tr!("pilotes.ring.coffre", why = e)));
                    return Err(e.into());
                }
                set_text(ctx, account, "code_by", "");
                command.reply(Ok(()));
                tracing::info!(instance = %ctx.instance(), "ring account connected");
                ctx.ready();
                return Ok(Some(Client::signed_in(
                    Arc::clone(net),
                    &pending.hardware_id,
                    &tokens,
                )));
            }
            Ok(None) => {
                tracing::info!(instance = %ctx.instance(), by = ?pending.code_by(), "ring asks for a code");
                command.reply(Ok(()));
            }
            Err(r) => {
                if let Refusal::Other(why) = &r {
                    tracing::warn!(instance = %ctx.instance(), error = %why, "ring sign-in refused");
                }
                command.reply(Err(refusal(&r)));
            }
        }
        show(ctx, &pending);
    }
}

/// One doorbell or camera, chime: what Moli tracks of it.
struct Tracked {
    device: RingDevice,
    watch: Watch,
    /// Its events are read from the older `history` (the newer place
    /// answered 404).
    history: bool,
    /// Its events could not be read, said once.
    unreadable: bool,
}

/// Connected: the devices, then their events and health, until stopped.
async fn connected(
    config: &Config,
    ctx: &mut DriverCtx,
    account: &DeviceId,
    mut client: Client,
) -> anyhow::Result<()> {
    let fail = |e: Failure| match e {
        Failure::SignInAgain => anyhow::anyhow!("{SIGN_IN_AGAIN}"),
        Failure::TooMany => anyhow::anyhow!("{TOO_MANY}"),
        Failure::Other(e) => e,
    };
    match client.session(&Store(ctx)).await {
        Ok(()) | Err(Failure::Other(_)) => {}
        Err(e) => return Err(fail(e)),
    }
    let mut tracked: HashMap<DeviceId, Tracked> = HashMap::new();
    discover(ctx, &mut client, &mut tracked)
        .await
        .map_err(fail)?;
    health(ctx, &mut client, &tracked).await.map_err(fail)?;
    set_text(ctx, account, "status", "connected");
    ctx.ready();
    tracing::info!(instance = %ctx.instance(), devices = tracked.len(), "ring connected");
    let mut poll = tokio::time::interval(config.poll());
    let mut devices = tokio::time::interval(DEVICES);
    let mut healths = tokio::time::interval(HEALTH);
    for timer in [&mut poll, &mut devices, &mut healths] {
        timer.set_missed_tick_behavior(MissedTickBehavior::Delay);
        timer.reset();
    }
    // Points to put back to « off », and when.
    let mut resets: HashMap<(DeviceId, &'static str), Instant> = HashMap::new();
    poll.reset_immediately();
    loop {
        let next_reset = resets.values().min().copied();
        tokio::select! {
            command = ctx.next_command() => {
                let Some(command) = command else { return Ok(()) };
                let why = if command.device.id == *account {
                    moli_i18n::tr!("pilotes.ring.deja_connecte")
                } else {
                    moli_i18n::tr!("pilotes.ring.lecture_seule")
                };
                command.reply(Err(why));
            }
            _ = poll.tick() => events(ctx, &mut client, &mut tracked, &mut resets).await.map_err(fail)?,
            _ = devices.tick() => discover(ctx, &mut client, &mut tracked).await.map_err(fail)?,
            _ = healths.tick() => health(ctx, &mut client, &tracked).await.map_err(fail)?,
            () = async {
                match next_reset {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending().await,
                }
            } => {
                let now = Instant::now();
                resets.retain(|(id, key), at| {
                    let due = *at <= now;
                    if due {
                        ctx.set_state(id, key, Value::Bool(false));
                    }
                    !due
                });
            }
        }
    }
}

/// The account's devices, shown in the house.
async fn discover(
    ctx: &DriverCtx,
    client: &mut Client,
    tracked: &mut HashMap<DeviceId, Tracked>,
) -> Result<(), Failure> {
    let answer = client.get(&Store(ctx), "/clients_api/ring_devices").await?;
    let found = model::devices(&answer);
    for d in &found {
        let id = ctx.device_id(&d.id.to_string());
        let new = !tracked.contains_key(&id);
        if new {
            tracing::info!(
                instance = %ctx.instance(),
                name = %d.name,
                model = %d.model,
                online = d.online,
                "ring device"
            );
            ctx.upsert_device(model::device(ctx, &id, d));
            // Nothing happens at the door until Ring says so.
            for key in ["doorbell", "motion"] {
                ctx.set_state(&id, key, Value::Bool(false));
            }
        }
        ctx.set_availability(&id, d.online);
        if let Some(b) = d.battery {
            ctx.set_state(&id, "battery", Value::Float(b));
        }
        if let Some(v) = d.volume {
            ctx.set_state(&id, "volume", Value::Float(v));
        }
        let entry = tracked.entry(id).or_insert_with(|| Tracked {
            device: d.clone(),
            watch: Watch::default(),
            history: false,
            unreadable: false,
        });
        if entry.device.name != d.name {
            ctx.upsert_device(model::device(ctx, &ctx.device_id(&d.id.to_string()), d));
        }
        entry.device = d.clone();
    }
    // Gone from the account.
    tracked.retain(|id, t| {
        let kept = found.iter().any(|d| d.id == t.device.id);
        if !kept {
            ctx.remove_device(id);
        }
        kept
    });
    Ok(())
}

/// Wi-Fi and battery of each device; one that does not answer is skipped.
async fn health(
    ctx: &DriverCtx,
    client: &mut Client,
    tracked: &HashMap<DeviceId, Tracked>,
) -> Result<(), Failure> {
    for (id, t) in tracked {
        match client.get(&Store(ctx), &t.device.health_path()).await {
            Ok(answer) => {
                let (signal, battery) = model::health(&answer);
                if let Some(s) = signal {
                    ctx.set_state(id, "wifi_signal", Value::Int(s));
                }
                if let (Some(b), false) = (battery, t.device.family == Family::Chime) {
                    ctx.set_state(id, "battery", Value::Float(b));
                }
            }
            Err(Failure::Other(e)) => {
                tracing::debug!(device = %t.device.name, error = %format!("{e:#}"), "ring health");
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// The latest events of each doorbell and camera; the new ones told.
async fn events(
    ctx: &DriverCtx,
    client: &mut Client,
    tracked: &mut HashMap<DeviceId, Tracked>,
    resets: &mut HashMap<(DeviceId, &'static str), Instant>,
) -> Result<(), Failure> {
    for (id, t) in tracked.iter_mut() {
        if t.device.family == Family::Chime || !t.device.online {
            continue;
        }
        let path = if t.history {
            t.device.history_path()
        } else {
            t.device.events_path()
        };
        let answer = match client.get(&Store(ctx), &path).await {
            Ok(answer) => answer,
            Err(Failure::Other(e)) => {
                let e = format!("{e:#}");
                if !t.history && e.contains(": HTTP 404") {
                    t.history = true;
                } else if !t.unreadable {
                    t.unreadable = true;
                    tracing::warn!(device = %t.device.name, error = %e, "ring events");
                }
                continue;
            }
            Err(e) => return Err(e),
        };
        t.unreadable = false;
        let now = jiff::Timestamp::now().as_millisecond();
        for event in t.watch.news(&model::events(&answer), now) {
            let (key, hold) = match event.what {
                Happened::Press if t.device.family == Family::Doorbell => ("doorbell", PRESS),
                Happened::Press => continue,
                Happened::Motion => ("motion", MOTION),
            };
            tracing::info!(device = %t.device.name, what = key, "ring event");
            ctx.pulse_state(id, key, Value::Bool(true));
            resets.insert((id.clone(), key), Instant::now() + hold);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_poll_has_a_floor_and_nothing_else_is_accepted() {
        let parsed: Config = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(parsed.poll(), Duration::from_secs(10));
        let fast: Config = serde_json::from_value(serde_json::json!({ "poll": 1 })).unwrap();
        assert_eq!(fast.poll(), Duration::from_secs(5));
        assert!(
            serde_json::from_value::<Config>(serde_json::json!({ "password": "x" })).is_err(),
            "a password never goes in the configuration"
        );
    }
}
