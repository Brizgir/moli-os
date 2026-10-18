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
//! Presses and motion are read from the devices' latest events every few
//! seconds (not Ring's push notifications): an event is told once, only if
//! it is less than [`model::FRESH`] old, and never from the first reading
//! after a start, so an old press never reads as someone at the door.

mod api;
mod auth;
mod images;
mod model;
mod net;
mod signin;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use moli_core::{Access, Device, DeviceId, Kind, PointSpec, Semantic, Value};
use moli_runtime::{BoxFuture, CommandRequest, Driver, DriverCtx, SecretKeeper};
use serde::Deserialize;
use tokio::sync::{Mutex, mpsc};
use tokio::time::{Instant, MissedTickBehavior};

use crate::api::{Client, Failure, Vault};
use crate::images::{Images, Shared, Taken};
use crate::model::{Family, Happened, ModeReading, RingDevice, Watch};
use crate::net::{Transport, Web};
use crate::signin::Refusal;

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

pub struct Ring {
    config: Config,
    net: Arc<dyn Transport>,
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
        }
    }

    #[cfg(test)]
    fn with_transport(config: Config, net: Arc<dyn Transport>) -> Self {
        Self { config, net }
    }
}

impl Driver for Ring {
    fn kind(&self) -> &'static str {
        "ring"
    }

    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(run(&self.config, Arc::clone(&self.net), ctx))
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

async fn run(config: &Config, net: Arc<dyn Transport>, ctx: &mut DriverCtx) -> anyhow::Result<()> {
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
        let failure = match connected(config, ctx, &account, &vault, client).await {
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
}

/// Connected: the devices, then their events, health and modes, until
/// stopped.
async fn connected(
    config: &Config,
    ctx: &mut DriverCtx,
    account: &DeviceId,
    vault: &Arc<dyn Vault>,
    mut client: Client,
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
    let result = watch(config, ctx, account, &mut session, images_taken).await;
    if result.is_err() {
        lost(ctx, &session);
    }
    result.map_err(fail)
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
) -> Result<(), Failure> {
    let mut poll = tokio::time::interval(config.poll());
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
    loop {
        let next_reset = resets.values().min().copied();
        tokio::select! {
            command = ctx.next_command() => {
                let Some(command) = command else { return Ok(()) };
                command_received(ctx, account, session, command).await?;
            }
            Some((id, at)) = taken.recv() => {
                set_text(ctx, &id, "image_at", &model::rfc3339(at));
            }
            _ = poll.tick() => events(ctx, session, &mut resets).await?,
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
    ctx: &DriverCtx,
    account: &DeviceId,
    session: &mut Session,
    command: CommandRequest,
) -> Result<(), Failure> {
    if command.device.id == *account {
        command.reply(Err(moli_i18n::tr!("pilotes.ring.deja_connecte")));
        return Ok(());
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
        let entry = session.tracked.entry(id).or_insert_with(|| Tracked {
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

/// The latest events of each doorbell and camera; the new ones told.
async fn events(
    ctx: &DriverCtx,
    session: &mut Session,
    resets: &mut HashMap<(DeviceId, &'static str), Instant>,
) -> Result<(), Failure> {
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

    async fn start(name: &str, net: Arc<Replay>) -> (Hub, InstanceId, CancellationToken, PathBuf) {
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
        let cancel = CancellationToken::new();
        let config: Config = serde_json::from_value(serde_json::json!({})).unwrap();
        spawn_driver(
            &hub,
            instance.clone(),
            Arc::new(Ring::with_transport(config, net)),
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
