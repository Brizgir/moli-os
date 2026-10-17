//! Ring's token endpoint (`oauth.ring.com`, client `ring_official_android`),
//! asked with a refresh token only: the one a person pasted from the
//! dashboard (made on their own computer, where Ring itself took the
//! e-mail, the password and the one-time code), then the one Ring gives
//! back at every renewal. No password ever goes through here. The hardware
//! id names Moli among the account's « authorized client devices »: the
//! same one for good, so the account does not fill up with Molis.
//!
//! Interface facts from the open projects that use the same API
//! (ring-client-api, python-ring-doorbell); no code of theirs.

use std::fmt::Write as _;
use std::time::Duration;

use http::{Method, Request, StatusCode};
use http_body_util::Full;
use moli_net::Body as Bytes;
use ring::rand::{SecureRandom as _, SystemRandom};
use serde_json::{Value as Json, json};

use crate::net::Transport;

pub(crate) const OAUTH_HOST: &str = "oauth.ring.com";
pub(crate) const USER_AGENT: &str = "android:com.ringapp";
const CLIENT_ID: &str = "ring_official_android";

/// A new hardware id: a random UUID (version 4), as the app makes one.
pub(crate) fn hardware_id() -> anyhow::Result<String> {
    let mut b = [0u8; 16];
    SystemRandom::new()
        .fill(&mut b)
        .map_err(|_| anyhow::anyhow!("no randomness"))?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex = b.iter().fold(String::new(), |mut hex, x| {
        let _ = write!(hex, "{x:02x}");
        hex
    });
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}

/// What the account gives once signed in.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Tokens {
    pub access: String,
    pub refresh: String,
    /// How long the access token lasts.
    pub lasts: Duration,
}

impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the tokens themselves.
        f.debug_struct("Tokens")
            .field("lasts", &self.lasts)
            .finish_non_exhaustive()
    }
}

/// Ring's answer to a renewal.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    Tokens(Tokens),
    /// Ring no longer accepts the refresh token: sign in again.
    Revoked,
    /// Too many tries: Ring makes one wait (about ten minutes).
    TooMany,
    Other(String),
}

/// Asks `oauth.ring.com` for tokens, with a refresh token.
pub(crate) async fn token(
    net: &dyn Transport,
    hardware_id: &str,
    refresh: &str,
) -> anyhow::Result<Answer> {
    let body = json!({
        "client_id": CLIENT_ID,
        "scope": "client",
        "grant_type": "refresh_token",
        "refresh_token": refresh,
    });
    let request = Request::builder()
        .method(Method::POST)
        .uri("/oauth/token")
        .header("content-type", "application/json")
        .header("accept", "application/json")
        .header("user-agent", USER_AGENT)
        .header("hardware_id", hardware_id)
        .header("2fa-support", "true")
        .body(Full::new(Bytes::from(body.to_string().into_bytes())))?;
    let (status, body) = net.send(OAUTH_HOST, request).await?;
    Ok(interpret(
        status,
        &serde_json::from_slice(&body).unwrap_or(Json::Null),
    ))
}

/// Ring's answer, read. Never quotes the request (a token lives in it).
pub(crate) fn interpret(status: StatusCode, body: &Json) -> Answer {
    let description = body["error_description"]
        .as_str()
        .or_else(|| body["err_msg"].as_str())
        .unwrap_or_default();
    let error = body["error"].as_str().unwrap_or_default();
    match status.as_u16() {
        200 => match (
            body["access_token"].as_str(),
            body["refresh_token"].as_str(),
        ) {
            (Some(access), Some(refresh)) if !access.is_empty() && !refresh.is_empty() => {
                Answer::Tokens(Tokens {
                    access: access.to_owned(),
                    refresh: refresh.to_owned(),
                    lasts: Duration::from_secs(body["expires_in"].as_u64().unwrap_or(3600)),
                })
            }
            _ => Answer::Other("no tokens in Ring's answer".into()),
        },
        429 => Answer::TooMany,
        _ if description.to_lowercase().contains("too many") => Answer::TooMany,
        // 412 (a verification code wanted) never comes with a refresh token
        // that Ring still accepts: the person makes a new one.
        400 | 401 | 412 => Answer::Revoked,
        code => Answer::Other(format!(
            "HTTP {code} {}",
            if description.is_empty() {
                error
            } else {
                description
            }
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::replay::Replay;

    fn fixture(name: &str) -> Json {
        let text = match name {
            "token" => include_str!("../../../integrations/ring/fixtures/oauth-token.json"),
            "bad-refresh" => {
                include_str!("../../../integrations/ring/fixtures/oauth-bad-refresh.json")
            }
            _ => unreachable!(),
        };
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn ring_s_answers_read() {
        let Answer::Tokens(t) = interpret(StatusCode::OK, &fixture("token")) else {
            panic!()
        };
        assert_eq!(t.lasts, Duration::from_secs(3600));
        assert!(t.refresh.starts_with("fake-refresh"));
        assert!(!format!("{t:?}").contains("fake"), "{t:?}");
        assert_eq!(
            interpret(StatusCode::UNAUTHORIZED, &fixture("bad-refresh")),
            Answer::Revoked
        );
        assert_eq!(
            interpret(StatusCode::BAD_REQUEST, &fixture("bad-refresh")),
            Answer::Revoked
        );
        assert_eq!(
            interpret(
                StatusCode::PRECONDITION_FAILED,
                &json!({ "tsv_state": "sms" })
            ),
            Answer::Revoked
        );
        assert_eq!(
            interpret(StatusCode::TOO_MANY_REQUESTS, &Json::Null),
            Answer::TooMany
        );
        assert!(matches!(
            interpret(StatusCode::OK, &json!({ "access_token": "a" })),
            Answer::Other(_)
        ));
        assert!(matches!(
            interpret(StatusCode::BAD_GATEWAY, &Json::Null),
            Answer::Other(_)
        ));
    }

    #[test]
    fn a_hardware_id_is_a_random_uuid() {
        let (a, b) = (hardware_id().unwrap(), hardware_id().unwrap());
        assert_ne!(a, b);
        assert_eq!(a.len(), 36);
        assert_eq!(&a[14..15], "4");
        assert_eq!(a.matches('-').count(), 4);
    }

    #[tokio::test]
    async fn only_a_refresh_token_ever_goes_to_ring() {
        let net = Replay::default().then(
            "/oauth/token",
            200,
            include_str!("../../../integrations/ring/fixtures/oauth-token.json"),
        );
        assert!(matches!(
            token(&net, "hw-test", "fake-refresh-0").await.unwrap(),
            Answer::Tokens(_)
        ));
        let (path, headers, body) = net.asked(0);
        assert_eq!(path, "/oauth/token");
        assert_eq!(net.host(0), OAUTH_HOST);
        assert_eq!(body["grant_type"], "refresh_token");
        assert_eq!(body["refresh_token"], "fake-refresh-0");
        assert!(body.get("password").is_none() && body.get("username").is_none());
        assert_eq!(headers["hardware_id"], "hw-test");
        assert!(headers.get("2fa-code").is_none());
    }
}
