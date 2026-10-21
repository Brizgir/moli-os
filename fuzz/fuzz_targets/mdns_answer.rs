//! An mDNS answer: any machine of the network can send one.
#![no_main]

use std::net::Ipv4Addr;

use libfuzzer_sys::fuzz_target;
use moli_net::mdns::answer;

fuzz_target!(|data: &[u8]| {
    let from = Ipv4Addr::new(192, 168, 1, 20);
    let _ = answer(data, from, &["_hue._tcp", "_esphomelib._tcp", "_matterc._udp"]);
});
