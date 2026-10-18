//! `/api/home/rooms`: the rooms a person declares from the dashboard, kept
//! in `data/home.json` (`rooms[]`: `name`, `icon`, `aliases`) beside what
//! the file already holds (favourites, hidden devices…), which stays as it
//! is.
//!
//! - `POST /api/home/rooms`: a new room (a person: PIN session).
//! - `PUT /api/home/rooms/{name}`: rename it, change its icon, its other
//!   names; a room only the devices named so far is declared on the way. A
//!   rename moves the devices labelled in it (`labels.toml`) and keeps the
//!   old name as an alias, so what a bridge calls the room still lands in it.
//! - `DELETE /api/home/rooms/{name}`: forget the declaration (the devices
//!   keep their labels).
//!
//! Every write is atomic (a unique temporary name, then a rename) and tells
//! the dashboards (`home_changed` on the event stream). Whether a room is
//! protected is `[guard] protected_rooms` in `moli.toml`, edited by hand:
//! nothing here changes it.

use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;

use axum::Extension;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use moli_core::Origin;
use moli_runtime::Hub;
use moli_runtime::guard::same_room;
use serde::Deserialize;
use serde_json::{Map, Value as Json, json};

use crate::caller::Caller;
use crate::session::Sessions;

/// `data/home.json`, and the lock that serialises its writers.
#[derive(Clone)]
pub(crate) struct HomeStore {
    path: Option<PathBuf>,
    lock: Arc<tokio::sync::Mutex<()>>,
}

impl HomeStore {
    pub(crate) fn new(path: Option<PathBuf>) -> Self {
        Self {
            path,
            lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }
}

/// A house has rooms, not a hotel.
pub(crate) const MAX_ROOMS: usize = 100;
/// A room's name, or one of its other names (as `MAX_LABEL` for a label).
pub(crate) const MAX_NAME: usize = 60;
const MAX_ALIASES: usize = 10;

/// The icons a room may take: each one is in `web/src/maison/lib/icons.js`.
pub(crate) const ROOM_ICONS: &[&str] = &[
    "home",
    "sofa-outline",
    "bed-outline",
    "bed-king-outline",
    "silverware-fork-knife",
    "shower-head",
    "toilet",
    "desk-lamp",
    "computer",
    "book",
    "tv",
    "speaker",
    "stairs",
    "door",
    "gate",
    "car",
    "package",
    "flower-outline",
    "leaf",
    "palm",
    "pool",
    "pine",
    "sun",
    "home-roof",
    "dog",
    "fan",
    "water",
    "printer",
];

/// What a person says about a room. `None` keeps what the file has.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RoomPatch {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub aliases: Option<Vec<String>>,
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, axum::Json(json!({ "error": message }))).into_response()
}

/// One line, single spaces, no control character; empty when nothing is left.
fn clean(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// A name a person typed: cleaned, present, not too long.
fn checked_name(text: &str) -> Result<String, String> {
    let name = clean(text);
    if name.is_empty() {
        return Err(moli_i18n::tr!("serveur.maison.nom_vide"));
    }
    if name.chars().count() > MAX_NAME {
        return Err(moli_i18n::tr!(
            "serveur.maison.nom_trop_long",
            max = MAX_NAME
        ));
    }
    Ok(name)
}

/// The names (name and aliases) a room object answers to.
fn names_of(room: &Map<String, Json>) -> Vec<&str> {
    room.get("name")
        .and_then(Json::as_str)
        .into_iter()
        .chain(
            room.get("aliases")
                .and_then(Json::as_array)
                .into_iter()
                .flatten()
                .filter_map(Json::as_str),
        )
        .collect()
}

/// The position of the room called `name` (any spelling) in `rooms`.
fn find(rooms: &[Json], name: &str) -> Option<usize> {
    rooms.iter().position(|r| {
        r.as_object()
            .is_some_and(|r| names_of(r).iter().any(|n| same_room(n, name)))
    })
}

/// `patch` applied to `room` (a fresh object for a new room); what the
/// other rooms are called refuses a name already taken.
fn apply(room: &mut Map<String, Json>, patch: RoomPatch, others: &[Json]) -> Result<(), String> {
    if let Some(name) = &patch.name {
        room.insert("name".into(), Json::String(checked_name(name)?));
    }
    let name = room.get("name").and_then(Json::as_str).map_or_else(
        || Err(moli_i18n::tr!("serveur.maison.nom_vide")),
        checked_name,
    )?;
    match patch.icon.as_deref().map(str::trim) {
        None => {}
        Some("") => {
            room.remove("icon");
        }
        Some(icon) if ROOM_ICONS.contains(&icon) => {
            room.insert("icon".into(), Json::String(icon.to_owned()));
        }
        Some(icon) => return Err(moli_i18n::tr!("serveur.maison.icone_inconnue", icon = icon)),
    }
    if let Some(aliases) = patch.aliases {
        let mut kept: Vec<String> = Vec::new();
        for alias in &aliases {
            let alias = clean(alias);
            if alias.is_empty() {
                continue;
            }
            if alias.chars().count() > MAX_NAME {
                return Err(moli_i18n::tr!(
                    "serveur.maison.nom_trop_long",
                    max = MAX_NAME
                ));
            }
            if same_room(&alias, &name) || kept.iter().any(|k| same_room(k, &alias)) {
                continue;
            }
            kept.push(alias);
        }
        if kept.len() > MAX_ALIASES {
            return Err(moli_i18n::tr!(
                "serveur.maison.trop_d_alias",
                max = MAX_ALIASES
            ));
        }
        if kept.is_empty() {
            room.remove("aliases");
        } else {
            room.insert(
                "aliases".into(),
                Json::Array(kept.into_iter().map(Json::String).collect()),
            );
        }
    }
    for mine in names_of(room) {
        if let Some(taken) = others
            .iter()
            .filter_map(Json::as_object)
            .find(|o| names_of(o).iter().any(|n| same_room(n, mine)))
        {
            let other = taken.get("name").and_then(Json::as_str).unwrap_or(mine);
            return Err(moli_i18n::tr!(
                "serveur.maison.doublon",
                name = mine,
                other = other
            ));
        }
    }
    Ok(())
}

/// The file's content, an object; `{}` when there is none.
async fn read(path: &FsPath) -> Result<Map<String, Json>, String> {
    match tokio::fs::read(path).await {
        Ok(bytes) => match serde_json::from_slice::<Json>(&bytes) {
            Ok(Json::Object(layout)) => Ok(layout),
            Ok(_) => Err(moli_i18n::tr!(
                "serveur.maison.fichier_illisible",
                error = "not an object"
            )),
            Err(e) => Err(moli_i18n::tr!(
                "serveur.maison.fichier_illisible",
                error = e
            )),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Map::new()),
        Err(e) => Err(e.to_string()),
    }
}

/// Written whole, under a name of its own, then put in place: a crash or
/// two writers never leave half a file.
async fn write(path: &FsPath, layout: &Map<String, Json>) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(layout)? + "\n";
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(
        ".{}.tmp",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    tokio::fs::write(&tmp, text).await?;
    tokio::fs::rename(&tmp, path).await
}

/// What a write changes in the file, besides what the dashboards are told.
#[derive(Debug)]
enum Change {
    Created,
    /// The room called `from` is now called `to`.
    Renamed {
        from: String,
        to: String,
    },
    Changed,
    Removed,
}

/// The rooms of `layout`, each an object (anything else in the list is kept
/// out of the way, as the file had it).
fn rooms_of(layout: &mut Map<String, Json>) -> &mut Vec<Json> {
    if !layout.get("rooms").is_some_and(Json::is_array) {
        layout.insert("rooms".into(), Json::Array(Vec::new()));
    }
    layout
        .get_mut("rooms")
        .and_then(Json::as_array_mut)
        .expect("rooms is an array")
}

/// `patch` on the room called `name` (a new room: `None`), in the file.
fn edit(
    layout: &mut Map<String, Json>,
    name: Option<&str>,
    patch: RoomPatch,
) -> Result<Change, (StatusCode, String)> {
    let rooms = rooms_of(layout);
    let at = name.and_then(|name| find(rooms, name));
    let invalid = |why| (StatusCode::UNPROCESSABLE_ENTITY, why);
    if let Some(at) = at {
        let mut room = rooms[at].as_object().cloned().unwrap_or_default();
        let before = room
            .get("name")
            .and_then(Json::as_str)
            .unwrap_or_default()
            .to_owned();
        let others: Vec<Json> = rooms
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != at)
            .map(|(_, r)| r.clone())
            .collect();
        apply(&mut room, patch, &others).map_err(invalid)?;
        let after = room
            .get("name")
            .and_then(Json::as_str)
            .unwrap_or_default()
            .to_owned();
        let renamed = !same_room(&before, &after);
        if renamed {
            // What a bridge or the plan calls it still lands here.
            let mut aliases: Vec<Json> = room
                .get("aliases")
                .and_then(Json::as_array)
                .cloned()
                .unwrap_or_default();
            if !aliases
                .iter()
                .filter_map(Json::as_str)
                .any(|a| same_room(a, &before))
            {
                aliases.push(Json::String(before.clone()));
            }
            room.insert("aliases".into(), Json::Array(aliases));
        }
        rooms[at] = Json::Object(room);
        Ok(if renamed {
            Change::Renamed {
                from: before,
                to: after,
            }
        } else {
            Change::Changed
        })
    } else {
        if rooms.len() >= MAX_ROOMS {
            return Err(invalid(moli_i18n::tr!(
                "serveur.maison.trop_de_pieces",
                max = MAX_ROOMS
            )));
        }
        let mut room = Map::new();
        let patch = RoomPatch {
            // A room the devices named so far, declared as it is called.
            name: patch.name.or_else(|| name.map(str::to_owned)),
            ..patch
        };
        apply(&mut room, patch, rooms).map_err(invalid)?;
        let renamed = name.filter(|n| {
            room.get("name")
                .and_then(Json::as_str)
                .is_some_and(|after| !same_room(n, after))
        });
        if let Some(from) = renamed {
            room.insert("aliases".into(), {
                let mut aliases = room
                    .get("aliases")
                    .and_then(Json::as_array)
                    .cloned()
                    .unwrap_or_default();
                aliases.push(Json::String(from.to_owned()));
                Json::Array(aliases)
            });
        }
        let to = room
            .get("name")
            .and_then(Json::as_str)
            .unwrap_or_default()
            .to_owned();
        rooms.push(Json::Object(room));
        Ok(match renamed {
            Some(from) => Change::Renamed {
                from: from.to_owned(),
                to,
            },
            None => Change::Created,
        })
    }
}

fn remove(layout: &mut Map<String, Json>, name: &str) -> Result<Change, (StatusCode, String)> {
    let rooms = rooms_of(layout);
    match find(rooms, name) {
        Some(at) => {
            rooms.remove(at);
            Ok(Change::Removed)
        }
        None => Err((
            StatusCode::NOT_FOUND,
            moli_i18n::tr!("serveur.maison.inconnue", name = name),
        )),
    }
}

/// A person's change to the rooms, written and told.
async fn change(
    hub: &Hub,
    store: &HomeStore,
    humans: &Sessions,
    caller: &Caller,
    headers: &HeaderMap,
    edit: impl FnOnce(&mut Map<String, Json>) -> Result<Change, (StatusCode, String)>,
) -> Response {
    if !humans.is_human(headers, &caller.key) {
        return error(
            StatusCode::FORBIDDEN,
            &moli_i18n::tr!("serveur.maison.humain"),
        );
    }
    let Some(path) = &store.path else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            &moli_i18n::tr!("serveur.maison.pas_de_dossier"),
        );
    };
    let _writing = store.lock.lock().await;
    let mut layout = match read(path).await {
        Ok(layout) => layout,
        Err(why) => return error(StatusCode::INTERNAL_SERVER_ERROR, &why),
    };
    let change = match edit(&mut layout) {
        Ok(change) => change,
        Err((status, why)) => return error(status, &why),
    };
    if let Err(e) = write(path, &layout).await {
        return error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
    }
    let status = match &change {
        Change::Created => StatusCode::CREATED,
        Change::Renamed { from, to } => {
            // The labels follow the room; the file is already written, so a
            // label that cannot be saved is said, not hidden.
            let actor = Some(caller.describe());
            if let Err(e) = hub.rename_room(from, to, Origin::Ui, actor).await {
                hub.home_changed();
                return error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
            }
            StatusCode::OK
        }
        Change::Changed | Change::Removed => StatusCode::OK,
    };
    hub.home_changed();
    (status, axum::Json(Json::Object(layout))).into_response()
}

pub(crate) async fn create(
    State(hub): State<Hub>,
    Extension(store): Extension<HomeStore>,
    Extension(humans): Extension<Arc<Sessions>>,
    caller: Caller,
    headers: HeaderMap,
    axum::Json(patch): axum::Json<RoomPatch>,
) -> Response {
    change(&hub, &store, &humans, &caller, &headers, |layout| {
        if patch.name.is_none() {
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                moli_i18n::tr!("serveur.maison.nom_vide"),
            ));
        }
        edit(layout, None, patch)
    })
    .await
}

pub(crate) async fn update(
    State(hub): State<Hub>,
    Extension(store): Extension<HomeStore>,
    Extension(humans): Extension<Arc<Sessions>>,
    caller: Caller,
    Path(name): Path<String>,
    headers: HeaderMap,
    axum::Json(patch): axum::Json<RoomPatch>,
) -> Response {
    change(&hub, &store, &humans, &caller, &headers, |layout| {
        edit(layout, Some(&name), patch)
    })
    .await
}

pub(crate) async fn delete(
    State(hub): State<Hub>,
    Extension(store): Extension<HomeStore>,
    Extension(humans): Extension<Arc<Sessions>>,
    caller: Caller,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Response {
    change(&hub, &store, &humans, &caller, &headers, |layout| {
        remove(layout, &name)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(rooms: Json) -> Map<String, Json> {
        let mut layout = Map::new();
        layout.insert("title".into(), json!("Maison"));
        layout.insert("hidden".into(), json!(["tuya:1"]));
        layout.insert("rooms".into(), rooms);
        layout
    }

    fn patch(name: &str, icon: &str, aliases: &[&str]) -> RoomPatch {
        RoomPatch {
            name: Some(name.into()),
            icon: Some(icon.into()),
            aliases: Some(aliases.iter().map(|a| (*a).to_owned()).collect()),
        }
    }

    fn names(layout: &Map<String, Json>) -> Vec<String> {
        layout["rooms"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["name"].as_str().unwrap().to_owned())
            .collect()
    }

    #[test]
    fn a_room_is_created_cleaned_and_the_rest_of_the_file_kept() {
        let mut l = layout(json!([{ "name": "Salon", "icon": "sofa-outline", "extra": 1 }]));
        let change = edit(
            &mut l,
            None,
            patch(
                "  Chambre\tdes   enfants ",
                "bed-outline",
                ["Chambre 2", "", " chambre 2 "].as_ref(),
            ),
        );
        assert!(matches!(change, Ok(Change::Created)));
        assert_eq!(names(&l), ["Salon", "Chambre des enfants"]);
        let room = &l["rooms"][1];
        assert_eq!(room["icon"], "bed-outline");
        assert_eq!(
            room["aliases"],
            json!(["Chambre 2"]),
            "aliases cleaned, deduplicated"
        );
        assert_eq!(
            l["hidden"],
            json!(["tuya:1"]),
            "the rest of the file is kept"
        );
        assert_eq!(
            l["rooms"][0]["extra"], 1,
            "unknown fields of a room are kept"
        );
    }

    #[test]
    fn names_are_checked() {
        let mut l = layout(json!([{ "name": "Salon", "aliases": ["Séjour"] }]));
        let refused = |l: &mut Map<String, Json>, p: RoomPatch| {
            let Err((status, why)) = edit(l, None, p) else {
                panic!("accepted")
            };
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
            why
        };
        assert!(
            !refused(&mut l, patch("   ", "home", &[])).is_empty(),
            "an empty name"
        );
        refused(&mut l, patch(&"a".repeat(61), "home", &[]));
        refused(&mut l, patch("salon", "home", &[]));
        refused(&mut l, patch("SÉJOUR", "home", &[]));
        refused(&mut l, patch("Cuisine", "home", &["sejour"]));
        refused(&mut l, patch("Cuisine", "no-such-icon", &[]));
        refused(
            &mut l,
            patch(
                "Cuisine",
                "home",
                &["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k"],
            ),
        );
        assert_eq!(names(&l), ["Salon"], "nothing written by a refusal");
        assert!(edit(&mut l, None, patch("Cuisine", "", &[])).is_ok());
        assert!(
            l["rooms"][1].get("icon").is_none(),
            "no icon: the dashboard's default"
        );
    }

    #[test]
    fn a_house_has_a_hundred_rooms_at_most() {
        let rooms: Vec<Json> = (0..MAX_ROOMS)
            .map(|i| json!({ "name": format!("P{i}") }))
            .collect();
        let mut l = layout(Json::Array(rooms));
        let Err((status, _)) = edit(&mut l, None, patch("Encore une", "home", &[])) else {
            panic!("accepted")
        };
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            edit(&mut l, Some("P3"), patch("P3 bis", "home", &[])).is_ok(),
            "an existing room still changes"
        );
    }

    #[test]
    fn a_rename_keeps_the_old_name_as_an_alias() {
        let mut l = layout(json!([{ "name": "Chambre 2", "icon": "bed-outline" }]));
        let change = edit(
            &mut l,
            Some("chambre 2"),
            RoomPatch {
                name: Some("Chambre des enfants".into()),
                ..RoomPatch::default()
            },
        );
        let Ok(Change::Renamed { from, to }) = change else {
            panic!("{change:?}")
        };
        assert_eq!(
            (from.as_str(), to.as_str()),
            ("Chambre 2", "Chambre des enfants")
        );
        assert_eq!(l["rooms"][0]["aliases"], json!(["Chambre 2"]));
        assert_eq!(
            l["rooms"][0]["icon"], "bed-outline",
            "untouched fields stay"
        );
        // Changing the icon only is not a rename.
        let change = edit(
            &mut l,
            Some("Chambre des enfants"),
            RoomPatch {
                icon: Some("bed-king-outline".into()),
                ..RoomPatch::default()
            },
        );
        assert!(matches!(change, Ok(Change::Changed)));
        // A room only the devices named so far: declared, renamed, aliased.
        let change = edit(
            &mut l,
            Some("Hue kitchen"),
            patch("Cuisine", "silverware-fork-knife", &[]),
        );
        assert!(matches!(change, Ok(Change::Renamed { .. })));
        assert_eq!(l["rooms"][1]["aliases"], json!(["Hue kitchen"]));
        assert!(remove(&mut l, "cuisine").is_ok());
        assert_eq!(names(&l), ["Chambre des enfants"]);
        assert_eq!(
            remove(&mut l, "Cuisine").unwrap_err().0,
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn the_file_is_written_whole_and_read_back() {
        let dir = std::env::temp_dir().join(format!("moli-home-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("home.json");
        std::fs::write(
            &path,
            "{\"title\": \"Maison\", \"favorites\": {\"lights\": []}}\n",
        )
        .unwrap();
        let mut l = read(&path).await.unwrap();
        edit(&mut l, None, patch("Salon", "sofa-outline", &[])).unwrap();
        write(&path, &l).await.unwrap();
        let again = read(&path).await.unwrap();
        assert_eq!(again["title"], "Maison");
        assert_eq!(again["favorites"], json!({ "lights": [] }));
        assert_eq!(names(&again), ["Salon"]);
        assert!(
            std::fs::read_dir(&dir).unwrap().count() == 1,
            "no temporary file left behind"
        );
        std::fs::write(&path, "not json").unwrap();
        assert!(
            read(&path).await.is_err(),
            "a broken file is never overwritten"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Every icon offered here exists in the dashboard's set.
    #[test]
    fn room_icons_are_in_the_bundle() {
        let icons = std::fs::read_to_string(
            FsPath::new(env!("CARGO_MANIFEST_DIR")).join("../../web/src/maison/lib/icons.js"),
        )
        .unwrap();
        for icon in ROOM_ICONS {
            let key = if icon.contains('-') {
                format!("  '{icon}': mdi")
            } else {
                format!("  {icon}: mdi")
            };
            assert!(icons.contains(&key), "{icon} is not in icons.js");
        }
    }
}
