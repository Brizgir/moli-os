//! Ring doorbells, cameras and chimes, through Ring's own cloud API
//! (unofficial: Ring publishes nothing, and its devices answer nothing on
//! the home network): a press at the door, motion, battery, Wi-Fi, a
//! chime's volume, the last image of each camera when the dashboard shows
//! it ([`images`]), and the location's mode (disarmed, home, away: the
//! app's « Modes »), read every half minute and set from the dashboard by
//! a person. The account is connected once from the dashboard: the person
//! pastes a refresh token made on their own computer (Ring itself takes
//! the e-mail, the password and the one-time code there: none of them
//! ever reaches Moli); Moli keeps only the refresh token Ring gives, in
//! the encrypted secret store, and writes it again each time Ring
//! replaces it.
//!
//! Presses and motion come by Ring's push notifications as soon as Ring
//! sends them ([`push`]: Moli receives them as the Ring app does), and are
//! read from the devices' latest events every few seconds too, as a safety
//! net (slower while the push works). Either way an event is told once
//! (same id), only if it is less than [`model::FRESH`] old, and never from
//! before the start (nor from the first reading after it), so an old press
//! never reads as someone at the door.

mod api;
mod auth;
mod ece;
mod fcm;
mod images;
mod mcs;
mod model;
mod net;
mod push;
mod signin;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use moli_core::{Access, Device, DeviceId, Kind, PointSpec, Semantic, Value};
use moli_runtime::{BoxFuture, CommandRequest, Driver, DriverCtx, SecretKeeper};
use serde::Deserialize;
use serde_json::Value as Json;
use tokio::sync::{Mutex, mpsc};
use tokio::time::{Instant, MissedTickBehavior};

use crate::api::{Client, Failure, Vault};
use crate::fcm::Credentials;
use crate::images::{Images, Shared, Taken};
use crate::mcs::{Dial, Google};
use crate::model::{Event, Family, Happened, ModeReading, Pushed, RingDevice, Watch};
use crate::net::{Transport, Web};
use crate::push::{Ask, Heard, Keep, Push};
use crate::signin::Refusal;

const REFRESH: &str = "refresh_token";
const HARDWARE_ID: &str = "hardware_id";
/// Moli's Firebase device and keys ([`fcm::Credentials`]).
const PUSH: &str = "push_credentials";
/// The poll while the push works: a safety net, well within
/// [`model::FRESH`] for an event the push would have missed.
const PUSH_POLL: Duration = Duration::from_secs(30);
/// Events the poll told while the push said it was up, before the push
/// registers again.
const MISSED: u32 = 3;
/// After the poll told an event the push did not bring (a camera whose
/// alerts are off in the Ring app, a push gone quiet), the poll keeps its
/// own pace this long, even while the push says it is up.
const WARY: Duration = Duration::from_secs(15 * 60);
/// The account's devices (one added, renamed, offline…).
const DEVICES: Duration = Duration::from_secs(5 * 60);
/// Wi-Fi and battery.
const HEALTH: Duration = Duration::from_secs(15 * 60);
/// A press shows this long on `doorbell`.
const PRESS: Duration = Duration::from_secs(10);
/// Motion stays on this long after the last one Ring saw.
const MOTION: Duration = Duration::from_secs(60);
/// The location's mode, as the app may have changed it.
const MODES: Duration = Duration::from_secs(30);
const RETRY_MIN: Duration = Duration::from_secs(30);
const RETRY_MAX: Duration = Duration::from_secs(15 * 60);
/// Ring asked to slow down: this long before trying again.
const SLOW_DOWN: Duration = Duration::from_secs(10 * 60);
const SIGN_IN_AGAIN: &str = "Ring no longer accepts this sign-in";
const TOO_MANY: &str = "Ring asks to slow down";

fn default_poll() -> u64 {
    10
}

fn yes() -> bool {
    true
}

fn default_siren() -> u64 {
    30
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Seconds between two readings of the latest events (5 at least);
    /// while the push works, 30 at least.
    #[serde(default = "default_poll")]
    pub poll: u64,
    /// Ring's push notifications (instant alerts); `false` for a network
    /// that blocks Google's push server (TCP 5228).
    #[serde(default = "yes")]
    pub push: bool,
    /// How long a camera's siren sounds when switched on (1 to 120 s).
    #[serde(default = "default_siren")]
    pub siren_seconds: u64,
}

impl Config {
    fn poll(&self) -> Duration {
        Duration::from_secs(self.poll.max(5))
    }

    fn siren(&self) -> u64 {
        self.siren_seconds.clamp(1, 120)
    }

    /// The poll's pace, with the push working or not; `wary` when the push
    /// lately missed an event the poll found.
    fn pace(&self, push_up: bool, wary: bool) -> Duration {
        if push_up && !wary {
            self.poll().max(PUSH_POLL)
        } else {
            self.poll()
        }
    }
}

pub struct Ring {
    config: Config,
    net: Arc<dyn Transport>,
    dial: Arc<dyn Dial>,
}

impl std::fmt::Debug for Ring {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ring")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl Ring {
    #[must_use]
    pub fn new(config: Config) -> Self {
        Self {
            config,
            net: Arc::new(Web),
            dial: Arc::new(Google),
        }
    }

    #[cfg(test)]
    fn with_transport(config: Config, net: Arc<dyn Transport>, dial: Arc<dyn Dial>) -> Self {
        Self { config, net, dial }
    }
}

impl Driver for Ring {
    fn kind(&self) -> &'static str {
        "ring"
    }

    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(run(
            &self.config,
            Arc::clone(&self.net),
            Arc::clone(&self.dial),
            ctx,
        ))
    }
}

/// The new refresh token, kept in the instance's encrypted store (from the
/// driver's loop or from a camera's image, asked by a surface).
struct Store(SecretKeeper);

impl Vault for Store {
    fn keep<'a>(&'a self, refresh: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move { Ok(self.0.store(REFRESH, refresh).await?) })
    }
}

impl Keep for Store {
    fn keep<'a>(&'a self, credentials: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move { Ok(self.0.store(PUSH, credentials).await?) })
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

/// The account: its state, and what the dashboard pastes to sign it in.
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
            // `active`, `fallback` (the poll only) or `off` (option).
            text_point(
                "push",
                &moli_i18n::tr!("pilotes.ring.alertes"),
                false,
                Semantic::Config,
            ),
            // What a person pastes: never journaled nor kept (`Secret`).
            text_point(
                "sign_in",
                &moli_i18n::tr!("pilotes.ring.jeton"),
                true,
                Semantic::Secret,
            ),
        ],
    }
}

fn set_text(ctx: &DriverCtx, id: &DeviceId, key: &str, text: &str) {
    ctx.set_state(id, key, Value::Text(text.into()));
}

async fn run(
    config: &Config,
    net: Arc<dyn Transport>,
    dial: Arc<dyn Dial>,
    ctx: &mut DriverCtx,
) -> anyhow::Result<()> {
    let vault: Arc<dyn Vault> = Arc::new(Store(ctx.secret_keeper()));
    let account = ctx.device_id("account");
    ctx.upsert_device(account_device(ctx, &account));
    ctx.set_availability(&account, true);
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
        let links = Links {
            net: Arc::clone(&net),
            dial: Arc::clone(&dial),
        };
        let failure = match connected(config, ctx, &account, &vault, client, &links).await {
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
        Refusal::Pasted => moli_i18n::tr!("pilotes.ring.jeton_faux"),
        Refusal::Refused => moli_i18n::tr!("pilotes.ring.jeton_refuse"),
        Refusal::TooMany => moli_i18n::tr!("pilotes.ring.trop"),
        Refusal::Other(why) => moli_i18n::tr!("pilotes.ring.refuse", why = why),
    }
}

/// Waits for the person to paste a token from the dashboard; the client,
/// signed in and kept. `None`: stopped.
async fn sign_in(
    ctx: &mut DriverCtx,
    account: &DeviceId,
    net: &Arc<dyn Transport>,
) -> anyhow::Result<Option<Client>> {
    set_text(ctx, account, "status", "sign_in");
    ctx.wait_for(moli_i18n::tr!("pilotes.ring.a_connecter"));
    loop {
        // `None` once Moli stops.
        let Some(command) = ctx.next_command().await else {
            return Ok(None);
        };
        if command.device.id != *account || &*command.key != "sign_in" {
            command.reply(Err(moli_i18n::tr!("pilotes.ring.pas_connecte")));
            continue;
        }
        let Value::Text(pasted) = &command.value else {
            command.reply(Err(moli_i18n::tr!("pilotes.ring.jeton_faux")));
            continue;
        };
        // A pasted token is spent by its first use: not without a store.
        if let Some(why) = ctx.secrets_problem() {
            command.reply(Err(moli_i18n::tr!("pilotes.ring.coffre", why = why)));
            continue;
        }
        let ours = ctx.secret(HARDWARE_ID);
        let connected = signin::connect(&**net, pasted, ours.as_deref()).await;
        match connected {
            Ok(c) => {
                let kept = async {
                    if ours.as_deref() != Some(c.hardware_id.as_str()) {
                        ctx.store_secret(HARDWARE_ID, &c.hardware_id).await?;
                    }
                    ctx.store_secret(REFRESH, &c.tokens.refresh).await
                };
                if let Err(e) = kept.await {
                    command.reply(Err(moli_i18n::tr!("pilotes.ring.coffre", why = e)));
                    return Err(e.into());
                }
                command.reply(Ok(()));
                tracing::info!(instance = %ctx.instance(), "ring account connected");
                ctx.ready();
                return Ok(Some(Client::signed_in(
                    Arc::clone(net),
                    &c.hardware_id,
                    &c.tokens,
                )));
            }
            Err(r) => {
                if let Refusal::Other(why) = &r {
                    tracing::warn!(instance = %ctx.instance(), error = %why, "ring sign-in refused");
                }
                command.reply(Err(refusal(&r)));
            }
        }
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

/// One location of the account (« Modes » in the Ring app): the device
/// that shows its mode.
struct Located {
    device: DeviceId,
    reading: Option<ModeReading>,
}

/// Everything the loop keeps: the client, shared with the cameras'
/// images, the devices and the locations.
struct Session {
    client: Shared,
    vault: Arc<dyn Vault>,
    tracked: HashMap<DeviceId, Tracked>,
    /// By Ring's location id.
    locations: HashMap<String, Located>,
    /// Where a camera tells when its new image was taken.
    taken: Taken,
    /// When Moli began to watch (ms): nothing from before is told.
    since: i64,
}

/// How Ring and Google's push server are reached.
struct Links {
    net: Arc<dyn Transport>,
    dial: Arc<dyn Dial>,
}

/// The push task, stopped with the connection to Ring.
struct Aborting(tokio::task::JoinHandle<()>);

impl Drop for Aborting {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// The push task's side of the loop.
struct Pushing {
    heard: mpsc::UnboundedReceiver<Heard>,
    asks: mpsc::UnboundedSender<Ask>,
    _task: Aborting,
}

/// Connected: the devices, then their events, health and modes, until
/// stopped.
async fn connected(
    config: &Config,
    ctx: &mut DriverCtx,
    account: &DeviceId,
    vault: &Arc<dyn Vault>,
    mut client: Client,
    links: &Links,
) -> anyhow::Result<()> {
    let fail = |e: Failure| match e {
        Failure::SignInAgain => anyhow::anyhow!("{SIGN_IN_AGAIN}"),
        Failure::TooMany => anyhow::anyhow!("{TOO_MANY}"),
        Failure::Other(e) => e,
    };
    match client.session(&**vault).await {
        Ok(()) => {}
        Err(Failure::Other(e)) => {
            tracing::debug!(instance = %ctx.instance(), error = %format!("{e:#}"), "ring session");
        }
        Err(e) => return Err(fail(e)),
    }
    let (taken, images_taken) = mpsc::unbounded_channel();
    let mut session = Session {
        client: Arc::new(Mutex::new(client)),
        vault: Arc::clone(vault),
        tracked: HashMap::new(),
        locations: HashMap::new(),
        taken,
        since: jiff::Timestamp::now().as_millisecond(),
    };
    discover(ctx, &mut session).await.map_err(fail)?;
    health(ctx, &session).await.map_err(fail)?;
    set_text(ctx, account, "status", "connected");
    ctx.ready();
    tracing::info!(
        instance = %ctx.instance(),
        devices = session.tracked.len(),
        locations = session.locations.len(),
        "ring connected"
    );
    let pushing = config.push.then(|| start_push(ctx, &session, links));
    set_text(
        ctx,
        account,
        "push",
        if config.push { "fallback" } else { "off" },
    );
    let result = watch(config, ctx, account, &mut session, images_taken, pushing).await;
    if result.is_err() {
        lost(ctx, &session);
        if config.push {
            set_text(ctx, account, "push", "fallback");
        }
    }
    result.map_err(fail)
}

/// The push task, beside the loop, with the same client to Ring.
fn start_push(ctx: &DriverCtx, session: &Session, links: &Links) -> Pushing {
    let (heard, heard_rx) = mpsc::unbounded_channel();
    let (asks, asks_rx) = mpsc::unbounded_channel();
    let push = Push {
        net: Arc::clone(&links.net),
        dial: Arc::clone(&links.dial),
        client: Arc::clone(&session.client),
        vault: Arc::clone(&session.vault),
        keep: Arc::new(Store(ctx.secret_keeper())),
        credentials: ctx.secret(PUSH).as_deref().and_then(Credentials::parse),
        heard,
        asks: asks_rx,
    };
    Pushing {
        heard: heard_rx,
        asks,
        _task: Aborting(tokio::spawn(push.run())),
    }
}

/// Ring out of reach: nothing is known of the door any more. No press, no
/// motion stays shown, and each device is out of reach until Ring says
/// otherwise (never a stale « someone at the door »).
fn lost(ctx: &DriverCtx, session: &Session) {
    for (id, t) in &session.tracked {
        if t.device.family != Family::Chime {
            for key in ["doorbell", "motion"] {
                ctx.set_state(id, key, Value::Bool(false));
            }
        }
        ctx.set_availability(id, false);
    }
    for l in session.locations.values() {
        ctx.set_availability(&l.device, false);
    }
}

/// The events, devices, health and modes, read until stopped or Ring
/// fails.
async fn watch(
    config: &Config,
    ctx: &mut DriverCtx,
    account: &DeviceId,
    session: &mut Session,
    mut taken: mpsc::UnboundedReceiver<(DeviceId, i64)>,
    mut pushing: Option<Pushing>,
) -> Result<(), Failure> {
    let mut poll = tokio::time::interval(config.pace(false, false));
    let mut devices = tokio::time::interval(DEVICES);
    let mut healths = tokio::time::interval(HEALTH);
    let mut modes = tokio::time::interval(MODES);
    for timer in [&mut poll, &mut devices, &mut healths, &mut modes] {
        timer.set_missed_tick_behavior(MissedTickBehavior::Delay);
        timer.reset();
    }
    // Points to put back to « off », and when.
    let mut resets: HashMap<(DeviceId, &'static str), Instant> = HashMap::new();
    poll.reset_immediately();
    let mut push_up = false;
    // Events the poll told while the push was up.
    let mut missed = 0;
    // Until when the poll keeps its own pace (the push missed an event).
    let mut wary: Option<Instant> = None;
    loop {
        let next_reset = resets.values().min().copied();
        tokio::select! {
            Some(heard) = async {
                match pushing.as_mut() {
                    Some(p) => p.heard.recv().await,
                    None => std::future::pending().await,
                }
            } => match heard {
                Heard::Up | Heard::Down => {
                    push_up = heard == Heard::Up;
                    missed = 0;
                    set_text(ctx, account, "push", if push_up { "active" } else { "fallback" });
                    tracing::info!(instance = %ctx.instance(), active = push_up, "ring instant alerts");
                    // The new pace from now on; at once when the push fell.
                    let pace = config.pace(push_up, wary.is_some());
                    let first = if push_up { Instant::now() + pace } else { Instant::now() };
                    poll = tokio::time::interval_at(first, pace);
                    poll.set_missed_tick_behavior(MissedTickBehavior::Delay);
                }
                Heard::Notified(p) => {
                    if notified(ctx, session, &mut resets, &p) {
                        missed = 0;
                    }
                }
            },
            command = ctx.next_command() => {
                let Some(command) = command else { return Ok(()) };
                command_received(config, ctx, account, session, command).await?;
            }
            Some((id, at)) = taken.recv() => {
                set_text(ctx, &id, "image_at", &model::rfc3339(at));
            }
            _ = poll.tick() => {
                let told = events(ctx, session, &mut resets).await?;
                let now = Instant::now();
                if push_up && told > 0 {
                    // The push did not bring these: read at our own pace a
                    // while, a motion is then seen in `poll` s, not 30.
                    if wary.is_none() {
                        tracing::info!(instance = %ctx.instance(), "ring push missed an event: polling faster");
                        poll = tokio::time::interval_at(now + config.poll(), config.poll());
                        poll.set_missed_tick_behavior(MissedTickBehavior::Delay);
                    }
                    wary = Some(now + WARY);
                    missed += told;
                    if missed >= MISSED {
                        missed = 0;
                        if let Some(p) = &pushing {
                            let _ = p.asks.send(Ask::Renew);
                        }
                    }
                } else if wary.is_some_and(|until| until <= now) {
                    wary = None;
                    let pace = config.pace(push_up, false);
                    poll = tokio::time::interval_at(now + pace, pace);
                    poll.set_missed_tick_behavior(MissedTickBehavior::Delay);
                }
            }
            _ = devices.tick() => discover(ctx, session).await?,
            _ = healths.tick() => health(ctx, session).await?,
            _ = modes.tick() => read_modes(ctx, session).await?,
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

/// An order: the location's mode, set as the app does, then read again
/// (the only thing Moli writes to Ring). Who may ask is the hub's guard's
/// business (`Semantic::Control`): here, every order carries a person's
/// intent.
async fn command_received(
    config: &Config,
    ctx: &DriverCtx,
    account: &DeviceId,
    session: &mut Session,
    command: CommandRequest,
) -> Result<(), Failure> {
    if command.device.id == *account {
        command.reply(Err(moli_i18n::tr!("pilotes.ring.deja_connecte")));
        return Ok(());
    }
    if let Some(tracked) = session.tracked.get(&command.device.id) {
        let id = tracked.device.id;
        return switch_received(config, ctx, session, id, command).await;
    }
    let location = session
        .locations
        .iter_mut()
        .find(|(_, l)| l.device == command.device.id)
        .map(|(id, l)| (id.clone(), l));
    let Some((location, located)) = location else {
        command.reply(Err(moli_i18n::tr!("pilotes.ring.lecture_seule")));
        return Ok(());
    };
    let mode = match (&*command.key, &command.value) {
        ("mode", Value::Text(mode)) if model::MODES.contains(&mode.as_ref()) => mode.to_string(),
        _ => {
            command.reply(Err(moli_i18n::tr!("pilotes.ring.mode_inconnu")));
            return Ok(());
        }
    };
    if located.reading.as_ref().is_some_and(|r| r.read_only) {
        command.reply(Err(moli_i18n::tr!("pilotes.ring.mode_alarme")));
        return Ok(());
    }
    let path = model::mode_path(&location);
    let body = serde_json::json!({ "mode": mode });
    let set = session
        .client
        .lock()
        .await
        .app_post(&*session.vault, &path, &body)
        .await;
    match set {
        Ok(_) => {}
        Err(Failure::Other(e)) => {
            tracing::warn!(error = %format!("{e:#}"), "ring mode refused");
            command.reply(Err(moli_i18n::tr!("pilotes.ring.mode_refuse")));
            return Ok(());
        }
        Err(e) => {
            command.reply(Err(e.to_string()));
            return Err(e);
        }
    }
    // Read again: what Ring now says is what the house shows.
    let read = read_mode(&session.client, &session.vault, &location).await;
    match read {
        Ok(Some(reading)) => {
            let took = reading.mode.as_deref() == Some(mode.as_str());
            show_mode(ctx, &located.device, &reading);
            located.reading = Some(reading);
            tracing::info!(mode = %mode, took, "ring mode set");
            command.reply(if took {
                Ok(())
            } else {
                Err(moli_i18n::tr!("pilotes.ring.mode_pas_pris"))
            });
            Ok(())
        }
        Ok(None) => {
            command.reply(Err(moli_i18n::tr!("pilotes.ring.mode_pas_pris")));
            Ok(())
        }
        Err(e) => {
            command.reply(Err(e.to_string()));
            Err(e)
        }
    }
}

/// A camera's switch: its light, its siren, its motion detection. The new
/// value shows at once; the device list read next confirms it.
async fn switch_received(
    config: &Config,
    ctx: &DriverCtx,
    session: &mut Session,
    id: u64,
    command: CommandRequest,
) -> Result<(), Failure> {
    let Value::Bool(on) = command.value else {
        command.reply(Err(moli_i18n::tr!("pilotes.ring.lecture_seule")));
        return Ok(());
    };
    let switch = if on { "on" } else { "off" };
    let (path, body, setting) = match &*command.key {
        "light" => (
            format!("/clients_api/doorbots/{id}/floodlight_light_{switch}"),
            Json::Null,
            false,
        ),
        "siren" if on => (
            format!(
                "/clients_api/doorbots/{id}/siren_on?duration={}",
                config.siren()
            ),
            Json::Null,
            false,
        ),
        "siren" => (
            format!("/clients_api/doorbots/{id}/siren_off"),
            Json::Null,
            false,
        ),
        "motion_detection" => (
            format!("/devices/v1/devices/{id}/settings"),
            serde_json::json!({ "motion_settings": { "motion_detection_enabled": on } }),
            true,
        ),
        _ => {
            command.reply(Err(moli_i18n::tr!("pilotes.ring.lecture_seule")));
            return Ok(());
        }
    };
    if command.abandoned() {
        command.reply(Err("expired".into()));
        return Ok(());
    }
    let mut client = session.client.lock().await;
    let done = if setting {
        client.patch(&*session.vault, &path, &body).await
    } else {
        // Ring wants a JSON body, `null` at least, on every PUT.
        client.put(&*session.vault, &path, &body).await
    };
    drop(client);
    match done {
        Ok(_) => {
            ctx.set_state(&command.device.id, &command.key, command.value.clone());
            tracing::info!(key = %command.key, on, "ring switch set");
            command.reply(Ok(()));
            Ok(())
        }
        Err(Failure::Other(e)) => {
            tracing::warn!(error = %format!("{e:#}"), key = %command.key, "ring switch refused");
            command.reply(Err(moli_i18n::tr!("pilotes.ring.ordre_refuse")));
            Ok(())
        }
        Err(e) => {
            command.reply(Err(e.to_string()));
            Err(e)
        }
    }
}

/// One location's mode; `None` when Ring will not say (no modes there).
async fn read_mode(
    client: &Shared,
    vault: &Arc<dyn Vault>,
    location: &str,
) -> Result<Option<ModeReading>, Failure> {
    let answer = client
        .lock()
        .await
        .app_get(&**vault, &model::mode_path(location))
        .await;
    match answer {
        Ok(answer) => Ok(model::mode(&answer)),
        Err(Failure::Other(e)) => {
            tracing::debug!(error = %format!("{e:#}"), "ring mode");
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

fn show_mode(ctx: &DriverCtx, device: &DeviceId, reading: &ModeReading) {
    if let Some(mode) = &reading.mode {
        ctx.set_state(device, "mode", Value::Text(mode.clone().into()));
    }
    if let Some(since) = reading.since {
        set_text(ctx, device, "mode_since", &model::rfc3339(since));
    }
    ctx.set_availability(device, reading.mode.is_some());
}

/// Every location's mode, as the app may have changed it.
async fn read_modes(ctx: &DriverCtx, session: &mut Session) -> Result<(), Failure> {
    for (location, located) in &mut session.locations {
        if let Some(reading) = read_mode(&session.client, &session.vault, location).await? {
            if located.reading.as_ref().map(|r| &r.mode) != Some(&reading.mode) {
                tracing::info!(mode = ?reading.mode, "ring mode");
            }
            show_mode(ctx, &located.device, &reading);
            located.reading = Some(reading);
        }
    }
    Ok(())
}

/// A camera's switches, as the device list says them.
fn show_switches(ctx: &DriverCtx, id: &DeviceId, d: &RingDevice) {
    let switches = [
        ("light", d.light),
        ("siren", d.siren),
        ("motion_detection", d.motion_detection),
    ];
    for (key, on) in switches {
        if let Some(on) = on {
            ctx.set_state(id, key, Value::Bool(on));
        }
    }
}

/// The account's devices, shown in the house; each doorbell and camera
/// gives its images; each location, its mode.
async fn discover(ctx: &DriverCtx, session: &mut Session) -> Result<(), Failure> {
    let answer = session
        .client
        .lock()
        .await
        .get(&*session.vault, "/clients_api/ring_devices")
        .await?;
    let found = model::devices(&answer);
    for d in &found {
        let id = ctx.device_id(&d.id.to_string());
        let new = !session.tracked.contains_key(&id);
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
            if d.family != Family::Chime {
                ctx.provide_snapshots(
                    &id,
                    Arc::new(Images::new(
                        Arc::clone(&session.client),
                        Arc::clone(&session.vault),
                        d,
                        id.clone(),
                        session.taken.clone(),
                    )),
                );
            }
        }
        ctx.set_availability(&id, d.online);
        if let Some(b) = d.battery {
            ctx.set_state(&id, "battery", Value::Float(b));
        }
        if let Some(v) = d.volume {
            ctx.set_state(&id, "volume", Value::Float(v));
        }
        show_switches(ctx, &id, d);
        let entry = session.tracked.entry(id).or_insert_with(|| Tracked {
            device: d.clone(),
            watch: Watch::new(session.since),
            history: false,
            unreadable: false,
        });
        if entry.device.name != d.name {
            ctx.upsert_device(model::device(ctx, &ctx.device_id(&d.id.to_string()), d));
        }
        entry.device = d.clone();
    }
    // Gone from the account.
    session.tracked.retain(|id, t| {
        let kept = found.iter().any(|d| d.id == t.device.id);
        if !kept {
            ctx.remove_device(id);
        }
        kept
    });
    // The locations: a mode each, when Ring has modes there.
    let mut locations: Vec<&str> = found
        .iter()
        .map(|d| d.location.as_str())
        .filter(|l| !l.is_empty())
        .collect();
    locations.sort_unstable();
    locations.dedup();
    for location in &locations {
        if session.locations.contains_key(*location) {
            continue;
        }
        let Some(reading) = read_mode(&session.client, &session.vault, location).await? else {
            continue;
        };
        let device = ctx.device_id(&format!("location-{location}"));
        tracing::info!(instance = %ctx.instance(), mode = ?reading.mode, "ring location");
        ctx.upsert_device(model::location_device(ctx, &device));
        show_mode(ctx, &device, &reading);
        session.locations.insert(
            (*location).to_owned(),
            Located {
                device,
                reading: Some(reading),
            },
        );
    }
    session.locations.retain(|location, l| {
        let kept = locations.contains(&location.as_str());
        if !kept {
            ctx.remove_device(&l.device);
        }
        kept
    });
    Ok(())
}

/// Wi-Fi and battery of each device; one that does not answer is skipped.
async fn health(ctx: &DriverCtx, session: &Session) -> Result<(), Failure> {
    for (id, t) in &session.tracked {
        let answer = session
            .client
            .lock()
            .await
            .get(&*session.vault, &t.device.health_path())
            .await;
        match answer {
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

/// Shows one event on its device's point (a press on a doorbell only),
/// for as long as it lasts; whether it was shown.
fn tell(
    ctx: &DriverCtx,
    id: &DeviceId,
    t: &Tracked,
    event: &Event,
    by: &str,
    resets: &mut HashMap<(DeviceId, &'static str), Instant>,
) -> bool {
    let (key, hold) = match event.what {
        Happened::Press if t.device.family == Family::Doorbell => ("doorbell", PRESS),
        Happened::Press => return false,
        Happened::Motion => ("motion", MOTION),
    };
    // How long after Ring saw it: what the push is for.
    let delay_ms = jiff::Timestamp::now().as_millisecond() - event.at;
    tracing::info!(device = %t.device.name, what = key, by, delay_ms, "ring event");
    ctx.pulse_state(id, key, Value::Bool(true));
    resets.insert((id.clone(), key), Instant::now() + hold);
    true
}

/// An event Ring pushed, told if new (not read nor pushed before), fresh,
/// and of a device Moli tracks.
fn notified(
    ctx: &DriverCtx,
    session: &mut Session,
    resets: &mut HashMap<(DeviceId, &'static str), Instant>,
    p: &Pushed,
) -> bool {
    let Some((id, t)) = session
        .tracked
        .iter_mut()
        .find(|(_, t)| t.device.id == p.device)
    else {
        tracing::debug!("ring push for a device Moli does not track");
        return false;
    };
    let now = jiff::Timestamp::now().as_millisecond();
    t.watch.heard(&p.event, now) && tell(ctx, id, t, &p.event, "push", resets)
}

/// The latest events of each doorbell and camera; the new ones told (how
/// many).
async fn events(
    ctx: &DriverCtx,
    session: &mut Session,
    resets: &mut HashMap<(DeviceId, &'static str), Instant>,
) -> Result<u32, Failure> {
    let mut told = 0;
    for (id, t) in &mut session.tracked {
        if t.device.family == Family::Chime || !t.device.online {
            continue;
        }
        let path = if t.history {
            t.device.history_path()
        } else {
            t.device.events_path()
        };
        let answer = session
            .client
            .lock()
            .await
            .get(&*session.vault, &path)
            .await;
        let answer = match answer {
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
            if tell(ctx, id, t, &event, "poll", resets) {
                told += 1;
            }
        }
    }
    Ok(told)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use moli_core::{InstanceId, Origin};
    use moli_runtime::{CommandError, Hub, HubOptions, MasterKey, spawn_driver};
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::net::replay::Replay;

    const DEVICES: &str = include_str!("../../../integrations/ring/fixtures/ring-devices.json");
    const HEALTH: &str = include_str!("../../../integrations/ring/fixtures/doorbot-health.json");
    const EVENTS: &str = include_str!("../../../integrations/ring/fixtures/events.json");
    const MODE: &str = include_str!("../../../integrations/ring/fixtures/mode-location.json");
    const LOCATION: &str = "00000000-0000-4000-8000-000000000001";
    const JPEG: [u8; 6] = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];

    fn token() -> String {
        serde_json::json!({
            "access_token": "fake-access-1",
            "refresh_token": "fake-refresh-1",
            "expires_in": 3600,
            "scope": "client",
            "token_type": "Bearer",
        })
        .to_string()
    }

    fn stamps(at: i64) -> String {
        serde_json::json!({ "timestamps": [{ "doorbot_id": 10_000_001, "timestamp": at }] })
            .to_string()
    }

    fn mode_answer(mode: &str, since: i64) -> String {
        serde_json::json!({ "mode": mode, "lastUpdateTimeMS": since, "readOnly": false })
            .to_string()
    }

    /// The account connected (its secrets in a store of its own), on a
    /// transport that answers whatever the loop reads, in any order.
    fn replay() -> Arc<Replay> {
        let net = Replay::default()
            .by_path()
            .then("/oauth/token", 200, &token());
        net.always("/clients_api/session", 200, "{}");
        net.always("/clients_api/ring_devices", 200, DEVICES);
        net.always("/clients_api/doorbots/10000001/health", 200, HEALTH);
        net.always("/clients_api/chimes/10000002/health", 404, "");
        net.always("/clients_api/locations/", 200, EVENTS);
        net.always("GET /api/v1/mode/location/", 200, MODE);
        Arc::new(net)
    }

    /// No push server in this test.
    struct NoDial;

    impl Dial for NoDial {
        fn dial(&self) -> BoxFuture<'_, anyhow::Result<Box<dyn mcs::Stream>>> {
            Box::pin(async { anyhow::bail!("no push server here") })
        }
    }

    /// A push server on the other end of a pipe, reached once.
    struct Pipe(std::sync::Mutex<Option<tokio::io::DuplexStream>>);

    impl Dial for Pipe {
        fn dial(&self) -> BoxFuture<'_, anyhow::Result<Box<dyn mcs::Stream>>> {
            let stream = self.0.lock().unwrap().take();
            Box::pin(async move {
                match stream {
                    Some(s) => Ok(Box::new(s) as Box<dyn mcs::Stream>),
                    None => anyhow::bail!("reached once"),
                }
            })
        }
    }

    async fn start(name: &str, net: Arc<Replay>) -> (Hub, InstanceId, CancellationToken, PathBuf) {
        let config = serde_json::json!({ "push": false });
        start_with(name, net, config, Arc::new(NoDial), None).await
    }

    async fn start_with(
        name: &str,
        net: Arc<Replay>,
        config: serde_json::Value,
        dial: Arc<dyn Dial>,
        push: Option<&Credentials>,
    ) -> (Hub, InstanceId, CancellationToken, PathBuf) {
        let vault = std::env::temp_dir().join(format!(
            "moli-ring-secrets-{}-{name}.enc",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&vault);
        let hub = Hub::new(HubOptions {
            secrets: Some((vault.clone(), MasterKey::generate().unwrap())),
            ..HubOptions::default()
        })
        .unwrap();
        let instance = InstanceId::from("ring");
        hub.store_secret(&instance, REFRESH, "fake-refresh-0")
            .await
            .unwrap();
        hub.store_secret(&instance, HARDWARE_ID, "hw-test")
            .await
            .unwrap();
        if let Some(push) = push {
            hub.store_secret(&instance, PUSH, &push.to_text())
                .await
                .unwrap();
        }
        let cancel = CancellationToken::new();
        let config: Config = serde_json::from_value(config).unwrap();
        spawn_driver(
            &hub,
            instance.clone(),
            Arc::new(Ring::with_transport(config, net, dial)),
            cancel.clone(),
        );
        for _ in 0..500 {
            if hub.stats().drivers_running == 1 {
                return (hub, instance, cancel, vault);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("ring never connected: {:?}", hub.snapshot().drivers);
    }

    fn text(hub: &Hub, point: &str) -> Option<String> {
        match hub.state(&point.into())?.value {
            Value::Text(t) => Some(t.to_string()),
            _ => None,
        }
    }

    /// How many calls about snapshots were made so far.
    fn snapshot_calls(net: &Replay) -> usize {
        net.calls()
            .iter()
            .filter(|(_, p)| p.contains("/snapshots/"))
            .count()
    }

    /// Lets the driver's loop show what a camera told it.
    async fn settle() {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    /// The hub shares one image among viewers for a real second: past it.
    fn a_real_second() {
        std::thread::sleep(Duration::from_millis(1050));
    }

    #[tokio::test(start_paused = true)]
    async fn an_image_is_asked_seldom_and_never_told_newer_than_it_is() {
        let net = replay();
        let (hub, _instance, cancel, vault) = start("images", Arc::clone(&net)).await;
        let bell = DeviceId::from("ring:10000001");
        let photo = hub.snapshot();
        let shown = |id: &str| {
            photo
                .devices
                .iter()
                .find(|d| d.device.id == DeviceId::from(id))
                .unwrap()
                .camera
        };
        assert!(shown("ring:10000001"), "the doorbell gives images");
        assert!(!shown("ring:10000002"), "a chime has none");
        assert_eq!(text(&hub, "ring:10000001/image_at"), None);

        // The first image: asked fresh, its time waited for, then read.
        net.always("PUT /clients_api/snapshots/update", 200, "{}");
        net.always("POST /clients_api/snapshots/timestamps", 200, &stamps(2000));
        net.push("GET /clients_api/snapshots/image/10000001", 200, &JPEG);
        let image = hub.camera_image(&bell).await.unwrap();
        assert_eq!(image.bytes, JPEG);
        assert_eq!(image.content_type, "image/jpeg");
        settle().await;
        assert_eq!(
            text(&hub, "ring:10000001/image_at").as_deref(),
            Some("1970-01-01T00:00:02Z")
        );
        assert_eq!(snapshot_calls(&net), 3, "update, one time, the image");
        let (_, _, body) = net.asked(net.calls().len() - 3);
        assert_eq!(
            body,
            serde_json::json!({ "doorbot_ids": [10_000_001], "refresh": true })
        );

        // Half a minute later: the same image, nothing asked of Ring (the
        // doorbell is on battery).
        a_real_second();
        tokio::time::advance(Duration::from_secs(30)).await;
        assert_eq!(hub.camera_image(&bell).await.unwrap().bytes, JPEG);
        assert_eq!(snapshot_calls(&net), 3);

        // A minute later: Ring gives back the same time → the image kept,
        // not read again, its time unchanged.
        a_real_second();
        tokio::time::advance(Duration::from_secs(61)).await;
        assert_eq!(hub.camera_image(&bell).await.unwrap().bytes, JPEG);
        assert_eq!(snapshot_calls(&net), 3 + 1 + 5, "asked, waited five times");
        settle().await;
        assert_eq!(
            text(&hub, "ring:10000001/image_at").as_deref(),
            Some("1970-01-01T00:00:02Z")
        );

        // A newer time but no image there (recording): the image kept.
        a_real_second();
        tokio::time::advance(Duration::from_secs(61)).await;
        net.push(
            "POST /clients_api/snapshots/timestamps",
            200,
            stamps(3000).as_bytes(),
        );
        net.push("GET /clients_api/snapshots/image/10000001", 404, b"");
        assert_eq!(hub.camera_image(&bell).await.unwrap().bytes, JPEG);
        assert_eq!(snapshot_calls(&net), 9 + 3);
        settle().await;
        assert_eq!(
            text(&hub, "ring:10000001/image_at").as_deref(),
            Some("1970-01-01T00:00:02Z")
        );

        // A newer image: shown, with its time.
        a_real_second();
        tokio::time::advance(Duration::from_secs(61)).await;
        let newer = [0xFF, 0xD8, 0xFF, 0xDB, 0x00, 0x43];
        net.push(
            "POST /clients_api/snapshots/timestamps",
            200,
            stamps(4000).as_bytes(),
        );
        net.push("GET /clients_api/snapshots/image/10000001", 200, &newer);
        assert_eq!(hub.camera_image(&bell).await.unwrap().bytes, newer);
        settle().await;
        assert_eq!(
            text(&hub, "ring:10000001/image_at").as_deref(),
            Some("1970-01-01T00:00:04Z")
        );
        cancel.cancel();
        let _ = std::fs::remove_file(&vault);
    }

    #[tokio::test(start_paused = true)]
    async fn the_mode_is_a_person_s_call_and_read_again_after_the_order() {
        let net = replay();
        let (hub, _instance, cancel, vault) = start("modes", Arc::clone(&net)).await;
        let watch = format!("ring:location-{LOCATION}");
        let mode = moli_core::PointId::from(format!("{watch}/mode"));
        let photo = hub.snapshot();
        let device = photo
            .devices
            .iter()
            .find(|d| d.device.id == DeviceId::from(watch.as_str()))
            .expect("the location's watch is a device");
        assert!(!device.camera);
        assert_eq!(
            device
                .device
                .points
                .iter()
                .find(|p| &*p.key == "mode")
                .unwrap()
                .semantic,
            Semantic::Control,
            "a person's call"
        );
        assert_eq!(
            text(&hub, &format!("{watch}/mode")).as_deref(),
            Some("home")
        );
        assert_eq!(
            text(&hub, &format!("{watch}/mode_since")).as_deref(),
            Some("2025-10-18T12:20:00Z")
        );

        // An agent, Moli, the dashboard without the code: a request, no
        // order to Ring.
        for origin in [
            Origin::Assistant,
            Origin::Mcp,
            Origin::Household,
            Origin::Api,
        ] {
            let held = hub
                .command(&mode, Value::from("disarmed"), origin, None)
                .await;
            assert!(
                matches!(held, Err(CommandError::NeedsApproval { .. })),
                "{origin:?}: {held:?}"
            );
        }
        assert!(
            !net.calls()
                .iter()
                .any(|(m, _)| m == "POST" && m.contains("mode")),
            "nothing sent"
        );
        assert_eq!(
            text(&hub, &format!("{watch}/mode")).as_deref(),
            Some("home")
        );

        // A person at the dashboard (the code given): set, then read again.
        net.always(
            "GET /api/v1/mode/location/",
            200,
            &mode_answer("disarmed", 7_000),
        );
        net.push(
            "POST /api/v1/mode/location/",
            200,
            mode_answer("disarmed", 7_000).as_bytes(),
        );
        hub.command(&mode, Value::from("disarmed"), Origin::Ui, None)
            .await
            .unwrap();
        let sent = net
            .calls()
            .iter()
            .position(|(m, p)| m == "POST" && p.starts_with("/api/v1/mode/location/"))
            .expect("the order went to Ring");
        let (path, _, body) = net.asked(sent);
        assert_eq!(path, format!("/api/v1/mode/location/{LOCATION}"));
        assert_eq!(body, serde_json::json!({ "mode": "disarmed" }));
        assert_eq!(net.host(sent), api::APP_HOST);
        assert!(
            matches!(net.calls().get(sent + 1), Some((m, p)) if m == "GET" && p.starts_with("/api/v1/mode/location/")),
            "read again right after"
        );
        assert_eq!(
            text(&hub, &format!("{watch}/mode")).as_deref(),
            Some("disarmed")
        );
        assert_eq!(
            text(&hub, &format!("{watch}/mode_since")).as_deref(),
            Some("1970-01-01T00:00:07Z")
        );

        cancel.cancel();
        let _ = std::fs::remove_file(&vault);
    }

    /// A floodlight camera of the account, its light on.
    const FLOODLIGHT: &str = r#"{ "doorbots": [], "chimes": [], "stickup_cams": [{
        "id": 10000003, "description": "Garage", "kind": "hp_cam_v1",
        "location_id": "00000000-0000-4000-8000-000000000001",
        "alerts": { "connection": "online" }, "battery_life": null,
        "led_status": "on", "siren_status": { "seconds_remaining": 0 },
        "settings": { "motion_detection_enabled": true } }] }"#;

    #[tokio::test(start_paused = true)]
    async fn a_camera_s_light_siren_and_motion_detection_are_switched() {
        let net = replay();
        net.always("/clients_api/ring_devices", 200, FLOODLIGHT);
        net.always("/clients_api/doorbots/10000003/health", 200, HEALTH);
        let (hub, _instance, cancel, vault) = start("switches", Arc::clone(&net)).await;
        let cam = "ring:10000003";
        let point = |key: &str| moli_core::PointId::from(format!("{cam}/{key}"));
        assert_eq!(flag(&hub, &format!("{cam}/light")), Some(true));
        assert_eq!(flag(&hub, &format!("{cam}/siren")), Some(false));
        assert_eq!(flag(&hub, &format!("{cam}/motion_detection")), Some(true));
        let sent = |net: &Replay, method: &str, start: &str| {
            net.calls()
                .iter()
                .position(|(m, p)| m == method && p.starts_with(start))
        };

        // A siren asked by an agent waits for a person: nothing goes to Ring.
        let held = hub
            .command(&point("siren"), Value::Bool(true), Origin::Assistant, None)
            .await;
        assert!(
            matches!(held, Err(CommandError::NeedsApproval { .. })),
            "{held:?}"
        );
        assert!(sent(&net, "PUT", "/clients_api/doorbots/10000003/siren").is_none());

        net.push(
            "PUT /clients_api/doorbots/10000003/floodlight_light_off",
            200,
            b"",
        );
        hub.command(&point("light"), Value::Bool(false), Origin::Ui, None)
            .await
            .unwrap();
        let at = sent(
            &net,
            "PUT",
            "/clients_api/doorbots/10000003/floodlight_light_off",
        )
        .expect("the light order went to Ring");
        assert_eq!(net.asked(at).2, serde_json::Value::Null, "a null body");
        settle().await;
        assert_eq!(
            flag(&hub, &format!("{cam}/light")),
            Some(false),
            "shown at once"
        );

        net.push("PUT /clients_api/doorbots/10000003/siren_on", 200, b"");
        hub.command(&point("siren"), Value::Bool(true), Origin::Ui, None)
            .await
            .unwrap();
        let at = sent(&net, "PUT", "/clients_api/doorbots/10000003/siren_on").unwrap();
        assert_eq!(
            net.asked(at).0,
            "/clients_api/doorbots/10000003/siren_on?duration=30"
        );

        net.push("PATCH /devices/v1/devices/10000003/settings", 200, b"");
        hub.command(
            &point("motion_detection"),
            Value::Bool(false),
            Origin::Ui,
            None,
        )
        .await
        .unwrap();
        let at = sent(&net, "PATCH", "/devices/v1/devices/10000003/settings").unwrap();
        assert_eq!(
            net.asked(at).2,
            serde_json::json!({ "motion_settings": { "motion_detection_enabled": false } })
        );

        cancel.cancel();
        let _ = std::fs::remove_file(&vault);
    }

    #[tokio::test(start_paused = true)]
    async fn an_automation_carries_a_person_s_intent_and_the_app_s_changes_are_seen() {
        let net = replay();
        let (hub, _instance, cancel, vault) = start("app", Arc::clone(&net)).await;
        let watch = format!("ring:location-{LOCATION}");
        let mode = moli_core::PointId::from(format!("{watch}/mode"));
        let shown = |hub: &Hub| text(hub, &format!("{watch}/mode"));

        // An automation a person approved carries their intent.
        net.always(
            "GET /api/v1/mode/location/",
            200,
            &mode_answer("away", 8_000),
        );
        net.push(
            "POST /api/v1/mode/location/",
            200,
            mode_answer("away", 8_000).as_bytes(),
        );
        hub.command(&mode, Value::from("away"), Origin::Automation, None)
            .await
            .unwrap();
        assert_eq!(shown(&hub).as_deref(), Some("away"));

        // Ring says it did not take it: the person is told, the house
        // shows what Ring says.
        net.push(
            "POST /api/v1/mode/location/",
            200,
            mode_answer("away", 8_000).as_bytes(),
        );
        let refused = hub
            .command(&mode, Value::from("disarmed"), Origin::Ui, None)
            .await;
        assert!(refused.is_err(), "{refused:?}");
        assert_eq!(shown(&hub).as_deref(), Some("away"));

        // Changed in the Ring app: seen within the minute.
        net.always(
            "GET /api/v1/mode/location/",
            200,
            &mode_answer("home", 9_000),
        );
        tokio::time::advance(Duration::from_secs(31)).await;
        settle().await;
        assert_eq!(shown(&hub).as_deref(), Some("home"));
        assert_eq!(
            text(&hub, &format!("{watch}/mode_since")).as_deref(),
            Some("1970-01-01T00:00:09Z")
        );
        cancel.cancel();
        let _ = std::fs::remove_file(&vault);
    }

    fn flag(hub: &Hub, point: &str) -> Option<bool> {
        match hub.state(&point.into())?.value {
            Value::Bool(b) => Some(b),
            _ => None,
        }
    }

    /// Waits (in the test's time) until `ok`.
    async fn until(what: &str, ok: impl Fn() -> bool) {
        for _ in 0..200 {
            if ok() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("never: {what}");
    }

    /// A press as Ring pushes it, happened `ago` ms before now.
    fn ring_push(ding: &str, ago: i64) -> Vec<u8> {
        let at = model::rfc3339(jiff::Timestamp::now().as_millisecond() - ago);
        let text = include_str!("../../../integrations/ring/fixtures/push-ding.json")
            .replace("7000000000000000011", ding)
            .replace("2025-10-18T15:06:40Z", &at);
        text.into_bytes()
    }

    #[tokio::test(start_paused = true)]
    async fn a_press_ring_pushes_is_told_at_once_and_once() {
        use tokio::io::AsyncWriteExt as _;
        let net = replay();
        net.always_bytes("/checkin", 200, &fcm::tests::checkin_answer(41, 42));
        net.always("PATCH /clients_api/device", 200, "{}");
        let push = fcm::tests::credentials();
        let (client, mut server) = tokio::io::duplex(64 * 1024);
        let dial = Arc::new(Pipe(std::sync::Mutex::new(Some(client))));
        let (hub, _instance, cancel, vault) = start_with(
            "push",
            Arc::clone(&net),
            serde_json::json!({}),
            dial,
            Some(&push),
        )
        .await;
        let login = mcs::tests::accept(&mut server).await;
        assert_eq!(
            login.str(3),
            Some("4000000000000000001"),
            "the kept device, not a new one"
        );
        until("instant alerts active", || {
            text(&hub, "ring:account/push").as_deref() == Some("active")
        })
        .await;
        // Ring was told the token, on Moli's own client device; nothing
        // was registered again.
        let calls = net.calls();
        let told = calls
            .iter()
            .position(|(m, p)| m == "PATCH" && p == "/clients_api/device")
            .expect("Ring told the push token");
        let (_, headers, body) = net.asked(told);
        assert_eq!(headers["hardware_id"], "hw-test");
        assert_eq!(body["device"]["push_notification_token"], "fake-fcm-token");
        assert_eq!(body["device"]["metadata"]["pn_service"], "fcm");
        assert!(
            !calls.iter().any(|(_, p)| p.contains("register")),
            "{calls:?}"
        );

        let frame = |n: u8, plain: Vec<u8>| {
            let sender = ece::Keys::generate().unwrap();
            let salt = [n; 16];
            let raw = ece::tests::aesgcm(&push.keys, &sender, &salt, &plain);
            let dh = format!("dh={}", ece::base64url(sender.public()));
            let salt = format!("salt={}", ece::base64url(&salt));
            mcs::tests::data(
                &format!("0:{n}"),
                &[("crypto-key", dh.as_str()), ("encryption", salt.as_str())],
                &raw,
            )
        };
        // An old press Google kept: never told.
        server
            .write_all(&frame(1, ring_push("7000000000000000020", 120_000)))
            .await
            .unwrap();
        settle().await;
        assert_eq!(flag(&hub, "ring:10000001/doorbell"), Some(false));
        // A press now: told at once, then put back.
        server
            .write_all(&frame(2, ring_push("7000000000000000021", 0)))
            .await
            .unwrap();
        until("the press", || {
            flag(&hub, "ring:10000001/doorbell") == Some(true)
        })
        .await;
        tokio::time::advance(PRESS + Duration::from_secs(1)).await;
        until("the press over", || {
            flag(&hub, "ring:10000001/doorbell") == Some(false)
        })
        .await;
        // Pushed again: told once only.
        server
            .write_all(&frame(3, ring_push("7000000000000000021", 0)))
            .await
            .unwrap();
        settle().await;
        assert_eq!(flag(&hub, "ring:10000001/doorbell"), Some(false));

        // The push server goes: back to the poll alone.
        drop(server);
        until("in fallback", || {
            text(&hub, "ring:account/push").as_deref() == Some("fallback")
        })
        .await;
        cancel.cancel();
        let _ = std::fs::remove_file(&vault);
    }

    #[tokio::test(start_paused = true)]
    async fn a_new_registration_is_kept_before_ring_is_told() {
        let net = replay();
        net.always_bytes("/checkin", 200, &fcm::tests::checkin_answer(41, 42));
        net.always("/c2dm/register3", 200, "token=fake-gcm-token");
        net.always(
            "/v1/projects/ring-17770/installations",
            200,
            r#"{"authToken":{"token":"fake-fis","expiresIn":"604800s"}}"#,
        );
        net.always(
            "/v1/projects/ring-17770/registrations",
            200,
            r#"{"token":"fake-fcm-new"}"#,
        );
        net.always("PATCH /clients_api/device", 200, "{}");
        let (client, mut server) = tokio::io::duplex(64 * 1024);
        let dial = Arc::new(Pipe(std::sync::Mutex::new(Some(client))));
        let (hub, instance, cancel, vault) = start_with(
            "register",
            Arc::clone(&net),
            serde_json::json!({}),
            dial,
            None,
        )
        .await;
        let login = mcs::tests::accept(&mut server).await;
        assert_eq!(login.str(3), Some("41"), "the new device");
        until("instant alerts active", || {
            text(&hub, "ring:account/push").as_deref() == Some("active")
        })
        .await;
        let kept = hub.secret(&instance, PUSH).expect("kept in the store");
        let kept = Credentials::parse(&kept).unwrap();
        assert_eq!(kept.token, "fake-fcm-new");
        assert_eq!(kept.android_id, 41);
        // The keys kept are the ones Firebase was given.
        let calls = net.calls();
        let registered = calls
            .iter()
            .position(|(_, p)| p.ends_with("/registrations"))
            .unwrap();
        let p256dh = net.asked(registered).2["web"]["p256dh"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(ece::base64url(kept.keys.public()), p256dh);
        let told = calls
            .iter()
            .position(|(m, _)| m == "PATCH")
            .expect("Ring told");
        assert!(told > registered);
        assert_eq!(
            net.asked(told).2["device"]["push_notification_token"],
            "fake-fcm-new"
        );
        cancel.cancel();
        let _ = std::fs::remove_file(&vault);
    }

    #[test]
    fn the_poll_has_a_floor_and_nothing_else_is_accepted() {
        let parsed: Config = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(parsed.poll(), Duration::from_secs(10));
        assert!(parsed.push, "instant alerts unless told otherwise");
        assert_eq!(
            parsed.pace(true, false),
            PUSH_POLL,
            "slower while the push works"
        );
        assert_eq!(parsed.pace(false, false), Duration::from_secs(10));
        assert_eq!(
            parsed.pace(true, true),
            Duration::from_secs(10),
            "its own pace while the push lately missed an event"
        );
        let fast: Config = serde_json::from_value(serde_json::json!({ "poll": 1 })).unwrap();
        assert_eq!(fast.poll(), Duration::from_secs(5));
        assert!(
            serde_json::from_value::<Config>(serde_json::json!({ "password": "x" })).is_err(),
            "a password never goes in the configuration"
        );
    }
}
