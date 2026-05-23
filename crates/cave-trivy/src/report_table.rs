// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Table report writer.
//!
//! Mirrors trivy's `pkg/report/table` — fixed-width columns sized to the
//! widest cell, severity-coloured prefix (`[CRITICAL]` / `[HIGH]` /
//! `[MEDIUM]` / `[LOW]` / `[UNKNOWN]`). Pure-text output; the upstream
//! tty colour escapes are deferred to cavectl's UI layer.

use crate::models::Report;

pub fn write(report: &Report) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Artifact: {} ({})\n",
        report.artifact_name, report.artifact_type
    ));
    if let Some(os) = &report.os {
        out.push_str(&format!("OS: {:?} {}\n", os.family, os.name));
    }
    out.push_str(&format!(
        "Total vulnerabilities: {}, misconfigurations: {}, secrets: {}\n\n",
        report.total_vulns(),
        report.total_misconfigs(),
        report.total_secrets()
    ));
    for r in &report.results {
        if r.vulnerabilities.is_empty()
            && r.misconfigurations.is_empty()
            && r.secrets.is_empty()
        {
            continue;
        }
        out.push_str(&format!("=== {} ({}) ===\n", r.target, r.class));
        if !r.vulnerabilities.is_empty() {
            let mut rows: Vec<Vec<String>> = vec![vec![
                "Severity".into(),
                "ID".into(),
                "Package".into(),
                "Installed".into(),
                "Fixed".into(),
            ]];
            for v in &r.vulnerabilities {
                rows.push(vec![
                    format!("[{}]", v.severity.as_str()),
                    v.id.clone(),
                    v.pkg_name.clone(),
                    v.installed_version.clone(),
                    v.fixed_version.clone().unwrap_or_else(|| "-".into()),
                ]);
            }
            out.push_str(&render_table(&rows));
            out.push('\n');
        }
        if !r.misconfigurations.is_empty() {
            let mut rows: Vec<Vec<String>> =
                vec![vec!["Severity".into(), "ID".into(), "Type".into(), "Title".into()]];
            for m in &r.misconfigurations {
                rows.push(vec![
                    format!("[{}]", m.severity.as_str()),
                    m.id.clone(),
                    m.r#type.clone(),
                    m.title.clone(),
                ]);
            }
            out.push_str(&render_table(&rows));
            out.push('\n');
        }
        if !r.secrets.is_empty() {
            let mut rows: Vec<Vec<String>> = vec![vec![
                "Severity".into(),
                "Rule".into(),
                "Category".into(),
                "Line".into(),
                "File".into(),
            ]];
            for s in &r.secrets {
                rows.push(vec![
                    format!("[{}]", s.severity.as_str()),
                    s.rule_id.clone(),
                    s.category.clone(),
                    s.start_line.to_string(),
                    s.file.clone(),
                ]);
            }
            out.push_str(&render_table(&rows));
            out.push('\n');
        }
    }
    out
}

fn render_table(rows: &[Vec<String>]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let cols = rows[0].len();
    let mut widths = vec![0; cols];
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if cell.len() > widths[i] {
                widths[i] = cell.len();
            }
        }
    }
    let mut out = String::new();
    for (rownum, row) in rows.iter().enumerate() {
        for (i, cell) in row.iter().enumerate() {
            out.push_str(&format!("{:width$}", cell, width = widths[i]));
            if i + 1 < cols {
                out.push_str("  ");
            }
        }
        out.push('\n');
        if rownum == 0 {
            for (i, w) in widths.iter().enumerate() {
                out.push_str(&"-".repeat(*w));
                if i + 1 < cols {
                    out.push_str("  ");
                }
            }
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Report, ScanResult, Vulnerability};
    use crate::severity::Severity;

    #[test]
    fn empty_report_renders_summary() {
        let r = Report::new("nginx:1.27", "container_image");
        let t = write(&r);
        assert!(t.contains("nginx:1.27"));
        assert!(t.contains("Total vulnerabilities: 0"));
    }

    #[test]
    fn vuln_table_aligned() {
        let mut r = Report::new("x", "container_image");
        let mut sr = ScanResult {
            target: "x".into(),
            class: "os".into(),
            ..Default::default()
        };
        sr.vulnerabilities.push(Vulnerability {
            id: "CVE-2026-1".into(),
            pkg_name: "openssl".into(),
            installed_version: "3.0.0".into(),
            fixed_version: Some("3.0.13".into()),
            severity: Severity::Critical,
            references: vec![],
            title: None,
        });
        r.results.push(sr);
        let t = write(&r);
        assert!(t.contains("[CRITICAL]"));
        assert!(t.contains("CVE-2026-1"));
        assert!(t.contains("3.0.13"));
    }

    #[test]
    fn secret_table_renders() {
        let mut r = Report::new("repo", "git");
        r.results.push(ScanResult {
            target: "repo".into(),
            class: "secrets".into(),
            secrets: vec![crate::models::Secret {
                rule_id: "github-pat".into(),
                category: "github".into(),
                severity: Severity::Critical,
                start_line: 1,
                end_line: 1,
                match_text: "ghp_…".into(),
                file: ".env".into(),
            }],
            ..Default::default()
        });
        let t = write(&r);
        assert!(t.contains("[CRITICAL]"));
        assert!(t.contains("github-pat"));
    }

    #[test]
    fn omits_empty_result_section() {
        let mut r = Report::new("x", "y");
        r.results.push(ScanResult::default());
        let t = write(&r);
        assert!(!t.contains("==="));
    }
}
