//! A Tuya discovery broadcast (UDP 6666/6667/7000), from anyone.
#![no_main]

use libfuzzer_sys::fuzz_target;
use moli_tuya::protocol::open_broadcast;

fuzz_target!(|data: &[u8]| {
    let _ = open_broadcast(data);
});
