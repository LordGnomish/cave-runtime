// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! PyPI upload token — port of `pkg/detectors/pypiuploadtoken/`.

use crate::detector::{Detector, VerifyRequest};
use crate::models::{DetectionResult, DetectorType};
use regex::Regex;
use std::sync::OnceLock;

pub struct PypiToken;

static RE: OnceLock<Regex> = OnceLock::new();

fn re() -> &'static Regex {
    RE.get_or_init(|| Regex::new(r"pypi-AgEIcHlwaS5vcmcC[a-zA-Z0-9_-]{50,180}").unwrap())
}

impl Detector for PypiToken {
    fn detector_type(&self) -> DetectorType {
        DetectorType::PypiUpload
    }
    fn description(&self) -> &'static str {
        "PyPI upload token (pypi-AgEIcHlwaS5vcmcC…)"
    }
    fn keywords(&self) -> &'static [&'static str] {
        &["pypi-"]
    }
    fn from_data(&self, data: &[u8]) -> Vec<DetectionResult> {
        let Ok(s) = std::str::from_utf8(data) else {
            return Vec::new();
        };
        re()
            .find_iter(s)
            .map(|m| DetectionResult::new(DetectorType::PypiUpload, m.as_str()))
            .collect()
    }
    fn build_verify_request(&self, raw: &str) -> Option<VerifyRequest> {
        Some(VerifyRequest {
            method: "GET".into(),
            url: "https://upload.pypi.org/legacy/".into(),
            headers: vec![(
                "Authorization".into(),
                format!(
                    "Basic {}",
                    base64::engine::Engine::encode(
                        &base64::engine::general_purpose::STANDARD,
                        format!("__token__:{}", raw).as_bytes(),
                    )
                ),
            )],
            body: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_token() {
        let token = format!("pypi-AgEIcHlwaS5vcmcC{}", "a".repeat(80));
        let r = PypiToken.from_data(token.as_bytes());
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn rejects_wrong_prefix() {
        assert!(PypiToken.from_data(b"pypi-someotherthing").is_empty());
    }
}
