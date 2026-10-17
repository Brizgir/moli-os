//! Signing in to Ring as its Android app does (`oauth.ring.com`, client
//! `ring_official_android`): once with the e-mail and password a person
//! typed in the dashboard, then the one-time code Ring sends (SMS, e-mail or
//! an authenticator app); afterwards only the refresh token, which Ring
//! replaces at every renewal. The hardware id names Moli among the
//! account's « authorized client devices »: the same one for good, so the
//! account does not fill up with Molis.
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

/// Ring's answer to a sign-in or a renewal.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    Tokens(Tokens),
    /// A one-time code is needed; Ring sent it by `sms`, `email`, or the
    /// person reads it in an authenticator app (`totp`).
    Code {
        by: String,
    },
    /// The code typed is wrong or expired.
    BadCode,
    /// The e-mail or the password is wrong.
    BadPassword,
    /// Ring no longer accepts the refresh token: sign in again.
    Revoked,
    /// Too many tries: Ring makes one wait (about ten minutes for codes).
    TooMany,
    Other(String),
}

/// How the account is asked.
pub(crate) enum Grant<'a> {
    Password {
        email: &'a str,
        password: &'a str,
        code: Option<&'a str>,
    },
    Refresh(&'a str),
}

impl Grant<'_> {
    fn body(&self) -> Json {
        match self {
            Self::Password {
                email, password, ..
            } => json!({
                "client_id": CLIENT_ID,
                "scope": "client",
                "grant_type": "password",
                "username": email,
                "password": password,
            }),
            Self::Refresh(token) => json!({
                "client_id": CLIENT_ID,
                "scope": "client",
                "grant_type": "refresh_token",
                "refresh_token": token,
            }),
        }
    }

    fn code(&self) -> &str {
        match self {
            Self::Password { code, .. } => code.unwrap_or_default(),
            Self::Refresh(_) => "",
        }
    }
}

/// Asks `oauth.ring.com` for tokens.
pub(crate) async fn token(
    net: &dyn Transport,
    hardware_id: &str,
    grant: &Grant<'_>,
) -> anyhow::Result<Answer> {
    let request = Request::builder()
        .method(Method::POST)
        .uri("/oauth/token")
        .header("content-type", "application/json")
        .header("accept", "application/json")
        .header("user-agent", USER_AGENT)
        .header("hardware_id", hardware_id)
        .header("2fa-support", "true")
        .header("2fa-code", grant.code())
        .body(Full::new(Bytes::from(
            grant.body().to_string().into_bytes(),
        )))?;
    let (status, body) = net.send(OAUTH_HOST, request).await?;
    Ok(interpret(
        status,
        &serde_json::from_slice(&body).unwrap_or(Json::Null),
        matches!(grant, Grant::Refresh(_)),
    ))
}

/// Ring's answer, read. `refreshing`: the grant was a refresh token.
pub(crate) fn interpret(status: StatusCode, body: &Json, refreshing: bool) -> Answer {
    let description = body["error_description"]
        .as_str()
        .or_else(|| body["err_msg"].as_str())
        .unwrap_or_default();
    let error = body["error"].as_str().unwrap_or_default();
    let lower = description.to_lowercase();
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
        412 => Answer::Code {
            by: body["tsv_state"].as_str().unwrap_or("sms").to_owned(),
        },
        429 => Answer::TooMany,
        _ if lower.contains("too many") => Answer::TooMany,
        400 if lower.contains("verification code") => Answer::BadCode,
        400 | 401 if refreshing => Answer::Revoked,
        400 | 401 if error == "invalid_grant" || error == "access_denied" || status == 401 => {
            Answer::BadPassword
        }
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

    fn fixture(name: &str) -> Json {
        let text = match name {
            "token" => include_str!("../../../integrations/ring/fixtures/oauth-token.json"),
            "2fa" => include_str!("../../../integrations/ring/fixtures/oauth-2fa.json"),
            "bad-code" => include_str!("../../../integrations/ring/fixtures/oauth-bad-code.json"),
            "bad-password" => {
                include_str!("../../../integrations/ring/fixtures/oauth-bad-password.json")
            }
            "bad-refresh" => {
                include_str!("../../../integrations/ring/fixtures/oauth-bad-refresh.json")
            }
            _ => unreachable!(),
        };
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn ring_s_answers_read() {
        let Answer::Tokens(t) = interpret(StatusCode::OK, &fixture("token"), false) else {
            panic!()
        };
        assert_eq!(t.lasts, Duration::from_secs(3600));
        assert!(t.refresh.starts_with("fake-refresh"));
        assert!(!format!("{t:?}").contains("fake"), "{t:?}");
        assert_eq!(
            interpret(StatusCode::PRECONDITION_FAILED, &fixture("2fa"), false),
            Answer::Code { by: "sms".into() }
        );
        assert_eq!(
            interpret(StatusCode::BAD_REQUEST, &fixture("bad-code"), false),
            Answer::BadCode
        );
        assert_eq!(
            interpret(StatusCode::UNAUTHORIZED, &fixture("bad-password"), false),
            Answer::BadPassword
        );
        assert_eq!(
            interpret(StatusCode::UNAUTHORIZED, &fixture("bad-refresh"), true),
            Answer::Revoked
        );
        assert_eq!(
            interpret(StatusCode::TOO_MANY_REQUESTS, &Json::Null, false),
            Answer::TooMany
        );
        assert!(matches!(
            interpret(StatusCode::OK, &json!({ "access_token": "a" }), false),
            Answer::Other(_)
        ));
        assert!(matches!(
            interpret(StatusCode::BAD_GATEWAY, &Json::Null, true),
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

    #[test]
    fn the_code_rides_in_a_header_and_never_with_a_refresh() {
        let grant = Grant::Password {
            email: "user@example.com",
            password: "x",
            code: Some("123456"),
        };
        assert_eq!(grant.code(), "123456");
        assert_eq!(grant.body()["grant_type"], "password");
        assert_eq!(grant.body()["username"], "user@example.com");
        let refresh = Grant::Refresh("r");
        assert_eq!(refresh.code(), "");
        assert_eq!(refresh.body()["refresh_token"], "r");
        assert!(refresh.body().get("password").is_none());
    }
}
