// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: aquasecurity/trivy@8a3177a pkg/report/template/template.go

//! Tiny template engine — `{{ field }}` placeholder substitution.
//!
//! Trivy supports Go `text/template`; we provide a focused subset:
//! - `{{ target }}` — `report.target`
//! - `{{ scanner }}` — `report.scanner`
//! - `{{ count }}` — total findings
//! - `{{ severity:CRITICAL }}` — count by severity (CRITICAL/HIGH/MEDIUM/LOW/INFO)
//! - `{{ findings }}` — JSON-encoded findings array
//!
//! Unknown placeholders are left intact so the caller notices.

use super::{Report, Severity};

pub fn render(template: &str, report: &Report) -> String {
    let mut out = template.to_string();
    out = out.replace("{{ target }}", &report.target);
    out = out.replace("{{ scanner }}", &report.scanner);
    out = out.replace("{{ count }}", &report.findings.len().to_string());

    for sev in [
        Severity::Critical,
        Severity::High,
        Severity::Medium,
        Severity::Low,
        Severity::Info,
    ] {
        let n = report.findings.iter().filter(|f| f.severity == sev).count();
        out = out.replace(
            &format!("{{{{ severity:{} }}}}", sev.as_str()),
            &n.to_string(),
        );
    }

    let findings_json =
        serde_json::to_string(&report.findings).unwrap_or_else(|_| "[]".to_string());
    out = out.replace("{{ findings }}", &findings_json);
    out
}
