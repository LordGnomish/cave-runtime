// SPDX-License-Identifier: AGPL-3.0-or-later
use chrono::{DateTime, Duration, Utc};
use ring::hmac::{self, Key, HMAC_SHA256};

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

pub struct PresignedUrl {
    pub url: String,
    pub expires_at: DateTime<Utc>,
    pub method: String,
}

pub struct PresignConfig {
    pub access_key: String,
    pub secret_key: Vec<u8>,
    pub region: String,
    pub endpoint: String,
}

impl PresignConfig {
    pub fn new(
        access_key: &str,
        secret_key: &[u8],
        region: &str,
        endpoint: &str,
    ) -> Self {
        Self {
            access_key: access_key.to_string(),
            secret_key: secret_key.to_vec(),
            region: region.to_string(),
            endpoint: endpoint.to_string(),
        }
    }

    fn sign(&self, message: &str) -> String {
        let key = Key::new(HMAC_SHA256, &self.secret_key);
        let sig = hmac::sign(&key, message.as_bytes());
        to_hex(sig.as_ref())
    }

    pub fn presign_get(&self, bucket: &str, key: &str, expires_in: Duration) -> PresignedUrl {
        let expires_at = Utc::now() + expires_in;
        let expires_ts = expires_at.timestamp();
        let message = format!(
            "GET\n{}\n{}\n{}\n{}",
            bucket, key, expires_ts, self.access_key
        );
        let signature = self.sign(&message);
        let url = format!(
            "{}/{}/{}?X-Access-Key={}&X-Expires={}&X-Signature={}",
            self.endpoint, bucket, key, self.access_key, expires_ts, signature
        );
        PresignedUrl {
            url,
            expires_at,
            method: "GET".to_string(),
        }
    }

    pub fn presign_put(&self, bucket: &str, key: &str, expires_in: Duration) -> PresignedUrl {
        let expires_at = Utc::now() + expires_in;
        let expires_ts = expires_at.timestamp();
        let message = format!(
            "PUT\n{}\n{}\n{}\n{}",
            bucket, key, expires_ts, self.access_key
        );
        let signature = self.sign(&message);
        let url = format!(
            "{}/{}/{}?X-Access-Key={}&X-Expires={}&X-Signature={}",
            self.endpoint, bucket, key, self.access_key, expires_ts, signature
        );
        PresignedUrl {
            url,
            expires_at,
            method: "PUT".to_string(),
        }
    }

    /// Verify a presigned URL is still valid and has correct signature.
    pub fn verify(&self, url: &str) -> bool {
        // Parse URL parameters
        let query_start = match url.find('?') {
            Some(pos) => pos,
            None => return false,
        };
        let path = &url[..query_start];
        let query = &url[query_start + 1..];

        // Parse query params
        let mut access_key = None;
        let mut expires_ts: Option<i64> = None;
        let mut signature = None;

        for param in query.split('&') {
            if let Some(val) = param.strip_prefix("X-Access-Key=") {
                access_key = Some(val);
            } else if let Some(val) = param.strip_prefix("X-Expires=") {
                expires_ts = val.parse().ok();
            } else if let Some(val) = param.strip_prefix("X-Signature=") {
                signature = Some(val);
            }
        }

        let (access_key, expires_ts, signature) = match (access_key, expires_ts, signature) {
            (Some(a), Some(e), Some(s)) => (a, e, s),
            _ => return false,
        };

        // Check expiry
        if expires_ts < Utc::now().timestamp() {
            return false;
        }

        // Re-derive bucket/key from path: endpoint/bucket/key
        // Remove endpoint prefix
        let path_without_endpoint = if let Some(stripped) = path.strip_prefix(&self.endpoint) {
            stripped.trim_start_matches('/')
        } else {
            path.trim_start_matches('/')
        };

        let mut parts = path_without_endpoint.splitn(2, '/');
        let bucket = parts.next().unwrap_or("");
        let key = parts.next().unwrap_or("");

        // Determine method from signature presence (we try both)
        for method in &["GET", "PUT"] {
            let message = format!(
                "{}\n{}\n{}\n{}\n{}",
                method, bucket, key, expires_ts, access_key
            );
            let expected_sig = self.sign(&message);
            if expected_sig == signature {
                return true;
            }
        }
        false
    }
}
