// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Minimal GraphQL surface — query-only portfolio + findings.
//!
//! Mirrors `org.dependencytrack.graphql` (the optional GraphQL endpoint
//! exposed alongside the REST API).  We implement a hand-rolled parser
//! that accepts the four queries we need:
//!   - `{ projects { uuid name version } }`
//!   - `{ vulnerabilities { vulnId severity } }`
//!   - `{ policies { uuid name } }`
//!   - `{ schema }`
//!
//! Returns `serde_json::Value` ready to wrap in a `{ "data": ... }` envelope.

use crate::models::{Project, Vulnerability};
use crate::policy::engine::Policy;
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq)]
pub enum GqlQuery {
    Projects,
    Vulnerabilities,
    Policies,
    Schema,
    Unknown(String),
}

/// Hand-rolled extractor — finds the first top-level field in a `{ field }`.
pub fn parse_query(raw: &str) -> GqlQuery {
    let trimmed = raw.trim().trim_start_matches('{').trim_end_matches('}').trim();
    let first = trimmed
        .split(|c: char| c.is_whitespace() || c == '{' || c == '(')
        .find(|s| !s.is_empty())
        .unwrap_or("");
    match first {
        "projects" => GqlQuery::Projects,
        "vulnerabilities" => GqlQuery::Vulnerabilities,
        "policies" => GqlQuery::Policies,
        "schema" | "__schema" => GqlQuery::Schema,
        other => GqlQuery::Unknown(other.to_string()),
    }
}

pub fn execute(
    query: &str,
    projects: &[Project],
    vulns: &[Vulnerability],
    policies: &[Policy],
) -> Value {
    match parse_query(query) {
        GqlQuery::Projects => json!({
            "data": {
                "projects": projects.iter().map(|p| json!({
                    "uuid": p.uuid.to_string(),
                    "name": p.name,
                    "version": p.version,
                    "classifier": p.classifier.as_str(),
                    "tags": p.tags,
                })).collect::<Vec<_>>()
            }
        }),
        GqlQuery::Vulnerabilities => json!({
            "data": {
                "vulnerabilities": vulns.iter().map(|v| json!({
                    "vulnId": v.vuln_id,
                    "source": format!("{:?}", v.source).to_uppercase(),
                    "severity": format!("{:?}", v.severity).to_uppercase(),
                    "cvssV3BaseScore": v.cvss_v3_base_score,
                })).collect::<Vec<_>>()
            }
        }),
        GqlQuery::Policies => json!({
            "data": {
                "policies": policies.iter().map(|p| json!({
                    "uuid": p.uuid.to_string(),
                    "name": p.name,
                    "conditions": p.conditions.len(),
                })).collect::<Vec<_>>()
            }
        }),
        GqlQuery::Schema => json!({
            "data": {
                "__schema": {
                    "queryType": {"name": "Query"},
                    "types": [
                        {"name": "Project"},
                        {"name": "Vulnerability"},
                        {"name": "Policy"},
                    ]
                }
            }
        }),
        GqlQuery::Unknown(field) => json!({
            "errors": [{"message": format!("unknown field: {}", field)}]
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Classifier, Severity, VulnSource};

    #[test]
    fn parse_basic_fields() {
        assert_eq!(parse_query("{ projects { name } }"), GqlQuery::Projects);
        assert_eq!(parse_query("{ vulnerabilities }"), GqlQuery::Vulnerabilities);
        assert_eq!(parse_query("{ policies }"), GqlQuery::Policies);
        assert_eq!(parse_query("{ __schema }"), GqlQuery::Schema);
        assert!(matches!(parse_query("{ nope }"), GqlQuery::Unknown(_)));
    }

    #[test]
    fn execute_projects_returns_data_envelope() {
        let p = Project::new("cave", Classifier::Application);
        let v = execute("{ projects }", &[p.clone()], &[], &[]);
        let arr = v["data"]["projects"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["name"], "cave");
        assert_eq!(arr[0]["classifier"], "APPLICATION");
    }

    #[test]
    fn execute_vulnerabilities_lists_summaries() {
        let mut vu = Vulnerability::new("CVE-1", VulnSource::Nvd);
        vu.severity = Severity::Critical;
        let v = execute("{ vulnerabilities }", &[], &[vu], &[]);
        assert_eq!(v["data"]["vulnerabilities"][0]["severity"], "CRITICAL");
    }

    #[test]
    fn execute_unknown_field_returns_errors() {
        let v = execute("{ noSuchField }", &[], &[], &[]);
        assert!(v.get("errors").is_some());
    }

    #[test]
    fn execute_schema_returns_introspection() {
        let v = execute("{ __schema }", &[], &[], &[]);
        assert_eq!(v["data"]["__schema"]["queryType"]["name"], "Query");
        assert_eq!(v["data"]["__schema"]["types"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn execute_policies_includes_condition_count() {
        let p = Policy::new("strict");
        let v = execute("{ policies }", &[], &[], &[p.clone()]);
        assert_eq!(v["data"]["policies"][0]["conditions"], 0);
    }
}
