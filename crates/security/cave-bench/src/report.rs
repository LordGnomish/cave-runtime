// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Report renderers — JSON / SARIF / HTML / Markdown.
//!
//! Upstream: kube-bench `cmd/output.go` + kubescape `core/printer/`.

use crate::models::{Finding, ScanSummary, Verdict};
use serde::{Deserialize, Serialize};

/// Output format.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Format {
    Json,
    Sarif,
    Html,
    Markdown,
}

/// SARIF 2.1.0 result-shaped record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SarifResult {
    #[serde(rename = "ruleId")]
    pub rule_id: String,
    pub level: String,
    pub message: SarifMessage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SarifMessage {
    pub text: String,
}

/// SARIF run wrapper.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SarifRun {
    pub tool: SarifTool,
    pub results: Vec<SarifResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SarifTool {
    pub driver: SarifDriver,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SarifDriver {
    pub name: String,
    pub version: String,
    #[serde(rename = "informationUri")]
    pub information_uri: String,
}

/// SARIF root document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SarifLog {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub version: String,
    pub runs: Vec<SarifRun>,
}

/// Render findings + summary into the chosen format.
pub fn render(format: Format, findings: &[Finding], summary: &ScanSummary) -> String {
    match format {
        Format::Json => to_json(findings, summary),
        Format::Sarif => to_sarif(findings).expect("SARIF encode"),
        Format::Html => to_html(findings, summary),
        Format::Markdown => to_markdown(findings, summary),
    }
}

fn to_json(findings: &[Finding], summary: &ScanSummary) -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "summary": summary,
        "findings": findings,
    })).unwrap_or_else(|_| "{}".into())
}

fn to_sarif(findings: &[Finding]) -> Result<String, serde_json::Error> {
    let log = SarifLog {
        schema: "https://json.schemastore.org/sarif-2.1.0.json".into(),
        version: "2.1.0".into(),
        runs: vec![SarifRun {
            tool: SarifTool {
                driver: SarifDriver {
                    name: "cave-bench".into(),
                    version: env!("CARGO_PKG_VERSION").into(),
                    information_uri: "https://github.com/cave-runtime/cave-runtime".into(),
                },
            },
            results: findings
                .iter()
                .map(|f| SarifResult {
                    rule_id: f.check_id.clone(),
                    level: match f.verdict {
                        Verdict::Pass => "none".into(),
                        Verdict::Fail => "error".into(),
                        Verdict::Warn => "warning".into(),
                        Verdict::Error => "error".into(),
                        Verdict::Info => "note".into(),
                        Verdict::NotApplicable => "none".into(),
                    },
                    message: SarifMessage { text: f.message.clone() },
                })
                .collect(),
        }],
    };
    serde_json::to_string_pretty(&log)
}

fn to_html(findings: &[Finding], summary: &ScanSummary) -> String {
    let mut s = String::new();
    s.push_str("<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">");
    s.push_str("<title>cave-bench report</title>");
    s.push_str("<style>body{font-family:system-ui,sans-serif;margin:2em}");
    s.push_str("table{border-collapse:collapse}td,th{border:1px solid #ddd;padding:.4em .8em}");
    s.push_str(".pass{color:#0a0}.fail{color:#c00;font-weight:bold}.warn{color:#a60}</style></head><body>");
    s.push_str(&format!("<h1>cave-bench report — {}</h1>", summary.profile_id));
    s.push_str(&format!(
        "<p>Total: {} · Pass: {} · Fail: {} · Warn: {} · Score: {:.2}%</p>",
        summary.total, summary.passed, summary.failed, summary.warned, summary.score * 100.0
    ));
    s.push_str("<table><thead><tr><th>ID</th><th>Verdict</th><th>Severity</th><th>Host</th><th>Message</th></tr></thead><tbody>");
    for f in findings {
        let cls = match f.verdict {
            Verdict::Pass => "pass",
            Verdict::Fail | Verdict::Error => "fail",
            _ => "warn",
        };
        s.push_str(&format!(
            "<tr><td>{}</td><td class=\"{}\">{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            html_escape(&f.check_id),
            cls,
            f.verdict.as_str(),
            f.severity.as_str(),
            html_escape(&f.host),
            html_escape(&f.message),
        ));
    }
    s.push_str("</tbody></table></body></html>");
    s
}

fn to_markdown(findings: &[Finding], summary: &ScanSummary) -> String {
    let mut s = String::new();
    s.push_str(&format!("# cave-bench report — `{}`\n\n", summary.profile_id));
    s.push_str(&format!(
        "**Total:** {} · **Pass:** {} · **Fail:** {} · **Warn:** {} · **Errored:** {} · **Score:** {:.2}%\n\n",
        summary.total, summary.passed, summary.failed, summary.warned, summary.errored, summary.score * 100.0
    ));
    s.push_str("| ID | Verdict | Severity | Host | Message |\n");
    s.push_str("|----|---------|----------|------|---------|\n");
    for f in findings {
        s.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            f.check_id, f.verdict.as_str(), f.severity.as_str(), f.host,
            f.message.replace('|', "\\|")
        ));
    }
    s
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Check, Framework, NodeType, Severity, Target};

    fn fixture() -> (Vec<Finding>, ScanSummary) {
        let c = Check::new("cis-1.2.1", Framework::CisK8s, NodeType::Master, "anonymous-auth");
        let findings = vec![
            Finding::pass(&c, "n1", "ok"),
            Finding::fail(&c, "n1", "violation"),
        ];
        let t = Target::host_files("/etc", "n1");
        let s = ScanSummary::compute("s1", "cis-1.10", t, &findings, 0, 1);
        (findings, s)
    }

    #[test]
    fn test_render_json_valid() {
        let (f, s) = fixture();
        let out = render(Format::Json, &f, &s);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert!(v.get("summary").is_some());
        assert!(v.get("findings").is_some());
    }

    #[test]
    fn test_render_sarif_valid() {
        let (f, s) = fixture();
        let out = render(Format::Sarif, &f, &s);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["version"], "2.1.0");
        assert_eq!(v["runs"][0]["tool"]["driver"]["name"], "cave-bench");
        assert_eq!(v["runs"][0]["results"][0]["ruleId"], "cis-1.2.1");
        assert_eq!(v["runs"][0]["results"][1]["level"], "error");
    }

    #[test]
    fn test_render_html_contains_table() {
        let (f, s) = fixture();
        let out = render(Format::Html, &f, &s);
        assert!(out.contains("<table"));
        assert!(out.contains("cis-1.2.1"));
    }

    #[test]
    fn test_render_markdown_contains_pipe_table() {
        let (f, s) = fixture();
        let out = render(Format::Markdown, &f, &s);
        assert!(out.starts_with("# cave-bench"));
        assert!(out.contains("| cis-1.2.1 |"));
    }

    #[test]
    fn test_html_escape() {
        assert_eq!(html_escape("<a&b>"), "&lt;a&amp;b&gt;");
    }

    #[test]
    fn test_markdown_pipe_escape() {
        let c = Check::new("c", Framework::CisK8s, NodeType::Master, "t");
        let mut f = Finding::fail(&c, "n", "has | pipe");
        f.evidence = None;
        let t = Target::host_files("/", "n");
        let s = ScanSummary::compute("s", "p", t, &[f.clone()], 0, 0);
        let out = render(Format::Markdown, &[f], &s);
        assert!(out.contains("has \\| pipe"));
    }
}
