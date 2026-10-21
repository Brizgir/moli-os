//! The neighbour table: its MACs come from answers on the network.
#![no_main]

use libfuzzer_sys::fuzz_target;
use moli_net::lan::{mac_hex, parse_arp};

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let _ = parse_arp(text);
    let _ = mac_hex(text);
});
