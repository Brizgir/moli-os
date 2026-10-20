//! What Ring's answers say, in Moli's words: the account's doorbells,
//! cameras and chimes (`ring_devices`), their health (Wi-Fi, battery),
//! their latest events (a press, a motion), with what makes an event new
//! enough to tell, whether read or pushed by Ring ([`pushed`]); when their
//! last snapshot was taken; and the mode of a location (disarmed, home,
//! away), as the app's « Modes » show it.

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use moli_core::{Access, Device, DeviceId, Kind, PointSpec, Semantic, Unit};
use moli_runtime::DriverCtx;
use serde_json::Value as Json;

/// What a Ring device is to Moli.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Family {
    Doorbell,
    Camera,
    Chime,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RingDevice {
    /// Ring's own number for the device.
    pub id: u64,
    pub family: Family,
    /// The name given in the Ring app.
    pub name: String,
    /// Ring's model code (`doorbell_v3`, `chime_pro`…).
    pub model: String,
    pub location: String,
    pub online: bool,
    /// Percent, for a device on battery.
    pub battery: Option<f64>,
    /// A chime's volume, 0 to 11.
    pub volume: Option<f64>,
    /// The floodlight or spotlight is on (`None`: the camera has none).
    pub light: Option<bool>,
    /// The siren sounds (`None`: the camera has none).
    pub siren: Option<bool>,
    /// Motion detection is on (`None`: Ring does not say).
    pub motion_detection: Option<bool>,
}

impl RingDevice {
    /// Where its health is read.
    pub(crate) fn health_path(&self) -> String {
        match self.family {
            Family::Chime => format!("/clients_api/chimes/{}/health", self.id),
            Family::Doorbell | Family::Camera => {
                format!("/clients_api/doorbots/{}/health", self.id)
            }
        }
    }

    /// Where its latest events are read (cameras and doorbells).
    pub(crate) fn events_path(&self) -> String {
        format!(
            "/clients_api/locations/{}/devices/{}/events?limit=5",
            self.location, self.id
        )
    }

    /// The older place for the same events.
    pub(crate) fn history_path(&self) -> String {
        format!("/clients_api/doorbots/{}/history?limit=5", self.id)
    }

    /// On battery: an image wakes it, so one is asked seldom.
    pub(crate) fn on_battery(&self) -> bool {
        self.battery.is_some()
    }
}

/// Where a location's mode is read and set (the app's own API).
pub(crate) fn mode_path(location: &str) -> String {
    format!("/api/v1/mode/location/{location}")
}

/// `POST snapshots/timestamps`, `PUT snapshots/update`: which devices.
pub(crate) fn doorbot_ids(id: u64) -> Json {
    serde_json::json!({ "doorbot_ids": [id] })
}

/// `{ "timestamp": ms }`, `"ms"`: a time in milliseconds since 1970.
fn millis(v: &Json) -> Option<i64> {
    match v {
        Json::Number(n) => n.as_i64(),
        Json::String(s) => s.trim().parse().ok(),
        _ => None,
    }
    .filter(|ms| *ms > 0)
}

/// When Ring's last snapshot of device `id` was taken (ms since 1970), from
/// a `snapshots/timestamps` answer; `None` if Ring lists none.
pub(crate) fn snapshot_taken(answer: &Json, id: u64) -> Option<i64> {
    answer["timestamps"]
        .as_array()?
        .iter()
        .find(|t| t["doorbot_id"].as_u64() == Some(id))
        .and_then(|t| millis(&t["timestamp"]))
}

/// The three modes of a location, as the Ring app names them.
pub(crate) const MODES: [&str; 3] = ["disarmed", "home", "away"];

/// A location's mode as Ring gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ModeReading {
    /// One of [`MODES`]; `None` when modes are off for this location
    /// (`disabled`, `unset`).
    pub mode: Option<String>,
    /// When it last changed (ms since 1970).
    pub since: Option<i64>,
    /// Set by an alarm base station: the app changes it, not this API.
    pub read_only: bool,
}

/// A `mode/location/<id>` answer.
pub(crate) fn mode(answer: &Json) -> Option<ModeReading> {
    let mode = answer["mode"].as_str()?;
    Some(ModeReading {
        mode: MODES.contains(&mode).then(|| mode.to_owned()),
        since: millis(&answer["lastUpdateTimeMS"]),
        read_only: answer["readOnly"].as_bool().unwrap_or(false),
    })
}

/// A time as the dashboard reads it (RFC 3339, UTC).
pub(crate) fn rfc3339(ms: i64) -> String {
    jiff::Timestamp::from_millisecond(ms)
        .map(|t| t.to_string())
        .unwrap_or_default()
}

/// Cameras with a floodlight or a spotlight (python-ring-doorbell's lists).
const LIGHT_CAMS: &[&str] = &[
    "hp_cam_v1",
    "floodlight_v2",
    "floodlight_pro",
    "cocoa_floodlight",
    "stickup_cam_v4",
    "hp_cam_v2",
    "spotlightw_v2",
    "cocoa_spotlight",
    "stickup_cam_longfin",
];

/// Cameras with a siren.
const SIREN_CAMS: &[&str] = &[
    "hp_cam_v1",
    "floodlight_v2",
    "floodlight_pro",
    "cocoa_floodlight",
    "stickup_cam_mini",
    "stickup_cam_mini_v2",
    "stickup_cam_mini_ptz_v1",
    "stickup_cam_v4",
    "hp_cam_v2",
    "spotlightw_v2",
    "cocoa_spotlight",
    "stickup_cam_longfin",
    "stickup_cam_lunar",
    "stickup_cam_elite",
    "stickup_cam_wired",
    "cocoa_camera",
];

/// A camera's switches: its light and its siren, when it has them.
fn switches(d: &Json, family: Family) -> (Option<bool>, Option<bool>) {
    let kind = d["kind"].as_str().unwrap_or_default();
    if family != Family::Camera {
        return (None, None);
    }
    let light = (LIGHT_CAMS.contains(&kind) || d["ring_cam_light_installed"] == "true")
        .then(|| d["led_status"] == "on");
    let siren = SIREN_CAMS
        .contains(&kind)
        .then(|| d["siren_status"]["seconds_remaining"].as_i64().unwrap_or(0) > 0);
    (light, siren)
}

/// A device's battery: two bays add up (Spotlight, Stick Up), never past 100.
fn battery(d: &Json) -> Option<f64> {
    match (percent(&d["battery_life"]), percent(&d["battery_life_2"])) {
        (None, None) => None,
        (a, b) => Some((a.unwrap_or(0.0) + b.unwrap_or(0.0)).min(100.0)),
    }
}

/// `"87"`, `87`, `null`: a battery level, when there is one.
fn percent(v: &Json) -> Option<f64> {
    let n = match v {
        Json::Number(n) => n.as_f64()?,
        Json::String(s) => s.trim().parse().ok()?,
        _ => return None,
    };
    (0.0..=100.0).contains(&n).then_some(n)
}

/// Every doorbell, camera and chime of a `ring_devices` answer.
pub(crate) fn devices(answer: &Json) -> Vec<RingDevice> {
    let lists = [
        ("doorbots", Family::Doorbell),
        ("authorized_doorbots", Family::Doorbell),
        ("stickup_cams", Family::Camera),
        ("chimes", Family::Chime),
    ];
    let mut seen = HashSet::new();
    lists
        .iter()
        .flat_map(|(list, family)| {
            answer[*list]
                .as_array()
                .into_iter()
                .flatten()
                .map(move |d| (d, *family))
        })
        .filter_map(|(d, family)| {
            let id = d["id"].as_u64()?;
            let (light, siren) = switches(d, family);
            seen.insert(id).then(|| RingDevice {
                id,
                family,
                name: d["description"]
                    .as_str()
                    .filter(|n| !n.trim().is_empty())
                    .map_or_else(|| format!("Ring {id}"), str::to_owned),
                model: d["kind"].as_str().unwrap_or_default().to_owned(),
                location: d["location_id"].as_str().unwrap_or_default().to_owned(),
                online: d["alerts"]["connection"].as_str() != Some("offline"),
                battery: battery(d),
                volume: d["settings"]["volume"].as_f64(),
                light,
                siren,
                motion_detection: (family != Family::Chime)
                    .then(|| d["settings"]["motion_detection_enabled"].as_bool())
                    .flatten(),
            })
        })
        .collect()
}

/// A health answer: (Wi-Fi signal in dBm, battery percent).
pub(crate) fn health(answer: &Json) -> (Option<i64>, Option<f64>) {
    let h = &answer["device_health"];
    let signal = h["latest_signal_strength"]
        .as_i64()
        .or_else(|| h["latest_signal_strength"].as_str()?.trim().parse().ok())
        .filter(|s| (-120..0).contains(s));
    (signal, percent(&h["battery_percentage"]))
}

/// What happened at a doorbell or a camera.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Happened {
    Press,
    Motion,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Event {
    pub id: String,
    pub what: Happened,
    /// When, in milliseconds since 1970.
    pub at: i64,
}

/// The events of an answer (`{ "events": […] }`, or a plain list for the
/// older `history`), newest first as Ring gives them. Live views and other
/// kinds are left out.
pub(crate) fn events(answer: &Json) -> Vec<Event> {
    let list = answer["events"].as_array().or_else(|| answer.as_array());
    list.into_iter()
        .flatten()
        .filter_map(|e| {
            let what = match e["kind"].as_str()? {
                "ding" => Happened::Press,
                "motion" => Happened::Motion,
                _ => return None,
            };
            let id = e["ding_id_str"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| e["id"].as_u64().map(|n| n.to_string()))
                .or_else(|| e["id"].as_str().map(str::to_owned))?;
            let at = e["created_at"]
                .as_str()?
                .parse::<jiff::Timestamp>()
                .ok()?
                .as_millisecond();
            Some(Event { id, what, at })
        })
        .collect()
}

/// An event older than this is not told: what happened two minutes ago at
/// the door is not someone ringing now.
pub(crate) const FRESH: Duration = Duration::from_secs(90);

/// How many event ids a device remembers: far more than Ring lists at once.
const SEEN: usize = 200;

/// The events already seen at one device, so each is told once at most,
/// whether Ring pushed it or it was read (same id both ways).
#[derive(Debug, Default)]
pub(crate) struct Watch {
    seen: HashSet<String>,
    /// The same ids, oldest first: the oldest are forgotten past [`SEEN`].
    order: VecDeque<String>,
    /// The first reading only learns what is there.
    started: bool,
    /// When Moli began to watch (ms): nothing that happened before is told.
    since: i64,
}

impl Watch {
    pub(crate) fn new(since: i64) -> Self {
        Self {
            since,
            ..Self::default()
        }
    }

    /// Whether `id` is new; remembered from now on (the last [`SEEN`]).
    fn first_time(&mut self, id: &str) -> bool {
        if !self.seen.insert(id.to_owned()) {
            return false;
        }
        self.order.push_back(id.to_owned());
        if self.order.len() > SEEN
            && let Some(old) = self.order.pop_front()
        {
            self.seen.remove(&old);
        }
        true
    }

    fn fresh(since: i64, e: &Event, now: i64) -> bool {
        let fresh = i64::try_from(FRESH.as_millis()).unwrap_or(i64::MAX);
        now - e.at <= fresh && e.at <= now + fresh && e.at >= since
    }

    /// Whether to tell an event Ring pushed, heard at `now` (ms): new (not
    /// pushed twice, nor read before), fresh enough, and not from before
    /// Moli began to watch.
    pub(crate) fn heard(&mut self, e: &Event, now: i64) -> bool {
        self.first_time(&e.id) && Self::fresh(self.since, e, now)
    }

    /// What to tell from a reading made at `now` (ms): new events, fresh
    /// enough, oldest first. The first reading tells nothing.
    pub(crate) fn news(&mut self, events: &[Event], now: i64) -> Vec<Event> {
        let first = !self.started;
        self.started = true;
        let since = self.since;
        let mut news: Vec<Event> = events
            .iter()
            .filter(|e| self.first_time(&e.id))
            .filter(|e| !first && Self::fresh(since, e, now))
            .cloned()
            .collect();
        news.sort_by_key(|e| e.at);
        news
    }
}

/// One event Ring pushed: which device, what, when.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pushed {
    /// Ring's number for the device.
    pub device: u64,
    pub event: Event,
}

/// A number Ring may give as text (`"10000001"`) or as a number.
fn id_text(v: &Json) -> Option<String> {
    v.as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| v.as_u64().map(|n| n.to_string()))
}

/// A push notification (the decrypted message: `{"data": {…}}`, each field
/// of `data` itself JSON as text), as an event; `None` for anything but a
/// press or a motion (intercom, alarm, unknown shape).
pub(crate) fn pushed(message: &Json) -> Option<Pushed> {
    let field = |key: &str| match &message["data"][key] {
        Json::String(text) => serde_json::from_str::<Json>(text).ok(),
        Json::Null => None,
        other => Some(other.clone()),
    };
    let android = field("android_config")?;
    let body = field("data")?;
    let ding = &body["event"]["ding"];
    let what = match android["category"].as_str()? {
        "com.ring.pn.live-event.ding" => Happened::Press,
        "com.ring.pn.live-event.motion" => Happened::Motion,
        _ => return None,
    };
    let device = id_text(&body["device"]["id"])?.parse().ok()?;
    let id = id_text(&ding["id"])?;
    let at = ding["created_at"]
        .as_str()
        .and_then(|t| t.parse::<jiff::Timestamp>().ok())
        .map(jiff::Timestamp::as_millisecond)
        .or_else(|| millis(&body["event"]["eventito"]["timestamp"]))
        .or_else(|| field("analytics").and_then(|a| millis(&a["triggered_at"])))?;
    Some(Pushed {
        device,
        event: Event { id, what, at },
    })
}

fn point(key: &str, label: &str, kind: Kind, unit: Option<Unit>, semantic: Semantic) -> PointSpec {
    PointSpec {
        key: key.into(),
        label: label.into(),
        kind,
        access: Access {
            read: true,
            write: false,
        },
        unit,
        semantic,
    }
}

fn numeric(min: f64, max: f64) -> Kind {
    Kind::Numeric {
        min: Some(min),
        max: Some(max),
        step: Some(1.0),
    }
}

/// A switch Moli can turn (light, siren, motion detection).
fn switch(key: &str, label: &str, semantic: Semantic) -> PointSpec {
    PointSpec {
        access: Access {
            read: true,
            write: true,
        },
        ..point(key, label, Kind::Binary, None, semantic)
    }
}

/// How a Ring device shows in the house: read, but for a camera's switches.
pub(crate) fn device(ctx: &DriverCtx, id: &DeviceId, d: &RingDevice) -> Device {
    let signal = point(
        "wifi_signal",
        &moli_i18n::tr!("pilotes.ring.wifi"),
        numeric(-120.0, 0.0),
        Unit::parse("dBm"),
        Semantic::SignalStrength,
    );
    let battery = point(
        "battery",
        &moli_i18n::tr!("pilotes.ring.batterie"),
        numeric(0.0, 100.0),
        Some(Unit::Percent),
        Semantic::Battery,
    );
    let motion = point(
        "motion",
        &moli_i18n::tr!("pilotes.ring.mouvement"),
        Kind::Binary,
        None,
        Semantic::Occupancy,
    );
    // When the image shown was taken: Ring may give back an old one.
    let image_at = point(
        "image_at",
        &moli_i18n::tr!("pilotes.ring.image_prise"),
        Kind::Text,
        None,
        Semantic::Other,
    );
    let mut points = match d.family {
        Family::Doorbell => vec![
            point(
                "doorbell",
                &moli_i18n::tr!("pilotes.ring.sonnette"),
                Kind::Binary,
                None,
                Semantic::Other,
            ),
            motion,
            battery,
            signal,
            image_at,
        ],
        Family::Camera => vec![motion, battery, signal, image_at],
        Family::Chime => vec![
            point(
                "volume",
                &moli_i18n::tr!("pilotes.ring.volume"),
                numeric(0.0, 11.0),
                None,
                Semantic::Config,
            ),
            signal,
        ],
    };
    if d.motion_detection.is_some() {
        points.push(switch(
            "motion_detection",
            &moli_i18n::tr!("pilotes.ring.detection"),
            Semantic::Config,
        ));
    }
    if d.light.is_some() {
        points.push(switch(
            "light",
            &moli_i18n::tr!("pilotes.ring.lumiere"),
            Semantic::OnOff,
        ));
    }
    if d.siren.is_some() {
        // An agent sounding a siren: a person approves first.
        points.push(switch(
            "siren",
            &moli_i18n::tr!("pilotes.ring.sirene"),
            Semantic::Control,
        ));
    }
    Device {
        id: id.clone(),
        instance: ctx.instance().clone(),
        native_name: d.name.clone().into(),
        manufacturer: Some("Ring".into()),
        model: Some(d.model.clone().into()),
        description: None,
        native_room: None,
        members: Vec::new(),
        points,
    }
}

/// The location's watch (« Modes » in the Ring app): its mode, set from the
/// dashboard by a person only (`Semantic::Control`: an agent, Moli or the
/// dashboard without the code ask, a human decides), and since when.
pub(crate) fn location_device(ctx: &DriverCtx, id: &DeviceId) -> Device {
    let mode = PointSpec {
        key: "mode".into(),
        label: moli_i18n::tr!("pilotes.ring.mode").into(),
        kind: Kind::Enum {
            values: MODES.iter().map(|m| Arc::from(*m)).collect(),
        },
        access: Access {
            read: true,
            write: true,
        },
        unit: None,
        semantic: Semantic::Control,
    };
    let since = point(
        "mode_since",
        &moli_i18n::tr!("pilotes.ring.mode_depuis"),
        Kind::Text,
        None,
        Semantic::Other,
    );
    Device {
        id: id.clone(),
        instance: ctx.instance().clone(),
        native_name: moli_i18n::tr!("pilotes.ring.surveillance").into(),
        manufacturer: Some("Ring".into()),
        model: Some("Modes".into()),
        description: Some(moli_i18n::tr!("pilotes.ring.surveillance_description").into()),
        native_room: None,
        members: Vec::new(),
        points: vec![mode, since],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(text: &str) -> Json {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn the_account_s_doorbell_and_chime() {
        let all = devices(&fixture(include_str!(
            "../../../integrations/ring/fixtures/ring-devices.json"
        )));
        assert_eq!(all.len(), 2);
        let bell = &all[0];
        assert_eq!(bell.family, Family::Doorbell);
        assert_eq!(bell.name, "Porte d'entrée");
        assert_eq!(bell.model, "doorbell_oyster");
        assert_eq!(bell.battery, Some(87.0));
        assert!(bell.online);
        assert_eq!(
            bell.events_path(),
            "/clients_api/locations/00000000-0000-4000-8000-000000000001/devices/10000001/events?limit=5"
        );
        let chime = &all[1];
        assert_eq!(chime.family, Family::Chime);
        assert_eq!(chime.volume, Some(6.0));
        assert_eq!(chime.battery, None);
        assert!(!chime.online, "the chime of the capture was offline");
        assert_eq!(chime.health_path(), "/clients_api/chimes/10000002/health");
    }

    #[test]
    fn a_device_listed_twice_is_one_and_odd_values_are_left_out() {
        let answer = serde_json::json!({
            "doorbots": [{ "id": 1, "description": "", "battery_life": null }],
            "authorized_doorbots": [{ "id": 1 }, { "id": 2, "battery_life": 87, "alerts": {} }],
            "chimes": [{ "description": "x" }],
            "other": [{ "id": 3 }]
        });
        let all = devices(&answer);
        assert_eq!(all.iter().map(|d| d.id).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(all[0].name, "Ring 1");
        assert_eq!(all[0].battery, None);
        assert_eq!(all[1].battery, Some(87.0));
        assert!(all[1].online);
        assert_eq!(percent(&serde_json::json!("250")), None);
    }

    #[test]
    fn health_gives_the_wifi_and_the_battery() {
        let h = fixture(include_str!(
            "../../../integrations/ring/fixtures/doorbot-health.json"
        ));
        assert_eq!(health(&h), (Some(-58), Some(87.0)));
        assert_eq!(health(&Json::Null), (None, None));
        let odd = serde_json::json!({ "device_health": { "latest_signal_strength": "7" } });
        assert_eq!(health(&odd), (None, None));
    }

    #[test]
    fn events_read_from_both_places() {
        let new = events(&fixture(include_str!(
            "../../../integrations/ring/fixtures/events.json"
        )));
        assert_eq!(new.len(), 2, "the live view is not an event: {new:?}");
        assert_eq!(new[0].what, Happened::Press);
        assert_eq!(new[0].id, "7000000000000000002");
        assert_eq!(new[1].what, Happened::Motion);
        assert!(new[0].at > new[1].at);
        let old = events(&fixture(include_str!(
            "../../../integrations/ring/fixtures/history.json"
        )));
        assert_eq!(old.len(), 1);
        assert_eq!(old[0].id, "7000000000000000009");
        assert_eq!(old[0].what, Happened::Press);
    }

    #[test]
    fn the_snapshot_s_time_and_the_location_s_mode() {
        let stamps = fixture(include_str!(
            "../../../integrations/ring/fixtures/snapshots-timestamps.json"
        ));
        assert_eq!(snapshot_taken(&stamps, 10_000_001), Some(1_760_800_000_000));
        assert_eq!(snapshot_taken(&stamps, 10_000_002), None);
        assert_eq!(snapshot_taken(&Json::Null, 10_000_001), None);
        let odd = serde_json::json!({ "timestamps": [{ "doorbot_id": 7, "timestamp": "0" }] });
        assert_eq!(snapshot_taken(&odd, 7), None);
        assert_eq!(rfc3339(1_760_800_000_000), "2025-10-18T15:06:40Z");

        let read = mode(&fixture(include_str!(
            "../../../integrations/ring/fixtures/mode-location.json"
        )))
        .unwrap();
        assert_eq!(read.mode.as_deref(), Some("home"));
        assert_eq!(read.since, Some(1_760_790_000_000));
        assert!(!read.read_only);
        let off = mode(&serde_json::json!({ "mode": "disabled", "readOnly": true })).unwrap();
        assert_eq!(off.mode, None);
        assert!(off.read_only);
        assert_eq!(mode(&serde_json::json!({ "error": "x" })), None);
        assert_eq!(
            mode_path("00000000-0000-4000-8000-000000000001"),
            "/api/v1/mode/location/00000000-0000-4000-8000-000000000001"
        );
        assert_eq!(doorbot_ids(5), serde_json::json!({ "doorbot_ids": [5] }));
    }

    fn event(id: &str, what: Happened, at: i64) -> Event {
        Event {
            id: id.into(),
            what,
            at,
        }
    }

    #[test]
    fn ring_s_pushes_read() {
        let ding = pushed(&fixture(include_str!(
            "../../../integrations/ring/fixtures/push-ding.json"
        )))
        .unwrap();
        assert_eq!(ding.device, 10_000_001);
        assert_eq!(ding.event.what, Happened::Press);
        assert_eq!(ding.event.id, "7000000000000000011");
        assert_eq!(ding.event.at, 1_760_800_000_000);
        let motion = pushed(&fixture(include_str!(
            "../../../integrations/ring/fixtures/push-motion.json"
        )))
        .unwrap();
        assert_eq!(motion.event.what, Happened::Motion);
        assert_eq!(motion.event.id, "7000000000000000012");
        assert_eq!(motion.event.at, 1_760_800_060_123);
        // Fields given as objects (not text) read the same; an intercom,
        // an alarm or an unknown shape is no event.
        let objects = serde_json::json!({ "data": {
            "android_config": { "category": "com.ring.pn.live-event.motion" },
            "data": { "device": { "id": "10000001" }, "event": {
                "ding": { "id": 7_000_000_000_000_000_013_u64 },
                "eventito": { "timestamp": 1_760_800_000_000_i64 } } }
        }});
        let read = pushed(&objects).unwrap();
        assert_eq!(read.event.id, "7000000000000000013");
        assert_eq!(read.event.at, 1_760_800_000_000);
        let intercom = objects
            .to_string()
            .replace("live-event.motion", "live-event.intercom");
        assert_eq!(pushed(&fixture(&intercom)), None);
        let alarm = serde_json::json!({ "data": { "gcmData": "{\"alarm_meta\":{}}" } });
        assert_eq!(pushed(&alarm), None);
        assert_eq!(pushed(&Json::Null), None);
    }

    #[test]
    fn a_pushed_event_is_told_once_even_when_read_after() {
        let now = 1_000_000_000_000;
        let mut watch = Watch::new(now);
        // A press before Moli watched: never told, pushed or read.
        assert!(!watch.heard(&event("0", Happened::Press, now - 1_000), now + 1_000));
        // Read first (a quiet start), then pushed: told once, by the push.
        assert!(watch.news(&[], now + 2_000).is_empty());
        let press = event("1", Happened::Press, now + 3_000);
        assert!(watch.heard(&press, now + 3_500));
        assert!(!watch.heard(&press, now + 3_600), "pushed twice");
        assert!(
            watch
                .news(std::slice::from_ref(&press), now + 10_000)
                .is_empty(),
            "then read"
        );
        // Read first, pushed after: not told again.
        let motion = event("2", Happened::Motion, now + 20_000);
        let told = watch.news(&[motion.clone(), press], now + 21_000);
        assert_eq!(told.len(), 1);
        assert_eq!(told[0], motion);
        assert!(!watch.heard(&motion, now + 22_000));
        // Pushed late (Google kept it): not told.
        let old = event("3", Happened::Press, now + 30_000);
        assert!(!watch.heard(&old, now + 30_000 + 91_000));
    }

    #[test]
    fn the_oldest_ids_are_forgotten_one_by_one() {
        let now = 1_000_000_000_000;
        let mut watch = Watch::new(now);
        for n in 0..250 {
            assert!(watch.heard(&event(&n.to_string(), Happened::Motion, now), now));
        }
        // The last 200 are still known, whatever reads them next.
        assert!(!watch.heard(&event("249", Happened::Motion, now), now));
        assert!(!watch.heard(&event("50", Happened::Motion, now), now));
        assert!(watch.heard(&event("49", Happened::Motion, now), now));
    }

    #[test]
    fn a_press_is_told_once_and_never_late() {
        let mut watch = Watch::default();
        let now = 1_000_000_000_000;
        let before = [event("1", Happened::Press, now - 5_000)];
        // Started a few seconds after a press: not told (it may be old news).
        assert!(watch.news(&before, now).is_empty());
        let next = [
            event("3", Happened::Press, now + 9_000),
            event("2", Happened::Motion, now + 8_000),
            before[0].clone(),
        ];
        let told = watch.news(&next, now + 10_000);
        assert_eq!(
            told.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            ["2", "3"]
        );
        assert!(watch.news(&next, now + 20_000).is_empty(), "told once");
        // Seen for the first time but long after (Ring was out of reach).
        let late = [event("4", Happened::Press, now), next[0].clone()];
        assert!(watch.news(&late, now + 200_000).is_empty());
    }

    #[test]
    fn a_camera_s_light_siren_and_bays() {
        let all = devices(&serde_json::json!({
            "stickup_cams": [
                { "id": 3, "kind": "hp_cam_v1", "battery_life": null, "led_status": "on",
                  "siren_status": { "seconds_remaining": 12 },
                  "settings": { "motion_detection_enabled": false } },
                { "id": 4, "kind": "stickup_cam_v4", "battery_life": 40, "battery_life_2": "75",
                  "led_status": "off", "siren_status": { "seconds_remaining": 0 } },
                { "id": 5, "kind": "stickup_cam_mini_v2" }
            ],
            "doorbots": [
                { "id": 6, "kind": "lpd_v1", "battery_life": 4081,
                  "settings": { "motion_detection_enabled": true } }
            ]
        }));
        let flood = &all[1];
        assert_eq!((flood.light, flood.siren), (Some(true), Some(true)));
        assert_eq!(flood.motion_detection, Some(false));
        assert_eq!(flood.battery, None);
        let spot = &all[2];
        assert_eq!((spot.light, spot.siren), (Some(false), Some(false)));
        assert_eq!(spot.battery, Some(100.0), "two bays, capped");
        assert_eq!(spot.motion_detection, None);
        let indoor = &all[3];
        assert_eq!((indoor.light, indoor.siren), (None, Some(false)));
        let bell = &all[0];
        assert_eq!(
            (bell.light, bell.siren),
            (None, None),
            "a doorbell has neither"
        );
        assert_eq!(bell.battery, None, "a wired doorbell's 4081 is no battery");
        assert_eq!(bell.motion_detection, Some(true));
    }
}
