//! Ecowitt weather stations (GW1200, GW2000, GW1100… gateways and their
//! sensors), locally: the gateway posts its readings to Moli every minute
//! (« Customized » upload, Ecowitt protocol, plain HTTP), Moli never asks.
//! No cloud, no account.
//!
//! The driver listens on its own port (49199 by default, aioecowitt's), only
//! for the home network: an address outside it is closed before a byte is
//! read. One request per connection, bounded (head 8 KiB, body 16 KiB, 10 s),
//! 4 at a time. The gateway is recognised by its `PASSKEY` (the MD5 of its
//! MAC address): the one in the options, or else the first one heard since
//! the start. One instance = one station.
//!
//! Devices: `outdoor` (the outdoor array: temperature, humidity, wind,
//! rain, light, UV, its batteries), `indoor` (the gateway: temperature,
//! humidity, pressure, CO₂), `ch1`…`ch8` (extra thermo-hygrometers), and
//! the other sensors Ecowitt sells: `lightning` (WH57), `air` (WH45),
//! `globe` (WN38), `soil<n>` (WH51), `probe<n>` (WN34), `leak<n>` (WH55),
//! `pm25_<n>` (WH41), `leaf<n>` (WN35). Each is declared when its first
//! reading arrives. Nothing is written: read-only.

mod http;
mod report;

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context as _, ensure};
use moli_core::{Access, Device, Kind, PointSpec, Value};
use moli_runtime::{BoxFuture, Driver, DriverCtx};
use serde::Deserialize;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Semaphore, mpsc};

pub use report::{Form, Part, Reading, parse_form, readings};

/// Time a gateway has to send its whole request.
const REQUEST_TIME: Duration = Duration::from_secs(10);
/// Requests read at the same time; more are closed at once.
const CONNECTIONS: usize = 4;
/// Posts waiting for the driver's loop; more are answered 503.
const QUEUE: usize = 4;
/// At most one warning a minute about refused requests.
const WARN_EVERY: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Port the gateway posts to (default 49199).
    #[serde(default = "default_port")]
    pub port: u16,
    /// Address to listen on (default: all of this machine's).
    #[serde(default)]
    pub listen: Option<IpAddr>,
    /// Path the gateway posts to, as set in its « Customized » page
    /// (default `/data/report/`).
    #[serde(default = "default_path")]
    pub path: String,
    /// The gateway's `PASSKEY` (32 hexadecimal digits): only it is heard.
    /// Without it, the first gateway heard since the start.
    #[serde(default)]
    pub passkey: Option<String>,
    /// Addresses allowed to post (the gateway's). Empty: the whole home
    /// network.
    #[serde(default)]
    pub allow: Vec<IpAddr>,
    /// Name of the outdoor device (default « Station météo »).
    #[serde(default)]
    pub name: Option<String>,
}

fn default_port() -> u16 {
    49199
}

fn default_path() -> String {
    "/data/report/".into()
}

#[derive(Debug)]
pub struct Ecowitt {
    config: Config,
}

impl Ecowitt {
    pub fn new(mut config: Config) -> anyhow::Result<Self> {
        ensure!(config.port != 0, "port: 1 to 65535");
        ensure!(
            config.path.starts_with('/')
                && config.path.len() <= 128
                && config
                    .path
                    .bytes()
                    .all(|b| b.is_ascii_graphic() && b != b'?' && b != b'#'),
            "path {:?}: starts with '/', printable ASCII, no '?', 128 characters at most",
            config.path
        );
        if let Some(key) = &mut config.passkey {
            ensure!(
                key.len() == 32 && key.bytes().all(|b| b.is_ascii_hexdigit()),
                "passkey: the gateway's PASSKEY, 32 hexadecimal digits"
            );
            *key = key.to_ascii_uppercase();
        }
        for ip in &config.allow {
            ensure!(http::local(*ip), "allow: {ip} is not on the home network");
        }
        Ok(Self { config })
    }
}

impl Driver for Ecowitt {
    fn kind(&self) -> &'static str {
        "ecowitt"
    }

    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(run(&self.config, ctx))
    }
}

/// What the connections share: who may post, and when the last warning was.
#[derive(Debug)]
struct Door {
    path: String,
    allow: Vec<IpAddr>,
    /// The gateway heard: from the options, or the first since the start.
    passkey: Mutex<Option<String>>,
    pinned: bool,
    warned: Mutex<Option<Instant>>,
}

impl Door {
    /// A refusal, in the log at most once a minute (with the address that
    /// knocked and why; never what it sent).
    fn refused(&self, peer: IpAddr, refusal: http::Refusal) {
        let mut warned = self
            .warned
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if warned.is_some_and(|t| t.elapsed() < WARN_EVERY) {
            tracing::debug!(%peer, status = refusal.status, why = refusal.why, "ecowitt post refused");
            return;
        }
        *warned = Some(Instant::now());
        tracing::warn!(%peer, status = refusal.status, why = refusal.why, "ecowitt post refused");
    }

    /// The form, if the gateway is the one this instance listens to.
    fn admit(&self, body: &[u8]) -> Result<Form, http::Refusal> {
        let form = parse_form(body).map_err(|_| http::Refusal {
            status: 400,
            why: "not a readable form",
        })?;
        let Some(key) = report::station(&form).passkey else {
            return Err(http::Refusal {
                status: 400,
                why: "no PASSKEY",
            });
        };
        let mut known = self
            .passkey
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match known.as_deref() {
            Some(k) if k == key => Ok(form),
            Some(_) if self.pinned => Err(http::Refusal {
                status: 403,
                why: "another PASSKEY than the one in the options",
            }),
            Some(_) => Err(http::Refusal {
                status: 403,
                why: "another gateway than the first one heard (set passkey)",
            }),
            None => {
                tracing::info!(
                    passkey = %key,
                    "ecowitt gateway heard: add passkey = \"{key}\" to the options to keep to it"
                );
                *known = Some(key);
                Ok(form)
            }
        }
    }
}

async fn run(config: &Config, ctx: &mut DriverCtx) -> anyhow::Result<()> {
    let address = SocketAddr::new(
        config.listen.unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
        config.port,
    );
    let listener = TcpListener::bind(address)
        .await
        .with_context(|| format!("listen on {address}"))?;
    ctx.wait_for(moli_i18n::tr!(
        "pilotes.ecowitt.attente",
        port = config.port,
        path = config.path
    ));
    let door = Arc::new(Door {
        path: config.path.clone(),
        allow: config.allow.clone(),
        passkey: Mutex::new(config.passkey.clone()),
        pinned: config.passkey.is_some(),
        warned: Mutex::new(None),
    });
    let slots = Arc::new(Semaphore::new(CONNECTIONS));
    let (posts, mut heard) = mpsc::channel::<Form>(QUEUE);
    let mut station = Station::new(config.name.clone());
    let mut tick = tokio::time::interval(Duration::from_secs(30));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            command = ctx.next_command() => match command {
                Some(command) => command.reply(Err(moli_i18n::tr!("pilotes.ecowitt.lecture_seule"))),
                None => return Ok(()),
            },
            accepted = listener.accept() => {
                let (stream, peer) = match accepted {
                    Ok(a) => a,
                    Err(e) => {
                        tracing::warn!(error = %e, "ecowitt: accept failed");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                };
                let peer = peer.ip().to_canonical();
                let allowed = http::local(peer) && (door.allow.is_empty() || door.allow.contains(&peer));
                if !allowed {
                    door.refused(peer, http::Refusal { status: 403, why: "outside the home network or not allowed" });
                    continue; // dropped: closed without a word
                }
                let Ok(slot) = Arc::clone(&slots).try_acquire_owned() else {
                    tracing::debug!(%peer, "ecowitt: too many connections");
                    continue;
                };
                let (door, posts) = (Arc::clone(&door), posts.clone());
                tokio::spawn(async move {
                    let _slot = slot;
                    serve(stream, peer, &door, &posts).await;
                });
            }
            Some(form) = heard.recv() => {
                // The first post, or the first after a silence.
                let back = !station.online;
                station.post(ctx, &form, Instant::now());
                if back {
                    ctx.ready();
                }
            }
            _ = tick.tick() => {
                if station.silent(ctx, Instant::now()) {
                    ctx.wait_for(moli_i18n::tr!(
                        "pilotes.ecowitt.silence",
                        minutes = station.silence().as_secs() / 60
                    ));
                }
            }
        }
    }
}

/// One connection: read, check, hand over, answer.
async fn serve(mut stream: TcpStream, peer: IpAddr, door: &Door, posts: &mpsc::Sender<Form>) {
    let outcome =
        match tokio::time::timeout(REQUEST_TIME, http::read(&mut stream, &door.path)).await {
            Err(_) => Err(http::Refusal {
                status: 408,
                why: "too slow",
            }),
            Ok(read) => read.and_then(|posted| door.admit(&posted.body)),
        };
    let status = match outcome {
        Ok(form) => match posts.try_send(form) {
            Ok(()) => 200,
            Err(_) => 503,
        },
        Err(refusal) => {
            door.refused(peer, refusal);
            refusal.status
        }
    };
    http::answer(&mut stream, status).await;
}

/// The station as heard: its devices, the points each one has shown, when
/// it last spoke.
#[derive(Debug)]
struct Station {
    name: Option<String>,
    /// Device → keys declared, in [`report::POINTS`] order.
    declared: BTreeMap<Part, Vec<&'static str>>,
    last: Option<Instant>,
    interval: Duration,
    online: bool,
    /// Fields left aside, already said in the log.
    unread_said: bool,
}

impl Station {
    fn new(name: Option<String>) -> Self {
        Self {
            name,
            declared: BTreeMap::new(),
            last: None,
            interval: Duration::from_secs(60),
            online: false,
            unread_said: false,
        }
    }

    /// No post for this long: the station is gone (three missed posts,
    /// three minutes at least).
    fn silence(&self) -> Duration {
        (self.interval * 3).max(Duration::from_secs(180))
    }

    /// The devices whose points grew with this form, and its readings.
    fn learn(&mut self, readings: &[Reading]) -> Vec<Part> {
        let mut grown = Vec::new();
        for r in readings {
            let keys = self.declared.entry(r.part).or_default();
            if !keys.contains(&r.key) {
                keys.push(r.key);
                if !grown.contains(&r.part) {
                    grown.push(r.part);
                }
            }
        }
        for part in &grown {
            if let Some(keys) = self.declared.get_mut(part) {
                keys.sort_by_key(|k| report::POINTS.iter().position(|p| p.key == *k));
            }
        }
        grown
    }

    fn post(&mut self, ctx: &DriverCtx, form: &Form, now: Instant) {
        let about = report::station(form);
        if let Some(s) = about.interval.filter(|s| (5..=3600).contains(s)) {
            self.interval = Duration::from_secs(s);
        }
        if !self.unread_said {
            self.unread_said = true;
            let unread = report::unread(form);
            tracing::info!(
                model = about.model.as_deref().unwrap_or("?"),
                firmware = about.firmware.as_deref().unwrap_or("?"),
                unread = %unread.join(","),
                "ecowitt station heard"
            );
        }
        let readings = readings(form);
        for part in self.learn(&readings) {
            ctx.upsert_device(self.device(ctx, part, form, &about));
        }
        for part in self.declared.keys() {
            ctx.set_availability(&ctx.device_id(&part.native()), true);
        }
        for r in readings {
            ctx.set_state(&ctx.device_id(&r.part.native()), r.key, r.value);
        }
        self.last = Some(now);
        self.online = true;
    }

    /// Marks every device offline when the station went quiet; true the
    /// first time.
    fn silent(&mut self, ctx: &DriverCtx, now: Instant) -> bool {
        let quiet = self
            .last
            .is_some_and(|t| now.duration_since(t) >= self.silence());
        if !(quiet && self.online) {
            return false;
        }
        self.online = false;
        for part in self.declared.keys() {
            ctx.set_availability(&ctx.device_id(&part.native()), false);
        }
        true
    }

    fn device(&self, ctx: &DriverCtx, part: Part, form: &Form, about: &report::Station) -> Device {
        let (name, model, description) = match part {
            Part::Outdoor => (
                self.name
                    .clone()
                    .unwrap_or_else(|| moli_i18n::tr!("pilotes.ecowitt.station")),
                outdoor_model(form),
                moli_i18n::tr!("pilotes.ecowitt.description_exterieur"),
            ),
            Part::Indoor => (
                moli_i18n::tr!("pilotes.ecowitt.passerelle"),
                about.model.clone(),
                about.firmware.clone().unwrap_or_default(),
            ),
            Part::Channel(n) => (
                moli_i18n::tr!("pilotes.ecowitt.canal", n = n),
                Some("WH31".into()),
                moli_i18n::tr!("pilotes.ecowitt.description_canal", n = n),
            ),
            Part::Lightning => sensor(moli_i18n::tr!("pilotes.ecowitt.foudre"), "WH57"),
            Part::Air => sensor(moli_i18n::tr!("pilotes.ecowitt.air"), "WH45"),
            Part::Globe => sensor(moli_i18n::tr!("pilotes.ecowitt.globe"), "WN38"),
            Part::Soil(n) => sensor(moli_i18n::tr!("pilotes.ecowitt.sol", n = n), "WH51"),
            Part::Probe(n) => sensor(moli_i18n::tr!("pilotes.ecowitt.sonde", n = n), "WN34"),
            Part::Leak(n) => sensor(moli_i18n::tr!("pilotes.ecowitt.fuite", n = n), "WH55"),
            Part::Pm25(n) => sensor(moli_i18n::tr!("pilotes.ecowitt.particules", n = n), "WH41"),
            Part::Leaf(n) => sensor(moli_i18n::tr!("pilotes.ecowitt.feuilles", n = n), "WN35"),
        };
        let keys = self.declared.get(&part).map_or(&[][..], Vec::as_slice);
        Device {
            id: ctx.device_id(&part.native()),
            instance: ctx.instance().clone(),
            native_name: name.into(),
            manufacturer: Some("Ecowitt".into()),
            model: model.map(Into::into),
            description: Some(description).filter(|d| !d.is_empty()).map(Into::into),
            native_room: None,
            members: Vec::new(),
            points: keys.iter().filter_map(|k| spec(k)).collect(),
        }
    }
}

/// The outdoor sensor, from the battery field only it sends.
/// A sensor of the station: its name and model, no description.
fn sensor(name: String, model: &str) -> (String, Option<String>, String) {
    (name, Some(model.to_owned()), String::new())
}

fn outdoor_model(form: &Form) -> Option<String> {
    [
        ("wh90batt", "WS90"),
        ("wh85batt", "WS85"),
        ("wh80batt", "WS80"),
        ("wh65batt", "WH65"),
    ]
    .iter()
    .find(|(field, _)| form.contains_key(*field))
    .map(|(_, model)| (*model).to_owned())
}

fn spec(key: &str) -> Option<PointSpec> {
    let point = report::point(key)?;
    Some(PointSpec {
        key: point.key.into(),
        label: moli_i18n::tr(&format!("pilotes.ecowitt.point.{key}")).into(),
        kind: if point.binary {
            Kind::Binary
        } else if point.text {
            Kind::Text
        } else {
            Kind::Numeric {
                min: None,
                max: None,
                step: None,
            }
        },
        access: Access {
            read: true,
            write: false,
        },
        unit: report::unit(point),
        semantic: point.semantic,
    })
}

/// Values of one device in a form, by key (tests and tools).
pub fn values(form: &Form, part: Part) -> BTreeMap<&'static str, Value> {
    readings(form)
        .into_iter()
        .filter(|r| r.part == part)
        .map(|r| (r.key, r.value))
        .collect()
}

#[cfg(test)]
mod tests;
