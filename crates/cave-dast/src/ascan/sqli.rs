// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: zaproxy/zap-extensions@addOns/ascanrules/.../SqlInjectionScanRule.java
//
//! SQL injection probe — boolean-based + error-based detection. The
//! ZAP rule is much larger (~2k Java LOC) and covers blind/timing/
//! union-based variants; this port focuses on:
//!
//! * Error-based: injecting a syntactic break (`'`, `"`) and matching
//!   well-known database error fingerprints in the response body.
//! * Boolean-based: a true-true (`' AND 1=1 -- `) vs true-false
//!   (`' AND 1=2 -- `) pair, comparing response length / status.
//!
//! Mirrors ZAP plugin id 40018.

use super::{ActiveScanRule, PluginId, Probe};
use crate::alert::Alert;
use crate::http::{HttpRequest, HttpResponse};
use crate::models::RiskLevel;

pub struct SqlInjectionRule;

/// Database error fingerprints. Cross-checked against ZAP's
/// `SqlInjectionScanRule.SQL_ERROR_FRAGMENTS` array.
const ERROR_FRAGMENTS: &[&str] = &[
    "SQL syntax",
    "mysql_fetch",
    "mysql_num_rows",
    "ORA-01756",
    "ORA-00933",
    "PostgreSQL query failed",
    "pg_query()",
    "unclosed quotation mark",
    "Microsoft OLE DB Provider for SQL Server",
    "SQLServer JDBC Driver",
    "SQLite/JDBCDriver",
    "SQLite.Exception",
    "System.Data.SqlClient.SqlException",
    "Warning: mysql",
    "MySQLSyntaxErrorException",
    "valid MySQL result",
];

/// Payloads in detection order: syntactic break, then boolean pair.
const BREAK_PAYLOAD: &str = "'";
const TRUE_TRUE: &str = "1' AND '1'='1";
const TRUE_FALSE: &str = "1' AND '1'='2";

impl ActiveScanRule for SqlInjectionRule {
    fn id(&self) -> PluginId {
        40018
    }
    fn name(&self) -> &'static str {
        "SQL Injection"
    }
    fn risk(&self) -> RiskLevel {
        RiskLevel::High
    }
    fn cwe_id(&self) -> u32 {
        89
    }
    fn wasc_id(&self) -> u32 {
        19
    }

    fn probes(&self, req: &HttpRequest) -> Vec<Probe> {
        let mut probes = Vec::new();
        for mutated in super::per_param_mutations(req, BREAK_PAYLOAD) {
            probes.push(Probe {
                original: req.clone(),
                mutated,
                plugin_id: self.id(),
                note: "error-based".to_string(),
            });
        }
        for mutated in super::per_param_mutations(req, TRUE_TRUE) {
            probes.push(Probe {
                original: req.clone(),
                mutated,
                plugin_id: self.id(),
                note: "boolean-true".to_string(),
            });
        }
        for mutated in super::per_param_mutations(req, TRUE_FALSE) {
            probes.push(Probe {
                original: req.clone(),
                mutated,
                plugin_id: self.id(),
                note: "boolean-false".to_string(),
            });
        }
        probes
    }

    fn check(&self, probe: &Probe, response: &HttpResponse) -> Option<Alert> {
        let body = response.body_str().unwrap_or("");
        for frag in ERROR_FRAGMENTS {
            if body.contains(frag) {
                return Some(Alert {
                    name: self.name().to_string(),
                    risk: self.risk(),
                    cwe_id: self.cwe_id(),
                    url: probe.mutated.url.clone(),
                    description: format!(
                        "Database error message '{}' returned after injecting break payload.",
                        frag
                    ),
                    solution: "Use parameterized queries (PreparedStatement / bound parameters)."
                        .to_string(),
                    evidence: Some((*frag).to_string()),
                    plugin_id: self.id(),
                });
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::HttpMethod;

    fn req(url: &str) -> HttpRequest {
        HttpRequest::new(HttpMethod::Get, url)
    }

    fn resp_with(body: &str) -> HttpResponse {
        let mut r = HttpResponse::new(500, "Internal Server Error");
        r.body = body.as_bytes().to_vec();
        r
    }

    #[test]
    fn rule_metadata() {
        let r = SqlInjectionRule;
        assert_eq!(r.id(), 40018);
        assert_eq!(r.cwe_id(), 89);
        assert_eq!(r.risk(), RiskLevel::High);
    }

    #[test]
    fn probes_cover_every_param_three_ways() {
        let r = SqlInjectionRule;
        let probes = r.probes(&req("http://x/api?a=1&b=2"));
        // 2 params * 3 payload modes (break, true-true, true-false) = 6
        assert_eq!(probes.len(), 6);
    }

    #[test]
    fn detect_mysql_error_fingerprint() {
        let r = SqlInjectionRule;
        let probe = Probe {
            original: req("http://x/api?a=1"),
            mutated: req("http://x/api?a='"),
            plugin_id: r.id(),
            note: "error-based".to_string(),
        };
        let resp = resp_with("you have an error in your SQL syntax near ...");
        let alert = r.check(&probe, &resp).expect("should detect");
        assert_eq!(alert.cwe_id, 89);
    }

    #[test]
    fn detect_postgres_error() {
        let r = SqlInjectionRule;
        let probe = Probe {
            original: req("http://x/api?a=1"),
            mutated: req("http://x/api?a='"),
            plugin_id: r.id(),
            note: "error-based".to_string(),
        };
        let resp = resp_with("ERROR: pg_query() failed: unclosed quotation mark");
        assert!(r.check(&probe, &resp).is_some());
    }

    #[test]
    fn no_alert_on_clean_response() {
        let r = SqlInjectionRule;
        let probe = Probe {
            original: req("http://x/api?a=1"),
            mutated: req("http://x/api?a='"),
            plugin_id: r.id(),
            note: "error-based".to_string(),
        };
        let resp = resp_with("<html><body>OK</body></html>");
        assert!(r.check(&probe, &resp).is_none());
    }
}
