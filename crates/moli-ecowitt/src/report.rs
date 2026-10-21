//! What the gateway posts, and what it means.
//!
//! The Ecowitt protocol (« Customized » upload) is a form
//! (`application/x-www-form-urlencoded`) in imperial units: `tempf` in °F,
//! `baromrelin` in inHg, `windspeedmph`, `dailyrain` in inches. Field names
//! and battery codes as Ecowitt documents them and as aioecowitt (Home
//! Assistant's Ecowitt library) reads them. Everything here is pure.

use std::collections::BTreeMap;

use moli_core::{Semantic, Unit, Value};

/// Longest form accepted: one gateway with every sensor Ecowitt sells posts
/// about 2 KiB.
pub const MAX_PAIRS: usize = 256;
const MAX_NAME: usize = 64;
const MAX_VALUE: usize = 128;

/// The form's fields, by name (the last one wins).
pub type Form = BTreeMap<String, String>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormError {
    TooMany,
    TooLong,
    NotText,
}

/// `a=1&b=x%20y` → `{a: 1, b: "x y"}`. Bounded: never more than
/// [`MAX_PAIRS`] fields, nor a name or value longer than a reading can be.
pub fn parse_form(body: &[u8]) -> Result<Form, FormError> {
    let mut form = Form::new();
    for (count, pair) in body
        .split(|b| *b == b'&')
        .filter(|p| !p.is_empty())
        .enumerate()
    {
        if count >= MAX_PAIRS {
            return Err(FormError::TooMany);
        }
        let (name, value) = match pair.iter().position(|b| *b == b'=') {
            Some(i) => (&pair[..i], &pair[i + 1..]),
            None => (pair, &[][..]),
        };
        let (name, value) = (decode(name)?, decode(value)?);
        if name.len() > MAX_NAME || value.len() > MAX_VALUE {
            return Err(FormError::TooLong);
        }
        form.insert(name, value);
    }
    Ok(form)
}

/// `%XX` and `+` decoded; the result must be UTF-8.
fn decode(raw: &[u8]) -> Result<String, FormError> {
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        match raw[i] {
            b'+' => out.push(b' '),
            b'%' => {
                let hex = raw
                    .get(i + 1..i + 3)
                    .and_then(|h| std::str::from_utf8(h).ok());
                match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(b) => {
                        out.push(b);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8(out).map_err(|_| FormError::NotText)
}

/// Which device of the station a reading belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Part {
    /// The outdoor array (thermo-hygrometer, anemometer, rain gauge,
    /// light and UV), and a separate rain gauge.
    Outdoor,
    /// The gateway or console indoors (and its pressure sensor).
    Indoor,
    /// An extra thermo-hygrometer on channel 1 to 8 (WH31, WN31).
    Channel(u8),
    /// The lightning sensor (WH57).
    Lightning,
    /// The air quality monitor (WH45, WH46: CO₂ and particles).
    Air,
    /// The black globe thermometer (WN38).
    Globe,
    /// A soil moisture probe, 1 to 16 (WH51).
    Soil(u8),
    /// A probe thermometer (soil, water), 1 to 8 (WN34).
    Probe(u8),
    /// A leak detector, 1 to 4 (WH55).
    Leak(u8),
    /// A particle sensor, 1 to 4 (WH41, WH43).
    Pm25(u8),
    /// A leaf wetness sensor, 1 to 8 (WN35).
    Leaf(u8),
}

impl Part {
    /// The device's native id inside the instance (stable: never a name).
    pub fn native(self) -> String {
        match self {
            Self::Outdoor => "outdoor".into(),
            Self::Indoor => "indoor".into(),
            Self::Channel(n) => format!("ch{n}"),
            Self::Lightning => "lightning".into(),
            Self::Air => "air".into(),
            Self::Globe => "globe".into(),
            Self::Soil(n) => format!("soil{n}"),
            Self::Probe(n) => format!("probe{n}"),
            Self::Leak(n) => format!("leak{n}"),
            Self::Pm25(n) => format!("pm25_{n}"),
            Self::Leaf(n) => format!("leaf{n}"),
        }
    }
}

/// How a raw field becomes a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Conv {
    /// °F → °C, one decimal.
    Fahrenheit,
    /// inHg → hPa, one decimal.
    InHg,
    /// mph → km/h, one decimal.
    Mph,
    /// inches (or inches an hour) → mm (mm/h), one decimal.
    Inch,
    /// As sent, rounded to that many decimals.
    Same(u8),
    /// A flag: 0 no, 1 yes (battery low, raining, a leak).
    Flag,
    /// A battery level 0 to 5 (6: on mains) → %.
    Level,
    /// Seconds since 1970 → the instant, as text (`2024-02-08T09:00:00Z`).
    Instant,
}

/// One point a station may have: its key, unit, meaning, and whether it is
/// an on/off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point {
    pub key: &'static str,
    pub unit: Option<&'static str>,
    pub semantic: Semantic,
    pub binary: bool,
    /// Text rather than a number (an instant).
    pub text: bool,
}

const fn num(key: &'static str, unit: &'static str, semantic: Semantic) -> Point {
    Point {
        key,
        unit: Some(unit),
        semantic,
        binary: false,
        text: false,
    }
}

const fn flag(key: &'static str, semantic: Semantic) -> Point {
    Point {
        key,
        unit: None,
        semantic,
        binary: true,
        text: false,
    }
}

/// A number without a unit (a count).
const fn count(key: &'static str) -> Point {
    Point {
        key,
        unit: None,
        semantic: Semantic::Other,
        binary: false,
        text: false,
    }
}

/// Every point, in the order the dashboard shows them. Keys follow the
/// `open-meteo` profile's where they mean the same (`temperature`,
/// `humidity`, `wind_speed`, `wind_gusts`): a measure and a forecast read
/// alike.
pub const POINTS: &[Point] = &[
    num("temperature", "°C", Semantic::Temperature),
    num("humidity", "%", Semantic::Humidity),
    num("dew_point", "°C", Semantic::Temperature),
    num("pressure", "hPa", Semantic::Pressure),
    num("pressure_abs", "hPa", Semantic::Pressure),
    num("wind_speed", "km/h", Semantic::Other),
    num("wind_gusts", "km/h", Semantic::Other),
    num("wind_gusts_max", "km/h", Semantic::Other),
    num("wind_direction", "°", Semantic::Other),
    num("wind_direction_avg", "°", Semantic::Other),
    flag("raining", Semantic::Other),
    num("rain_rate", "mm/h", Semantic::Other),
    num("rain_today", "mm", Semantic::Other),
    num("rain_event", "mm", Semantic::Other),
    num("rain_hour", "mm", Semantic::Other),
    num("rain_24h", "mm", Semantic::Other),
    num("rain_week", "mm", Semantic::Other),
    num("rain_month", "mm", Semantic::Other),
    num("rain_year", "mm", Semantic::Other),
    num("rain_total", "mm", Semantic::Other),
    num("solar_radiation", "W/m²", Semantic::Other),
    num("illuminance", "lx", Semantic::Illuminance),
    num("uv_index", "UV", Semantic::Other),
    flag("battery_low", Semantic::BatteryLow),
    num("battery_voltage", "V", Semantic::Voltage),
    num("capacitor_voltage", "V", Semantic::Voltage),
    num("rain_battery_voltage", "V", Semantic::Voltage),
    num("vpd", "hPa", Semantic::Other),
    num("black_globe", "°C", Semantic::Other),
    num("wet_bulb_globe", "°C", Semantic::Other),
    num("lightning_distance", "km", Semantic::Other),
    count("lightning_count"),
    Point {
        key: "lightning_last",
        unit: None,
        semantic: Semantic::Other,
        binary: false,
        text: true,
    },
    num("co2", "ppm", Semantic::Other),
    num("co2_24h", "ppm", Semantic::Other),
    num("pm1", "µg/m³", Semantic::Other),
    num("pm1_24h", "µg/m³", Semantic::Other),
    num("pm25", "µg/m³", Semantic::Other),
    num("pm25_24h", "µg/m³", Semantic::Other),
    num("pm4", "µg/m³", Semantic::Other),
    num("pm4_24h", "µg/m³", Semantic::Other),
    num("pm10", "µg/m³", Semantic::Other),
    num("pm10_24h", "µg/m³", Semantic::Other),
    num("soil_moisture", "%", Semantic::Other),
    num("leaf_wetness", "%", Semantic::Other),
    flag("leak", Semantic::WaterLeak),
    num("battery", "%", Semantic::Battery),
];

/// The point of a key.
pub fn point(key: &str) -> Option<&'static Point> {
    POINTS.iter().find(|p| p.key == key)
}

/// The unit of a point, for the hub.
pub fn unit(point: &Point) -> Option<Unit> {
    point.unit.and_then(Unit::parse)
}

/// Fields of the station itself, read for its description, never points.
const META: &[&str] = &[
    "PASSKEY",
    "ws90_ver",
    "stationtype",
    "dateutc",
    "freq",
    "model",
    "runtime",
    "heap",
    "interval",
];

/// Fixed fields: name on the wire, device, key, conversion.
const FIELDS: &[(&str, Part, &str, Conv)] = &[
    // The gateway (or the WH25/WN32P indoor sensor).
    ("tempinf", Part::Indoor, "temperature", Conv::Fahrenheit),
    ("humidityin", Part::Indoor, "humidity", Conv::Same(0)),
    ("baromrelin", Part::Indoor, "pressure", Conv::InHg),
    ("baromabsin", Part::Indoor, "pressure_abs", Conv::InHg),
    ("wh25batt", Part::Indoor, "battery_low", Conv::Flag),
    // The outdoor array.
    ("tempf", Part::Outdoor, "temperature", Conv::Fahrenheit),
    ("humidity", Part::Outdoor, "humidity", Conv::Same(0)),
    ("winddir", Part::Outdoor, "wind_direction", Conv::Same(0)),
    (
        "winddir_avg10m",
        Part::Outdoor,
        "wind_direction_avg",
        Conv::Same(0),
    ),
    ("windspeedmph", Part::Outdoor, "wind_speed", Conv::Mph),
    ("windgustmph", Part::Outdoor, "wind_gusts", Conv::Mph),
    ("maxdailygust", Part::Outdoor, "wind_gusts_max", Conv::Mph),
    (
        "solarradiation",
        Part::Outdoor,
        "solar_radiation",
        Conv::Same(1),
    ),
    ("uv", Part::Outdoor, "uv_index", Conv::Same(0)),
    // Batteries: a flag on the 7-in-1 (WH65, WS69), a voltage on the
    // others (WH80, WH68, WS80, WS90, WS85).
    ("wh65batt", Part::Outdoor, "battery_low", Conv::Flag),
    ("wh80batt", Part::Outdoor, "battery_voltage", Conv::Same(2)),
    ("wh68batt", Part::Outdoor, "battery_voltage", Conv::Same(2)),
    ("wh90batt", Part::Outdoor, "battery_voltage", Conv::Same(2)),
    ("wh85batt", Part::Outdoor, "battery_voltage", Conv::Same(2)),
    (
        "ws90cap_volt",
        Part::Outdoor,
        "capacitor_voltage",
        Conv::Same(1),
    ),
    (
        "ws85cap_volt",
        Part::Outdoor,
        "capacitor_voltage",
        Conv::Same(1),
    ),
    (
        "wh40batt",
        Part::Outdoor,
        "rain_battery_voltage",
        Conv::Same(2),
    ),
    // The WS90's piezo says whether it rains now (0 dry, 1 rain).
    ("srain_piezo", Part::Outdoor, "raining", Conv::Flag),
    ("vpd", Part::Outdoor, "vpd", Conv::InHg),
    // The console's own CO₂ sensor (HP2560, GW3000).
    ("co2in", Part::Indoor, "co2", Conv::Same(0)),
    ("co2in_24h", Part::Indoor, "co2_24h", Conv::Same(0)),
    // The lightning sensor: distance in km already, empty before a strike.
    (
        "lightning",
        Part::Lightning,
        "lightning_distance",
        Conv::Same(0),
    ),
    (
        "lightning_num",
        Part::Lightning,
        "lightning_count",
        Conv::Same(0),
    ),
    (
        "lightning_time",
        Part::Lightning,
        "lightning_last",
        Conv::Instant,
    ),
    ("wh57batt", Part::Lightning, "battery", Conv::Level),
    // The air quality monitor.
    ("tf_co2", Part::Air, "temperature", Conv::Fahrenheit),
    ("humi_co2", Part::Air, "humidity", Conv::Same(0)),
    ("co2", Part::Air, "co2", Conv::Same(0)),
    ("co2_24h", Part::Air, "co2_24h", Conv::Same(0)),
    ("pm1_co2", Part::Air, "pm1", Conv::Same(1)),
    ("pm1_24h_co2", Part::Air, "pm1_24h", Conv::Same(1)),
    ("pm25_co2", Part::Air, "pm25", Conv::Same(1)),
    ("pm25_24h_co2", Part::Air, "pm25_24h", Conv::Same(1)),
    ("pm4_co2", Part::Air, "pm4", Conv::Same(1)),
    ("pm4_24h_co2", Part::Air, "pm4_24h", Conv::Same(1)),
    ("pm10_co2", Part::Air, "pm10", Conv::Same(1)),
    ("pm10_24h_co2", Part::Air, "pm10_24h", Conv::Same(1)),
    ("co2_batt", Part::Air, "battery", Conv::Level),
    // The black globe thermometer (heat stress).
    ("bgt", Part::Globe, "black_globe", Conv::Fahrenheit),
    ("wbgt", Part::Globe, "wet_bulb_globe", Conv::Fahrenheit),
    ("bgtbatt", Part::Globe, "battery_voltage", Conv::Same(2)),
];

/// A numbered sensor's field: `<prefix><n>`, `n` from 1 to the highest;
/// device, key, conversion.
type Numbered = (&'static str, u8, fn(u8) -> Part, &'static str, Conv);

const NUMBERED: &[Numbered] = &[
    (
        "soilmoisture",
        16,
        Part::Soil,
        "soil_moisture",
        Conv::Same(0),
    ),
    ("soilbatt", 16, Part::Soil, "battery_voltage", Conv::Same(1)),
    ("tf_ch", 8, Part::Probe, "temperature", Conv::Fahrenheit),
    ("tf_batt", 8, Part::Probe, "battery_voltage", Conv::Same(2)),
    ("leak_ch", 4, Part::Leak, "leak", Conv::Flag),
    ("leakbatt", 4, Part::Leak, "battery", Conv::Level),
    ("pm25_ch", 4, Part::Pm25, "pm25", Conv::Same(1)),
    ("pm25_avg_24h_ch", 4, Part::Pm25, "pm25_24h", Conv::Same(1)),
    ("pm25batt", 4, Part::Pm25, "battery", Conv::Level),
    (
        "leafwetness_ch",
        8,
        Part::Leaf,
        "leaf_wetness",
        Conv::Same(0),
    ),
    ("leaf_batt", 8, Part::Leaf, "battery_voltage", Conv::Same(2)),
];

/// Rain, from a tipping bucket (WH65, WS69, WH40) or a piezo (WS90, WS85:
/// `*_piezo`). With both, the bucket wins (it is the reference; the piezo
/// stays in the form, unread).
const RAIN: &[(&str, &str, &str)] = &[
    ("rainratein", "rrain_piezo", "rain_rate"),
    ("dailyrainin", "drain_piezo", "rain_today"),
    ("eventrainin", "erain_piezo", "rain_event"),
    ("hourlyrainin", "hrain_piezo", "rain_hour"),
    ("last24hrainin", "last24hrain_piezo", "rain_24h"),
    ("weeklyrainin", "wrain_piezo", "rain_week"),
    ("monthlyrainin", "mrain_piezo", "rain_month"),
    ("yearlyrainin", "yrain_piezo", "rain_year"),
    ("totalrainin", "train_piezo", "rain_total"),
];

/// One reading: a device of the station, a key of [`POINTS`], a value
/// (`Null` when the field says nothing usable).
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub part: Part,
    pub key: &'static str,
    pub value: Value,
}

/// What the gateway says about itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Station {
    /// `PASSKEY`: the MD5 of the gateway's MAC address, in capitals.
    /// Identifies the gateway; not a secret (any listener on the network
    /// sees it).
    pub passkey: Option<String>,
    /// `model`, `GW2000A`.
    pub model: Option<String>,
    /// `stationtype`: the firmware, `GW2000A_V3.1.4`.
    pub firmware: Option<String>,
    /// `interval`: seconds between two posts, when the gateway says it.
    pub interval: Option<u64>,
}

pub fn station(form: &Form) -> Station {
    let text = |k: &str| {
        form.get(k)
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    };
    Station {
        passkey: text("PASSKEY").map(|p| p.to_ascii_uppercase()),
        model: text("model"),
        firmware: text("stationtype"),
        interval: text("interval").and_then(|i| i.parse().ok()),
    }
}

/// A number the gateway sent, if it is one.
fn number(raw: &str) -> Option<f64> {
    raw.trim().parse::<f64>().ok().filter(|n| n.is_finite())
}

fn round(n: f64, decimals: u8) -> f64 {
    let f = 10f64.powi(i32::from(decimals));
    (n * f).round() / f
}

fn convert(raw: &str, conv: Conv) -> Value {
    let Some(n) = number(raw) else {
        return Value::Null;
    };
    match conv {
        Conv::Fahrenheit => Value::Float(round((n - 32.0) * 5.0 / 9.0, 1)),
        Conv::InHg => Value::Float(round(n * 33.863_886, 1)),
        Conv::Mph => Value::Float(round(n * 1.609_344, 1)),
        Conv::Inch => Value::Float(round(n * 25.4, 1)),
        Conv::Same(d) => Value::Float(round(n, d)),
        Conv::Flag => Value::Bool(n >= 1.0),
        Conv::Level => Value::Float((n.clamp(0.0, 5.0) * 20.0).round()),
        Conv::Instant => {
            // Whole seconds since 1970: exact in an f64 for any real date.
            #[allow(clippy::cast_possible_truncation)]
            let seconds = n.round() as i64;
            if seconds <= 0 {
                return Value::Null;
            }
            jiff::Timestamp::from_second(seconds)
                .map_or(Value::Null, |t| Value::Text(t.to_string().into()))
        }
    }
}

/// Dew point (°C) from temperature (°C) and relative humidity (%), by the
/// Magnus formula (Sonntag 1990 constants), as Ecowitt's own apps show it.
fn dew_point(t: f64, rh: f64) -> Option<f64> {
    if !(1.0..=100.0).contains(&rh) {
        return None;
    }
    let (a, b) = (17.62, 243.12);
    let g = (rh / 100.0).ln() + a * t / (b + t);
    Some(round(b * g / (a - g), 1))
}

/// Solar radiation (W/m²) as lux: Ecowitt's own factor (126.7 lx per
/// W/m², the « 1/0.0079 » aioecowitt uses). An estimate: a sensor of light
/// and one of power do not see the same colours.
fn lux(w: f64) -> f64 {
    (w / 0.0079).round()
}

/// Every reading in the form. Fields the station does not have are simply
/// absent (a reading is never invented); a field present but unreadable
/// reads `Null` (no measure, never a stale one).
pub fn readings(form: &Form) -> Vec<Reading> {
    let mut out = Vec::new();
    let mut push = |part, key, value| out.push(Reading { part, key, value });
    for (wire, part, key, conv) in FIELDS {
        if let Some(raw) = form.get(*wire) {
            push(*part, *key, convert(raw, *conv));
        }
    }
    for (bucket, piezo, key) in RAIN {
        if let Some(raw) = form.get(*bucket).or_else(|| form.get(*piezo)) {
            push(Part::Outdoor, *key, convert(raw, Conv::Inch));
        }
    }
    for n in 1..=8u8 {
        let part = Part::Channel(n);
        if let Some(raw) = form.get(&format!("temp{n}f")) {
            push(part, "temperature", convert(raw, Conv::Fahrenheit));
        }
        if let Some(raw) = form.get(&format!("humidity{n}")) {
            push(part, "humidity", convert(raw, Conv::Same(0)));
        }
        if let Some(raw) = form.get(&format!("batt{n}")) {
            push(part, "battery_low", convert(raw, Conv::Flag));
        }
    }
    for (prefix, max, part, key, conv) in NUMBERED {
        for n in 1..=*max {
            if let Some(raw) = form.get(&format!("{prefix}{n}")) {
                push(part(n), *key, convert(raw, *conv));
            }
        }
    }
    // Derived, like Ecowitt's apps and aioecowitt: dew point and light.
    let get = |part: Part, key: &str| {
        out.iter()
            .find(|r| r.part == part && r.key == key)
            .and_then(|r| r.value.as_f64())
    };
    let dew = get(Part::Outdoor, "temperature")
        .zip(get(Part::Outdoor, "humidity"))
        .and_then(|(t, rh)| dew_point(t, rh));
    let light = get(Part::Outdoor, "solar_radiation").map(lux);
    let outdoor = form.contains_key("tempf") && form.contains_key("humidity");
    if outdoor {
        out.push(Reading {
            part: Part::Outdoor,
            key: "dew_point",
            value: dew.map_or(Value::Null, Value::Float),
        });
    }
    if form.contains_key("solarradiation") {
        out.push(Reading {
            part: Part::Outdoor,
            key: "illuminance",
            value: light.map_or(Value::Null, Value::Float),
        });
    }
    out
}

/// Names of the fields this driver leaves aside (the soil probes' raw
/// values, the piezo's with a bucket gauge, a sensor Ecowitt sells next…),
/// for one log line, so a new sensor is noticed. With both rain gauges, the piezo's fields are read but unused.
pub fn unread(form: &Form) -> Vec<&str> {
    let known = |name: &str| {
        META.contains(&name)
            || FIELDS.iter().any(|(w, ..)| *w == name)
            || RAIN.iter().any(|(b, p, _)| *b == name || *p == name)
            || (1..=8u8).any(|n| {
                name == format!("temp{n}f")
                    || name == format!("humidity{n}")
                    || name == format!("batt{n}")
            })
            || NUMBERED
                .iter()
                .any(|(prefix, max, ..)| (1..=*max).any(|n| name == format!("{prefix}{n}")))
    };
    form.keys()
        .map(String::as_str)
        .filter(|name| !known(name))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forms_decode_and_stay_bounded() {
        let form = parse_form(b"a=1&b=x+y%20z&c&=&d=%zz&d=2").unwrap();
        assert_eq!(form["a"], "1");
        assert_eq!(form["b"], "x y z");
        assert_eq!(form["c"], "");
        assert_eq!(form["d"], "2", "the last one wins");
        assert_eq!(parse_form(b"x=%").unwrap()["x"], "%");
        assert_eq!(parse_form(b"x=%4").unwrap()["x"], "%4");
        assert_eq!(parse_form(b"x=%ff"), Err(FormError::NotText));
        let many: Vec<String> = (0..=MAX_PAIRS).map(|i| format!("k{i}=1")).collect();
        assert_eq!(
            parse_form(many.join("&").as_bytes()),
            Err(FormError::TooMany)
        );
        let long = format!("k={}", "9".repeat(MAX_VALUE + 1));
        assert_eq!(parse_form(long.as_bytes()), Err(FormError::TooLong));
    }

    #[test]
    fn conversions() {
        assert_eq!(convert("68.0", Conv::Fahrenheit), Value::Float(20.0));
        assert_eq!(convert("29.92", Conv::InHg), Value::Float(1013.2));
        assert_eq!(convert("10", Conv::Mph), Value::Float(16.1));
        assert_eq!(convert("0.5", Conv::Inch), Value::Float(12.7));
        assert_eq!(convert("0", Conv::Flag), Value::Bool(false));
        assert_eq!(convert("1", Conv::Flag), Value::Bool(true));
        assert_eq!(convert("--", Conv::Fahrenheit), Value::Null);
        assert_eq!(convert("", Conv::Mph), Value::Null);
        assert_eq!(convert("NaN", Conv::Same(1)), Value::Null);
        assert_eq!(dew_point(20.0, 50.0), Some(9.3));
        assert_eq!(dew_point(20.0, 0.0), None);
        assert_eq!(convert("4", Conv::Level), Value::Float(80.0));
        assert_eq!(convert("6", Conv::Level), Value::Float(100.0), "on mains");
        assert_eq!(
            convert("1707382800", Conv::Instant),
            Value::Text("2024-02-08T09:00:00Z".into())
        );
        assert_eq!(convert("0", Conv::Instant), Value::Null);
    }

    #[test]
    fn every_key_read_has_a_point() {
        let keys = FIELDS
            .iter()
            .map(|(_, _, k, _)| *k)
            .chain(RAIN.iter().map(|(_, _, k)| *k))
            .chain(NUMBERED.iter().map(|(_, _, _, k, _)| *k))
            .chain(["temperature", "humidity", "battery_low"])
            .chain(["dew_point", "illuminance"]);
        for key in keys {
            let p = point(key).unwrap_or_else(|| panic!("{key}: no point"));
            assert!(
                p.binary || p.text || unit(p).is_some() || key == "lightning_count",
                "{key}: no unit"
            );
        }
    }
}
