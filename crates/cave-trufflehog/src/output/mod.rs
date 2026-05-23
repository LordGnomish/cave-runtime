// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Output writers — port of `pkg/output/`. Four formats with parity to
//! upstream column / field names. Each writer is a free function so the
//! engine can pick by `OutputFormat` at scan time.

pub mod github_actions;
pub mod json;
pub mod plain;

use crate::error::Result;
use crate::models::Finding;
use serde::{Deserialize, Serialize};
use std::io::Write;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputFormat {
    Json,
    Jsonl,
    Plain,
    GithubActions,
}

impl OutputFormat {
    pub fn write<W: Write>(&self, w: &mut W, findings: &[Finding]) -> Result<()> {
        match self {
            OutputFormat::Json => json::write_json(w, findings),
            OutputFormat::Jsonl => json::write_jsonl(w, findings),
            OutputFormat::Plain => plain::write_plain(w, findings),
            OutputFormat::GithubActions => github_actions::write_gha(w, findings),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{DetectionResult, DetectorType, SourceMetadata};

    fn mk_finding() -> Finding {
        Finding {
            result: DetectionResult::new(DetectorType::Stripe, "sk_live_x"),
            chunk_source: "git".into(),
            source_metadata: SourceMetadata::default(),
            redacted: "sk_l…".into(),
        }
    }

    #[test]
    fn dispatch_json() {
        let mut buf = Vec::new();
        OutputFormat::Json.write(&mut buf, &[mk_finding()]).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.starts_with('['));
    }

    #[test]
    fn dispatch_jsonl() {
        let mut buf = Vec::new();
        OutputFormat::Jsonl
            .write(&mut buf, &[mk_finding(), mk_finding()])
            .unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert_eq!(s.lines().count(), 2);
    }

    #[test]
    fn dispatch_plain() {
        let mut buf = Vec::new();
        OutputFormat::Plain.write(&mut buf, &[mk_finding()]).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("Stripe"));
    }

    #[test]
    fn dispatch_gha() {
        let mut buf = Vec::new();
        OutputFormat::GithubActions
            .write(&mut buf, &[mk_finding()])
            .unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.starts_with("::error"));
    }
}
