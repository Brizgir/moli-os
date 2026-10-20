//! Protobuf messages, as ESPHome and Android TV devices send them.
#![no_main]

use libfuzzer_sys::fuzz_target;
use moli_net::proto::{Fields, parse, read_varint};

/// Reads every field the drivers might ask for, down a few levels.
fn walk(fields: &Fields<'_>, depth: u32) {
    for number in 0..8 {
        let _ = fields.has(number);
        let _ = fields.uint(number);
        let _ = fields.str(number);
        let _ = fields.bool(number);
        let _ = fields.float(number);
        if depth < 4 {
            for inner in fields.all_msgs(number) {
                walk(&inner, depth + 1);
            }
        }
    }
}

fuzz_target!(|data: &[u8]| {
    let mut pos = 0;
    while pos < data.len() && read_varint(data, &mut pos).is_some() {}
    if let Some(fields) = parse(data) {
        walk(&fields, 0);
    }
});
