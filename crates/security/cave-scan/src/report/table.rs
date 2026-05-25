// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: aquasecurity/trivy@8a3177a pkg/report/table/table.go

//! Plain-text table renderer for terminal output.

use super::{Report, Severity};

pub fn render(report: &Report) -> String {
    let mut out = String::new();
    out.push_str(&format!("Target: {}\n", report.target));
    out.push_str(&format!("Scanner: {}\n", report.scanner));
    out.push_str(&format!("Findings: {}\n", report.findings.len()));
    out.push('\n');

    if report.findings.is_empty() {
        out.push_str("No findings.\n");
        return out;
    }

    // Compute column widths.
    let mut w_id = 4usize;
    let mut w_sev = 8usize;
    let mut w_title = 5usize;
    for f in &report.findings {
        w_id = w_id.max(f.id.len());
        w_sev = w_sev.max(f.severity.as_str().len());
        w_title = w_title.max(f.title.len());
    }

    out.push_str(&format!(
        "{:<w_id$}  {:<w_sev$}  {:<w_title$}  {}\n",
        "ID",
        "SEVERITY",
        "TITLE",
        "LOCATION",
        w_id = w_id,
        w_sev = w_sev,
        w_title = w_title
    ));
    out.push_str(&format!(
        "{}  {}  {}  {}\n",
        "-".repeat(w_id),
        "-".repeat(w_sev),
        "-".repeat(w_title),
        "-".repeat(8)
    ));

    for f in &report.findings {
        out.push_str(&format!(
            "{:<w_id$}  {:<w_sev$}  {:<w_title$}  {}\n",
            f.id,
            f.severity.as_str(),
            f.title,
            f.location,
            w_id = w_id,
            w_sev = w_sev,
            w_title = w_title
        ));
    }

    // Counts by severity.
    let mut counts = [0usize; 5];
    for f in &report.findings {
        let idx = match f.severity {
            Severity::Critical => 0,
            Severity::High => 1,
            Severity::Medium => 2,
            Severity::Low => 3,
            Severity::Info => 4,
        };
        counts[idx] += 1;
    }
    out.push('\n');
    out.push_str(&format!(
        "Total: {} (CRITICAL={}, HIGH={}, MEDIUM={}, LOW={}, INFO={})\n",
        report.findings.len(),
        counts[0],
        counts[1],
        counts[2],
        counts[3],
        counts[4]
    ));
    out
}
