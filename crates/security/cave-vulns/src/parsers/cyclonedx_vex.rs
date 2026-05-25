// SPDX-License-Identifier: AGPL-3.0-or-later
//! CycloneDX VEX (Vulnerability Exploitability eXchange) parser.
//!
//! Spec: <https://cyclonedx.org/capabilities/vex/> — the `vulnerabilities[]`
//! block of a CycloneDX 1.5/1.6 document carries per-CVE analysis records:
//! `state` (`not_affected` / `exploitable` / `in_triage` / `resolved` …),
//! `justification` (`code_not_present`, `vulnerable_code_not_in_execute_path` …),
//! `response` (`will_not_fix`, `update`, `rollback` …) and `detail`.
//!
//! cave-vulns ingests these and maps them onto Finding state transitions so
//! a VEX statement of `not_affected` flips `false_p=true` while `resolved`
//! flips `is_mitigated=true`. Mirrors DependencyTrack's
//! `dt/persistence/CycloneDXVexImporter.java`.

use crate::finding::{FindingSeverity, StateTransition};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VexState {
    Resolved,
    ResolvedWithPedigree,
    Exploitable,
    InTriage,
    FalsePositive,
    NotAffected,
}

impl VexState {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "resolved" => Self::Resolved,
            "resolved_with_pedigree" | "resolved-with-pedigree" => Self::ResolvedWithPedigree,
            "exploitable" => Self::Exploitable,
            "in_triage" | "in-triage" => Self::InTriage,
            "false_positive" | "false-positive" => Self::FalsePositive,
            "not_affected" | "not-affected" => Self::NotAffected,
            _ => return None,
        })
    }

    /// Translate the VEX state into the DefectDojo state transition that
    /// most closely matches its semantics.
    pub fn into_transition(&self) -> Option<StateTransition> {
        match self {
            Self::Resolved | Self::ResolvedWithPedigree => Some(StateTransition::Mitigate),
            Self::FalsePositive => Some(StateTransition::MarkFalsePositive),
            Self::NotAffected => Some(StateTransition::MarkOutOfScope),
            Self::Exploitable => Some(StateTransition::Verify),
            Self::InTriage => Some(StateTransition::SubmitForReview),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VexJustification {
    CodeNotPresent,
    CodeNotReachable,
    RequiresConfiguration,
    RequiresDependency,
    RequiresEnvironment,
    ProtectedByCompiler,
    ProtectedAtRuntime,
    ProtectedAtPerimeter,
    ProtectedByMitigatingControl,
    Other(String),
}

impl VexJustification {
    pub fn parse(s: &str) -> Self {
        let key = s.trim().to_ascii_lowercase().replace('-', "_");
        match key.as_str() {
            "code_not_present" => Self::CodeNotPresent,
            "code_not_reachable" => Self::CodeNotReachable,
            "requires_configuration" => Self::RequiresConfiguration,
            "requires_dependency" => Self::RequiresDependency,
            "requires_environment" => Self::RequiresEnvironment,
            "protected_by_compiler" => Self::ProtectedByCompiler,
            "protected_at_runtime" => Self::ProtectedAtRuntime,
            "protected_at_perimeter" => Self::ProtectedAtPerimeter,
            "protected_by_mitigating_control" => Self::ProtectedByMitigatingControl,
            _ => Self::Other(s.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct VexAnalysis {
    pub state: VexState,
    pub justification: Option<VexJustification>,
    pub responses: Vec<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VexStatement {
    pub bom_ref: Option<String>,
    pub cve: Option<String>,
    pub severity: Option<FindingSeverity>,
    pub source: Option<String>,
    pub analysis: Option<VexAnalysis>,
}

#[derive(Debug, thiserror::Error)]
pub enum VexParseError {
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("CycloneDX document missing `vulnerabilities` array")]
    NoVulnerabilities,
}

pub fn parse_vex(json: &str) -> Result<Vec<VexStatement>, VexParseError> {
    let v: Value = serde_json::from_str(json)?;
    let arr = match v.get("vulnerabilities").and_then(|x| x.as_array()) {
        Some(a) => a,
        None => return Err(VexParseError::NoVulnerabilities),
    };
    Ok(arr.iter().map(parse_one).collect())
}

fn parse_one(v: &Value) -> VexStatement {
    let bom_ref = v.get("bom-ref").and_then(Value::as_str).map(String::from);
    let cve = v.get("id").and_then(Value::as_str).map(String::from);
    let severity = v
        .get("ratings")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(|r| r.get("severity").and_then(Value::as_str))
        .and_then(FindingSeverity::parse);
    let source = v
        .get("source")
        .and_then(|s| s.get("name").and_then(Value::as_str))
        .map(String::from);
    let analysis = v.get("analysis").map(parse_analysis);
    VexStatement {
        bom_ref,
        cve,
        severity,
        source,
        analysis,
    }
}

fn parse_analysis(v: &Value) -> VexAnalysis {
    let state = v
        .get("state")
        .and_then(Value::as_str)
        .and_then(VexState::parse)
        .unwrap_or(VexState::InTriage);
    let justification = v
        .get("justification")
        .and_then(Value::as_str)
        .map(VexJustification::parse);
    let responses = v
        .get("response")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let detail = v.get("detail").and_then(Value::as_str).map(String::from);
    VexAnalysis {
        state,
        justification,
        responses,
        detail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_resolved_vex() {
        let json = r#"{
            "vulnerabilities": [{
                "id": "CVE-2023-12345",
                "source": {"name": "NVD"},
                "ratings": [{"severity": "critical"}],
                "analysis": {
                    "state": "resolved",
                    "justification": "code_not_reachable",
                    "response": ["update"],
                    "detail": "Upgraded to 2.0.1"
                }
            }]
        }"#;
        let v = parse_vex(json).unwrap();
        assert_eq!(v.len(), 1);
        let stmt = &v[0];
        assert_eq!(stmt.cve.as_deref(), Some("CVE-2023-12345"));
        assert_eq!(stmt.severity, Some(FindingSeverity::Critical));
        let an = stmt.analysis.as_ref().unwrap();
        assert_eq!(an.state, VexState::Resolved);
        assert_eq!(an.justification, Some(VexJustification::CodeNotReachable));
        assert_eq!(an.responses, vec!["update"]);
        assert_eq!(an.detail.as_deref(), Some("Upgraded to 2.0.1"));
    }

    #[test]
    fn parse_not_affected_vex() {
        let json = r#"{
            "vulnerabilities": [{
                "id": "CVE-2024-1",
                "analysis": {"state": "not_affected", "justification": "code_not_present"}
            }]
        }"#;
        let stmt = &parse_vex(json).unwrap()[0];
        let an = stmt.analysis.as_ref().unwrap();
        assert_eq!(an.state, VexState::NotAffected);
        assert_eq!(an.state.into_transition(), Some(StateTransition::MarkOutOfScope));
    }

    #[test]
    fn parse_false_positive() {
        let json = r#"{"vulnerabilities":[{"id":"CVE-X","analysis":{"state":"false_positive"}}]}"#;
        let stmt = &parse_vex(json).unwrap()[0];
        assert_eq!(stmt.analysis.as_ref().unwrap().state, VexState::FalsePositive);
        assert_eq!(
            stmt.analysis.as_ref().unwrap().state.into_transition(),
            Some(StateTransition::MarkFalsePositive)
        );
    }

    #[test]
    fn parse_exploitable_marks_verify() {
        let an = VexAnalysis {
            state: VexState::Exploitable,
            justification: None,
            responses: vec![],
            detail: None,
        };
        assert_eq!(an.state.into_transition(), Some(StateTransition::Verify));
    }

    #[test]
    fn vex_state_parse_dash_and_underscore() {
        assert_eq!(VexState::parse("not-affected"), Some(VexState::NotAffected));
        assert_eq!(VexState::parse("not_affected"), Some(VexState::NotAffected));
        assert_eq!(VexState::parse("False-Positive"), Some(VexState::FalsePositive));
    }

    #[test]
    fn vex_justification_unknown_falls_back_to_other() {
        let j = VexJustification::parse("legacy_reason");
        assert_eq!(j, VexJustification::Other("legacy_reason".to_string()));
    }

    #[test]
    fn parse_empty_vulnerabilities() {
        let v = parse_vex(r#"{"vulnerabilities": []}"#).unwrap();
        assert!(v.is_empty());
    }

    #[test]
    fn parse_missing_vulnerabilities_errors() {
        let err = parse_vex(r#"{"bomFormat":"CycloneDX"}"#).unwrap_err();
        assert!(matches!(err, VexParseError::NoVulnerabilities));
    }

    #[test]
    fn parse_invalid_json() {
        assert!(parse_vex("not json").is_err());
    }

    #[test]
    fn parse_no_analysis_block() {
        let stmt = &parse_vex(r#"{"vulnerabilities":[{"id":"CVE-A"}]}"#).unwrap()[0];
        assert!(stmt.analysis.is_none());
        assert_eq!(stmt.cve.as_deref(), Some("CVE-A"));
    }

    #[test]
    fn parse_bom_ref_carries_through() {
        let json = r#"{"vulnerabilities":[{"bom-ref":"ref-1","id":"CVE-1"}]}"#;
        let stmt = &parse_vex(json).unwrap()[0];
        assert_eq!(stmt.bom_ref.as_deref(), Some("ref-1"));
    }
}
