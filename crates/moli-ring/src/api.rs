//! Ring's cloud: an OAuth account (password once, with the code Ring sends
//! by SMS or e-mail, then a refresh token) and the clients API.
//!
//! Only the refresh token is kept, in Moli's encrypted store; the password
//! and the code are forgotten once it is obtained. Ring rotates the refresh
//! token: each new one is handed to the driver to store.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use anyhow::{Context as _, bail};
use http::{Method, Request, StatusCode};
use http_body_util::Full;
use moli_net::Body;
use serde_json::{Value as Json, json};
use tokio::time::Instant;

const OAUTH_HOST: &str = "oauth.ring.com";
const API_HOST: &str = "api.ring.com";
const CLIENT_ID: &str = "ring_official_android";
/// `base64("ring_official_android:")`: the app's client, no secret.
const CLIENT_BASIC: &str = "Basic cmluZ19vZmZpY2lhbF9hbmRyb2lkOg==";
/// The name the account shows among its authorized devices.
const USER_AGENT: &str = "moli-os/ring";
const API_VERSION: &str = "11";
const TIMEOUT: Duration = Duration::from_secs(10);
/// A device list for a large account stays well under this.
const MAX_ANSWER: usize = 4 * 1024 * 1024;
/// Renew the access token this long before it ends.
const MARGIN: Duration = Duration::from_secs(120);

/// Why the account cannot be used without a person.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Refused {
    /// Ring sent a code (SMS, e-mail, app): it must be given.
    NeedsCode,
    /// E-mail, password or code refused.
    Credentials,
    /// The refresh token no longer works (revoked, expired): sign in again.
    SignedOut,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NeedsCode => "ring wants a verification code",
            Self::Credentials => "ring refused the credentials",
            Self::SignedOut => "ring signed moli out",
        })
    }
}

impl std::error::Error for Refused {}

/// An access token and when to renew it. Never printed.
struct Access {
    token: String,
    renew_at: Instant,
}

/// What survives a driver restart: the access token (no new sign-in per
/// restart) and whether Ring knows this client's session.
#[derive(Default)]
pub(crate) struct Shared {
    access: Mutex<Option<Access>>,
    refresh: Mutex<Option<String>>,
    /// A refresh token Ring rotated, not stored yet.
    rotated: Mutex<Option<String>>,
    session: AtomicBool,
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Shared(<redacted>)")
    }
}

/// The tokens an OAuth answer carries.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Tokens {
    pub access: String,
    pub refresh: String,
    pub expires_in: u64,
}

/// Reads an OAuth answer: tokens, a code to ask for, or a refusal.
pub(crate) fn tokens(status: StatusCode, body: &[u8]) -> Result<Tokens, Refused> {
    let answer: Json = serde_json::from_slice(body).unwrap_or(Json::Null);
    // 412 « precondition failed » with `tsv_state`: two-step verification.
    if status == StatusCode::PRECONDITION_FAILED || answer.get("tsv_state").is_some() {
        return Err(Refused::NeedsCode);
    }
    let token = |key: &str| answer[key].as_str().filter(|t| !t.is_empty());
    match (
        status.is_success(),
        token("access_token"),
        token("refresh_token"),
    ) {
        (true, Some(access), Some(refresh)) => Ok(Tokens {
            access: access.to_owned(),
            refresh: refresh.to_owned(),
            expires_in: answer["expires_in"].as_u64().unwrap_or(3600),
        }),
        _ => Err(Refused::Credentials),
    }
}

/// `application/x-www-form-urlencoded`.
fn form(fields: &[(&str, &str)]) -> String {
    let mut out = String::new();
    for (i, (name, value)) in fields.iter().enumerate() {
        if i > 0 {
            out.push('&');
        }
        encode(&mut out, name);
        out.push('=');
        encode(&mut out, value);
    }
    out
}

fn encode(out: &mut String, text: &str) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push('%');
            out.push(char::from(HEX[usize::from(byte >> 4)]));
            out.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
}

/// One account, signed in.
#[derive(Clone)]
pub(crate) struct Cloud {
    shared: std::sync::Arc<Shared>,
    hardware_id: String,
}

impl std::fmt::Debug for Cloud {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Cloud(<redacted>)")
    }
}

async fn send(host: &str, request: Request<Full<Body>>) -> anyhow::Result<(StatusCode, Body)> {
    moli_net::web(host, 443, true, request, TIMEOUT, MAX_ANSWER).await
}

impl Cloud {
    pub(crate) fn new(shared: std::sync::Arc<Shared>, hardware_id: String) -> Self {
        Self {
            shared,
            hardware_id,
        }
    }

    /// Signs in with the account's password (and the code Ring sent, once
    /// it asked for one). The password is never sent anywhere else.
    pub(crate) async fn sign_in(
        &self,
        email: &str,
        password: &str,
        code: Option<&str>,
    ) -> anyhow::Result<String> {
        let body = form(&[
            ("client_id", CLIENT_ID),
            ("grant_type", "password"),
            ("scope", "client"),
            ("username", email),
            ("password", password),
        ]);
        let mut request = Request::builder()
            .method(Method::POST)
            .uri("/oauth/token")
            .header("authorization", CLIENT_BASIC)
            .header("content-type", "application/x-www-form-urlencoded")
            .header("user-agent", USER_AGENT)
            .header("hardware_id", &self.hardware_id)
            .header("2fa-support", "true");
        if let Some(code) = code {
            request = request.header("2fa-code", code);
        }
        let request = request.body(Full::new(Body::from(body)))?;
        let (status, bytes) = send(OAUTH_HOST, request).await.context("ring sign-in")?;
        let tokens = tokens(status, &bytes)?;
        self.adopt(&tokens);
        Ok(tokens.refresh)
    }

    /// Uses a refresh token kept from a previous sign-in.
    pub(crate) fn resume(&self, refresh: String) {
        *self
            .shared
            .refresh
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(refresh);
    }

    /// A refresh token Ring rotated since the last call, to store.
    pub(crate) fn rotated(&self) -> Option<String> {
        self.shared
            .rotated
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    fn adopt(&self, tokens: &Tokens) {
        let lease = Duration::from_secs(tokens.expires_in).saturating_sub(MARGIN);
        *self
            .shared
            .access
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Access {
            token: tokens.access.clone(),
            renew_at: Instant::now() + lease,
        });
        let mut refresh = self
            .shared
            .refresh
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if refresh.as_deref() != Some(tokens.refresh.as_str()) {
            *refresh = Some(tokens.refresh.clone());
            *self
                .shared
                .rotated
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(tokens.refresh.clone());
        }
    }

    fn forget_access(&self) {
        *self
            .shared
            .access
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
    }

    /// A valid access token, renewed with the refresh token when due.
    async fn access(&self) -> anyhow::Result<String> {
        {
            let access = self
                .shared
                .access
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if let Some(a) = access.as_ref().filter(|a| Instant::now() < a.renew_at) {
                return Ok(a.token.clone());
            }
        }
        let refresh = self
            .shared
            .refresh
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .ok_or(Refused::SignedOut)?;
        let body = form(&[
            ("client_id", CLIENT_ID),
            ("grant_type", "refresh_token"),
            ("scope", "client"),
            ("refresh_token", &refresh),
        ]);
        let request = Request::builder()
            .method(Method::POST)
            .uri("/oauth/token")
            .header("authorization", CLIENT_BASIC)
            .header("content-type", "application/x-www-form-urlencoded")
            .header("user-agent", USER_AGENT)
            .header("hardware_id", &self.hardware_id)
            .body(Full::new(Body::from(body)))?;
        let (status, bytes) = send(OAUTH_HOST, request).await.context("ring token")?;
        if status.is_server_error() {
            bail!("ring token: {status}");
        }
        // Anything else that is not tokens: the grant is gone.
        let tokens = tokens(status, &bytes).map_err(|_| Refused::SignedOut)?;
        self.adopt(&tokens);
        Ok(tokens.access)
    }

    /// One call to the clients API. A 401 renews the token once.
    pub(crate) async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<&Json>,
    ) -> anyhow::Result<(StatusCode, Body)> {
        for attempt in 0..2 {
            let token = self.access().await?;
            let mut request = Request::builder()
                .method(method.clone())
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .header("user-agent", USER_AGENT)
                .header("hardware_id", &self.hardware_id)
                .header("accept", "application/json");
            // Ring wants a JSON body (`null` at least) on every PUT.
            let body = match (body, &method) {
                (Some(body), _) => Some(body.to_string()),
                (None, &Method::PUT) => Some("null".to_owned()),
                (None, _) => None,
            };
            if body.is_some() {
                request = request.header("content-type", "application/json");
            }
            let request = request.body(Full::new(Body::from(body.unwrap_or_default())))?;
            let (status, bytes) = send(API_HOST, request)
                .await
                .with_context(|| format!("ring {method}"))?;
            if status == StatusCode::UNAUTHORIZED && attempt == 0 {
                self.forget_access();
                continue;
            }
            return Ok((status, bytes));
        }
        Err(Refused::SignedOut.into())
    }

    /// A call whose answer must be a success, as JSON (`null` when empty).
    pub(crate) async fn json(
        &self,
        method: Method,
        path: &str,
        body: Option<&Json>,
    ) -> anyhow::Result<Json> {
        let (status, bytes) = self.call(method.clone(), path, body).await?;
        if !status.is_success() {
            // The path holds only ids: safe to say.
            bail!("ring {method} {path}: {status}");
        }
        if bytes.is_empty() {
            return Ok(Json::Null);
        }
        serde_json::from_slice(&bytes).with_context(|| format!("ring {path}: not JSON"))
    }

    /// Declares this client to Ring once (as the app does): what the
    /// account lists among its authorized devices.
    pub(crate) async fn open_session(&self) -> anyhow::Result<()> {
        if self.shared.session.load(Ordering::Relaxed) {
            return Ok(());
        }
        let body = json!({ "device": {
            "hardware_id": self.hardware_id,
            "metadata": { "api_version": API_VERSION, "device_model": USER_AGENT },
            "os": "android",
        }});
        self.json(Method::POST, "/clients_api/session", Some(&body))
            .await?;
        self.shared.session.store(true, Ordering::Relaxed);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_oauth_answers() {
        let ok = br#"{"access_token":"a","token_type":"bearer","expires_in":3600,"refresh_token":"r","scope":"client"}"#;
        assert_eq!(
            tokens(StatusCode::OK, ok),
            Ok(Tokens {
                access: "a".into(),
                refresh: "r".into(),
                expires_in: 3600
            })
        );
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../integrations/ring/fixtures/ring_oauth_2fa.json");
        let code = std::fs::read(path).unwrap();
        assert_eq!(
            tokens(StatusCode::PRECONDITION_FAILED, &code),
            Err(Refused::NeedsCode)
        );
        assert_eq!(
            tokens(StatusCode::OK, &code),
            Err(Refused::NeedsCode),
            "the state alone says it"
        );
        assert_eq!(
            tokens(StatusCode::UNAUTHORIZED, br#"{"error":"invalid_grant"}"#),
            Err(Refused::Credentials)
        );
        assert_eq!(
            tokens(StatusCode::OK, br#"{"access_token":""}"#),
            Err(Refused::Credentials)
        );
    }

    #[test]
    fn encodes_forms() {
        assert_eq!(
            form(&[("username", "user@example.com"), ("password", "a b&c=d/é")]),
            "username=user%40example.com&password=a%20b%26c%3Dd%2F%C3%A9"
        );
    }

    #[test]
    fn rotation_is_handed_over_once() {
        let cloud = Cloud::new(std::sync::Arc::default(), "hw".into());
        cloud.resume("old".into());
        cloud.adopt(&Tokens {
            access: "a".into(),
            refresh: "old".into(),
            expires_in: 3600,
        });
        assert_eq!(cloud.rotated(), None, "same token: nothing to store");
        cloud.adopt(&Tokens {
            access: "b".into(),
            refresh: "new".into(),
            expires_in: 3600,
        });
        assert_eq!(cloud.rotated().as_deref(), Some("new"));
        assert_eq!(cloud.rotated(), None);
        assert!(!format!("{cloud:?}").contains("new"), "never printed");
    }
}
