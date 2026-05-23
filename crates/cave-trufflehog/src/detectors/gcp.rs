// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! GCP service-account JSON key — port of `pkg/detectors/gcp/`. Detects
//! the canonical service-account credentials.json `private_key` field.

use crate::detector::Detector;
use crate::models::{DetectionResult, DetectorType};
use regex::Regex;
use std::sync::OnceLock;

pub struct GcpServiceAccount;

static RE: OnceLock<Regex> = OnceLock::new();

fn re() -> &'static Regex {
    RE.get_or_init(|| {
        Regex::new(
            r#""type"\s*:\s*"service_account"[\s\S]+?"private_key_id"\s*:\s*"([a-f0-9]{40})""#,
        )
        .unwrap()
    })
}

impl Detector for GcpServiceAccount {
    fn detector_type(&self) -> DetectorType {
        DetectorType::Gcp
    }
    fn description(&self) -> &'static str {
        "GCP service-account JSON key with type=service_account + private_key_id"
    }
    fn keywords(&self) -> &'static [&'static str] {
        &["service_account"]
    }
    fn from_data(&self, data: &[u8]) -> Vec<DetectionResult> {
        let Ok(s) = std::str::from_utf8(data) else {
            return Vec::new();
        };
        re()
            .captures_iter(s)
            .map(|c| {
                let id = c.get(1).unwrap().as_str();
                DetectionResult::new(DetectorType::Gcp, id)
                    .with_extra("artifact", "service_account_json")
                    .with_extra("private_key_id", id)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_service_account_json() {
        let s = br#"{"type":"service_account","project_id":"x","private_key_id":"abcdef0123456789abcdef0123456789abcdef01","private_key":"-----BEGIN PRIVATE KEY-----"}"#;
        let r = GcpServiceAccount.from_data(s);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].raw.len(), 40);
    }

    #[test]
    fn rejects_user_credentials() {
        let s = br#"{"type":"authorized_user","client_id":"x"}"#;
        assert!(GcpServiceAccount.from_data(s).is_empty());
    }
}
