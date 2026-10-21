//! What an Ecowitt station says, in Moli's words.
//!
//! The station posts a form of `field=value` pairs, in imperial units
//! whatever its screen shows (°F, inHg, inches, mph). Each field known here
//! becomes a point, converted to the house's units (°C, hPa, mm, km/h), on
//! the device it belongs to: the station itself (outdoor array, rain, wind,
//! lightning), the console indoors, and one device per extra sensor
//! (thermo-hygrometer channel, soil probe, leak detector…), so each can be
//! placed in its own room. A few values the station does not send are
//! derived, as Home Assistant's `aioecowitt` does: dew points, feels-like
//! temperature, wind chill, an illuminance estimate.
//!
//! Field names and meanings: Ecowitt's « Customized » upload, as documented
//! by `aioecowitt` (Apache-2.0); the formulas are the public NWS ones (heat
//! index, wind chill) and the Magnus-type dew point `meteocalc` uses.

use std::collections::{BTreeMap, HashMap};

use moli_core::{Access, Kind, PointSpec, Semantic, Unit, Value};

const HPA_PER_INHG: f64 = 33.863_9;
const MM_PER_INCH: f64 = 25.4;
const KMH_PER_MPH: f64 = 1.609_344;
/// Ecowitt's own estimate of daylight from solar radiation (lux per W/m²,
/// inverted): rough, but what the station's apps show.
const WATT_M2_PER_LUX: f64 = 0.0079;

/// Fields that describe the station, never a measure.
const META: &[&str] = &[
    "PASSKEY",
    "stationtype",
    "model",
    "freq",
    "dateutc",
    "runtime",
    "heap",
    "interval",
    "fields",
    "ws90_ver",
    "ws85_ver",
];

/// Which device of the station a point lives on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    /// The station: outdoor array, rain, wind, light, lightning, pressure.
    Station,
    /// The console (or gateway) indoors.
    Indoor,
    /// The WH45/WH46 air quality monitor (CO₂, particles).
    AirQuality,
    /// A WN31/WH31 thermo-hygrometer, channel 1 to 8.
    Channel(u8),
    /// A WH51 soil moisture probe, 1 to 16.
    Soil(u8),
    /// A soil moisture, temperature and conductivity probe, 1 to 16.
    SoilEc(u8),
    /// A WN34 probe thermometer (soil, water), 1 to 8.
    WaterTemp(u8),
    /// A WH55 leak detector, 1 to 4.
    Leak(u8),
    /// A WH41/WH43 particle sensor, 1 to 4.
    Pm25(u8),
    /// A WN35 leaf wetness sensor, 1 to 8.
    Leaf(u8),
    /// A WH54 distance sensor (water or snow depth), 1 to 4.
    Depth(u8),
}

impl Group {
    /// The device's native id: the station's passkey, and a suffix per sensor.
    pub fn native(self, passkey: &str) -> String {
        match self {
            Self::Station => passkey.to_owned(),
            Self::Indoor => format!("{passkey}_indoor"),
            Self::AirQuality => format!("{passkey}_wh45"),
            Self::Channel(n) => format!("{passkey}_ch{n}"),
            Self::Soil(n) => format!("{passkey}_soil{n}"),
            Self::SoilEc(n) => format!("{passkey}_soilec{n}"),
            Self::WaterTemp(n) => format!("{passkey}_tf{n}"),
            Self::Leak(n) => format!("{passkey}_leak{n}"),
            Self::Pm25(n) => format!("{passkey}_pm25_{n}"),
            Self::Leaf(n) => format!("{passkey}_leaf{n}"),
            Self::Depth(n) => format!("{passkey}_lds{n}"),
        }
    }

    /// The sensor's model: the station's own for the station and console.
    pub fn model(self, station: &str) -> String {
        match self {
            Self::Station | Self::Indoor => station,
            Self::AirQuality => "WH45",
            Self::Channel(_) => "WN31",
            Self::Soil(_) => "WH51",
            Self::SoilEc(_) => "WH52",
            Self::WaterTemp(_) => "WN34",
            Self::Leak(_) => "WH55",
            Self::Pm25(_) => "WH41",
            Self::Leaf(_) => "WN35",
            Self::Depth(_) => "WH54",
        }
        .to_owned()
    }

    /// The name the device is first shown with (a label renames it).
    pub fn name(self, station: &str) -> String {
        use moli_i18n::tr;
        match self {
            Self::Station => tr!("pilotes.ecowitt.appareil.station", modele = station),
            Self::Indoor => tr!("pilotes.ecowitt.appareil.interieur", modele = station),
            Self::AirQuality => tr!("pilotes.ecowitt.appareil.air"),
            Self::Channel(n) => tr!("pilotes.ecowitt.appareil.canal", n = n),
            Self::Soil(n) => tr!("pilotes.ecowitt.appareil.sol", n = n),
            Self::SoilEc(n) => tr!("pilotes.ecowitt.appareil.sol_ec", n = n),
            Self::WaterTemp(n) => tr!("pilotes.ecowitt.appareil.sonde", n = n),
            Self::Leak(n) => tr!("pilotes.ecowitt.appareil.fuite", n = n),
            Self::Pm25(n) => tr!("pilotes.ecowitt.appareil.particules", n = n),
            Self::Leaf(n) => tr!("pilotes.ecowitt.appareil.feuilles", n = n),
            Self::Depth(n) => tr!("pilotes.ecowitt.appareil.distance", n = n),
        }
    }
}

/// How a field's text becomes a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Conv {
    Fahrenheit,
    InHg,
    /// Inches, or inches per hour.
    Inches,
    Mph,
    Number,
    Count,
    /// A battery level 0 to 5 (6: on mains), as a percentage.
    Level,
    /// Not zero: true (a low battery, a leak, rain falling).
    Flag,
    /// Seconds since 1970, as an instant.
    Timestamp,
}

/// One field of the station, placed: which device, which point, how read.
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub group: Group,
    pub spec: PointSpec,
    conv: Conv,
}

/// What the station said once, in Moli's words.
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    /// The station's identity: an MD5 of its hardware address, 32 hex digits.
    pub passkey: String,
    /// `GW2000A`, `HP2551`… (`model`, else the start of `stationtype`).
    pub model: String,
    /// `GW2000A_V3.1.0`: the firmware.
    pub firmware: Option<String>,
    /// Seconds between two uploads, when the station says so.
    pub interval: Option<u64>,
    /// Every device's points and values, in the order the station gave them.
    pub groups: BTreeMap<Group, Vec<(PointSpec, Value)>>,
    /// Fields this driver does not know (logged once each).
    pub unknown: Vec<String>,
}

/// Why a form is not a reading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotAReading {
    NoPasskey,
    BadPasskey,
}

impl Reading {
    pub fn parse(form: &[(String, String)]) -> Result<Self, NotAReading> {
        let raw: HashMap<&str, &str> = form.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let passkey = raw.get("PASSKEY").ok_or(NotAReading::NoPasskey)?.trim();
        if passkey.len() != 32 || !passkey.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(NotAReading::BadPasskey);
        }
        let firmware = raw
            .get("stationtype")
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
        let model = raw
            .get("model")
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                firmware
                    .as_deref()
                    .map(|f| f.split('_').next().unwrap_or(f).to_owned())
            })
            .unwrap_or_else(|| "Ecowitt".to_owned());
        let interval = raw.get("interval").and_then(|s| s.trim().parse().ok());

        let mut groups: BTreeMap<Group, Vec<(PointSpec, Value)>> = BTreeMap::new();
        let mut unknown = Vec::new();
        // The station's own order, not the map's.
        for (key, value) in form {
            if META.contains(&key.as_str()) || ignored(key) {
                continue;
            }
            match classify(key) {
                Some(field) => groups
                    .entry(field.group)
                    .or_default()
                    .push((field.spec, convert(field.conv, value))),
                None => unknown.push(key.clone()),
            }
        }
        for (group, spec, value) in derived(&raw) {
            groups.entry(group).or_default().push((spec, value));
        }
        Ok(Self {
            passkey: passkey.to_ascii_uppercase(),
            model,
            firmware,
            interval,
            groups,
            unknown,
        })
    }
}

/// Raw readings Home Assistant keeps hidden too: the probes' analog values.
fn ignored(key: &str) -> bool {
    key.starts_with("soilad") || key.starts_with("soil_ec_ad") || key.starts_with("soil_ec_hum_ad")
}

/// `prefix<n>suffix` with `n` in `1..=max`.
fn channel(key: &str, prefix: &str, suffix: &str, max: u8) -> Option<u8> {
    let digits = key.strip_prefix(prefix)?.strip_suffix(suffix)?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) || digits.starts_with('0') {
        return None;
    }
    digits.parse().ok().filter(|n| (1..=max).contains(n))
}

/// A numbered sensor's field: prefix, suffix, highest number, device, point, reading.
type Numbered = (
    &'static str,
    &'static str,
    u8,
    fn(u8) -> Group,
    &'static str,
    Conv,
);

/// A field of the station, placed; `None` when it is not known here.
#[allow(clippy::too_many_lines)] // One table, read top to bottom.
fn classify(key: &str) -> Option<Field> {
    use Conv::{Count, Fahrenheit, Flag, InHg, Inches, Level, Mph, Number, Timestamp};
    use Group::{AirQuality, Indoor, Station};
    let at = |group, point: &str, conv| Some(field(group, point, None, conv));
    let of = |group, point: &str, sensor: &str, conv| Some(field(group, point, Some(sensor), conv));
    match key {
        "tempf" => return at(Station, "temperature", Fahrenheit),
        "humidity" => return at(Station, "humidity", Number),
        "baromrelin" => return at(Station, "pressure", InHg),
        "baromabsin" => return at(Station, "pressure_absolute", InHg),
        "vpd" => return at(Station, "vpd", InHg),
        "winddir" => return at(Station, "wind_direction", Number),
        "winddir_avg10m" => return at(Station, "wind_direction_avg10m", Number),
        "windspeedmph" => return at(Station, "wind_speed", Mph),
        "windspdmph_avg10m" => return at(Station, "wind_speed_avg10m", Mph),
        "windgustmph" => return at(Station, "wind_gust", Mph),
        "maxdailygust" => return at(Station, "wind_gust_max_day", Mph),
        "solarradiation" => return at(Station, "solar_radiation", Number),
        "uv" => return at(Station, "uv", Number),
        "rainratein" => return at(Station, "rain_rate", Inches),
        "eventrainin" => return at(Station, "rain_event", Inches),
        "hourlyrainin" => return at(Station, "rain_hour", Inches),
        "dailyrainin" => return at(Station, "rain_day", Inches),
        "weeklyrainin" => return at(Station, "rain_week", Inches),
        "monthlyrainin" => return at(Station, "rain_month", Inches),
        "yearlyrainin" => return at(Station, "rain_year", Inches),
        "totalrainin" => return at(Station, "rain_total", Inches),
        "last24hrainin" => return at(Station, "rain_24h", Inches),
        "rrain_piezo" => return at(Station, "piezo_rain_rate", Inches),
        "erain_piezo" => return at(Station, "piezo_rain_event", Inches),
        "hrain_piezo" => return at(Station, "piezo_rain_hour", Inches),
        "drain_piezo" => return at(Station, "piezo_rain_day", Inches),
        "wrain_piezo" => return at(Station, "piezo_rain_week", Inches),
        "mrain_piezo" => return at(Station, "piezo_rain_month", Inches),
        "yrain_piezo" => return at(Station, "piezo_rain_year", Inches),
        "last24hrain_piezo" => return at(Station, "piezo_rain_24h", Inches),
        "srain_piezo" => return at(Station, "raining", Flag),
        "lightning" => return at(Station, "lightning_distance", Number),
        "lightning_num" => return at(Station, "lightning_count", Count),
        "lightning_time" => return at(Station, "lightning_last", Timestamp),
        "bgt" => return at(Station, "black_globe", Fahrenheit),
        "wbgt" => return at(Station, "wet_bulb_globe", Fahrenheit),
        "wh65batt" => return of(Station, "battery_low", "WH65", Flag),
        "wh26batt" => return of(Station, "battery_low", "WH26", Flag),
        "wh68batt" => return of(Station, "battery_voltage", "WH68", Number),
        "wh80batt" => return of(Station, "battery_voltage", "WH80", Number),
        "wh85batt" => return of(Station, "battery_voltage", "WH85", Number),
        "wh90batt" => return of(Station, "battery_voltage", "WH90", Number),
        "wh40batt" => return of(Station, "battery_voltage", "WH40", Number),
        "wn20batt" => return of(Station, "battery_voltage", "WN20", Number),
        "bgtbatt" => return of(Station, "battery_voltage", "WN38", Number),
        "wh57batt" => return of(Station, "battery", "WH57", Level),
        "ws85cap_volt" => return of(Station, "capacitor", "WH85", Number),
        "ws90cap_volt" => return of(Station, "capacitor", "WH90", Number),
        "tempinf" => return at(Indoor, "temperature", Fahrenheit),
        "humidityin" => return at(Indoor, "humidity", Number),
        "co2in" => return at(Indoor, "co2", Number),
        "co2in_24h" => return at(Indoor, "co2_24h", Number),
        "wh25batt" => return at(Indoor, "battery_low", Flag),
        "console_batt" => return at(Indoor, "battery_voltage", Number),
        "tf_co2" => return at(AirQuality, "temperature", Fahrenheit),
        "humi_co2" => return at(AirQuality, "humidity", Number),
        "co2" => return at(AirQuality, "co2", Number),
        "co2_24h" => return at(AirQuality, "co2_24h", Number),
        "pm1_co2" => return at(AirQuality, "pm1", Number),
        "pm1_24h_co2" => return at(AirQuality, "pm1_24h", Number),
        "pm25_co2" => return at(AirQuality, "pm25", Number),
        "pm25_24h_co2" => return at(AirQuality, "pm25_24h", Number),
        "pm4_co2" => return at(AirQuality, "pm4", Number),
        "pm4_24h_co2" => return at(AirQuality, "pm4_24h", Number),
        "pm10_co2" => return at(AirQuality, "pm10", Number),
        "pm10_24h_co2" => return at(AirQuality, "pm10_24h", Number),
        "co2_batt" => return at(AirQuality, "battery", Level),
        _ => {}
    }
    // The extra sensors, numbered.
    let numbered: [Numbered; 23] = [
        ("temp", "f", 8, Group::Channel, "temperature", Fahrenheit),
        ("humidity", "", 8, Group::Channel, "humidity", Number),
        ("batt", "", 8, Group::Channel, "battery_low", Flag),
        ("soilmoisture", "", 16, Group::Soil, "soil_moisture", Number),
        ("soilbatt", "", 16, Group::Soil, "battery_voltage", Number),
        (
            "soil_ec_hum",
            "",
            16,
            Group::SoilEc,
            "soil_moisture",
            Number,
        ),
        (
            "soil_ec_temp",
            "",
            16,
            Group::SoilEc,
            "temperature",
            Fahrenheit,
        ),
        (
            "soil_ec_batt",
            "",
            16,
            Group::SoilEc,
            "battery_voltage",
            Number,
        ),
        (
            "soil_ec",
            "",
            16,
            Group::SoilEc,
            "soil_conductivity",
            Number,
        ),
        ("tf_ch", "", 8, Group::WaterTemp, "temperature", Fahrenheit),
        (
            "tf_batt",
            "",
            8,
            Group::WaterTemp,
            "battery_voltage",
            Number,
        ),
        ("leak_ch", "", 4, Group::Leak, "leak", Flag),
        ("leakbatt", "", 8, Group::Leak, "battery", Level),
        ("pm25_ch", "", 4, Group::Pm25, "pm25", Number),
        ("pm25_avg_24h_ch", "", 4, Group::Pm25, "pm25_24h", Number),
        ("pm25batt", "", 8, Group::Pm25, "battery", Level),
        ("leafwetness_ch", "", 8, Group::Leaf, "leaf_wetness", Number),
        ("leaf_batt", "", 8, Group::Leaf, "battery_voltage", Number),
        ("depth_ch", "", 4, Group::Depth, "depth", Number),
        ("air_ch", "", 4, Group::Depth, "air_gap", Number),
        ("thi_ch", "", 4, Group::Depth, "depth_total", Number),
        ("ldsheat_ch", "", 4, Group::Depth, "heater_count", Count),
        ("ldsbatt", "", 4, Group::Depth, "battery_voltage", Number),
    ];
    numbered
        .iter()
        .find_map(|(prefix, suffix, max, group, point, conv)| {
            channel(key, prefix, suffix, *max).map(|n| field(group(n), point, None, *conv))
        })
}

/// A point of `group`; `sensor` names the sensor when the station carries
/// several of a kind (its batteries): the key and label say which.
fn field(group: Group, point: &str, sensor: Option<&str>, conv: Conv) -> Field {
    let base = point.strip_prefix("piezo_").unwrap_or(point);
    let (kind, unit, semantic) = shape(base, conv);
    let named = moli_i18n::tr(&format!("pilotes.ecowitt.point.{base}"));
    let (key, label) = match sensor {
        Some(sensor) => (
            format!("{point}_{}", sensor.to_ascii_lowercase()),
            moli_i18n::tr!("pilotes.ecowitt.capteur", mesure = named, capteur = sensor),
        ),
        None if base != point => (
            point.to_owned(),
            moli_i18n::tr!("pilotes.ecowitt.piezo", mesure = named),
        ),
        None => (point.to_owned(), named),
    };
    Field {
        group,
        spec: PointSpec {
            key: key.into(),
            label: label.into(),
            kind,
            access: Access {
                read: true,
                write: false,
            },
            unit: unit.and_then(Unit::parse),
            semantic,
        },
        conv,
    }
}

/// What a point is: its shape, unit and meaning. Only the air or soil
/// temperature of a device is a `Temperature`: a dew point or a feels-like
/// is not the warmth of a room.
fn shape(base: &str, conv: Conv) -> (Kind, Option<&'static str>, Semantic) {
    let number = Kind::Numeric {
        min: None,
        max: None,
        step: None,
    };
    let percent = Kind::Numeric {
        min: Some(0.0),
        max: Some(100.0),
        step: None,
    };
    match (base, conv) {
        (_, Conv::Timestamp) => (Kind::Text, None, Semantic::Other),
        ("leak", _) => (Kind::Binary, None, Semantic::WaterLeak),
        ("battery_low", _) => (Kind::Binary, None, Semantic::BatteryLow),
        (_, Conv::Flag) => (Kind::Binary, None, Semantic::Other),
        ("temperature", _) => (number, Some("°C"), Semantic::Temperature),
        ("dew_point" | "feels_like" | "wind_chill" | "black_globe" | "wet_bulb_globe", _) => {
            (number, Some("°C"), Semantic::Other)
        }
        ("humidity", _) => (percent, Some("%"), Semantic::Humidity),
        ("pressure", _) => (number, Some("hPa"), Semantic::Pressure),
        ("pressure_absolute" | "vpd", _) => (number, Some("hPa"), Semantic::Other),
        ("wind_direction" | "wind_direction_avg10m", _) => (
            Kind::Numeric {
                min: Some(0.0),
                max: Some(360.0),
                step: None,
            },
            Some("°"),
            Semantic::Other,
        ),
        ("wind_speed" | "wind_speed_avg10m" | "wind_gust" | "wind_gust_max_day", _) => {
            (number, Some("km/h"), Semantic::Other)
        }
        ("solar_radiation", _) => (number, Some("W/m²"), Semantic::Other),
        ("illuminance", _) => (number, Some("lx"), Semantic::Illuminance),
        ("rain_rate", _) => (number, Some("mm/h"), Semantic::Other),
        (b, _) if b.starts_with("rain_") => (number, Some("mm"), Semantic::Other),
        ("lightning_distance", _) => (number, Some("km"), Semantic::Other),
        ("co2" | "co2_24h", _) => (number, Some("ppm"), Semantic::Other),
        (b, _) if b.starts_with("pm") => (number, Some("µg/m³"), Semantic::Other),
        ("soil_moisture" | "leaf_wetness", _) => (percent, Some("%"), Semantic::Other),
        ("soil_conductivity", _) => (number, Some("µS/cm"), Semantic::Other),
        ("depth" | "air_gap" | "depth_total", _) => (number, Some("mm"), Semantic::Other),
        ("battery", _) => (percent, Some("%"), Semantic::Battery),
        ("battery_voltage" | "capacitor", _) => (number, Some("V"), Semantic::Other),
        _ => (number, None, Semantic::Other),
    }
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

fn celsius(fahrenheit: f64) -> f64 {
    (fahrenheit - 32.0) / 1.8
}

fn number(text: &str) -> Option<f64> {
    text.trim().parse::<f64>().ok().filter(|x| x.is_finite())
}

/// A field's text as a value; empty or unreadable: no value (`null`).
#[allow(clippy::cast_possible_truncation)]
fn convert(conv: Conv, text: &str) -> Value {
    let Some(x) = number(text) else {
        return Value::Null;
    };
    match conv {
        Conv::Fahrenheit => Value::Float(round1(celsius(x))),
        Conv::InHg => Value::Float(round1(x * HPA_PER_INHG)),
        Conv::Inches => Value::Float(round1(x * MM_PER_INCH)),
        Conv::Mph => Value::Float(round1(x * KMH_PER_MPH)),
        Conv::Number => Value::Float(x),
        Conv::Count => Value::Int(x.round() as i64),
        Conv::Level => Value::Float((x.clamp(0.0, 5.0) * 20.0).round()),
        Conv::Flag => Value::Bool(x.abs() > 0.0),
        Conv::Timestamp => {
            let seconds = x.round() as i64;
            if seconds <= 0 {
                return Value::Null;
            }
            jiff::Timestamp::from_second(seconds)
                .map_or(Value::Null, |t| Value::Text(t.to_string().into()))
        }
    }
}

/// Values the station does not send, from those it does.
fn derived(raw: &HashMap<&str, &str>) -> Vec<(Group, PointSpec, Value)> {
    let get = |key: &str| raw.get(key).and_then(|v| number(v));
    let mut out = Vec::new();
    let mut push = |group, point: &str, value: Value| {
        out.push((group, field(group, point, None, Conv::Number).spec, value));
    };
    let mut dew = |group, temperature: &str, humidity: &str| {
        if let (Some(t), Some(h)) = (get(temperature), get(humidity))
            && h > 0.0
        {
            push(
                group,
                "dew_point",
                Value::Float(round1(dew_point(celsius(t), h))),
            );
        }
    };
    dew(Group::Station, "tempf", "humidity");
    dew(Group::Indoor, "tempinf", "humidityin");
    for n in 1..=8u8 {
        dew(
            Group::Channel(n),
            &format!("temp{n}f"),
            &format!("humidity{n}"),
        );
    }
    drop(dew);
    if let (Some(t), Some(h), Some(w)) = (get("tempf"), get("humidity"), get("windspeedmph")) {
        push(
            Group::Station,
            "feels_like",
            Value::Float(round1(celsius(feels_like(t, h, w)))),
        );
        push(
            Group::Station,
            "wind_chill",
            wind_chill(t, w).map_or(Value::Null, |c| Value::Float(round1(celsius(c)))),
        );
    }
    if let Some(radiation) = get("solarradiation") {
        push(
            Group::Station,
            "illuminance",
            Value::Float(round1(radiation / WATT_M2_PER_LUX)),
        );
    }
    out
}

/// Dew point (°C) of air at `t` °C and `rh` % (Magnus form, the constants
/// `meteocalc` uses, over water above 0 °C and over ice below).
fn dew_point(t: f64, rh: f64) -> f64 {
    let (b, c) = if t > 0.0 {
        (17.368, 238.88)
    } else {
        (17.966, 247.15)
    };
    let gamma = (rh / 100.0).ln() + b * t / (c + t);
    c * gamma / (b - gamma)
}

/// Wind chill (°F, NWS 2001), defined at 50 °F and below, wind 3 mph and up.
fn wind_chill(t: f64, mph: f64) -> Option<f64> {
    (t <= 50.0 && mph >= 3.0).then(|| {
        let v = mph.powf(0.16);
        35.74 + 0.6215 * t - 35.75 * v + 0.4275 * t * v
    })
}

/// Heat index (°F, NWS: Steadman's simple form, Rothfusz's regression from
/// 80 °F, and its two adjustments).
fn heat_index(t: f64, rh: f64) -> f64 {
    let simple = 0.5 * (t + 61.0 + (t - 68.0) * 1.2 + rh * 0.094);
    if simple < 80.0 {
        return simple;
    }
    let mut hi = -42.379 + 2.049_015_23 * t + 10.143_331_27 * rh
        - 0.224_755_41 * t * rh
        - 6.837_83e-3 * t * t
        - 5.481_717e-2 * rh * rh
        + 1.228_74e-3 * t * t * rh
        + 8.5282e-4 * t * rh * rh
        - 1.99e-6 * t * t * rh * rh;
    if rh < 13.0 && (80.0..=112.0).contains(&t) {
        hi -= (13.0 - rh) / 4.0 * ((17.0 - (t - 95.0).abs()) / 17.0).sqrt();
    } else if rh > 85.0 && (80.0..=87.0).contains(&t) {
        hi += (rh - 85.0) / 10.0 * ((87.0 - t) / 5.0);
    }
    hi
}

/// Feels-like (°F): wind chill when cold and windy, heat index when hot,
/// the air's own temperature between.
fn feels_like(t: f64, rh: f64, mph: f64) -> f64 {
    if let Some(chill) = wind_chill(t, mph) {
        chill
    } else if t >= 80.0 {
        heat_index(t, rh)
    } else {
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A form as the station posts it, from `fixtures/` (`key=value` lines).
    fn form(text: &str) -> Vec<(String, String)> {
        text.lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect()
    }

    fn value(reading: &Reading, group: Group, key: &str) -> Value {
        reading.groups[&group]
            .iter()
            .find(|(spec, _)| &*spec.key == key)
            .map_or_else(|| panic!("{group:?} has no {key}"), |(_, v)| v.clone())
    }

    fn float(reading: &Reading, group: Group, key: &str) -> f64 {
        value(reading, group, key)
            .as_f64()
            .unwrap_or_else(|| panic!("{key} is not a number"))
    }

    fn gw2000a() -> Reading {
        Reading::parse(&form(include_str!(
            "../../../integrations/ecowitt/fixtures/gw2000a.kv"
        )))
        .unwrap()
    }

    #[test]
    fn the_station_is_known_by_its_passkey() {
        let r = gw2000a();
        assert_eq!(r.passkey, "0123456789ABCDEF0123456789ABCDEF");
        assert_eq!(r.model, "GW2000A");
        assert_eq!(r.firmware.as_deref(), Some("GW2000A_V3.1.0"));
        assert_eq!(r.interval, Some(60));
        assert!(r.unknown.is_empty(), "{:?}", r.unknown);
        assert_eq!(Group::Station.native(&r.passkey), r.passkey);
        assert_eq!(
            Group::Channel(2).native(&r.passkey),
            "0123456789ABCDEF0123456789ABCDEF_ch2"
        );
        let no_key = form("tempf=50\n");
        assert_eq!(Reading::parse(&no_key), Err(NotAReading::NoPasskey));
        let bad = form("PASSKEY=../../x\n");
        assert_eq!(Reading::parse(&bad), Err(NotAReading::BadPasskey));
    }

    #[test]
    fn imperial_becomes_metric() {
        let r = gw2000a();
        // 87.08 °F, 27.885 inHg, 2.91 mph, 0.098 in: as aioecowitt reads them.
        assert!((float(&r, Group::Station, "temperature") - 30.6).abs() < 1e-9);
        assert!((float(&r, Group::Station, "pressure") - 944.3).abs() < 1e-9);
        assert!((float(&r, Group::Station, "wind_speed") - 4.7).abs() < 1e-9);
        assert!((float(&r, Group::Station, "rain_day") - 2.5).abs() < 1e-9);
        assert!((float(&r, Group::Station, "piezo_rain_day") - 2.5).abs() < 1e-9);
        assert!((float(&r, Group::Indoor, "temperature") - 26.0).abs() < 1e-9);
        assert!((float(&r, Group::Station, "uv") - 4.0).abs() < 1e-9);
        assert_eq!(value(&r, Group::Station, "raining"), Value::Bool(false));
        assert_eq!(
            value(&r, Group::Station, "lightning_last"),
            Value::Text("2024-02-08T09:00:00Z".into())
        );
        assert_eq!(value(&r, Group::Station, "lightning_count"), Value::Int(3));
        // No strike yet: no distance, not zero kilometres.
        assert_eq!(value(&r, Group::Station, "lightning_distance"), Value::Null);
    }

    #[test]
    fn derived_values_match_aioecowitt() {
        let r = gw2000a();
        // aioecowitt's own test on the same readings: 13.0, 15.7, 29.7 °C.
        assert!((float(&r, Group::Station, "dew_point") - 13.0).abs() < 1e-9);
        assert!((float(&r, Group::Indoor, "dew_point") - 15.7).abs() < 1e-9);
        assert!((float(&r, Group::Station, "feels_like") - 29.7).abs() < 1e-9);
        // Too warm for a wind chill.
        assert_eq!(value(&r, Group::Station, "wind_chill"), Value::Null);
        assert!((float(&r, Group::Station, "illuminance") - 67_167.1).abs() < 1e-6);
        // Cold and windy: wind chill and feels-like agree.
        let cold = Reading::parse(&form(
            "PASSKEY=0123456789ABCDEF0123456789ABCDEF\ntempf=23.0\nhumidity=80\nwindspeedmph=15.0\n",
        ))
        .unwrap();
        let chill = float(&cold, Group::Station, "wind_chill");
        assert!((chill - (-12.2)).abs() < 1e-9, "{chill}");
        assert!((float(&cold, Group::Station, "feels_like") - chill).abs() < 1e-9);
    }

    #[test]
    fn each_sensor_is_its_own_device() {
        let r = gw2000a();
        let groups: Vec<Group> = r.groups.keys().copied().collect();
        assert_eq!(
            groups,
            [
                Group::Station,
                Group::Indoor,
                Group::Channel(1),
                Group::Soil(1),
                Group::Leak(1),
                Group::Leaf(1)
            ]
        );
        assert!((float(&r, Group::Channel(1), "temperature") - 20.0).abs() < 1e-9);
        assert!((float(&r, Group::Channel(1), "dew_point") - 9.3).abs() < 1e-9);
        assert_eq!(
            value(&r, Group::Channel(1), "battery_low"),
            Value::Bool(false)
        );
        assert_eq!(value(&r, Group::Leak(1), "leak"), Value::Bool(false));
        assert!((float(&r, Group::Leak(1), "battery") - 100.0).abs() < 1e-9);
        assert!((float(&r, Group::Soil(1), "soil_moisture") - 41.0).abs() < 1e-9);
        // The station's batteries say which sensor they power.
        assert!((float(&r, Group::Station, "battery_voltage_wh90") - 2.9).abs() < 1e-9);
        assert!((float(&r, Group::Indoor, "battery_voltage") - 2.5).abs() < 1e-9);
        // Raw probe readings stay out, as in Home Assistant.
        assert!(
            r.groups[&Group::Soil(1)]
                .iter()
                .all(|(s, _)| !s.key.contains("ad"))
        );
    }

    #[test]
    fn points_have_meanings_and_units() {
        let r = gw2000a();
        let spec = |group, key: &str| {
            r.groups[&group]
                .iter()
                .find(|(s, _)| &*s.key == key)
                .map(|(s, _)| s.clone())
                .unwrap()
        };
        let t = spec(Group::Station, "temperature");
        assert_eq!(t.semantic, Semantic::Temperature);
        assert_eq!(t.unit, Some(Unit::Celsius));
        assert!(!t.access.write, "a weather station is read-only");
        assert_eq!(
            spec(Group::Station, "dew_point").semantic,
            Semantic::Other,
            "a dew point is not the warmth of a room"
        );
        assert_eq!(spec(Group::Leak(1), "leak").semantic, Semantic::WaterLeak);
        assert_eq!(spec(Group::Leak(1), "leak").kind, Kind::Binary);
        assert_eq!(
            spec(Group::Station, "rain_rate")
                .unit
                .as_ref()
                .map(Unit::symbol),
            Some("mm/h")
        );
        // Every label is a sentence of the catalogue, never a raw key.
        for (spec, _) in r.groups.values().flatten() {
            assert!(!spec.label.contains("pilotes."), "{}", spec.label);
        }
        for group in r.groups.keys() {
            assert!(!group.name(&r.model).contains("pilotes."));
        }
    }

    #[test]
    fn channels_are_read_exactly() {
        assert_eq!(channel("temp3f", "temp", "f", 8), Some(3));
        assert_eq!(channel("tempf", "temp", "f", 8), None);
        assert_eq!(channel("tempinf", "temp", "f", 8), None);
        assert_eq!(channel("temp9f", "temp", "f", 8), None);
        assert_eq!(channel("temp01f", "temp", "f", 8), None);
        assert_eq!(channel("soil_ec12", "soil_ec", "", 16), Some(12));
        assert_eq!(channel("soil_ec_hum2", "soil_ec", "", 16), None);
        assert_eq!(
            classify("soil_ec_hum2").map(|f| (f.group, f.spec.key.to_string())),
            Some((Group::SoilEc(2), "soil_moisture".to_owned()))
        );
        assert!(classify("tempc").is_none());
    }

    #[test]
    fn levels_and_flags() {
        assert_eq!(convert(Conv::Level, "5"), Value::Float(100.0));
        assert_eq!(convert(Conv::Level, "6"), Value::Float(100.0), "on mains");
        assert_eq!(convert(Conv::Level, "1"), Value::Float(20.0));
        assert_eq!(convert(Conv::Flag, "1"), Value::Bool(true));
        assert_eq!(convert(Conv::Flag, "0"), Value::Bool(false));
        assert_eq!(convert(Conv::Number, ""), Value::Null);
        assert_eq!(convert(Conv::Number, "nan"), Value::Null);
        assert_eq!(convert(Conv::Timestamp, "0"), Value::Null);
    }
}
