//! S3 presigned URL — query-string SigV4 form.
//!
//! Reference: <https://docs.aws.amazon.com/AmazonS3/latest/userguide/RESTAuthentication.html>.
//!
//! The request encodes the auth metadata in the URL itself:
//!
//! ```text
//! ?X-Amz-Algorithm=AWS4-HMAC-SHA256
//! &X-Amz-Credential=KEY/DATE/REGION/SERVICE/aws4_request
//! &X-Amz-Date=YYYYMMDDTHHMMSSZ
//! &X-Amz-Expires=N
//! &X-Amz-SignedHeaders=...
//! &X-Amz-Signature=...
//! ```

use chrono::{DateTime, Duration, Utc};

use super::sigv4::{SigV4Credentials, SigV4Request, sign};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresignedUrl {
    pub url: String,
    pub expires_at: DateTime<Utc>,
}

/// Build a presigned URL for `GET /bucket/key` valid for `expires_in`. The
/// signature covers the host header and the X-Amz-* query parameters.
pub fn presign_get(
    creds: &SigV4Credentials,
    host: &str,
    bucket: &str,
    key: &str,
    timestamp: DateTime<Utc>,
    expires_in: Duration,
) -> PresignedUrl {
    let amz_date = timestamp.format("%Y%m%dT%H%M%SZ").to_string();
    let date_stamp = timestamp.format("%Y%m%d").to_string();
    let credential = format!(
        "{}/{}/{}/{}/aws4_request",
        creds.access_key_id, date_stamp, creds.region, creds.service
    );
    let expires_str = expires_in.num_seconds().to_string();

    let mut query: Vec<(String, String)> = vec![
        ("X-Amz-Algorithm".into(), "AWS4-HMAC-SHA256".into()),
        ("X-Amz-Credential".into(), credential.clone()),
        ("X-Amz-Date".into(), amz_date.clone()),
        ("X-Amz-Expires".into(), expires_str.clone()),
        ("X-Amz-SignedHeaders".into(), "host".into()),
    ];
    let q_refs: Vec<(&str, &str)> = query.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let uri = format!("/{}/{}", bucket, key);
    let req = SigV4Request {
        method: "GET",
        uri: &uri,
        query: q_refs,
        headers: vec![("host", host)],
        payload: b"",
        timestamp,
    };
    let sig = sign(creds, &req);
    query.push(("X-Amz-Signature".into(), sig.signature.clone()));
    let qs: Vec<String> = query
        .iter()
        .map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v)))
        .collect();
    let url = format!("https://{}{}?{}", host, uri, qs.join("&"));
    PresignedUrl {
        url,
        expires_at: timestamp + expires_in,
    }
}

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        let unreserved = b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~';
        if unreserved {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn creds() -> SigV4Credentials {
        SigV4Credentials {
            access_key_id: "AKID".into(),
            secret_access_key: "SECRET".into(),
            region: "us-east-1".into(),
            service: "s3".into(),
        }
    }

    fn fixed_time() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2025, 1, 1, 12, 0, 0).unwrap()
    }

    #[test]
    fn presign_get_url_contains_amz_params() {
        let url = presign_get(&creds(), "b.s3.amazonaws.com", "b", "k", fixed_time(), Duration::hours(1));
        assert!(url.url.contains("X-Amz-Algorithm=AWS4-HMAC-SHA256"));
        assert!(url.url.contains("X-Amz-Credential=AKID%2F20250101"));
        assert!(url.url.contains("X-Amz-Date=20250101T120000Z"));
        assert!(url.url.contains("X-Amz-Expires=3600"));
        assert!(url.url.contains("X-Amz-Signature="));
    }

    #[test]
    fn presign_expires_at_is_now_plus_duration() {
        let url = presign_get(&creds(), "h", "b", "k", fixed_time(), Duration::seconds(900));
        assert_eq!(url.expires_at, fixed_time() + Duration::seconds(900));
    }

    #[test]
    fn presign_signature_changes_with_key() {
        let u1 = presign_get(&creds(), "h", "b", "k1", fixed_time(), Duration::hours(1));
        let u2 = presign_get(&creds(), "h", "b", "k2", fixed_time(), Duration::hours(1));
        assert_ne!(u1.url, u2.url);
    }

    #[test]
    fn presign_signature_changes_with_credential() {
        let mut c2 = creds();
        c2.secret_access_key = "OTHER".into();
        let u1 = presign_get(&creds(), "h", "b", "k", fixed_time(), Duration::hours(1));
        let u2 = presign_get(&c2, "h", "b", "k", fixed_time(), Duration::hours(1));
        assert_ne!(u1.url, u2.url);
    }

    #[test]
    fn presign_url_starts_with_https_host() {
        let url = presign_get(&creds(), "examplebucket.s3.amazonaws.com", "b", "k", fixed_time(), Duration::hours(1));
        assert!(url.url.starts_with("https://examplebucket.s3.amazonaws.com/"));
    }
}
