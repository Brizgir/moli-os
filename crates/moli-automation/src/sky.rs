//! The « Soleil » device: where the sun stands, as points, for automations
//! and agents (Home Assistant's `sun.sun`). « Close the blinds when the sun
//! drops below 10° » is a threshold on `elevation`; « light the porch at
//! dusk », on `elevation` below -6°. Moli runs it by itself, as instance
//! [`INSTANCE`], whenever `[automations]` places the house: nothing to
//! declare.

use std::time::Duration;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use moli_core::{Access, Device, DeviceId, InstanceId, Kind, PointSpec, Semantic, Unit, Value};
use moli_runtime::{BoxFuture, Driver, DriverCtx};

use crate::sun::{self, Event};

/// The instance (and so the device's prefix: `soleil:maison/…`).
pub const INSTANCE: &str = "soleil";
const DEVICE: &str = "maison";
/// The sun moves a degree in about four minutes: a look a minute, in whole
/// degrees, is a change every few minutes (what the history keeps).
const EVERY: Duration = Duration::from_secs(60);

#[derive(Debug)]
pub struct Sky {
    latitude: f64,
    longitude: f64,
    tz: TimeZone,
}

impl Sky {
    /// `None` when the house is not placed (no latitude or longitude) or
    /// its time zone is unknown.
    #[must_use]
    pub fn new(config: &crate::Config) -> Option<Self> {
        Some(Self {
            latitude: config.latitude?,
            longitude: config.longitude?,
            tz: TimeZone::get(&config.timezone).ok()?,
        })
    }

    /// The points' values at `now`.
    fn readings(&self, now: Timestamp) -> Vec<(&'static str, Value)> {
        let (elevation, azimuth) = sun::position(now, self.latitude, self.longitude);
        let date = now.to_zoned(self.tz.clone()).date();
        let clock = |what| {
            sun::event(date, self.latitude, self.longitude, &self.tz, what).map_or(
                Value::Null,
                |at| {
                    let z = at.to_zoned(self.tz.clone());
                    Value::Text(format!("{:02}:{:02}", z.hour(), z.minute()).into())
                },
            )
        };
        // `+ 0.0`: a sun just under the horizon reads 0°, never « -0° ».
        vec![
            ("elevation", Value::Float(elevation.round() + 0.0)),
            ("azimuth", Value::Float(azimuth.round().rem_euclid(360.0))),
            (
                "up",
                Value::Bool(sun::is_up(now, self.latitude, self.longitude)),
            ),
            ("dawn", clock(Event::Dawn)),
            ("sunrise", clock(Event::Rise)),
            ("sunset", clock(Event::Set)),
            ("dusk", clock(Event::Dusk)),
        ]
    }
}

impl Driver for Sky {
    fn kind(&self) -> &'static str {
        "sun"
    }

    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let id = ctx.device_id(DEVICE);
            ctx.upsert_device(device(&id, ctx.instance()));
            ctx.set_availability(&id, true);
            ctx.ready();
            loop {
                for (key, value) in self.readings(Timestamp::now()) {
                    ctx.set_state(&id, key, value);
                }
                tokio::select! {
                    command = ctx.next_command() => match command {
                        // Read-only points: the hub never sends any.
                        Some(command) => {
                            command.reply(Err(moli_i18n::tr!("serveur.soleil.lecture_seule")));
                        }
                        None => return Ok(()),
                    },
                    () = tokio::time::sleep(EVERY) => {}
                }
            }
        })
    }
}

fn device(id: &DeviceId, instance: &InstanceId) -> Device {
    let degrees = |key: &str, label: String, min: f64, max: f64| PointSpec {
        key: key.into(),
        label: label.into(),
        kind: Kind::Numeric {
            min: Some(min),
            max: Some(max),
            step: Some(1.0),
        },
        access: READ_ONLY,
        unit: Some(Unit::Other("°".into())),
        semantic: Semantic::Other,
    };
    let clock = |key: &str, label: String| PointSpec {
        key: key.into(),
        label: label.into(),
        kind: Kind::Text,
        access: READ_ONLY,
        unit: None,
        semantic: Semantic::Other,
    };
    Device {
        id: id.clone(),
        instance: instance.clone(),
        native_name: moli_i18n::tr!("serveur.soleil.appareil").into(),
        manufacturer: Some("Moli".into()),
        model: Some(moli_i18n::tr!("serveur.soleil.modele").into()),
        description: None,
        native_room: None,
        members: Vec::new(),
        points: vec![
            degrees(
                "elevation",
                moli_i18n::tr!("serveur.soleil.point.elevation"),
                -90.0,
                90.0,
            ),
            degrees(
                "azimuth",
                moli_i18n::tr!("serveur.soleil.point.azimuth"),
                0.0,
                359.0,
            ),
            PointSpec {
                key: "up".into(),
                label: moli_i18n::tr!("serveur.soleil.point.up").into(),
                kind: Kind::Binary,
                access: READ_ONLY,
                unit: None,
                semantic: Semantic::Other,
            },
            clock("dawn", moli_i18n::tr!("serveur.soleil.point.dawn")),
            clock("sunrise", moli_i18n::tr!("serveur.soleil.point.sunrise")),
            clock("sunset", moli_i18n::tr!("serveur.soleil.point.sunset")),
            clock("dusk", moli_i18n::tr!("serveur.soleil.point.dusk")),
        ],
    }
}

const READ_ONLY: Access = Access {
    read: true,
    write: false,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn paris() -> Sky {
        Sky::new(&crate::Config {
            latitude: Some(48.85),
            longitude: Some(2.35),
            timezone: "Europe/Paris".into(),
        })
        .unwrap()
    }

    #[test]
    fn an_unplaced_house_has_no_sun() {
        assert!(Sky::new(&crate::Config::default()).is_none());
    }

    #[test]
    fn a_october_noon_in_paris() {
        let readings = paris().readings("2026-10-03T12:00:00Z".parse().unwrap());
        let get = |key: &str| readings.iter().find(|(k, _)| *k == key).unwrap().1.clone();
        assert_eq!(get("elevation"), Value::Float(37.0));
        assert_eq!(get("azimuth"), Value::Float(186.0));
        assert_eq!(get("up"), Value::Bool(true));
        assert_eq!(get("dawn"), Value::Text("07:21".into()));
        assert_eq!(get("sunrise"), Value::Text("07:53".into()));
        assert_eq!(get("sunset"), Value::Text("19:25".into()));
        assert_eq!(get("dusk"), Value::Text("19:57".into()));
    }

    #[test]
    fn the_points_are_declared() {
        let sky = paris();
        let id = DeviceId::from("soleil:maison");
        let device = device(&id, &InstanceId::from(INSTANCE));
        for (key, _) in sky.readings(Timestamp::now()) {
            assert!(device.points.iter().any(|p| &*p.key == key), "{key}");
        }
    }
}
