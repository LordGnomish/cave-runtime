// SPDX-License-Identifier: AGPL-3.0-or-later
//! Report generation — JSON now, HTML stretch.
//!
//! Source: DefectDojo/django-DefectDojo@6eab8738
//!         dojo/reports/views.py (executive + detailed report generators).
//!         JSON is the v2 API serialisation format; PDF is Phase 2.

use crate::finding::{Finding, FindingSeverity};
use crate::hierarchy::{Engagement, Product};
use crate::sla::{rollup, SlaConfiguration};
use chrono::Utc;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ExecutiveSummary {
    pub generated_at: String,
    pub product: Option<String>,
    pub engagement: Option<String>,
    pub total_findings: usize,
    pub active_findings: usize,
    pub by_severity: SeverityCounts,
    pub sla_breached: usize,
    pub sla_breaching_soon: usize,
    pub top_components: Vec<(String, usize)>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SeverityCounts {
    pub critical: usize,
    pub high: usize,
    pub medium: usize,
    pub low: usize,
    pub info: usize,
}

/// Build the executive summary as a JSON-serialisable struct.
/// Source: dojo/reports/views.py::ReportBuilder.run.
pub fn executive_summary(
    product: Option<&Product>,
    engagement: Option<&Engagement>,
    findings: &[Finding],
    sla: &SlaConfiguration,
) -> ExecutiveSummary {
    let mut by_sev = SeverityCounts::default();
    let mut active = 0usize;
    let mut components: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for f in findings {
        if f.state.active { active += 1; }
        match f.severity {
            FindingSeverity::Critical => by_sev.critical += 1,
            FindingSeverity::High => by_sev.high += 1,
            FindingSeverity::Medium => by_sev.medium += 1,
            FindingSeverity::Low => by_sev.low += 1,
            FindingSeverity::Info => by_sev.info += 1,
        }
        if let Some(name) = &f.component_name {
            *components.entry(name.clone()).or_insert(0) += 1;
        }
    }
    let now = Utc::now();
    let sla_rep = rollup(sla, findings, now);
    let mut top: Vec<(String, usize)> = components.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    top.truncate(10);
    ExecutiveSummary {
        generated_at: now.to_rfc3339(),
        product: product.map(|p| p.name.clone()),
        engagement: engagement.map(|e| e.name.clone()),
        total_findings: findings.len(),
        active_findings: active,
        by_severity: by_sev,
        sla_breached: sla_rep.breached,
        sla_breaching_soon: sla_rep.breaching_soon,
        top_components: top,
    }
}

/// JSON report — pretty-printed RFC8259.
pub fn to_json(summary: &ExecutiveSummary) -> String {
    serde_json::to_string_pretty(summary).expect("ExecutiveSummary is always serializable")
}

/// Minimal HTML executive summary. Used by the Portal "Download HTML"
/// button; readable without CSS. WCAG AA: semantic headings.
pub fn to_html(summary: &ExecutiveSummary) -> String {
    let s = summary;
    format!(
        r#"<!doctype html>
<html lang="en"><head>
  <meta charset="utf-8">
  <title>Vulnerability Report — {prod}</title>
</head>
<body>
  <h1>Executive Vulnerability Summary</h1>
  <p>Generated: <time>{ts}</time></p>
  <p>Product: <strong>{prod}</strong> | Engagement: <strong>{eng}</strong></p>
  <h2>Severity counts</h2>
  <ul>
    <li>Critical: <strong>{c}</strong></li>
    <li>High: <strong>{h}</strong></li>
    <li>Medium: <strong>{m}</strong></li>
    <li>Low: <strong>{l}</strong></li>
    <li>Info: <strong>{i}</strong></li>
  </ul>
  <h2>SLA</h2>
  <p>Breached: <strong>{br}</strong> · Breaching soon (≤7d): <strong>{soon}</strong></p>
  <h2>Top components</h2>
  <ol>{top}</ol>
</body></html>"#,
        prod = s.product.clone().unwrap_or_else(|| "(all)".into()),
        ts = s.generated_at,
        eng = s.engagement.clone().unwrap_or_else(|| "(all)".into()),
        c = s.by_severity.critical, h = s.by_severity.high,
        m = s.by_severity.medium, l = s.by_severity.low, i = s.by_severity.info,
        br = s.sla_breached, soon = s.sla_breaching_soon,
        top = s.top_components.iter()
            .map(|(n, c)| format!("<li>{n} (×{c})</li>"))
            .collect::<String>(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hierarchy::Product;
    use chrono::Duration;
    use uuid::Uuid;

    fn fin(sev: FindingSeverity, comp: &str, days_ago: i64) -> Finding {
        let mut f = Finding::new("X", sev);
        f.component_name = Some(comp.into());
        f.date = Utc::now() - Duration::days(days_ago);
        f
    }

    #[test]
    fn executive_summary_counts_severities() {
        let p = Product::new(Uuid::new_v4(), "App1");
        let findings = vec![
            fin(FindingSeverity::Critical, "openssl", 1),
            fin(FindingSeverity::High, "openssl", 1),
            fin(FindingSeverity::Medium, "lodash", 1),
        ];
        let s = executive_summary(Some(&p), None, &findings, &SlaConfiguration::default());
        assert_eq!(s.by_severity.critical, 1);
        assert_eq!(s.by_severity.high, 1);
        assert_eq!(s.by_severity.medium, 1);
        assert_eq!(s.total_findings, 3);
    }

    #[test]
    fn top_components_ranked_by_count() {
        let findings = vec![
            fin(FindingSeverity::Low, "openssl", 1),
            fin(FindingSeverity::Low, "openssl", 1),
            fin(FindingSeverity::Low, "lodash", 1),
        ];
        let s = executive_summary(None, None, &findings, &SlaConfiguration::default());
        assert_eq!(s.top_components[0].0, "openssl");
        assert_eq!(s.top_components[0].1, 2);
    }

    #[test]
    fn sla_breach_counts_propagate() {
        let findings = vec![fin(FindingSeverity::Critical, "x", 30)];
        let s = executive_summary(None, None, &findings, &SlaConfiguration::default());
        assert_eq!(s.sla_breached, 1);
    }

    #[test]
    fn to_json_emits_valid_json() {
        let s = executive_summary(None, None, &[], &SlaConfiguration::default());
        let j = to_json(&s);
        let _: serde_json::Value = serde_json::from_str(&j).unwrap();
    }

    #[test]
    fn to_html_includes_severity_labels() {
        let p = Product::new(Uuid::new_v4(), "MyApp");
        let s = executive_summary(Some(&p), None, &[fin(FindingSeverity::High, "x", 1)], &SlaConfiguration::default());
        let html = to_html(&s);
        assert!(html.contains("<h1>"));
        assert!(html.contains("Critical:"));
        assert!(html.contains("MyApp"));
    }

    #[test]
    fn active_findings_excludes_inactive() {
        let f1 = fin(FindingSeverity::Low, "x", 1);
        let mut f2 = fin(FindingSeverity::Low, "y", 1);
        f2.state.active = false;
        let s = executive_summary(None, None, &vec![f1, f2], &SlaConfiguration::default());
        assert_eq!(s.active_findings, 1);
        assert_eq!(s.total_findings, 2);
    }
}
