//! Ecowitt weather stations (and the Ambient Weather, Froggit, Sainlogic…
//! made by Fine Offset): the station posts its readings to Moli, Moli
//! never asks.
//!
//! On the station (WS View Plus or the gateway's web page), « Weather
//! Services » › « Customized »: protocol Ecowitt, server the address of
//! Moli's machine, port `port` (49199), path `path` (any by default), an
//! interval of 16 s to 10 min. Every upload is a form of readings, turned
//! into points by `model`: the station and its outdoor sensors, the console
//! indoors, and one device per extra sensor.
//!
//! What Home Assistant does with its `ecowitt` integration and the
//! `aioecowitt` library (a webhook, then the readings converted and
//! derived), as a driver of its own: its own small listener, only the local
//! network heard, everything bounded. Read-only: a station takes no order.

mod http;
mod model;

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, ensure};
use moli_core::{Device, DeviceId, PointSpec};
use moli_runtime::{BoxFuture, Driver, DriverCtx};
use serde::Deserialize;
use tokio::io::AsyncWriteExt as _;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Semaphore, mpsc};

pub use model::{Group, NotAReading, Reading};

/// The port `aioecowitt` listens on: a station set up for it finds Moli.
const DEFAULT_PORT: u16 = 49199;
/// Without `stations`, at most this many stations are heard.
const MAX_STATIONS: usize = 4;
/// Connections read at once; more wait for none, they are closed.
const MAX_CONNECTIONS: usize = 8;
/// A device is offline after this many intervals without a word of it…
const STALE_INTERVALS: u32 = 3;
/// …and never sooner than this (Home Assistant's 5 minutes).
const STALE_MIN: Duration = Duration::from_secs(5 * 60);
/// Names logged once (unknown fields, refused stations): bounded.
const MAX_WARNED: usize = 256;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// The port the station posts to (« Customized » › Port).
    #[serde(default = "default_port")]
    pub port: u16,
    /// The path the station posts to (« Customized » › Path); any path when
    /// absent.
    #[serde(default)]
    pub path: Option<String>,
    /// The passkeys of the stations heard (32 hex digits, in the log at
    /// their first upload). Empty: any station of the local network, four
    /// at most.
    #[serde(default)]
    pub stations: Vec<String>,
}

fn default_port() -> u16 {
    DEFAULT_PORT
}

#[derive(Debug)]
pub struct Ecowitt {
    config: Config,
    /// `stations`, in capitals.
    allowed: Vec<String>,
}

impl Ecowitt {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        ensure!(config.port != 0, "port: 1 to 65535");
        if let Some(path) = &config.path {
            ensure!(
                path.starts_with('/') && !path.contains(['?', ' ']),
                "path {path:?}: like /data/report/"
            );
        }
        let mut allowed = Vec::new();
        for passkey in &config.stations {
            ensure!(
                passkey.len() == 32 && passkey.bytes().all(|b| b.is_ascii_hexdigit()),
                "stations: {passkey:?} is not a passkey (32 hex digits)"
            );
            allowed.push(passkey.to_ascii_uppercase());
        }
        Ok(Self { config, allowed })
    }
}

impl Driver for Ecowitt {
    fn kind(&self) -> &'static str {
        "ecowitt"
    }

    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(run(self, ctx))
    }
}

type Form = Vec<(String, String)>;

/// A device heard: as published, and when it was last heard of.
#[derive(Debug)]
struct Heard {
    device: Device,
    last: Instant,
    stale_after: Duration,
    online: bool,
}

#[derive(Debug, Default)]
struct State {
    stations: HashSet<String>,
    devices: HashMap<DeviceId, Heard>,
    warned: HashSet<String>,
    ready: bool,
}

impl State {
    /// True the first time `what` is said (and while the list has room).
    fn first(&mut self, what: String) -> bool {
        self.warned.len() < MAX_WARNED && self.warned.insert(what)
    }
}

async fn run(this: &Ecowitt, ctx: &mut DriverCtx) -> anyhow::Result<()> {
    let port = this.config.port;
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)))
        .await
        .with_context(|| moli_i18n::tr!("pilotes.ecowitt.port_pris", port = port))?;
    ctx.wait_for(moli_i18n::tr!("pilotes.ecowitt.en_attente", port = port));
    let (forms, mut uploads) = mpsc::channel::<Form>(16);
    let permits = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    let path: Option<Arc<str>> = this.config.path.as_deref().map(Arc::from);
    let mut state = State::default();
    let mut tick = tokio::time::interval(Duration::from_secs(30));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            command = ctx.next_command() => match command {
                Some(command) => command.reply(Err(moli_i18n::tr!("pilotes.ecowitt.lecture_seule"))),
                None => return Ok(()),
            },
            accepted = listener.accept() => match accepted {
                Ok((stream, peer)) => {
                    // The house's network only; and a few at a time.
                    if !local(peer.ip()) {
                        if state.first(format!("peer {}", peer.ip())) {
                            tracing::warn!(peer = %peer.ip(), "ecowitt: upload from outside the local network refused");
                        }
                        continue;
                    }
                    let Ok(permit) = permits.clone().try_acquire_owned() else {
                        continue;
                    };
                    let forms = forms.clone();
                    let path = path.clone();
                    tokio::spawn(async move {
                        serve(stream, path.as_deref(), &forms).await;
                        drop(permit);
                    });
                }
                Err(e) => {
                    // Out of descriptors, mostly: let it breathe.
                    tracing::debug!(error = %e, "ecowitt: accept");
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            },
            Some(form) = uploads.recv() => receive(this, ctx, &mut state, &form),
            _ = tick.tick() => {
                for (id, heard) in &mut state.devices {
                    if heard.online && heard.last.elapsed() > heard.stale_after {
                        heard.online = false;
                        ctx.set_availability(id, false);
                    }
                }
            }
        }
    }
}

/// One connection: one upload read, answered, closed.
async fn serve(mut stream: TcpStream, path: Option<&str>, forms: &mpsc::Sender<Form>) {
    let read = tokio::time::timeout(http::READ_LIMIT, http::read(&mut stream))
        .await
        .unwrap_or(Err(http::Refusal::Timeout));
    let answer = match read {
        Ok((head, _)) if head.method != "POST" => http::Refusal::MethodNotAllowed.answer(),
        Ok((head, _)) if path.is_some_and(|p| !same_path(p, &head.path)) => {
            http::Refusal::NotFound.answer()
        }
        Ok((_, body)) => match forms.try_send(http::form(&body)) {
            Ok(()) => http::ok(),
            Err(_) => http::Refusal::Busy.answer(),
        },
        Err(refusal) => refusal.answer(),
    };
    let _ = tokio::time::timeout(Duration::from_secs(5), async {
        let _ = stream.write_all(&answer).await;
        let _ = stream.shutdown().await;
    })
    .await;
}

/// `/data/report` and `/data/report/` are the same path.
fn same_path(a: &str, b: &str) -> bool {
    a.trim_end_matches('/') == b.trim_end_matches('/')
}

/// An address of the house's network: private, link-local or this machine.
fn local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_loopback() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return local(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            v6.is_loopback() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
    }
}

/// An upload: its devices published (or completed), their values set.
fn receive(this: &Ecowitt, ctx: &DriverCtx, state: &mut State, form: &Form) {
    let reading = match Reading::parse(form) {
        Ok(reading) => reading,
        Err(e) => {
            tracing::debug!(error = ?e, "ecowitt: not a station's upload");
            return;
        }
    };
    let passkey = reading.passkey.clone();
    if !this.allowed.is_empty() && !this.allowed.contains(&passkey) {
        if state.first(format!("station {passkey}")) {
            tracing::warn!(%passkey, model = %reading.model, "ecowitt: a station not in `stations` was refused");
        }
        return;
    }
    if !state.stations.contains(&passkey) {
        if state.stations.len() >= MAX_STATIONS {
            if state.first(format!("station {passkey}")) {
                tracing::warn!(%passkey, "ecowitt: too many stations, this one is not heard (list them in `stations`)");
            }
            return;
        }
        tracing::info!(%passkey, model = %reading.model, firmware = ?reading.firmware, "ecowitt: station heard");
        state.stations.insert(passkey.clone());
    }
    for field in &reading.unknown {
        if state.first(format!("field {field}")) {
            tracing::info!(%field, "ecowitt: field not known, ignored");
        }
    }
    let stale_after = reading
        .interval
        .map_or(STALE_MIN, |s| Duration::from_secs(s) * STALE_INTERVALS)
        .max(STALE_MIN);
    let now = Instant::now();
    for (group, values) in &reading.groups {
        let id = ctx.device_id(&group.native(&passkey));
        let specs = values.iter().map(|(spec, _)| spec);
        if let Some(heard) = state.devices.get_mut(&id) {
            let new: Vec<PointSpec> = specs
                .filter(|spec| heard.device.point(&spec.key).is_none())
                .cloned()
                .collect();
            if !new.is_empty() {
                // A sensor paired since, a field a new firmware sends.
                heard.device.points.extend(new);
                ctx.upsert_device(heard.device.clone());
            }
        } else {
            let device = device(ctx, &id, *group, &reading, specs.cloned().collect());
            ctx.upsert_device(device.clone());
            state.devices.insert(
                id.clone(),
                Heard {
                    device,
                    last: now,
                    stale_after,
                    online: false,
                },
            );
        }
        let Some(heard) = state.devices.get_mut(&id) else {
            continue;
        };
        for (spec, value) in values {
            ctx.set_state(&id, &spec.key, value.clone());
        }
        heard.last = now;
        heard.stale_after = stale_after;
        if !heard.online {
            heard.online = true;
            ctx.set_availability(&id, true);
        }
    }
    if !state.ready {
        state.ready = true;
        ctx.ready();
    }
}

fn device(
    ctx: &DriverCtx,
    id: &DeviceId,
    group: Group,
    reading: &Reading,
    points: Vec<PointSpec>,
) -> Device {
    Device {
        id: id.clone(),
        instance: ctx.instance().clone(),
        native_name: group.name(&reading.model).into(),
        manufacturer: Some("Ecowitt".into()),
        model: Some(group.model(&reading.model).into()),
        description: match group {
            Group::Station => reading.firmware.as_deref().map(Into::into),
            _ => None,
        },
        native_room: None,
        members: Vec::new(),
        points,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(stations: &[&str]) -> Config {
        Config {
            port: DEFAULT_PORT,
            path: None,
            stations: stations.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    #[test]
    fn options_are_checked() {
        assert!(Ecowitt::new(config(&[])).is_ok());
        assert!(Ecowitt::new(config(&["0123456789abcdef0123456789abcdef"])).is_ok());
        assert!(Ecowitt::new(config(&["aa:bb:cc:dd:ee:ff"])).is_err());
        let mut bad = config(&[]);
        bad.path = Some("data".into());
        assert!(Ecowitt::new(bad).is_err());
    }

    #[test]
    fn only_the_house_network_is_heard() {
        for ip in [
            "192.168.0.20",
            "10.0.0.3",
            "172.16.4.2",
            "127.0.0.1",
            "169.254.1.1",
            "fe80::1",
            "fd00::5",
            "::ffff:192.168.0.7",
        ] {
            assert!(local(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "100.64.0.1", "2001:db8::1", "::ffff:1.1.1.1"] {
            assert!(!local(ip.parse().unwrap()), "{ip}");
        }
        assert!(same_path("/data/report", "/data/report/"));
        assert!(!same_path("/data/report", "/other"));
    }
}
