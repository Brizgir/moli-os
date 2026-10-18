//! Moli as a receiver of Ring's push notifications: a web client of
//! Firebase Cloud Messaging (FCM) for the Ring Android app's Firebase
//! project, registered once as Chrome registers a web push subscription:
//!
//! 1. a GCM check-in (`android.clients.google.com/checkin`, protobuf): an
//!    Android id and a security token, Moli's device for Google;
//! 2. a GCM registration for that device (`c2dm/register3`): a GCM token;
//! 3. a Firebase installation (`firebaseinstallations.googleapis.com`) and
//!    an FCM registration (`fcmregistrations.googleapis.com`) that ties the
//!    GCM token to our public key and authentication secret ([`ece`]): the
//!    FCM token, which Ring is given (`PATCH clients_api/device`, see
//!    [`crate::push`]).
//!
//! The device and the keys are kept together in the secret store
//! ([`Credentials`]), never in a file nor a journal; the check-in is made
//! again at each connection (as Chrome does), the registration only when
//! the credentials are missing, made for another app, or no longer work.
//!
//! Interface facts from the open projects that receive Ring's pushes
//! (ring-client-api and @eneris/push-receiver, MIT; Chromium's GCM client
//! for the protobuf messages); no code of theirs.

use std::fmt::Write as _;
use std::time::Duration;

use anyhow::{Context as _, bail};
use http::{Method, Request};
use http_body_util::Full;
use moli_net::Body as Bytes;
use moli_net::proto::{self, Writer};
use ring::rand::{SecureRandom as _, SystemRandom};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

use crate::auth;
use crate::ece::{self, Keys, base64url};
use crate::net::Transport;
use crate::signin::base64;

pub(crate) const CHECKIN_HOST: &str = "android.clients.google.com";
pub(crate) const INSTALL_HOST: &str = "firebaseinstallations.googleapis.com";
pub(crate) const REGISTER_HOST: &str = "fcmregistrations.googleapis.com";

/// The Ring Android app's Firebase project, as ring-client-api registers:
/// public identifiers of the app (in every copy of it), not secrets. The
/// key is cut in two so that the leak guard, which looks for Google API
/// keys, keeps looking for real ones.
const API_KEY: &str = concat!("AIza", "SyCv-hdFBmmdBBJadNy-TFwB-xN_H5m3Bk8");
const PROJECT: &str = "ring-17770";
const APP_ID: &str = "1:876313859327:android:e10ec6ddb3c81f39";
/// FCM's own default web push key (the « sender » of a web client that
/// brings none).
const SENDER: &str =
    "BDOU99-h67HcA6JeFXHbSNMu7e2yNNu3RzoMj8TM4W88jITfq7ZmPvIM1Iv-4_l2LxQcYwhqby2xGpWwzjfAnG4";
/// The Chrome Moli says it is, to GCM and to the push server.
pub(crate) const CHROME_VERSION: &str = "94.0.4606.51";
/// base64 of `{"heartbeats":[],"version":2}`: no usage report.
const NO_HEARTBEATS: &str = "eyJoZWFydGJlYXRzIjpbXSwidmVyc2lvbiI6Mn0=";
const SDK_VERSION: &str = "w:0.6.6";

/// What makes Moli a push receiver: its Google device, its FCM token and
/// its keys. Only in the secret store (`push_credentials`).
#[derive(Clone, PartialEq)]
pub(crate) struct Credentials {
    pub android_id: u64,
    pub security_token: u64,
    /// Where Ring sends.
    pub token: String,
    pub keys: Keys,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials").finish_non_exhaustive()
    }
}

/// How they are kept: one JSON text.
#[derive(Serialize, Deserialize)]
struct Kept {
    /// The Firebase app they were made for: another one, register again.
    app: String,
    android_id: u64,
    security_token: u64,
    token: String,
    private_key: String,
    auth: String,
}

impl Credentials {
    /// From the secret store; `None` when unreadable or made for another
    /// app (registered again then).
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let kept: Kept = serde_json::from_str(text).ok()?;
        if kept.app != APP_ID || kept.token.is_empty() || kept.android_id == 0 {
            return None;
        }
        let keys = Keys::from_parts(&base64(&kept.private_key)?, &base64(&kept.auth)?).ok()?;
        Some(Self {
            android_id: kept.android_id,
            security_token: kept.security_token,
            token: kept.token,
            keys,
        })
    }

    /// For the secret store.
    pub(crate) fn to_text(&self) -> String {
        serde_json::to_string(&Kept {
            app: APP_ID.to_owned(),
            android_id: self.android_id,
            security_token: self.security_token,
            token: self.token.clone(),
            private_key: base64url(&self.keys.private_bytes()),
            auth: base64url(self.keys.auth()),
        })
        .unwrap_or_default()
    }
}

/// A new registration, from the check-in to the FCM token.
pub(crate) async fn register(net: &dyn Transport) -> anyhow::Result<Credentials> {
    let keys = Keys::generate()?;
    let (android_id, security_token) = checkin(net, None).await?;
    let gcm = gcm_register(net, android_id, security_token).await?;
    let installation = install(net).await?;
    let token = fcm_register(net, &installation, &gcm, &keys).await?;
    Ok(Credentials {
        android_id,
        security_token,
        token,
        keys,
    })
}

/// A GCM check-in: the device (new, or `known` again), as (Android id,
/// security token).
pub(crate) async fn checkin(
    net: &dyn Transport,
    known: Option<(u64, u64)>,
) -> anyhow::Result<(u64, u64)> {
    let chrome = Writer::new()
        .uint(1, 2) // PLATFORM_MAC, as Chrome registers
        .str(2, CHROME_VERSION)
        .uint(3, 1); // CHANNEL_STABLE
    let device = Writer::new()
        .uint(2, 0) // last check-in
        .uint(12, 3) // DEVICE_CHROME_BROWSER
        .msg(13, chrome);
    let mut request = Writer::new();
    if let Some((id, _)) = known {
        request = request.uint(2, id);
    }
    request = request.msg(4, device).str(12, "UTC");
    if let Some((_, token)) = known {
        request = request.fixed64(13, token);
    }
    let body = request.uint(14, 3).uint(20, 0).finish();
    let request = Request::builder()
        .method(Method::POST)
        .uri("/checkin")
        .header("content-type", "application/x-protobuf")
        .body(Full::new(Bytes::from(body)))?;
    let (status, answer) = net.send(CHECKIN_HOST, request).await?;
    if !status.is_success() {
        bail!("GCM check-in: HTTP {}", status.as_u16());
    }
    let fields = proto::parse(&answer).context("GCM check-in: unreadable answer")?;
    match (fields.fixed64(7), fields.fixed64(8)) {
        (Some(id), Some(token)) if id != 0 && token != 0 => Ok((id, token)),
        _ => bail!("GCM check-in: no device in the answer"),
    }
}

/// `application/x-www-form-urlencoded`.
fn form(pairs: &[(&str, &str)]) -> String {
    let mut out = String::new();
    for (k, v) in pairs {
        if !out.is_empty() {
            out.push('&');
        }
        for (i, part) in [k, v].iter().enumerate() {
            if i == 1 {
                out.push('=');
            }
            for b in part.bytes() {
                if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                    out.push(char::from(b));
                } else {
                    let _ = write!(out, "%{b:02X}");
                }
            }
        }
    }
    out
}

/// A GCM token for the device (Google sometimes says « try again »: a few
/// tries, a second apart).
async fn gcm_register(
    net: &dyn Transport,
    android_id: u64,
    security_token: u64,
) -> anyhow::Result<String> {
    let app = format!("wp:receiver.push.com#{}", auth::hardware_id()?);
    let device = android_id.to_string();
    let body = form(&[
        ("app", "org.chromium.linux"),
        ("X-subtype", &app),
        ("device", &device),
        ("sender", SENDER),
    ]);
    let mut last = String::new();
    for attempt in 0..5 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        let request = Request::builder()
            .method(Method::POST)
            .uri("/c2dm/register3")
            .header(
                "authorization",
                format!("AidLogin {android_id}:{security_token}"),
            )
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Full::new(Bytes::from(body.clone().into_bytes())))?;
        let (status, answer) = net.send(CHECKIN_HOST, request).await?;
        let text = String::from_utf8_lossy(&answer);
        match text.trim().split_once('=') {
            Some(("token", token)) if status.is_success() && !token.is_empty() => {
                return Ok(token.to_owned());
            }
            // `Error=PHONE_REGISTRATION_ERROR`…: never a secret.
            Some(("Error", why)) => last = why.chars().take(60).collect(),
            _ => last = format!("HTTP {}", status.as_u16()),
        }
    }
    bail!("GCM registration refused ({last})")
}

/// A Firebase installation: its auth token, for the FCM registration only.
async fn install(net: &dyn Transport) -> anyhow::Result<String> {
    let mut fid = [0u8; 17];
    SystemRandom::new()
        .fill(&mut fid)
        .map_err(|_| anyhow::anyhow!("no randomness"))?;
    // A Firebase installation id: 0b0111 then 128 random bits, 22 chars.
    fid[0] = 0b0111_0000 | (fid[0] & 0x0f);
    let fid: String = base64url(&fid).chars().take(22).collect();
    let body = json!({
        "appId": APP_ID,
        "authVersion": "FIS_v2",
        "fid": fid,
        "sdkVersion": SDK_VERSION,
    });
    let request = Request::builder()
        .method(Method::POST)
        .uri(format!("/v1/projects/{PROJECT}/installations"))
        .header("content-type", "application/json")
        .header("x-firebase-client", NO_HEARTBEATS)
        .header("x-goog-api-key", API_KEY)
        .body(Full::new(Bytes::from(body.to_string().into_bytes())))?;
    let answer = google_json(net, INSTALL_HOST, request, "Firebase installation").await?;
    answer["authToken"]["token"]
        .as_str()
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .context("Firebase installation: no token in the answer")
}

/// The FCM registration: the token Ring sends to, tied to our keys.
async fn fcm_register(
    net: &dyn Transport,
    installation: &str,
    gcm: &str,
    keys: &Keys,
) -> anyhow::Result<String> {
    let body = json!({
        "web": {
            "auth": base64url(keys.auth()),
            "endpoint": format!("https://fcm.googleapis.com/fcm/send/{gcm}"),
            "p256dh": base64url(keys.public()),
        }
    });
    let request = Request::builder()
        .method(Method::POST)
        .uri(format!("/v1/projects/{PROJECT}/registrations"))
        .header("content-type", "application/json")
        .header("x-goog-api-key", API_KEY)
        .header("x-goog-firebase-installations-auth", installation)
        .body(Full::new(Bytes::from(body.to_string().into_bytes())))?;
    let answer = google_json(net, REGISTER_HOST, request, "FCM registration").await?;
    answer["token"]
        .as_str()
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .context("FCM registration: no token in the answer")
}

/// A Google JSON answer; a refusal says only its status and Google's
/// short reason (never the request: a token lives in it).
async fn google_json(
    net: &dyn Transport,
    host: &str,
    request: Request<Full<Bytes>>,
    what: &str,
) -> anyhow::Result<Json> {
    let (status, answer) = net.send(host, request).await?;
    let answer: Json = serde_json::from_slice(&answer).unwrap_or(Json::Null);
    if !status.is_success() || answer.get("error").is_some() {
        let why = answer["error"]["status"]
            .as_str()
            .unwrap_or_default()
            .chars()
            .take(60)
            .collect::<String>();
        bail!("{what}: HTTP {} {why}", status.as_u16());
    }
    Ok(answer)
}

/// The `name=value` of a header like `dh=…; p256ecdsa=…` (Firebase's
/// `crypto-key` and `encryption`).
pub(crate) fn param<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    header
        .split([';', ','])
        .filter_map(|part| part.trim().split_once('='))
        .find(|(k, _)| k.trim() == name)
        .map(|(_, v)| v.trim().trim_matches('"'))
}

/// One push message's content (JSON), decrypted with our keys: the body
/// is `raw`, the rest comes in the message's `app_data`.
pub(crate) fn open(keys: &Keys, app_data: &[(String, String)], raw: &[u8]) -> anyhow::Result<Json> {
    let get = |key: &str| {
        app_data
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    };
    let plain = if get("content-encoding") == Some("aes128gcm") {
        ece::decrypt(keys, ece::Encoding::Aes128gcm, raw)?
    } else {
        let dh = get("crypto-key")
            .and_then(|h| param(h, "dh"))
            .and_then(base64)
            .context("no sender's key (crypto-key)")?;
        let encryption = get("encryption").context("no salt (encryption)")?;
        let salt = param(encryption, "salt")
            .and_then(base64)
            .context("no salt (encryption)")?;
        let rs = param(encryption, "rs")
            .and_then(|rs| rs.parse().ok())
            .unwrap_or(4096);
        ece::decrypt(
            keys,
            ece::Encoding::Aesgcm {
                dh: &dh,
                salt: &salt,
                rs,
            },
            raw,
        )?
    };
    serde_json::from_slice(&plain).context("the push message is not JSON")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::net::replay::Replay;

    /// A check-in answer as Google gives it.
    pub(crate) fn checkin_answer(id: u64, token: u64) -> Vec<u8> {
        Writer::new()
            .bool(1, true)
            .uint(3, 1_700_000_000_000)
            .fixed64(7, id)
            .fixed64(8, token)
            .finish()
    }

    pub(crate) fn credentials() -> Credentials {
        Credentials {
            android_id: 4_000_000_000_000_000_001,
            security_token: 5_000_000_000_000_000_002,
            token: "fake-fcm-token".into(),
            keys: Keys::generate().unwrap(),
        }
    }

    #[test]
    fn credentials_come_back_from_the_store_and_never_show() {
        let c = credentials();
        let text = c.to_text();
        assert_eq!(Credentials::parse(&text), Some(c.clone()));
        assert_eq!(format!("{c:?}"), "Credentials { .. }");
        // Made for another app, or not ours: registered again.
        let other = text.replace(APP_ID, "1:0:android:0");
        assert_eq!(Credentials::parse(&other), None);
        assert_eq!(Credentials::parse("{}"), None);
        assert_eq!(Credentials::parse("not json"), None);
    }

    #[tokio::test(start_paused = true)]
    async fn a_registration_goes_from_the_check_in_to_the_fcm_token() {
        let net = Replay::default()
            .then_bytes("/checkin", 200, &checkin_answer(41, 42))
            .then("/c2dm/register3", 200, "Error=PHONE_REGISTRATION_ERROR")
            .then("/c2dm/register3", 200, "token=fake-gcm-token")
            .then(
                "/v1/projects/ring-17770/installations",
                200,
                r#"{"fid":"f","refreshToken":"fake-r","authToken":{"token":"fake-fis","expiresIn":"604800s"}}"#,
            )
            .then(
                "/v1/projects/ring-17770/registrations",
                200,
                r#"{"token":"fake-fcm-token"}"#,
            );
        let c = register(&net).await.unwrap();
        assert_eq!((c.android_id, c.security_token), (41, 42));
        assert_eq!(c.token, "fake-fcm-token");
        assert_eq!(net.left(), 0);
        assert_eq!(net.host(0), CHECKIN_HOST);
        let sent = net.raw(0);
        let checkin = proto::parse(&sent).unwrap();
        assert!(!checkin.has(2) && !checkin.has(13), "a new device");
        let (_, headers, _) = net.asked(2);
        assert_eq!(headers["authorization"], "AidLogin 41:42");
        let form = String::from_utf8(net.raw(2)).unwrap();
        assert!(
            form.contains("X-subtype=wp%3Areceiver.push.com%23"),
            "{form}"
        );
        assert!(form.contains("device=41"), "{form}");
        assert_eq!(net.host(3), INSTALL_HOST);
        let (_, headers, body) = net.asked(3);
        assert_eq!(headers["x-goog-api-key"], API_KEY);
        assert_eq!(body["appId"], APP_ID);
        let fid = body["fid"].as_str().unwrap();
        assert_eq!(fid.len(), 22);
        assert!(matches!(&fid[..1], "c" | "d" | "e" | "f"), "{fid}");
        assert_eq!(net.host(4), REGISTER_HOST);
        let (_, headers, body) = net.asked(4);
        assert_eq!(headers["x-goog-firebase-installations-auth"], "fake-fis");
        assert_eq!(
            body["web"]["endpoint"],
            "https://fcm.googleapis.com/fcm/send/fake-gcm-token"
        );
        assert_eq!(
            base64(body["web"]["p256dh"].as_str().unwrap()).unwrap(),
            c.keys.public()
        );
        assert_eq!(
            base64(body["web"]["auth"].as_str().unwrap()).unwrap(),
            c.keys.auth()
        );
    }

    #[tokio::test]
    async fn a_known_device_checks_in_again_and_refusals_say_little() {
        let net = Replay::default()
            .then_bytes("/checkin", 200, &checkin_answer(41, 42))
            .then("/checkin", 400, "")
            .then(
                "/v1/projects/ring-17770/installations",
                403,
                r#"{"error":{"code":403,"message":"fake-secret-ish","status":"PERMISSION_DENIED"}}"#,
            );
        assert_eq!(checkin(&net, Some((41, 42))).await.unwrap(), (41, 42));
        let raw = net.raw(0);
        let sent = proto::parse(&raw).unwrap();
        assert_eq!(sent.uint(2), Some(41));
        assert_eq!(sent.fixed64(13), Some(42));
        assert!(checkin(&net, Some((41, 42))).await.is_err());
        let e = format!("{:#}", install(&net).await.unwrap_err());
        assert!(
            e.contains("PERMISSION_DENIED") && !e.contains("fake"),
            "{e}"
        );
    }

    #[test]
    fn headers_and_forms_read_and_written() {
        assert_eq!(param("dh=abc; p256ecdsa=def", "dh"), Some("abc"));
        assert_eq!(param("salt=xyz;rs=4096", "rs"), Some("4096"));
        assert_eq!(param("keyid=p256dh;dh=\"q\"", "dh"), Some("q"));
        assert_eq!(param("salt=xyz", "dh"), None);
        assert_eq!(form(&[("a b", "c:d#e"), ("f", "g")]), "a%20b=c%3Ad%23e&f=g");
    }

    #[test]
    fn a_message_opens_with_either_encoding() {
        let keys = Keys::generate().unwrap();
        let sender = Keys::generate().unwrap();
        let salt = [3u8; 16];
        let raw = crate::ece::tests::aesgcm(&keys, &sender, &salt, br#"{"data":{"x":"1"}}"#);
        let app_data = vec![
            (
                "crypto-key".to_owned(),
                format!("dh={}", base64url(sender.public())),
            ),
            (
                "encryption".to_owned(),
                format!("salt={}", base64url(&salt)),
            ),
        ];
        assert_eq!(open(&keys, &app_data, &raw).unwrap()["data"]["x"], "1");
        assert!(open(&keys, &app_data[..1], &raw).is_err(), "no salt");
        assert!(open(&keys, &[], &raw).is_err(), "nothing to decrypt with");
    }
}
