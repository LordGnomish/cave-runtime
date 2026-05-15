// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Presigned URL generation and verification.
//!
//! Uses HMAC-SHA256 to sign URL parameters. Compatible with AWS SigV4 presigned
//! URL structure but with a simplified signing scheme for the cave-store context.

use crate::error::{StoreError, StoreResult};
use base64::Engine;
use chrono::Utc;
use ring::hmac;

pub struct PresignedUrlParams {
    pub bucket: String,
    pub key: String,
    pub method: String, // GET, PUT, DELETE, HEAD
    pub expires_in_secs: u64,
    pub access_key: String,
    pub extra_headers: std::collections::HashMap<String, String>,
}

pub struct PresignedUrl {
    pub url: String,
    pub expires_at: i64,
}

/// Generate a presigned URL.
pub fn generate(
    base_url: &str,
    params: &PresignedUrlParams,
    secret_key: &[u8],
) -> PresignedUrl {
    let now = Utc::now().timestamp();
    let expires_at = now + params.expires_in_secs as i64;

    // Canonical string to sign: METHOD\nbucket\nkey\nexpires\naccess_key
    let canonical = format!(
        "{}\n{}\n{}\n{}\n{}",
        params.method, params.bucket, params.key, expires_at, params.access_key
    );
    let signing_key = hmac::Key::new(hmac::HMAC_SHA256, secret_key);
    let signature = hmac::sign(&signing_key, canonical.as_bytes());
    let sig_hex = hex::encode(signature.as_ref());

    let url = format!(
        "{base_url}/{bucket}/{key}?X-Cave-Algorithm=CAVE1-HMAC-SHA256&X-Cave-Credential={ak}&X-Cave-Expires={exp}&X-Cave-Signature={sig}",
        base_url = base_url,
        bucket = url_encode(&params.bucket),
        key = url_encode(&params.key),
        ak = url_encode(&params.access_key),
        exp = expires_at,
        sig = sig_hex,
    );

    PresignedUrl { url, expires_at }
}

/// Verify a presigned URL is valid and not expired.
pub fn verify(
    method: &str,
    bucket: &str,
    key: &str,
    access_key: &str,
    expires_at: i64,
    signature: &str,
    secret_key: &[u8],
) -> StoreResult<()> {
    let now = Utc::now().timestamp();
    if now > expires_at {
        return Err(StoreError::RequestExpired);
    }

    let canonical = format!("{}\n{}\n{}\n{}\n{}", method, bucket, key, expires_at, access_key);
    let signing_key = hmac::Key::new(hmac::HMAC_SHA256, secret_key);
    let expected = hmac::sign(&signing_key, canonical.as_bytes());
    let expected_hex = hex::encode(expected.as_ref());

    if expected_hex != signature {
        return Err(StoreError::SignatureMismatch);
    }
    Ok(())
}

fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b'/' => out.push('/'),
            _ => {
                out.push('%');
                out.push_str(&format!("{:02X}", b));
            }
        }
    }
    out
}
