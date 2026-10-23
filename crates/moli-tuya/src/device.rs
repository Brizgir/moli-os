//! One Tuya device: a TCP connection kept open (6668), its session key,
//! status pushes, heartbeats, orders. Reconnects on its own.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use moli_core::Value;
use serde_json::{Map, Value as Json, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::{Instant, timeout};

use crate::model::TuyaDevice;
use crate::protocol::{Codec, Frame, Version, cmd};

const PORT: u16 = 6668;
const CONNECT: Duration = Duration::from_secs(5);
const REPLY: Duration = Duration::from_secs(5);
const HEARTBEAT: Duration = Duration::from_secs(10);
/// Silent this long: the connection is dead.
const SILENCE: Duration = Duration::from_secs(35);
const BACKOFF_MAX: Duration = Duration::from_secs(300);
/// Plugs only report power when asked (or when it changes a lot).
const REFRESH_POWER: Duration = Duration::from_secs(30);
const POWER_DPS: [u32; 3] = [18, 19, 20];

/// Where the device is, from the device file or discovery.
pub type Address = Option<(String, Version)>;

/// What a device task reports to the driver.
#[derive(Debug)]
pub enum Up {
    State(usize, Vec<(String, Value)>),
    Online(usize, bool),
    /// The device proved, with its key, that it answers at `to` now, no
    /// longer at `from` (the address in the device file).
    Moved {
        index: usize,
        from: String,
        to: String,
    },
}

/// An order: DP values to set, and who waits for the outcome.
#[derive(Debug)]
pub struct Order {
    pub dps: Map<String, Json>,
    pub done: oneshot::Sender<Result<(), String>>,
    pub at: Instant,
}

/// Orders sent, waiting for the device's acknowledgement.
type Pending = Vec<(Instant, oneshot::Sender<Result<(), String>>)>;

/// An order older than this is never sent: the caller gave up, and a
/// heater switched on hours later is worse than a refusal.
const ORDER_TTL: Duration = Duration::from_secs(5);

impl Order {
    fn stale(&self) -> bool {
        self.at.elapsed() > ORDER_TTL || self.done.is_closed()
    }

    fn refuse(self, why: String) {
        let _ = self.done.send(Err(why));
    }
}

pub struct Link {
    pub index: usize,
    pub device: Arc<TuyaDevice>,
    pub key: [u8; 16],
    pub address: watch::Receiver<Address>,
    pub orders: mpsc::Receiver<Order>,
    pub up: mpsc::Sender<Up>,
    /// Whether a session is up: discovery looks for a pinned device only
    /// when it is not.
    pub reachable: Arc<AtomicBool>,
}

#[derive(Debug, thiserror::Error)]
enum LinkError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Protocol(#[from] crate::protocol::ProtocolError),
    #[error("no answer")]
    Timeout,
}

/// The oldest protocol version a device is spoken to in: what its file
/// says (3.4+), then what it proved by negotiating a session. 3.3 has no
/// device proof: a broadcast, which anyone on the network can send, may
/// move a device's address, never bring it back to an older version.
#[derive(Debug)]
struct Floor {
    version: Option<Version>,
    /// An older announcement is said once in the log.
    said: bool,
}

impl Floor {
    fn new(file: Option<Version>) -> Self {
        Self {
            version: file.filter(|v| v.negotiates()),
            said: false,
        }
    }

    /// The device negotiated a session in `version`: it holds its key.
    fn proved(&mut self, version: Version) {
        self.version = self.version.max(Some(version));
    }

    /// The version to speak to `device`, announced in `announced`.
    fn version(&mut self, device: &str, announced: Version) -> Version {
        match self.version {
            Some(floor) if announced < floor => {
                if !self.said {
                    self.said = true;
                    tracing::warn!(
                        device,
                        ?announced,
                        kept = ?floor,
                        "tuya device announced with an older protocol version: ignored"
                    );
                }
                floor
            }
            _ => announced,
        }
    }
}

/// A diffuser's timer, as the device itself reports it. When it runs out
/// the device switches its light off (`switch_led`) and resets `countdown`
/// and `countdown_left`, but leaves `switch` and `mist_mode` as they are:
/// the mist goes on, while the app promises the device goes off. Moli
/// finishes the job. A timer Moli itself just wrote (« 0 »: a person
/// cancelled it) is not an end: the diffuser goes on.
#[derive(Debug)]
struct MistTimer {
    /// DP ids, as the frames carry them.
    switch: String,
    countdown: String,
    left: String,
    /// What the device last said.
    on: Option<bool>,
    program: Option<String>,
    minutes: Option<i64>,
    /// `countdown_left` went from > 0 to 0, and nothing was done about it.
    ran_out: bool,
    /// Moli wrote `countdown` since the device last showed a running timer.
    written: bool,
}

impl MistTimer {
    /// Only a device with a mist, a main switch and the timer pair.
    fn new(device: &TuyaDevice) -> Option<Self> {
        device.dp_by_code("mist_mode")?;
        let switch = device
            .dp_by_code("switch")
            .filter(|dp| dp.kind == crate::model::DpKind::Boolean)?;
        let countdown = device.dp_by_code("countdown")?;
        let left = device.dp_by_code("countdown_left")?;
        Some(Self {
            switch: switch.id.to_string(),
            countdown: countdown.id.to_string(),
            left: left.id.to_string(),
            on: None,
            program: None,
            minutes: None,
            ran_out: false,
            written: false,
        })
    }

    /// An order of Moli leaves: what the device reports next is its doing.
    fn written(&mut self, dps: &Map<String, Json>) {
        if dps.contains_key(&self.countdown) {
            self.written = true;
        }
    }

    /// A frame from the device. `true`: its timer just ran out on its own
    /// and the mist is still on; the driver switches the device off.
    fn observe(&mut self, command: u32, dps: &Map<String, Json>) -> bool {
        if let Some(Json::Bool(on)) = dps.get(&self.switch) {
            self.on = Some(*on);
        }
        if let Some(Json::String(program)) = dps.get(&self.countdown) {
            self.program = Some(program.clone());
        }
        if let Some(minutes) = dps.get(&self.left).and_then(Json::as_i64) {
            let was_running = self.minutes.is_some_and(|m| m > 0);
            self.minutes = Some(minutes);
            if minutes > 0 {
                // A timer runs: whatever Moli wrote before is behind us.
                self.ran_out = false;
                self.written = false;
            } else if was_running {
                self.ran_out = true;
            }
        }
        // Only what the device says on its own, once the timer is cleared:
        // the reply to a query or an order is never an end.
        if command != cmd::STATUS || !self.ran_out || self.program.as_deref() != Some("0") {
            return false;
        }
        self.ran_out = false;
        if std::mem::take(&mut self.written) {
            return false;
        }
        self.on != Some(false)
    }

    /// The order that finishes the job: main switch off.
    fn off(&self) -> Map<String, Json> {
        let mut dps = Map::new();
        dps.insert(self.switch.clone(), Json::Bool(false));
        dps
    }
}

impl Link {
    pub async fn run(mut self) {
        let mut backoff = Duration::from_secs(2);
        let mut online = None;
        // Discovery gone (its ports taken): the address only comes from
        // the device file; never a reason to stop.
        let mut watching = true;
        let mut floor = Floor::new(self.device.version);
        // Where the device file has it: an address proven elsewhere is
        // reported once (only an address the file had is ever corrected).
        let mut settled = self.device.ip.clone();
        loop {
            let Some((ip, announced)) = self.address.borrow().clone() else {
                if online.is_none() {
                    self.set_online(&mut online, false).await;
                }
                if !self.idle(None, &mut watching).await {
                    return;
                }
                continue;
            };
            let version = floor.version(&self.device.name, announced);
            let started = Instant::now();
            let result = self
                .session(&ip, version, (&mut online, &mut settled), &mut floor)
                .await;
            if self.up.is_closed() {
                return;
            }
            if online != Some(false) {
                self.set_online(&mut online, false).await;
            }
            if let Err(e) = result {
                tracing::debug!(device = self.device.name, ip, error = %e, "tuya link down");
            }
            // A connection that lived a while was not the device's fault.
            if started.elapsed() > Duration::from_secs(60) {
                backoff = Duration::from_secs(2);
            }
            if !self.idle(Some(backoff), &mut watching).await {
                return;
            }
            backoff = (backoff * 2).min(BACKOFF_MAX);
        }
    }

    /// Says the device came online or went away.
    async fn set_online(&self, online: &mut Option<bool>, now: bool) {
        *online = Some(now);
        self.reachable.store(now, Ordering::Relaxed);
        let _ = self.up.send(Up::Online(self.index, now)).await;
    }

    /// The device proved its key at `ip`: reported once if the device file
    /// has it elsewhere.
    async fn settle(&self, ip: &str, settled: &mut Option<String>) {
        let Some(from) = settled.as_deref().filter(|from| *from != ip) else {
            return;
        };
        let moved = Up::Moved {
            index: self.index,
            from: from.to_owned(),
            to: ip.to_owned(),
        };
        ip.clone_into(settled.get_or_insert_default());
        let _ = self.up.send(moved).await;
    }

    /// Away from the device: refuse orders until the backoff ends (if any)
    /// or the address changes. `false`: the driver is gone.
    async fn idle(&mut self, backoff: Option<Duration>, watching: &mut bool) -> bool {
        let wake = tokio::time::sleep(backoff.unwrap_or(Duration::from_secs(86_400)));
        tokio::pin!(wake);
        loop {
            tokio::select! {
                () = &mut wake, if backoff.is_some() => return true,
                changed = self.address.changed(), if *watching => {
                    if changed.is_err() {
                        *watching = false;
                        continue;
                    }
                    return true;
                }
                order = self.orders.recv() => match order {
                    Some(order) => order.refuse(moli_i18n::tr!(
                        "pilotes.tuya.injoignable",
                        name = self.device.name.trim()
                    )),
                    None => return false,
                },
            }
        }
    }

    /// One connection, until it fails.
    async fn session(
        &mut self,
        ip: &str,
        version: Version,
        (online, settled): (&mut Option<bool>, &mut Option<String>),
        floor: &mut Floor,
    ) -> Result<(), LinkError> {
        let stream = timeout(CONNECT, TcpStream::connect((ip, PORT)))
            .await
            .map_err(|_| LinkError::Timeout)??;
        stream.set_nodelay(true)?;
        let (mut reader, mut writer) = stream.into_split();
        let mut codec = Codec::new(version, self.key);
        let mut buf = Vec::with_capacity(1024);

        if version.negotiates() {
            let (start, local) = codec.session_start();
            writer.write_all(&start).await?;
            let answer = loop {
                let frame = next_frame(&codec, &mut reader, &mut buf).await?;
                if frame.cmd == cmd::SESS_KEY_NEG_RESP {
                    break frame;
                }
            };
            let finish = codec.session_finish(local, &answer)?;
            floor.proved(version);
            writer.write_all(&finish).await?;
            self.settle(ip, settled).await;
        }
        // 3.3: the first payload opened with the key is the proof.
        let mut proved = version.negotiates();

        let id = self.device.id.as_str();
        let query = if version == Version::V33 {
            codec.encode(
                cmd::DP_QUERY,
                json!({"gwId": id, "devId": id, "uid": id, "t": now()})
                    .to_string()
                    .as_bytes(),
            )
        } else {
            codec.encode(cmd::DP_QUERY_NEW, b"{}")
        };
        writer.write_all(&query).await?;

        let wants_power = self.device.dps.iter().any(|d| POWER_DPS.contains(&d.id));
        let mut mist = MistTimer::new(&self.device);
        let mut heartbeat = tokio::time::interval(HEARTBEAT);
        heartbeat.tick().await;
        let mut refresh = tokio::time::interval(REFRESH_POWER);
        refresh.tick().await;
        let mut heard = Instant::now();
        // Orders sent, waiting for the device's acknowledgement.
        let mut pending: Pending = Vec::new();
        loop {
            let mut chunk = [0u8; 2048];
            tokio::select! {
                read = reader.read(&mut chunk) => {
                    let n = read?;
                    if n == 0 {
                        return Err(LinkError::Io(std::io::ErrorKind::UnexpectedEof.into()));
                    }
                    heard = Instant::now();
                    buf.extend_from_slice(&chunk[..n]);
                    while let Some(frame) = codec.decode(&mut buf)? {
                        if *online != Some(true) {
                            self.set_online(online, true).await;
                        }
                        acknowledged(&frame, &mut pending);
                        if self.heard(&mut codec, &mut writer, &frame, mist.as_mut(), &mut pending).await? && !proved {
                            proved = true;
                            self.settle(ip, settled).await;
                        }
                    }
                }
                _ = heartbeat.tick() => {
                    if heard.elapsed() > SILENCE {
                        return Err(LinkError::Timeout);
                    }
                    let id = self.device.id.as_str();
                    writer.write_all(&codec.encode(cmd::HEART_BEAT, json!({"gwId": id, "devId": id}).to_string().as_bytes())).await?;
                }
                _ = refresh.tick(), if wants_power => {
                    writer.write_all(&codec.encode(cmd::UPDATEDPS, br#"{"dpId":[18,19,20]}"#)).await?;
                }
                () = tokio::time::sleep_until(pending.first().map_or_else(Instant::now, |(t, _)| *t + REPLY)), if !pending.is_empty() => {
                    let (_, done) = pending.remove(0);
                    let _ = done.send(Err(moli_i18n::tr!("pilotes.tuya.sans_accuse")));
                }
                order = self.orders.recv() => {
                    let Some(order) = order else { return Ok(()) };
                    if order.stale() {
                        order.refuse(moli_i18n::tr!("pilotes.tuya.expire"));
                        continue;
                    }
                    if let Some(timer) = mist.as_mut() {
                        timer.written(&order.dps);
                    }
                    send_order(&mut codec, &mut writer, id, &order.dps).await?;
                    pending.push((Instant::now(), order.done));
                }
            }
        }
    }

    /// A frame from the device: its DPs are published. `true`: its payload
    /// opened with the device's key. A diffuser whose timer just ran out on
    /// its own, mist still on, is switched off by the driver's own order, as
    /// the app promises. Not a person's nor an automation's doing: the log
    /// says so.
    async fn heard(
        &self,
        codec: &mut Codec,
        writer: &mut OwnedWriteHalf,
        frame: &Frame,
        mist: Option<&mut MistTimer>,
        pending: &mut Pending,
    ) -> Result<bool, LinkError> {
        let dps = self.report(codec, frame).await;
        let opened = dps.is_some();
        let (Some(dps), Some(timer)) = (dps, mist) else {
            return Ok(opened);
        };
        if !timer.observe(frame.cmd, &dps) {
            return Ok(opened);
        }
        tracing::info!(
            device = self.device.name,
            origin = "driver",
            "tuya diffuser timer ran out: switching it off, as the app promises"
        );
        let (done, outcome) = oneshot::channel();
        send_order(codec, writer, &self.device.id, &timer.off()).await?;
        pending.push((Instant::now(), done));
        let name = self.device.name.clone();
        tokio::spawn(async move {
            let failed = match outcome.await {
                Ok(Ok(())) => return,
                Ok(Err(why)) => why,
                Err(_) => moli_i18n::tr!("pilotes.tuya.connexion_perdue"),
            };
            tracing::warn!(
                device = name,
                origin = "driver",
                error = %failed,
                "tuya diffuser not switched off after its timer"
            );
        });
        Ok(true)
    }

    /// Publishes the DPs a frame carries, and hands them back raw (`Some`:
    /// the payload opened with the device's key).
    async fn report(&self, codec: &Codec, frame: &Frame) -> Option<Map<String, Json>> {
        if !matches!(
            frame.cmd,
            cmd::STATUS
                | cmd::DP_QUERY
                | cmd::DP_QUERY_NEW
                | cmd::CONTROL
                | cmd::CONTROL_NEW
                | cmd::UPDATEDPS
        ) {
            return None;
        }
        let json = match codec.open_json(&frame.payload) {
            Ok(Some(json)) => json,
            Ok(None) => return None,
            Err(e) => {
                tracing::debug!(device = self.device.name, error = %e, "unreadable tuya payload");
                return None;
            }
        };
        // 3.4+ nest the DPs under `data`.
        let dps = json
            .get("dps")
            .or_else(|| json.get("data").and_then(|d| d.get("dps")))
            .and_then(Json::as_object)?;
        let values = self.device.decode_dps(dps);
        if !values.is_empty() {
            let _ = self.up.send(Up::State(self.index, values)).await;
        }
        Some(dps.clone())
    }
}

/// The device took the order, or refused it: those waiting are told.
fn acknowledged(frame: &Frame, pending: &mut Pending) {
    if !matches!(frame.cmd, cmd::CONTROL | cmd::CONTROL_NEW) {
        return;
    }
    let outcome = match frame.retcode {
        Some(0) | None => Ok(()),
        Some(code) => Err(moli_i18n::tr!("pilotes.tuya.refuse", code = code)),
    };
    for (_, done) in pending.drain(..) {
        let _ = done.send(outcome.clone());
    }
}

async fn send_order(
    codec: &mut Codec,
    writer: &mut OwnedWriteHalf,
    id: &str,
    dps: &Map<String, Json>,
) -> Result<(), LinkError> {
    let frame = if codec.version == Version::V33 {
        codec.encode(
            cmd::CONTROL,
            json!({"devId": id, "uid": id, "t": now(), "dps": dps})
                .to_string()
                .as_bytes(),
        )
    } else {
        codec.encode(
            cmd::CONTROL_NEW,
            json!({"protocol": 5, "t": now().parse::<u64>().unwrap_or(0), "data": {"dps": dps}})
                .to_string()
                .as_bytes(),
        )
    };
    writer.write_all(&frame).await?;
    Ok(())
}

async fn next_frame(
    codec: &Codec,
    reader: &mut tokio::net::tcp::OwnedReadHalf,
    buf: &mut Vec<u8>,
) -> Result<Frame, LinkError> {
    let deadline = Instant::now() + REPLY;
    loop {
        if let Some(frame) = codec.decode(buf)? {
            return Ok(frame);
        }
        let mut chunk = [0u8; 1024];
        let n = tokio::time::timeout_at(deadline, reader.read(&mut chunk))
            .await
            .map_err(|_| LinkError::Timeout)??;
        if n == 0 {
            return Err(LinkError::Io(std::io::ErrorKind::UnexpectedEof.into()));
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

fn now() -> String {
    (moli_core::now_ms() / 1000).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    const KEY: [u8; 16] = *b"0123456789abcdef";

    /// An aroma diffuser as the cloud describes it, completed by the driver.
    fn diffuser() -> TuyaDevice {
        let mut device: TuyaDevice = serde_json::from_value(json!({
            "id": "bf000000000000000001", "name": "Diffuseur", "category": "jsq", "version": "3.3",
            "dps": [
                { "id": 1, "code": "switch", "type": "Boolean", "writable": true },
                { "id": 11, "code": "switch_led", "type": "Boolean", "writable": true },
                { "id": 13, "code": "countdown", "type": "String" },
                { "id": 14, "code": "countdown_left", "type": "Integer", "unit": "min" },
                { "id": 103, "code": "mist_mode", "type": "String" },
            ]
        }))
        .unwrap();
        device.complete();
        device
    }

    fn dps(json: Json) -> Map<String, Json> {
        serde_json::from_value(json).unwrap()
    }

    /// The timer set an hour ago, a minute left: what a query shows.
    fn running() -> MistTimer {
        let mut timer = MistTimer::new(&diffuser()).unwrap();
        assert!(!timer.observe(
            cmd::DP_QUERY,
            &dps(json!({"1": true, "11": true, "13": "1", "14": 1, "103": "small"}))
        ));
        timer
    }

    /// What the device reports when its timer runs out: light off, timer
    /// cleared, mist untouched.
    fn ran_out() -> Map<String, Json> {
        dps(json!({"11": false, "13": "0", "14": 0}))
    }

    #[test]
    fn a_timer_that_runs_out_switches_the_diffuser_off() {
        let mut timer = running();
        assert!(timer.observe(cmd::STATUS, &ran_out()));
        assert_eq!(timer.off(), dps(json!({"1": false})));
        // Said once: the same report again, or the device's answer to the
        // order, change nothing.
        assert!(!timer.observe(cmd::STATUS, &ran_out()));
        assert!(!timer.observe(cmd::CONTROL, &dps(json!({"1": false}))));
    }

    #[test]
    fn a_timer_moli_cancelled_leaves_the_diffuser_on() {
        let mut timer = running();
        // A person cancels the timer in Moli: `countdown` = « 0 » goes out,
        // the device confirms with the same report as an end.
        timer.written(&dps(json!({"13": "0"})));
        assert!(!timer.observe(cmd::STATUS, &ran_out()));
        // Later a timer is set again and runs out: the device goes off.
        assert!(!timer.observe(cmd::STATUS, &dps(json!({"13": "1", "14": 60}))));
        assert!(!timer.observe(cmd::STATUS, &dps(json!({"14": 1}))));
        assert!(timer.observe(cmd::STATUS, &ran_out()));
    }

    #[test]
    fn a_timer_moli_set_is_not_a_cancellation() {
        let mut timer = running();
        // Moli sets a new timer: the device shows it running, then it ends.
        timer.written(&dps(json!({"13": "1"})));
        assert!(!timer.observe(cmd::STATUS, &dps(json!({"13": "1", "14": 60}))));
        assert!(timer.observe(cmd::STATUS, &ran_out()));
    }

    #[test]
    fn an_end_is_only_what_the_device_says_on_its_own() {
        // The timer cleared in a reply to Moli's query: not an end seen live.
        let mut timer = running();
        assert!(!timer.observe(cmd::DP_QUERY, &ran_out()));
        // Already off (a person switched it off, the device cleared its
        // timer): nothing to finish.
        let mut timer = running();
        assert!(!timer.observe(
            cmd::STATUS,
            &dps(json!({"1": false, "11": false, "13": "0", "14": 0}))
        ));
        // No timer ever seen running: a cleared timer is not an end.
        let mut timer = MistTimer::new(&diffuser()).unwrap();
        assert!(!timer.observe(cmd::STATUS, &ran_out()));
    }

    #[test]
    fn a_device_without_mist_has_no_timer_to_finish() {
        let plug: TuyaDevice = serde_json::from_value(json!({
            "id": "bf000000000000000002", "name": "Prise", "category": "cz",
            "dps": [
                { "id": 1, "code": "switch", "type": "Boolean", "writable": true },
                { "id": 13, "code": "countdown", "type": "Integer", "writable": true },
                { "id": 14, "code": "countdown_left", "type": "Integer" },
            ]
        }))
        .unwrap();
        assert!(MistTimer::new(&plug).is_none());
        let mut no_timer = diffuser();
        no_timer.dps.retain(|dp| dp.code != "countdown_left");
        assert!(MistTimer::new(&no_timer).is_none());
    }

    /// A frame as a device sends it: a return code after the header.
    fn from_device(command: u32, payload: &[u8]) -> Vec<u8> {
        let mut frame = Codec::new(Version::V33, KEY).encode(command, payload);
        frame.splice(16..16, [0, 0, 0, 0]);
        let total = frame.len();
        frame[12..16].copy_from_slice(&u32::try_from(total - 16).unwrap().to_be_bytes());
        let crc = crc32fast::hash(&frame[..total - 8]).to_be_bytes();
        frame[total - 8..total - 4].copy_from_slice(&crc);
        frame
    }

    /// What Moli sent: the command and the JSON it carries.
    async fn request(stream: &mut TcpStream) -> (u32, Json) {
        let mut header = [0; 16];
        stream.read_exact(&mut header).await.unwrap();
        let size = u32::from_be_bytes(header[12..16].try_into().unwrap()) as usize;
        let mut body = vec![0; size];
        stream.read_exact(&mut body).await.unwrap();
        let json = Codec::new(Version::V33, KEY)
            .open_json(&body[..size - 8])
            .unwrap()
            .unwrap_or(Json::Null);
        (u32::from_be_bytes(header[8..12].try_into().unwrap()), json)
    }

    #[tokio::test]
    async fn on_the_wire_the_end_of_a_timer_becomes_a_switch_off_order() {
        let listener = TcpListener::bind(("127.0.0.31", PORT)).await.unwrap();
        let device = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            assert_eq!(request(&mut stream).await.0, cmd::DP_QUERY);
            stream
                .write_all(&from_device(
                    cmd::DP_QUERY,
                    br#"{"dps":{"1":true,"11":true,"13":"1","14":1,"103":"small"}}"#,
                ))
                .await
                .unwrap();
            // A minute later: the light goes off, the mist does not.
            stream
                .write_all(&from_device(
                    cmd::STATUS,
                    br#"{"dps":{"11":false,"13":"0","14":0}}"#,
                ))
                .await
                .unwrap();
            let (command, json) = request(&mut stream).await;
            assert_eq!(command, cmd::CONTROL);
            assert_eq!(json["dps"], json!({"1": false}));
            stream
                .write_all(&from_device(cmd::CONTROL, b""))
                .await
                .unwrap();
            // Nothing else follows (the heartbeat is far away).
            assert!(
                timeout(Duration::from_millis(200), stream.read(&mut [0; 64]))
                    .await
                    .is_err()
            );
        });
        let (up, mut events) = mpsc::channel(8);
        let (_, address) = watch::channel(None);
        let (_orders, orders) = mpsc::channel(1);
        let mut link = Link {
            index: 0,
            device: Arc::new(diffuser()),
            key: KEY,
            address,
            orders,
            up,
            reachable: Arc::new(AtomicBool::new(false)),
        };
        let session = tokio::spawn(async move {
            link.session(
                "127.0.0.31",
                Version::V33,
                (&mut Some(false), &mut None),
                &mut Floor::new(None),
            )
            .await
        });
        assert!(matches!(events.recv().await, Some(Up::Online(0, true))));
        assert!(matches!(events.recv().await, Some(Up::State(0, _))));
        assert!(
            matches!(events.recv().await, Some(Up::State(0, values)) if values.contains(&("countdown_left".into(), Value::Int(0))))
        );
        device.await.unwrap();
        session.abort();
    }

    #[test]
    fn a_broadcast_never_lowers_the_version_a_device_proved() {
        // Nothing known yet: the announcement decides.
        let mut floor = Floor::new(None);
        assert_eq!(floor.version("prise", Version::V33), Version::V33);
        // A 3.5 session negotiated: 3.3 or 3.4 announced, 3.5 spoken.
        floor.proved(Version::V35);
        assert_eq!(floor.version("prise", Version::V33), Version::V35);
        assert_eq!(floor.version("prise", Version::V34), Version::V35);
        // The device file says 3.4: never 3.3, a newer one is taken.
        let mut floor = Floor::new(Some(Version::V34));
        assert_eq!(floor.version("prise", Version::V33), Version::V34);
        assert_eq!(floor.version("prise", Version::V35), Version::V35);
        // 3.3 in the file proves nothing.
        let mut floor = Floor::new(Some(Version::V33));
        assert_eq!(floor.version("prise", Version::V33), Version::V33);
    }
    fn link(ip: Option<&str>) -> (Link, mpsc::Receiver<Up>) {
        let device: TuyaDevice = serde_json::from_value(json!({
            "id": "bf00fictif0diffuseur", "name": "Diffuseur", "ip": ip, "dps": []
        }))
        .unwrap();
        let (up, rx) = mpsc::channel(8);
        let (_, address) = watch::channel(None);
        let (_, orders) = mpsc::channel(1);
        let link = Link {
            index: 3,
            device: Arc::new(device),
            key: *b"0123456789abcdef",
            address,
            orders,
            up,
            reachable: Arc::new(AtomicBool::new(false)),
        };
        (link, rx)
    }

    #[tokio::test]
    async fn an_address_proven_elsewhere_than_the_file_is_reported_once() {
        let (link, mut rx) = link(Some("192.168.1.10"));
        let mut settled = link.device.ip.clone();
        // Where the file says: nothing to report.
        link.settle("192.168.1.10", &mut settled).await;
        assert!(rx.try_recv().is_err());
        // Elsewhere: reported, then known.
        link.settle("192.168.1.42", &mut settled).await;
        let Ok(Up::Moved { index, from, to }) = rx.try_recv() else {
            panic!("expected a move");
        };
        assert_eq!(
            (index, from.as_str(), to.as_str()),
            (3, "192.168.1.10", "192.168.1.42")
        );
        link.settle("192.168.1.42", &mut settled).await;
        assert!(rx.try_recv().is_err());
        // No address in the file: none is added.
        let (link, mut rx) = self::link(None);
        let mut settled = None;
        link.settle("192.168.1.42", &mut settled).await;
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn being_online_is_shared_with_discovery() {
        let (link, mut rx) = link(Some("192.168.1.10"));
        let mut online = None;
        link.set_online(&mut online, true).await;
        assert!(link.reachable.load(Ordering::Relaxed));
        assert!(matches!(rx.try_recv(), Ok(Up::Online(3, true))));
        link.set_online(&mut online, false).await;
        assert!(!link.reachable.load(Ordering::Relaxed));
    }
}
