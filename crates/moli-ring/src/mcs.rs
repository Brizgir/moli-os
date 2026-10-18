//! The push server's connection (MCS, `mtalk.google.com:5228`, TLS): a
//! login with the GCM device ([`crate::fcm`]), then messages as they come,
//! each a tag byte, a varint length and a protobuf (Chromium's
//! `mcs.proto`). Moli answers the server's heartbeats and sends its own
//! when the line is quiet: a link that answers nothing is given up, and
//! the push task connects again.

use std::time::Duration;

use anyhow::{Context as _, bail};
use moli_net::proto::{self, Writer, read_varint, varint};
use moli_runtime::BoxFuture;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

use crate::fcm::CHROME_VERSION;

pub(crate) const HOST: &str = "mtalk.google.com";
pub(crate) const PORT: u16 = 5228;
const VERSION: u8 = 41;
const HEARTBEAT_PING: u8 = 0;
const HEARTBEAT_ACK: u8 = 1;
const LOGIN_REQUEST: u8 = 2;
const LOGIN_RESPONSE: u8 = 3;
const CLOSE: u8 = 4;
const DATA_MESSAGE: u8 = 8;
const STREAM_ERROR: u8 = 10;
/// Larger than any notification; more is not a push server.
const MAX_MESSAGE: usize = 256 * 1024;
const LOGIN_WAIT: Duration = Duration::from_secs(20);

/// A byte stream to the push server (TLS; a pipe in the tests).
pub(crate) trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}

/// How the push server is reached.
pub(crate) trait Dial: Send + Sync {
    fn dial(&self) -> BoxFuture<'_, anyhow::Result<Box<dyn Stream>>>;
}

/// The real one.
pub(crate) struct Google;

impl Dial for Google {
    fn dial(&self) -> BoxFuture<'_, anyhow::Result<Box<dyn Stream>>> {
        Box::pin(async {
            let stream = moli_net::public_tls_stream(HOST, PORT).await?;
            Ok(Box::new(stream) as Box<dyn Stream>)
        })
    }
}

/// The server said no to our device: registered again.
#[derive(Debug)]
pub(crate) struct Refused;

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the push server refused our device")
    }
}

impl std::error::Error for Refused {}

/// A data message: what the sender gave, still encrypted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Message {
    /// Told back at the next login, so it is not sent again.
    pub persistent_id: Option<String>,
    pub app_data: Vec<(String, String)>,
    pub raw: Vec<u8>,
}

pub(crate) struct Connection {
    stream: Box<dyn Stream>,
    buf: Vec<u8>,
    /// Messages received since the login (the server counts them too).
    received: u64,
    reported: u64,
    heartbeat: Duration,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("received", &self.received)
            .finish_non_exhaustive()
    }
}

/// One frame: tag, varint length, message.
pub(crate) fn frame(tag: u8, message: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    varint(message.len() as u64, &mut out);
    out.extend_from_slice(message);
    out
}

/// The login request: the GCM device, the messages already received.
pub(crate) fn login_request(
    android_id: u64,
    security_token: u64,
    received: &[String],
    heartbeat: Duration,
) -> Vec<u8> {
    let id = android_id.to_string();
    let mut login = Writer::new()
        .str(1, &format!("chrome-{CHROME_VERSION}"))
        .str(2, "mcs.android.com")
        .str(3, &id)
        .str(4, &id)
        .str(5, &security_token.to_string())
        .str(6, &format!("android-{android_id:x}"))
        .msg(8, Writer::new().str(1, "new_vc").str(2, "1"));
    for persistent in received {
        login = login.str(10, persistent);
    }
    let interval = u64::try_from(heartbeat.as_millis()).unwrap_or(u64::MAX);
    login
        .bool(12, false)
        .msg(13, Writer::new().str(1, "").bool(2, true).uint(3, interval))
        .bool(14, true) // use_rmq2
        .uint(16, 2) // auth by Android id
        .uint(17, 1) // network type
        .finish()
}

/// Logs in on `stream`: the connection, ready for messages. [`Refused`]
/// when the server rejects the device.
pub(crate) async fn login(
    mut stream: Box<dyn Stream>,
    android_id: u64,
    security_token: u64,
    received: &[String],
    heartbeat: Duration,
) -> anyhow::Result<Connection> {
    let request = login_request(android_id, security_token, received, heartbeat);
    let mut hello = vec![VERSION];
    hello.extend(frame(LOGIN_REQUEST, &request));
    stream.write_all(&hello).await?;
    stream.flush().await?;
    let mut conn = Connection {
        stream,
        buf: Vec::new(),
        // The login's answer is the first message of the stream.
        received: 1,
        reported: 0,
        heartbeat,
    };
    tokio::time::timeout(LOGIN_WAIT, conn.handshake())
        .await
        .context("the push server did not answer the login")??;
    Ok(conn)
}

impl Connection {
    /// More bytes from the stream (cancel-safe: what is read stays in
    /// `buf`).
    async fn fill(&mut self) -> anyhow::Result<()> {
        let mut chunk = [0u8; 4096];
        let n = self.stream.read(&mut chunk).await?;
        if n == 0 {
            bail!("the push server closed the connection");
        }
        self.buf.extend_from_slice(&chunk[..n]);
        Ok(())
    }

    /// One whole frame out of `buf`, if there is one.
    fn take(&mut self) -> anyhow::Result<Option<(u8, Vec<u8>)>> {
        let Some(&tag) = self.buf.first() else {
            return Ok(None);
        };
        let mut pos = 1;
        let Some(len) = read_varint(&self.buf, &mut pos) else {
            if self.buf.len() > 6 {
                bail!("unreadable length from the push server");
            }
            return Ok(None);
        };
        let len = usize::try_from(len)?;
        if len > MAX_MESSAGE {
            bail!("a push message of {len} bytes");
        }
        if self.buf.len() < pos + len {
            return Ok(None);
        }
        let message = self.buf[pos..pos + len].to_vec();
        self.buf.drain(..pos + len);
        Ok(Some((tag, message)))
    }

    async fn next_frame(&mut self) -> anyhow::Result<(u8, Vec<u8>)> {
        loop {
            if let Some(frame) = self.take()? {
                return Ok(frame);
            }
            self.fill().await?;
        }
    }

    /// The version byte, then frames until the login's answer.
    async fn handshake(&mut self) -> anyhow::Result<()> {
        while self.buf.is_empty() {
            self.fill().await?;
        }
        let version = self.buf.remove(0);
        if version < VERSION && version != 38 {
            bail!("push server version {version}");
        }
        loop {
            let (tag, message) = self.next_frame().await?;
            match tag {
                LOGIN_RESPONSE => {
                    let answer = proto::parse(&message).context("unreadable login answer")?;
                    if answer.has(3) {
                        return Err(Refused.into());
                    }
                    return Ok(());
                }
                CLOSE => return Err(Refused.into()),
                _ => {}
            }
        }
    }

    async fn send(&mut self, tag: u8) -> anyhow::Result<()> {
        let mut message = Writer::new();
        if self.received != self.reported {
            self.reported = self.received;
            message = message.uint(2, self.received);
        }
        let bytes = frame(tag, &message.finish());
        self.stream.write_all(&bytes).await?;
        self.stream.flush().await?;
        Ok(())
    }

    /// The next data message. Heartbeats are answered on the way; after a
    /// quiet heartbeat period Moli pings, and one more without a word from
    /// the server ends the connection.
    pub(crate) async fn next(&mut self) -> anyhow::Result<Message> {
        let mut pinged = false;
        loop {
            let frame = match tokio::time::timeout(self.heartbeat, self.next_frame()).await {
                Ok(frame) => frame?,
                Err(_) if pinged => bail!("the push server stopped answering"),
                Err(_) => {
                    self.send(HEARTBEAT_PING).await?;
                    pinged = true;
                    continue;
                }
            };
            pinged = false;
            self.received += 1;
            match frame {
                (HEARTBEAT_PING, _) => self.send(HEARTBEAT_ACK).await?,
                (CLOSE, _) => bail!("the push server closed the connection"),
                (STREAM_ERROR, _) => bail!("the push server reported a stream error"),
                (DATA_MESSAGE, bytes) => {
                    let m = proto::parse(&bytes).context("unreadable push message")?;
                    return Ok(Message {
                        persistent_id: m.str(9).map(str::to_owned),
                        app_data: m
                            .all_msgs(7)
                            .filter_map(|d| Some((d.str(1)?.to_owned(), d.str(2)?.to_owned())))
                            .collect(),
                        raw: m.bytes(21).unwrap_or_default().to_vec(),
                    });
                }
                // Acks, the server's own stanzas: nothing to do.
                _ => {}
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use tokio::io::DuplexStream;

    use super::*;

    /// The server's side of a pipe: the login read, then answered.
    pub(crate) async fn accept(server: &mut DuplexStream) -> proto::Fields<'static> {
        let mut head = [0u8; 2];
        server.read_exact(&mut head).await.unwrap();
        assert_eq!(head, [VERSION, LOGIN_REQUEST]);
        let mut len = Vec::new();
        loop {
            let b = server.read_u8().await.unwrap();
            len.push(b);
            if b & 0x80 == 0 {
                break;
            }
        }
        let mut pos = 0;
        let len = usize::try_from(read_varint(&len, &mut pos).unwrap()).unwrap();
        let mut login = vec![0u8; len];
        server.read_exact(&mut login).await.unwrap();
        let mut answer = vec![VERSION];
        answer.extend(frame(LOGIN_RESPONSE, &Writer::new().str(1, "ok").finish()));
        server.write_all(&answer).await.unwrap();
        proto::parse(Box::leak(login.into_boxed_slice())).unwrap()
    }

    /// A data message frame, as the server sends one.
    pub(crate) fn data(persistent: &str, app_data: &[(&str, &str)], raw: &[u8]) -> Vec<u8> {
        let mut m = Writer::new()
            .str(3, "876313859327")
            .str(5, "org.chromium.linux")
            .str(9, persistent);
        for (k, v) in app_data {
            m = m.msg(7, Writer::new().str(1, k).str(2, v));
        }
        frame(DATA_MESSAGE, &m.bytes(21, raw).finish())
    }

    #[tokio::test(start_paused = true)]
    async fn logs_in_answers_heartbeats_and_hands_over_messages() {
        let (client, mut server) = tokio::io::duplex(64 * 1024);
        let serve = tokio::spawn(async move {
            let login = accept(&mut server).await;
            assert_eq!(login.str(3), Some("41"));
            assert_eq!(login.str(5), Some("42"));
            assert_eq!(login.str(6), Some("android-29"));
            let acked: Vec<_> = login
                .all_bytes(10)
                .map(|b| String::from_utf8(b.to_vec()).unwrap())
                .collect();
            assert_eq!(acked, ["0:old"]);
            // A ping, cut in two writes, then a message.
            let ping = frame(HEARTBEAT_PING, &[]);
            server.write_all(&ping[..1]).await.unwrap();
            server.write_all(&ping[1..]).await.unwrap();
            let mut ack = [0u8; 2];
            server.read_exact(&mut ack).await.unwrap();
            assert_eq!(ack[0], HEARTBEAT_ACK);
            let ack_len = usize::from(ack[1]);
            let mut ack = vec![0u8; ack_len];
            server.read_exact(&mut ack).await.unwrap();
            assert_eq!(proto::parse(&ack).unwrap().uint(2), Some(2), "login, ping");
            server
                .write_all(&data("0:new", &[("encryption", "salt=x")], b"secret"))
                .await
                .unwrap();
            // Then silence: Moli pings once, then gives up.
            let mut ping = [0u8; 2];
            server.read_exact(&mut ping).await.unwrap();
            assert_eq!(ping[0], HEARTBEAT_PING);
            server
        });
        let mut conn = login(
            Box::new(client),
            41,
            42,
            &["0:old".to_owned()],
            Duration::from_secs(300),
        )
        .await
        .unwrap();
        let m = conn.next().await.unwrap();
        assert_eq!(m.persistent_id.as_deref(), Some("0:new"));
        assert_eq!(m.app_data, [("encryption".to_owned(), "salt=x".to_owned())]);
        assert_eq!(m.raw, b"secret");
        let _server = {
            let quiet = conn.next().await.unwrap_err();
            assert!(quiet.to_string().contains("stopped answering"), "{quiet}");
            serve.await.unwrap()
        };
    }

    #[tokio::test]
    async fn a_refused_device_is_its_own_failure() {
        let (client, mut server) = tokio::io::duplex(4096);
        tokio::spawn(async move {
            let mut buf = vec![0u8; 512];
            let _ = server.read(&mut buf).await;
            let error = Writer::new().uint(1, 401).str(2, "bad device");
            let answer = Writer::new().str(1, "x").msg(3, error).finish();
            let mut out = vec![VERSION];
            out.extend(frame(LOGIN_RESPONSE, &answer));
            server.write_all(&out).await.unwrap();
            let _ = server.read(&mut buf).await;
        });
        let e = login(Box::new(client), 41, 42, &[], Duration::from_secs(300))
            .await
            .unwrap_err();
        assert!(e.is::<Refused>(), "{e}");
    }

    #[tokio::test]
    async fn an_oversized_or_garbled_frame_ends_the_connection() {
        let (client, mut server) = tokio::io::duplex(4096);
        tokio::spawn(async move {
            let _ = accept(&mut server).await;
            let mut huge = vec![DATA_MESSAGE];
            varint(10 * 1024 * 1024, &mut huge);
            server.write_all(&huge).await.unwrap();
            let mut buf = [0u8; 64];
            let _ = server.read(&mut buf).await;
        });
        let mut conn = login(Box::new(client), 41, 42, &[], Duration::from_secs(300))
            .await
            .unwrap();
        assert!(conn.next().await.is_err());
    }
}
