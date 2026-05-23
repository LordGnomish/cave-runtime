// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! AWS access key — port of `pkg/detectors/aws/access_keys/accesskey.go`.
//! Detects `AKIA`/`ABIA`/`ACCA` prefixes, with optional 40-char base64-ish
//! secret captured nearby (multi-part credential).

use crate::detector::Detector;
use crate::models::{DetectionResult, DetectorType};
use regex::Regex;
use std::sync::OnceLock;

pub struct AwsAccessKey;

static ID_RE: OnceLock<Regex> = OnceLock::new();
static SECRET_RE: OnceLock<Regex> = OnceLock::new();

fn id_re() -> &'static Regex {
    ID_RE.get_or_init(|| Regex::new(r"\b((?:AKIA|ABIA|ACCA)[A-Z0-9]{16})\b").unwrap())
}

fn secret_re() -> &'static Regex {
    SECRET_RE.get_or_init(|| Regex::new(r"[A-Za-z0-9/+=]{40}").unwrap())
}

impl Detector for AwsAccessKey {
    fn detector_type(&self) -> DetectorType {
        DetectorType::Aws
    }
    fn description(&self) -> &'static str {
        "AWS access key ID (AKIA / ABIA / ACCA) — paired with a 40-char secret when present"
    }
    fn keywords(&self) -> &'static [&'static str] {
        &["AKIA", "ABIA", "ACCA"]
    }
    fn from_data(&self, data: &[u8]) -> Vec<DetectionResult> {
        let Ok(s) = std::str::from_utf8(data) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for id_m in id_re().find_iter(s) {
            let id = id_m.as_str();
            let mut r = DetectionResult::new(DetectorType::Aws, id).with_extra(
                "rotation_guide",
                "https://howtorotate.com/docs/tutorials/aws/",
            );
            r.secret_parts.insert("access_key_id".into(), id.into());
            // Multi-part credential — look ahead 0..512 bytes for a 40-char secret.
            let span_end = (id_m.end() + 512).min(s.len());
            if let Some(sec_m) = secret_re().find(&s[id_m.end()..span_end]) {
                r.secret_parts
                    .insert("secret_access_key".into(), sec_m.as_str().into());
                r.raw_v2 = Some(format!("{}:{}", id, sec_m.as_str()));
            }
            out.push(r);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_standard_akia_prefix() {
        let r = AwsAccessKey.from_data(b"AKIAIOSFODNN7EXAMPLE");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].raw, "AKIAIOSFODNN7EXAMPLE");
        assert_eq!(
            r[0].secret_parts.get("access_key_id").unwrap(),
            "AKIAIOSFODNN7EXAMPLE"
        );
    }

    #[test]
    fn detects_abia_and_acca() {
        let r = AwsAccessKey.from_data(b"ABIAABCDEFGHIJKLMNOP\nACCAABCDEFGHIJKLMNOP");
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn pairs_secret_when_within_window() {
        let s = b"AKIAIOSFODNN7EXAMPLE wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY12";
        let r = AwsAccessKey.from_data(s);
        assert_eq!(r.len(), 1);
        assert!(r[0].raw_v2.as_deref().unwrap().contains(":"));
        assert!(r[0].secret_parts.contains_key("secret_access_key"));
    }

    #[test]
    fn rejects_invalid_prefix() {
        assert!(AwsAccessKey.from_data(b"XXIAIOSFODNN7EXAMPLE").is_empty());
    }
}
