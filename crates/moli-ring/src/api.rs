//! Ring's own API (`api.ring.com/clients_api`), with a short-lived access
//! token: renewed from the refresh token before it ends or when Ring says
//! it has, and every renewal gives a new refresh token, written to the
//! secret store at once (the old one may not work again).

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use http::{Method, Request, StatusCode};
use http_body_util::Full;
use moli_net::Body as Bytes;
use moli_runtime::BoxFuture;
use serde_json::{Value as Json, json};
use tokio::time::Instant;

use crate::auth::{self, Answer, Grant, USER_AGENT};
use crate::net::Transport;

pub(crate) const API_HOST: &str = "api.ring.com";
/// Renewed this long before Ring would refuse it.
const MARGIN: Duration = Duration::from_secs(5 * 60);

/// Where the refresh token goes each time Ring gives a new one.
pub(crate) trait Vault: Send + Sync {
    fn keep<'a>(&'a self, refresh: &'a str) -> BoxFuture<'a, anyhow::Result<()>>;
}

#[derive(Debug)]
pub(crate) enum Failure {
    /// The refresh token no longer works: the person signs in again.
    SignInAgain,
    /// Ring asks to slow down.
    TooMany,
    Other(anyhow::Error),
}

impl From<anyhow::Error> for Failure {
    fn from(e: anyhow::Error) -> Self {
        Self::Other(e)
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SignInAgain => f.write_str("Ring no longer accepts this sign-in"),
            Self::TooMany => f.write_str("Ring asks to slow down"),
            Self::Other(e) => write!(f, "{e:#}"),
        }
    }
}

pub(crate) struct Client {
    net: Arc<dyn Transport>,
    hardware_id: String,
    refresh: String,
    access: Option<(String, Instant)>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client").finish_non_exhaustive()
    }
}

impl Client {
    pub(crate) fn new(net: Arc<dyn Transport>, hardware_id: &str, refresh: &str) -> Self {
        Self {
            net,
            hardware_id: hardware_id.to_owned(),
            refresh: refresh.to_owned(),
            access: None,
        }
    }

    /// Right after a sign-in: the tokens are fresh.
    pub(crate) fn signed_in(
        net: Arc<dyn Transport>,
        hardware_id: &str,
        tokens: &auth::Tokens,
    ) -> Self {
        let mut client = Self::new(net, hardware_id, &tokens.refresh);
        client.access = Some((tokens.access.clone(), Instant::now() + tokens.lasts));
        client
    }

    /// A new access token (and refresh token, kept in `vault` before use).
    async fn renew(&mut self, vault: &dyn Vault) -> Result<(), Failure> {
        let answer = auth::token(
            &*self.net,
            &self.hardware_id,
            &Grant::Refresh(&self.refresh),
        )
        .await?;
        match answer {
            Answer::Tokens(t) => {
                vault
                    .keep(&t.refresh)
                    .await
                    .context("the new Ring token could not be kept")?;
                self.refresh = t.refresh;
                self.access = Some((t.access, Instant::now() + t.lasts));
                Ok(())
            }
            Answer::Revoked | Answer::BadPassword | Answer::Code { .. } | Answer::BadCode => {
                Err(Failure::SignInAgain)
            }
            Answer::TooMany => Err(Failure::TooMany),
            Answer::Other(why) => Err(Failure::Other(anyhow::anyhow!("Ring sign-in: {why}"))),
        }
    }

    async fn bearer(&mut self, vault: &dyn Vault) -> Result<String, Failure> {
        let due = self
            .access
            .as_ref()
            .is_none_or(|(_, until)| Instant::now() + MARGIN >= *until);
        if due {
            self.renew(vault).await?;
        }
        Ok(self
            .access
            .as_ref()
            .map(|(token, _)| token.clone())
            .unwrap_or_default())
    }

    async fn once(
        &mut self,
        vault: &dyn Vault,
        method: &Method,
        path: &str,
        body: Option<&Json>,
    ) -> Result<(StatusCode, Bytes), Failure> {
        let bearer = self.bearer(vault).await?;
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", format!("Bearer {bearer}"))
            .header("hardware_id", self.hardware_id.as_str())
            .header("user-agent", USER_AGENT)
            .header("accept", "application/json");
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        let request = builder
            .body(Full::new(Bytes::from(
                body.map(Json::to_string).unwrap_or_default().into_bytes(),
            )))
            .map_err(|e| Failure::Other(e.into()))?;
        Ok(self.net.send(API_HOST, request).await?)
    }

    /// One call; an access token Ring says is over is renewed once.
    async fn call(
        &mut self,
        vault: &dyn Vault,
        method: Method,
        path: &str,
        body: Option<&Json>,
    ) -> Result<Json, Failure> {
        let (mut status, mut answer) = self.once(vault, &method, path, body).await?;
        if status == StatusCode::UNAUTHORIZED {
            self.access = None;
            (status, answer) = self.once(vault, &method, path, body).await?;
        }
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Err(Failure::TooMany);
        }
        if status == StatusCode::UNAUTHORIZED {
            return Err(Failure::SignInAgain);
        }
        if !status.is_success() {
            let text = String::from_utf8_lossy(&answer[..answer.len().min(200)]).into_owned();
            return Err(Failure::Other(anyhow::anyhow!(
                "{API_HOST}{path}: HTTP {} {text}",
                status.as_u16()
            )));
        }
        Ok(serde_json::from_slice(&answer).unwrap_or(Json::Null))
    }

    pub(crate) async fn get(&mut self, vault: &dyn Vault, path: &str) -> Result<Json, Failure> {
        self.call(vault, Method::GET, path, None).await
    }

    /// Moli as a client of the account, as the app opens one.
    pub(crate) async fn session(&mut self, vault: &dyn Vault) -> Result<(), Failure> {
        let body = json!({
            "device": {
                "hardware_id": self.hardware_id,
                "metadata": { "api_version": 11, "device_model": "Moli OS" },
                "os": "android",
            }
        });
        self.call(vault, Method::POST, "/clients_api/session", Some(&body))
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::net::replay::Replay;

    #[derive(Default)]
    struct Kept(Mutex<Vec<String>>);

    impl Vault for Kept {
        fn keep<'a>(&'a self, refresh: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
            Box::pin(async move {
                self.0.lock().unwrap().push(refresh.to_owned());
                Ok(())
            })
        }
    }

    struct Broken;

    impl Vault for Broken {
        fn keep<'a>(&'a self, _: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
            Box::pin(async { anyhow::bail!("no secret store") })
        }
    }

    fn token(n: u32, lasts: u64) -> String {
        json!({
            "access_token": format!("fake-access-{n}"),
            "refresh_token": format!("fake-refresh-{n}"),
            "expires_in": lasts,
            "scope": "client",
            "token_type": "Bearer",
        })
        .to_string()
    }

    const DEVICES: &str = include_str!("../fixtures/ring-devices.json");

    #[tokio::test(start_paused = true)]
    async fn every_new_refresh_token_is_kept_before_it_is_used() {
        let net = Arc::new(
            Replay::default()
                .then("/oauth/token", 200, &token(1, 3600))
                .then("/clients_api/ring_devices", 200, DEVICES)
                .then("/clients_api/ring_devices", 200, DEVICES)
                // An hour later: the access token is about to end.
                .then("/oauth/token", 200, &token(2, 3600))
                .then("/clients_api/ring_devices", 200, DEVICES),
        );
        let kept = Kept::default();
        let mut client = Client::new(net.clone(), "hw-test", "fake-refresh-0");
        client
            .get(&kept, "/clients_api/ring_devices")
            .await
            .unwrap();
        client
            .get(&kept, "/clients_api/ring_devices")
            .await
            .unwrap();
        tokio::time::advance(Duration::from_secs(3600 - 60)).await;
        let answer = client
            .get(&kept, "/clients_api/ring_devices")
            .await
            .unwrap();
        assert!(answer["doorbots"].is_array());
        assert_eq!(
            *kept.0.lock().unwrap(),
            ["fake-refresh-1", "fake-refresh-2"]
        );
        assert_eq!(net.left(), 0);
        // The first renewal sent the stored token, the second the new one.
        let (path, headers, body) = net.asked(0);
        assert_eq!(path, "/oauth/token");
        assert_eq!(body["refresh_token"], "fake-refresh-0");
        assert_eq!(headers["hardware_id"], "hw-test");
        assert_eq!(net.asked(3).2["refresh_token"], "fake-refresh-1");
        assert_eq!(net.host(0), "oauth.ring.com");
        assert_eq!(net.host(4), "api.ring.com");
        let (_, headers, _) = net.asked(4);
        assert_eq!(headers["authorization"], "Bearer fake-access-2");
        assert_eq!(headers["hardware_id"], "hw-test");
    }

    #[tokio::test(start_paused = true)]
    async fn an_access_token_ring_ended_is_renewed_once() {
        let net = Arc::new(
            Replay::default()
                .then("/clients_api/ring_devices", 401, "")
                .then("/oauth/token", 200, &token(2, 3600))
                .then("/clients_api/ring_devices", 200, DEVICES),
        );
        let kept = Kept::default();
        let first = auth::Tokens {
            access: "fake-access-1".into(),
            refresh: "fake-refresh-1".into(),
            lasts: Duration::from_secs(3600),
        };
        let mut client = Client::signed_in(net.clone(), "hw-test", &first);
        client
            .get(&kept, "/clients_api/ring_devices")
            .await
            .unwrap();
        assert_eq!(*kept.0.lock().unwrap(), ["fake-refresh-2"]);
        assert_eq!(net.asked(1).2["refresh_token"], "fake-refresh-1");
    }

    #[tokio::test(start_paused = true)]
    async fn a_refused_refresh_token_asks_for_a_new_sign_in() {
        let net = Arc::new(Replay::default().then(
            "/oauth/token",
            401,
            include_str!("../fixtures/oauth-bad-refresh.json"),
        ));
        let mut client = Client::new(net, "hw-test", "fake-refresh-0");
        let kept = Kept::default();
        assert!(matches!(
            client.get(&kept, "/clients_api/ring_devices").await,
            Err(Failure::SignInAgain)
        ));
        assert!(kept.0.lock().unwrap().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_token_that_cannot_be_kept_is_not_used() {
        let net = Arc::new(Replay::default().then("/oauth/token", 200, &token(1, 3600)));
        let mut client = Client::new(net.clone(), "hw-test", "fake-refresh-0");
        let e = client
            .get(&Broken, "/clients_api/ring_devices")
            .await
            .unwrap_err();
        assert!(e.to_string().contains("could not be kept"), "{e}");
        assert_eq!(net.left(), 0, "no call with a token that would be lost");
    }

    #[tokio::test(start_paused = true)]
    async fn slow_down_is_its_own_failure() {
        let net = Arc::new(
            Replay::default()
                .then("/oauth/token", 200, &token(1, 3600))
                .then("/clients_api/ring_devices", 429, ""),
        );
        let mut client = Client::new(net, "hw-test", "fake-refresh-0");
        assert!(matches!(
            client
                .get(&Kept::default(), "/clients_api/ring_devices")
                .await,
            Err(Failure::TooMany)
        ));
    }
}
