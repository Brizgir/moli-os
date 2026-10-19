//! The door the gateway knocks at: one HTTP/1.1 request per connection,
//! read by hand and bounded everywhere (head, body, time), then closed.
//!
//! Ecowitt gateways speak plain HTTP only (no TLS) and post a form with a
//! `Content-Length`. Nothing else is accepted. The query string, if any, is
//! dropped unread and never logged (the Wunderground protocol puts a
//! station's password there).

use std::net::IpAddr;

use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

/// Longest request head (request line and headers).
pub const MAX_HEAD: usize = 8 * 1024;
/// Longest body: a gateway with every sensor posts about 2 KiB.
pub const MAX_BODY: usize = 16 * 1024;

/// Why a request is turned away: an HTTP status, and a short reason for the
/// log (never the request's content).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refusal {
    pub status: u16,
    pub why: &'static str,
}

const fn refuse(status: u16, why: &'static str) -> Refusal {
    Refusal { status, why }
}

/// A request that passed: its body.
#[derive(Debug)]
pub struct Posted {
    pub body: Vec<u8>,
}

/// Reads one request: `POST <path>` (trailing `/` optional, query ignored),
/// a form, a `Content-Length` of at most [`MAX_BODY`].
pub async fn read<S: AsyncRead + Unpin>(stream: &mut S, path: &str) -> Result<Posted, Refusal> {
    let mut buf = Vec::with_capacity(1024);
    let end = loop {
        if let Some(i) = find(&buf, b"\r\n\r\n") {
            break i;
        }
        if buf.len() >= MAX_HEAD {
            return Err(refuse(431, "head too long"));
        }
        let mut chunk = [0u8; 1024];
        let room = (MAX_HEAD + 4 - buf.len()).min(chunk.len());
        let n = stream
            .read(&mut chunk[..room])
            .await
            .map_err(|_| refuse(400, "read failed"))?;
        if n == 0 {
            return Err(refuse(400, "closed early"));
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = std::str::from_utf8(&buf[..end]).map_err(|_| refuse(400, "head not text"))?;
    let mut lines = head.split("\r\n");
    let mut request = lines.next().unwrap_or("").split(' ');
    let (method, target, version) = (request.next(), request.next(), request.next());
    let (Some(method), Some(target), Some(version)) = (method, target, version) else {
        return Err(refuse(400, "bad request line"));
    };
    if !version.starts_with("HTTP/1.") || request.next().is_some() {
        return Err(refuse(400, "bad request line"));
    }
    if method != "POST" {
        // A GET with a query: the Wunderground protocol.
        return Err(refuse(405, "not a POST (protocol must be Ecowitt)"));
    }
    let asked = target.split('?').next().unwrap_or("");
    if asked.trim_end_matches('/') != path.trim_end_matches('/') {
        return Err(refuse(404, "another path"));
    }
    let mut length = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            return Err(refuse(400, "bad header"));
        };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => {
                let n: usize = value.parse().map_err(|_| refuse(400, "bad length"))?;
                if length.is_some_and(|l| l != n) {
                    return Err(refuse(400, "two lengths"));
                }
                length = Some(n);
            }
            "transfer-encoding" => return Err(refuse(411, "chunked body")),
            "content-type" => {
                let kind = value.split(';').next().unwrap_or("").trim();
                if !kind.eq_ignore_ascii_case("application/x-www-form-urlencoded") {
                    return Err(refuse(415, "not a form"));
                }
            }
            _ => {}
        }
    }
    let length = length.ok_or(refuse(411, "no length"))?;
    if length > MAX_BODY {
        return Err(refuse(413, "body too long"));
    }
    let mut body = buf.split_off(end + 4);
    if body.len() > length {
        return Err(refuse(400, "more than announced"));
    }
    let have = body.len();
    body.resize(length, 0);
    stream
        .read_exact(&mut body[have..])
        .await
        .map_err(|_| refuse(400, "body cut short"))?;
    Ok(Posted { body })
}

/// The answer, then the connection closes. The gateway only looks at the
/// status.
pub async fn answer<S: AsyncWrite + Unpin>(stream: &mut S, status: u16) {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        411 => "Length Required",
        413 => "Content Too Large",
        415 => "Unsupported Media Type",
        431 => "Request Header Fields Too Large",
        _ => "Service Unavailable",
    };
    let body = if status == 200 { "OK" } else { reason };
    let text = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(text.as_bytes()).await;
    let _ = stream.shutdown().await;
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Whether an address is on the home network: private, link-local or this
/// machine. Anything else (a port forwarded by mistake, a tunnel) is
/// refused before a byte is read.
pub fn local(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(v4) => v4.is_private() || v4.is_link_local() || v4.is_loopback(),
        IpAddr::V6(v6) => {
            let first = v6.segments()[0];
            v6.is_loopback() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn ask(request: &[u8]) -> Result<Vec<u8>, Refusal> {
        let (mut client, mut server) = tokio::io::duplex(64 * 1024);
        client.write_all(request).await.unwrap();
        client.shutdown().await.unwrap();
        read(&mut server, "/data/report/").await.map(|p| p.body)
    }

    fn post(path: &str, headers: &str, body: &str) -> Vec<u8> {
        format!("POST {path} HTTP/1.1\r\nHost: 192.168.1.10:49199\r\n{headers}\r\n{body}")
            .into_bytes()
    }

    #[tokio::test]
    async fn a_gateway_post_is_read() {
        let body = "PASSKEY=X&tempf=50.0";
        let headers = format!(
            "Content-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n",
            body.len()
        );
        assert_eq!(
            ask(&post("/data/report/", &headers, body)).await.unwrap(),
            body.as_bytes()
        );
        // Without the trailing slash, or with a query (dropped).
        assert!(ask(&post("/data/report", &headers, body)).await.is_ok());
        assert!(
            ask(&post("/data/report/?ID=x", &headers, body))
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn everything_else_is_refused() {
        let ok = "Content-Length: 3\r\n";
        let status = |r: Result<Vec<u8>, Refusal>| r.unwrap_err().status;
        assert_eq!(
            status(ask(b"GET /data/report/?ID=x&PASSWORD=y HTTP/1.1\r\n\r\n").await),
            405
        );
        assert_eq!(status(ask(&post("/other", ok, "a=1")).await), 404);
        assert_eq!(status(ask(&post("/data/report/", "", "a=1")).await), 411);
        assert_eq!(
            status(
                ask(&post(
                    "/data/report/",
                    "Transfer-Encoding: chunked\r\n",
                    "3\r\na=1\r\n0\r\n\r\n"
                ))
                .await
            ),
            411
        );
        let big = format!("Content-Length: {}\r\n", MAX_BODY + 1);
        assert_eq!(status(ask(&post("/data/report/", &big, "")).await), 413);
        assert_eq!(
            status(
                ask(&post(
                    "/data/report/",
                    "Content-Type: application/json\r\nContent-Length: 2\r\n",
                    "{}"
                ))
                .await
            ),
            415
        );
        assert_eq!(
            status(ask(&post("/data/report/", "Content-Length: 10\r\n", "a=1")).await),
            400,
            "body shorter than announced"
        );
        let mut long = post("/data/report/", ok, "a=1");
        long.splice(
            20..20,
            "X-Pad: ".bytes().chain(std::iter::repeat_n(b'a', MAX_HEAD)),
        );
        assert_eq!(status(ask(&long).await), 431);
        assert_eq!(status(ask(b"POST /data/report/\r\n\r\n").await), 400);
    }

    #[test]
    fn only_the_home_network() {
        for ip in [
            "192.168.1.20",
            "10.0.0.5",
            "172.16.3.4",
            "127.0.0.1",
            "169.254.1.1",
        ] {
            assert!(local(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["::1", "fd00::1", "fe80::1", "::ffff:192.168.1.20"] {
            assert!(local(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "100.64.0.1", "2001:db8::1", "::ffff:8.8.8.8"] {
            assert!(!local(ip.parse().unwrap()), "{ip}");
        }
    }
}
