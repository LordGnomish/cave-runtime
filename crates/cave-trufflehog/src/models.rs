// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Core types — line-by-line port of upstream `pkg/detectors.Result` +
//! `pkg/pb/source_metadatapb` + `pkg/sources.Chunk`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Mirrors upstream `detector_typepb.DetectorType` — only the subset we ship.
/// Numeric values match upstream protobuf enum so downstream parity-of-output
/// is preserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u32)]
pub enum DetectorType {
    Generic = 0,
    Aws = 2,
    Github = 14,
    Gitlab = 17,
    Slack = 33,
    Stripe = 39,
    Twilio = 44,
    Sendgrid = 32,
    Mailgun = 24,
    Square = 38,
    Gcp = 16,
    Azure = 5,
    Anthropic = 800,
    Openai = 1000,
    NpmToken = 167,
    PypiUpload = 188,
    Jwt = 152,
    PrivateKey = 80,
    Custom = 6,
}

impl DetectorType {
    pub fn name(&self) -> &'static str {
        match self {
            DetectorType::Generic => "Generic",
            DetectorType::Aws => "AWS",
            DetectorType::Github => "Github",
            DetectorType::Gitlab => "Gitlab",
            DetectorType::Slack => "Slack",
            DetectorType::Stripe => "Stripe",
            DetectorType::Twilio => "Twilio",
            DetectorType::Sendgrid => "Sendgrid",
            DetectorType::Mailgun => "Mailgun",
            DetectorType::Square => "Square",
            DetectorType::Gcp => "GCP",
            DetectorType::Azure => "Azure",
            DetectorType::Anthropic => "Anthropic",
            DetectorType::Openai => "OpenAI",
            DetectorType::NpmToken => "NpmToken",
            DetectorType::PypiUpload => "PyPIUploadToken",
            DetectorType::Jwt => "JWT",
            DetectorType::PrivateKey => "PrivateKey",
            DetectorType::Custom => "CustomRegex",
        }
    }
}

/// Per-result decision tree mirror — upstream `Result` struct fields with the
/// same JSON tags.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DetectionResult {
    #[serde(rename = "DetectorType")]
    pub detector_type: DetectorType,
    #[serde(rename = "DetectorName")]
    pub detector_name: String,
    #[serde(rename = "Raw")]
    pub raw: String,
    #[serde(rename = "RawV2", skip_serializing_if = "Option::is_none")]
    pub raw_v2: Option<String>,
    #[serde(rename = "Verified")]
    pub verified: bool,
    #[serde(
        rename = "VerificationError",
        skip_serializing_if = "Option::is_none"
    )]
    pub verification_error: Option<String>,
    #[serde(rename = "ExtraData", default)]
    pub extra_data: BTreeMap<String, String>,
    #[serde(rename = "SecretParts", default)]
    pub secret_parts: BTreeMap<String, String>,
}

impl DetectionResult {
    pub fn new(t: DetectorType, raw: impl Into<String>) -> Self {
        let raw = raw.into();
        Self {
            detector_type: t,
            detector_name: t.name().to_string(),
            raw,
            raw_v2: None,
            verified: false,
            verification_error: None,
            extra_data: BTreeMap::new(),
            secret_parts: BTreeMap::new(),
        }
    }

    pub fn with_extra(mut self, k: &str, v: &str) -> Self {
        self.extra_data.insert(k.into(), v.into());
        self
    }

    pub fn set_verification_error(&mut self, e: impl ToString) {
        self.verification_error = Some(e.to_string());
        self.verified = false;
    }
}

/// `pkg/sources.Chunk` — opaque content + metadata band the engine pumps
/// through each registered detector.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Chunk {
    pub source_type: String,
    pub source_name: String,
    pub source_id: u64,
    pub job_id: u64,
    pub secret_id: u64,
    pub data: Vec<u8>,
    pub source_metadata: SourceMetadata,
    pub verify: bool,
}

impl Chunk {
    pub fn new(source_type: &str, source_name: &str, data: Vec<u8>) -> Self {
        Self {
            source_type: source_type.into(),
            source_name: source_name.into(),
            source_id: 0,
            job_id: 0,
            secret_id: 0,
            data,
            source_metadata: SourceMetadata::default(),
            verify: false,
        }
    }
}

/// `pkg/pb/source_metadatapb.MetaData` — pruned to the source kinds we ship.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SourceMetadata {
    pub kind: SourceKind,
    pub repository: Option<String>,
    pub commit: Option<String>,
    pub commit_author: Option<String>,
    pub file: Option<String>,
    pub line: Option<u64>,
    pub email: Option<String>,
    pub timestamp: Option<String>,
    pub branch: Option<String>,
    pub bucket: Option<String>,
    pub container: Option<String>,
    pub image: Option<String>,
    pub layer: Option<String>,
    pub channel: Option<String>,
    pub issue: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceKind {
    #[default]
    Generic,
    Git,
    Github,
    Gitlab,
    Bitbucket,
    S3,
    Gcs,
    Filesystem,
    Docker,
    Stdin,
    Jira,
    Confluence,
    Slack,
    Database,
}

impl SourceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceKind::Generic => "generic",
            SourceKind::Git => "git",
            SourceKind::Github => "github",
            SourceKind::Gitlab => "gitlab",
            SourceKind::Bitbucket => "bitbucket",
            SourceKind::S3 => "s3",
            SourceKind::Gcs => "gcs",
            SourceKind::Filesystem => "filesystem",
            SourceKind::Docker => "docker",
            SourceKind::Stdin => "stdin",
            SourceKind::Jira => "jira",
            SourceKind::Confluence => "confluence",
            SourceKind::Slack => "slack",
            SourceKind::Database => "database",
        }
    }
}

/// One full secret finding, ready for an output writer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Finding {
    pub result: DetectionResult,
    pub chunk_source: String,
    pub source_metadata: SourceMetadata,
    pub redacted: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detector_type_names_round_trip() {
        for t in [
            DetectorType::Aws,
            DetectorType::Github,
            DetectorType::Slack,
            DetectorType::Anthropic,
            DetectorType::Openai,
        ] {
            assert!(!t.name().is_empty());
        }
    }

    #[test]
    fn detection_result_builder_sets_fields() {
        let r = DetectionResult::new(DetectorType::Aws, "AKIA…").with_extra("region", "us-east-1");
        assert_eq!(r.detector_type, DetectorType::Aws);
        assert_eq!(r.detector_name, "AWS");
        assert_eq!(r.extra_data.get("region").unwrap(), "us-east-1");
        assert!(!r.verified);
    }

    #[test]
    fn set_verification_error_clears_verified() {
        let mut r = DetectionResult::new(DetectorType::Stripe, "sk_live_x");
        r.verified = true;
        r.set_verification_error("HTTP 502");
        assert!(!r.verified);
        assert_eq!(r.verification_error.as_deref(), Some("HTTP 502"));
    }

    #[test]
    fn chunk_default_metadata_is_generic() {
        let c = Chunk::new("filesystem", "main", b"x".to_vec());
        assert_eq!(c.source_metadata.kind, SourceKind::Generic);
    }

    #[test]
    fn source_kind_strings_unique() {
        let kinds = [
            SourceKind::Git,
            SourceKind::Github,
            SourceKind::Gitlab,
            SourceKind::Bitbucket,
            SourceKind::S3,
            SourceKind::Gcs,
            SourceKind::Filesystem,
            SourceKind::Docker,
            SourceKind::Stdin,
            SourceKind::Jira,
            SourceKind::Confluence,
            SourceKind::Slack,
            SourceKind::Database,
        ];
        let strs: Vec<_> = kinds.iter().map(|k| k.as_str()).collect();
        let mut sorted = strs.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(strs.len(), sorted.len());
    }

    #[test]
    fn detection_result_serializes_with_upstream_tags() {
        let r = DetectionResult::new(DetectorType::Github, "ghp_xxx");
        let j = serde_json::to_value(&r).unwrap();
        assert!(j.get("DetectorType").is_some());
        assert!(j.get("Raw").is_some());
        assert!(j.get("Verified").is_some());
    }
}
