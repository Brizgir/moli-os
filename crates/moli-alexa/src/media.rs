//! What each speaker plays, from every answer Amazon may give: the account's
//! media sessions (`np/list-media-sessions`, one call for all the speakers),
//! each speaker's own player (`np/player`) for those the sessions say nothing
//! of, then the multi-room groups: while a group plays, Amazon answers « idle »
//! for its members, and the group's player says it plays for them
//! (`isPlayingInLemur`, each member's volume in `lemurVolume`).
//!
//! A speaker none of them describes (a player that answers 400 or a state
//! of `null`, as an Echo Dot does while it plays) is **unknown**, never
//! taken for idle: the dashboard then asserts nothing it would act on.
//!
//! The sessions' list is read loosely (its shape is not published): the
//! debug log describes the shape of what came, never a value from it.

use std::collections::HashMap;
use std::fmt::Write as _;

use serde_json::Value as Json;

use crate::echo::Echo;

pub(crate) const PLAYING: &str = "PLAYING";
const PAUSED: &str = "PAUSED";
/// No source said anything of the speaker.
pub(crate) const UNKNOWN: &str = "UNKNOWN";
/// Where a sessions' answer may keep its list.
const LISTS: [&str; 3] = ["mediaSessionList", "mediaSessions", "sessions"];
/// Where a session may keep what it plays.
const NOW: [&str; 2] = ["nowPlayingData", "playerInfo"];

/// What a speaker plays, as the dashboard shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Playing {
    /// `PLAYING`, `PAUSED`, `IDLE`…
    pub state: String,
    pub title: String,
    pub artist: String,
    pub volume: Option<f64>,
    /// A group's members' volumes while it plays: (serial, 0-100).
    pub members: Vec<(String, f64)>,
}

impl Playing {
    pub(crate) fn unknown() -> Self {
        Self {
            state: UNKNOWN.into(),
            ..Self::default()
        }
    }

    /// No source said whether the speaker plays.
    pub(crate) fn is_unknown(&self) -> bool {
        self.state == UNKNOWN
    }

    /// The dashboard's `playing`: `None` while unknown.
    pub(crate) fn plays(&self) -> Option<bool> {
        (!self.is_unknown()).then_some(self.state == PLAYING)
    }

    /// Something to show: playing, paused, or a title. An idle session says
    /// nothing the speaker's own player would not say better.
    fn tells(&self) -> bool {
        self.state == PLAYING || self.state == PAUSED || !self.title.is_empty()
    }
}

/// What a player, or a session's `nowPlayingData`, says. No state at all
/// (`null`, absent) is unknown, not idle.
pub(crate) fn now_playing(p: &Json) -> Playing {
    let state = ["playerState", "state"]
        .iter()
        .find_map(|k| p[*k].as_str())
        .unwrap_or_default()
        .to_uppercase();
    let text = |k: &str| p["infoText"][k].as_str().unwrap_or_default().to_owned();
    Playing {
        state: if state.is_empty() {
            UNKNOWN.into()
        } else {
            state
        },
        title: text("title"),
        artist: text("subText1"),
        volume: p["volume"]["volume"].as_f64(),
        members: p["lemurVolume"]["memberVolume"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(serial, v)| Some((serial.clone(), v.as_f64().or(v["volume"].as_f64())?)))
            .collect(),
    }
}

/// What `/api/media/state` says (the older per-speaker state the open
/// clients read): `currentState` or `state`, a title when it gives one,
/// its volume. No state at all is unknown.
pub(crate) fn media_state(answer: &Json) -> Playing {
    let state = ["currentState", "state", "playerState"]
        .iter()
        .find_map(|k| answer[*k].as_str())
        .unwrap_or_default()
        .to_uppercase();
    let text = |keys: &[&str]| {
        keys.iter()
            .find_map(|k| {
                answer[*k]
                    .as_str()
                    .or_else(|| answer["infoText"][*k].as_str())
            })
            .unwrap_or_default()
            .to_owned()
    };
    Playing {
        state: if state.is_empty() {
            UNKNOWN.into()
        } else {
            state
        },
        title: text(&["title"]),
        artist: text(&["artist", "subText1"]),
        volume: answer["volume"]
            .as_f64()
            .or(answer["volume"]["volume"].as_f64()),
        members: Vec::new(),
    }
}

/// The shape of a `/api/media/state` answer, as [`player_shape`].
pub(crate) fn media_shape(answer: &Json) -> String {
    format!(
        "keys=[{}] state={} title={}",
        keys(answer),
        word(&answer["currentState"]),
        !media_state(answer).title.is_empty(),
    )
}

/// One media session: where it plays, and what.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Session {
    /// (serial, device type when given).
    pub endpoints: Vec<(String, Option<String>)>,
    pub playing: Playing,
}

/// The sessions in a `list-media-sessions` answer.
pub(crate) fn sessions(answer: &Json) -> Vec<Session> {
    list(answer).iter().filter_map(session).collect()
}

fn list(answer: &Json) -> &[Json] {
    if let Some(all) = answer.as_array() {
        return all;
    }
    LISTS
        .iter()
        .find_map(|k| answer[*k].as_array())
        .map_or(&[], Vec::as_slice)
}

fn endpoints(s: &Json) -> Vec<&Json> {
    ["endpointList", "endpoints"]
        .iter()
        .find_map(|k| s[*k].as_array())
        .map_or_else(|| vec![&s["endpoint"], s], |all| all.iter().collect())
}

fn now(s: &Json) -> &Json {
    NOW.iter()
        .map(|k| &s[*k])
        .find(|v| v.is_object())
        .unwrap_or(s)
}

fn session(s: &Json) -> Option<Session> {
    let endpoints: Vec<_> = endpoints(s).into_iter().filter_map(endpoint).collect();
    (!endpoints.is_empty()).then(|| Session {
        endpoints,
        playing: now_playing(now(s)),
    })
}

/// A session's speaker: its serial and type, in `id`, `dopplerId` or itself.
fn endpoint(e: &Json) -> Option<(String, Option<String>)> {
    let places = [&e["id"], &e["dopplerId"], e];
    let find = |keys: &[&str]| {
        places
            .iter()
            .find_map(|p| keys.iter().find_map(|k| p[*k].as_str()))
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
    };
    Some((
        find(&["deviceSerialNumber", "serialNumber"])?,
        find(&["deviceType", "deviceTypeId"]),
    ))
}

/// What the sessions say of `speakers`, by serial: a playing session over a
/// paused one; idle sessions are left to the speakers' own players.
pub(crate) fn from_sessions(speakers: &[&Echo], sessions: &[Session]) -> HashMap<String, Playing> {
    let mut found: HashMap<String, Playing> = HashMap::new();
    for s in sessions.iter().filter(|s| s.playing.tells()) {
        for (serial, kind) in &s.endpoints {
            let Some(e) = speakers
                .iter()
                .find(|e| e.serial == *serial && kind.as_ref().is_none_or(|k| *k == e.device_type))
            else {
                continue;
            };
            if found.get(&e.serial).is_none_or(|p| p.state != PLAYING) {
                found.insert(e.serial.clone(), s.playing.clone());
            }
        }
    }
    found
}

/// A group that plays makes its members play (with their own volume when
/// the group gives it); a member that plays something else keeps it. The
/// members it lit, by serial.
pub(crate) fn through_groups(
    speakers: &[&Echo],
    states: &mut HashMap<String, Playing>,
) -> Vec<String> {
    let mut lit: Vec<(String, Playing)> = Vec::new();
    for g in speakers.iter().filter(|e| e.group) {
        let Some(p) = states.get(&g.serial).filter(|p| p.state == PLAYING) else {
            continue;
        };
        for m in &g.members {
            let known = speakers.iter().any(|e| e.serial == *m && !e.group);
            let plays = states.get(m).is_some_and(|s| s.state == PLAYING);
            if !known || plays || lit.iter().any(|(s, _)| s == m) {
                continue;
            }
            let volume = p.members.iter().find(|(s, _)| s == m).map(|(_, v)| *v);
            lit.push((
                m.clone(),
                Playing {
                    volume,
                    members: Vec::new(),
                    ..p.clone()
                },
            ));
        }
    }
    lit.into_iter()
        .map(|(serial, p)| {
            states.insert(serial.clone(), p);
            serial
        })
        .collect()
}

/// The shape of a sessions' answer for the debug log: its keys, how many
/// sessions and speakers, their states, whether a title came; never a
/// serial, a name, a title or an id.
pub(crate) fn sessions_shape(answer: &Json) -> String {
    let all = list(answer);
    let mut out = format!("keys=[{}] sessions={}", keys(answer), all.len());
    for s in all.iter().take(10) {
        let n = now(s);
        let state = if n["playerState"].is_null() {
            &n["state"]
        } else {
            &n["playerState"]
        };
        let found = endpoints(s).into_iter().filter_map(endpoint).count();
        let _ = write!(
            out,
            " {{keys=[{}] now=[{}] endpoints={found} state={} title={}}}",
            keys(s),
            keys(n),
            word(state),
            !now_playing(n).title.is_empty(),
        );
    }
    out
}

/// The shape of a speaker's `np/player` answer, as [`sessions_shape`].
pub(crate) fn player_shape(answer: &Json) -> String {
    let p = &answer["playerInfo"];
    let lemur = p["isPlayingInLemur"]
        .as_bool()
        .map_or("-", |b| if b { "yes" } else { "no" });
    format!(
        "keys=[{}] player=[{}] state={} title={} lemur={lemur} members={}",
        keys(answer),
        keys(p),
        word(&p["state"]),
        !now_playing(p).title.is_empty(),
        now_playing(p).members.len(),
    )
}

/// An object's keys that are field names (`infoText`), not serials or ids.
fn keys(v: &Json) -> String {
    v.as_object()
        .into_iter()
        .flatten()
        .map(|(k, _)| k.as_str())
        .filter(|k| {
            k.len() <= 40
                && k.bytes().any(|b| b.is_ascii_lowercase())
                && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// A state word (`PLAYING`): `-` when there is none, `?` for anything else.
fn word(v: &Json) -> &str {
    match v.as_str() {
        None => "-",
        Some(s)
            if !s.is_empty()
                && s.len() <= 24
                && s.bytes().all(|b| b.is_ascii_uppercase() || b == b'_') =>
        {
            s
        }
        Some(_) => "?",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn echo(serial: &str, group: bool, members: &[&str]) -> Echo {
        Echo {
            serial: serial.into(),
            device_type: if group {
                "A3C9PE6TNYLTCH"
            } else {
                "A32DOYMUN6DTXA"
            }
            .into(),
            name: serial.into(),
            family: if group { "WHA" } else { "ECHO" }.into(),
            group,
            members: members.iter().map(|m| (*m).to_owned()).collect(),
            online: true,
        }
    }

    fn fixture(name: &str) -> Json {
        let text = match name {
            "sessions" => include_str!("../fixtures/list-media-sessions.json"),
            "group" => include_str!("../fixtures/player-group-playing.json"),
            "member" => include_str!("../fixtures/player-state-null.json"),
            "media" => include_str!("../fixtures/media-state.json"),
            _ => unreachable!(),
        };
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn the_sessions_say_who_plays() {
        let (garage, salon, office) = (
            echo("G0000000000000A1", false, &[]),
            echo("G0000000000000B2", false, &[]),
            echo("G0000000000000C3", false, &[]),
        );
        let all = sessions(&fixture("sessions"));
        assert_eq!(all.len(), 3);
        let states = from_sessions(&[&garage, &salon, &office], &all);
        let p = &states[&garage.serial];
        assert_eq!(
            (p.state.as_str(), p.title.as_str(), p.artist.as_str()),
            ("PLAYING", "Un titre", "Un artiste")
        );
        assert_eq!(states[&salon.serial].state, "PAUSED");
        // An idle session is left to the speaker's own player.
        assert!(!states.contains_key(&office.serial));
        // Another device type under the same serial is not this speaker.
        let mut other = garage.clone();
        other.device_type = "A1RABVCI4QCIKC".into();
        assert!(from_sessions(&[&other], &all).is_empty());
    }

    #[test]
    fn the_sessions_list_is_read_loosely() {
        let flat = json!([{ "deviceSerialNumber": "A", "deviceType": "T",
                            "playerInfo": { "state": "playing", "infoText": { "title": "x" } } }]);
        let s = sessions(&flat);
        assert_eq!(s[0].endpoints, [("A".to_owned(), Some("T".to_owned()))]);
        assert_eq!(s[0].playing.state, "PLAYING");
        let nested = json!({ "mediaSessions": [{ "endpoint": { "dopplerId": { "deviceSerialNumber": "B" } },
                                                 "nowPlayingData": { "playerState": "PAUSED" } }] });
        assert_eq!(sessions(&nested)[0].endpoints, [("B".to_owned(), None)]);
        assert!(sessions(&json!({})).is_empty());
        assert!(sessions(&Json::Null).is_empty());
        assert!(sessions(&json!({ "mediaSessionList": [{ "nowPlayingData": {} }] })).is_empty());
    }

    #[test]
    fn no_state_is_unknown_not_idle() {
        // An Echo Dot answers `state: null` to np/player while it plays.
        let p = crate::api::player_state(&fixture("member"));
        assert_eq!(p.state, UNKNOWN);
        assert_eq!(p.plays(), None);
        assert_eq!(p.volume, Some(30.0));
        assert_eq!(now_playing(&json!({})).plays(), None);
        assert_eq!(
            now_playing(&json!({ "state": "IDLE" })).plays(),
            Some(false)
        );
        assert_eq!(
            now_playing(&json!({ "state": "playing" })).plays(),
            Some(true)
        );
        assert_eq!(Playing::unknown().plays(), None);
        // A session that says nothing is left to the next source.
        let silent = Session {
            endpoints: vec![("A".into(), None)],
            playing: Playing::unknown(),
        };
        assert!(from_sessions(&[&echo("A", false, &[])], &[silent]).is_empty());
    }

    #[test]
    fn the_media_state_is_read_loosely() {
        let p = media_state(&fixture("media"));
        assert_eq!(p.state, "PLAYING");
        assert_eq!(p.plays(), Some(true));
        assert_eq!(p.volume, Some(25.0));
        assert!(p.title.is_empty());
        assert_eq!(media_state(&json!({ "state": "paused" })).state, "PAUSED");
        assert!(media_state(&json!({})).is_unknown());
        let shape = media_shape(&fixture("media"));
        assert!(shape.starts_with("keys=["), "{shape}");
        assert!(shape.contains("state=PLAYING title=false"), "{shape}");
        assert!(!shape.contains("fictif"), "{shape}");
    }

    #[test]
    fn a_group_that_plays_makes_its_members_play() {
        let (garage, salon, cellar) = (
            echo("G0000000000000A1", false, &[]),
            echo("G0000000000000B2", false, &[]),
            echo("G0000000000000D4", false, &[]),
        );
        let everywhere = echo(
            "GROUP000000000E5",
            true,
            &[&garage.serial, &salon.serial, &cellar.serial, "OFFLINE"],
        );
        let speakers = [&garage, &salon, &cellar, &everywhere];
        let group = crate::api::player_state(&fixture("group"));
        assert_eq!(group.members.len(), 2);
        let member = crate::api::player_state(&fixture("member"));
        // A member whose player says nothing is lit by its group all the same.
        assert!(member.is_unknown());
        let elsewhere = Playing {
            state: PLAYING.into(),
            title: "Autre chose".into(),
            ..Playing::default()
        };
        let mut states = HashMap::from([
            (everywhere.serial.clone(), group.clone()),
            (garage.serial.clone(), member.clone()),
            (salon.serial.clone(), member),
            (cellar.serial.clone(), elsewhere.clone()),
        ]);
        let lit = through_groups(&speakers, &mut states);
        assert_eq!(lit, [garage.serial.clone(), salon.serial.clone()]);
        let g = &states[&garage.serial];
        assert_eq!(
            (g.state.as_str(), g.title.as_str()),
            ("PLAYING", "Un titre")
        );
        assert_eq!(g.volume, Some(30.0));
        assert!(g.members.is_empty());
        assert_eq!(states[&salon.serial].volume, Some(45.0));
        assert_eq!(states[&cellar.serial], elsewhere);
        assert!(!states.contains_key("OFFLINE"));
        // A paused group lights no one.
        let mut paused = HashMap::from([(
            everywhere.serial.clone(),
            Playing {
                state: PAUSED.into(),
                ..group
            },
        )]);
        assert!(through_groups(&speakers, &mut paused).is_empty());
    }

    #[test]
    fn the_shapes_name_nothing_of_the_house() {
        let sessions = sessions_shape(&fixture("sessions"));
        let group = player_shape(&fixture("group"));
        for shape in [&sessions, &group] {
            for private in [
                "G0000000000000",
                "GROUP0000",
                "Un titre",
                "Un artiste",
                "amzn1.",
            ] {
                assert!(!shape.contains(private), "{private} in {shape}");
            }
        }
        assert!(sessions.starts_with("keys=[mediaSessionList] sessions=3 "));
        assert!(sessions.contains("endpoints=1 state=PLAYING title=true"));
        assert!(group.contains("state=PLAYING title=true lemur=yes members=2"));
        assert_eq!(word(&json!("Playing now")), "?");
        assert_eq!(keys(&json!({ "G0911ABC": 1, "infoText": 2 })), "infoText");
    }
}
