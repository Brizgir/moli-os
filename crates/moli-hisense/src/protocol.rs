//! What goes over the TV's MQTT: topics, the orders Moli sends, the
//! messages the TV answers or broadcasts, read tolerantly (its JSON puts
//! numbers in strings at will). Facts from hisensetv (newAM),
//! ha_hisense_tv (sehaas), hisense_vidaa (stevene1919) and pyvidaa
//! (warrenrees), all MIT; no code copied.

use serde_json::{Value as Json, json};

/// `/remoteapp/tv/<service>/<client_id>/actions/<action>`: an order.
#[must_use]
pub fn action(client_id: &str, service: &str, name: &str) -> String {
    format!("/remoteapp/tv/{service}/{client_id}/actions/{name}")
}

/// `/remoteapp/tv/<service>/<client_id>/data/<name>`: a request for data
/// (`gettoken`).
#[must_use]
pub fn request(client_id: &str, service: &str, name: &str) -> String {
    format!("/remoteapp/tv/{service}/{client_id}/data/{name}")
}

/// What the TV says to this client, and to everyone.
#[must_use]
pub fn listened(client_id: &str) -> Vec<String> {
    let own = |rest: &str| format!("/remoteapp/mobile/{client_id}/{rest}");
    let all = |rest: &str| format!("/remoteapp/mobile/broadcast/{rest}");
    vec![
        // The wildcards first; some firmware refuses them, hence the exact
        // topics too (a double delivery is harmless: every reading is
        // idempotent).
        own("#"),
        all("#"),
        own("ui_service/data/state"),
        own("ui_service/data/sourcelist"),
        own("ui_service/data/applist"),
        own("ui_service/data/authentication"),
        own("ui_service/data/authenticationcode"),
        own("ui_service/data/vidaa_app_connect"),
        own("platform_service/data/tokenissuance"),
        own("platform_service/data/getvolume"),
        all("ui_service/state"),
        all("ui_service/volume"),
        all("platform_service/actions/volumechange"),
        all("platform_service/actions/tvsleep"),
    ]
}

/// The keys the TV takes, by the names Moli's remotes send (the same as
/// the other TVs'), and the TV's own names. Mind `KEY_BACK`: it rewinds;
/// going back is `KEY_RETURNS`.
pub const KEYS: [(&str, &str); 36] = [
    ("CursorUp", "KEY_UP"),
    ("CursorDown", "KEY_DOWN"),
    ("CursorLeft", "KEY_LEFT"),
    ("CursorRight", "KEY_RIGHT"),
    ("Confirm", "KEY_OK"),
    ("Back", "KEY_RETURNS"),
    ("Exit", "KEY_EXIT"),
    ("Home", "KEY_HOME"),
    ("Options", "KEY_MENU"),
    ("Info", "KEY_INFO"),
    ("PlayPause", "KEY_PLAY"),
    ("Play", "KEY_PLAY"),
    ("Pause", "KEY_PAUSE"),
    ("Stop", "KEY_STOP"),
    ("Previous", "KEY_BACK"),
    ("Next", "KEY_FORWARDS"),
    ("Rewind", "KEY_BACK"),
    ("FastForward", "KEY_FORWARDS"),
    ("VolumeUp", "KEY_VOLUMEUP"),
    ("VolumeDown", "KEY_VOLUMEDOWN"),
    ("Mute", "KEY_MUTE"),
    ("ChannelStepUp", "KEY_CHANNELUP"),
    ("ChannelStepDown", "KEY_CHANNELDOWN"),
    ("Subtitle", "KEY_SUBTITLE"),
    ("Red", "KEY_RED"),
    ("Green", "KEY_GREEN"),
    ("Yellow", "KEY_YELLOW"),
    ("Blue", "KEY_BLUE"),
    ("Digit0", "KEY_0"),
    ("Digit1", "KEY_1"),
    ("Digit2", "KEY_2"),
    ("Digit3", "KEY_3"),
    ("Digit4", "KEY_4"),
    ("Digit5", "KEY_5"),
    ("Digit6", "KEY_6"),
    ("Digit7", "KEY_7"),
];

/// The other digits, kept apart so [`KEYS`] reads as the remote does.
const MORE_DIGITS: [(&str, &str); 2] = [("Digit8", "KEY_8"), ("Digit9", "KEY_9")];

/// Power: a toggle, pressed only when the state known differs.
pub const KEY_POWER: &str = "KEY_POWER";
pub const KEY_MUTE: &str = "KEY_MUTE";

/// Every key name Moli's remotes may send.
pub fn key_names() -> impl Iterator<Item = &'static str> {
    KEYS.iter().chain(MORE_DIGITS.iter()).map(|(k, _)| *k)
}

#[must_use]
pub fn key_of(name: &str) -> Option<&'static str> {
    KEYS.iter()
        .chain(MORE_DIGITS.iter())
        .find(|(k, _)| *k == name)
        .map(|(_, tv)| *tv)
}

/// An input (`sourcelist`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub id: String,
    pub name: String,
    /// What the TV shows (a renamed input: « Décodeur »).
    pub display: String,
}

impl Source {
    /// The word a person or a remote uses for it.
    #[must_use]
    pub fn label(&self) -> &str {
        if self.display.trim().is_empty() {
            &self.name
        } else {
            &self.display
        }
    }
}

/// An app (`applist`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct App {
    pub id: String,
    pub name: String,
    pub url: String,
}

/// What the TV is doing (`state`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    /// The TV's own word (`sourceswitch`, `livetv`, `app`, `remote_launcher`,
    /// `fake_sleep_0`…).
    pub kind: String,
    pub source: Option<String>,
    pub source_id: Option<String>,
    pub app: Option<String>,
    /// What is on screen: the input, the programme, the app.
    pub screen: Option<String>,
}

impl State {
    /// Standby (`fake_sleep_0`, `poweroff`): the broker still answers, the
    /// screen is off. `fake_sleep_1` is the TV coming back on.
    #[must_use]
    pub fn off(&self) -> bool {
        matches!(self.kind.as_str(), "fake_sleep_0" | "poweroff")
    }

    /// The TV wants a code (the pairing is gone, or never was).
    #[must_use]
    pub fn asks_code(&self) -> bool {
        matches!(self.kind.as_str(), "authentication" | "authenticationcode")
    }
}

/// One message from the TV, by its topic.
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    State(State),
    /// Going to standby (`tvsleep`).
    Sleep,
    /// `volume_type` 0: the level; 2: muted or not.
    Volume {
        level: Option<u8>,
        muted: Option<bool>,
    },
    Sources(Vec<Source>),
    Apps(Vec<App>),
    /// The code is on screen (`authentication`, often empty).
    CodeShown,
    /// `vidaa_app_connect` taken (`connect_result`).
    Connected(Option<i64>),
    /// The code typed was right, or why not.
    CodeAnswer(Result<(), String>),
    Tokens(crate::credentials::Tokens),
}

/// A number the TV wrote as a number or as a string.
#[allow(clippy::cast_possible_truncation)] // rounded first
fn number(v: &Json) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_f64().map(|f| f.round() as i64))
        .or_else(|| v.as_str()?.trim().parse().ok())
}

fn text(v: &Json) -> Option<String> {
    v.as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| number(v).map(|n| n.to_string()))
}

fn state(payload: &Json) -> State {
    let kind = text(&payload["statetype"]).unwrap_or_default();
    let mut s = State {
        kind: kind.clone(),
        ..State::default()
    };
    match kind.as_str() {
        "sourceswitch" => {
            s.source = text(&payload["sourcename"]);
            s.source_id = text(&payload["sourceid"]);
            s.screen = text(&payload["displayname"]).or_else(|| s.source.clone());
        }
        "livetv" => {
            s.source = Some("TV".into());
            s.source_id = text(&payload["sourceid"]);
            let channel = text(&payload["channel_name"]);
            let programme = text(&payload["progname"]);
            s.screen = match (channel, programme) {
                (Some(c), Some(p)) => Some(format!("{c} : {p}")),
                (c, p) => c.or(p),
            };
        }
        "app" => {
            s.app = text(&payload["name"]).or_else(|| text(&payload["url"]));
            s.screen.clone_from(&s.app);
        }
        _ => s.screen = Some(kind),
    }
    s
}

fn sources(payload: &Json) -> Vec<Source> {
    payload
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| {
            let id = text(&v["sourceid"])?;
            let name = text(&v["sourcename"]).unwrap_or_else(|| id.clone());
            Some(Source {
                id,
                display: text(&v["displayname"]).unwrap_or_else(|| name.clone()),
                name,
            })
        })
        .collect()
}

fn apps(payload: &Json) -> Vec<App> {
    payload
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| {
            let name = text(&v["name"])?;
            Some(App {
                id: text(&v["appId"]).unwrap_or_default(),
                name,
                url: text(&v["url"]).unwrap_or_default(),
            })
        })
        .collect()
}

fn volume(payload: &Json) -> Message {
    let value = number(&payload["volume_value"])
        .or_else(|| number(&payload["volume"]))
        .or_else(|| number(&payload["value"]));
    match number(&payload["volume_type"]).unwrap_or(0) {
        0 => Message::Volume {
            level: value.and_then(|v| u8::try_from(v.clamp(0, 100)).ok()),
            muted: None,
        },
        2 => Message::Volume {
            level: None,
            muted: value.map(|v| v == 1),
        },
        _ => Message::Volume {
            level: None,
            muted: None,
        },
    }
}

fn tokens(payload: &Json) -> Option<Message> {
    let days = |key: &str| number(&payload[key]).and_then(|d| u64::try_from(d).ok());
    Some(Message::Tokens(crate::credentials::Tokens {
        access: text(&payload["accesstoken"])?,
        access_days: days("accesstoken_duration_day"),
        refresh: text(&payload["refreshtoken"]),
        refresh_days: days("refreshtoken_duration_day"),
    }))
}

/// Reads one message. `None`: not for Moli, or unreadable.
#[must_use]
pub fn read(topic: &str, payload: &[u8]) -> Option<Message> {
    let tail = topic.rsplit('/').take(3).collect::<Vec<_>>();
    let [name, kind, service] = tail[..] else {
        return None;
    };
    if (service, kind, name) == ("ui_service", "data", "authentication") {
        return Some(Message::CodeShown);
    }
    if (service, kind, name) == ("platform_service", "actions", "tvsleep") {
        return Some(Message::Sleep);
    }
    let payload: Json = serde_json::from_slice(payload).ok()?;
    match (service, kind, name) {
        // `/remoteapp/mobile/broadcast/ui_service/state`: `kind` is the
        // service, `service` the broadcast.
        ("ui_service", "data", "state") | ("broadcast", "ui_service", "state")
            if payload.is_object() =>
        {
            Some(Message::State(state(&payload)))
        }
        ("ui_service", "data", "sourcelist") => Some(Message::Sources(sources(&payload))),
        ("ui_service", "data", "applist") => Some(Message::Apps(apps(&payload))),
        ("ui_service", "data", "authenticationcode") => {
            let result = number(&payload["result"]);
            Some(Message::CodeAnswer(if result == Some(1) {
                Ok(())
            } else {
                Err(text(&payload["info"])
                    .unwrap_or_else(|| format!("result {}", result.unwrap_or(-1))))
            }))
        }
        ("ui_service", "data", "vidaa_app_connect") => {
            Some(Message::Connected(number(&payload["connect_result"])))
        }
        ("platform_service", "data", "tokenissuance") => tokens(&payload),
        (_, _, "getvolume" | "volume" | "volumechange") => Some(volume(&payload)),
        _ => None,
    }
}

/// `sendkey`: the key's name, bare.
#[must_use]
pub fn press(client_id: &str, key: &str) -> (String, String) {
    (
        action(client_id, "remote_service", "sendkey"),
        key.to_owned(),
    )
}

/// `changevolume`: the level, bare.
#[must_use]
pub fn set_volume(client_id: &str, level: u8) -> (String, String) {
    (
        action(client_id, "platform_service", "changevolume"),
        level.to_string(),
    )
}

#[must_use]
pub fn change_source(client_id: &str, source: &Source) -> (String, String) {
    (
        action(client_id, "ui_service", "changesource"),
        json!({"sourceid": source.id, "sourcename": source.name}).to_string(),
    )
}

#[must_use]
pub fn launch(client_id: &str, app: &App) -> (String, String) {
    (
        action(client_id, "ui_service", "launchapp"),
        json!({"appId": app.id, "name": app.name, "url": app.url}).to_string(),
    )
}

/// Asks the TV to show a code (newer firmware).
#[must_use]
pub fn connect(client_id: &str) -> (String, String) {
    (
        action(client_id, "ui_service", "vidaa_app_connect"),
        json!({"app_version": 2, "connect_result": 0, "device_type": "Mobile App"}).to_string(),
    )
}

/// The code typed, as a number (a string is refused by some firmware).
#[must_use]
pub fn code(client_id: &str, code: u32) -> (String, String) {
    (
        action(client_id, "ui_service", "authenticationcode"),
        json!({"authNum": code}).to_string(),
    )
}

/// Takes the code dialog off the screen.
#[must_use]
pub fn close_code(client_id: &str) -> (String, String) {
    (
        action(client_id, "ui_service", "authenticationcodeclose"),
        String::new(),
    )
}

/// Asks for tokens: fresh ones after the code (`""`), or renewed ones.
#[must_use]
pub fn get_token(client_id: &str, refresh: &str) -> (String, String) {
    (
        request(client_id, "platform_service", "gettoken"),
        json!({"refreshtoken": refresh}).to_string(),
    )
}

/// The Android packages Moli's living room sends, by the names the TV's
/// app list uses (several spellings, lowercase, no spaces).
const PACKAGES: [(&str, &[&str]); 7] = [
    ("com.netflix.ninja", &["netflix"]),
    ("com.google.android.youtube.tv", &["youtube"]),
    (
        "com.disney.disneyplus",
        &["disney+", "disneyplus", "disney"],
    ),
    (
        "com.amazon.amazonvideo.livingroom",
        &["primevideo", "amazonprimevideo", "amazon", "prime"],
    ),
    ("org.jellyfin.androidtv", &["jellyfin"]),
    ("com.spotify.tv.android", &["spotify"]),
    ("com.plexapp.android", &["plex"]),
];

/// The apps a TV almost always has, as the VIDAA app launches them when
/// its list is not at hand (ids seen on several models; may vary).
const USUAL: [(&str, &str, &str); 3] = [
    ("1", "Netflix", "netflix"),
    ("3", "YouTube", "youtube"),
    ("2", "Prime Video", "amazon"),
];

fn squash(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != '_')
        .flat_map(char::to_lowercase)
        .collect()
}

/// The app meant by `wanted` (a name, an Android package, an id or a url),
/// among `known` (the TV's list), else among the usual ones.
#[must_use]
pub fn find_app(wanted: &str, known: &[App]) -> Option<App> {
    let wanted = wanted.trim();
    let mut spellings = vec![squash(wanted)];
    if let Some((_, names)) = PACKAGES.iter().find(|(pkg, _)| *pkg == wanted) {
        spellings.extend(names.iter().map(|n| (*n).to_owned()));
    }
    let matches = |app: &App| {
        let name = squash(&app.name);
        app.id == wanted
            || app.url == wanted
            || spellings.iter().any(|s| !s.is_empty() && name == *s)
    };
    known
        .iter()
        .find(|a| matches(a))
        .cloned()
        .or_else(|| {
            known
                .iter()
                .find(|a| {
                    let name = squash(&a.name);
                    spellings
                        .iter()
                        .any(|s| s.len() >= 4 && name.contains(s.as_str()))
                })
                .cloned()
        })
        .or_else(|| {
            USUAL
                .iter()
                .map(|(id, name, url)| App {
                    id: (*id).into(),
                    name: (*name).into(),
                    url: (*url).into(),
                })
                .find(|a| matches(a))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURES: &str = "../../../integrations/hisense/fixtures/";
    macro_rules! fixture {
        ($name:literal) => {
            include_bytes!(concat!("../../../integrations/hisense/fixtures/", $name))
        };
    }

    fn own(rest: &str) -> String {
        format!("/remoteapp/mobile/02:00:00:00:00:01$his$3A8DE7_vidaacommon_001/{rest}")
    }

    #[test]
    fn the_state_says_what_is_on_screen() {
        let _ = FIXTURES;
        let Some(Message::State(s)) = read(
            "/remoteapp/mobile/broadcast/ui_service/state",
            fixture!("state-sourceswitch.json"),
        ) else {
            panic!("a state")
        };
        assert_eq!(s.source.as_deref(), Some("HDMI 2"));
        assert_eq!(s.source_id.as_deref(), Some("3"));
        assert_eq!(s.screen.as_deref(), Some("HDMI 2"));
        assert!(!s.off());
        // Addressed to the client: the same reading.
        let Some(Message::State(app)) =
            read(&own("ui_service/data/state"), fixture!("state-app.json"))
        else {
            panic!("a state")
        };
        assert_eq!(app.app.as_deref(), Some("Netflix"));
        assert_eq!(app.screen.as_deref(), Some("Netflix"));
        let Some(Message::State(tv)) =
            read(&own("ui_service/data/state"), fixture!("state-livetv.json"))
        else {
            panic!("a state")
        };
        assert_eq!(tv.source.as_deref(), Some("TV"));
        assert_eq!(tv.screen.as_deref(), Some("Chaîne 2 : Le journal"));
        let Some(Message::State(asleep)) = read(
            &own("ui_service/data/state"),
            fixture!("state-fake-sleep.json"),
        ) else {
            panic!("a state")
        };
        assert!(asleep.off());
        assert_eq!(asleep.screen.as_deref(), Some("fake_sleep_0"));
        assert!(
            !State {
                kind: "fake_sleep_1".into(),
                ..State::default()
            }
            .off()
        );
        assert!(
            State {
                kind: "authentication".into(),
                ..State::default()
            }
            .asks_code()
        );
        assert_eq!(
            read(
                "/remoteapp/mobile/broadcast/platform_service/actions/tvsleep",
                b""
            ),
            Some(Message::Sleep)
        );
    }

    #[test]
    fn the_inputs_and_apps_are_listed() {
        let Some(Message::Sources(sources)) = read(
            &own("ui_service/data/sourcelist"),
            fixture!("sourcelist.json"),
        ) else {
            panic!("sources")
        };
        assert_eq!(sources.len(), 6);
        assert_eq!(sources[4].id, "5");
        assert_eq!(sources[4].name, "HDMI 4");
        assert_eq!(sources[4].label(), "PC", "what the TV shows");
        let Some(Message::Apps(apps)) =
            read(&own("ui_service/data/applist"), fixture!("applist.json"))
        else {
            panic!("apps")
        };
        assert_eq!(apps.len(), 4);
        assert_eq!(apps[3].name, "Disney+");
        assert_eq!(find_app("com.netflix.ninja", &apps).unwrap().id, "1");
        assert_eq!(find_app("disney+", &apps).unwrap().id, "295");
        assert_eq!(find_app("Prime video", &apps).unwrap().id, "2");
        assert_eq!(
            find_app("com.amazon.amazonvideo.livingroom", &apps)
                .unwrap()
                .name,
            "Prime Video"
        );
        assert_eq!(
            find_app("YouTube", &[]).unwrap().url,
            "youtube",
            "the usual ones without a list"
        );
        assert!(find_app("com.limelight", &apps).is_none());
        let (topic, payload) = launch("c", &apps[0]);
        assert_eq!(topic, "/remoteapp/tv/ui_service/c/actions/launchapp");
        assert_eq!(payload, r#"{"appId":"1","name":"Netflix","url":"netflix"}"#);
        assert_eq!(
            change_source("c", &sources[2]).1,
            r#"{"sourceid":"3","sourcename":"HDMI 2"}"#
        );
    }

    #[test]
    fn the_volume_is_read_whatever_its_spelling() {
        assert_eq!(
            read(
                &own("platform_service/data/getvolume"),
                fixture!("getvolume.json")
            ),
            Some(Message::Volume {
                level: Some(23),
                muted: None
            })
        );
        assert_eq!(
            read(
                "/remoteapp/mobile/broadcast/platform_service/actions/volumechange",
                fixture!("volumechange-mute.json")
            ),
            Some(Message::Volume {
                level: None,
                muted: Some(true)
            })
        );
        assert_eq!(
            read(
                "/remoteapp/mobile/broadcast/ui_service/volume",
                br#"{"volume_type":0,"volume_value":100}"#
            ),
            Some(Message::Volume {
                level: Some(100),
                muted: None
            })
        );
        assert_eq!(
            set_volume("c", 40),
            (
                "/remoteapp/tv/platform_service/c/actions/changevolume".into(),
                "40".into()
            )
        );
    }

    #[test]
    fn the_code_is_answered_and_tokens_issued() {
        assert_eq!(
            read(
                &own("ui_service/data/authenticationcode"),
                fixture!("authenticationcode-ok.json")
            ),
            Some(Message::CodeAnswer(Ok(())))
        );
        assert_eq!(
            read(
                &own("ui_service/data/authenticationcode"),
                fixture!("authenticationcode-wrong.json")
            ),
            Some(Message::CodeAnswer(Err("Wrong authNum!!".into())))
        );
        assert_eq!(
            read(&own("ui_service/data/authentication"), b""),
            Some(Message::CodeShown)
        );
        assert_eq!(
            read(
                &own("ui_service/data/vidaa_app_connect"),
                br#"{"connect_result":1}"#
            ),
            Some(Message::Connected(Some(1)))
        );
        let Some(Message::Tokens(t)) = read(
            &own("platform_service/data/tokenissuance"),
            fixture!("tokenissuance.json"),
        ) else {
            panic!("tokens")
        };
        assert_eq!(t.access, "accesstoken-fictif-0001");
        assert_eq!(t.access_days, Some(2));
        assert_eq!(t.refresh.as_deref(), Some("refreshtoken-fictif-0001"));
        assert_eq!(t.refresh_days, Some(30));
        assert_eq!(code("c", 1234).1, r#"{"authNum":1234}"#);
        assert_eq!(
            get_token("c", "").0,
            "/remoteapp/tv/platform_service/c/data/gettoken"
        );
        assert_eq!(
            connect("c").1,
            r#"{"app_version":2,"connect_result":0,"device_type":"Mobile App"}"#
        );
        assert_eq!(
            read(
                &own("ui_service/data/hotelmodechange"),
                br#"{"hotel_mode":"off"}"#
            ),
            None
        );
        assert_eq!(read(&own("ui_service/data/state"), b"not json"), None);
    }

    #[test]
    fn the_keys_are_the_ones_every_remote_sends() {
        for key in [
            "CursorUp",
            "CursorDown",
            "CursorLeft",
            "CursorRight",
            "Confirm",
            "Back",
            "Home",
            "Options",
            "PlayPause",
            "Previous",
            "Next",
            "VolumeUp",
            "VolumeDown",
            "Mute",
            "ChannelStepUp",
            "ChannelStepDown",
            "Digit0",
            "Digit9",
        ] {
            assert!(key_of(key).is_some(), "{key}");
        }
        assert_eq!(key_of("Back"), Some("KEY_RETURNS"), "KEY_BACK rewinds");
        assert_eq!(key_of("Previous"), Some("KEY_BACK"));
        assert_eq!(key_of("Confirm"), Some("KEY_OK"));
        assert_eq!(key_of("Digit9"), Some("KEY_9"));
        assert_eq!(key_of("Source"), None);
        assert_eq!(key_names().count(), 38);
        assert_eq!(
            press("c", "KEY_OK"),
            (
                "/remoteapp/tv/remote_service/c/actions/sendkey".into(),
                "KEY_OK".into()
            )
        );
        let topics = listened("c");
        assert_eq!(topics[0], "/remoteapp/mobile/c/#");
        assert!(topics.contains(&"/remoteapp/mobile/broadcast/ui_service/state".to_owned()));
    }
}
