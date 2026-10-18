//! A device's own document and the actions on it (a home's heating
//! schedules…), through `/api/devices/{id}/service/{name}`. Reading is free
//! (a guest: its rooms only); acting is a person's gesture only: the
//! dashboard (`x-moli-origin: ui`) with a human session (the code), never an
//! agent, a script nor an automation, as for `PUT /api/plan`.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use moli_core::{DeviceId, Origin};
use moli_runtime::Hub;
use moli_runtime::service::ServiceError;
use serde_json::{Value as Json_, json};

use crate::caller::Caller;
use crate::session::Sessions;

/// Names a driver gives: short, plain.
fn plain(name: &str) -> bool {
    (1..=32).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
}

fn refuse(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

fn answer(result: Result<Json_, ServiceError>) -> Response {
    match result {
        Ok(body) => Json(body).into_response(),
        Err(e) => {
            let status = match &e {
                ServiceError::UnknownDevice
                | ServiceError::NoService
                | ServiceError::UnknownAction(_) => StatusCode::NOT_FOUND,
                ServiceError::Invalid(_) => StatusCode::UNPROCESSABLE_ENTITY,
                ServiceError::Refused(_) => StatusCode::BAD_GATEWAY,
                ServiceError::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            };
            refuse(status, &e.to_string())
        }
    }
}

/// A guest reads what is in its rooms only.
#[allow(clippy::result_large_err)]
fn guest_may(hub: &Hub, caller: &Caller, device: &DeviceId) -> Result<(), Response> {
    match caller.person.as_ref().filter(|p| p.is_guest()) {
        Some(guest) if !guest.guest_sees(hub, device) => Err(refuse(
            StatusCode::FORBIDDEN,
            &moli_i18n::tr!("serveur.personnes.invite_piece"),
        )),
        _ => Ok(()),
    }
}

pub(crate) async fn read(
    State(hub): State<Hub>,
    caller: Caller,
    Path((id, what)): Path<(String, String)>,
) -> Response {
    let id = DeviceId::from(id);
    if let Err(no) = guest_may(&hub, &caller, &id) {
        return no;
    }
    if !plain(&what) {
        return refuse(StatusCode::NOT_FOUND, "unknown service");
    }
    answer(hub.service_read(&id, &what).await)
}

pub(crate) async fn write(
    State(hub): State<Hub>,
    Extension(humans): Extension<Arc<Sessions>>,
    caller: Caller,
    headers: HeaderMap,
    Path((id, action)): Path<(String, String)>,
    Json(input): Json<Json_>,
) -> Response {
    let dashboard = headers.get("x-moli-origin").and_then(|v| v.to_str().ok()) == Some("ui");
    let guest = caller
        .person
        .as_ref()
        .is_some_and(crate::people::PersonRef::is_guest);
    if !dashboard || guest || !humans.is_human(&headers, &caller.key) {
        return refuse(
            StatusCode::FORBIDDEN,
            &moli_i18n::tr!("serveur.service.humain"),
        );
    }
    if !plain(&action) {
        return refuse(StatusCode::NOT_FOUND, "unknown action");
    }
    let id = DeviceId::from(id);
    answer(
        hub.service_write(&id, &action, input, Origin::Ui, Some(caller.describe()))
            .await,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_names_reach_a_driver() {
        assert!(plain("schedules"));
        assert!(plain("sync"));
        assert!(!plain(""));
        assert!(!plain("../x"));
        assert!(!plain("Sync"));
        assert!(!plain(&"a".repeat(33)));
    }
}
