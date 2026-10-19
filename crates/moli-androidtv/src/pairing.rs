//! Pairing a remote with an Android / Google TV (« polo », TLS on 6467),
//! the way the TV's own phone app does: the remote presents a client
//! certificate Moli made; the TV shows a 6-character hexadecimal code; the
//! secret sent back is a SHA-256 of both RSA public keys and the code. Once
//! the TV acknowledges it, that certificate is trusted on 6466 ([`remote`]).
//!
//! Messages are an `OuterMessage { protocol_version = 1, status = 2, … }`
//! framed by a varint length, like the remote's. The sequence: pairing
//! request → its ack; options (input, hexadecimal, 6 symbols) → the TV's
//! options; configuration → its ack (the code is on screen); secret → its
//! ack (paired).
//!
//! [`remote`]: crate::remote

use std::time::Duration;

use anyhow::{Context as _, bail};
use moli_net::PinnedTls;
use moli_net::TlsStream;
use ring::digest::{SHA256, digest};
use tokio::net::TcpStream;

use crate::proto::{Fields, Writer, parse};
use crate::remote::{read_frame, send};

pub const PORT: u16 = 6467;
/// What the phone apps send (androidtvremote2 too).
const PROTOCOL: u64 = 2;
const STATUS_OK: u64 = 200;
const STATUS_BAD_SECRET: u64 = 402;
/// `Options.Encoding.ENCODING_TYPE_HEXADECIMAL`, 6 symbols.
const HEXADECIMAL: u64 = 3;
const SYMBOLS: u64 = 6;
/// `Options.RoleType.ROLE_TYPE_INPUT`: the code is typed on our side.
const ROLE_INPUT: u64 = 1;
/// Each step the TV answers by itself (never a person's wait).
const STEP: Duration = Duration::from_secs(10);
/// The service the TV pairs with: the remote's.
const SERVICE: &str = "atvremote";

/// A client certificate and its key, PEM, as kept in the secret store.
#[derive(Clone)]
pub struct ClientCert {
    pub cert_pem: String,
    pub key_pem: String,
}

impl std::fmt::Debug for ClientCert {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientCert").finish_non_exhaustive()
    }
}

/// A new self-signed certificate (RSA 2048, SHA-256, ten years from
/// 2025), named `name`: what the TV lists among its paired remotes.
pub fn new_client_cert(name: &str) -> anyhow::Result<ClientCert> {
    use rsa::pkcs8::{EncodePrivateKey as _, LineEnding};
    let key = rsa::RsaPrivateKey::new(&mut rand_core::OsRng, 2048).context("RSA key")?;
    let key_pem = key
        .to_pkcs8_pem(LineEnding::LF)
        .context("RSA key (PKCS#8)")?
        .to_string();
    let pair = rcgen::KeyPair::from_pem_and_sign_algo(&key_pem, &rcgen::PKCS_RSA_SHA256)
        .context("RSA key for the certificate")?;
    let mut params = rcgen::CertificateParams::new(vec![name.to_owned()])?;
    params.distinguished_name = rcgen::DistinguishedName::new();
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, name);
    params.not_before = rcgen::date_time_ymd(2025, 1, 1);
    params.not_after = rcgen::date_time_ymd(2035, 1, 1);
    params.serial_number = Some(rcgen::SerialNumber::from(1000u64));
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Constrained(0));
    let cert = params.self_signed(&pair).context("certificate")?;
    Ok(ClientCert {
        cert_pem: cert.pem(),
        key_pem,
    })
}

/// The DER of the first certificate of a PEM text.
fn pem_der(pem: &str) -> Option<Vec<u8>> {
    let body: String = pem
        .lines()
        .skip_while(|l| !l.starts_with("-----BEGIN CERTIFICATE"))
        .skip(1)
        .take_while(|l| !l.starts_with("-----END"))
        .collect();
    base64(&body)
}

fn base64(text: &str) -> Option<Vec<u8>> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text
        .bytes()
        .filter(|c| !c.is_ascii_whitespace() && *c != b'=')
    {
        let v = ALPHABET.iter().position(|a| *a == c)?;
        acc = (acc << 6) | u32::try_from(v).ok()?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((acc >> bits) & 0xFF).ok()?);
        }
    }
    Some(out)
}

/// One DER element: (tag, content, rest). Lengths up to 4 bytes.
fn element(input: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, rest) = input.split_first()?;
    let (&first, rest) = rest.split_first()?;
    let (len, rest) = if first < 0x80 {
        (usize::from(first), rest)
    } else {
        let n = usize::from(first & 0x7F);
        if n == 0 || n > 4 || rest.len() < n {
            return None;
        }
        let len = rest[..n]
            .iter()
            .fold(0usize, |acc, &b| (acc << 8) | usize::from(b));
        (len, &rest[n..])
    };
    (rest.len() >= len).then(|| (tag, &rest[..len], &rest[len..]))
}

/// An unsigned DER integer, big-endian without the leading zeros.
fn unsigned(int: &[u8]) -> &[u8] {
    let zeros = int.iter().take_while(|b| **b == 0).count();
    &int[zeros.min(int.len().saturating_sub(1))..]
}

/// The RSA public key of a certificate (DER): its modulus and exponent,
/// big-endian, without leading zeros. `None`: not an RSA certificate.
#[must_use]
pub fn rsa_public_key(cert: &[u8]) -> Option<(&[u8], &[u8])> {
    const RSA: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x01, 0x01];
    let (_, cert, _) = element(cert)?;
    let (_, tbs, _) = element(cert)?;
    let mut rest = tbs;
    if rest.first() == Some(&0xA0) {
        rest = element(rest)?.2; // explicit version
    }
    // serial, signature algorithm, issuer, validity, subject
    for _ in 0..5 {
        rest = element(rest)?.2;
    }
    let (_, spki, _) = element(rest)?;
    let (_, algorithm, after) = element(spki)?;
    let (_, oid, _) = element(algorithm)?;
    if oid != RSA {
        return None;
    }
    let (tag, bits, _) = element(after)?;
    if tag != 0x03 || bits.first() != Some(&0) {
        return None;
    }
    let (_, key, _) = element(&bits[1..])?;
    let (tag, modulus, rest) = element(key)?;
    let (tag2, exponent, _) = element(rest)?;
    (tag == 0x02 && tag2 == 0x02).then(|| (unsigned(modulus), unsigned(exponent)))
}

/// Why a code was not taken.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// Not 6 hexadecimal characters.
    Malformed,
    /// Not the code on screen (checked here, before anything is sent: the
    /// person may type it again).
    Wrong,
    /// The TV said no (402) or closed the pairing: start again.
    Tv(String),
}

/// The secret for `code` (6 hex characters, as the TV shows it): SHA-256
/// of the client's modulus and exponent, the TV's, and the code's last two
/// bytes; its first byte must be the code's first.
pub fn secret(code: &str, client: &[u8], server: &[u8]) -> Result<[u8; 32], Refused> {
    let code = code.trim();
    if code.len() != 6 || !code.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(Refused::Malformed);
    }
    let byte = |i: usize| u8::from_str_radix(&code[i..i + 2], 16).map_err(|_| Refused::Malformed);
    let (check, tail) = (byte(0)?, [byte(2)?, byte(4)?]);
    let (Some((cn, ce)), Some((sn, se))) = (rsa_public_key(client), rsa_public_key(server)) else {
        return Err(Refused::Tv("not an RSA certificate".into()));
    };
    let input: Vec<u8> = [cn, ce, sn, se, &tail[..]].concat();
    let hash = digest(&SHA256, &input);
    let mut out = [0u8; 32];
    out.copy_from_slice(hash.as_ref());
    if out[0] != check {
        return Err(Refused::Wrong);
    }
    Ok(out)
}

fn outer() -> Writer {
    Writer::new().uint(1, PROTOCOL).uint(2, STATUS_OK)
}

fn encoding() -> Writer {
    Writer::new().uint(1, HEXADECIMAL).uint(2, SYMBOLS)
}

/// The first message: who asks (`client_name`: shown on the TV).
fn pairing_request(client_name: &str) -> Vec<u8> {
    outer()
        .msg(10, Writer::new().str(1, SERVICE).str(2, client_name))
        .finish()
}

/// What answers each of the TV's steps; `Ok(None)`: the code is on screen.
fn next(m: &Fields<'_>) -> anyhow::Result<Option<Vec<u8>>> {
    let status = m.uint(2).unwrap_or(0);
    if status != STATUS_OK {
        bail!("the TV answered status {status}");
    }
    if m.has(11) {
        // Options: we type the code (input), hexadecimal, 6 symbols.
        let options = Writer::new().msg(1, encoding()).uint(3, ROLE_INPUT);
        return Ok(Some(outer().msg(20, options).finish()));
    }
    if m.has(20) {
        let configuration = Writer::new().msg(1, encoding()).uint(2, ROLE_INPUT);
        return Ok(Some(outer().msg(30, configuration).finish()));
    }
    if m.has(31) {
        return Ok(None);
    }
    bail!("unexpected pairing message from the TV")
}

/// A pairing under way: the code is on the TV's screen.
pub struct Pairing {
    stream: TlsStream<TcpStream>,
    cert: ClientCert,
    client: Vec<u8>,
    server: Vec<u8>,
    /// What the TV calls itself (from its ack), if it said.
    pub tv_name: Option<String>,
}

impl std::fmt::Debug for Pairing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pairing")
            .field("tv_name", &self.tv_name)
            .finish_non_exhaustive()
    }
}

async fn read(stream: &mut TlsStream<TcpStream>) -> anyhow::Result<Vec<u8>> {
    tokio::time::timeout(STEP, read_frame(stream))
        .await
        .context("the TV did not answer the pairing")?
}

impl Pairing {
    /// Connects with `cert` and asks to pair, until the TV shows its code.
    pub async fn start(host: &str, cert: ClientCert, client_name: &str) -> anyhow::Result<Self> {
        Self::start_at(host, PORT, cert, client_name).await
    }

    async fn start_at(
        host: &str,
        port: u16,
        cert: ClientCert,
        client_name: &str,
    ) -> anyhow::Result<Self> {
        let client = pem_der(&cert.cert_pem).context("client certificate (PEM)")?;
        let tls = PinnedTls::with_client_cert(None, &cert.cert_pem, &cert.key_pem)?;
        let mut stream = tls
            .connect(host, port)
            .await
            .context("android tv pairing")?;
        let server = stream
            .get_ref()
            .1
            .peer_certificates()
            .and_then(|c| c.first())
            .map(|c| c.as_ref().to_vec())
            .context("the TV showed no certificate")?;
        send(&mut stream, &pairing_request(client_name)).await?;
        let mut tv_name = None;
        loop {
            let frame = read(&mut stream).await?;
            let m = parse(&frame).context("unreadable pairing message")?;
            if let Some(name) = m.msg(11).and_then(|ack| ack.str(1)) {
                tv_name = Some(name.to_owned()).filter(|n| !n.is_empty());
            }
            match next(&m)? {
                Some(reply) => send(&mut stream, &reply).await?,
                None => break,
            }
        }
        Ok(Self {
            stream,
            cert,
            client,
            server,
            tv_name,
        })
    }

    /// Sends the secret for `code`. `Ok`: paired, the certificate is now
    /// the TV's trusted remote. [`Refused::Malformed`] or
    /// [`Refused::Wrong`]: nothing was sent, another try is possible.
    pub async fn finish(&mut self, code: &str) -> Result<ClientCert, Refused> {
        let secret = secret(code, &self.client, &self.server)?;
        let message = outer().msg(40, Writer::new().bytes(1, &secret)).finish();
        let tv = |e: anyhow::Error| Refused::Tv(format!("{e:#}"));
        send(&mut self.stream, &message).await.map_err(tv)?;
        let frame = read(&mut self.stream).await.map_err(tv)?;
        let m = parse(&frame).ok_or_else(|| Refused::Tv("unreadable answer".into()))?;
        match m.uint(2) {
            Some(STATUS_OK) if m.has(41) => Ok(self.cert.clone()),
            Some(STATUS_BAD_SECRET) => Err(Refused::Tv("the TV refused the code".into())),
            other => Err(Refused::Tv(format!("the TV answered {other:?}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A certificate made once (generation is slow in a debug build).
    fn cert() -> &'static ClientCert {
        static CERT: std::sync::OnceLock<ClientCert> = std::sync::OnceLock::new();
        CERT.get_or_init(|| new_client_cert("moli").unwrap())
    }

    #[test]
    fn a_new_certificate_is_rsa_with_its_key_readable() {
        let c = cert();
        let der = pem_der(&c.cert_pem).unwrap();
        let (n, e) = rsa_public_key(&der).unwrap();
        assert_eq!(n.len(), 256, "2048 bits");
        assert_eq!(e, [0x01, 0x00, 0x01]);
        // rustls takes both (the TLS client authentication).
        PinnedTls::with_client_cert(None, &c.cert_pem, &c.key_pem).unwrap();
    }

    #[test]
    fn the_secret_hashes_both_keys_and_the_code() {
        let der = pem_der(&cert().cert_pem).unwrap();
        let (n, e) = rsa_public_key(&der).unwrap();
        // With the same certificate on both sides, as a TV's could be: the
        // hash of n, e, n, e and the code's two last bytes.
        let tail = [0x12, 0x34];
        let expected = digest(&SHA256, &[n, e, n, e, &tail[..]].concat());
        let first = expected.as_ref()[0];
        let code = format!("{first:02X}1234");
        assert_eq!(secret(&code, &der, &der).unwrap(), expected.as_ref());
        // Lowercase as typed, spaces around: the same.
        assert!(secret(&format!(" {} ", code.to_lowercase()), &der, &der).is_ok());
        // Another first byte: wrong, before anything is sent.
        let wrong = format!("{:02X}1234", first.wrapping_add(1));
        assert_eq!(secret(&wrong, &der, &der), Err(Refused::Wrong));
        assert_eq!(secret("12345", &der, &der), Err(Refused::Malformed));
        assert_eq!(secret("12345G", &der, &der), Err(Refused::Malformed));
    }

    #[test]
    fn the_exponent_is_hashed_as_androidtvremote2_does() {
        // Python's `bytes.fromhex(f"0{e:X}")` for 65537: 01 00 01; and the
        // modulus's `f"{n:X}"`: its bytes without leading zero.
        assert_eq!(unsigned(&[0x00, 0x01, 0x00, 0x01]), [0x01, 0x00, 0x01]);
        assert_eq!(unsigned(&[0x00, 0xC1, 0x02]), [0xC1, 0x02]);
        assert_eq!(unsigned(&[0x00]), [0x00]);
    }

    #[test]
    fn the_steps_answer_the_tv() {
        let request = parse(&pairing_request("Moli")).map(|m| {
            let r = m.msg(10).unwrap();
            (
                m.uint(1),
                m.uint(2),
                r.str(1).map(str::to_owned),
                r.str(2).map(str::to_owned),
            )
        });
        assert_eq!(
            request,
            Some((
                Some(2),
                Some(200),
                Some("atvremote".into()),
                Some("Moli".into())
            ))
        );
        // The TV's ack (its name): our options.
        let ack = outer().msg(11, Writer::new().str(1, "Google TV")).finish();
        let options = next(&parse(&ack).unwrap()).unwrap().unwrap();
        let options = parse(&options).unwrap();
        let o = options.msg(20).unwrap();
        assert_eq!(o.uint(3), Some(ROLE_INPUT));
        assert_eq!(o.msg(1).unwrap().uint(1), Some(HEXADECIMAL));
        assert_eq!(o.msg(1).unwrap().uint(2), Some(6));
        // The TV's options: our configuration.
        let tv_options = outer().msg(20, Writer::new().msg(1, encoding())).finish();
        let configuration = next(&parse(&tv_options).unwrap()).unwrap().unwrap();
        let configuration = parse(&configuration).unwrap();
        let c = configuration.msg(30).unwrap();
        assert_eq!(c.uint(2), Some(ROLE_INPUT));
        assert_eq!(c.msg(1).unwrap().uint(1), Some(HEXADECIMAL));
        // Its configuration ack: the code is on screen.
        let shown = outer().msg(31, Writer::new()).finish();
        assert_eq!(next(&parse(&shown).unwrap()).unwrap(), None);
        // An error status ends it.
        let error = Writer::new().uint(1, 2).uint(2, 400).finish();
        assert!(next(&parse(&error).unwrap()).is_err());
    }

    /// A TV that pairs as the real ones do: its own RSA certificate, the
    /// same steps, and a secret checked against the code it « shows ».
    async fn fake_tv(code_tail: [u8; 2], client: Vec<u8>) -> (u16, tokio::task::JoinHandle<bool>) {
        use rustls::pki_types::pem::PemObject as _;
        use rustls::pki_types::{CertificateDer, PrivateKeyDer};
        let tv = new_client_cert("tv").unwrap();
        let der = pem_der(&tv.cert_pem).unwrap();
        let config = rustls::ServerConfig::builder_with_provider(std::sync::Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(der.clone())],
            PrivateKeyDer::from_pem_slice(tv.key_pem.as_bytes()).unwrap(),
        )
        .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(config));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut s = acceptor.accept(tcp).await.unwrap();
            let ok = |field: u32, body: Writer| outer().msg(field, body).finish();
            // Request → ack (its name); options → options; configuration → ack.
            let m = read_frame(&mut s).await.unwrap();
            assert!(parse(&m).unwrap().has(10));
            send(&mut s, &ok(11, Writer::new().str(1, "Salon TV")))
                .await
                .unwrap();
            let m = read_frame(&mut s).await.unwrap();
            assert!(parse(&m).unwrap().has(20));
            send(&mut s, &ok(20, Writer::new().msg(1, encoding())))
                .await
                .unwrap();
            let m = read_frame(&mut s).await.unwrap();
            assert!(parse(&m).unwrap().has(30));
            send(&mut s, &ok(31, Writer::new())).await.unwrap();
            // The secret: checked as the TV does.
            let m = read_frame(&mut s).await.unwrap();
            let m = parse(&m).unwrap();
            let got = m.msg(40).and_then(|x| x.bytes(1)).unwrap().to_vec();
            let (cn, ce) = rsa_public_key(&client).unwrap();
            let (sn, se) = rsa_public_key(&der).unwrap();
            let want = digest(&SHA256, &[cn, ce, sn, se, &code_tail[..]].concat());
            let right = got == want.as_ref();
            let status = if right { STATUS_OK } else { STATUS_BAD_SECRET };
            let reply = Writer::new()
                .uint(1, PROTOCOL)
                .uint(2, status)
                .msg(41, Writer::new().bytes(1, &got))
                .finish();
            send(&mut s, &reply).await.unwrap();
            right
        });
        (port, task)
    }

    async fn start_on(host: &str, port: u16, cert: ClientCert) -> anyhow::Result<Pairing> {
        Pairing::start_at(host, port, cert, "Moli").await
    }

    #[tokio::test]
    async fn pairs_with_a_tv_over_tls() {
        let client = cert().clone();
        let client_der = pem_der(&client.cert_pem).unwrap();
        let tail = [0xAB, 0x12];
        let (port, tv) = fake_tv(tail, client_der.clone()).await;
        // The code's first byte is the hash's: computed like the TV does.
        let mut pairing = start_on("127.0.0.1", port, client.clone()).await.unwrap();
        assert_eq!(pairing.tv_name.as_deref(), Some("Salon TV"));
        let (cn, ce) = rsa_public_key(&client_der).unwrap();
        let (sn, se) = rsa_public_key(&pairing.server).unwrap();
        let first = digest(&SHA256, &[cn, ce, sn, se, &tail[..]].concat()).as_ref()[0];
        // A wrong code is caught before anything is sent: the same pairing goes on.
        let wrong = format!("{:02X}AB12", first.wrapping_add(1));
        assert_eq!(pairing.finish(&wrong).await.unwrap_err(), Refused::Wrong);
        let kept = pairing.finish(&format!("{first:02x}ab12")).await.unwrap();
        assert_eq!(kept.cert_pem, client.cert_pem);
        assert!(tv.await.unwrap(), "the TV took the secret");
    }

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn the_documented_exchange_is_followed_byte_for_byte() {
        let fixture = include_str!("../../../integrations/androidtv/fixtures/pairing.txt");
        let lines: Vec<(&str, Vec<u8>)> = fixture
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| l.split_once(": "))
            .map(|(who, h)| (who, hex(h)))
            .collect();
        assert_eq!(lines[0], ("moli", pairing_request("Moli")));
        for pair in lines[1..5].chunks(2) {
            let (("tv", tv), ("moli", ours)) = (&pair[0], &pair[1]) else {
                panic!("tv then moli");
            };
            assert_eq!(next(&parse(tv).unwrap()).unwrap().as_ref(), Some(ours));
        }
        // The configuration ack: the code is on screen, nothing to answer.
        assert_eq!(next(&parse(&lines[5].1).unwrap()).unwrap(), None);
        let ack = parse(&lines[6].1).unwrap();
        assert_eq!((ack.uint(2), ack.has(41)), (Some(STATUS_OK), true));
        let refused = parse(&lines[7].1).unwrap();
        assert_eq!(refused.uint(2), Some(STATUS_BAD_SECRET));
    }
}
