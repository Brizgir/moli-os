//! The Alexa app's web API on the account's site (`alexa.amazon.fr`…): a
//! session (cookies and CSRF from the refresh token), the speakers, what they
//! play, and the orders, mostly as « sequences » (the routines' format).

use std::time::Duration;

use anyhow::{Context as _, bail};
use http::{Method, Request, StatusCode};
use http_body_util::Full;
use moli_net::Body as Bytes;
use serde_json::{Value as Json, json};

use crate::auth::{self, Jar, USER_AGENT};
use crate::echo::Echo;
use crate::media::{self, Playing};

const LIMIT: Duration = Duration::from_secs(20);
const MAX_BODY: usize = 4 * 1024 * 1024;
/// The routines editor's version, which the music providers' list asks for.
const ROUTINES_VERSION: &str = "3.0.264101";

/// An answer that says the session is over: refresh it, then try again.
#[derive(Debug)]
pub(crate) enum Failure {
    Expired,
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
            Self::Expired => f.write_str("the Alexa session expired"),
            Self::Other(e) => write!(f, "{e:#}"),
        }
    }
}

pub(crate) struct Session {
    /// `alexa.amazon.fr`.
    host: String,
    /// `fr-FR`.
    pub locale: String,
    jar: Jar,
    csrf: String,
    /// The account's customer id (the sequences' `customerId`).
    pub customer: String,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("host", &self.host)
            .field("customer", &self.customer)
            .finish_non_exhaustive()
    }
}

impl Session {
    /// Cookies from the refresh token, the CSRF cookie, the account's id.
    pub(crate) async fn open(refresh: &str, domain: &str, locale: &str) -> anyhow::Result<Self> {
        let mut jar = auth::cookies(refresh, domain).await?;
        let host = format!("alexa.{domain}");
        let csrf = auth::csrf(&mut jar, &host).await?;
        let mut session = Self {
            host,
            locale: locale.to_owned(),
            jar,
            csrf,
            customer: String::new(),
        };
        let me = session
            .get("/api/users/me?platform=ios&version=2.2.651540.0")
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        me["id"]
            .as_str()
            .context("no customer id in /api/users/me")?
            .clone_into(&mut session.customer);
        if let Some(market) = me["marketPlaceDomainName"].as_str()
            && !market.ends_with(domain)
        {
            tracing::warn!(market, domain, "the Amazon account belongs to another site");
        }
        Ok(session)
    }

    async fn send(&self, method: Method, path: &str, body: Option<&Json>) -> Result<Json, Failure> {
        self.send_as(method, path, body, false).await
    }

    /// `routines`: with the routines editor's version header, which some
    /// endpoints of the app's newer pages ask for.
    async fn send_as(
        &self,
        method: Method,
        path: &str,
        body: Option<&Json>,
        routines: bool,
    ) -> Result<Json, Failure> {
        let referer = format!("https://{}/spa/index.html", self.host);
        let origin = format!("https://{}", self.host);
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header("user-agent", USER_AGENT)
            .header("accept", "application/json; charset=utf-8")
            .header("accept-language", self.locale.as_str())
            .header("referer", referer)
            .header("origin", origin)
            .header("csrf", self.csrf.as_str())
            .header("cookie", self.jar.header());
        if body.is_some() {
            builder = builder.header("content-type", "application/json; charset=utf-8");
        }
        if routines {
            builder = builder.header("routines-version", ROUTINES_VERSION);
        }
        let request = builder
            .body(Full::new(Bytes::from(
                body.map(Json::to_string).unwrap_or_default().into_bytes(),
            )))
            .map_err(|e| Failure::Other(e.into()))?;
        let (status, answer) =
            moli_net::web(&self.host, 443, true, request, LIMIT, MAX_BODY).await?;
        if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            return Err(Failure::Expired);
        }
        if !status.is_success() {
            // The error's message, never the query (it names the speaker).
            let text = error_message(&answer);
            tracing::debug!(
                path = endpoint(path),
                status = status.as_u16(),
                message = %text,
                "alexa: refused"
            );
            return Err(Failure::Other(anyhow::anyhow!(
                "{}{}: HTTP {} {text}",
                self.host,
                endpoint(path),
                status.as_u16()
            )));
        }
        // Many orders answer 200 with no body.
        Ok(serde_json::from_slice(&answer).unwrap_or(Json::Null))
    }

    pub(crate) async fn get(&self, path: &str) -> Result<Json, Failure> {
        self.send(Method::GET, path, None).await
    }

    pub(crate) async fn post(&self, path: &str, body: &Json) -> Result<Json, Failure> {
        self.send(Method::POST, path, Some(body)).await
    }

    /// Every device of the account (`devices-v2`).
    pub(crate) async fn devices(&self) -> Result<Json, Failure> {
        self.get("/api/devices-v2/device?cached=false").await
    }

    /// The account's music service by default (`AMAZON_MUSIC`, `SPOTIFY`…).
    pub(crate) async fn default_music(&self) -> Option<String> {
        let path = "/api/behaviors/entities?skillId=amzn1.ask.1p.music";
        let referer = format!("https://{}/spa/index.html", self.host);
        let request = Request::builder()
            .method(Method::GET)
            .uri(path)
            .header("user-agent", USER_AGENT)
            .header("accept", "application/json; charset=utf-8")
            .header("routines-version", ROUTINES_VERSION)
            .header("referer", referer)
            .header("csrf", self.csrf.as_str())
            .header("cookie", self.jar.header())
            .body(Full::new(Bytes::new()))
            .ok()?;
        let (_, answer) = moli_net::web(&self.host, 443, true, request, LIMIT, MAX_BODY)
            .await
            .ok()?;
        default_provider(&serde_json::from_slice(&answer).ok()?)
    }

    /// Runs `start` (one operation, or several side by side).
    pub(crate) async fn run(&self, start: &Json) -> Result<(), Failure> {
        self.post("/api/behaviors/preview", &preview(start))
            .await
            .map(|_| ())
    }

    /// Music by search, as the Alexa app plays it: Amazon checks the search
    /// first and answers the payload to play, so a search it refuses is an
    /// error here rather than a silent speaker.
    pub(crate) async fn play(
        &self,
        echo: &Echo,
        phrase: &str,
        provider: &str,
    ) -> Result<(), Failure> {
        let mut node = music(echo, &self.customer, &self.locale, phrase, provider);
        let answer = self
            .post(
                "/api/behaviors/operation/validate",
                &json!({
                    "type": node["type"],
                    "operationPayload": node["operationPayload"].to_string(),
                }),
            )
            .await?;
        node["operationPayload"] = checked(&answer).map_err(Failure::Other)?;
        self.run(&node).await
    }

    /// Play, pause, next… (`PlayCommand`, `PauseCommand`, `NextCommand`).
    pub(crate) async fn player(&self, echo: &Echo, command: &str) -> Result<(), Failure> {
        let path = format!(
            "/api/np/command?deviceSerialNumber={}&deviceType={}",
            echo.serial, echo.device_type
        );
        self.post(
            &path,
            &json!({ "type": command, "contentFocusClientId": null }),
        )
        .await
        .map(|_| ())
    }

    /// What `echo`'s own player says, and the shape of the answer for the
    /// debug log. A player that answers 400, or a state of `null`, says
    /// nothing: on a real account an Echo Dot answers both while it plays.
    pub(crate) async fn playing(&self, echo: &Echo) -> Result<(Playing, String), Failure> {
        let path = format!(
            "/api/np/player?deviceSerialNumber={}&deviceType={}&screenWidth=1392",
            echo.serial, echo.device_type
        );
        match self.get(&path).await {
            Ok(answer) => Ok((player_state(&answer), media::player_shape(&answer))),
            Err(Failure::Other(e)) if status(&e) == Some(400) => {
                Ok((Playing::unknown(), "HTTP 400".into()))
            }
            Err(e) => Err(e),
        }
    }

    /// The older media state of `echo` (`/api/media/state`), for a speaker
    /// whose player says nothing. A 400 or 404 says nothing either.
    pub(crate) async fn media_state(&self, echo: &Echo) -> Result<(Playing, String), Failure> {
        let path = format!(
            "/api/media/state?deviceSerialNumber={}&deviceType={}&screenWidth=1392",
            echo.serial, echo.device_type
        );
        match self.get(&path).await {
            Ok(answer) => Ok((media::media_state(&answer), media::media_shape(&answer))),
            Err(Failure::Other(e)) if matches!(status(&e), Some(400 | 404)) => Ok((
                Playing::unknown(),
                format!("HTTP {}", status(&e).unwrap_or(0)),
            )),
            Err(e) => Err(e),
        }
    }

    /// The account's media sessions, as `form` asks for them. `None`: the
    /// site refuses this form (4xx, 501); an account that refuses it is not
    /// taken for a session that expired, which would renew the session in
    /// a loop. `for_echo`: the speaker the [`SessionsForm::Device`] form
    /// names (the answer still lists the account's sessions).
    pub(crate) async fn media_sessions(
        &self,
        form: SessionsForm,
        for_echo: Option<&Echo>,
    ) -> Result<Option<Json>, Failure> {
        let base = "/api/np/list-media-sessions";
        let path = match (form, for_echo) {
            (SessionsForm::Device, Some(e)) => format!(
                "{base}?deviceSerialNumber={}&deviceType={}",
                e.serial, e.device_type
            ),
            (SessionsForm::Device, None) => return Ok(None),
            (SessionsForm::Plain | SessionsForm::Routines, _) => base.to_owned(),
        };
        let routines = form == SessionsForm::Routines;
        match self.send_as(Method::GET, &path, None, routines).await {
            Ok(answer) => Ok(Some(answer)),
            Err(Failure::Expired) => Ok(None),
            Err(Failure::Other(e))
                if status(&e).is_some_and(|s| (400..500).contains(&s) || s == 501) =>
            {
                tracing::debug!(form = %form, status = status(&e), "alexa: no media sessions list");
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }

    /// Every speaker's volume: (serial, 0-100).
    pub(crate) async fn volumes(&self) -> Result<Vec<(String, f64)>, Failure> {
        let answer = self
            .get("/api/devices/deviceType/dsn/audio/v1/allDeviceVolumes")
            .await?;
        Ok(volumes(&answer))
    }
}

/// How the media sessions' list is asked for: the forms the open clients
/// use, tried in this order until one answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionsForm {
    /// `?deviceSerialNumber=…&deviceType=…` of one speaker (alexa-remote2).
    Device,
    /// No parameter, as the app's first pages did.
    Plain,
    /// No parameter, with the routines editor's version header.
    Routines,
}

impl SessionsForm {
    pub(crate) const ALL: [Self; 3] = [Self::Device, Self::Plain, Self::Routines];
}

impl std::fmt::Display for SessionsForm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Device => "device",
            Self::Plain => "plain",
            Self::Routines => "routines",
        })
    }
}

pub(crate) fn player_state(answer: &Json) -> Playing {
    media::now_playing(&answer["playerInfo"])
}

/// A path without its query: the query names the speaker.
fn endpoint(path: &str) -> &str {
    path.split('?').next().unwrap_or(path)
}

/// What an error answer says, for the log: its `message` (or the first of
/// its `errors`), else its text, 120 characters at most, one line. Amazon's
/// messages describe the request, never the account.
pub(crate) fn error_message(answer: &[u8]) -> String {
    let text = String::from_utf8_lossy(&answer[..answer.len().min(1024)]);
    let said = serde_json::from_str::<Json>(&text).ok().and_then(|j| {
        [
            &j["message"],
            &j["errors"][0]["message"],
            &j["error"]["message"],
        ]
        .into_iter()
        .find_map(|m| m.as_str().map(str::to_owned))
    });
    let text = said.unwrap_or_else(|| text.into_owned());
    let line: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.starts_with('<') {
        return "(html)".into();
    }
    line.chars().take(120).collect()
}

/// An HTTP status in an error from [`Session::send`].
pub(crate) fn status(e: &anyhow::Error) -> Option<u16> {
    let text = format!("{e:#}");
    let (_, after) = text.split_once(": HTTP ")?;
    after.get(..3)?.parse().ok()
}

/// Amazon refused the order for coming too soon after another (429):
/// sent again once, a little later.
pub(crate) fn rate_limited(e: &Failure) -> bool {
    matches!(e, Failure::Other(e) if status(e) == Some(429))
}

pub(crate) fn volumes(answer: &Json) -> Vec<(String, f64)> {
    answer["volumes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| Some((v["dsn"].as_str()?.to_owned(), v["speakerVolume"].as_f64()?)))
        .collect()
}

fn default_provider(answer: &Json) -> Option<String> {
    let list = answer.as_array()?;
    let plays = |p: &&Json| {
        p["availability"].as_str().is_none_or(|a| a == "AVAILABLE")
            && ["supportedOperations", "supportedProperties"]
                .iter()
                .any(|k| {
                    p[*k]
                        .as_array()
                        .is_some_and(|ops| ops.iter().any(|o| o == "Alexa.Music.PlaySearchPhrase"))
                })
    };
    list.iter()
        .filter(plays)
        .find(|p| p["providerData"]["isDefaultMusicProvider"].as_bool() == Some(true))
        .or_else(|| list.iter().find(plays))
        .and_then(|p| p["id"].as_str())
        .map(str::to_owned)
}

/// The payload Amazon gives back for a search it accepts.
pub(crate) fn checked(answer: &Json) -> anyhow::Result<Json> {
    match (answer["result"].as_str(), &answer["operationPayload"]) {
        (Some("VALID"), payload) if !payload.is_null() => Ok(payload.clone()),
        (result, _) => bail!(
            "Amazon refused the search ({})",
            result.unwrap_or("no answer")
        ),
    }
}

/// The routines' envelope: `sequenceJson` is JSON in a string.
pub(crate) fn preview(start: &Json) -> Json {
    let sequence = json!({
        "@type": "com.amazon.alexa.behaviors.model.Sequence",
        "startNode": start,
    });
    json!({
        "behaviorId": "PREVIEW",
        "sequenceJson": sequence.to_string(),
        "status": "ENABLED",
    })
}

/// One operation on `echo`; `extra` adds its own fields to the payload.
pub(crate) fn operation(
    kind: &str,
    echo: &Echo,
    customer: &str,
    locale: &str,
    extra: &Json,
) -> Json {
    let mut payload = json!({
        "deviceType": echo.device_type,
        "deviceSerialNumber": echo.serial,
        "locale": locale,
        "customerId": customer,
    });
    if let (Some(payload), Some(extra)) = (payload.as_object_mut(), extra.as_object()) {
        payload.extend(extra.clone());
    }
    json!({
        "@type": "com.amazon.alexa.behaviors.model.OpaquePayloadOperationNode",
        "type": kind,
        "operationPayload": payload,
    })
}

/// The skill that carries an operation sits beside its payload, as the
/// Alexa app sends it: an order spoken with the skill inside the payload is
/// accepted, and the Echo stays silent.
fn by(skill: &str, mut node: Json) -> Json {
    node["skillId"] = json!(skill);
    node
}

/// Several operations side by side.
pub(crate) fn together(nodes: Vec<Json>) -> Json {
    if nodes.len() == 1 {
        return nodes.into_iter().next().unwrap_or(Json::Null);
    }
    json!({
        "@type": "com.amazon.alexa.behaviors.model.ParallelNode",
        "nodesToExecute": nodes,
    })
}

pub(crate) fn music(
    echo: &Echo,
    customer: &str,
    locale: &str,
    phrase: &str,
    provider: &str,
) -> Json {
    operation(
        "Alexa.Music.PlaySearchPhrase",
        echo,
        customer,
        locale,
        &json!({
            "searchPhrase": phrase,
            "sanitizedSearchPhrase": phrase,
            "musicProviderId": provider,
        }),
    )
}

pub(crate) fn text_command(echo: &Echo, customer: &str, locale: &str, text: &str) -> Json {
    by(
        "amzn1.ask.1p.tellalexa",
        operation(
            "Alexa.TextCommand",
            echo,
            customer,
            locale,
            &json!({ "text": text.to_lowercase() }),
        ),
    )
}

pub(crate) fn speak(echo: &Echo, customer: &str, locale: &str, text: &str) -> Json {
    by(
        "amzn1.ask.1p.saysomething",
        operation(
            "Alexa.Speak",
            echo,
            customer,
            locale,
            &json!({ "textToSpeak": text }),
        ),
    )
}

/// An announcement (its jingle, then the words) on `targets`.
pub(crate) fn announce(targets: &[&Echo], customer: &str, locale: &str, text: &str) -> Json {
    let devices: Vec<Json> = targets
        .iter()
        .map(|e| json!({ "deviceSerialNumber": e.serial, "deviceTypeId": e.device_type }))
        .collect();
    by(
        "amzn1.ask.1p.routines.messaging",
        json!({
            "@type": "com.amazon.alexa.behaviors.model.OpaquePayloadOperationNode",
            "type": "AlexaAnnouncement",
            "operationPayload": {
                "customerId": customer,
                "expireAfter": "PT5S",
                "content": [{
                    "locale": locale,
                    "display": { "title": "Moli", "body": text },
                    "speak": { "type": "text", "value": text },
                }],
                "target": { "customerId": customer, "devices": devices },
            },
        }),
    )
}

pub(crate) fn volume(echo: &Echo, customer: &str, locale: &str, value: i64) -> Json {
    by(
        "amzn1.ask.1p.alexadevicecontrols",
        operation(
            "Alexa.DeviceControls.Volume",
            echo,
            customer,
            locale,
            &json!({ "value": value.clamp(0, 100) }),
        ),
    )
}

/// What a phrase may hold: Alexa speaks at most 250 characters.
/// Typographic punctuation is made plain on the way (« ’ » → « ' »…).
pub(crate) fn sayable(text: &str) -> anyhow::Result<String> {
    let text: String = text
        .trim()
        .chars()
        .map(|c| match c {
            '\u{2018}' | '\u{2019}' | '\u{02bc}' => '\'',
            '\u{201c}' | '\u{201d}' | '\u{00ab}' | '\u{00bb}' => '"',
            '\u{00a0}' | '\u{202f}' => ' ',
            '\u{2013}' | '\u{2014}' => '-',
            other => other,
        })
        .collect::<String>()
        .replace('\u{2026}', "...");
    let text = text.trim();
    if text.is_empty() {
        bail!("nothing to say");
    }
    if text.chars().count() > 250 {
        bail!("at most 250 characters");
    }
    Ok(text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn echo(serial: &str) -> Echo {
        Echo {
            serial: serial.into(),
            device_type: "A3S5BH2HU6VAYF".into(),
            name: "Salon".into(),
            family: "ECHO".into(),
            group: false,
            members: Vec::new(),
            online: true,
        }
    }

    #[test]
    fn a_sequence_is_json_in_a_string() {
        let e = echo("G0911");
        let body = preview(&music(&e, "A1CUST", "fr-FR", "du jazz", "AMAZON_MUSIC"));
        assert_eq!(body["behaviorId"], "PREVIEW");
        let seq: Json = serde_json::from_str(body["sequenceJson"].as_str().unwrap()).unwrap();
        assert_eq!(seq["@type"], "com.amazon.alexa.behaviors.model.Sequence");
        let p = &seq["startNode"]["operationPayload"];
        assert_eq!(seq["startNode"]["type"], "Alexa.Music.PlaySearchPhrase");
        assert_eq!(p["deviceSerialNumber"], "G0911");
        assert_eq!(p["customerId"], "A1CUST");
        assert_eq!(p["locale"], "fr-FR");
        assert_eq!(p["searchPhrase"], "du jazz");
        assert_eq!(p["musicProviderId"], "AMAZON_MUSIC");
    }

    #[test]
    fn several_speakers_at_once_run_side_by_side() {
        let (a, b) = (echo("A"), echo("B"));
        let both = together(vec![
            speak(&a, "C", "fr-FR", "Bonjour"),
            speak(&b, "C", "fr-FR", "Bonjour"),
        ]);
        assert_eq!(
            both["@type"],
            "com.amazon.alexa.behaviors.model.ParallelNode"
        );
        assert_eq!(both["nodesToExecute"].as_array().unwrap().len(), 2);
        let one = together(vec![volume(&a, "C", "fr-FR", 140)]);
        assert_eq!(one["operationPayload"]["value"], 100);
        let a_note = announce(&[&a, &b], "C", "fr-FR", "Le linge est prêt");
        assert_eq!(
            a_note["operationPayload"]["target"]["devices"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            text_command(&a, "C", "fr-FR", "Quelle Heure")["operationPayload"]["text"],
            "quelle heure"
        );
    }

    #[test]
    fn a_search_plays_what_amazon_checked() {
        let payload = json!({ "searchPhrase": "du jazz", "sanitizedSearchPhrase": "jazz" });
        assert_eq!(
            checked(&json!({ "result": "VALID", "operationPayload": payload })).unwrap(),
            payload
        );
        assert!(checked(&json!({ "result": "INVALID" })).is_err());
        assert!(checked(&Json::Null).is_err());
    }

    #[test]
    fn the_skill_sits_beside_the_payload() {
        let e = echo("G0911");
        let said = speak(&e, "C", "fr-FR", "Bonjour");
        assert_eq!(said["type"], "Alexa.Speak");
        assert_eq!(said["skillId"], "amzn1.ask.1p.saysomething");
        let p = &said["operationPayload"];
        assert_eq!(p["textToSpeak"], "Bonjour");
        assert!(p.get("skillId").is_none() && p.get("target").is_none());
        let asked = text_command(&e, "C", "fr-FR", "Quelle heure");
        assert_eq!(asked["skillId"], "amzn1.ask.1p.tellalexa");
        assert!(asked["operationPayload"].get("skillId").is_none());
        let note = announce(&[&e], "C", "fr-FR", "Bonjour");
        assert_eq!(note["skillId"], "amzn1.ask.1p.routines.messaging");
        assert!(note["operationPayload"].get("skillId").is_none());
        assert_eq!(
            volume(&e, "C", "fr-FR", 20)["skillId"],
            "amzn1.ask.1p.alexadevicecontrols"
        );
    }

    #[test]
    fn what_plays_and_how_loud() {
        let answer = json!({ "playerInfo": {
            "state": "PLAYING",
            "infoText": { "title": "So What", "subText1": "Miles Davis" },
            "volume": { "volume": 35, "muted": false }
        }});
        let p = player_state(&answer);
        assert_eq!(
            (p.state.as_str(), p.title.as_str(), p.artist.as_str()),
            ("PLAYING", "So What", "Miles Davis")
        );
        assert_eq!(p.volume, Some(35.0));
        assert!(player_state(&json!({})).is_unknown());
        let e = anyhow::anyhow!("alexa.amazon.fr/api/np/x: HTTP 404 Not Found");
        assert_eq!(status(&e), Some(404));
        assert_eq!(status(&anyhow::anyhow!("timed out")), None);
        let v =
            volumes(&json!({ "volumes": [{ "dsn": "A", "speakerVolume": 40 }, { "dsn": "B" }] }));
        assert_eq!(v, [("A".to_owned(), 40.0)]);
    }

    #[test]
    fn a_refusal_is_logged_by_its_message_only() {
        let body = include_bytes!("../fixtures/error-400.json");
        assert_eq!(
            error_message(body),
            "Invalid request: a parameter is missing"
        );
        assert_eq!(error_message(br#"{"errors":[{"message":"nope"}]}"#), "nope");
        assert_eq!(
            error_message(b"Rate exceeded:\n  Too many requests"),
            "Rate exceeded: Too many requests"
        );
        assert_eq!(error_message(b"<html><body>Bad</body></html>"), "(html)");
        assert_eq!(error_message(b"").len(), 0);
        assert_eq!(error_message(&[b'x'; 500]).chars().count(), 120);
        assert_eq!(
            endpoint("/api/np/player?deviceSerialNumber=G0911&deviceType=A1"),
            "/api/np/player"
        );
        let e = Failure::Other(anyhow::anyhow!(
            "alexa.amazon.fr/api/behaviors/preview: HTTP 429 Rate exceeded: Too many requests"
        ));
        assert!(rate_limited(&e));
        assert!(!rate_limited(&Failure::Expired));
        assert!(!rate_limited(&Failure::Other(anyhow::anyhow!(
            "x: HTTP 500 oops"
        ))));
        assert_eq!(SessionsForm::Device.to_string(), "device");
        assert_eq!(SessionsForm::ALL.len(), 3);
    }

    #[test]
    fn the_default_music_service_is_the_accounts() {
        let entities = json!([
            { "id": "TUNEIN", "availability": "AVAILABLE", "supportedOperations": ["Alexa.Music.PlaySearchPhrase"] },
            { "id": "SPOTIFY", "availability": "AVAILABLE", "supportedOperations": ["Alexa.Music.PlaySearchPhrase"],
              "providerData": { "isDefaultMusicProvider": true } },
            { "id": "OFF", "availability": "UNAVAILABLE", "supportedOperations": ["Alexa.Music.PlaySearchPhrase"],
              "providerData": { "isDefaultMusicProvider": true } }
        ]);
        assert_eq!(default_provider(&entities).as_deref(), Some("SPOTIFY"));
        assert_eq!(default_provider(&json!([])), None);
        assert!(sayable("  ").is_err());
        assert!(sayable(&"a".repeat(251)).is_err());
        assert_eq!(sayable(" Bonjour ").unwrap(), "Bonjour");
        assert_eq!(
            sayable("Yo c\u{2019}est \u{00ab}\u{202f}lui\u{202f}\u{00bb}\u{2026}").unwrap(),
            "Yo c'est \" lui \"..."
        );
    }
}
