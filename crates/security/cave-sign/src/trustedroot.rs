// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Trusted root materials — Fulcio + Rekor + CT-log public keys + issuance
//! validity windows.
//!
//! Maps to:
//!   * pkg/cosign/trustedroot      → TrustedRoot (sigstore-go trustroot.json)
//!   * cmd/cosign/cli/trustedroot  → `cosign trusted-root`
//!
//! Full TUF bootstrap is **out of MVP**; we ship parse + lookup for the
//! `trusted_root.json` shape so the verifier can take a static root from
//! cave-vault or a sovereign cave-tuf instance.

use crate::error::{Result, SignError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TrustedRoot {
    pub media_type: String,
    pub fulcio_certs: Vec<TrustEntry>,
    pub rekor_logs: Vec<TrustEntry>,
    pub ct_logs: Vec<TrustEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrustEntry {
    pub id: String,
    pub pub_key_pem: String,
    pub valid_for: ValidityRange,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ValidityRange {
    /// RFC 3339 start.
    pub start: String,
    /// RFC 3339 end — optional means "still valid".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<String>,
}

impl TrustedRoot {
    pub fn from_json(s: &str) -> Result<Self> {
        serde_json::from_str(s)
            .map_err(|e| SignError::TrustedRoot(format!("decode: {}", e)))
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| SignError::TrustedRoot(format!("encode: {}", e)))
    }

    pub fn fulcio_active(&self, at_rfc3339: &str) -> Vec<&TrustEntry> {
        self.fulcio_certs
            .iter()
            .filter(|e| e.valid_for.contains(at_rfc3339))
            .collect()
    }

    pub fn rekor_for_id(&self, id: &str) -> Option<&TrustEntry> {
        self.rekor_logs.iter().find(|e| e.id == id)
    }

    /// Build a cave-local trusted root suitable for offline tests.
    pub fn cave_default() -> Self {
        Self {
            media_type: "application/vnd.dev.sigstore.trustedroot+json;version=0.1".into(),
            fulcio_certs: vec![TrustEntry {
                id: "cave-fulcio-mock-root".into(),
                pub_key_pem: "-----BEGIN PUBLIC KEY-----\nMOCK\n-----END PUBLIC KEY-----".into(),
                valid_for: ValidityRange {
                    start: "2026-01-01T00:00:00Z".into(),
                    end: None,
                },
            }],
            rekor_logs: vec![TrustEntry {
                id: "cave-rekor-mock".into(),
                pub_key_pem: "-----BEGIN PUBLIC KEY-----\nMOCK\n-----END PUBLIC KEY-----".into(),
                valid_for: ValidityRange {
                    start: "2026-01-01T00:00:00Z".into(),
                    end: None,
                },
            }],
            ct_logs: vec![TrustEntry {
                id: "cave-ctlog-mock".into(),
                pub_key_pem: "-----BEGIN PUBLIC KEY-----\nMOCK\n-----END PUBLIC KEY-----".into(),
                valid_for: ValidityRange {
                    start: "2026-01-01T00:00:00Z".into(),
                    end: None,
                },
            }],
        }
    }
}

impl ValidityRange {
    pub fn contains(&self, t: &str) -> bool {
        if t < self.start.as_str() {
            return false;
        }
        if let Some(end) = &self.end {
            return t <= end.as_str();
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cave_default_roundtrip() {
        let root = TrustedRoot::cave_default();
        let s = root.to_json().unwrap();
        let back = TrustedRoot::from_json(&s).unwrap();
        assert_eq!(root, back);
    }

    #[test]
    fn validity_contains_open_range() {
        let v = ValidityRange {
            start: "2026-01-01T00:00:00Z".into(),
            end: None,
        };
        assert!(v.contains("2026-12-31T00:00:00Z"));
        assert!(!v.contains("2025-12-31T00:00:00Z"));
    }

    #[test]
    fn validity_contains_closed_range() {
        let v = ValidityRange {
            start: "2026-01-01T00:00:00Z".into(),
            end: Some("2026-12-31T00:00:00Z".into()),
        };
        assert!(v.contains("2026-06-01T00:00:00Z"));
        assert!(!v.contains("2027-01-01T00:00:00Z"));
    }

    #[test]
    fn fulcio_active_filters() {
        let r = TrustedRoot::cave_default();
        assert_eq!(r.fulcio_active("2026-06-01T00:00:00Z").len(), 1);
        assert_eq!(r.fulcio_active("2025-01-01T00:00:00Z").len(), 0);
    }

    #[test]
    fn rekor_for_id_lookup() {
        let r = TrustedRoot::cave_default();
        assert!(r.rekor_for_id("cave-rekor-mock").is_some());
        assert!(r.rekor_for_id("not-here").is_none());
    }

    #[test]
    fn invalid_json_fails() {
        let err = TrustedRoot::from_json("{").expect_err("must reject");
        assert!(matches!(err, SignError::TrustedRoot(_)));
    }
}
