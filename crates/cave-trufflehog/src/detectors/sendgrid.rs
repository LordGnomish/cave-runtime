// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! SendGrid API key — port of `pkg/detectors/sendgrid/`.

use crate::detector::{Detector, VerifyRequest};
use crate::models::{DetectionResult, DetectorType};
use regex::Regex;
use std::sync::OnceLock;

pub struct SendgridKey;

static RE: OnceLock<Regex> = OnceLock::new();

fn re() -> &'static Regex {
    RE.get_or_init(|| Regex::new(r"SG\.[a-zA-Z0-9_-]{20,28}\.[a-zA-Z0-9_-]{40,50}").unwrap())
}

impl Detector for SendgridKey {
    fn detector_type(&self) -> DetectorType {
        DetectorType::Sendgrid
    }
    fn description(&self) -> &'static str {
        "SendGrid API key (SG.<id>.<secret>)"
    }
    fn keywords(&self) -> &'static [&'static str] {
        &["SG."]
    }
    fn from_data(&self, data: &[u8]) -> Vec<DetectionResult> {
        let Ok(s) = std::str::from_utf8(data) else {
            return Vec::new();
        };
        re()
            .find_iter(s)
            .map(|m| DetectionResult::new(DetectorType::Sendgrid, m.as_str()))
            .collect()
    }
    fn build_verify_request(&self, raw: &str) -> Option<VerifyRequest> {
        Some(VerifyRequest {
            method: "GET".into(),
            url: "https://api.sendgrid.com/v3/user/profile".into(),
            headers: vec![("Authorization".into(), format!("Bearer {}", raw))],
            body: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_key() {
        let token = format!("SG.{}.{}", "a".repeat(22), "b".repeat(43));
        let r = SendgridKey.from_data(token.as_bytes());
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn rejects_truncated() {
        assert!(SendgridKey.from_data(b"SG.short.short").is_empty());
    }
}
