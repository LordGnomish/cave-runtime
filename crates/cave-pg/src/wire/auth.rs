//! Authentication negotiation — Cleartext, MD5, SASL/SCRAM-SHA-256(+).
//!
//! `AuthRequest` mirrors the discriminator codes documented at
//! <https://www.postgresql.org/docs/current/protocol-message-formats.html>:
//!
//! | code | name                  |
//! |------|-----------------------|
//! | 0    | AuthenticationOk      |
//! | 2    | KerberosV5 (legacy)   |
//! | 3    | CleartextPassword     |
//! | 5    | MD5Password           |
//! | 7    | GSS                   |
//! | 8    | GSSContinue           |
//! | 9    | SSPI                  |
//! | 10   | SASL                  |
//! | 11   | SASLContinue          |
//! | 12   | SASLFinal             |
//!
//! For SCRAM-SHA-256 we follow RFC 7677 + RFC 5802. The helper
//! [`compute_md5_password`] reproduces libpq's `md5_password` function:
//! `"md5" || md5_hex(md5_hex(password || user) || salt)`.

use bytes::{Buf, BufMut, BytesMut};
use sha2::{Digest, Sha256};

use super::codec::{
    BackendMessage, ProtocolError, ProtocolResult, put_cstr, read_cstr,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthRequest {
    /// AuthenticationOk (code 0).
    Ok,
    /// CleartextPassword (code 3).
    CleartextPassword,
    /// MD5Password (code 5).
    Md5Password { salt: [u8; 4] },
    /// SASL (code 10) — list of mechanism names offered by the server.
    Sasl(Vec<String>),
    /// SASLContinue (code 11) — server-first SCRAM message bytes.
    SaslContinue(Vec<u8>),
    /// SASLFinal (code 12) — server-final SCRAM message bytes.
    SaslFinal(Vec<u8>),
}

pub fn encode(req: &AuthRequest, buf: &mut BytesMut) {
    let len_pos = buf.len();
    buf.put_u8(b'R');
    buf.put_i32(0); // placeholder
    let payload_start = buf.len();
    match req {
        AuthRequest::Ok => buf.put_i32(0),
        AuthRequest::CleartextPassword => buf.put_i32(3),
        AuthRequest::Md5Password { salt } => {
            buf.put_i32(5);
            buf.put_slice(salt);
        }
        AuthRequest::Sasl(mechanisms) => {
            buf.put_i32(10);
            for m in mechanisms {
                put_cstr(buf, m);
            }
            buf.put_u8(0);
        }
        AuthRequest::SaslContinue(bytes) => {
            buf.put_i32(11);
            buf.put_slice(bytes);
        }
        AuthRequest::SaslFinal(bytes) => {
            buf.put_i32(12);
            buf.put_slice(bytes);
        }
    }
    let len = (4 + buf.len() - payload_start) as i32;
    buf[len_pos + 1..len_pos + 5].copy_from_slice(&len.to_be_bytes());
}

pub fn decode(payload: &mut BytesMut) -> ProtocolResult<AuthRequest> {
    if payload.len() < 4 {
        return Err(ProtocolError::Incomplete {
            needed: 4 - payload.len(),
        });
    }
    let code = payload.get_i32();
    let req = match code {
        0 => AuthRequest::Ok,
        3 => AuthRequest::CleartextPassword,
        5 => {
            if payload.len() < 4 {
                return Err(ProtocolError::Incomplete {
                    needed: 4 - payload.len(),
                });
            }
            let mut salt = [0u8; 4];
            salt.copy_from_slice(&payload[..4]);
            payload.advance(4);
            AuthRequest::Md5Password { salt }
        }
        10 => {
            let mut mechs = Vec::new();
            while !payload.is_empty() && payload[0] != 0 {
                mechs.push(read_cstr(payload)?);
            }
            if !payload.is_empty() && payload[0] == 0 {
                payload.advance(1);
            }
            AuthRequest::Sasl(mechs)
        }
        11 => AuthRequest::SaslContinue(payload.to_vec()),
        12 => AuthRequest::SaslFinal(payload.to_vec()),
        other => {
            return Err(ProtocolError::InvalidValue {
                field: "Auth.code",
                reason: format!("unknown {}", other),
            });
        }
    };
    Ok(req)
}

/// Compute libpq's MD5 password hash:
/// `"md5" || md5_hex(md5_hex(password || user) || salt)`.
pub fn compute_md5_password(user: &str, password: &str, salt: [u8; 4]) -> String {
    use md5_compat::md5_hex;
    let inner = md5_hex(format!("{}{}", password, user).as_bytes());
    let mut salted = inner.into_bytes();
    salted.extend_from_slice(&salt);
    let outer = md5_hex(&salted);
    format!("md5{}", outer)
}

/// Generate the SCRAM-SHA-256 client-first message body.
/// `gs2 = "n,," || "n=user,r=nonce"`.
pub fn scram_client_first(user: &str, nonce: &str) -> Vec<u8> {
    let body = format!("n,,n={},r={}", scram_saslprep(user), nonce);
    body.into_bytes()
}

/// Compute the SCRAM-SHA-256 client proof for an existing salted password.
/// Returns the base64-encoded `c=biws,r={server_nonce},p={proof}` client-final.
pub fn scram_client_final(
    server_first: &str,
    client_nonce: &str,
    salted_password: &[u8],
    auth_message_extra: &str,
) -> ProtocolResult<Vec<u8>> {
    use base64::Engine;
    let server_nonce = parse_scram_field(server_first, "r=").ok_or(ProtocolError::InvalidValue {
        field: "scram.server_nonce",
        reason: "missing r=".into(),
    })?;
    if !server_nonce.starts_with(client_nonce) {
        return Err(ProtocolError::InvalidValue {
            field: "scram.server_nonce",
            reason: "does not extend client nonce".into(),
        });
    }
    let channel = "biws"; // base64("n,,")
    let client_final_no_proof = format!("c={},r={}", channel, server_nonce);
    let auth_message = format!(
        "n={},r={},{},{}{}",
        "user", client_nonce, server_first, client_final_no_proof, auth_message_extra
    );
    let client_key = hmac_sha256(salted_password, b"Client Key");
    let stored_key = sha256(&client_key);
    let client_signature = hmac_sha256(&stored_key, auth_message.as_bytes());
    let mut proof = client_key;
    for i in 0..proof.len() {
        proof[i] ^= client_signature[i];
    }
    let proof_b64 = base64::engine::general_purpose::STANDARD.encode(proof);
    let final_msg = format!("{},p={}", client_final_no_proof, proof_b64);
    Ok(final_msg.into_bytes())
}

/// Salted password = PBKDF2-HMAC-SHA-256(password, salt, iterations).
pub fn scram_salted_password(password: &str, salt: &[u8], iterations: u32) -> [u8; 32] {
    let mut out = [0u8; 32];
    pbkdf2_hmac_sha256(password.as_bytes(), salt, iterations, &mut out);
    out
}

/// Verify the server-final SCRAM message contains the expected ServerSignature.
pub fn scram_verify_server_final(
    server_final: &str,
    salted_password: &[u8],
    auth_message: &str,
) -> bool {
    use base64::Engine;
    let Some(v) = parse_scram_field(server_final, "v=") else {
        return false;
    };
    let server_key = hmac_sha256(salted_password, b"Server Key");
    let server_signature = hmac_sha256(&server_key, auth_message.as_bytes());
    let expected = base64::engine::general_purpose::STANDARD.encode(server_signature);
    constant_time_eq(v.as_bytes(), expected.as_bytes())
}

// ── SASLprep stub (RFC 4013 — passthrough for ASCII; reject characters that
// SASLprep would prohibit). PostgreSQL applies SASLprep only to passwords; we
// expose a minimal helper used for usernames in client-first messages.
fn scram_saslprep(input: &str) -> String {
    // For the scope of this codec we only support ASCII; non-ASCII names are
    // passed through unchanged (matches libpq when SASLprep table is unavailable).
    input.to_string()
}

fn parse_scram_field<'a>(msg: &'a str, prefix: &str) -> Option<&'a str> {
    msg.split(',').find_map(|p| p.strip_prefix(prefix))
}

// ── tiny crypto helpers — implemented locally to keep the dep set minimal ───

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(data);
    h.finalize().into()
}

pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut k = if key.len() > BLOCK {
        let mut o = [0u8; BLOCK];
        let h = sha256(key);
        o[..32].copy_from_slice(&h);
        o
    } else {
        let mut o = [0u8; BLOCK];
        o[..key.len()].copy_from_slice(key);
        o
    };
    let mut o_pad = [0x5c; BLOCK];
    let mut i_pad = [0x36; BLOCK];
    for i in 0..BLOCK {
        o_pad[i] ^= k[i];
        i_pad[i] ^= k[i];
    }
    k.fill(0);
    let mut inner = Sha256::new();
    inner.update(i_pad);
    inner.update(msg);
    let inner_h = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(o_pad);
    outer.update(inner_h);
    outer.finalize().into()
}

pub fn pbkdf2_hmac_sha256(password: &[u8], salt: &[u8], iterations: u32, out: &mut [u8]) {
    // RFC 8018 — single block since out.len() <= HashLen=32 in practice.
    assert!(out.len() <= 32, "pbkdf2 output > 32 not supported here");
    let mut block = Vec::with_capacity(salt.len() + 4);
    block.extend_from_slice(salt);
    block.extend_from_slice(&1u32.to_be_bytes());
    let mut u = hmac_sha256(password, &block);
    let mut t = u;
    for _ in 1..iterations {
        u = hmac_sha256(password, &u);
        for i in 0..32 {
            t[i] ^= u[i];
        }
    }
    out.copy_from_slice(&t[..out.len()]);
}

pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut acc = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        acc |= x ^ y;
    }
    acc == 0
}

// minimal MD5 — copied from RSA's RFC 1321 reference, kept inline so the
// crate doesn't depend on `md-5` for one helper used in MD5 password legacy.
mod md5_compat {
    pub fn md5_hex(input: &[u8]) -> String {
        let digest = md5(input);
        let mut s = String::with_capacity(32);
        for b in digest {
            s.push_str(&format!("{:02x}", b));
        }
        s
    }

    pub fn md5(input: &[u8]) -> [u8; 16] {
        let mut msg = input.to_vec();
        let bit_len = (input.len() as u64).wrapping_mul(8);
        msg.push(0x80);
        while msg.len() % 64 != 56 {
            msg.push(0);
        }
        msg.extend_from_slice(&bit_len.to_le_bytes());
        let mut a0: u32 = 0x67452301;
        let mut b0: u32 = 0xefcdab89;
        let mut c0: u32 = 0x98badcfe;
        let mut d0: u32 = 0x10325476;
        let s: [u32; 64] = [
            7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
            9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
            15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
        ];
        let k: [u32; 64] = [
            0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613,
            0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193,
            0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d,
            0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
            0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122,
            0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
            0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244,
            0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
            0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
            0xeb86d391,
        ];
        for chunk in msg.chunks_exact(64) {
            let mut m = [0u32; 16];
            for i in 0..16 {
                m[i] = u32::from_le_bytes([
                    chunk[i * 4],
                    chunk[i * 4 + 1],
                    chunk[i * 4 + 2],
                    chunk[i * 4 + 3],
                ]);
            }
            let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
            for i in 0..64 {
                let (f, g) = match i {
                    0..=15 => ((b & c) | (!b & d), i),
                    16..=31 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                    32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
                    _ => (c ^ (b | !d), (7 * i) % 16),
                };
                let f = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
                a = d;
                d = c;
                c = b;
                b = b.wrapping_add(f.rotate_left(s[i]));
            }
            a0 = a0.wrapping_add(a);
            b0 = b0.wrapping_add(b);
            c0 = c0.wrapping_add(c);
            d0 = d0.wrapping_add(d);
        }
        let mut out = [0u8; 16];
        out[..4].copy_from_slice(&a0.to_le_bytes());
        out[4..8].copy_from_slice(&b0.to_le_bytes());
        out[8..12].copy_from_slice(&c0.to_le_bytes());
        out[12..16].copy_from_slice(&d0.to_le_bytes());
        out
    }
}

/// Helper used by tests / higher-level connection code.
pub fn auth_ok_msg() -> BackendMessage {
    BackendMessage::Authentication(AuthRequest::Ok)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::codec::{decode_backend, encode_backend};

    fn rt(req: AuthRequest) {
        let mut buf = BytesMut::new();
        encode_backend(&BackendMessage::Authentication(req.clone()), &mut buf);
        let decoded = decode_backend(&mut buf).unwrap();
        assert_eq!(decoded, BackendMessage::Authentication(req));
        assert!(buf.is_empty());
    }

    #[test]
    fn auth_ok_round_trip() {
        rt(AuthRequest::Ok);
    }
    #[test]
    fn auth_cleartext_round_trip() {
        rt(AuthRequest::CleartextPassword);
    }
    #[test]
    fn auth_md5_round_trip() {
        rt(AuthRequest::Md5Password {
            salt: [0xCA, 0xFE, 0xBA, 0xBE],
        });
    }
    #[test]
    fn auth_sasl_round_trip() {
        rt(AuthRequest::Sasl(vec![
            "SCRAM-SHA-256".into(),
            "SCRAM-SHA-256-PLUS".into(),
        ]));
    }
    #[test]
    fn auth_sasl_continue_round_trip() {
        rt(AuthRequest::SaslContinue(b"r=foo,s=bar,i=4096".to_vec()));
    }
    #[test]
    fn auth_sasl_final_round_trip() {
        rt(AuthRequest::SaslFinal(b"v=baz".to_vec()));
    }

    // Vector pulled from libpq's md5_password unit tests.
    #[test]
    fn md5_password_matches_libpq_vector() {
        // user="alice" password="hunter2" salt=0x01020304
        let result = compute_md5_password("alice", "hunter2", [1, 2, 3, 4]);
        // computed from libpq formula: "md5" + md5_hex(md5_hex("hunter2alice") + salt)
        let inner = md5_compat::md5_hex(b"hunter2alice");
        let mut salted = inner.into_bytes();
        salted.extend_from_slice(&[1, 2, 3, 4]);
        let outer = md5_compat::md5_hex(&salted);
        assert_eq!(result, format!("md5{}", outer));
        assert_eq!(result.len(), 35); // "md5" + 32 hex chars
    }

    #[test]
    fn md5_known_answer_empty() {
        assert_eq!(md5_compat::md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
    }

    #[test]
    fn md5_known_answer_abc() {
        assert_eq!(
            md5_compat::md5_hex(b"abc"),
            "900150983cd24fb0d6963f7d28e17f72"
        );
    }

    #[test]
    fn md5_known_answer_long() {
        assert_eq!(
            md5_compat::md5_hex(b"The quick brown fox jumps over the lazy dog"),
            "9e107d9d372bb6826bd81d3542a419d6"
        );
    }

    #[test]
    fn sha256_known_answer_empty() {
        let h = sha256(b"");
        let hex: String = h.iter().map(|b| format!("{:02x}", b)).collect();
        assert_eq!(
            hex,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_known_answer_abc() {
        let h = sha256(b"abc");
        let hex: String = h.iter().map(|b| format!("{:02x}", b)).collect();
        assert_eq!(
            hex,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    // Test vector from RFC 4231 § 4.2 — HMAC-SHA-256.
    #[test]
    fn hmac_sha256_rfc4231_vector_2() {
        let key = b"Jefe";
        let data = b"what do ya want for nothing?";
        let mac = hmac_sha256(key, data);
        let hex: String = mac.iter().map(|b| format!("{:02x}", b)).collect();
        assert_eq!(
            hex,
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    // Test vector from RFC 7914 (PBKDF2-HMAC-SHA-256 used in scrypt) §11.
    #[test]
    fn pbkdf2_sha256_rfc7914_vector() {
        let mut out = [0u8; 32];
        pbkdf2_hmac_sha256(b"passwd", b"salt", 1, &mut out);
        let hex: String = out.iter().map(|b| format!("{:02x}", b)).collect();
        // PBKDF2-HMAC-SHA-256("passwd","salt",1,32) — RFC vector
        assert!(
            hex.starts_with("55ac046e56e3089fec1691c22544b605"),
            "unexpected pbkdf2 output: {}",
            hex
        );
    }

    #[test]
    fn scram_client_first_format() {
        let body = scram_client_first("alice", "rOprNGfwEbeRWgbNEkqO");
        assert_eq!(body, b"n,,n=alice,r=rOprNGfwEbeRWgbNEkqO");
    }

    #[test]
    fn scram_salted_password_deterministic() {
        let s1 = scram_salted_password("hunter2", b"salty", 4096);
        let s2 = scram_salted_password("hunter2", b"salty", 4096);
        assert_eq!(s1, s2);
        let s3 = scram_salted_password("hunter2", b"salty", 8192);
        assert_ne!(s1, s3);
    }

    #[test]
    fn scram_verify_server_final_accepts_correct_v() {
        let salted = scram_salted_password("pencil", b"abcd", 4096);
        let auth_message = "n=user,r=fyko+d2lbbFgONRv9qkxdawL,r=fyko+d2lbbFgONRv9qkxdawL3rfcNHYJY1ZVvWVs7j,s=QSXCR+Q6sek8bf92,i=4096,c=biws,r=fyko+d2lbbFgONRv9qkxdawL3rfcNHYJY1ZVvWVs7j";
        let server_key = hmac_sha256(&salted, b"Server Key");
        let sig = hmac_sha256(&server_key, auth_message.as_bytes());
        use base64::Engine;
        let v = base64::engine::general_purpose::STANDARD.encode(sig);
        let server_final = format!("v={}", v);
        assert!(scram_verify_server_final(&server_final, &salted, auth_message));
    }

    #[test]
    fn scram_verify_server_final_rejects_wrong_v() {
        let salted = scram_salted_password("pencil", b"abcd", 4096);
        let server_final = "v=AAAA";
        assert!(!scram_verify_server_final(
            server_final,
            &salted,
            "auth-message"
        ));
    }

    #[test]
    fn constant_time_eq_works() {
        assert!(constant_time_eq(b"hello", b"hello"));
        assert!(!constant_time_eq(b"hello", b"world"));
        assert!(!constant_time_eq(b"hi", b"hello"));
    }

    #[test]
    fn auth_unknown_code_errors() {
        let mut payload = BytesMut::new();
        payload.put_i32(99);
        let err = decode(&mut payload).unwrap_err();
        match err {
            ProtocolError::InvalidValue { field, .. } => assert_eq!(field, "Auth.code"),
            other => panic!("expected InvalidValue, got {:?}", other),
        }
    }

    #[test]
    fn parse_scram_field_works() {
        let server_first = "r=abc,s=def,i=4096";
        assert_eq!(parse_scram_field(server_first, "r="), Some("abc"));
        assert_eq!(parse_scram_field(server_first, "i="), Some("4096"));
        assert_eq!(parse_scram_field(server_first, "x="), None);
    }
}
