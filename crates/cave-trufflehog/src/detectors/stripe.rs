// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Stripe live key — port of `pkg/detectors/stripe/stripe.go`. Excludes
//! `sk_test_` keys (test mode); verification against `api.stripe.com/v1/charges`.

use crate::detector::{Detector, VerifyRequest};
use crate::models::{DetectionResult, DetectorType};
use regex::Regex;
use std::sync::OnceLock;

pub struct StripeKey;

static RE: OnceLock<Regex> = OnceLock::new();

fn re() -> &'static Regex {
    RE.get_or_init(|| Regex::new(r"[rs]k_live_[a-zA-Z0-9]{20,247}").unwrap())
}

impl Detector for StripeKey {
    fn detector_type(&self) -> DetectorType {
        DetectorType::Stripe
    }
    fn description(&self) -> &'static str {
        "Stripe live secret/restricted API key (sk_live / rk_live)"
    }
    fn keywords(&self) -> &'static [&'static str] {
        &["k_live_"]
    }
    fn from_data(&self, data: &[u8]) -> Vec<DetectionResult> {
        let Ok(s) = std::str::from_utf8(data) else {
            return Vec::new();
        };
        re()
            .find_iter(s)
            .map(|m| {
                DetectionResult::new(DetectorType::Stripe, m.as_str()).with_extra(
                    "rotation_guide",
                    "https://howtorotate.com/docs/tutorials/stripe/",
                )
            })
            .collect()
    }
    fn build_verify_request(&self, raw: &str) -> Option<VerifyRequest> {
        Some(VerifyRequest {
            method: "GET".into(),
            url: "https://api.stripe.com/v1/charges".into(),
            headers: vec![
                ("Authorization".into(), format!("Bearer {}", raw)),
                ("Content-Type".into(), "application/json".into()),
            ],
            body: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_live_key() {
        let r = StripeKey.from_data(b"key=sk_live_1234567890abcdefghij");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].detector_type, DetectorType::Stripe);
    }

    #[test]
    fn detects_restricted_key() {
        let r = StripeKey.from_data(b"rk_live_1234567890abcdefghij");
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn skips_test_key() {
        let r = StripeKey.from_data(b"sk_test_1234567890abcdefghij");
        assert!(r.is_empty());
    }

    #[test]
    fn verify_request_includes_auth_header() {
        let req = StripeKey.build_verify_request("sk_live_x").unwrap();
        assert_eq!(req.url, "https://api.stripe.com/v1/charges");
        assert!(
            req.headers
                .iter()
                .any(|(k, v)| k == "Authorization" && v.contains("sk_live_x"))
        );
    }
}
