// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Plain-text writer — port of `pkg/output/plain.go`. Human-grokkable
//! one-block-per-finding layout used by `cavectl secret scan … --plain`.

use crate::error::Result;
use crate::models::Finding;
use std::io::Write;

pub fn write_plain<W: Write>(w: &mut W, findings: &[Finding]) -> Result<()> {
    for f in findings {
        writeln!(w, "Found unverified result")?;
        writeln!(w, "Detector Type: {}", f.result.detector_name)?;
        if f.result.verified {
            writeln!(w, "Verified: true")?;
        }
        if let Some(err) = &f.result.verification_error {
            writeln!(w, "Verification Error: {}", err)?;
        }
        writeln!(w, "Raw result: {}", f.redacted)?;
        if let Some(file) = &f.source_metadata.file {
            writeln!(w, "File: {}", file)?;
        }
        if let Some(commit) = &f.source_metadata.commit {
            writeln!(w, "Commit: {}", commit)?;
        }
        if !f.result.extra_data.is_empty() {
            writeln!(w, "Extra Data:")?;
            for (k, v) in &f.result.extra_data {
                writeln!(w, "  {}: {}", k, v)?;
            }
        }
        writeln!(w)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{DetectionResult, DetectorType, SourceMetadata};

    fn mk() -> Finding {
        let mut r = DetectionResult::new(DetectorType::Github, "ghp_x").with_extra(
            "token_type",
            "PAT",
        );
        r.verified = true;
        Finding {
            result: r,
            chunk_source: "filesystem".into(),
            source_metadata: SourceMetadata {
                file: Some("/repo/x.txt".into()),
                commit: Some("c0ffee".into()),
                ..Default::default()
            },
            redacted: "ghp_…".into(),
        }
    }

    #[test]
    fn renders_detector_and_file() {
        let mut buf = Vec::new();
        write_plain(&mut buf, &[mk()]).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("Detector Type: Github"));
        assert!(s.contains("File: /repo/x.txt"));
        assert!(s.contains("Commit: c0ffee"));
        assert!(s.contains("Verified: true"));
    }

    #[test]
    fn extra_data_indented() {
        let mut buf = Vec::new();
        write_plain(&mut buf, &[mk()]).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("  token_type: PAT"));
    }

    #[test]
    fn empty_input_produces_empty_output() {
        let mut buf = Vec::new();
        write_plain(&mut buf, &[]).unwrap();
        assert!(buf.is_empty());
    }
}
