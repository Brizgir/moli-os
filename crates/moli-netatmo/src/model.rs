//! What Netatmo's Energy answers mean for Moli: `homesdata` gives the homes,
//! their heated rooms and their modules; `homestatus` gives each room's
//! temperature, setpoint and heating, each module's battery and signal.
//! Orders become `setroomthermpoint` and `setthermmode` forms.

use std::sync::Arc;

use moli_core::{Access, Kind, PointSpec, Semantic, Unit, Value};
use serde_json::Value as Json;

use crate::encode;
use crate::schedule::{self, Schedule};

/// Netatmo's own bounds for a room's setpoint (°C).
pub(crate) const SETPOINT_MIN: f64 = 7.0;
pub(crate) const SETPOINT_MAX: f64 = 30.0;
pub(crate) const SETPOINT_STEP: f64 = 0.5;
/// A manual setpoint lasts this long unless the home or the config says
/// otherwise (Netatmo's own default in its app).
pub(crate) const DEFAULT_MINUTES: u32 = 180;

/// The Energy modules: relay, thermostat, valve, OpenTherm relay and thermostat.
const ENERGY: [&str; 5] = ["NAPlug", "NATherm1", "NRV", "OTH", "OTM"];
/// The ones that heat a room (a room without one is not shown).
const HEATERS: [&str; 3] = ["NATherm1", "NRV", "OTM"];
/// The ones on batteries.
const BATTERIES: [&str; 2] = ["NATherm1", "NRV"];

pub(crate) const HOME_MODES: [&str; 3] = ["schedule", "away", "hg"];
pub(crate) const ROOM_MODES: [&str; 6] = ["schedule", "manual", "max", "away", "hg", "off"];
const BATTERY_STATES: [&str; 6] = ["max", "full", "high", "medium", "low", "very_low"];

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Home {
    pub id: String,
    pub name: String,
    pub rooms: Vec<Room>,
    pub modules: Vec<Module>,
    /// The home's own duration for a manual setpoint, in minutes.
    pub default_minutes: Option<u32>,
    /// Every room `(id, name)`, heated or not: a schedule may name any.
    pub all_rooms: Vec<(String, String)>,
    /// Its heating schedules, one selected.
    pub schedules: Vec<Schedule>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Room {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Module {
    pub id: String,
    /// `NATherm1`, `NRV`, `NAPlug`…
    pub kind: String,
    pub name: String,
    pub room: Option<String>,
}

impl Module {
    /// Netatmo's product name.
    pub(crate) fn model(&self) -> &str {
        match self.kind.as_str() {
            "NATherm1" => "Smart Thermostat",
            "NRV" => "Smart Radiator Valve",
            "NAPlug" => "Relay",
            "OTH" => "OpenTherm Relay",
            "OTM" => "OpenTherm Thermostat",
            other => other,
        }
    }

    pub(crate) fn has_battery(&self) -> bool {
        BATTERIES.contains(&self.kind.as_str())
    }

    /// Its Moli id: the address without its colons.
    pub(crate) fn native(&self) -> String {
        format!("module-{}", self.id.replace(':', "").to_lowercase())
    }
}

impl Home {
    pub(crate) fn native(&self) -> String {
        format!("home-{}", self.id)
    }

    pub(crate) fn room_native(&self, room: &Room) -> String {
        format!("room-{}-{}", self.id, room.id)
    }

    pub(crate) fn room_name(&self, id: &str) -> Option<&str> {
        self.rooms
            .iter()
            .find(|r| r.id == id)
            .map(|r| r.name.as_str())
    }

    /// The selected schedule's name, if one is.
    pub(crate) fn active_schedule(&self) -> Option<&Schedule> {
        self.schedules.iter().find(|s| s.selected)
    }

    /// The schedules' names, each once: the `schedule` point's values.
    pub(crate) fn schedule_names(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for s in &self.schedules {
            if !names.contains(&s.name) {
                names.push(s.name.clone());
            }
        }
        names
    }

    /// The schedule a person names (the first of that name).
    pub(crate) fn schedule_named(&self, name: &str) -> Option<&Schedule> {
        self.schedules.iter().find(|s| s.name == name)
    }

    /// What a surface reads: the home, its rooms, its schedules.
    pub(crate) fn schedules_json(&self) -> Json {
        serde_json::json!({
            "home": { "id": self.id, "name": self.name },
            "rooms": self.rooms.iter().map(|r| serde_json::json!({ "id": r.id, "name": r.name })).collect::<Vec<_>>(),
            "active": self.active_schedule().map(|s| s.id.as_str()),
            "schedules": self.schedules.iter().map(Schedule::to_json).collect::<Vec<_>>(),
        })
    }
}

fn text(json: &Json, key: &str) -> String {
    json[key].as_str().unwrap_or_default().to_owned()
}

/// The homes with Energy modules, their heated rooms and their modules.
pub(crate) fn homes(answer: &Json) -> Vec<Home> {
    let mut found = Vec::new();
    for home in answer["body"]["homes"].as_array().into_iter().flatten() {
        let modules: Vec<Module> = home["modules"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|m| ENERGY.contains(&m["type"].as_str().unwrap_or_default()))
            .map(|m| Module {
                id: text(m, "id"),
                kind: text(m, "type"),
                name: text(m, "name"),
                room: m["room_id"].as_str().map(str::to_owned),
            })
            .filter(|m| !m.id.is_empty())
            .collect();
        let heated = |room: &str| {
            modules
                .iter()
                .any(|m| m.room.as_deref() == Some(room) && HEATERS.contains(&m.kind.as_str()))
        };
        let all_rooms: Vec<Room> = home["rooms"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|r| Room {
                id: text(r, "id"),
                name: text(r, "name"),
            })
            .filter(|r| !r.id.is_empty())
            .collect();
        let rooms: Vec<Room> = all_rooms
            .iter()
            .filter(|r| heated(&r.id))
            .cloned()
            .collect();
        if rooms.is_empty() {
            continue;
        }
        found.push(Home {
            id: text(home, "id"),
            name: text(home, "name"),
            rooms,
            modules,
            default_minutes: home["therm_setpoint_default_duration"]
                .as_u64()
                .and_then(|m| u32::try_from(m).ok())
                .filter(|m| *m > 0),
            all_rooms: all_rooms.into_iter().map(|r| (r.id, r.name)).collect(),
            schedules: schedule::schedules(home),
        });
    }
    found
}

/// One room's state, as `homestatus` gives it.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RoomStatus {
    pub id: String,
    pub reachable: Option<bool>,
    pub measured: Option<f64>,
    pub setpoint: Option<f64>,
    pub mode: Option<String>,
    /// When a manual setpoint ends (Unix seconds).
    pub setpoint_end: Option<u64>,
    pub power_request: Option<f64>,
}

/// One module's state.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ModuleStatus {
    pub id: String,
    pub reachable: Option<bool>,
    pub battery: Option<String>,
    pub battery_mv: Option<f64>,
    pub rf_strength: Option<f64>,
    pub boiler: Option<bool>,
}

pub(crate) fn status(answer: &Json) -> (Vec<RoomStatus>, Vec<ModuleStatus>) {
    let home = &answer["body"]["home"];
    let rooms = home["rooms"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| RoomStatus {
            id: text(r, "id"),
            reachable: r["reachable"].as_bool(),
            measured: r["therm_measured_temperature"].as_f64(),
            setpoint: r["therm_setpoint_temperature"].as_f64(),
            mode: r["therm_setpoint_mode"].as_str().map(str::to_owned),
            setpoint_end: r["therm_setpoint_end_time"].as_u64().filter(|t| *t > 0),
            power_request: r["heating_power_request"].as_f64(),
        })
        .collect();
    let modules = home["modules"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|m| ModuleStatus {
            id: text(m, "id"),
            reachable: m["reachable"].as_bool(),
            battery: m["battery_state"].as_str().map(str::to_owned),
            battery_mv: m["battery_level"].as_f64(),
            rf_strength: m["rf_strength"].as_f64(),
            boiler: m["boiler_status"].as_bool(),
        })
        .collect();
    (rooms, modules)
}

/// A room's points from its status; `boiler`: what its thermostat says,
/// for a room without valves (no power request of its own).
pub(crate) fn room_values(status: &RoomStatus, boiler: Option<bool>) -> Vec<(&'static str, Value)> {
    let mut values = Vec::new();
    if let Some(t) = status.measured {
        values.push(("temperature", Value::Float(t)));
    }
    if let Some(t) = status.setpoint {
        values.push(("setpoint", Value::Float(t)));
    }
    if let Some(mode) = &status.mode {
        values.push(("setpoint_mode", Value::Text(mode.as_str().into())));
    }
    // Milliseconds, as every timestamp Moli shows; none: no end (or no
    // manual setpoint).
    values.push((
        "setpoint_end",
        status
            .setpoint_end
            .filter(|_| status.mode.as_deref() == Some("manual"))
            .map_or(Value::Null, |t| {
                Value::Int(i64::try_from(t * 1000).unwrap_or(i64::MAX))
            }),
    ));
    if let Some(heating) = status.power_request.map(|p| p > 0.0).or(boiler) {
        values.push(("heating", Value::Bool(heating)));
    }
    values
}

/// A module's points from its status.
pub(crate) fn module_values(status: &ModuleStatus) -> Vec<(&'static str, Value)> {
    let mut values = vec![("reachable", Value::Bool(status.reachable.unwrap_or(true)))];
    if let Some(state) = &status.battery {
        values.push(("battery", Value::Text(state.as_str().into())));
        values.push((
            "battery_low",
            Value::Bool(matches!(state.as_str(), "low" | "very_low")),
        ));
    }
    if let Some(mv) = status.battery_mv {
        values.push(("battery_voltage", Value::Float(mv)));
    }
    // Netatmo gives the strength without its sign (60 good, 90 weak): dBm.
    if let Some(rf) = status.rf_strength {
        values.push(("rf_signal", Value::Float(-rf.abs())));
    }
    values
}

/// The home's mode, from its rooms: away or frost guard when every room
/// says so, the schedule otherwise.
pub(crate) fn home_mode(rooms: &[RoomStatus]) -> Option<&'static str> {
    let modes: Vec<&str> = rooms.iter().filter_map(|r| r.mode.as_deref()).collect();
    if modes.is_empty() {
        return None;
    }
    Some(if modes.iter().all(|m| *m == "away") {
        "away"
    } else if modes.iter().all(|m| *m == "hg") {
        "hg"
    } else {
        "schedule"
    })
}

fn point(
    key: &str,
    label: &str,
    kind: Kind,
    write: bool,
    unit: Option<Unit>,
    semantic: Semantic,
) -> PointSpec {
    PointSpec {
        key: key.into(),
        label: label.into(),
        kind,
        access: Access { read: true, write },
        unit,
        semantic,
    }
}

fn values(list: &[&str]) -> Kind {
    Kind::Enum {
        values: list.iter().map(|v| Arc::from(*v)).collect(),
    }
}

fn numeric() -> Kind {
    Kind::Numeric {
        min: None,
        max: None,
        step: None,
    }
}

pub(crate) fn room_points() -> Vec<PointSpec> {
    use moli_i18n::tr;
    vec![
        point(
            "temperature",
            &tr!("pilotes.netatmo.temperature"),
            numeric(),
            false,
            Some(Unit::Celsius),
            Semantic::Temperature,
        ),
        point(
            "setpoint",
            &tr!("pilotes.netatmo.consigne"),
            Kind::Numeric {
                min: Some(SETPOINT_MIN),
                max: Some(SETPOINT_MAX),
                step: Some(SETPOINT_STEP),
            },
            true,
            Some(Unit::Celsius),
            Semantic::Config,
        ),
        point(
            "setpoint_mode",
            &tr!("pilotes.netatmo.mode_consigne"),
            values(&ROOM_MODES),
            true,
            None,
            Semantic::Config,
        ),
        point(
            "setpoint_end",
            &tr!("pilotes.netatmo.fin_consigne"),
            numeric(),
            false,
            Some(Unit::Other("ms".into())),
            Semantic::Other,
        ),
        point(
            "heating",
            &tr!("pilotes.netatmo.chauffe"),
            Kind::Binary,
            false,
            None,
            Semantic::Other,
        ),
    ]
}

/// The home's points: its mode, and its active schedule among `schedules`
/// (the names; writing one switches to it).
pub(crate) fn home_points(schedules: &[String]) -> Vec<PointSpec> {
    use moli_i18n::tr;
    let names: Vec<&str> = schedules.iter().map(String::as_str).collect();
    vec![
        point(
            "mode",
            &tr!("pilotes.netatmo.mode"),
            values(&HOME_MODES),
            true,
            None,
            Semantic::Config,
        ),
        point(
            "schedule",
            &tr!("pilotes.netatmo.planning"),
            values(&names),
            !names.is_empty(),
            None,
            Semantic::Config,
        ),
    ]
}

pub(crate) fn module_points(module: &Module) -> Vec<PointSpec> {
    use moli_i18n::tr;
    let mut points = vec![
        point(
            "reachable",
            &tr!("pilotes.netatmo.joignable"),
            Kind::Binary,
            false,
            None,
            Semantic::Other,
        ),
        point(
            "rf_signal",
            &tr!("pilotes.netatmo.signal"),
            numeric(),
            false,
            Some(Unit::Other("dBm".into())),
            Semantic::SignalStrength,
        ),
    ];
    if module.has_battery() {
        points.extend([
            point(
                "battery",
                &tr!("pilotes.netatmo.batterie"),
                values(&BATTERY_STATES),
                false,
                None,
                Semantic::Other,
            ),
            point(
                "battery_low",
                &tr!("pilotes.netatmo.batterie_faible"),
                Kind::Binary,
                false,
                None,
                Semantic::BatteryLow,
            ),
            point(
                "battery_voltage",
                &tr!("pilotes.netatmo.tension"),
                numeric(),
                false,
                Some(Unit::MilliVolt),
                Semantic::Voltage,
            ),
        ]);
    }
    points
}

/// What an order is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    Home { home: String },
    Room { home: String, room: String },
}

impl Target {
    pub(crate) fn home(&self) -> &str {
        match self {
            Self::Home { home } | Self::Room { home, .. } => home,
        }
    }
}

/// Why an order is not sent (a catalogue key, with its values).
#[derive(Debug, PartialEq)]
pub(crate) enum Wrong {
    OutOfBounds,
    RoomMode,
    ReadOnly(String),
    /// No schedule of that name in the home.
    NoSuchSchedule(String),
}

/// The request for an order: its path and its form. `now`: Unix seconds;
/// `home`: the home as last described, for what names its schedules.
pub(crate) fn order(
    target: &Target,
    key: &str,
    value: &Value,
    minutes: u32,
    now: u64,
    home: Option<&Home>,
) -> Result<(&'static str, String), Wrong> {
    match (target, key) {
        (Target::Room { home, room }, "setpoint") => {
            let wanted = value
                .as_f64()
                .filter(|t| (SETPOINT_MIN..=SETPOINT_MAX).contains(t))
                .ok_or(Wrong::OutOfBounds)?;
            let temp = (wanted / SETPOINT_STEP).round() * SETPOINT_STEP;
            let end = now + u64::from(minutes) * 60;
            Ok((
                "/api/setroomthermpoint",
                encode::form(&[
                    ("home_id", home),
                    ("room_id", room),
                    ("mode", "manual"),
                    ("temp", &format!("{temp}")),
                    ("endtime", &end.to_string()),
                ]),
            ))
        }
        (Target::Room { home, room }, "setpoint_mode") => match value {
            // Back to the home's schedule (Netatmo calls it `home`).
            Value::Text(t) if &**t == "schedule" => Ok((
                "/api/setroomthermpoint",
                encode::form(&[("home_id", home), ("room_id", room), ("mode", "home")]),
            )),
            _ => Err(Wrong::RoomMode),
        },
        (Target::Home { home }, "mode") => match value {
            Value::Text(t) if HOME_MODES.contains(&&**t) => Ok((
                "/api/setthermmode",
                encode::form(&[("home_id", home), ("mode", t)]),
            )),
            _ => Err(Wrong::ReadOnly(key.to_owned())),
        },
        (Target::Home { home: id }, "schedule") => {
            let Value::Text(name) = value else {
                return Err(Wrong::ReadOnly(key.to_owned()));
            };
            let schedule = home
                .filter(|h| h.id == *id)
                .and_then(|h| h.schedule_named(name))
                .ok_or_else(|| Wrong::NoSuchSchedule(name.to_string()))?;
            Ok((
                "/api/switchhomeschedule",
                schedule::switch_form(id, &schedule.id),
            ))
        }
        (_, key) => Err(Wrong::ReadOnly(key.to_owned())),
    }
}

/// The path that reads one home's state.
pub(crate) fn status_path(home: &str) -> String {
    format!("/api/homestatus?{}", encode::form(&[("home_id", home)]))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOMESDATA: &str = include_str!("fixtures/homesdata.json");
    const HOMESTATUS: &str = include_str!("fixtures/homestatus.json");

    fn fixture(text: &str) -> Json {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn only_heated_rooms_and_energy_modules_are_kept() {
        let homes = homes(&fixture(HOMESDATA));
        // The second home has only a camera: not an Energy home.
        assert_eq!(homes.len(), 1);
        let home = &homes[0];
        assert_eq!(home.name, "Maison");
        let rooms: Vec<&str> = home.rooms.iter().map(|r| r.name.as_str()).collect();
        // The garage has no thermostat nor valve.
        assert_eq!(rooms, ["Salon", "Chambre", "Bureau"]);
        let kinds: Vec<&str> = home.modules.iter().map(|m| m.kind.as_str()).collect();
        assert_eq!(kinds, ["NAPlug", "NATherm1", "NRV", "NRV"]);
        assert_eq!(home.default_minutes, Some(180));
        assert_eq!(home.modules[2].native(), "module-aabbccddee03");
        assert_eq!(home.modules[1].model(), "Smart Thermostat");
        assert!(!home.modules[0].has_battery());
        assert_eq!(home.room_name("1002"), Some("Chambre"));
        assert_eq!(home.all_rooms.len(), 4);
        assert_eq!(home.schedules.len(), 2);
        assert_eq!(
            home.active_schedule().map(|s| s.name.as_str()),
            Some("Semaine")
        );
        assert_eq!(home.schedule_names(), ["Semaine", "Vacances"]);
        let json = home.schedules_json();
        assert_eq!(json["active"], "sched-1");
        assert_eq!(json["rooms"].as_array().unwrap().len(), 3);
        assert_eq!(json["schedules"][1]["name"], "Vacances");
    }

    #[test]
    fn the_status_gives_each_point() {
        let (rooms, modules) = status(&fixture(HOMESTATUS));
        let salon = &rooms[0];
        // The living room has a thermostat, no valve: its boiler says.
        assert_eq!(
            room_values(salon, Some(true)),
            [
                ("temperature", Value::Float(19.5)),
                ("setpoint", Value::Float(20.0)),
                ("setpoint_mode", Value::Text("schedule".into())),
                ("setpoint_end", Value::Null),
                ("heating", Value::Bool(true)),
            ]
        );
        let bureau = &rooms[2];
        assert_eq!(bureau.power_request, Some(0.0));
        assert!(room_values(bureau, Some(true)).contains(&("heating", Value::Bool(false))));
        let chambre = room_values(&rooms[1], None);
        assert!(chambre.contains(&("setpoint_mode", Value::Text("manual".into()))));
        assert!(chambre.contains(&("setpoint_end", Value::Int(1_700_010_000_000))));
        let valve = modules
            .iter()
            .find(|m| m.id == "aa:bb:cc:dd:ee:04")
            .unwrap();
        assert_eq!(
            module_values(valve),
            [
                ("reachable", Value::Bool(false)),
                ("battery", Value::Text("low".into())),
                ("battery_low", Value::Bool(true)),
                ("battery_voltage", Value::Float(2650.0)),
                ("rf_signal", Value::Float(-85.0)),
            ]
        );
        // The relay says nothing of its reach: it answers, so it is reachable.
        let relay = modules
            .iter()
            .find(|m| m.id == "aa:bb:cc:dd:ee:01")
            .unwrap();
        assert_eq!(
            module_values(relay),
            [
                ("reachable", Value::Bool(true)),
                ("rf_signal", Value::Float(-62.0)),
            ]
        );
    }

    #[test]
    fn the_home_mode_follows_the_rooms() {
        let room = |mode: &str| RoomStatus {
            mode: Some(mode.into()),
            ..RoomStatus::default()
        };
        assert_eq!(home_mode(&[room("away"), room("away")]), Some("away"));
        assert_eq!(home_mode(&[room("hg"), room("hg")]), Some("hg"));
        assert_eq!(home_mode(&[room("away"), room("manual")]), Some("schedule"));
        assert_eq!(home_mode(&[]), None);
    }

    #[test]
    fn a_setpoint_is_manual_for_a_while_and_within_bounds() {
        let room = Target::Room {
            home: "home-1".into(),
            room: "1003".into(),
        };
        let (path, form) = order(&room, "setpoint", &Value::Float(19.3), 180, 1_000, None).unwrap();
        assert_eq!(path, "/api/setroomthermpoint");
        assert_eq!(
            form,
            "home_id=home-1&room_id=1003&mode=manual&temp=19.5&endtime=11800"
        );
        for wrong in [6.5, 30.5, f64::NAN] {
            assert_eq!(
                order(&room, "setpoint", &Value::Float(wrong), 180, 0, None),
                Err(Wrong::OutOfBounds),
                "{wrong}"
            );
        }
        assert_eq!(
            order(
                &room,
                "setpoint",
                &Value::Text("chaud".into()),
                180,
                0,
                None
            ),
            Err(Wrong::OutOfBounds)
        );
        assert!(order(&room, "setpoint", &Value::Float(7.0), 180, 0, None).is_ok());
        assert!(order(&room, "setpoint", &Value::Float(30.0), 180, 0, None).is_ok());
    }

    #[test]
    fn modes_become_their_requests() {
        let room = Target::Room {
            home: "h".into(),
            room: "r".into(),
        };
        assert_eq!(
            order(
                &room,
                "setpoint_mode",
                &Value::Text("schedule".into()),
                180,
                0,
                None
            ),
            Ok((
                "/api/setroomthermpoint",
                "home_id=h&room_id=r&mode=home".into()
            ))
        );
        assert_eq!(
            order(
                &room,
                "setpoint_mode",
                &Value::Text("max".into()),
                180,
                0,
                None
            ),
            Err(Wrong::RoomMode)
        );
        assert_eq!(
            order(&room, "temperature", &Value::Float(20.0), 180, 0, None),
            Err(Wrong::ReadOnly("temperature".into()))
        );
        let home = Target::Home { home: "h".into() };
        assert_eq!(
            order(&home, "mode", &Value::Text("hg".into()), 180, 0, None),
            Ok(("/api/setthermmode", "home_id=h&mode=hg".into()))
        );
        assert!(order(&home, "mode", &Value::Text("off".into()), 180, 0, None).is_err());
        assert_eq!(status_path("a b"), "/api/homestatus?home_id=a%20b");
    }

    #[test]
    fn a_schedule_is_switched_by_its_name() {
        let homes = homes(&fixture(HOMESDATA));
        let known = &homes[0];
        let home = Target::Home {
            home: "home-1".into(),
        };
        assert_eq!(
            order(
                &home,
                "schedule",
                &Value::Text("Vacances".into()),
                180,
                0,
                Some(known)
            ),
            Ok((
                "/api/switchhomeschedule",
                "home_id=home-1&schedule_id=sched-2".into()
            ))
        );
        assert_eq!(
            order(
                &home,
                "schedule",
                &Value::Text("Été".into()),
                180,
                0,
                Some(known)
            ),
            Err(Wrong::NoSuchSchedule("Été".into()))
        );
        assert!(
            order(
                &home,
                "schedule",
                &Value::Text("Vacances".into()),
                180,
                0,
                None
            )
            .is_err()
        );
        let points = home_points(&known.schedule_names());
        assert_eq!(points[1].key.as_ref(), "schedule");
        assert!(points[1].access.write);
        assert!(!home_points(&[])[1].access.write);
    }
}
