//! Photos of what the cameras saw: when a camera's doorbell is pressed or
//! it sees motion (a person, a car…), Moli asks it for an image and keeps
//! it in the house, `days` days.
//!
//! - Taken at the start of the event (a point going on), one at a time per
//!   camera. A camera that answers with an older image (a Ring on battery
//!   gives none while it records) is asked again a little later, a few
//!   times; an image older than the event is never kept as its photo.
//! - Never in a protected room: nobody keeps pictures of a bedroom.
//! - `GET /api/captures?device=<id>&days=<n>`: the photos kept, newest first.
//!   `GET /api/captures/{device}/{name}`: one of them. The same rules as a
//!   live image: no guest, a human for a camera now in a protected room.
//!
//! Files: `<data>/captures/<device id in hex>/<event ms>-<point>.<jpg|png>`.

use std::collections::{HashMap, HashSet};
use std::path::{Path as FsPath, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use moli_core::{DeviceId, Event, Value};
use moli_runtime::Hub;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;
use tokio_util::sync::CancellationToken;

use crate::caller::Caller;
use crate::session::Sessions;

/// What a camera tells that is worth a photo.
pub const KEYS: &[&str] = &[
    "doorbell", "motion", "person", "vehicle", "animal", "package",
];
/// When to ask, from the event: at once, then again past a battery
/// camera's pause between two images (a minute for a Ring).
const TRIES: [Duration; 3] = [
    Duration::ZERO,
    Duration::from_secs(70),
    Duration::from_secs(140),
];
/// An image this much older than the event still shows it (clocks differ).
const SLACK_MS: i64 = 10_000;
/// At most this many photos per camera, the newest kept.
const MAX_PER_CAMERA: usize = 2_000;
/// The old ones are removed this often.
const PRUNE: Duration = Duration::from_secs(60 * 60);
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// The data directory: the photos are in its `captures/`.
#[derive(Clone)]
pub(crate) struct CapturesDir(pub(crate) Option<PathBuf>);

/// Keeps a photo of each event of the house's cameras until `cancel`, and
/// removes those older than `days` days.
pub fn start(
    hub: &Hub,
    data_dir: &FsPath,
    days: u32,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    let root = data_dir.join("captures");
    let mut events = hub.subscribe();
    let hub = hub.clone();
    tokio::spawn(async move {
        let busy: Arc<Mutex<HashSet<DeviceId>>> = Arc::default();
        // Whether each point was on, to see it go on.
        let mut on: HashMap<moli_core::PointId, bool> = HashMap::new();
        let mut prune = tokio::time::interval(PRUNE);
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                _ = prune.tick() => {
                    let root = root.clone();
                    let _ = tokio::task::spawn_blocking(move || prune_old(&root, days, now_ms())).await;
                }
                event = events.recv() => match event {
                    Ok(Event::State { point, value, ts }) => {
                        let Some((device, key)) = point.split() else { continue };
                        if !KEYS.contains(&key) {
                            continue;
                        }
                        let now = is_on(&value);
                        let was = on.insert(point.clone(), now).unwrap_or(false);
                        if now && !was {
                            let key = key.to_owned();
                            event_started(&hub, &root, &busy, device, key, ts, &cancel);
                        }
                    }
                    Ok(_) => {}
                    // Missed changes: only the next ones get a photo.
                    Err(RecvError::Lagged(missed)) => {
                        tracing::debug!(missed, "captures lagged");
                    }
                    Err(RecvError::Closed) => break,
                },
            }
        }
    })
}

fn is_on(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::Int(n) => *n > 0,
        Value::Float(n) => *n > 0.0,
        _ => false,
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// A camera's event began: its photo, unless one is being taken.
fn event_started(
    hub: &Hub,
    root: &FsPath,
    busy: &Arc<Mutex<HashSet<DeviceId>>>,
    device: DeviceId,
    key: String,
    ts: u64,
    cancel: &CancellationToken,
) {
    if !hub.device(&device).is_some_and(|d| d.camera) || hub.protected_room(&device).is_some() {
        return;
    }
    if !busy
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(device.clone())
    {
        return;
    }
    let (hub, busy, cancel) = (hub.clone(), Arc::clone(busy), cancel.clone());
    let dir = root.join(dir_name(&device));
    let at = i64::try_from(ts).unwrap_or(i64::MAX);
    tokio::spawn(async move {
        tokio::select! {
            () = cancel.cancelled() => {}
            () = capture(&hub, &dir, &device, &key, at) => {}
        }
        busy.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&device);
    });
}

async fn capture(hub: &Hub, dir: &FsPath, device: &DeviceId, key: &str, at: i64) {
    let mut waited = Duration::ZERO;
    for wait in TRIES {
        tokio::time::sleep(wait.saturating_sub(waited)).await;
        waited = wait;
        let image = match hub.camera_image(device).await {
            Ok(image) => image,
            Err(e) => {
                tracing::debug!(%device, error = %e, "capture: no image");
                continue;
            }
        };
        if image.taken.is_some_and(|t| t + SLACK_MS < at) {
            tracing::debug!(%device, "capture: the image is older than the event");
            continue;
        }
        let extension = match image.content_type.as_str() {
            "image/jpeg" => "jpg",
            "image/png" => "png",
            _ => return,
        };
        let path = dir.join(format!("{at}-{key}.{extension}"));
        let written = async {
            tokio::fs::create_dir_all(dir).await?;
            tokio::fs::write(&path, &image.bytes).await
        };
        match written.await {
            Ok(()) => tracing::info!(%device, key, "capture kept"),
            Err(e) => tracing::warn!(%device, error = %e, "capture not written"),
        }
        return;
    }
    tracing::info!(%device, key, "capture: no image of the event");
}

/// A device's folder: its id in hex (any id, any file system).
fn dir_name(device: &DeviceId) -> String {
    use std::fmt::Write as _;
    device.as_str().bytes().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// A photo's name: `<ms>-<point>.<jpg|png>`, nothing else.
fn parse_name(name: &str) -> Option<(i64, &'static str, &'static str)> {
    let (stem, extension) = name.rsplit_once('.')?;
    let kind = match extension {
        "jpg" => "image/jpeg",
        "png" => "image/png",
        _ => return None,
    };
    let (ts, key) = stem.split_once('-')?;
    if ts.is_empty() || !ts.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let key = KEYS.iter().find(|k| **k == key)?;
    Some((ts.parse().ok()?, key, kind))
}

/// Removes the photos older than `days` days, and past the most kept per
/// camera; then the empty folders.
fn prune_old(root: &FsPath, days: u32, now: i64) {
    let Ok(cameras) = std::fs::read_dir(root) else {
        return;
    };
    let oldest = now - i64::from(days) * DAY_MS;
    for camera in cameras.flatten() {
        let dir = camera.path();
        let Ok(files) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut kept: Vec<(i64, PathBuf)> = Vec::new();
        for file in files.flatten() {
            let path = file.path();
            let name = file.file_name();
            match name.to_str().and_then(parse_name) {
                Some((ts, _, _)) if ts >= oldest => kept.push((ts, path)),
                _ => {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
        kept.sort_by_key(|(ts, _)| std::cmp::Reverse(*ts));
        for (_, path) in kept.iter().skip(MAX_PER_CAMERA) {
            let _ = std::fs::remove_file(path);
        }
        if kept.is_empty() {
            let _ = std::fs::remove_dir(&dir);
        }
    }
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

/// A guest sees no camera; a camera now in a protected room shows itself
/// to a human only (as its live image).
fn refused(
    hub: &Hub,
    humans: &Sessions,
    caller: &Caller,
    headers: &HeaderMap,
    device: &DeviceId,
) -> Option<Response> {
    if caller
        .person
        .as_ref()
        .is_some_and(crate::people::PersonRef::is_guest)
    {
        return Some(error(
            StatusCode::FORBIDDEN,
            &moli_i18n::tr!("serveur.personnes.invite_limite"),
        ));
    }
    if let Some(room) = hub.protected_room(device)
        && !humans.is_human(headers, &caller.key)
    {
        return Some(error(
            StatusCode::FORBIDDEN,
            &moli_i18n::tr!("serveur.camera.piece_protegee", room = room),
        ));
    }
    None
}

#[derive(Deserialize)]
pub(crate) struct ListQuery {
    device: String,
    /// The last days (7 by default).
    #[serde(default)]
    days: Option<u32>,
}

#[derive(Serialize)]
struct Kept {
    ts: i64,
    key: &'static str,
    name: String,
}

/// The photos kept of one camera, newest first.
pub(crate) async fn list(
    State(hub): State<Hub>,
    Extension(humans): Extension<Arc<Sessions>>,
    Extension(CapturesDir(dir)): Extension<CapturesDir>,
    caller: Caller,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> Response {
    let device = DeviceId::from(q.device);
    if let Some(refusal) = refused(&hub, &humans, &caller, &headers, &device) {
        return refusal;
    }
    let Some(dir) = dir else {
        return Json(Vec::<Kept>::new()).into_response();
    };
    let folder = dir.join("captures").join(dir_name(&device));
    let since = now_ms() - i64::from(q.days.unwrap_or(7).clamp(1, 366)) * DAY_MS;
    let read = tokio::task::spawn_blocking(move || {
        let mut kept: Vec<Kept> = std::fs::read_dir(folder)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|f| {
                let name = f.file_name().into_string().ok()?;
                let (ts, key, _) = parse_name(&name)?;
                (ts >= since).then_some(Kept { ts, key, name })
            })
            .collect();
        kept.sort_by_key(|k| std::cmp::Reverse(k.ts));
        kept.truncate(500);
        kept
    })
    .await
    .unwrap_or_default();
    Json(read).into_response()
}

/// One photo.
pub(crate) async fn image(
    State(hub): State<Hub>,
    Extension(humans): Extension<Arc<Sessions>>,
    Extension(CapturesDir(dir)): Extension<CapturesDir>,
    caller: Caller,
    headers: HeaderMap,
    Path((device, name)): Path<(String, String)>,
) -> Response {
    let device = DeviceId::from(device);
    if let Some(refusal) = refused(&hub, &humans, &caller, &headers, &device) {
        return refusal;
    }
    let (Some(dir), Some((_, _, kind))) = (dir, parse_name(&name)) else {
        return error(
            StatusCode::NOT_FOUND,
            &moli_i18n::tr!("serveur.camera.photo_introuvable"),
        );
    };
    let path = dir.join("captures").join(dir_name(&device)).join(&name);
    match tokio::fs::read(path).await {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, kind),
                // Never changes, but only for those allowed to see it.
                (header::CACHE_CONTROL, "private, max-age=86400"),
            ],
            bytes,
        )
            .into_response(),
        Err(_) => error(
            StatusCode::NOT_FOUND,
            &moli_i18n::tr!("serveur.camera.photo_introuvable"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_strict() {
        assert_eq!(
            parse_name("1792336979000-motion.jpg"),
            Some((1_792_336_979_000, "motion", "image/jpeg"))
        );
        assert_eq!(
            parse_name("12-doorbell.png"),
            Some((12, "doorbell", "image/png"))
        );
        for bad in [
            "../12-motion.jpg",
            "12-motion.svg",
            "-motion.jpg",
            "12-other.jpg",
            "12x-motion.jpg",
            "12-motion",
        ] {
            assert_eq!(parse_name(bad), None, "{bad}");
        }
    }

    #[test]
    fn folders_are_hex() {
        assert_eq!(dir_name(&DeviceId::from("r:1")), "723a31");
    }

    #[test]
    fn old_photos_go() {
        let root = std::env::temp_dir().join(format!("moli-captures-{}", std::process::id()));
        let camera = root.join("723a31");
        std::fs::create_dir_all(&camera).unwrap();
        let now = 100 * DAY_MS;
        let old = format!("{}-motion.jpg", now - 8 * DAY_MS);
        let young = format!("{}-doorbell.jpg", now - DAY_MS);
        for name in [&old, &young, &"stray.txt".to_owned()] {
            std::fs::write(camera.join(name), b"x").unwrap();
        }
        prune_old(&root, 7, now);
        assert!(!camera.join(&old).exists());
        assert!(!camera.join("stray.txt").exists());
        assert!(camera.join(&young).exists());
        std::fs::remove_file(camera.join(&young)).unwrap();
        prune_old(&root, 7, now);
        assert!(!camera.exists(), "an empty folder goes");
        let _ = std::fs::remove_dir_all(&root);
    }
}
