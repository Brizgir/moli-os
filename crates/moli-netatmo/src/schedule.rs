//! The home's heating schedules, as `homesdata` gives them (`schedules[]` of
//! type `therm`): zones (a name, a kind, a setpoint per room) and a week
//! timetable (which zone from which minute, Monday 0:00 being minute 0).
//! A person edits one in the dashboard; it is checked strictly here before
//! `synchomeschedule` or `createnewhomeschedule` sees it, as a JSON body.

use serde_json::{Map, Value as Json, json};

use crate::encode;
use crate::model::{SETPOINT_MAX, SETPOINT_MIN, SETPOINT_STEP};

/// Minutes in a week: an offset is below it.
pub(crate) const WEEK: u32 = 7 * 24 * 60;
/// Netatmo takes a few zones and a few slots per day: these are well
/// beyond any real schedule, so that nothing absurd is ever sent.
pub(crate) const MAX_ZONES: usize = 20;
pub(crate) const MAX_SLOTS: usize = 1000;
pub(crate) const MAX_NAME: usize = 60;
/// Netatmo's own frost guard and away temperatures when a schedule says none.
const DEFAULT_HG: f64 = 7.0;
const DEFAULT_AWAY: f64 = 12.0;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ZoneRoom {
    pub id: String,
    pub temp: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Zone {
    pub id: u32,
    pub name: String,
    /// Netatmo's kind: 0 comfort, 1 night, 4 custom, 5 eco, 8 comfort+…
    pub kind: u32,
    pub rooms: Vec<ZoneRoom>,
}

/// From `start` (minutes since Monday 0:00), `zone` applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Slot {
    pub zone: u32,
    pub start: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Schedule {
    pub id: String,
    pub name: String,
    pub selected: bool,
    pub default: bool,
    pub hg_temp: f64,
    pub away_temp: f64,
    pub zones: Vec<Zone>,
    pub timetable: Vec<Slot>,
}

fn number(json: &Json) -> Option<f64> {
    json.as_f64().filter(|t| t.is_finite())
}

fn zone_of(json: &Json) -> Option<Zone> {
    let rooms = json["rooms"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            Some(ZoneRoom {
                id: r["id"].as_str()?.to_owned(),
                temp: number(&r["therm_setpoint_temperature"])?,
            })
        })
        .collect();
    Some(Zone {
        id: u32::try_from(json["id"].as_u64()?).ok()?,
        name: json["name"].as_str().unwrap_or_default().to_owned(),
        kind: u32::try_from(json["type"].as_u64().unwrap_or(4)).unwrap_or(4),
        rooms,
    })
}

/// One home's heating schedules (`home` is a `homes[]` entry of `homesdata`).
pub(crate) fn schedules(home: &Json) -> Vec<Schedule> {
    home["schedules"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| s["type"].as_str().is_none_or(|t| t == "therm"))
        .filter_map(|s| {
            Some(Schedule {
                id: s["id"].as_str()?.to_owned(),
                name: s["name"].as_str().unwrap_or_default().to_owned(),
                selected: s["selected"].as_bool().unwrap_or(false),
                default: s["default"].as_bool().unwrap_or(false),
                hg_temp: number(&s["hg_temp"]).unwrap_or(DEFAULT_HG),
                away_temp: number(&s["away_temp"]).unwrap_or(DEFAULT_AWAY),
                zones: s["zones"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(zone_of)
                    .collect(),
                timetable: s["timetable"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|t| {
                        Some(Slot {
                            zone: u32::try_from(t["zone_id"].as_u64()?).ok()?,
                            start: u32::try_from(t["m_offset"].as_u64()?).ok()?,
                        })
                    })
                    .collect(),
            })
        })
        .collect()
}

impl Schedule {
    /// What a surface reads: the same shape a person sends back edited.
    pub(crate) fn to_json(&self) -> Json {
        json!({
            "id": self.id,
            "name": self.name,
            "selected": self.selected,
            "default": self.default,
            "hg_temp": self.hg_temp,
            "away_temp": self.away_temp,
            "zones": self.zones.iter().map(|z| json!({
                "id": z.id,
                "name": z.name,
                "type": z.kind,
                "rooms": z.rooms.iter().map(|r| json!({ "id": r.id, "temp": r.temp })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "timetable": self.timetable.iter().map(|s| json!({ "zone": s.zone, "start": s.start })).collect::<Vec<_>>(),
        })
    }
}

/// Why a schedule is not sent: a catalogue key and its values.
#[derive(Debug, PartialEq)]
pub(crate) enum Wrong {
    /// `field`: what is missing or not of the right shape.
    Shape(&'static str),
    Name,
    Temperature(String),
    NoZone,
    TooManyZones,
    ZoneTwice(u32),
    ZoneKind(u32),
    /// A zone names a room the home has not.
    UnknownRoom(String),
    /// A zone (name) says nothing for a heated room (name).
    RoomMissing(String, String),
    NoSlot,
    TooManySlots,
    /// The first slot is not Monday 0:00.
    NotFromMonday,
    /// Slots are not strictly increasing, or past the week.
    SlotOrder(u32),
    /// A slot names a zone the schedule has not.
    UnknownZone(u32),
}

/// A schedule as a person sends it: checked against the home.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Draft {
    pub name: String,
    pub hg_temp: f64,
    pub away_temp: f64,
    pub zones: Vec<Zone>,
    pub timetable: Vec<Slot>,
}

/// A temperature Netatmo takes: 7 to 30 °C by halves.
fn setpoint(json: &Json, what: &str) -> Result<f64, Wrong> {
    let t = number(json).ok_or_else(|| Wrong::Temperature(what.to_owned()))?;
    let steps = t / SETPOINT_STEP;
    if !(SETPOINT_MIN..=SETPOINT_MAX).contains(&t) || (steps - steps.round()).abs() > 1e-6 {
        return Err(Wrong::Temperature(what.to_owned()));
    }
    Ok(steps.round() * SETPOINT_STEP)
}

/// A name of at most [`MAX_NAME`] characters, none a control one;
/// `empty`: may it be empty (a zone Netatmo names by its kind).
fn clean_name(json: &Json, empty: bool) -> Result<String, Wrong> {
    let name = json.as_str().ok_or(Wrong::Shape("name"))?.trim();
    let clean = name.chars().count() <= MAX_NAME
        && (empty || !name.is_empty())
        && !name.chars().any(char::is_control);
    if !clean {
        return Err(Wrong::Name);
    }
    Ok(name.to_owned())
}

fn small(json: &Json, field: &'static str) -> Result<u32, Wrong> {
    json.as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .ok_or(Wrong::Shape(field))
}

impl Draft {
    /// Reads and checks what a person sent. `rooms`: every room of the
    /// home `(id, name)`; `heated`: the ids of those with a thermostat or a
    /// valve (each zone says a temperature for each of them).
    pub(crate) fn parse(
        input: &Json,
        rooms: &[(String, String)],
        heated: &[&str],
    ) -> Result<Self, Wrong> {
        let name = clean_name(&input["name"], false)?;
        let hg_temp = setpoint(&input["hg_temp"], "hg_temp")?;
        let away_temp = setpoint(&input["away_temp"], "away_temp")?;
        let zones_in = input["zones"].as_array().ok_or(Wrong::Shape("zones"))?;
        if zones_in.is_empty() {
            return Err(Wrong::NoZone);
        }
        if zones_in.len() > MAX_ZONES {
            return Err(Wrong::TooManyZones);
        }
        let mut zones: Vec<Zone> = Vec::with_capacity(zones_in.len());
        for z in zones_in {
            let id = small(&z["id"], "zones[].id")?;
            if zones.iter().any(|known| known.id == id) {
                return Err(Wrong::ZoneTwice(id));
            }
            let kind = small(&z["type"], "zones[].type")?;
            if kind > 8 {
                return Err(Wrong::ZoneKind(kind));
            }
            let zone_name = clean_name(&z["name"], true)?;
            let mut seen: Vec<ZoneRoom> = Vec::new();
            for r in z["rooms"].as_array().ok_or(Wrong::Shape("zones[].rooms"))? {
                let room = r["id"].as_str().ok_or(Wrong::Shape("zones[].rooms[].id"))?;
                if !rooms.iter().any(|(id, _)| id == room) {
                    return Err(Wrong::UnknownRoom(room.to_owned()));
                }
                let temp = setpoint(&r["temp"], &format!("{zone_name} / {room}"))?;
                if let Some(known) = seen.iter_mut().find(|k| k.id == room) {
                    known.temp = temp;
                } else {
                    seen.push(ZoneRoom {
                        id: room.to_owned(),
                        temp,
                    });
                }
            }
            for id in heated {
                if !seen.iter().any(|r| r.id == *id) {
                    let room_name = rooms
                        .iter()
                        .find(|(rid, _)| rid == id)
                        .map_or_else(|| (*id).to_owned(), |(_, n)| n.clone());
                    let zone = if zone_name.is_empty() {
                        id.to_string()
                    } else {
                        zone_name
                    };
                    return Err(Wrong::RoomMissing(zone, room_name));
                }
            }
            zones.push(Zone {
                id,
                name: zone_name,
                kind,
                rooms: seen,
            });
        }
        let slots = input["timetable"]
            .as_array()
            .ok_or(Wrong::Shape("timetable"))?;
        if slots.is_empty() {
            return Err(Wrong::NoSlot);
        }
        if slots.len() > MAX_SLOTS {
            return Err(Wrong::TooManySlots);
        }
        let mut timetable: Vec<Slot> = Vec::with_capacity(slots.len());
        for s in slots {
            let zone = small(&s["zone"], "timetable[].zone")?;
            let start = small(&s["start"], "timetable[].start")?;
            if !zones.iter().any(|z| z.id == zone) {
                return Err(Wrong::UnknownZone(zone));
            }
            match timetable.last() {
                None if start != 0 => return Err(Wrong::NotFromMonday),
                Some(last) if start <= last.start => return Err(Wrong::SlotOrder(start)),
                _ if start >= WEEK => return Err(Wrong::SlotOrder(start)),
                _ => {}
            }
            timetable.push(Slot { zone, start });
        }
        Ok(Self {
            name,
            hg_temp,
            away_temp,
            zones,
            timetable,
        })
    }

    /// The JSON body of `synchomeschedule` (with `schedule_id`) or of
    /// `createnewhomeschedule` (without): `zones` and `timetable` are JSON
    /// arrays in the body, as Netatmo reads them.
    pub(crate) fn body(&self, home: &str, schedule: Option<&str>) -> String {
        let mut body = Map::new();
        body.insert("home_id".into(), json!(home));
        if let Some(id) = schedule {
            body.insert("schedule_id".into(), json!(id));
        }
        body.insert("name".into(), json!(self.name));
        body.insert("hg_temp".into(), json!(self.hg_temp));
        body.insert("away_temp".into(), json!(self.away_temp));
        body.insert(
            "zones".into(),
            Json::Array(
                self.zones
                    .iter()
                    .map(|z| {
                        json!({
                            "id": z.id,
                            "name": z.name,
                            "type": z.kind,
                            "rooms": z.rooms.iter().map(|r| json!({
                                "id": r.id,
                                "therm_setpoint_temperature": r.temp,
                            })).collect::<Vec<_>>(),
                        })
                    })
                    .collect(),
            ),
        );
        body.insert(
            "timetable".into(),
            Json::Array(
                self.timetable
                    .iter()
                    .map(|s| json!({ "zone_id": s.zone, "m_offset": s.start }))
                    .collect(),
            ),
        );
        Json::Object(body).to_string()
    }
}

/// The form of `switchhomeschedule`.
pub(crate) fn switch_form(home: &str, schedule: &str) -> String {
    encode::form(&[("home_id", home), ("schedule_id", schedule)])
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOMESDATA: &str = include_str!("fixtures/homesdata.json");

    fn home() -> Json {
        serde_json::from_str::<Json>(HOMESDATA).unwrap()["body"]["homes"][0].clone()
    }

    fn rooms() -> Vec<(String, String)> {
        ["1001", "1002", "1003", "1004"]
            .iter()
            .zip(["Salon", "Chambre", "Bureau", "Garage"])
            .map(|(id, name)| ((*id).to_owned(), name.to_owned()))
            .collect()
    }

    const HEATED: [&str; 3] = ["1001", "1002", "1003"];

    #[test]
    fn the_schedules_of_a_home_are_read() {
        let found = schedules(&home());
        assert_eq!(found.len(), 2, "the event schedule is not a heating one");
        let week = &found[0];
        assert_eq!(week.id, "sched-1");
        assert_eq!(week.name, "Semaine");
        assert!(week.selected && week.default);
        assert!((week.hg_temp - 7.0).abs() < 1e-9 && (week.away_temp - 12.0).abs() < 1e-9);
        let names: Vec<&str> = week.zones.iter().map(|z| z.name.as_str()).collect();
        assert_eq!(names, ["Confort", "Nuit", "Éco"]);
        assert_eq!(week.zones[1].kind, 1);
        assert_eq!(
            week.zones[0].rooms[1],
            ZoneRoom {
                id: "1002".into(),
                temp: 18.0
            }
        );
        assert_eq!(week.timetable[0], Slot { zone: 1, start: 0 });
        assert_eq!(
            week.timetable[1],
            Slot {
                zone: 0,
                start: 390
            }
        );
        assert_eq!(week.timetable.len(), 23);
        assert!(!found[1].selected);
        assert!(
            (found[1].hg_temp - 7.0).abs() < 1e-9,
            "no hg_temp: Netatmo's default"
        );
        let json = week.to_json();
        assert_eq!(json["zones"][0]["rooms"][0]["temp"], 20.0);
        assert_eq!(json["timetable"][1]["start"], 390);
    }

    fn draft() -> Json {
        json!({
            "name": "Essai",
            "hg_temp": 7,
            "away_temp": 12.5,
            "zones": [
                { "id": 0, "name": "Confort", "type": 0, "rooms": [
                    { "id": "1001", "temp": 20 }, { "id": "1002", "temp": 18 }, { "id": "1003", "temp": 19.5 }
                ] },
                { "id": 1, "name": "Nuit", "type": 1, "rooms": [
                    { "id": "1001", "temp": 17 }, { "id": "1002", "temp": 17 }, { "id": "1003", "temp": 16 }
                ] }
            ],
            "timetable": [
                { "zone": 1, "start": 0 }, { "zone": 0, "start": 390 }, { "zone": 1, "start": 1320 },
                { "zone": 0, "start": 1440 + 420 }
            ]
        })
    }

    #[test]
    fn a_draft_becomes_the_body_netatmo_reads() {
        let draft = Draft::parse(&draft(), &rooms(), &HEATED).unwrap();
        assert_eq!(draft.name, "Essai");
        let body = draft.body("home-1", Some("sched-1"));
        let json: Json = serde_json::from_str(&body).unwrap();
        assert_eq!(json["home_id"], "home-1");
        assert_eq!(json["schedule_id"], "sched-1");
        assert_eq!(json["name"], "Essai");
        assert_eq!(json["hg_temp"], 7.0);
        assert_eq!(json["away_temp"], 12.5);
        assert_eq!(
            json["zones"][0],
            json!({ "id": 0, "name": "Confort", "type": 0, "rooms": [
                { "id": "1001", "therm_setpoint_temperature": 20.0 },
                { "id": "1002", "therm_setpoint_temperature": 18.0 },
                { "id": "1003", "therm_setpoint_temperature": 19.5 }
            ] })
        );
        assert_eq!(
            json["timetable"],
            json!([
                { "zone_id": 1, "m_offset": 0 }, { "zone_id": 0, "m_offset": 390 },
                { "zone_id": 1, "m_offset": 1320 }, { "zone_id": 0, "m_offset": 1860 }
            ])
        );
        assert!(json["zones"].is_array() && json["timetable"].is_array());
        let created: Json = serde_json::from_str(&draft.body("home-1", None)).unwrap();
        assert!(created.get("schedule_id").is_none());
        assert_eq!(
            switch_form("home-1", "sched 2"),
            "home_id=home-1&schedule_id=sched%202"
        );
    }

    /// The test draft with `patch` applied, checked.
    fn check(patch: &dyn Fn(&mut Json)) -> Result<Draft, Wrong> {
        let mut d = draft();
        patch(&mut d);
        Draft::parse(&d, &rooms(), &HEATED)
    }

    #[test]
    fn names_temperatures_and_zones_are_checked_strictly() {
        assert_eq!(
            check(&|d| d["name"] = json!("  ")).unwrap_err(),
            Wrong::Name
        );
        assert_eq!(
            check(&|d| d["name"] = json!("x".repeat(61))).unwrap_err(),
            Wrong::Name
        );
        assert_eq!(
            check(&|d| d["name"] = json!("a\u{7}b")).unwrap_err(),
            Wrong::Name
        );
        // A zone Netatmo names by its kind only: kept nameless.
        assert!(check(&|d| d["zones"][0]["name"] = json!("")).is_ok());
        assert_eq!(
            check(&|d| d["hg_temp"] = json!(6.5)).unwrap_err(),
            Wrong::Temperature("hg_temp".into())
        );
        assert_eq!(
            check(&|d| d["away_temp"] = json!(30.5)).unwrap_err(),
            Wrong::Temperature("away_temp".into())
        );
        assert!(matches!(
            check(&|d| d["zones"][0]["rooms"][0]["temp"] = json!(20.3)).unwrap_err(),
            Wrong::Temperature(_)
        ));
        assert!(matches!(
            check(&|d| d["zones"][0]["rooms"][0]["temp"] = json!("chaud")).unwrap_err(),
            Wrong::Temperature(_)
        ));
        assert_eq!(
            check(&|d| d["zones"][0]["rooms"][0]["id"] = json!("9999")).unwrap_err(),
            Wrong::UnknownRoom("9999".into())
        );
        // The garage has no heater: a zone may still say something for it.
        assert!(
            check(&|d| d["zones"][0]["rooms"]
                .as_array_mut()
                .unwrap()
                .push(json!({ "id": "1004", "temp": 15 })))
            .is_ok()
        );
        assert_eq!(
            check(&|d| {
                d["zones"][1]["rooms"].as_array_mut().unwrap().pop();
            })
            .unwrap_err(),
            Wrong::RoomMissing("Nuit".into(), "Bureau".into())
        );
        assert_eq!(
            check(&|d| d["zones"][1]["id"] = json!(0)).unwrap_err(),
            Wrong::ZoneTwice(0)
        );
        assert_eq!(
            check(&|d| d["zones"][1]["type"] = json!(9)).unwrap_err(),
            Wrong::ZoneKind(9)
        );
        assert_eq!(
            check(&|d| d["zones"] = json!([])).unwrap_err(),
            Wrong::NoZone
        );
        assert_eq!(
            check(&|d| {
                let zone = d["zones"][0].clone();
                let zones = d["zones"].as_array_mut().unwrap();
                for i in 2..=MAX_ZONES as u64 {
                    let mut z = zone.clone();
                    z["id"] = json!(i);
                    zones.push(z);
                }
            })
            .unwrap_err(),
            Wrong::TooManyZones
        );
    }

    #[test]
    fn the_week_is_checked_strictly() {
        assert_eq!(
            check(&|d| d["timetable"] = json!([])).unwrap_err(),
            Wrong::NoSlot
        );
        assert_eq!(
            check(&|d| d["timetable"][0]["start"] = json!(30)).unwrap_err(),
            Wrong::NotFromMonday
        );
        assert_eq!(
            check(&|d| d["timetable"][2]["start"] = json!(390)).unwrap_err(),
            Wrong::SlotOrder(390)
        );
        assert_eq!(
            check(&|d| d["timetable"][3]["start"] = json!(WEEK)).unwrap_err(),
            Wrong::SlotOrder(WEEK)
        );
        assert_eq!(
            check(&|d| d["timetable"][3]["zone"] = json!(7)).unwrap_err(),
            Wrong::UnknownZone(7)
        );
        assert_eq!(
            check(&|d| {
                let slots = d["timetable"].as_array_mut().unwrap();
                for i in 4..=MAX_SLOTS as u64 {
                    slots.push(json!({ "zone": 0, "start": 1860 + i }));
                }
            })
            .unwrap_err(),
            Wrong::TooManySlots
        );
        assert_eq!(
            check(&|d| d["timetable"] = json!("lundi")).unwrap_err(),
            Wrong::Shape("timetable")
        );
        // Bounds are fine, halves are fine.
        assert!(check(&|d| d["zones"][0]["rooms"][0]["temp"] = json!(30)).is_ok());
        assert!(check(&|d| d["hg_temp"] = json!(7)).is_ok());
    }
}
