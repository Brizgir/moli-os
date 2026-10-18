//! Amazon Echo speakers in the house, through the Alexa app's own web API
//! (unofficial: Amazon publishes nothing to drive music): music by search on
//! a speaker or a multi-room group, play and pause, volume, a sentence, an
//! announcement, an Alexa command. The account is connected once from the
//! dashboard: Amazon's own sign-in page, then its last address pasted into
//! Moli (no password ever goes through Moli); the refresh token it gives is
//! kept in the encrypted secret store.

mod api;
mod auth;
mod echo;
mod encode;
mod media;
mod queue;
mod watch;

use std::collections::HashMap;
use std::time::Duration;

use moli_core::{Access, Device, DeviceId, Kind, PointSpec, Semantic, Value};
use moli_runtime::{BoxFuture, CommandRequest, Driver, DriverCtx};
use serde::Deserialize;
use tokio::time::{Instant, MissedTickBehavior};

use crate::api::{Failure, Session, SessionsForm};
use crate::echo::Echo;
use crate::media::Playing;
use crate::queue::Queue;
use crate::watch::{Motion, Watch};

const REFRESH: &str = "refresh_token";
const SERIAL: &str = "device_serial";
/// What the speakers play, looked at this often.
const POLL: Duration = Duration::from_secs(30);
/// The speakers' volumes.
const VOLUMES: Duration = Duration::from_secs(120);
/// The volumes again, this soon after an order on one (the slider would
/// otherwise go back to the old value until the next reading).
const VOLUMES_AFTER_ORDER: Duration = Duration::from_secs(5);
/// The account's speakers (one added, renamed in the Alexa app…): one
/// cheap call, so a new name shows in the house within minutes.
const DEVICES: Duration = Duration::from_secs(3 * 60);
/// Amazon asked to slow down (429): the next listing waits this long.
const DEVICES_THROTTLED: Duration = Duration::from_secs(15 * 60);
/// The site's cookies are good two weeks: renewed well before.
const SESSION: Duration = Duration::from_secs(4 * 24 * 3600);
const RETRY_MIN: Duration = Duration::from_secs(30);
/// Why a session ends without a fault: new cookies are due, or Amazon
/// ended it; a new one starts at once.
const RENEW: &str = "the Alexa session is renewed";
const RETRY_MAX: Duration = Duration::from_secs(15 * 60);

fn default_domain() -> String {
    "amazon.fr".into()
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// The account's Amazon site: `amazon.fr`, `amazon.de`, `amazon.com`…
    #[serde(default = "default_domain")]
    pub domain: String,
    /// The music service by default (`AMAZON_MUSIC`, `SPOTIFY`, `DEEZER`,
    /// `TUNEIN`…); empty: the account's own.
    #[serde(default)]
    pub music: String,
}

impl Config {
    /// (`fr-FR`, `fr_FR`) from the site.
    fn languages(&self) -> (&'static str, &'static str) {
        match self.domain.rsplit('.').next().unwrap_or_default() {
            "fr" => ("fr-FR", "fr_FR"),
            "de" => ("de-DE", "de_DE"),
            "it" => ("it-IT", "it_IT"),
            "es" => ("es-ES", "es_ES"),
            "uk" => ("en-GB", "en_GB"),
            _ => ("en-US", "en_US"),
        }
    }
}

#[derive(Debug)]
pub struct Alexa {
    config: Config,
}

impl Alexa {
    #[must_use]
    pub fn new(config: Config) -> Self {
        Self { config }
    }
}

impl Driver for Alexa {
    fn kind(&self) -> &'static str {
        "alexa"
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
        access: Access { read: true, write },
        unit: None,
        semantic: Semantic::Config,
    }
}

/// The account: its state, and where the dashboard signs it in.
fn account_device(ctx: &DriverCtx, id: &DeviceId) -> Device {
    Device {
        id: id.clone(),
        instance: ctx.instance().clone(),
        native_name: moli_i18n::tr!("pilotes.alexa.compte").into(),
        manufacturer: Some("Amazon".into()),
        model: Some("Alexa".into()),
        description: Some(moli_i18n::tr!("pilotes.alexa.description").into()),
        native_room: None,
        members: Vec::new(),
        points: vec![
            text_point(
                "status",
                &moli_i18n::tr!("pilotes.alexa.etat_compte"),
                false,
            ),
            text_point(
                "sign_in_url",
                &moli_i18n::tr!("pilotes.alexa.page_amazon"),
                false,
            ),
            text_point(
                "sign_in",
                &moli_i18n::tr!("pilotes.alexa.adresse_collee"),
                true,
            ),
        ],
    }
}

fn set_text(ctx: &DriverCtx, id: &DeviceId, key: &str, text: &str) {
    ctx.set_state(id, key, Value::Text(text.into()));
}

async fn run(config: &Config, ctx: &mut DriverCtx) -> anyhow::Result<()> {
    let account = ctx.device_id("account");
    ctx.upsert_device(account_device(ctx, &account));
    ctx.set_availability(&account, true);
    let mut pause = RETRY_MIN;
    loop {
        let Some(refresh) = ctx.secret(REFRESH) else {
            if !sign_in(config, ctx, &account).await? {
                return Ok(());
            }
            continue;
        };
        set_text(ctx, &account, "status", "connecting");
        let started = Instant::now();
        match connected(config, ctx, &account, &refresh).await {
            Ok(()) => return Ok(()),
            Err(e) if format!("{e:#}").contains(RENEW) => {
                tracing::info!(instance = %ctx.instance(), "alexa session renewed");
            }
            Err(e) if format!("{e:#}").contains(auth::SIGN_IN_AGAIN) => {
                tracing::warn!(instance = %ctx.instance(), error = %format!("{e:#}"), "alexa: sign in again");
                ctx.forget_secret(REFRESH).await?;
            }
            Err(e) => {
                tracing::warn!(instance = %ctx.instance(), error = %format!("{e:#}"), "alexa unreachable");
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

/// Waits for the person to sign in from the dashboard. `false`: stopped.
async fn sign_in(config: &Config, ctx: &mut DriverCtx, account: &DeviceId) -> anyhow::Result<bool> {
    let (_, language) = config.languages();
    let mut pending = auth::SignIn::new(language)?;
    set_text(ctx, account, "sign_in_url", &pending.url);
    set_text(ctx, account, "status", "sign_in");
    ctx.wait_for(moli_i18n::tr!("pilotes.alexa.a_connecter"));
    loop {
        // `None` once Moli stops.
        let Some(command) = ctx.next_command().await else {
            return Ok(false);
        };
        if command.device.id != *account || &*command.key != "sign_in" {
            command.reply(Err(moli_i18n::tr!("pilotes.alexa.pas_connecte")));
            continue;
        }
        let Value::Text(pasted) = &command.value else {
            command.reply(Err(moli_i18n::tr!("pilotes.alexa.adresse_fausse")));
            continue;
        };
        let code = match auth::code_of(pasted) {
            Ok(code) => code,
            Err(auth::Pasted::NotTheEnd) => {
                command.reply(Err(moli_i18n::tr!("pilotes.alexa.adresse_fausse")));
                continue;
            }
            Err(auth::Pasted::NoCode) => {
                command.reply(Err(moli_i18n::tr!("pilotes.alexa.adresse_sans_code")));
                continue;
            }
        };
        match auth::register(&pending, &code).await {
            Ok(registered) => {
                if let Err(e) = keep(ctx, &registered.refresh_token, &pending.serial).await {
                    command.reply(Err(moli_i18n::tr!("pilotes.alexa.coffre", why = e)));
                    return Err(e);
                }
                set_text(ctx, account, "sign_in_url", "");
                command.reply(Ok(()));
                tracing::info!(instance = %ctx.instance(), "alexa account connected");
                ctx.ready();
                return Ok(true);
            }
            Err(e) => {
                tracing::warn!(instance = %ctx.instance(), error = %format!("{e:#}"), "alexa sign-in refused");
                // A code serves once: a new page for the next try.
                pending = auth::SignIn::new(language)?;
                set_text(ctx, account, "sign_in_url", &pending.url);
                command.reply(Err(moli_i18n::tr!(
                    "pilotes.alexa.refuse",
                    why = format!("{e:#}")
                )));
            }
        }
    }
}

async fn keep(ctx: &DriverCtx, refresh: &str, serial: &str) -> anyhow::Result<()> {
    ctx.store_secret(REFRESH, refresh).await?;
    ctx.store_secret(SERIAL, serial).await?;
    Ok(())
}

/// The speakers by Moli device, and the account's session.
struct Home {
    session: Session,
    speakers: HashMap<DeviceId, Echo>,
    music: String,
    /// Speakers whose state could not be read, said once per session.
    unreadable: std::collections::HashSet<String>,
    /// The form of `list-media-sessions` the site answered last, if any.
    form: Option<SessionsForm>,
    /// The forms the site refused (4xx) in this session: not asked again.
    refused: Vec<SessionsForm>,
    /// Speakers whose `/api/media/state` the site refuses.
    no_media_state: std::collections::HashSet<String>,
    /// The last shape logged, to log it again only when it changes.
    shape: String,
    /// The orders that wait their turn (one at a time, a second apart).
    queue: Queue<CommandRequest>,
    /// Whether the sessions that say « PLAYING » still move.
    watch: Watch,
}

impl Home {
    fn echo(&self, serial: &str) -> Option<&Echo> {
        self.speakers.values().find(|e| e.serial == serial)
    }

    /// The forms of the sessions' list still worth asking, the one that
    /// answered last first.
    fn forms(&self) -> Vec<SessionsForm> {
        self.form
            .into_iter()
            .chain(SessionsForm::ALL)
            .filter(|f| !self.refused.contains(f))
            .fold(Vec::new(), |mut all, f| {
                if !all.contains(&f) {
                    all.push(f);
                }
                all
            })
    }
}

/// Connected: the speakers, then their orders and their state, until stopped.
async fn connected(
    config: &Config,
    ctx: &mut DriverCtx,
    account: &DeviceId,
    refresh: &str,
) -> anyhow::Result<()> {
    let (locale, _) = config.languages();
    let session = Session::open(refresh, &config.domain, locale).await?;
    let music = if config.music.is_empty() {
        session
            .default_music()
            .await
            .unwrap_or_else(|| "AMAZON_MUSIC".into())
    } else {
        config.music.clone()
    };
    let mut home = Home {
        session,
        speakers: HashMap::new(),
        music,
        unreadable: std::collections::HashSet::new(),
        form: None,
        refused: Vec::new(),
        no_media_state: std::collections::HashSet::new(),
        shape: String::new(),
        queue: Queue::default(),
        watch: Watch::default(),
    };
    discover(ctx, &mut home)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    set_text(ctx, account, "status", "connected");
    ctx.ready();
    tracing::info!(
        instance = %ctx.instance(),
        speakers = home.speakers.len(),
        music = %home.music,
        "alexa connected"
    );
    let mut poll = tokio::time::interval(POLL);
    let mut volumes = tokio::time::interval(VOLUMES);
    let mut devices = tokio::time::interval(DEVICES);
    for timer in [&mut poll, &mut volumes, &mut devices] {
        timer.set_missed_tick_behavior(MissedTickBehavior::Delay);
        timer.reset();
    }
    let renew_at = Instant::now() + SESSION;
    loop {
        tokio::select! {
            command = ctx.next_command() => {
                let Some(command) = command else { return Ok(()) };
                // Several values for one point while they wait: the last
                // one only, the others are taken as done.
                if let Some(old) = home.queue.push(command.point.to_string(), command) {
                    old.reply(Ok(()));
                }
            }
            () = tokio::time::sleep_until(home.queue.ready_at(Instant::now())), if !home.queue.is_empty() => {
                let Some(command) = home.queue.pop(Instant::now()) else { continue };
                if command.abandoned() {
                    command.reply(Err("expired".into()));
                    continue;
                }
                let result = order(config, &mut home, refresh, &command).await;
                home.queue.sent(Instant::now());
                if result.is_ok() {
                    ordered(&mut home, &command);
                    confirm(ctx, &home, &command, &mut volumes);
                }
                command.reply(result.map_err(|e| moli_i18n::tr!("pilotes.alexa.echec", why = e)));
                poll.reset_after(Duration::from_secs(3));
            }
            _ = poll.tick() => playing(ctx, &mut home).await?,
            _ = volumes.tick() => match home.session.volumes().await {
                Ok(all) => {
                    for (id, e) in &home.speakers {
                        if let Some((_, v)) = all.iter().find(|(s, _)| *s == e.serial) {
                            ctx.set_state(id, "volume", Value::Float(*v));
                        }
                    }
                }
                Err(e) => tracing::warn!(instance = %ctx.instance(), error = %e, "alexa volumes"),
            },
            _ = devices.tick() => match discover(ctx, &mut home).await {
                Ok(()) => {}
                Err(Failure::Expired) => anyhow::bail!("{RENEW}"),
                // The speakers already known keep working: the list is
                // read again next time, later if Amazon asked for a pause.
                Err(Failure::Other(e)) => {
                    let throttled = throttled(&e);
                    tracing::warn!(instance = %ctx.instance(), error = %format!("{e:#}"), throttled, "alexa devices");
                    if throttled {
                        devices.reset_after(DEVICES_THROTTLED);
                    }
                }
            },
            () = tokio::time::sleep_until(renew_at) => {
                // New cookies: `connected` starts again.
                anyhow::bail!("{RENEW}");
            }
        }
    }
}

/// An order Amazon accepted: what it set is shown at once (the volume),
/// and read back soon after.
fn confirm(
    ctx: &DriverCtx,
    home: &Home,
    command: &CommandRequest,
    volumes: &mut tokio::time::Interval,
) {
    if &*command.key != "volume" {
        return;
    }
    let group = home
        .speakers
        .get(&command.device.id)
        .is_some_and(|e| e.group);
    if let (Some(level), false) = (command.value.as_f64(), group) {
        ctx.set_state(&command.device.id, "volume", Value::Float(level.round()));
    }
    volumes.reset_after(VOLUMES_AFTER_ORDER);
}

/// A play or pause order Amazon accepted: what the speaker's session (and
/// its members', for a group) did before says nothing of what comes next.
fn ordered(home: &mut Home, command: &CommandRequest) {
    if !matches!(&*command.key, "playing" | "play") {
        return;
    }
    let Some(e) = home.speakers.get(&command.device.id) else {
        return;
    };
    let now = std::time::Instant::now();
    for serial in std::iter::once(&e.serial).chain(&e.members) {
        home.watch.reset(serial, now);
    }
}

/// Whether Amazon answered 429 (too many requests).
fn throttled(e: &anyhow::Error) -> bool {
    format!("{e:#}").contains(": HTTP 429")
}

/// The account's speakers and groups, shown in the house. Read again
/// every few minutes: a speaker renamed in the Alexa app is published
/// again under its new name (same device, its points and history kept).
async fn discover(ctx: &DriverCtx, home: &mut Home) -> Result<(), Failure> {
    let answer = home.session.devices().await?;
    let before = std::mem::take(&mut home.speakers);
    for e in echo::speakers(&answer) {
        let id = ctx.device_id(&e.serial);
        match before.get(&id) {
            None => tracing::info!(
                instance = %ctx.instance(),
                name = %e.name,
                family = %e.family,
                device_type = %e.device_type,
                group = e.group,
                online = e.online,
                "alexa speaker"
            ),
            Some(known) if known.name != e.name => tracing::info!(
                instance = %ctx.instance(),
                from = %known.name,
                to = %e.name,
                "alexa speaker renamed"
            ),
            Some(_) => {}
        }
        // The hub keeps the device as it is when nothing changed.
        ctx.upsert_device(echo::device(ctx, &id, &e));
        ctx.set_availability(&id, e.online);
        home.speakers.insert(id, e);
    }
    Ok(())
}

/// What each speaker plays: the account's media sessions, each speaker's
/// own player for the rest, its older media state when the player says
/// nothing, then the groups that play for their members. A speaker no
/// source describes is unknown (a session over: an error, `connected`
/// starts again).
async fn playing(ctx: &DriverCtx, home: &mut Home) -> anyhow::Result<()> {
    let mut online: Vec<Echo> = home
        .speakers
        .values()
        .filter(|e| e.online)
        .cloned()
        .collect();
    online.sort_by(|a, b| a.serial.cmp(&b.serial));
    let online: Vec<&Echo> = online.iter().collect();
    let first = online.iter().find(|e| !e.group).or(online.first()).copied();
    let (sessions, sessions_shape) = media_sessions(ctx, home, first).await;
    let mut states = media::from_sessions(&online, &sessions);
    let mut sources: HashMap<String, String> = states
        .keys()
        .map(|s| (s.clone(), "sessions".to_owned()))
        .collect();
    for e in &online {
        if states.contains_key(&e.serial) {
            continue;
        }
        // The device form of the list may describe this speaker only.
        if home.form == Some(SessionsForm::Device) && first.is_some_and(|f| f.serial != e.serial) {
            let (own, _) = media_sessions(ctx, home, Some(e)).await;
            if let Some(p) = media::from_sessions(&[e], &own).remove(&e.serial) {
                sources.insert(e.serial.clone(), "sessions(device)".into());
                states.insert(e.serial.clone(), p);
                continue;
            }
        }
        let (p, source) = match speaker_state(home, e).await {
            Ok(found) => found,
            Err(Failure::Expired) => anyhow::bail!("{RENEW}"),
            Err(Failure::Other(err)) => {
                if home.unreadable.insert(e.serial.clone()) {
                    tracing::warn!(speaker = %e.name, error = %format!("{err:#}"), "alexa player state");
                }
                (Playing::unknown(), "error".into())
            }
        };
        sources.insert(e.serial.clone(), source);
        states.insert(e.serial.clone(), p);
    }
    stand_still(home, &online, &mut states, &mut sources).await?;
    for m in media::through_groups(&online, &mut states) {
        sources.insert(m, "group".into());
    }
    for (id, e) in &home.speakers {
        let Some(p) = states.get(&e.serial) else {
            continue;
        };
        // Unknown: `playing` is null, never « stopped » the dashboard
        // would act on.
        ctx.set_state(id, "playing", p.plays().map_or(Value::Null, Value::Bool));
        set_text(ctx, id, "state", &p.state.to_lowercase());
        set_text(ctx, id, "title", &p.title);
        set_text(ctx, id, "artist", &p.artist);
        if let (Some(v), false) = (p.volume, e.group) {
            ctx.set_state(id, "volume", Value::Float(v));
        }
    }
    // Speakers by their kind and their place in the list, never by name.
    let now = online
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let p = states.get(&e.serial);
            format!(
                "#{i} {}/{}: {} from {}{}",
                e.family,
                e.device_type,
                p.map_or("?", |p| p.state.as_str()),
                sources.get(&e.serial).map_or("nowhere", String::as_str),
                if p.is_some_and(|p| !p.title.is_empty()) {
                    " titled"
                } else {
                    ""
                },
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    let now = format!("sessions {{{sessions_shape}}}; {now}");
    if home.shape != now {
        tracing::debug!(instance = %ctx.instance(), shape = %now, "alexa playing");
        home.shape = now;
    }
    Ok(())
}

/// A session that says « PLAYING » is believed while it moves, or while
/// the speaker's own state says it plays too; else the speaker shows paused
/// (idle once still 10 min). Groups keep what their session says. The
/// verdict goes to the log's source (`progress=advances`, `frozen`…).
async fn stand_still(
    home: &mut Home,
    online: &[&Echo],
    states: &mut HashMap<String, Playing>,
    sources: &mut HashMap<String, String>,
) -> anyhow::Result<()> {
    let now = std::time::Instant::now();
    let mut watched = Vec::new();
    for e in online.iter().filter(|e| !e.group) {
        let from_sessions = sources
            .get(&e.serial)
            .is_some_and(|s| s.starts_with("sessions"));
        let Some(p) = states
            .get(&e.serial)
            .filter(|p| from_sessions && p.state == media::PLAYING)
            .cloned()
        else {
            continue;
        };
        watched.push(e.serial.as_str());
        let (motion, still) = home.watch.judge(&e.serial, &p, now);
        let own = if matches!(motion, Motion::New | Motion::Still | Motion::NoProgress) {
            match speaker_state(home, e).await {
                Ok((own, _)) => Some(own),
                Err(Failure::Expired) => anyhow::bail!("{RENEW}"),
                Err(Failure::Other(_)) => None,
            }
        } else {
            None
        };
        let (shown, word) = watch::judged(&p, motion, still, own.as_ref());
        if word == "confirmed" {
            home.watch.confirm(&e.serial, now);
        }
        if let Some(source) = sources.get_mut(&e.serial) {
            source.push_str(" progress=");
            source.push_str(word);
        }
        states.insert(e.serial.clone(), shown);
    }
    home.watch.keep(&watched);
    Ok(())
}

/// What one speaker's own sources say: its player, then (when the player
/// says nothing) its older media state; unknown when neither does. With
/// the source for the log.
async fn speaker_state(home: &mut Home, e: &Echo) -> Result<(Playing, String), Failure> {
    let (p, answer) = home.session.playing(e).await?;
    if !p.is_unknown() || home.no_media_state.contains(&e.serial) {
        return Ok((p, format!("player {{{answer}}}")));
    }
    let (m, media) = home.session.media_state(e).await?;
    if m.is_unknown() {
        if media.starts_with("HTTP") {
            home.no_media_state.insert(e.serial.clone());
        }
        // The player's volume is still worth showing.
        return Ok((
            Playing {
                volume: p.volume.or(m.volume),
                ..Playing::unknown()
            },
            format!("player {{{answer}}} media {{{media}}}"),
        ));
    }
    Ok((m, format!("media {{{media}}}")))
}

/// The account's media sessions, by the forms the site still answers
/// (the one that answered last first; the device form names `echo`), and
/// the form and shape of the answer for the log.
async fn media_sessions(
    ctx: &DriverCtx,
    home: &mut Home,
    echo: Option<&Echo>,
) -> (Vec<media::Session>, String) {
    for form in home.forms() {
        match home.session.media_sessions(form, echo).await {
            Ok(Some(answer)) => {
                if home.form != Some(form) {
                    tracing::info!(instance = %ctx.instance(), form = %form, "alexa: media sessions list answers");
                    home.form = Some(form);
                }
                return (
                    media::sessions(&answer),
                    format!("form={form} {}", media::sessions_shape(&answer)),
                );
            }
            Ok(None) => {
                if form != SessionsForm::Device || echo.is_some() {
                    home.refused.push(form);
                }
                if home.form == Some(form) {
                    home.form = None;
                }
            }
            Err(e) => {
                tracing::debug!(instance = %ctx.instance(), form = %form, error = %e, "alexa media sessions");
                return (Vec::new(), format!("form={form} unreachable"));
            }
        }
    }
    (Vec::new(), "none answers on this account".into())
}

/// One order; a session that expired is renewed once and the order sent
/// again; an order refused for coming too soon (429) is sent again once
/// after a pause.
async fn order(
    config: &Config,
    home: &mut Home,
    refresh: &str,
    command: &CommandRequest,
) -> Result<(), String> {
    match send(home, command).await {
        Err(Failure::Expired) => {
            let (locale, _) = config.languages();
            home.session = Session::open(refresh, &config.domain, locale)
                .await
                .map_err(|e| format!("{e:#}"))?;
            send(home, command).await.map_err(|e| e.to_string())
        }
        Err(e) if api::rate_limited(&e) => {
            tracing::debug!(point = %command.point, "alexa: too many orders, one more try");
            tokio::time::sleep(queue::BACKOFF).await;
            send(home, command).await.map_err(|e| e.to_string())
        }
        other => other.map_err(|e| e.to_string()),
    }
}

fn text(value: &Value) -> Result<String, Failure> {
    match value {
        Value::Text(t) => api::sayable(t).map_err(Failure::Other),
        _ => Err(Failure::Other(anyhow::anyhow!("text expected"))),
    }
}

async fn send(home: &Home, command: &CommandRequest) -> Result<(), Failure> {
    let e = home
        .speakers
        .get(&command.device.id)
        .ok_or_else(|| Failure::Other(anyhow::anyhow!("unknown speaker")))?;
    let s = &home.session;
    let (customer, locale) = (s.customer.as_str(), s.locale.as_str());
    // A group's members, for what a group cannot do itself.
    let members: Vec<&Echo> = e.members.iter().filter_map(|m| home.echo(m)).collect();
    let each = |f: &dyn Fn(&Echo) -> serde_json::Value| {
        if e.group && !members.is_empty() {
            api::together(members.iter().map(|m| f(m)).collect())
        } else {
            f(e)
        }
    };
    match (&*command.key, &command.value) {
        ("playing", Value::Bool(play)) => {
            s.player(e, if *play { "PlayCommand" } else { "PauseCommand" })
                .await
        }
        ("volume", v) => {
            let level = v
                .as_f64()
                .ok_or_else(|| Failure::Other(anyhow::anyhow!("a number is expected")))?;
            #[allow(clippy::cast_possible_truncation)]
            let level = level.round() as i64;
            s.run(&api::volume(e, customer, locale, level)).await
        }
        ("play", v) => {
            let phrase = &text(v)?;
            s.play(e, phrase, &home.music).await
        }
        ("say", v) => {
            let words = &text(v)?;
            s.run(&each(&|m| api::speak(m, customer, locale, words)))
                .await
        }
        ("announce", v) => {
            let words = &text(v)?;
            let targets: Vec<&Echo> = if e.group { members.clone() } else { vec![e] };
            s.run(&api::announce(&targets, customer, locale, words))
                .await
        }
        ("command", v) => {
            let words = &text(v)?;
            s.run(&each(&|m| api::text_command(m, customer, locale, words)))
                .await
        }
        (key, _) => Err(Failure::Other(anyhow::anyhow!("{key} cannot be written"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_site_gives_the_languages() {
        let config = |domain: &str| Config {
            domain: domain.into(),
            music: String::new(),
        };
        assert_eq!(config("amazon.fr").languages(), ("fr-FR", "fr_FR"));
        assert_eq!(config("amazon.co.uk").languages(), ("en-GB", "en_GB"));
        assert_eq!(config("amazon.com").languages(), ("en-US", "en_US"));
        let parsed: Config = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(parsed.domain, "amazon.fr");
        assert!(serde_json::from_value::<Config>(serde_json::json!({ "password": "x" })).is_err());
    }
}
