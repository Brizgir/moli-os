//! Netatmo's API with an access token kept fresh: renewed before it ends
//! (and once more if Netatmo refuses it), the rotated refresh token filed
//! at once, requests spaced out, and a pause that doubles when Netatmo says
//! the quota is reached (nothing is sent during it).

use std::time::Duration;

use http::{Method, Request};
use http_body_util::Full;
use moli_net::Body as Bytes;
use moli_runtime::BoxFuture;
use serde_json::Value as Json;
use tokio::time::Instant;

use crate::auth::{self, App, Refused};

const LIMIT: Duration = Duration::from_secs(15);
const MAX_BODY: usize = 2 * 1024 * 1024;
/// Never two requests closer than this: no burst.
const GAP: Duration = Duration::from_secs(1);
/// The access token is renewed this long before it ends.
const RENEW_BEFORE: Duration = Duration::from_secs(5 * 60);
/// The first pause once the quota is reached, then doubled up to the max.
pub(crate) const QUOTA_MIN: Duration = Duration::from_secs(60);
const QUOTA_MAX: Duration = Duration::from_secs(30 * 60);

/// One answer: its HTTP status and its JSON (`null` when it has none).
#[derive(Debug)]
pub(crate) struct Answer {
    pub status: u16,
    pub body: Json,
}

/// How requests reach Netatmo (HTTPS for real, a script in the tests).
pub(crate) trait Transport: Send + Sync {
    fn send<'a>(
        &'a self,
        method: Method,
        path: &'a str,
        bearer: Option<&'a str>,
        form: Option<String>,
    ) -> BoxFuture<'a, anyhow::Result<Answer>>;
}

/// Where the newest refresh token is filed (the encrypted store).
pub(crate) trait Vault: Send + Sync {
    fn keep<'a>(&'a self, refresh: &'a str) -> BoxFuture<'a, anyhow::Result<()>>;
}

/// Netatmo itself, over HTTPS.
#[derive(Debug)]
pub(crate) struct Https;

impl Transport for Https {
    fn send<'a>(
        &'a self,
        method: Method,
        path: &'a str,
        bearer: Option<&'a str>,
        form: Option<String>,
    ) -> BoxFuture<'a, anyhow::Result<Answer>> {
        Box::pin(async move {
            let mut builder = Request::builder()
                .method(method)
                .uri(path)
                .header("accept", "application/json")
                .header("user-agent", "Moli OS");
            if let Some(token) = bearer {
                builder = builder.header("authorization", format!("Bearer {token}"));
            }
            if form.is_some() {
                builder = builder.header(
                    "content-type",
                    "application/x-www-form-urlencoded;charset=UTF-8",
                );
            }
            let request = builder.body(Full::new(Bytes::from(
                form.unwrap_or_default().into_bytes(),
            )))?;
            let (status, body) =
                moli_net::web(auth::HOST, 443, true, request, LIMIT, MAX_BODY).await?;
            Ok(Answer {
                status: status.as_u16(),
                body: serde_json::from_slice(&body).unwrap_or(Json::Null),
            })
        })
    }
}

/// Why a request gave nothing.
#[derive(Debug)]
pub(crate) enum Failure {
    /// The refresh token no longer works: the person signs in again.
    SignInAgain(String),
    /// The app's keys are refused: the person checks them.
    BadApp(String),
    /// Netatmo limits the requests: nothing is sent for this long.
    Quota(Duration),
    /// An order that waited too long: never sent.
    Late,
    /// Netatmo refused the request itself (a value, a room…).
    Refused(String),
    /// Netatmo unreachable or failing: tried again later.
    Network(anyhow::Error),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SignInAgain(why) => write!(f, "Netatmo wants a new sign-in ({why})"),
            Self::BadApp(why) => write!(f, "Netatmo refuses the app's keys ({why})"),
            Self::Quota(wait) => {
                write!(f, "Netatmo's quota reached, quiet for {}s", wait.as_secs())
            }
            Self::Late => f.write_str("the order came too late, not sent"),
            Self::Refused(why) => write!(f, "Netatmo refused: {why}"),
            Self::Network(e) => write!(f, "{e:#}"),
        }
    }
}

/// What an API answer says.
#[derive(Debug, PartialEq)]
enum Verdict {
    Fine,
    /// The access token is refused (codes 2 and 3): renew it.
    Expired,
    /// Code 26, or HTTP 429.
    Quota,
    Refused(String),
    Server(String),
}

fn verdict(answer: &Answer) -> Verdict {
    let code = answer.body["error"]["code"].as_i64();
    let message = answer.body["error"]["message"]
        .as_str()
        .or_else(|| answer.body["error"].as_str())
        .unwrap_or("no message");
    if answer.status == 429 || code == Some(26) {
        return Verdict::Quota;
    }
    if matches!(code, Some(2 | 3)) || answer.status == 401 {
        return Verdict::Expired;
    }
    if answer.status >= 500 {
        return Verdict::Server(format!("HTTP {}: {message}", answer.status));
    }
    if (200..300).contains(&answer.status) && code.is_none() {
        return Verdict::Fine;
    }
    Verdict::Refused(match code {
        Some(code) => format!("{message} (code {code})"),
        None => format!("{message} (HTTP {})", answer.status),
    })
}

pub(crate) struct Client<T> {
    transport: T,
    app: App,
    refresh: String,
    access: Option<(String, Instant)>,
    quiet_until: Option<Instant>,
    pause: Duration,
    last: Option<Instant>,
}

impl<T> std::fmt::Debug for Client<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("app", &self.app)
            .finish_non_exhaustive()
    }
}

impl<T: Transport> Client<T> {
    pub(crate) fn new(transport: T, app: App, refresh: String) -> Self {
        Self {
            transport,
            app,
            refresh,
            access: None,
            quiet_until: None,
            pause: QUOTA_MIN,
            last: None,
        }
    }

    /// How long Netatmo still wants quiet, if it does.
    pub(crate) fn quiet_for(&self) -> Option<Duration> {
        let until = self.quiet_until?;
        let now = Instant::now();
        (now < until).then(|| until - now)
    }

    fn quiet(&self) -> Result<(), Failure> {
        self.quiet_for()
            .map_or(Ok(()), |wait| Err(Failure::Quota(wait)))
    }

    fn slow_down(&mut self) -> Failure {
        self.quiet_until = Some(Instant::now() + self.pause);
        let failure = Failure::Quota(self.pause);
        self.pause = (self.pause * 2).min(QUOTA_MAX);
        failure
    }

    fn calm(&mut self) {
        self.pause = QUOTA_MIN;
        self.quiet_until = None;
    }

    async fn pace(&mut self) {
        if let Some(last) = self.last {
            tokio::time::sleep_until(last + GAP).await;
        }
        self.last = Some(Instant::now());
    }

    /// A valid access token, renewed when it ends soon; a new refresh token
    /// is filed before anything else uses it.
    async fn access(&mut self, vault: &dyn Vault) -> Result<String, Failure> {
        if let Some((token, until)) = &self.access
            && Instant::now() + RENEW_BEFORE < *until
        {
            return Ok(token.clone());
        }
        self.quiet()?;
        self.pace().await;
        let form = auth::refresh_form(&self.app, &self.refresh);
        let answer = self
            .transport
            .send(Method::POST, auth::TOKEN_PATH, None, Some(form))
            .await
            .map_err(Failure::Network)?;
        match auth::tokens(answer.status, &answer.body, Some(&self.refresh)) {
            Ok(tokens) => {
                if tokens.refresh != self.refresh {
                    vault
                        .keep(&tokens.refresh)
                        .await
                        .map_err(Failure::Network)?;
                    self.refresh = tokens.refresh;
                }
                self.access = Some((tokens.access.clone(), Instant::now() + tokens.lifetime));
                self.calm();
                Ok(tokens.access)
            }
            Err(Refused::Quota) => Err(self.slow_down()),
            Err(Refused::SignInAgain(why)) => Err(Failure::SignInAgain(why)),
            Err(Refused::BadApp(why)) => Err(Failure::BadApp(why)),
            Err(Refused::Other(why)) => {
                Err(Failure::Network(anyhow::anyhow!("token renewal: {why}")))
            }
        }
    }

    /// One request; `wanted` is asked right before it leaves (an order
    /// nobody waits for any more is never sent).
    pub(crate) async fn call(
        &mut self,
        vault: &dyn Vault,
        method: Method,
        path: &str,
        form: Option<String>,
        wanted: &(dyn Fn() -> bool + Sync),
    ) -> Result<Json, Failure> {
        for attempt in 0..2 {
            let token = self.access(vault).await?;
            self.quiet()?;
            self.pace().await;
            if !wanted() {
                return Err(Failure::Late);
            }
            let answer = self
                .transport
                .send(method.clone(), path, Some(&token), form.clone())
                .await
                .map_err(Failure::Network)?;
            match verdict(&answer) {
                Verdict::Fine => {
                    self.calm();
                    return Ok(answer.body);
                }
                Verdict::Expired if attempt == 0 => self.access = None,
                Verdict::Expired => break,
                Verdict::Quota => return Err(self.slow_down()),
                Verdict::Refused(why) => return Err(Failure::Refused(why)),
                Verdict::Server(why) => return Err(Failure::Network(anyhow::anyhow!(why))),
            }
        }
        Err(Failure::Network(anyhow::anyhow!(
            "Netatmo refused a freshly renewed access token"
        )))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use serde_json::json;

    use super::*;

    /// One request: its path, its bearer token, its form.
    type Sent = (String, Option<String>, Option<String>);

    /// Answers given in order; every request is written down.
    #[derive(Default)]
    pub(crate) struct Script {
        answers: Mutex<VecDeque<(u16, Json)>>,
        pub sent: Mutex<Vec<Sent>>,
    }

    impl Script {
        pub(crate) fn new(answers: Vec<(u16, Json)>) -> Self {
            Self {
                answers: Mutex::new(answers.into()),
                sent: Mutex::default(),
            }
        }

        pub(crate) fn paths(&self) -> Vec<String> {
            self.sent
                .lock()
                .unwrap()
                .iter()
                .map(|s| s.0.clone())
                .collect()
        }
    }

    impl Transport for &Script {
        fn send<'a>(
            &'a self,
            _method: Method,
            path: &'a str,
            bearer: Option<&'a str>,
            form: Option<String>,
        ) -> BoxFuture<'a, anyhow::Result<Answer>> {
            self.sent
                .lock()
                .unwrap()
                .push((path.to_owned(), bearer.map(str::to_owned), form));
            let next = self.answers.lock().unwrap().pop_front();
            Box::pin(async move {
                let (status, body) = next.ok_or_else(|| anyhow::anyhow!("no answer left"))?;
                Ok(Answer { status, body })
            })
        }
    }

    #[derive(Default)]
    pub(crate) struct Kept(pub Mutex<Vec<String>>);

    impl Vault for Kept {
        fn keep<'a>(&'a self, refresh: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
            self.0.lock().unwrap().push(refresh.to_owned());
            Box::pin(async { Ok(()) })
        }
    }

    pub(crate) fn app() -> App {
        App {
            client_id: "client-id-example".into(),
            client_secret: "client-secret-example".into(),
        }
    }

    pub(crate) fn token(access: &str, refresh: &str) -> (u16, Json) {
        (
            200,
            json!({ "access_token": access, "refresh_token": refresh, "expires_in": 10800 }),
        )
    }

    fn ok() -> (u16, Json) {
        (200, json!({ "status": "ok", "body": {} }))
    }

    const YES: &(dyn Fn() -> bool + Sync) = &|| true;

    #[tokio::test(start_paused = true)]
    async fn the_rotated_refresh_token_is_filed_before_use() {
        let script = Script::new(vec![token("access-1", "refresh-2"), ok(), ok()]);
        let kept = Kept::default();
        let mut client = Client::new(&script, app(), "refresh-1".into());
        client
            .call(&kept, Method::GET, "/api/homesdata", None, YES)
            .await
            .unwrap();
        assert_eq!(*kept.0.lock().unwrap(), ["refresh-2"]);
        let sent = script.sent.lock().unwrap().clone();
        assert!(
            sent[0]
                .2
                .as_deref()
                .unwrap()
                .contains("refresh_token=refresh-1")
        );
        assert_eq!(sent[1].1.as_deref(), Some("access-1"));
        drop(sent);
        // The token is good for three hours: no renewal for the next call.
        client
            .call(&kept, Method::GET, "/api/homestatus", None, YES)
            .await
            .unwrap();
        assert_eq!(
            script.paths(),
            ["/oauth2/token", "/api/homesdata", "/api/homestatus"]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_ending_token_is_renewed_and_a_refused_one_once_more() {
        let script = Script::new(vec![
            token("access-1", "refresh-1"),
            ok(),
            // Three hours later, minus less than five minutes: renewed.
            token("access-2", "refresh-2"),
            (
                403,
                json!({ "error": { "code": 3, "message": "Access token expired" } }),
            ),
            token("access-3", "refresh-3"),
            ok(),
        ]);
        let kept = Kept::default();
        let mut client = Client::new(&script, app(), "refresh-1".into());
        client
            .call(&kept, Method::GET, "/api/homestatus", None, YES)
            .await
            .unwrap();
        assert!(
            kept.0.lock().unwrap().is_empty(),
            "same token: nothing filed"
        );
        tokio::time::advance(Duration::from_secs(10800 - 200)).await;
        client
            .call(&kept, Method::GET, "/api/homestatus", None, YES)
            .await
            .unwrap();
        assert_eq!(*kept.0.lock().unwrap(), ["refresh-2", "refresh-3"]);
        let sent = script.sent.lock().unwrap().clone();
        assert_eq!(sent[5].1.as_deref(), Some("access-3"));
        assert!(
            sent[4]
                .2
                .as_deref()
                .unwrap()
                .contains("refresh_token=refresh-2")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_refused_refresh_token_asks_for_a_new_sign_in() {
        let script = Script::new(vec![(400, json!({ "error": "invalid_grant" }))]);
        let mut client = Client::new(&script, app(), "refresh-1".into());
        let failure = client
            .call(&Kept::default(), Method::GET, "/api/homesdata", None, YES)
            .await
            .unwrap_err();
        assert!(matches!(failure, Failure::SignInAgain(_)), "{failure}");
        let script = Script::new(vec![(400, json!({ "error": "invalid_client" }))]);
        let mut client = Client::new(&script, app(), "refresh-1".into());
        let failure = client
            .call(&Kept::default(), Method::GET, "/api/homesdata", None, YES)
            .await
            .unwrap_err();
        assert!(matches!(failure, Failure::BadApp(_)), "{failure}");
    }

    #[tokio::test(start_paused = true)]
    async fn the_quota_silences_the_client_longer_each_time() {
        let quota = || {
            (
                403,
                json!({ "error": { "code": 26, "message": "User usage reached" } }),
            )
        };
        let script = Script::new(vec![token("a", "r"), quota(), quota(), ok()]);
        let kept = Kept::default();
        let mut client = Client::new(&script, app(), "r".into());
        let call = async |client: &mut Client<&Script>| {
            client
                .call(&kept, Method::GET, "/api/homestatus", None, YES)
                .await
        };
        assert!(matches!(call(&mut client).await, Err(Failure::Quota(d)) if d == QUOTA_MIN));
        // Quiet: nothing leaves.
        assert!(matches!(call(&mut client).await, Err(Failure::Quota(_))));
        assert_eq!(script.paths().len(), 2);
        tokio::time::advance(QUOTA_MIN).await;
        assert!(matches!(call(&mut client).await, Err(Failure::Quota(d)) if d == QUOTA_MIN * 2));
        tokio::time::advance(QUOTA_MIN * 2).await;
        call(&mut client).await.unwrap();
        assert_eq!(client.quiet_for(), None);
        assert_eq!(client.pause, QUOTA_MIN);
    }

    #[tokio::test(start_paused = true)]
    async fn an_order_nobody_waits_for_is_never_sent() {
        let script = Script::new(vec![token("a", "r")]);
        let mut client = Client::new(&script, app(), "r".into());
        let failure = client
            .call(
                &Kept::default(),
                Method::POST,
                "/api/setroomthermpoint",
                Some("x=1".into()),
                &|| false,
            )
            .await
            .unwrap_err();
        assert!(matches!(failure, Failure::Late));
        assert_eq!(script.paths(), ["/oauth2/token"]);
    }

    #[tokio::test(start_paused = true)]
    async fn requests_are_spaced_out() {
        let script = Script::new(vec![token("a", "r"), ok(), ok()]);
        let kept = Kept::default();
        let mut client = Client::new(&script, app(), "r".into());
        let started = Instant::now();
        for _ in 0..2 {
            client
                .call(&kept, Method::GET, "/api/homestatus", None, YES)
                .await
                .unwrap();
        }
        assert!(started.elapsed() >= GAP * 2, "{:?}", started.elapsed());
    }

    #[test]
    fn answers_are_read_by_code_then_status() {
        let answer = |status, body| Answer { status, body };
        assert_eq!(
            verdict(&answer(200, json!({ "status": "ok" }))),
            Verdict::Fine
        );
        assert_eq!(
            verdict(&answer(
                403,
                json!({ "error": { "code": 2, "message": "Invalid access token" } })
            )),
            Verdict::Expired
        );
        assert_eq!(verdict(&answer(429, Json::Null)), Verdict::Quota);
        assert!(matches!(
            verdict(&answer(400, json!({ "error": { "code": 21, "message": "Invalid argument" } }))),
            Verdict::Refused(m) if m.contains("Invalid argument")
        ));
        assert!(matches!(
            verdict(&answer(503, Json::Null)),
            Verdict::Server(_)
        ));
    }
}
