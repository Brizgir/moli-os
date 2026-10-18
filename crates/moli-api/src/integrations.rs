//! Keys a person gives Moli for an integration (the Tuya cloud project's
//! Access ID and Secret, the Netatmo app's client ID and secret): typed in
//! the dashboard by a human (code), filed in Moli's encrypted store under
//! the driver's name, never sent back. The driver, waiting for them, goes
//! on with them.

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use moli_core::InstanceId;
use moli_runtime::Hub;
use serde::Deserialize;
use serde_json::json;

use crate::caller::Caller;
use crate::session::Sessions;

const ID: &str = "cloud_access_id";
const SECRET: &str = "cloud_access_secret";

fn instances_of(hub: &Hub, kind: &str) -> Vec<InstanceId> {
    hub.snapshot()
        .drivers
        .iter()
        .filter(|d| d.kind == kind)
        .map(|d| d.instance.clone())
        .collect()
}

fn tuya_instances(hub: &Hub) -> Vec<InstanceId> {
    instances_of(hub, "tuya")
}

/// Whether each Tuya instance has cloud keys filed (never the keys).
pub(crate) async fn tuya_cloud(State(hub): State<Hub>) -> Response {
    let instances: Vec<_> = tuya_instances(&hub)
        .into_iter()
        .map(|i| {
            let configured = hub.secret(&i, ID).is_some() && hub.secret(&i, SECRET).is_some();
            json!({ "instance": i, "configured": configured })
        })
        .collect();
    Json(json!({ "instances": instances })).into_response()
}

#[derive(Deserialize)]
pub(crate) struct Keys {
    #[serde(default)]
    instance: Option<String>,
    access_id: String,
    access_secret: String,
}

fn plausible(value: &str, min: usize) -> bool {
    (min..=64).contains(&value.len()) && value.chars().all(|c| c.is_ascii_alphanumeric())
}

pub(crate) async fn set_tuya_cloud(
    State(hub): State<Hub>,
    Extension(humans): Extension<Arc<Sessions>>,
    caller: Caller,
    headers: HeaderMap,
    Json(keys): Json<Keys>,
) -> Response {
    let refuse = |status: StatusCode, message: &str| {
        (status, Json(json!({ "error": message }))).into_response()
    };
    if !humans.is_human(&headers, &caller.key) {
        return refuse(
            StatusCode::FORBIDDEN,
            &moli_i18n::tr!("serveur.cle.tuya_humain"),
        );
    }
    let instances = tuya_instances(&hub);
    let instance = match keys.instance.as_deref() {
        Some(name) => InstanceId::from(name),
        None => match instances.first() {
            Some(i) => i.clone(),
            None => {
                return refuse(
                    StatusCode::NOT_FOUND,
                    &moli_i18n::tr!("serveur.pilote.absent", pilote = "tuya"),
                );
            }
        },
    };
    if !instances.contains(&instance) {
        return refuse(
            StatusCode::NOT_FOUND,
            &moli_i18n::tr!("serveur.cle.tuya_pilote_inconnu"),
        );
    }
    let (id, secret) = (keys.access_id.trim(), keys.access_secret.trim());
    if !plausible(id, 10) || !plausible(secret, 24) {
        return refuse(
            StatusCode::UNPROCESSABLE_ENTITY,
            &moli_i18n::tr!("serveur.cle.tuya_invalide"),
        );
    }
    for (name, value) in [(ID, id), (SECRET, secret)] {
        if let Err(e) = hub.store_secret(&instance, name, value).await {
            return refuse(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
        }
    }
    tracing::info!(%instance, "tuya cloud keys filed from the dashboard");
    Json(json!({ "stored": true, "instance": instance })).into_response()
}

const NETATMO_ID: &str = "client_id";
const NETATMO_SECRET: &str = "client_secret";

/// Whether each Netatmo instance has its app's keys filed (never the keys).
pub(crate) async fn netatmo_app(State(hub): State<Hub>) -> Response {
    let instances: Vec<_> = instances_of(&hub, "netatmo")
        .into_iter()
        .map(|i| {
            let configured =
                hub.secret(&i, NETATMO_ID).is_some() && hub.secret(&i, NETATMO_SECRET).is_some();
            json!({ "instance": i, "configured": configured })
        })
        .collect();
    Json(json!({ "instances": instances })).into_response()
}

#[derive(Deserialize)]
pub(crate) struct NetatmoApp {
    #[serde(default)]
    instance: Option<String>,
    client_id: String,
    client_secret: String,
}

/// What an app's key looks like on dev.netatmo.com.
fn netatmo_key(value: &str, min: usize) -> bool {
    (min..=128).contains(&value.len())
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

pub(crate) async fn set_netatmo_app(
    State(hub): State<Hub>,
    Extension(humans): Extension<Arc<Sessions>>,
    caller: Caller,
    headers: HeaderMap,
    Json(keys): Json<NetatmoApp>,
) -> Response {
    let refuse = |status: StatusCode, message: &str| {
        (status, Json(json!({ "error": message }))).into_response()
    };
    if !humans.is_human(&headers, &caller.key) {
        return refuse(
            StatusCode::FORBIDDEN,
            &moli_i18n::tr!("serveur.cle.netatmo_humain"),
        );
    }
    let instances = instances_of(&hub, "netatmo");
    let instance = match keys.instance.as_deref() {
        Some(name) => InstanceId::from(name),
        None => match instances.first() {
            Some(i) => i.clone(),
            None => {
                return refuse(
                    StatusCode::NOT_FOUND,
                    &moli_i18n::tr!("serveur.pilote.absent", pilote = "netatmo"),
                );
            }
        },
    };
    if !instances.contains(&instance) {
        return refuse(
            StatusCode::NOT_FOUND,
            &moli_i18n::tr!("serveur.cle.netatmo_pilote_inconnu"),
        );
    }
    let (id, secret) = (keys.client_id.trim(), keys.client_secret.trim());
    if !netatmo_key(id, 10) || !netatmo_key(secret, 16) {
        return refuse(
            StatusCode::UNPROCESSABLE_ENTITY,
            &moli_i18n::tr!("serveur.cle.netatmo_invalide"),
        );
    }
    for (name, value) in [(NETATMO_ID, id), (NETATMO_SECRET, secret)] {
        if let Err(e) = hub.store_secret(&instance, name, value).await {
            return refuse(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
        }
    }
    tracing::info!(%instance, "netatmo app keys filed from the dashboard");
    Json(json!({ "stored": true, "instance": instance })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn netatmo_keys_look_like_keys() {
        assert!(netatmo_key("0123456789abcdef01234567", 10));
        assert!(netatmo_key("AbCdEfGhIjKlMnOpQrStUvWxYz012345", 16));
        assert!(!netatmo_key("short", 10));
        assert!(!netatmo_key("has space in it", 10));
        assert!(!netatmo_key("0123456789\n", 10));
    }
}
