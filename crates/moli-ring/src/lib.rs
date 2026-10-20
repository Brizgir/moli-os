//! Ring doorbells, cameras and chimes, through Ring's cloud (there is no
//! local API): who rang, motion, battery, Wi-Fi, images; the floodlight,
//! the siren and motion detection can be switched.
//!
//! Signing in follows the app: e-mail and password once, then the code
//! Ring sends; only the refresh token is kept (encrypted store), the
//! password and the code are forgotten. The device list is read every
//! minute, each camera's history every `history_seconds`: a ring shows up
//! within that delay (Ring's push channel is not used yet).

mod api;
mod model;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::bail;
use moli_core::{Access, Device, Kind, PointSpec, Semantic, Unit, Value};
use moli_net::Method;
use moli_runtime::media::{Image, SnapshotSource};
use moli_runtime::{BoxFuture, CommandRequest, Driver, DriverCtx};
use serde::Deserialize;
use serde_json::json;
use tokio::time::Instant;

use api::{Cloud, Refused, Shared};
use model::{Family, Happened, RingDevice, Seen};

const EMAIL: &str = "email";
const PASSWORD: &str = "password";
const CODE: &str = "code_2fa";
const REFRESH: &str = "refresh_token";
const HARDWARE_ID: &str = "hardware_id";

/// The device list: names, battery, light, siren, connection.
const DEVICES: Duration = Duration::from_secs(60);
/// Wi-Fi signal: changes slowly.
const HEALTH: Duration = Duration::from_secs(600);
/// After an order, the list is read again soon to confirm it.
const CONFIRM: Duration = Duration::from_secs(3);
/// How long `doorbell` and `motion` stay true after the event.
const HOLD: Duration = Duration::from_secs(30);
/// An event older than this when found is the past, not news.
const MAX_AGE_MS: i64 = 5 * 60 * 1000;
/// History entries read per camera and pass.
const PAGE: u32 = 5;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// How often each camera's history is read (who rang, motion).
    #[serde(default = "default_history")]
    pub history_seconds: u64,
    /// How long the siren sounds when switched on (1 to 120 s).
    #[serde(default = "default_siren")]
    pub siren_seconds: u64,
}

fn default_history() -> u64 {
    15
}

fn default_siren() -> u64 {
    30
}

#[derive(Debug)]
pub struct Ring {
    config: Config,
    /// Survives driver restarts: no new sign-in per restart.
    shared: Arc<Shared>,
}

impl Ring {
    /// # Errors
    /// An option out of range.
    pub fn new(config: Config) -> anyhow::Result<Self> {
        if !(10..=3600).contains(&config.history_seconds) {
            bail!("history_seconds: 10 to 3600");
        }
        if !(1..=120).contains(&config.siren_seconds) {
            bail!("siren_seconds: 1 to 120");
        }
        Ok(Self {
            config,
            shared: Arc::default(),
        })
    }
}

impl Driver for Ring {
    fn kind(&self) -> &'static str {
        "ring"
    }

    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(run(&self.config, Arc::clone(&self.shared), ctx))
    }
}

/// Waits for a person (shown everywhere), until the next restart.
async fn park(ctx: &DriverCtx, reason: String) -> anyhow::Result<()> {
    ctx.wait_for(reason);
    ctx.cancelled().await;
    Ok(())
}

/// A random id that names this Moli among the account's devices, kept so
/// that Ring sees one device, not one per restart.
fn new_hardware_id() -> anyhow::Result<String> {
    use ring::rand::{SecureRandom as _, SystemRandom};
    let mut raw = [0u8; 16];
    SystemRandom::new()
        .fill(&mut raw)
        .map_err(|_| anyhow::anyhow!("no system randomness"))?;
    // A version 4 UUID.
    raw[6] = (raw[6] & 0x0f) | 0x40;
    raw[8] = (raw[8] & 0x3f) | 0x80;
    let hex: String = raw
        .iter()
        .flat_map(|b| [b >> 4, b & 0x0f])
        .map(|n| char::from_digit(u32::from(n), 16).unwrap_or('0'))
        .collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}

async fn run(config: &Config, shared: Arc<Shared>, ctx: &mut DriverCtx) -> anyhow::Result<()> {
    if !ctx.can_store_secrets() {
        let why = ctx.secrets_problem().unwrap_or("?").to_owned();
        let reason = moli_i18n::tr!("pilotes.ring.coffre", why = why);
        return park(ctx, reason).await;
    }
    let hardware_id = if let Some(id) = ctx.secret(HARDWARE_ID) {
        id
    } else {
        let id = new_hardware_id()?;
        ctx.store_secret(HARDWARE_ID, &id).await?;
        id
    };
    let cloud = Cloud::new(shared, hardware_id);
    if let Some(refresh) = ctx.secret(REFRESH) {
        cloud.resume(refresh);
    } else {
        match sign_in(ctx, &cloud).await? {
            Ok(()) => {}
            Err(reason) => return park(ctx, reason).await,
        }
    }
    match serve(config, ctx, &cloud).await {
        Err(e) if e.downcast_ref::<Refused>() == Some(&Refused::SignedOut) => {
            ctx.forget_secret(REFRESH).await?;
            let reason = moli_i18n::tr!(
                "pilotes.ring.deconnecte",
                instance = ctx.instance(),
                password = PASSWORD
            );
            park(ctx, reason).await
        }
        other => other,
    }
}

/// First sign-in, from the e-mail and password a person filed (and the
/// code, once Ring sent one). `Err(reason)`: what the person must do.
async fn sign_in(ctx: &DriverCtx, cloud: &Cloud) -> anyhow::Result<Result<(), String>> {
    let (Some(email), Some(password)) = (ctx.secret(EMAIL), ctx.secret(PASSWORD)) else {
        return Ok(Err(moli_i18n::tr!(
            "pilotes.ring.identifiants_manquants",
            instance = ctx.instance(),
            email = EMAIL,
            password = PASSWORD
        )));
    };
    let code = ctx.secret(CODE);
    let error = match cloud.sign_in(&email, &password, code.as_deref()).await {
        Ok(refresh) => {
            ctx.store_secret(REFRESH, &refresh).await?;
            // Only the refresh token is kept, as Home Assistant does.
            ctx.forget_secret(PASSWORD).await?;
            ctx.forget_secret(CODE).await?;
            let _ = cloud.rotated();
            tracing::info!(instance = %ctx.instance(), "ring account signed in");
            return Ok(Ok(()));
        }
        Err(e) => e,
    };
    let Some(refused) = error.downcast_ref::<Refused>() else {
        return Err(error);
    };
    if code.is_some() {
        // A used or wrong code is never sent again.
        ctx.forget_secret(CODE).await?;
        if *refused != Refused::NeedsCode {
            return Ok(Err(moli_i18n::tr!(
                "pilotes.ring.code_refuse",
                instance = ctx.instance(),
                code = CODE
            )));
        }
    }
    Ok(Err(match refused {
        Refused::NeedsCode => moli_i18n::tr!(
            "pilotes.ring.code_demande",
            instance = ctx.instance(),
            code = CODE
        ),
        _ => moli_i18n::tr!(
            "pilotes.ring.refuse",
            instance = ctx.instance(),
            password = PASSWORD
        ),
    }))
}

fn binary(key: &str, label: &str, write: bool, semantic: Semantic) -> PointSpec {
    PointSpec {
        key: key.into(),
        label: label.into(),
        kind: Kind::Binary,
        access: Access { read: true, write },
        unit: None,
        semantic,
    }
}

fn numeric(key: &str, label: &str, range: (f64, f64), unit: Unit, semantic: Semantic) -> PointSpec {
    PointSpec {
        key: key.into(),
        label: label.into(),
        kind: Kind::Numeric {
            min: Some(range.0),
            max: Some(range.1),
            step: Some(1.0),
        },
        access: Access {
            read: true,
            write: false,
        },
        unit: Some(unit),
        semantic,
    }
}

fn points(device: &RingDevice) -> Vec<PointSpec> {
    let tr = |key: &str| moli_i18n::tr(&format!("pilotes.ring.{key}"));
    let mut points = Vec::new();
    if device.family == Family::Doorbot {
        points.push(binary("doorbell", &tr("sonnette"), false, Semantic::Other));
    }
    if device.family.has_video() {
        points.push(binary(
            "motion",
            &tr("mouvement"),
            false,
            Semantic::Occupancy,
        ));
    }
    if device.motion_detection.is_some() {
        points.push(binary(
            "motion_detection",
            &tr("detection"),
            true,
            Semantic::Config,
        ));
    }
    if device.light.is_some() {
        points.push(binary("light", &tr("lumiere"), true, Semantic::OnOff));
    }
    if device.siren.is_some() {
        // An agent sounding a siren: a person approves first.
        points.push(binary("siren", &tr("sirene"), true, Semantic::Control));
    }
    if device.battery.is_some() {
        points.push(numeric(
            "battery",
            &tr("batterie"),
            (0.0, 100.0),
            Unit::Percent,
            Semantic::Battery,
        ));
    }
    points.push(numeric(
        "rssi",
        &tr("wifi"),
        (-120.0, 0.0),
        Unit::Other("dBm".into()),
        Semantic::SignalStrength,
    ));
    points
}

fn describe(ctx: &DriverCtx, device: &RingDevice) -> Device {
    Device {
        id: ctx.device_id(&device.id.to_string()),
        instance: ctx.instance().clone(),
        native_name: device.name.as_str().into(),
        manufacturer: Some("Ring".into()),
        model: (!device.kind.is_empty()).then(|| device.kind.as_str().into()),
        description: None,
        native_room: None,
        members: Vec::new(),
        points: points(device),
    }
}

/// One camera's images: Ring takes a fresh one if it can (a battery camera
/// often cannot), then gives its latest.
#[derive(Debug)]
struct Snapshots {
    cloud: Cloud,
    id: u64,
}

impl SnapshotSource for Snapshots {
    fn snapshot(&self) -> BoxFuture<'_, anyhow::Result<Image>> {
        Box::pin(async move {
            let ask = json!({ "doorbot_ids": [self.id] });
            let stamp = |answer: &serde_json::Value| answer["timestamps"][0]["timestamp"].as_i64();
            let before = self
                .cloud
                .json(
                    Method::POST,
                    "/clients_api/snapshots/timestamps",
                    Some(&ask),
                )
                .await
                .ok()
                .and_then(|a| stamp(&a));
            for _ in 0..4 {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let now = self
                    .cloud
                    .json(
                        Method::POST,
                        "/clients_api/snapshots/timestamps",
                        Some(&ask),
                    )
                    .await
                    .ok()
                    .and_then(|a| stamp(&a));
                if now.is_some() && now != before {
                    break;
                }
            }
            let path = format!("/clients_api/snapshots/image/{}", self.id);
            let (status, bytes) = self.cloud.call(Method::GET, &path, None).await?;
            if status.is_success() && bytes.starts_with(&[0xFF, 0xD8]) {
                return Ok(Image {
                    content_type: "image/jpeg".into(),
                    bytes: bytes.to_vec(),
                });
            }
            bail!(moli_i18n::tr!("pilotes.ring.aucune_image"))
        })
    }
}

/// What the driver knows of each device between passes.
#[derive(Default)]
struct State {
    devices: BTreeMap<u64, RingDevice>,
    published: BTreeMap<u64, Device>,
    seen: BTreeMap<u64, Seen>,
    /// Points raised by an event, and when they fall back.
    falls: BTreeMap<(u64, &'static str), Instant>,
}

async fn serve(config: &Config, ctx: &mut DriverCtx, cloud: &Cloud) -> anyhow::Result<()> {
    cloud.open_session().await?;
    let mut state = State::default();
    let history_every = Duration::from_secs(config.history_seconds);
    let now = Instant::now();
    let (mut devices_at, mut health_at, mut history_at) = (now, now, now + history_every);
    loop {
        let next_fall = state.falls.values().min().copied();
        let wake = [Some(devices_at), Some(history_at), next_fall]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or(devices_at);
        tokio::select! {
            command = ctx.next_command() => match command {
                Some(command) => {
                    let result = order(config, ctx, cloud, &state, &command).await;
                    if result.is_ok() {
                        devices_at = devices_at.min(Instant::now() + CONFIRM);
                    }
                    command.reply(result);
                }
                None => return Ok(()),
            },
            () = tokio::time::sleep_until(wake) => {
                let now = Instant::now();
                if now >= devices_at {
                    let health = now >= health_at;
                    refresh(ctx, cloud, &mut state, health).await?;
                    if health {
                        health_at = now + HEALTH;
                    }
                    devices_at = now + DEVICES;
                    ctx.ready();
                }
                if now >= history_at {
                    histories(ctx, cloud, &mut state).await?;
                    history_at = Instant::now() + history_every;
                }
                fall(ctx, &mut state, now);
            }
        }
        if let Some(refresh) = cloud.rotated() {
            ctx.store_secret(REFRESH, &refresh).await?;
        }
    }
}

/// Reads the device list (and the Wi-Fi of each, when `health`).
async fn refresh(
    ctx: &DriverCtx,
    cloud: &Cloud,
    state: &mut State,
    health: bool,
) -> anyhow::Result<()> {
    let list = cloud
        .json(Method::GET, "/clients_api/ring_devices", None)
        .await?;
    let list = model::devices(&list);
    for device in &list {
        let described = describe(ctx, device);
        let id = described.id.clone();
        if state.published.get(&device.id) != Some(&described) {
            ctx.upsert_device(described.clone());
            if device.family.has_video() && !state.published.contains_key(&device.id) {
                ctx.provide_snapshots(
                    &id,
                    Arc::new(Snapshots {
                        cloud: cloud.clone(),
                        id: device.id,
                    }),
                );
            }
            state.published.insert(device.id, described);
        }
        ctx.set_availability(&id, device.online);
        let values = [
            ("battery", device.battery.map(Value::Int)),
            ("light", device.light.map(Value::Bool)),
            ("siren", device.siren.map(Value::Bool)),
            ("motion_detection", device.motion_detection.map(Value::Bool)),
        ];
        for (key, value) in values {
            if let Some(value) = value {
                ctx.set_state(&id, key, value);
            }
        }
        if health {
            let path = format!("/clients_api/{}/{}/health", device.family.api(), device.id);
            match cloud.json(Method::GET, &path, None).await {
                Ok(answer) => {
                    if let Some(dbm) = model::signal(&answer) {
                        ctx.set_state(&id, "rssi", Value::Int(dbm));
                    }
                }
                Err(e) if e.downcast_ref::<Refused>().is_some() => return Err(e),
                Err(e) => tracing::debug!(device = %id, error = %e, "ring health"),
            }
        }
    }
    // A device removed from the account leaves Moli too.
    let current: Vec<u64> = list.iter().map(|d| d.id).collect();
    for gone in ctx.devices().into_iter().filter(|d| {
        !current
            .iter()
            .any(|id| ctx.device_id(&id.to_string()) == *d)
    }) {
        ctx.remove_device(&gone);
    }
    state.published.retain(|id, _| current.contains(id));
    state.seen.retain(|id, _| current.contains(id));
    state.devices = list.into_iter().map(|d| (d.id, d)).collect();
    Ok(())
}

/// Reads each camera's latest events; a new ring or motion raises its point.
async fn histories(ctx: &DriverCtx, cloud: &Cloud, state: &mut State) -> anyhow::Result<()> {
    let cameras: Vec<u64> = state
        .devices
        .values()
        .filter(|d| d.family.has_video() && d.online)
        .map(|d| d.id)
        .collect();
    let now_ms = i64::try_from(moli_core::now_ms()).unwrap_or(i64::MAX);
    for id in cameras {
        let path = format!("/clients_api/doorbots/{id}/history?limit={PAGE}");
        let page = match cloud.json(Method::GET, &path, None).await {
            Ok(answer) => model::history(&answer),
            Err(e) if e.downcast_ref::<Refused>().is_some() => return Err(e),
            Err(e) => {
                // One camera that fails does not hide the others.
                tracing::debug!(device = id, error = %e, "ring history");
                continue;
            }
        };
        let news = state
            .seen
            .entry(id)
            .or_default()
            .news(&page, now_ms, MAX_AGE_MS);
        let device = ctx.device_id(&id.to_string());
        for happened in news {
            let key = happened.key();
            // Two rings in a row: two events, even if still true.
            ctx.pulse_state(&device, key, Value::Bool(true));
            state.falls.insert((id, key), Instant::now() + HOLD);
            if happened == Happened::Ding {
                tracing::info!(device = %device, "ring: someone rang");
            }
        }
    }
    Ok(())
}

/// Points raised by an event fall back once their time is up.
fn fall(ctx: &DriverCtx, state: &mut State, now: Instant) {
    state.falls.retain(|(id, key), at| {
        if *at > now {
            return true;
        }
        ctx.set_state(&ctx.device_id(&id.to_string()), key, Value::Bool(false));
        false
    });
}

/// Carries out an order; the new value is shown at once, and the device
/// list is read again shortly to confirm it.
async fn order(
    config: &Config,
    ctx: &DriverCtx,
    cloud: &Cloud,
    state: &State,
    command: &CommandRequest,
) -> Result<(), String> {
    let device = state
        .devices
        .values()
        .find(|d| ctx.device_id(&d.id.to_string()) == command.device.id)
        .ok_or_else(|| moli_i18n::tr!("pilotes.ring.inconnu"))?;
    let Value::Bool(on) = command.value else {
        return Err(moli_i18n::tr!("pilotes.ring.oui_non"));
    };
    let id = device.id;
    let switch = if on { "on" } else { "off" };
    let (method, path, body) = match &*command.key {
        "light" => (
            Method::PUT,
            format!("/clients_api/doorbots/{id}/floodlight_light_{switch}"),
            None,
        ),
        "siren" if on => (
            Method::PUT,
            format!(
                "/clients_api/doorbots/{id}/siren_on?duration={}",
                config.siren_seconds
            ),
            None,
        ),
        "siren" => (
            Method::PUT,
            format!("/clients_api/doorbots/{id}/siren_off"),
            None,
        ),
        "motion_detection" => (
            Method::PATCH,
            format!("/devices/v1/devices/{id}/settings"),
            Some(json!({ "motion_settings": { "motion_detection_enabled": on } })),
        ),
        _ => return Err(moli_i18n::tr!("pilotes.ring.lecture_seule")),
    };
    if command.abandoned() {
        return Err("expired".into());
    }
    cloud
        .json(method, &path, body.as_ref())
        .await
        .map_err(|e| moli_i18n::tr!("pilotes.ring.echec", cause = e.root_cause()))?;
    ctx.set_state(&command.device.id, &command.key, command.value.clone());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_are_bounded() {
        let config = |history_seconds, siren_seconds| Config {
            history_seconds,
            siren_seconds,
        };
        assert!(Ring::new(config(15, 30)).is_ok());
        assert!(Ring::new(config(2, 30)).is_err(), "would hammer Ring");
        assert!(Ring::new(config(15, 0)).is_err());
        assert!(Ring::new(config(15, 600)).is_err());
    }

    #[test]
    fn hardware_ids_are_uuids() {
        let id = new_hardware_id().unwrap();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        assert_ne!(id, new_hardware_id().unwrap());
    }

    #[test]
    fn points_follow_what_the_device_has() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../integrations/ring/fixtures/ring_devices.json");
        let list = model::devices(&serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap());
        let keys = |d: &RingDevice| -> Vec<String> {
            points(d).iter().map(|p| p.key.to_string()).collect()
        };
        assert_eq!(
            keys(&list[0]),
            ["doorbell", "motion", "motion_detection", "battery", "rssi"]
        );
        assert_eq!(
            keys(&list[2]),
            ["motion", "motion_detection", "light", "siren", "rssi"]
        );
        assert_eq!(keys(&list[4]), ["rssi"], "a chime");
        let siren = points(&list[2])
            .into_iter()
            .find(|p| &*p.key == "siren")
            .unwrap();
        assert_eq!(siren.semantic, Semantic::Control, "an agent asks first");
    }
}
