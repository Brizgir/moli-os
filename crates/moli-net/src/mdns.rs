//! Who announces what on the home network (mDNS / DNS-SD, RFC 6762 and 6763).
//!
//! One question, asked once: the PTR records of a few service types
//! (`_hue._tcp`…), sent to the mDNS group from an ordinary port. Responders
//! answer such a « legacy » question by unicast to that port (RFC 6762
//! § 6.7): nothing listens on 5353, nothing stays open after the wait.
//!
//! A service is kept only from a private IPv4 address, and only when its
//! address is the one that answered: like SSDP, nobody points Moli at
//! another machine. Names and TXT entries are bounded and filtered: they
//! end up in front of a person and of an agent.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;

use tokio::net::UdpSocket;

const GROUP: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(224, 0, 0, 251), 5353);
const TYPE_A: u16 = 1;
const TYPE_PTR: u16 = 12;
const TYPE_TXT: u16 = 16;
const TYPE_SRV: u16 = 33;
const CLASS_IN: u16 = 1;
/// Top bit of the class: « cache flush » in answers, « unicast » in questions.
const CLASS_MASK: u16 = 0x7fff;
/// A DNS name (RFC 1035) and the compression pointers followed to read one.
const MAX_NAME: usize = 255;
const MAX_JUMPS: usize = 16;
/// At most this many types in one question, records in one answer, services
/// kept: a home network, not the Internet.
const MAX_TYPES: usize = 32;
const MAX_RECORDS: usize = 128;
const MAX_SERVICES: usize = 256;
const MAX_TEXT: usize = 64;
const MAX_TXT_ENTRIES: usize = 16;

/// A service found on the network.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Service {
    /// The type asked for, as asked (`_hue._tcp`).
    pub kind: String,
    /// The instance's name (`Hue Bridge - 1A2B3C`), filtered.
    pub name: String,
    pub address: Ipv4Addr,
    /// From its SRV record, when it came with the answer.
    pub port: Option<u16>,
    /// Its TXT entries (`key=value`), filtered.
    pub txt: Vec<String>,
}

/// The services of `kinds` (`_hue._tcp`…) that answer within `wait`.
pub async fn browse(kinds: &[&str], wait: Duration) -> anyhow::Result<Vec<Service>> {
    let query = query(kinds)?;
    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    socket.set_multicast_ttl_v4(255)?;
    socket.send_to(&query, GROUP).await?;
    let mut found: Vec<Service> = Vec::new();
    let mut buf = vec![0u8; 9000];
    let deadline = tokio::time::Instant::now() + wait;
    // Asked again once: one datagram lost on Wi-Fi is not a missing device.
    let mut again = Some(tokio::time::Instant::now() + Duration::from_millis(400));
    loop {
        let until = again.map_or(deadline, |at| at.min(deadline));
        match tokio::time::timeout_at(until, socket.recv_from(&mut buf)).await {
            Err(_) if again.is_some() && until < deadline => {
                again = None;
                socket.send_to(&query, GROUP).await?;
            }
            Err(_) => break,
            Ok(Err(e)) => {
                tracing::debug!(error = %e, "mDNS answer not read");
                break;
            }
            Ok(Ok((n, from))) => {
                let std::net::IpAddr::V4(from) = from.ip() else {
                    continue;
                };
                for service in answer(&buf[..n], from, kinds) {
                    if found.len() < MAX_SERVICES && !found.contains(&service) {
                        found.push(service);
                    }
                }
            }
        }
    }
    found.sort_by(|a, b| (a.address, &a.kind, &a.name).cmp(&(b.address, &b.kind, &b.name)));
    Ok(found)
}

/// One question for the PTR records of `kinds`.
pub fn query(kinds: &[&str]) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(
        !kinds.is_empty() && kinds.len() <= MAX_TYPES,
        "1 to {MAX_TYPES} service types"
    );
    let count = u16::try_from(kinds.len())?;
    // Id 0, a standard query, no recursion; then the questions.
    let mut out = vec![0, 0, 0, 0];
    out.extend_from_slice(&count.to_be_bytes());
    out.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    for kind in kinds {
        anyhow::ensure!(service_type(kind), "{kind:?}: a service like _hue._tcp");
        for label in kind.split('.').chain(["local"]) {
            out.push(u8::try_from(label.len())?);
            out.extend_from_slice(label.as_bytes());
        }
        out.push(0);
        out.extend_from_slice(&TYPE_PTR.to_be_bytes());
        out.extend_from_slice(&CLASS_IN.to_be_bytes());
    }
    Ok(out)
}

/// `_name._tcp` or `_name._udp`, name of letters, digits and `-`. Up to a
/// DNS label (63), not RFC 6335's 15: devices announce longer ones
/// (`_androidtvremote2._tcp`).
#[must_use]
pub fn service_type(kind: &str) -> bool {
    let Some((name, proto)) = kind.split_once('.') else {
        return false;
    };
    let Some(name) = name.strip_prefix('_') else {
        return false;
    };
    (1..=62).contains(&name.len())
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && matches!(proto, "_tcp" | "_udp")
}

/// One record of an answer.
#[derive(Debug)]
struct Record {
    name: Vec<String>,
    data: Data,
}

#[derive(Debug)]
enum Data {
    Ptr(Vec<String>),
    Srv { port: u16, target: Vec<String> },
    Txt(Vec<String>),
    A(Ipv4Addr),
    Other,
}

/// The services of `kinds` in one datagram from `from`.
#[must_use]
pub fn answer(datagram: &[u8], from: Ipv4Addr, kinds: &[&str]) -> Vec<Service> {
    if !from.is_private() {
        return Vec::new();
    }
    let Some(records) = records(datagram) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for record in &records {
        let Data::Ptr(instance) = &record.data else {
            continue;
        };
        let Some(kind) = kinds.iter().find(|k| same(&record.name, &labels_of(k))) else {
            continue;
        };
        // `<instance>.<type>.local`: the instance is the first label.
        if instance.len() != 4 || !same(&instance[1..], &record.name) {
            continue;
        }
        let srv = records.iter().find_map(|r| match &r.data {
            Data::Srv { port, target } if same(&r.name, instance) => Some((*port, target)),
            _ => None,
        });
        // Its host's address, when given, must be the one that answered.
        let address = srv.and_then(|(_, target)| {
            records.iter().find_map(|r| match r.data {
                Data::A(ip) if same(&r.name, target) => Some(ip),
                _ => None,
            })
        });
        if address.is_some_and(|ip| ip != from) {
            continue;
        }
        let txt = records
            .iter()
            .find_map(|r| match &r.data {
                Data::Txt(entries) if same(&r.name, instance) => Some(entries.clone()),
                _ => None,
            })
            .unwrap_or_default();
        found.push(Service {
            kind: (*kind).to_owned(),
            name: clean(&instance[0]),
            address: from,
            port: srv.map(|(port, _)| port),
            txt,
        });
    }
    found
}

fn labels_of(kind: &str) -> Vec<String> {
    kind.split('.')
        .chain(["local"])
        .map(str::to_owned)
        .collect()
}

/// DNS names compare without regard to ASCII case.
fn same(a: &[String], b: &[String]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.eq_ignore_ascii_case(y))
}

/// Printable, on one line, at most `MAX_TEXT` characters.
fn clean(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(MAX_TEXT)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// Every record of a response (answers, authority, additional), or `None`
/// for anything that is not a well-formed response.
fn records(msg: &[u8]) -> Option<Vec<Record>> {
    let word =
        |at: usize| -> Option<u16> { Some(u16::from_be_bytes([*msg.get(at)?, *msg.get(at + 1)?])) };
    // A response (QR set), no error.
    let flags = word(2)?;
    if flags & 0x8000 == 0 || flags & 0x000f != 0 {
        return None;
    }
    let questions = usize::from(word(4)?);
    let count = usize::from(word(6)?) + usize::from(word(8)?) + usize::from(word(10)?);
    let mut at = 12;
    for _ in 0..questions.min(MAX_TYPES) {
        at = name(msg, at)?.1 + 4;
    }
    let mut out = Vec::new();
    for _ in 0..count.min(MAX_RECORDS) {
        let (owner, next) = name(msg, at)?;
        let kind = word(next)?;
        let class = word(next + 2)? & CLASS_MASK;
        let length = usize::from(word(next + 8)?);
        let start = next + 10;
        let end = start.checked_add(length)?;
        let rdata = msg.get(start..end)?;
        let data = match kind {
            _ if class != CLASS_IN => Data::Other,
            TYPE_PTR => Data::Ptr(name(msg, start)?.0),
            TYPE_SRV if length >= 7 => Data::Srv {
                port: u16::from_be_bytes([rdata[4], rdata[5]]),
                target: name(msg, start + 6)?.0,
            },
            TYPE_TXT => Data::Txt(txt(rdata)),
            TYPE_A if length == 4 => Data::A(Ipv4Addr::new(rdata[0], rdata[1], rdata[2], rdata[3])),
            _ => Data::Other,
        };
        out.push(Record { name: owner, data });
        at = end;
    }
    Some(out)
}

/// The name at `at` and where the record goes on after it. A compression
/// pointer only goes backwards, at most `MAX_JUMPS` times: no loop, no
/// reading past the datagram.
fn name(msg: &[u8], mut at: usize) -> Option<(Vec<String>, usize)> {
    let mut labels = Vec::new();
    let mut size = 0;
    let mut after = None;
    let mut jumps = 0;
    loop {
        let length = *msg.get(at)?;
        match length {
            0 => return Some((labels, after.unwrap_or(at + 1))),
            1..=63 => {
                let label = msg.get(at + 1..at + 1 + usize::from(length))?;
                size += label.len() + 1;
                if size > MAX_NAME {
                    return None;
                }
                labels.push(String::from_utf8_lossy(label).into_owned());
                at += 1 + usize::from(length);
            }
            0xc0..=0xff => {
                let target = usize::from(u16::from_be_bytes([length, *msg.get(at + 1)?]) & 0x3fff);
                jumps += 1;
                if target >= at || jumps > MAX_JUMPS {
                    return None;
                }
                after.get_or_insert(at + 2);
                at = target;
            }
            _ => return None,
        }
    }
}

/// TXT strings, filtered, at most `MAX_TXT_ENTRIES`.
fn txt(rdata: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(&length) = rdata.get(at) {
        let Some(entry) = rdata.get(at + 1..at + 1 + usize::from(length)) else {
            break;
        };
        let entry = clean(&String::from_utf8_lossy(entry));
        if !entry.is_empty() && out.len() < MAX_TXT_ENTRIES {
            out.push(entry);
        }
        at += 1 + usize::from(length);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FROM: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 20);

    fn push_name(out: &mut Vec<u8>, name: &str) {
        for label in name.split('.') {
            out.push(u8::try_from(label.len()).unwrap());
            out.extend_from_slice(label.as_bytes());
        }
        out.push(0);
    }

    fn push_record(out: &mut Vec<u8>, name: &str, kind: u16, rdata: &[u8]) {
        push_name(out, name);
        out.extend_from_slice(&kind.to_be_bytes());
        out.extend_from_slice(&(0x8000 | CLASS_IN).to_be_bytes());
        out.extend_from_slice(&120u32.to_be_bytes());
        out.extend_from_slice(&u16::try_from(rdata.len()).unwrap().to_be_bytes());
        out.extend_from_slice(rdata);
    }

    /// A Hue bridge's answer: PTR, SRV, TXT and A, no compression.
    fn hue(address: [u8; 4]) -> Vec<u8> {
        let mut msg = vec![0, 0, 0x84, 0, 0, 0, 0, 1, 0, 0, 0, 3];
        let mut ptr = Vec::new();
        push_name(&mut ptr, "Hue Bridge - 1A2B3C._hue._tcp.local");
        push_record(&mut msg, "_hue._tcp.local", TYPE_PTR, &ptr);
        let mut srv = vec![0, 0, 0, 0, 0x01, 0xbb];
        push_name(&mut srv, "hue-bridge.local");
        push_record(
            &mut msg,
            "Hue Bridge - 1A2B3C._hue._tcp.local",
            TYPE_SRV,
            &srv,
        );
        let txt = b"\x0dbridgeid=abcd\x0emodelid=BSB002";
        push_record(
            &mut msg,
            "Hue Bridge - 1A2B3C._hue._tcp.local",
            TYPE_TXT,
            txt,
        );
        push_record(&mut msg, "hue-bridge.local", TYPE_A, &address);
        msg
    }

    #[test]
    fn a_question_asks_each_type_as_ptr() {
        let q = query(&["_hue._tcp", "_ipp._tcp"]).unwrap();
        assert_eq!(&q[..6], &[0, 0, 0, 0, 0, 2]);
        assert_eq!(&q[12..17], b"\x04_hue");
        assert!(query(&["hue"]).is_err());
        assert!(query(&[]).is_err());
        assert!(query(&["_x._tcp.evil"]).is_err());
    }

    #[test]
    fn service_types_are_checked() {
        assert!(service_type("_esphomelib._tcp"));
        assert!(service_type("_matterc._udp"));
        assert!(!service_type("_hue._http"));
        assert!(!service_type("hue._tcp"));
        assert!(!service_type("_._tcp"));
        assert!(!service_type("_a.b._tcp"));
        assert!(service_type("_androidtvremote2._tcp"));
        assert!(!service_type(&format!("_{}._tcp", "a".repeat(63))));
    }

    #[test]
    fn a_bridge_is_found_with_its_port_and_txt() {
        let found = answer(&hue([192, 168, 1, 20]), FROM, &["_hue._tcp"]);
        assert_eq!(
            found,
            vec![Service {
                kind: "_hue._tcp".into(),
                name: "Hue Bridge - 1A2B3C".into(),
                address: FROM,
                port: Some(443),
                txt: vec!["bridgeid=abcd".into(), "modelid=BSB002".into()],
            }]
        );
    }

    #[test]
    fn an_address_elsewhere_or_public_is_refused() {
        assert!(answer(&hue([192, 168, 1, 99]), FROM, &["_hue._tcp"]).is_empty());
        let public = Ipv4Addr::new(8, 8, 8, 8);
        assert!(answer(&hue([8, 8, 8, 8]), public, &["_hue._tcp"]).is_empty());
    }

    #[test]
    fn a_type_not_asked_for_is_ignored() {
        assert!(answer(&hue([192, 168, 1, 20]), FROM, &["_ipp._tcp"]).is_empty());
    }

    #[test]
    fn compression_pointers_are_followed_backwards_only() {
        let mut msg = vec![0, 0, 0x84, 0, 0, 0, 0, 1, 0, 0, 0, 0];
        // _hue._tcp.local at 12; the PTR's data points back to it.
        push_name(&mut msg, "_hue._tcp.local");
        let owner = 12u16;
        msg.extend_from_slice(&TYPE_PTR.to_be_bytes());
        msg.extend_from_slice(&CLASS_IN.to_be_bytes());
        msg.extend_from_slice(&120u32.to_be_bytes());
        msg.extend_from_slice(&6u16.to_be_bytes());
        msg.extend_from_slice(b"\x03Hue");
        msg.extend_from_slice(&(0xc000 | owner).to_be_bytes());
        let found = answer(&msg, FROM, &["_hue._tcp"]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "Hue");
        assert_eq!(found[0].port, None);

        // A pointer to itself: refused, not a loop.
        let looping = [0, 0, 0x84, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0xc0, 12];
        assert!(answer(&looping, FROM, &["_hue._tcp"]).is_empty());
    }

    #[test]
    fn truncated_or_odd_datagrams_give_nothing() {
        let full = hue([192, 168, 1, 20]);
        for end in 0..full.len() {
            let _ = answer(&full[..end], FROM, &["_hue._tcp"]);
        }
        // A question, not an answer.
        let mut question = full.clone();
        question[2] = 0;
        assert!(answer(&question, FROM, &["_hue._tcp"]).is_empty());
    }

    #[test]
    fn names_are_filtered_and_bounded() {
        assert_eq!(clean(" Salon\u{7}\n "), "Salon");
        assert_eq!(clean(&"x".repeat(200)).len(), MAX_TEXT);
    }
}
