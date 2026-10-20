//! What the home network announces, and which integration each thing is
//! (docs/ARCHITECTURE.md, D4).
//!
//! One mDNS question for every service type the catalogue declares (plus a
//! few known ones Moli does not drive yet: Matter, HomeKit…), and one SSDP
//! search per precise target. Nothing runs until asked: no task, no socket
//! left open. The network is asked at most once a minute, whoever asks; the
//! others get that answer.
//!
//! Only private IPv4 addresses, each the device's own (moli-net checks it).
//! A result is a suggestion for a person or an agent: nothing is configured
//! from it, and an agent never chooses a URL (D5).

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use moli_net::{mdns, upnp};
use serde::Serialize;
use tokio::sync::Mutex;

/// `(integration, mDNS types, SSDP targets)`, from `integrations/*/integration.toml`.
pub const CATALOGUE: &[(&str, &[&str], &[&str])] =
    include!(concat!(env!("OUT_DIR"), "/catalogue.rs"));

/// Seen in houses, not driven by Moli yet: worth telling a person about.
pub const OTHERS: &[(&str, &str)] = &[
    ("_matter._tcp", "Matter"),
    ("_matterc._udp", "Matter"),
    ("_hap._tcp", "HomeKit"),
    ("_googlecast._tcp", "Google Cast"),
    ("_airplay._tcp", "AirPlay"),
    ("_shelly._tcp", "Shelly"),
    ("_mqtt._tcp", "MQTT"),
    ("_home-assistant._tcp", "Home Assistant"),
];

/// Long enough for a device on Wi-Fi to answer, short enough to wait for.
pub const WAIT: Duration = Duration::from_secs(3);
/// The network is asked at most this often.
const AGAIN_AFTER: Duration = Duration::from_secs(60);

/// One thing found on the network.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Found {
    /// `mdns` or `ssdp`.
    pub via: &'static str,
    /// The mDNS type or the SSDP target it answered.
    pub service: String,
    /// The catalogue's package that drives it, if any.
    pub integration: Option<&'static str>,
    /// What it is, when no package drives it (`Matter`…).
    pub family: Option<&'static str>,
    /// Its name (mDNS) or its description's URL (SSDP).
    pub name: String,
    pub address: Ipv4Addr,
    pub port: Option<u16>,
    /// Its TXT entries (mDNS), filtered.
    pub txt: Vec<String>,
}

/// A scan and when it was made.
#[derive(Clone, Debug, Serialize)]
pub struct Scan {
    /// Seconds since the network was asked.
    pub age_s: u64,
    pub found: Arc<Vec<Found>>,
}

/// The last scan (and the lock that makes callers wait for the one under way).
static LAST: Mutex<Option<(Instant, Arc<Vec<Found>>)>> = Mutex::const_new(None);

/// What the network announces: the last scan if it is less than a minute
/// old, a new one otherwise.
pub async fn scan() -> anyhow::Result<Scan> {
    let mut last = LAST.lock().await;
    if let Some((at, found)) = last.as_ref()
        && at.elapsed() < AGAIN_AFTER
    {
        return Ok(Scan {
            age_s: at.elapsed().as_secs(),
            found: Arc::clone(found),
        });
    }
    let found = Arc::new(ask(WAIT).await?);
    *last = Some((Instant::now(), Arc::clone(&found)));
    Ok(Scan { age_s: 0, found })
}

/// Every mDNS type to ask about: the catalogue's, then the others.
#[must_use]
pub fn mdns_types() -> Vec<&'static str> {
    let mut types: Vec<&str> = CATALOGUE
        .iter()
        .flat_map(|(_, mdns, _)| mdns.iter().copied())
        .chain(OTHERS.iter().map(|(service, _)| *service))
        .collect();
    types.sort_unstable();
    types.dedup();
    types
}

/// The SSDP targets that name a kind of device (`urn:…`): `upnp:rootdevice`
/// and `ssdp:all` answer for everything and say nothing.
#[must_use]
pub fn ssdp_targets() -> Vec<&'static str> {
    let mut targets: Vec<&str> = CATALOGUE
        .iter()
        .flat_map(|(_, _, ssdp)| ssdp.iter().copied())
        .filter(|target| target.starts_with("urn:"))
        .collect();
    targets.sort_unstable();
    targets.dedup();
    targets
}

/// Which package drives `service`, or which family it belongs to.
#[must_use]
pub fn identify(service: &str) -> (Option<&'static str>, Option<&'static str>) {
    let integration = CATALOGUE
        .iter()
        .find(|(_, mdns, ssdp)| mdns.contains(&service) || ssdp.contains(&service))
        .map(|(id, _, _)| *id);
    let family = OTHERS
        .iter()
        .find(|(s, _)| *s == service)
        .map(|(_, family)| *family);
    (integration, family.filter(|_| integration.is_none()))
}

/// Asks the network now (mDNS and SSDP at the same time), `wait` long.
pub async fn ask(wait: Duration) -> anyhow::Result<Vec<Found>> {
    let types = mdns_types();
    let targets = ssdp_targets();
    let searches = targets.iter().map(|target| upnp::discover(target, wait));
    let (services, answers) = tokio::join!(
        mdns::browse(&types, wait),
        futures::future::join_all(searches)
    );
    let mut found: Vec<Found> = services?
        .into_iter()
        .map(|s| {
            let (integration, family) = identify(&s.kind);
            Found {
                via: "mdns",
                service: s.kind,
                integration,
                family,
                name: s.name,
                address: s.address,
                port: s.port,
                txt: s.txt,
            }
        })
        .collect();
    for (target, answer) in targets.iter().zip(answers) {
        // A search that failed (no network yet) leaves the others.
        let Ok(locations) = answer else { continue };
        for location in locations {
            // A device of the home network, its description on itself.
            let Ok(address) = location.host.parse::<Ipv4Addr>() else {
                continue;
            };
            if !address.is_private() {
                continue;
            }
            let (integration, family) = identify(target);
            let thing = Found {
                via: "ssdp",
                service: (*target).to_owned(),
                integration,
                family,
                name: location.to_string(),
                address,
                port: Some(location.port),
                txt: Vec::new(),
            };
            if !found.contains(&thing) {
                found.push(thing);
            }
        }
    }
    found.sort_by(|a, b| (a.address, a.via, &a.service).cmp(&(b.address, b.via, &b.service)));
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalogue_is_compiled_in() {
        let (hue, _, _) = CATALOGUE.iter().find(|(id, _, _)| *id == "hue").unwrap();
        assert_eq!(*hue, "hue");
        assert_eq!(identify("_hue._tcp"), (Some("hue"), None));
        assert_eq!(identify("_esphomelib._tcp"), (Some("esphome"), None));
        assert_eq!(identify("_matterc._udp"), (None, Some("Matter")));
        assert_eq!(identify("_unknown._tcp"), (None, None));
    }

    #[test]
    fn one_question_holds_every_type() {
        let types = mdns_types();
        assert!(
            types.len() <= 32,
            "{} types: more than one question",
            types.len()
        );
        assert!(types.iter().all(|t| mdns::service_type(t)), "{types:?}");
        assert!(mdns::query(&types).is_ok());
    }

    #[test]
    fn only_precise_ssdp_targets_are_searched() {
        let targets = ssdp_targets();
        assert!(targets.contains(&"urn:schemas-upnp-org:device:ZonePlayer:1"));
        assert!(!targets.contains(&"upnp:rootdevice"));
        assert_eq!(
            identify("urn:schemas-upnp-org:device:ZonePlayer:1"),
            (Some("sonos"), None)
        );
    }
}
