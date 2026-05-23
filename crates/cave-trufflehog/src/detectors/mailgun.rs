// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Mailgun API key — port of `pkg/detectors/mailgun/`.

use crate::detector::{Detector, VerifyRequest};
use crate::models::{DetectionResult, DetectorType};
use regex::Regex;
use std::sync::OnceLock;

pub struct MailgunKey;

static RE: OnceLock<Regex> = OnceLock::new();

fn re() -> &'static Regex {
    // key-<32-hex>  (legacy)  OR  <32-hex>-<8-hex>-<8-hex>  (private API key)
    RE.get_or_init(|| {
        Regex::new(r"key-[a-f0-9]{32}|[a-f0-9]{32}-[a-f0-9]{8}-[a-f0-9]{8}").unwrap()
    })
}

impl Detector for MailgunKey {
    fn detector_type(&self) -> DetectorType {
        DetectorType::Mailgun
    }
    fn description(&self) -> &'static str {
        "Mailgun legacy API key (key-…) or private API key (xxxxxxxx-xxxxxxxx-xxxxxxxx)"
    }
    fn keywords(&self) -> &'static [&'static str] {
        &["key-", "mailgun"]
    }
    fn from_data(&self, data: &[u8]) -> Vec<DetectionResult> {
        let Ok(s) = std::str::from_utf8(data) else {
            return Vec::new();
        };
        re()
            .find_iter(s)
            .map(|m| DetectionResult::new(DetectorType::Mailgun, m.as_str()))
            .collect()
    }
    fn build_verify_request(&self, raw: &str) -> Option<VerifyRequest> {
        Some(VerifyRequest {
            method: "GET".into(),
            url: "https://api.mailgun.net/v3/domains".into(),
            headers: vec![(
                "Authorization".into(),
                format!("Basic {}", base64_encode(format!("api:{}", raw).as_bytes())),
            )],
            body: None,
        })
    }
}

fn base64_encode(b: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy_fixture() -> String {
        // Constructed at runtime so source-scanning tools don't flag the
        // literal — value is obviously non-real (all `a`).
        format!("key-{}", "a".repeat(32))
    }

    fn private_fixture() -> String {
        format!("{}-{}-{}", "a".repeat(32), "a".repeat(8), "a".repeat(8))
    }

    #[test]
    fn detects_legacy() {
        let s = legacy_fixture();
        let r = MailgunKey.from_data(s.as_bytes());
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn detects_private_key_format() {
        let s = private_fixture();
        let r = MailgunKey.from_data(s.as_bytes());
        assert_eq!(r.len(), 1);
    }
}
