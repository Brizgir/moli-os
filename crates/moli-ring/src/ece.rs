//! Web push messages decrypted: RFC 8291 (« aes128gcm », RFC 8188), and the
//! older « aesgcm » draft (draft-ietf-webpush-encryption-04) that Firebase
//! still uses for the messages it relays to a web client. The receiver's
//! key pair (P-256) and authentication secret are made once, given to
//! Firebase at registration, and kept in the secret store with the rest of
//! the push credentials ([`crate::fcm`]).
//!
//! ECDH needs a private key that lasts (the same one decrypts every
//! message): `ring` only offers single-use keys, so the curve comes from
//! `p256` (pure Rust); HKDF and AES-GCM stay with `ring`.

use std::fmt::Write as _;

use anyhow::{Context as _, bail};
use p256::elliptic_curve::sec1::ToEncodedPoint as _;
use p256::{PublicKey, SecretKey};
use ring::aead::{AES_128_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use ring::hkdf;
use ring::rand::{SecureRandom as _, SystemRandom};

/// The receiver's keys: never written anywhere but the secret store.
#[derive(Clone)]
pub(crate) struct Keys {
    secret: SecretKey,
    /// Uncompressed point (65 bytes), as Firebase is given it.
    public: Vec<u8>,
    auth: [u8; 16],
}

impl std::fmt::Debug for Keys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Keys").finish_non_exhaustive()
    }
}

impl PartialEq for Keys {
    fn eq(&self, other: &Self) -> bool {
        self.public == other.public && self.auth == other.auth
    }
}

impl Keys {
    /// A new key pair and authentication secret.
    pub(crate) fn generate() -> anyhow::Result<Self> {
        let random = SystemRandom::new();
        let mut auth = [0u8; 16];
        random
            .fill(&mut auth)
            .map_err(|_| anyhow::anyhow!("no randomness"))?;
        // A random 32-byte string is a valid scalar but for about 2^-32 of
        // them: drawn again then.
        for _ in 0..8 {
            let mut d = [0u8; 32];
            random
                .fill(&mut d)
                .map_err(|_| anyhow::anyhow!("no randomness"))?;
            if let Ok(keys) = Self::from_parts(&d, &auth) {
                return Ok(keys);
            }
        }
        bail!("no valid key drawn")
    }

    /// From the private scalar (32 bytes) and the authentication secret.
    pub(crate) fn from_parts(private: &[u8], auth: &[u8]) -> anyhow::Result<Self> {
        let secret =
            SecretKey::from_slice(private).map_err(|_| anyhow::anyhow!("not a private key"))?;
        let public = secret
            .public_key()
            .to_encoded_point(false)
            .as_bytes()
            .to_vec();
        let auth = auth.try_into().context("authentication secret")?;
        Ok(Self {
            secret,
            public,
            auth,
        })
    }

    pub(crate) fn private_bytes(&self) -> Vec<u8> {
        self.secret.to_bytes().to_vec()
    }

    pub(crate) fn public(&self) -> &[u8] {
        &self.public
    }

    pub(crate) fn auth(&self) -> &[u8] {
        &self.auth
    }

    /// The x coordinate of our private key times the sender's point.
    fn agree(&self, sender: &[u8]) -> anyhow::Result<Vec<u8>> {
        let sender = PublicKey::from_sec1_bytes(sender)
            .map_err(|_| anyhow::anyhow!("not a public key (sender)"))?;
        let shared = (sender.to_projective() * *self.secret.to_nonzero_scalar()).to_affine();
        let point = shared.to_encoded_point(false);
        Ok(point.x().context("shared point at infinity")?.to_vec())
    }
}

/// How a message was encrypted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Encoding<'a> {
    /// RFC 8291: salt, record size and the sender's key are in the body.
    Aes128gcm,
    /// The older draft: the sender's key (`dh`) and the salt come apart
    /// (Firebase gives them as `crypto-key` and `encryption`).
    Aesgcm {
        dh: &'a [u8],
        salt: &'a [u8],
        rs: usize,
    },
}

struct Len(usize);

impl hkdf::KeyType for Len {
    fn len(&self) -> usize {
        self.0
    }
}

fn extract(salt: &[u8], ikm: &[u8]) -> hkdf::Prk {
    hkdf::Salt::new(hkdf::HKDF_SHA256, salt).extract(ikm)
}

fn expand(prk: &hkdf::Prk, info: &[&[u8]], len: usize) -> anyhow::Result<Vec<u8>> {
    let mut out = vec![0u8; len];
    prk.expand(info, Len(len))
        .and_then(|okm| okm.fill(&mut out))
        .map_err(|_| anyhow::anyhow!("HKDF"))?;
    Ok(out)
}

/// `len` as two bytes, big-endian (the « aesgcm » context).
fn be16(len: usize) -> anyhow::Result<[u8; 2]> {
    Ok(u16::try_from(len).context("too long")?.to_be_bytes())
}

/// Decrypts one message's body.
pub(crate) fn decrypt(keys: &Keys, encoding: Encoding<'_>, body: &[u8]) -> anyhow::Result<Vec<u8>> {
    match encoding {
        Encoding::Aes128gcm => {
            // salt (16) | rs (4) | idlen (1) | keyid (the sender's key)
            let header = body.get(..21).context("header cut short")?;
            let salt = &header[..16];
            let rs = usize::try_from(u32::from_be_bytes(header[16..20].try_into()?))?;
            let idlen = usize::from(header[20]);
            let sender = body.get(21..21 + idlen).context("key id cut short")?;
            let data = &body[21 + idlen..];
            let ecdh = keys.agree(sender)?;
            let ikm = expand(
                &extract(&keys.auth, &ecdh),
                &[b"WebPush: info\0", &keys.public, sender],
                32,
            )?;
            let prk = extract(salt, &ikm);
            let cek = expand(&prk, &[b"Content-Encoding: aes128gcm\0"], 16)?;
            let nonce = expand(&prk, &[b"Content-Encoding: nonce\0"], NONCE_LEN)?;
            if rs <= 16 {
                bail!("record size {rs}");
            }
            open_records(&cek, &nonce, data, rs, |plain, last| {
                // data | delimiter (2 last, 1 otherwise) | zeros
                let end = plain
                    .iter()
                    .rposition(|b| *b != 0)
                    .context("no delimiter")?;
                match (plain[end], last) {
                    (2, true) | (1, false) => Ok(end),
                    _ => bail!("bad padding delimiter"),
                }
                .map(|end| (0, end))
            })
        }
        Encoding::Aesgcm { dh, salt, rs } => {
            let ecdh = keys.agree(dh)?;
            let ikm = expand(
                &extract(&keys.auth, &ecdh),
                &[b"Content-Encoding: auth\0"],
                32,
            )?;
            let context = [
                b"P-256\0".as_slice(),
                &be16(keys.public.len())?,
                &keys.public,
                &be16(dh.len())?,
                dh,
            ]
            .concat();
            let prk = extract(salt, &ikm);
            let cek = expand(&prk, &[b"Content-Encoding: aesgcm\0", &context], 16)?;
            let nonce = expand(&prk, &[b"Content-Encoding: nonce\0", &context], NONCE_LEN)?;
            // `rs` counts the plaintext; each record carries a 16-byte tag.
            open_records(&cek, &nonce, body, rs + 16, |plain, _| {
                // padding length (2) | zeros | data
                let pad = usize::from(u16::from_be_bytes(
                    plain.get(..2).context("record cut short")?.try_into()?,
                ));
                let start = 2 + pad;
                if start > plain.len() || plain[2..start].iter().any(|b| *b != 0) {
                    bail!("bad padding");
                }
                Ok((start, plain.len()))
            })
        }
    }
}

/// Each record opened with its own nonce (the base nonce XOR its number);
/// `keep` says which part of a record's plaintext is data.
fn open_records(
    cek: &[u8],
    nonce: &[u8],
    data: &[u8],
    rs: usize,
    keep: impl Fn(&[u8], bool) -> anyhow::Result<(usize, usize)>,
) -> anyhow::Result<Vec<u8>> {
    if data.is_empty() {
        bail!("nothing to decrypt");
    }
    let key = LessSafeKey::new(
        UnboundKey::new(&AES_128_GCM, cek).map_err(|_| anyhow::anyhow!("content key"))?,
    );
    let count = data.len().div_ceil(rs);
    let mut out = Vec::with_capacity(data.len());
    for (seq, record) in data.chunks(rs).enumerate() {
        let mut n: [u8; NONCE_LEN] = nonce.try_into()?;
        for (b, s) in n[NONCE_LEN - 8..]
            .iter_mut()
            .zip((seq as u64).to_be_bytes())
        {
            *b ^= s;
        }
        let mut buf = record.to_vec();
        let plain = key
            .open_in_place(Nonce::assume_unique_for_key(n), Aad::empty(), &mut buf)
            .map_err(|_| anyhow::anyhow!("the message does not decrypt with our keys"))?;
        let (start, end) = keep(plain, seq + 1 == count)?;
        out.extend_from_slice(&plain[start..end]);
    }
    Ok(out)
}

/// URL-safe base64 without padding (what Firebase is given).
pub(crate) fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..=chunk.len() {
            let _ = out.write_char(char::from(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize]));
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::signin::base64;

    fn b64(s: &str) -> Vec<u8> {
        base64(&s.replace([' ', '\n'], "")).unwrap()
    }

    // RFC 8291, section 5 and appendix A.
    const UA_PRIVATE: &str = "q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94";
    const UA_PUBLIC: &str =
        "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4";
    const AUTH: &str = "BTBZMqHH6r4Tts7J_aSIgg";
    const AS_PUBLIC: &str =
        "BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8";
    const MESSAGE: &str = "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN";

    fn rfc_keys() -> Keys {
        Keys::from_parts(&b64(UA_PRIVATE), &b64(AUTH)).unwrap()
    }

    #[test]
    fn rfc_8291_s_example_decrypts() {
        let keys = rfc_keys();
        assert_eq!(keys.public(), b64(UA_PUBLIC), "the public key follows");
        assert_eq!(
            keys.agree(&b64(AS_PUBLIC)).unwrap(),
            b64("kyrL1jIIOHEzg3sM2ZWRHDRB62YACZhhSlknJ672kSs"),
            "ecdh_secret"
        );
        let plain = decrypt(&keys, Encoding::Aes128gcm, &b64(MESSAGE)).unwrap();
        assert_eq!(plain, b"When I grow up, I want to be a watermelon");
    }

    #[test]
    fn a_changed_byte_or_other_keys_never_decrypt() {
        let mut message = b64(MESSAGE);
        let last = message.len() - 1;
        message[last] ^= 1;
        assert!(decrypt(&rfc_keys(), Encoding::Aes128gcm, &message).is_err());
        let others = Keys::generate().unwrap();
        assert!(decrypt(&others, Encoding::Aes128gcm, &b64(MESSAGE)).is_err());
        assert!(decrypt(&rfc_keys(), Encoding::Aes128gcm, &[0; 10]).is_err());
    }

    /// Encrypts as a sender would, with the « aesgcm » draft (the sender's
    /// private key given): for the tests of the receiving side.
    pub(crate) fn aesgcm(keys: &Keys, sender: &Keys, salt: &[u8], plain: &[u8]) -> Vec<u8> {
        let ecdh = sender.agree(keys.public()).unwrap();
        let ikm = expand(
            &extract(keys.auth(), &ecdh),
            &[b"Content-Encoding: auth\0"],
            32,
        )
        .unwrap();
        let context = [
            b"P-256\0".as_slice(),
            &be16(65).unwrap(),
            keys.public(),
            &be16(65).unwrap(),
            sender.public(),
        ]
        .concat();
        let prk = extract(salt, &ikm);
        let cek = expand(&prk, &[b"Content-Encoding: aesgcm\0", &context], 16).unwrap();
        let nonce = expand(&prk, &[b"Content-Encoding: nonce\0", &context], NONCE_LEN).unwrap();
        let key = LessSafeKey::new(UnboundKey::new(&AES_128_GCM, &cek).unwrap());
        let mut record = [&[0u8, 0][..], plain].concat();
        key.seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce.try_into().unwrap()),
            Aad::empty(),
            &mut record,
        )
        .unwrap();
        record
    }

    #[test]
    fn the_older_aesgcm_draft_decrypts_too() {
        let keys = Keys::generate().unwrap();
        let sender = Keys::generate().unwrap();
        let salt = [7u8; 16];
        let body = aesgcm(&keys, &sender, &salt, br#"{"data":{}}"#);
        let encoding = Encoding::Aesgcm {
            dh: sender.public(),
            salt: &salt,
            rs: 4096,
        };
        assert_eq!(decrypt(&keys, encoding, &body).unwrap(), br#"{"data":{}}"#);
        let wrong = Encoding::Aesgcm {
            dh: sender.public(),
            salt: &[8u8; 16],
            rs: 4096,
        };
        assert!(decrypt(&keys, wrong, &body).is_err(), "another salt");
    }

    #[test]
    fn keys_come_back_from_their_parts_and_never_show() {
        let keys = Keys::generate().unwrap();
        let again = Keys::from_parts(&keys.private_bytes(), keys.auth()).unwrap();
        assert_eq!(again, keys);
        assert_eq!(again.public().len(), 65);
        assert_eq!(format!("{keys:?}"), "Keys { .. }");
        assert!(
            Keys::from_parts(&[0; 32], keys.auth()).is_err(),
            "zero scalar"
        );
        assert!(Keys::from_parts(&keys.private_bytes(), &[1; 3]).is_err());
    }

    #[test]
    fn base64url_round_trips() {
        for n in 0..40u8 {
            let bytes: Vec<u8> = (0..n).map(|i| i.wrapping_mul(37)).collect();
            let text = base64url(&bytes);
            assert!(!text.contains(['=', '+', '/']), "{text}");
            assert_eq!(base64(&text).unwrap(), bytes);
        }
        assert_eq!(base64url(b"Man"), "TWFu");
        assert_eq!(base64url(b"Ma"), "TWE");
    }
}
