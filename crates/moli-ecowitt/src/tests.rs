//! Real posts (anonymised: the `PASSKEY`s are made up), decoded, then sent
//! to the driver over a socket.

use std::time::Duration;

use moli_core::{InstanceId, PointId, Value};
use moli_runtime::{Hub, HubOptions, spawn_driver};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio_util::sync::CancellationToken;

use super::*;

/// GW1200B with a WS90 (its battery), a WH40 rain gauge, a WH57 and a
/// WN38: aioecowitt's tests (`GW1200B_WN38_DATA`), as the form it came in.
const GW1200: &[u8] =
    include_bytes!("../../../integrations/ecowitt/fixtures/gw1200b-ws90-wh40.form");
/// GW2000A firmware 3.3.2 with a WS90 alone, 868 MHz (piezo rain).
const GW2000: &[u8] = include_bytes!("../../../integrations/ecowitt/fixtures/gw2000a-ws90.form");
/// GW2000A firmware 2.2.3, WS90 and WH57: empty lightning fields.
const GW2000_WH57: &[u8] =
    include_bytes!("../../../integrations/ecowitt/fixtures/gw2000a-ws90-wh57.form");

fn f(n: f64) -> Value {
    Value::Float(n)
}

#[test]
fn a_gw1200_with_a_bucket_gauge() {
    let form = parse_form(GW1200).unwrap();
    let about = report::station(&form);
    assert_eq!(about.model.as_deref(), Some("GW1200B"));
    assert_eq!(about.firmware.as_deref(), Some("GW1200B_V1.4.6"));
    assert_eq!(about.interval, Some(60));
    assert_eq!(
        about.passkey.as_deref(),
        Some("0123456789ABCDEF0123456789ABCDEF")
    );

    let indoor = values(&form, Part::Indoor);
    assert_eq!(indoor["temperature"], f(16.3));
    assert_eq!(indoor["humidity"], f(31.0));
    assert_eq!(indoor["pressure"], f(1013.4));
    assert_eq!(indoor["pressure_abs"], f(833.0));
    assert_eq!(indoor.len(), 4);

    let out = values(&form, Part::Outdoor);
    assert_eq!(out["temperature"], f(7.7));
    assert_eq!(out["humidity"], f(34.0));
    assert_eq!(out["dew_point"], f(-7.2));
    assert_eq!(out["wind_direction"], f(325.0));
    assert_eq!(out["wind_speed"], f(8.6));
    assert_eq!(out["wind_gusts"], f(11.9));
    assert_eq!(out["wind_gusts_max"], f(27.0));
    assert_eq!(out["solar_radiation"], f(0.0));
    assert_eq!(out["illuminance"], f(0.0));
    assert_eq!(out["uv_index"], f(0.0));
    assert_eq!(out["rain_rate"], f(0.0));
    assert_eq!(out["rain_today"], f(2.5));
    assert_eq!(out["rain_week"], f(6.2));
    assert_eq!(out["rain_total"], f(6.3));
    assert_eq!(out["battery_voltage"], f(2.9));
    assert_eq!(out["rain_battery_voltage"], f(1.4));
    assert!(!out.contains_key("raining"), "no piezo: nothing invented");
    assert_eq!(outdoor_model(&form).as_deref(), Some("WS90"));
    assert!(
        report::unread(&form).is_empty(),
        "{:?}",
        report::unread(&form)
    );
    let globe = values(&form, Part::Globe);
    assert_eq!(globe["black_globe"], f(6.2));
    assert_eq!(globe["wet_bulb_globe"], f(3.3));
    assert_eq!(globe["battery_voltage"], f(1.54));
    assert_eq!(values(&form, Part::Lightning)["battery"], f(80.0));
}

#[test]
fn a_ws90_alone_rains_on_its_piezo() {
    let form = parse_form(GW2000).unwrap();
    let out = values(&form, Part::Outdoor);
    assert_eq!(out["temperature"], f(35.1));
    assert_eq!(out["dew_point"], f(8.0));
    assert_eq!(out["wind_direction_avg"], f(135.0));
    assert_eq!(out["solar_radiation"], f(861.3));
    assert_eq!(out["illuminance"], f(109_025.0));
    assert_eq!(out["uv_index"], f(8.0));
    assert_eq!(out["raining"], Value::Bool(false));
    assert_eq!(out["rain_today"], f(0.0));
    assert_eq!(out["rain_week"], f(0.6));
    assert_eq!(out["rain_24h"], f(0.0));
    assert_eq!(out["capacitor_voltage"], f(5.3));
    assert_eq!(out["battery_voltage"], f(3.02));
    assert!(!out.contains_key("rain_total"), "the WS90 sends no total");
    assert_eq!(values(&form, Part::Indoor)["pressure"], f(992.1));
    assert!(report::unread(&form).is_empty());
    assert_eq!(values(&form, Part::Outdoor)["vpd"], f(45.8));
}

#[test]
fn empty_fields_and_a_lightning_sensor() {
    let form = parse_form(GW2000_WH57).unwrap();
    let out = values(&form, Part::Outdoor);
    assert_eq!(out["temperature"], f(23.3));
    assert_eq!(out["dew_point"], f(7.4));
    assert!(report::unread(&form).is_empty());
    let lightning = values(&form, Part::Lightning);
    assert_eq!(
        lightning["lightning_distance"],
        Value::Null,
        "no strike yet: no distance, not zero kilometres"
    );
    assert_eq!(lightning["battery"], f(100.0));
}

#[test]
fn extra_channels_and_unreadable_values() {
    let form =
        parse_form(b"PASSKEY=A&temp2f=50.0&humidity2=60&batt2=1&tempf=--&humidity=55").unwrap();
    let ch2 = values(&form, Part::Channel(2));
    assert_eq!(ch2["temperature"], f(10.0));
    assert_eq!(ch2["humidity"], f(60.0));
    assert_eq!(ch2["battery_low"], Value::Bool(true));
    let out = values(&form, Part::Outdoor);
    assert_eq!(
        out["temperature"],
        Value::Null,
        "no measure, never a stale one"
    );
    assert_eq!(out["dew_point"], Value::Null);
}

#[test]
fn every_sensor_ecowitt_sells_is_a_device_of_its_own() {
    let form = parse_form(
        b"PASSKEY=A&co2in=612&tf_co2=68.0&humi_co2=45&co2=830&pm25_co2=4.2&co2_batt=6\
          &soilmoisture3=41&soilbatt3=1.5&soilad3=120&tf_ch2=50.0&leak_ch1=1&leakbatt1=3\
          &pm25_ch4=12.3&pm25_avg_24h_ch4=9.8&leafwetness_ch1=14&leaf_batt1=1.78\
          &lightning=12&lightning_num=3&lightning_time=1707382800",
    )
    .unwrap();
    assert_eq!(values(&form, Part::Indoor)["co2"], f(612.0));
    let air = values(&form, Part::Air);
    assert_eq!(air["temperature"], f(20.0));
    assert_eq!(air["co2"], f(830.0));
    assert_eq!(air["pm25"], f(4.2));
    assert_eq!(air["battery"], f(100.0), "on mains");
    let soil = values(&form, Part::Soil(3));
    assert_eq!(soil["soil_moisture"], f(41.0));
    assert_eq!(soil["battery_voltage"], f(1.5));
    assert_eq!(values(&form, Part::Probe(2))["temperature"], f(10.0));
    let leak = values(&form, Part::Leak(1));
    assert_eq!(leak["leak"], Value::Bool(true));
    assert_eq!(leak["battery"], f(60.0));
    assert_eq!(values(&form, Part::Pm25(4))["pm25_24h"], f(9.8));
    assert_eq!(values(&form, Part::Leaf(1))["leaf_wetness"], f(14.0));
    let lightning = values(&form, Part::Lightning);
    assert_eq!(lightning["lightning_distance"], f(12.0));
    assert_eq!(lightning["lightning_count"], f(3.0));
    assert_eq!(
        lightning["lightning_last"],
        Value::Text("2024-02-08T09:00:00Z".into())
    );
    // The probes' raw values stay aside, said once in the log.
    assert_eq!(report::unread(&form), ["soilad3"]);
    let mut station = Station::new(None);
    let parts = station.learn(&readings(&form));
    assert_eq!(parts.len(), 8);
    for keys in station.declared.values() {
        for k in keys {
            assert!(spec(k).is_some(), "{k}");
        }
    }
    assert_eq!(Part::Pm25(4).native(), "pm25_4");
    assert_eq!(spec("lightning_last").unwrap().kind, Kind::Text);
    assert_eq!(spec("leak").unwrap().kind, Kind::Binary);
}

#[test]
fn devices_grow_with_their_points_in_a_stable_order() {
    let mut station = Station::new(None);
    let first = readings(&parse_form(b"tempf=50&uv=1").unwrap());
    assert_eq!(station.learn(&first), [Part::Outdoor]);
    assert!(station.learn(&first).is_empty(), "nothing new");
    let more = readings(&parse_form(b"humidity=50&tempf=50&tempinf=70").unwrap());
    assert_eq!(station.learn(&more), [Part::Indoor, Part::Outdoor]);
    assert_eq!(
        station.declared[&Part::Outdoor],
        ["temperature", "humidity", "dew_point", "uv_index"]
    );
    for keys in station.declared.values() {
        for k in keys {
            assert!(spec(k).is_some(), "{k}");
        }
    }
}

#[test]
fn options_are_checked() {
    let config = |text: &str| -> anyhow::Result<Ecowitt> { Ecowitt::new(toml::from_str(text)?) };
    let ok = config("").unwrap();
    assert_eq!(
        (ok.config.port, ok.config.path.as_str()),
        (49199, "/data/report/")
    );
    let pinned = config("passkey = \"0123456789abcdef0123456789abcdef\"").unwrap();
    assert_eq!(
        pinned.config.passkey.as_deref(),
        Some("0123456789ABCDEF0123456789ABCDEF")
    );
    assert!(config("passkey = \"nope\"").is_err());
    assert!(config("path = \"data\"").is_err());
    assert!(config("path = \"/a?b\"").is_err());
    assert!(config("port = 0").is_err());
    assert!(config("allow = [\"8.8.8.8\"]").is_err());
    assert!(config("allow = [\"192.168.1.20\"]").is_ok());
    assert!(config("token = \"x\"").is_err(), "unknown option");
}

async fn post(port: u16, body: &[u8]) -> String {
    let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    let head = format!(
        "POST /data/report HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    s.write_all(head.as_bytes()).await.unwrap();
    s.write_all(body).await.unwrap();
    let mut answer = String::new();
    s.read_to_string(&mut answer).await.unwrap();
    answer
}

#[tokio::test]
async fn the_driver_hears_the_first_gateway_and_only_it() {
    let port = {
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        probe.local_addr().unwrap().port()
    };
    let driver = Ecowitt::new(Config {
        port,
        listen: Some("127.0.0.1".parse().unwrap()),
        path: default_path(),
        passkey: None,
        allow: Vec::new(),
        name: Some("Jardin".into()),
    })
    .unwrap();
    let hub = Hub::new(HubOptions::default()).unwrap();
    let cancel = CancellationToken::new();
    spawn_driver(
        &hub,
        InstanceId::from("meteo"),
        Arc::new(driver),
        cancel.clone(),
    );
    let mut answer = String::new();
    for _ in 0..200 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            answer = post(port, GW2000).await;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    let point = |d: &str, k: &str| PointId::from(format!("meteo:{d}/{k}"));
    let mut seen = None;
    for _ in 0..200 {
        seen = hub.state(&point("outdoor", "temperature"));
        if seen.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(seen.unwrap().value, f(35.1));
    assert_eq!(
        hub.state(&point("indoor", "pressure")).unwrap().value,
        f(992.1)
    );
    let outdoor = hub
        .device(&moli_core::DeviceId::from("meteo:outdoor"))
        .unwrap();
    assert_eq!(&*outdoor.device.native_name, "Jardin");
    assert_eq!(outdoor.device.model.as_deref(), Some("WS90"));
    assert!(outdoor.device.point("rain_today").is_some());

    // Another gateway: refused, nothing changes.
    let answer = post(port, GW1200).await;
    assert!(answer.starts_with("HTTP/1.1 403"), "{answer}");
    assert_eq!(
        hub.state(&point("outdoor", "temperature")).unwrap().value,
        f(35.1)
    );
    // A command: read-only.
    let refused = hub
        .command(
            &point("outdoor", "temperature"),
            f(1.0),
            moli_core::Origin::Api,
            None,
        )
        .await;
    assert!(refused.is_err());
    cancel.cancel();
}
