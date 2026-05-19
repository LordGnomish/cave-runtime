// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: zaproxy/zap-extensions@addOns/reports/.../HtmlReport.java
//
//! HTML report renderer. Mirrors ZAP's classic HTML report — a single
//! self-contained HTML document grouping alerts by risk.

use crate::alert::{cwe_to_owasp, Alert};
use crate::models::RiskLevel;

pub fn render_html_report(target: &str, alerts: &[Alert]) -> String {
    let mut s = String::new();
    s.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n");
    s.push_str("<meta charset=\"utf-8\">\n");
    s.push_str(&format!(
        "<title>cave-dast report — {}</title>\n",
        html_escape(target)
    ));
    s.push_str(
        "<style>\
body{font-family:system-ui,sans-serif;max-width:1100px;margin:2rem auto;padding:0 1rem}\
h1{margin-top:0}\
.summary{display:flex;gap:1rem;margin:1rem 0}\
.pill{padding:.3rem .6rem;border-radius:.4rem;color:#fff;font-weight:600;font-size:.9rem}\
.High{background:#b91c1c}.Medium{background:#d97706}.Low{background:#0369a1}.Informational{background:#4b5563}\
table{width:100%;border-collapse:collapse;margin-top:1rem}\
th,td{border-bottom:1px solid #ddd;padding:.5rem .6rem;vertical-align:top;text-align:left}\
th{background:#f5f5f5}\
.alert{margin:1.2rem 0;padding:1rem;border-left:4px solid #ddd;background:#fafafa}\
.alert.High{border-left-color:#b91c1c}\
.alert.Medium{border-left-color:#d97706}\
.alert.Low{border-left-color:#0369a1}\
.alert.Informational{border-left-color:#4b5563}\
code{background:#eee;padding:.1rem .3rem;border-radius:.2rem}\
</style>\n",
    );
    s.push_str("</head>\n<body>\n");
    s.push_str(&format!("<h1>cave-dast report</h1>\n"));
    s.push_str(&format!(
        "<p><strong>Target:</strong> <code>{}</code></p>\n",
        html_escape(target)
    ));

    let mut high = 0;
    let mut medium = 0;
    let mut low = 0;
    let mut info = 0;
    for a in alerts {
        match a.risk {
            RiskLevel::High => high += 1,
            RiskLevel::Medium => medium += 1,
            RiskLevel::Low => low += 1,
            RiskLevel::Informational => info += 1,
        }
    }
    s.push_str("<div class=\"summary\">\n");
    s.push_str(&format!(
        "<span class=\"pill High\">High: {}</span>",
        high
    ));
    s.push_str(&format!(
        "<span class=\"pill Medium\">Medium: {}</span>",
        medium
    ));
    s.push_str(&format!(
        "<span class=\"pill Low\">Low: {}</span>",
        low
    ));
    s.push_str(&format!(
        "<span class=\"pill Informational\">Informational: {}</span>",
        info
    ));
    s.push_str("</div>\n");

    s.push_str("<h2>Findings</h2>\n");
    if alerts.is_empty() {
        s.push_str("<p>No issues detected.</p>\n");
    } else {
        for a in alerts {
            let risk_class = match a.risk {
                RiskLevel::High => "High",
                RiskLevel::Medium => "Medium",
                RiskLevel::Low => "Low",
                RiskLevel::Informational => "Informational",
            };
            s.push_str(&format!("<div class=\"alert {}\">\n", risk_class));
            s.push_str(&format!(
                "<h3>{} <span class=\"pill {}\">{}</span></h3>\n",
                html_escape(&a.name),
                risk_class,
                risk_class
            ));
            s.push_str(&format!(
                "<p><strong>URL:</strong> <code>{}</code></p>\n",
                html_escape(&a.url)
            ));
            s.push_str(&format!(
                "<p><strong>CWE:</strong> {}{}</p>\n",
                a.cwe_id,
                cwe_to_owasp(a.cwe_id)
                    .map(|c| format!(" &middot; <strong>OWASP:</strong> {}", c.code()))
                    .unwrap_or_default()
            ));
            s.push_str(&format!(
                "<p><strong>Plugin id:</strong> {}</p>\n",
                a.plugin_id
            ));
            s.push_str(&format!(
                "<p>{}</p>\n",
                html_escape(&a.description)
            ));
            if let Some(ev) = &a.evidence {
                s.push_str(&format!(
                    "<p><strong>Evidence:</strong> <code>{}</code></p>\n",
                    html_escape(ev)
                ));
            }
            s.push_str(&format!(
                "<p><strong>Solution:</strong> {}</p>\n",
                html_escape(&a.solution)
            ));
            s.push_str("</div>\n");
        }
    }

    s.push_str("</body>\n</html>\n");
    s
}

fn html_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_alerts() -> Vec<Alert> {
        vec![
            Alert {
                name: "SQL Injection".to_string(),
                risk: RiskLevel::High,
                cwe_id: 89,
                url: "http://x/api?id=1".to_string(),
                description: "Database error returned.".to_string(),
                solution: "Use parameterized queries.".to_string(),
                evidence: Some("SQL syntax".to_string()),
                plugin_id: 40018,
            },
            Alert {
                name: "Missing X-Frame-Options".to_string(),
                risk: RiskLevel::Low,
                cwe_id: 693,
                url: "http://x/".to_string(),
                description: "Header missing.".to_string(),
                solution: "Add DENY.".to_string(),
                evidence: None,
                plugin_id: 10020,
            },
        ]
    }

    #[test]
    fn report_contains_doctype_and_title() {
        let html = render_html_report("http://x/", &sample_alerts());
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("<title>cave-dast report — http://x/</title>"));
    }

    #[test]
    fn report_summary_counts() {
        let html = render_html_report("t", &sample_alerts());
        assert!(html.contains("High: 1"));
        assert!(html.contains("Medium: 0"));
        assert!(html.contains("Low: 1"));
    }

    #[test]
    fn report_escapes_user_content() {
        let alerts = vec![Alert {
            name: "<script>evil()</script>".to_string(),
            risk: RiskLevel::High,
            cwe_id: 79,
            url: "http://x/?<a>=1".to_string(),
            description: "x".to_string(),
            solution: "y".to_string(),
            evidence: Some("<b>".to_string()),
            plugin_id: 1,
        }];
        let html = render_html_report("t", &alerts);
        assert!(!html.contains("<script>evil()</script>"));
        assert!(html.contains("&lt;script&gt;evil()&lt;/script&gt;"));
        assert!(html.contains("&lt;a&gt;"));
        assert!(html.contains("&lt;b&gt;"));
    }

    #[test]
    fn report_owasp_link_present() {
        let html = render_html_report("t", &sample_alerts());
        assert!(html.contains("A03:2021"));
        assert!(html.contains("A05:2021")); // CWE 693 → A05.
    }

    #[test]
    fn empty_alerts_renders_no_issues() {
        let html = render_html_report("t", &[]);
        assert!(html.contains("No issues detected."));
    }
}
