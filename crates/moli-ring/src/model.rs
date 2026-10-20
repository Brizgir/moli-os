//! What Ring's cloud says, read without any I/O: the devices of the
//! account (`/clients_api/ring_devices`), a camera's history (dings and
//! motions) and its health (Wi-Fi signal).

use std::collections::VecDeque;

use serde_json::Value as Json;

/// Doorbells with a battery (the others report a mains figure, like 4081).
const BATTERY_DOORBELLS: &[&str] = &[
    "doorbot",
    "doorbell",
    "doorbell_v3",
    "doorbell_v4",
    "doorbell_v5",
    "doorbell_scallop_lite",
    "doorbell_scallop",
    "doorbell_oyster",
    "cocoa_doorbell",
    "cocoa_doorbell_v2",
    "df_doorbell_clownfish",
    "doorbell_portal",
];
const BATTERY_CAMS: &[&str] = &[
    "stickup_cam_v4",
    "stickup_cam",
    "stickup_cam_v3",
    "stickup_cam_lunar",
    "cocoa_camera",
];
/// Cameras with a floodlight or a spotlight.
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

/// How a device is reached in the API.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Family {
    /// A doorbell (owned or shared with the account).
    Doorbot,
    /// A camera (stick up, floodlight, spotlight, indoor).
    Camera,
    /// A chime: no camera, no history.
    Chime,
}

impl Family {
    /// Where its health is read.
    pub(crate) fn api(self) -> &'static str {
        match self {
            Self::Doorbot | Self::Camera => "doorbots",
            Self::Chime => "chimes",
        }
    }

    pub(crate) fn has_video(self) -> bool {
        self != Self::Chime
    }
}

/// One device of the account, as the device list describes it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RingDevice {
    /// The API's id: stable, and what every call names.
    pub id: u64,
    pub family: Family,
    pub kind: String,
    pub name: String,
    pub online: bool,
    /// Percent; `None` when the device has no battery.
    pub battery: Option<i64>,
    /// The floodlight or spotlight is on (`None`: it has none).
    pub light: Option<bool>,
    /// The siren sounds (`None`: it has none).
    pub siren: Option<bool>,
    /// Motion detection is enabled (`None`: not adjustable).
    pub motion_detection: Option<bool>,
}

/// The devices of an account. Intercoms and other devices (under `other`)
/// are left out: their events come through push only.
pub(crate) fn devices(json: &Json) -> Vec<RingDevice> {
    [
        ("doorbots", Family::Doorbot),
        ("authorized_doorbots", Family::Doorbot),
        ("stickup_cams", Family::Camera),
        ("chimes", Family::Chime),
    ]
    .into_iter()
    .flat_map(|(key, family)| {
        json[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(move |d| device(d, family))
    })
    .collect()
}

fn device(json: &Json, family: Family) -> Option<RingDevice> {
    let id = id(&json["id"])?;
    let kind = json["kind"].as_str().unwrap_or_default().to_owned();
    let name = json["description"]
        .as_str()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map_or_else(|| format!("Ring {id}"), str::to_owned);
    let has = |list: &[&str]| list.contains(&kind.as_str());
    let battery = match family {
        Family::Doorbot if has(BATTERY_DOORBELLS) => battery(json),
        Family::Camera if has(BATTERY_CAMS) || json.get("battery_life_2").is_some() => {
            battery(json)
        }
        _ => None,
    };
    let camera = family == Family::Camera;
    let light = (camera && (has(LIGHT_CAMS) || json["ring_cam_light_installed"] == "true"))
        .then(|| json["led_status"] == "on");
    let siren = (camera && has(SIREN_CAMS)).then(|| {
        json["siren_status"]["seconds_remaining"]
            .as_i64()
            .unwrap_or(0)
            > 0
    });
    let motion_detection = family
        .has_video()
        .then(|| json["settings"]["motion_detection_enabled"].as_bool())
        .flatten();
    Some(RingDevice {
        id,
        family,
        kind,
        name,
        online: json["alerts"]["connection"] != "offline",
        battery,
        light,
        siren,
        motion_detection,
    })
}

/// A number, or a number written as text (both happen).
fn id(json: &Json) -> Option<u64> {
    json.as_u64()
        .or_else(|| json.as_str().and_then(|s| s.parse().ok()))
}

/// A whole number, given as a number (sometimes a float) or as text.
#[allow(clippy::cast_possible_truncation)]
fn level(json: &Json) -> Option<i64> {
    json.as_i64()
        .or_else(|| {
            json.as_f64()
                .filter(|f| f.is_finite())
                .map(|f| f.round() as i64)
        })
        .or_else(|| json.as_str().and_then(|s| s.trim().parse().ok()))
}

/// Two battery bays add up; never above 100.
fn battery(json: &Json) -> Option<i64> {
    let first = level(&json["battery_life"]);
    let second = level(&json["battery_life_2"]);
    if first.is_none() && second.is_none() {
        return None;
    }
    Some((first.unwrap_or(0) + second.unwrap_or(0)).clamp(0, 100))
}

/// What happened at a camera.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Happened {
    /// Someone rang.
    Ding,
    Motion,
}

impl Happened {
    /// The point it raises.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Ding => "doorbell",
            Self::Motion => "motion",
        }
    }
}

/// One entry of a camera's history.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Event {
    pub id: u64,
    /// `None`: a kind Moli does not show (a live view asked for…).
    pub happened: Option<Happened>,
    /// When, in milliseconds since the epoch (`None`: unreadable).
    pub at_ms: Option<i64>,
}

pub(crate) fn history(json: &Json) -> Vec<Event> {
    json.as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            Some(Event {
                id: id(&e["id"])?,
                happened: match e["kind"].as_str() {
                    Some("ding") => Some(Happened::Ding),
                    Some("motion") => Some(Happened::Motion),
                    _ => None,
                },
                at_ms: e["created_at"]
                    .as_str()
                    .and_then(|t| t.parse::<jiff::Timestamp>().ok())
                    .map(jiff::Timestamp::as_millisecond),
            })
        })
        .collect()
}

/// The Wi-Fi signal a health answer gives, in dBm.
pub(crate) fn signal(json: &Json) -> Option<i64> {
    level(&json["device_health"]["latest_signal_strength"])
}

/// How many event ids a camera remembers (its history page holds fewer).
const SEEN: usize = 64;

/// The events a camera already showed: only a new one is news.
#[derive(Debug, Default)]
pub(crate) struct Seen {
    ids: VecDeque<u64>,
    /// The first page only teaches what is old: nothing rings at startup.
    primed: bool,
}

impl Seen {
    /// The events of `page` that are news: never seen, not older than
    /// `max_age_ms` (an outage does not ring hours later), oldest first.
    pub(crate) fn news(&mut self, page: &[Event], now_ms: i64, max_age_ms: i64) -> Vec<Happened> {
        let mut news = Vec::new();
        for event in page.iter().rev() {
            if self.ids.contains(&event.id) {
                continue;
            }
            self.ids.push_back(event.id);
            if self.ids.len() > SEEN {
                self.ids.pop_front();
            }
            let fresh = event.at_ms.is_some_and(|at| now_ms - at <= max_age_ms);
            if self.primed && fresh {
                news.extend(event.happened);
            }
        }
        self.primed = true;
        news
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Json {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../integrations/ring/fixtures")
            .join(name);
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn reads_every_family_but_intercoms() {
        let list = devices(&fixture("ring_devices.json"));
        let ids: Vec<u64> = list.iter().map(|d| d.id).collect();
        assert_eq!(ids, [100_001, 100_002, 100_003, 100_004, 100_005]);

        let door = &list[0];
        assert_eq!(door.family, Family::Doorbot);
        assert_eq!(door.name, "Entrée");
        assert_eq!(door.battery, Some(87), "battery given as text");
        assert_eq!(door.motion_detection, Some(true));
        assert_eq!((door.light, door.siren), (None, None));

        let shared = &list[1];
        assert!(!shared.online);
        assert_eq!(
            shared.battery, None,
            "a wired doorbell's 4081 is no battery"
        );
        assert_eq!(shared.motion_detection, None);

        let flood = &list[2];
        assert_eq!(flood.family, Family::Camera);
        assert_eq!(flood.light, Some(true));
        assert_eq!(flood.siren, Some(true), "seconds remaining");
        assert_eq!(flood.battery, None);
        assert_eq!(flood.motion_detection, Some(false));

        let spot = &list[3];
        assert_eq!(spot.battery, Some(100), "two bays, capped");
        assert_eq!((spot.light, spot.siren), (Some(false), Some(false)));

        let chime = &list[4];
        assert_eq!(chime.family, Family::Chime);
        assert_eq!(chime.motion_detection, None);
    }

    #[test]
    fn reads_history_and_health() {
        let page = history(&fixture("ring_history.json"));
        assert_eq!(page.len(), 3);
        assert_eq!(page[0].happened, Some(Happened::Ding));
        assert_eq!(page[0].id, 7_300_000_000_000_000_003);
        assert_eq!(page[1].happened, Some(Happened::Motion));
        assert_eq!(page[2].happened, None);
        assert_eq!(page[2].id, 7_300_000_000_000_000_001, "id given as text");
        assert_eq!(page[0].at_ms, Some(1_790_841_630_000));
        assert_eq!(signal(&fixture("ring_health.json")), Some(-58));
    }

    #[test]
    fn only_new_and_fresh_events_are_news() {
        let page = history(&fixture("ring_history.json"));
        let now = page[0].at_ms.unwrap() + 5_000;
        let mut seen = Seen::default();
        assert!(
            seen.news(&page, now, 300_000).is_empty(),
            "the first page is the past"
        );
        assert!(seen.news(&page, now, 300_000).is_empty(), "nothing new");

        let mut next = vec![
            Event {
                id: 7_300_000_000_000_000_005,
                happened: Some(Happened::Ding),
                at_ms: Some(now),
            },
            Event {
                id: 7_300_000_000_000_000_004,
                happened: Some(Happened::Motion),
                at_ms: Some(now - 1_000),
            },
        ];
        next.extend(page.clone());
        assert_eq!(
            seen.news(&next, now, 300_000),
            [Happened::Motion, Happened::Ding],
            "oldest first"
        );

        let old = [Event {
            id: 1,
            happened: Some(Happened::Ding),
            at_ms: Some(now - 3_600_000),
        }];
        assert!(
            seen.news(&old, now, 300_000).is_empty(),
            "an hour-old ring found after an outage does not ring"
        );
    }
}
