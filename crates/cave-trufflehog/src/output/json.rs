// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! JSON + JSONL writers — port of `pkg/output/json.go` and the streaming
//! upstream-compatible JSON Lines reporter.

use crate::error::{Error, Result};
use crate::models::Finding;
use std::io::Write;

pub fn write_json<W: Write>(w: &mut W, findings: &[Finding]) -> Result<()> {
    let s = serde_json::to_string_pretty(findings).map_err(|e| Error::Serialization(e.to_string()))?;
    w.write_all(s.as_bytes())?;
    Ok(())
}

pub fn write_jsonl<W: Write>(w: &mut W, findings: &[Finding]) -> Result<()> {
    for f in findings {
        let s = serde_json::to_string(f).map_err(|e| Error::Serialization(e.to_string()))?;
        w.write_all(s.as_bytes())?;
        w.write_all(b"\n")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{DetectionResult, DetectorType, SourceMetadata};

    fn fixture() -> Vec<Finding> {
        vec![
            Finding {
                result: DetectionResult::new(DetectorType::Aws, "AKIA…"),
                chunk_source: "git".into(),
                source_metadata: SourceMetadata::default(),
                redacted: "AKIA…".into(),
            },
            Finding {
                result: DetectionResult::new(DetectorType::Stripe, "sk_live_x"),
                chunk_source: "filesystem".into(),
                source_metadata: SourceMetadata::default(),
                redacted: "sk_…".into(),
            },
        ]
    }

    #[test]
    fn pretty_json_array() {
        let mut buf = Vec::new();
        write_json(&mut buf, &fixture()).unwrap();
        let s = String::from_utf8(buf).unwrap();
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert!(v.is_array());
        assert_eq!(v.as_array().unwrap().len(), 2);
    }

    #[test]
    fn jsonl_line_per_finding() {
        let mut buf = Vec::new();
        write_jsonl(&mut buf, &fixture()).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert_eq!(s.lines().count(), 2);
        for l in s.lines() {
            let _: serde_json::Value = serde_json::from_str(l).unwrap();
        }
    }

    #[test]
    fn empty_findings_emit_empty_array() {
        let mut buf = Vec::new();
        write_json(&mut buf, &[]).unwrap();
        assert_eq!(String::from_utf8(buf).unwrap(), "[]");
    }

    #[test]
    fn empty_findings_jsonl_empty() {
        let mut buf = Vec::new();
        write_jsonl(&mut buf, &[]).unwrap();
        assert!(buf.is_empty());
    }
}
