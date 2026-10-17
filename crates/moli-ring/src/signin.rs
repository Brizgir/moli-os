//! The sign-in a person makes from the dashboard: the account's e-mail and
//! password, then the code Ring sends. Moli passes them to Ring and keeps
//! them in memory only while it waits for the code (Ring wants the password
//! again along with it); they are never stored, journaled or shown.

use std::time::Duration;

use serde::Deserialize;
use tokio::time::Instant;

use crate::auth::{self, Answer, Grant, Tokens};
use crate::net::Transport;

/// How long Moli waits for the code before forgetting the password.
pub(crate) const CODE_WAIT: Duration = Duration::from_secs(10 * 60);

#[derive(Deserialize)]
struct Typed {
    email: String,
    password: String,
}

/// The e-mail and the password, in memory only.
struct Credentials {
    email: String,
    password: String,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials").finish_non_exhaustive()
    }
}

/// Why a step did not sign in: a key of the `pilotes.ring` catalogue, and
/// Ring's words when they help.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// Not an e-mail and a password.
    Typed,
    BadPassword,
    BadCode,
    /// A code, but no e-mail and password before it (or they were forgotten).
    NoCredentials,
    TooMany,
    Other(String),
}

#[derive(Debug)]
pub(crate) struct SignIn {
    pub hardware_id: String,
    waiting: Option<(Credentials, String, Instant)>,
}

impl SignIn {
    pub(crate) fn new(hardware_id: String) -> Self {
        Self {
            hardware_id,
            waiting: None,
        }
    }

    /// How Ring sent the code it waits for (`sms`, `email`, `totp`).
    pub(crate) fn code_by(&self) -> Option<&str> {
        self.waiting.as_ref().map(|(_, by, _)| by.as_str())
    }

    /// When the password is forgotten, if Moli waits for a code.
    pub(crate) fn deadline(&self) -> Option<Instant> {
        self.waiting
            .as_ref()
            .map(|(_, _, since)| *since + CODE_WAIT)
    }

    pub(crate) fn forget(&mut self) {
        self.waiting = None;
    }

    /// The e-mail and password typed (`{"email": …, "password": …}`).
    /// `Ok(None)`: Ring wants a code now.
    pub(crate) async fn credentials(
        &mut self,
        net: &dyn Transport,
        typed: &str,
    ) -> Result<Option<Tokens>, Refusal> {
        self.waiting = None;
        let typed: Typed = serde_json::from_str(typed).map_err(|_| Refusal::Typed)?;
        let email = typed.email.trim();
        if !email.contains('@') || typed.password.is_empty() {
            return Err(Refusal::Typed);
        }
        let credentials = Credentials {
            email: email.to_owned(),
            password: typed.password,
        };
        let answer = self.ask(net, &credentials, None).await?;
        match answer {
            Answer::Tokens(t) => Ok(Some(t)),
            Answer::Code { by } => {
                self.waiting = Some((credentials, by, Instant::now()));
                Ok(None)
            }
            Answer::BadPassword | Answer::Revoked => Err(Refusal::BadPassword),
            other => Err(refusal(other)),
        }
    }

    /// The code Ring sent.
    pub(crate) async fn code(
        &mut self,
        net: &dyn Transport,
        code: &str,
    ) -> Result<Tokens, Refusal> {
        let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
        if code.is_empty() || code.len() > 10 || !code.chars().all(|c| c.is_ascii_digit()) {
            return Err(Refusal::BadCode);
        }
        let Some((credentials, by, since)) = self.waiting.take() else {
            return Err(Refusal::NoCredentials);
        };
        let answer = self.ask(net, &credentials, Some(&code)).await;
        let keep = |me: &mut Self, credentials| me.waiting = Some((credentials, by, since));
        match answer {
            Ok(Answer::Tokens(t)) => Ok(t),
            Ok(Answer::BadCode | Answer::Code { .. }) => {
                keep(self, credentials);
                Err(Refusal::BadCode)
            }
            Ok(Answer::BadPassword | Answer::Revoked) => Err(Refusal::BadPassword),
            Ok(other) => {
                keep(self, credentials);
                Err(refusal(other))
            }
            Err(e) => {
                keep(self, credentials);
                Err(e)
            }
        }
    }

    async fn ask(
        &self,
        net: &dyn Transport,
        credentials: &Credentials,
        code: Option<&str>,
    ) -> Result<Answer, Refusal> {
        let grant = Grant::Password {
            email: &credentials.email,
            password: &credentials.password,
            code,
        };
        auth::token(net, &self.hardware_id, &grant)
            .await
            .map_err(|e| Refusal::Other(format!("{e:#}")))
    }
}

fn refusal(answer: Answer) -> Refusal {
    match answer {
        Answer::TooMany => Refusal::TooMany,
        Answer::BadCode => Refusal::BadCode,
        Answer::BadPassword | Answer::Revoked => Refusal::BadPassword,
        Answer::Other(why) => Refusal::Other(why),
        Answer::Tokens(_) | Answer::Code { .. } => Refusal::Other("unexpected answer".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::replay::Replay;

    const TYPED: &str = r#"{"email":" user@example.com ","password":"not-a-real-one"}"#;
    const TOKEN: &str = include_str!("../../../integrations/ring/fixtures/oauth-token.json");
    const TWO_FA: &str = include_str!("../../../integrations/ring/fixtures/oauth-2fa.json");
    const BAD_CODE: &str = include_str!("../../../integrations/ring/fixtures/oauth-bad-code.json");

    #[tokio::test]
    async fn the_code_ring_asks_for_then_the_tokens() {
        let net = Replay::default()
            .then("/oauth/token", 412, TWO_FA)
            .then("/oauth/token", 400, BAD_CODE)
            .then("/oauth/token", 200, TOKEN);
        let mut sign_in = SignIn::new("hw-test".into());
        assert_eq!(sign_in.credentials(&net, TYPED).await, Ok(None));
        assert_eq!(sign_in.code_by(), Some("sms"));
        assert!(sign_in.deadline().is_some());
        assert_eq!(
            sign_in.code(&net, "000000").await,
            Err(Refusal::BadCode),
            "a wrong code: try again"
        );
        assert_eq!(sign_in.code_by(), Some("sms"), "still waiting");
        let tokens = sign_in.code(&net, " 123 456 ").await.unwrap();
        assert_eq!(tokens.refresh, "fake-refresh-token");
        assert_eq!(sign_in.code_by(), None, "the password is forgotten");
        // Each time the e-mail and password go to Ring, with the code after.
        let (_, first, body) = net.asked(0);
        assert_eq!(body["username"], "user@example.com");
        assert_eq!(body["grant_type"], "password");
        assert_eq!(first["2fa-code"], "");
        assert_eq!(first["2fa-support"], "true");
        assert_eq!(first["hardware_id"], "hw-test");
        let (_, last, body) = net.asked(2);
        assert_eq!(last["2fa-code"], "123456");
        assert_eq!(body["password"], "not-a-real-one");
        assert!(!format!("{sign_in:?}").contains("not-a-real-one"));
    }

    #[tokio::test]
    async fn a_wrong_password_or_odd_typing_is_refused() {
        let net = Replay::default().then(
            "/oauth/token",
            401,
            include_str!("../../../integrations/ring/fixtures/oauth-bad-password.json"),
        );
        let mut sign_in = SignIn::new("hw-test".into());
        assert_eq!(
            sign_in.credentials(&net, TYPED).await,
            Err(Refusal::BadPassword)
        );
        for odd in ["", "x", r#"{"email":"nobody","password":"x"}"#] {
            assert_eq!(sign_in.credentials(&net, odd).await, Err(Refusal::Typed));
        }
        assert_eq!(net.left(), 0, "nothing odd sent to Ring");
        assert_eq!(
            sign_in.code(&net, "123456").await,
            Err(Refusal::NoCredentials)
        );
        assert_eq!(sign_in.code(&net, "12a456").await, Err(Refusal::BadCode));
    }

    #[tokio::test]
    async fn without_two_steps_the_password_gives_the_tokens() {
        let net = Replay::default().then("/oauth/token", 200, TOKEN);
        let mut sign_in = SignIn::new("hw-test".into());
        assert!(sign_in.credentials(&net, TYPED).await.unwrap().is_some());
        let net = Replay::default().then("/oauth/token", 429, "");
        assert_eq!(
            sign_in.credentials(&net, TYPED).await,
            Err(Refusal::TooMany)
        );
    }
}
