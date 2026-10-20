//! A UPnP description or SOAP answer: XML written by the device.
#![no_main]

use libfuzzer_sys::fuzz_target;
use moli_net::upnp::{control_url, tag, unescape};

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    // First line: the name looked for. The rest: the document.
    let (name, xml) = text.split_once('\n').unwrap_or(("", text));
    if let Some(value) = tag(xml, name) {
        let _ = unescape(value);
    }
    let _ = control_url(xml, name);
    let _ = unescape(xml);
});
