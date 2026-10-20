//! The catalogue's announcements (`integrations/*/integration.toml`,
//! `[discovery]`) compiled in: the scratch image has no `integrations/`
//! folder, and the manifests stay the only place they are written.

use std::fmt::Write as _;
use std::path::Path;

#[derive(serde::Deserialize)]
struct Manifest {
    integration: Integration,
    #[serde(default)]
    discovery: Discovery,
}

#[derive(serde::Deserialize)]
struct Integration {
    id: String,
}

#[derive(serde::Deserialize, Default)]
struct Discovery {
    #[serde(default)]
    mdns: Vec<String>,
    #[serde(default)]
    ssdp: Vec<String>,
}

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../integrations");
    println!("cargo::rerun-if-changed={}", dir.display());
    let mut packages: Vec<_> = std::fs::read_dir(&dir)
        .expect("integrations/")
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("integration.toml"))
        .filter(|path| path.is_file())
        .collect();
    packages.sort();
    let mut out = String::from("&[\n");
    for path in packages {
        println!("cargo::rerun-if-changed={}", path.display());
        let text = std::fs::read_to_string(&path).expect("readable manifest");
        let manifest: Manifest =
            toml::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let d = manifest.discovery;
        if d.mdns.is_empty() && d.ssdp.is_empty() {
            continue;
        }
        writeln!(
            out,
            "    ({:?}, &{:?}, &{:?}),",
            manifest.integration.id, d.mdns, d.ssdp
        )
        .expect("a String");
    }
    out.push(']');
    let target = Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("catalogue.rs");
    std::fs::write(target, out).expect("OUT_DIR is writable");
}
