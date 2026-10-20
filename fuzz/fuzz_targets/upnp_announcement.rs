//! An SSDP datagram from any device of the network.
#![no_main]

use std::net::Ipv4Addr;

use libfuzzer_sys::fuzz_target;
use moli_net::upnp::{Location, announcement};

fuzz_target!(|data: &[u8]| {
    // The listener reads datagrams lossily, as here.
    let text = String::from_utf8_lossy(data);
    let _ = announcement(&text, Ipv4Addr::new(192, 168, 1, 10));
    let _ = Location::parse(&text);
});
