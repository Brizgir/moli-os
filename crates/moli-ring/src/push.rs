//! Ring's push notifications, as the Ring app receives them: Moli registers
//! as a Firebase client ([`fcm`]), tells Ring its token (`PATCH
//! clients_api/device`, on the same « authorized client device » as the
//! rest: the hardware id), and stays connected to Google's push server
//! ([`mcs`]). Each press or motion Ring sends is decrypted ([`crate::ece`])
//! and handed to the driver's loop at once ([`Heard::Notified`]), which
//! tells it under the same rules as a read event (fresh, once, never from
//! before the start).
//!
//! The poll goes on beside it, as a safety net: slower while the push
//! works ([`Heard::Up`]), at its own pace again as soon as it falls
//! ([`Heard::Down`]); the push task connects again with an exponential
//! back-off. Ring's session is renewed every 12 hours with the token told
//! again (Ring forgets a client's push token after about a day outside the
//! US, as ring-client-api found).

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use moli_runtime::BoxFuture;
use serde_json::json;
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::api::{Failure, Vault};
use crate::fcm::{self, Credentials};
use crate::images::Shared;
use crate::mcs::{self, Dial, Refused};
use crate::model::{self, Pushed};
use crate::net::Transport;

/// The push server's heartbeat: a quiet line is checked this often.
const HEARTBEAT: Duration = Duration::from_secs(5 * 60);
/// Ring's session, renewed (and the token told again) this often.
const SESSION: Duration = Duration::from_secs(12 * 3600);
const RETRY_MIN: Duration = Duration::from_secs(2);
const RETRY_MAX: Duration = Duration::from_secs(5 * 60);
/// A new registration asked by the driver, not more often than this.
const RENEW_EVERY: Duration = Duration::from_secs(24 * 3600);
/// Persistent ids told back to the server at the next login.
const ACKED: usize = 100;

/// What the push task tells the driver's loop.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Heard {
    /// Connected and Ring told: events come as they happen.
    Up,
    /// Fallen: the poll is all there is until `Up` again.
    Down,
    Notified(Pushed),
}

/// What the driver's loop asks of the push task.
#[derive(Debug)]
pub(crate) enum Ask {
    /// Events came by the poll while the push said it was up: register
    /// again (at most once a day).
    Renew,
}

/// Where the push credentials go (the secret store).
pub(crate) trait Keep: Send + Sync {
    fn keep<'a>(&'a self, credentials: &'a str) -> BoxFuture<'a, anyhow::Result<()>>;
}

pub(crate) struct Push {
    pub net: Arc<dyn Transport>,
    pub dial: Arc<dyn Dial>,
    pub client: Shared,
    pub vault: Arc<dyn Vault>,
    pub keep: Arc<dyn Keep>,
    /// From the secret store; `None`: registered at the first attempt.
    pub credentials: Option<Credentials>,
    pub heard: mpsc::UnboundedSender<Heard>,
    pub asks: mpsc::UnboundedReceiver<Ask>,
}

impl std::fmt::Debug for Push {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Push").finish_non_exhaustive()
    }
}

/// Gives Ring the token it sends to, on Moli's own client device.
pub(crate) async fn tell_ring(
    client: &Shared,
    vault: &Arc<dyn Vault>,
    token: &str,
) -> Result<(), Failure> {
    let body = json!({
        "device": {
            "metadata": {
                "api_version": 11,
                "device_model": "Moli OS",
                "pn_dict_version": "2.0.0",
                "pn_service": "fcm",
            },
            "os": "android",
            "push_notification_token": token,
        }
    });
    client
        .lock()
        .await
        .patch(&**vault, "/clients_api/device", &body)
        .await
        .map(|_| ())
}

/// The state of one connection attempt, kept across attempts.
#[derive(Debug, Default)]
struct Across {
    /// Messages received, told back at the next login.
    acked: Vec<String>,
    up: bool,
    renewed: Option<Instant>,
}

impl Push {
    /// Until the driver stops it (the task is aborted with the connection
    /// to Ring).
    pub(crate) async fn run(mut self) {
        let mut across = Across::default();
        let mut pause = RETRY_MIN;
        let mut failing = false;
        loop {
            let started = Instant::now();
            let result = self.attempt(&mut across).await;
            if across.up {
                // It worked for a while: the next failure is news again.
                across.up = false;
                failing = false;
                let _ = self.heard.send(Heard::Down);
            }
            let wait = match result {
                // Asked to register again: at once.
                Ok(()) => Duration::ZERO,
                Err(e) => {
                    let e = format!("{e:#}");
                    if failing {
                        tracing::debug!(error = %e, "ring push still down");
                    } else {
                        tracing::warn!(error = %e, "ring push down: events come by the poll");
                    }
                    failing = true;
                    if started.elapsed() > RETRY_MAX {
                        pause = RETRY_MIN;
                    }
                    let wait = pause;
                    pause = (pause * 2).min(RETRY_MAX);
                    wait
                }
            };
            tokio::time::sleep(wait).await;
        }
    }

    /// The credentials: kept ones checked in again, or a new registration
    /// (kept before use, so a restart does not register once more).
    async fn credentials(&mut self, across: &mut Across) -> anyhow::Result<Credentials> {
        if let Some(c) = &self.credentials {
            if let Err(e) = fcm::checkin(&*self.net, Some((c.android_id, c.security_token))).await {
                tracing::debug!(error = %format!("{e:#}"), "ring push check-in");
            }
            return Ok(c.clone());
        }
        let c = fcm::register(&*self.net).await?;
        self.keep
            .keep(&c.to_text())
            .await
            .context("the push credentials could not be kept")?;
        tracing::info!("ring push registered");
        across.acked.clear();
        self.credentials = Some(c.clone());
        Ok(c)
    }

    /// One connection, until it falls (`Err`) or a new registration is
    /// asked (`Ok`).
    async fn attempt(&mut self, across: &mut Across) -> anyhow::Result<()> {
        let c = self.credentials(across).await?;
        tell_ring(&self.client, &self.vault, &c.token)
            .await
            .map_err(|e| anyhow::anyhow!("Ring did not take the push token: {e}"))?;
        let stream = self.dial.dial().await?;
        let login = mcs::login(
            stream,
            c.android_id,
            c.security_token,
            &across.acked,
            HEARTBEAT,
        )
        .await;
        let mut conn = match login {
            Err(e) if e.is::<Refused>() => {
                // A device Google no longer knows: registered again.
                self.credentials = None;
                return Err(e);
            }
            other => other?,
        };
        across.acked.clear();
        across.up = true;
        let _ = self.heard.send(Heard::Up);
        tracing::info!("ring push connected");
        let mut session = tokio::time::interval_at(Instant::now() + SESSION, SESSION);
        loop {
            tokio::select! {
                message = conn.next() => {
                    let message = message?;
                    if let Some(id) = message.persistent_id.clone() {
                        across.acked.push(id);
                        if across.acked.len() > ACKED {
                            across.acked.remove(0);
                        }
                    }
                    match fcm::open(&c.keys, &message.app_data, &message.raw) {
                        Ok(content) => {
                            if let Some(p) = model::pushed(&content) {
                                let _ = self.heard.send(Heard::Notified(p));
                            } else {
                                tracing::debug!("ring push: not a press nor a motion");
                            }
                        }
                        // Firebase sometimes sends one that does not decrypt;
                        // the next ones do.
                        Err(e) => tracing::debug!(error = %format!("{e:#}"), "ring push unreadable"),
                    }
                }
                _ = session.tick() => {
                    if let Err(e) = self.client.lock().await.session(&*self.vault).await {
                        tracing::debug!(error = %e, "ring session");
                    }
                    tell_ring(&self.client, &self.vault, &c.token)
                        .await
                        .map_err(|e| anyhow::anyhow!("Ring did not take the push token: {e}"))?;
                }
                Some(Ask::Renew) = self.asks.recv() => {
                    if across.renewed.is_none_or(|t| t.elapsed() > RENEW_EVERY) {
                        across.renewed = Some(Instant::now());
                        tracing::warn!("ring push: events came by the poll only, registering again");
                        self.credentials = None;
                        return Ok(());
                    }
                }
            }
        }
    }
}
