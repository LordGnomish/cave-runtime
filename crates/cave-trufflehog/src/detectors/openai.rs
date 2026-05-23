// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! OpenAI API key — port of `pkg/detectors/openai/`. Includes the legacy
//! `sk-…48` and the project-key `sk-proj-…` formats.

use crate::detector::{Detector, VerifyRequest};
use crate::models::{DetectionResult, DetectorType};
use regex::Regex;
use std::sync::OnceLock;

pub struct OpenaiKey;

static RE: OnceLock<Regex> = OnceLock::new();

fn re() -> &'static Regex {
    RE.get_or_init(|| {
        Regex::new(r"sk-(?:proj-[a-zA-Z0-9_-]{40,200}|[a-zA-Z0-9]{48})").unwrap()
    })
}

impl Detector for OpenaiKey {
    fn detector_type(&self) -> DetectorType {
        DetectorType::Openai
    }
    fn description(&self) -> &'static str {
        "OpenAI API key (sk-… 48-char legacy, or sk-proj-… project key)"
    }
    fn keywords(&self) -> &'static [&'static str] {
        &["sk-"]
    }
    fn from_data(&self, data: &[u8]) -> Vec<DetectionResult> {
        let Ok(s) = std::str::from_utf8(data) else {
            return Vec::new();
        };
        re()
            .find_iter(s)
            .filter(|m| !m.as_str().starts_with("sk-ant-"))
            .map(|m| DetectionResult::new(DetectorType::Openai, m.as_str()))
            .collect()
    }
    fn build_verify_request(&self, raw: &str) -> Option<VerifyRequest> {
        Some(VerifyRequest {
            method: "GET".into(),
            url: "https://api.openai.com/v1/models".into(),
            headers: vec![("Authorization".into(), format!("Bearer {}", raw))],
            body: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_legacy_key() {
        let token = format!("sk-{}", "a".repeat(48));
        let r = OpenaiKey.from_data(token.as_bytes());
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn detects_project_key() {
        let token = format!("sk-proj-{}", "a".repeat(60));
        let r = OpenaiKey.from_data(token.as_bytes());
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn does_not_steal_anthropic() {
        let token = format!("sk-ant-{}", "a".repeat(60));
        let r = OpenaiKey.from_data(token.as_bytes());
        assert!(r.is_empty());
    }
}
