//! AWS Signature Version 4 — pure, sans-IO computation.
//!
//! Reference: <https://docs.aws.amazon.com/general/latest/gr/sigv4_signing.html>.
//!
//! Steps:
//!
//! 1. Build the **canonical request** = `METHOD\nURI\nQUERY\nHEADERS\n
//!    SIGNED_HEADERS\nPAYLOAD_HASH`.
//! 2. Build the **string to sign** = `AWS4-HMAC-SHA256\nDATE\n
//!    CREDENTIAL_SCOPE\nSHA256(canonical_request)`.
//! 3. Derive the **signing key** by chained HMAC-SHA256:
//!    `kSecret → kDate → kRegion → kService → kSigning`.
//! 4. Compute `signature = hex(HMAC-SHA256(kSigning, string_to_sign))`.
//!
//! All character encoding follows RFC 3986 unreserved-character preservation
//! (S3 special-cases the path: `/` is *not* percent-encoded).

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct SigV4Credentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub region: String,
    pub service: String,
}

#[derive(Debug, Clone)]
pub struct SigV4Request<'a> {
    pub method: &'a str,
    pub uri: &'a str,
    pub query: Vec<(&'a str, &'a str)>,
    pub headers: Vec<(&'a str, &'a str)>,
    pub payload: &'a [u8],
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SigV4Signature {
    pub canonical_request: String,
    pub string_to_sign: String,
    pub signature: String,
    pub authorization_header: String,
}

pub fn sign(creds: &SigV4Credentials, req: &SigV4Request<'_>) -> SigV4Signature {
    let amz_date = req.timestamp.format("%Y%m%dT%H%M%SZ").to_string();
    let date_stamp = req.timestamp.format("%Y%m%d").to_string();
    let credential_scope = format!(
        "{}/{}/{}/aws4_request",
        date_stamp, creds.region, creds.service
    );

    let canonical_uri = canonical_uri(req.uri);
    let canonical_query = canonical_query(&req.query);
    let payload_hash = sha256_hex(req.payload);

    let mut headers = req.headers.to_vec();
    // x-amz-content-sha256 is required for S3 SigV4.
    if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("x-amz-content-sha256")) {
        headers.push(("x-amz-content-sha256", payload_hash_static(req.payload)));
    }
    if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("x-amz-date")) {
        headers.push(("x-amz-date", leak_date(&amz_date)));
    }
    let (canonical_headers, signed_headers) = canonical_headers(&headers);

    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        req.method.to_uppercase(),
        canonical_uri,
        canonical_query,
        canonical_headers,
        signed_headers,
        payload_hash
    );

    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{}\n{}",
        amz_date,
        credential_scope,
        sha256_hex(canonical_request.as_bytes())
    );

    let k_date = hmac_sha256(
        format!("AWS4{}", creds.secret_access_key).as_bytes(),
        date_stamp.as_bytes(),
    );
    let k_region = hmac_sha256(&k_date, creds.region.as_bytes());
    let k_service = hmac_sha256(&k_region, creds.service.as_bytes());
    let k_signing = hmac_sha256(&k_service, b"aws4_request");
    let signature = hex(&hmac_sha256(&k_signing, string_to_sign.as_bytes()));

    let authorization_header = format!(
        "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
        creds.access_key_id, credential_scope, signed_headers, signature
    );

    SigV4Signature {
        canonical_request,
        string_to_sign,
        signature,
        authorization_header,
    }
}

fn canonical_uri(uri: &str) -> String {
    if uri.is_empty() || uri == "/" {
        return "/".to_string();
    }
    // S3 keeps "/" un-encoded; encode each segment separately, then re-join.
    let segments: Vec<String> = uri
        .split('/')
        .map(|s| percent_encode(s, /*encode_slash*/ true))
        .collect();
    segments.join("/")
}

fn canonical_query(query: &[(&str, &str)]) -> String {
    let mut pairs: Vec<(String, String)> = query
        .iter()
        .map(|(k, v)| (percent_encode(k, true), percent_encode(v, true)))
        .collect();
    pairs.sort();
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("&")
}

fn canonical_headers(headers: &[(&str, &str)]) -> (String, String) {
    let mut owned: Vec<(String, String)> = headers
        .iter()
        .map(|(k, v)| (k.to_lowercase(), v.trim().to_string()))
        .collect();
    owned.sort_by(|a, b| a.0.cmp(&b.0));
    let mut canonical = String::new();
    let mut signed = Vec::new();
    for (k, v) in &owned {
        canonical.push_str(k);
        canonical.push(':');
        canonical.push_str(v);
        canonical.push('\n');
        signed.push(k.clone());
    }
    (canonical, signed.join(";"))
}

fn percent_encode(input: &str, encode_slash: bool) -> String {
    let mut out = String::new();
    for ch in input.bytes() {
        let unreserved = ch.is_ascii_alphanumeric()
            || ch == b'-'
            || ch == b'_'
            || ch == b'.'
            || ch == b'~'
            || (ch == b'/' && !encode_slash);
        if unreserved {
            out.push(ch as char);
        } else {
            out.push_str(&format!("%{:02X}", ch));
        }
    }
    out
}

pub fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    hex(&h.finalize())
}

pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{:02x}", b));
    }
    out
}

pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> Vec<u8> {
    const BLOCK: usize = 64;
    let mut k = if key.len() > BLOCK {
        let mut o = vec![0u8; BLOCK];
        let mut h = Sha256::new();
        h.update(key);
        let h = h.finalize();
        o[..32].copy_from_slice(&h);
        o
    } else {
        let mut o = vec![0u8; BLOCK];
        o[..key.len()].copy_from_slice(key);
        o
    };
    let mut o_pad = vec![0x5cu8; BLOCK];
    let mut i_pad = vec![0x36u8; BLOCK];
    for i in 0..BLOCK {
        o_pad[i] ^= k[i];
        i_pad[i] ^= k[i];
    }
    k.fill(0);
    let mut inner = Sha256::new();
    inner.update(&i_pad);
    inner.update(msg);
    let inner_h = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(&o_pad);
    outer.update(inner_h);
    outer.finalize().to_vec()
}

// ── helpers to keep static lifetimes for the headers we inject ──────────────

fn payload_hash_static(payload: &[u8]) -> &'static str {
    if payload.is_empty() {
        // SHA256("") — reused by every empty-body S3 call.
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    } else {
        // Heap-leak the hash — ok for a sigv4 round (one allocation per call).
        Box::leak(sha256_hex(payload).into_boxed_str())
    }
}

fn leak_date(amz_date: &str) -> &'static str {
    Box::leak(amz_date.to_string().into_boxed_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn fixed_time() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2013, 5, 24, 0, 0, 0).unwrap()
    }

    #[test]
    fn sha256_empty_known_answer() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_abc_known_answer() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn hmac_sha256_jefe_known_answer() {
        // RFC 4231 § 4.2.
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(
            hex(&mac),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn percent_encode_alpha_unchanged() {
        assert_eq!(percent_encode("hello", true), "hello");
    }

    #[test]
    fn percent_encode_space_to_hex() {
        assert_eq!(percent_encode("a b", true), "a%20b");
    }

    #[test]
    fn percent_encode_slash_when_encoded() {
        assert_eq!(percent_encode("a/b", true), "a%2Fb");
    }

    #[test]
    fn percent_encode_slash_when_not_encoded() {
        assert_eq!(percent_encode("a/b", false), "a/b");
    }

    #[test]
    fn canonical_uri_root() {
        assert_eq!(canonical_uri(""), "/");
        assert_eq!(canonical_uri("/"), "/");
    }

    #[test]
    fn canonical_uri_keeps_slash() {
        assert_eq!(canonical_uri("/path/to/object"), "/path/to/object");
    }

    #[test]
    fn canonical_uri_encodes_special() {
        assert_eq!(canonical_uri("/with space/x"), "/with%20space/x");
    }

    #[test]
    fn canonical_query_sorts_lexicographically() {
        let q = canonical_query(&[("b", "2"), ("a", "1")]);
        assert_eq!(q, "a=1&b=2");
    }

    #[test]
    fn canonical_query_percent_encodes() {
        let q = canonical_query(&[("name", "a b")]);
        assert_eq!(q, "name=a%20b");
    }

    #[test]
    fn canonical_headers_lowercased_sorted() {
        let (canonical, signed) = canonical_headers(&[
            ("Host", "example.com"),
            ("X-Amz-Date", "20130524T000000Z"),
        ]);
        assert!(canonical.contains("host:example.com\n"));
        assert!(canonical.contains("x-amz-date:20130524T000000Z\n"));
        assert_eq!(signed, "host;x-amz-date");
    }

    #[test]
    fn sign_returns_authorization_header() {
        let creds = SigV4Credentials {
            access_key_id: "AKIDEXAMPLE".into(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
            region: "us-east-1".into(),
            service: "s3".into(),
        };
        let req = SigV4Request {
            method: "GET",
            uri: "/",
            query: vec![],
            headers: vec![("host", "examplebucket.s3.amazonaws.com")],
            payload: b"",
            timestamp: fixed_time(),
        };
        let sig = sign(&creds, &req);
        assert!(sig
            .authorization_header
            .starts_with("AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20130524/us-east-1/s3/aws4_request"));
        assert!(sig.authorization_header.contains("SignedHeaders=host"));
        assert_eq!(sig.signature.len(), 64);
    }

    #[test]
    fn sign_deterministic_for_fixed_input() {
        let creds = SigV4Credentials {
            access_key_id: "AKIDEXAMPLE".into(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
            region: "us-east-1".into(),
            service: "s3".into(),
        };
        let req = SigV4Request {
            method: "GET",
            uri: "/",
            query: vec![],
            headers: vec![("host", "examplebucket.s3.amazonaws.com")],
            payload: b"",
            timestamp: fixed_time(),
        };
        let s1 = sign(&creds, &req);
        let s2 = sign(&creds, &req);
        assert_eq!(s1.signature, s2.signature);
    }

    #[test]
    fn sign_changes_when_payload_changes() {
        let creds = SigV4Credentials {
            access_key_id: "AKIDEXAMPLE".into(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
            region: "us-east-1".into(),
            service: "s3".into(),
        };
        let req1 = SigV4Request {
            method: "PUT",
            uri: "/k",
            query: vec![],
            headers: vec![("host", "b.s3.amazonaws.com")],
            payload: b"a",
            timestamp: fixed_time(),
        };
        let req2 = SigV4Request {
            method: "PUT",
            uri: "/k",
            query: vec![],
            headers: vec![("host", "b.s3.amazonaws.com")],
            payload: b"b",
            timestamp: fixed_time(),
        };
        let s1 = sign(&creds, &req1);
        let s2 = sign(&creds, &req2);
        assert_ne!(s1.signature, s2.signature);
    }

    #[test]
    fn sign_includes_credential_scope() {
        let creds = SigV4Credentials {
            access_key_id: "K".into(),
            secret_access_key: "S".into(),
            region: "eu-west-1".into(),
            service: "s3".into(),
        };
        let req = SigV4Request {
            method: "GET",
            uri: "/",
            query: vec![],
            headers: vec![("host", "x")],
            payload: b"",
            timestamp: fixed_time(),
        };
        let sig = sign(&creds, &req);
        assert!(sig.string_to_sign.contains("20130524/eu-west-1/s3/aws4_request"));
    }

    #[test]
    fn canonical_request_payload_hash_appended_last() {
        let creds = SigV4Credentials {
            access_key_id: "K".into(),
            secret_access_key: "S".into(),
            region: "us-east-1".into(),
            service: "s3".into(),
        };
        let req = SigV4Request {
            method: "GET",
            uri: "/",
            query: vec![],
            headers: vec![("host", "x")],
            payload: b"",
            timestamp: fixed_time(),
        };
        let sig = sign(&creds, &req);
        assert!(sig.canonical_request.ends_with(
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        ));
    }
}
