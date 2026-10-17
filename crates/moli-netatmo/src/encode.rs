//! The few encodings Netatmo's API needs: percent-encoding, forms, an
//! address's query parameters, hexadecimal.

/// Percent-encoding of everything but the unreserved characters.
pub(crate) fn percent(text: &str) -> String {
    text.bytes()
        .fold(String::with_capacity(text.len()), |mut s, b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                s.push(char::from(b));
            } else {
                s.push('%');
                s.push(char::from(b"0123456789ABCDEF"[usize::from(b >> 4)]));
                s.push(char::from(b"0123456789ABCDEF"[usize::from(b & 15)]));
            }
            s
        })
}

/// `a=1&b=2`, each value percent-encoded.
pub(crate) fn form(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", percent(k), percent(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Lowercase hexadecimal.
pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            s.push(char::from(b"0123456789abcdef"[usize::from(b >> 4)]));
            s.push(char::from(b"0123456789abcdef"[usize::from(b & 15)]));
            s
        })
}

fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..=i + 2]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The decoded value of parameter `name` in an address's query.
pub(crate) fn query_param(address: &str, name: &str) -> Option<String> {
    let query = address.split_once('?')?.1;
    let query = query.split('#').next().unwrap_or(query);
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        (decode(k) == name).then(|| decode(v))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_encoded_and_read_back() {
        assert_eq!(percent("a b/é"), "a%20b%2F%C3%A9");
        assert_eq!(form(&[("x", "1 2"), ("y", "&")]), "x=1%202&y=%26");
        assert_eq!(hex(&[0, 0xab]), "00ab");
        let url = "http://localhost/moli-netatmo?state=s1&code=c%2Fd#x";
        assert_eq!(query_param(url, "code").as_deref(), Some("c/d"));
        assert_eq!(query_param(url, "state").as_deref(), Some("s1"));
        assert_eq!(query_param(url, "missing"), None);
        assert_eq!(query_param("no query", "a"), None);
    }
}
