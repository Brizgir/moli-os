//! Tuya Wi-Fi devices, local first: no cloud once imported, except for
//! what never listens on the LAN (battery sensors, devices behind a
//! gateway), read from the household's cloud project when its keys are
//! there (`cloud`).
//!
//! `moli-os tuya import` reads, once, what the Tuya cloud knows (device
//! list, data points, local keys — through the account Home Assistant
//! already uses) and files it: keys in the encrypted secret store, the rest
//! in `tuya.json` in the data directory. From then on each device is reached on the LAN
//! (TCP 6668); addresses and protocol versions come from the device file,
//! the devices' own UDP broadcasts, or, when those are not heard, from
//! asking the listeners of the home network with each key (`seek`). A
//! device the box gave another address is followed by its broadcasts and,
//! once it proved its key there, its new address is written back to the
//! device file.

pub mod cloud;
mod device;
pub mod model;
pub mod protocol;
mod seek;

use std::collections::{HashMap, HashSet};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use moli_core::DeviceId;
use moli_runtime::{BoxFuture, Driver, DriverCtx};
use serde::Deserialize;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, oneshot, watch};

use device::{Address, Link, Order, Up};
pub use model::{DeviceFile, Dp, DpKind, TuyaDevice};
use protocol::Version;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Written by `moli-os tuya import`. Missing: `tuya.json` in the data
    /// directory, where the import writes it.
    #[serde(default)]
    pub devices_file: Option<PathBuf>,
    /// Listen to the devices' broadcasts for their address and version.
    #[serde(default = "yes")]
    pub discovery: bool,
    /// Look for the devices still without an address on the home network
    /// (the /24 of the known ones), with their keys.
    #[serde(default = "yes")]
    pub seek: bool,
    /// The cloud project's keys (Access ID then Secret, a line each), put
    /// there by the start script from the vault. Missing or empty: no cloud.
    #[serde(default = "default_cloud_keys")]
    pub cloud_keys: PathBuf,
    /// Where the Tuya account lives: `eu` (Central Europe), `eu-west`, `us`…
    #[serde(default = "default_region")]
    pub cloud_region: String,
}

fn default_cloud_keys() -> PathBuf {
    PathBuf::from("/run/secrets/tuya-cloud")
}

fn default_region() -> String {
    "eu".into()
}

/// The device file in `data_dir`, as `moli-os tuya import` writes it.
#[must_use]
pub fn default_devices_file(data_dir: &Path) -> PathBuf {
    data_dir.join("tuya.json")
}

fn yes() -> bool {
    true
}

/// Secret name of a device's local key.
#[must_use]
pub fn key_name(device_id: &str) -> String {
    format!("key-{device_id}")
}

#[derive(Debug)]
pub struct Tuya {
    config: Config,
    devices_file: PathBuf,
}

impl Tuya {
    /// `data_dir` gives the device file when the options name none.
    #[must_use]
    pub fn new(config: Config, data_dir: &Path) -> Self {
        let devices_file = config
            .devices_file
            .clone()
            .unwrap_or_else(|| default_devices_file(data_dir));
        Self {
            config,
            devices_file,
        }
    }
}

impl Driver for Tuya {
    fn kind(&self) -> &'static str {
        "tuya"
    }

    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(run(&self.config, &self.devices_file, ctx))
    }
}

fn parse_key(key: &str) -> Option<[u8; 16]> {
    <[u8; 16]>::try_from(key.as_bytes()).ok()
}

async fn run(config: &Config, devices_file: &Path, ctx: &mut DriverCtx) -> anyhow::Result<()> {
    let file = match tokio::fs::read(devices_file).await {
        Ok(bytes) => DeviceFile::parse(&bytes)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            ctx.wait_for(moli_i18n::tr!(
                "pilotes.tuya.aucun_appareil",
                file = devices_file.display()
            ));
            ctx.cancelled().await;
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };

    let (up_tx, mut up) = mpsc::channel::<Up>(256);
    let mut orders: HashMap<DeviceId, (usize, mpsc::Sender<Order>)> = HashMap::new();
    let mut followed: HashMap<String, Followed> = HashMap::new();
    let mut wanted = Vec::new();
    let mut devices: Vec<(DeviceId, Arc<TuyaDevice>)> = Vec::new();
    let mut tasks = tokio::task::JoinSet::new();
    let mut missing_keys = 0;
    let keys = cloud_keys(config, ctx).await;
    let mut silent_seen = false;
    let mut watched = cloud::Watched::new();
    for dev in file.devices {
        // Never on the LAN: a battery sensor without an address, a device
        // behind a gateway. The cloud speaks for them, when it can.
        let silent = dev.gateway.is_some()
            || (dev.ip.is_none() && cloud::SLEEPY.contains(&dev.category.as_str()));
        silent_seen |= silent;
        if silent && keys.is_some() {
            let device = dev.device(ctx.instance());
            let id = device.id.clone();
            ctx.upsert_device(device);
            let dev = Arc::new(dev);
            watched.insert(dev.id.clone(), (devices.len(), Arc::clone(&dev)));
            devices.push((id, dev));
            continue;
        }
        if dev.gateway.is_some() {
            continue; // behind a hub: not reachable on its own
        }
        let Some(key) = ctx
            .secret(&key_name(&dev.id))
            .as_deref()
            .and_then(parse_key)
        else {
            missing_keys += 1;
            continue;
        };
        let device = dev.device(ctx.instance());
        let id = device.id.clone();
        ctx.upsert_device(device);
        let dev = Arc::new(dev);
        let (follow, addr_rx, reachable) = Followed::start(&dev, key);
        if dev.ip.is_none() {
            wanted.push(seek::Wanted {
                id: dev.id.clone(),
                name: dev.name.trim().to_owned(),
                key,
                address: Arc::clone(&follow.address),
            });
        }
        followed.insert(dev.id.clone(), follow);
        let (order_tx, order_rx) = mpsc::channel(8);
        let index = devices.len();
        orders.insert(id.clone(), (index, order_tx));
        devices.push((id, Arc::clone(&dev)));
        tasks.spawn(
            Link {
                index,
                device: dev,
                key,
                address: addr_rx,
                orders: order_rx,
                up: up_tx.clone(),
                reachable,
            }
            .run(),
        );
    }
    // Keys given later in the dashboard: a restart takes them.
    let await_keys = keys.is_none() && silent_seen;
    if let Some(keys) = keys {
        start_cloud(config, keys, watched, up_tx.clone(), &mut tasks);
    }
    drop(up_tx);
    if missing_keys > 0 {
        tracing::warn!(instance = %ctx.instance(), missing_keys, "tuya devices without a local key are skipped (re-run the import)");
    }
    locate(config, &devices, followed, wanted, &mut tasks);
    ctx.ready();
    tracing::info!(instance = %ctx.instance(), devices = devices.len(), "tuya devices loaded");

    let keys_came = serve(ctx, &mut up, &orders, &devices, devices_file, await_keys).await;
    tasks.abort_all();
    if keys_came {
        anyhow::bail!(moli_i18n::tr!("pilotes.tuya.cles_cloud"));
    }
    Ok(())
}

/// The vault's file first, else what a person gave in the dashboard.
async fn cloud_keys(config: &Config, ctx: &DriverCtx) -> Option<cloud::Keys> {
    match tokio::fs::read_to_string(&config.cloud_keys).await {
        Ok(text) => cloud::Keys::parse(&text),
        Err(_) => None,
    }
    .or_else(|| stored_keys(ctx))
}

/// The cloud keys a person gave in the dashboard, filed under this driver.
fn stored_keys(ctx: &DriverCtx) -> Option<cloud::Keys> {
    let id = ctx.secret(cloud::STORED_ID)?;
    let secret = ctx.secret(cloud::STORED_SECRET)?;
    cloud::Keys::new(&id, &secret)
}

/// Orders go to the devices, what they say goes to the hub, until the end.
/// Returns whether cloud keys arrived meanwhile (then the driver restarts).
async fn serve(
    ctx: &mut DriverCtx,
    up: &mut mpsc::Receiver<Up>,
    orders: &HashMap<DeviceId, (usize, mpsc::Sender<Order>)>,
    devices: &[(DeviceId, Arc<TuyaDevice>)],
    devices_file: &Path,
    await_keys: bool,
) -> bool {
    let mut look = tokio::time::interval(std::time::Duration::from_secs(20));
    loop {
        tokio::select! {
            _ = look.tick(), if await_keys => {
                if stored_keys(ctx).is_some() {
                    tracing::info!(instance = %ctx.instance(), "tuya cloud keys given in the dashboard");
                    return true;
                }
            }
            // `None` once the driver is cancelled.
            command = ctx.next_command() => {
                let Some(command) = command else { break };
                dispatch(command, orders, devices);
            }
            message = up.recv() => match message {
                Some(Up::State(index, values)) => {
                    let id = &devices[index].0;
                    for (key, value) in values {
                        ctx.set_state(id, &key, value);
                    }
                }
                Some(Up::Online(index, online)) => ctx.set_availability(&devices[index].0, online),
                Some(Up::Moved { index, from, to }) => {
                    let (id, dev) = &devices[index];
                    tracing::info!(instance = %ctx.instance(), device = dev.name.trim(), from, to, "tuya device answers at a new address");
                    ctx.journal_address(id, &from, &to);
                    if let Err(e) = write_address(devices_file, &dev.id, &to).await {
                        // Followed anyway until the next start.
                        tracing::warn!(file = %devices_file.display(), error = %e, "tuya device file not updated");
                    }
                }
                None => break,
            },
        }
    }
    false
}

/// Follows, through the cloud, the devices that never speak on the LAN.
fn start_cloud(
    config: &Config,
    keys: cloud::Keys,
    watched: cloud::Watched,
    up: mpsc::Sender<Up>,
    tasks: &mut tokio::task::JoinSet<()>,
) {
    if watched.is_empty() {
        return;
    }
    if let Some(region) = cloud::Region::named(&config.cloud_region) {
        tasks.spawn(cloud::watch(keys, region, watched, up));
    } else {
        tracing::warn!(region = %config.cloud_region, "tuya cloud: unknown region");
    }
}

/// Starts what finds the devices' addresses: their broadcasts, and the
/// search by key for those still without one.
fn locate(
    config: &Config,
    devices: &[(DeviceId, Arc<TuyaDevice>)],
    followed: HashMap<String, Followed>,
    wanted: Vec<seek::Wanted>,
    tasks: &mut tokio::task::JoinSet<()>,
) {
    // The network is the one the known devices are on.
    let known: HashSet<Ipv4Addr> = devices
        .iter()
        .filter_map(|(_, d)| d.ip.as_deref()?.parse().ok())
        .collect();
    if config.seek
        && !wanted.is_empty()
        && let Some(first) = known.iter().min()
    {
        tasks.spawn(seek::seek(seek::neighbours(*first), wanted, known.clone()));
    }
    if config.discovery {
        tasks.spawn(discover(followed));
    }
}

/// Hands a command to its device's link; the reply comes from the link.
fn dispatch(
    command: moli_runtime::CommandRequest,
    orders: &HashMap<DeviceId, (usize, mpsc::Sender<Order>)>,
    devices: &[(DeviceId, Arc<TuyaDevice>)],
) {
    let Some((index, sender)) = orders.get(&command.device.id) else {
        let known = devices.iter().any(|(id, _)| *id == command.device.id);
        command.reply(Err(if known {
            moli_i18n::tr!("pilotes.tuya.lecture_seule_cloud")
        } else {
            moli_i18n::tr!("pilotes.tuya.appareil_inconnu")
        }));
        return;
    };
    let raw = devices[*index]
        .1
        .dp_by_code(&command.key)
        .and_then(|dp| Some((dp.id, dp.encode(&command.value)?)));
    let Some((dp, raw)) = raw else {
        let message = moli_i18n::tr!("pilotes.tuya.ne_se_regle_pas", point = command.key);
        command.reply(Err(message));
        return;
    };
    let (done, outcome) = oneshot::channel();
    let mut dps = serde_json::Map::new();
    dps.insert(dp.to_string(), raw);
    let order = Order {
        dps,
        done,
        at: tokio::time::Instant::now(),
    };
    if sender.try_send(order).is_err() {
        command.reply(Err(moli_i18n::tr!("pilotes.tuya.occupe")));
        return;
    }
    tokio::spawn(async move {
        let result = outcome
            .await
            .unwrap_or_else(|_| Err(moli_i18n::tr!("pilotes.tuya.connexion_perdue")));
        command.reply(result);
    });
}

/// A device's address, as discovery keeps it.
#[derive(Clone)]
struct Followed {
    address: Arc<watch::Sender<Address>>,
    /// `ip` and `version` both in the device file.
    pinned: Option<Pinned>,
}

#[derive(Clone)]
struct Pinned {
    key: [u8; 16],
    reachable: Arc<AtomicBool>,
}

/// A pinned device announced elsewhere is asked there at most this often.
const CHECK_AGAIN: Duration = Duration::from_secs(60);

/// What one announcement does to a device.
#[derive(Debug, PartialEq, Eq)]
enum Heard {
    Nothing,
    /// Taken as it is (the announced version passes the link's floor).
    Take,
    /// A pinned device out of reach, announced elsewhere: taken only if it
    /// answers there with its key.
    Check,
}

impl Followed {
    /// Where `dev` is first, as its file says; what its link reads, and
    /// whether the link is up.
    fn start(dev: &TuyaDevice, key: [u8; 16]) -> (Self, watch::Receiver<Address>, Arc<AtomicBool>) {
        let start = dev
            .ip
            .clone()
            .map(|ip| (ip, dev.version.unwrap_or(Version::V33)));
        let (address, rx) = watch::channel(start);
        let reachable = Arc::new(AtomicBool::new(false));
        // Both in the file: a person said where and how; an announcement
        // moves it only once it is out of reach, and its key proved it.
        let pinned = (dev.ip.is_some() && dev.version.is_some()).then(|| Pinned {
            key,
            reachable: Arc::clone(&reachable),
        });
        let follow = Self {
            address: Arc::new(address),
            pinned,
        };
        (follow, rx, reachable)
    }

    fn heard(&self, ip: &str) -> Heard {
        let Some(pinned) = &self.pinned else {
            return Heard::Take;
        };
        let here = self
            .address
            .borrow()
            .as_ref()
            .is_some_and(|(at, _)| at == ip);
        if here || pinned.reachable.load(Ordering::Relaxed) {
            Heard::Nothing
        } else {
            Heard::Check
        }
    }

    /// Takes `ip`, announced in `version`. `false`: nothing changed.
    fn take(&self, id: &str, ip: &str, version: Version) -> bool {
        self.address.send_if_modified(|current| {
            let next = Some((ip.to_owned(), version));
            if *current == next {
                return false;
            }
            tracing::info!(device = id, ip, ?version, "tuya device found");
            *current = next;
            true
        })
    }

    /// A pinned device announced at `ip`: moved there, in the version its
    /// file gives, if it answers at `at` with its own key.
    async fn check(&self, id: &str, ip: &str, at: SocketAddr) -> bool {
        let Some(pinned) = &self.pinned else {
            return false;
        };
        let Some((old, version)) = self.address.borrow().clone() else {
            return false;
        };
        if !seek::identify(at, id, pinned.key, version).await {
            tracing::debug!(
                device = id,
                ip,
                "tuya announcement not confirmed by the device's key"
            );
            return false;
        }
        tracing::info!(
            device = id,
            old,
            new = ip,
            "tuya device found at a new address"
        );
        self.take(id, ip, version)
    }
}

/// A broadcast (`datagram`, from `from`) → the device it announces, where
/// it is and its version. Where the broadcast came from, not what it
/// claims: an `ip` that differs from the sender is not believed.
fn announcement(datagram: &[u8], from: SocketAddr) -> Option<(String, String, Version)> {
    let json = protocol::open_broadcast(datagram)?;
    let id = json.get("gwId")?.as_str()?;
    let version = json
        .get("version")
        .and_then(|v| v.as_str())
        .and_then(Version::parse)?;
    Some((id.to_owned(), from.ip().to_string(), version))
}

/// Listens to the devices' broadcasts (3.1–3.3 on 6666/6667, 3.4+ also on
/// 7000) and keeps every device's address and version up to date. A device
/// with both pinned in the file keeps them while it answers; out of reach,
/// it follows its announcements once its key proved them (`Followed::check`).
/// Anyone can broadcast: a device is never spoken to in an older version
/// than it proved, or than its file says (`device::Floor`).
async fn discover(followed: HashMap<String, Followed>) {
    let mut sockets = Vec::new();
    for port in [6666u16, 6667, 7000] {
        match broadcast_socket(port) {
            Ok(socket) => sockets.push(socket),
            Err(e) => tracing::warn!(port, error = %e, "tuya discovery port unavailable"),
        }
    }
    if sockets.is_empty() {
        // Keep the senders: links go on with the addresses they have.
        std::future::pending::<()>().await;
    }
    let (found_tx, mut found) = mpsc::channel::<(String, String, Version)>(64);
    for socket in sockets {
        let found_tx = found_tx.clone();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 2048];
            loop {
                let Ok((n, from)) = socket.recv_from(&mut buf).await else {
                    return;
                };
                let Some(heard) = announcement(&buf[..n], from) else {
                    continue;
                };
                if found_tx.send(heard).await.is_err() {
                    return;
                }
            }
        });
    }
    drop(found_tx);
    // (device, address) last asked, so a device announcing every few
    // seconds is not asked each time.
    let mut asked: HashMap<(String, String), Instant> = HashMap::new();
    let mut checks = tokio::task::JoinSet::new();
    while let Some((id, ip, version)) = found.recv().await {
        while checks.try_join_next().is_some() {}
        let Some(device) = followed.get(&id) else {
            continue;
        };
        match device.heard(&ip) {
            Heard::Nothing => {}
            Heard::Take => {
                device.take(&id, &ip, version);
            }
            Heard::Check => {
                let now = Instant::now();
                asked.retain(|_, at| now.duration_since(*at) < CHECK_AGAIN);
                if asked.contains_key(&(id.clone(), ip.clone())) {
                    continue;
                }
                let Ok(at) = ip.parse::<Ipv4Addr>() else {
                    continue;
                };
                asked.insert((id.clone(), ip.clone()), now);
                let device = device.clone();
                checks.spawn(async move {
                    device.check(&id, &ip, (at, seek::PORT).into()).await;
                });
            }
        }
    }
}

/// Writes `id`'s new address in the device file (the rest as the import
/// writes it), through a temporary file. Nothing to change: not written.
async fn write_address(file: &Path, id: &str, ip: &str) -> anyhow::Result<()> {
    let bytes = tokio::fs::read(file).await?;
    let Some(text) = with_address(&bytes, id, ip)? else {
        return Ok(());
    };
    let tmp = file.with_extension("json.tmp");
    tokio::fs::write(&tmp, text).await?;
    tokio::fs::rename(&tmp, file).await?;
    Ok(())
}

/// The device file with `id` at `ip`; `None` when it already is, or when
/// the file does not have the device or has no address for it (one a
/// person removed is not put back).
fn with_address(bytes: &[u8], id: &str, ip: &str) -> serde_json::Result<Option<String>> {
    let mut file: DeviceFile = serde_json::from_slice(bytes)?;
    let Some(device) = file.devices.iter_mut().find(|d| d.id == id) else {
        return Ok(None);
    };
    match &device.ip {
        Some(old) if old != ip => device.ip = Some(ip.to_owned()),
        _ => return Ok(None),
    }
    Ok(Some(serde_json::to_string_pretty(&file)? + "\n"))
}

/// Shared with anything else listening (Home Assistant's localtuya):
/// broadcasts reach every socket bound with SO_REUSEPORT.
fn broadcast_socket(port: u16) -> std::io::Result<UdpSocket> {
    let socket = socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::DGRAM, None)?;
    socket.set_reuse_address(true)?;
    #[cfg(unix)]
    socket.set_reuse_port(true)?;
    socket.set_broadcast(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)).into())?;
    UdpSocket::from_std(socket.into())
}

// ---- import ------------------------------------------------------------------

/// What the import script sends: devices with their local key.
#[derive(Debug, Deserialize)]
pub struct ImportFile {
    pub devices: Vec<ImportDevice>,
}

#[derive(Deserialize)]
pub struct ImportDevice {
    #[serde(flatten)]
    pub device: TuyaDevice,
    #[serde(default)]
    pub local_key: Option<String>,
}

impl std::fmt::Debug for ImportDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImportDevice")
            .field("device", &self.device)
            .field("local_key", &self.local_key.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// The device file after an import, and the keys to store. Hand-edited
/// fields (`ip`, `version`, `room`) of known devices are kept.
#[must_use]
pub fn merge_import(
    import: ImportFile,
    existing: Option<DeviceFile>,
) -> (DeviceFile, Vec<(String, String)>) {
    let previous: HashMap<String, TuyaDevice> = existing
        .map(|f| f.devices.into_iter().map(|d| (d.id.clone(), d)).collect())
        .unwrap_or_default();
    let mut keys = Vec::new();
    let devices = import
        .devices
        .into_iter()
        .map(
            |ImportDevice {
                 mut device,
                 local_key,
             }| {
                if let Some(key) = local_key.filter(|k| parse_key(k).is_some()) {
                    keys.push((key_name(&device.id), key));
                }
                if let Some(old) = previous.get(&device.id) {
                    device.ip = device.ip.or_else(|| old.ip.clone());
                    device.version = device.version.or(old.version);
                    device.room = device.room.or_else(|| old.room.clone());
                }
                device
            },
        )
        .collect();
    (DeviceFile { devices }, keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_file_defaults_to_the_data_dir() {
        let config: Config = serde_json::from_str("{}").unwrap();
        let tuya = Tuya::new(config, Path::new("/data"));
        assert_eq!(tuya.devices_file, Path::new("/data/tuya.json"));
        let config: Config =
            serde_json::from_str(r#"{"devices_file": "/elsewhere.json"}"#).unwrap();
        let tuya = Tuya::new(config, Path::new("/data"));
        assert_eq!(tuya.devices_file, Path::new("/elsewhere.json"));
    }

    #[test]
    fn imports_keep_hand_edits_and_separate_keys() {
        let import: ImportFile = serde_json::from_value(serde_json::json!({
            "devices": [
                { "id": "a", "name": "Salon", "local_key": "0123456789abcdef", "dps": [] },
                { "id": "b", "name": "Volet", "local_key": "short", "dps": [] },
            ]
        }))
        .unwrap();
        let existing =
            DeviceFile {
                devices: vec![serde_json::from_value(serde_json::json!({
                "id": "b", "name": "Volet", "room": "Chambre à coucher", "ip": "192.168.0.13"
            }))
            .unwrap()],
            };
        let (file, keys) = merge_import(import, Some(existing));
        assert_eq!(
            keys,
            vec![("key-a".to_owned(), "0123456789abcdef".to_owned())]
        );
        assert_eq!(file.devices[1].room.as_deref(), Some("Chambre à coucher"));
        assert_eq!(file.devices[1].ip.as_deref(), Some("192.168.0.13"));
        let text = serde_json::to_string(&file).unwrap();
        assert!(
            !text.contains("0123456789abcdef"),
            "no key in the device file"
        );
    }
    // ---- a device the box gave another address (fictitious announcements)

    const ID: &str = "bf00fictif0diffuseur";
    const KEY: [u8; 16] = *b"0123456789abcdef";

    fn followed(at: &str, pinned: bool) -> (Followed, watch::Receiver<Address>) {
        let (tx, rx) = watch::channel(Some((at.to_owned(), Version::V33)));
        let followed = Followed {
            address: Arc::new(tx),
            pinned: pinned.then(|| Pinned {
                key: KEY,
                reachable: Arc::new(AtomicBool::new(false)),
            }),
        };
        (followed, rx)
    }

    #[test]
    fn an_announcement_names_the_device_and_where_it_came_from() {
        let datagram = protocol::broadcast_33(
            format!(r#"{{"ip":"192.168.1.42","gwId":"{ID}","active":2,"encrypt":true,"productKey":"fictif","version":"3.3"}}"#)
                .as_bytes(),
        );
        let from: SocketAddr = ([192, 168, 1, 42], 6667).into();
        assert_eq!(
            announcement(&datagram, from),
            Some((ID.to_owned(), "192.168.1.42".to_owned(), Version::V33))
        );
        // What it claims is not believed: where it came from is.
        let elsewhere: SocketAddr = ([192, 168, 1, 7], 6667).into();
        assert_eq!(
            announcement(&datagram, elsewhere).map(|(_, ip, _)| ip),
            Some("192.168.1.7".to_owned())
        );
        // Without an id or a version, or not a broadcast at all: nothing.
        let anonymous = protocol::broadcast_33(br#"{"ip":"192.168.1.42","version":"3.3"}"#);
        assert_eq!(announcement(&anonymous, from), None);
        assert_eq!(announcement(b"not a broadcast", from), None);
    }

    #[test]
    fn a_pinned_device_follows_its_announcements_only_out_of_reach() {
        // Not pinned: followed as before.
        let (free, _rx) = followed("192.168.1.10", false);
        assert_eq!(free.heard("192.168.1.42"), Heard::Take);
        // Pinned and announced where it is: nothing to do.
        let (pinned, _rx) = followed("192.168.1.10", true);
        assert_eq!(pinned.heard("192.168.1.10"), Heard::Nothing);
        // Out of reach, announced elsewhere: asked there.
        assert_eq!(pinned.heard("192.168.1.42"), Heard::Check);
        // Answering where the file says: an announcement elsewhere is ignored.
        let reachable = &pinned.pinned.as_ref().unwrap().reachable;
        reachable.store(true, Ordering::Relaxed);
        assert_eq!(pinned.heard("192.168.1.42"), Heard::Nothing);
    }

    #[tokio::test]
    async fn a_pinned_device_moves_once_its_key_answers_there() {
        // The device, now at another address (loopback stands for it).
        let port = seek::fake_device(KEY).await;
        let there: SocketAddr = ([127, 0, 0, 1], port).into();

        // Someone else announcing it, without its key: not taken.
        let (pinned, rx) = followed("192.168.1.10", true);
        let mut impostor = pinned.clone();
        impostor.pinned.as_mut().unwrap().key = *b"fedcba9876543210";
        assert!(!impostor.check(ID, "127.0.0.1", there).await);
        assert_eq!(rx.borrow().as_ref().unwrap().0, "192.168.1.10");

        // The device itself: taken, in the version of its file.
        assert!(pinned.check(ID, "127.0.0.1", there).await);
        assert_eq!(*rx.borrow(), Some(("127.0.0.1".to_owned(), Version::V33)));
    }

    #[test]
    fn the_new_address_is_written_in_the_device_file() {
        let file = serde_json::json!({ "devices": [
            { "id": ID, "name": "Diffuseur", "category": "xxj", "ip": "192.168.1.10",
              "version": "3.3", "room": "Salon", "dps": [] },
            { "id": "bf00fictif0prise0001", "name": "Prise", "ip": "192.168.1.11", "dps": [] },
            { "id": "bf00fictif0volet0001", "name": "Volet", "dps": [] },
        ]})
        .to_string();
        let text = with_address(file.as_bytes(), ID, "192.168.1.42")
            .unwrap()
            .expect("moved");
        let written = DeviceFile::parse(text.as_bytes()).unwrap();
        assert_eq!(written.devices[0].ip.as_deref(), Some("192.168.1.42"));
        // The rest as it was.
        assert_eq!(written.devices[0].version, Some(Version::V33));
        assert_eq!(written.devices[0].room.as_deref(), Some("Salon"));
        assert_eq!(written.devices[1].ip.as_deref(), Some("192.168.1.11"));
        assert_eq!(written.devices[2].ip, None);
        // Already there, unknown, or without an address in the file: unchanged.
        assert_eq!(
            with_address(text.as_bytes(), ID, "192.168.1.42").unwrap(),
            None
        );
        assert_eq!(
            with_address(file.as_bytes(), "bf00fictif0inconnu01", "192.168.1.42").unwrap(),
            None
        );
        assert_eq!(
            with_address(file.as_bytes(), "bf00fictif0volet0001", "192.168.1.42").unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn the_device_file_is_rewritten_in_place() {
        let dir = std::env::temp_dir().join(format!("moli-tuya-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tuya.json");
        let file = serde_json::json!({ "devices": [
            { "id": ID, "name": "Diffuseur", "ip": "192.168.1.10", "version": "3.3", "dps": [] },
        ]});
        std::fs::write(&path, file.to_string()).unwrap();
        write_address(&path, ID, "192.168.1.42").await.unwrap();
        let written = DeviceFile::parse(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(written.devices[0].ip.as_deref(), Some("192.168.1.42"));
        assert!(!dir.join("tuya.json.tmp").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
