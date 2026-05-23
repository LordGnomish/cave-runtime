// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! GitHub Actions writer — port of `pkg/output/github_actions.go`.
//! Emits one `::error file=…,line=…::…` workflow command per finding so
//! the GitHub PR annotates the offending source line directly.

use crate::error::Result;
use crate::models::Finding;
use std::io::Write;

pub fn write_gha<W: Write>(w: &mut W, findings: &[Finding]) -> Result<()> {
    for f in findings {
        let file = f.source_metadata.file.clone().unwrap_or_default();
        let line = f.source_metadata.line.unwrap_or(1);
        let msg = format!(
            "{} detected ({}{})",
            f.result.detector_name,
            if f.result.verified { "verified" } else { "unverified" },
            f.source_metadata
                .commit
                .as_ref()
                .map(|c| format!(", commit {}", c))
                .unwrap_or_default()
        );
        writeln!(
            w,
            "::error file={},line={}::{}",
            gha_escape(&file),
            line,
            gha_escape(&msg),
        )?;
    }
    Ok(())
}

fn gha_escape(s: &str) -> String {
    s.replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{DetectionResult, DetectorType, SourceMetadata};

    fn mk_with(file: &str, line: u64, verified: bool) -> Finding {
        let mut r = DetectionResult::new(DetectorType::Slack, "xoxb-x");
        r.verified = verified;
        Finding {
            result: r,
            chunk_source: "filesystem".into(),
            source_metadata: SourceMetadata {
                file: Some(file.into()),
                line: Some(line),
                ..Default::default()
            },
            redacted: "xoxb-…".into(),
        }
    }

    #[test]
    fn emits_one_workflow_command_per_finding() {
        let mut buf = Vec::new();
        write_gha(
            &mut buf,
            &[mk_with("a.go", 12, false), mk_with("b.go", 7, true)],
        )
        .unwrap();
        let s = String::from_utf8(buf).unwrap();
        let lines: Vec<_> = s.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("::error file=a.go,line=12::Slack"));
        assert!(lines[1].contains("verified"));
    }

    #[test]
    fn escapes_special_chars() {
        assert_eq!(gha_escape("a\rb\nc"), "a%0Db%0Ac");
        assert_eq!(gha_escape("100%"), "100%25");
    }

    #[test]
    fn no_line_defaults_to_1() {
        let mut f = mk_with("x.go", 0, false);
        f.source_metadata.line = None;
        let mut buf = Vec::new();
        write_gha(&mut buf, &[f]).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("line=1"));
    }
}
