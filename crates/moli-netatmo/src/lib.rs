//! Netatmo Energy in the house (thermostat, radiator valves, their relay),
//! through Netatmo's official cloud API: the relay has no local interface.
//! Each heated room is a device (its temperature, its setpoint, its heating),
//! the home's heating mode is another, and each module tells its battery
//! and its signal.
//!
//! The person creates their own app on dev.netatmo.com; its client id and
//! secret go to the encrypted store (the dashboard's Netatmo card, or
//! `moli-os secrets set`). They then accept on Netatmo's authorization page
//! and paste the address it ends on into Moli, which exchanges its code and
//! keeps the refresh token (rotated by Netatmo: the newest is always filed).
//!
//! Netatmo is asked every five minutes at most, never in bursts, and goes
//! quiet longer each time it says its quota is reached. An order that could
//! not leave within the hub's deadline is never sent later: a radiator
//! heating hours after a « failed » is worse than a refusal.
//!
//! The home's heating schedules are the home device's own document (a
//! [`Service`]): read by anyone, changed by a person only (the dashboard,
//! with the code), never by an agent nor an automation; each change is
//! checked strictly, sent alone, then `homesdata` is read again.

mod api;
mod auth;
mod encode;
mod model;
mod schedule;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use http::Method;
use moli_core::{Device, DeviceId, Kind, PointSpec, Semantic, Value};
use moli_runtime::service::{Service, ServiceError};
use moli_runtime::{BoxFuture, CommandRequest, Driver, DriverCtx};
use serde::Deserialize;
use serde_json::Value as Json;
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;

use crate::api::{Client, Failure, Https, Payload, Transport as _, Vault};
use crate::auth::App;
use crate::model::{Home, Target};
use crate::schedule::Draft;

const CLIENT_ID: &str = "client_id";
const CLIENT_SECRET: &str = "client_secret";
const REFRESH: &str = "refresh_token";
/// The homes, rooms and modules (one added, renamed in the Netatmo app…):
/// one `homesdata` call, so a new name shows in the house within minutes.
const STRUCTURE: Duration = Duration::from_secs(15 * 60);
/// The store, looked at this often while Moli waits for the person.
const LOOK: Duration = Duration::from_secs(5);
/// Never asked more often than this, whatever the config says.
const POLL_MIN: u64 = 120;
/// The schedules a dashboard reads are read again from Netatmo when older
/// than this (changed in the Netatmo app meanwhile).
const SCHEDULES_FRESH: Duration = Duration::from_secs(60);
const RETRY_MIN: Duration = Duration::from_secs(30);
const RETRY_MAX: Duration = Duration::from_secs(30 * 60);

fn default_redirect() -> String {
    "http://localhost/moli-netatmo".into()
}

fn default_poll() -> u64 {
    300
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// The redirect URI given to the app on dev.netatmo.com: the page
    /// Netatmo sends the browser back to, whose address is pasted into Moli
    /// (it need not exist).
    #[serde(default = "default_redirect")]
    pub redirect_uri: String,
    /// How long a setpoint set from Moli lasts, in minutes (1 to 720);
    /// none: the home's own duration in the Netatmo app, else 3 hours.
    #[serde(default)]
    pub setpoint_minutes: Option<u32>,
    /// Netatmo is asked this often, in seconds (120 at least).
    #[serde(default = "default_poll")]
    pub poll_seconds: u64,
}

impl Config {
    fn poll(&self) -> Duration {
        Duration::from_secs(self.poll_seconds.max(POLL_MIN))
    }

    fn minutes(&self, home: Option<&Home>) -> u32 {
        self.setpoint_minutes
            .filter(|m| (1..=720).contains(m))
            .or_else(|| home.and_then(|h| h.default_minutes))
            .unwrap_or(model::DEFAULT_MINUTES)
    }
}

#[derive(Debug)]
pub struct Netatmo {
    config: Config,
}

impl Netatmo {
    #[must_use]
    pub fn new(config: Config) -> Self {
        Self { config }
    }
}

impl Driver for Netatmo {
    fn kind(&self) -> &'static str {
        "netatmo"
    }

    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(run(&self.config, ctx))
    }
}

fn text_point(key: &str, label: &str, write: bool) -> PointSpec {
    PointSpec {
        key: key.into(),
        label: label.into(),
        kind: Kind::Text,
        access: moli_core::Access { read: true, write },
        unit: None,
        semantic: Semantic::Config,
    }
}

/// The account: its state, and where the dashboard connects it.
fn account_device(ctx: &DriverCtx, id: &DeviceId) -> Device {
    use moli_i18n::tr;
    Device {
        id: id.clone(),
        instance: ctx.instance().clone(),
        native_name: tr!("pilotes.netatmo.compte").into(),
        manufacturer: Some("Netatmo".into()),
        model: Some("Energy API".into()),
        description: Some(tr!("pilotes.netatmo.description").into()),
        native_room: None,
        members: Vec::new(),
        points: vec![
            text_point("status", &tr!("pilotes.netatmo.etat_compte"), false),
            text_point(
                "redirect_uri",
                &tr!("pilotes.netatmo.adresse_retour"),
                false,
            ),
            text_point("sign_in_url", &tr!("pilotes.netatmo.page_netatmo"), false),
            text_point("sign_in", &tr!("pilotes.netatmo.adresse_collee"), true),
        ],
    }
}

fn set_text(ctx: &DriverCtx, id: &DeviceId, key: &str, text: &str) {
    ctx.set_state(id, key, Value::Text(text.into()));
}

/// The person's app, once both its keys are in the store.
fn app_keys(ctx: &DriverCtx) -> Option<App> {
    Some(App {
        client_id: ctx.secret(CLIENT_ID).filter(|v| !v.trim().is_empty())?,
        client_secret: ctx.secret(CLIENT_SECRET).filter(|v| !v.trim().is_empty())?,
    })
}

/// Why `connected` ended.
#[derive(Debug)]
enum Stop {
    SignInAgain(String),
    BadApp(String),
    Fault(anyhow::Error),
}

/// A failure the connection cannot go on with, as a [`Stop`]; the others
/// back, for the caller to deal with.
fn fatal(failure: Failure) -> Result<Failure, Stop> {
    match failure {
        Failure::SignInAgain(why) => Err(Stop::SignInAgain(why)),
        Failure::BadApp(why) => Err(Stop::BadApp(why)),
        other => Ok(other),
    }
}

/// Any failure as a [`Stop`] (the connection cannot start without it).
fn stop(failure: Failure) -> Stop {
    fatal(failure).map_or_else(|stop| stop, |f| Stop::Fault(anyhow::anyhow!("{f}")))
}

async fn run(config: &Config, ctx: &mut DriverCtx) -> anyhow::Result<()> {
    let account = ctx.device_id("account");
    ctx.upsert_device(account_device(ctx, &account));
    ctx.set_availability(&account, true);
    set_text(ctx, &account, "redirect_uri", &config.redirect_uri);
    let mut pause = RETRY_MIN;
    loop {
        let Some(app) = app_keys(ctx) else {
            if !wait_for_app(ctx, &account, None).await {
                return Ok(());
            }
            continue;
        };
        let Some(refresh) = ctx.secret(REFRESH) else {
            if !sign_in(config, ctx, &account, &app).await? {
                return Ok(());
            }
            continue;
        };
        set_text(ctx, &account, "status", "connecting");
        let started = Instant::now();
        match connected(config, ctx, &account, app.clone(), refresh).await {
            Ok(()) => return Ok(()),
            Err(Stop::SignInAgain(why)) => {
                tracing::warn!(instance = %ctx.instance(), why = %why, "netatmo: sign in again");
                ctx.forget_secret(REFRESH).await?;
            }
            Err(Stop::BadApp(why)) => {
                tracing::warn!(instance = %ctx.instance(), why = %why, "netatmo refuses the app's keys");
                set_text(ctx, &account, "status", "app_refused");
                ctx.wait_for(moli_i18n::tr!("pilotes.netatmo.app_refusee", why = why));
                if !wait_for_app(ctx, &account, Some(&app)).await {
                    return Ok(());
                }
            }
            Err(Stop::Fault(e)) => {
                tracing::warn!(instance = %ctx.instance(), error = %format!("{e:#}"), "netatmo unreachable");
                set_text(ctx, &account, "status", "error");
                if started.elapsed() > RETRY_MAX {
                    pause = RETRY_MIN;
                }
                tokio::select! {
                    () = tokio::time::sleep(pause) => {}
                    () = ctx.cancelled() => return Ok(()),
                }
                pause = (pause * 2).min(RETRY_MAX);
            }
        }
    }
}

/// Waits for the app's keys (other than `refused`, when Netatmo refused
/// those). `false`: stopped.
async fn wait_for_app(ctx: &mut DriverCtx, account: &DeviceId, refused: Option<&App>) -> bool {
    if refused.is_none() {
        set_text(ctx, account, "status", "app_missing");
        set_text(ctx, account, "sign_in_url", "");
        ctx.wait_for(moli_i18n::tr!("pilotes.netatmo.app_a_creer"));
    }
    loop {
        tokio::select! {
            command = ctx.next_command() => {
                let Some(command) = command else { return false };
                command.reply(Err(moli_i18n::tr!("pilotes.netatmo.pas_d_app")));
            }
            () = tokio::time::sleep(LOOK) => {
                if let Some(app) = app_keys(ctx) && Some(&app) != refused {
                    return true;
                }
            }
        }
    }
}

/// Waits for the person to accept on Netatmo's page and paste its address.
/// `false`: stopped; `true`: connected, or the app's keys changed.
async fn sign_in(
    config: &Config,
    ctx: &mut DriverCtx,
    account: &DeviceId,
    app: &App,
) -> anyhow::Result<bool> {
    let mut pending = auth::SignIn::new(app, &config.redirect_uri)?;
    set_text(ctx, account, "sign_in_url", &pending.url);
    set_text(ctx, account, "status", "sign_in");
    ctx.wait_for(moli_i18n::tr!("pilotes.netatmo.a_connecter"));
    loop {
        let command = tokio::select! {
            command = ctx.next_command() => command,
            () = tokio::time::sleep(LOOK) => {
                if app_keys(ctx).as_ref() != Some(app) {
                    // New keys: a new page for them.
                    return Ok(true);
                }
                continue;
            }
        };
        let Some(command) = command else {
            return Ok(false);
        };
        if command.device.id != *account || &*command.key != "sign_in" {
            command.reply(Err(moli_i18n::tr!("pilotes.netatmo.pas_connecte")));
            continue;
        }
        let Value::Text(pasted) = &command.value else {
            command.reply(Err(moli_i18n::tr!("pilotes.netatmo.sans_code")));
            continue;
        };
        let code = match auth::code_of(pasted, &pending.state) {
            Ok(code) => code,
            Err(why) => {
                command.reply(Err(match why {
                    auth::Pasted::NoCode => moli_i18n::tr!("pilotes.netatmo.sans_code"),
                    auth::Pasted::Denied => moli_i18n::tr!("pilotes.netatmo.refus_page"),
                    auth::Pasted::OldPage => moli_i18n::tr!("pilotes.netatmo.ancienne_page"),
                }));
                continue;
            }
        };
        match exchange(app, &code, &config.redirect_uri).await {
            Ok(tokens) => {
                if let Err(e) = ctx.store_secret(REFRESH, &tokens.refresh).await {
                    command.reply(Err(moli_i18n::tr!("pilotes.netatmo.coffre", why = e)));
                    return Err(e.into());
                }
                set_text(ctx, account, "sign_in_url", "");
                command.reply(Ok(()));
                tracing::info!(instance = %ctx.instance(), "netatmo account connected");
                return Ok(true);
            }
            Err(why) => {
                tracing::warn!(instance = %ctx.instance(), why = %why, "netatmo refused the code");
                // A code serves once: a new page for the next try.
                pending = auth::SignIn::new(app, &config.redirect_uri)?;
                set_text(ctx, account, "sign_in_url", &pending.url);
                command.reply(Err(moli_i18n::tr!(
                    "pilotes.netatmo.code_refuse",
                    why = why
                )));
            }
        }
    }
}

/// The pasted code, exchanged for tokens.
async fn exchange(app: &App, code: &str, redirect: &str) -> Result<auth::Tokens, String> {
    let answer = Https
        .send(
            Method::POST,
            auth::TOKEN_PATH,
            None,
            Some(Payload::Form(auth::code_form(app, code, redirect))),
        )
        .await
        .map_err(|e| format!("{e:#}"))?;
    auth::tokens(answer.status, &answer.body, None).map_err(|refused| match refused {
        auth::Refused::SignInAgain(why)
        | auth::Refused::BadApp(why)
        | auth::Refused::Other(why) => why,
        auth::Refused::Quota => "quota".into(),
    })
}

/// Files the rotated refresh token in the instance's encrypted store.
struct Store<'a>(&'a DriverCtx);

impl Vault for Store<'_> {
    fn keep<'a>(&'a self, refresh: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            self.0.store_secret(REFRESH, refresh).await?;
            tracing::info!(instance = %self.0.instance(), "netatmo refresh token renewed");
            Ok(())
        })
    }
}

/// The homes Moli shows, as Netatmo last described them.
#[derive(Debug, Default)]
struct House {
    homes: Vec<Home>,
    /// When `homesdata` was last read.
    fetched: Option<Instant>,
}

/// What a surface asks of a home's schedules (see [`Desk`]).
#[derive(Debug)]
enum Request {
    Read,
    Sync(Json),
    Create(Json),
    Activate(Json),
}

/// One question or action, and where its answer goes.
#[derive(Debug)]
struct Ask {
    home: String,
    request: Request,
    reply: oneshot::Sender<Result<Json, ServiceError>>,
}

/// The home device's service: the schedules. Each ask is handed to the
/// driver's loop (the one holder of the client: one request at a time,
/// paced like the rest), which answers once Netatmo has.
#[derive(Debug)]
struct Desk {
    home: String,
    asks: mpsc::Sender<Ask>,
}

impl Desk {
    fn ask(&self, request: Request) -> BoxFuture<'_, Result<Json, ServiceError>> {
        Box::pin(async move {
            let (reply, answer) = oneshot::channel();
            let ask = Ask {
                home: self.home.clone(),
                request,
                reply,
            };
            self.asks
                .try_send(ask)
                .map_err(|_| ServiceError::Unavailable(moli_i18n::tr!("pilotes.netatmo.occupe")))?;
            answer.await.unwrap_or_else(|_| {
                Err(ServiceError::Unavailable(moli_i18n::tr!(
                    "pilotes.netatmo.occupe"
                )))
            })
        })
    }
}

impl Service for Desk {
    fn read(&self, what: &str) -> BoxFuture<'_, Result<Json, ServiceError>> {
        match what {
            "schedules" => self.ask(Request::Read),
            other => {
                let other = other.to_owned();
                Box::pin(async move { Err(ServiceError::UnknownAction(other)) })
            }
        }
    }

    fn write(&self, action: &str, input: Json) -> BoxFuture<'_, Result<Json, ServiceError>> {
        match action {
            "sync" => self.ask(Request::Sync(input)),
            "create" => self.ask(Request::Create(input)),
            "activate" => self.ask(Request::Activate(input)),
            other => {
                let other = other.to_owned();
                Box::pin(async move { Err(ServiceError::UnknownAction(other)) })
            }
        }
    }
}

impl House {
    fn home(&self, id: &str) -> Option<&Home> {
        self.homes.iter().find(|h| h.id == id)
    }

    /// What an order on `device` is about.
    fn target(&self, ctx: &DriverCtx, device: &DeviceId) -> Option<Target> {
        self.homes.iter().find_map(|home| {
            if ctx.device_id(&home.native()) == *device {
                return Some(Target::Home {
                    home: home.id.clone(),
                });
            }
            home.rooms
                .iter()
                .find(|room| ctx.device_id(&home.room_native(room)) == *device)
                .map(|room| Target::Room {
                    home: home.id.clone(),
                    room: room.id.clone(),
                })
        })
    }
}

/// Connected: the homes, then their state and the orders, until stopped.
async fn connected(
    config: &Config,
    ctx: &mut DriverCtx,
    account: &DeviceId,
    app: App,
    refresh: String,
) -> Result<(), Stop> {
    let mut client = Client::new(Https, app, refresh);
    let mut house = House::default();
    // A few asks wait at most: a dashboard is not a queue.
    let (asks, mut asked) = mpsc::channel::<Ask>(4);
    discover(ctx, &mut client, &mut house, &asks)
        .await
        .map_err(stop)?;
    set_text(ctx, account, "status", "connected");
    ctx.ready();
    let poll = config.poll();
    let mut next_poll = Instant::now();
    let mut next_structure = Instant::now() + STRUCTURE;
    let mut failures = 0u32;
    loop {
        tokio::select! {
            command = ctx.next_command() => {
                let Some(command) = command else { return Ok(()) };
                order(config, ctx, &mut client, &mut house, &asks, command).await?;
            }
            ask = asked.recv() => {
                let Some(ask) = ask else { return Ok(()) };
                serve(ctx, &mut client, &mut house, &asks, ask).await?;
            }
            () = tokio::time::sleep_until(next_poll) => {
                let result = if Instant::now() >= next_structure {
                    next_structure = Instant::now() + STRUCTURE;
                    discover(ctx, &mut client, &mut house, &asks).await
                } else {
                    Ok(())
                };
                let result = match result {
                    Ok(()) => read_all(ctx, &mut client, &house).await,
                    Err(f) => Err(f),
                };
                match result {
                    Ok(()) => {
                        if failures > 0 {
                            set_text(ctx, account, "status", "connected");
                        }
                        failures = 0;
                        next_poll = Instant::now() + poll;
                    }
                    Err(f) => {
                        failures += 1;
                        next_poll = Instant::now() + match fatal(f)? {
                            Failure::Quota(wait) => {
                                set_text(ctx, account, "status", "quota");
                                tracing::warn!(instance = %ctx.instance(), wait = wait.as_secs(), "netatmo quota reached");
                                wait.max(poll)
                            }
                            other => {
                                set_text(ctx, account, "status", "error");
                                tracing::warn!(instance = %ctx.instance(), error = %other, failures, "netatmo state");
                                (poll * 2u32.saturating_pow(failures.min(4))).min(RETRY_MAX)
                            }
                        };
                    }
                }
            }
        }
    }
}

/// The account's Energy homes, shown in the house (and what is gone, removed).
async fn discover(
    ctx: &DriverCtx,
    client: &mut Client<Https>,
    house: &mut House,
    asks: &mpsc::Sender<Ask>,
) -> Result<(), Failure> {
    let answer = client
        .call(&Store(ctx), Method::GET, "/api/homesdata", None, &|| true)
        .await?;
    house.fetched = Some(Instant::now());
    let homes = model::homes(&answer);
    if homes.is_empty() {
        tracing::warn!(instance = %ctx.instance(), "no Netatmo Energy home in this account");
        house.homes.clear();
        return Ok(());
    }
    let mut shown = HashSet::from([ctx.device_id("account")]);
    for home in &homes {
        let rooms: Vec<DeviceId> = home
            .rooms
            .iter()
            .map(|r| ctx.device_id(&home.room_native(r)))
            .collect();
        for (room, id) in home.rooms.iter().zip(&rooms) {
            ctx.upsert_device(room_device(ctx, id, room));
            shown.insert(id.clone());
        }
        let id = ctx.device_id(&home.native());
        ctx.upsert_device(home_device(ctx, &id, home, rooms));
        ctx.set_availability(&id, true);
        ctx.set_state(
            &id,
            "schedule",
            home.active_schedule()
                .map_or(Value::Null, |s| Value::Text(s.name.as_str().into())),
        );
        ctx.provide_service(
            &id,
            Arc::new(Desk {
                home: home.id.clone(),
                asks: asks.clone(),
            }),
        );
        shown.insert(id);
        for module in &home.modules {
            let id = ctx.device_id(&module.native());
            let room = module.room.as_deref().and_then(|r| home.room_name(r));
            ctx.upsert_device(module_device(ctx, &id, module, room));
            shown.insert(id);
        }
        tracing::info!(
            instance = %ctx.instance(),
            home = %home.name,
            rooms = home.rooms.len(),
            modules = home.modules.len(),
            "netatmo home"
        );
    }
    for gone in ctx.devices().into_iter().filter(|d| !shown.contains(d)) {
        ctx.remove_device(&gone);
    }
    house.homes = homes;
    Ok(())
}

fn room_device(ctx: &DriverCtx, id: &DeviceId, room: &model::Room) -> Device {
    Device {
        id: id.clone(),
        instance: ctx.instance().clone(),
        native_name: room.name.as_str().into(),
        manufacturer: Some("Netatmo".into()),
        model: Some("Energy".into()),
        description: Some(moli_i18n::tr!("pilotes.netatmo.description_piece").into()),
        // The guard reads it: a protected room stays protected.
        native_room: Some(room.name.as_str().into()),
        members: Vec::new(),
        points: model::room_points(),
    }
}

fn home_device(ctx: &DriverCtx, id: &DeviceId, home: &Home, rooms: Vec<DeviceId>) -> Device {
    Device {
        id: id.clone(),
        instance: ctx.instance().clone(),
        native_name: home.name.as_str().into(),
        manufacturer: Some("Netatmo".into()),
        model: Some("Energy".into()),
        description: Some(moli_i18n::tr!("pilotes.netatmo.description_maison").into()),
        native_room: None,
        // The mode reaches every room: the guard checks them all.
        members: rooms,
        points: model::home_points(&home.schedule_names()),
    }
}

fn module_device(
    ctx: &DriverCtx,
    id: &DeviceId,
    module: &model::Module,
    room: Option<&str>,
) -> Device {
    let name = if module.name.is_empty() {
        module.model()
    } else {
        module.name.as_str()
    };
    Device {
        id: id.clone(),
        instance: ctx.instance().clone(),
        native_name: name.into(),
        manufacturer: Some("Netatmo".into()),
        model: Some(module.model().into()),
        description: None,
        native_room: room.map(Into::into),
        members: Vec::new(),
        points: model::module_points(module),
    }
}

async fn read_all(
    ctx: &DriverCtx,
    client: &mut Client<Https>,
    house: &House,
) -> Result<(), Failure> {
    for home in &house.homes {
        read_home(ctx, client, home).await?;
    }
    Ok(())
}

/// One home's state, onto its devices.
async fn read_home(
    ctx: &DriverCtx,
    client: &mut Client<Https>,
    home: &Home,
) -> Result<(), Failure> {
    let answer = client
        .call(
            &Store(ctx),
            Method::GET,
            &model::status_path(&home.id),
            None,
            &|| true,
        )
        .await?;
    let (rooms, modules) = model::status(&answer);
    for status in &rooms {
        let Some(room) = home.rooms.iter().find(|r| r.id == status.id) else {
            continue;
        };
        let id = ctx.device_id(&home.room_native(room));
        // A room with a thermostat and no valve: the boiler says if it heats.
        let boiler = home
            .modules
            .iter()
            .filter(|m| m.room.as_deref() == Some(room.id.as_str()) && m.kind == "NATherm1")
            .find_map(|m| modules.iter().find(|s| s.id == m.id))
            .and_then(|s| s.boiler);
        for (key, value) in model::room_values(status, boiler) {
            ctx.set_state(&id, key, value);
        }
        ctx.set_availability(&id, status.reachable.unwrap_or(true));
    }
    if let Some(mode) = model::home_mode(&rooms) {
        ctx.set_state(
            &ctx.device_id(&home.native()),
            "mode",
            Value::Text(mode.into()),
        );
    }
    for status in &modules {
        let Some(module) = home.modules.iter().find(|m| m.id == status.id) else {
            continue;
        };
        let id = ctx.device_id(&module.native());
        for (key, value) in model::module_values(status) {
            ctx.set_state(&id, key, value);
        }
        ctx.set_availability(&id, status.reachable.unwrap_or(true));
    }
    Ok(())
}

/// What a person reads when a schedule is refused before leaving.
fn wrong_schedule(wrong: &schedule::Wrong) -> String {
    use moli_i18n::tr;
    use schedule::Wrong;
    match wrong {
        Wrong::Shape(field) => tr!("pilotes.netatmo.planning_forme", field = field),
        Wrong::Name => tr!("pilotes.netatmo.planning_nom", max = schedule::MAX_NAME),
        Wrong::Temperature(what) => tr!(
            "pilotes.netatmo.planning_temperature",
            what = what,
            min = model::SETPOINT_MIN,
            max = model::SETPOINT_MAX
        ),
        Wrong::NoZone => tr!("pilotes.netatmo.planning_sans_zone"),
        Wrong::TooManyZones => tr!(
            "pilotes.netatmo.planning_trop_de_zones",
            max = schedule::MAX_ZONES
        ),
        Wrong::ZoneTwice(id) => tr!("pilotes.netatmo.planning_zone_double", id = id),
        Wrong::ZoneKind(kind) => tr!("pilotes.netatmo.planning_zone_type", kind = kind),
        Wrong::UnknownRoom(room) => tr!("pilotes.netatmo.planning_piece_inconnue", room = room),
        Wrong::RoomMissing(zone, room) => tr!(
            "pilotes.netatmo.planning_piece_manquante",
            zone = zone,
            room = room
        ),
        Wrong::NoSlot => tr!("pilotes.netatmo.planning_sans_creneau"),
        Wrong::TooManySlots => tr!(
            "pilotes.netatmo.planning_trop_de_creneaux",
            max = schedule::MAX_SLOTS
        ),
        Wrong::NotFromMonday => tr!("pilotes.netatmo.planning_lundi"),
        Wrong::SlotOrder(start) => tr!("pilotes.netatmo.planning_ordre", start = start),
        Wrong::UnknownZone(zone) => tr!("pilotes.netatmo.planning_zone_inconnue", zone = zone),
    }
}

/// A failure, for a person at the dashboard.
fn service_failure(failure: &Failure) -> ServiceError {
    match failure {
        Failure::Refused(_) => ServiceError::Refused(failure_text(failure)),
        other => ServiceError::Unavailable(failure_text(other)),
    }
}

/// The schedule a person names in `input["id"]`, in `home`.
fn schedule_id<'a>(home: &'a Home, input: &Json) -> Result<&'a str, ServiceError> {
    let id = input["id"].as_str().unwrap_or_default();
    home.schedules
        .iter()
        .find(|s| s.id == id)
        .map(|s| s.id.as_str())
        .ok_or_else(|| {
            ServiceError::Invalid(moli_i18n::tr!(
                "pilotes.netatmo.planning_inconnu",
                name = id
            ))
        })
}

/// The request a schedule ask becomes: its path and body, once checked.
fn schedule_request(
    home: &Home,
    request: &Request,
) -> Result<Option<(&'static str, Payload)>, ServiceError> {
    let heated: Vec<&str> = home.rooms.iter().map(|r| r.id.as_str()).collect();
    Ok(Some(match request {
        Request::Read => return Ok(None),
        Request::Sync(input) => {
            let id = schedule_id(home, input)?;
            let draft = Draft::parse(input, &home.all_rooms, &heated)
                .map_err(|w| ServiceError::Invalid(wrong_schedule(&w)))?;
            (
                "/api/synchomeschedule",
                Payload::Json(draft.body(&home.id, Some(id))),
            )
        }
        Request::Create(input) => {
            let draft = Draft::parse(input, &home.all_rooms, &heated)
                .map_err(|w| ServiceError::Invalid(wrong_schedule(&w)))?;
            (
                "/api/createnewhomeschedule",
                Payload::Json(draft.body(&home.id, None)),
            )
        }
        Request::Activate(input) => {
            let id = schedule_id(home, input)?;
            (
                "/api/switchhomeschedule",
                Payload::Form(schedule::switch_form(&home.id, id)),
            )
        }
    }))
}

/// One ask about a home's schedules: a read (from Netatmo again when the
/// last one is old), or one write, checked first, then `homesdata` again
/// so that the answer is what Netatmo now holds.
async fn serve(
    ctx: &DriverCtx,
    client: &mut Client<Https>,
    house: &mut House,
    asks: &mpsc::Sender<Ask>,
    ask: Ask,
) -> Result<(), Stop> {
    let Ask {
        home: home_id,
        request,
        reply,
    } = ask;
    let Some(home) = house.home(&home_id) else {
        let _ = reply.send(Err(ServiceError::UnknownDevice));
        return Ok(());
    };
    let sent = match schedule_request(home, &request) {
        Ok(Some((path, body))) => {
            let sent = client
                .call(&Store(ctx), Method::POST, path, Some(body), &|| true)
                .await;
            match sent {
                Ok(_) => {
                    tracing::info!(instance = %ctx.instance(), path, "netatmo schedule written");
                    Ok(true)
                }
                Err(f) => Err(f),
            }
        }
        Ok(None) => Ok(false),
        Err(e) => {
            let _ = reply.send(Err(e));
            return Ok(());
        }
    };
    let written = match sent {
        Ok(written) => written,
        Err(f) => {
            tracing::warn!(instance = %ctx.instance(), error = %f, "netatmo schedule not written");
            let _ = reply.send(Err(service_failure(&f)));
            fatal(f)?;
            return Ok(());
        }
    };
    let stale = house
        .fetched
        .is_none_or(|at| at.elapsed() > SCHEDULES_FRESH);
    if (written || stale)
        && let Err(f) = discover(ctx, client, house, asks).await
    {
        tracing::warn!(instance = %ctx.instance(), error = %f, "netatmo schedules after a change");
        if written {
            // Written, but what Netatmo holds now is unknown: say so.
            let _ = reply.send(Err(ServiceError::Unavailable(moli_i18n::tr!(
                "pilotes.netatmo.planning_relecture",
                why = failure_text(&f)
            ))));
            fatal(f)?;
            return Ok(());
        }
        fatal(f)?;
    }
    let _ = reply.send(
        house
            .home(&home_id)
            .map(Home::schedules_json)
            .ok_or(ServiceError::UnknownDevice),
    );
    Ok(())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// What the person reads when an order fails.
fn failure_text(failure: &Failure) -> String {
    use moli_i18n::tr;
    match failure {
        Failure::Late => tr!("pilotes.netatmo.trop_tard"),
        Failure::Quota(wait) => tr!(
            "pilotes.netatmo.quota",
            minutes = wait.as_secs().div_ceil(60)
        ),
        Failure::Refused(why) => tr!("pilotes.netatmo.refuse", why = why),
        Failure::Network(e) => tr!("pilotes.netatmo.injoignable", why = format!("{e:#}")),
        Failure::SignInAgain(_) => tr!("pilotes.netatmo.a_reconnecter"),
        Failure::BadApp(why) => tr!("pilotes.netatmo.app_refusee", why = why),
    }
}

/// One order, sent within its deadline or never, then the home read again.
async fn order(
    config: &Config,
    ctx: &DriverCtx,
    client: &mut Client<Https>,
    house: &mut House,
    asks: &mpsc::Sender<Ask>,
    command: CommandRequest,
) -> Result<(), Stop> {
    let Some(target) = house.target(ctx, &command.device.id) else {
        command.reply(Err(moli_i18n::tr!("pilotes.netatmo.inconnu")));
        return Ok(());
    };
    let home = house.home(target.home());
    let minutes = config.minutes(home);
    let request = model::order(
        &target,
        &command.key,
        &command.value,
        minutes,
        unix_now(),
        home,
    );
    let (path, form) = match request {
        Ok(request) => request,
        Err(wrong) => {
            command.reply(Err(match wrong {
                model::Wrong::OutOfBounds => moli_i18n::tr!(
                    "pilotes.netatmo.hors_bornes",
                    min = model::SETPOINT_MIN,
                    max = model::SETPOINT_MAX
                ),
                model::Wrong::RoomMode => moli_i18n::tr!("pilotes.netatmo.mode_piece"),
                model::Wrong::ReadOnly(key) => {
                    moli_i18n::tr!("pilotes.netatmo.lecture_seule", key = key)
                }
                model::Wrong::NoSuchSchedule(name) => {
                    moli_i18n::tr!("pilotes.netatmo.planning_inconnu", name = name)
                }
            }));
            return Ok(());
        }
    };
    let sent = client
        .call(
            &Store(ctx),
            Method::POST,
            path,
            Some(Payload::Form(form)),
            &|| !command.abandoned(),
        )
        .await;
    match sent {
        Ok(_) => {
            tracing::info!(instance = %ctx.instance(), point = %command.point, value = ?command.value, minutes, "netatmo order sent");
            let switched = &*command.key == "schedule";
            command.reply(Ok(()));
            // What Netatmo now says, at once (a switched schedule shows in
            // `homesdata`, the rest in `homestatus`).
            let read = if switched {
                discover(ctx, client, house, asks).await
            } else {
                match house.home(target.home()) {
                    Some(home) => read_home(ctx, client, home).await,
                    None => Ok(()),
                }
            };
            if let Err(f) = read {
                let f = fatal(f)?;
                tracing::warn!(instance = %ctx.instance(), error = %f, "netatmo state after an order");
            }
        }
        Err(f) => {
            tracing::warn!(instance = %ctx.instance(), point = %command.point, error = %f, "netatmo order not done");
            command.reply(Err(failure_text(&f)));
            fatal(f)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_config_has_safe_defaults() {
        let config: Config = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(config.redirect_uri, "http://localhost/moli-netatmo");
        assert_eq!(config.poll(), Duration::from_secs(300));
        assert_eq!(config.minutes(None), 180);
        let home = Home {
            id: "h".into(),
            name: "Maison".into(),
            rooms: Vec::new(),
            modules: Vec::new(),
            default_minutes: Some(120),
            all_rooms: Vec::new(),
            schedules: Vec::new(),
        };
        assert_eq!(config.minutes(Some(&home)), 120);
        let config: Config = serde_json::from_value(
            serde_json::json!({ "poll_seconds": 10, "setpoint_minutes": 60 }),
        )
        .unwrap();
        assert_eq!(config.poll(), Duration::from_secs(POLL_MIN));
        assert_eq!(config.minutes(Some(&home)), 60);
        let config: Config =
            serde_json::from_value(serde_json::json!({ "setpoint_minutes": 5000 })).unwrap();
        assert_eq!(config.minutes(None), 180);
        // The app's keys never go into moli.toml.
        assert!(
            serde_json::from_value::<Config>(serde_json::json!({ "client_secret": "x" })).is_err()
        );
    }

    #[test]
    fn a_schedule_ask_is_checked_before_it_becomes_a_request() {
        let answer: Json = serde_json::from_str(include_str!("fixtures/homesdata.json")).unwrap();
        let homes = model::homes(&answer);
        let home = &homes[0];
        assert!(schedule_request(home, &Request::Read).unwrap().is_none());
        let (path, body) = schedule_request(
            home,
            &Request::Activate(serde_json::json!({ "id": "sched-2" })),
        )
        .unwrap()
        .unwrap();
        assert_eq!(path, "/api/switchhomeschedule");
        assert_eq!(
            body,
            Payload::Form("home_id=home-1&schedule_id=sched-2".into())
        );
        assert!(matches!(
            schedule_request(
                home,
                &Request::Activate(serde_json::json!({ "id": "nope" }))
            ),
            Err(ServiceError::Invalid(_))
        ));
        // A schedule sent back as read, edited: the body Netatmo reads.
        let mut edited = home.schedules[0].to_json();
        edited["name"] = serde_json::json!("Semaine 2");
        edited["zones"][0]["rooms"][2]["temp"] = serde_json::json!(21);
        let (path, body) = schedule_request(home, &Request::Sync(edited.clone()))
            .unwrap()
            .unwrap();
        assert_eq!(path, "/api/synchomeschedule");
        let Payload::Json(text) = body else {
            panic!("a JSON body")
        };
        let sent: Json = serde_json::from_str(&text).unwrap();
        assert_eq!(sent["schedule_id"], "sched-1");
        assert_eq!(sent["name"], "Semaine 2");
        assert_eq!(
            sent["zones"][0]["rooms"][2]["therm_setpoint_temperature"],
            21.0
        );
        assert_eq!(sent["timetable"].as_array().unwrap().len(), 23);
        let (path, body) = schedule_request(home, &Request::Create(edited.clone()))
            .unwrap()
            .unwrap();
        assert_eq!(path, "/api/createnewhomeschedule");
        assert!(!body.text().contains("schedule_id"));
        edited["zones"][1]["rooms"][0]["temp"] = serde_json::json!(6);
        let refused = schedule_request(home, &Request::Sync(edited)).unwrap_err();
        assert!(
            matches!(refused, ServiceError::Invalid(ref why) if why.contains("Nuit")),
            "{refused}"
        );
    }

    #[test]
    fn failures_are_said_to_the_person() {
        assert!(!failure_text(&Failure::Late).is_empty());
        assert!(!failure_text(&Failure::Quota(Duration::from_secs(61))).is_empty());
    }
}
