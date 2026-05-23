// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Square access token / OAuth secret — port of `pkg/detectors/square/`.

use crate::detector::{Detector, VerifyRequest};
use crate::models::{DetectionResult, DetectorType};
use regex::Regex;
use std::sync::OnceLock;

pub struct SquareKey;

static RE: OnceLock<Regex> = OnceLock::new();

fn re() -> &'static Regex {
    // sq0atp- (access token), sq0csp- (client secret), EAAA…SQ (OAuth API key)
    RE.get_or_init(|| {
        Regex::new(r"sq0(?:atp|csp)-[a-zA-Z0-9_-]{22}|EAAA[a-zA-Z0-9_-]{60}").unwrap()
    })
}

impl Detector for SquareKey {
    fn detector_type(&self) -> DetectorType {
        DetectorType::Square
    }
    fn description(&self) -> &'static str {
        "Square access token (sq0atp-), client secret (sq0csp-), OAuth API key (EAAA…)"
    }
    fn keywords(&self) -> &'static [&'static str] {
        &["sq0atp-", "sq0csp-", "EAAA"]
    }
    fn from_data(&self, data: &[u8]) -> Vec<DetectionResult> {
        let Ok(s) = std::str::from_utf8(data) else {
            return Vec::new();
        };
        re()
            .find_iter(s)
            .map(|m| DetectionResult::new(DetectorType::Square, m.as_str()))
            .collect()
    }
    fn build_verify_request(&self, raw: &str) -> Option<VerifyRequest> {
        Some(VerifyRequest {
            method: "GET".into(),
            url: "https://connect.squareup.com/v2/locations".into(),
            headers: vec![("Authorization".into(), format!("Bearer {}", raw))],
            body: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_access_token() {
        let token = format!("sq0atp-{}", "A".repeat(22));
        let r = SquareKey.from_data(token.as_bytes());
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn detects_client_secret() {
        let token = format!("sq0csp-{}", "B".repeat(22));
        let r = SquareKey.from_data(token.as_bytes());
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn detects_eaaa_oauth_key() {
        let token = format!("EAAA{}", "C".repeat(60));
        let r = SquareKey.from_data(token.as_bytes());
        assert_eq!(r.len(), 1);
    }
}
