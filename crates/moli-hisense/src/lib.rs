//! Hisense VIDAA TVs: the VIDAA app's remote protocol, MQTT over TLS on
//! the TV itself (port 36669). The TV's certificate is pinned at first
//! contact (secret `pin`); recent TVs also want the app's client
//! certificate, given as secrets `client_cert` / `client_key` (PEM, never
//! in the repository: it is the app's).
//!
//! Pairing is built in: the TV shows a 4-digit code, typed at the
//! dashboard (`pairing_code`, « secret »). Newer firmware then hands out
//! tokens (kept in secret `session`, with the client id and user name the
//! TV tied them to); older firmware authorizes the client id itself.
//! Which login the TV wants (`credentials.rs`) is read from its UPnP
//! description, or forced by the `auth` option.
//!
//! Power is a toggle key, pressed only when the state known differs. In
//! standby the broker still answers (`fake_sleep_0`); in deep standby it
//! does not: the TV is offline, and comes back by Wake-on-LAN (option
//! `mac`).

mod credentials;
mod protocol;

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use moli_core::{Access, Device, DeviceId, Kind, PointSpec, Semantic, Value};
use moli_net::PinnedTls;
use moli_runtime::{BoxFuture, CommandRequest, Driver, DriverCtx};
use rumqttc::{
    AsyncClient, ConnectReturnCode, ConnectionError, Event, EventLoop, MqttOptions, Packet, QoS,
    TlsConfiguration, Transport,
};
use serde::Deserialize;
use tokio::time::Instant;

pub use credentials::Auth;
use credentials::{Credentials, Login, Session};
use protocol::{App, Message, Source, State};

const DEFAULT_PORT: u16 = 36669;
const TICK: Duration = Duration::from_secs(5);
/// `gettvstate` this often (some firmware only tells a change).
const POLL: Duration = Duration::from_secs(30);
/// Unpaired: asked again this often for a code, until the TV showed one.
const ASK_AGAIN: Duration = Duration::from_secs(30);
/// A code asked for, nothing back: asked again later.
const WAITING_FOR: Duration = Duration::from_secs(15);
/// The code taken, no tokens yet.
const TOKENS_FOR: Duration = Duration::from_secs(10);
/// The code typed, no answer.
const CODE_FOR: Duration = Duration::from_secs(10);
/// After Wake-on-LAN: a TV back in standby is switched on.
const WAKE_FOR: Duration = Duration::from_secs(180);
/// The TV off or gone: tried again after this.
const OFF_PAUSE: Duration = Duration::from_secs(30);
/// Every login refused: tried again after this.
const REFUSED_PAUSE: Duration = Duration::from_secs(300);
const KEEP_ALIVE: Duration = Duration::from_secs(20);

const PIN: &str = "pin";
const CLIENT_CERT: &str = "client_cert";
const CLIENT_KEY: &str = "client_key";
const CLIENT_MAC: &str = "client_mac";
const SESSION: &str = "session";

/// `pairing`: `paired`, `code` (on screen, to type), `needed` (ask for a
/// code: write `code`), `waiting` (asking the TV, or for its tokens).
pub const STATES: [&str; 4] = ["paired", "code", "needed", "waiting"];

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    /// The TV's, for Wake-on-LAN from deep standby.
    #[serde(default)]
    pub mac: Option<String>,
    /// The device's name (default: « Télé Hisense »).
    #[serde(default)]
    pub name: Option<String>,
    /// `auto` (from the TV's description), `static`, `legacy`, `middle`,
    /// `modern`: which login the TV takes.
    #[serde(default)]
    pub auth: Option<String>,
}

fn default_port() -> u16 {
    DEFAULT_PORT
}

#[derive(Debug)]
pub struct Hisense {
    config: Config,
}

impl Hisense {
    #[must_use]
    pub fn new(config: Config) -> Self {
        Self { config }
    }
}

impl Driver for Hisense {
    fn kind(&self) -> &'static str {
        "hisense"
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

fn enumeration<'a>(values: impl Iterator<Item = &'a str>) -> Kind {
    Kind::Enum {
        values: values.map(Into::into).collect(),
    }
}

fn device(ctx: &DriverCtx, id: &DeviceId, name: &str, sources: &[Source]) -> Device {
    let tr = |key: &str| moli_i18n::tr!(key);
    Device {
        id: id.clone(),
        instance: ctx.instance().clone(),
        native_name: name.into(),
        manufacturer: Some("Hisense".into()),
        model: Some("VIDAA".into()),
        description: None,
        native_room: None,
        members: Vec::new(),
        points: vec![
            spec(
                "power",
                &tr("pilotes.hisense.allumee"),
                Kind::Binary,
                RW,
                Semantic::OnOff,
            ),
            spec(
                "volume",
                &tr("pilotes.hisense.volume"),
                Kind::Numeric {
                    min: Some(0.0),
                    max: Some(100.0),
                    step: Some(1.0),
                },
                RW,
                Semantic::infer("volume", None),
            ),
            spec(
                "mute",
                &tr("pilotes.hisense.sourdine"),
                Kind::Binary,
                RW,
                Semantic::infer("mute", None),
            ),
            // The inputs, as the TV lists them (empty until it did).
            spec(
                "source",
                &tr("pilotes.hisense.source"),
                enumeration(sources.iter().map(Source::label)),
                RW,
                Semantic::Other,
            ),
            // Written: opens that app (its name, or the Android package the
            // living room sends).
            spec(
                "app",
                &tr("pilotes.hisense.application"),
                Kind::Text,
                RW,
                Semantic::Other,
            ),
            spec(
                "screen",
                &tr("pilotes.hisense.a_l_ecran"),
                Kind::Text,
                R,
                Semantic::Other,
            ),
            spec(
                "key",
                &tr("pilotes.hisense.touche"),
                enumeration(protocol::key_names()),
                W,
                Semantic::Other,
            ),
            // Written `code`: the TV shows a new code.
            spec(
                "pairing",
                &tr("pilotes.hisense.appairage_etat"),
                enumeration(STATES.into_iter()),
                RW,
                Semantic::Config,
            ),
            // The 4 digits on the TV (a number: the dashboard reads the
            // size from `max`): never journaled nor kept.
            spec(
                "pairing_code",
                &tr("pilotes.hisense.code"),
                Kind::Numeric {
                    min: Some(0.0),
                    max: Some(9999.0),
                    step: Some(1.0),
                },
                W,
                Semantic::Secret,
            ),
        ],
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pairing {
    Paired,
    Needed,
    /// A code asked for.
    Waiting,
    /// On screen.
    Code,
    /// The code taken, the tokens asked for.
    Tokens,
}

/// Why a connection ended, and what next.
enum Outcome {
    /// Right away (another login, a renewed one).
    Reconnect,
    Pause(Duration),
    /// A person is needed: the driver stops trying.
    Stop,
}

/// A refusal of the login (CONNACK 4 or 5).
fn refused(error: &ConnectionError) -> bool {
    matches!(
        error,
        ConnectionError::ConnectionRefused(
            ConnectReturnCode::BadUserNamePassword | ConnectReturnCode::NotAuthorized
        )
    )
}

/// The TV's description, where it says which login it takes.
async fn describe(host: &str) -> Option<u32> {
    for port in credentials::DESCRIPTION_PORTS {
        match moli_net::upnp::fetch(host, port, credentials::DESCRIPTION_PATH).await {
            Ok(xml) => {
                let version = credentials::transport_protocol(&xml);
                tracing::debug!(host, port, ?version, "hisense tv description");
                return version;
            }
            Err(e) => tracing::debug!(host, port, error = %format!("{e:#}"), "no tv description"),
        }
    }
    None
}

fn send(client: &AsyncClient, order: (String, String)) -> Result<(), String> {
    client
        .try_publish(order.0, QoS::AtMostOnce, false, order.1)
        .map_err(|e| e.to_string())
}

struct Tv<'a> {
    config: &'a Config,
    forced: Option<Auth>,
    id: DeviceId,
    name: String,
    client_mac: String,
    session: Option<Session>,
    transport_protocol: Option<u32>,
    /// Unpaired: which login of the order is being tried.
    attempt: usize,
    /// The login of the connection at hand.
    current: Option<(Auth, Credentials)>,
    /// The refresh token to renew with, once connected.
    renew: Option<String>,
    tls: Option<PinnedTls>,
    connected: bool,
    pairing: Pairing,
    since: Instant,
    /// Asks for a code by itself (unpaired, until the TV showed one).
    auto: bool,
    asked: Option<Instant>,
    /// The person's code, answered when the TV does.
    code_order: Option<(CommandRequest, Instant)>,
    on: Option<bool>,
    muted: Option<bool>,
    sources: Vec<Source>,
    apps: Vec<App>,
    woken: Option<Instant>,
    polled: Instant,
    last_error: Option<String>,
    online: Option<bool>,
}

impl<'a> Tv<'a> {
    fn new(
        config: &'a Config,
        forced: Option<Auth>,
        id: DeviceId,
        name: String,
        client_mac: String,
        session: Option<Session>,
    ) -> Self {
        let paired = session.is_some();
        Self {
            config,
            forced,
            id,
            name,
            client_mac,
            transport_protocol: session.as_ref().and_then(|s| s.transport_protocol),
            session,
            attempt: 0,
            current: None,
            renew: None,
            tls: None,
            connected: false,
            pairing: if paired {
                Pairing::Paired
            } else {
                Pairing::Needed
            },
            since: Instant::now(),
            auto: !paired,
            asked: None,
            code_order: None,
            on: None,
            muted: None,
            sources: Vec::new(),
            apps: Vec::new(),
            woken: None,
            polled: Instant::now(),
            last_error: None,
            online: None,
        }
    }

    fn auth(&self) -> Option<Auth> {
        self.current.as_ref().map(|(a, _)| *a)
    }

    fn client_id(&self) -> &str {
        self.current
            .as_ref()
            .map_or("", |(_, c)| c.client_id.as_str())
    }

    fn paired(&self) -> bool {
        self.pairing == Pairing::Paired
    }

    /// Says where the pairing is (the point, and the driver's status).
    fn show(&self, ctx: &DriverCtx) {
        let word = match self.pairing {
            Pairing::Paired => "paired",
            Pairing::Code => "code",
            Pairing::Needed => "needed",
            Pairing::Waiting | Pairing::Tokens => "waiting",
        };
        ctx.set_state(&self.id, "pairing", Value::Text(word.into()));
        let instance = ctx.instance();
        match self.pairing {
            Pairing::Code => ctx.wait_for(moli_i18n::tr!(
                "pilotes.hisense.taper_le_code",
                instance = instance
            )),
            Pairing::Paired => ctx.ready(),
            _ => ctx.wait_for(moli_i18n::tr!(
                "pilotes.hisense.a_appairer",
                instance = instance
            )),
        }
    }

    /// Unpaired, the TV is there to be paired; paired, it is there when its
    /// broker answers.
    fn presence(&mut self, ctx: &DriverCtx) {
        let now = self.connected || !self.paired();
        if self.online != Some(now) {
            self.online = Some(now);
            ctx.set_availability(&self.id, now);
        }
    }

    /// The MQTT client for one connection: the login kept, renewed, or the
    /// next one to try.
    async fn connection(&mut self, ctx: &DriverCtx) -> anyhow::Result<(AsyncClient, EventLoop)> {
        let at = now();
        self.renew = None;
        let kept = match &self.session {
            Some(s) => {
                let login = |password: String| Credentials {
                    client_id: s.client_id.clone(),
                    username: s.username.clone(),
                    password,
                };
                match s.login(at) {
                    Login::Access(p) | Login::Password(p) => Some((s.auth, login(p))),
                    Login::Refresh(r) => {
                        self.renew = Some(r.clone());
                        Some((s.auth, login(r)))
                    }
                    Login::Expired => None,
                }
            }
            None => None,
        };
        let (auth, creds) = if let Some((auth, creds)) = kept {
            (auth.unwrap_or(Auth::Modern), creds)
        } else {
            if self.session.is_some() {
                tracing::info!(instance = %ctx.instance(), "hisense tv: the session lapsed, pairing again");
                self.forget_session(ctx).await;
                self.show(ctx);
            }
            if self.transport_protocol.is_none() {
                self.transport_protocol = describe(&self.config.host).await;
            }
            let order = self.logins();
            let auth = order[self.attempt % order.len()];
            let static_id = format!("moli_{}", ctx.instance());
            (
                auth,
                credentials::credentials(auth, &self.client_mac, at, &static_id),
            )
        };
        let pin = ctx.secret(PIN);
        let tls = match (ctx.secret(CLIENT_CERT), ctx.secret(CLIENT_KEY)) {
            (Some(cert), Some(key)) => {
                PinnedTls::with_client_cert(pin, &cert, &key).map_err(|e| {
                    ctx.wait_for(moli_i18n::tr!(
                        "pilotes.hisense.certificat_client_illisible",
                        error = format!("{e:#}"),
                        instance = ctx.instance()
                    ));
                    e
                })?
            }
            _ => PinnedTls::new(pin)?,
        };
        let mut options =
            MqttOptions::new(creds.client_id.clone(), &self.config.host, self.config.port);
        options.set_credentials(creds.username.clone(), creds.password.clone());
        options.set_keep_alive(KEEP_ALIVE);
        options.set_max_packet_size(256 * 1024, 16 * 1024);
        options.set_transport(Transport::tls_with_config(TlsConfiguration::Rustls(
            Arc::clone(&tls.config),
        )));
        tracing::debug!(instance = %ctx.instance(), auth = auth.name(), paired = self.paired(), "hisense tv: connecting");
        self.tls = Some(tls);
        self.current = Some((auth, creds));
        Ok(AsyncClient::new(options, 32))
    }

    /// The logins to try, unpaired.
    fn logins(&self) -> Vec<Auth> {
        self.forced.map_or_else(
            || Auth::order(self.transport_protocol).to_vec(),
            |a| vec![a],
        )
    }

    async fn keep_pin(&self, ctx: &DriverCtx) {
        if ctx.secret(PIN).is_some() {
            return;
        }
        if let Some(seen) = self.tls.as_ref().and_then(PinnedTls::seen_fingerprint) {
            match ctx.store_secret(PIN, &seen).await {
                Ok(()) => {
                    tracing::info!(instance = %ctx.instance(), "hisense tv certificate pinned");
                }
                Err(e) => {
                    tracing::warn!(instance = %ctx.instance(), error = %e, "tv certificate not pinned");
                }
            }
        }
    }

    async fn forget_session(&mut self, ctx: &DriverCtx) {
        self.session = None;
        if let Err(e) = ctx.forget_secret(SESSION).await {
            tracing::warn!(instance = %ctx.instance(), error = %e, "hisense session not forgotten");
        }
        self.pairing = Pairing::Needed;
        self.auto = true;
        self.asked = None;
    }

    async fn keep_session(&mut self, ctx: &DriverCtx, session: Session) {
        match serde_json::to_string(&session) {
            Ok(json) => {
                if let Err(e) = ctx.store_secret(SESSION, &json).await {
                    tracing::warn!(instance = %ctx.instance(), error = %e, "hisense session not kept");
                }
            }
            Err(e) => {
                tracing::warn!(instance = %ctx.instance(), error = %e, "hisense session unwritable");
            }
        }
        self.session = Some(session);
    }

    /// What the TV is doing, its volume, its inputs and apps.
    fn poll(&mut self, client: &AsyncClient) {
        let id = self.client_id().to_owned();
        for (service, action) in [
            ("ui_service", "gettvstate"),
            ("platform_service", "getvolume"),
            ("ui_service", "sourcelist"),
            ("ui_service", "applist"),
        ] {
            let _ = send(
                client,
                (protocol::action(&id, service, action), String::new()),
            );
        }
        self.polled = Instant::now();
    }

    /// Asks the TV to show a code: its action on newer firmware; any
    /// request from an unauthorized client on the old one.
    fn ask_code(&mut self, ctx: &DriverCtx, client: &AsyncClient) {
        let id = self.client_id().to_owned();
        let order = if self.auth().is_some_and(Auth::issues_tokens) {
            protocol::connect(&id)
        } else {
            (
                protocol::action(&id, "ui_service", "gettvstate"),
                String::new(),
            )
        };
        let _ = send(client, order);
        self.pairing = Pairing::Waiting;
        self.since = Instant::now();
        self.asked = Some(Instant::now());
        self.show(ctx);
    }

    async fn on_connected(&mut self, ctx: &DriverCtx, client: &AsyncClient) {
        self.connected = true;
        self.last_error = None;
        self.keep_pin(ctx).await;
        for topic in protocol::listened(self.client_id()) {
            if let Err(e) = client.try_subscribe(topic, QoS::AtMostOnce) {
                tracing::warn!(instance = %ctx.instance(), error = %e, "tv subscription not sent");
            }
        }
        if self.paired() {
            self.attempt = 0;
            if let Some(refresh) = self.renew.take() {
                tracing::info!(instance = %ctx.instance(), "hisense tv: renewing the tokens");
                let _ = send(client, protocol::get_token(self.client_id(), &refresh));
            }
            self.poll(client);
            ctx.ready();
        } else if self.auto {
            self.ask_code(ctx, client);
        } else {
            self.show(ctx);
        }
    }

    /// The pairing done on firmware that gives no tokens: the TV
    /// authorized this client id with this login.
    async fn paired_without_tokens(&mut self, ctx: &DriverCtx, client: &AsyncClient) {
        let Some((auth, creds)) = self.current.clone() else {
            return;
        };
        let _ = send(client, protocol::close_code(&creds.client_id));
        self.keep_session(
            ctx,
            Session {
                auth: Some(auth),
                transport_protocol: self.transport_protocol,
                client_id: creds.client_id,
                username: creds.username,
                password: Some(creds.password),
                ..Session::default()
            },
        )
        .await;
        tracing::info!(instance = %ctx.instance(), auth = auth.name(), "hisense tv paired (no tokens)");
        self.now_paired(ctx, client);
    }

    fn now_paired(&mut self, ctx: &DriverCtx, client: &AsyncClient) {
        self.pairing = Pairing::Paired;
        self.auto = false;
        self.attempt = 0;
        self.show(ctx);
        self.poll(client);
    }

    async fn on_message(&mut self, message: Message, ctx: &DriverCtx, client: &AsyncClient) {
        match message {
            Message::State(state) => {
                if state.asks_code() {
                    self.code_on_screen(ctx).await;
                    return;
                }
                if self.pairing == Pairing::Waiting && !self.auth().is_some_and(Auth::issues_tokens)
                {
                    // The old firmware answered an unauthorized client: it
                    // is authorized after all.
                    self.paired_without_tokens(ctx, client).await;
                }
                self.apply(ctx, client, &state);
            }
            Message::Sleep => {
                self.on = Some(false);
                ctx.set_state(&self.id, "power", Value::Bool(false));
            }
            Message::Volume { level, muted } => {
                if let Some(level) = level {
                    ctx.set_state(&self.id, "volume", Value::Int(i64::from(level)));
                }
                if let Some(muted) = muted {
                    self.muted = Some(muted);
                    ctx.set_state(&self.id, "mute", Value::Bool(muted));
                }
            }
            Message::Sources(sources) => {
                if sources != self.sources {
                    self.sources = sources;
                    ctx.upsert_device(device(ctx, &self.id, &self.name, &self.sources));
                }
            }
            Message::Apps(apps) => self.apps = apps,
            Message::CodeShown => self.code_on_screen(ctx).await,
            Message::Connected(result) => {
                tracing::debug!(instance = %ctx.instance(), ?result, "hisense tv took the pairing request");
                if self.pairing == Pairing::Waiting {
                    self.auto = false;
                    self.pairing = Pairing::Code;
                    self.show(ctx);
                }
            }
            Message::CodeAnswer(result) => self.on_code_answer(ctx, client, result).await,
            Message::Tokens(tokens) => {
                let Some((auth, creds)) = self.current.clone() else {
                    return;
                };
                let mut session = self.session.clone().unwrap_or_else(|| Session {
                    auth: Some(auth),
                    transport_protocol: self.transport_protocol,
                    client_id: creds.client_id,
                    username: creds.username,
                    ..Session::default()
                });
                session.take(&tokens, now());
                self.keep_session(ctx, session).await;
                if self.paired() {
                    tracing::info!(instance = %ctx.instance(), "hisense tv tokens renewed");
                } else {
                    tracing::info!(instance = %ctx.instance(), auth = auth.name(), "hisense tv paired");
                    self.now_paired(ctx, client);
                }
            }
        }
    }

    /// The TV shows a code: to type (a pairing believed done is gone).
    async fn code_on_screen(&mut self, ctx: &DriverCtx) {
        if self.paired() {
            tracing::warn!(instance = %ctx.instance(), "hisense tv asks for a code again: the pairing is gone");
            self.forget_session(ctx).await;
        }
        self.auto = false;
        if self.pairing != Pairing::Tokens {
            self.pairing = Pairing::Code;
        }
        self.show(ctx);
    }

    async fn on_code_answer(
        &mut self,
        ctx: &DriverCtx,
        client: &AsyncClient,
        result: Result<(), String>,
    ) {
        let order = self.code_order.take().map(|(o, _)| o);
        match result {
            Ok(()) => {
                if let Some(order) = order {
                    order.reply(Ok(()));
                }
                if self.pairing != Pairing::Code {
                    return;
                }
                if self.auth().is_some_and(Auth::issues_tokens) {
                    let id = self.client_id().to_owned();
                    let _ = send(client, protocol::get_token(&id, ""));
                    let _ = send(client, protocol::close_code(&id));
                    self.pairing = Pairing::Tokens;
                    self.since = Instant::now();
                    self.show(ctx);
                } else {
                    self.paired_without_tokens(ctx, client).await;
                }
            }
            Err(info) => {
                tracing::info!(instance = %ctx.instance(), info, "hisense tv refused the code");
                if let Some(order) = order {
                    order.reply(Err(moli_i18n::tr!(
                        "pilotes.hisense.code_faux",
                        info = info
                    )));
                }
            }
        }
    }

    /// What the TV is doing, into its points.
    fn apply(&mut self, ctx: &DriverCtx, client: &AsyncClient, state: &State) {
        let on = !state.off();
        self.on = Some(on);
        ctx.set_state(&self.id, "power", Value::Bool(on));
        let text = |v: &Option<String>| v.as_deref().map_or(Value::Null, |s| Value::Text(s.into()));
        ctx.set_state(&self.id, "screen", text(&state.screen));
        // The input as the enumeration names it (what the TV shows).
        let source = state
            .source_id
            .as_ref()
            .and_then(|id| self.sources.iter().find(|s| s.id == *id))
            .map(|s| s.label().to_owned())
            .or_else(|| state.source.clone());
        ctx.set_state(&self.id, "source", text(&source));
        ctx.set_state(&self.id, "app", text(&state.app));
        if let Some(woken) = self.woken {
            if woken.elapsed() > WAKE_FOR {
                self.woken = None;
            } else if !on {
                // Back from deep standby into standby: switched on.
                tracing::info!(instance = %ctx.instance(), "hisense tv woken: switching it on");
                let _ = send(
                    client,
                    protocol::press(self.client_id(), protocol::KEY_POWER),
                );
                self.woken = None;
            } else {
                self.woken = None;
            }
        }
    }

    fn tick(&mut self, ctx: &DriverCtx, client: &AsyncClient) {
        if let Some((_, at)) = &self.code_order
            && at.elapsed() > CODE_FOR
            && let Some((order, _)) = self.code_order.take()
        {
            order.reply(Err(moli_i18n::tr!("pilotes.hisense.sans_reponse")));
        }
        if !self.connected {
            return;
        }
        match self.pairing {
            Pairing::Paired => {
                if self.polled.elapsed() >= POLL {
                    let id = self.client_id().to_owned();
                    let _ = send(
                        client,
                        (
                            protocol::action(&id, "ui_service", "gettvstate"),
                            String::new(),
                        ),
                    );
                    self.polled = Instant::now();
                }
            }
            Pairing::Needed => {
                if self.auto && self.asked.is_none_or(|at| at.elapsed() >= ASK_AGAIN) {
                    self.ask_code(ctx, client);
                }
            }
            Pairing::Waiting => {
                if self.since.elapsed() > WAITING_FOR {
                    tracing::debug!(instance = %ctx.instance(), "hisense tv: no answer to the pairing request");
                    self.pairing = Pairing::Needed;
                    self.show(ctx);
                }
            }
            Pairing::Tokens => {
                if self.since.elapsed() > TOKENS_FOR {
                    tracing::warn!(instance = %ctx.instance(), "hisense tv took the code but gave no tokens");
                    self.pairing = Pairing::Needed;
                    self.show(ctx);
                }
            }
            Pairing::Code => {}
        }
    }

    /// The connection ended: why, and what next.
    async fn on_error(&mut self, ctx: &DriverCtx, error: &ConnectionError) -> Outcome {
        self.connected = false;
        if let Some((order, _)) = self.code_order.take() {
            order.reply(Err(moli_i18n::tr!("pilotes.hisense.injoignable")));
        }
        let instance = ctx.instance();
        let text = format!("{error:#}");
        if text.contains(moli_net::PIN_MISMATCH) {
            ctx.wait_for(moli_i18n::tr!(
                "pilotes.hisense.certificat_change",
                instance = instance,
                key = PIN
            ));
            return Outcome::Stop;
        }
        if refused(error) {
            self.keep_pin(ctx).await;
            if let Some(session) = self.session.clone() {
                if self.renew.is_none() && session.refresh_token.is_some() {
                    // The access token refused before its time: renewed.
                    tracing::info!(%instance, "hisense tv refused the access token: renewing");
                    let mut session = session;
                    session.access_until = Some(0);
                    self.keep_session(ctx, session).await;
                } else {
                    // The TV forgot the pairing (deep standby wipes the
                    // tokens on some firmware): paired again.
                    tracing::warn!(%instance, "hisense tv refused the session: pairing again");
                    self.forget_session(ctx).await;
                    self.show(ctx);
                }
                return Outcome::Reconnect;
            }
            let order = self.logins();
            self.attempt += 1;
            if self.attempt < order.len() {
                tracing::info!(%instance, next = order[self.attempt].name(), "hisense tv refused the login: trying the next one");
                return Outcome::Reconnect;
            }
            self.attempt = 0;
            ctx.wait_for(moli_i18n::tr!(
                "pilotes.hisense.identifiants_refuses",
                instance = instance,
                tried = order
                    .iter()
                    .map(|a| a.name())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            return Outcome::Pause(REFUSED_PAUSE);
        }
        // Off, or gone: deep standby closes the broker.
        if self.paired() && self.on != Some(false) {
            self.on = Some(false);
            ctx.set_state(&self.id, "power", Value::Bool(false));
        }
        if self.last_error.as_deref() == Some(&text) {
            tracing::debug!(%instance, error = text, "hisense tv unreachable");
        } else {
            tracing::info!(%instance, error = text, "hisense tv unreachable");
            self.last_error = Some(text);
        }
        Outcome::Pause(OFF_PAUSE)
    }

    /// An order: the pairing's, or the TV's.
    async fn order(
        &mut self,
        command: CommandRequest,
        ctx: &DriverCtx,
        client: Option<&AsyncClient>,
    ) {
        let live = client.filter(|_| self.connected);
        let result = match (&*command.key, &command.value) {
            ("pairing", Value::Text(word)) if &**word == "code" => match live {
                Some(client) => {
                    self.ask_code(ctx, client);
                    Ok(())
                }
                None => Err(moli_i18n::tr!("pilotes.hisense.injoignable")),
            },
            ("pairing", _) => Err(moli_i18n::tr!("pilotes.hisense.code_mal_forme")),
            ("pairing_code", value) => {
                let Some(code) = value.as_f64().filter(|n| (0.0..=9999.0).contains(n)) else {
                    command.reply(Err(moli_i18n::tr!("pilotes.hisense.code_mal_forme")));
                    return;
                };
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let code = code.round() as u32;
                let Some(client) = live else {
                    command.reply(Err(moli_i18n::tr!("pilotes.hisense.injoignable")));
                    return;
                };
                if !matches!(
                    self.pairing,
                    Pairing::Code | Pairing::Needed | Pairing::Waiting
                ) {
                    command.reply(Err(moli_i18n::tr!("pilotes.hisense.pas_de_code")));
                    return;
                }
                if let Some((old, _)) = self.code_order.take() {
                    old.reply(Err(moli_i18n::tr!("pilotes.hisense.sans_reponse")));
                }
                self.pairing = Pairing::Code;
                let sent = send(client, protocol::code(self.client_id(), code));
                match sent {
                    Ok(()) => self.code_order = Some((command, Instant::now())),
                    Err(e) => command.reply(Err(e)),
                }
                return;
            }
            ("power", Value::Bool(true)) if live.is_none() => self.wake().await,
            _ if live.is_none() => Err(moli_i18n::tr!("pilotes.hisense.injoignable")),
            _ if !self.paired() => Err(moli_i18n::tr!("pilotes.hisense.appairage")),
            (key, value) => {
                let client = live.unwrap_or_else(|| unreachable!("live checked above"));
                self.apply_order(client, key, value)
            }
        };
        command.reply(result);
    }

    fn apply_order(&self, client: &AsyncClient, key: &str, value: &Value) -> Result<(), String> {
        let id = self.client_id();
        match (key, value) {
            // POWER toggles: pressed only when it changes something.
            ("power", Value::Bool(wanted)) => {
                if self.on == Some(*wanted) {
                    Ok(())
                } else {
                    send(client, protocol::press(id, protocol::KEY_POWER))
                }
            }
            ("volume", v) => {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let level = v.as_f64().unwrap_or(0.0).round().clamp(0.0, 100.0) as u8;
                send(client, protocol::set_volume(id, level))
            }
            ("mute", Value::Bool(wanted)) => {
                if self.muted == Some(*wanted) {
                    Ok(())
                } else {
                    send(client, protocol::press(id, protocol::KEY_MUTE))
                }
            }
            ("source", Value::Text(wanted)) => {
                let wanted = wanted.trim();
                match self
                    .sources
                    .iter()
                    .find(|s| s.label() == wanted || s.name == wanted || s.id == wanted)
                {
                    Some(source) => send(client, protocol::change_source(id, source)),
                    None => Err(moli_i18n::tr!(
                        "pilotes.hisense.source_inconnue",
                        source = wanted
                    )),
                }
            }
            ("app", Value::Text(wanted)) => match protocol::find_app(wanted, &self.apps) {
                Some(app) => send(client, protocol::launch(id, &app)),
                None => Err(moli_i18n::tr!("pilotes.hisense.quelle_appli", app = wanted)),
            },
            ("key", Value::Text(name)) => match protocol::key_of(name) {
                Some(key) => send(client, protocol::press(id, key)),
                None => Err(moli_i18n::tr!(
                    "pilotes.hisense.touche_inconnue",
                    key = name
                )),
            },
            (key, _) => Err(moli_i18n::tr!(
                "pilotes.hisense.ne_se_regle_pas",
                point = key
            )),
        }
    }

    /// The broker is down: the TV sleeps deeply (or is gone). Wake-on-LAN,
    /// then `KEY_POWER` if it comes back in standby.
    async fn wake(&mut self) -> Result<(), String> {
        let Some(mac) = &self.config.mac else {
            return Err(moli_i18n::tr!("pilotes.hisense.sans_mac"));
        };
        moli_net::wol::wake(mac, Some(&self.config.host))
            .await
            .map_err(|e| moli_i18n::tr!("pilotes.hisense.reveil", error = format!("{e:#}")))?;
        self.woken = Some(Instant::now());
        Ok(())
    }

    /// Waits `delay`, answering orders (a wake-up) meanwhile. `false`: the
    /// driver is stopping.
    async fn pause(&mut self, ctx: &mut DriverCtx, delay: Duration) -> bool {
        let wake = tokio::time::sleep(delay);
        tokio::pin!(wake);
        loop {
            tokio::select! {
                () = &mut wake => return true,
                command = ctx.next_command() => match command {
                    Some(command) => self.order(command, ctx, None).await,
                    None => return false,
                },
            }
        }
    }
}

async fn run(config: &Config, ctx: &mut DriverCtx) -> anyhow::Result<()> {
    let forced = match Auth::parse(config.auth.as_deref().unwrap_or("")) {
        Ok(forced) => forced,
        Err(word) => {
            ctx.wait_for(moli_i18n::tr!("pilotes.hisense.auth_inconnue", word = word));
            ctx.cancelled().await;
            return Ok(());
        }
    };
    let id = ctx.device_id(&config.host);
    let name = config
        .name
        .clone()
        .unwrap_or_else(|| moli_i18n::tr!("pilotes.hisense.tele"));
    ctx.upsert_device(device(ctx, &id, &name, &[]));
    ctx.set_availability(&id, false);
    // The pin, the made-up MAC and the pairing live in the store.
    if !ctx.can_store_secrets() {
        ctx.wait_for(moli_i18n::tr!(
            "pilotes.hisense.coffre",
            why = ctx.secrets_problem().unwrap_or("?")
        ));
        ctx.cancelled().await;
        return Ok(());
    }
    let client_mac = if let Some(mac) = ctx.secret(CLIENT_MAC) {
        mac
    } else {
        let mac = credentials::random_mac()?;
        ctx.store_secret(CLIENT_MAC, &mac).await?;
        mac
    };
    let session: Option<Session> = ctx
        .secret(SESSION)
        .and_then(|json| serde_json::from_str(&json).ok());
    let mut tv = Tv::new(config, forced, id, name, client_mac, session);
    tv.show(ctx);
    tv.presence(ctx);
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        let (client, mut events) = match tv.connection(ctx).await {
            Ok(connection) => connection,
            Err(e) => {
                tracing::warn!(instance = %ctx.instance(), error = %format!("{e:#}"), "hisense tv: no connection");
                if !tv.pause(ctx, OFF_PAUSE).await {
                    return Ok(());
                }
                continue;
            }
        };
        let outcome = loop {
            tokio::select! {
                command = ctx.next_command() => {
                    let Some(command) = command else { return Ok(()) };
                    tv.order(command, ctx, Some(&client)).await;
                }
                _ = tick.tick() => tv.tick(ctx, &client),
                event = events.poll() => match event {
                    Ok(Event::Incoming(Packet::ConnAck(_))) => tv.on_connected(ctx, &client).await,
                    Ok(Event::Incoming(Packet::Publish(publish))) => {
                        if let Some(message) = protocol::read(&publish.topic, &publish.payload) {
                            tv.on_message(message, ctx, &client).await;
                        }
                    }
                    Ok(_) => {}
                    Err(e) => break tv.on_error(ctx, &e).await,
                },
            }
            tv.presence(ctx);
        };
        tv.connected = false;
        tv.presence(ctx);
        drop(events);
        drop(client);
        match outcome {
            Outcome::Reconnect => {}
            Outcome::Pause(delay) => {
                if !tv.pause(ctx, delay).await {
                    return Ok(());
                }
            }
            Outcome::Stop => {
                ctx.cancelled().await;
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_of_the_login_is_told_apart() {
        assert!(refused(&ConnectionError::ConnectionRefused(
            ConnectReturnCode::BadUserNamePassword
        )));
        assert!(refused(&ConnectionError::ConnectionRefused(
            ConnectReturnCode::NotAuthorized
        )));
        assert!(!refused(&ConnectionError::ConnectionRefused(
            ConnectReturnCode::ServiceUnavailable
        )));
        assert!(!refused(&ConnectionError::NetworkTimeout));
    }

    #[test]
    fn the_config_reads_its_options() {
        let config: Config =
            toml_like(r#"{"host":"192.168.1.10","mac":"02:00:00:00:00:02","auth":"modern"}"#);
        assert_eq!(config.port, DEFAULT_PORT);
        assert_eq!(
            Auth::parse(config.auth.as_deref().unwrap()),
            Ok(Some(Auth::Modern))
        );
        assert!(serde_json::from_str::<Config>(r#"{"host":"h","extra":1}"#).is_err());
    }

    fn toml_like(json: &str) -> Config {
        serde_json::from_str(json).unwrap()
    }
}
