// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! npm access token — port of `pkg/detectors/npmtoken_v2/`.

use crate::detector::{Detector, VerifyRequest};
use crate::models::{DetectionResult, DetectorType};
use regex::Regex;
use std::sync::OnceLock;

pub struct NpmToken;

static RE: OnceLock<Regex> = OnceLock::new();

fn re() -> &'static Regex {
    RE.get_or_init(|| Regex::new(r"npm_[A-Za-z0-9]{36}").unwrap())
}

impl Detector for NpmToken {
    fn detector_type(&self) -> DetectorType {
        DetectorType::NpmToken
    }
    fn description(&self) -> &'static str {
        "npm access token (npm_<36-char>)"
    }
    fn keywords(&self) -> &'static [&'static str] {
        &["npm_"]
    }
    fn from_data(&self, data: &[u8]) -> Vec<DetectionResult> {
        let Ok(s) = std::str::from_utf8(data) else {
            return Vec::new();
        };
        re()
            .find_iter(s)
            .map(|m| DetectionResult::new(DetectorType::NpmToken, m.as_str()))
            .collect()
    }
    fn build_verify_request(&self, raw: &str) -> Option<VerifyRequest> {
        Some(VerifyRequest {
            method: "GET".into(),
            url: "https://registry.npmjs.org/-/whoami".into(),
            headers: vec![("Authorization".into(), format!("Bearer {}", raw))],
            body: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_token() {
        let token = format!("npm_{}", "a".repeat(36));
        let r = NpmToken.from_data(token.as_bytes());
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn rejects_short_token() {
        assert!(NpmToken.from_data(b"npm_short").is_empty());
    }
}
