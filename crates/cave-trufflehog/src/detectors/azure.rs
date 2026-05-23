// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Azure Storage account key + SAS token + connection string — port of
//! `pkg/detectors/azurestorage/`.

use crate::detector::Detector;
use crate::models::{DetectionResult, DetectorType};
use regex::Regex;
use std::sync::OnceLock;

pub struct AzureStorageKey;

static CONN_RE: OnceLock<Regex> = OnceLock::new();
static ACCOUNT_KEY_RE: OnceLock<Regex> = OnceLock::new();

fn conn_re() -> &'static Regex {
    CONN_RE.get_or_init(|| {
        Regex::new(
            r"DefaultEndpointsProtocol=https?;AccountName=[A-Za-z0-9]+;AccountKey=[A-Za-z0-9+/=]{88}",
        )
        .unwrap()
    })
}

fn account_key_re() -> &'static Regex {
    ACCOUNT_KEY_RE.get_or_init(|| Regex::new(r"[A-Za-z0-9+/]{86}==").unwrap())
}

impl Detector for AzureStorageKey {
    fn detector_type(&self) -> DetectorType {
        DetectorType::Azure
    }
    fn description(&self) -> &'static str {
        "Azure Storage account key (88-char base64) or full connection string"
    }
    fn keywords(&self) -> &'static [&'static str] {
        &["AccountKey=", "DefaultEndpointsProtocol="]
    }
    fn from_data(&self, data: &[u8]) -> Vec<DetectionResult> {
        let Ok(s) = std::str::from_utf8(data) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for m in conn_re().find_iter(s) {
            out.push(
                DetectionResult::new(DetectorType::Azure, m.as_str())
                    .with_extra("kind", "connection_string"),
            );
        }
        // Bare-key fallback inside config files that embed only the key.
        for m in account_key_re().find_iter(s) {
            // De-dup against connection-string matches.
            if out.iter().any(|r| r.raw.contains(m.as_str())) {
                continue;
            }
            out.push(
                DetectionResult::new(DetectorType::Azure, m.as_str())
                    .with_extra("kind", "account_key"),
            );
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_connection_string() {
        let s = format!(
            "DefaultEndpointsProtocol=https;AccountName=mystore;AccountKey={}",
            "a".repeat(88)
        );
        let r = AzureStorageKey.from_data(s.as_bytes());
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].extra_data.get("kind").unwrap(), "connection_string");
    }

    #[test]
    fn detects_bare_account_key() {
        let s = format!("{}==", "A".repeat(86));
        let r = AzureStorageKey.from_data(s.as_bytes());
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].extra_data.get("kind").unwrap(), "account_key");
    }
}
