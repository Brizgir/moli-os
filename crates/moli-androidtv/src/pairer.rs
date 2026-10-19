//! The pairing as a driver runs it, from the dashboard: two points (the
//! state, `pairing`, and the code a person types, `pairing_code`, secret),
//! a code asked of the TV when someone asks (or by itself, once, for a
//! driver that is nothing without it), the certificate kept on success
//! (`atv_cert`, `atv_key`) and the remote started.

use std::time::Duration;

use anyhow::Context as _;
use moli_core::{Access, DeviceId, Kind, PointSpec, Semantic, Value};
use moli_runtime::{CommandRequest, DriverCtx};
use tokio::task::JoinHandle;

use crate::link::{ATV_CERT, ATV_KEY, Android};
use crate::pairing::{self, ClientCert, Pairing, Refused};

/// Unpaired, the TV asleep: asked again this often for a code, until it
/// showed one once (then only when a person asks: a code popping up on the
/// TV at every start would be a nuisance).
const ASK_AGAIN: Duration = Duration::from_secs(30);
/// What the TV lists among its paired remotes.
const CLIENT_NAME: &str = "Moli";

/// `pairing`: `paired`, `code` (on the TV's screen, to type), `needed` (ask
/// for a code: write `code`), `waiting` (asking the TV).
pub const STATES: [&str; 4] = ["paired", "code", "needed", "waiting"];

/// The two points of a pairing.
#[must_use]
pub fn points() -> [PointSpec; 2] {
    [
        // Written `code`: the TV shows a new code.
        PointSpec {
            key: "pairing".into(),
            label: moli_i18n::tr!("pilotes.androidtv.appairage_etat").into(),
            kind: Kind::Enum {
                values: STATES.iter().map(|v| (*v).into()).collect(),
            },
            access: Access {
                read: true,
                write: true,
            },
            unit: None,
            semantic: Semantic::Config,
        },
        // The code a person reads on the TV: never journaled nor kept.
        PointSpec {
            key: "pairing_code".into(),
            label: moli_i18n::tr!("pilotes.androidtv.code").into(),
            kind: Kind::Text,
            access: Access {
                read: false,
                write: true,
            },
            unit: None,
            semantic: Semantic::Secret,
        },
    ]
}

pub struct Pairer {
    host: String,
    /// The driver is nothing unpaired (it waits for it, and asks by itself).
    essential: bool,
    /// The certificate offered (made once per run).
    cert: Option<ClientCert>,
    /// Asking the TV for a code.
    starting: Option<JoinHandle<anyhow::Result<Pairing>>>,
    /// The code is on screen.
    open: Option<Pairing>,
    auto: bool,
    asked: Option<tokio::time::Instant>,
}

impl std::fmt::Debug for Pairer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pairer")
            .field("open", &self.open.is_some())
            .finish_non_exhaustive()
    }
}

impl Pairer {
    /// For the TV at `host`; `essential`: the driver waits for the pairing
    /// and asks for a code by itself, unpaired.
    #[must_use]
    pub fn new(host: &str, essential: bool, paired: bool) -> Self {
        Self {
            host: host.to_owned(),
            essential,
            cert: None,
            starting: None,
            open: None,
            auto: essential && !paired,
            asked: None,
        }
    }

    /// Says where the pairing is (the point, and the driver's status).
    pub fn show(&self, ctx: &DriverCtx, id: &DeviceId, state: &str) {
        ctx.set_state(id, "pairing", Value::Text(state.into()));
        let instance = ctx.instance();
        match state {
            "code" => ctx.wait_for(moli_i18n::tr!(
                "pilotes.androidtv.taper_le_code",
                instance = instance
            )),
            "needed" | "waiting" if self.essential => ctx.wait_for(moli_i18n::tr!(
                "pilotes.androidtv.a_appairer",
                instance = instance
            )),
            _ => ctx.ready(),
        }
    }

    fn start(&mut self) {
        if self.starting.is_some() {
            return;
        }
        self.open = None;
        self.asked = Some(tokio::time::Instant::now());
        let cert = self.cert.clone();
        let host = self.host.clone();
        self.starting = Some(tokio::spawn(async move {
            let cert = match cert {
                Some(cert) => cert,
                // Slow (an RSA key): off the runtime's threads.
                None => tokio::task::spawn_blocking(|| pairing::new_client_cert("moli")).await??,
            };
            Pairing::start(&host, cert, CLIENT_NAME).await
        }));
    }

    /// A pairing order (`pairing`, `pairing_code`) is answered here; any
    /// other is given back.
    pub async fn order(
        &mut self,
        command: CommandRequest,
        ctx: &DriverCtx,
        id: &DeviceId,
        link: &mut Android,
    ) -> Option<CommandRequest> {
        match (&*command.key, &command.value) {
            ("pairing_code", Value::Text(code)) => {
                let code = code.to_string();
                let result = self.code(&code, ctx, id, link).await;
                command.reply(result);
            }
            ("pairing", Value::Text(word)) if &**word == "code" => {
                if let Some(why) = ctx.secrets_problem() {
                    command.reply(Err(moli_i18n::tr!("pilotes.androidtv.coffre", why = why)));
                    return None;
                }
                self.start();
                self.show(ctx, id, "waiting");
                command.reply(Ok(()));
            }
            ("pairing" | "pairing_code", _) => {
                command.reply(Err(moli_i18n::tr!("pilotes.androidtv.code_mal_forme")));
            }
            _ => return Some(command),
        }
        None
    }

    /// The code typed at the dashboard.
    async fn code(
        &mut self,
        code: &str,
        ctx: &DriverCtx,
        id: &DeviceId,
        link: &mut Android,
    ) -> Result<(), String> {
        let Some(open) = self.open.as_mut() else {
            return Err(moli_i18n::tr!("pilotes.androidtv.pas_de_code"));
        };
        match open.finish(code).await {
            Ok(cert) => {
                let kept = async {
                    ctx.store_secret(ATV_CERT, &cert.cert_pem).await?;
                    ctx.store_secret(ATV_KEY, &cert.key_pem).await
                };
                if let Err(e) = kept.await {
                    return Err(moli_i18n::tr!("pilotes.androidtv.coffre", why = e));
                }
                tracing::info!(instance = %ctx.instance(), "android tv paired");
                self.open = None;
                self.auto = false;
                link.paired(ctx);
                self.show(ctx, id, "paired");
                Ok(())
            }
            Err(Refused::Malformed) => Err(moli_i18n::tr!("pilotes.androidtv.code_mal_forme")),
            Err(Refused::Wrong) => Err(moli_i18n::tr!("pilotes.androidtv.code_faux")),
            Err(Refused::Tv(why)) => {
                tracing::warn!(instance = %ctx.instance(), error = %why, "android tv pairing refused");
                self.open = None;
                self.show(ctx, id, "needed");
                Err(moli_i18n::tr!("pilotes.androidtv.code_refuse"))
            }
        }
    }

    /// Until the TV answered a request for a code (forever without one):
    /// then [`Pairer::on_started`].
    pub async fn started(&mut self) -> anyhow::Result<Pairing> {
        match &mut self.starting {
            Some(task) => {
                let result = task.await.context("pairing task");
                self.starting = None;
                result?
            }
            None => std::future::pending().await,
        }
    }

    /// The TV shows its code, or did not answer.
    pub fn on_started(&mut self, result: anyhow::Result<Pairing>, ctx: &DriverCtx, id: &DeviceId) {
        match result {
            Ok(open) => {
                tracing::info!(instance = %ctx.instance(), tv = ?open.tv_name, "android tv: pairing code on screen");
                self.auto = false;
                self.open = Some(open);
                self.show(ctx, id, "code");
            }
            Err(e) => {
                tracing::debug!(instance = %ctx.instance(), error = %format!("{e:#}"), "android tv: no pairing code");
                self.show(ctx, id, "needed");
            }
        }
    }

    /// Every few seconds: unpaired and essential, a code is asked for by
    /// itself until the TV showed one once.
    pub fn tick(&mut self, ctx: &DriverCtx, paired: bool) {
        let due = self.auto
            && !paired
            && self.starting.is_none()
            && self.open.is_none()
            && self.asked.is_none_or(|at| at.elapsed() >= ASK_AGAIN);
        if !due {
            return;
        }
        if ctx.can_store_secrets() {
            self.start();
        } else {
            self.auto = false;
        }
    }
}
