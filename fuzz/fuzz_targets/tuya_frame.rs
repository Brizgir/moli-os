//! A TCP stream from a Tuya device, cut into frames.
#![no_main]

use libfuzzer_sys::fuzz_target;
use moli_tuya::protocol::{Codec, Version};

fuzz_target!(|data: &[u8]| {
    // First byte: the protocol version. The rest: what the socket gave.
    let Some((&first, stream)) = data.split_first() else {
        return;
    };
    let version = match first % 3 {
        0 => Version::V33,
        1 => Version::V34,
        _ => Version::V35,
    };
    let codec = Codec::new(version, *b"0123456789abcdef");
    let _ = codec.open_json(stream);
    let mut buf = stream.to_vec();
    // Each frame taken shortens the buffer: the loop ends.
    while let Ok(Some(frame)) = codec.decode(&mut buf) {
        let _ = codec.open_json(&frame.payload);
    }
});
