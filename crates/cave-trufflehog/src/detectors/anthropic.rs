// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Anthropic API key — port of `pkg/detectors/anthropic/`.

use crate::detector::{Detector, VerifyRequest};
use crate::models::{DetectionResult, DetectorType};
use regex::Regex;
use std::sync::OnceLock;

pub struct AnthropicKey;

static RE: OnceLock<Regex> = OnceLock::new();

fn re() -> &'static Regex {
    RE.get_or_init(|| Regex::new(r"sk-ant-[a-zA-Z0-9_-]{60,150}").unwrap())
}

impl Detector for AnthropicKey {
    fn detector_type(&self) -> DetectorType {
        DetectorType::Anthropic
    }
    fn description(&self) -> &'static str {
        "Anthropic Claude API key (sk-ant-…)"
    }
    fn keywords(&self) -> &'static [&'static str] {
        &["sk-ant-"]
    }
    fn from_data(&self, data: &[u8]) -> Vec<DetectionResult> {
        let Ok(s) = std::str::from_utf8(data) else {
            return Vec::new();
        };
        re()
            .find_iter(s)
            .map(|m| DetectionResult::new(DetectorType::Anthropic, m.as_str()))
            .collect()
    }
    fn build_verify_request(&self, raw: &str) -> Option<VerifyRequest> {
        Some(VerifyRequest {
            method: "POST".into(),
            url: "https://api.anthropic.com/v1/messages".into(),
            headers: vec![
                ("x-api-key".into(), raw.into()),
                ("anthropic-version".into(), "2023-06-01".into()),
                ("content-type".into(), "application/json".into()),
            ],
            body: Some(
                br#"{"model":"claude-3-haiku-20240307","max_tokens":1,"messages":[{"role":"user","content":"ping"}]}"#
                    .to_vec(),
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_key() {
        let token = format!("sk-ant-{}", "a".repeat(60));
        let r = AnthropicKey.from_data(token.as_bytes());
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn rejects_too_short() {
        assert!(AnthropicKey.from_data(b"sk-ant-short").is_empty());
    }

    #[test]
    fn verify_request_uses_api_key_header() {
        let req = AnthropicKey.build_verify_request("sk-ant-x").unwrap();
        assert!(req.headers.iter().any(|(k, _)| k == "x-api-key"));
    }
}
