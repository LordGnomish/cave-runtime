// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Twilio Account SID + Auth Token — port of `pkg/detectors/twilio/`.

use crate::detector::Detector;
use crate::models::{DetectionResult, DetectorType};
use regex::Regex;
use std::sync::OnceLock;

pub struct TwilioKey;

static SID_RE: OnceLock<Regex> = OnceLock::new();
static TOKEN_RE: OnceLock<Regex> = OnceLock::new();

fn sid_re() -> &'static Regex {
    SID_RE.get_or_init(|| Regex::new(r"AC[a-f0-9]{32}").unwrap())
}
fn token_re() -> &'static Regex {
    TOKEN_RE.get_or_init(|| Regex::new(r"\b[a-f0-9]{32}\b").unwrap())
}

impl Detector for TwilioKey {
    fn detector_type(&self) -> DetectorType {
        DetectorType::Twilio
    }
    fn description(&self) -> &'static str {
        "Twilio Account SID (ACxxx) paired with auth token within 256 bytes"
    }
    fn keywords(&self) -> &'static [&'static str] {
        &["AC"]
    }
    fn from_data(&self, data: &[u8]) -> Vec<DetectionResult> {
        let Ok(s) = std::str::from_utf8(data) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for m in sid_re().find_iter(s) {
            let span_end = (m.end() + 256).min(s.len());
            let mut r = DetectionResult::new(DetectorType::Twilio, m.as_str());
            r.secret_parts.insert("account_sid".into(), m.as_str().into());
            if let Some(t) = token_re().find(&s[m.end()..span_end]) {
                r.secret_parts.insert("auth_token".into(), t.as_str().into());
                r.raw_v2 = Some(format!("{}:{}", m.as_str(), t.as_str()));
            }
            out.push(r);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_sid() -> String {
        // Construct at runtime so static analysers don't flag the source as
        // containing a literal SID — the value is obviously non-real (all `a`).
        format!("AC{}", "a".repeat(32))
    }

    fn fixture_token() -> String {
        format!("{}", "b".repeat(32))
    }

    #[test]
    fn detects_sid_alone() {
        let s = fixture_sid();
        let r = TwilioKey.from_data(s.as_bytes());
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn pairs_sid_and_token() {
        let s = format!("{} token={}", fixture_sid(), fixture_token());
        let r = TwilioKey.from_data(s.as_bytes());
        assert_eq!(r.len(), 1);
        assert!(r[0].raw_v2.is_some());
        assert!(r[0].secret_parts.contains_key("auth_token"));
    }

    #[test]
    fn rejects_uppercase_hex_only() {
        // Token regex is lowercase hex.
        assert!(
            TwilioKey
                .from_data(b"ACABCDEFFEDCBAFEDCBAFEDCBAFEDCBAFE")
                .is_empty()
        );
    }
}
