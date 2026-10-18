//! `/api/dashboard`: how the home page is arranged, kept in
//! `data/dashboards.json` beside `home.json` (which says what the house
//! *has*; this file says how each person wants to *see* it).
//!
//! The file holds one set of layouts for the house and one per person:
//!
//! ```json
//! { "version": 1,
//!   "house":  { "layouts": { "default": { "desktop": { "cards": [ … ] }, "phone": { "cards": [ … ] } } } },
//!   "people": { "sam": { "layouts": { "default": …, "soir": { "name": "Soir", "desktop": …, "phone": … } } } } }
//! ```
//!
//! A card is a line of an ordered list: `{ id, type, size, hidden, title,
//! options }`. The dashboard lays the list out itself (a card has a width,
//! never a position), so nothing here overlaps or leaves a hole. Which
//! named layout is active is the device's business (the browser), and the
//! default arrangement is never written: the dashboard computes it from
//! `home.json` and the devices, exactly as the page looked before.
//!
//! - `GET /api/dashboard`: the house's layouts and the caller's own, with
//!   what the caller may change; never another person's.
//! - `PUT|DELETE /api/dashboard/house`: the owner (as for the people).
//! - `PUT|DELETE /api/dashboard/me`: a person signed in (not a guest).
//!
//! Every write is validated whole (422), atomic (a unique temporary name,
//! then a rename), and told (`dashboard_changed` on the event stream). A
//! card of a type this version does not know (written by a newer Moli) is
//! kept as it is, so a rollback loses nothing.

use std::collections::HashSet;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;

use axum::Extension;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use moli_runtime::Hub;
use serde::Deserialize;
use serde_json::{Map, Value as Json, json};

use crate::caller::Caller;
use crate::people::People;
use crate::session::Sessions;

/// `data/dashboards.json`, and the lock that serialises its writers.
#[derive(Clone)]
pub(crate) struct DashboardStore {
    path: Option<PathBuf>,
    lock: Arc<tokio::sync::Mutex<()>>,
}

/// The file's format; an older file is brought here when read.
pub(crate) const VERSION: u64 = 1;

/// What a card can be: the dashboard's `CARD_TYPES`
/// (`web/src/maison/lib/layout.js`), checked by a test.
pub(crate) const CARD_TYPES: &[&str] = &[
    "sky",
    "modes",
    "notices",
    "heating",
    "lights",
    "climate",
    "remote",
    "outside",
    "doorbell",
    "music",
    "today",
    "energy",
    "routines",
    "room",
    "speaker",
    "diffuser",
    "camera",
    "presence",
    "assistant",
    "machine",
    "printer",
];

const SIZES: &[&str] = &["s", "m", "l"];
const SCREENS: &[&str] = &["desktop", "phone"];
/// A page, not a wall.
pub(crate) const MAX_CARDS: usize = 60;
/// Morning, evening, away, holidays… a dozen is plenty.
pub(crate) const MAX_LAYOUTS: usize = 12;
pub(crate) const MAX_TITLE: usize = 60;
/// A card's options, serialised.
pub(crate) const MAX_OPTIONS: usize = 2048;
/// A whole body: a dozen layouts of sixty cards fit many times over.
pub(crate) const MAX_BODY: usize = 256 * 1024;

impl DashboardStore {
    pub(crate) fn new(path: Option<PathBuf>) -> Self {
        Self {
            path,
            lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// The store next to `home.json` (none: nowhere to write).
    pub(crate) fn beside(home: Option<&FsPath>) -> Self {
        Self::new(
            home.and_then(FsPath::parent)
                .map(|dir| dir.join("dashboards.json")),
        )
    }

    /// Someone left the house: their layouts go with them.
    pub(crate) async fn forget_person(&self, id: &str) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let _writing = self.lock.lock().await;
        let mut file = read(path).await?;
        let gone = file
            .get_mut("people")
            .and_then(Json::as_object_mut)
            .is_some_and(|people| people.remove(id).is_some());
        if gone {
            if file
                .get("people")
                .and_then(Json::as_object)
                .is_some_and(Map::is_empty)
            {
                file.remove("people");
            }
            write(path, &file).await.map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, axum::Json(json!({ "error": message }))).into_response()
}

/// One line, single spaces, no control character.
fn clean(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// A card as a person sends it; anything else is refused.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CardIn {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    size: Option<String>,
    #[serde(default)]
    hidden: bool,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    options: Option<Map<String, Json>>,
}

/// A screen as a person sends it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScreenIn {
    cards: Vec<CardIn>,
}

/// A layout as a person sends it: `name` for a named one.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LayoutIn {
    #[serde(default)]
    name: Option<String>,
    desktop: ScreenIn,
    phone: ScreenIn,
}

fn invalid(why: String) -> (StatusCode, String) {
    (StatusCode::UNPROCESSABLE_ENTITY, why)
}

/// A key (a layout's, a card's): short, plain, lower-case.
fn plain_key(key: &str, max: usize) -> bool {
    !key.is_empty()
        && key.len() <= max
        && key
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// A card checked and written in its canonical form. `known`: the types
/// the file already held for this scope (a newer Moli's, kept).
fn checked_card(card: CardIn, known: &HashSet<String>) -> Result<Json, String> {
    if !plain_key(&card.id, 16) || card.id.len() < 4 {
        return Err(moli_i18n::tr!("serveur.tableau.identifiant", id = card.id));
    }
    if !CARD_TYPES.contains(&card.kind.as_str()) && !known.contains(&card.kind) {
        return Err(moli_i18n::tr!(
            "serveur.tableau.type_inconnu",
            kind = card.kind
        ));
    }
    let size = card.size.unwrap_or_else(|| "m".to_owned());
    if !SIZES.contains(&size.as_str()) {
        return Err(moli_i18n::tr!(
            "serveur.tableau.taille_inconnue",
            size = size
        ));
    }
    let mut out = Map::new();
    out.insert("id".into(), Json::String(card.id));
    out.insert("type".into(), Json::String(card.kind));
    out.insert("size".into(), Json::String(size));
    if card.hidden {
        out.insert("hidden".into(), Json::Bool(true));
    }
    if let Some(title) = card.title {
        let title = clean(&title);
        if title.chars().count() > MAX_TITLE {
            return Err(moli_i18n::tr!(
                "serveur.tableau.titre_trop_long",
                max = MAX_TITLE
            ));
        }
        if !title.is_empty() {
            out.insert("title".into(), Json::String(title));
        }
    }
    if let Some(options) = card.options
        && !options.is_empty()
    {
        let text = serde_json::to_string(&options).unwrap_or_default();
        if text.len() > MAX_OPTIONS {
            return Err(moli_i18n::tr!(
                "serveur.tableau.options_trop_lourdes",
                max = MAX_OPTIONS
            ));
        }
        out.insert("options".into(), Json::Object(options));
    }
    Ok(Json::Object(out))
}

fn checked_screen(screen: ScreenIn, known: &HashSet<String>) -> Result<Json, String> {
    if screen.cards.len() > MAX_CARDS {
        return Err(moli_i18n::tr!(
            "serveur.tableau.trop_de_cartes",
            max = MAX_CARDS
        ));
    }
    let mut ids = HashSet::new();
    let mut cards = Vec::with_capacity(screen.cards.len());
    for card in screen.cards {
        if !ids.insert(card.id.clone()) {
            return Err(moli_i18n::tr!("serveur.tableau.doublon", id = card.id));
        }
        cards.push(checked_card(card, known)?);
    }
    Ok(json!({ "cards": cards }))
}

/// The card types a scope's layouts hold.
fn types_of(layouts: Option<&Json>) -> HashSet<String> {
    let mut set = HashSet::new();
    for layout in layouts
        .and_then(Json::as_object)
        .into_iter()
        .flat_map(Map::values)
    {
        for screen in SCREENS {
            for card in layout
                .get(*screen)
                .and_then(|s| s.get("cards"))
                .and_then(Json::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(kind) = card.get("type").and_then(Json::as_str) {
                    set.insert(kind.to_owned());
                }
            }
        }
    }
    set
}

/// A scope's layouts, checked whole: `default` present, names on the
/// others, every card sound.
fn checked_layouts(wanted: Json, known: &HashSet<String>) -> Result<Json, (StatusCode, String)> {
    let Json::Object(wanted) = wanted else {
        return Err(invalid(moli_i18n::tr!("serveur.tableau.defaut_manquant")));
    };
    if !wanted.contains_key("default") {
        return Err(invalid(moli_i18n::tr!("serveur.tableau.defaut_manquant")));
    }
    if wanted.len() > MAX_LAYOUTS {
        return Err(invalid(moli_i18n::tr!(
            "serveur.tableau.trop_de_dispositions",
            max = MAX_LAYOUTS
        )));
    }
    let mut out = Map::new();
    for (key, layout) in wanted {
        if !plain_key(&key, 32) {
            return Err(invalid(moli_i18n::tr!("serveur.tableau.cle", key = key)));
        }
        let layout: LayoutIn = serde_json::from_value(layout)
            .map_err(|e| invalid(moli_i18n::tr!("serveur.tableau.forme", error = e)))?;
        let mut object = Map::new();
        if key != "default" {
            let name = layout.name.as_deref().map(clean).unwrap_or_default();
            if name.is_empty() {
                return Err(invalid(moli_i18n::tr!("serveur.tableau.nom_vide")));
            }
            if name.chars().count() > MAX_TITLE {
                return Err(invalid(moli_i18n::tr!(
                    "serveur.tableau.titre_trop_long",
                    max = MAX_TITLE
                )));
            }
            object.insert("name".into(), Json::String(name));
        }
        object.insert(
            "desktop".into(),
            checked_screen(layout.desktop, known).map_err(invalid)?,
        );
        object.insert(
            "phone".into(),
            checked_screen(layout.phone, known).map_err(invalid)?,
        );
        out.insert(key, Json::Object(object));
    }
    Ok(Json::Object(out))
}

/// The file's content, brought to this version; `{}` when there is none.
async fn read(path: &FsPath) -> Result<Map<String, Json>, String> {
    let mut file = match tokio::fs::read(path).await {
        Ok(bytes) => match serde_json::from_slice::<Json>(&bytes) {
            Ok(Json::Object(file)) => file,
            Ok(_) => {
                return Err(moli_i18n::tr!(
                    "serveur.tableau.fichier_illisible",
                    error = "not an object"
                ));
            }
            Err(e) => {
                return Err(moli_i18n::tr!(
                    "serveur.tableau.fichier_illisible",
                    error = e
                ));
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Map::new(),
        Err(e) => return Err(e.to_string()),
    };
    migrate(&mut file);
    Ok(file)
}

/// An older file brought to `VERSION` (nothing to change yet: the first
/// format); a newer one is read as it is.
fn migrate(file: &mut Map<String, Json>) {
    let version = file.get("version").and_then(Json::as_u64).unwrap_or(0);
    if version < VERSION {
        file.insert("version".into(), json!(VERSION));
    }
}

/// Written whole, under a name of its own, then put in place.
async fn write(path: &FsPath, file: &Map<String, Json>) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(file)? + "\n";
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

/// The layouts stored for a scope, if any.
fn layouts_of<'a>(file: &'a Map<String, Json>, scope: &Scope) -> Option<&'a Json> {
    match scope {
        Scope::House => file.get("house")?.get("layouts"),
        Scope::Person(id) => file.get("people")?.get(id)?.get("layouts"),
    }
}

/// `layouts` put in place for a scope (`None`: forgotten).
fn set_layouts(file: &mut Map<String, Json>, scope: &Scope, layouts: Option<Json>) {
    match scope {
        Scope::House => match layouts {
            Some(layouts) => {
                file.insert("house".into(), json!({ "layouts": layouts }));
            }
            None => {
                file.remove("house");
            }
        },
        Scope::Person(id) => {
            if !file.get("people").is_some_and(Json::is_object) {
                file.insert("people".into(), Json::Object(Map::new()));
            }
            let people = file
                .get_mut("people")
                .and_then(Json::as_object_mut)
                .expect("people is an object");
            match layouts {
                Some(layouts) => {
                    people.insert(id.clone(), json!({ "layouts": layouts }));
                }
                None => {
                    people.remove(id);
                }
            }
            if people.is_empty() {
                file.remove("people");
            }
        }
    }
}

/// Whose arrangement.
#[derive(Debug, Clone, PartialEq)]
enum Scope {
    House,
    Person(String),
}

impl Scope {
    fn name(&self) -> String {
        match self {
            Self::House => "house".to_owned(),
            Self::Person(id) => format!("person:{id}"),
        }
    }
}

/// The person who may keep an arrangement of their own: signed in, not a
/// guest (a guest sees the house's default, filtered to their rooms).
fn own_scope(caller: &Caller) -> Option<Scope> {
    caller
        .person
        .as_ref()
        .filter(|p| !p.is_guest())
        .map(|p| Scope::Person(p.id.clone()))
}

/// `GET /api/dashboard`: what the caller sees and may change.
pub(crate) async fn get(
    Extension(store): Extension<DashboardStore>,
    Extension(humans): Extension<Arc<Sessions>>,
    Extension(persons): Extension<Arc<People>>,
    caller: Caller,
    headers: HeaderMap,
) -> Response {
    let guest = caller
        .person
        .as_ref()
        .is_some_and(crate::people::PersonRef::is_guest);
    let file = match &store.path {
        Some(path) if !guest => match read(path).await {
            Ok(file) => file,
            Err(why) => return error(StatusCode::INTERNAL_SERVER_ERROR, &why),
        },
        _ => Map::new(),
    };
    let mine = own_scope(&caller);
    let can_edit_house =
        !guest && crate::people_api::is_owner(&caller, &humans, &persons, &headers);
    let can_edit_mine = mine.is_some() && humans.is_human(&headers, &caller.key);
    axum::Json(json!({
        "house": layouts_of(&file, &Scope::House).cloned(),
        "mine": mine.as_ref().and_then(|s| layouts_of(&file, s)).cloned(),
        "can_edit_house": can_edit_house,
        "can_edit_mine": can_edit_mine,
    }))
    .into_response()
}

/// Who may change a scope's arrangement; the refusal otherwise.
fn allowed(
    scope: &Scope,
    caller: &Caller,
    humans: &Sessions,
    persons: &People,
    headers: &HeaderMap,
) -> Result<(), Box<Response>> {
    match scope {
        Scope::House => {
            if caller
                .person
                .as_ref()
                .is_some_and(crate::people::PersonRef::is_guest)
                || !crate::people_api::is_owner(caller, humans, persons, headers)
            {
                return Err(Box::new(error(
                    StatusCode::FORBIDDEN,
                    &moli_i18n::tr!("serveur.tableau.proprietaire"),
                )));
            }
        }
        Scope::Person(_) => {
            if !humans.is_human(headers, &caller.key) {
                return Err(Box::new(error(
                    StatusCode::FORBIDDEN,
                    &moli_i18n::tr!("serveur.tableau.personne"),
                )));
            }
        }
    }
    Ok(())
}

/// A change to a scope's arrangement, checked, written and told.
async fn change(hub: &Hub, store: &DashboardStore, scope: Scope, wanted: Option<Json>) -> Response {
    let Some(path) = &store.path else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            &moli_i18n::tr!("serveur.tableau.pas_de_dossier"),
        );
    };
    let _writing = store.lock.lock().await;
    let mut file = match read(path).await {
        Ok(file) => file,
        Err(why) => return error(StatusCode::INTERNAL_SERVER_ERROR, &why),
    };
    let layouts = match wanted {
        Some(wanted) => {
            let known = types_of(layouts_of(&file, &scope));
            match checked_layouts(wanted, &known) {
                Ok(layouts) => Some(layouts),
                Err((status, why)) => return error(status, &why),
            }
        }
        None => None,
    };
    set_layouts(&mut file, &scope, layouts.clone());
    if let Err(e) = write(path, &file).await {
        return error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
    }
    hub.dashboard_changed(&scope.name());
    axum::Json(json!({ "layouts": layouts })).into_response()
}

pub(crate) async fn put_house(
    State(hub): State<Hub>,
    Extension(store): Extension<DashboardStore>,
    Extension(humans): Extension<Arc<Sessions>>,
    Extension(persons): Extension<Arc<People>>,
    caller: Caller,
    headers: HeaderMap,
    axum::Json(wanted): axum::Json<Json>,
) -> Response {
    if let Err(refused) = allowed(&Scope::House, &caller, &humans, &persons, &headers) {
        return *refused;
    }
    change(&hub, &store, Scope::House, Some(wanted)).await
}

pub(crate) async fn delete_house(
    State(hub): State<Hub>,
    Extension(store): Extension<DashboardStore>,
    Extension(humans): Extension<Arc<Sessions>>,
    Extension(persons): Extension<Arc<People>>,
    caller: Caller,
    headers: HeaderMap,
) -> Response {
    if let Err(refused) = allowed(&Scope::House, &caller, &humans, &persons, &headers) {
        return *refused;
    }
    change(&hub, &store, Scope::House, None).await
}

/// The caller's own scope, or why there is none (403).
fn mine(caller: &Caller) -> Result<Scope, Box<Response>> {
    own_scope(caller).ok_or_else(|| {
        Box::new(error(
            StatusCode::FORBIDDEN,
            &moli_i18n::tr!("serveur.tableau.personne"),
        ))
    })
}

pub(crate) async fn put_me(
    State(hub): State<Hub>,
    Extension(store): Extension<DashboardStore>,
    Extension(humans): Extension<Arc<Sessions>>,
    Extension(persons): Extension<Arc<People>>,
    caller: Caller,
    headers: HeaderMap,
    axum::Json(wanted): axum::Json<Json>,
) -> Response {
    let scope = match mine(&caller) {
        Ok(scope) => scope,
        Err(refused) => return *refused,
    };
    if let Err(refused) = allowed(&scope, &caller, &humans, &persons, &headers) {
        return *refused;
    }
    change(&hub, &store, scope, Some(wanted)).await
}

pub(crate) async fn delete_me(
    State(hub): State<Hub>,
    Extension(store): Extension<DashboardStore>,
    Extension(humans): Extension<Arc<Sessions>>,
    Extension(persons): Extension<Arc<People>>,
    caller: Caller,
    headers: HeaderMap,
) -> Response {
    let scope = match mine(&caller) {
        Ok(scope) => scope,
        Err(refused) => return *refused,
    };
    if let Err(refused) = allowed(&scope, &caller, &humans, &persons, &headers) {
        return *refused;
    }
    change(&hub, &store, scope, None).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(id: &str, kind: &str) -> Json {
        json!({ "id": id, "type": kind })
    }

    fn layouts(cards: &[Json]) -> Json {
        json!({ "default": { "desktop": { "cards": cards }, "phone": { "cards": [] } } })
    }

    fn refused(wanted: Json) -> String {
        match checked_layouts(wanted, &HashSet::new()) {
            Ok(ok) => panic!("accepted: {ok}"),
            Err((status, why)) => {
                assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{why}");
                why
            }
        }
    }

    #[test]
    fn a_layout_is_checked_and_written_in_its_canonical_form() {
        let wanted = json!({
            "default": {
                "desktop": { "cards": [
                    { "id": "k7f2", "type": "heating", "size": "l", "title": "  Le\tchauffage ", "options": { "rooms": ["Cuisine"] } },
                    { "id": "ab12", "type": "music", "hidden": true, "title": "", "options": {} }
                ] },
                "phone": { "cards": [ card("k7f2", "heating") ] }
            },
            "soir": { "name": " Soir ", "desktop": { "cards": [] }, "phone": { "cards": [] } }
        });
        let out = checked_layouts(wanted, &HashSet::new()).unwrap();
        assert_eq!(
            out["default"]["desktop"]["cards"][0],
            json!({ "id": "k7f2", "type": "heating", "size": "l", "title": "Le chauffage", "options": { "rooms": ["Cuisine"] } })
        );
        assert_eq!(
            out["default"]["desktop"]["cards"][1],
            json!({ "id": "ab12", "type": "music", "size": "m", "hidden": true }),
            "size m by default; an empty title and empty options are dropped"
        );
        assert_eq!(out["default"].get("name"), None);
        assert_eq!(out["soir"]["name"], "Soir");
    }

    #[test]
    fn what_is_refused() {
        assert!(!refused(json!({})).is_empty(), "no default layout");
        refused(json!([]));
        refused(json!({ "default": { "desktop": { "cards": [] } } }));
        refused(
            json!({ "default": { "desktop": { "cards": [] }, "phone": { "cards": [] }, "extra": 1 } }),
        );
        refused(
            json!({ "Soir": { "name": "Soir", "desktop": { "cards": [] }, "phone": { "cards": [] } }, "default": { "desktop": { "cards": [] }, "phone": { "cards": [] } } }),
        );
        refused(
            json!({ "default": { "desktop": { "cards": [] }, "phone": { "cards": [] } }, "soir": { "desktop": { "cards": [] }, "phone": { "cards": [] } } }),
        );
        refused(layouts(&[card("k7f2", "no-such-card")]));
        refused(layouts(&[card("K7", "music")]));
        refused(layouts(&[card("k7f2", "music"), card("k7f2", "today")]));
        refused(layouts(&[
            json!({ "id": "k7f2", "type": "music", "size": "xl" }),
        ]));
        refused(layouts(&[json!({ "id": "k7f2", "type": "music", "x": 1 })]));
        refused(layouts(&[
            json!({ "id": "k7f2", "type": "music", "title": "a".repeat(61) }),
        ]));
        refused(layouts(&[
            json!({ "id": "k7f2", "type": "music", "options": { "big": "x".repeat(MAX_OPTIONS) } }),
        ]));
        let many: Vec<Json> = (0..=MAX_CARDS)
            .map(|i| card(&format!("c{i:04}"), "room"))
            .collect();
        refused(layouts(&many));
        let mut too_many = Map::new();
        for i in 0..=MAX_LAYOUTS {
            let key = if i == 0 {
                "default".to_owned()
            } else {
                format!("l{i}")
            };
            too_many.insert(
                key,
                json!({ "name": "L", "desktop": { "cards": [] }, "phone": { "cards": [] } }),
            );
        }
        refused(Json::Object(too_many));
    }

    #[test]
    fn a_card_from_a_newer_moli_is_kept_when_the_file_already_had_it() {
        let stored = json!({ "default": { "desktop": { "cards": [ card("zz99", "hologram") ] }, "phone": { "cards": [] } } });
        let known = types_of(Some(&stored));
        assert!(known.contains("hologram"));
        let out = checked_layouts(
            layouts(&[card("zz99", "hologram"), card("ab12", "music")]),
            &known,
        )
        .unwrap();
        assert_eq!(out["default"]["desktop"]["cards"][0]["type"], "hologram");
        assert!(checked_layouts(layouts(&[card("zz99", "hologram")]), &HashSet::new()).is_err());
    }

    #[test]
    fn scopes_are_kept_apart_and_a_person_s_layouts_go_with_them() {
        let mut file = Map::new();
        set_layouts(
            &mut file,
            &Scope::House,
            Some(layouts(&[card("ab12", "music")])),
        );
        set_layouts(
            &mut file,
            &Scope::Person("sam".into()),
            Some(layouts(&[card("ab12", "today")])),
        );
        set_layouts(&mut file, &Scope::Person("lou".into()), Some(layouts(&[])));
        assert_eq!(
            layouts_of(&file, &Scope::House).unwrap()["default"]["desktop"]["cards"][0]["type"],
            "music"
        );
        assert_eq!(
            layouts_of(&file, &Scope::Person("sam".into())).unwrap()["default"]["desktop"]["cards"]
                [0]["type"],
            "today"
        );
        assert_eq!(layouts_of(&file, &Scope::Person("ana".into())), None);
        set_layouts(&mut file, &Scope::Person("sam".into()), None);
        assert_eq!(layouts_of(&file, &Scope::Person("sam".into())), None);
        assert!(layouts_of(&file, &Scope::Person("lou".into())).is_some());
        set_layouts(&mut file, &Scope::Person("lou".into()), None);
        assert_eq!(file.get("people"), None, "nobody left: no entry");
        set_layouts(&mut file, &Scope::House, None);
        assert_eq!(file.get("house"), None);
        assert_eq!(Scope::Person("sam".into()).name(), "person:sam");
    }

    #[tokio::test]
    async fn the_file_is_versioned_written_whole_and_read_back() {
        let dir = std::env::temp_dir().join(format!("moli-dashboards-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("dashboards.json");
        assert_eq!(
            read(&path).await.unwrap()["version"],
            json!(VERSION),
            "no file: an empty, current one"
        );
        std::fs::write(&path, "{\"house\": {\"layouts\": {}}, \"future\": true}\n").unwrap();
        let mut file = read(&path).await.unwrap();
        assert_eq!(
            file["version"],
            json!(VERSION),
            "an old file is brought to this version"
        );
        assert_eq!(file["future"], true, "what is not understood is kept");
        set_layouts(
            &mut file,
            &Scope::House,
            Some(layouts(&[card("ab12", "music")])),
        );
        write(&path, &file).await.unwrap();
        let again = read(&path).await.unwrap();
        assert_eq!(
            again["house"]["layouts"]["default"]["desktop"]["cards"][0]["id"],
            "ab12"
        );
        assert_eq!(again["future"], true);
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            1,
            "no temporary file left behind"
        );
        std::fs::write(&path, "{\"version\": 99}").unwrap();
        let newer = read(&path).await.unwrap();
        assert_eq!(newer["version"], 99, "a newer file is read as it is");
        std::fs::write(&path, "not json").unwrap();
        assert!(
            read(&path).await.is_err(),
            "a broken file is never overwritten"
        );
        let store = DashboardStore::new(Some(path.clone()));
        assert!(store.forget_person("sam").await.is_err());
        std::fs::write(&path, serde_json::to_string(&again).unwrap()).unwrap();
        let mut file = read(&path).await.unwrap();
        set_layouts(&mut file, &Scope::Person("sam".into()), Some(layouts(&[])));
        write(&path, &file).await.unwrap();
        store.forget_person("sam").await.unwrap();
        assert_eq!(read(&path).await.unwrap().get("people"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The card types here are the dashboard's (`CARD_TYPES` of `layout.js`).
    #[test]
    fn card_types_are_the_dashboard_s() {
        let js = std::fs::read_to_string(
            FsPath::new(env!("CARGO_MANIFEST_DIR")).join("../../web/src/maison/lib/layout.js"),
        )
        .unwrap();
        let line = js
            .lines()
            .find(|l| l.starts_with("export const CARD_TYPES"))
            .expect("CARD_TYPES in layout.js");
        let ids: Vec<&str> = line.split('\'').skip(1).step_by(2).collect();
        assert_eq!(ids, CARD_TYPES);
    }
}
