//! The sun over the house, computed here: the house knows the sun without
//! asking the internet.
//!
//! A port of astral (Apache-2.0, <https://github.com/sffjunkie/astral>),
//! the library behind Home Assistant's `sun`: NOAA's solar calculator
//! (Julian centuries, equation of time, declination), atmospheric
//! refraction, and the sun's apparent radius, so that sunrise is when its
//! upper edge clears the horizon. Within seconds of the almanac where the
//! short NOAA formula it replaces drifted by minutes (ten near the polar
//! circle). Two things astral does not do are done here: a day always gets
//! its own events, even 14 hours away from UTC, and the noon of a far-east
//! house lands on the right day.

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};

/// Half the sun's apparent diameter (32 arc minutes), in degrees: below
/// this (refraction included) the sun has set.
const APPARENT_RADIUS: f64 = 32.0 / (60.0 * 2.0);
/// Civil twilight: the sun 6° below the horizon (Home Assistant's dawn and
/// dusk).
const CIVIL: f64 = 6.0;

/// A moment of the sun's day.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// Civil dawn: enough light outside to see without lamps.
    Dawn,
    Rise,
    /// Highest in the sky.
    Noon,
    Set,
    /// Civil dusk: lamps needed outside.
    Dusk,
}

/// Sunrise and sunset of a (local) day, `None` during polar days and nights.
pub fn times(
    date: Date,
    latitude: f64,
    longitude: f64,
    tz: &TimeZone,
) -> Option<(Timestamp, Timestamp)> {
    Some((
        event(date, latitude, longitude, tz, Event::Rise)?,
        event(date, latitude, longitude, tz, Event::Set)?,
    ))
}

/// When `what` happens on that (local) day; `None` when it does not (the
/// sun never sets in a polar day, never sinks 6° on a June night up north).
pub fn event(
    date: Date,
    latitude: f64,
    longitude: f64,
    tz: &TimeZone,
    what: Event,
) -> Option<Timestamp> {
    let on = |day: Date| match what {
        Event::Noon => Some(noon(day, longitude)),
        Event::Dawn => transit(day, latitude, longitude, 90.0 + CIVIL, true),
        Event::Rise => transit(day, latitude, longitude, 90.0 + APPARENT_RADIUS, true),
        Event::Set => transit(day, latitude, longitude, 90.0 + APPARENT_RADIUS, false),
        Event::Dusk => transit(day, latitude, longitude, 90.0 + CIVIL, false),
    };
    // Computed for the UTC day of that name: far from UTC, the house's day
    // may hold the event of the UTC day before or after (astral's search).
    let local = |at: Timestamp| at.to_zoned(tz.clone()).date();
    let first = on(date)?;
    if local(first) == date {
        return Some(first);
    }
    let other = if local(first) < date {
        date.tomorrow().ok()?
    } else {
        date.yesterday().ok()?
    };
    on(other).filter(|at| local(*at) == date)
}

/// Where the sun stands at `at`: elevation above the horizon (refraction
/// included, as the eye sees it) and azimuth (0° north, 90° east), in
/// degrees.
#[allow(clippy::similar_names)]
pub fn position(at: Timestamp, latitude: f64, longitude: f64) -> (f64, f64) {
    let latitude = latitude.clamp(-89.8, 89.8);
    #[allow(clippy::cast_precision_loss)]
    let day_fraction = at.as_millisecond().rem_euclid(86_400_000) as f64 / 86_400_000.0;
    let date = at.to_zoned(TimeZone::UTC).date();
    let jc = julian_century(julian_day(date) + day_fraction);
    let declination = declination(jc);
    // True solar time, in minutes: 1440 a turn.
    let solar = (day_fraction * 1440.0 + eq_of_time(jc) + 4.0 * longitude).rem_euclid(1440.0);
    let hour_angle = solar / 4.0 - 180.0;
    let (sl, cl) = latitude.to_radians().sin_cos();
    let (sd, cd) = declination.to_radians().sin_cos();
    let cos_zenith = (cl * cd * hour_angle.to_radians().cos() + sl * sd).clamp(-1.0, 1.0);
    let zenith = cos_zenith.acos().to_degrees();
    let denominator = cl * zenith.to_radians().sin();
    let mut azimuth = if denominator.abs() > 0.001 {
        let a = ((sl * zenith.to_radians().cos() - sd) / denominator).clamp(-1.0, 1.0);
        let a = 180.0 - a.acos().to_degrees();
        if hour_angle > 0.0 { -a } else { a }
    } else if latitude > 0.0 {
        180.0
    } else {
        0.0
    };
    if azimuth < 0.0 {
        azimuth += 360.0;
    }
    (90.0 - (zenith - refraction(zenith)), azimuth)
}

/// The sun is up: its upper edge above the horizon (true all day long in a
/// polar day, never in a polar night).
pub fn is_up(at: Timestamp, latitude: f64, longitude: f64) -> bool {
    position(at, latitude, longitude).0 > -APPARENT_RADIUS
}

// ---- NOAA's solar calculator, as astral writes it ----------------------------

/// The Julian day at 0:00 UTC of a (Gregorian) date.
fn julian_day(date: Date) -> f64 {
    let (mut year, mut month) = (i32::from(date.year()), i32::from(date.month()));
    if month <= 2 {
        year -= 1;
        month += 12;
    }
    let a = year / 100;
    let b = 2 - a + a / 4;
    // Integer parts, as in the original (`int(365.25 * …)`): both are
    // positive for any date jiff holds.
    let days = (1461 * (year + 4716)) / 4 + (306_001 * (month + 1)) / 10_000;
    f64::from(days + i32::from(date.day()) + b) - 1524.5
}

fn julian_century(julian_day: f64) -> f64 {
    (julian_day - 2_451_545.0) / 36_525.0
}

fn mean_longitude(jc: f64) -> f64 {
    (280.466_46 + jc * (36_000.769_83 + 0.000_303_2 * jc)).rem_euclid(360.0)
}

fn mean_anomaly(jc: f64) -> f64 {
    357.529_11 + jc * (35_999.050_29 - 0.000_153_7 * jc)
}

fn eccentricity(jc: f64) -> f64 {
    0.016_708_634 - jc * (0.000_042_037 + 0.000_000_126_7 * jc)
}

fn equation_of_center(jc: f64) -> f64 {
    let m = mean_anomaly(jc).to_radians();
    m.sin() * (1.914_602 - jc * (0.004_817 + 0.000_014 * jc))
        + (2.0 * m).sin() * (0.019_993 - 0.000_101 * jc)
        + (3.0 * m).sin() * 0.000_289
}

fn apparent_longitude(jc: f64) -> f64 {
    let omega = 125.04 - 1934.136 * jc;
    mean_longitude(jc) + equation_of_center(jc) - 0.005_69 - 0.004_78 * omega.to_radians().sin()
}

fn obliquity(jc: f64) -> f64 {
    let seconds = 21.448 - jc * (46.815 + jc * (0.000_59 - jc * 0.001_813));
    let mean = 23.0 + (26.0 + seconds / 60.0) / 60.0;
    mean + 0.002_56 * (125.04 - 1934.136 * jc).to_radians().cos()
}

fn declination(jc: f64) -> f64 {
    (obliquity(jc).to_radians().sin() * apparent_longitude(jc).to_radians().sin())
        .asin()
        .to_degrees()
}

/// Sundial time minus clock time, in minutes.
fn eq_of_time(jc: f64) -> f64 {
    let l0 = mean_longitude(jc).to_radians();
    let e = eccentricity(jc);
    let m = mean_anomaly(jc).to_radians();
    let y = (obliquity(jc).to_radians() / 2.0).tan().powi(2);
    let etime = y * (2.0 * l0).sin() - 2.0 * e * m.sin() + 4.0 * e * y * m.sin() * (2.0 * l0).cos()
        - 0.5 * y * y * (4.0 * l0).sin()
        - 1.25 * e * e * (2.0 * m).sin();
    etime.to_degrees() * 4.0
}

/// How much the air lifts the sun's image, in degrees, at that zenith angle.
fn refraction(zenith: f64) -> f64 {
    let elevation = 90.0 - zenith;
    if elevation >= 85.0 {
        return 0.0;
    }
    let te = elevation.to_radians().tan();
    let arcseconds = if elevation > 5.0 {
        58.1 / te - 0.07 / te.powi(3) + 0.000_086 / te.powi(5)
    } else if elevation > -0.575 {
        1735.0
            + elevation * (-518.2 + elevation * (103.4 + elevation * (-12.79 + elevation * 0.711)))
    } else {
        -20.774 / te
    };
    arcseconds / 3600.0
}

/// Midnight UTC of `date`, plus some minutes.
fn utc_minutes(date: Date, minutes: f64) -> Timestamp {
    #[allow(clippy::cast_possible_truncation)]
    let ms = (minutes * 60_000.0).round() as i64;
    let midnight = date
        .to_zoned(TimeZone::UTC)
        .map_or(Timestamp::UNIX_EPOCH, |z| z.timestamp());
    midnight
        .checked_add(SignedDuration::from_millis(ms))
        .unwrap_or(midnight)
}

/// When the sun crosses that zenith angle (seen through the air) on the
/// UTC day `date`, rising or setting; `None` if it never does.
fn transit(
    date: Date,
    latitude: f64,
    longitude: f64,
    zenith: f64,
    rising: bool,
) -> Option<Timestamp> {
    let latitude = latitude.clamp(-89.8, 89.8).to_radians();
    let zenith = zenith + refraction(zenith);
    let jd = julian_day(date);
    let mut minutes = 0.0;
    // Twice: the second pass uses the sun of the moment found by the first.
    for _ in 0..2 {
        let jc = julian_century(jd + minutes / 1440.0);
        let declination = declination(jc).to_radians();
        let h = (zenith.to_radians().cos() - latitude.sin() * declination.sin())
            / (latitude.cos() * declination.cos());
        if !(-1.0..=1.0).contains(&h) {
            return None;
        }
        let hour_angle = if rising { h.acos() } else { -h.acos() };
        let mut offset = (-longitude - hour_angle.to_degrees()) * 4.0 - eq_of_time(jc);
        if offset < -720.0 {
            offset += 1440.0;
        }
        minutes = 720.0 + offset;
    }
    Some(utc_minutes(date, minutes))
}

/// Solar noon on the UTC day `date`.
fn noon(date: Date, longitude: f64) -> Timestamp {
    let jc = julian_century(julian_day(date));
    utc_minutes(date, 720.0 - 4.0 * longitude - eq_of_time(jc))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    /// Within `tolerance` seconds of the expected instant.
    #[track_caller]
    fn near(got: Option<Timestamp>, expected: &str, tolerance: i64) {
        let got = got.unwrap_or_else(|| panic!("no time, expected {expected}"));
        let gap = (got.as_second() - utc(expected).as_second()).abs();
        assert!(gap <= tolerance, "{got} instead of {expected}");
    }

    // Expected values: astral 3.2 (Home Assistant's), same places and days.
    // Ours land within two seconds of it.

    #[test]
    fn paris_matches_astral() {
        let paris = TimeZone::get("Europe/Paris").unwrap();
        let day = Date::new(2026, 10, 3).unwrap();
        let at = |what| event(day, 48.85, 2.35, &paris, what);
        near(at(Event::Dawn), "2026-10-03T05:21:04Z", 2);
        near(at(Event::Rise), "2026-10-03T05:53:09Z", 2);
        near(at(Event::Set), "2026-10-03T17:25:15Z", 2);
        near(at(Event::Dusk), "2026-10-03T17:57:16Z", 2);
        near(at(Event::Noon), "2026-10-03T11:39:47Z", 2);
        let summer = Date::new(2026, 6, 21).unwrap();
        let (rise, set) = times(summer, 48.85, 2.35, &paris).unwrap();
        near(Some(rise), "2026-06-21T03:47:18Z", 2);
        near(Some(set), "2026-06-21T19:57:31Z", 2);
    }

    #[test]
    fn far_from_greenwich_the_day_is_the_houses() {
        // Auckland (UTC+13 in January): its sunrise is the evening before in UTC.
        let auckland = TimeZone::get("Pacific/Auckland").unwrap();
        let (rise, set) =
            times(Date::new(2026, 1, 15).unwrap(), -36.85, 174.76, &auckland).unwrap();
        near(Some(rise), "2026-01-14T17:18:01Z", 2);
        near(Some(set), "2026-01-15T07:41:58Z", 2);
        // Kiritimati (UTC+14): the old formula gave the next day's sunrise;
        // astral itself puts its noon on the wrong day.
        let kiritimati = TimeZone::get("Pacific/Kiritimati").unwrap();
        let day = Date::new(2026, 3, 1).unwrap();
        let at = |what| event(day, 1.87, -157.4, &kiritimati, what);
        near(at(Event::Rise), "2026-02-28T16:39:54Z", 2);
        near(at(Event::Set), "2026-03-01T04:44:09Z", 2);
        let noon = at(Event::Noon).unwrap().to_zoned(kiritimati.clone());
        assert_eq!(noon.date(), day);
        assert_eq!(noon.hour(), 12);
    }

    #[test]
    fn near_the_polar_circle_it_stays_exact() {
        // Tromsø in May: the old formula set the sun ten minutes early, and
        // the night never gets dark enough for a dusk.
        let oslo = TimeZone::get("Europe/Oslo").unwrap();
        let day = Date::new(2026, 5, 10).unwrap();
        let (rise, set) = times(day, 69.65, 18.96, &oslo).unwrap();
        near(Some(rise), "2026-05-10T00:26:17Z", 3);
        near(Some(set), "2026-05-10T21:00:59Z", 3);
        assert_eq!(event(day, 69.65, 18.96, &oslo, Event::Dusk), None);
    }

    #[test]
    fn polar_night_has_no_sun_and_polar_day_no_night() {
        let utc_tz = TimeZone::UTC;
        assert!(times(Date::new(2026, 12, 21).unwrap(), 80.0, 0.0, &utc_tz).is_none());
        assert!(!is_up(utc("2026-12-21T12:00:00Z"), 80.0, 0.0));
        // Midsummer at 80° N: the sun is up at midnight.
        assert!(times(Date::new(2026, 6, 21).unwrap(), 80.0, 0.0, &utc_tz).is_none());
        assert!(is_up(utc("2026-06-21T00:00:00Z"), 80.0, 0.0));
    }

    #[test]
    fn position_matches_astral() {
        let close = |at: &str, elevation: f64, azimuth: f64| {
            let (e, a) = position(utc(at), 48.85, 2.35);
            assert!((e - elevation).abs() < 0.02, "{at}: elevation {e}");
            assert!((a - azimuth).abs() < 0.02, "{at}: azimuth {a}");
        };
        close("2026-10-03T12:00:00Z", 36.9083, 186.3547);
        close("2026-10-03T20:30:00Z", -29.97, 302.0874);
        close("2026-06-21T04:00:00Z", 1.2497, 54.0499);
    }

    #[test]
    fn up_switches_at_sunrise_and_sunset() {
        let paris = TimeZone::get("Europe/Paris").unwrap();
        let (rise, set) = times(Date::new(2026, 10, 3).unwrap(), 48.85, 2.35, &paris).unwrap();
        let minute = SignedDuration::from_secs(60);
        assert!(!is_up(rise.checked_sub(minute).unwrap(), 48.85, 2.35));
        assert!(is_up(rise.checked_add(minute).unwrap(), 48.85, 2.35));
        assert!(is_up(set.checked_sub(minute).unwrap(), 48.85, 2.35));
        assert!(!is_up(set.checked_add(minute).unwrap(), 48.85, 2.35));
    }
}
