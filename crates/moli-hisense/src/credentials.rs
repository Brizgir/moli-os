//! Who Moli is to the TV: the MQTT client id, user name and password its
//! broker takes, and the session kept once paired.
//!
//! Old firmware takes one fixed login (`hisenseservice` /
//! `multimqttservice`) and authorizes the client id after a code. Newer
//! firmware computes the three from a MAC address (the phone's: here one
//! Moli makes up, locally administered), the time in seconds and a few
//! constants, as the VIDAA app does (`libmqttcrypt.so`, read by pyvidaa,
//! MIT, whose verified vector this module's algorithm reproduces; no code
//! copied). Which scheme a TV wants is told by the `transport_protocol` of
//! its UPnP description (port 38400, or 18400).

use std::fmt::Write as _;

use md5::{Digest as _, Md5};
use serde::{Deserialize, Serialize};

/// The fixed login of pre-dynamic firmware.
pub const STATIC_USERNAME: &str = "hisenseservice";
pub const STATIC_PASSWORD: &str = "multimqttservice";
const BRAND: &str = "his";
const OPERATION: &str = "vidaacommon";
const PATTERN: &str = "38D65DC30F45109A369A86FCE866A85B";
const SUFFIX_MODERN: &str = "h!i@s#$v%i^d&a*a";
const SUFFIX_LEGACY: &str = "h*i&s%e!r^v0i1c9";
const TIME_XOR: u64 = 0x5698_1477_2b03_a968;
/// `transport_protocol` from which the modern scheme applies.
pub const MODERN_FROM: u32 = 3290;
/// From which the middle one applies (below: the old firmware).
pub const MIDDLE_FROM: u32 = 3000;
/// Where the TV describes itself (`/MediaServer/rendererdevicedesc.xml`).
pub const DESCRIPTION_PORTS: [u16; 2] = [38400, 18400];
pub const DESCRIPTION_PATH: &str = "/MediaServer/rendererdevicedesc.xml";

/// How the TV checks a login.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Auth {
    /// The fixed login; the client id is authorized after a code.
    Static,
    /// Computed, the time in clear in the user name, the old suffix.
    Legacy,
    /// Computed, the time masked, the old suffix.
    Middle,
    /// Computed, the time masked, the new suffix; tokens after the code.
    Modern,
}

impl Auth {
    pub const ALL: [Self; 4] = [Self::Static, Self::Legacy, Self::Middle, Self::Modern];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Static => "static",
            Self::Legacy => "legacy",
            Self::Middle => "middle",
            Self::Modern => "modern",
        }
    }

    /// The `auth` option: one of the names, or `auto` (`None`).
    pub fn parse(word: &str) -> Result<Option<Self>, String> {
        match word.trim() {
            "" | "auto" => Ok(None),
            other => Self::ALL
                .into_iter()
                .find(|a| a.name() == other)
                .map(Some)
                .ok_or_else(|| other.to_owned()),
        }
    }

    /// The schemes to try, the likeliest first, for a `transport_protocol`
    /// (unknown: the newest first).
    #[must_use]
    pub fn order(transport_protocol: Option<u32>) -> [Self; 4] {
        match transport_protocol {
            Some(v) if v < MIDDLE_FROM => [Self::Static, Self::Legacy, Self::Middle, Self::Modern],
            Some(v) if v < MODERN_FROM => [Self::Middle, Self::Modern, Self::Legacy, Self::Static],
            _ => [Self::Modern, Self::Middle, Self::Legacy, Self::Static],
        }
    }

    /// Whether the TV hands out tokens after the code (else the code alone
    /// authorizes the client id), and has an action to ask for a code (the
    /// old firmware shows one when an unauthorized client asks anything).
    #[must_use]
    pub fn issues_tokens(self) -> bool {
        matches!(self, Self::Middle | Self::Modern)
    }
}

/// One login.
#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    pub client_id: String,
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("client_id", &self.client_id)
            .finish_non_exhaustive()
    }
}

fn md5_upper(s: &str) -> String {
    Md5::digest(s.as_bytes())
        .iter()
        .fold(String::with_capacity(32), |mut hex, b| {
            let _ = write!(hex, "{b:02X}");
            hex
        })
}

/// The login for `auth` at `timestamp` (seconds), as `mac` (the client's,
/// `aa:bb:cc:dd:ee:ff`, kept as written: it is hashed). `static_id`: the
/// client id of the fixed login.
#[must_use]
pub fn credentials(auth: Auth, mac: &str, timestamp: u64, static_id: &str) -> Credentials {
    if auth == Auth::Static {
        return Credentials {
            client_id: static_id.to_owned(),
            username: STATIC_USERNAME.into(),
            password: STATIC_PASSWORD.into(),
        };
    }
    let race = &md5_upper(&format!("{PATTERN}${mac}"))[..6];
    let client_id = format!("{mac}${BRAND}${race}_{OPERATION}_001");
    let username = match auth {
        Auth::Legacy => format!("{BRAND}${timestamp}"),
        _ => format!("{BRAND}${}", timestamp ^ TIME_XOR),
    };
    let remainder = timestamp
        .to_string()
        .bytes()
        .map(|d| u64::from(d - b'0'))
        .sum::<u64>()
        % 10;
    let suffix = match auth {
        Auth::Modern => SUFFIX_MODERN,
        _ => SUFFIX_LEGACY,
    };
    let value = &md5_upper(&format!("{BRAND}{remainder}{suffix}"))[..6];
    Credentials {
        client_id,
        username,
        password: md5_upper(&format!("{timestamp}${value}")),
    }
}

/// A MAC address Moli makes up for itself: locally administered
/// (`02:…`), random, kept in the secret store (the TV ties the pairing to
/// it).
pub fn random_mac() -> anyhow::Result<String> {
    use ring::rand::SecureRandom as _;
    let mut bytes = [0u8; 6];
    ring::rand::SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| anyhow::anyhow!("no randomness"))?;
    bytes[0] = 0x02;
    Ok(bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":"))
}

/// The `transport_protocol` of a TV's UPnP description: an element, or
/// `transport_protocol=NNNN` in a text (both seen in the wild).
#[must_use]
pub fn transport_protocol(xml: &str) -> Option<u32> {
    if let Some(text) = moli_net::upnp::tag(xml, "transport_protocol")
        && let Ok(v) = text.trim().parse()
    {
        return Some(v);
    }
    let at = xml.find("transport_protocol")? + "transport_protocol".len();
    let rest = xml[at..].trim_start_matches([' ', '=', ':', '>', '"']);
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// What the TV gave after the code (`tokenissuance`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tokens {
    pub access: String,
    /// How long it lasts, in days, if the TV said.
    pub access_days: Option<u64>,
    pub refresh: Option<String>,
    pub refresh_days: Option<u64>,
}

/// Lasting without the TV saying: what the app's firmware is known to give.
const DEFAULT_ACCESS_DAYS: u64 = 2;
const DEFAULT_REFRESH_DAYS: u64 = 30;
/// Renewed this long before it lapses.
const MARGIN: u64 = 6 * 3600;

/// The pairing kept between runs (secret `session`, JSON).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub auth: Option<Auth>,
    #[serde(default)]
    pub transport_protocol: Option<u32>,
    pub client_id: String,
    pub username: String,
    /// Tokenless firmware: the password the TV took with this client id.
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub access_token: Option<String>,
    /// Unix seconds.
    #[serde(default)]
    pub access_until: Option<u64>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub refresh_until: Option<u64>,
}

/// What a kept session connects with now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Login {
    /// The access token as password.
    Access(String),
    /// The refresh token as password, then `gettoken` with it.
    Refresh(String),
    /// The password kept (tokenless firmware).
    Password(String),
    /// Both tokens lapsed: pair again.
    Expired,
}

impl Session {
    #[must_use]
    pub fn login(&self, now: u64) -> Login {
        let live = |until: Option<u64>| until.is_none_or(|t| now + MARGIN < t);
        match (&self.access_token, &self.refresh_token, &self.password) {
            (Some(access), _, _) if live(self.access_until) => Login::Access(access.clone()),
            (_, Some(refresh), _) if live(self.refresh_until) => Login::Refresh(refresh.clone()),
            (None, None, Some(password)) => Login::Password(password.clone()),
            _ => Login::Expired,
        }
    }

    /// Keeps what the TV issued at `now`.
    pub fn take(&mut self, tokens: &Tokens, now: u64) {
        let access_days = tokens.access_days.unwrap_or(DEFAULT_ACCESS_DAYS);
        self.access_token = Some(tokens.access.clone());
        self.access_until = Some(now + access_days * 86_400);
        if let Some(refresh) = &tokens.refresh {
            let refresh_days = tokens.refresh_days.unwrap_or(DEFAULT_REFRESH_DAYS);
            self.refresh_token = Some(refresh.clone());
            self.refresh_until = Some(now + refresh_days * 86_400);
        }
        self.password = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: &str = "02:00:00:00:00:01";

    #[test]
    fn the_computed_login_matches_the_apps_algorithm() {
        // Values produced by pyvidaa's `credentials.py` (MIT) for this made-up
        // MAC at 1 700 000 000 s; its own vector, from a logcat capture of the
        // VIDAA app, is reproduced by the same code (not kept here: a real
        // phone's MAC).
        let modern = credentials(Auth::Modern, MAC, 1_700_000_000, "moli");
        assert_eq!(
            modern.client_id,
            "02:00:00:00:00:01$his$3A8DE7_vidaacommon_001"
        );
        assert_eq!(modern.username, "his$6239759786369374312");
        assert_eq!(modern.password, "E4B16BE39DA56468522CACF7411FB50F");
        let middle = credentials(Auth::Middle, MAC, 1_700_000_000, "moli");
        assert_eq!(middle.username, modern.username, "same masked time");
        assert_eq!(middle.password, "30CC23251954D2B965504DC81E944805");
        let legacy = credentials(Auth::Legacy, MAC, 1_700_000_000, "moli");
        assert_eq!(legacy.username, "his$1700000000", "the time in clear");
        assert_eq!(legacy.password, middle.password, "same suffix");
        assert_eq!(legacy.client_id, modern.client_id);
        let fixed = credentials(Auth::Static, MAC, 1_700_000_000, "moli_tv");
        assert_eq!(fixed.client_id, "moli_tv");
        assert_eq!(fixed.username, STATIC_USERNAME);
        assert_eq!(fixed.password, STATIC_PASSWORD);
    }

    #[test]
    fn the_scheme_follows_the_transport_protocol() {
        assert_eq!(Auth::order(Some(3290))[0], Auth::Modern);
        assert_eq!(Auth::order(Some(3285))[0], Auth::Middle);
        assert_eq!(Auth::order(Some(2999))[0], Auth::Static);
        assert_eq!(Auth::order(None)[0], Auth::Modern);
        assert_eq!(Auth::parse("auto"), Ok(None));
        assert_eq!(Auth::parse("legacy"), Ok(Some(Auth::Legacy)));
        assert_eq!(Auth::parse("v2"), Err("v2".into()));
        assert!(Auth::Modern.issues_tokens() && !Auth::Legacy.issues_tokens());
    }

    #[test]
    fn the_transport_protocol_is_read_from_the_description() {
        const XML: &str =
            include_str!("../../../integrations/hisense/fixtures/rendererdevicedesc.xml");
        assert_eq!(transport_protocol(XML), Some(3290));
        assert_eq!(
            transport_protocol("<modelDescription>transport_protocol=3000</modelDescription>"),
            Some(3000)
        );
        assert_eq!(transport_protocol("<root/>"), None);
    }

    #[test]
    fn a_session_knows_what_to_connect_with() {
        let mut session = Session {
            auth: Some(Auth::Modern),
            client_id: "id".into(),
            username: "user".into(),
            ..Session::default()
        };
        assert_eq!(session.login(1_000), Login::Expired, "nothing yet");
        session.take(
            &Tokens {
                access: "a".into(),
                access_days: Some(2),
                refresh: Some("r".into()),
                refresh_days: Some(30),
            },
            1_000,
        );
        assert_eq!(session.login(1_000), Login::Access("a".into()));
        assert_eq!(
            session.login(1_000 + 2 * 86_400),
            Login::Refresh("r".into())
        );
        assert_eq!(session.login(1_000 + 31 * 86_400), Login::Expired);
        let fixed = Session {
            auth: Some(Auth::Static),
            client_id: "moli_tv".into(),
            username: STATIC_USERNAME.into(),
            password: Some(STATIC_PASSWORD.into()),
            ..Session::default()
        };
        assert_eq!(fixed.login(1_000), Login::Password(STATIC_PASSWORD.into()));
        let json = serde_json::to_string(&session).unwrap();
        assert_eq!(serde_json::from_str::<Session>(&json).unwrap(), session);
    }

    #[test]
    fn the_made_up_mac_is_local() {
        let mac = random_mac().unwrap();
        assert_eq!(mac.len(), 17);
        assert!(mac.starts_with("02:"));
    }
}
