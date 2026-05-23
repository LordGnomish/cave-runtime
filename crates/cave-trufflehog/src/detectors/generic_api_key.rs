// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Generic high-entropy api/secret/token detector — port of
//! `pkg/detectors/generic/`. Combines a contextual keyword pre-filter
//! (`api_key`, `secret`, `password`, `token`) with a shannon-entropy floor.

use crate::custom_detectors::shannon_entropy;
use crate::detector::Detector;
use crate::models::{DetectionResult, DetectorType};
use regex::Regex;
use std::sync::OnceLock;

pub struct GenericApiKey;

const MIN_ENTROPY: f64 = 4.0;

static RE: OnceLock<Regex> = OnceLock::new();

fn re() -> &'static Regex {
    RE.get_or_init(|| {
        Regex::new(
            r#"(?i)(?:api[_-]?key|secret|password|token|auth)["'= :]+([A-Za-z0-9+/=_-]{20,80})"#,
        )
        .unwrap()
    })
}

impl Detector for GenericApiKey {
    fn detector_type(&self) -> DetectorType {
        DetectorType::Generic
    }
    fn description(&self) -> &'static str {
        "Generic high-entropy api_key / secret / password / token / auth value"
    }
    fn keywords(&self) -> &'static [&'static str] {
        &["api_key", "secret", "password", "token", "auth"]
    }
    fn from_data(&self, data: &[u8]) -> Vec<DetectionResult> {
        let Ok(s) = std::str::from_utf8(data) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for cap in re().captures_iter(s) {
            let raw = cap.get(1).unwrap().as_str();
            if shannon_entropy(raw) < MIN_ENTROPY {
                continue;
            }
            out.push(
                DetectionResult::new(DetectorType::Generic, raw)
                    .with_extra("source", "generic-high-entropy"),
            );
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_high_entropy_value() {
        let r = GenericApiKey.from_data(b"api_key=\"AbCdEf1234567890QwErTy!@xyzMNOP\"");
        assert!(r.len() >= 1);
    }

    #[test]
    fn skips_low_entropy_value() {
        let r = GenericApiKey.from_data(b"password=\"aaaaaaaaaaaaaaaaaaaa\"");
        assert!(r.is_empty());
    }

    #[test]
    fn skips_without_keyword() {
        let r = GenericApiKey.from_data(b"foo=AbCdEf1234567890QwErTy!@xyzMNOP");
        assert!(r.is_empty());
    }

    #[test]
    fn detects_token_key_form() {
        let r = GenericApiKey.from_data(b"TOKEN: AbCdEf1234567890QwErTy!@xyzMNOP");
        assert!(r.len() >= 1);
    }
}
