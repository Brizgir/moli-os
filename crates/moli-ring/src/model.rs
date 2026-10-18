//! What Ring's answers say, in Moli's words: the account's doorbells,
//! cameras and chimes (`ring_devices`), their health (Wi-Fi, battery),
//! their latest events (a press, a motion), with what makes an event new
//! enough to tell; when their last snapshot was taken; and the mode of a
//! location (disarmed, home, away), as the app's « Modes » show it.

use std::collections::HashSet;
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
                battery: percent(&d["battery_life"]),
                volume: d["settings"]["volume"].as_f64(),
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

/// The events already seen at one device, so each is told once at most.
#[derive(Debug, Default)]
pub(crate) struct Watch {
    seen: HashSet<String>,
    /// The first reading only learns what is there.
    started: bool,
}

impl Watch {
    /// What to tell from a reading made at `now` (ms): new events, fresh
    /// enough, oldest first. The first reading tells nothing.
    pub(crate) fn news(&mut self, events: &[Event], now: i64) -> Vec<Event> {
        let first = !self.started;
        self.started = true;
        let fresh = i64::try_from(FRESH.as_millis()).unwrap_or(i64::MAX);
        let mut news: Vec<Event> = events
            .iter()
            .filter(|e| self.seen.insert(e.id.clone()))
            .filter(|e| !first && now - e.at <= fresh && e.at <= now + fresh)
            .cloned()
            .collect();
        // Bounded: what Ring still lists is enough to remember.
        if self.seen.len() > 200 {
            self.seen = events.iter().map(|e| e.id.clone()).collect();
        }
        news.sort_by_key(|e| e.at);
        news
    }
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

/// How a Ring device shows in the house (read only).
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
    let points = match d.family {
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
}
