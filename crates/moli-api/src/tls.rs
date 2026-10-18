//! HTTPS, served by Moli itself (`[server.tls]`): a second listener, next
//! to the HTTP one, for the browsers that want a secure context before they
//! open the microphone (the voice, from a phone that reaches the house
//! through its VPN). No relay in front of Moli: the peer's address stays the
//! real one, so who is calling is judged exactly as over HTTP
//! ([`crate::caller`]).
//!
//! - the certificate and its key are files (a `tailscale cert`, a Let's
//!   Encrypt…), read at the start and **again when they change** (their
//!   modification time is looked at every minute, and at `SIGHUP`): a
//!   renewal needs no restart;
//! - a certificate missing, unreadable, expired or not yet valid closes the
//!   HTTPS listener (connections are dropped at once) with a plain reason in
//!   `/api/health` and the Système card; HTTP goes on;
//! - the house is told 14 days before the certificate expires, and when the
//!   listener closes or opens again;
//! - the names the certificate carries (its SANs) are hosts this instance is
//!   reached by ([`crate::guard`]), nothing to add to `allowed_hosts`;
//! - cookies set over HTTPS are `Secure` ([`Encrypted`]);
//! - TLS 1.2 and 1.3 only, rustls's safe defaults (AEAD suites, no weak
//!   algorithm), HTTP/1.1 announced over ALPN.
//!
//! The key never leaves its file: it is neither logged, nor kept in the
//! secret store, nor shown anywhere.

use std::fmt;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use axum::serve::{ListenerExt as _, TapIo};
use moli_core::now_ms;
use moli_runtime::Hub;
use rustls::ServerConfig;
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use serde_json::{Value as Json, json};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::server::TlsStream;
use tokio_util::sync::CancellationToken;

/// How often the files are looked at (two `stat`s: nothing).
const POLL: Duration = Duration::from_secs(60);
/// The house is told this long before the certificate expires.
const WARN_BEFORE_MS: u64 = 14 * 24 * 3_600_000;
/// A handshake that takes longer is someone holding a socket, not a browser.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Handshakes in flight at once; the rest wait in the kernel's backlog.
const MAX_HANDSHAKES: usize = 64;
/// The SAN extension: `id-ce-subjectAltName`, 2.5.29.17.
const OID_SAN: &[u8] = &[0x55, 0x1d, 0x11];

/// `[server.tls]`.
#[derive(Clone, Debug)]
pub struct TlsOptions {
    /// The certificate chain (PEM): the leaf first.
    pub cert_file: PathBuf,
    /// Its private key (PEM, PKCS#8, SEC1 or PKCS#1). Never printed.
    pub key_file: PathBuf,
    /// Where the HTTPS listener binds (the HTTP address with its own port).
    pub listen: SocketAddr,
}

/// Marks a request that came in over HTTPS (the TLS listener's router adds
/// it): its cookies are `Secure`.
#[derive(Clone, Copy, Debug)]
pub struct Encrypted;

/// What a certificate says of itself, as far as Moli cares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CertInfo {
    /// Its subject alternative names: DNS names (lower case) and addresses.
    pub(crate) names: Vec<String>,
    /// Validity, in milliseconds since the epoch.
    pub(crate) not_before: u64,
    pub(crate) not_after: u64,
}

/// Why the HTTPS listener is closed.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Closed {
    Unreadable(String),
    Expired,
    NotYetValid,
}

impl Closed {
    fn describe(&self, cert: Option<&CertInfo>) -> String {
        match self {
            Self::Unreadable(error) => {
                moli_i18n::tr!("serveur.tls.raison.illisible", error = error)
            }
            Self::Expired => moli_i18n::tr!(
                "serveur.tls.raison.expire",
                date = date(cert.map_or(0, |c| c.not_after))
            ),
            Self::NotYetValid => moli_i18n::tr!(
                "serveur.tls.raison.pas_encore",
                date = date(cert.map_or(0, |c| c.not_before))
            ),
        }
    }
}

#[derive(Default)]
struct State {
    /// What the listener serves; `None`: closed.
    config: Option<Arc<ServerConfig>>,
    /// The last certificate read (kept while expired: its dates say why).
    cert: Option<CertInfo>,
    closed: Option<Closed>,
    /// Modification times last seen (certificate, key).
    stamps: Option<(Option<SystemTime>, Option<SystemTime>)>,
    /// The expiry the house was already warned about.
    warned: Option<u64>,
    /// Whether the house was told the listener is closed (told once per
    /// closing, and once when it reopens).
    told_closed: bool,
}

struct Inner {
    options: TlsOptions,
    hub: Hub,
    state: Mutex<State>,
}

/// The HTTPS listener's certificate, as it is right now; shared by the
/// listener, the host guard and the surfaces that show its state.
#[derive(Clone)]
pub struct Tls(Arc<Inner>);

impl fmt::Debug for Tls {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tls")
            .field("listen", &self.0.options.listen)
            .field("open", &self.current().is_some())
            .finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Tls {
    /// Reads the certificate once (a missing or bad one is not an error: the
    /// listener starts closed and says why) and tells the house.
    #[must_use]
    pub fn new(options: TlsOptions, hub: Hub) -> Self {
        let tls = Self(Arc::new(Inner {
            options,
            hub,
            state: Mutex::new(State::default()),
        }));
        tls.reload();
        tls
    }

    pub fn listen(&self) -> SocketAddr {
        self.0.options.listen
    }

    /// What the listener serves right now, if open.
    pub(crate) fn current(&self) -> Option<Arc<ServerConfig>> {
        lock(&self.0.state).config.clone()
    }

    /// The names of the certificate (lower case), for the host guard: those
    /// of the last certificate read, even expired (a browser that reaches a
    /// closed listener gets no answer anyway).
    pub(crate) fn names(&self) -> Vec<String> {
        lock(&self.0.state)
            .cert
            .as_ref()
            .map(|c| c.names.clone())
            .unwrap_or_default()
    }

    /// For `/api/health` and `/api/system`: never the key, never its path.
    pub(crate) fn status(&self) -> Json {
        let state = lock(&self.0.state);
        let cert = state.cert.as_ref();
        json!({
            "listen": self.0.options.listen.to_string(),
            "port": self.0.options.listen.port(),
            "open": state.config.is_some(),
            "reason": state.closed.as_ref().map(|c| c.describe(cert)),
            "names": cert.map(|c| c.names.clone()).unwrap_or_default(),
            "expires": cert.map(|c| c.not_after),
            "expires_in_days": cert.map(|c| c.not_after.saturating_sub(now_ms()) / 86_400_000),
            "cert_file": self.0.options.cert_file.display().to_string(),
        })
    }

    /// Reads the files again, whatever their dates (a `SIGHUP`).
    pub fn reload(&self) {
        let stamps = self.stamps();
        let read = load(&self.0.options.cert_file, &self.0.options.key_file);
        let mut state = lock(&self.0.state);
        state.stamps = Some(stamps);
        match read {
            Ok((config, cert)) => {
                state.cert = Some(cert);
                state.config = Some(Arc::new(config));
                state.closed = None;
            }
            Err(e) => {
                tracing::warn!(error = format!("{e:#}"), "HTTPS certificate not loaded");
                state.config = None;
                // The whole chain: the file, then why (missing, bad PEM…).
                state.closed = Some(Closed::Unreadable(format!("{e:#}")));
            }
        }
        drop(state);
        self.check();
    }

    fn stamps(&self) -> (Option<SystemTime>, Option<SystemTime>) {
        let mtime = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        (
            mtime(&self.0.options.cert_file),
            mtime(&self.0.options.key_file),
        )
    }

    /// Every minute: files changed (or still unreadable) → read again;
    /// then the dates, against the clock.
    fn tick(&self) {
        let stamps = self.stamps();
        let again = {
            let state = lock(&self.0.state);
            state.stamps != Some(stamps) || matches!(state.closed, Some(Closed::Unreadable(_)))
        };
        if again {
            self.reload();
        } else {
            self.check();
        }
    }

    /// The certificate against the clock: closes an expired one, reopens
    /// one whose time has come, warns ahead; tells the house of changes.
    fn check(&self) {
        let now = now_ms();
        let mut state = lock(&self.0.state);
        let Some(cert) = state.cert.clone() else {
            // Unreadable: told below, like any closing.
            return self.tell(&mut state);
        };
        if matches!(state.closed, Some(Closed::Unreadable(_))) {
            return self.tell(&mut state);
        }
        if now >= cert.not_after {
            state.config = None;
            state.closed = Some(Closed::Expired);
        } else if now < cert.not_before {
            state.config = None;
            state.closed = Some(Closed::NotYetValid);
        } else if state.config.is_none() {
            // Its time has come (a certificate dated in the future, read
            // early), or a previous reload failed: read again.
            drop(state);
            return self.reload();
        } else {
            state.closed = None;
            if cert.not_after - now < WARN_BEFORE_MS && state.warned != Some(cert.not_after) {
                state.warned = Some(cert.not_after);
                let days = (cert.not_after - now) / 86_400_000;
                tracing::warn!(days, "HTTPS certificate expires soon");
                self.0.hub.notice(notice(moli_i18n::tr!(
                    "serveur.tls.notice_expire",
                    names = cert.names.join(", "),
                    days = days,
                    date = date(cert.not_after)
                )));
            }
        }
        self.tell(&mut state);
    }

    /// Tells the house once when the listener closes, once when it opens
    /// again.
    fn tell(&self, state: &mut State) {
        match (&state.closed, state.told_closed) {
            (Some(closed), false) => {
                state.told_closed = true;
                let reason = closed.describe(state.cert.as_ref());
                tracing::warn!(%reason, "HTTPS closed");
                self.0.hub.notice(notice(moli_i18n::tr!(
                    "serveur.tls.notice_ferme",
                    reason = reason
                )));
            }
            (None, true) => {
                state.told_closed = false;
                if let Some(cert) = &state.cert {
                    tracing::info!(names = ?cert.names, "HTTPS open again");
                    self.0.hub.notice(notice(moli_i18n::tr!(
                        "serveur.tls.notice_ouvert",
                        names = cert.names.join(", "),
                        date = date(cert.not_after)
                    )));
                }
            }
            _ => {}
        }
    }

    /// Watches the files and the clock until `cancel`; `SIGHUP` reads them
    /// at once.
    pub fn watch(self, cancel: CancellationToken) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut every = tokio::time::interval(POLL);
            every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            every.tick().await;
            #[cfg(unix)]
            let mut hangup =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup()).ok();
            loop {
                #[cfg(unix)]
                let hup = async {
                    match hangup.as_mut() {
                        Some(h) => h.recv().await,
                        None => std::future::pending().await,
                    }
                };
                #[cfg(not(unix))]
                let hup = std::future::pending::<Option<()>>();
                tokio::select! {
                    () = cancel.cancelled() => return,
                    _ = every.tick() => self.tick(),
                    _ = hup => {
                        tracing::info!("SIGHUP: reading the HTTPS certificate again");
                        self.reload();
                    }
                }
            }
        })
    }

    /// Binds the HTTPS listener. The socket is bound even while the
    /// certificate is missing: connections are then dropped, and a renewed
    /// certificate opens it without a restart. The peer's address is the
    /// `ConnectInfo<SocketAddr>` of each request, as over HTTP.
    pub async fn bind(&self) -> io::Result<HttpsListener> {
        let tcp = TcpListener::bind(self.0.options.listen).await?;
        let (tx, ready) = mpsc::channel(MAX_HANDSHAKES);
        let listener = TlsListener {
            tcp,
            tls: self.clone(),
            tx,
            ready,
            handshakes: Arc::new(tokio::sync::Semaphore::new(MAX_HANDSHAKES)),
        };
        Ok(listener.tap_io(nodelay))
    }
}

fn notice(message: String) -> moli_core::Notice {
    moli_core::Notice {
        title: Some(moli_i18n::tr!("serveur.tls.notice_titre")),
        message,
        from: Some("Moli".into()),
        ts: now_ms(),
    }
}

/// The certificate chain and key, read and checked: a server
/// configuration, and what the leaf says of itself.
fn load(cert_file: &Path, key_file: &Path) -> anyhow::Result<(ServerConfig, CertInfo)> {
    use anyhow::Context as _;
    let cert_pem =
        std::fs::read(cert_file).with_context(|| format!("certificate {}", cert_file.display()))?;
    let key_pem = std::fs::read(key_file).with_context(|| format!("key {}", key_file.display()))?;
    let chain = CertificateDer::pem_slice_iter(&cert_pem)
        .collect::<Result<Vec<_>, _>>()
        .context("certificate (PEM)")?;
    let leaf = chain
        .first()
        .ok_or_else(|| anyhow::anyhow!("no certificate in {}", cert_file.display()))?;
    let info = parse(leaf).ok_or_else(|| anyhow::anyhow!("certificate not understood (X.509)"))?;
    // The key's error is rustls's: it names the format, never the bytes.
    let key = PrivateKeyDer::from_pem_slice(&key_pem).context("key (PEM)")?;
    let mut config =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .context("TLS versions")?
            .with_no_client_auth()
            .with_single_cert(chain, key)
            .context("certificate and key")?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok((config, info))
}

// ---- X.509, the little Moli needs ----------------------------------------

/// One DER element: its tag, its content, what follows.
fn tlv(input: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, rest) = input.split_first()?;
    let (&first, mut rest) = rest.split_first()?;
    let len = if first < 0x80 {
        usize::from(first)
    } else {
        let n = usize::from(first & 0x7f);
        if n == 0 || n > 4 || rest.len() < n {
            return None;
        }
        let (bytes, after) = rest.split_at(n);
        rest = after;
        bytes
            .iter()
            .fold(0usize, |acc, b| (acc << 8) | usize::from(*b))
    };
    (rest.len() >= len).then(|| {
        let (content, after) = rest.split_at(len);
        (tag, content, after)
    })
}

/// The next element if it has `tag`: its content and what follows; else
/// the input untouched.
fn optional(input: &[u8], tag: u8) -> (Option<&[u8]>, &[u8]) {
    match tlv(input) {
        Some((t, content, rest)) if t == tag => (Some(content), rest),
        _ => (None, input),
    }
}

fn expect(input: &[u8], tag: u8) -> Option<(&[u8], &[u8])> {
    let (t, content, rest) = tlv(input)?;
    (t == tag).then_some((content, rest))
}

const SEQUENCE: u8 = 0x30;
const INTEGER: u8 = 0x02;
const OCTET_STRING: u8 = 0x04;
const BOOLEAN: u8 = 0x01;
const OID: u8 = 0x06;
const UTC_TIME: u8 = 0x17;
const GENERALIZED_TIME: u8 = 0x18;

/// Validity and subject alternative names of a certificate (DER).
pub(crate) fn parse(cert: &CertificateDer<'_>) -> Option<CertInfo> {
    let (cert, _) = expect(cert.as_ref(), SEQUENCE)?;
    let (tbs, _) = expect(cert, SEQUENCE)?;
    let (_, rest) = optional(tbs, 0xa0); // version
    let (_, rest) = expect(rest, INTEGER)?; // serial
    let (_, rest) = expect(rest, SEQUENCE)?; // signature algorithm
    let (_, rest) = expect(rest, SEQUENCE)?; // issuer
    let (validity, rest) = expect(rest, SEQUENCE)?;
    let (not_before, after) = time(validity)?;
    let (not_after, _) = time(after)?;
    let (_, rest) = expect(rest, SEQUENCE)?; // subject
    let (_, rest) = expect(rest, SEQUENCE)?; // public key
    let (_, rest) = optional(rest, 0xa1);
    let (_, rest) = optional(rest, 0xa2);
    let (extensions, _) = optional(rest, 0xa3);
    let names = extensions
        .and_then(|e| expect(e, SEQUENCE))
        .map(|(list, _)| san(list))
        .unwrap_or_default();
    Some(CertInfo {
        names,
        not_before,
        not_after,
    })
}

/// The SAN extension's names, out of the extension list.
fn san(mut list: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    while let Some((ext, rest)) = expect(list, SEQUENCE) {
        list = rest;
        let Some((oid, after)) = expect(ext, OID) else {
            continue;
        };
        let (_, after) = optional(after, BOOLEAN);
        if oid != OID_SAN {
            continue;
        }
        let Some((value, _)) = expect(after, OCTET_STRING) else {
            continue;
        };
        let Some((mut general, _)) = expect(value, SEQUENCE) else {
            continue;
        };
        while let Some((tag, content, rest)) = tlv(general) {
            general = rest;
            match (tag, content.len()) {
                // dNSName [2] IA5String
                (0x82, _) => {
                    if let Ok(name) = std::str::from_utf8(content) {
                        names.push(name.trim_end_matches('.').to_ascii_lowercase());
                    }
                }
                // iPAddress [7] OCTET STRING
                (0x87, 4) => {
                    let b: [u8; 4] = content.try_into().unwrap_or_default();
                    names.push(IpAddr::from(b).to_string());
                }
                (0x87, 16) => {
                    let b: [u8; 16] = content.try_into().unwrap_or_default();
                    names.push(IpAddr::from(b).to_string());
                }
                _ => {}
            }
        }
    }
    names
}

/// A `Time` (UTCTime `YYMMDDHHMMSSZ` or GeneralizedTime `YYYYMMDDHHMMSSZ`)
/// in milliseconds since the epoch, and what follows.
fn time(input: &[u8]) -> Option<(u64, &[u8])> {
    let (tag, content, rest) = tlv(input)?;
    let text = std::str::from_utf8(content).ok()?.strip_suffix('Z')?;
    let (year, tail) = match tag {
        UTC_TIME if text.len() == 12 => {
            let yy: i64 = text[..2].parse().ok()?;
            (if yy < 50 { 2000 + yy } else { 1900 + yy }, &text[2..])
        }
        GENERALIZED_TIME if text.len() == 14 => (text[..4].parse().ok()?, &text[4..]),
        _ => return None,
    };
    let field = |i: usize| -> Option<i64> { tail.get(i..i + 2)?.parse().ok() };
    let (month, day, hour, minute, second) =
        (field(0)?, field(2)?, field(4)?, field(6)?, field(8)?);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let days = days_from_civil(year, month, day);
    let seconds = days * 86_400 + hour * 3600 + minute * 60 + second;
    Some((u64::try_from(seconds).ok()? * 1000, rest))
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `YYYY-MM-DD` of a time in milliseconds since the epoch (UTC).
fn date(ms: u64) -> String {
    let z = i64::try_from(ms / 86_400_000).unwrap_or(0) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

// ---- the listener ----------------------------------------------------------

type Handshake = Result<(TlsStream<TcpStream>, SocketAddr), ()>;

/// [`TlsListener`] as `axum::serve` takes it, its peers' addresses known to
/// the requests (axum's own `Connected` covers a tapped listener).
pub type HttpsListener = TapIo<TlsListener, fn(&mut TlsStream<TcpStream>)>;

/// Small answers leave at once, like the HTTP listener's.
fn nodelay(stream: &mut TlsStream<TcpStream>) {
    let _ = stream.get_ref().0.set_nodelay(true);
}

/// A TCP listener whose connections come out encrypted. The handshakes run
/// on their own tasks (at most [`MAX_HANDSHAKES`] at once, each within
/// [`HANDSHAKE_TIMEOUT`]): a client that stalls its handshake holds up
/// nobody else. While the certificate is unusable, connections are dropped.
#[derive(Debug)]
pub struct TlsListener {
    tcp: TcpListener,
    tls: Tls,
    tx: mpsc::Sender<Handshake>,
    ready: mpsc::Receiver<Handshake>,
    handshakes: Arc<tokio::sync::Semaphore>,
}

impl axum::serve::Listener for TlsListener {
    type Io = TlsStream<TcpStream>;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            tokio::select! {
                Some(done) = self.ready.recv() => {
                    if let Ok(ready) = done {
                        return ready;
                    }
                }
                incoming = self.tcp.accept(), if self.handshakes.available_permits() > 0 => {
                    let (tcp, peer) = match incoming {
                        Ok(tup) => tup,
                        Err(e) => {
                            tracing::debug!(error = %e, "HTTPS accept");
                            tokio::time::sleep(Duration::from_millis(50)).await;
                            continue;
                        }
                    };
                    let Some(config) = self.tls.current() else {
                        tracing::debug!(%peer, "HTTPS closed: connection dropped");
                        drop(tcp);
                        continue;
                    };
                    let Ok(permit) = Arc::clone(&self.handshakes).try_acquire_owned() else {
                        continue;
                    };
                    let tx = self.tx.clone();
                    tokio::spawn(async move {
                        let done = tokio::time::timeout(
                            HANDSHAKE_TIMEOUT,
                            TlsAcceptor::from(config).accept(tcp),
                        )
                        .await;
                        let result = match done {
                            Ok(Ok(stream)) => Ok((stream, peer)),
                            Ok(Err(e)) => {
                                tracing::debug!(%peer, error = %e, "TLS handshake failed");
                                Err(())
                            }
                            Err(_) => {
                                tracing::debug!(%peer, "TLS handshake timed out");
                                Err(())
                            }
                        };
                        drop(permit);
                        let _ = tx.send(result).await;
                    });
                }
            }
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.tcp.local_addr()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use moli_runtime::HubOptions;
    use ring::rand::SystemRandom;
    use ring::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair as _};

    // ---- a certificate made on the spot (no key in the repository) -------

    fn der(tag: u8, content: &[u8]) -> Vec<u8> {
        let mut out = vec![tag];
        let len = content.len();
        if len < 0x80 {
            out.push(u8::try_from(len).unwrap());
        } else if len < 0x100 {
            out.extend([0x81, u8::try_from(len).unwrap()]);
        } else {
            out.extend([
                0x82,
                u8::try_from(len >> 8).unwrap(),
                u8::try_from(len & 0xff).unwrap(),
            ]);
        }
        out.extend_from_slice(content);
        out
    }

    fn seq(parts: &[Vec<u8>]) -> Vec<u8> {
        der(SEQUENCE, &parts.concat())
    }

    fn utc(ms: u64) -> Vec<u8> {
        let d = date(ms);
        let secs = (ms / 1000) % 86_400;
        let text = format!(
            "{}{}{}{:02}{:02}{:02}Z",
            &d[2..4],
            &d[5..7],
            &d[8..10],
            secs / 3600,
            (secs / 60) % 60,
            secs % 60
        );
        der(UTC_TIME, text.as_bytes())
    }

    fn base64(bytes: &[u8]) -> String {
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let n = chunk
                .iter()
                .chain([0u8, 0].iter())
                .take(3)
                .fold(0u32, |acc, b| (acc << 8) | u32::from(*b));
            for i in 0..4 {
                if i <= chunk.len() {
                    out.push(char::from(T[((n >> (18 - 6 * i)) & 63) as usize]));
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    fn pem(label: &str, der: &[u8]) -> String {
        format!(
            "-----BEGIN {label}-----\n{}\n-----END {label}-----\n",
            base64(der)
        )
    }

    /// A self-signed ECDSA P-256 certificate for `names`, valid between the
    /// two instants (ms), and its key: both PEM.
    pub(crate) fn forge(names: &[&str], not_before: u64, not_after: u64) -> (String, String) {
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
        let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng)
            .unwrap();
        let ecdsa_sha256 = der(OID, &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02]);
        let sig_alg = seq(&[ecdsa_sha256]);
        let name = seq(&[der(
            0x31,
            &seq(&[der(OID, &[0x55, 0x04, 0x03]), der(0x0c, b"moli test")]),
        )]);
        let spki = seq(&[
            seq(&[
                der(OID, &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01]),
                der(OID, &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07]),
            ]),
            der(0x03, &[&[0u8][..], key.public_key().as_ref()].concat()),
        ]);
        let general: Vec<Vec<u8>> = names
            .iter()
            .map(|n| match n.parse::<IpAddr>() {
                Ok(IpAddr::V4(v4)) => der(0x87, &v4.octets()),
                Ok(IpAddr::V6(v6)) => der(0x87, &v6.octets()),
                Err(_) => der(0x82, n.as_bytes()),
            })
            .collect();
        let extensions = der(
            0xa3,
            &seq(&[seq(&[der(OID, OID_SAN), der(OCTET_STRING, &seq(&general))])]),
        );
        let tbs = seq(&[
            der(0xa0, &der(INTEGER, &[2])),
            der(INTEGER, &[0x42]),
            sig_alg.clone(),
            name.clone(),
            seq(&[utc(not_before), utc(not_after)]),
            name,
            spki,
            extensions,
        ]);
        let signature = key.sign(&rng, &tbs).unwrap();
        let cert = seq(&[
            tbs,
            sig_alg,
            der(0x03, &[&[0u8][..], signature.as_ref()].concat()),
        ]);
        (
            pem("CERTIFICATE", &cert),
            pem("PRIVATE KEY", pkcs8.as_ref()),
        )
    }

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("moli-tls-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(dir: &Path, names: &[&str], not_before: u64, not_after: u64) -> TlsOptions {
        let (cert, key) = forge(names, not_before, not_after);
        std::fs::write(dir.join("moli.crt"), cert).unwrap();
        std::fs::write(dir.join("moli.key"), key).unwrap();
        TlsOptions {
            cert_file: dir.join("moli.crt"),
            key_file: dir.join("moli.key"),
            listen: "127.0.0.1:0".parse().unwrap(),
        }
    }

    const DAY: u64 = 86_400_000;

    fn hub() -> Hub {
        Hub::new(HubOptions::default()).unwrap()
    }

    #[test]
    fn dates_round_trip() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(days_from_civil(2026, 10, 18), 20_744);
        assert_eq!(date(20_744 * DAY + 3_600_000), "2026-10-18");
        assert_eq!(
            date(u64::try_from(days_from_civil(2000, 2, 29)).unwrap() * DAY),
            "2000-02-29"
        );
        let utc = der(UTC_TIME, b"261018120000Z");
        let (t, rest) = time(&utc).unwrap();
        assert_eq!(t, 20_744 * DAY + 12 * 3_600_000);
        assert!(rest.is_empty());
        let generalized = der(GENERALIZED_TIME, b"20261018120000Z");
        let (t, _) = time(&generalized).unwrap();
        assert_eq!(t, 20_744 * DAY + 12 * 3_600_000);
        assert!(time(&der(UTC_TIME, b"26101812000Z")).is_none());
        assert!(time(&der(UTC_TIME, b"261318120000Z")).is_none(), "month 13");
    }

    #[test]
    fn a_certificate_tells_its_names_and_dates() {
        let now = now_ms() / 1000 * 1000;
        let (cert, _) = forge(
            &["Moli.Example.NET", "192.168.1.9", "fd00::9"],
            now,
            now + 90 * DAY,
        );
        let der = CertificateDer::from_pem_slice(cert.as_bytes()).unwrap();
        let info = parse(&der).unwrap();
        assert_eq!(info.names, ["moli.example.net", "192.168.1.9", "fd00::9"]);
        assert_eq!(info.not_before, now);
        assert_eq!(info.not_after, now + 90 * DAY);
        assert!(parse(&CertificateDer::from(vec![0x30, 0x01, 0xff])).is_none());
        assert!(parse(&CertificateDer::from(Vec::new())).is_none());
    }

    #[test]
    fn a_good_certificate_opens_an_expired_one_closes() {
        let dir = dir("open");
        let now = now_ms();
        let tls = Tls::new(
            write(&dir, &["moli.example.net"], now - DAY, now + 60 * DAY),
            hub(),
        );
        assert!(tls.current().is_some());
        assert_eq!(tls.names(), ["moli.example.net"]);
        let status = tls.status();
        assert_eq!(status["open"], json!(true));
        assert!(status["reason"].is_null());
        assert_eq!(status["expires_in_days"], json!(59));
        assert!(
            !status.to_string().contains("moli.key"),
            "the key is nobody's business"
        );

        let expired = Tls::new(
            write(&dir, &["moli.example.net"], now - 100 * DAY, now - DAY),
            hub(),
        );
        assert!(expired.current().is_none());
        assert_eq!(expired.status()["open"], json!(false));
        assert!(
            expired.status()["reason"]
                .as_str()
                .unwrap()
                .contains(&date(now - DAY))
        );
        assert_eq!(
            expired.names(),
            ["moli.example.net"],
            "its names still count for the guard"
        );

        let early = Tls::new(
            write(&dir, &["moli.example.net"], now + DAY, now + 60 * DAY),
            hub(),
        );
        assert!(early.current().is_none());
        assert!(
            early.status()["reason"]
                .as_str()
                .unwrap()
                .contains(&date(now + DAY))
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_or_broken_files_close_the_listener_with_a_reason() {
        let dir = dir("broken");
        let options = TlsOptions {
            cert_file: dir.join("absent.crt"),
            key_file: dir.join("absent.key"),
            listen: "127.0.0.1:0".parse().unwrap(),
        };
        let tls = Tls::new(options.clone(), hub());
        assert!(tls.current().is_none());
        let reason = tls.status()["reason"].as_str().unwrap().to_owned();
        assert!(reason.contains("absent.crt"), "{reason}");
        // The certificate of one key, the key of another: refused.
        let now = now_ms();
        let (cert, _) = forge(&["a.example"], now - DAY, now + DAY);
        let (_, other_key) = forge(&["a.example"], now - DAY, now + DAY);
        std::fs::write(&options.cert_file, cert).unwrap();
        std::fs::write(&options.key_file, other_key).unwrap();
        tls.reload();
        assert!(tls.current().is_none());
        let reason = tls.status()["reason"].as_str().unwrap().to_owned();
        assert!(
            !reason.contains("BEGIN"),
            "no key material in a reason: {reason}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_renewed_certificate_is_picked_up_without_a_restart() {
        let dir = dir("renew");
        let now = now_ms();
        let options = write(&dir, &["old.example"], now - DAY, now + DAY);
        let tls = Tls::new(options.clone(), hub());
        assert_eq!(tls.names(), ["old.example"]);
        // Same files, nothing changed: no reload.
        tls.tick();
        assert_eq!(tls.names(), ["old.example"]);
        // Renewed (a later modification time, as a renewal writes).
        let (cert, key) = forge(&["new.example"], now - DAY, now + 90 * DAY);
        std::fs::write(&options.cert_file, cert).unwrap();
        std::fs::write(&options.key_file, key).unwrap();
        let later = SystemTime::now() + Duration::from_secs(5);
        for f in [&options.cert_file, &options.key_file] {
            std::fs::File::options()
                .write(true)
                .open(f)
                .unwrap()
                .set_modified(later)
                .unwrap();
        }
        tls.tick();
        assert_eq!(tls.names(), ["new.example"]);
        assert!(tls.current().is_some());
        // Gone: closed, with the reason; back: open again.
        std::fs::remove_file(&options.key_file).unwrap();
        tls.tick();
        assert!(tls.current().is_none());
        assert!(
            tls.status()["reason"]
                .as_str()
                .unwrap()
                .contains("moli.key")
        );
        let (_, key) = forge(&["x"], 0, 1);
        std::fs::write(&options.key_file, key).unwrap(); // the wrong key, still closed
        tls.tick();
        assert!(tls.current().is_none());
        let (cert, key) = forge(&["new.example"], now - DAY, now + 90 * DAY);
        std::fs::write(&options.cert_file, cert).unwrap();
        std::fs::write(&options.key_file, key).unwrap();
        tls.tick();
        assert!(tls.current().is_some());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_house_hears_of_an_expiry_coming_once() {
        let dir = dir("warn");
        let now = now_ms();
        let hub = hub();
        let mut events = hub.subscribe();
        let tls = Tls::new(
            write(&dir, &["soon.example"], now - DAY, now + 10 * DAY),
            hub.clone(),
        );
        tls.tick();
        tls.tick();
        let mut notices = 0;
        while let Ok(event) = events.try_recv() {
            if let moli_core::Event::Notice { notice } = event {
                assert!(
                    notice.message.contains("soon.example"),
                    "{}",
                    notice.message
                );
                notices += 1;
            }
        }
        assert_eq!(notices, 1, "warned once, not at every look");
        let _ = std::fs::remove_dir_all(dir);
    }

    // ---- the listener, end to end --------------------------------------------

    /// A client that trusts whatever certificate it is shown (tests only).
    #[derive(Debug)]
    struct Trusting(Arc<rustls::crypto::CryptoProvider>);

    impl rustls::client::danger::ServerCertVerifier for Trusting {
        fn verify_server_cert(
            &self,
            _: &CertificateDer<'_>,
            _: &[CertificateDer<'_>],
            _: &rustls::pki_types::ServerName<'_>,
            _: &[u8],
            _: rustls::pki_types::UnixTime,
        ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _: &[u8],
            _: &CertificateDer<'_>,
            _: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _: &[u8],
            _: &CertificateDer<'_>,
            _: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            self.0.signature_verification_algorithms.supported_schemes()
        }
    }

    async fn https_get(addr: SocketAddr, host: &str) -> anyhow::Result<String> {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ClientConfig::builder_with_provider(Arc::clone(&provider))
            .with_safe_default_protocol_versions()?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(Trusting(provider)))
            .with_no_client_auth();
        let tcp = TcpStream::connect(addr).await?;
        let name = rustls::pki_types::ServerName::try_from(host.to_owned())?;
        let mut stream = tokio_rustls::TlsConnector::from(Arc::new(config))
            .connect(name, tcp)
            .await?;
        stream
            .write_all(
                format!("GET /who HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await?;
        let mut out = String::new();
        stream.read_to_string(&mut out).await?;
        Ok(out)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_listener_serves_https_and_closes_without_a_certificate() {
        use axum::extract::ConnectInfo;
        use axum::routing::get;
        let dir = dir("serve");
        let now = now_ms();
        let options = write(&dir, &["moli.example.net"], now - DAY, now + DAY);
        let tls = Tls::new(options.clone(), hub());
        let listener = tls.bind().await.unwrap();
        let addr = axum::serve::Listener::local_addr(&listener).unwrap();
        let app = axum::Router::new().route(
            "/who",
            get(
                |ConnectInfo(peer): ConnectInfo<SocketAddr>,
                 encrypted: Option<axum::Extension<Encrypted>>| async move {
                    format!("{} {}", peer.ip(), encrypted.is_some())
                },
            ),
        );
        let cancel = CancellationToken::new();
        let stop = cancel.clone();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                app.layer(axum::Extension(Encrypted))
                    .into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(async move { stop.cancelled().await })
            .await
            .unwrap();
        });
        let answer = https_get(addr, "moli.example.net").await.unwrap();
        assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
        assert!(
            answer.ends_with("127.0.0.1 true"),
            "the real peer, over TLS: {answer}"
        );

        // The certificate goes: the next connection gets nothing.
        std::fs::remove_file(&options.cert_file).unwrap();
        tls.tick();
        assert!(tls.current().is_none());
        assert!(https_get(addr, "moli.example.net").await.is_err());
        // Back: open again, no restart.
        let (cert, key) = forge(&["moli.example.net"], now - DAY, now + DAY);
        std::fs::write(&options.cert_file, cert).unwrap();
        std::fs::write(&options.key_file, key).unwrap();
        tls.reload();
        let answer = https_get(addr, "moli.example.net").await.unwrap();
        assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
        cancel.cancel();
        let _ = tokio::time::timeout(Duration::from_secs(5), server).await;
        let _ = std::fs::remove_dir_all(dir);
    }
}
