//! Netatmo's OAuth2 « authorization code » sign-in, with the person's own
//! app (dev.netatmo.com): Moli makes the address of Netatmo's authorization
//! page, the person accepts there and pastes the address the page ends on
//! (or only its code); Moli exchanges the code for tokens. The refresh token
//! changes at each renewal: the newest is always the one kept.

use std::time::Duration;

use ring::rand::{SecureRandom as _, SystemRandom};
use serde_json::Value as Json;

use crate::encode;

/// What Moli asks for: read the heating, and change it.
pub(crate) const SCOPES: &str = "read_thermostat write_thermostat";
pub(crate) const HOST: &str = "api.netatmo.com";
pub(crate) const TOKEN_PATH: &str = "/oauth2/token";

/// The person's app on dev.netatmo.com (from the encrypted store).
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct App {
    pub client_id: String,
    pub client_secret: String,
}

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the secret.
        f.debug_struct("App")
            .field("client_id", &self.client_id)
            .finish_non_exhaustive()
    }
}

/// A sign-in in progress: the page for the person, and the `state` it must
/// come back with (an older page's address is refused).
#[derive(Clone, Debug)]
pub(crate) struct SignIn {
    pub state: String,
    pub url: String,
}

impl SignIn {
    pub(crate) fn new(app: &App, redirect: &str) -> anyhow::Result<Self> {
        let mut bytes = [0u8; 16];
        SystemRandom::new()
            .fill(&mut bytes)
            .map_err(|_| anyhow::anyhow!("no randomness"))?;
        let state = encode::hex(&bytes);
        let url = format!(
            "https://{HOST}/oauth2/authorize?{}",
            encode::form(&[
                ("client_id", &app.client_id),
                ("redirect_uri", redirect),
                ("scope", SCOPES),
                ("state", &state),
            ])
        );
        Ok(Self { state, url })
    }
}

/// Why a pasted text gives no code (a catalogue key each).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Pasted {
    /// Neither an address with a code, nor a code.
    NoCode,
    /// The person refused on Netatmo's page.
    Denied,
    /// The address of an older page (its `state` is not this one's).
    OldPage,
}

/// The authorization code in what the person pasted: the address Netatmo's
/// page ended on (`…?state=…&code=…`), or the code alone.
pub(crate) fn code_of(pasted: &str, state: &str) -> Result<String, Pasted> {
    let pasted = pasted.trim();
    if pasted.contains('?') {
        if encode::query_param(pasted, "error").is_some() {
            return Err(Pasted::Denied);
        }
        let code = encode::query_param(pasted, "code")
            .filter(|c| plausible_code(c))
            .ok_or(Pasted::NoCode)?;
        if let Some(back) = encode::query_param(pasted, "state")
            && back != state
        {
            return Err(Pasted::OldPage);
        }
        return Ok(code);
    }
    if plausible_code(pasted) {
        Ok(pasted.to_owned())
    } else {
        Err(Pasted::NoCode)
    }
}

fn plausible_code(code: &str) -> bool {
    (8..=256).contains(&code.len())
        && code
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '|' | '.'))
}

/// The form that exchanges a code for tokens.
pub(crate) fn code_form(app: &App, code: &str, redirect: &str) -> String {
    encode::form(&[
        ("grant_type", "authorization_code"),
        ("client_id", &app.client_id),
        ("client_secret", &app.client_secret),
        ("code", code),
        ("redirect_uri", redirect),
        ("scope", SCOPES),
    ])
}

/// The form that renews the access token.
pub(crate) fn refresh_form(app: &App, refresh: &str) -> String {
    encode::form(&[
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh),
        ("client_id", &app.client_id),
        ("client_secret", &app.client_secret),
    ])
}

/// What the token endpoint gives.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Tokens {
    pub access: String,
    /// The refresh token to keep from now on (Netatmo may hand a new one at
    /// each renewal; the old one then stops working).
    pub refresh: String,
    pub lifetime: Duration,
}

impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tokens")
            .field("lifetime", &self.lifetime)
            .finish_non_exhaustive()
    }
}

/// Why the token endpoint said no.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Refused {
    /// The refresh token (or the code) is no longer valid: sign in again.
    SignInAgain(String),
    /// The app's keys are wrong (client id or secret).
    BadApp(String),
    /// Too many requests: wait.
    Quota,
    Other(String),
}

/// Reads the token endpoint's answer; `previous` is the refresh token sent
/// (kept when Netatmo does not hand a new one).
pub(crate) fn tokens(
    status: u16,
    answer: &Json,
    previous: Option<&str>,
) -> Result<Tokens, Refused> {
    if status == 429 {
        return Err(Refused::Quota);
    }
    if let Some(error) = answer["error"].as_str() {
        let detail = answer["error_description"].as_str().unwrap_or(error);
        return Err(match error {
            "invalid_grant" => Refused::SignInAgain(detail.to_owned()),
            "invalid_client" | "unauthorized_client" => Refused::BadApp(detail.to_owned()),
            _ => Refused::Other(detail.to_owned()),
        });
    }
    let access = answer["access_token"]
        .as_str()
        .filter(|t| !t.is_empty())
        .ok_or_else(|| Refused::Other(format!("no access token (HTTP {status})")))?;
    let refresh = answer["refresh_token"]
        .as_str()
        .filter(|t| !t.is_empty())
        .or(previous)
        .ok_or_else(|| Refused::Other(format!("no refresh token (HTTP {status})")))?;
    let seconds = answer["expires_in"]
        .as_u64()
        .or_else(|| answer["expire_in"].as_u64())
        .unwrap_or(3 * 3600);
    Ok(Tokens {
        access: access.to_owned(),
        refresh: refresh.to_owned(),
        lifetime: Duration::from_secs(seconds),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn app() -> App {
        App {
            client_id: "client-id-example".into(),
            client_secret: "client-secret-example".into(),
        }
    }

    #[test]
    fn the_authorization_page_asks_for_the_heating_only() {
        let a = SignIn::new(&app(), "http://localhost/moli-netatmo").unwrap();
        let b = SignIn::new(&app(), "http://localhost/moli-netatmo").unwrap();
        assert_ne!(a.state, b.state);
        assert!(
            a.url
                .starts_with("https://api.netatmo.com/oauth2/authorize?")
        );
        assert_eq!(
            encode::query_param(&a.url, "scope").as_deref(),
            Some(SCOPES)
        );
        assert_eq!(
            encode::query_param(&a.url, "state").as_deref(),
            Some(a.state.as_str())
        );
        assert_eq!(
            encode::query_param(&a.url, "redirect_uri").as_deref(),
            Some("http://localhost/moli-netatmo")
        );
        assert!(!a.url.contains("client-secret-example"));
        assert!(!format!("{:?}", app()).contains("secret-example"));
    }

    #[test]
    fn the_code_is_found_in_the_address_or_alone() {
        let end = "http://localhost/moli-netatmo?state=abc&code=0123456789abcdef";
        assert_eq!(code_of(end, "abc").as_deref(), Ok("0123456789abcdef"));
        assert_eq!(
            code_of(&format!("  {end}\n"), "abc").as_deref(),
            Ok("0123456789abcdef")
        );
        assert_eq!(code_of(end, "other"), Err(Pasted::OldPage));
        assert_eq!(
            code_of(" 0123456789abcdef ", "abc").as_deref(),
            Ok("0123456789abcdef")
        );
        assert_eq!(
            code_of(
                "http://localhost/moli-netatmo?state=abc&error=access_denied",
                "abc"
            ),
            Err(Pasted::Denied)
        );
        assert_eq!(
            code_of("http://localhost/moli-netatmo?state=abc", "abc"),
            Err(Pasted::NoCode)
        );
        assert_eq!(code_of("bonjour tout le monde", "abc"), Err(Pasted::NoCode));
        assert_eq!(code_of("short", "abc"), Err(Pasted::NoCode));
    }

    #[test]
    fn the_forms_carry_what_netatmo_asks() {
        let f = code_form(&app(), "the-code-123", "http://localhost/x");
        assert!(f.contains("grant_type=authorization_code"), "{f}");
        assert!(f.contains("code=the-code-123"), "{f}");
        assert!(f.contains("redirect_uri=http%3A%2F%2Flocalhost%2Fx"), "{f}");
        let r = refresh_form(&app(), "refresh-1");
        assert!(
            r.contains("grant_type=refresh_token&refresh_token=refresh-1"),
            "{r}"
        );
    }

    #[test]
    fn a_renewal_keeps_the_newest_refresh_token() {
        let rotated = tokens(
            200,
            &json!({ "access_token": "access-2", "refresh_token": "refresh-2", "expires_in": 10800 }),
            Some("refresh-1"),
        )
        .unwrap();
        assert_eq!(rotated.refresh, "refresh-2");
        assert_eq!(rotated.lifetime, Duration::from_secs(10800));
        let same = tokens(
            200,
            &json!({ "access_token": "access-2", "expire_in": 60 }),
            Some("refresh-1"),
        )
        .unwrap();
        assert_eq!(same.refresh, "refresh-1");
        assert_eq!(same.lifetime, Duration::from_secs(60));
        assert!(!format!("{rotated:?}").contains("refresh-2"));
    }

    #[test]
    fn token_refusals_are_told_apart() {
        assert!(matches!(
            tokens(400, &json!({ "error": "invalid_grant" }), Some("r")),
            Err(Refused::SignInAgain(_))
        ));
        assert!(matches!(
            tokens(400, &json!({ "error": "invalid_client" }), Some("r")),
            Err(Refused::BadApp(_))
        ));
        assert_eq!(tokens(429, &Json::Null, Some("r")), Err(Refused::Quota));
        assert!(matches!(
            tokens(200, &json!({}), Some("r")),
            Err(Refused::Other(_))
        ));
    }
}
