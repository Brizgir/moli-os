//! Just enough HTTP/1.1 to hear a station: one `POST` with a form, one
//! « OK », the connection closed. Everything is bounded (head, body, time):
//! anyone on the network can knock.

use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt as _};

/// A station's head is a few hundred bytes.
pub const MAX_HEAD: usize = 8 * 1024;
/// A station's form is 1 to 3 KiB, even with every sensor paired.
pub const MAX_BODY: usize = 32 * 1024;
/// The whole request, from the first byte.
pub const READ_LIMIT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Head {
    pub method: String,
    /// Without its query.
    pub path: String,
    pub length: usize,
}

/// Why a request is not heard: the status it gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    BadRequest,
    NotFound,
    MethodNotAllowed,
    Timeout,
    LengthRequired,
    TooLarge,
    Busy,
    HeadTooLarge,
}

impl Refusal {
    fn status(self) -> (u16, &'static str) {
        match self {
            Self::BadRequest => (400, "Bad Request"),
            Self::NotFound => (404, "Not Found"),
            Self::MethodNotAllowed => (405, "Method Not Allowed"),
            Self::Timeout => (408, "Request Timeout"),
            Self::LengthRequired => (411, "Length Required"),
            Self::TooLarge => (413, "Content Too Large"),
            Self::Busy => (503, "Service Unavailable"),
            Self::HeadTooLarge => (431, "Request Header Fields Too Large"),
        }
    }

    pub fn answer(self) -> Vec<u8> {
        let (status, reason) = self.status();
        answer(status, reason, reason)
    }
}

/// What the station waits for after its upload.
pub fn ok() -> Vec<u8> {
    answer(200, "OK", "OK")
}

fn answer(status: u16, reason: &str, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

/// The request line and the headers that matter.
pub fn head(text: &str) -> Result<Head, Refusal> {
    let mut lines = text.split("\r\n");
    let mut request = lines.next().unwrap_or_default().split(' ');
    let (Some(method), Some(target), Some(version), None) = (
        request.next(),
        request.next(),
        request.next(),
        request.next(),
    ) else {
        return Err(Refusal::BadRequest);
    };
    if !version.starts_with("HTTP/1.") || method.is_empty() || !target.starts_with('/') {
        return Err(Refusal::BadRequest);
    }
    let mut length = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            return Err(Refusal::BadRequest);
        };
        let name = name.trim();
        if name.eq_ignore_ascii_case("transfer-encoding") {
            // No station chunks its form: never parse what is never sent.
            return Err(Refusal::LengthRequired);
        }
        if name.eq_ignore_ascii_case("content-length") {
            let n: usize = value.trim().parse().map_err(|_| Refusal::BadRequest)?;
            if length.is_some_and(|l| l != n) {
                return Err(Refusal::BadRequest);
            }
            length = Some(n);
        }
    }
    let length = match (method, length) {
        ("POST", None) => return Err(Refusal::LengthRequired),
        (_, length) => length.unwrap_or(0),
    };
    if length > MAX_BODY {
        return Err(Refusal::TooLarge);
    }
    Ok(Head {
        method: method.to_owned(),
        path: target.split('?').next().unwrap_or(target).to_owned(),
        length,
    })
}

/// One request: its head and its whole body. The caller bounds the time.
pub async fn read<S: AsyncRead + Unpin>(stream: &mut S) -> Result<(Head, Vec<u8>), Refusal> {
    let mut buffer = Vec::with_capacity(2048);
    let mut chunk = [0u8; 2048];
    let end = loop {
        if let Some(i) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if buffer.len() > MAX_HEAD {
            return Err(Refusal::HeadTooLarge);
        }
        let n = stream
            .read(&mut chunk)
            .await
            .map_err(|_| Refusal::BadRequest)?;
        if n == 0 {
            return Err(Refusal::BadRequest);
        }
        buffer.extend_from_slice(&chunk[..n]);
    };
    if end > MAX_HEAD {
        return Err(Refusal::HeadTooLarge);
    }
    let text = std::str::from_utf8(&buffer[..end]).map_err(|_| Refusal::BadRequest)?;
    let head = head(text)?;
    let mut body = buffer.split_off(end + 4);
    while body.len() < head.length {
        let n = stream
            .read(&mut chunk)
            .await
            .map_err(|_| Refusal::BadRequest)?;
        if n == 0 {
            return Err(Refusal::BadRequest);
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(head.length);
    Ok((head, body))
}

/// `a=1&b=x+y` → `[(a, 1), (b, x y)]` (`application/x-www-form-urlencoded`).
pub fn form(body: &[u8]) -> Vec<(String, String)> {
    body.split(|b| *b == b'&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| match pair.iter().position(|b| *b == b'=') {
            Some(i) => (decode(&pair[..i]), decode(&pair[i + 1..])),
            None => (decode(pair), String::new()),
        })
        .collect()
}

fn decode(text: &[u8]) -> String {
    let hex = |b: u8| char::from(b).to_digit(16);
    let mut out = Vec::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        match text[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < text.len() => {
                match (hex(text[i + 1]), hex(text[i + 2])) {
                    (Some(h), Some(l)) => {
                        // Two hex digits: below 256.
                        out.push(u8::try_from(h * 16 + l).unwrap_or(b'?'));
                        i += 2;
                    }
                    _ => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_station_upload_is_read() {
        let body = b"PASSKEY=0123456789ABCDEF0123456789ABCDEF&stationtype=GW2000A_V3.1.0&dateutc=2024-02-08+10%3A36%3A28&tempf=87.08&lightning=";
        let request = [
            format!(
                "POST /data/report/ HTTP/1.1\r\nHost: 192.168.0.20:49199\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n",
                body.len()
            )
            .into_bytes(),
            body.to_vec(),
        ]
        .concat();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let (head, got) = runtime.block_on(read(&mut request.as_slice())).unwrap();
        assert_eq!(head.method, "POST");
        assert_eq!(head.path, "/data/report/");
        assert_eq!(got, body);
        let form = form(&got);
        assert_eq!(form[2], ("dateutc".into(), "2024-02-08 10:36:28".into()));
        assert_eq!(form[4], ("lightning".into(), String::new()));
    }

    #[test]
    fn what_is_not_an_upload_is_refused() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let read_all = |text: &[u8]| runtime.block_on(read(&mut &text[..])).map(|(h, _)| h);
        assert_eq!(
            read_all(b"POST / HTTP/1.1\r\n\r\n"),
            Err(Refusal::LengthRequired)
        );
        assert_eq!(
            read_all(b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n"),
            Err(Refusal::LengthRequired)
        );
        assert_eq!(
            read_all(b"POST / HTTP/1.1\r\nContent-Length: 999999\r\n\r\n"),
            Err(Refusal::TooLarge)
        );
        assert_eq!(
            read_all(b"POST / HTTP/1.1\r\nContent-Length: 1\r\nContent-Length: 2\r\n\r\n"),
            Err(Refusal::BadRequest)
        );
        assert_eq!(read_all(b"hello\r\n\r\n"), Err(Refusal::BadRequest));
        // Cut short: the body never comes.
        assert_eq!(
            read_all(b"POST / HTTP/1.1\r\nContent-Length: 10\r\n\r\nabc"),
            Err(Refusal::BadRequest)
        );
        // A head without end.
        let endless = vec![b'a'; MAX_HEAD + 4096];
        assert_eq!(read_all(&endless), Err(Refusal::HeadTooLarge));
        assert_eq!(read_all(b"GET /x?y=1 HTTP/1.1\r\n\r\n").unwrap().path, "/x");
    }

    #[test]
    fn forms_are_decoded() {
        assert_eq!(
            form(b"a=1&&b=x+y%21&c&d=%zz%4"),
            [
                ("a".into(), "1".into()),
                ("b".into(), "x y!".into()),
                ("c".into(), String::new()),
                ("d".into(), "%zz%4".into()),
            ]
        );
        assert!(String::from_utf8(ok()).unwrap().ends_with("\r\n\r\nOK"));
    }
}
