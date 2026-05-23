// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! JWT — port of `pkg/detectors/jwt/`. Strict three-part shape; lenient
//! verifier (we only ship the regex + no signature check; the engine raises
//! `Indeterminate` for JWTs unless a custom verifier endpoint is configured).

use crate::detector::Detector;
use crate::models::{DetectionResult, DetectorType};
use regex::Regex;
use std::sync::OnceLock;

pub struct JwtToken;

static RE: OnceLock<Regex> = OnceLock::new();

fn re() -> &'static Regex {
    RE.get_or_init(|| {
        Regex::new(r"eyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{20,}").unwrap()
    })
}

impl Detector for JwtToken {
    fn detector_type(&self) -> DetectorType {
        DetectorType::Jwt
    }
    fn description(&self) -> &'static str {
        "JSON Web Token (JWT) — header.payload.signature, base64url-encoded"
    }
    fn keywords(&self) -> &'static [&'static str] {
        &["eyJ"]
    }
    fn from_data(&self, data: &[u8]) -> Vec<DetectionResult> {
        let Ok(s) = std::str::from_utf8(data) else {
            return Vec::new();
        };
        re()
            .find_iter(s)
            .map(|m| DetectionResult::new(DetectorType::Jwt, m.as_str()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_jwt() {
        // header.payload.signature where each segment is base64url
        let token = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NSJ9.SIGNATURE_SIGNATURE_SIGNATURE";
        let r = JwtToken.from_data(token.as_bytes());
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn rejects_two_segment_only() {
        assert!(JwtToken.from_data(b"eyJabc.eyJabc").is_empty());
    }
}
