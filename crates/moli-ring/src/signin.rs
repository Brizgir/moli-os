//! The sign-in a person makes from the dashboard: they paste a Ring
//! refresh token made on their own computer (`ring-auth-cli`, where Ring
//! itself asks the e-mail, the password and the one-time code). Moli never
//! sees the password; it checks the token with Ring once, keeps what Ring
//! gives back, and never logs, journals or shows what was pasted.
//!
//! Two shapes are accepted: the bare refresh token, or what `ring-auth-cli`
//! prints, a base64 `{"rt": <refresh token>, "hid": <hardware id>}`: then
//! Moli takes that hardware id too, so it is the same « authorized client
//! device » of the account as the computer that made the token.

use serde::Deserialize;

use crate::auth::{self, Answer, Tokens};
use crate::net::Transport;

/// A pasted text longer than this is not a token.
const MAX_PASTED: usize = 8 * 1024;

/// Why a pasted text did not sign in: a key of the `pilotes.ring` catalogue,
/// and Ring's words when they help.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// Not a Ring token.
    Pasted,
    /// Ring does not accept it (revoked, used up, made for another account
    /// client): a new one is needed.
    Refused,
    TooMany,
    Other(String),
}

/// The account, signed in with the pasted token.
pub(crate) struct Connected {
    /// The hardware id the tokens are tied to: the pasted one, else `ours`.
    pub hardware_id: String,
    pub tokens: Tokens,
}

impl std::fmt::Debug for Connected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connected").finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
struct Wrapped {
    rt: String,
    #[serde(default)]
    hid: Option<String>,
}

/// What `ring-auth-cli` prints, or a bare token: (refresh token, hardware
/// id if the text names one). `None`: not a token at all (nothing is
/// sent to Ring).
fn parse(pasted: &str) -> Option<(String, Option<String>)> {
    let pasted = pasted.trim();
    if pasted.is_empty() || pasted.len() > MAX_PASTED || !pasted.is_ascii() {
        return None;
    }
    // A token is one word; a pasted terminal line may be trimmed of quotes.
    let pasted = pasted.trim_matches(|c| c == '"' || c == '\'');
    if pasted.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    if let Some(wrapped) = base64(pasted)
        .and_then(|bytes| serde_json::from_slice::<Wrapped>(&bytes).ok())
        .filter(|w| token_like(&w.rt))
    {
        let hid = wrapped.hid.filter(|h| {
            (8..=64).contains(&h.len())
                && h.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ':')
        });
        return Some((wrapped.rt, hid));
    }
    token_like(pasted).then(|| (pasted.to_owned(), None))
}

fn token_like(s: &str) -> bool {
    s.len() >= 16
        && s.len() <= MAX_PASTED
        && s.chars()
            .all(|c| c.is_ascii_graphic() && !matches!(c, '"' | '\'' | '<' | '>' | '\\'))
}

/// Standard or URL-safe base64, padding optional; `None` if not base64.
pub(crate) fn base64(text: &str) -> Option<Vec<u8>> {
    let text = text.trim_end_matches('=');
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((acc >> bits) & 0xff).ok()?);
        }
    }
    Some(out)
}

/// The pasted token, checked with Ring: the tokens Ring gives for it (the
/// refresh token is replaced at once: the pasted one is spent). `ours`:
/// the hardware id kept from an earlier sign-in, if any.
pub(crate) async fn connect(
    net: &dyn Transport,
    pasted: &str,
    ours: Option<&str>,
) -> Result<Connected, Refusal> {
    let Some((refresh, hid)) = parse(pasted) else {
        return Err(Refusal::Pasted);
    };
    let hardware_id = match (hid, ours) {
        (Some(hid), _) => hid,
        (None, Some(ours)) => ours.to_owned(),
        (None, None) => auth::hardware_id().map_err(|e| Refusal::Other(format!("{e:#}")))?,
    };
    let answer = auth::token(net, &hardware_id, &refresh)
        .await
        .map_err(|e| Refusal::Other(format!("{e:#}")))?;
    match answer {
        Answer::Tokens(tokens) => Ok(Connected {
            hardware_id,
            tokens,
        }),
        Answer::Revoked => Err(Refusal::Refused),
        Answer::TooMany => Err(Refusal::TooMany),
        Answer::Other(why) => Err(Refusal::Other(why)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::replay::Replay;

    const TOKEN: &str = include_str!("../../../integrations/ring/fixtures/oauth-token.json");
    const BAD: &str = include_str!("../../../integrations/ring/fixtures/oauth-bad-refresh.json");

    /// What `ring-auth-cli` prints: base64 of `{"rt": …, "hid": …}`.
    fn wrapped(rt: &str, hid: Option<&str>) -> String {
        let json = match hid {
            Some(hid) => format!(r#"{{"rt":"{rt}","hid":"{hid}"}}"#),
            None => format!(r#"{{"rt":"{rt}"}}"#),
        };
        let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in json.as_bytes().chunks(3) {
            let n = chunk
                .iter()
                .enumerate()
                .fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
            for i in 0..4 {
                if i <= chunk.len() {
                    out.push(alphabet[((n >> (18 - 6 * i)) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    #[test]
    fn a_pasted_text_is_a_bare_token_or_what_the_cli_prints() {
        assert_eq!(
            parse("  fake-refresh-token-pasted-0\n"),
            Some(("fake-refresh-token-pasted-0".into(), None))
        );
        assert_eq!(
            parse(&wrapped(
                "fake-refresh-token-pasted-1",
                Some("hw-from-the-cli")
            )),
            Some((
                "fake-refresh-token-pasted-1".into(),
                Some("hw-from-the-cli".into())
            ))
        );
        let no_hid = wrapped("fake-refresh-token-pasted-2", None);
        assert_eq!(
            parse(&format!("\"{no_hid}\"")),
            Some(("fake-refresh-token-pasted-2".into(), None))
        );
        // A hardware id that is not one is left out, the token kept.
        assert_eq!(
            parse(&wrapped("fake-refresh-token-pasted-3", Some("x"))),
            Some(("fake-refresh-token-pasted-3".into(), None))
        );
        for odd in [
            "",
            "short",
            "two words here are not a token",
            "user@example.com\npassword",
            &"x".repeat(MAX_PASTED + 1),
            "<script>alert(1)</script>xx",
            "jeton-avec-accent-é-non-ascii",
        ] {
            assert_eq!(parse(odd), None, "{odd:?}");
        }
    }

    #[tokio::test]
    async fn the_pasted_token_is_checked_with_ring_and_its_hardware_id_taken() {
        let net = Replay::default().then("/oauth/token", 200, TOKEN);
        let pasted = wrapped("fake-refresh-token-pasted-1", Some("hw-from-the-cli"));
        let c = connect(&net, &pasted, Some("hw-ours")).await.unwrap();
        assert_eq!(c.hardware_id, "hw-from-the-cli");
        assert_eq!(c.tokens.refresh, "fake-refresh-token");
        assert!(!format!("{c:?}").contains("fake"));
        let (_, headers, body) = net.asked(0);
        assert_eq!(headers["hardware_id"], "hw-from-the-cli");
        assert_eq!(body["refresh_token"], "fake-refresh-token-pasted-1");
        assert_eq!(body["grant_type"], "refresh_token");
        assert!(body.get("password").is_none());
        // A bare token: the hardware id kept from before, else a new one.
        let net = Replay::default().then("/oauth/token", 200, TOKEN);
        let c = connect(&net, "fake-refresh-token-pasted-2", Some("hw-ours"))
            .await
            .unwrap();
        assert_eq!(c.hardware_id, "hw-ours");
        let net = Replay::default().then("/oauth/token", 200, TOKEN);
        let c = connect(&net, "fake-refresh-token-pasted-2", None)
            .await
            .unwrap();
        assert_eq!(c.hardware_id.len(), 36);
    }

    #[tokio::test]
    async fn what_is_not_a_token_never_reaches_ring_and_a_refused_one_says_so() {
        let net = Replay::default().then("/oauth/token", 401, BAD);
        for odd in ["", "x", r#"{"email":"nobody","password":"x"}"#] {
            assert_eq!(connect(&net, odd, None).await.unwrap_err(), Refusal::Pasted);
        }
        assert_eq!(net.left(), 1, "nothing odd sent to Ring");
        assert_eq!(
            connect(&net, "fake-refresh-token-revoked", None)
                .await
                .unwrap_err(),
            Refusal::Refused
        );
        let net = Replay::default().then("/oauth/token", 429, "");
        assert_eq!(
            connect(&net, "fake-refresh-token-pasted-9", None)
                .await
                .unwrap_err(),
            Refusal::TooMany
        );
    }
}
